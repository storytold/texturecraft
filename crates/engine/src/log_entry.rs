use std::ops::Deref;

use chrono::{DateTime, Utc};

use crate::LogLevel;
use crate::log::LIBRARY_SOURCE;

/// A session message with its occurrence time, source and severity
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    pub time: DateTime<Utc>,
    pub source: String,
    pub level: LogLevel,
    pub message: String,
}

impl LogEntry {
    pub fn new(level: LogLevel, source: &str, message: impl Into<String>) -> LogEntry {
        LogEntry { time: Utc::now(), source: source.to_string(), level, message: message.into() }
    }

    /// The panel's line: `[Source] message`, no time stamp and no level tag
    pub fn line(&self) -> String {
        format!("[{}] {}", self.source, self.message)
    }

    /// True for entries of the Log panel's Assets tab
    pub fn is_asset(&self) -> bool {
        self.source == LIBRARY_SOURCE
    }
}

/// The message text, so `entry.contains(..)` and `entry.starts_with(..)` read the message
impl Deref for LogEntry {
    type Target = str;
    fn deref(&self) -> &str {
        &self.message
    }
}

impl PartialEq<String> for LogEntry {
    fn eq(&self, other: &String) -> bool {
        self.message == *other
    }
}

impl PartialEq<str> for LogEntry {
    fn eq(&self, other: &str) -> bool {
        self.message == other
    }
}
