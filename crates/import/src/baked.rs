//! The mesh maps the file baked (ambient occlusion, curvature, normal, …) as TextureCraft's baked maps of a texture set.
//!
//! The document lists them per texture set (`additionalMaps` and `normalMapBitmap`, by resource URL, or
//! `resourcealias:<Map>` when nothing is assigned, which the original then ignores). Sets the document does not
//! describe fall back to resource names of the form `<Map> from Mesh <set>`. Baked normal maps are stored in the
//! project's normal map convention and become OpenGL here.

use std::collections::BTreeMap;
use std::sync::Arc;

use texturecraft_compose::BakedMaps;
use texturecraft_core::BakedMap;
use texturecraft_import_reader::decode::document::{BitmapRef, NormalMapSyscoord};
use texturecraft_import_reader::project::ProjectFile;

use crate::image::Rgba;
use crate::resources::{read_image, split_resource_url};

/// File-format data: resource-name prefixes of baked maps, for sets the document does not describe.
const NAME_PREFIXES: &[(&str, Option<u64>)] = &[
    ("Normal Map from Mesh ", None),
    ("Color Map from Mesh ", Some(0)),
    ("Ambient Occlusion Map from Mesh ", Some(1)),
    ("Curvature Map from Mesh ", Some(2)),
    ("World Space Normals ", Some(3)),
    ("Position ", Some(4)),
    ("Thickness Map from Mesh ", Some(5)),
];

/// TextureCraft's map for a `DataAdditionalMap.usage` (`None` = the normal map); `None` for usages it has no map for.
pub fn baked_kind(usage: Option<u64>) -> Option<BakedMap> {
    Some(match usage {
        None => BakedMap::Normal,
        Some(0) => BakedMap::Id,
        Some(1) => BakedMap::AmbientOcclusion,
        Some(2) => BakedMap::Curvature,
        Some(3) => BakedMap::WorldNormal,
        Some(4) => BakedMap::Position,
        Some(5) => BakedMap::Thickness,
        Some(_) => return None,
    })
}

/// The map a resource name (or a URL without its leading slash) says it is.
pub fn baked_kind_of_name(name: &str) -> Option<BakedMap> {
    NAME_PREFIXES.iter().find(|(prefix, _)| name.starts_with(prefix)).and_then(|(_, usage)| baked_kind(*usage))
}

/// A baked map to read.
#[derive(Debug, Clone, PartialEq)]
pub struct BakedSource {
    /// `None` for a usage TextureCraft has no map for.
    pub kind: Option<BakedMap>,
    pub usage: Option<u64>,
    pub name: String,
    pub resource: String,
}

/// The baked maps of a set: from the document's references when it has any (empty slots skipped), else from resource
/// names.
pub fn sources(project: &ProjectFile, set_name: &str, refs: &[BitmapRef]) -> Vec<BakedSource> {
    if !refs.is_empty() {
        return refs
            .iter()
            .filter_map(|r| {
                let (name, id) = split_resource_url(&r.url)?;
                Some(BakedSource { kind: baked_kind(r.usage), usage: r.usage, name: name.to_string(), resource: id.to_string() })
            })
            .collect();
    }
    let Some(toc) = project.resource_toc().ok().flatten() else { return Vec::new() };
    toc.entries
        .iter()
        .filter_map(|e| {
            let usage = NAME_PREFIXES.iter().find(|(prefix, _)| e.name.strip_prefix(prefix) == Some(set_name))?.1;
            Some(BakedSource { kind: baked_kind(usage), usage, name: e.name.clone(), resource: e.version.clone()? })
        })
        .collect()
}

/// One map read and laid out as TextureCraft holds it at `resolution` (nearest resampling when the bake has another
/// size), the normal map in OpenGL convention. Returns the dense values and how many bytes the stored block was short.
pub fn read(
    project: &ProjectFile,
    source: &BakedSource,
    kind: BakedMap,
    resolution: u32,
    syscoord: Option<NormalMapSyscoord>,
) -> Result<(Vec<f32>, usize), String> {
    let read = read_image(project, &source.resource).map_err(|e| e.to_string())?;
    let flip_green = kind == BakedMap::Normal && syscoord == Some(NormalMapSyscoord::DirectX);
    Ok((dense(&read.image, kind, resolution, flip_green), read.missing_bytes))
}

/// `image` as `kind.components()` floats per texel at `resolution` square, nearest sampling.
pub fn dense(image: &Rgba, kind: BakedMap, resolution: u32, flip_green: bool) -> Vec<f32> {
    let comps = kind.components();
    let r = resolution.max(1) as usize;
    let mut out = Vec::with_capacity(r * r * comps);
    for y in 0..r {
        let sy = (y * image.height as usize / r).min(image.height.saturating_sub(1) as usize) as u32;
        for x in 0..r {
            let sx = (x * image.width as usize / r).min(image.width.saturating_sub(1) as usize) as u32;
            let p = image.get(sx, sy);
            let p = p.map(|v| if v.is_finite() { v } else { 0.0 });
            if comps == 3 {
                out.extend([p[0], if flip_green { 1.0 - p[1] } else { p[1] }, p[2]]);
            } else {
                out.push(p[0]);
            }
        }
    }
    out
}

/// Collects a set's maps.
pub fn maps(resolution: u32, maps: BTreeMap<BakedMap, Vec<f32>>) -> BakedMaps {
    BakedMaps { resolution, maps: maps.into_iter().map(|(k, v)| (k, Arc::new(v))).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usages_and_names_map_to_maps() {
        assert_eq!(baked_kind(None), Some(BakedMap::Normal));
        assert_eq!(baked_kind(Some(3)), Some(BakedMap::WorldNormal));
        assert_eq!(baked_kind(Some(9)), None);
        assert_eq!(baked_kind_of_name("Ambient Occlusion Map from Mesh Cup"), Some(BakedMap::AmbientOcclusion));
        assert_eq!(baked_kind_of_name("Something"), None);
    }

    #[test]
    fn maps_are_resampled_and_normals_flipped() {
        let img = Rgba { width: 2, height: 1, px: vec![[0.2, 0.3, 1.0, 1.0], [0.8, 0.9, 1.0, 1.0]] };
        let ao = dense(&img, BakedMap::AmbientOcclusion, 4, false);
        assert_eq!(ao.len(), 16);
        assert_eq!((ao[0], ao[3]), (0.2, 0.8));
        let n = dense(&img, BakedMap::Normal, 2, true);
        assert!((n[1] - 0.7).abs() < 1e-6, "green flipped: {n:?}");
        let m = maps(2, [(BakedMap::Normal, n)].into());
        assert_eq!(m.height(BakedMap::Normal), 2);
    }
}
