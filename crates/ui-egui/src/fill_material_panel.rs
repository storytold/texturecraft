//! A fill's Material section: the channel strip, the orb with the Material | Channels switch, then one slot card per channel that is on (Channels) or the fill's whole material (Material)

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, Painter, Rect};
use egui_phosphor::regular as ic;
use serde_json::{Value, json};
use texturecraft_engine::Session;
use texturecraft_engine::compose::ImageRes;
use texturecraft_engine::core::{
    Channel, ChannelKind, CookResolution, Fill, LayerId, LinkParam, MaterialLink, NormalFormat, OutputResolution, Procedural, ProceduralKind, ResolutionMode,
    Source,
};
use texturecraft_engine::fill_material_cmds::whole_materials;
use texturecraft_engine::materials::find;
use texturecraft_engine::paint::alphas;
use texturecraft_engine::texel::linear_to_srgb;

use crate::TextureCraftApp;
use crate::color_picker::{ColorPick, SwatchEdit};
use crate::dock::{anchors_below, project_effect_anchors_below, with};
use crate::fill_panel::{FillKey, fill_key};
use crate::i18n::{n_, tr};
use crate::material_orb::material_orb;
use crate::orb_material::OrbMaterial;
use crate::pattern_panel::tile_generator_sections;
use crate::pattern_target::PatternTarget;
use crate::picker_group::PickerGroup;
use crate::picker_tab::PickerTab;
use crate::property_sheet::PropertySheet;
use crate::resolution_band::{resolution_band, resolution_row};
use crate::resource_picker;
use crate::resource_picker_item::ResourcePickerItem;
use crate::slot_thumbnail::{paint_thumbnail, thumbnail};
use crate::source_choice::SourceChoice;
use crate::source_slot::SourceSlot;
use crate::state::UiState;
use crate::theme::Tokens;
use crate::tile_picture::TilePicture;
use crate::widgets::press_edit;

/// The switch's two sides: the fill's whole material, or one source per channel
const MODES: [&str; 2] = [n_("Material"), n_("Channels")];

/// The switch's side that shows the fill's whole material
const MATERIAL_MODE: usize = 0;

/// The switch's side that shows one source per channel
const CHANNELS_MODE: usize = 1;

/// Decimals a linked material's number input shows
const FLOAT_DECIMALS: usize = 2;

/// Pattern scale a procedural's slider runs over (typed values stay inside it too)
const PATTERN_SCALE: std::ops::RangeInclusive<f32> = 0.1..=32.0;

/// Pattern contrast a procedural's slider runs over
const PATTERN_CONTRAST: std::ops::RangeInclusive<f32> = 0.0..=2.0;

/// Pattern balance a procedural's slider runs over: below the middle darker, above lighter
const PATTERN_BALANCE: std::ops::RangeInclusive<f32> = -1.0..=1.0;

/// The size a pattern's Resolution reads the texture set at when none is open: a new project's default
const FALLBACK_TEXTURE_SET_SIZE: u32 = 2048;

