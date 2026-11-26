//! The Filter effect's properties: FILTER over the channel pills (content), the filter card holding its parameters and Attributes, then its image inputs' cards

use std::ops::RangeInclusive;

use egui::{Painter, Rect, Stroke, StrokeKind, Ui};
use serde_json::{Value, json};
use texturecraft_engine::core::{
    AnisotropicKuwahara, Bevel, BevelBlur, BevelCorner, BevelDirection, BevelSmooth, Channel, ColorMatch, ConversionMode, CurvePreset, DirectionalLight,
    EffectKind, Filter, GrayscaleConversion, GrayscaleMode, HeightToNormal, HighpassSpace, ImageInput, LayerId, LightingOutput, MaskOutline, Mirror,
    MirrorCorner, MirrorMode, OutlinePosition, PbrValidate, SlopeBlending, SourceColorMode, SourceSlopeBlur, SourceWarp, StylizedLighting, TargetColorMode,
    ThresholdMode, ValidationMode, WarpBlending, WarpMode, WarpSourceMode,
};
use texturecraft_engine::filter_lib::{
    BEVEL_SMOOTHING_MAX, BLUR_INTENSITY_SLIDER_MAX, DISTANCE_BEVEL_SMOOTHING_MAX, FilterDef, GRADIENT_QUANTITY_MIN, MAX_CAVITY_SMOOTHNESS,
    MAX_DIELECTRIC_REFLECTANCE, MAX_HEIGHT_TO_NORMAL, MAX_LIGHT_INTENSITY, SKY_INTENSITY_SLIDER_MAX, SMOOTH_DISTANCE_MAX, SOURCE_TILING_MAX, SOURCE_TILING_MIN,
    SUN_INTENSITY_MAX, build, definition, scalar_params,
};
use texturecraft_engine::paint::FULL_TURN_DEGREES;

use crate::TextureCraftApp;
use crate::fill_material_panel::pill_name;
use crate::i18n::{tr, trf};
use crate::property_sheet::PropertySheet;
use crate::theme::{AXIS_COLORS, OPPOSITE_COLORS, Tokens};

/// Where TextureCraft's filters come from: the repository, without its scheme
const AUTHOR_URL: &str = "github.com/storytold/texturecraft";

/// What a filter's angle rows span, in degrees: one full turn of their dial
pub(crate) const DIAL_DEGREES: RangeInclusive<f32> = 0.0..=FULL_TURN_DEGREES;

/// The decimals Height To Normal's Normal Intensity shows
const INTENSITY_DECIMALS: usize = 2;

/// The widest Divider chip text: the chip keeps one width whatever it shows
const WIDEST_DIVIDER: &str = "÷1000";

/// The properties of filter effect `effect` (`filter`), with its channel pills when it sits on the layer content
pub(crate) fn filter_properties(app: &mut TextureCraftApp, ui: &mut Ui, effect: LayerId, filter: &Filter) {
    let on_mask = crate::filter_slot::is_on_mask(app, effect.0);
    PropertySheet::show(ui, ("filter_sheet", effect.0), |sheet| {
        sheet.section(tr("Filter"), false);
        if !on_mask {
            let off: Vec<Channel> = app.session.doc.project.effect(effect).map(|(_, found)| found.off_channels.iter().copied().collect()).unwrap_or_default();
            channel_pills(app, sheet, effect, &off);
        }
        let def = definition(filter);
        let defaults = def.and_then(defaults_of);
        let changed = defaults.as_ref().is_some_and(|defaults| defaults != filter);
        if filter_card(app, sheet, effect, filter, def, changed) {
            app.run("effect.filter.restoreDefaults", json!({"effect": effect.0}));
        }
        if *filter == Filter::Empty {
            return;
        }

        // The card holds the filter's settings, as a material channel's card does: sub-groups are captions in it, Attributes its last folded row
        sheet.slot_rule();
        sheet.hold_card();
        parameters(app, sheet, effect.0, filter);
        if sheet.closing_fold(tr("Attributes")) {
            attributes(app, sheet, def);
        }
        sheet.release_card();

        // Inputs every mode reads: their own slot cards after the filter's
        for input in ImageInput::of(filter).into_iter().filter(|input| !input.by_mode) {
            input_slot(app, sheet, effect, input, false);
        }
        if let Filter::GradientDynamic { color, .. } = filter {
            gradient_source(app, sheet, effect.0, *color);
        }
    });
}

/// The filter's read-only Attributes (library entry `def`): what it does, who made it, the words it is found by and the size it works at
fn attributes(app: &TextureCraftApp, sheet: &mut PropertySheet<'_>, def: Option<&FilterDef>) {
    let about = def.and_then(|def| def.about());
    if let Some(about) = about {
        sheet.attribute_text(tr("Description"), tr(about.description));
    }
    sheet.attribute(tr("Category"), tr("Filter"));
    sheet.attribute(tr("Author"), "TextureCraft");
    sheet.attribute(tr("Author URL"), AUTHOR_URL);
    if let Some(about) = about {
        sheet.attribute(tr("Tags"), &about.tags.iter().map(|tag| tr(tag)).collect::<Vec<_>>().join(", "));
    }
    if let Ok(texture_set) = app.session.doc.project.texture_set(app.session.ts()) {
        sheet.attribute(tr("Size"), &format!("{0} × {0}", texture_set.resolution));
    }
}

/// The slot card of image input `input` of filter effect `effect` (its row only, inside the card open now, when `in_card`): the source it reads, picked like a mask fill's
fn input_slot(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, effect: LayerId, input: ImageInput, in_card: bool) {
    let Some((reader, source)) = app.session.doc.project.effect(effect).map(|(owner, found)| (owner, found.input(&input))) else { return };
    crate::fill_material_panel::source_card(app, sheet, source, &json!({"effect": effect.0}), crate::source_slot::SourceSlot::Input(input), reader, in_card);
}

/// The library defaults of filter `def`
fn defaults_of(def: &FilterDef) -> Option<Filter> {
    match build(def.id, &Value::Null) {
        Ok(EffectKind::Filter(defaults)) => Some(defaults),
        _ => None,
    }
}

/// One pill per texture set channel, lit where the filter works; Alt+click keeps only the clicked one
fn channel_pills(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, effect: LayerId, off: &[Channel]) {
    let channels = app.session.doc.project.texture_set(app.session.ts()).map(|texture_set| texture_set.channel_list()).unwrap_or_default();
    let names: Vec<&str> = channels.iter().map(|channel| tr(pill_name(*channel))).collect();
    let on: Vec<bool> = channels.iter().map(|channel| !off.contains(channel)).collect();
    let Some((index, solo)) = sheet.channel_pills(&names, &on) else { return };
    let Some(clicked) = channels.get(index) else { return };

    let switched: serde_json::Map<String, Value> = if solo {
        channels.iter().map(|channel| (channel.id().to_string(), json!(channel == clicked))).collect()
    } else {
        std::iter::once((clicked.id().to_string(), json!(!on.get(index).copied().unwrap_or(true)))).collect()
    };
    app.run("effect.set", json!({"effect": effect.0, "channels": switched}));
}

/// How soon a filter card whose picture is still rendering looks for it again
const THUMBNAIL_POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// The glyph size on a slot without a finished picture
const FILTER_GLYPH_SIZE: f32 = 12.0;

/// How round a filter card's picture and a swatch's corners are, in points
const SWATCH_CORNER_RADIUS: u8 = 3;

/// The width of a picture's outline, in points
const OUTLINE_WIDTH: f32 = 1.0;

