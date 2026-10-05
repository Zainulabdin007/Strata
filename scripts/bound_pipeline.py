from pathlib import Path
p=Path('crates/strata-storage/src/pipeline.rs');s=p.read_text()
s=s.replace('    job_tx: Option<Sender<EncodeJob>>,','    job_tx: Option<Sender<EncodeJob>>,\n    credit_tx: Sender<()>,')
s=s.replace('''        // Encode→writer is unbounded so ordered assembly cannot deadlock when
        // high-seq results fill a bounded channel while low-seq is blocked.
        // Memory is still bounded by the parse→encode queue (raw records).
        let (enc_tx, enc_rx) = crossbeam_channel::unbounded::<EncodeResult>();''','''        // A permit covers submission through ordered write, including reorder buffers.
        // The writer always drains results, so a late low-sequence result cannot deadlock.
        let (enc_tx, enc_rx) = bounded::<EncodeResult>(options.encode_queue_capacity);
        let (credit_tx, credit_rx) = bounded::<()>(options.parse_queue_capacity + options.encode_queue_capacity + workers);''')
s=s.replace('''        let writer_handle = thread::spawn(move || {
            writer_loop(''','''        let writer_handle = thread::spawn(move || {
            let failure = Arc::clone(&writer_failed);
            let result = writer_loop(''')
s=s.replace('''                encode_out_len_w,
            )
        });''','''                encode_out_len_w,
                credit_rx,
            );
            if result.is_err() { failure.store(true, Ordering::Release); }
            result
        });''')
s=s.replace('            job_tx: Some(job_tx),','            job_tx: Some(job_tx),\n            credit_tx,')
s=s.replace('''        let seq = self.next_seq;''','''        self.credit_tx.send(()).map_err(|_| Error::InvalidState { message: "writer disconnected".into() })?;
        let seq = self.next_seq;''')
s=s.replace('''            let _ = handle.join();''','''            if handle.join().is_err() { self.failed.store(true, Ordering::Release); }''')
s=s.replace('''        self.flush_busy_time();
        self.writer_handle''','''        self.flush_busy_time();
        if self.failed.load(Ordering::Acquire) {
            if let Some(handle) = self.writer_handle.take() { let _ = handle.join(); }
            return Err(Error::InvalidState { message: "ingest worker or writer failed".into() });
        }
        self.writer_handle''')
# only writer_loop has trailing exact signature line pattern
s=s.replace('''    encode_out_len: Arc<AtomicU64>,
) -> Result<DatasetManifest> {''','''    encode_out_len: Arc<AtomicU64>,
    credit_rx: Receiver<()>,
) -> Result<DatasetManifest> {''')
s=s.replace('''                    writer.write_encoded_block(encoded, msg.row_count)?;
                    next_seq += 1;''','''                    writer.write_encoded_block(encoded, msg.row_count)?;
                    let _ = credit_rx.recv();
                    next_seq += 1;''')
s=s.replace('''    writer.finish()
}''','''    if failed.load(Ordering::Acquire) { return Err(Error::InvalidState { message: "pipeline failed before completion".into() }); }
    if let Some(m) = &metrics { m.set_queue_depths(0, 0, 0); }
    writer.finish()
}''')
s=s.replace('''    /// Reserved encode→writer capacity hint (results channel is unbounded to avoid
    /// ordered-merge deadlock; memory is bounded by `parse_queue_capacity`).''','''    /// Capacity of the encode→writer channel. Global permits bound reordering too.''')
# Join on drop; never detach a writer that could still mutate a failed dataset.
a=s.index('\n#[allow(clippy::too_many_arguments)]')
s=s[:a]+'''
impl Drop for IngestPipeline {
    fn drop(&mut self) {
        if self.writer_handle.is_some() { self.failed.store(true, Ordering::Release); }
        drop(self.job_tx.take());
        for handle in self.worker_handles.drain(..) { let _ = handle.join(); }
        if let Some(handle) = self.writer_handle.take() { let _ = handle.join(); }
    }
}
''' +s[a:]
p.write_text(s)
