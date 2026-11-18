//! A pattern's properties under its fill source slot, laid out like the reference app's: attributes, resolution, parameters, image inputs

use egui::{RichText, Ui};
use egui_phosphor::regular as ic;
use serde_json::{Value, json};
use texturecraft_engine::core::{
    OutputResolution, Procedural, ProceduralKind, ResolutionMode, TileBlending, TileGenerator, TileInputDistribution, TileShape, TileSizeMode, TileSymmetry,
};

use crate::TextureCraftApp;
use crate::dock::{image_drop, image_menu};
use crate::i18n::{tr, trf};
use crate::pattern_target::PatternTarget;
use crate::resource_panel::{ATTRIBUTE_KEY_WIDTH, NOTE_FONT_SIZE, factor_label, output_size_label, random_seed};
use crate::theme::Tokens;
use crate::tile_edits::TileEdits;
use crate::widgets::{combo, section, slider_edit};

/// The pattern scale slider's range (repeats per projection unit)
const SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.1..=100.0;
const CONTRAST_RANGE: std::ops::RangeInclusive<f32> = 0.0..=8.0;

/// The settings of `pattern`: its attributes, resolution, parameters and, for the Tile Generator, its image inputs
pub(crate) fn pattern_properties(app: &mut TextureCraftApp, ui: &mut Ui, target: &PatternTarget, pattern: &Procedural) {
    let id = |name: &str| format!("pattern_{name}_{}", target.salt);
    section(ui, &id("attributes"), tr("Attributes"), false, |ui| attributes(ui, pattern.kind));
    section(ui, &id("resolution"), tr("Resolution"), false, |ui| resolution(app, ui, target, pattern));
    section(ui, &id("parameters"), tr("Parameters"), true, |ui| {
        seed_row(app, ui, target, pattern);
        match pattern.tile.as_deref() {
            Some(tile) => tile_parameters(app, ui, target, pattern, tile),
            None => pattern_parameters(app, ui, target, pattern),
        }
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tr("Restore defaults")).clicked() {
                    restore_defaults(app, target, pattern);
                }
            });
        });
    });
    if let Some(tile) = pattern.tile.as_deref() {
        section(ui, &id("inputs"), tr("Image inputs"), false, |ui| image_inputs(app, ui, target, pattern, tile));
    }
}

/// Sends `pattern` again (its kind, scale, seed, contrast, balance and invert) with `extra` to its fill
pub(crate) fn send(app: &mut TextureCraftApp, target: &PatternTarget, pattern: &Procedural, extra: Value) {
    let mut params = target.base.clone();
    let fields = json!({
        "procedural": pattern.kind.label(),
        "scale": pattern.scale,
        "seed": pattern.seed,
        "contrast": pattern.contrast,
        "balance": pattern.balance,
        "invert": pattern.invert,
    });
    if let (Some(params), Value::Object(fields), Value::Object(extra)) = (params.as_object_mut(), fields, extra) {
        params.extend(fields);
        params.extend(extra);
    }
    app.run(target.command, params);
}

/// What a pattern makes, its category and its tags (read only)
fn attributes(ui: &mut Ui, kind: ProceduralKind) {
    let tokens = Tokens::get(ui.ctx());
    let description = match kind {
        ProceduralKind::TileGenerator => tr("Shapes laid out in a grid of tiles: rows, offsets, random size, rotation and brightness."),
        _ => tr("A procedural pattern mixing two values."),
    };
    let tags = [tr("pattern"), tr(kind.label()), tr("grayscale")].join(", ");
    for (key, value) in [(tr("Description"), description), (tr("Category"), tr("Patterns")), (tr("Tags"), tags.as_str())] {
        ui.horizontal_top(|ui| {
            ui.add_sized([ATTRIBUTE_KEY_WIDTH, 0.0], egui::Label::new(RichText::new(key).color(tokens.text_dim)));
            ui.add(egui::Label::new(value).wrap());
        });
    }
}

