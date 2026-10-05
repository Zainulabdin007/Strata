//! Durability, verify/recover, and crash-injection tests.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use strata_codec::{compressor_for, CompressionCodec};
use strata_core::{
    DurabilityMode, Field, FieldValue, IntegerWidth, LogicalType, Record, Schema, TimestampUnit,
};
use strata_format::SegmentId;
use strata_storage::{
    recover_dataset, verify_dataset, DatasetReader, DatasetWriter, DatasetWriterOptions,
    SegmentPath, SegmentWriter,
};

fn schema() -> Schema {
    Schema::new(
        "durability_ticks",
        vec![
            Field::required("seq", LogicalType::unsigned_integer(IntegerWidth::Bits64)),
            Field::required("ts", LogicalType::timestamp(TimestampUnit::Nanoseconds)),
            Field::required("symbol", LogicalType::categorical_string(Some(8))),
        ],
    )
    .unwrap()
}

fn records(n: u64) -> Vec<Record> {
    (0..n)
        .map(|i| {
            Record::new()
                .with("seq", FieldValue::UnsignedInteger(i))
                .with("ts", FieldValue::Timestamp(i as i64 * 1000))
                .with(
                    "symbol",
                    FieldValue::CategoricalString(format!("S{}", i % 8)),
                )
        })
        .collect()
}

fn crash_writer_bin() -> PathBuf {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_strata-crash-writer") {
        return PathBuf::from(path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug/strata-crash-writer")
}

fn ensure_crash_writer() -> PathBuf {
    let bin = crash_writer_bin();
    if bin.exists() {
        return bin;
    }
    let status = Command::new(env!("CARGO"))
        .args([
            "build",
            "-p",
            "strata-integration",
            "--bin",
            "strata-crash-writer",
        ])
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."))
        .status()
        .expect("cargo build crash writer");
    assert!(status.success(), "failed to build strata-crash-writer");
    let bin = crash_writer_bin();
    assert!(bin.exists(), "missing crash writer at {}", bin.display());
    bin
}

#[test]
fn incomplete_tail_does_not_corrupt_prior_blocks() {
    let schema = schema();
    let root = std::env::temp_dir().join(format!("strata-tail-{}-{}", std::process::id(), "prior"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("segments")).unwrap();
    let seg_path = root.join("segments/000000.strata");

    let mut writer = SegmentWriter::create_with_metrics(
        SegmentPath::new(&seg_path),
        schema,
        SegmentId::new("seg-0"),
        100,
        None,
        Default::default(),
        Arc::from(compressor_for(CompressionCodec::None, None)),
        DurabilityMode::Strict,
        1,
    )
    .unwrap();

    writer.write_records(&records(300)[..200]).unwrap();
    let acked = writer.acknowledged_records();
    assert!(acked >= 200);
    drop(writer); // crash before finish / footer

    // Incomplete mid-block tail after valid frames.
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&seg_path)
            .unwrap();
        f.write_all(&[1, 0, 0, 0, 0, 0, 10, 0, 0, 0]).unwrap();
        f.write_all(b"INCOMPLETE").unwrap();
        f.sync_all().unwrap();
    }

    let report = recover_dataset(&root).unwrap();
    assert!(report.record_count >= 200);
    assert!(report.manifest_rebuilt);

    let verify = verify_dataset(&root).unwrap();
    assert!(verify.ok(), "{:?}", verify.issues);

    let decoded = DatasetReader::open(&root)
        .unwrap()
        .read_all_records()
        .unwrap();
    assert!(decoded.len() >= 200);
    assert_eq!(decoded[0].get("seq"), Some(&FieldValue::UnsignedInteger(0)));
    assert_eq!(
        decoded[199].get("seq"),
        Some(&FieldValue::UnsignedInteger(199))
    );
}

#[test]
fn verify_detects_missing_manifest() {
    let schema = schema();
    let root = std::env::temp_dir().join(format!(
        "strata-verify-{}-{}",
        std::process::id(),
        "manifest"
    ));
    let _ = std::fs::remove_dir_all(&root);
    let mut writer = DatasetWriter::create(
        &root,
        schema,
        DatasetWriterOptions {
            block_row_count: 50,
            durability: DurabilityMode::Fast,
            ..Default::default()
        },
    )
    .unwrap();
    writer.write_records(&records(100)).unwrap();
    writer.finish().unwrap();
    std::fs::remove_file(root.join("manifest.json")).unwrap();

    let report = verify_dataset(&root).unwrap();
    assert!(report
        .issues
        .iter()
        .any(|i| i.message.contains("manifest.json missing")));
}

