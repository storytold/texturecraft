/// A user template library folder removed with its owner
pub(super) struct TemplateLibrary(pub(super) std::path::PathBuf);

impl TemplateLibrary {
    pub(super) fn new(test: &str) -> Self {
        let path = std::env::temp_dir().join(format!("texturecraft-output-templates-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TemplateLibrary {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}
