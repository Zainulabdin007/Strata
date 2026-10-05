//! Bound a single source record before the CSV/JSON parser can grow its buffers.
use std::io::{self, Read};
/// Reader limiting a CSV record (including quoted newlines) or JSON line to 1 MiB.
pub struct RecordGuard<R> {
    inner: R,
    csv: bool,
    quoted: bool,
    bytes: usize,
}
impl<R: Read> RecordGuard<R> {
    /// Wrap a source. CSV uses double-quote framing; JSONL ends at every newline.
    pub fn new(inner: R, csv: bool) -> Self {
        Self {
            inner,
            csv,
            quoted: false,
            bytes: 0,
        }
    }
}
impl<R: Read> Read for RecordGuard<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(out)?;
        for byte in &out[..count] {
            self.bytes += 1;
            if self.bytes > 1024 * 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "input record exceeds 1 MiB safety limit",
                ));
            }
            if self.csv && *byte == b'"' {
                self.quoted = !self.quoted;
            }
            if *byte == b'\n' && (!self.csv || !self.quoted) {
                self.bytes = 0;
            }
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_records_before_parser_allocation() {
        let data = vec![b'x'; 1024 * 1024 + 1];
        assert!(RecordGuard::new(&data[..], false)
            .read_to_end(&mut Vec::new())
            .is_err());
        let csv = b"id,text\n1,\"a\n\"\"b\"\n2,c\n";
        let mut got = Vec::new();
        RecordGuard::new(&csv[..], true)
            .read_to_end(&mut got)
            .unwrap();
        assert_eq!(got, csv);
    }
}
