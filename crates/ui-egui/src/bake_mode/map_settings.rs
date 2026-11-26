use super::controls::{choice, integer, number};
use crate::i18n::tr;
use crate::property_sheet::PropertySheet;
use serde::Serialize;
use serde_json::{Value, json};
use texturecraft_engine::core::{
    AmbientOcclusionAttenuation, AmbientOcclusionSettings, BackfaceMode, BakedMap, BentNormalSettings, CurvatureMethod, CurvatureSettings, HeightNormalization,
    HeightSettings, IdColorGenerator, IdColorSource, IdSettings, MeshMapSettings, PositionAxis, PositionMode, PositionNormalizationScale, PositionSettings,
    RayDistribution, RaySettings, SelfOcclusion, ThicknessNormalization, ThicknessSettings,
};

/// Most secondary rays a slider offers, as the reference's does (the bake settings command accepts the same)
const MAX_SECONDARY_RAYS: u32 = 256;

/// Smallest distance, radius or divisor a slider offers: the bake settings command's lower limit
const SHORTEST_DISTANCE: f32 = 0.00001;

/// Whether `map` has parameters (and so a Restore Defaults)
pub(super) fn has_parameters(map: BakedMap) -> bool {
    !matches!(map, BakedMap::Normal | BakedMap::WorldNormal | BakedMap::Opacity)
}

/// Draw `map`'s parameters; `restore` sets them back to the defaults. Maps without parameters say so
pub(super) fn show(sheet: &mut PropertySheet<'_>, map: BakedMap, settings: &mut MeshMapSettings, restore: bool) -> Option<Value> {
    match map {
        BakedMap::Normal | BakedMap::WorldNormal | BakedMap::Opacity => {
            sheet.dim_line(tr("No Parameters"));
            None
        }
        BakedMap::Id => parameters(sheet, "id", &mut settings.id.parameters, restore, id),
        BakedMap::AmbientOcclusion => parameters(sheet, "ao", &mut settings.ao.parameters, restore, ambient_occlusion),
        BakedMap::Curvature => parameters(sheet, "curvature", &mut settings.curvature.parameters, restore, curvature),
        BakedMap::Position => parameters(sheet, "position", &mut settings.position.parameters, restore, position),
        BakedMap::Thickness => parameters(sheet, "thickness", &mut settings.thickness.parameters, restore, thickness),
        BakedMap::Height => parameters(sheet, "height", &mut settings.height.parameters, restore, height),
        BakedMap::BentNormal => parameters(sheet, "bent_normal", &mut settings.bent_normal.parameters, restore, bent_normal),
    }
}

/// A map's controls; on `restore` only its parameters go back to the defaults
fn parameters<T: Default + Serialize>(
    sheet: &mut PropertySheet<'_>,
    map_key: &str,
    parameters: &mut T,
    restore: bool,
    controls: impl FnOnce(&mut PropertySheet<'_>, &mut T),
) -> Option<Value> {
    controls(sheet, parameters);
    if restore {
        *parameters = T::default();
        return Some(json!({"maps": {map_key: {"parameters": parameters}}}));
    }
    None
}

/// Settings that belong to the one above: indented under it, greyed while it rules them out
fn dependent(sheet: &mut PropertySheet<'_>, applies: bool, body: impl FnOnce(&mut PropertySheet<'_>)) {
    sheet.greyed(applies, None, |sheet| sheet.indented(body));
}

fn id(sheet: &mut PropertySheet<'_>, settings: &mut IdSettings) {
    choice(
        sheet,
        tr("Color Source"),
        &mut settings.source,
        &[
            (IdColorSource::VertexColor, tr("Vertex Color")),
            (IdColorSource::MaterialColor, tr("Material Color")),
            (IdColorSource::FileId, tr("File ID")),
            (IdColorSource::MeshId, tr("Mesh ID / Polygroup")),
            (IdColorSource::Submesh, tr("Submesh")),
        ],
    );

    // Keep dependent controls visible when disabled
    let generates_colors = matches!(settings.source, IdColorSource::FileId | IdColorSource::MeshId | IdColorSource::Submesh);
    dependent(sheet, generates_colors, |sheet| {
        choice(
            sheet,
            tr("Color Generator"),
            &mut settings.generator,
            &[(IdColorGenerator::Random, tr("Random")), (IdColorGenerator::HueShift, tr("Hue Shift")), (IdColorGenerator::Grayscale, tr("Grayscale"))],
        );
    });
}

