from pathlib import Path

def edit(p,a,b):
 p=Path(p);s=p.read_text();assert a in s,(p,a[:60]);p.write_text(s.replace(a,b))
edit('Cargo.toml','rust-version = "1.75"','rust-version = "1.89"')
edit('crates/strata-core/src/config/profile.rs','    pub extensions: Vec<String>,','    pub extensions: Vec<String>,\n    /// Relative-path wildcard filters (* matches any path characters, ? one character).\n    #[serde(default)]\n    pub include: Vec<String>,\n    /// Relative-path wildcard exclusions, applied after include.\n    #[serde(default)]\n    pub exclude: Vec<String>,')
edit('crates/strata-core/src/config/profile.rs','            extensions: Vec::new(),','            extensions: Vec::new(),\n            include: Vec::new(),\n            exclude: Vec::new(),')
p=Path('crates/strata-input/src/discover.rs');s=p.read_text();a=s.index('    let mut files = Vec::new();');b=s.index('\n    files.sort();',a);s=s[:a]+'''    let mut files = Vec::new();
    let mut dirs = vec![path.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|source| Error::Io { path: dir.clone(), source })? {
            let entry = entry.map_err(|source| Error::Io { path: dir.clone(), source })?;
            let ty = entry.file_type().map_err(|source| Error::Io { path: entry.path(), source })?;
            // Never follow directory symlinks (cycles and escapes).
            if ty.is_dir() && profile.input.recursive { dirs.push(entry.path()); }
            else if ty.is_file() && extension_matches(&entry.path(), &extensions) { files.push(entry.path()); }
        }
    }
    files.retain(|file| {
        let relative = file.strip_prefix(path).unwrap_or(file).to_string_lossy().replace('\\\\', "/");
        (profile.input.include.is_empty() || profile.input.include.iter().any(|p| wildcard(p, &relative))) && !profile.input.exclude.iter().any(|p| wildcard(p, &relative))
    });
''' +s[b:];a=s.index('fn visit_dir_shallow');b=s.index('/// Returns true',a);s=s[:a]+'''/// Whole relative-path wildcard matching. `*` crosses separators; `?` matches one character.
/// Iterative backtracking uses constant auxiliary space, with no recursive glob expansion.
pub fn wildcard(pattern: &str, path: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect(); let s: Vec<char> = path.chars().collect();
    let (mut i, mut j, mut star, mut retry) = (0, 0, None, 0);
    while j < s.len() {
        if i < p.len() && (p[i] == '?' || p[i] == s[j]) { i += 1; j += 1; }
        else if i < p.len() && p[i] == '*' { star = Some(i); i += 1; retry = j; }
        else if let Some(k) = star { retry += 1; j = retry; i = k + 1; }
        else { return false; }
    }
    while i < p.len() && p[i] == '*' { i += 1; }
    i == p.len()
}

''' +s[b:];p.write_text(s)
# Exact CSV schema: all declared columns, no duplicate or unexpected columns.
p=Path('crates/strata-input/src/parse.rs');s=p.read_text().replace('        for field in &schema.fields {\n            if !headers', '''        if headers.len() != schema.len() || headers.iter().collect::<std::collections::HashSet<_>>().len() != headers.len() {
            return Err(Error::MalformedCsv { path: path.to_path_buf(), line: 1, message: "CSV columns must exactly match schema (no extra or duplicate headers)".into() });
        }
        for field in &schema.fields {
            if !headers''');s=s.replace('            let mut record = Record::new();\n            for field in &schema.fields {\n                let raw = obj', '''            if obj.keys().any(|name| schema.field(name).is_none()) {
                return Err(Error::MalformedJsonl { path: path.to_path_buf(), line: line_no, message: "unexpected column outside declared schema".into() });
            }
            let mut record = Record::new();
            for field in &schema.fields {
                let raw = obj''');p.write_text(s)