/// The card naming the filter (or None; `def` its library entry): a click opens the filter picker under it, × empties it; returns whether its ⟲ (live while `changed`) was clicked
fn filter_card(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, effect: LayerId, filter: &Filter, def: Option<&FilterDef>, changed: bool) -> bool {
    let ctx = sheet.ui.ctx().clone();
    let empty = *filter == Filter::Empty;
    let thumbnail = def.and_then(|def| app.thumbs.filter(&ctx, def.id));
    let tokens = Tokens::get(&ctx);
    let picture = |painter: &Painter, rect: Rect| match &thumbnail {
        Some(texture) => {
            painter.image(texture.id(), rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }

        // A saved filter outside the catalog has no picture request to wait for
        None if !empty => {
            let glyph = if def.is_some() { egui_phosphor::regular::HOURGLASS_MEDIUM } else { egui_phosphor::regular::FUNNEL };
            painter.rect_filled(rect.shrink(OUTLINE_WIDTH), SWATCH_CORNER_RADIUS, tokens.button);
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(FILTER_GLYPH_SIZE), tokens.text_dim);
            if def.is_some() {
                ctx.request_repaint_after(THUMBNAIL_POLL);
            }
        }
        None => {
            painter.rect_stroke(rect.shrink(OUTLINE_WIDTH), SWATCH_CORNER_RADIUS, Stroke::new(OUTLINE_WIDTH, tokens.text_dim), StrokeKind::Inside);
        }
    };
    let (name, source) = if empty { (tr("Filter"), tr("None")) } else { (tr(filter.label()), tr("Filter")) };
    let slot = if empty { sheet.slot(&picture, name, source, None, false, false) } else { sheet.slot_restorable(&picture, name, source, changed) };
    if slot.clear {
        app.run("effect.set", json!({"effect": effect.0, "filter": texturecraft_engine::mask_cmds::EMPTY_FILTER}));
    }
    crate::filter_slot::picker(app, &slot.pick, effect.0, def.map(|def| def.id));
    slot.reset
}

/// The seed of a legacy seeded filter as stored ([`scalar_params`] hands it over as an f32, which rounds seeds past 2^24)
fn legacy_seed(filter: &Filter) -> Option<u32> {
    match filter {
        Filter::Warp { seed, .. } | Filter::DirectionalWarp { seed, .. } | Filter::SlopeBlur { seed, .. } => Some(*seed),
        _ => None,
    }
}

/// The filter's own rows: Seed first, Intensity with its Divider chip, angles with their dial, then the color editors
fn parameters(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, filter: &Filter) {
    let set = |app: &mut TextureCraftApp, key: &str, value: Value, merge: bool| {
        let mut params = json!({ "effect": id, "merge": merge });
        params[key] = value;
        app.run("effect.set", params);
    };
    match filter {
        Filter::ColorBalance { shadows, midtones, highlights } => return color_balance(app, sheet, id, [highlights, midtones, shadows]),
        Filter::ColorCorrect { shadows, midtones, highlights } => return color_correct(app, sheet, id, [shadows, midtones, highlights]),
        Filter::Hsl { hue, saturation, lightness, perceptive } => return hsl(app, sheet, id, [*hue, *saturation, *lightness], *perceptive),
        Filter::GradientMap { .. } => return gradient(app, sheet, id, filter),
        Filter::GradientCurve { preset, invert, balance, contrast } => return gradient_curve(app, sheet, id, *preset, *invert, [*balance, *contrast]),
        Filter::GradientDynamic { vertical, position, .. } => return gradient_dynamic(app, sheet, id, *vertical, *position),
        Filter::HaloGlow { on_mask, precise, color, .. } => return glow(app, sheet, id, filter, (*on_mask, *precise), *color),
        Filter::DropShadow { color, .. } => return drop_shadow(app, sheet, id, filter, *color),
        Filter::BakedLighting { .. } => return baked_lighting_stylized(app, sheet, id, filter),
        Filter::Threshold { level, mode } => return threshold(app, sheet, id, *level, *mode),
        Filter::Mirror(mirror) => return mirror_rows(app, sheet, id, mirror),
        Filter::Transform(transform) => return crate::filter_transform_rows::transform_rows(app, sheet, id, transform),
        Filter::MaskOutline(outline) => return mask_outline(app, sheet, id, outline),
        Filter::DistanceBevel(bevel) => return bevel_rows(app, sheet, id, bevel),
        Filter::BevelSmooth(smooth) => return bevel_smooth(app, sheet, id, smooth),
        Filter::SourceWarp(warp) => return source_warp(app, sheet, id, warp),
        Filter::SourceSlopeBlur(slope) => return slope_blur(app, sheet, id, slope),
        Filter::HighPass { radius, space, alpha } => return highpass(app, sheet, id, *radius, *space, *alpha),
        Filter::GrayscaleConversion(conversion) => return grayscale_conversion(app, sheet, id, conversion),
        Filter::Grayscale => return grayscale_conversion(app, sheet, id, &GrayscaleConversion::of_luminance()),
        Filter::AnisotropicKuwahara(kuwahara) => return anisotropic_kuwahara(app, sheet, id, kuwahara),
        Filter::PbrValidate(validate) => return pbr_validate(app, sheet, id, validate),
        Filter::HeightToNormal(filter) => return height_to_normal(app, sheet, id, filter),
        Filter::ColorMatch(matcher) => return color_match(app, sheet, id, matcher),
        _ => {}
    }
    let params = scalar_params(filter);

    // Invert leads its filter's rows (Invert, Height Adjust)
    let leading = match filter {
        Filter::Invert | Filter::InvertOff => Some(*filter == Filter::Invert),
        Filter::HeightAdjust { invert, .. } => Some(*invert),
        _ => None,
    };
    if let Some(mut on) = leading
        && sheet.toggle(tr("Invert"), &mut on)
    {
        set(app, "invert", json!(on), false);
    }
    if let Some(&(_, label, ..)) = params.iter().find(|(key, ..)| *key == "seed")
        && let Some(mut seed) = legacy_seed(filter)
        && sheet.seed(tr(label), &mut seed)
    {
        set(app, "seed", json!(seed), false);
    }
    let divided = divided_key(filter);
    for (key, label, mut value, min, max) in params.into_iter().filter(|(key, ..)| *key != "seed") {
        let edit = match divided {
            Some((divided_param, divider)) if divided_param == key => intensity_row(app, sheet, id, &mut value, min..=max, divider),
            _ if matches!(key, "angle" | "azimuth" | "horizontalRotation" | "verticalRotation") => sheet.angle(tr(label), &mut value, min..=max),
            _ => range_slider(sheet, tr(label), &mut value, min..=max, decimals(key, min, max)),
        };
        if let Some(merge) = edit {
            set(app, key, json!(value), merge);
        }
    }

    let trailing = match filter {
        Filter::Blur { wrap, .. } => Some(("wrap", tr("Blur Wrap"), *wrap)),
        Filter::Bevel { wrap, .. } => Some(("wrap", tr("Bevel Wrap"), *wrap)),
        Filter::Posterize { alpha, .. } => Some(("applyToAlpha", tr("Apply to Alpha"), *alpha)),
        Filter::Mosaic { alpha, .. } => Some(("applyToAlpha", tr("Apply To Alpha"), *alpha)),
        Filter::Fxaa { alpha } => Some(("affectAlphaChannel", tr("Affect Alpha Channel"), *alpha)),
        Filter::HistogramScan { invert_position, .. } => Some(("invertPosition", tr("Invert Position"), *invert_position)),
        _ => None,
    };
    if let Some((key, label, mut on)) = trailing
        && sheet.toggle(label, &mut on)
    {
        set(app, key, json!(on), false);
    }

    if let Filter::ReplaceColor { from, to, .. } = filter {
        replace_colors(app, sheet, id, *from, *to);
    }
}

/// Highpass: Color Space, Radius (in Blur Intensity's unit) and Apply To Alpha
fn highpass(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, radius: f32, space: HighpassSpace, alpha: bool) {
    if let (Some(picked), _) = sheet.dropdown(tr("Color Space"), space, &HighpassSpace::ALL, |space| tr(space.label()).to_owned(), None) {
        app.run("effect.set", json!({"effect": id, "colorSpace": picked.id()}));
    }
    slider_rows(app, sheet, id, &[("intensity", tr("Radius"), radius / Filter::BLUR_INTENSITY, 0.0..=BLUR_INTENSITY_SLIDER_MAX)]);
    let mut alpha = alpha;
    if sheet.toggle(tr("Apply To Alpha"), &mut alpha) {
        app.run("effect.set", json!({"effect": id, "applyToAlpha": alpha}));
    }
}

