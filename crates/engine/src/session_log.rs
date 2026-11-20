use std::ops::Deref;

use crate::log::DEFAULT_SOURCE;
use crate::{LogEntry, LogLevel};

/// The current session's log in occurrence order
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionLog(Vec<LogEntry>);

#[cfg(not(target_arch = "wasm32"))]
static PERFORMANCE_LOG: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn set_performance_log_path(path: std::path::PathBuf) {
    let _ = PERFORMANCE_LOG.set(path);
}

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
        self.0.push(entry);
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

    pub(crate) fn prepend(&mut self, mut earlier: SessionLog) {
        earlier.0.append(&mut self.0);
        self.0 = earlier.0;
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
