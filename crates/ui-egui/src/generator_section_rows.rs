//! The Parameters rows of Curvature, Ambient Occlusion, Position and World Space Normals: the global block, the kind's own card, its axes, Use Texture; and the earlier model's ramp

use egui::{Painter, Rect};
use serde_json::json;
use texturecraft_engine::core::{
    AxisBlend, CurvatureDetail, CurvatureMode, Generator, GeneratorGrayscale, GeneratorKind, LayerId, SectionParameters, TextureBlend, TextureLayer,
};

use crate::TextureCraftApp;
use crate::generator_input_card::input_card;
use crate::generator_input_slot::GeneratorInputSlot;
use crate::generator_params::{merged, section_params, texture_params};
use crate::i18n::tr;
use crate::property_sheet::{PropertySheet, StackSlider};
use crate::theme::AXIS_COLORS;

/// Global Invert, then the preview beside Global Blur, Balance and Contrast: what every generator applies to its finished value
pub(crate) fn global_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, generator: &Generator, picture: &dyn Fn(&Painter, Rect)) {
    let (mut invert, mut global) = (generator.invert, generator.global);
    let invert_edit = sheet.toggle(tr("Global Invert"), &mut invert).then_some(false);
    let mut stack = [
        StackSlider::unit(tr("Global Blur"), &mut global.blur),
        StackSlider::unit(tr("Global Balance"), &mut global.balance),
        StackSlider::unit(tr("Global Contrast"), &mut global.contrast),
    ];
    let stack_edit = sheet.picture_sliders(picture, &mut stack).into_iter().flatten().next();
    if let Some(merge) = invert_edit.or(stack_edit) {
        app.run(
            "effect.set",
            json!({"effect": id, "invert": invert, "blur": global.blur, "balance": global.balance, "contrast": global.contrast, "merge": merge}),
        );
    }
}

/// The kind's own card (Curvature's mode and sizes in it), then one card per axis, showing `shown`; `solo` keeps the Curvature size under the pointer for the preview
pub(crate) fn section_rows(
    app: &mut TextureCraftApp,
    sheet: &mut PropertySheet<'_>,
    id: u64,
    generator: &Generator,
    shown: SectionParameters,
    solo: &mut Option<usize>,
) {
    let kind = generator.kind;
    let title = match kind {
        GeneratorKind::Curvature => tr("Curvature"),
        GeneratorKind::AmbientOcclusion => tr("Ambient Occlusion"),
        GeneratorKind::Position => tr("Position Gradient"),
        _ => tr("World Space Normal"),
    };
    let mut edited = shown;
    let mut edit = None;
    if sheet.card(title, Some(true)) {
        edit = sheet.toggle(tr("Invert"), &mut edited.invert).then_some(false);
        if kind == GeneratorKind::Curvature {
            *solo = curvature_detail_rows(app, sheet, id, generator.detail.as_ref());
        }
        if SectionParameters::has_blur(kind) {
            edit = edit.or(sheet.slider(tr("Blur"), &mut edited.blur, SectionParameters::BLUR_RANGE, 2));
        }
        if SectionParameters::has_balance(kind) {
            edit = edit.or(sheet.slider(tr("Balance"), &mut edited.balance, 0.0..=1.0, 2));
        }
        edit = edit.or(sheet.slider(tr("Contrast"), &mut edited.contrast, 0.0..=1.0, 2));
        if SectionParameters::has_brightness(kind) {
            edit = edit.or(sheet.signed_slider(tr("Brightness"), &mut edited.brightness, SectionParameters::BRIGHTNESS_RANGE, 2));
        }
    }
    sheet.close_card();

    if SectionParameters::has_axes(kind) {
        let axes = [("X", tr("Right to Left")), ("Y", tr("Top To Bottom")), ("Z", tr("Front to Back"))];
        for ((axis, (letter, name)), tint) in edited.axes.iter_mut().zip(axes).zip(AXIS_COLORS) {
            edit = edit.or(sheet.axis_card((letter, tint), name, &mut axis.weight, 0.0..=1.0, 2));
            edit = edit.or(sheet.toggle(tr("Invert"), &mut axis.invert).then_some(false));
            if let (Some(picked), _) = sheet.dropdown(tr("Blending Mode"), axis.blend, &AxisBlend::ALL, |blend| axis_blend_label(blend).to_string(), None) {
                axis.blend = picked;
                edit = Some(false);
            }
            sheet.close_card();
        }
    }
    if let Some(merge) = edit {
        app.run("effect.set", merged(json!({"effect": id, "merge": merge}), section_params(kind, &edited)));
    }
}

