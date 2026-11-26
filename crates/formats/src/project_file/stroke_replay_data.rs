//! Opaque stroke replay payload. The engine owns its schema.

use std::{borrow::Cow, fmt, sync::Arc};

use crate::Result;

type Encode = dyn Fn() -> Result<Vec<u8>> + Send + Sync;

/// Stored stroke operations, or an immutable snapshot encoded by the save worker.
#[derive(Clone)]
pub struct StrokeReplayData(Source);

#[derive(Clone)]
enum Source {
    Loaded(Arc<Vec<u8>>),
    Snapshot(Arc<Encode>),
}

impl StrokeReplayData {
    pub(super) fn loaded(bytes: Vec<u8>) -> Self {
        Self(Source::Loaded(Arc::new(bytes)))
    }

    pub fn snapshot(encode: impl Fn() -> Result<Vec<u8>> + Send + Sync + 'static) -> Self {
        Self(Source::Snapshot(Arc::new(encode)))
    }

    pub fn bytes(&self) -> Result<Cow<'_, [u8]>> {
        match &self.0 {
            Source::Loaded(bytes) => Ok(Cow::Borrowed(bytes.as_slice())),
            Source::Snapshot(encode) => encode().map(Cow::Owned),
        }
    }
}

impl fmt::Debug for StrokeReplayData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Source::Loaded(bytes) => formatter.debug_struct("StrokeReplayData").field("bytes", &bytes.len()).finish(),
            Source::Snapshot(_) => formatter.debug_struct("StrokeReplayData").finish_non_exhaustive(),
        }
    }
}
