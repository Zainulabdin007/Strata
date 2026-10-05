//! Loopback HTTP transport; all engine operations live in strata-engine.
use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use strata_engine as engine;
use strata_metrics::{MetricsHandle, MetricsSnapshot};
use tiny_http::{Header, Request, Response, Server};
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long, default_value_t = 9160)]
    port: u16,
}
#[derive(Clone, Serialize, Deserialize)]
struct Job {
    id: u64,
    operation: String,
    dataset: String,
    status: String,
    result: Option<Value>,
    error: Option<String>,
}
#[derive(Default)]
struct Jobs {
    next: u64,
    history: Vec<Job>,
    metrics: Option<MetricsHandle>,
    final_metrics: Option<MetricsSnapshot>,
}
struct State {
    root: PathBuf,
    port: u16,
    jobs: Mutex<Jobs>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobRequest {
    operation: String,
    dataset: String,
    input: Option<String>,
    profile: Option<String>,
    goal: Option<String>,
    experiment: Option<engine::ExperimentOptions>,
    #[serde(default)]
    options: engine::IngestOptions,
}
fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
impl State {
    fn dataset(&self, name: &str) -> Result<PathBuf> {
        if !valid_name(name) {
            bail!("invalid dataset name");
        }
        engine::confined(&self.root, &format!("datasets/{name}"))
    }
    fn metrics(&self) -> Value {
        let j = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        json!({"metrics":j.final_metrics.clone().or_else(||j.metrics.as_ref().map(MetricsHandle::snapshot)),"job":j.history.iter().rev().find(|j|j.operation=="ingest")})
    }
    fn start(self: &Arc<Self>, r: JobRequest) -> Result<Value> {
        if !valid_name(&r.dataset) {
            bail!("invalid dataset name");
        }
        if !["ingest", "analysis", "optimization"].contains(&r.operation.as_str()) {
            bail!("unknown operation");
        }
        let dataset = if r.operation == "ingest" {
            engine::confined(&self.root, "datasets")?;
            let path = self.root.join("datasets").join(&r.dataset);
            if path.exists() {
                engine::confined(&self.root, &format!("datasets/{}", r.dataset))?;
            }
            path
        } else {
            self.dataset(&r.dataset)?
        };
        let input = r
            .input
            .as_deref()
            .map(|p| engine::confined(&self.root, p))
            .transpose()?;
        let profile = r
            .profile
            .as_deref()
            .map(|p| engine::confined(&self.root, p))
            .transpose()?;
        if r.operation == "ingest" && (input.is_none() || profile.is_none()) {
            bail!("ingest requires input and profile");
        }
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if jobs.history.iter().any(|j| j.status == "running") {
            bail!("an engine job is already running");
        }
        jobs.next += 1;
        let id = jobs.next;
        let job = Job {
            id,
            operation: r.operation.clone(),
            dataset: r.dataset.clone(),
            status: "running".into(),
            result: None,
            error: None,
        };
        if jobs.history.len() >= 128 {
            jobs.history.remove(0);
        }
        jobs.history.push(job.clone());
        let metrics = MetricsHandle::new();
        if r.operation == "ingest" {
            jobs.metrics = Some(metrics.clone());
            jobs.final_metrics = None;
        }
        drop(jobs);
        let state = Arc::clone(self);
        std::thread::spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Value> {
                    match r.operation.as_str() {
                        "ingest" => {
                            let text = engine::read_small(&profile.context("profile required")?)?;
                            let p = engine::parse_profile(&text)?;
                            let snap = engine::ingest(
                                &input.context("input required")?,
                                &dataset,
                                &p,
                                &r.options,
                                metrics.clone(),
                            )?;
                            std::fs::write(dataset.join("profile.toml"), text)?;
                            engine::save_json(&dataset.join("ingest-metrics.json"), &snap)?;
                            Ok(json!(snap))
                        }
                        "analysis" => {
                            let v = engine::analyze(&dataset)?;
                            engine::save_json(&dataset.join("analysis.json"), &v)?;
                            Ok(v)
                        }
                        "optimization" => {
                            let p = engine::parse_profile(&engine::read_small(
                                &profile.unwrap_or_else(|| dataset.join("profile.toml")),
                            )?)?;
                            let mut v = engine::optimize(
                                &dataset,
                                &p,
                                r.goal.as_deref().unwrap_or("balanced"),
                            )?;
                            if let Some(options) = &r.experiment {
                                v["experiment"] = engine::experiment(
                                    &dataset,
                                    &p,
                                    r.goal.as_deref().unwrap_or("balanced"),
                                    options,
                                )?;
                            }
                            engine::save_json(&dataset.join("optimization.json"), &v)?;
                            Ok(v)
                        }
                        _ => bail!("unknown operation"),
                    }
                }))
                .unwrap_or_else(|_| {
                    Err(anyhow::anyhow!("engine worker panicked; operation failed"))
                });
            let mut jobs = state.jobs.lock().unwrap_or_else(|e| e.into_inner());
            if r.operation == "ingest" {
                jobs.final_metrics = Some(metrics.finish());
            }
            if let Some(j) = jobs.history.iter_mut().find(|j| j.id == id) {
                match result {
                    Ok(v) => {
                        j.status = "complete".into();
                        j.result = Some(v)
                    }
                    Err(e) => {
                        j.status = "failed".into();
                        j.error = Some(format!("{e:#}"))
                    }
                }
                let _ = engine::save_json(&state.root.join("jobs").join(format!("{id}.json")), j);
            }
        });
        Ok(json!(job))
    }
}
fn route(s: &Arc<State>, method: &str, path: &str, body: &[u8]) -> Result<Value> {
    let p: Vec<_> = path.trim_matches('/').split('/').collect();
    match (method, p.as_slice()) {
        ("GET", ["api", "health"]) => {
            Ok(json!({"status":"ok","local_only":true,"metrics_source":"strata-metrics"}))
        }
        ("GET", ["api", "metrics"]) | ("GET", ["api", "ingest"]) => Ok(s.metrics()),
        ("GET", ["api", "jobs"]) => Ok(json!(
            s.jobs.lock().unwrap_or_else(|e| e.into_inner()).history
        )),
        ("GET", ["api", "jobs", id]) => {
            let id: u64 = id.parse()?;
            Ok(json!(s
                .jobs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .history
                .iter()
                .find(|j| j.id == id)
                .context("job not found")?))
        }
        ("POST", ["api", "jobs"]) => s.start(serde_json::from_slice(body)?),
        ("POST", ["api", "profiles"]) => {
            let value: Value = serde_json::from_slice(body)?;
            let id = value["job_id"].as_u64().context("job_id required")?;
            let name = value["name"].as_str().context("name required")?;
            if !valid_name(name) {
                bail!("invalid profile name");
            }
            let jobs = s.jobs.lock().unwrap_or_else(|e| e.into_inner());
            let j = jobs
                .history
                .iter()
                .find(|j| j.id == id && j.status == "complete" && j.operation == "optimization")
                .context("completed optimization job required")?;
            let result = j.result.as_ref().context("missing result")?;
            let text = result["experiment"]["profile_toml"]
                .as_str()
                .or_else(|| result["report"]["profile_toml"].as_str())
                .context("profile missing")?;
            engine::parse_profile(text)?;
            let path = s.root.join("profiles").join(format!("{name}.toml"));
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            Ok(json!({"path":format!("profiles/{name}.toml")}))
        }
        ("GET", ["api", "datasets"]) => {
            let mut out = Vec::new();
            for e in std::fs::read_dir(s.root.join("datasets"))? {
                let e = e?;
                let name = e.file_name().to_string_lossy().into_owned();
                if e.file_type()?.is_dir()
                    && valid_name(&name)
                    && e.path().join("manifest.json").exists()
                {
                    match engine::inspect(&e.path()) {Ok(v)=>out.push(json!({"name":name,"metrics":v["metrics"],"schema":v["manifest"]["schema"]})),Err(e)=>out.push(json!({"name":name,"error":e.to_string()}))}
                }
            }
            out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
            Ok(json!(out))
        }
        ("GET", ["api", "datasets", name]) | ("GET", ["api", "datasets", name, "metadata"]) => {
            engine::inspect(&s.dataset(name)?)
        }
        ("GET", ["api", "datasets", name, "segments"]) => {
            Ok(engine::inspect(&s.dataset(name)?)?["segments"].clone())
        }
        ("GET", ["api", "datasets", name, "blocks"]) => {
            let d = engine::inspect(&s.dataset(name)?)?;
            let mut out = Vec::new();
            for seg in d["segments"].as_array().context("segments missing")? {
                for b in seg["metadata"]["blocks"]
                    .as_array()
                    .context("blocks missing")?
                {
                    out.push(json!({"segment":seg["ordinal"],"block":b}));
                }
            }
            Ok(json!(out))
        }
        ("GET", ["api", "datasets", name, kind])
            if ["columns", "plans", "metrics"].contains(kind) =>
        {
            let d = s.dataset(name)?;
            let f = match *kind {
                "columns" => "analysis.json",
                "plans" => "optimization.json",
                _ => "ingest-metrics.json",
            };
            if d.join(f).exists() {
                Ok(serde_json::from_str(&engine::read_small(&d.join(f))?)?)
            } else {
                Ok(Value::Null)
            }
        }
        ("POST", ["api", "datasets", name, "query"]) => {
            let b: Value = serde_json::from_slice(body)?;
            engine::query(
                &s.dataset(name)?,
                b["query"].as_str().context("query required")?,
            )
        }
        ("GET", ["api", "benchmarks"]) => {
            let mut out = Vec::new();
            for dir in [
                s.root.join("benchmarks/strata-bench"),
                s.root.join("benchmarks/baseline"),
                s.root.join("benchmarks/verified-gates"),
                s.root.join("benchmarks/nasdaq/results"),
            ] {
                if !dir.exists() {
                    continue;
                }
                for e in std::fs::read_dir(dir)?.take(1000) {
                    let e = e?;
                    if e.path().extension().is_some_and(|v| v == "json") {
                        match engine::read_small(&e.path())
                            .and_then(|v| Ok(serde_json::from_str::<Value>(&v)?))
                        {
                            Ok(v) => out.push(v),
                            Err(e) => out.push(json!({"error":e.to_string()})),
                        }
                    }
                }
            }
            Ok(json!(out))
        }
        _ => bail!("endpoint not found"),
    }
}
fn handle(s: Arc<State>, mut r: Request) {
    let host = r
        .headers()
        .iter()
        .find(|h| h.field.equiv("Host"))
        .map(|h| h.value.as_str())
        .unwrap_or("");
    let origin = r
        .headers()
        .iter()
        .find(|h| h.field.equiv("Origin"))
        .map(|h| h.value.as_str());
    let hosts = [
        format!("127.0.0.1:{}", s.port),
        format!("localhost:{}", s.port),
    ];
    let origins = [
        format!("http://127.0.0.1:{}", s.port),
        format!("http://localhost:{}", s.port),
        "http://127.0.0.1:5173".into(),
        "http://localhost:5173".into(),
    ];
    if !hosts.iter().any(|h| h == host) || origin.is_some_and(|o| !origins.iter().any(|v| v == o)) {
        let _ = r.respond(Response::from_string("local origin required").with_status_code(403));
        return;
    }
    let method = r.method().as_str().to_owned();
    let path = r.url().split('?').next().unwrap_or("").to_owned();
    if method == "POST"
        && !r
            .headers()
            .iter()
            .any(|h| h.field.equiv("X-Strata-Local") && h.value.as_str() == "1")
    {
        let _ =
            r.respond(Response::from_string("X-Strata-Local: 1 required").with_status_code(403));
        return;
    }
    if method == "GET" && path == "/api/events" {
        let mut w = r.into_writer();
        if write!(w,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n").is_err(){return;}
        for _ in 0..15 {
            if writeln!(w, "event: metrics\ndata: {}\n", s.metrics())
                .and_then(|_| w.flush())
                .is_err()
            {
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        return;
    }
    if method == "GET" && !path.starts_with("/api/") {
        let static_root = s.root.join("strata-lab/dist");
        let relative = if path == "/" {
            "index.html"
        } else {
            path.trim_start_matches('/')
        };
        if let Ok(root) = static_root.canonicalize() {
            if let Ok(file) = engine::confined(&root, relative) {
                if file.is_file() {
                    let mime = match file.extension().and_then(|s| s.to_str()) {
                        Some("js") => "text/javascript",
                        Some("css") => "text/css",
                        Some("html") => "text/html",
                        _ => "application/octet-stream",
                    };
                    if let Ok(data) = std::fs::read(file) {
                        let _ = r.respond(
                            Response::from_data(data)
                                .with_header(Header::from_bytes("Content-Type", mime).unwrap()),
                        );
                        return;
                    }
                }
            }
        }
        let _ = r.respond(Response::empty(404));
        return;
    }
    if r.body_length().is_some_and(|n| n > 1024 * 1024) {
        let _ = r.respond(Response::empty(413));
        return;
    }
    let mut b = Vec::new();
    if r.as_reader()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut b)
        .is_err()
        || b.len() > 1024 * 1024
    {
        let _ = r.respond(Response::empty(413));
        return;
    }
    let (status, value) = match route(&s, &method, &path, &b) {
        Ok(v) => (200, v),
        Err(e) => (400, json!({"error":format!("{e:#}")})),
    };
    let response = Response::from_string(value.to_string())
        .with_status_code(status)
        .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
    let _ = r.respond(response);
}
fn main() -> Result<()> {
    let a = Args::parse();
    let root = a.root.canonicalize()?;
    for d in ["datasets", "jobs", "profiles"] {
        std::fs::create_dir_all(root.join(d))?;
        engine::confined(&root, d)?;
    }
    let mut jobs = Jobs::default();
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(root.join("jobs"))? {
        let entry = entry?;
        if let Some(id) = entry
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<u64>().ok())
        {
            jobs.next = jobs.next.max(id);
            entries.push((id, entry.path()));
        }
    }
    entries.sort_by_key(|(id, _)| std::cmp::Reverse(*id));
    for (_, path) in entries.into_iter().take(128).rev() {
        if let Ok(text) = engine::read_small(&path) {
            if let Ok(mut job) = serde_json::from_str::<Job>(&text) {
                if job.status == "running" || job.status == "queued" {
                    job.status = "failed".into();
                    job.error =
                        Some("server restarted; use safe ingest resume if applicable".into());
                }
                jobs.history.push(job);
            }
        }
    }
    let s = Arc::new(State {
        root,
        port: a.port,
        jobs: Mutex::new(jobs),
    });
    let server = Server::http((std::net::Ipv4Addr::LOCALHOST, a.port))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("Strata local API http://127.0.0.1:{}", a.port);
    let (tx, rx) = crossbeam_channel::bounded::<Request>(32);
    for _ in 0..8 {
        let rx = rx.clone();
        let s = Arc::clone(&s);
        std::thread::spawn(move || {
            for r in rx {
                handle(Arc::clone(&s), r);
            }
        });
    }
    for r in server.incoming_requests() {
        if let Err(crossbeam_channel::TrySendError::Full(r)) = tx.try_send(r) {
            let _ = r.respond(Response::empty(503));
        }
    }
    Ok(())
}