/// Grayscale Conversion: Channel Input (its Custom Input right under it), Invert, Balance, Contrast, Mode and the Channels Weights
fn grayscale_conversion(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, conversion: &GrayscaleConversion) {
    let inputs = [tr("Current Channel"), tr("Custom Input")];
    if let Some(picked) = sheet.switch(tr("Channel Input"), &inputs, usize::from(conversion.custom_input)) {
        app.run("effect.set", json!({"effect": id, "channelInput": if picked == 1 { "customInput" } else { "currentChannel" }}));
    }
    if conversion.custom_input {
        input_slot(app, sheet, LayerId(id), ImageInput::CUSTOM_INPUT, true);
    }
    let mut invert = conversion.invert;
    if sheet.toggle(tr("Invert"), &mut invert) {
        app.run("effect.set", json!({"effect": id, "invert": invert}));
    }
    slider_rows(app, sheet, id, &[("balance", tr("Balance"), conversion.balance, 0.0..=1.0), ("contrast", tr("Contrast"), conversion.contrast, 0.0..=1.0)]);
    if let (Some(picked), _) = sheet.dropdown(tr("Mode"), conversion.mode, &ConversionMode::ALL, |mode| tr(mode.label()).to_owned(), None) {
        app.run("effect.set", json!({"effect": id, "mode": picked.id()}));
    }
    if conversion.mode == ConversionMode::ChannelsWeights {
        let [red, green, blue, alpha] = conversion.weights;
        let rows = [
            ("red", tr("Red"), red, 0.0..=1.0),
            ("green", tr("Green"), green, 0.0..=1.0),
            ("blue", tr("Blue"), blue, 0.0..=1.0),
            ("alpha", tr("Alpha"), alpha, 0.0..=1.0),
        ];
        slider_rows(app, sheet, id, &rows);
    }
}

/// Anisotropic Kuwahara: Extract Direction (its Custom Input right under it), Radius, Smoothness, Sharpness, Tensor Smoothness, Anisotropy and its Angle
fn anisotropic_kuwahara(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, kuwahara: &AnisotropicKuwahara) {
    let inputs = [tr("Current Channel"), tr("Custom Input")];
    if let Some(picked) = sheet.switch(tr("Extract Direction"), &inputs, usize::from(kuwahara.custom_input)) {
        app.run("effect.set", json!({"effect": id, "extractDirection": if picked == 1 { "customInput" } else { "currentChannel" }}));
    }
    if kuwahara.custom_input {
        input_slot(app, sheet, LayerId(id), ImageInput::CUSTOM_INPUT, true);
    }
    let rows = [
        ("radius", tr("Radius"), kuwahara.radius, 0.0..=AnisotropicKuwahara::MAX_RADIUS),
        ("smoothness", tr("Smoothness"), kuwahara.smoothness, 0.0..=1.0),
        ("sharpness", tr("Sharpness"), kuwahara.sharpness, AnisotropicKuwahara::MIN_SHARPNESS..=1.0),
        ("tensorSmoothness", tr("Tensor Smoothness"), kuwahara.tensor_smoothness, 0.0..=1.0),
        ("anisotropy", tr("Anisotropy"), kuwahara.anisotropy, 0.0..=1.0),
    ];
    slider_rows(app, sheet, id, &rows);
    let mut angle = kuwahara.angle;
    if let Some(merge) = sheet.angle(tr("Anisotropy Angle"), &mut angle, DIAL_DEGREES) {
        app.run("effect.set", json!({"effect": id, "angle": angle, "merge": merge}));
    }
}

/// PBR Validate (Metallic Roughness): Validation Mode, Albedo Dark Range Threshold and Metal Reflectance Range, each label above its switch
fn pbr_validate(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, validate: &PbrValidate) {
    let modes: Vec<&str> = ValidationMode::ALL.iter().map(|mode| tr(mode.label())).collect();
    let mode = ValidationMode::ALL.iter().position(|mode| *mode == validate.mode).unwrap_or_default();
    if let Some(picked) = sheet.stacked_switch(tr("Validation Mode"), &modes, mode).and_then(|index| ValidationMode::ALL.get(index)) {
        app.run("effect.set", json!({"effect": id, "validationMode": picked.id()}));
    }
    let dark = PbrValidate::ALBEDO_DARK.iter().position(|value| *value == validate.albedo_dark).unwrap_or_default();
    if let Some(picked) = sheet.stacked_switch(tr("Albedo Dark Range Threshold"), &[tr("50 sRGB"), tr("30 sRGB")], dark) {
        app.run("effect.set", json!({"effect": id, "albedoDarkRangeThreshold": PbrValidate::ALBEDO_DARK.get(picked)}));
    }
    let low = PbrValidate::METAL_LOW.iter().position(|value| *value == validate.metal_low).unwrap_or_default();
    if let Some(picked) = sheet.stacked_switch(tr("Metal Reflectance Range"), &[tr("70-100% Reflective"), tr("60-100% Reflective")], low) {
        app.run("effect.set", json!({"effect": id, "metalReflectanceRange": PbrValidate::METAL_LOW.get(picked)}));
    }
}

/// Height To Normal: Overwrite Existing Normal, Use World Units, then Surface Size and Height Depth (whole cm) or Normal Intensity
fn height_to_normal(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, filter: &HeightToNormal) {
    for (key, label, on) in
        [("overwriteExistingNormal", tr("Overwrite Existing Normal"), filter.overwrite), ("useWorldUnits", tr("Use World Units"), filter.world_units)]
    {
        let mut on = on;
        if sheet.toggle(label, &mut on) {
            app.run("effect.set", json!({"effect": id, key: on}));
        }
    }
    let rows = if filter.world_units {
        vec![
            ("surfaceSize", tr("Surface Size (cm)"), filter.surface_size, HeightToNormal::MIN_SURFACE_SIZE..=HeightToNormal::MAX_SURFACE_SIZE, true),
            ("heightDepth", tr("Height Depth (cm)"), filter.height_depth, 0.0..=HeightToNormal::MAX_HEIGHT_DEPTH, true),
        ]
    } else {
        vec![("normalIntensity", tr("Normal Intensity"), filter.intensity, 0.0..=HeightToNormal::MAX_INTENSITY, false)]
    };
    for (key, label, stored, range, whole_centimetres) in rows {
        let mut value = stored;
        let decimals = if whole_centimetres { 0 } else { INTENSITY_DECIMALS };
        let slid = sheet.slider(label, &mut value, range, decimals);
        if !whole_centimetres {
            if let Some(merge) = slid {
                app.run("effect.set", json!({"effect": id, key: value, "merge": merge}));
            }
            continue;
        }

        // Whole centimetres: a press's first whole-number change starts its undo step, changes that round to the stored value apply nothing
        let value = value.round();
        let press = sheet.ui.make_persistent_id(("height_to_normal_press", id, key));
        if let Some(merge) = crate::widgets::press_edit(sheet.ui, press, slid.is_some() && value != stored) {
            app.run("effect.set", json!({"effect": id, key: value, "merge": merge}));
        }
    }
}

/// Color Match: each Color Mode with its color or image slot right under it, Custom Color Variation and, while it is on, the Color Variation card (Hue with its dial, Chroma, Luma)
fn color_match(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, matcher: &ColorMatch) {
    let color = |app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, key: &str, label: &str, linear: [f32; 3]| {
        let pick = sheet.color_bar_led(label, linear, app.picking_color, &app.session.doc.project.swatches);
        if let Some(picked) = crate::fill_material_panel::take_color_pick(app, pick) {
            app.run("effect.set", json!({"effect": id, key: picked, "merge": pick.merge}));
        }
    };
    if let (Some(picked), _) = sheet.dropdown(tr("Source Color Mode"), matcher.source_mode, &SourceColorMode::ALL, |mode| tr(mode.label()).to_owned(), None) {
        app.run("effect.set", json!({"effect": id, "sourceColorMode": picked.id()}));
    }
    match matcher.source_mode {
        SourceColorMode::Parameter => color(app, sheet, "sourceColor", tr("Source Color"), matcher.source_color),
        SourceColorMode::Input => input_slot(app, sheet, LayerId(id), ImageInput::SOURCE_INPUT, true),
        SourceColorMode::Average => {}
    }
    let targets: Vec<&str> = TargetColorMode::ALL.iter().map(|mode| tr(mode.label())).collect();
    let target = usize::from(matcher.target_mode == TargetColorMode::ImageInput);
    if let Some(picked) = sheet.switch(tr("Target Color Mode"), &targets, target).and_then(|index| TargetColorMode::ALL.get(index)) {
        app.run("effect.set", json!({"effect": id, "targetColorMode": picked.id()}));
    }
    match matcher.target_mode {
        TargetColorMode::Parameter => color(app, sheet, "targetColor", tr("Target Color"), matcher.target_color),
        TargetColorMode::ImageInput => input_slot(app, sheet, LayerId(id), ImageInput::TARGET_INPUT, true),
    }
    let mut custom = matcher.custom_variation;
    if sheet.toggle(tr("Custom Color Variation"), &mut custom) {
        app.run("effect.set", json!({"effect": id, "customColorVariation": custom}));
    }
    if !matcher.custom_variation {
        return;
    }

    if sheet.card(tr("Color Variation"), Some(true)) {
        let mut hue = matcher.hue;
        if let Some(merge) = sheet.angle(tr("Hue"), &mut hue, DIAL_DEGREES) {
            app.run("effect.set", json!({"effect": id, "hue": hue, "merge": merge}));
        }
        slider_rows(app, sheet, id, &[("chroma", tr("Chroma"), matcher.chroma, 0.0..=1.0), ("luma", tr("Luma"), matcher.luma, 0.0..=1.0)]);
    }
    sheet.close_card();
}