/// What the pattern's resolution is based on, the factor applied to it, and the size it comes to
fn resolution(app: &mut TextureCraftApp, ui: &mut Ui, target: &PatternTarget, pattern: &Procedural) {
    let tokens = Tokens::get(ui.ctx());
    let current = pattern.resolution;
    combo(ui, tr("Resolution"), tr(current.mode.label()), |ui| {
        for mode in ResolutionMode::ALL {
            if ui.selectable_label(mode == current.mode, tr(mode.label())).clicked() {
                send(app, target, pattern, json!({"resolution": format!("{mode:?}")}));
            }
        }
    })
    .on_hover_text(tr("Auto and UV tile use the UV tile's resolution, Texture set the texture set's, Custom a size of its own."));
    if current.mode == ResolutionMode::Custom {
        combo(ui, tr("Size"), format!("{0} × {0}", current.custom), |ui| {
            for size in OutputResolution::CUSTOM_SIZES {
                if ui.selectable_label(size == current.custom, format!("{size} × {size}")).clicked() {
                    send(app, target, pattern, json!({"customResolution": size}));
                }
            }
        });
    }
    combo(ui, tr("Factor"), factor_label(current.factor), |ui| {
        for factor in OutputResolution::FACTORS {
            if ui.selectable_label(factor == current.factor, factor_label(factor)).clicked() {
                send(app, target, pattern, json!({"resolutionFactor": factor}));
            }
        }
    })
    .on_hover_text(tr("Scales the base resolution: below 1 the pattern is drawn coarser and smoothed back up."));

    let Ok(texture_set) = app.session.doc.project.texture_set(app.session.ts()) else { return };
    let size_label = output_size_label(&current, texture_set);
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Output size")).color(tokens.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(size_label));
    });
}

/// The seed with a button that picks a new one and a pencil that opens a field to type it
fn seed_row(app: &mut TextureCraftApp, ui: &mut Ui, target: &PatternTarget, pattern: &Procedural) {
    let typing_id = ui.make_persistent_id(("pattern_seed_typing", &target.salt));
    let mut typing = ui.data(|data| data.get_temp::<bool>(typing_id)).unwrap_or(false);
    ui.horizontal(|ui| {
        ui.label(tr("Seed"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.selectable_label(typing, ic::PENCIL_SIMPLE).on_hover_text(tr("Type the seed")).clicked() {
                typing = !typing;
            }
            if typing {
                let mut typed = pattern.seed;
                let field = ui.add(egui::DragValue::new(&mut typed).speed(1.0));
                if field.changed() {
                    let merge = field.dragged() && !field.drag_started();
                    send(app, target, &Procedural { seed: typed, ..pattern.clone() }, json!({"merge": merge}));
                }
            } else if ui.button(tr("Random")).on_hover_text(trf("Seed {seed}", &[("seed", &pattern.seed)])).clicked() {
                send(app, target, &Procedural { seed: random_seed(pattern.seed), ..pattern.clone() }, json!({}));
            }
        });
    });
    ui.data_mut(|data| data.insert_temp(typing_id, typing));
}

/// Scale, contrast and invert of a pattern other than the Tile Generator
fn pattern_parameters(app: &mut TextureCraftApp, ui: &mut Ui, target: &PatternTarget, pattern: &Procedural) {
    let mut edited = pattern.clone();
    let invert_edit = ui.checkbox(&mut edited.invert, tr("Invert")).changed().then_some(false);
    let scale_edit = slider_edit(ui, tr("Pattern scale"), &mut edited.scale, SCALE_RANGE, 1);
    let contrast_edit = slider_edit(ui, tr("Contrast"), &mut edited.contrast, CONTRAST_RANGE, 2);
    if let Some(merge) = invert_edit.or(scale_edit).or(contrast_edit) {
        send(app, target, &edited, json!({"merge": merge}));
    }
}

/// Puts the pattern's parameters and resolution back to the defaults of its kind; its image inputs stay
fn restore_defaults(app: &mut TextureCraftApp, target: &PatternTarget, pattern: &Procedural) {
    let defaults = Procedural::new(pattern.kind);
    let mut extra = json!({
        "resolution": format!("{:?}", defaults.resolution.mode),
        "customResolution": defaults.resolution.custom,
        "resolutionFactor": defaults.resolution.factor,
    });

    // The images stay: Restore defaults puts back the parameters, not the inputs
    if let Some(tile) = pattern.tile.as_deref() {
        extra["tileDefaults"] = json!(true);
        extra["tile"] = json!({"inputs": tile.inputs, "backgroundImage": tile.background_image});
    }
    send(app, target, &defaults, extra);
}

