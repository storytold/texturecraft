//! The Parameters rows of the kinds with only their own controls: 3D Distance, 3D Linear Gradient, UV Border Distance, Grayscale Conversion, Dripping Rust, UV Random Color and UV Checker

use egui::{Painter, Rect};
use serde_json::{Value, json};
use texturecraft_engine::core::{
    BorderDistanceParameters, DistanceParameters, DrippingRustParameters, Generator, GeneratorGrayscale, GrayscaleConversionParameters, KindControls,
    LinearGradientParameters, UvCheckerParameters, UvRandomColorParameters,
};

use crate::TextureCraftApp;
use crate::fill_material_panel::take_color_pick;
use crate::generator_params::{DRIPS_AXES, kind_params, merged};
use crate::generator_point_pick::GeneratorPointPick;
use crate::i18n::{tr, trf};
use crate::property_sheet::{NumberField, PropertySheet, StackSlider};
use crate::theme::AXIS_COLORS;

/// The first edit of a stack beside the preview
fn first_edit<const N: usize>(edits: [Option<bool>; N]) -> Option<bool> {
    edits.into_iter().flatten().next()
}

/// The rows of generator effect `id`, whose own controls are `controls`; `picture` draws the preview
pub(crate) fn kind_rows(
    app: &mut TextureCraftApp,
    sheet: &mut PropertySheet<'_>,
    id: u64,
    generator: &Generator,
    controls: KindControls,
    picture: &dyn Fn(&Painter, Rect),
) {
    if generator.kind.includes_randomness() {
        let mut seed = generator.seed;
        if sheet.seed(tr("Seed"), &mut seed) {
            app.run("effect.set", json!({"effect": id, "seed": seed}));
        }
    }

    // UV Random Color and UV Checker make colors: Invert is the reference's on the grayscale kinds only
    if !matches!(controls, KindControls::UvRandomColor(_) | KindControls::UvChecker(_)) {
        let mut invert = generator.invert;
        if sheet.toggle(tr("Invert"), &mut invert) {
            app.run("effect.set", json!({"effect": id, "invert": invert}));
        }
    }

    let edit = match controls {
        KindControls::Distance(distance) => distance_rows(sheet, id, distance, picture),
        KindControls::LinearGradient(gradient) => linear_gradient_rows(sheet, id, gradient, picture),
        KindControls::BorderDistance(border) => border_rows(sheet, border, picture),
        KindControls::Grayscale(grayscale) => grayscale_rows(sheet, grayscale, picture),
        KindControls::DrippingRust(rust) => rust_rows(sheet, rust, picture),
        KindControls::UvRandomColor(random) => random_color_rows(app, sheet, id, random),
        KindControls::UvChecker(checker) => checker_rows(app, sheet, checker, picture),
    };
    if let Some((merge, edited)) = edit {
        app.run("effect.set", merged(json!({"effect": id, "merge": merge}), edited));
    }
}

/// An edit: whether it continues the last undo step, and the controls to send
type KindEdit = Option<(bool, Value)>;

/// The controls `controls` to send when `merge` says one was edited
fn sent(merge: Option<bool>, controls: KindControls) -> KindEdit {
    merge.map(|merge| (merge, kind_params(&controls)))
}

/// A point's X, Y and Z (across the mesh's evenly scaled bounds) with its pick-on-mesh button; returns its edit
fn point_row(sheet: &mut PropertySheet<'_>, label: &str, point: &mut [f32; 3], pick: GeneratorPointPick) -> Option<bool> {
    let mut fields: Vec<NumberField> = ["X", "Y", "Z"]
        .into_iter()
        .zip(AXIS_COLORS)
        .zip(point.iter())
        .map(|((tag, color), value)| NumberField { tag, tag_color: Some(color), value: *value, speed: POINT_DRAG_SPEED, decimals: 2, suffix: "" })
        .collect();
    let ctx = sheet.ui.ctx().clone();
    let armed = crate::generator_point_pick::armed(&ctx) == Some(pick);
    let (edited, clicked) = sheet.point_fields(label, &mut fields, armed);
    if clicked {
        crate::generator_point_pick::toggle(&ctx, pick);
    }
    let (index, merge) = edited?;
    let (field, coordinate) = (fields.get(index)?, point.get_mut(index)?);
    *coordinate = field.value.clamp(0.0, 1.0);
    Some(merge)
}

/// Change of a point's coordinate per pixel dragged
const POINT_DRAG_SPEED: f32 = 0.005;

/// 3D Distance: Radius, Offset and Contrast beside the preview, then its point
fn distance_rows(sheet: &mut PropertySheet<'_>, id: u64, shown: DistanceParameters, picture: &dyn Fn(&Painter, Rect)) -> KindEdit {
    let mut edited = shown;
    let radius = StackSlider {
        label: tr("Radius"),
        value: &mut edited.radius,
        range: DistanceParameters::RADIUS_RANGE,
        typed: DistanceParameters::RADIUS_RANGE,
        decimals: 2,
        suffix: "",
        dial: false,
    };
    let mut stack = [radius, StackSlider::unit(tr("Offset"), &mut edited.offset), StackSlider::unit(tr("Contrast"), &mut edited.contrast)];
    let mut edit = first_edit(sheet.picture_sliders(picture, &mut stack));
    edit = edit.or(point_row(sheet, tr("3D Position"), &mut edited.point, GeneratorPointPick { effect: id, point: "distance" }));
    sent(edit, KindControls::Distance(edited))
}