/// The Material section of the fill at `base`; `on` holds the channels the fill writes
pub(crate) fn material_section(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, fill: &Fill, base: &Value, on: &[Channel], reader: LayerId) {
    sheet.section(tr("Material"), false);
    let channels = app.session.doc.project.texture_set(app.session.ts()).map(|texture_set| texture_set.channel_list()).unwrap_or_default();
    let names: Vec<&str> = channels.iter().map(|channel| tr(pill_name(*channel))).collect();
    let enabled: Vec<bool> = channels.iter().map(|channel| on.contains(channel)).collect();
    if let Some((index, solo)) = sheet.channel_pills(&names, &enabled)
        && let Some(channel) = channels.get(index)
    {
        let enable = solo || !enabled.get(index).copied().unwrap_or(false);
        app.run("layer.fill.set", with(base, json!({"channel": channel.id(), "enabled": enable, "solo": solo})));
    }

    let tokens = Tokens::get(sheet.ui.ctx());
    let salt = fill_key(base);
    let look = OrbMaterial::of_fill(fill, on, &app.session.images);
    let channel_look = channel_orb_look(&app.ui, fill, on, &app.session.images);
    let shown = channel_look.as_ref().unwrap_or(&look);
    let orb = |painter: &Painter, rect: Rect| material_orb(painter, rect, shown, &tokens, ("orb", salt));
    let link = fill.link.as_deref().zip(base.get("layer").and_then(Value::as_u64)).map(|(link, layer)| (link, LayerId(layer)));
    let mode_id = sheet.ui.make_persistent_id(("material_mode", salt));

    // A fill opens on the side that made it: a material (linked or built-in) on Material, separate sources on Channels
    let made_whole = link.is_some() || fill.material.is_some();
    let mode = sheet.ui.data(|data| data.get_temp::<usize>(mode_id)).unwrap_or(if made_whole { MATERIAL_MODE } else { CHANNELS_MODE });
    let modes = MODES.map(tr);
    if let Some(picked) = sheet.orb_switch(&orb, &modes, mode) {
        sheet.ui.data_mut(|data| data.insert_temp(mode_id, picked));
    }

    match link {
        Some((link, layer)) if mode == MATERIAL_MODE => {
            let shown: Vec<Channel> = channels.into_iter().filter(|channel| on.contains(channel)).collect();
            material_card(app, sheet, link, layer, &look, salt);
            mapping_band(app, sheet, fill, &shown, layer);
            let texture_set = texture_set_size(app, link.size);
            let picked = resolution_band(sheet, link.resolution.unwrap_or(CookResolution::TEXTURE_SET), texture_set).filter(|_| LINK_EDITABLE);
            if let Some(picked) = picked {
                let size = resolution_json(picked);
                if app.run("material.link.resolution", with(&json!({"layer": layer.0, "cook": false}), size)).is_some() {
                    app.cook_link(layer);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            if sheet.button(ic::ARROW_SQUARE_OUT, tr("Open in MaterialCraft")).clicked() {
                crate::dock::open_link_in_materialcraft(app, layer, link);
            }
        }
        None if mode == MATERIAL_MODE => whole_material_card(app, sheet, fill, base, &look, salt),
        _ => {
            for channel in channels.into_iter().filter(|channel| on.contains(channel)) {
                source_card(app, sheet, fill.source(channel), base, SourceSlot::Channel(channel), reader, false);
            }
        }
    }
}

/// Whether a linked material's settings can change here: a change cooks it again, which needs the MaterialCraft CLI the browser has not got (it shows them as they were cooked)
const LINK_EDITABLE: bool = cfg!(not(target_arch = "wasm32"));

/// What the orb shows instead of the whole material, following the Layers panel's channel list: the channel picked there alone (None: the whole material)
fn channel_orb_look(ui: &UiState, fill: &Fill, on: &[Channel], images: &HashMap<String, Arc<ImageRes>>) -> Option<OrbMaterial> {
    (!ui.layer_material_view).then(|| OrbMaterial::of_channel(fill, ui.layer_channel, on, images))
}

/// `resolution` as the commands read it: `{"relative": factor}` or `{"fixed": size}`
fn resolution_json(resolution: CookResolution) -> Value {
    match resolution {
        CookResolution::Relative(factor) => json!({"relative": factor}),
        CookResolution::Fixed(size) => json!({"fixed": size}),
    }
}

/// The size of the texture set being painted, `fallback` when none is
pub(crate) fn texture_set_size(app: &TextureCraftApp, fallback: u32) -> u32 {
    app.session.doc.project.texture_set(app.session.ts()).map_or(fallback, |texture_set| texture_set.resolution)
}

/// A channel's name on its pill: the reference strip's short Title Case words, the full name where it has none
pub(crate) fn pill_name(channel: Channel) -> &'static str {
    match channel {
        Channel::BaseColor => n_("Color"),
        Channel::Metallic => n_("Metal"),
        Channel::Roughness => n_("Rough"),
        Channel::Normal => n_("Normal"),
        Channel::Height => n_("Height"),
        Channel::Opacity => n_("Opacity"),
        other => other.label(),
    }
}

/// A linear colour as the sRGB colour the screen shows
pub(crate) fn shown_color(linear: [f32; 3]) -> Color32 {
    let [r, g, b] = linear.map(|channel_value| (linear_to_srgb(channel_value.clamp(0.0, 1.0)) * 255.0).round() as u8);
    Color32::from_rgb(r, g, b)
}

/// What the slot's source line says
pub(crate) fn source_line(app: &TextureCraftApp, source: &Source) -> String {
    match source {
        Source::Uniform(_) => tr("Uniform").to_owned(),
        Source::Procedural { pattern, .. } => format!("{} · {}", tr(pattern.kind.label()), tr("procedural")),
        Source::Image { name, .. } => match app.session.library_alpha(name) {
            Some(alpha) => format!("{} · {}", tr(&alpha.label), tr("alpha")),
            None => format!("{name} · {}", tr("image")),
        },
        Source::Anchor { anchor, .. } => {
            let name = app.session.doc.project.effect(*anchor).map_or_else(|| tr("missing").to_owned(), |(_, effect)| effect.name.clone());
            format!("{} · {name}", tr("Anchor"))
        }
    }
}

/// One source's card (a channel's, a mask fill's grayscale or a filter's image input): its slot (the row opens the source picker), then the source's own settings; `in_card`: only its slot row, inside the card open now
pub(crate) fn source_card(
    app: &mut TextureCraftApp,
    sheet: &mut PropertySheet<'_>,
    source: Source,
    base: &Value,
    slot_of: SourceSlot,
    reader: LayerId,
    in_card: bool,
) {
    let set = |extra: Value| with(base, with(&slot_of.fields(), extra));
    let tokens = Tokens::get(sheet.ui.ctx());
    let picture = thumbnail(app, &sheet.ui.ctx().clone(), &source);
    let line = source_line(app, &source);
    let uniform = matches!(source, Source::Uniform(_));
    let procedural = matches!(source, Source::Procedural { .. });
    let thumb = |painter: &egui::Painter, rect: egui::Rect| paint_thumbnail(painter, rect, &picture, &tokens);
    let slot = if in_card {
        sheet.slot_in_card(&thumb, slot_of.name(), &line, !uniform)
    } else {
        sheet.slot(&thumb, slot_of.name(), &line, None, !uniform, procedural)
    };

    let popup = sheet.ui.make_persistent_id(("source_picker", fill_key(base), slot_of.key()));
    if slot.pick.clicked() {
        resource_picker::toggle(sheet.ui.ctx(), popup);
    }
    let current = source_choice(&app.session, &source);

    // From Uniform the picker opens at the top, not at its Uniform tile at the end
    let reveal = current != SourceChoice::Uniform;
    let uniform = match source {
        Source::Uniform(value) => value,
        _ => slot_of.default_value(),
    };

    // Inherited mask fills can only read anchors below both the reader and the effect's owner
    let effect = base.get("effect").and_then(Value::as_u64).map(LayerId);
    let tabs = || {
        let anchors = match effect {
            Some(effect) => project_effect_anchors_below(&app.session.doc.project, reader, effect),
            None => anchors_below(&app.session.doc.project, reader),
        };
        source_tabs(&app.session, &anchors, shown_color(uniform), true)
    };
    let picked = resource_picker::show(&slot.pick, popup, &mut app.thumbs, Some(&current), reveal, tabs);
    if let Some(choice) = picked.filter(|choice| *choice != current) {
        pick_source(app, sheet.ui.ctx(), slot_of, choice, &set);
    }
    if slot.clear {
        app.run("layer.fill.set", set(json!({"value": slot_of.default_value()})));
    }
    if let Some(name) = crate::dock::image_drop(sheet.ui, &slot.pick) {
        app.run("layer.fill.set", set(json!({"image": name})));
    }

    // Embedded procedural slots share the surrounding mode card
    if in_card {
        if let Source::Uniform(value) = source {
            uniform_rows(app, sheet, slot_of, value, &set);
        } else if let Source::Procedural { pattern, .. } = source {
            let mut edited = pattern.clone();
            if let Some((key, value, merge)) = scale_and_contrast(sheet, &mut edited) {
                let mut settings = json!({"procedural": pattern.kind.label(), "merge": merge});
                settings[key] = value;
                app.run("layer.fill.set", set(settings));
            }
        }
        return;
    }

    match source {
        Source::Uniform(value) => uniform_rows(app, sheet, slot_of, value, &set),
        Source::Procedural { pattern, low, high, .. } => procedural_rows(app, sheet, slot_of, &pattern, [low, high], slot.reset, &set),
        Source::Image { normal_format, .. } if slot_of.kind() == ChannelKind::NormalMap => {
            if sheet.fold(tr("Attributes"), None, false) {
                let formats = [tr("OpenGL"), tr("DirectX")];
                let chosen = usize::from(normal_format == NormalFormat::DirectX);
                if let Some(picked) = sheet.switch(tr("Normal format"), &formats, chosen) {
                    app.run("layer.fill.set", set(json!({"normalFormat": if picked == 1 { "directX" } else { "openGl" }})));
                }
            }
        }
        Source::Anchor { anchor, channel, .. } if slot_of == SourceSlot::Grayscale && !crate::dock::anchor_in_mask(app, anchor) => {
            anchor_channel_row(app, sheet, base, reader, channel);
        }
        Source::Image { .. } | Source::Anchor { .. } => {}
    }
    sheet.close_card();
}

/// The channel a mask fill reads from a content anchor point as gray (a mask anchor point reads its mask's one gray)
fn anchor_channel_row(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, base: &Value, reader: LayerId, channel: Channel) {
    let project = &app.session.doc.project;
    let channels = project.locate(reader).and_then(|(set, _, _)| project.texture_set(set).ok()).map(|set| set.channel_list()).unwrap_or_default();
    if let (Some(picked), _) = sheet.dropdown(tr("Channel"), channel, &channels, |option| tr(option.label()).to_string(), None) {
        app.run("effect.set", with(base, json!({"anchorChannel": picked.id()})));
    }
}

/// Runs the fill command `choice` asks for on the slot (Import opens the import dialog)
fn pick_source(app: &mut TextureCraftApp, ctx: &egui::Context, slot_of: SourceSlot, choice: SourceChoice, set: &impl Fn(Value) -> Value) {
    match choice {
        SourceChoice::Uniform => {
            app.run("layer.fill.set", set(json!({"value": slot_of.default_value()})));
        }
        SourceChoice::Procedural(kind) => {
            let (low, high) = slot_of.procedural_ends();
            app.run("layer.fill.set", set(json!({"procedural": kind.label(), "low": low, "high": high})));
        }
        SourceChoice::Image(name) => {
            app.run("layer.fill.set", set(json!({"image": name})));
        }
        SourceChoice::Alpha(id) => {
            app.run("layer.fill.set", set(json!({"alpha": id})));
        }
        SourceChoice::Import => crate::menus::invoke(app, ctx, "resource.import"),
        SourceChoice::Anchor(anchor) => {
            app.run("layer.fill.set", set(json!({"anchor": anchor.0})));
        }
    }
}

/// The picker's choice `source` is: a library alpha's image is its alpha
pub(crate) fn source_choice(session: &Session, source: &Source) -> SourceChoice {
    match source {
        Source::Uniform(_) => SourceChoice::Uniform,
        Source::Procedural { pattern, .. } => SourceChoice::Procedural(pattern.kind),
        Source::Image { name, .. } => {
            session.library_alpha(name).map_or_else(|| SourceChoice::Image(name.clone()), |alpha| SourceChoice::Alpha(alpha.id.as_str()))
        }
        Source::Anchor { anchor, .. } => SourceChoice::Anchor(*anchor),
    }
}

/// The source picker's tabs: Resources (Procedurals, Textures with Import... first, Alphas in their shelf groups, Uniform) and the stack's Anchor points
pub(crate) fn source_tabs(session: &Session, anchors: &[(LayerId, String)], uniform: Color32, include_alphas: bool) -> Vec<PickerTab<SourceChoice>> {
    let procedurals =
        ProceduralKind::ALL.into_iter().map(|kind| tile(SourceChoice::Procedural(kind), tr(kind.label()), None, TilePicture::Procedural(kind))).collect();
    let textures = ResourcePickerItem::textures(session, SourceChoice::Import, |name| SourceChoice::Image(name.to_owned()));
    let mut resources =
        vec![PickerGroup { header: Some(tr("Procedurals")), items: procedurals }, PickerGroup { header: Some(tr("Textures")), items: textures }];
    if include_alphas {
        let items = alphas::library()
            .iter()
            .map(|alpha| tile(SourceChoice::Alpha(alpha.id.as_str()), tr(&alpha.label), Some(tr(alpha.category)), TilePicture::Alpha(alpha)))
            .collect();
        resources.push(PickerGroup { header: Some(tr("Alphas")), items });
    }

    let uniform = tile(SourceChoice::Uniform, tr("Uniform value"), None, TilePicture::Swatch(uniform));
    resources.push(PickerGroup { header: Some(tr("Uniform")), items: vec![uniform] });
    let anchors = anchors.iter().map(|(anchor, label)| tile(SourceChoice::Anchor(*anchor), label, None, TilePicture::Glyph(ic::ANCHOR))).collect();
    vec![
        PickerTab { icon: ic::SQUARES_FOUR, name: tr("Resources"), groups: resources },
        PickerTab { icon: ic::ANCHOR, name: tr("Anchor points"), groups: vec![PickerGroup { header: None, items: anchors }] },
    ]
}

/// One tile of the source picker
fn tile(id: SourceChoice, label: &str, caption: Option<&'static str>, picture: TilePicture) -> ResourcePickerItem<SourceChoice> {
    ResourcePickerItem { id, label: label.to_owned(), caption, picture }
}

/// A uniform source: the colour bar (a click opens the picker) or the gray ramp with its invert glyph
fn uniform_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, slot_of: SourceSlot, value: [f32; 3], set: &impl Fn(Value) -> Value) {
    let label = slot_of.name();
    if slot_of.kind() == ChannelKind::Color {
        let pick = sheet.color_bar(label, value, app.picking_color, &app.session.doc.project.swatches);
        if let Some(color) = take_color_pick(app, pick) {
            app.run("layer.fill.set", set(json!({"color": color, "merge": pick.merge})));
        }
        return;
    }

    let mut gray = value[0];
    let edit = sheet.ramp(label, &mut gray, slot_of.range());
    if edit.invert {
        let inverted = if slot_of.kind() == ChannelKind::Signed { -gray } else { 1.0 - gray };
        app.run("layer.fill.set", set(json!({"value": inverted})));
    } else if let Some(merge) = edit.changed {
        app.run("layer.fill.set", set(json!({"value": gray, "merge": merge})));
    }
}