/// Threshold: its level, then the Mode comparing a value with it (>, ≥, <, ≤)
fn threshold(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, level: f32, mode: ThresholdMode) {
    let mut level = level;
    if let Some(merge) = sheet.slider(tr("Threshold"), &mut level, 0.0..=1.0, 2) {
        app.run("effect.set", json!({"effect": id, "level": level, "merge": merge}));
    }
    let signs = ThresholdMode::ALL.map(ThresholdMode::sign);
    let current = ThresholdMode::ALL.iter().position(|candidate| *candidate == mode).unwrap_or_default();
    if let Some(picked) = sheet.switch(tr("Mode"), &signs, current).and_then(|index| ThresholdMode::ALL.get(index)) {
        app.run("effect.set", json!({"effect": id, "mode": picked.id()}));
    }
}

/// Mirror: Display Debug and Mode, then the axis's offset and Invert switch, or the Corner Type of a corner mirror
fn mirror_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, mirror: &Mirror) {
    let set = |app: &mut TextureCraftApp, key: &str, value: Value, merge: bool| {
        app.run("effect.set", json!({"effect": id, key: value, "merge": merge}));
    };
    let mut debug = mirror.debug;
    if sheet.toggle(tr("Display Debug"), &mut debug) {
        set(app, "displayDebug", json!(debug), false);
    }
    if let (Some(picked), _) = sheet.dropdown(tr("Mode"), mirror.mode, &MirrorMode::ALL, |mode| tr(mode.label()).to_owned(), None) {
        set(app, "mode", json!(picked.id()), false);
    }
    let ([x_offset, y_offset], [x_invert, y_invert]) = (mirror.offset, mirror.invert);
    let (offset_key, offset_label, mut offset, invert_key, invert_label, mut invert) = match mirror.mode {
        MirrorMode::AxisX => ("axisXOffset", tr("Axis X Offset"), x_offset, "invertAxisX", tr("Invert Axis X"), x_invert),
        MirrorMode::AxisY => ("axisYOffset", tr("Axis Y Offset"), y_offset, "invertAxisY", tr("Invert Axis Y"), y_invert),
        MirrorMode::Corner => {
            let name = |corner: MirrorCorner| tr(corner.label()).to_owned();
            if let (Some(picked), _) = sheet.dropdown(tr("Corner Type"), mirror.corner, &MirrorCorner::ALL, name, None) {
                set(app, "cornerType", json!(picked.id()), false);
            }
            return;
        }
    };
    if let Some(merge) = sheet.slider(offset_label, &mut offset, 0.0..=1.0, 2) {
        set(app, offset_key, json!(offset), merge);
    }
    if sheet.toggle(invert_label, &mut invert) {
        set(app, invert_key, json!(invert), false);
    }
}

/// Mask Outline: Mask Position and Start From Black, then the band's Threshold, Width, Blur and Curve Shape
fn mask_outline(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, outline: &MaskOutline) {
    let names = OutlinePosition::ALL.map(|position| tr(position.label()));
    let current = OutlinePosition::ALL.iter().position(|position| *position == outline.position).unwrap_or_default();
    if let Some(picked) = sheet.switch(tr("Mask Position"), &names, current).and_then(|index| OutlinePosition::ALL.get(index)) {
        app.run("effect.set", json!({"effect": id, "maskPosition": picked.id()}));
    }
    let mut from_black = outline.from_black;
    if sheet.toggle(tr("Start From Black"), &mut from_black) {
        app.run("effect.set", json!({"effect": id, "startFromBlack": from_black}));
    }
    for (key, label, value) in [("threshold", tr("Threshold"), outline.threshold), ("width", tr("Width"), outline.width), ("blur", tr("Blur"), outline.blur)] {
        let mut value = value;
        if let Some(merge) = sheet.slider(label, &mut value, 0.0..=1.0, 2) {
            app.run("effect.set", json!({"effect": id, key: value, "merge": merge}));
        }
    }
    let mut curve = outline.curve;
    if let Some(merge) = sheet.signed_slider(tr("Curve Shape"), &mut curve, -1.0..=1.0, 2) {
        app.run("effect.set", json!({"effect": id, "curveShape": curve, "merge": merge}));
    }
}

/// A choice's shown name and the id the command takes for it
type ChoiceNames<T> = (fn(T) -> &'static str, fn(T) -> &'static str);

/// A switch among `all` (named by `label`) setting `key` to the picked one's `id`
fn choice_switch<T: Copy + PartialEq>(
    app: &mut TextureCraftApp,
    sheet: &mut PropertySheet<'_>,
    id: u64,
    (label, key): (&str, &str),
    all: &[T],
    current: T,
    (name, id_of): ChoiceNames<T>,
) {
    let names: Vec<&str> = all.iter().map(|choice| tr(name(*choice))).collect();
    let current_index = all.iter().position(|choice| *choice == current).unwrap_or_default();
    if let Some(picked) = sheet.switch(label, &names, current_index).and_then(|index| all.get(index)) {
        app.run("effect.set", json!({"effect": id, key: id_of(*picked)}));
    }
}

/// Sliders setting `effect.set` keys: (key, label, value, range), signed ranges filling from their middle
fn slider_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, rows: &[(&str, &str, f32, std::ops::RangeInclusive<f32>)]) {
    for (key, label, value, range) in rows {
        let mut value = *value;
        if let Some(merge) = range_slider(sheet, label, &mut value, range.clone(), 2) {
            app.run("effect.set", json!({"effect": id, *key: value, "merge": merge}));
        }
    }
}

/// A slider over `range`, filling from its middle when the range is signed
fn range_slider(sheet: &mut PropertySheet<'_>, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, decimals: usize) -> Option<bool> {
    if *range.start() < 0.0 { sheet.signed_slider(label, value, range, decimals) } else { sheet.slider(label, value, range, decimals) }
}

/// The Bevel Wrap switch of a bevel
fn bevel_wrap(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, wrap: bool) {
    let mut wrap = wrap;
    if sheet.toggle(tr("Bevel Wrap"), &mut wrap) {
        app.run("effect.set", json!({"effect": id, "wrap": wrap}));
    }
}

/// Bevel: Bevel Wrap, the signed Distance (inwards below 0), Corner Type, Smoothing and Blur Mode
fn bevel_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, bevel: &Bevel) {
    bevel_wrap(app, sheet, id, bevel.wrap);
    slider_rows(app, sheet, id, &[("distance", tr("Distance"), bevel.distance, -1.0..=1.0)]);
    choice_switch(app, sheet, id, (tr("Corner Type"), "cornerType"), &BevelCorner::ALL, bevel.corner, (BevelCorner::label, BevelCorner::id));
    slider_rows(app, sheet, id, &[("smoothing", tr("Smoothing"), bevel.smoothing, 0.0..=DISTANCE_BEVEL_SMOOTHING_MAX)]);
    choice_switch(app, sheet, id, (tr("Blur Mode"), "blurMode"), &BevelBlur::ALL, bevel.blur, (BevelBlur::label, BevelBlur::id));
}