/// The Tile Generator's settings, in the reference app's sections
fn tile_parameters(app: &mut TextureCraftApp, ui: &mut Ui, target: &PatternTarget, pattern: &Procedural, tile: &TileGenerator) {
    let mut edits = TileEdits::default();
    let mut invert = pattern.invert;
    if ui.checkbox(&mut invert, tr("Invert")).changed() {
        edits.merge = Some(false);
    }
    edits.slider(ui, tr("Position"), "position", tile.position, 0.0..=1.0);
    edits.slider(ui, tr("Contrast"), "contrast", tile.contrast, 0.0..=1.0);
    let mut amount = tile.amount;
    let mut amount_edit = None;
    for (label, count) in [tr("X amount"), tr("Y amount")].into_iter().zip(amount.iter_mut()) {
        let mut shown = *count as f32;
        if let Some(merge) = slider_edit(ui, label, &mut shown, *TileGenerator::AMOUNT.start() as f32..=*TileGenerator::AMOUNT.end() as f32, 0) {
            *count = shown.round() as u32;
            amount_edit = Some(merge);
        }
    }
    if let Some(merge) = amount_edit {
        edits.set("amount", json!(amount), merge);
    }
    edits.check(ui, tr("Non-square expansion"), "nonSquareExpansion", tile.non_square_expansion);

    let id = |name: &str| format!("tile_{name}_{}", target.salt);
    ui.indent(id("sections"), |ui| {
        section(ui, &id("pattern"), tr("Pattern"), false, |ui| shape_section(ui, &mut edits, tile));
        section(ui, &id("size"), tr("Size"), false, |ui| size_section(ui, &mut edits, tile));
        section(ui, &id("position"), tr("Position"), false, |ui| position_section(ui, &mut edits, tile));
        section(ui, &id("rotation"), tr("Rotation"), false, |ui| {
            edits.angle(ui, tr("Rotation"), "rotation", tile.rotation);
            edits.slider(ui, tr("Rotation random"), "rotationRandom", tile.rotation_random, 0.0..=1.0);
        });
        section(ui, &id("color"), tr("Color"), false, |ui| color_section(ui, &mut edits, tile));
    });

    if let Some(merge) = edits.merge {
        let edited = (invert != pattern.invert).then(|| Procedural { invert, ..pattern.clone() });
        send(app, target, edited.as_ref().unwrap_or(pattern), json!({"tile": Value::Object(edits.settings), "merge": merge}));
    }
}

/// The shape each tile draws and how it is turned and mirrored
fn shape_section(ui: &mut Ui, edits: &mut TileEdits, tile: &TileGenerator) {
    edits.choice(ui, tr("Type"), "shape", tile.shape, &TileShape::ALL, TileShape::label);
    for (index, (label, size)) in [tr("Resolution X"), tr("Resolution Y")].into_iter().zip(tile.shape_resolution).enumerate() {
        combo(ui, label, size.to_string(), |ui| {
            for option in TileGenerator::SHAPE_RESOLUTIONS {
                if ui.selectable_label(option == size, option.to_string()).clicked() {
                    let mut resolution = tile.shape_resolution;
                    if let Some(side) = resolution.get_mut(index) {
                        *side = option;
                    }
                    edits.set("shapeResolution", json!(resolution), false);
                }
            }
        });
    }
    edits.slider(ui, tr("Specific"), "specific", tile.specific, 0.0..=1.0);
    edits.angle(ui, tr("Angle"), "shapeAngle", tile.shape_angle);
    edits.slider(ui, tr("Scale"), "shapeScale", tile.shape_scale, 0.0..=1.0);
    edits.pair(ui, [tr("Size X"), tr("Size Y")], "shapeSize", tile.shape_size, 0.0..=1.0);
    edits.count(ui, tr("Tiling"), "shapeTiling", tile.shape_tiling, TileGenerator::SHAPE_TILING);
    edits.check(ui, tr("Rotation 45°"), "shapeDiagonal", tile.shape_diagonal);
    edits.check(ui, tr("Non-square expansion"), "shapeNonSquareExpansion", tile.shape_non_square_expansion);
    edits.choice(ui, tr("Input distribution"), "inputDistribution", tile.input_distribution, &TileInputDistribution::ALL, TileInputDistribution::label);
    combo(ui, tr("Rotation"), format!("{}°", u32::from(tile.quarter_turns) * 90), |ui| {
        for turns in 0..4u8 {
            if ui.selectable_label(turns == tile.quarter_turns, format!("{}°", u32::from(turns) * 90)).clicked() {
                edits.set("quarterTurns", json!(turns), false);
            }
        }
    });
    edits.slider(ui, tr("Rotation random"), "quarterTurnRandom", tile.quarter_turn_random, 0.0..=1.0);
    edits.check(ui, tr("Quincunx flip"), "quincunxFlip", tile.quincunx_flip);
    edits.slider(ui, tr("Symmetry random"), "symmetryRandom", tile.symmetry_random, 0.0..=1.0);
    edits.choice(ui, tr("Symmetry mode"), "symmetry", tile.symmetry, &TileSymmetry::ALL, TileSymmetry::label);
}