fn rays(sheet: &mut PropertySheet<'_>, settings: &mut RaySettings) {
    integer(sheet, tr("Secondary Rays"), &mut settings.rays, 1..=MAX_SECONDARY_RAYS);
    number(sheet, tr("Min Occluder Distance"), &mut settings.min_distance, SHORTEST_DISTANCE..=1.0, 5);
    number(sheet, tr("Max Occluder Distance"), &mut settings.max_distance, SHORTEST_DISTANCE..=if settings.relative_to_bounding_box { 1.0 } else { 10.0 }, 5);
    if sheet.check(tr("Relative to Bounding Box"), &mut settings.relative_to_bounding_box) && settings.relative_to_bounding_box {
        settings.max_distance = settings.max_distance.min(1.0);
    }
    number(sheet, tr("Spread Angle"), &mut settings.spread, 0.0..=180.0, 0);
    choice(sheet, tr("Distribution"), &mut settings.distribution, &[(RayDistribution::Uniform, tr("Uniform")), (RayDistribution::Cosine, tr("Cosine"))]);
}

fn backface(sheet: &mut PropertySheet<'_>, settings: &mut BackfaceMode) {
    choice(
        sheet,
        tr("Ignore Backface"),
        settings,
        &[(BackfaceMode::Never, tr("Never")), (BackfaceMode::Always, tr("Always")), (BackfaceMode::ByMeshName, tr("By Mesh Name"))],
    );
}

fn occlusion(sheet: &mut PropertySheet<'_>, label: &str, settings: &mut SelfOcclusion) {
    choice(sheet, label, settings, &[(SelfOcclusion::Always, tr("Always")), (SelfOcclusion::SameMeshName, tr("Only Same Mesh Name"))]);
}

fn ambient_occlusion(sheet: &mut PropertySheet<'_>, settings: &mut AmbientOcclusionSettings) {
    rays(sheet, &mut settings.rays);
    backface(sheet, &mut settings.ignore_backface);
    occlusion(sheet, tr("Self Occlusion"), &mut settings.self_occlusion);
    choice(
        sheet,
        tr("Attenuation"),
        &mut settings.attenuation,
        &[
            (AmbientOcclusionAttenuation::None, tr("None")),
            (AmbientOcclusionAttenuation::Smooth, tr("Smooth")),
            (AmbientOcclusionAttenuation::Linear, tr("Linear")),
        ],
    );
    sheet.check(tr("Ground Plane"), &mut settings.ground_plane.enabled);
    dependent(sheet, settings.ground_plane.enabled, |sheet| number(sheet, tr("Ground Plane Offset"), &mut settings.ground_plane.offset, 0.0..=100.0, 2));
}

fn curvature(sheet: &mut PropertySheet<'_>, settings: &mut CurvatureSettings) {
    choice(
        sheet,
        tr("Method"),
        &mut settings.method,
        &[(CurvatureMethod::Mesh, tr("Generate from Mesh")), (CurvatureMethod::NormalMap, tr("Generate from Normal Map"))],
    );
    match settings.method {
        CurvatureMethod::NormalMap => {
            number(sheet, tr("Details"), &mut settings.normal_map.details, 0.0..=1.0, 2);
            sheet.check(tr("Enable Seams"), &mut settings.normal_map.enable_seams);
            let seams = settings.normal_map.enable_seams;
            dependent(sheet, seams, |sheet| number(sheet, tr("Seams Intensity"), &mut settings.normal_map.seams_intensity, 0.0..=1.0, 2));
        }
        CurvatureMethod::Mesh => {
            let mesh = &mut settings.mesh;
            integer(sheet, tr("Secondary Rays"), &mut mesh.rays, 1..=MAX_SECONDARY_RAYS);
            number(sheet, tr("Sampling Radius"), &mut mesh.sampling_radius, SHORTEST_DISTANCE..=if mesh.relative_to_bounding_box { 10.0 } else { 1.0 }, 5);
            if sheet.check(tr("Relative to Bounding Box"), &mut mesh.relative_to_bounding_box) && !mesh.relative_to_bounding_box {
                mesh.sampling_radius = mesh.sampling_radius.min(1.0);
            }
            occlusion(sheet, tr("Self Intersection"), &mut mesh.self_intersection);
            sheet.check(tr("Auto Tonemapping (per UV Tile)"), &mut mesh.auto_tonemapping);
            dependent(sheet, !mesh.auto_tonemapping, |sheet| {
                number(sheet, tr("Value Bounds Min"), &mut mesh.value_min, -10.0..=10.0, 2);
                number(sheet, tr("Value Bounds Max"), &mut mesh.value_max, -10.0..=10.0, 2);
            });
        }
    }
}

