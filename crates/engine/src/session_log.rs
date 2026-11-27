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