/// How big each shape is: the controls of its size mode, then scale and its randomness
fn size_section(ui: &mut Ui, edits: &mut TileEdits, tile: &TileGenerator) {
    edits.choice(ui, tr("Size mode"), "sizeMode", tile.size_mode, &TileSizeMode::ALL, TileSizeMode::label);
    edits.pair(ui, [tr("Middle size horizontal"), tr("Middle size vertical")], "middleSize", tile.middle_size, 0.0..=1.0);
    match tile.size_mode {
        TileSizeMode::NormalInterstice => {
            // Each axis's gap, then its randomness, as the reference lists them
            let (mut gap, mut random) = (tile.interstice, tile.interstice_random);
            let mut gap_edit = None;
            let mut random_edit = None;
            let labels = [(tr("Interstice X"), tr("Interstice X random")), (tr("Interstice Y"), tr("Interstice Y random"))];
            for ((gap_label, random_label), (gap, random)) in labels.into_iter().zip(gap.iter_mut().zip(random.iter_mut())) {
                gap_edit = gap_edit.or(slider_edit(ui, gap_label, gap, 0.0..=1.0, 2));
                random_edit = random_edit.or(slider_edit(ui, random_label, random, 0.0..=1.0, 2));
            }
            if let Some(merge) = gap_edit {
                edits.set("interstice", json!(gap), merge);
            }
            if let Some(merge) = random_edit {
                edits.set("intersticeRandom", json!(random), merge);
            }
        }
        TileSizeMode::NormalSize => edits.pair(ui, [tr("Size X"), tr("Size Y")], "size", tile.size, 0.0..=1.0),

        // The shape is the largest square its cell holds: no size of its own
        TileSizeMode::KeepRatio => {}
        TileSizeMode::Absolute => edits.pair(ui, [tr("Size X"), tr("Size Y")], "sizeAbsolute", tile.size_absolute, 0.0..=1.0),
        TileSizeMode::Pixel => edits.pair(ui, [tr("Size X"), tr("Size Y")], "sizePixel", tile.size_pixel, 0.0..=TileGenerator::MAX_SIZE_PIXELS),
    }
    if matches!(tile.size_mode, TileSizeMode::Absolute | TileSizeMode::Pixel) {
        edits.pair(ui, [tr("Size random X"), tr("Size random Y")], "sizeRandom", tile.size_random, 0.0..=1.0);
    }
    edits.slider(ui, tr("Scale"), "scale", tile.scale, 0.0..=TileGenerator::MAX_SCALE);
    edits.slider(ui, tr("Scale random"), "scaleRandom", tile.scale_random, 0.0..=1.0);
    edits.count(ui, tr("Scale random seed"), "scaleRandomSeed", tile.scale_random_seed, 0..=TileGenerator::MAX_RANDOM_SEED);
}

/// Where each shape sits: the row offset, its randomness and the shift of the whole grid
fn position_section(ui: &mut Ui, edits: &mut TileEdits, tile: &TileGenerator) {
    edits.slider(ui, tr("Offset"), "offset", tile.offset, 0.0..=1.0);
    edits.slider(ui, tr("Offset random"), "offsetRandom", tile.offset_random, 0.0..=1.0);
    edits.count(ui, tr("Offset random seed"), "offsetRandomSeed", tile.offset_random_seed, 0..=TileGenerator::MAX_RANDOM_SEED);
    edits.check(ui, tr("Vertical offset"), "verticalOffset", tile.vertical_offset);
    edits.pair(ui, [tr("Position random X"), tr("Position random Y")], "positionRandom", tile.position_random, 0.0..=1.0);
    edits.pair(ui, [tr("Global offset X"), tr("Global offset Y")], "globalOffset", tile.global_offset, 0.0..=1.0);
}

