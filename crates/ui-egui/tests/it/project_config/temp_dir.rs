use std::path::{Path, PathBuf};

/// A test folder removed when its owner drops
pub(super) struct TempDir(PathBuf);

impl TempDir {
    pub(super) fn new(test: &str) -> Self {
        let path = std::env::temp_dir().join(format!("texturecraft-project-config-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();

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