/// The colour `pick` picked, after starting or ending an eyedropper pick its eyedropper asked for and making the change its Swatches row asked for
pub(crate) fn take_color_pick(app: &mut TextureCraftApp, pick: ColorPick) -> Option<[f32; 3]> {
    if pick.eyedropper {
        app.picking_color = if app.picking_color == Some(pick.id) { None } else { Some(pick.id) };
    }
    match pick.swatch {
        Some(SwatchEdit::Add(color)) => {
            app.run("swatch.add", json!({"color": color}));
        }
        Some(SwatchEdit::Remove(index)) => {
            app.run("swatch.remove", json!({"index": index}));
        }
        None => {}
    }
    pick.picked
}

/// A pattern's Scale and Contrast rows; returns the edit (key, value, whether it continues a drag)
fn scale_and_contrast(sheet: &mut PropertySheet<'_>, edited: &mut Procedural) -> Option<(&'static str, Value, bool)> {
    let mut edit = None;
    if let Some(merge) = sheet.slider(tr("Scale"), &mut edited.scale, PATTERN_SCALE, 2) {
        edit = Some(("scale", json!(edited.scale), merge));
    }
    if let Some(merge) = sheet.slider(tr("Contrast"), &mut edited.contrast, PATTERN_CONTRAST, 2) {
        edit = edit.or(Some(("contrast", json!(edited.contrast), merge)));
    }
    edit
}