/// Bevel Smooth: Direction, the ramp's Distance, Smoothing, Curve Offset and Shape, Mask Threshold, Distance Map Multiplier and Bevel Wrap
fn bevel_smooth(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, smooth: &BevelSmooth) {
    choice_switch(app, sheet, id, (tr("Direction"), "direction"), &BevelDirection::ALL, smooth.direction, (BevelDirection::label, BevelDirection::id));
    slider_rows(
        app,
        sheet,
        id,
        &[
            ("distance", tr("Distance"), smooth.distance, 0.0..=SMOOTH_DISTANCE_MAX),
            ("smoothing", tr("Smoothing"), smooth.smoothing, 0.0..=BEVEL_SMOOTHING_MAX),
            ("curveOffset", tr("Curve Offset"), smooth.curve_offset, -1.0..=1.0),
            ("curveShape", tr("Curve Shape"), smooth.curve_shape, -1.0..=1.0),
            ("maskThreshold", tr("Mask Threshold"), smooth.threshold, 0.0..=1.0),
            ("distanceMapMultiplier", tr("Distance Map Multiplier"), smooth.multiplier, 0.0..=1.0),
        ],
    );
    bevel_wrap(app, sheet, id, smooth.wrap);
}

/// Warp: Seed, Warp Mode, Intensity with its Divider, the directional modes' Angle (Multi Directional's Directions, Blending Mode), then Source Parameters
fn source_warp(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, warp: &SourceWarp) {
    let set = |app: &mut TextureCraftApp, key: &str, value: Value, merge: bool| {
        app.run("effect.set", json!({"effect": id, key: value, "merge": merge}));
    };
    let mut seed = warp.seed;
    if sheet.seed(tr("Seed"), &mut seed) {
        set(app, "seed", json!(seed), false);
    }
    if let (Some(picked), _) = sheet.dropdown(tr("Warp Mode"), warp.mode, &WarpMode::ALL, |mode| tr(mode.label()).to_owned(), None) {
        set(app, "warpMode", json!(picked.id()), false);
    }
    let mut intensity = warp.intensity;
    if let Some(merge) = intensity_row(app, sheet, id, &mut intensity, 0.0..=1.0, warp.divider) {
        set(app, "intensity", json!(intensity), merge);
    }
    if warp.mode != WarpMode::Warp {
        let mut angle = warp.angle;
        if let Some(merge) = sheet.angle(tr("Angle"), &mut angle, DIAL_DEGREES) {
            set(app, "angle", json!(angle), merge);
        }
    }
    if warp.mode == WarpMode::MultiDirectional {
        let counts = SourceWarp::DIRECTIONS.map(|count| count.to_string());
        let names: Vec<&str> = counts.iter().map(String::as_str).collect();
        let current_index = SourceWarp::DIRECTIONS.iter().position(|count| *count == warp.directions).unwrap_or_default();
        if let Some(picked) = sheet.switch(tr("Directions"), &names, current_index).and_then(|index| SourceWarp::DIRECTIONS.get(index)) {
            set(app, "directions", json!(picked), false);
        }
        let name = |blending: WarpBlending| tr(blending.label()).to_owned();
        if let (Some(picked), _) = sheet.dropdown(tr("Blending Mode"), warp.blending, &WarpBlending::ALL, name, None) {
            set(app, "blendingMode", json!(picked.id()), false);
        }
    }
    if sheet.card(tr("Source Parameters"), Some(true)) {
        let source = &warp.source;
        let name = |mode: WarpSourceMode| tr(mode.label()).to_owned();
        if let (Some(picked), _) = sheet.dropdown(tr("Source Mode"), source.mode, &WarpSourceMode::ALL, name, None) {
            set(app, "sourceMode", json!(picked.id()), false);
        }
        if source.mode == WarpSourceMode::CustomNoise {
            input_slot(app, sheet, LayerId(id), ImageInput::CUSTOM_NOISE, true);
        }
        slider_rows(
            app,
            sheet,
            id,
            &[
                ("sourceBlur", tr("Source Blur"), source.blur, 0.0..=1.0),
                ("sourceBalance", tr("Source Balance"), source.balance, 0.0..=1.0),
                ("sourceContrast", tr("Source Contrast"), source.contrast, 0.0..=1.0),
                ("sourceTiling", tr("Source Tiling"), source.tiling, SOURCE_TILING_MIN..=SOURCE_TILING_MAX),
            ],
        );
    }
    sheet.close_card();
}

/// Blur Slope: Seed, Intensity with its Divider, Blending Mode (each drawn as what it does to a bar), Quality, then Source Parameters
fn slope_blur(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, slope: &SourceSlopeBlur) {
    let set = |app: &mut TextureCraftApp, key: &str, value: Value, merge: bool| {
        app.run("effect.set", json!({"effect": id, key: value, "merge": merge}));
    };
    let mut seed = slope.seed;
    if sheet.seed(tr("Seed"), &mut seed) {
        set(app, "seed", json!(seed), false);
    }
    let mut intensity = slope.intensity;
    if let Some(merge) = intensity_row(app, sheet, id, &mut intensity, 0.0..=1.0, slope.divider) {
        set(app, "intensity", json!(intensity), merge);
    }
    let tokens = Tokens::get(sheet.ui.ctx());
    let picture = |painter: &Painter, rect: Rect, blending: SlopeBlending| slope_blending_picture(painter, rect, blending, &tokens);
    let name = |blending: SlopeBlending| tr(blending.label()).to_owned();
    if let (Some(picked), _) = sheet.dropdown(tr("Blending Mode"), slope.blending, &SlopeBlending::ALL, name, Some(&picture)) {
        set(app, "blendingMode", json!(picked.id()), false);
    }
    slider_rows(app, sheet, id, &[("quality", tr("Quality"), slope.quality, 0.0..=1.0)]);
    if sheet.card(tr("Source Parameters"), Some(true)) {
        let source = &slope.source;
        let name = |mode: WarpSourceMode| tr(mode.label()).to_owned();
        if let (Some(picked), _) = sheet.dropdown(tr("Source Type"), source.mode, &WarpSourceMode::ALL, name, None) {
            set(app, "sourceMode", json!(picked.id()), false);
        }
        if source.mode == WarpSourceMode::CustomNoise {
            input_slot(app, sheet, LayerId(id), ImageInput::CUSTOM_NOISE, true);
        }
        let mut shown_blur = source.blur * SLOPE_SOURCE_BLUR_DISPLAY_MAX;
        if let Some(merge) = sheet.slider(tr("Blur"), &mut shown_blur, 0.0..=SLOPE_SOURCE_BLUR_DISPLAY_MAX, 2) {
            set(app, "sourceBlur", json!(shown_blur / SLOPE_SOURCE_BLUR_DISPLAY_MAX), merge);
        }
        slider_rows(
            app,
            sheet,
            id,
            &[
                ("sourceBalance", tr("Position"), source.balance, 0.0..=1.0),
                ("sourceContrast", tr("Contrast"), source.contrast, 0.0..=1.0),
                ("sourceTiling", tr("Source Tiling"), source.tiling, SOURCE_TILING_MIN..=SOURCE_TILING_MAX),
            ],
        );
    }
    sheet.close_card();
}

/// The display range for Blur Slope's source blur, stored on the shared 0..1 scale
const SLOPE_SOURCE_BLUR_DISPLAY_MAX: f32 = 20.0;

/// A light bar on the field after the slope pass: smeared (Blur), eaten away (Min) or grown (Max)
fn slope_blending_picture(painter: &Painter, rect: Rect, blending: SlopeBlending, tokens: &Tokens) {
    painter.rect_filled(rect, 0, tokens.field);
    let bar = |left: f32, right: f32| {
        let (top, bottom) = (rect.top() + rect.height() * SLOPE_BAR_TOP, rect.top() + rect.height() * (1.0 - SLOPE_BAR_TOP));
        Rect::from_min_max(egui::pos2(rect.left() + rect.width() * left, top), egui::pos2(rect.left() + rect.width() * right, bottom))
    };
    match blending {
        SlopeBlending::Blur => {
            for (step, spread) in SLOPE_BLUR_SPREADS.iter().enumerate() {
                let share = (step + 1) as f32 / SLOPE_BLUR_SPREADS.len() as f32;
                let tint = tokens.text.gamma_multiply(share * SLOPE_BLUR_LAYER_ALPHA);
                painter.rect_filled(
                    bar(BLUR_BAR.0 - spread, BLUR_BAR.1 + spread).expand2(egui::vec2(0.0, spread * rect.height() * SLOPE_BLUR_VERTICAL_SHARE)),
                    SLOPE_BLUR_LAYER_ROUNDING,
                    tint,
                );
            }
        }
        SlopeBlending::Min => {
            painter.rect_filled(bar(MIN_BAR.0, MIN_BAR.1), 0, tokens.text);
        }
        SlopeBlending::Max => {
            painter.rect_filled(bar(MAX_BAR.0, MAX_BAR.1), 0, tokens.text);
        }
    }
    painter.rect_stroke(rect, 0, Stroke::new(OUTLINE_WIDTH, tokens.field_border), StrokeKind::Inside);
}

