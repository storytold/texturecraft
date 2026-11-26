//! The per-baker settings of a texture set mapped onto TextureCraft's map parameters: the secondary rays of ambient
//! occlusion and thickness, ambient-occlusion attenuation and ground plane, thickness range, curvature (from the mesh
//! or from the normal map), position normalisation and the ID colours.
//!
//! Enumerations are numbered as the file stores them. The ray distribution, the attenuation, the position
//! normalisation and the thickness range were matched against bakes of the reference app; the backface mode, the ID
//! colour source and generator and the normal-map curvature settings follow the order and defaults of TextureCraft's
//! own options, which mirror the file's. Values whose meaning is not known stay listed.

use super::{Outcome, bounded, flag, int};
use texturecraft_core::{
    AmbientOcclusionAttenuation, AmbientOcclusionSettings, BackfaceMode, BakeSettings, BakedMap, CurvatureMethod, CurvatureSettings, IdColorGenerator,
    IdColorSource, IdSettings, PositionAxis, PositionMode, PositionNormalizationScale, PositionSettings, RayDistribution, RaySettings, ThicknessNormalization,
    ThicknessSettings,
};
use texturecraft_import_reader::model::baking::{TweakList, TweakValue};

/// Secondary rays per sample TextureCraft accepts.
pub const RAYS: (f32, f32) = (1.0, 256.0);
/// Ray distances: the smallest, and the largest relative to the bounding box and in mesh units.
pub const DISTANCE: (f32, f32, f32) = (0.00001, 1.0, 10.0);
/// Mesh curvature sampling radius: the smallest, the largest relative to the bounding box and in mesh units.
pub const SAMPLING_RADIUS: (f32, f32, f32) = (0.00001, 10.0, 1.0);
/// Curvature value bounds.
pub const CURVATURE_BOUNDS: (f32, f32) = (-10.0, 10.0);
/// Ambient-occlusion ground plane offset.
pub const GROUND_OFFSET: (f32, f32) = (0.0, 100.0);

const RELATIVE: &str = "Detail.SecondaryCommon.MaxDistanceRelativeScale";
/// The curvature baker that works on the mesh (the other one, `Curvature`, works on the normal map).
pub const MESH_CURVATURE_BAKER: &str = "Curvature1FromDetail";

/// The settings `list` of the baker of `kind` into `b`; returns one line per setting not taken over.
pub(super) fn map_baker(kind: BakedMap, list: &TweakList, b: &mut BakeSettings) -> Vec<String> {
    let baker = short_baker(list);
    let mut notes = Vec::new();
    let maps = &mut b.maps;
    for (k, v) in &list.tweaks {
        let outcome = match kind {
            BakedMap::AmbientOcclusion => map_ao(list, k, v, &mut maps.ao.parameters),
            BakedMap::Thickness => map_thickness(list, k, v, &mut maps.thickness.parameters),
            BakedMap::Curvature if baker == MESH_CURVATURE_BAKER => map_mesh_curvature(list, k, v, &mut maps.curvature.parameters),
            BakedMap::Curvature => map_normal_map_curvature(k, v, &mut maps.curvature.parameters),
            BakedMap::Position => map_position(list, k, v, &mut maps.position.parameters),
            BakedMap::Id => map_id(k, v, &mut maps.id.parameters),
            _ => Outcome::Listed,
        };
        if let Some(note) = outcome.note(k, v) {
            notes.push(format!("{baker} {note}"));
        }
    }
    let rays = match kind {
        BakedMap::AmbientOcclusion => Some(&mut maps.ao.parameters.rays),
        BakedMap::Thickness => Some(&mut maps.thickness.parameters.rays),
        _ => None,
    };
    if let Some(rays) = rays.filter(|r| r.min_distance > r.max_distance) {
        rays.min_distance = rays.max_distance;
        notes.push(format!("{baker} minimum distance above the maximum; the maximum used"));
    }
    let curvature = &mut maps.curvature.parameters.mesh;
    if kind == BakedMap::Curvature && !curvature.auto_tonemapping && curvature.value_min >= curvature.value_max {
        (curvature.value_min, curvature.value_max) = (-1.0, 1.0);
        notes.push(format!("{baker} tone mapping minimum not below the maximum; -1..1 used"));
    }
    notes
}

