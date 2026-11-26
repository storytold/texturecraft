//! Texture sets the original paints per UV tile (one texture per tile) instead of once over UV 0..1.
//!
//! TextureCraft holds such sets ([`texturecraft_core::TextureSet::uv_tiles`], an atlas of tiles), but where the original
//! keeps each tile's pixels (layer pixels, stored results, per-tile settings) has not been seen in any file yet. Until
//! it has, such a set imports as one texture over UV 0..1: every tile shows the same texture, and the report says so
//! instead of guessing a layout. The texture cache is read for one tile only: its middle folder level is the same in
//! every file seen, and several such folders in one set would otherwise give a channel more than one stored result.

use std::collections::{BTreeMap, BTreeSet};

use texturecraft_core::udim_at;
use texturecraft_geom::Mesh;
use texturecraft_import_reader::model::ProjectTextureSet;
use texturecraft_import_reader::project::CacheEntryRef;

/// The UDIM numbers of the tiles mesh material `slot`'s UVs reach (by triangle centroid), ascending; tiles outside the
/// UDIM range are left out.
pub fn mesh_tiles(mesh: &Mesh, slot: usize) -> BTreeSet<u32> {
    mesh.uv_tiles(slot).into_iter().filter_map(|(c, r)| udim_at([c as f32 + 0.5, r as f32 + 0.5])).collect()
}

/// Why a set imports as one texture although the original paints it per UV tile (`None` for a classic set).
pub fn per_tile_note(set: &ProjectTextureSet, tiles: &BTreeSet<u32>) -> Option<String> {
    if !set.uses_uv_tiles {
        return None;
    }
    let list = match tiles.len() {
        0 => "no tile".to_string(),
        _ => tiles.iter().map(u32::to_string).collect::<Vec<_>>().join(", "),
    };
    Some(format!(
        "painted per UV tile in the original ({} tile settings; the mesh's UVs reach {list}); imported as one texture over UV 0..1, so every tile shows the same texture (where the file keeps each tile's pixels is not known yet)",
        set.uv_tile_entries
    ))
}

/// The entries of the one cache tile folder used: the folder with the most entries (the first by name on a tie), and
/// the number of tile folders left out.
pub fn one_tile<'a>(entries: Vec<&'a CacheEntryRef>) -> (Vec<&'a CacheEntryRef>, usize) {
    let mut by_tile: BTreeMap<Option<&str>, Vec<&'a CacheEntryRef>> = BTreeMap::new();
    for e in entries {
        by_tile.entry(e.tile.as_deref()).or_default().push(e);
    }
    let others = by_tile.len().saturating_sub(1);
    let mut best: Option<Vec<&'a CacheEntryRef>> = None;
    for list in by_tile.into_values() {
        if best.as_ref().is_none_or(|b| list.len() > b.len()) {
            best = Some(list);
        }
    }
    (best.unwrap_or_default(), others)
}

/// The report note for cache tile folders left out by [`one_tile`].
pub fn other_tiles_note(others: usize) -> String {
    format!("stored results for {} UV tiles; one is used (the one with most channels) and {others} left out", others + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(tile: Option<&str>, bit: u64) -> CacheEntryRef {
        CacheEntryRef { path: format!("/c/s/{tile:?}/{bit}"), set: "s".into(), tile: tile.map(str::to_string), channel_bit: bit, len: 1 }
    }

    fn set(uses: bool, entries: usize) -> ProjectTextureSet {
        ProjectTextureSet {
            uid: None,
            material_uid: None,
            name: "S".into(),
            resolution_log2: None,
            channels: Vec::new(),
            shader_model: None,
            layers: Vec::new(),
            mesh_maps: Vec::new(),
            uses_uv_tiles: uses,
            uv_tile_entries: entries,
            unhandled: Vec::new(),
        }
    }

    #[test]
    fn one_cache_tile_is_read_per_set() {
        let a = [entry(Some("7"), 1), entry(Some("7"), 2), entry(Some("3"), 1)];
        let (kept, others) = one_tile(a.iter().collect());
        assert_eq!((kept.len(), others), (2, 1));
        assert!(kept.iter().all(|e| e.tile.as_deref() == Some("7")));
        // a tie keeps the first folder by name; a classic set keeps everything
        let b = [entry(Some("9"), 1), entry(Some("4"), 2)];
        let (kept, others) = one_tile(b.iter().collect());
        assert_eq!((kept[0].tile.as_deref(), others), (Some("4"), 1));
        let c = [entry(None, 1), entry(None, 2)];
        assert_eq!(one_tile(c.iter().collect()).0.len(), 2);
        let (none, others) = one_tile(Vec::new());
        assert!(none.is_empty() && others == 0);
        assert!(other_tiles_note(2).contains("3 UV tiles"));
    }

    #[test]
    fn per_tile_sets_get_a_note_and_the_mesh_tiles() {
        let mesh = texturecraft_geom::primitives::udim_plane();
        let tiles = mesh_tiles(&mesh, 0);
        assert_eq!(tiles.iter().copied().collect::<Vec<_>>(), [1001, 1002]);
        assert_eq!(per_tile_note(&set(false, 0), &tiles), None);
        let note = per_tile_note(&set(true, 2), &tiles).unwrap();
        assert!(note.contains("2 tile settings") && note.contains("1001, 1002") && note.contains("same texture"), "{note}");
        assert!(per_tile_note(&set(true, 0), &BTreeSet::new()).unwrap().contains("reach no tile"));
        let classic = texturecraft_geom::primitives::plane(1.0, 1);
        assert_eq!(mesh_tiles(&classic, 0).into_iter().collect::<Vec<_>>(), [1001]);
    }
}
