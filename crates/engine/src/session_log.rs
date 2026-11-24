use std::ops::Deref;

use crate::log::DEFAULT_SOURCE;
use crate::{LogEntry, LogLevel};

/// The current session's log in occurrence order
#[derive(Clone, Debug, Default)]
pub struct SessionLog {
    entries: Vec<LogEntry>,
    // IDs stay in occurrence order and are never reused after a clear or retain.
    occurrences: Vec<u64>,
    last_occurrence: u64,
}

impl PartialEq for SessionLog {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

impl Eq for SessionLog {}

#[cfg(not(target_arch = "wasm32"))]
static PERFORMANCE_LOG: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn set_performance_log_path(path: std::path::PathBuf) {
    let _ = PERFORMANCE_LOG.set(path);
}

impl SessionLog {
    pub fn new() -> SessionLog {
        Self::default()
    }

    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    /// Entries added after a previous occurrence in this log, in append order.
    /// Pass zero to start at the beginning.
    pub fn entries_after(&self, occurrence: u64) -> impl Iterator<Item = (u64, &LogEntry)> {
        let start = self.occurrences.partition_point(|&id| id <= occurrence);
        self.occurrences[start..].iter().copied().zip(&self.entries[start..])
    }

    fn record(&mut self, entry: LogEntry) {
        self.last_occurrence += 1;
        self.occurrences.push(self.last_occurrence);
        self.entries.push(entry);
    }

    /// An info entry without a source of its own
    pub fn push(&mut self, message: String) {
        self.record(LogEntry::new(LogLevel::Info, DEFAULT_SOURCE, message));
    }

    pub fn info(&mut self, source: &str, message: impl Into<String>) {
        self.record(LogEntry::new(LogLevel::Info, source, message));
    }

    /// Keep timings in the session log and, on desktop, in the local diagnostic log.
    pub fn performance(&mut self, message: impl Into<String>) {
        let entry = LogEntry::new(LogLevel::Info, "Performance", message);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = PERFORMANCE_LOG.get() {
            use std::io::Write as _;
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(file, "[{}] {}", entry.time.to_rfc3339(), entry.message);
            }
        }
        self.record(entry);
    }

    pub fn warn(&mut self, source: &str, message: impl Into<String>) {
        self.record(LogEntry::new(LogLevel::Warning, source, message));
    }

    pub fn error(&mut self, source: &str, message: impl Into<String>) {
        self.record(LogEntry::new(LogLevel::Error, source, message));
    }

    /// Several warnings of one source
    pub fn warnings(&mut self, source: &str, messages: impl IntoIterator<Item = String>) {
        for message in messages {
            self.warn(source, message);
        }
    }

    /// Append the entries of another store
    pub fn append(&mut self, other: SessionLog) {
        for entry in other.entries {
            self.record(entry);
        }
    }

    /// Put the entries of `earlier` before this log's; every entry gets a new occurrence after
    /// the ones this log handed out, so none is reused
    pub(crate) fn prepend(&mut self, earlier: SessionLog) {
        let mut merged = SessionLog { last_occurrence: self.last_occurrence, ..SessionLog::default() };
        for entry in earlier.entries.into_iter().chain(std::mem::take(&mut self.entries)) {
            merged.record(entry);
        }
        *self = merged;
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.occurrences.clear();
    }

    /// Drop the entries for which `keep` is false
    pub fn retain(&mut self, keep: impl FnMut(&LogEntry) -> bool) {
        // Evaluate the predicate once and apply the same decisions to both vectors.
        let kept: Vec<_> = self.entries.iter().map(keep).collect();
        let mut decisions = kept.iter().copied();
        self.entries.retain(|_| decisions.next().unwrap_or_default());
        let mut decisions = kept.into_iter();
        self.occurrences.retain(|_| decisions.next().unwrap_or_default());
    }
}

impl Deref for SessionLog {
    type Target = [LogEntry];
    fn deref(&self) -> &[LogEntry] {
        &self.entries
    }
}

impl<'a> IntoIterator for &'a SessionLog {
    type Item = &'a LogEntry;
    type IntoIter = std::slice::Iter<'a, LogEntry>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::SessionLog;

    /// Entries put before a log keep their order and get occurrences after the ones the log
    /// handed out, so a reader past them still reads every later entry
    #[test]
    fn prepended_entries_come_first_with_new_occurrences() {
        let mut log = SessionLog::new();
        log.push("opened".into());
        let mut earlier = SessionLog::new();
        earlier.performance("decoded");

        log.prepend(earlier);
        log.push("later".into());

        let messages: Vec<&str> = log.entries().iter().map(|entry| entry.message.as_str()).collect();
        assert_eq!(messages, ["decoded", "opened", "later"]);
        let occurrences: Vec<u64> = log.entries_after(0).map(|(occurrence, _)| occurrence).collect();
        assert_eq!(occurrences, [2, 3, 4]);
    }
}