/// Where a blending picture's bar starts down its square (and ends as far from the bottom)
const SLOPE_BAR_TOP: f32 = 0.25;

/// How far each soft layer of the Blur picture's bar spreads past the bar (shares of the square), widest first
const SLOPE_BLUR_SPREADS: [f32; 4] = [0.12, 0.08, 0.04, 0.0];

/// How opaque the Blur picture's soft layers stack up to
const SLOPE_BLUR_LAYER_ALPHA: f32 = 0.45;

/// How much of a Blur layer's sideways spread it also grows up and down: half, so the smear reads wider than tall
const SLOPE_BLUR_VERTICAL_SHARE: f32 = 0.5;

/// How round a Blur layer's corners are, in points: soft, where the Min and Max bars are square
const SLOPE_BLUR_LAYER_ROUNDING: u8 = 1;

/// The left and right ends of each slope blur mode's bar, as shares of the pictogram's width
const BLUR_BAR: (f32, f32) = (0.3, 0.62);
const MIN_BAR: (f32, f32) = (0.42, 0.58);
const MAX_BAR: (f32, f32) = (0.18, 0.82);

/// The tone groups' names and the `effect.set` keys they edit, in the order a panel shows them
const SHADOWS: (&str, &str) = ("Shadows", "shadows");
const MIDTONES: (&str, &str) = ("Midtones", "midtones");
const HIGHLIGHTS: (&str, &str) = ("Highlights", "highlights");

/// Color Balance: a card per tone (Highlights, Midtones, Shadows), each with Cyan ↔ Red, Magenta ↔ Green and Yellow ↔ Blue
fn color_balance(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, tones: [&[f32; 3]; 3]) {
    let ends = [(tr("Cyan"), tr("Red")), (tr("Magenta"), tr("Green")), (tr("Yellow"), tr("Blue"))];
    let rights = ends.map(|(_, right)| right);
    for ((name, key), values) in [HIGHLIGHTS, MIDTONES, SHADOWS].into_iter().zip(tones) {
        if !sheet.card(tr(name), Some(true)) {
            sheet.close_card();
            continue;
        }

        let mut balance = *values;
        let mut edit = None;
        for (component, (value, (left, right))) in balance.iter_mut().zip(ends).enumerate() {
            let (left_tint, right_tint) =
                (OPPOSITE_COLORS.get(component).copied().unwrap_or_default(), AXIS_COLORS.get(component).copied().unwrap_or_default());
            edit = edit.or(sheet.tinted_slider((left, left_tint), (right, right_tint), &rights, value));
        }
        sheet.close_card();
        if let Some(merge) = edit {
            app.run("effect.set", json!({"effect": id, key: balance, "merge": merge}));
        }
    }
}

/// Color Correct: a card per tone (Shadows, Midtones, Highlights), each with Contrast, Luminosity and Saturation
fn color_correct(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, tones: [&[f32; 3]; 3]) {
    for ((name, key), values) in [SHADOWS, MIDTONES, HIGHLIGHTS].into_iter().zip(tones) {
        if !sheet.card(tr(name), Some(true)) {
            sheet.close_card();
            continue;
        }

        let [mut contrast, mut luminosity, mut saturation] = *values;
        let edit = sheet
            .signed_slider(tr("Contrast"), &mut contrast, -1.0..=1.0, 2)
            .or(sheet.signed_slider(tr("Luminosity"), &mut luminosity, -1.0..=1.0, 2))
            .or(sheet.slider(tr("Saturation"), &mut saturation, 0.0..=1.0, 2));
        sheet.close_card();
        if let Some(merge) = edit {
            app.run("effect.set", json!({"effect": id, key: [contrast, luminosity, saturation], "merge": merge}));
        }
    }
}

/// HSL Perceptive: Mode (Regular | Perceptive), then Hue, Saturation and Lightness on 0..1 with 0.5 leaving them as they are
fn hsl(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, [hue, saturation, lightness]: [f32; 3], perceptive: bool) {
    if let Some(mode) = sheet.switch(tr("Mode"), &[tr("Regular"), tr("Perceptive")], usize::from(perceptive)) {
        app.run("effect.set", json!({"effect": id, "mode": if mode == 1 { "perceptive" } else { "regular" }}));
    }

    // Shown around 0.5: the hue as half a turn either way (a stored turn past half wraps round), saturation and lightness as −1..1 halved
    let rows = [
        ("hue", tr("Hue"), if hue == 0.5 { 1.0 } else { (hue + 0.5).rem_euclid(1.0) }, 1.0),
        ("saturation", tr("Saturation"), 0.5 + saturation / 2.0, 2.0),
        ("lightness", tr("Lightness"), 0.5 + lightness / 2.0, 2.0),
    ];
    for (key, label, shown, scale) in rows {
        let mut value = shown.clamp(0.0, 1.0);
        if let Some(merge) = sheet.centered_slider(label, &mut value, 0.0..=1.0, 0.5, 2) {
            app.run("effect.set", json!({"effect": id, key: (value - 0.5) * scale, "merge": merge}));
        }
    }
}

/// Gradient: Display Gradient (Debug), the Input Transformation card, then the Colors card (the ramp, Color Quantity, Blend Color in Linear and a row per color)
fn gradient(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, filter: &Filter) {
    let Filter::GradientMap { colors, positions, input, linear, debug, .. } = filter else { return };
    let set = |app: &mut TextureCraftApp, params: Value| {
        let mut params = params;
        params["effect"] = json!(id);
        app.run("effect.set", params);
    };
    let mut shown = *debug;
    if sheet.toggle(tr("Display Gradient (Debug)"), &mut shown) {
        set(app, json!({"debug": shown}));
    }

    if sheet.card(tr("Input Transformation"), Some(true)) {
        let name = |mode: GrayscaleMode| tr(mode.label()).to_owned();
        if let (Some(mode), _) = sheet.dropdown(tr("Grayscale Mode"), input.mode, &GrayscaleMode::ALL, name, None) {
            set(app, json!({"grayscaleMode": mode.id()}));
        }
        for (key, label, value) in
            [("blur", tr("Blur Intensity"), input.blur), ("balance", tr("Balance"), input.balance), ("contrast", tr("Contrast"), input.contrast)]
        {
            let mut value = value;
            let range = if key == "blur" { 0.0..=BLUR_INTENSITY_SLIDER_MAX } else { 0.0..=1.0 };
            if let Some(merge) = sheet.slider(label, &mut value, range, 2) {
                set(app, json!({key: value, "merge": merge}));
            }
        }
        let mut invert = input.invert;
        if sheet.toggle(tr("Invert"), &mut invert) {
            set(app, json!({"invert": invert}));
        }
    }
    sheet.close_card();

    if sheet.card(tr("Colors"), Some(true)) {
        let used = filter.ramp_colors();

        // An evenly spaced ramp (projects saved before) shows its places, and keeps them once one moves
        let last = used.len().saturating_sub(1).max(1) as f32;
        let mut places: Vec<f32> = (0..colors.len()).map(|index| positions.get(index).copied().unwrap_or(index as f32 / last)).collect();
        if let Some(drag) = sheet.gradient_ramp(used, &places, &|t| filter.ramp_at(t).unwrap_or([0.0; 3])) {
            if let Some(place) = places.get_mut(drag.index) {
                *place = drag.position;
            }
            set(app, json!({"positions": places, "merge": drag.merge}));
        }
        let mut quantity = used.len() as f32;
        let slid = sheet.slider(tr("Color Quantity"), &mut quantity, f32::from(GRADIENT_QUANTITY_MIN)..=f32::from(Filter::MAX_GRADIENT_COLORS), 0).is_some();
        let quantity = quantity.round();

        // A press's first whole-number change starts its undo step: changes that round to the count it has apply nothing
        let press = sheet.ui.make_persistent_id(("color_quantity_press", id));
        if let Some(merge) = crate::widgets::press_edit(sheet.ui, press, slid && quantity as usize != used.len()) {
            set(app, json!({"quantity": quantity, "merge": merge}));
        }
        let mut in_linear = *linear;
        if sheet.toggle(tr("Blend Color in Linear"), &mut in_linear) {
            set(app, json!({"linear": in_linear}));
        }
        for (index, color) in used.iter().enumerate() {
            let label = trf("Color {number}", &[("number", &(index + 1))]);
            let mut place = places.get(index).copied().unwrap_or(0.0);
            let (pick, moved) = sheet.color_stop(&label, *color, &mut place, app.picking_color, &app.session.doc.project.swatches);
            if let Some(picked) = crate::fill_material_panel::take_color_pick(app, pick) {
                let mut recolored = colors.clone();
                if let Some(slot) = recolored.get_mut(index) {
                    *slot = picked;
                }
                set(app, json!({"colors": recolored, "merge": pick.merge}));
            }
            if let Some(merge) = moved {
                if let Some(slot) = places.get_mut(index) {
                    *slot = place;
                }
                set(app, json!({"positions": places, "merge": merge}));
            }
        }
    }
    sheet.close_card();
}