#[test]
fn crash_writer_strict_acked_records_survive_kill() {
    let bin = ensure_crash_writer();
    let out = std::env::temp_dir().join(format!(
        "strata-crash-strict-{}-{}",
        std::process::id(),
        "kill"
    ));
    let _ = std::fs::remove_dir_all(&out);

    let mut child = Command::new(&bin)
        .args([
            "--output",
            out.to_str().unwrap(),
            "--durability",
            "strict",
            "--records",
            "8000",
            "--block-rows",
            "256",
            "--crash-after-ack",
            "1024",
            "--wait-for-kill",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn crash writer");

    let mut ack = 0u64;
    let stdout = child.stdout.take().expect("stdout");
    for line in BufReader::new(stdout).lines() {
        let line = line.unwrap();
        if let Some(rest) = line.strip_prefix("ACK ") {
            ack = rest.trim().parse().unwrap_or(ack);
        }
        if line.starts_with("READY_FOR_KILL") {
            break;
        }
    }
    assert!(ack >= 1024, "expected ack>=1024, got {ack}");

    let _ = child.kill();
    let _ = child.wait();

    let recovered = recover_dataset(&out).unwrap();
    assert!(
        recovered.record_count >= ack,
        "recovered {} < acked {ack}",
        recovered.record_count
    );
    let verify = verify_dataset(&out).unwrap();
    assert!(verify.ok(), "{:?}", verify.issues);

    let decoded = DatasetReader::open(&out)
        .unwrap()
        .read_all_records()
        .unwrap();
    assert!(decoded.len() as u64 >= ack);
    for (i, rec) in decoded.iter().enumerate().take(ack as usize) {
        assert_eq!(rec.get("seq"), Some(&FieldValue::UnsignedInteger(i as u64)));
    }
}

#[test]
fn crash_writer_durable_sync_window() {
    let bin = ensure_crash_writer();
    let out = std::env::temp_dir().join(format!(
        "strata-crash-durable-{}-{}",
        std::process::id(),
        "kill"
    ));
    let _ = std::fs::remove_dir_all(&out);

    let mut child = Command::new(&bin)
        .args([
            "--output",
            out.to_str().unwrap(),
            "--durability",
            "durable",
            "--records",
            "8000",
            "--block-rows",
            "128",
            "--sync-every-blocks",
            "4",
            "--crash-after-ack",
            "1024",
            "--wait-for-kill",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn crash writer");

    let mut ack = 0u64;
    let stdout = child.stdout.take().expect("stdout");
    for line in BufReader::new(stdout).lines() {
        let line = line.unwrap();
        if let Some(rest) = line.strip_prefix("ACK ") {
            ack = rest.trim().parse().unwrap_or(ack);
        }
        if line.starts_with("READY_FOR_KILL") {
            break;
        }
    }
    assert!(ack >= 1024, "expected ack>=1024, got {ack}");

    let _ = child.kill();
    let _ = child.wait();

    let recovered = recover_dataset(&out).unwrap();
    // DURABLE: acked prefix within the sync window must survive.
    assert!(
        recovered.record_count >= ack,
        "recovered {} < acked {ack}",
        recovered.record_count
    );
    let verify = verify_dataset(&out).unwrap();
    assert!(verify.ok(), "{:?}", verify.issues);
}

#[test]
fn crash_writer_fast_recovers_without_corruption() {
    let bin = ensure_crash_writer();
    let out = std::env::temp_dir().join(format!(
        "strata-crash-fast-{}-{}",
        std::process::id(),
        "abort"
    ));
    let _ = std::fs::remove_dir_all(&out);

    let mut child = Command::new(&bin)
        .args([
            "--output",
            out.to_str().unwrap(),
            "--durability",
            "fast",
            "--records",
            "5000",
            "--block-rows",
            "128",
            "--crash-after-ack",
            "512",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let mut saw_crash = false;
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.starts_with("CRASHING") || line.starts_with("ACK ") {
                saw_crash = true;
            }
        }
    }
    let _ = child.wait_timeout_or_kill(Duration::from_secs(10));
    assert!(saw_crash || out.join("segments").exists());

    // FAST may lose recent acks; recovery must not corrupt any salvaged prefix.
    if out.join("segments").exists() {
        match recover_dataset(&out) {
            Ok(_) => {
                let verify = verify_dataset(&out).unwrap();
                assert!(verify.ok(), "{:?}", verify.issues);
            }
            Err(_) => {
                // Empty / unsalvageable active segment is allowed for FAST.
            }
        }
    }
}

trait WaitOrKill {
    fn wait_timeout_or_kill(&mut self, timeout: Duration) -> std::process::ExitStatus;
}

impl WaitOrKill for std::process::Child {
    fn wait_timeout_or_kill(&mut self, timeout: Duration) -> std::process::ExitStatus {
        let start = std::time::Instant::now();
        loop {
            match self.try_wait() {
                Ok(Some(status)) => return status,
                Ok(None) if start.elapsed() < timeout => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                _ => {
                    let _ = self.kill();
                    return self.wait().expect("wait after kill");
                }
            }
        }
    }
}