/// A pattern's Balance and Seed rows; returns the edit like [`scale_and_contrast`]
fn balance_and_seed(sheet: &mut PropertySheet<'_>, edited: &mut Procedural) -> Option<(&'static str, Value, bool)> {
    let mut edit = None;
    if let Some(merge) = sheet.signed_slider(tr("Balance"), &mut edited.balance, PATTERN_BALANCE, 2) {
        edit = Some(("balance", json!(edited.balance), merge));
    }
    if sheet.seed(tr("Seed"), &mut edited.seed) {
        edit = edit.or(Some(("seed", json!(edited.seed), false)));
    }
    edit
}

/// A procedural source: Scale and Contrast, then its Attributes (seed, balance, invert and its two values), each sending only its own parameter
fn procedural_rows(
    app: &mut TextureCraftApp,
    sheet: &mut PropertySheet<'_>,
    slot_of: SourceSlot,
    pattern: &Procedural,
    [low, high]: [[f32; 3]; 2],
    reset: bool,
    set: &impl Fn(Value) -> Value,
) {
    let kind = pattern.kind.label();
    if reset {
        let fresh = Procedural::new(pattern.kind);
        let settings = json!({
            "procedural": kind, "scale": fresh.scale, "contrast": fresh.contrast, "seed": fresh.seed,
            "balance": fresh.balance, "invert": fresh.invert, "low": low, "high": high, "merge": false,
            "resolution": "auto", "customResolution": fresh.resolution.custom, "resolutionFactor": fresh.resolution.factor,
        });
        app.run("layer.fill.set", set(settings));
        return;
    }

    let mut edited = pattern.clone();
    let mut edit = scale_and_contrast(sheet, &mut edited);

    // Grayscale source cards show Balance and Seed alongside Scale and Contrast
    let gray = matches!(slot_of, SourceSlot::Grayscale | SourceSlot::Input(_));
    if gray {
        edit = edit.or(balance_and_seed(sheet, &mut edited));
    }
    if sheet.fold(tr("Attributes"), None, false) {
        if !gray {
            edit = edit.or(balance_and_seed(sheet, &mut edited));
        }
        if sheet.toggle(tr("Invert"), &mut edited.invert) {
            edit = edit.or(Some(("invert", json!(edited.invert), false)));
        }
        let ends = [("low", tr("Low"), low), ("high", tr("High"), high)];
        if slot_of.kind() == ChannelKind::Color {
            for (key, label, value) in ends {
                let pick = sheet.color_bar_led(label, value, app.picking_color, &app.session.doc.project.swatches);
                if let Some(picked) = take_color_pick(app, pick) {
                    edit = edit.or(Some((key, json!(picked), pick.merge)));
                }
            }
        } else {
            for (key, label, value) in ends {
                let mut gray = value[0];
                if let Some(merge) = sheet.slider(label, &mut gray, slot_of.range(), 2) {
                    edit = edit.or(Some((key, json!([gray, gray, gray]), merge)));
                }
            }
        }
    }

    // The size the pattern is made at, as a cooked texture would be (the texture set's own by default)
    let texture_set = texture_set_size(app, FALLBACK_TEXTURE_SET_SIZE);
    if let Some(picked) = resolution_row(sheet, band_size(pattern.resolution, texture_set), texture_set) {
        let size = match picked {
            CookResolution::Relative(factor) => json!({"resolution": "auto", "resolutionFactor": factor}),
            CookResolution::Fixed(size) => json!({"resolution": "custom", "customResolution": size, "resolutionFactor": 0}),
        };
        app.run("layer.fill.set", set(with(&json!({"procedural": kind}), size)));
        return;
    }

    if let Some((key, value, merge)) = edit {
        let mut settings = json!({"procedural": kind, "merge": merge});
        settings[key] = value;
        app.run("layer.fill.set", set(settings));
    }

    // The Tile Generator's own parameters and image inputs follow the rows every pattern has
    if let Some(tile) = pattern.tile.as_deref() {
        let base = set(json!({"low": low, "high": high}));
        let target = PatternTarget { command: "layer.fill.set", salt: base.to_string(), base };
        sheet.embed(|ui| tile_generator_sections(app, ui, &target, pattern, tile));
    }
}

