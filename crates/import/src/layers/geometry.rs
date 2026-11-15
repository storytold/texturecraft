//! A layer's geometry mask ([`ProjectGeometryMask`]) as TextureCraft's [`GeometryMask`].
//!
//! A mask by mesh becomes the list of the set's mesh parts it includes. A mask by UV tile becomes a UV-tile mask when
//! its meaning is clear without the tile list (the file's list layout is not known yet): with the default on and an
//! empty list the layer shows everywhere, with the default off and an empty list nowhere. A list of tiles, or a type we
//! don't know, is not computable: the layer would cover more than it does in the original.

use texturecraft_core::{GeometryMask, UDIM_RANGE};
use texturecraft_import_reader::blockers::Blocker;
use texturecraft_import_reader::model::{GeometryMaskKind, ProjectGeometryMask};

use crate::status::Status;

/// The mapped mask (`None`: the layer shows everywhere) and how exact it is. `parts` are the names of the set's mesh
/// parts; `tiles` the set's UV tiles (empty: the one tile 1001).
pub fn map(mask: Option<&ProjectGeometryMask>, parts: &[String], tiles: &[u32]) -> (Option<GeometryMask>, Status) {
    let Some(mask) = mask.filter(|m| m.restricts()) else { return (None, Status::Exact) };
    match &mask.kind {
        GeometryMaskKind::Meshes => {
            let included: Vec<String> = parts.iter().filter(|n| mask.includes_mesh(n)).cloned().collect();
            match included.len() == parts.len() {
                true => (None, Status::Exact),
                false => (Some(GeometryMask::Parts(included)), Status::Exact),
            }
        }
        GeometryMaskKind::UvTiles if mask.uv_tile_entries == 0 => match mask.uv_tile_default {
            true => (None, Status::Exact),
            false => (Some(GeometryMask::UvTiles(Vec::new())), Status::Exact),
        },
        GeometryMaskKind::UvTiles => {
            let all: Vec<u32> = if tiles.is_empty() { vec![*UDIM_RANGE.start()] } else { tiles.to_vec() };
            let shown = if mask.uv_tile_default { all } else { Vec::new() };
            let why =
                Blocker::feature("geometry-mask-uv-tile-list", format!("geometry mask listing {} UV tiles (list layout not known)", mask.uv_tile_entries));
            (Some(GeometryMask::UvTiles(shown)), Status::not_computable(why))
        }
        GeometryMaskKind::Other(t) => (None, Status::not_computable(Blocker::feature("geometry-mask-type", format!("geometry mask of unknown type {t}")))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask(kind: GeometryMaskKind, mesh_default: bool, list: &[&str], tile_default: bool, entries: usize) -> ProjectGeometryMask {
        ProjectGeometryMask {
            enabled: true,
            kind,
            mesh_default,
            mesh_list: list.iter().map(|s| s.to_string()).collect(),
            uv_tile_default: tile_default,
            uv_tile_entries: entries,
        }
    }

    #[test]
    fn mesh_masks_list_the_included_parts() {
        let parts = vec!["Body".to_string(), "Eyes".to_string()];
        let (m, s) = map(Some(&mask(GeometryMaskKind::Meshes, true, &["Body", "Gone"], true, 0)), &parts, &[]);
        assert_eq!((m, s.is_exact()), (Some(GeometryMask::Parts(vec!["Eyes".into()])), true));
        let (m, _) = map(Some(&mask(GeometryMaskKind::Meshes, true, &["Gone"], true, 0)), &parts, &[]);
        assert_eq!(m, None, "every part included");
        assert_eq!(map(None, &parts, &[]).0, None);
    }

    #[test]
    fn uv_tile_masks_map_when_clear_and_lock_otherwise() {
        let (m, s) = map(Some(&mask(GeometryMaskKind::UvTiles, true, &[], false, 0)), &[], &[]);
        assert_eq!((m, s.is_exact()), (Some(GeometryMask::UvTiles(vec![])), true));
        let (m, s) = map(Some(&mask(GeometryMaskKind::UvTiles, true, &[], true, 2)), &[], &[]);
        assert_eq!(m, Some(GeometryMask::UvTiles(vec![1001])));
        assert_eq!(s.lock_reasons()[0].key, "feature:geometry-mask-uv-tile-list");
        assert!(!map(Some(&mask(GeometryMaskKind::Other(9), true, &[], true, 0)), &[], &[]).1.is_computable());
    }
}
