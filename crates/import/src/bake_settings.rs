//! The file's baking settings of one texture set (`/baking/baking.ini`) mapped onto TextureCraft's [`BakeSettings`]:
//! output size, padding, diffusion, antialiasing, which bakers are on, the high-poly matching (low as high, name
//! suffixes, backfaces), the cage distances and each baker's own settings ([`bakers`]). Every setting without a counterpart, and every value outside the range
//! TextureCraft's bake settings accept, is listed so the report can say it was not taken over.

mod bakers;
mod high;

pub use high::MeshRequest;

use texturecraft_core::{BakeAntialiasing, BakeResolution, BakeSettings, BakedMap};
use texturecraft_import_reader::model::baking::{MaterialBaking, TweakValue};

/// The ranges TextureCraft's bake settings accept (the engine refuses settings outside them).
pub mod range {
    /// Padding width in texels.
    pub const DILATION: u32 = 128;
    /// Frontal and rear cage distances.
    pub const CAGE_DISTANCE: (f32, f32) = (0.0, 0.05);
}

/// File-format data: the bakers' `usage` numbers.
fn usage_map(usage: u64) -> Option<BakedMap> {
    Some(match usage {
        0 => BakedMap::Id,
        1 => BakedMap::AmbientOcclusion,
        2 => BakedMap::Curvature,
        3 => BakedMap::WorldNormal,
        4 => BakedMap::Position,
        5 => BakedMap::Thickness,
        6 => BakedMap::Normal,
        _ => return None,
    })
}

/// One set's settings as TextureCraft's, and what of them was not taken over.
#[derive(Debug, Clone, PartialEq)]
pub struct MappedBake {
    pub settings: BakeSettings,
    /// One line per setting not taken over (`key = value`, with the reason where it is not "no counterpart").
    pub unmapped: Vec<String>,
    /// The high-poly and cage mesh files the original baked with (not in the file; reported).
    pub meshes: MeshRequest,
}

/// What became of one setting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Outcome {
    /// Taken over as it is (or it changes nothing).
    Taken,
    /// Outside the range TextureCraft accepts: the nearest value in range was taken.
    Clamped(f32),
    /// No counterpart.
    Listed,
}

impl Outcome {
    pub(crate) fn of(taken: bool) -> Outcome {
        if taken { Outcome::Taken } else { Outcome::Listed }
    }

    /// The report line of setting `key` = `v` (none when taken over).
    pub(crate) fn note(self, key: &str, v: &TweakValue) -> Option<String> {
        match self {
            Outcome::Taken => None,
            Outcome::Clamped(used) => Some(format!("{key} = {v} (outside TextureCraft's range; {used} used)")),
            Outcome::Listed => Some(format!("{key} = {v}")),
        }
    }
}

/// A boolean as the file stores it (a bool, or a number in older files).
pub(crate) fn flag(v: &TweakValue) -> Option<bool> {
    match v {
        TweakValue::Bool(b) => Some(*b),
        other => other.as_f32().map(|x| x != 0.0),
    }
}

