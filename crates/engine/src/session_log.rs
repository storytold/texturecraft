use std::ops::Deref;

use crate::log::DEFAULT_SOURCE;
use crate::{LogEntry, LogLevel};

/// The current session's log in occurrence order
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionLog(Vec<LogEntry>);

impl SessionLog {
    pub fn new() -> SessionLog {
        SessionLog(Vec::new())
    }

    pub fn entries(&self) -> &[LogEntry] {
        &self.0
    }

    /// An info entry without a source of its own
    pub fn push(&mut self, message: String) {
        self.0.push(LogEntry::new(LogLevel::Info, DEFAULT_SOURCE, message));
    }

    pub fn info(&mut self, source: &str, message: impl Into<String>) {
        self.0.push(LogEntry::new(LogLevel::Info, source, message));
    }

    pub fn warn(&mut self, source: &str, message: impl Into<String>) {
        self.0.push(LogEntry::new(LogLevel::Warning, source, message));
    }

    pub fn error(&mut self, source: &str, message: impl Into<String>) {
        self.0.push(LogEntry::new(LogLevel::Error, source, message));
    }

    /// Several warnings of one source
    pub fn warnings(&mut self, source: &str, messages: impl IntoIterator<Item = String>) {
        for message in messages {
            self.warn(source, message);
        }
    }

    /// Append the entries of another store
    pub fn append(&mut self, other: SessionLog) {
        self.0.extend(other.0);
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Drop the entries for which `keep` is false
    pub fn retain(&mut self, keep: impl FnMut(&LogEntry) -> bool) {
        self.0.retain(keep);
    }
}

impl Deref for SessionLog {
    type Target = [LogEntry];
    fn deref(&self) -> &[LogEntry] {
        &self.0
    }
}

impl<'a> IntoIterator for &'a SessionLog {
    type Item = &'a LogEntry;
    type IntoIter = std::slice::Iter<'a, LogEntry>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
