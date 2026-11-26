use std::time::Duration;

pub(crate) struct Row {
    pub(crate) stage: String,
    pub(crate) took: Duration,
    pub(crate) budget: Option<Duration>,
    pub(crate) note: String,
}
