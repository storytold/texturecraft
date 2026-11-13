//! The mesh maps the file baked (ambient occlusion, curvature, normal, …) as TextureCraft's baked maps of a texture set.
//!
//! The document lists them per texture set (`additionalMaps` and `normalMapBitmap`, by resource URL, or
//! `resourcealias:<Map>` when nothing is assigned, which the original then ignores). Sets the document does not
//! describe fall back to resource names of the form `<Map> from Mesh <set>`. Baked normal maps are stored in the
//! project's normal map convention and become OpenGL here.

use std::collections::BTreeMap;
use std::sync::Arc;

use texturecraft_compose::{BakedMaps, Texels};
use texturecraft_core::BakedMap;
use texturecraft_graph::tex::Tex;
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

/// One map read and laid out as TextureCraft holds it at `resolution` (resampled bilinearly when the bake has another
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

/// `image` as `kind.components()` floats per texel at `resolution` square. A bake of another size is resampled
/// bilinearly at texel centres, wrapping at the edges: the way the graph runtime samples an image of another size, and
/// so the way a graph reads a mesh map baked smaller or larger than its set (nearest sampling turned each texel of a
/// half-size bake into a 2×2 block, and graphs reading it, ambient-occlusion masks among them, steps off along edges).
pub fn dense(image: &Rgba, kind: BakedMap, resolution: u32, flip_green: bool) -> Vec<f32> {
    let comps = kind.components();
    let r = resolution.max(1) as usize;
    let same = image.width as usize == r && image.height as usize == r;
    let resampled = (!same && image.width > 0 && image.height > 0).then(|| {
        let src = Tex { w: image.width as usize, h: image.height as usize, px: image.px.clone() };
        Tex::from_fn(r, r, |u, v| src.sample(u, v))
    });
    let mut out = Vec::with_capacity(r * r * comps);
    for y in 0..r {
        for x in 0..r {
            let p = match &resampled {
                Some(t) => t.px.get(y * r + x).copied().unwrap_or([0.0; 4]),
                None => image.get(x as u32, y as u32),
            };
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
    BakedMaps { resolution, maps: maps.into_iter().map(|(k, v)| (k, Arc::new(Texels::new(v)))).collect() }
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
        // texel centres 1/8 and 3/8 of the way across: bilinear between the two texels, wrapping at the left edge
        assert!((ao[0] - 0.35).abs() < 1e-6 && (ao[1] - 0.35).abs() < 1e-6, "{ao:?}");
        assert!((ao[2] - 0.65).abs() < 1e-6 && (ao[3] - 0.65).abs() < 1e-6, "{ao:?}");
        let n = dense(&img, BakedMap::Normal, 2, true);
        assert!((n[1] - 0.7).abs() < 1e-6, "green flipped: {n:?}");
        let m = maps(2, [(BakedMap::Normal, n)].into());
        assert_eq!(m.height(BakedMap::Normal), 2);
    }

    #[test]
    fn a_bake_of_the_set_size_is_copied() {
        let px: Vec<[f32; 4]> = (0..9).map(|i| [i as f32 / 8.0, 0.0, 0.0, 1.0]).collect();
        let img = Rgba { width: 3, height: 3, px: px.clone() };
        let ao = dense(&img, BakedMap::AmbientOcclusion, 3, false);
        assert_eq!(ao, px.iter().map(|p| p[0]).collect::<Vec<_>>());
    }

    #[test]
    fn a_half_size_bake_is_smooth_not_blocky() {
        // a horizontal ramp baked at 8 texels, read by a 16-texel set: nearest sampling repeated every value twice
        let img = Rgba { width: 8, height: 8, px: (0..64).map(|i| [(i % 8) as f32 / 7.0, 0.0, 0.0, 1.0]).collect() };
        let ao = dense(&img, BakedMap::AmbientOcclusion, 16, false);
        let row: Vec<f32> = ao[16 * 5..16 * 6].to_vec();
        for x in 1..14 {
            assert!(row[x + 1] > row[x], "strictly rising inside the ramp at {x}: {row:?}");
        }
        // between source texels 2 and 3 (values 2/7 and 3/7), a quarter and three quarters of the way
        assert!((row[5] - (2.25 / 7.0)).abs() < 1e-5 && (row[6] - (2.75 / 7.0)).abs() < 1e-5, "{row:?}");
    }

    #[test]
    fn a_double_size_bake_averages_texel_pairs() {
        let img = Rgba { width: 4, height: 4, px: (0..16).map(|i| [if i % 2 == 0 { 0.0 } else { 1.0 }, 0.0, 0.0, 1.0]).collect() };
        let ao = dense(&img, BakedMap::AmbientOcclusion, 2, false);
        assert!(ao.iter().all(|v| (v - 0.5).abs() < 1e-6), "{ao:?}");
    }
}