/// A whole number as the file stores it (an int, a bool, or a float without fraction).
pub(crate) fn int(v: &TweakValue) -> Option<i64> {
    match v {
        TweakValue::Int(i) => Some(*i),
        TweakValue::Bool(b) => Some(i64::from(*b)),
        TweakValue::Float(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e9 => Some(*f as i64),
        _ => None,
    }
}

fn text(v: &TweakValue) -> Option<&str> {
    match v {
        TweakValue::Text(t) => Some(t),
        _ => None,
    }
}

/// Stores the number `v` into `slot`, clamped to `lo..=hi`; listed when `v` is no finite number.
pub(crate) fn bounded(v: &TweakValue, lo: f32, hi: f32, slot: &mut f32) -> Outcome {
    match v.as_f32().filter(|x| x.is_finite()) {
        Some(x) if (lo..=hi).contains(&x) => {
            *slot = x;
            Outcome::Taken
        }
        Some(x) => {
            *slot = x.clamp(lo, hi);
            Outcome::Clamped(*slot)
        }
        None => Outcome::Listed,
    }
}

/// `settings` (the set's, if the file has any) as TextureCraft's, at `resolution` when the file names no output size.
pub fn map(settings: Option<&MaterialBaking>, resolution: u32) -> MappedBake {
    let mut bake = BakeSettings::default();
    bake.common.resolution = BakeResolution::from_pixels(resolution).unwrap_or_default();
    let mut unmapped = Vec::new();
    let Some(p) = settings else {
        unmapped.push("no baking settings in the file; defaults used".to_string());
        return MappedBake { settings: bake, unmapped, meshes: MeshRequest::default() };
    };
    for (k, v) in &p.common.tweaks {
        if let Some(note) = map_common(k, v, &mut bake).note(k, v) {
            unmapped.push(note);
        }
    }
    for (k, v) in &p.detail.tweaks {
        if let Some(note) = map_detail(k, v, &mut bake).note(k, v) {
            unmapped.push(note);
        }
    }
    let enabled: Vec<BakedMap> = p.bakers.iter().filter(|b| b.enabled).filter_map(|b| usage_map(b.usage?)).collect();
    if !p.bakers.is_empty() {
        for map in BakedMap::ALL {
            bake.maps.set_enabled(map, enabled.contains(&map));
        }
    }
    // every baker's settings are kept (a baker switched on later bakes with them); only the enabled ones are reported
    for baker in &p.bakers {
        let notes = match baker.usage.and_then(usage_map) {
            Some(kind) => bakers::map_baker(kind, &baker.params, &mut bake),
            None => baker.params.tweaks.iter().map(|(k, v)| format!("{} {k} = {v}", bakers::short_baker(&baker.params))).collect(),
        };
        if baker.enabled {
            unmapped.extend(notes);
        }
    }
    if let Some(method) = bakers::curvature_method(
        p.bakers.iter().filter(|b| b.enabled && b.usage.and_then(usage_map) == Some(BakedMap::Curvature)).map(|b| bakers::short_baker(&b.params)),
    ) {
        bake.maps.curvature.parameters.method = method;
    }
    MappedBake { settings: bake, unmapped, meshes: MeshRequest::of(p) }
}

/// One common setting (size, padding, diffusion).
fn map_common(key: &str, v: &TweakValue, bake: &mut BakeSettings) -> Outcome {
    match key {
        // log2 of width and height
        "Common.OutputSize" => match v {
            TweakValue::Int2([w, h]) => {
                let side = |log2: i32| u32::try_from(log2).ok().filter(|l| *l <= 14).and_then(|l| BakeResolution::from_pixels(1 << l));
                match (side(*w), side(*h)) {
                    (Some(w), Some(h)) => {
                        bake.common.resolution = w;
                        bake.common.resolution_height = (h != w).then_some(h);
                        Outcome::Taken
                    }
                    _ => Outcome::Listed,
                }
            }
            _ => Outcome::Listed,
        },
        "Common.DilationWidth" => {
            let mut width = bake.common.dilation as f32;
            let outcome = bounded(v, 0.0, range::DILATION as f32, &mut width);
            bake.common.dilation = width as u32;
            outcome
        }
        "Common.ApplyDiffusion" => Outcome::of(flag(v).map(|b| bake.common.apply_diffusion = b).is_some()),
        _ => Outcome::Listed,
    }
}

/// One detail setting: how rays leave the low poly and which high poly they hit.
fn map_detail(key: &str, v: &TweakValue, bake: &mut BakeSettings) -> Outcome {
    let (lo, hi) = range::CAGE_DISTANCE;
    match key {
        "Detail.SubSampling" => Outcome::of(
            v.as_f32()
                .or_else(|| flag(v).map(f32::from))
                .map(|s| bake.common.antialiasing = if s > 0.0 { BakeAntialiasing::Subsampling2x2 } else { BakeAntialiasing::None })
                .is_some(),
        ),
        "Detail.LowAsHigh" => Outcome::of(flag(v).map(|b| bake.high_poly.use_low_as_high = b).is_some()),
        "Detail.IgnoreBackface" => Outcome::of(flag(v).map(|b| bake.high_poly.ignore_backface = b).is_some()),
        "Detail.LowpolySuffix" => Outcome::of(text(v).map(|s| bake.high_poly.low_suffix = s.to_string()).is_some()),
        "Detail.HipolySuffix" => Outcome::of(text(v).map(|s| bake.high_poly.high_suffix = s.to_string()).is_some()),
        "Detail.BackfaceCullingSuffix" => Outcome::of(text(v).map(|s| bake.high_poly.ignore_backface_suffix = s.to_string()).is_some()),
        "Detail.MaxHeight" => bounded(v, lo, hi, &mut bake.cage.max_frontal),
        "Detail.MaxDepth" => bounded(v, lo, hi, &mut bake.cage.max_rear),
        "Detail.RelativeScale" => Outcome::of(flag(v).map(|b| bake.cage.relative_to_bounding_box = b).is_some()),
        "Detail.SmoothNormals" => Outcome::of(flag(v).map(|b| bake.cage.average_normals = b).is_some()),
        // a cage file is not part of the project file: only "no cage" is taken over
        "Detail.UseCage" => Outcome::of(flag(v) == Some(false)),
        // TextureCraft has one filter, the file's first
        "Detail.FilterMethod" => Outcome::of(v.as_f32() == Some(0.0)),
        // the mesh files are not in the project file: reported as [`MeshRequest`]
        "Detail.HipolyMesh" | "Detail.CageMesh" => Outcome::Taken,
        _ => Outcome::Listed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_import_reader::model::baking::{BakerSettings, TweakList};

    fn list(tweaks: &[(&str, TweakValue)]) -> TweakList {
        TweakList { baker: String::new(), tweaks: tweaks.iter().map(|(k, v)| (k.to_string(), v.clone())).collect() }
    }

    fn baking(common: &[(&str, TweakValue)], detail: &[(&str, TweakValue)], bakers: Vec<BakerSettings>) -> MaterialBaking {
        MaterialBaking { material_uid: None, enabled: true, common: list(common), detail: list(detail), bakers }
    }

    #[test]
    fn sizes_padding_bakers_and_rays_map_and_the_rest_is_listed() {
        let defaults = map(None, 1024);
        assert_eq!((defaults.settings.common.resolution, defaults.unmapped.len()), (BakeResolution::R1024, 1));
        let p = baking(
            &[("Common.OutputSize", TweakValue::Int2([12, 11])), ("Common.DilationWidth", TweakValue::Float(8.0))],
            &[("Detail.SubSampling", TweakValue::Float(1.0)), ("Detail.Odd", TweakValue::Float(1.0))],
            vec![BakerSettings {
                usage: Some(1),
                enabled: true,
                params: TweakList { baker: "Bakers.AO".into(), tweaks: vec![("Detail.SecondaryCommon.NbSecondary".into(), TweakValue::Float(64.0))] },
            }],
        );
        let MappedBake { settings: b, unmapped, .. } = map(Some(&p), 2048);
        assert_eq!((b.common.resolution, b.common.resolution_height), (BakeResolution::R4096, Some(BakeResolution::R2048)));
        assert_eq!((b.common.dilation, b.common.antialiasing), (8, BakeAntialiasing::Subsampling2x2));
        assert_eq!(b.maps.ao.parameters.rays.rays, 64);
        assert!(b.maps.is_enabled(BakedMap::AmbientOcclusion) && !b.maps.is_enabled(BakedMap::Curvature));
        assert_eq!(unmapped, ["Detail.Odd = 1"]);
    }

    #[test]
    fn high_poly_matching_and_cage_distances_are_taken_over() {
        let text = |s: &str| TweakValue::Text(s.into());
        let p = baking(
            &[("Common.OutputSize", TweakValue::Int2([10, 10])), ("Common.ApplyDiffusion", TweakValue::Bool(false))],
            &[
                ("Detail.LowAsHigh", TweakValue::Bool(true)),
                ("Detail.HipolyMesh", text("")),
                ("Detail.UseCage", TweakValue::Bool(false)),
                ("Detail.CageMesh", text("")),
                ("Detail.MaxHeight", TweakValue::Float(0.02)),
                ("Detail.MaxDepth", TweakValue::Float(0.03)),
                ("Detail.RelativeScale", TweakValue::Bool(false)),
                ("Detail.SmoothNormals", TweakValue::Int(0)),
                ("Detail.IgnoreBackface", TweakValue::Bool(false)),
                ("Detail.FilterMethod", TweakValue::Int(0)),
                ("Detail.LowpolySuffix", text("_lp")),
                ("Detail.HipolySuffix", text("_hp")),
                ("Detail.BackfaceCullingSuffix", text("_bf")),
            ],
            Vec::new(),
        );
        let MappedBake { settings: b, unmapped, .. } = map(Some(&p), 512);
        assert!(unmapped.is_empty(), "{unmapped:?}");
        assert_eq!((b.common.resolution, b.common.resolution_height, b.common.apply_diffusion), (BakeResolution::R1024, None, false));
        let h = &b.high_poly;
        assert!(h.use_low_as_high && !h.ignore_backface);
        assert_eq!((h.low_suffix.as_str(), h.high_suffix.as_str(), h.ignore_backface_suffix.as_str()), ("_lp", "_hp", "_bf"));
        assert_eq!((b.cage.max_frontal, b.cage.max_rear, b.cage.relative_to_bounding_box, b.cage.average_normals), (0.02, 0.03, false, false));
    }

    #[test]
    fn values_out_of_range_or_without_counterpart_are_listed() {
        let p = baking(
            &[("Common.OutputSize", TweakValue::Int2([20, 10])), ("Common.DilationWidth", TweakValue::Int(300))],
            &[
                ("Detail.MaxHeight", TweakValue::Float(0.1)),
                ("Detail.MaxDepth", TweakValue::Float(f32::NAN)),
                ("Detail.UseCage", TweakValue::Bool(true)),
                ("Detail.FilterMethod", TweakValue::Int(1)),
                ("Detail.LowpolySuffix", TweakValue::Int(3)),
            ],
            Vec::new(),
        );
        let MappedBake { settings: b, unmapped, .. } = map(Some(&p), 2048);
        assert_eq!((b.common.resolution, b.common.dilation), (BakeResolution::R2048, range::DILATION));
        assert_eq!((b.cage.max_frontal, b.cage.max_rear), (range::CAGE_DISTANCE.1, 0.01));
        assert_eq!(b.high_poly.low_suffix, "_low");
        assert_eq!(unmapped.len(), 7, "{unmapped:?}");
        assert!(unmapped[2].starts_with("Detail.MaxHeight = 0.1 (outside"), "{unmapped:?}");
    }
}
