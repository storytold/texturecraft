use std::io::{self, Write};
use std::sync::atomic::Ordering;

use super::SaveProgress;

// Batch updates to reduce contention between compression workers
const PROGRESS_UPDATE_BYTES: u64 = 64 * 1024;

pub(super) struct ProgressWriter<'a> {
    pub(super) target: &'a mut dyn Write,
    pub(super) progress: &'a SaveProgress,
    pub(super) remaining: u64,
    pub(super) pending: u64,
}

impl ProgressWriter<'_> {
    pub(super) fn report(&mut self) {
        self.progress.processed.fetch_add(std::mem::take(&mut self.pending), Ordering::Relaxed);
    }
}

impl Write for ProgressWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.target.write(bytes)?;
        let counted = (written as u64).min(self.remaining);
        self.remaining -= counted;
        self.pending += counted;
        if self.pending >= PROGRESS_UPDATE_BYTES {
            self.report();
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.target.flush()
    }
}