/// Gradient Curve: Preset (each drawn as its curve), Invert, Balance and Contrast
fn gradient_curve(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, preset: CurvePreset, invert: bool, [balance, contrast]: [f32; 2]) {
    let tokens = Tokens::get(sheet.ui.ctx());
    let picture = |painter: &Painter, rect: Rect, preset: CurvePreset| curve_picture(painter, rect, preset, &tokens);
    let name = |preset: CurvePreset| tr(preset.label()).to_owned();
    if let (Some(picked), _) = sheet.dropdown(tr("Preset"), preset, &CurvePreset::ALL, name, Some(&picture)) {
        app.run("effect.set", json!({"effect": id, "preset": picked.id()}));
    }
    let mut inverted = invert;
    if sheet.toggle(tr("Invert"), &mut inverted) {
        app.run("effect.set", json!({"effect": id, "invert": inverted}));
    }
    for (key, label, value) in [("balance", tr("Balance"), balance), ("contrast", tr("Contrast"), contrast)] {
        let mut value = value;
        if let Some(merge) = sheet.slider(label, &mut value, 0.0..=1.0, 2) {
            app.run("effect.set", json!({"effect": id, key: value, "merge": merge}));
        }
    }
}

/// A curve preset's shape over a dark square
fn curve_picture(painter: &Painter, rect: Rect, preset: CurvePreset, tokens: &Tokens) {
    painter.rect_filled(rect, CURVE_PICTURE_ROUNDING, tokens.field);
    let inner = rect.shrink(CURVE_PICTURE_INSET);
    let points: Vec<egui::Pos2> = (0..=CURVE_PICTURE_STEPS)
        .map(|step| {
            let t = step as f32 / CURVE_PICTURE_STEPS as f32;
            egui::pos2(inner.left() + inner.width() * t, inner.bottom() - inner.height() * preset.at(t))
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(CURVE_PICTURE_STROKE, tokens.accent)));
}

/// A curve picture's inset in its square, in points
const CURVE_PICTURE_INSET: f32 = 2.0;

/// How round a curve picture's square is, in points
const CURVE_PICTURE_ROUNDING: u8 = 2;

/// How many straight pieces a curve picture's line is drawn with
const CURVE_PICTURE_STEPS: usize = 30;

/// The width of a curve picture's line, in points
const CURVE_PICTURE_STROKE: f32 = 1.5;

/// Gradient Dynamic: Gradient Orientation and Gradient Input Position (its Gradient Source is a card after the filter's)
fn gradient_dynamic(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, vertical: bool, position: f32) {
    if let Some(picked) = sheet.switch(tr("Gradient Orientation"), &[tr("Horizontal"), tr("Vertical")], usize::from(vertical)) {
        app.run("effect.set", json!({"effect": id, "orientation": if picked == 1 { "vertical" } else { "horizontal" }}));
    }
    let mut value = position;
    if let Some(merge) = sheet.slider(tr("Gradient Input Position"), &mut value, 0.0..=1.0, 2) {
        app.run("effect.set", json!({"effect": id, "position": value, "merge": merge}));
    }
}

/// Gradient Dynamic's Gradient Source: its own slot card after the filter's, a uniform color whose bar opens the picker
fn gradient_source(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, color: [f32; 3]) {
    let swatch = |painter: &Painter, rect: Rect| {
        painter.rect_filled(rect, SWATCH_CORNER_RADIUS, crate::fill_material_panel::shown_color(color));
    };

    // The card's row opens the picker of the colour bar under it: its source is a uniform color only
    let card = sheet.slot(&swatch, tr("Gradient Source"), tr("Uniform"), None, false, false);
    let pick = sheet.color_bar(tr("Gradient Source"), color, app.picking_color, &app.session.doc.project.swatches);
    if card.pick.clicked() {
        sheet.open_color_picker(pick.id, color);
    }
    sheet.close_card();
    if let Some(picked) = crate::fill_material_panel::take_color_pick(app, pick) {
        app.run("effect.set", json!({"effect": id, "color": picked, "merge": pick.merge}));
    }
}

/// Glow: Used on a Mask, Mode (Soft | Precise), Width (inner below 0), Spread, Opacity and its Color
fn glow(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, filter: &Filter, (on_mask, precise): (bool, bool), color: [f32; 3]) {
    let mut used = on_mask;
    if sheet.toggle(tr("Used on a Mask"), &mut used) {
        app.run("effect.set", json!({"effect": id, "usedOnMask": used}));
    }
    if let Some(mode) = sheet.switch(tr("Mode"), &[tr("Soft"), tr("Precise")], usize::from(precise)) {
        app.run("effect.set", json!({"effect": id, "mode": if mode == 1 { "precise" } else { "soft" }}));
    }
    for (key, label, mut value, min, max) in scalar_params(filter) {
        if let Some(merge) = range_slider(sheet, tr(label), &mut value, min..=max, 2) {
            app.run("effect.set", json!({"effect": id, key: value, "merge": merge}));
        }
    }
    color_row(app, sheet, id, color);
}

/// Drop Shadow: Angle with its dial, Distance (either way), Size, Spread, Opacity and its Color
fn drop_shadow(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, filter: &Filter, color: [f32; 3]) {
    for (key, label, mut value, min, max) in scalar_params(filter) {
        let edit = if key == "angle" { sheet.angle(tr(label), &mut value, min..=max) } else { range_slider(sheet, tr(label), &mut value, min..=max, 2) };
        if let Some(merge) = edit {
            app.run("effect.set", json!({"effect": id, key: value, "merge": merge}));
        }
    }
    color_row(app, sheet, id, color);
}

