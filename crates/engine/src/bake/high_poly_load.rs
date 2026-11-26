use std::sync::Arc;

use texturecraft_formats::MeshFile;

use super::mesh_map_command::{ADD_HIGH_POLY_LABEL, parse_mesh, unique_name};
use super::{BakeMesh, BakeMeshes, MergedHigh};
use crate::{Result, Session};

/// A high poly file parsed and merged with the meshes it joins, which a worker thread can prepare while the session stays in use
pub struct HighPolyLoad {
    /// The name the file was picked by, before it was made unique in the list
    file_name: String,
    added: BakeMesh,
    /// The list the mesh joined, with it at the end
    high: Vec<BakeMesh>,
    /// `high` as bakes see it
    merged: Option<MergedHigh>,
    /// Files the mesh refers to that could not be read, as messages for the log
    warnings: Vec<String>,
}

impl HighPolyLoad {
    /// Parse `file` and merge it with the high polys of `meshes`
    pub fn prepare(meshes: &BakeMeshes, file: MeshFile) -> Result<Self> {
        let mut added = parse_mesh(&file.name, file.bytes)?;
        let taken: Vec<&str> = meshes.high.iter().map(|high| high.name.as_str()).collect();
        added.name = unique_name(&file.name, &taken);

        let mut joined = BakeMeshes { high: meshes.high.clone(), ..Default::default() };
        joined.high.push(added.clone());
        joined.rebuild();
        Ok(Self { file_name: file.name, added, high: joined.high, merged: joined.merged, warnings: file.warnings })
    }

    /// Read the mesh file at `path`, then [`HighPolyLoad::prepare`] it
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read(meshes: &BakeMeshes, path: &str) -> Result<Self> {
        Self::prepare(meshes, crate::file_cmds::read_mesh("bake.highPoly.add", path)?)
    }

    /// Whether `current` is still the list this mesh was merged into
    fn joins(&self, current: &[BakeMesh]) -> bool {
        let joined = &self.high[..self.high.len() - 1];
        joined.len() == current.len()
            && joined.iter().zip(current).all(|(joined, current)| joined.name == current.name && Arc::ptr_eq(&joined.mesh, &current.mesh))
    }
}

impl Session {
    /// Add a prepared high poly as one undo step; every texture set then bakes from the high polys. Returns its name
    pub fn apply_high_poly_load(&mut self, load: HighPolyLoad) -> String {
        let before = self.doc.clone();
        let meshes = &mut self.doc.bake_meshes;
        let mut added = load.added.clone();
        if load.joins(&meshes.high) {
            meshes.high = load.high;
            meshes.merged = load.merged;
        } else {
            // The list changed while the file loaded, so the mesh joins the list as it is now
            let taken: Vec<&str> = meshes.high.iter().map(|high| high.name.as_str()).collect();
            added.name = unique_name(&load.file_name, &taken);
            meshes.high.push(added.clone());
            meshes.rebuild();
        }

        let textures = added.base_color.iter().flatten().filter(|color| !color.texels.is_empty()).count();
        self.log.info("Baker", format!("High poly {}: {} triangles, {textures} base color textures", added.name, added.mesh.triangle_count()));
        self.log.warnings("Baker", load.warnings);
        for set in &mut self.doc.project.texture_sets {
            set.bake.high_poly.use_low_as_high = false;
        }
        self.push_history(ADD_HIGH_POLY_LABEL, before);
        added.name
    }
}
