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

pub(crate) fn write_performance_log(entry: &LogEntry) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(path) = PERFORMANCE_LOG.get() {
        use std::io::Write as _;
        static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

        let _guard = WRITE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "[{}] {}", entry.time.to_rfc3339(), entry.message);
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = entry;
}

/// Keep one JSON Lines record per imported graph cook beside the desktop performance log.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_graph_cpu_report(report: &texturecraft_compose::graphs::GraphCpuReport) -> std::io::Result<()> {
    use std::io::Write as _;
    static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    let Some(log) = PERFORMANCE_LOG.get() else { return Ok(()) };
    let path = if log.file_name().is_some_and(|name| name == "performance.log") {
        log.with_file_name("spp-graph-cpu.jsonl")
    } else {
        log.with_extension("spp-graph-cpu.jsonl")
    };
    #[derive(serde::Serialize)]
    struct Entry<'a> {
        version: u8,
        time: String,
        report: &'a texturecraft_compose::graphs::GraphCpuReport,
    }
    let mut line = serde_json::to_vec(&Entry { version: 1, time: chrono::Utc::now().to_rfc3339(), report })?;
    line.push(b'\n');
    let _guard = WRITE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path)?.write_all(&line)
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
        write_performance_log(&entry);
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
