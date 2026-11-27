use std::collections::BTreeMap;

use texturecraft_engine::Session;
use texturecraft_engine::bake::BakePaintMap;
use texturecraft_engine::core::{BakePaintTarget, CageType, HighPolySettings, SkewSettings};
use texturecraft_engine::geom::Mesh;

use super::shared_handle::SharedHandle;

/// Everything a texture set's cage shell and ray reach are built from except the distances, which only move its corners
#[derive(Clone, Debug, PartialEq)]
pub(super) struct CageShellKey {
    mesh_id: u64,
    high: SharedHandle<Mesh>,
    cage_file: SharedHandle<Mesh>,
    material: usize,
    uv_tiles: Vec<u32>,
    tile_resolutions: BTreeMap<u32, u32>,
    cage_kind: CageType,
    relative_to_bounding_box: bool,
    average_normals: bool,
    /// Whether the set has a cage at all, the name matching the automatic cage fits by and the backfaces the reach leaves out
    high_poly: HighPolySettings,
    skew: SkewSettings,
    /// The painted skew and cage maps
    paint: [SharedHandle<BakePaintMap>; 2],
}

impl CageShellKey {
    pub(super) fn of(session: &Session, texture_set: usize) -> Option<CageShellKey> {
        let set = session.doc.project.texture_sets.get(texture_set)?;
        let paint = |target| SharedHandle::of(session.doc.bake_paint.get(&(texture_set, target)));
        Some(CageShellKey {
            mesh_id: session.mesh_id,
            high: SharedHandle::of(session.doc.bake_meshes.merged_high().map(|high| &high.mesh)),
            cage_file: SharedHandle::of(session.doc.bake_meshes.cage.as_ref().map(|cage| &cage.mesh)),
            material: set.material,
            uv_tiles: set.uv_tiles.clone(),
            tile_resolutions: set.tile_resolutions.clone(),
            cage_kind: set.bake.cage.kind,
            relative_to_bounding_box: set.bake.cage.relative_to_bounding_box,
            average_normals: set.bake.cage.average_normals,
            high_poly: set.bake.high_poly.clone(),
            skew: set.bake.skew,
            paint: [paint(BakePaintTarget::Skew), paint(BakePaintTarget::Cage)],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_engine::core::MeshMatchMode;
    use texturecraft_engine::geom::primitives;

    #[test]
    fn the_key_changes_with_the_name_matching_the_automatic_cage_is_fitted_by() {
        let mut session = Session::new_project("Cage", primitives::plane(1.0, 1), "plane", None, 128).unwrap();
        let before = CageShellKey::of(&session, 0);

        session.doc.project.texture_sets[0].bake.high_poly.match_mode = MeshMatchMode::ByMeshName;
        let by_name = CageShellKey::of(&session, 0);
        session.doc.project.texture_sets[0].bake.high_poly.high_suffix = "_hp".into();

        assert_ne!(before, by_name);
        assert_ne!(by_name, CageShellKey::of(&session, 0));
    }
}
