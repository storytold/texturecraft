//! The Parameters rows of the wear kinds and the Light generator, and the Micro Details rows they share with Curvature and Ambient Occlusion

use egui::{Painter, Rect};
use serde_json::json;
use texturecraft_engine::core::{Generator, GeneratorKind, LayerId, LightParameters, MicroCurvatureType, MicroDetails, WearParameters};

use crate::TextureCraftApp;
use crate::generator_input_card::input_card;
use crate::generator_input_slot::GeneratorInputSlot;
use crate::generator_params::{light_params, merged, micro_params, wear_params};
use crate::i18n::tr;
use crate::property_sheet::{PropertySheet, StackSlider};

/// The wear controls of generator effect `id` on `layer` in the reference's order, showing `wear`; `picture` draws the preview
pub(crate) fn wear_rows(
    app: &mut TextureCraftApp,
    sheet: &mut PropertySheet<'_>,
    (layer, id): (LayerId, u64),
    generator: &Generator,
    wear: WearParameters,
    picture: &dyn Fn(&Painter, Rect),
) {
    let kind = generator.kind;
    let mut seed = generator.seed;
    if sheet.seed(tr("Seed"), &mut seed) {
        app.run("effect.set", json!({"effect": id, "seed": seed}));
    }
    let mut invert = generator.invert;
    if sheet.toggle(tr("Invert"), &mut invert) {
        app.run("effect.set", json!({"effect": id, "invert": invert}));
    }

    let dirt = kind == GeneratorKind::Dirt;
    let (level_label, contrast_label) = if dirt { (tr("Dirt Coverage"), tr("Dirt Definition")) } else { (tr("Wear Coverage"), tr("Wear Definition")) };
    let mut edited = wear;
    let mut stack = [
        StackSlider::unit(level_label, &mut edited.level),
        StackSlider::unit(contrast_label, &mut edited.contrast),
        StackSlider::unit(tr("Pattern Influence"), &mut edited.grunge_amount),
    ];
    let mut edit = sheet.picture_sliders(picture, &mut stack).into_iter().flatten().next();
    if kind != GeneratorKind::FiberGlassEdgeWear {
        edit = edit.or(sheet.slider(tr("Pattern Repeats"), &mut edited.grunge_scale, WearParameters::GRUNGE_SCALE_RANGE, 0));
    }
    if dirt {
        edit = edit.or(sheet.slider(tr("Keep Raised Edges Clean"), &mut edited.edges_masking, 0.0..=1.0, 2));
    } else {
        edit = edit.or(sheet.slider(tr("Edge Falloff"), &mut edited.edges_smoothness, WearParameters::EDGES_SMOOTHNESS_RANGE, 2));
        edit = edit.or(sheet.slider(tr("Keep Recesses Clean"), &mut edited.ao_masking, 0.0..=1.0, 2));
        edit = edit.or(sheet.slider(tr("Convex Edge Influence"), &mut edited.curvature_weight, 0.0..=1.0, 2));
    }

    if sheet.switch_card(tr("Project on Three Axes"), &mut edited.triplanar) {
        edit = Some(false);
    }
    if edited.triplanar {
        edit = edit.or(sheet.slider(tr("Projection Blend Hardness"), &mut edited.triplanar_contrast, 0.0..=1.0, 2));
    }
    sheet.close_card();

    if sheet.switch_card(tr("Custom Wear Pattern"), &mut edited.custom_grunge) {
        edit = Some(false);
    }
    if edited.custom_grunge {
        input_card(app, sheet, (layer, id), GeneratorInputSlot::Grunge, generator.grunge.as_ref(), true);
    }
    sheet.close_card();

    if let Some(merge) = edit {
        app.run("effect.set", merged(json!({"effect": id, "merge": merge}), wear_params(kind, &edited)));
    }

    if sheet.card(tr("Micro Details"), Some(false)) {
        micro_rows(app, sheet, (layer, id), generator);
    }
    sheet.close_card();
}