/// Curvature's Mode, each mode pictured, and its detail sizes as columns (`detail` None: saved before detail sizes); returns the size under the pointer
fn curvature_detail_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, detail: Option<&CurvatureDetail>) -> Option<usize> {
    let shown = detail.cloned().unwrap_or_default();
    let ctx = sheet.ui.ctx().clone();
    let pictures: Vec<_> = CurvatureMode::ALL
        .into_iter()
        .map(|mode| (mode, app.thumbs.generator_picture(&ctx, crate::shelf_preview::ShelfPreview::Curvature { mode, solo: None }, MODE_PICTURE_PX)))
        .collect();
    let draw = |painter: &Painter, rect: Rect, mode: CurvatureMode| {
        if let Some((_, Some(texture))) = pictures.iter().find(|(shown, _)| *shown == mode) {
            painter.image(texture.id(), rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
    };
    let name = |mode: CurvatureMode| curvature_mode_label(mode).to_string();
    if let (Some(mode), _) = sheet.dropdown(tr("Mode"), shown.mode, &CurvatureMode::ALL, name, Some(&draw)) {
        app.run("effect.set", json!({"effect": id, "mode": format!("{mode:?}")}));
    }

    let names = [tr("Sharp"), tr("Fine"), tr("Soft"), tr("Medium"), tr("Large"), tr("Big"), tr("Huge")];
    let mut weights = shown.weights;
    let columns = sheet.size_columns(&names, &mut weights);
    if let Some((_, merge)) = columns.edit {
        app.run("effect.set", json!({"effect": id, "sizes": weights, "merge": merge}));
    }
    columns.hovered
}

/// The width a Mode picture is rendered at, in pixels: sharp at twice the dropdown's picture
const MODE_PICTURE_PX: usize = 48;

/// Use Texture's switch card: while on, the texture's slot and controls
pub(crate) fn texture_card(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, (layer, id): (LayerId, u64), shown: TextureLayer) {
    let mut edited = shown.clone();
    let mut edit = sheet.switch_card(tr("Use Texture"), &mut edited.enabled).then_some(false);
    if edited.enabled {
        input_card(app, sheet, (layer, id), GeneratorInputSlot::Texture, shown.source.as_ref(), true);
        edit = edit.or(sheet.toggle(tr("Invert"), &mut edited.invert).then_some(false));
        let grayscale = |mode: GeneratorGrayscale| tr(mode.label()).to_string();
        if let (Some(picked), _) = sheet.dropdown(tr("Grayscale Conversion"), edited.grayscale, &GeneratorGrayscale::ALL, grayscale, None) {
            edited.grayscale = picked;
            edit = Some(false);
        }
        let blend = |blend: TextureBlend| tr(blend.label()).to_string();
        if let (Some(picked), _) = sheet.dropdown(tr("Blending Mode"), edited.blend, &TextureBlend::ALL, blend, None) {
            edited.blend = picked;
            edit = Some(false);
        }
        edit = edit.or(sheet.slider(tr("Scale"), &mut edited.scale, TextureLayer::SCALE_RANGE, 2));
        edit = edit.or(sheet.signed_slider(tr("Contrast"), &mut edited.contrast, -1.0..=1.0, 2));
        edit = edit.or(sheet.signed_slider(tr("Brightness"), &mut edited.brightness, -1.0..=1.0, 2));
        edit = edit.or(sheet.toggle(tr("Triplanar"), &mut edited.triplanar).then_some(false));
        if edited.triplanar {
            edit = edit.or(sheet.slider(tr("Triplanar Contrast"), &mut edited.triplanar_contrast, TextureLayer::TRIPLANAR_CONTRAST_RANGE, 2));
        }
        edit = edit.or(sheet.slider(tr("Texture Opacity"), &mut edited.opacity, 0.0..=1.0, 2));
    }
    sheet.close_card();
    if let Some(merge) = edit {
        app.run("effect.set", merged(json!({"effect": id, "merge": merge}), texture_params(&edited)));
    }
}

/// The earlier model's ramp: where it starts, how soft it is, the noise breaking it up, its offset and its seed
pub(crate) fn ramp_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, generator: &Generator) {
    let (mut level, mut softness, mut noise, mut noise_scale, mut offset) =
        (generator.level, generator.softness, generator.noise_amount, generator.noise_scale, generator.offset);
    let mut edit = sheet.slider(tr("Level"), &mut level, 0.0..=1.0, 2);
    edit = edit.or(sheet.slider(tr("Softness"), &mut softness, 0.0..=1.0, 2));
    edit = edit.or(sheet.slider(tr("Noise Amount"), &mut noise, 0.0..=Generator::MAX_NOISE_AMOUNT, 2));
    edit = edit.or(sheet.slider(tr("Noise Scale"), &mut noise_scale, EARLIER_NOISE_SCALE, 1));
    edit = edit.or(sheet.signed_slider(tr("Offset"), &mut offset, -1.0..=1.0, 2));
    if let Some(merge) = edit {
        app.run(
            "effect.set",
            json!({"effect": id, "level": level, "softness": softness, "noise": noise, "noiseScale": noise_scale, "offset": offset, "merge": merge}),
        );
    }
    let mut seed = generator.seed;
    if sheet.seed(tr("Seed"), &mut seed) {
        app.run("effect.set", json!({"effect": id, "seed": seed}));
    }
}

/// What the earlier model's Noise Scale slider spans: the noise sizes it was tuned over
const EARLIER_NOISE_SCALE: std::ops::RangeInclusive<f32> = 0.1..=64.0;

/// The translated name of axis blending mode `blend`
fn axis_blend_label(blend: AxisBlend) -> &'static str {
    match blend {
        AxisBlend::Normal => tr("Normal"),
        AxisBlend::Add => tr("Linear Dodge (Add)"),
        AxisBlend::Subtract => tr("Subtract"),
        AxisBlend::Multiply => tr("Multiply"),
        AxisBlend::AddSub => tr("Add/Sub"),
        AxisBlend::Max => tr("Max (Lighten)"),
    }
}

/// The translated name of curvature mode `mode`
fn curvature_mode_label(mode: CurvatureMode) -> &'static str {
    match mode {
        CurvatureMode::Edges => tr("Edges"),
        CurvatureMode::Cavities => tr("Cavities"),
        CurvatureMode::Both => tr("Dual"),
        CurvatureMode::Raw => tr("Unprocessed"),
    }
}
