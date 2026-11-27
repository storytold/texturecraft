use std::path::PathBuf;

/// A library folder removed when its owner drops
pub(super) struct Library(pub(super) PathBuf);

impl Library {
    pub(super) fn new(test: &str) -> Self {
        let path = std::env::temp_dir().join(format!("texturecraft-template-guards-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub(super) fn files(&self) -> usize {
        std::fs::read_dir(&self.0).unwrap().flatten().filter(|entry| entry.path().extension().is_some_and(|extension| extension == "json")).count()
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}