/// Which curvature baker TextureCraft runs: the mesh one when the file has it on, else the normal-map one when that is
/// on (older files have only that one).
pub(super) fn curvature_method<'a>(enabled_bakers: impl Iterator<Item = &'a str>) -> Option<CurvatureMethod> {
    let mut method = None;
    for baker in enabled_bakers {
        match baker {
            MESH_CURVATURE_BAKER => return Some(CurvatureMethod::Mesh),
            "Curvature" => method = Some(CurvatureMethod::NormalMap),
            _ => {}
        }
    }
    method
}

/// `GLMapBakerManager.AOFromDetail` → `AOFromDetail`.
pub(super) fn short_baker(list: &TweakList) -> &str {
    list.baker.rsplit('.').next().unwrap_or(&list.baker)
}

fn distribution(v: &TweakValue) -> Option<RayDistribution> {
    match int(v)? {
        0 => Some(RayDistribution::Uniform),
        1 => Some(RayDistribution::Cosine),
        _ => None,
    }
}

fn whole(v: &TweakValue, (lo, hi): (f32, f32), slot: &mut u32) -> Outcome {
    let mut x = *slot as f32;
    let outcome = bounded(v, lo, hi, &mut x);
    *slot = x.round() as u32;
    outcome
}

/// A setting every secondary-ray baker shares; `None` for the baker's own.
fn map_rays(list: &TweakList, key: &str, v: &TweakValue, rays: &mut RaySettings) -> Option<Outcome> {
    let relative = list.bool(RELATIVE).unwrap_or(rays.relative_to_bounding_box);
    let (min, rel_max, abs_max) = DISTANCE;
    Some(match key {
        "Detail.SecondaryCommon.NbSecondary" => whole(v, RAYS, &mut rays.rays),
        "Detail.SecondaryCommon.MinDistance" => bounded(v, min, rel_max, &mut rays.min_distance),
        "Detail.SecondaryCommon.MaxDistance" => bounded(v, min, if relative { rel_max } else { abs_max }, &mut rays.max_distance),
        RELATIVE => Outcome::of(flag(v).map(|r| rays.relative_to_bounding_box = r).is_some()),
        "Detail.SecondaryCommon.Spread" => bounded(v, 0.0, 180.0, &mut rays.spread),
        "Detail.SecondaryCommon.Distribution" => Outcome::of(distribution(v).map(|d| rays.distribution = d).is_some()),
        // TextureCraft has one filter, the file's first
        "Detail.SecondaryCommon.FilterMethodSecondary" => Outcome::of(v.as_f32() == Some(0.0)),
        _ => return None,
    })
}

fn map_ao(list: &TweakList, key: &str, v: &TweakValue, ao: &mut AmbientOcclusionSettings) -> Outcome {
    if let Some(outcome) = map_rays(list, key, v, &mut ao.rays) {
        return outcome;
    }
    match key {
        "Detail.SecondaryCommon.IgnoreBackfaceSecondary" => {
            let mode = match v {
                TweakValue::Bool(b) => Some(if *b { BackfaceMode::Always } else { BackfaceMode::Never }),
                v => match int(v) {
                    Some(0) => Some(BackfaceMode::Never),
                    Some(1) => Some(BackfaceMode::Always),
                    Some(2) => Some(BackfaceMode::ByMeshName),
                    _ => None,
                },
            };
            Outcome::of(mode.map(|m| ao.ignore_backface = m).is_some())
        }
        "Detail.AO.Attenuation" => {
            let attenuation = match int(v) {
                Some(0) => Some(AmbientOcclusionAttenuation::None),
                Some(1) => Some(AmbientOcclusionAttenuation::Smooth),
                Some(2) => Some(AmbientOcclusionAttenuation::Linear),
                _ => None,
            };
            Outcome::of(attenuation.map(|a| ao.attenuation = a).is_some())
        }
        "Detail.AO.GroundPlane" => Outcome::of(flag(v).map(|g| ao.ground_plane.enabled = g).is_some()),
        "Detail.AO.GroundOffset" => bounded(v, GROUND_OFFSET.0, GROUND_OFFSET.1, &mut ao.ground_plane.offset),
        _ => Outcome::Listed,
    }
}