# Canonical total file count and resumable counter restoration.
edit('crates/strata-metrics/src/runtime.rs','    files_seen: u64,','    files_seen: u64,\n    total_files: u64,')
edit('crates/strata-metrics/src/runtime.rs','            files_seen: 0,','            files_seen: 0,\n            total_files: 0,')
edit('crates/strata-metrics/src/runtime.rs','            files_seen: self.files_seen,','            files_seen: self.files_seen,\n            total_files: self.total_files,')
edit('crates/strata-metrics/src/snapshot.rs','    pub files_seen: u64,','    pub files_seen: u64,\n    /// Total discovered files for this operation.\n    #[serde(default)]\n    pub total_files: u64,')
edit('crates/strata-metrics/src/snapshot.rs','            seen = self.files_seen,','            seen = self.total_files.max(self.files_seen),')
p=Path('crates/strata-metrics/src/runtime.rs');s=p.read_text();needle='    /// Marks an input file as opened';i=s.index(needle);s=s[:i]+'''    /// Record the discovered input count.
    pub fn set_total_files(&mut self, count: u64) { self.total_files = count; }
    /// Replace the physical size with a measured committed storage size.
    pub fn set_physical_bytes(&mut self, bytes: u64) { self.physical_bytes_written = bytes; }
    /// Restore a completed-file checkpoint, excluding downtime from wall time.
    pub fn restore(&mut self, s: &MetricsSnapshot) {
        self.started = Instant::now().checked_sub(Duration::from_millis(s.elapsed.get())).unwrap_or_else(Instant::now);
        self.finished = None;
        self.records_processed=s.records_processed; self.records_read=s.records_read;
        self.logical_input_bytes=s.logical_input_bytes.get();self.semantic_encoded_bytes=s.semantic_encoded_bytes.get();
        self.compressed_bytes=s.compressed_bytes.get();self.physical_bytes_written=s.physical_bytes_written.get();
        self.blocks_completed=s.blocks_completed;self.segments_completed=s.segments_completed;
        self.files_completed=s.files_completed;self.files_seen=s.files_seen;self.total_files=s.total_files;
        self.source_input_bytes=s.source_input_bytes.get();self.parse_nanos=s.parse_time.nanos;self.encode_nanos=s.encode_time.nanos;
        self.compression_nanos=s.compression_time.nanos;self.write_nanos=s.write_time.nanos;
        self.current_input_file=None;self.queue_depth_parse=0;self.queue_depth_encode=0;self.queue_depth_write=0;self.active_workers=0;
    }

''' +s[i:];p.write_text(s)
edit('crates/strata-engine/src/lib.rs','    pub sync_every_blocks: Option<u32>,','    pub sync_every_blocks: Option<u32>,\n    pub resume: bool,\n    pub include: Vec<String>,\n    pub exclude: Vec<String>,\n    pub extensions: Vec<String>,')
edit('crates/strata-engine/src/lib.rs','sync_every_blocks: None }','sync_every_blocks: None, resume: false, include: Vec::new(), exclude: Vec::new(), extensions: Vec::new() }')
edit('crates/strata-engine/src/lib.rs','pub fn ingest(input:', 'fn ingest_file(input:')
edit('crates/strata-engine/src/lib.rs','metrics: MetricsHandle) -> Result<MetricsSnapshot> {','metrics: MetricsHandle) -> Result<()> {')
edit('crates/strata-engine/src/lib.rs','    Ok(metrics.finish())','    Ok(())')
edit('crates/strata-engine/src/lib.rs','//! Shared local orchestration.','//! Shared local orchestration.')
p=Path('crates/strata-engine/src/lib.rs');p.write_text(p.read_text()+'\nmod folder;\npub use folder::ingest;\n')
# CLI options map directly to shared engine.
p=Path('crates/strata-cli/src/main.rs');s=p.read_text();needle='    /// Input file or directory.\n    input: PathBuf,';s=s.replace(needle,needle+'''\n    /// Resume from fully committed files; changed completed inputs are rejected.
    #[arg(long)]
    resume: bool,
    /// Include relative-path wildcard (* crosses directories, ? matches one character).
    #[arg(long)]
    include: Vec<String>,
    /// Exclude relative-path wildcard.
    #[arg(long)]
    exclude: Vec<String>,
    /// Extension filter, repeated or comma-separated.
    #[arg(long, value_delimiter=',')]
    extensions: Vec<String>,''',1);s=s.replace('durability: args.durability, sync_every_blocks: args.sync_every_blocks,','durability: args.durability, sync_every_blocks: args.sync_every_blocks,\n        resume:args.resume, include:args.include, exclude:args.exclude, extensions:args.extensions,');p.write_text(s)