/// 3D Linear Gradient: Balance and Contrast beside the preview, then its start and end
fn linear_gradient_rows(sheet: &mut PropertySheet<'_>, id: u64, shown: LinearGradientParameters, picture: &dyn Fn(&Painter, Rect)) -> KindEdit {
    let mut edited = shown;
    let mut stack = [StackSlider::unit(tr("Balance"), &mut edited.balance), StackSlider::unit(tr("Contrast"), &mut edited.contrast)];
    let mut edit = first_edit(sheet.picture_sliders(picture, &mut stack));
    edit = edit.or(point_row(sheet, tr("3D Position Start"), &mut edited.start, GeneratorPointPick { effect: id, point: "start" }));
    edit = edit.or(point_row(sheet, tr("3D Position End"), &mut edited.end, GeneratorPointPick { effect: id, point: "end" }));
    sent(edit, KindControls::LinearGradient(edited))
}

/// UV Border Distance: Distance, Smoothness and Balance beside the preview, then Contrast
fn border_rows(sheet: &mut PropertySheet<'_>, shown: BorderDistanceParameters, picture: &dyn Fn(&Painter, Rect)) -> KindEdit {
    let mut edited = shown;
    let mut stack = [
        StackSlider::unit(tr("Distance"), &mut edited.distance),
        StackSlider::unit(tr("Smoothness"), &mut edited.smoothness),
        StackSlider::unit(tr("Balance"), &mut edited.balance),
    ];
    let mut edit = first_edit(sheet.picture_sliders(picture, &mut stack));
    edit = edit.or(sheet.slider(tr("Contrast"), &mut edited.contrast, 0.0..=1.0, 2));
    sent(edit, KindControls::BorderDistance(edited))
}

/// Grayscale Conversion: its type, Balance, Contrast and Tile beside the preview, then Rotation and Safe Rotation (its image is in Image inputs)
fn grayscale_rows(sheet: &mut PropertySheet<'_>, shown: GrayscaleConversionParameters, picture: &dyn Fn(&Painter, Rect)) -> KindEdit {
    let mut edited = shown;
    let mut edit = None;
    let name = |grayscale: GeneratorGrayscale| tr(grayscale.label()).to_string();
    if let (Some(picked), _) = sheet.dropdown(tr("Grayscale Type"), edited.grayscale, &GeneratorGrayscale::ALL, name, None) {
        edited.grayscale = picked;
        edit = Some(false);
    }
    let mut stack = [
        StackSlider::unit(tr("Balance"), &mut edited.balance),
        StackSlider::unit(tr("Contrast"), &mut edited.contrast),
        StackSlider::whole(tr("Tile"), &mut edited.tile, GrayscaleConversionParameters::TILE_RANGE),
    ];
    edit = edit.or(first_edit(sheet.picture_sliders(picture, &mut stack)));
    edited.tile = edited.tile.round();
    edit = edit.or(sheet.angle(tr("Rotation"), &mut edited.rotation, 0.0..=360.0));
    if sheet.toggle(tr("Safe Rotation"), &mut edited.safe_rotation) {
        edit = Some(false);
    }
    sent(edit, KindControls::Grayscale(edited))
}

/// The rust coverage and specks sit beside the preview, with streak controls below
fn rust_rows(sheet: &mut PropertySheet<'_>, shown: DrippingRustParameters, picture: &dyn Fn(&Painter, Rect)) -> KindEdit {
    let mut edited = shown;
    let mut stack = [
        StackSlider::unit(tr("Rust Coverage"), &mut edited.spreading),
        StackSlider::unit(tr("Speck Definition"), &mut edited.contrast),
        StackSlider::unit(tr("Speck Softness"), &mut edited.spreading_smoothness),
    ];
    let mut edit = first_edit(sheet.picture_sliders(picture, &mut stack));
    edit = edit.or(sheet.slider(tr("Streak Reach"), &mut edited.drips_intensity, 0.0..=1.0, 2));
    edit = edit.or(sheet.slider(tr("Streak Softness"), &mut edited.drips_smoothness, 0.0..=1.0, 2));
    let mut samples = edited.drips_samples as f32;
    if let Some(merge) = sheet.slider(tr("Samples per Streak"), &mut samples, 0.0..=DrippingRustParameters::MAX_SAMPLES as f32, 0) {
        edited.drips_samples = samples.round() as u32;
        edit = edit.or(Some(merge));
    }
    let axes = [tr("X"), tr("Y"), tr("Z")];
    let name = |axis: u8| axes.get(usize::from(axis)).copied().unwrap_or_default().to_string();
    if let (Some(picked), _) = sheet.dropdown(tr("Flow Axis"), edited.axis, &[0, 1, 2], name, None) {
        edited.axis = picked;
        edit = Some(false);
    }
    debug_assert_eq!(DRIPS_AXES.len(), axes.len());
    sent(edit, KindControls::DrippingRust(edited))
}

