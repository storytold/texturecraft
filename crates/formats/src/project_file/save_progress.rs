use std::sync::atomic::{AtomicU64, Ordering};

use super::entries::Entry;

mod progress_writer;

use progress_writer::ProgressWriter;

// Leave room on the indicator for syncing and replacing the archive
const MAX_PAYLOAD_PROGRESS: f32 = 0.95;

/// Estimated archive progress, based on the payload processed by the entry writers.
/// Completing the archive, syncing it to disk, and replacing the destination can take longer.
#[derive(Default)]
pub struct SaveProgress {
    total: AtomicU64,
    processed: AtomicU64,
}

impl SaveProgress {
    pub fn fraction(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }

        let processed = self.processed.load(Ordering::Relaxed).min(total);
        MAX_PAYLOAD_PROGRESS * (processed as f64 / total as f64) as f32
    }

    pub(super) fn track<'a>(&'a self, entries: &mut [Entry<'a>]) {
        self.processed.store(0, Ordering::Relaxed);
        self.total.store(entries.iter().fold(0u64, |sum, entry| sum.saturating_add(entry.weight)), Ordering::Relaxed);
        for entry in entries {
            let body = std::mem::replace(&mut entry.body, Box::new(|_| Ok(())));
            let remaining = entry.weight;
            entry.body = Box::new(move |target| {
                let mut writer = ProgressWriter { target, progress: self, remaining, pending: 0 };
                body(&mut writer)?;
                writer.report();
                Ok(())
            });
        }
    }
}
