//! The file's baking settings of one texture set (`/baking/baking.ini`) mapped onto TextureCraft's [`BakeSettings`]
//! where they fit: output size, padding, antialiasing, which bakers are on, and the ambient-occlusion and thickness
//! rays. Every other setting is listed so the report can say it was not taken over.

use texturecraft_core::{BakeAntialiasing, BakeResolution, BakeSettings, BakedMap};
use texturecraft_import_reader::model::baking::{MaterialBaking, TweakValue};

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

/// Settings that change nothing inside the islands or only steer inputs resolved elsewhere.
const NO_EFFECT: &[&str] =
    &["Detail.LowAsHigh", "Detail.HipolyMesh", "Detail.CageMesh", "Detail.LowpolySuffix", "Detail.HipolySuffix", "Detail.BackfaceCullingSuffix"];

fn flag(v: &TweakValue) -> Option<bool> {
    match v {
        TweakValue::Bool(b) => Some(*b),
        other => other.as_f32().map(|x| x != 0.0),
    }
}

/// `settings` (the set's, if the file has any) as TextureCraft's, and the settings not taken over.
pub fn map(settings: Option<&MaterialBaking>, resolution: u32) -> (BakeSettings, Vec<String>) {
    let mut bake = BakeSettings::default();
    bake.common.resolution = BakeResolution::from_pixels(resolution).unwrap_or_default();
    let mut unmapped = Vec::new();
    let Some(p) = settings else {
        unmapped.push("no baking settings in the file; defaults used".to_string());
        return (bake, unmapped);
    };
    if let Some(TweakValue::Int2([w, h])) = p.common.get("Common.OutputSize") {
        let log2 = (*w).max(*h);
        if let Some(r) = (0..=14).contains(&log2).then(|| 1u32 << log2).and_then(BakeResolution::from_pixels) {
            bake.common.resolution = r;
        }
    }
    if let Some(w) = p.common.f32("Common.DilationWidth").filter(|w| w.is_finite()) {
        bake.common.dilation = w.clamp(0.0, 128.0) as u32;
    }
    for (k, v) in &p.detail.tweaks {
        let mapped = match k.as_str() {
            "Detail.SubSampling" => v
                .as_f32()
                .or_else(|| flag(v).map(f32::from))
                .map(|s| bake.common.antialiasing = if s > 0.0 { BakeAntialiasing::Subsampling2x2 } else { BakeAntialiasing::None })
                .is_some(),
            "Detail.RelativeScale" => flag(v) == Some(true),
            "Detail.UseCage" => flag(v) == Some(false),
            k if NO_EFFECT.contains(&k) => true,
            _ => false,
        };
        if !mapped {
            unmapped.push(format!("{k} = {v}"));
        }
    }
    let enabled: Vec<BakedMap> = p.bakers.iter().filter(|b| b.enabled).filter_map(|b| usage_map(b.usage?)).collect();
    if !p.bakers.is_empty() {
        for map in BakedMap::ALL {
            bake.maps.set_enabled(map, enabled.contains(&map));
        }
    }
    for baker in p.bakers.iter().filter(|b| b.enabled) {
        let kind = baker.usage.and_then(usage_map);
        for (k, v) in &baker.params.tweaks {
            let mapped = match (kind, k.as_str()) {
                (Some(BakedMap::AmbientOcclusion), "Detail.SecondaryCommon.NbSecondary") => {
                    v.as_f32().map(|n| bake.maps.ao.parameters.rays.rays = n.clamp(1.0, 4096.0) as u32).is_some()
                }
                (Some(BakedMap::AmbientOcclusion), "Detail.SecondaryCommon.MaxDistance") => {
                    v.as_f32().filter(|d| d.is_finite()).map(|d| bake.maps.ao.parameters.rays.max_distance = d.max(1e-5)).is_some()
                }
                (Some(BakedMap::AmbientOcclusion), "Detail.SecondaryCommon.Spread") => {
                    v.as_f32().filter(|d| d.is_finite()).map(|s| bake.maps.ao.parameters.rays.spread = s.clamp(0.0, 180.0)).is_some()
                }
                (Some(BakedMap::Thickness), "Detail.SecondaryCommon.NbSecondary") => {
                    v.as_f32().map(|n| bake.maps.thickness.parameters.rays.rays = n.clamp(1.0, 4096.0) as u32).is_some()
                }
                (Some(BakedMap::Thickness), "Detail.SecondaryCommon.MaxDistance") => {
                    v.as_f32().filter(|d| d.is_finite()).map(|d| bake.maps.thickness.parameters.rays.max_distance = d.max(1e-5)).is_some()
                }
                _ => false,
            };
            if !mapped {
                let baker_name = baker.params.baker.rsplit('.').next().unwrap_or(&baker.params.baker);
                unmapped.push(format!("{baker_name} {k} = {v}"));
            }
        }
    }
    (bake, unmapped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_import_reader::model::baking::{BakerSettings, TweakList};

    #[test]
    fn sizes_padding_bakers_and_rays_map_and_the_rest_is_listed() {
        let (defaults, notes) = map(None, 1024);
        assert_eq!((defaults.common.resolution, notes.len()), (BakeResolution::R1024, 1));
        let p = MaterialBaking {
            material_uid: None,
            enabled: true,
            common: TweakList {
                baker: String::new(),
                tweaks: vec![("Common.OutputSize".into(), TweakValue::Int2([12, 11])), ("Common.DilationWidth".into(), TweakValue::Float(8.0))],
            },
            detail: TweakList {
                baker: String::new(),
                tweaks: vec![("Detail.SubSampling".into(), TweakValue::Float(1.0)), ("Detail.Odd".into(), TweakValue::Float(1.0))],
            },
            bakers: vec![BakerSettings {
                usage: Some(1),
                enabled: true,
                params: TweakList { baker: "Bakers.AO".into(), tweaks: vec![("Detail.SecondaryCommon.NbSecondary".into(), TweakValue::Float(64.0))] },
            }],
        };
        let (b, notes) = map(Some(&p), 2048);
        assert_eq!((b.common.resolution, b.common.dilation, b.common.antialiasing), (BakeResolution::R4096, 8, BakeAntialiasing::Subsampling2x2));
        assert_eq!(b.maps.ao.parameters.rays.rays, 64);
        assert!(b.maps.is_enabled(BakedMap::AmbientOcclusion) && !b.maps.is_enabled(BakedMap::Curvature));
        assert_eq!(notes.len(), 1, "{notes:?}");
    }
}