/// UV Random Color: where its colors come from; a custom gradient shows the gradient editor (as the Gradient filter's)
fn random_color_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, shown: UvRandomColorParameters) -> KindEdit {
    let mut edited = shown.clone();
    let mode = usize::from(edited.custom_gradient);
    if let Some(picked) = sheet.switch(tr("Color Source Mode"), &[tr("Random"), tr("Custom Gradient")], mode) {
        edited.custom_gradient = picked == 1;
        return sent(Some(false), KindControls::UvRandomColor(edited));
    }

    if !edited.custom_gradient {
        return None;
    }

    let used = shown.used_colors().to_vec();
    let mut places = shown.used_positions();
    if let Some(drag) = sheet.gradient_ramp(&used, &places, &|t| shown.at(t).unwrap_or([0.0; 3])) {
        if let Some(place) = places.get_mut(drag.index) {
            *place = drag.position;
        }
        return Some((drag.merge, gradient_places(&shown, &places)));
    }

    let mut quantity = used.len() as f32;
    let range = f32::from(UvRandomColorParameters::MIN_QUANTITY)..=UvRandomColorParameters::MAX_GRADIENT_COLORS as f32;
    let slid = sheet.slider(tr("Color Quantity"), &mut quantity, range, 0).is_some();

    // A press's first whole-number change starts its undo step: changes that round to the count it has apply nothing
    let press = sheet.ui.make_persistent_id(("uv_random_color_quantity", id));
    let quantity = quantity.round();
    if let Some(merge) = crate::widgets::press_edit(sheet.ui, press, slid && quantity as usize != used.len()) {
        return Some((merge, json!({"gradientQuantity": quantity})));
    }

    let mut edit = None;
    for (index, color) in used.iter().enumerate() {
        let label = trf("Color {number}", &[("number", &(index + 1))]);
        let mut place = places.get(index).copied().unwrap_or(0.0);
        let (pick, moved) = sheet.color_stop(&label, *color, &mut place, app.picking_color, &app.session.doc.project.swatches);
        if let Some(picked) = take_color_pick(app, pick) {
            if let Some(slot) = edited.gradient.get_mut(index) {
                *slot = picked;
            }
            edit = Some((pick.merge, gradient_colors(&edited)));
        }
        if let Some(merge) = moved {
            if let Some(slot) = places.get_mut(index) {
                *slot = place;
            }
            edit = Some((merge, gradient_places(&shown, &places)));
        }
    }
    edit
}

/// The colors of `random` to send: all of them, then its count again (the colors reset it)
fn gradient_colors(random: &UvRandomColorParameters) -> Value {
    let mut params = json!({"gradientColors": random.gradient});
    if let Some(quantity) = random.quantity {
        params["gradientQuantity"] = json!(quantity);
    }
    params
}

/// The places to send: `used` (the used colors' places) and the hidden colors' own
fn gradient_places(random: &UvRandomColorParameters, used: &[f32]) -> Value {
    let hidden = random.positions.iter().skip(used.len()).copied();
    json!({"gradientPositions": used.iter().copied().chain(hidden).collect::<Vec<f32>>()})
}

/// UV Checker: its color mode with the uniform color, Checker Grid Tile, Checker Grid Opacity and UV Borders Width beside the preview, then UV Borders Color
fn checker_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, shown: UvCheckerParameters, picture: &dyn Fn(&Painter, Rect)) -> KindEdit {
    let mut edited = shown;
    let mut edit = None;
    if let Some(picked) = sheet.switch(tr("Checker Color Mode"), &[tr("Uniform"), tr("Random")], usize::from(edited.random_colors)) {
        edited.random_colors = picked == 1;
        edit = Some(false);
    }
    if !edited.random_colors {
        let pick = sheet.color_bar_led(tr("Checker Color"), edited.checker_color, app.picking_color, &app.session.doc.project.swatches);
        if let Some(picked) = take_color_pick(app, pick) {
            edited.checker_color = picked;
            edit = Some(pick.merge);
        }
    }

    let mut tile = edited.grid_tile as f32;
    let tiles = *UvCheckerParameters::GRID_TILE_RANGE.start() as f32..=*UvCheckerParameters::GRID_TILE_RANGE.end() as f32;
    let mut stack = [
        StackSlider::whole(tr("Checker Grid Tile"), &mut tile, tiles),
        StackSlider::unit(tr("Checker Grid Opacity"), &mut edited.grid_opacity),
        StackSlider::unit(tr("UV Borders Width"), &mut edited.border_width),
    ];
    edit = edit.or(first_edit(sheet.picture_sliders(picture, &mut stack)));
    edited.grid_tile = tile.round() as u32;

    let pick = sheet.color_bar_led(tr("UV Borders Color"), edited.border_color, app.picking_color, &app.session.doc.project.swatches);
    if let Some(picked) = take_color_pick(app, pick) {
        edited.border_color = picked;
        edit = Some(pick.merge);
    }
    sent(edit, KindControls::UvChecker(edited))
}