/// The Resolution band's reading of a pattern's saved output resolution: a size of its own is Fixed, any other follows the texture set
pub(crate) fn band_size(resolution: OutputResolution, texture_set: u32) -> CookResolution {
    match resolution.mode {
        ResolutionMode::Custom => CookResolution::Fixed(resolution.output_size(texture_set, texture_set)),
        _ => CookResolution::Relative(resolution.factor),
    }
}

/// The Material side: the fill's whole material in one card (the row lists its presets), its inputs, then opening it in MaterialCraft
fn material_card(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, link: &MaterialLink, layer: LayerId, look: &OrbMaterial, salt: FillKey) {
    let tokens = Tokens::get(sheet.ui.ctx());
    let base = json!({"layer": layer.0, "cook": false});
    let slot = sheet.slot(&|painter, rect| material_orb(painter, rect, look, &tokens, ("card", salt)), &link.name, tr("Material"), None, false, false);
    let presets: Vec<usize> = (0..link.presets.len()).collect();
    let preset_name = |index: usize| link.presets.get(index).map(|preset| preset.name.clone()).unwrap_or_default();
    let editable = LINK_EDITABLE;
    if editable
        && let (Some(index), _) = sheet.hanging_list(&slot.pick, usize::MAX, &presets, &preset_name, None)
        && app.run("material.link.preset", with(&base, json!({"name": preset_name(index)}))).is_some()
    {
        app.cook_link(layer);
    }

    let pending = sheet.ui.make_persistent_id(("material_cook_pending", layer.0));
    for param in &link.params {
        if let Some((value, merge)) = link_input(sheet, layer, param, editable)
            && app.run("material.link.set", with(&base, json!({"id": param.id, "value": value, "merge": merge}))).is_some()
        {
            sheet.ui.data_mut(|data| data.insert_temp(pending, true));
        }
    }

    // A drag stores every step; the maps re-cook once the pointer lets go
    let waiting = sheet.ui.data(|data| data.get_temp::<bool>(pending)).unwrap_or(false);
    if waiting && !sheet.ui.input(|input| input.pointer.any_down()) {
        sheet.ui.data_mut(|data| data.remove::<bool>(pending));
        app.cook_link(layer);
    }
    sheet.close_card();
}

