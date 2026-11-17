use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) struct TestFolder(PathBuf);

impl TestFolder {
    pub(super) fn new(tag: &str) -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!("tc_pbr_{tag}_{}_{id}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        Self(directory)
    }
}

impl std::ops::Deref for TestFolder {
    type Target = PathBuf;

    fn deref(&self) -> &PathBuf {
        &self.0
    }
}

impl Drop for TestFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
