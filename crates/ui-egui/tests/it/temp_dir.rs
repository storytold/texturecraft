use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A test folder removed when its owner drops
pub(super) struct TempDir(PathBuf);

impl TempDir {
    pub(super) fn new(test: &str) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);

        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("texturecraft-{test}-{}-{id}", std::process::id()));
        std::fs::create_dir(&path).unwrap();

        Self(path)
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}
