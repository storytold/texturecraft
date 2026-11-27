use std::path::Path;
use std::sync::Arc;

use texturecraft_core::PROJECT_EXT;
use texturecraft_formats::{ProjectFile, project_file::SaveProgress};
use texturecraft_geom::Mesh;

use super::{BakePaintMaps, SaveOutput, ext_of, fe, prepare_archive, save_target::SaveTarget};
use crate::Result;
use crate::session::Session;

/// A manual save of the document at the moment it was requested. Large payloads are shared
/// until the worker assembles the archive.
pub struct SaveTask {
    target: SaveTarget,
    archive: ProjectFile<Arc<Vec<u8>>, Arc<Vec<u8>>>,
    bake_paint: BakePaintMaps,
    uv_source: Option<Arc<Mesh>>,
}

impl Session {
    pub fn save_task(&self, path: &str, adopt: bool) -> SaveTask {
        let path = if ext_of(path) == PROJECT_EXT { path.to_string() } else { format!("{path}.{PROJECT_EXT}") };
        let saved_revision = (!self.stroke_active()).then_some(self.save_revision);

        SaveTask {
            target: SaveTarget { project_id: self.project_id, saved_revision, adopt, path },
            archive: self.capture_archive(self.doc.resources.clone(), Arc::clone),
            bake_paint: self.doc.bake_paint.clone(),
            uv_source: self.saved_uv_source().cloned(),
        }
    }
}

impl SaveTask {
    /// Complete the atomic write on the worker. Waiting for a previous save to release the
    /// RAM allowance also happens here.
    pub fn run(mut self, progress: Option<&SaveProgress>) -> Result<SaveOutput> {
        let mut reservation = texturecraft_formats::RamReservation::for_save().map_err(fe)?;
        prepare_archive(&mut self.archive, self.uv_source.as_deref(), &self.bake_paint);
        drop(self.uv_source);
        drop(self.bake_paint);

        let path = Path::new(&self.target.path);
        match progress {
            Some(progress) => reservation.save_with_progress(path, &self.archive, progress),
            None => reservation.save(path, &self.archive),
        }
        .map_err(fe)?;

        Ok(SaveOutput { path: self.target.path, project_id: self.target.project_id, saved_revision: self.target.saved_revision, adopt: self.target.adopt })
    }
}