/// The Light generator's controls of effect `id`, showing `shown`; `invert` adds its Invert row (None on the earlier model, whose global invert it is)
pub(crate) fn light_rows(
    app: &mut TextureCraftApp,
    sheet: &mut PropertySheet<'_>,
    id: u64,
    invert: Option<bool>,
    shown: LightParameters,
    picture: &dyn Fn(&Painter, Rect),
) {
    if let Some(mut invert) = invert
        && sheet.toggle(tr("Invert"), &mut invert)
    {
        app.run("effect.set", json!({"effect": id, "invert": invert}));
    }

    let mut edited = shown;
    let angle = |label, value| StackSlider {
        label,
        value,
        range: LightParameters::ANGLE_RANGE,
        typed: LightParameters::ANGLE_RANGE,
        decimals: 0,
        suffix: "°",
        dial: true,
    };
    let mut stack = [
        angle(tr("Horizontal Angle"), &mut edited.horizontal),
        angle(tr("Vertical Angle"), &mut edited.vertical),
        StackSlider::unit(tr("Light Attenuation"), &mut edited.attenuation),
    ];
    let mut edit = sheet.picture_sliders(picture, &mut stack).into_iter().flatten().next();
    edit = edit.or(sheet.slider(tr("Highlight Glossiness"), &mut edited.glossiness, LightParameters::GLOSSINESS_RANGE, 2));
    edit = edit.or(sheet.slider(tr("Highlight Level"), &mut edited.highlight_level, 0.0..=1.0, 2));
    if let Some(merge) = edit {
        app.run("effect.set", merged(json!({"effect": id, "merge": merge}), light_params(&edited)));
    }
}

/// Curvature's and Ambient Occlusion's Use Micro Details: its switch card, holding the micro details rows while on
pub(crate) fn micro_switch_card(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, (layer, id): (LayerId, u64), generator: &Generator) {
    let Some(defaults) = MicroDetails::defaults(generator.kind) else { return };

    let mut on = generator.micro.as_ref().map_or(defaults.enabled, |micro| micro.enabled);
    if sheet.switch_card(tr("Use Micro Details"), &mut on) {
        app.run("effect.set", json!({"effect": id, "useMicroDetails": on}));
    }
    if on {
        micro_rows(app, sheet, (layer, id), generator);
    }
    sheet.close_card();
}

/// The micro details rows of generator effect `id`, inside the card open now: each image's switch with its slot while on, then the kind's settings
fn micro_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, (layer, id): (LayerId, u64), generator: &Generator) {
    let kind = generator.kind;
    let Some(defaults) = MicroDetails::defaults(kind) else { return };

    let shown = generator.micro.as_deref().cloned().unwrap_or(defaults);
    let mut edited = shown.clone();
    let mut edit = None;
    if sheet.toggle(tr("Micro Height"), &mut edited.height) {
        edit = Some(false);
    }
    if edited.height {
        input_card(app, sheet, (layer, id), GeneratorInputSlot::MicroHeight, shown.height_source.as_ref(), true);
    }
    if sheet.toggle(tr("Micro Normal"), &mut edited.normal) {
        edit = Some(false);
    }
    if edited.normal {
        input_card(app, sheet, (layer, id), GeneratorInputSlot::MicroNormal, shown.normal_source.as_ref(), true);
    }
    if MicroDetails::has_curvature(kind) {
        let name = |kind: MicroCurvatureType| tr(kind.label()).to_string();
        if let (Some(picked), _) = sheet.dropdown(tr("Curvature Type"), edited.curvature_type, &MicroCurvatureType::ALL, name, None) {
            edited.curvature_type = picked;
            edit = Some(false);
        }
        let range = MicroDetails::curvature_intensity_range(kind);
        edit = edit.or(sheet.slider(tr("Curvature Intensity"), &mut edited.curvature_intensity, range, 2));
        edit = edit.or(sheet.slider(tr("Height Details Intensity"), &mut edited.height_intensity, MicroDetails::HEIGHT_INTENSITY_RANGE, 2));
    }
    if MicroDetails::has_occlusion(kind) {
        edit = edit.or(sheet.slider(tr("AO Radius"), &mut edited.ao_radius, 0.0..=1.0, 2));
        edit = edit.or(sheet.slider(tr("AO Depth"), &mut edited.ao_depth, 0.0..=1.0, 2));
    }
    if let Some(merge) = edit {
        app.run("effect.set", merged(json!({"effect": id, "merge": merge}), micro_params(kind, &edited)));
    }
}