fn position(sheet: &mut PropertySheet<'_>, settings: &mut PositionSettings) {
    // Axis selection applies only in One Axis mode
    let mut one_axis = settings.axis != PositionAxis::All;
    choice(sheet, tr("Mode"), &mut one_axis, &[(false, tr("All Axis")), (true, tr("One Axis"))]);
    if one_axis && settings.axis == PositionAxis::All {
        settings.axis = PositionAxis::X;
    } else if !one_axis {
        settings.axis = PositionAxis::All;
    }
    dependent(sheet, one_axis, |sheet| {
        let mut axis = if one_axis { settings.axis } else { PositionAxis::X };
        choice(sheet, tr("Axis"), &mut axis, &[(PositionAxis::X, "X"), (PositionAxis::Y, "Y"), (PositionAxis::Z, "Z")]);
        if one_axis {
            settings.axis = axis;
        }
    });
    choice(
        sheet,
        tr("Normalization Type"),
        &mut settings.normalization,
        &[(PositionMode::BoundingBox, tr("BBox")), (PositionMode::BoundingSphere, tr("BSphere")), (PositionMode::None, tr("None"))],
    );
    choice(
        sheet,
        tr("Normalization Scale"),
        &mut settings.scale,
        &[(PositionNormalizationScale::PerMaterial, tr("Per Material")), (PositionNormalizationScale::FullScene, tr("Full Scene"))],
    );
}

fn thickness(sheet: &mut PropertySheet<'_>, settings: &mut ThicknessSettings) {
    rays(sheet, &mut settings.rays);
    occlusion(sheet, tr("Self Occlusion"), &mut settings.self_occlusion);
    choice(
        sheet,
        tr("Normalization"),
        &mut settings.normalization,
        &[
            (ThicknessNormalization::RelativeToRayDistances, tr("Relative to Ray Distances")),
            (ThicknessNormalization::RelativeToMinMax, tr("Relative to Min/Max (per UV Tile)")),
            (ThicknessNormalization::None, tr("None")),
        ],
    );
}

fn height(sheet: &mut PropertySheet<'_>, settings: &mut HeightSettings) {
    choice(
        sheet,
        tr("Normalization"),
        &mut settings.normalization,
        &[
            (HeightNormalization::RelativeToRayDistances, tr("Relative to Ray Distances")),
            (HeightNormalization::RelativeToLowMesh, tr("Relative to Low Mesh (per UV Tile)")),
            (HeightNormalization::RelativeToMinMax, tr("Relative to Min/Max (per UV Tile)")),
            (HeightNormalization::Manual, tr("Manual")),
        ],
    );
    let manual = settings.normalization == HeightNormalization::Manual;
    dependent(sheet, manual, |sheet| number(sheet, tr("Scaling Divisor"), &mut settings.scaling_divisor, SHORTEST_DISTANCE..=100.0, 5));
}

fn bent_normal(sheet: &mut PropertySheet<'_>, settings: &mut BentNormalSettings) {
    rays(sheet, &mut settings.rays);
    backface(sheet, &mut settings.ignore_backface);
    occlusion(sheet, tr("Self Occlusion"), &mut settings.self_occlusion);
}