fn map_thickness(list: &TweakList, key: &str, v: &TweakValue, thickness: &mut ThicknessSettings) -> Outcome {
    if let Some(outcome) = map_rays(list, key, v, &mut thickness.rays) {
        return outcome;
    }
    match key {
        // stretched over the smallest and largest value found, or relative to the ray distance
        "Detail.Distance.MaximizeRange" => Outcome::of(
            flag(v)
                .map(|m| thickness.normalization = if m { ThicknessNormalization::RelativeToMinMax } else { ThicknessNormalization::RelativeToRayDistances })
                .is_some(),
        ),
        _ => Outcome::Listed,
    }
}

fn map_mesh_curvature(list: &TweakList, key: &str, v: &TweakValue, curvature: &mut CurvatureSettings) -> Outcome {
    let mesh = &mut curvature.mesh;
    let relative = list.bool(RELATIVE).unwrap_or(mesh.relative_to_bounding_box);
    let (min, rel_max, abs_max) = SAMPLING_RADIUS;
    let (lo, hi) = CURVATURE_BOUNDS;
    match key {
        "Detail.SecondaryCommon.NbSecondary" => whole(v, RAYS, &mut mesh.rays),
        "Detail.Curvature1.SamplingRadius" => bounded(v, min, if relative { rel_max } else { abs_max }, &mut mesh.sampling_radius),
        RELATIVE => Outcome::of(flag(v).map(|r| mesh.relative_to_bounding_box = r).is_some()),
        "Detail.SecondaryCommon.FilterMethodSecondary" => Outcome::of(v.as_f32() == Some(0.0)),
        "Detail.Curvature1.AutoMinMax" => Outcome::of(flag(v).map(|a| mesh.auto_tonemapping = a).is_some()),
        "Detail.Curvature1.TonemappingMin" => bounded(v, lo, hi, &mut mesh.value_min),
        "Detail.Curvature1.TonemappingMax" => bounded(v, lo, hi, &mut mesh.value_max),
        _ => Outcome::Listed,
    }
}

fn map_normal_map_curvature(key: &str, v: &TweakValue, curvature: &mut CurvatureSettings) -> Outcome {
    let normal_map = &mut curvature.normal_map;
    match key {
        "Curvature.CurvMult" => bounded(v, 0.0, 1.0, &mut normal_map.details),
        "Curvature.EnableSeams" => Outcome::of(flag(v).map(|s| normal_map.enable_seams = s).is_some()),
        "Curvature.SeamsPower" => bounded(v, 0.0, 1.0, &mut normal_map.seams_intensity),
        // per texel, as TextureCraft computes it
        "Curvature.PerVertex" => Outcome::of(flag(v) == Some(false)),
        _ => Outcome::Listed,
    }
}

fn map_position(list: &TweakList, key: &str, v: &TweakValue, position: &mut PositionSettings) -> Outcome {
    let all_axes = list.get("Position.Mode").and_then(int) == Some(0);
    match key {
        "Position.Mode" if all_axes => {
            position.axis = PositionAxis::All;
            Outcome::Taken
        }
        // one axis only matters without "all axes"
        "Position.Axis" if all_axes => Outcome::Taken,
        // one scale for every axis, the bounding box's longest side
        "Position.Normalization" if int(v) == Some(1) => {
            position.normalization = PositionMode::BoundingSphere;
            Outcome::Taken
        }
        "Position.NormalizationScale" => {
            let scale = match int(v) {
                Some(0) => Some(PositionNormalizationScale::PerMaterial),
                Some(1) => Some(PositionNormalizationScale::FullScene),
                _ => None,
            };
            Outcome::of(scale.map(|s| position.scale = s).is_some())
        }
        _ => Outcome::Listed,
    }
}

