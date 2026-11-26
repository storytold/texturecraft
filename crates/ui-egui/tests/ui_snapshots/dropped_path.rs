use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug)]
pub struct DroppedPath(PathBuf);

impl egui::DroppedFile for DroppedPath {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|error| error.to_string())
    }
}

pub fn handle(path: PathBuf) -> egui::DroppedFileHandle {
    Arc::new(DroppedPath(path))
}
