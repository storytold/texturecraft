/// A temporary library folder removed when the test ends, including failures
pub(super) struct Library(pub(super) std::path::PathBuf);

impl Drop for Library {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}