/// The Material side of a fill without a linked material: one card holding the built-in material its sources came from (the row lists every plain material, × returns the channels to their defaults)
fn whole_material_card(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, fill: &Fill, base: &Value, look: &OrbMaterial, salt: FillKey) {
    let tokens = Tokens::get(sheet.ui.ctx());
    let held = fill.material.as_deref().and_then(find);
    let name = held.map_or_else(|| tr("None"), |material| tr(material.label));
    let slot = sheet.slot(&|painter, rect| material_orb(painter, rect, look, &tokens, ("card", salt)), name, tr("Material"), None, held.is_some(), false);
    let popup = sheet.ui.make_persistent_id(("material_picker", salt));
    if slot.pick.clicked() {
        resource_picker::toggle(sheet.ui.ctx(), popup);
    }
    let current = held.map(|material| material.id);
    let picked = resource_picker::show(&slot.pick, popup, &mut app.thumbs, current.as_ref(), current.is_some(), material_tabs);
    if let Some(id) = picked.filter(|id| Some(*id) != current) {
        app.run("layer.fill.material", with(base, json!({"material": id})));
    }
    if slot.clear {
        app.run("layer.fill.material", with(base, json!({"material": null})));
    }
    sheet.close_card();
}

/// The material picker's one tab: every plain material's preview ball, grouped under its shelf category in library order
fn material_tabs() -> Vec<PickerTab<&'static str>> {
    let mut groups: Vec<PickerGroup<&'static str>> = Vec::new();
    for material in whole_materials() {
        let item = ResourcePickerItem { id: material.id, label: tr(material.label).to_owned(), caption: None, picture: TilePicture::Material(material.id) };
        let header = Some(tr(material.category));
        match groups.iter_mut().find(|group| group.header == header) {
            Some(group) => group.items.push(item),
            None => groups.push(PickerGroup { header, items: vec![item] }),
        }
    }
    vec![PickerTab { icon: ic::SPHERE, name: tr("Materials"), groups }]
}

