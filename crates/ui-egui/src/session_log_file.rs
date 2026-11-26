//! The current desktop run's rolling log, separate from the clearable Log panel.
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};
use texturecraft_engine::SessionLog;

// Retain two segments to bound each run's log while preserving recent history
const MAX_LOG_SEGMENT_BYTES: u64 = 8 * 1024 * 1024;
/// Runs whose log folders stay in the logs folder, the new run's included, so the folder stays bounded
const KEPT_SESSION_FOLDERS: usize = 10;
const SESSION_FOLDER_PREFIX: &str = "session-";

pub(crate) struct SessionLogFile {
    pub path: PathBuf,
    file: File,
    bytes: u64,
    log_id: u64,
    cursor: u64,
}

/// Remove all but the `keep` newest session folders in `logs`. Their names start with a UTC time, so
/// name order is age order. Errors are ignored: a folder an older run still holds open stays for a later start.
fn prune(logs: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(logs) else { return };
    let mut folders: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| entry.file_name().to_str().is_some_and(|name| name.starts_with(SESSION_FOLDER_PREFIX)))
        .map(|entry| entry.path())
        .collect();
    folders.sort();
    let old = folders.len().saturating_sub(keep);
    for folder in &folders[..old] {
        let _ = std::fs::remove_dir_all(folder);
    }
}

impl SessionLogFile {
    /// Start this run's log in a new session folder under `logs`, after removing the oldest runs' folders.
    pub fn start(logs: &Path) -> io::Result<Self> {
        prune(logs, KEPT_SESSION_FOLDERS - 1);
        let folder = format!("{SESSION_FOLDER_PREFIX}{}-{}", chrono::Utc::now().format("%Y%m%dT%H%M%S%.6fZ"), std::process::id());
        Self::new(logs.join(folder).join("log.txt"))
    }

    pub fn new(path: PathBuf) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new().create_new(true).append(true).open(&path)?;
        Ok(Self { path, file, bytes: 0, log_id: 0, cursor: 0 })
    }

    pub fn text(&mut self) -> io::Result<String> {
        self.file.flush()?;
        let previous = self.path.with_file_name("log.previous.txt");
        let mut text = match std::fs::read_to_string(previous) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error),
        };
        text.push_str(&std::fs::read_to_string(&self.path)?);
        Ok(text)
    }

    pub fn sync(&mut self, log_id: u64, log: &SessionLog) -> io::Result<()> {
        if self.log_id != log_id {
            self.log_id = log_id;
            self.cursor = 0;
        }
        for (occurrence, entry) in log.entries_after(self.cursor) {
            let text = crate::windows::log_text(std::slice::from_ref(entry));
            let line = text.split_once('\n').map(|(_, line)| line).unwrap_or_default();

            // Keep one previous segment when the active log rolls
            if self.bytes.saturating_add(line.len() as u64) > MAX_LOG_SEGMENT_BYTES {
                self.file.flush()?;
                std::fs::copy(&self.path, self.path.with_file_name("log.previous.txt"))?;
                self.file.set_len(0)?;
                self.bytes = 0;
            }
            self.file.write_all(line.as_bytes())?;
            self.bytes = self.bytes.saturating_add(line.len() as u64);
            self.cursor = occurrence;
        }
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log() -> SessionLogFile {
        let dir = std::env::temp_dir().join(format!("texturecraft-session-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.txt", chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        SessionLogFile::new(path).unwrap()
    }

    fn remove_log(file: SessionLogFile) {
        let path = file.path.clone();
        drop(file);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn starting_a_run_removes_only_the_oldest_session_folders() {
        let logs = std::env::temp_dir().join(format!("texturecraft-session-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&logs);
        let older: Vec<PathBuf> = (0..KEPT_SESSION_FOLDERS + 2).map(|i| logs.join(format!("{SESSION_FOLDER_PREFIX}20260101T0000{i:02}.000000Z-1"))).collect();
        for folder in &older {
            std::fs::create_dir_all(folder).unwrap();
        }
        let other = logs.join("kept-by-the-user");
        std::fs::create_dir_all(&other).unwrap();

        let file = SessionLogFile::start(&logs).unwrap();

        let survivors = older.iter().filter(|folder| folder.exists()).count();
        assert_eq!(survivors + 1, KEPT_SESSION_FOLDERS, "the new run and the newest earlier runs remain");
        assert!(!older[0].exists() && older.last().unwrap().exists(), "the oldest go first");
        assert!(other.exists() && file.path.exists(), "other folders and the new log stay");
        drop(file);
        std::fs::remove_dir_all(logs).unwrap();
    }

    #[test]
    fn log_file_keeps_messages_after_panel_clear_and_across_projects() {
        let mut file = temp_log();
        let mut log = SessionLog::new();
        log.push("first".into());
        file.sync(1, &log).unwrap();
        file.sync(1, &log).unwrap();
        log.clear();
        file.sync(1, &log).unwrap();
        log.error("UI", "second");
        file.sync(1, &log).unwrap();
        let mut next_project = SessionLog::new();
        next_project.info("UI", "new project");
        file.sync(2, &next_project).unwrap();
        let text = file.text().unwrap();
        assert_eq!(text.matches("first").count(), 1);
        assert!(text.contains("second") && text.contains("new project"));
        remove_log(file);
    }

    #[test]
    fn removing_the_tail_does_not_hide_an_earlier_timestamp_append() {
        let mut file = temp_log();
        let mut pending = SessionLog::new();
        pending.push("deferred message".into());

        let mut log = SessionLog::new();
        log.push("kept message".into());
        log.push("removed tail".into());
        file.sync(1, &log).unwrap();

        assert!(pending[0].time <= log[1].time);
        log.retain(|entry| entry.message != "removed tail");
        log.append(pending);
        file.sync(1, &log).unwrap();

        let text = file.text().unwrap();
        for message in ["deferred message", "kept message", "removed tail"] {
            assert_eq!(text.matches(message).count(), 1, "{message}");
        }
        remove_log(file);
    }

    #[test]
    fn removing_an_earlier_timestamp_tail_does_not_replay_the_survivors() {
        let mut file = temp_log();
        let mut pending = SessionLog::new();
        pending.push("removed tail".into());

        let mut log = SessionLog::new();
        log.push("kept message".into());
        log.append(pending);
        assert!(log[0].time >= log[1].time);
        file.sync(1, &log).unwrap();

        log.retain(|entry| entry.message != "removed tail");
        file.sync(1, &log).unwrap();
        file.sync(1, &log).unwrap();

        let text = file.text().unwrap();
        assert_eq!(text.matches("kept message").count(), 1);
        assert_eq!(text.matches("removed tail").count(), 1);
        remove_log(file);
    }
}
