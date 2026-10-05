from pathlib import Path
# Keep existing public glob names and implementation, fix directory traversal without symlink recursion.
p=Path('crates/strata-core/src/config/profile.rs');s=p.read_text();a=s.index('    /// Relative-path wildcard filters');b=s.index('    /// Optional include globs',a);s=s[:a]+s[b:];s=s.replace('            include: Vec::new(),\n            exclude: Vec::new(),\n','');p.write_text(s)
p=Path('crates/strata-input/src/discover.rs');s=p.read_text();s=s.replace('(profile.input.include.is_empty() || profile.input.include.iter().any(|p| wildcard(p, &relative))) && !profile.input.exclude.iter().any(|p| wildcard(p, &relative))','path_allowed(&relative, &include, &exclude)');a=s.index('/// Whole relative-path wildcard');b=s.index('/// Returns true',a);s=s[:a]+s[b:];p.write_text(s)
p=Path('crates/strata-metrics/src/snapshot.rs');s=p.read_text().replace('    /// Total discovered files for this operation.\n    #[serde(default)]\n    pub total_files: u64,\n','');p.write_text(s)
p=Path('crates/strata-metrics/src/runtime.rs');s=p.read_text().replace('    files_seen: u64,\n    total_files: u64,','    files_seen: u64,').replace('            total_files: 0,\n            total_files: 0,','            total_files: 0,').replace('            total_files: self.total_files,\n            total_files: self.total_files,','            total_files: self.total_files,');i=s.index('    /// Marks an input file as opened');s=s[:i]+'''    /// Replace with the actual size of committed dataset files.
    pub fn set_physical_bytes(&mut self, bytes: u64) { self.physical_bytes_written = bytes; }
    /// Restore committed-file counters, excluding restart downtime.
    pub fn restore(&mut self, s: &MetricsSnapshot) {
        self.started=Instant::now().checked_sub(Duration::from_millis(s.elapsed.get())).unwrap_or_else(Instant::now);self.finished=None;
        self.records_processed=s.records_processed;self.records_read=s.records_read;
        self.logical_input_bytes=s.logical_input_bytes.get();self.semantic_encoded_bytes=s.semantic_encoded_bytes.get();self.compressed_bytes=s.compressed_bytes.get();self.physical_bytes_written=s.physical_bytes_written.get();
        self.blocks_completed=s.blocks_completed;self.segments_completed=s.segments_completed;self.files_completed=s.files_completed;self.files_seen=s.files_seen;self.total_files=s.total_files;self.source_input_bytes=s.source_input_bytes.get();
        self.parse_nanos=s.parse_time.nanos;self.encode_nanos=s.encode_time.nanos;self.compression_nanos=s.compression_time.nanos;self.write_nanos=s.write_time.nanos;
        self.current_input_file=None;self.queue_depth_parse=0;self.queue_depth_encode=0;self.queue_depth_write=0;self.active_workers=0;
    }

''' +s[i:];p.write_text(s)
# Replace the unsafe resume implementation; keep the writer configuration and parser code.
p=Path('crates/strata-engine/src/lib.rs');s=p.read_text();a=s.index('#[derive(Debug, Clone, Serialize, Deserialize, Default)]\nstruct IngestProgress');b=s.index('/// Storage inventory',a)
old=s[a:b];opts=old[old.index('    let schema = profile.schema()?;'):old.index('    let mut file_errors:')];opts=opts.replace('IngestPipeline::start(output,','IngestPipeline::start(output,')
fn='''fn ingest_file(input:&Path, output:&Path, profile:&CompressionProfile, options:&IngestOptions, metrics:MetricsHandle)->Result<strata_storage::DatasetManifest> {
'''+opts+'''    metrics.begin_input_file(input.display().to_string(), std::fs::metadata(input)?.len());
    let started=Instant::now();
    let parsed=parser.for_each_batch(input,&schema,profile.block.row_count as usize,&mut |batch|{
        metrics.add_records_read(batch.len() as u64);
        pipeline.submit(batch.records).map_err(|e|strata_input::Error::MalformedCsv{path:input.to_path_buf(),line:0,message:e.to_string()})
    });
    metrics.add_parse_time(started.elapsed());
    if let Err(e)=parsed { drop(pipeline); return Err(e.into()); }
    let manifest=pipeline.finish()?;
    metrics.finish_input_file();
    Ok(manifest)
}

mod folder;
pub use folder::ingest;

'''
s=s[:a]+fn+s[b:];s=s.replace('use strata_input::{discover_inputs_with, DiscoverOptions};\n','');s=s.replace('    /// Skip input files listed in `{output}/.ingest-progress.json` when restarting\n    /// into an empty output directory (safe resume after deleting a partial dataset).','    /// Resume only fully committed files, verifying their source fingerprints.');s=s.replace('    pub exclude_globs: Vec<String>,','    pub exclude_globs: Vec<String>,\n    /// Optional extension override.\n    pub extensions: Vec<String>,');s=s.replace('            exclude_globs: Vec::new(),','            exclude_globs: Vec::new(),\n            extensions: Vec::new(),');p.write_text(s)
p=Path('crates/strata-cli/src/main.rs');s=p.read_text().replace('    /// Resume after a failed directory ingest: clear partial output, skip files\n    /// already recorded in `.ingest-progress.json`.','    /// Resume verified committed files; interrupted files are retried without duplicate rows.');s=s.replace('    exclude_globs: Vec<String>,','    exclude_globs: Vec<String>,\n    /// Override extensions (comma separated).\n    #[arg(long, value_delimiter=\',\')]\n    extensions: Vec<String>,',1);s=s.replace('include_globs: args.include_globs, exclude_globs: args.exclude_globs,','include_globs: args.include_globs, exclude_globs: args.exclude_globs, extensions:args.extensions,');p.write_text(s)