/// Baked Lighting Stylized: a card each for the Sun, the Sky, Light 1 and 2, Occlusion and cavity, the Material and Normal details (one saved before shows its plain settings)
fn baked_lighting_stylized(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, filter: &Filter) {
    let Filter::BakedLighting { azimuth, elevation, light, ambient, occlusion, stylized } = filter else { return };
    let lighting = stylized.as_deref().unwrap_or(&StylizedLighting::PLAIN);
    let set = |app: &mut TextureCraftApp, params: Value| {
        let mut params = params;
        params["effect"] = json!(id);
        app.run("effect.set", params);
    };

    if sheet.card(tr("Sun"), Some(true)) {
        let mut value = *light;
        if let Some(merge) = sheet.slider(tr("Sun Intensity"), &mut value, 0.0..=SUN_INTENSITY_MAX, 2) {
            set(app, json!({"light": value, "merge": merge}));
        }
        for (key, label, value) in
            [("azimuth", tr("Sun Horizontal Angle"), *azimuth), ("elevation", tr("Sun Vertical Angle"), elevation.rem_euclid(FULL_TURN_DEGREES))]
        {
            let mut value = value;
            if let Some(merge) = sheet.angle(label, &mut value, DIAL_DEGREES) {
                set(app, json!({key: value, "merge": merge}));
            }
        }
        let pick = sheet.color_bar_led(tr("Sun Color"), lighting.sun_color, app.picking_color, &app.session.doc.project.swatches);
        if let Some(picked) = crate::fill_material_panel::take_color_pick(app, pick) {
            set(app, json!({"sunColor": picked, "merge": pick.merge}));
        }
    }
    sheet.close_card();

    if sheet.card(tr("Sky"), Some(true)) {
        let mut value = *ambient;
        if let Some(merge) = sheet.slider(tr("Sky Intensity"), &mut value, 0.0..=SKY_INTENSITY_SLIDER_MAX, 2) {
            set(app, json!({"ambient": value, "merge": merge}));
        }
        for (key, label, color) in [
            ("skyColor", tr("Sky Color"), lighting.sky_color),
            ("horizonColor", tr("Horizon Color"), lighting.horizon_color),
            ("groundColor", tr("Ground Color"), lighting.ground_color),
        ] {
            let pick = sheet.color_bar_led(label, color, app.picking_color, &app.session.doc.project.swatches);
            if let Some(picked) = crate::fill_material_panel::take_color_pick(app, pick) {
                set(app, json!({key: picked, "merge": pick.merge}));
            }
        }
    }
    sheet.close_card();

    for (key, title, extra) in [("light1", tr("Light 1"), lighting.lights[0]), ("light2", tr("Light 2"), lighting.lights[1])] {
        if sheet.card(title, Some(true)) {
            extra_light(app, sheet, id, key, extra);
        }
        sheet.close_card();
    }

    if sheet.card(tr("Occlusion and cavity"), Some(true)) {
        let rows = [
            ("occlusion", tr("Diffuse AO"), *occlusion, 1.0),
            ("diffuseCavity", tr("Diffuse Cavity"), lighting.diffuse_cavity, 1.0),
            ("specularAo", tr("Specular AO"), lighting.specular_ao, 1.0),
            ("specularCavity", tr("Specular Cavity"), lighting.specular_cavity, 1.0),
            ("cavitySmoothness", tr("Cavity Smoothness"), lighting.cavity_smoothness, MAX_CAVITY_SMOOTHNESS),
            ("edgesIntensity", tr("Edges Intensity"), lighting.edges_intensity, 1.0),
            ("edgesSmoothness", tr("Edges Smoothness"), lighting.edges_smoothness, 1.0),
        ];
        for (key, label, value, max) in rows {
            let mut value = value;
            if let Some(merge) = sheet.slider(label, &mut value, 0.0..=max, 2) {
                set(app, json!({key: value, "merge": merge}));
            }
        }
    }
    sheet.close_card();

    if sheet.card(tr("Material"), Some(true)) {
        let input = |glossiness: bool| tr(if glossiness { "Specular / Glossiness" } else { "Metal / Roughness" }).to_owned();
        if let (Some(glossiness), _) = sheet.dropdown(tr("Input"), lighting.specular_glossiness, &[false, true], input, None) {
            set(app, json!({"input": if glossiness { "specularGlossiness" } else { "metalRoughness" }}));
        }
        let output = |output: LightingOutput| tr(output.label()).to_owned();
        if let (Some(picked), _) = sheet.dropdown(tr("Output"), lighting.output, &LightingOutput::ALL, output, None) {
            set(app, json!({"output": picked.id()}));
        }
        let mut value = lighting.dielectric_reflectance;
        if let Some(merge) = sheet.slider(tr("Dielectric Reflectance"), &mut value, 0.0..=MAX_DIELECTRIC_REFLECTANCE, 2) {
            set(app, json!({"dielectricReflectance": value, "merge": merge}));
        }
    }
    sheet.close_card();

    if sheet.card(tr("Normal details"), Some(true)) {
        let details = |mesh_only: bool| tr(if mesh_only { "Mesh only" } else { "Mesh + Height + Normal" }).to_owned();
        if let (Some(mesh_only), _) = sheet.dropdown(tr("Normal Details Type"), lighting.mesh_only, &[true, false], details, None) {
            set(app, json!({"normalDetails": if mesh_only { "mesh" } else { "meshHeightNormal" }}));
        }
        let mut value = lighting.height_to_normal;
        if let Some(merge) = sheet.slider(tr("Height to Normal Intensity"), &mut value, 0.0..=MAX_HEIGHT_TO_NORMAL, 2) {
            set(app, json!({"heightToNormal": value, "merge": merge}));
        }
    }
    sheet.close_card();
}

/// Light 1 or 2's rows (`key` names it to `effect.set`): Horizontal and Vertical Angle with their dials, Intensity and Color
fn extra_light(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, key: &str, light: DirectionalLight) {
    let set = |app: &mut TextureCraftApp, part: Value, merge: bool| {
        app.run("effect.set", json!({"effect": id, key: part, "merge": merge}));
    };
    for (part, label, value) in
        [("azimuth", tr("Horizontal Angle"), light.azimuth), ("elevation", tr("Vertical Angle"), light.elevation.rem_euclid(FULL_TURN_DEGREES))]
    {
        let mut value = value;
        if let Some(merge) = sheet.angle(label, &mut value, DIAL_DEGREES) {
            set(app, json!({part: value}), merge);
        }
    }
    let mut intensity = light.intensity;
    if let Some(merge) = sheet.slider(tr("Intensity"), &mut intensity, 0.0..=MAX_LIGHT_INTENSITY, 2) {
        set(app, json!({"intensity": intensity}), merge);
    }
    let pick = sheet.color_bar_led(tr("Color"), light.color, app.picking_color, &app.session.doc.project.swatches);
    if let Some(picked) = crate::fill_material_panel::take_color_pick(app, pick) {
        set(app, json!({"color": picked}), pick.merge);
    }
}

/// A light or shadow filter's Color: its bar opens the colour picker, the eyedropper picks from the screen
fn color_row(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, color: [f32; 3]) {
    let pick = sheet.color_bar_led(tr("Color"), color, app.picking_color, &app.session.doc.project.swatches);
    if let Some(picked) = crate::fill_material_panel::take_color_pick(app, pick) {
        app.run("effect.set", json!({"effect": id, "color": picked, "merge": pick.merge}));
    }
}

/// The parameter a filter's Divider divides and the divider it has
fn divided_key(filter: &Filter) -> Option<(&'static str, u16)> {
    match filter {
        Filter::Warp { divider, .. } | Filter::DirectionalWarp { divider, .. } => Some(("intensity", *divider)),
        Filter::SlopeBlur { divider, .. } => Some(("radius", *divider)),
        _ => None,
    }
}

/// Intensity with its "÷10" Divider chip before the value; the chip's list picks 1, 10, 100 or 1000
fn intensity_row(
    app: &mut TextureCraftApp,
    sheet: &mut PropertySheet<'_>,
    id: u64,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    divider: u16,
) -> Option<bool> {
    let shown = |divider: u16| format!("÷{divider}");
    let (edit, chip) = sheet.slider_with_chip(tr("Intensity"), value, range, 2, &shown(divider), WIDEST_DIVIDER);
    let (picked, _) = sheet.hanging_list(&chip, divider, &Filter::DIVIDERS, &shown, None);
    if let Some(divider) = picked {
        app.run("effect.set", json!({"effect": id, "divider": divider}));
    }
    edit
}

/// A range wider than this shows one decimal
const WIDE_RANGE: f32 = 10.0;

/// Decimals a parameter's value shows: whole numbers for counts, two for blur intensities, one for wide ranges
fn decimals(key: &str, min: f32, max: f32) -> usize {
    if matches!(key, "levels" | "pixelsAmount") {
        0
    } else if key == "intensity" {
        2
    } else if max - min > WIDE_RANGE {
        1
    } else {
        2
    }
}

/// Replace Color's two colors: each bar opens the colour picker, its eyedropper picks from the screen
fn replace_colors(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, from: [f32; 3], to: [f32; 3]) {
    for (key, label, linear) in [("from", tr("Color to replace"), from), ("to", tr("Replacement color"), to)] {
        let pick = sheet.color_bar_led(label, linear, app.picking_color, &app.session.doc.project.swatches);
        if let Some(picked) = crate::fill_material_panel::take_color_pick(app, pick) {
            app.run("effect.set", json!({"effect": id, key: picked, "merge": pick.merge}));
        }
    }
}