/// Each shape's brightness, the tiles hidden and how overlapping shapes combine
fn color_section(ui: &mut Ui, edits: &mut TileEdits, tile: &TileGenerator) {
    edits.gray(ui, tr("Color"), "color", tile.color);
    edits.slider(ui, tr("Luminance random"), "luminanceRandom", tile.luminance_random, 0.0..=1.0);
    edits.check(ui, tr("Luminance by number"), "luminanceByNumber", tile.luminance_by_number);
    edits.check(ui, tr("Luminance by scale"), "luminanceByScale", tile.luminance_by_scale);
    edits.check(ui, tr("Checker mask"), "checkerMask", tile.checker_mask);
    edits.check(ui, tr("Horizontal mask"), "horizontalMask", tile.horizontal_mask);
    edits.check(ui, tr("Vertical mask"), "verticalMask", tile.vertical_mask);
    edits.slider(ui, tr("Random mask"), "randomMask", tile.random_mask, 0.0..=1.0);
    edits.check(ui, tr("Invert mask"), "invertMask", tile.invert_mask);
    edits.choice(ui, tr("Blending"), "blending", tile.blending, &TileBlending::ALL, TileBlending::label);
    edits.gray(ui, tr("Background"), "background", tile.background);
    edits.slider(ui, tr("Global opacity"), "opacity", tile.opacity, 0.0..=1.0);
    edits.check(ui, tr("Reverse rendering order"), "reverseOrder", tile.reverse_order);
}

/// The images the Tile Generator can draw in its tiles, and the one under them
fn image_inputs(app: &mut TextureCraftApp, ui: &mut Ui, target: &PatternTarget, pattern: &Procedural, tile: &TileGenerator) {
    let tokens = Tokens::get(ui.ctx());
    let mut inputs = tile.inputs.clone();
    let mut changed = false;
    for index in 0..TileGenerator::MAX_INPUTS {
        let current = inputs.get(index).cloned();
        let label = trf("Pattern input {number}", &[("number", &(index + 1))]);
        if let Some(picked) = input_slot(app, ui, &label, current.as_deref()) {
            match (picked, inputs.get_mut(index)) {
                (Some(name), Some(slot)) => *slot = name,
                (Some(name), None) => inputs.push(name),
                (None, Some(_)) => {
                    inputs.remove(index);
                }
                (None, None) => {}
            }
            changed = true;
        }

        // Inputs fill in order: one empty slot after the last image
        if current.is_none() {
            break;
        }
    }
    if changed {
        send(app, target, pattern, json!({"tile": {"inputs": inputs}}));
    }
    if let Some(picked) = input_slot(app, ui, tr("Background"), tile.background_image.as_deref()) {
        send(app, target, pattern, json!({"tile": {"backgroundImage": picked}}));
    }
    if tile.shape != TileShape::Image {
        ui.label(RichText::new(tr("The pattern inputs are drawn when the shape type is Custom image input.")).size(NOTE_FONT_SIZE).color(tokens.text_dim));
    }
}

/// One image input: its name and image, a menu to pick another and a button that empties it. `Some(pick)` when changed (`None`: emptied)
fn input_slot(app: &mut TextureCraftApp, ui: &mut Ui, label: &str, current: Option<&str>) -> Option<Option<String>> {
    let tokens = Tokens::get(ui.ctx());
    let mut picked = None;
    let group = ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(label).strong());
                ui.label(RichText::new(current.unwrap_or(tr("Empty"))).size(NOTE_FONT_SIZE).color(tokens.text_dim));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if current.is_some() && ui.button(ic::X).on_hover_text(tr("Empty this input")).clicked() {
                    picked = Some(None);
                }
                ui.menu_button(ic::IMAGE, |ui| {
                    if let Some(name) = image_menu(app, ui) {
                        picked = Some(Some(name));
                    }
                })
                .response
                .on_hover_text(tr("Pick an image"));
            });
        });
    });
    if let Some(name) = image_drop(ui, &group.response) {
        picked = Some(Some(name));
    }
    picked
}