fn map_id(key: &str, v: &TweakValue, id: &mut IdSettings) -> Outcome {
    match key {
        "Detail.Color.ColorSource" => {
            let source = match int(v) {
                Some(0) => Some(IdColorSource::VertexColor),
                Some(1) => Some(IdColorSource::MaterialColor),
                _ => None,
            };
            Outcome::of(source.map(|s| id.source = s).is_some())
        }
        "Detail.Color.ColorGenerator" => {
            let generator = match int(v) {
                Some(0) => Some(IdColorGenerator::Random),
                Some(1) => Some(IdColorGenerator::HueShift),
                Some(2) => Some(IdColorGenerator::Grayscale),
                _ => None,
            };
            Outcome::of(generator.map(|g| id.generator = g).is_some())
        }
        _ => Outcome::Listed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(baker: &str, tweaks: &[(&str, TweakValue)]) -> TweakList {
        TweakList { baker: format!("Bakers.{baker}"), tweaks: tweaks.iter().map(|(k, v)| (k.to_string(), v.clone())).collect() }
    }

    use TweakValue::{Bool, Float, Int};

    #[test]
    fn secondary_rays_attenuation_and_ground_plane_are_taken_over() {
        let mut b = BakeSettings::default();
        let ao = list(
            "AOFromDetail",
            &[
                ("Detail.SecondaryCommon.NbSecondary", Int(40)),
                ("Detail.SecondaryCommon.MinDistance", Float(0.001)),
                ("Detail.SecondaryCommon.MaxDistance", Float(5.0)),
                ("Detail.SecondaryCommon.Spread", Float(90.0)),
                ("Detail.SecondaryCommon.Distribution", Int(0)),
                ("Detail.SecondaryCommon.IgnoreBackfaceSecondary", Int(2)),
                ("Detail.SecondaryCommon.FilterMethodSecondary", Int(0)),
                ("Detail.AO.Attenuation", Int(1)),
                ("Detail.AO.GroundPlane", Bool(true)),
                ("Detail.AO.GroundOffset", Float(0.5)),
                // after the distance it qualifies: still read first
                (RELATIVE, Bool(false)),
            ],
        );
        assert_eq!(map_baker(BakedMap::AmbientOcclusion, &ao, &mut b), Vec::<String>::new());
        let p = b.maps.ao.parameters;
        assert_eq!((p.rays.rays, p.rays.min_distance, p.rays.max_distance, p.rays.spread), (40, 0.001, 5.0, 90.0));
        assert!(!p.rays.relative_to_bounding_box);
        assert_eq!(
            (p.rays.distribution, p.ignore_backface, p.attenuation),
            (RayDistribution::Uniform, BackfaceMode::ByMeshName, AmbientOcclusionAttenuation::Smooth)
        );
        assert_eq!((p.ground_plane.enabled, p.ground_plane.offset), (true, 0.5));

        let thickness = list(
            "ThicknessFromDetail",
            &[
                ("Detail.SecondaryCommon.NbSecondary", Int(1000)),
                ("Detail.SecondaryCommon.MaxDistance", Float(0.05)),
                ("Detail.SecondaryCommon.MinDistance", Float(0.5)),
                ("Detail.SecondaryCommon.Distribution", Int(1)),
                ("Detail.Distance.MaximizeRange", Int(0)),
                ("Detail.Distance.Unknown", Int(0)),
            ],
        );
        let notes = map_baker(BakedMap::Thickness, &thickness, &mut b);
        let t = b.maps.thickness.parameters;
        assert_eq!((t.rays.rays, t.rays.max_distance, t.rays.min_distance), (256, 0.05, 0.05));
        assert_eq!((t.rays.distribution, t.normalization), (RayDistribution::Cosine, ThicknessNormalization::RelativeToRayDistances));
        assert_eq!(notes.len(), 3, "{notes:?}");
        assert!(notes[0].starts_with("ThicknessFromDetail Detail.SecondaryCommon.NbSecondary = 1000 (outside"), "{notes:?}");
    }

    #[test]
    fn both_curvature_bakers_position_and_id_are_taken_over() {
        let mut b = BakeSettings::default();
        let mesh = list(
            MESH_CURVATURE_BAKER,
            &[
                ("Detail.SecondaryCommon.NbSecondary", Int(16)),
                ("Detail.Curvature1.SamplingRadius", Float(0.002)),
                (RELATIVE, Bool(true)),
                ("Detail.Curvature1.AutoMinMax", Bool(false)),
                ("Detail.Curvature1.TonemappingMin", Float(-2.0)),
                ("Detail.Curvature1.TonemappingMax", Float(3.0)),
            ],
        );
        assert!(map_baker(BakedMap::Curvature, &mesh, &mut b).is_empty());
        let m = b.maps.curvature.parameters.mesh;
        assert_eq!((m.rays, m.sampling_radius, m.auto_tonemapping, m.value_min, m.value_max), (16, 0.002, false, -2.0, 3.0));
        let normal_map = list(
            "Curvature",
            &[
                ("Curvature.PerVertex", Int(0)),
                ("Curvature.CurvMult", Float(0.25)),
                ("Curvature.EnableSeams", Bool(false)),
                ("Curvature.SeamsPower", Float(0.5)),
            ],
        );
        assert!(map_baker(BakedMap::Curvature, &normal_map, &mut b).is_empty());
        let n = b.maps.curvature.parameters.normal_map;
        assert_eq!((n.details, n.enable_seams, n.seams_intensity), (0.25, false, 0.5));
        assert_eq!(curvature_method(["Curvature"].into_iter()), Some(CurvatureMethod::NormalMap));
        assert_eq!(curvature_method(["Curvature", MESH_CURVATURE_BAKER].into_iter()), Some(CurvatureMethod::Mesh));
        assert_eq!(curvature_method(std::iter::empty()), None);

        let position = list(
            "Position",
            &[("Position.Mode", Int(0)), ("Position.Axis", Int(2)), ("Position.Normalization", Int(1)), ("Position.NormalizationScale", Int(0))],
        );
        b.maps.position.parameters.normalization = PositionMode::None;
        assert!(map_baker(BakedMap::Position, &position, &mut b).is_empty());
        let p = b.maps.position.parameters;
        assert_eq!((p.axis, p.normalization, p.scale), (PositionAxis::All, PositionMode::BoundingSphere, PositionNormalizationScale::PerMaterial));
        // one axis, or another normalisation: not known, listed
        let one_axis = list("Position", &[("Position.Mode", Int(1)), ("Position.Axis", Int(2)), ("Position.Normalization", Int(0))]);
        assert_eq!(map_baker(BakedMap::Position, &one_axis, &mut b).len(), 3);

        let id = list("ColorFromDetail", &[("Detail.Color.ColorSource", Int(0)), ("Detail.Color.ColorGenerator", Int(2))]);
        assert!(map_baker(BakedMap::Id, &id, &mut b).is_empty());
        assert_eq!((b.maps.id.parameters.source, b.maps.id.parameters.generator), (IdColorSource::VertexColor, IdColorGenerator::Grayscale));
        assert_eq!(map_baker(BakedMap::Id, &list("ColorFromDetail", &[("Detail.Color.ColorSource", Int(4))]), &mut b).len(), 1);
    }

    #[test]
    fn bounds_the_engine_refuses_are_repaired_and_listed() {
        let mut b = BakeSettings::default();
        let mesh = list(
            MESH_CURVATURE_BAKER,
            &[
                ("Detail.Curvature1.AutoMinMax", Bool(false)),
                ("Detail.Curvature1.TonemappingMin", Float(2.0)),
                ("Detail.Curvature1.TonemappingMax", Float(1.0)),
            ],
        );
        let notes = map_baker(BakedMap::Curvature, &mesh, &mut b);
        assert_eq!((b.maps.curvature.parameters.mesh.value_min, b.maps.curvature.parameters.mesh.value_max), (-1.0, 1.0));
        assert_eq!(notes.len(), 1, "{notes:?}");
        let mut b = BakeSettings::default();
        let ao = list("AOFromDetail", &[("Detail.SecondaryCommon.MaxDistance", Float(f32::INFINITY)), ("Detail.SecondaryCommon.Spread", Float(400.0))]);
        assert_eq!(map_baker(BakedMap::AmbientOcclusion, &ao, &mut b).len(), 2);
        assert_eq!((b.maps.ao.parameters.rays.max_distance, b.maps.ao.parameters.rays.spread), (1.0, 180.0));
    }
}
