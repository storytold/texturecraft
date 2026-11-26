use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use web_time::Instant;

use crate::{LogEntry, LogLevel};

const HISTORY_LINES: usize = 64;
const LINE_CHARS: usize = 512;
const NODE_LOG_INTERVAL: Duration = Duration::from_secs(1);

pub(super) struct LiveLog {
    file: String,
    started: Instant,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    lines: VecDeque<String>,
    /// The most recent node remains visible between persistent log entries.
    pending_node: Option<String>,
    last_node: Option<Instant>,
}

impl LiveLog {
    pub(super) fn new(file: &str) -> Self {
        let file = file.rsplit(['/', '\\']).next().unwrap_or_default();
        Self { file: sanitize(file), started: Instant::now(), state: Mutex::default() }
    }

    pub(super) fn record(&self, message: &str) {
        if let Some(line) = self.push(message) {
            let entry = LogEntry::new(LogLevel::Info, "Performance", format!("Import {}  {line}", self.file));
            crate::session_log::write_performance_log(&entry);
        }
    }

    pub(super) fn push(&self, message: &str) -> Option<String> {
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        let line = format!("{:>7.2}s  {}", now.duration_since(self.started).as_secs_f64(), sanitize(message));
        if message.starts_with("Node #") {
            if state.last_node.is_some_and(|last| now.duration_since(last) < NODE_LOG_INTERVAL) {
                state.pending_node = Some(line);
                return None;
            }
            state.last_node = Some(now);
        }
        state.pending_node = None;
        if state.lines.len() == HISTORY_LINES {
            state.lines.pop_front();
        }
        state.lines.push_back(line.clone());
        Some(line)
    }

    pub(super) fn lines(&self) -> Vec<String> {
        let state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let skip = usize::from(state.pending_node.is_some() && state.lines.len() == HISTORY_LINES);
        state.lines.iter().skip(skip).chain(&state.pending_node).cloned().collect()
    }

    pub(super) fn file(&self) -> &str {
        &self.file
    }
}

fn sanitize(message: &str) -> String {
    message.chars().take(LINE_CHARS).map(|ch| if ch.is_control() { ' ' } else { ch }).collect()
}