/// Channels mapping: which of the material's outputs each channel that is on reads
fn mapping_band(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, fill: &Fill, shown: &[Channel], layer: LayerId) {
    let Some(link) = &fill.link else { return };
    if !sheet.band(tr("Channels mapping"), true, None, None).open {
        return;
    }

    let outputs: Vec<String> = fill.link_outputs().into_keys().collect();
    let indices: Vec<usize> = (0..outputs.len()).collect();
    for channel in shown {
        let reads = link.output_of(*channel);
        let current = outputs.iter().position(|output| output == reads).unwrap_or(usize::MAX);
        let name = |index: usize| outputs.get(index).map_or(reads, String::as_str).to_owned();
        if let (Some(index), _) = sheet.dropdown(tr(channel.label()), current, &indices, name, None)
            && let Some(output) = outputs.get(index)
        {
            app.run("material.link.map", json!({"layer": layer.0, "channel": channel.id(), "output": output}));
        }
    }
}

/// One input of the material: a slider for numbers, a toggle, a dropdown for choices, an egui editor for colours and vectors; its new typed value when it changed and whether it continues a drag
fn link_input(sheet: &mut PropertySheet<'_>, layer: LayerId, param: &LinkParam, editable: bool) -> Option<(Value, bool)> {
    let kind = param.default.get("t").and_then(Value::as_str).unwrap_or("");
    let value = param.value.get("v").unwrap_or(&Value::Null);
    let number = value.as_f64().unwrap_or(0.0) as f32;
    let (low, high) = if param.max > param.min { (param.min, param.max) } else { (0.0, 1.0) };
    let changed = match kind {
        "bool" => {
            let mut on = value.as_bool().unwrap_or(number != 0.0);
            sheet.toggle(&param.label, &mut on).then(|| (json!(on), false))
        }
        "enum" if !param.options.is_empty() => {
            let options: Vec<usize> = (0..param.options.len()).collect();
            let name = |index: usize| param.options.get(index).cloned().unwrap_or_default();
            sheet.dropdown(&param.label, number.max(0.0) as usize, &options, name, None).0.map(|index| (json!(index), false))
        }
        "float" => {
            let mut edited = number;
            sheet.slider(&param.label, &mut edited, low.min(number)..=high.max(number), FLOAT_DECIMALS).map(|merge| (json!(edited), merge))
        }
        "int" | "enum" => {
            let mut edited = number;
            let moved = sheet.slider(&param.label, &mut edited, low.min(number)..=high.max(number), 0).is_some();

            // Only a new integer starts an edit or schedules another cook
            let whole = edited.round();
            let press = sheet.ui.make_persistent_id(("link_input_press", layer.0, &param.id));
            press_edit(sheet.ui, press, moved && whole != number).map(|merge| (json!(whole), merge))
        }

        // Colours and vectors keep their egui editors until the sheet draws them
        _ => sheet.embed(|ui| crate::dock::link_param_editor(ui, layer, param)),
    };
    changed.filter(|_| editable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_orb_shows_the_channel_the_layers_panel_shows_and_the_whole_material_on_material() {
        let mut ui = UiState::default();
        let on = [Channel::BaseColor, Channel::Roughness];

        let whole = OrbMaterial::of_fill(&Fill::default(), &on, &HashMap::new());
        let starting = channel_orb_look(&ui, &Fill::default(), &on, &HashMap::new());
        ui.layer_material_view = false;
        ui.layer_channel = Channel::Roughness;
        let roughness = channel_orb_look(&ui, &Fill::default(), &on, &HashMap::new()).unwrap();

        assert!(starting.is_none(), "the list starts on Material: the whole material");
        assert_eq!(whole.alone(), None);
        assert_eq!(roughness.alone(), Some(Channel::Roughness));
        assert_ne!(whole.key(), roughness.key(), "a new pick draws a new orb");
    }
}
