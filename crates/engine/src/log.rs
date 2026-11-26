//! The session log: every entry has its time, a source, a level and a message. The Log panel
//! shows `[Source] message` coloured by level, and Help > Export log... writes the time and the
//! level as text.

pub use crate::log_entry::LogEntry;
pub use crate::log_level::LogLevel;
pub use crate::session_log::SessionLog;

/// The source of entries that came without one (plain `push`)
pub const DEFAULT_SOURCE: &str = "TextureCraft";

/// The source of everything about resources, templates, presets and materials: the Log
/// panel's Assets tab shows these entries
pub const LIBRARY_SOURCE: &str = "Library agent";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_keep_source_level_and_message() {
        let mut log = SessionLog::new();
        log.warn("Exporter", "map missing");
        log.error(LIBRARY_SOURCE, "cannot be deleted");
        log.push("plain".into());
        assert_eq!(log.len(), 3);
        assert_eq!(log[0].line(), "[Exporter] map missing");
        assert_eq!(log[0].level, LogLevel::Warning);
        assert_eq!(log[1].level, LogLevel::Error);
        assert!(log[1].is_asset());
        assert_eq!(log[2].level, LogLevel::Info);
        assert!(log[2].starts_with("pl"));
    }

    #[test]
    fn retain_and_clear() {
        let mut log = SessionLog::new();
        log.info("A", "a");
        log.info(LIBRARY_SOURCE, "b");
        log.retain(|entry| !entry.is_asset());
        assert_eq!(log.len(), 1);
        log.clear();
        assert!(log.is_empty());
    }
}
