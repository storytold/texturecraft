//! Export dialog › Output templates: create and edit export presets. Built-in presets are
//! read-only; the selected user preset is edited as a draft that is saved through
//! `export.preset.save` whenever it validates, so a half-typed file name never reaches disk.
//! Maps dragged from the Input maps, Mesh maps and Converted maps panes onto an output map's
//! channel swatches fill its channels.

use egui::{Color32, Rect, RichText, Ui, pos2, vec2};
use egui_phosphor::regular as ic;
use serde_json::{Value, json};
use texturecraft_engine::core::{BakedMap, Channel};
use texturecraft_engine::formats::export::{FILE_TOKENS, MAX_PRESET_NAME, sanitize_file_stem};
use texturecraft_engine::formats::{Dest, ExportPreset, FileFormat, MapSpec, SourceRef, VirtualMap};

use crate::TextureCraftApp;
use crate::export_window::{ExportLook, GUTTER, TAB_ROW_HEIGHT, drawn, template_label};
use crate::i18n::{n_, tr, trf};
use crate::theme::Tokens;
use crate::unavailable::NOT_AVAILABLE;
use crate::widgets::chip;

mod channel_target;
mod map_drag;
mod map_lists;
mod map_preview_cache;
mod map_preview_pictures;
mod output_card;
mod pending_drop;
mod planned_output;
mod row_edit;
mod source_pick;
mod swatch_look;

pub use channel_target::ChannelTarget;
use map_drag::MapDrag;
use map_lists::{CONVERTED_MAPS, INPUT_MAPS, MESH_MAPS, MapEntry};
use map_preview_cache::MapPreviewCache;
use map_preview_pictures::MapPreviewPictures;
use output_card::OutputCard;
pub use pending_drop::PendingDrop;
use planned_output::PlannedOutput;
use row_edit::RowEdit;
use source_pick::SourcePick;
use swatch_look::SwatchLook;

const RENAME_ID: &str = "exp_rename";
/// Initial name for a new template
const NEW_PRESET: &str = "new_export_preset";

// Layout in points from the window content's top left corner, the side panes from its right edge, measured on the locked mock

/// Width of the Presets column
const PRESETS_WIDTH: f32 = 216.2;
/// Middle of the column titles
const HEADER_Y: f32 = 73.0;
/// Centre of the Presets header's first glyph, from the column's right edge
const PRESET_ICON_FROM_RIGHT: f32 = 14.0;
/// Step between the Presets header's glyphs
const PRESET_ICON_STEP: f32 = 22.0;
/// Size of the Presets header's glyphs
const PRESET_ICON_SIZE: f32 = 13.0;
/// Top of the lists under the titles
const LIST_TOP: f32 = 88.0;
/// Height of a Presets row
const PRESET_ROW_HEIGHT: f32 = 20.0;
/// Centre of a Presets row's file glyph
const PRESET_ICON_X: f32 = 16.0;
/// Where a Presets row's name starts
const PRESET_NAME_LEFT: f32 = 26.3;
/// Size of a Presets row's file glyph
const PRESET_ROW_ICON_SIZE: f32 = 12.0;
/// The column titles' left edge, inside their column
const TITLE_INDENT: f32 = 7.5;

/// The middle column's margin to the dividers on both sides
const MIDDLE_MARGIN: f32 = 10.0;
/// Middle of the Current preset line
const CURRENT_PRESET_Y: f32 = 69.0;
/// Gap after "Current preset:"
const NAME_GAP: f32 = 5.0;
/// From the template's name to its lock's centre
const LOCK_GAP: f32 = 12.0;
/// Middle of the Output maps row
const OUTPUT_MAPS_Y: f32 = 106.0;
/// The air around a Create button's name
const CREATE_PADDING: f32 = 22.0;
/// Gap between the Create buttons
const CREATE_GAP: f32 = 6.0;
/// Gap before "Create:"
const CREATE_LABEL_GAP: f32 = 2.0;
/// Top of the first output map card
const CARDS_TOP: f32 = 127.0;
/// The count strip's top, above the body's bottom
const COUNT_TOP_ABOVE_BOTTOM: f32 = 24.0;
/// The count strip's bottom, above the body's bottom
const COUNT_BOTTOM_ABOVE_BOTTOM: f32 = 1.0;
/// The count's indent in its strip
const COUNT_TEXT_INDENT: f32 = 6.0;

/// Height of an output map card
const CARD_HEIGHT: f32 = 48.0;
/// Gap below a card
const CARD_GAP: f32 = 6.0;
/// Corner radius of a card
const CARD_RADIUS: u8 = 3;
/// Inset of a card's swatches from its top and bottom
const CARD_INSET: f32 = 6.0;
/// Centre of a card's grip
const GRIP_X: f32 = 10.0;
/// Width of the grip's click area
const GRIP_WIDTH: f32 = 16.0;
/// Size of the grip's glyph
const GRIP_SIZE: f32 = 14.0;
/// Where a card's file name field starts
const NAME_FIELD_LEFT: f32 = 20.0;
/// Width of the file name field
const NAME_FIELD_WIDTH: f32 = 150.0;
/// The file name field's least width in a narrow window
const MIN_NAME_FIELD_WIDTH: f32 = 60.0;
/// Gap before the "$" button
const DOLLAR_GAP: f32 = 4.0;
/// Width of the "$" button
const DOLLAR_WIDTH: f32 = 22.0;
/// Gap between "$" and the swatches
const SWATCHES_GAP: f32 = 8.0;
/// The lists' and icons' rows, above and below the card's middle
const CARD_ROW_OFFSET: f32 = 10.0;
/// Centre of a card's icons, from its right edge
const CARD_ICON_FROM_RIGHT: f32 = 14.0;
/// From the icons to the lists' chevrons
const CARD_CHEVRON_STEP: f32 = 22.0;
/// Gap between a list's value and its chevron
const CARD_VALUE_GAP: f32 = 8.0;
/// Size of a card's icons
const CARD_ICON_SIZE: f32 = 12.0;
/// Values the cards' lists line up on, so cards with shorter ones keep the same column
const CARD_VALUE_SAMPLES: [&str; 3] = ["png", "8 bits", "16 bits"];

/// The Input maps divider, from the window's right edge
const INPUTS_DIVIDER_FROM_RIGHT: f32 = 342.3;
/// The Mesh maps divider, from the window's right edge
const SIDE_DIVIDER_FROM_RIGHT: f32 = 161.9;
/// Where the Input maps pane starts after its divider
const PANE_FROM_DIVIDER: f32 = 2.0;
/// Where it ends before the next one
const INPUTS_END_BEFORE_DIVIDER: f32 = 3.5;
/// Where the Mesh maps pane starts after its divider
const SIDE_PANE_FROM_DIVIDER: f32 = 4.0;
/// The divider between Mesh maps and Converted maps, below the body's middle
const SIDE_SPLIT_BELOW_MIDDLE: f32 = 10.0;
/// Gap between Mesh maps' last row and that divider
const MESH_BOTTOM_GAP: f32 = 6.0;
/// Middle of Converted maps' title, below the divider
const CONVERTED_TITLE_Y: f32 = 18.0;
/// Top of Converted maps' rows, below the divider
const CONVERTED_ROWS_TOP: f32 = 33.0;
/// A map entry's inset in its pane
const ENTRY_INSET: f32 = 3.0;
/// Height of a map entry
const ENTRY_HEIGHT: f32 = 20.0;
/// Pitch of the map entries
const ENTRY_PITCH: f32 = 21.0;
/// Height of a group's name row
const GROUP_HEIGHT: f32 = 18.0;
/// Indent of the titles and group names
const GROUP_INDENT: f32 = 4.0;
/// Where a used map's square starts
const MARKER_LEFT: f32 = 9.0;
/// Side of that square
const MARKER_SIZE: f32 = 10.0;
/// Its corner radius
const MARKER_RADIUS: u8 = 1;
/// How much lighter its edge is than its fill
const MARKER_EDGE_LIGHTEN: u8 = 40;
/// Where a map entry's name starts
const ENTRY_NAME_LEFT: f32 = 25.0;
/// How strongly a map TextureCraft does not have yet is dimmed
const UNAVAILABLE_FADE: f32 = 0.6;

/// Naming pattern's info glyph, from the window's right edge
const NAMING_INFO_FROM_RIGHT: f32 = 20.8;
/// Its dropdown's left edge, from the window's right edge
const NAMING_COMBO_LEFT_FROM_RIGHT: f32 = 182.4;
/// Its dropdown's right edge, from the window's right edge
const NAMING_COMBO_RIGHT_FROM_RIGHT: f32 = 34.0;
/// Its label's right edge, from the window's right edge
const NAMING_LABEL_FROM_RIGHT: f32 = 188.0;
/// Size of the info glyph
const INFO_SIZE: f32 = 14.0;

/// The accent-free frame around a drop target: its stroke and its gap to the target
const DROP_HIGHLIGHT_STROKE_POINTS: f32 = 1.5;
const DROP_HIGHLIGHT_GAP_POINTS: f32 = 2.0;
/// Side of the corner mark of an inverted swatch
const INVERTED_MARK_POINTS: f32 = 8.0;
/// A swatch's file preview: the file's width, and the side of each channel's picture beside it
const PREVIEW_FILE_WIDTH_POINTS: f32 = 150.0;
const PREVIEW_CHANNEL_WIDTH_POINTS: f32 = 34.0;

/// Width of a channel swatch; its height is the card's less its inset
const SWATCH_WIDTH: f32 = 34.0;
/// Gap between swatches
const SWATCH_GAP: f32 = 2.0;
/// Middle of a swatch's letters, from its top
const SWATCH_LABEL_Y: f32 = 11.0;
/// A swatch's underline: its inset from the sides
const SWATCH_BAR_INSET: f32 = 6.0;
/// Its top, from the swatch's bottom
const SWATCH_BAR_BOTTOM: f32 = 10.0;
/// Its height
const SWATCH_BAR_HEIGHT: f32 = 2.0;

/// Golden-ratio spacing keeps neighbouring map hues distinct
const HUE_STEP: f32 = 0.618_034;
const SWATCH_SATURATION: f32 = 0.45;
const SWATCH_VALUE: f32 = 0.42;

/// The underline of R
const BAR_RED: Color32 = Color32::from_rgb(230, 70, 70);
/// The underline of G
const BAR_GREEN: Color32 = Color32::from_rgb(80, 200, 90);
/// The underline of B
const BAR_BLUE: Color32 = Color32::from_rgb(80, 130, 240);
/// The underline of A
const BAR_ALPHA: Color32 = Color32::from_rgb(220, 220, 220);
/// The underline of RGB and gray
const BAR_WHOLE: Color32 = Color32::WHITE;

/// What a cleared channel writes: black, or opaque in the alpha channel
const CLEARED_VALUE: f32 = 0.0;
const CLEARED_ALPHA: f32 = 1.0;

/// The reference's file name variables in its order; ones TextureCraft does not have are greyed
const REFERENCE_TOKENS: [&str; 6] = ["$project", "$mesh", "$textureSet", "$udim", "$uvTileName", "$colorSpace"];

/// The file types an output map lists, by extension in the reference order; the ones that
/// TextureCraft cannot write yet are greyed
const MAP_FILE_TYPES: [&str; 14] = ["bmp", "jpg", "jng", "pgm", "png", "tga", "tif", "hdr", "exr", "j2k", "jp2", "webp", "psd", "sbsar"];

/// An output file's channel layout.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Layout {
    Rgb,
    Rgba,
    Gray,
    GrayAlpha,
}

impl Layout {
    const ALL: [Layout; 4] = [Layout::Rgb, Layout::Rgba, Layout::Gray, Layout::GrayAlpha];

    fn label(self) -> &'static str {
        match self {
            Layout::Rgb => "RGB",
            Layout::Rgba => "RGBA",
            Layout::Gray => n_("Gray"),
            Layout::GrayAlpha => n_("Gray + A"),
        }
    }

    fn dests(self) -> &'static [Dest] {
        match self {
            Layout::Rgb => &[Dest::R, Dest::G, Dest::B],
            Layout::Rgba => &[Dest::R, Dest::G, Dest::B, Dest::A],
            Layout::Gray => &[Dest::L],
            Layout::GrayAlpha => &[Dest::L, Dest::A],
        }
    }

    fn of(map: &MapSpec) -> Layout {
        let has = |d: Dest| map.channels.iter().any(|(x, _)| *x == d);
        match (has(Dest::L), has(Dest::A)) {
            (true, true) => Layout::GrayAlpha,
            (true, false) => Layout::Gray,
            (false, true) => Layout::Rgba,
            (false, false) => Layout::Rgb,
        }
    }
}

/// The Create buttons: (label, the layout the new output map gets, whether R, G and
/// B show as separate swatches, the new map's file name)
const CREATE: [(&str, Layout, bool, &str); 5] = [
    (n_("Gray"), Layout::Gray, false, "Gray"),
    ("RGB", Layout::Rgb, false, "RGB"),
    ("R+G+B", Layout::Rgb, true, "RGB"),
    ("RGB+A", Layout::Rgba, false, "RGBA"),
    ("R+G+B+A", Layout::Rgba, true, "RGBA"),
];
/// What a new output map's file name starts with, so each texture set (and mesh) writes a file
/// of its own; the layout's name follows it
const NEW_MAP_PREFIX: &str = "$mesh_$textureSet_";

/// The swatch right-click menu's constants: (label, value)
const CONSTANTS: [(&str, f32); 3] = [(n_("Black"), 0.0), (n_("Gray 50%"), 0.5), (n_("White"), 1.0)];
/// How fast dragging changes a constant
const CONSTANT_DRAG_SPEED: f64 = 0.005;

/// The naming patterns the input maps can be shown with (the reference's second one names
/// another vendor's material and is left out)
const NAMING_PATTERNS: [&str; 1] = ["OpenPBR"];

/// `s` reading component `k` (only 3-component sources have more than one).
fn with_component(s: &SourceRef, k: u8) -> SourceRef {
    let k = if s.components() == 3 { k.min(2) } else { 0 };
    let mut s = s.clone();
    if let SourceRef::Channel { component, .. } | SourceRef::Virtual { component, .. } | SourceRef::Baked { component, .. } = &mut s {
        *component = k;
    }
    s
}

/// The component `s` reads (0 for constants).
fn component_of(s: &SourceRef) -> u8 {
    match s {
        SourceRef::Channel { component, .. } | SourceRef::Virtual { component, .. } | SourceRef::Baked { component, .. } => *component,
        SourceRef::Constant(_) => 0,
    }
}

/// Same input map (any component), or the same constant.
fn same_source(a: &SourceRef, b: &SourceRef) -> bool {
    match (a, b) {
        (SourceRef::Constant(x), SourceRef::Constant(y)) => (x - y).abs() < 1e-4,
        _ => with_component(a, 0) == with_component(b, 0),
    }
}

/// The component a channel reads by default: R, G, B their own, L and A the first.
fn slot_component(d: Dest) -> u8 {
    match d {
        Dest::G => 1,
        Dest::B => 2,
        _ => 0,
    }
}

fn is_rgb(d: Dest) -> bool {
    matches!(d, Dest::R | Dest::G | Dest::B)
}

/// Switch `map` to `layout`, keeping the sources of channels both layouts have; the color
/// source moves between R, G, B and L.
fn set_layout(map: &mut MapSpec, layout: Layout) {
    let get = |d: Dest| map.channels.iter().find(|(x, _)| *x == d).map(|(_, s)| s.clone());
    let color = get(Dest::R).or_else(|| get(Dest::L)).unwrap_or(SourceRef::Constant(0.0));

    // A source that moves to a new channel keeps its inversion
    let color_inverted = map.inverted.contains(if get(Dest::R).is_some() { &Dest::R } else { &Dest::L });
    let was_inverted = |d: Dest| match get(d) {
        Some(_) => map.inverted.contains(&d),
        None => !matches!(d, Dest::A) && color_inverted,
    };
    let inverted: Vec<Dest> = layout.dests().iter().copied().filter(|&d| was_inverted(d)).collect();
    let channels = layout
        .dests()
        .iter()
        .map(|&d| {
            let src = match d {
                Dest::A => get(d).unwrap_or(SourceRef::Constant(1.0)),
                _ => get(d).unwrap_or_else(|| with_component(&color, slot_component(d))),
            };
            (d, src)
        })
        .collect();
    map.channels = channels;
    map.inverted = inverted;
}

/// A new empty png / 8 bits output map named `file`
fn new_map(layout: Layout, file: &str) -> MapSpec {
    let mut map = MapSpec { file: file.into(), format: FileFormat::Png, bit_depth: 8, channels: Vec::new(), inverted: Vec::new() };
    set_layout(&mut map, layout);
    map
}

/// `name`, or with "_1", "_2", ... when a map of `maps` already has it, as the reference names new rows
fn unique_file(maps: &[MapSpec], name: &str) -> String {
    let taken = |file: &str| maps.iter().any(|map| map.file == file);
    if !taken(name) {
        return name.to_string();
    }

    (1..).map(|n| format!("{name}_{n}")).find(|file| !taken(file)).unwrap_or_else(|| name.to_string())
}

/// An input map dropped on channel `dest`: a color or normal input dropped on R, G or B fills
/// all three.
fn drop_input(map: &mut MapSpec, dest: Dest, input: &SourceRef) {
    let fill_rgb = input.components() == 3 && is_rgb(dest);
    for (d, s) in &mut map.channels {
        if *d == dest || (fill_rgb && is_rgb(*d)) {
            *s = with_component(input, slot_component(*d));
        }
    }
}

/// The saved (built-in or user) preset selected in the dialog.
fn selected(app: &TextureCraftApp) -> Option<ExportPreset> {
    app.session.export_preset(&app.ui.export.preset)
}

/// Keep the draft on the selected user preset; a valid draft that differs from the saved preset
/// was changed elsewhere (an agent) and reloads, an invalid one keeps the user's edit.
fn sync_draft(app: &mut TextureCraftApp) {
    let e = &mut app.ui.export;
    match app.session.export_presets.list.iter().find(|p| p.name == e.preset) {
        None => {
            e.draft = None;
            e.draft_maps.clear();
            e.draft_base = None;
            e.draft_split.clear();
            e.pending_drop = None;
        }
        Some(saved) => {
            if e.draft.as_ref().is_none_or(|d| d.name != saved.name || (d != saved && d.validate().is_ok())) {
                e.draft = Some(saved.clone());
                e.draft_maps = (0..saved.maps.len()).map(Some).collect();
                e.draft_base = Some(saved.clone());
                e.draft_split.clear();
                e.pending_drop = None;
            } else if e.draft_base.as_ref() != Some(saved) {
                // Match only unambiguous maps from the previous saved order
                for origin in &mut e.draft_maps {
                    *origin = origin
                        .and_then(|index| {
                            let previous_maps = &e.draft_base.as_ref()?.maps;
                            let previous = previous_maps.get(index)?;
                            previous_maps.iter().filter(|map| *map == previous).nth(1).is_none().then_some(previous)
                        })
                        .and_then(|previous| {
                            let mut matches = saved.maps.iter().enumerate().filter(|(_, map)| *map == previous);
                            let (index, _) = matches.next()?;
                            matches.next().is_none().then_some(index)
                        });
                }
                e.draft_base = Some(saved.clone());
            }
        }
    }
}

pub use crate::ask::Ask;
pub use crate::list_action::ListAction;

enum MapAction {
    /// A new output map with this layout, its R, G and B split or not, and this file name
    Add(Layout, bool, &'static str),
    Move {
        from: usize,
        to: usize,
    },
    Duplicate(usize),
    Remove(usize),
}

pub(crate) fn clear_map_previews(ctx: &egui::Context) {
    MapPreviewCache::clear(ctx);
}

/// The Output templates tab in `body`: presets, outputs, Input maps, then Mesh maps above Converted maps
pub fn tab(app: &mut TextureCraftApp, ui: &mut Ui, body: Rect, builtins: &[ExportPreset]) {
    // Four swatches and the type menu share a row between the two map panes
    const MIN_EDITOR_WIDTH: f32 = 960.0;
    const MIN_EDITOR_HEIGHT: f32 = CARDS_TOP + 2.0 * CARD_HEIGHT + COUNT_TOP_ABOVE_BOTTOM;
    let minimum = vec2(MIN_EDITOR_WIDTH, MIN_EDITOR_HEIGHT);
    if body.width() >= minimum.x && body.height() >= minimum.y {
        template_panes(app, ui, body, builtins);
        return;
    }

    ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
        egui::ScrollArea::both().id_salt("export_template_editor").auto_shrink([false; 2]).show(ui, |ui| {
            let (content, _) = ui.allocate_exact_size(ui.available_size().max(minimum), egui::Sense::hover());
            template_panes(app, ui, content, builtins);
        });
    });
}

fn template_panes(app: &mut TextureCraftApp, ui: &mut Ui, body: Rect, builtins: &[ExportPreset]) {
    sync_draft(app);
    let top = body.top() - TAB_ROW_HEIGHT;
    let names = crate::export_window::template_groups(app, builtins).1;
    let presets_right = body.left() + PRESETS_WIDTH;
    let inputs_divider = body.right() - INPUTS_DIVIDER_FROM_RIGHT;
    let side_divider = body.right() - SIDE_DIVIDER_FROM_RIGHT;
    for x in [presets_right, inputs_divider, side_divider] {
        drawn::divider(ui, Rect::from_min_max(pos2(x, body.top()), pos2(x + 1.0, body.bottom())));
    }

    let list_action = preset_list(app, ui, Rect::from_min_max(body.min, pos2(presets_right, body.bottom())), top, &names);
    let middle = Rect::from_min_max(pos2(presets_right + 1.0 + MIDDLE_MARGIN, body.top()), pos2(inputs_divider - MIDDLE_MARGIN, body.bottom()));
    ui.scope(|ui| outputs(app, ui, middle, top));

    // The panes mark the maps the selected template's outputs read
    let marked = marked_sources(app);
    let inputs = Rect::from_min_max(pos2(inputs_divider + PANE_FROM_DIVIDER, body.top()), pos2(side_divider - INPUTS_END_BEFORE_DIVIDER, body.bottom()));
    input_maps(ui, inputs, top, &marked);
    let side = Rect::from_min_max(pos2(side_divider + SIDE_PANE_FROM_DIVIDER, body.top()), body.max);
    mesh_and_converted_maps(ui, side, top, &marked);
    if let Some(a) = list_action {
        apply(app, ui.ctx(), a);
    }
}

/// Select template `name` from outside the list (the Settings tab's dropdown); an unfinished
/// edit of the current one is asked about first.
pub fn select(app: &mut TextureCraftApp, name: String) {
    if name != app.ui.export.preset && unfinished(app).is_some() {
        app.ui.export.ask = Some(Ask::Leave(ListAction::Select(name)));
    } else {
        app.ui.export.preset = name;
    }
}

/// "Naming pattern", its list and its info glyph at the tab row's right end
pub(crate) fn naming_pattern(ui: &mut Ui, window_right: f32, y: f32) {
    let look = ExportLook::get(ui.ctx());
    let info_at = pos2(window_right - NAMING_INFO_FROM_RIGHT, y);
    drawn::glyph(ui, info_at, ic::INFO, INFO_SIZE, look.text_dim);
    let info = ui.interact(Rect::from_center_size(info_at, vec2(INFO_SIZE, INFO_SIZE)), ui.scope_id().with("exp_naming_info"), egui::Sense::hover());
    info.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, ic::INFO));
    info.on_hover_ui(|ui| {
        ui.label(tr("Choose how the input maps are named in this editor."));
        ui.label(tr("The active texture set’s shader can affect the available map names."));
    });
    let combo = drawn::control_rect(window_right - NAMING_COMBO_LEFT_FROM_RIGHT, window_right - NAMING_COMBO_RIGHT_FROM_RIGHT, y);
    drawn::combo(ui, combo, "exp_naming", NAMING_PATTERNS[0], |ui| {
        for p in NAMING_PATTERNS {
            let _ = ui.selectable_label(true, p);
        }
    });
    let label = tr("Naming pattern");
    let left = window_right - NAMING_LABEL_FROM_RIGHT - drawn::text_width(ui, label, false);
    drawn::label_at(ui, left, y, label, look.text, false);
}

/// The Presets list in `column` with its header's tools; a right click offers Rename, Duplicate and Remove, F2 renames
fn preset_list(app: &mut TextureCraftApp, ui: &mut Ui, column: Rect, top: f32, names: &[String]) -> Option<ListAction> {
    let look = ExportLook::get(ui.ctx());
    let mut act = None;
    let selected_name = app.ui.export.preset.clone();
    let user = app.ui.export.draft.is_some();
    drawn::label_at(ui, column.left() + TITLE_INDENT, top + HEADER_Y, tr("Presets"), look.text, true);

    // Template actions, Duplicate, "−" and "+" from left to right, without tooltips as in the reference
    let icon_at = |slot_from_right: f32| pos2(column.right() - PRESET_ICON_FROM_RIGHT - slot_from_right * PRESET_ICON_STEP, top + HEADER_Y);
    if drawn::icon_button(ui, icon_at(0.0), ic::PLUS, PRESET_ICON_SIZE, tr("New template"), true, false).clicked() {
        act = Some(ListAction::New);
    }
    if drawn::icon_button(ui, icon_at(1.0), ic::MINUS, PRESET_ICON_SIZE, tr("Delete the selected template"), user, false).clicked() {
        act = Some(ListAction::Remove(selected_name.clone()));
    }
    if drawn::icon_button(ui, icon_at(2.0), ic::COPY, PRESET_ICON_SIZE, tr("Duplicate the selected template"), true, false).clicked() {
        act = Some(ListAction::Duplicate(selected_name.clone()));
    }

    // TextureCraft's template files: a focusable menu, as the row's right click is pointer only
    let actions = drawn::icon_button(ui, icon_at(3.0), ic::DOTS_THREE, PRESET_ICON_SIZE, tr("Template actions"), true, false);
    let opened_now = actions.clicked();
    egui::Popup::menu(&actions).id(ui.make_persistent_id("exp_template_actions")).show(|ui| template_file_items(ui, &mut act, true, opened_now));
    if std::mem::take(&mut app.ui.export.rename_key) && user {
        act = Some(ListAction::StartRename(selected_name.clone()));
    }

    // The rows scroll, the scroll bar in its strip at the column's right
    let rows = Rect::from_min_max(pos2(column.left(), top + LIST_TOP), column.max);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rows), |ui| {
        egui::ScrollArea::vertical().id_salt("exp_presets").auto_shrink([false; 2]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let width = rows.width() - GUTTER;
            for name in names {
                let is_user = app.session.export_presets.list.iter().any(|preset| preset.name == *name);
                let (rect, _) = ui.allocate_exact_size(vec2(width, PRESET_ROW_HEIGHT), egui::Sense::hover());
                match app.ui.export.renaming.as_mut().filter(|_| *name == selected_name) {
                    Some(name_buffer) => {
                        let field = egui::TextEdit::singleline(name_buffer).id(egui::Id::unique(RENAME_ID)).margin(vec2(PRESET_NAME_LEFT, 0.0));
                        let rename_field = ui.put(rect, field);
                        if rename_field.lost_focus() {
                            act = Some(if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                                ListAction::Select(name.clone())
                            } else {
                                ListAction::Rename(name_buffer.clone())
                            });
                        }
                    }
                    None => {
                        let label = template_label(&app.session, name);
                        let response = preset_row(ui, rect, name, label, *name == selected_name, is_user).on_hover_text(name);
                        if response.double_clicked() && is_user {
                            act = Some(ListAction::StartRename(name.clone()));
                        } else if response.clicked() {
                            act = Some(ListAction::Select(name.clone()));
                        }
                        response.context_menu(|ui| {
                            let read_only = tr("Built-in templates are read-only: duplicate one to edit it");
                            if ui.add_enabled(is_user, egui::Button::new(tr("Rename"))).on_disabled_hover_text(read_only).clicked() {
                                act = Some(ListAction::StartRename(name.clone()));
                            }
                            if ui.button(tr("Duplicate")).clicked() {
                                act = Some(ListAction::Duplicate(name.clone()));
                            }
                            if ui.add_enabled(is_user, egui::Button::new(tr("Remove"))).on_disabled_hover_text(read_only).clicked() {
                                act = Some(ListAction::Remove(name.clone()));
                            }

                            // TextureCraft's template files, after the reference's entries
                            ui.separator();
                            template_file_items(ui, &mut act, *name == selected_name, false);
                        });
                    }
                }
            }
        });
    });
    act
}

/// Import, and Export of the selected template when `with_export`; `focus_first` moves the keyboard
/// focus to Import so a menu opened from the keyboard can be used without the pointer. A key press
/// does not close a menu like a click does, so the menu closes itself
fn template_file_items(ui: &mut Ui, act: &mut Option<ListAction>, with_export: bool, focus_first: bool) {
    let import = ui.button(tr("Import a template (.json)"));
    if focus_first {
        import.request_focus();
    }
    if import.clicked() {
        *act = Some(ListAction::Import);
        ui.close();
    }
    if with_export && ui.button(tr("Export the selected template (.json)")).clicked() {
        *act = Some(ListAction::Export);
        ui.close();
    }
}

/// A Presets row: a locked file glyph for a built-in template, the name cut with "…", the selected one in bold
fn preset_row(ui: &mut Ui, rect: Rect, name: &str, label: &str, selected: bool, is_user: bool) -> egui::Response {
    let look = ExportLook::get(ui.ctx());
    let response = ui.interact(rect, ui.scope_id().with(("exp_preset_row", name)), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 0.0, look.chrome);
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, look.raised);
    }
    let icon = if is_user { ic::FILE_TEXT } else { ic::FILE_LOCK };
    drawn::glyph(ui, pos2(rect.left() + PRESET_ICON_X, rect.center().y), icon, PRESET_ROW_ICON_SIZE, look.text_dim);
    let shown = drawn::fitted(ui, label, rect.width() - PRESET_NAME_LEFT, false, selected);
    let color = if selected { look.text } else { look.list_text };
    drawn::text_at(ui, rect.left() + PRESET_NAME_LEFT, rect.center().y, &shown, color, selected);
    drawn::paint_focus(ui, &response);
    response
}

/// The error of the selected template's draft when it does not validate: that edit is not saved.
fn unfinished(app: &TextureCraftApp) -> Option<String> {
    app.ui.export.draft.as_ref().and_then(|d| d.validate().err()).map(|e| e.to_string())
}

/// Run `act`; deleting a template and leaving one with an unfinished edit ask first.
fn apply(app: &mut TextureCraftApp, ctx: &egui::Context, act: ListAction) {
    let leaves = match &act {
        ListAction::Select(name) | ListAction::StartRename(name) => *name != app.ui.export.preset,
        ListAction::New | ListAction::Duplicate(_) | ListAction::Imported(_) => true,
        _ => false,
    };
    if let ListAction::Remove(name) = &act {
        app.ui.export.ask = Some(Ask::Delete(name.clone()));
    } else if leaves && unfinished(app).is_some() {
        app.ui.export.ask = Some(Ask::Leave(act));
    } else {
        perform(app, ctx, act);
    }
}

fn perform(app: &mut TextureCraftApp, ctx: &egui::Context, act: ListAction) {
    app.ui.export.renaming = None;
    match act {
        ListAction::Select(name) => app.ui.export.preset = name,
        ListAction::New => {
            save_new(app, &ExportPreset { name: NEW_PRESET.into(), maps: Vec::new() });
        }
        ListAction::Duplicate(name) => {
            if let Some(p) = app.session.export_preset(&name) {
                let name = format!("{}_copy", template_label(&app.session, &name)).chars().take(MAX_PRESET_NAME).collect();
                save_new(app, &ExportPreset { name, ..p });
            }
        }
        ListAction::StartRename(name) => {
            app.ui.export.preset = name.clone();
            app.ui.export.renaming = Some(name);
            ctx.memory_mut(|m| m.request_focus(egui::Id::unique(RENAME_ID)));
        }
        ListAction::Rename(name) => rename(app, name.trim()),
        ListAction::Remove(name) => remove(app, name),
        ListAction::Import => import_file(app, ctx),
        ListAction::Imported(p) => add_imported(app, &p),
        ListAction::Export => export_file(app),
    }
}

fn remove(app: &mut TextureCraftApp, name: String) {
    if app.run("export.preset.remove", json!({ "name": name })).is_some() {
        if app.ui.export.preset == name {
            app.ui.export.preset = crate::state::ExportUi::default().preset;
        }
        crate::export_window::template_renamed(&mut app.ui.export, &name, None);
        app.toast(trf("Deleted template \"{name}\"", &[("name", &name)]), false);
    }
}

/// The open question, if any: Escape or Cancel keeps things as they are.
pub(crate) fn ask(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let Some(question) = app.ui.export.ask.clone() else { return };

    // The edit was fixed while the question was open: nothing is dropped any more
    if matches!(question, Ask::Leave(_)) && unfinished(app).is_none() {
        app.ui.export.ask = None;
        return;
    }

    let mut cancel = false;
    let mut yes = false;
    let (title, text, yes_label, no_label) = match &question {
        Ask::Delete(name) => (
            tr("Delete Template"),
            trf("Delete the template \"{name}\"? Its file is removed and this cannot be undone.", &[("name", name)]),
            tr("Delete"),
            tr("Cancel"),
        ),
        Ask::Leave(_) => (
            tr("Unfinished Template Edit"),
            trf(
                "The last edit of the template \"{name}\" is not saved: {error}. Switching away drops it.",
                &[("name", &app.ui.export.preset), ("error", &unfinished(app).unwrap_or_default())],
            ),
            tr("Discard changes"),
            tr("Keep editing"),
        ),
    };
    let mut open = true;
    crate::windows::dialog(ctx, "export_template_ask", title, &mut open).default_width(360.0).show(ctx, |ui| {
        ui.label(text);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            yes = ui.button(yes_label).clicked();
            cancel |= ui.button(no_label).clicked();
        });
    });
    if yes {
        app.ui.export.ask = None;
        match question {
            Ask::Delete(name) => remove(app, name),
            Ask::Leave(act) => perform(app, ctx, act),
        }
    } else if cancel || !open {
        app.ui.export.ask = None;
    }
}

/// Save `preset` as a new user preset (renamed when its name is taken) and select it.
fn save_new(app: &mut TextureCraftApp, preset: &ExportPreset) -> Option<String> {
    let v = serde_json::to_value(preset).ok()?;
    let r = app.run("export.preset.save", json!({ "preset": v, "unique": true }))?;
    let name = r.get("name").and_then(Value::as_str)?.to_string();
    app.ui.export.preset = name.clone();
    Some(name)
}

fn rename(app: &mut TextureCraftApp, name: &str) {
    let Some(mut p) = app.ui.export.draft.clone() else { return };
    if name.is_empty() || name == p.name {
        return;
    }

    let old = std::mem::replace(&mut p.name, name.to_string());
    let Ok(v) = serde_json::to_value(&p) else { return };
    if let Some(r) = app.run("export.preset.save", json!({ "preset": v, "rename": old }))
        && let Some(n) = r.get("name").and_then(Value::as_str)
    {
        app.ui.export.preset = n.to_string();
        crate::export_window::template_renamed(&mut app.ui.export, &old, Some(n));
    }
}

/// Add an imported preset (renamed when its name is taken) and select it.
pub fn import(app: &mut TextureCraftApp, preset: &ExportPreset) {
    if unfinished(app).is_some() {
        app.ui.export.ask = Some(Ask::Leave(ListAction::Imported(preset.clone())));
    } else {
        add_imported(app, preset);
    }
}

fn add_imported(app: &mut TextureCraftApp, preset: &ExportPreset) {
    if let Some(name) = save_new(app, preset) {
        app.toast(trf("Imported template \"{name}\"", &[("name", &name)]), false);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn import_file(app: &mut TextureCraftApp, _ctx: &egui::Context) {
    use crate::file_dialogs::{Bound, FileDialog};
    app.ask_file(FileDialog::open().filter(tr("Export template"), &["json"]), Bound::App, |app, paths| {
        let Some(path) = paths.first() else { return };
        match texturecraft_engine::formats::load_preset(path) {
            Ok(p) => import(app, &p),
            Err(e) => app.toast(trf("Couldn't import {name}: {error}", &[("name", &path.display()), ("error", &e)]), true),
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn import_file(_app: &mut TextureCraftApp, ctx: &egui::Context) {
    crate::web_files::import_preset(ctx);
}

fn export_file(app: &mut TextureCraftApp) {
    let Some(p) = selected(app) else { return };
    let name = format!("{}.json", sanitize_file_stem(&p.name));
    let json = match texturecraft_engine::formats::preset_to_json(&p) {
        Ok(json) => json,
        Err(e) => {
            export_failed(app, &e);
            return;
        }
    };
    write_json(app, p.name, name, json);
}

fn export_failed(app: &mut TextureCraftApp, error: &dyn std::fmt::Display) {
    app.toast(trf("Couldn't export the template: {error}", &[("error", error)]), true);
}

fn exported(app: &mut TextureCraftApp, name: &str, to: &str) {
    app.toast(trf("Exported template \"{name}\" to {path}", &[("name", &name), ("path", &to)]), false);
}

/// Asks where to save `json` with the native dialog.
#[cfg(not(target_arch = "wasm32"))]
fn write_json(app: &mut TextureCraftApp, preset: String, name: String, json: String) {
    use crate::file_dialogs::{Bound, FileDialog};
    app.ask_file(FileDialog::save().filter(tr("Export template"), &["json"]).file_name(name), Bound::App, move |app, paths| {
        let Some(path) = paths.first() else { return };
        match std::fs::write(path, &json) {
            Ok(()) => exported(app, &preset, &path.display().to_string()),
            Err(e) => export_failed(app, &e),
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn write_json(app: &mut TextureCraftApp, preset: String, name: String, json: String) {
    match crate::web_files::download(&name, json.as_bytes()) {
        Ok(()) => exported(app, &preset, &name),
        Err(e) => export_failed(app, &e),
    }
}

/// Read the user's templates: the library folder on the desktop, the app storage on the web.
pub fn load_library(app: &mut TextureCraftApp, storage: Option<&dyn eframe::Storage>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = storage;
        let skipped = app.session.load_export_presets(crate::user_dir("export-presets"));
        app.session.log.warnings(texturecraft_engine::log::LIBRARY_SOURCE, skipped.into_iter().map(|w| format!("Export template skipped: {w}")));
    }
    #[cfg(target_arch = "wasm32")]
    for preset in storage.and_then(|s| eframe::get_value::<Vec<ExportPreset>>(s, STORAGE_KEY)).unwrap_or_default() {
        if let Err(error) = app.session.export_presets.restore(preset) {
            app.session.log.warnings(texturecraft_engine::log::LIBRARY_SOURCE, [format!("Export template skipped: {error}")]);
        }
    }
}

#[cfg(target_arch = "wasm32")]
const STORAGE_KEY: &str = "export_presets";

/// Keep the user's templates in the app storage (the web has no library folder).
pub fn save_library(app: &TextureCraftApp, storage: &mut dyn eframe::Storage) {
    #[cfg(target_arch = "wasm32")]
    eframe::set_value(storage, STORAGE_KEY, &app.session.export_presets.list);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (app, storage);
}

/// The selected template's output maps in `column`, editable for user templates
fn outputs(app: &mut TextureCraftApp, ui: &mut Ui, column: Rect, top: f32) {
    let look = ExportLook::get(ui.ctx());
    let tokens = Tokens::get(ui.ctx());
    let editable = app.ui.export.draft.is_some();
    let Some(mut preset) = app.ui.export.draft.clone().or_else(|| selected(app)) else {
        MapPreviewCache::clear(ui.ctx());
        drawn::label_at(ui, column.left(), top + CURRENT_PRESET_Y, tr("Select an output template"), look.text_dim, false);
        return;
    };

    let params = plan_params(app, &preset);
    let preview = match &params {
        Ok(params) => MapPreviewCache::plan(app, ui.ctx(), params),
        Err(error) => {
            MapPreviewCache::clear(ui.ctx());
            Err(error.clone())
        }
    };
    let mut edit = RowEdit::default();

    // "Current preset:" in bold, the template's name, a lock on a built-in one
    let y = top + CURRENT_PRESET_Y;
    let title = tr("Current preset:");
    let name_left = column.left() + drawn::text_width(ui, title, true) + NAME_GAP;
    drawn::label_at(ui, column.left(), y, title, look.text, true);
    let name = drawn::label_at(ui, name_left, y, template_label(&app.session, &app.ui.export.preset), look.text, false);
    if !editable {
        let lock_at = pos2(name.rect.right() + LOCK_GAP, y);
        drawn::glyph(ui, lock_at, ic::LOCK_SIMPLE, CARD_ICON_SIZE, look.text_dim);
        let lock =
            ui.interact(Rect::from_center_size(lock_at, vec2(CARD_ICON_SIZE, CARD_ICON_SIZE)), ui.scope_id().with("exp_preset_lock"), egui::Sense::hover());
        lock.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, tr("Read-only template")));
        lock.on_hover_text(tr("Built-in templates are read-only: duplicate one to edit it"));
    }

    // A built-in template's cards and buttons keep their full look: the lock says they are read-only
    ui.visuals_mut().disabled_alpha = 1.0;
    let y = top + OUTPUT_MAPS_Y;
    drawn::label_at(ui, column.left(), y, tr("Output maps"), look.text, true);
    let mut right = column.right();
    for (label, layout, split, file) in CREATE.iter().rev() {
        let width = drawn::text_width(ui, tr(label), false) + CREATE_PADDING;
        if drawn::button(ui, drawn::control_rect(right - width, right, y), tr(label), false, editable).clicked() {
            edit.action = Some(MapAction::Add(*layout, *split, file));
        }
        right -= width + CREATE_GAP;
    }
    let create = tr("Create:");
    drawn::label_at(ui, right - CREATE_LABEL_GAP - drawn::text_width(ui, create, false), y, create, look.text, false);

    let mut changed = false;
    let map_count = preset.maps.len();
    let mut split = app.ui.export.draft_split.clone();
    split.resize(map_count, false);

    // The cards scroll between the header and the count strip; their lists line up on one column
    let strip =
        Rect::from_min_max(pos2(column.left(), column.bottom() - COUNT_TOP_ABOVE_BOTTOM), pos2(column.right(), column.bottom() - COUNT_BOTTOM_ABOVE_BOTTOM));
    let cards = Rect::from_min_max(pos2(column.left(), top + CARDS_TOP), pos2(column.right(), strip.top() - CARD_GAP));
    let values =
        preset.maps.iter().flat_map(|map| [map.format.extension().to_string(), crate::export_window::depth_label(Some(map.format), map.bit_depth, false)]);
    let value_width = values.chain(CARD_VALUE_SAMPLES.map(str::to_string)).map(|value| drawn::text_width(ui, &value, false)).fold(0.0, f32::max);
    ui.scope_builder(egui::UiBuilder::new().max_rect(cards), |ui| {
        egui::ScrollArea::vertical().id_salt("exp_outputs").auto_shrink([false; 2]).show(ui, |ui| {
            let height = map_count as f32 * (CARD_HEIGHT + CARD_GAP);
            let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::hover());
            ui.add_enabled_ui(editable, |ui| {
                for ((index, map), row_split) in preset.maps.iter_mut().enumerate().zip(split.iter_mut()) {
                    let planned = preview.as_ref().ok().and_then(|outputs| outputs.get(index)).and_then(Option::as_ref);
                    let top = area.top() + index as f32 * (CARD_HEIGHT + CARD_GAP);
                    let card = Rect::from_min_size(pos2(area.left(), top), vec2(area.width(), CARD_HEIGHT));
                    let row = OutputCard { index, count: map_count, value_width };
                    changed |= output_row(ui, card, &row, map, row_split, planned, &mut edit);
                }
            });
        });
    });

    // The hovered output's pictures show from the next frame on
    if let (Some(index), Ok(params), Ok(plan)) = (edit.hovered_output, &params, &preview)
        && plan.get(index).is_some_and(Option::is_some)
    {
        MapPreviewCache::fill(app, ui.ctx(), params, index);
    }

    // Count only maps that have sources in the current set; an edit that cannot be saved says why there
    ui.painter().rect_filled(strip, 0.0, look.control);
    let text_left = strip.left() + COUNT_TEXT_INDENT;
    match &preview {
        Ok(planned) => {
            let exported = planned.iter().flatten().count();
            drawn::label_at(ui, text_left, strip.center().y, &trf("{n} output maps will be exported", &[("n", &exported)]), look.text, false);
        }
        Err(e) => {
            drawn::label_at(ui, text_left, strip.center().y, &trf("Not saved: {error}", &[("error", e)]), tokens.danger, false);
        }
    }
    if !editable {
        return;
    }

    // A drop asks for the source channel from the next frame on; one waiting from before shows now
    let mut pending = app.ui.export.pending_drop.take();
    if edit.pending.is_some() {
        pending = edit.pending.take();
        ui.ctx().request_repaint();
    } else {
        changed |= source_channel_popup(ui, &mut pending, &mut preset.maps);
    }

    // Where each map came from in the saved template, moved along with the maps
    let mut origin = app.ui.export.draft_maps.clone();
    if origin.len() != map_count {
        origin = (0..map_count).map(Some).collect();
    }
    let acted = match edit.action {
        Some(MapAction::Add(layout, split_rgb, file)) => {
            let file = unique_file(&preset.maps, &format!("{NEW_MAP_PREFIX}{file}"));
            preset.maps.push(new_map(layout, &file));
            origin.push(None);
            split.push(split_rgb);
            true
        }
        Some(MapAction::Move { from, to }) if from < map_count && to < map_count && from != to => {
            let map = preset.maps.remove(from);
            preset.maps.insert(to, map);
            let map_origin = origin.remove(from);
            origin.insert(to, map_origin);
            let map_split = split.remove(from);
            split.insert(to, map_split);
            true
        }
        Some(MapAction::Duplicate(index)) if index < map_count => {
            let copy = preset.maps.get(index).cloned();
            let copy_split = split.get(index).copied().unwrap_or(false);
            if let Some(copy) = copy {
                preset.maps.insert(index + 1, copy);
                origin.insert(index + 1, None);
                split.insert(index + 1, copy_split);
            }
            true
        }
        Some(MapAction::Remove(index)) if index < map_count => {
            preset.maps.remove(index);
            origin.remove(index);
            split.remove(index);
            true
        }
        _ => false,
    };

    // Indices changed under a waiting drop: it is dropped
    if acted {
        pending = None;
    }

    app.ui.export.draft_split = split;
    app.ui.export.pending_drop = pending;

    if acted || changed {
        commit(app, preset, origin);
    }
}

/// Keep the edited draft; save it when valid. `origin` gives each map's index in the saved
/// template: once saved, the texture set overrides of the maps follow them.
fn commit(app: &mut TextureCraftApp, preset: ExportPreset, mut origin: Vec<Option<usize>>) {
    if preset.validate().is_ok()
        && let Ok(v) = serde_json::to_value(&preset)
        && app.run("export.preset.save", json!({ "preset": v })).is_some()
    {
        crate::export_window::template_maps_moved(&mut app.ui.export, &preset.name, &origin);
        origin = (0..preset.maps.len()).map(Some).collect();
        app.ui.export.draft_base = app.session.export_preset(&preset.name);
    }
    app.ui.export.draft = Some(preset);
    app.ui.export.draft_maps = origin;
}

/// The `export.plan` and `export.preview` params of `preset` for the current texture set with the window's settings
fn plan_params(app: &TextureCraftApp, preset: &ExportPreset) -> Result<Value, String> {
    let mut params = crate::export_window::set_params(app, app.session.ts());
    let export = &app.ui.export;
    let same_template = params.get("preset").and_then(Value::as_str) == Some(export.preset.as_str());

    if !same_template && let Some(params) = params.as_object_mut() {
        params.remove("maps");
    } else if let (Some(_), Some(maps)) = (&export.draft, params.get_mut("maps").and_then(Value::as_array_mut)) {
        // Draft positions have no override until they map to a saved output
        maps.retain_mut(|settings| {
            let Some(saved_index) = settings["map"].as_u64().and_then(|index| usize::try_from(index).ok()) else { return false };
            let Some(draft_index) = export.draft_maps.iter().position(|origin| *origin == Some(saved_index)) else { return false };

            settings["map"] = json!(draft_index);
            true
        });
    }

    params["preset"] = serde_json::to_value(preset).map_err(|error| error.to_string())?;
    Ok(params)
}

/// The file each map of `params`' template writes for the texture set (None: no sources), via `export.plan`; a PSD's maps each get the file and their layer
fn planned_outputs(app: &mut TextureCraftApp, params: &Value) -> Result<Vec<Option<PlannedOutput>>, String> {
    let plan = texturecraft_engine::execute(&mut app.session, "export.plan", params).map_err(|e| e.to_string())?;
    let map_count = params.pointer("/preset/maps").and_then(Value::as_array).map_or(0, Vec::len);
    let index = |value: &Value| value.get("map").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());

    let mut outputs: Vec<Option<PlannedOutput>> = vec![None; map_count];
    let tile_files = tr("UV tile files");
    for file in plan.as_array().into_iter().flatten() {
        let Some(name) = file.get("file").and_then(Value::as_str) else { continue };

        let tiled = file["tile"].is_number();
        let display_name = if tiled { file["udimPattern"].as_str().unwrap_or(tile_files) } else { name };
        let mut remember = |entry: &Value, layer_name: Option<&str>| {
            let Some(slot) = index(entry).and_then(|index| outputs.get_mut(index)) else { return };

            if let Some(previous) = slot {
                // Some disambiguated or truncated paths have no common tile pattern
                if tiled && previous.file != display_name && previous.file != tile_files {
                    previous.file = tile_files.into();
                }
                return;
            }

            *slot = Some(PlannedOutput {
                file: match layer_name {
                    Some(layer_name) if !tiled => format!("{name} › {layer_name}"),
                    _ => display_name.to_owned(),
                },
                tiled,
                size: file.get("size").and_then(Value::as_u64).unwrap_or(0),
                format: file.get("format").and_then(Value::as_str).and_then(FileFormat::from_extension),
                bit_depth: file.get("bitDepth").and_then(Value::as_u64).and_then(|depth| u8::try_from(depth).ok()).unwrap_or(0),
                dithering: file.get("dithering").and_then(Value::as_bool).unwrap_or(false),
            });
        };
        match file.get("layers").and_then(Value::as_array).filter(|layers| !layers.is_empty()) {
            Some(layers) => {
                for layer in layers {
                    remember(layer, layer.get("name").and_then(Value::as_str));
                }
            }
            None => remember(file, None),
        }
    }

    Ok(outputs)
}

/// One output map card: grip, file name, "$", swatches, then file type over bit depth beside duplicate and remove
fn output_row(ui: &mut Ui, card: Rect, row: &OutputCard, map: &mut MapSpec, split: &mut bool, planned: Option<&PlannedOutput>, edit: &mut RowEdit) -> bool {
    let look = ExportLook::get(ui.ctx());
    let index = row.index;

    // The card takes a dragged row before its controls, which sit above it
    let card_response = ui.interact(card, ui.scope_id().with(("exp_card", index)), egui::Sense::hover());
    ui.painter().rect_filled(card, CARD_RADIUS, look.control);
    let mid = card.center().y;
    let mut changed = false;
    grip(ui, Rect::from_center_size(pos2(card.left() + GRIP_X, mid), vec2(GRIP_WIDTH, card.height())), index, row.count, edit);

    // The name field gives way in a narrow window, so the swatches and lists keep their room
    let swatches_width = swatch_targets(map, *split).len() as f32 * (SWATCH_WIDTH + SWATCH_GAP);
    let lists_width = CARD_ICON_FROM_RIGHT + CARD_CHEVRON_STEP + CARD_VALUE_GAP + row.value_width + SWATCHES_GAP;
    let room = card.width() - NAME_FIELD_LEFT - DOLLAR_GAP - DOLLAR_WIDTH - SWATCHES_GAP - swatches_width - lists_width;
    let name_width = room.clamp(MIN_NAME_FIELD_WIDTH, NAME_FIELD_WIDTH);
    let name_rect = drawn::control_rect(card.left() + NAME_FIELD_LEFT, card.left() + NAME_FIELD_LEFT + name_width, mid);
    changed |= name_field(ui, name_rect, index, &mut map.file, planned);
    let dollar = drawn::control_rect(name_rect.right() + DOLLAR_GAP, name_rect.right() + DOLLAR_GAP + DOLLAR_WIDTH, mid);
    changed |= token_menu(ui, dollar, index, &mut map.file);
    let swatches = Rect::from_min_max(pos2(dollar.right() + SWATCHES_GAP, card.top() + CARD_INSET), pos2(card.right(), card.bottom() - CARD_INSET));
    changed |= channel_swatches(ui, swatches, index, map, split, planned, edit);

    // The right block: file type over bit depth on one column, duplicate over remove beside them
    let icon_x = card.right() - CARD_ICON_FROM_RIGHT;
    let chevron_x = icon_x - CARD_CHEVRON_STEP;
    let value_left = chevron_x - CARD_VALUE_GAP - row.value_width;
    let (upper, lower) = (mid - CARD_ROW_OFFSET, mid + CARD_ROW_OFFSET);
    changed |= format_menu(ui, (value_left, chevron_x, upper), index, map);
    changed |= depth_menu(ui, (value_left, chevron_x, lower), index, map);
    let duplicate = trf("Duplicate output {number}", &[("number", &(index + 1))]);
    if drawn::icon_button(ui, pos2(icon_x, upper), ic::COPY, CARD_ICON_SIZE, &duplicate, ui.is_enabled(), false).clicked() {
        edit.action = Some(MapAction::Duplicate(index));
    }
    let remove = trf("Remove output {number}", &[("number", &(index + 1))]);
    if drawn::icon_button(ui, pos2(icon_x, lower), ic::X, CARD_ICON_SIZE, &remove, ui.is_enabled(), false).clicked() {
        edit.action = Some(MapAction::Remove(index));
    }

    // Another card's grip dropped here moves that card here (the type check keeps a map list drop)
    let row_dragged_over = card_response.dnd_hover_payload::<MapDrag>().is_some_and(|from| from.0 != index);
    if row_dragged_over {
        drop_highlight(ui, card);
    }
    if row_dragged_over && let Some(from) = card_response.dnd_release_payload::<MapDrag>() {
        edit.action = Some(MapAction::Move { from: from.0, to: index });
    }
    changed
}

/// The file name field; until it is typed in it shows the name's end, cut with "…"; true when it changed
fn name_field(ui: &mut Ui, rect: Rect, index: usize, file: &mut String, planned: Option<&PlannedOutput>) -> bool {
    let look = ExportLook::get(ui.ctx());
    let id = ui.scope_id().with(("exp_map_name", index));
    let typing = ui.memory(|memory| memory.has_focus(id));
    let text_color = if typing { look.text } else { Color32::TRANSPARENT };
    let field = egui::TextEdit::singleline(file).id(id).margin(vec2(drawn::VALUE_INSET, 0.0)).background_color(look.control).text_color(text_color);
    let response = ui.put(rect, field).on_hover_ui(|ui| {
        ui.label(tr("File name without extension; variables insert names"));
        ui.label(planned_text(planned));
    });
    if !typing {
        let shown = drawn::fitted(ui, file, rect.width() - 2.0 * drawn::VALUE_INSET, true, false);
        let color = if drawn::looks_disabled(ui, ui.is_enabled()) { look.text_dim } else { look.text };
        drawn::text_at(ui, rect.left() + drawn::VALUE_INSET, rect.center().y, &shown, color, false);
    }
    response.changed()
}

/// A card's grip: drag it onto another card, or use the arrow keys or its menu, to move the map
fn grip(ui: &mut Ui, rect: Rect, index: usize, count: usize, edit: &mut RowEdit) {
    let look = ExportLook::get(ui.ctx());
    let enabled = ui.is_enabled();
    let response = ui.interact(rect, ui.scope_id().with(("exp_grip", index)), if enabled { egui::Sense::click_and_drag() } else { egui::Sense::hover() });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, tr("Reorder")));
    let color = if enabled && (response.hovered() || response.dragged()) { look.text } else { look.text_dim };
    drawn::glyph(ui, rect.center(), ic::DOTS_SIX_VERTICAL, GRIP_SIZE, color);
    drawn::paint_focus(ui, &response);
    let response = response.on_hover_cursor(egui::CursorIcon::Grab);
    response.dnd_set_drag_payload(MapDrag(index));

    // Up and down arrows move the focused grip's map instead of the focus
    let mut to = None;
    if response.has_focus() {
        let arrows = egui::EventFilter { vertical_arrows: true, ..Default::default() };
        ui.memory_mut(|memory| memory.set_focus_lock_filter(response.id, arrows));
        if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            to = index.checked_sub(1);
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) && index + 1 < count {
            to = Some(index + 1);
        }
    }
    response.context_menu(|ui| {
        if ui.add_enabled(index > 0, egui::Button::new(tr("Move up"))).clicked() {
            to = index.checked_sub(1);
        }
        if ui.add_enabled(index + 1 < count, egui::Button::new(tr("Move down"))).clicked() {
            to = Some(index + 1);
        }
    });
    if let Some(to) = to {
        edit.action = Some(MapAction::Move { from: index, to });
    }
}

/// The "$" button in `rect`: the file name variables in the reference's order, then $channel
fn token_menu(ui: &mut Ui, rect: Rect, index: usize, file: &mut String) -> bool {
    let mut changed = false;
    let id = ui.scope_id().with(("exp_tokens", index));
    let button = drawn::button_with_id(ui, id, rect, "$", "$", false, ui.is_enabled()).on_hover_text(tr("Insert a variable"));
    egui::Popup::menu(&button).show(|ui| {
        let mut add = |ui: &mut Ui, token: &str| {
            if ui.button(RichText::new(token).monospace()).clicked() {
                file.push_str(token);
                changed = true;
                ui.close();
            }
        };
        for token in REFERENCE_TOKENS {
            if FILE_TOKENS.iter().any(|(ours, _)| *ours == token) {
                add(ui, token);
            } else {
                ui.add_enabled(false, egui::Button::new(RichText::new(token).monospace())).on_disabled_hover_text(tr(NOT_AVAILABLE));
            }
        }
        for &(token, _) in FILE_TOKENS.iter().filter(|(token, _)| !REFERENCE_TOKENS.contains(token)) {
            add(ui, token);
        }
    });
    changed
}

/// The file type menu of an output map ("png ⌄") at `place`: (value's left, chevron's centre, middle)
fn format_menu(ui: &mut Ui, (left, chevron, y): (f32, f32, f32), index: usize, map: &mut MapSpec) -> bool {
    let mut changed = false;
    let id = ui.scope_id().with(("exp_map_format", index));
    let menu = drawn::text_dropdown(ui, id, left, chevron, y, map.format.extension()).on_hover_text(tr("File format"));
    egui::Popup::menu(&menu).show(|ui| {
        for extension in MAP_FILE_TYPES {
            let Some(format) = FileFormat::from_extension(extension) else {
                ui.add_enabled(false, egui::Button::selectable(false, extension)).on_disabled_hover_text(tr(NOT_AVAILABLE));
                continue;
            };

            // A new format starts at its default bit depth, as in the reference
            if ui.selectable_label(map.format == format, extension).clicked() {
                if map.format != format {
                    map.bit_depth = format.default_depth();
                    map.format = format;
                    changed = true;
                }
                ui.close();
            }
        }
    });
    changed
}

/// The bit depth menu of an output map ("8 bits ⌄"); templates do not keep dithering yet
fn depth_menu(ui: &mut Ui, (left, chevron, y): (f32, f32, f32), index: usize, map: &mut MapSpec) -> bool {
    let format = map.format;
    let mut changed = false;
    let shown = crate::export_window::depth_label(Some(format), map.bit_depth, false);
    let id = ui.scope_id().with(("exp_map_depth", index));
    let menu = drawn::text_dropdown(ui, id, left, chevron, y, &shown).on_hover_text(tr("Bit depth"));
    egui::Popup::menu(&menu).show(|ui| {
        for (depth, dithering) in crate::export_window::depth_options(format) {
            let label = crate::export_window::depth_label(Some(format), depth, dithering);
            if dithering {
                ui.add_enabled(false, egui::Button::selectable(false, label)).on_disabled_hover_text(tr(NOT_AVAILABLE));
            } else if ui.selectable_label(map.bit_depth == depth, label).clicked() {
                changed |= map.bit_depth != depth;
                map.bit_depth = depth;
                ui.close();
            }
        }
    });
    changed
}

/// R, G and B's sources, when the map has them
fn rgb_sources(map: &MapSpec) -> Option<[&SourceRef; 3]> {
    let get = |dest: Dest| map.channels.iter().find(|(d, _)| *d == dest).map(|(_, s)| s);
    Some([get(Dest::R)?, get(Dest::G)?, get(Dest::B)?])
}

/// Whether R, G and B read one map (or one constant) and are inverted alike, so one RGB swatch
/// can stand for them
fn rgb_one_map(map: &MapSpec) -> bool {
    let Some([red, green, blue]) = rgb_sources(map) else { return true };

    let inverted = [Dest::R, Dest::G, Dest::B].map(|dest| map.inverted.contains(&dest));
    same_source(red, green) && same_source(green, blue) && inverted.iter().all(|on| *on == inverted[0])
}

/// Whether R, G and B read a colour map's R, G and B in that order
fn rgb_spread(map: &MapSpec) -> bool {
    rgb_sources(map).is_some_and(|sources| sources.iter().zip(0u8..).all(|(source, k)| source.components() == 3 && component_of(source) == k))
}

/// Whether one RGB swatch shows R, G and B: one map, read as its R, G and B or as one channel in
/// all three
fn rgb_collapsed(map: &MapSpec) -> bool {
    let Some(sources) = rgb_sources(map) else { return true };

    let one_component = sources.iter().all(|source| component_of(source) == component_of(sources[0]));
    rgb_one_map(map) && (rgb_spread(map) || one_component)
}

/// The swatches an output map shows: Gr, or RGB (R, G, B when split), then A when it has alpha
fn swatch_targets(map: &MapSpec, split: bool) -> Vec<ChannelTarget> {
    let layout = Layout::of(map);
    let mut targets = match layout {
        Layout::Gray | Layout::GrayAlpha => vec![ChannelTarget::One(Dest::L)],
        Layout::Rgb | Layout::Rgba if split || !rgb_collapsed(map) => [Dest::R, Dest::G, Dest::B].into_iter().map(ChannelTarget::One).collect(),
        Layout::Rgb | Layout::Rgba => vec![ChannelTarget::Rgb],
    };
    if matches!(layout, Layout::GrayAlpha | Layout::Rgba) {
        targets.push(ChannelTarget::One(Dest::A));
    }
    targets
}

/// The letters on a swatch
fn target_label(target: ChannelTarget) -> &'static str {
    match target {
        ChannelTarget::Rgb => "RGB",
        ChannelTarget::One(Dest::R) => "R",
        ChannelTarget::One(Dest::G) => "G",
        ChannelTarget::One(Dest::B) => "B",
        ChannelTarget::One(Dest::A) => "A",
        ChannelTarget::One(Dest::L) => "Gr",
    }
}

/// The source the swatch shows (R's for the RGB swatch)
fn source_of(map: &MapSpec, target: ChannelTarget) -> Option<&SourceRef> {
    let dest = target.dests().first()?;
    map.channels.iter().find(|(d, _)| d == dest).map(|(_, s)| s)
}

/// The choices of the drop popup ("Which part of <map>?"): "Red, green and blue" only for a colour map on
/// the RGB swatch; None marks a choice TextureCraft cannot read yet (a map's gray or alpha)
fn source_choices(source: &SourceRef, target: ChannelTarget) -> Vec<(&'static str, Option<SourcePick>)> {
    if source.components() != 3 {
        return vec![(n_("Gray value"), Some(SourcePick::Component(0))), (n_("Alpha channel"), None)];
    }

    let mut choices = Vec::new();
    if target == ChannelTarget::Rgb {
        choices.push((n_("Red, green and blue"), Some(SourcePick::AllRgb)));
    }
    choices.extend([
        (n_("Gray value"), None),
        (n_("Red channel"), Some(SourcePick::Component(0))),
        (n_("Green channel"), Some(SourcePick::Component(1))),
        (n_("Blue channel"), Some(SourcePick::Component(2))),
        (n_("Alpha channel"), None),
    ]);
    choices
}

/// Fill the swatch `target` from `source` as `pick` says
fn apply_pick(map: &mut MapSpec, target: ChannelTarget, source: &SourceRef, pick: SourcePick) {
    for (dest, channel_source) in &mut map.channels {
        if !target.dests().contains(dest) {
            continue;
        }

        let component = match pick {
            SourcePick::AllRgb => slot_component(*dest),
            SourcePick::Component(component) => component,
        };
        *channel_source = with_component(source, component);
    }
}

/// The part of its source a swatch reads, as its tooltip names it
fn read_channel_name(map: &MapSpec, source: &SourceRef, target: ChannelTarget) -> &'static str {
    if source.components() != 3 {
        return n_("gray value");
    }

    if target == ChannelTarget::Rgb && rgb_spread(map) {
        return "RGB";
    }

    match component_of(source) {
        0 => n_("red channel"),
        1 => n_("green channel"),
        _ => n_("blue channel"),
    }
}

/// A swatch's tooltip: "Empty (writes black)" ("Empty (writes white)" for alpha) when empty, the
/// other way round when the swatch is inverted, else "<map>, <part>"; another constant
/// (TextureCraft's own) gives its value
fn swatch_tooltip(map: &MapSpec, source: &SourceRef, target: ChannelTarget, inverted: bool) -> String {
    let alpha = target == ChannelTarget::One(Dest::A);
    let cleared = if alpha { CLEARED_ALPHA } else { CLEARED_VALUE };
    let written = if inverted { 1.0 - cleared } else { cleared };
    match source {
        SourceRef::Constant(value) if (*value - cleared).abs() < f32::EPSILON => {
            if written > 0.5 {
                tr("Empty (writes white)").to_string()
            } else {
                tr("Empty (writes black)").to_string()
            }
        }
        SourceRef::Constant(value) => trf("Constant {value}", &[("value", &format!("{value:.2}"))]),
        _ => {
            let name = map_lists::name_of(source).map_or_else(String::new, |name| tr(name).to_string());
            trf("{map}, {channel}", &[("map", &name), ("channel", &tr(read_channel_name(map, source, target)))])
        }
    }
}

/// A map's colour on swatches and list markers (None for constants)
fn source_color(source: &SourceRef) -> Option<Color32> {
    let channels = Channel::ALL.len();
    let converted = VirtualMap::ALL.len();
    let index = match source {
        SourceRef::Channel { channel, .. } => Channel::ALL.iter().position(|c| c == channel)?,
        SourceRef::Virtual { map, .. } => channels + VirtualMap::ALL.iter().position(|m| m == map)?,
        SourceRef::Baked { map, .. } => channels + converted + BakedMap::ALL.iter().position(|m| m == map)?,
        SourceRef::Constant(_) => return None,
    };

    let hue = (index as f32 * HUE_STEP).fract();
    Some(egui::ecolor::Hsva::new(hue, SWATCH_SATURATION, SWATCH_VALUE, 1.0).into())
}

/// The underline of a swatch: the colour of the channel its letters name (white for RGB and gray)
fn swatch_bar(target: ChannelTarget) -> Color32 {
    match target {
        ChannelTarget::One(Dest::R) => BAR_RED,
        ChannelTarget::One(Dest::G) => BAR_GREEN,
        ChannelTarget::One(Dest::B) => BAR_BLUE,
        ChannelTarget::One(Dest::A) => BAR_ALPHA,
        ChannelTarget::Rgb | ChannelTarget::One(Dest::L) => BAR_WHOLE,
    }
}

/// "→ <file>", or why the output writes nothing
fn planned_text(planned: Option<&PlannedOutput>) -> String {
    match planned {
        Some(planned) => format!("{} {}", ic::ARROW_RIGHT, planned.file),
        None => tr("Not exported: no source in this texture set").to_string(),
    }
}

/// Below a swatch's tooltip: the file its output writes, each channel with its source, then the file's size and type (built-in templates' swatches show it too)
fn output_preview(ui: &mut Ui, index: usize, map: &MapSpec, planned: Option<&PlannedOutput>) {
    let tokens = Tokens::get(ui.ctx());
    ui.separator();
    let Some(planned) = planned else {
        ui.label(RichText::new(planned_text(None)).color(tokens.text_dim));
        return;
    };

    let title = if planned.tiled { trf("Atlas preview: {file}", &[("file", &planned.file)]) } else { planned.file.clone() };
    ui.label(RichText::new(title).color(tokens.text_strong));
    if let Some(Some(pictures)) = MapPreviewCache::get(ui.ctx(), index) {
        preview_pictures(ui, map, &pictures);
    }

    let file_type = planned.format.map_or("", FileFormat::extension);
    let depth = crate::export_window::depth_label(planned.format, planned.bit_depth, planned.dithering);
    let details = if planned.tiled { format!("{file_type} {depth}") } else { format!("{size} x {size} · {file_type} {depth}", size = planned.size) };
    ui.label(RichText::new(details).color(tokens.text_dim));
}

/// The file's picture with each channel's grey picture, letter and source beside it
fn preview_pictures(ui: &mut Ui, map: &MapSpec, pictures: &MapPreviewPictures) {
    let tokens = Tokens::get(ui.ctx());
    let picture = |texture, width: f32| egui::Image::new(texture).fit_to_exact_size(vec2(width, width * pictures.aspect)).corner_radius(tokens.radius);
    ui.horizontal_top(|ui| {
        ui.add(picture(&pictures.file, PREVIEW_FILE_WIDTH_POINTS));
        if pictures.channels.is_empty() {
            return;
        }

        ui.vertical(|ui| {
            for (dest, texture) in &pictures.channels {
                let target = ChannelTarget::One(*dest);
                let gray = ChannelTarget::One(Dest::L);
                let source_target = match dest {
                    Dest::R | Dest::G | Dest::B if source_of(map, gray).is_some() => gray,
                    _ => target,
                };
                let fallback = SourceRef::Constant(if *dest == Dest::A { CLEARED_ALPHA } else { CLEARED_VALUE });
                let source = source_of(map, source_target).unwrap_or(&fallback);
                let inverted = source_target.dests().iter().any(|dest| map.inverted.contains(dest));

                ui.horizontal(|ui| {
                    ui.add(picture(texture, PREVIEW_CHANNEL_WIDTH_POINTS));
                    ui.label(RichText::new(target_label(target)).color(tokens.text_strong));
                    ui.label(RichText::new(swatch_tooltip(map, source, source_target, inverted)).color(tokens.text_dim));
                });
            }
        });
    });
}

/// A card's channel swatches from `area`'s left: drop targets for the map lists, with click and right-click menus
fn channel_swatches(ui: &mut Ui, area: Rect, index: usize, map: &mut MapSpec, split: &mut bool, planned: Option<&PlannedOutput>, edit: &mut RowEdit) -> bool {
    let mut changed = false;
    let shows_split = *split || !rgb_collapsed(map);
    let mut left = area.left();
    for (slot, target) in swatch_targets(map, *split).into_iter().enumerate() {
        let Some(source) = source_of(map, target).cloned() else { continue };

        let label = target_label(target);
        let inverted = target.dests().iter().all(|dest| map.inverted.contains(dest));
        let tooltip = swatch_tooltip(map, &source, target, inverted);
        let mut name = trf("{channel}: {tooltip}", &[("channel", &label), ("tooltip", &tooltip)]);
        if inverted {
            name = trf("{swatch}, inverted", &[("swatch", &name)]);
        }
        let constant = matches!(source, SourceRef::Constant(_));
        let look = SwatchLook { fill: source_color(&source), bar: (!constant).then(|| swatch_bar(target)), inverted };
        let rect = Rect::from_min_max(pos2(left, area.top()), pos2(left + SWATCH_WIDTH, area.bottom()));
        left += SWATCH_WIDTH + SWATCH_GAP;
        let response = swatch(ui, ui.scope_id().with(("exp_swatch", index, slot)), rect, label, &name, &look);
        if response.contains_pointer() {
            edit.hovered_output = Some(index);
        }
        let hover = |ui: &mut Ui| {
            ui.label(&tooltip);
            if inverted {
                ui.label(tr("Inverted"));
            }
            output_preview(ui, index, map, planned);
        };
        let response = response.on_hover_ui(hover).on_disabled_hover_ui(hover);

        // A map dragged over the swatch, or dropped on it; a row's grip passes through to its row
        let map_dragged_over = response.dnd_hover_payload::<SourceRef>().is_some();
        if map_dragged_over {
            drop_highlight(ui, response.rect.expand(DROP_HIGHLIGHT_GAP_POINTS));
        }
        if map_dragged_over && let Some(input) = response.dnd_release_payload::<SourceRef>() {
            edit.pending = Some(PendingDrop { map: index, target, source: (*input).clone(), position: response.rect.center_bottom() });
        }

        let rgb = matches!(target, ChannelTarget::Rgb | ChannelTarget::One(Dest::R | Dest::G | Dest::B));
        let menu_id = ui.make_persistent_id(("exp_swatch_menu", index, slot));
        egui::Popup::menu(&response).id(menu_id).show(|ui| {
            if ui.button(tr("Clear channel")).clicked() {
                clear_channel(map, target);
                changed = true;
            }
            if rgb {
                // Collapsing would drop maps that R, G and B read differently
                let mut checked = shows_split;
                let can_toggle = !shows_split || rgb_one_map(map);
                let item = ui.add_enabled(can_toggle, egui::Checkbox::new(&mut checked, tr("Separate R, G and B")));
                let item = item.on_disabled_hover_text(tr("R, G and B can only be joined when they read the same map and are inverted alike"));
                if item.changed() {
                    changed |= split_or_collapse(map, split, shows_split);
                }
            }
        });
        response.context_menu(|ui| changed |= swatch_extras_menu(ui, map, target));
    }
    changed
}

/// TextureCraft's own swatch tools on the right click (the reference has none there): invert,
/// constants and the output map's channel layout. True when the map changed.
fn swatch_extras_menu(ui: &mut Ui, map: &mut MapSpec, target: ChannelTarget) -> bool {
    let mut changed = false;
    let on = target.dests().iter().all(|dest| map.inverted.contains(dest));
    if ui.selectable_label(on, tr("Invert")).on_hover_text(tr("Write 1 minus the value in this channel")).clicked() {
        map.inverted.retain(|dest| !target.dests().contains(dest));
        if !on {
            map.inverted.extend_from_slice(target.dests());
        }
        changed = true;
        ui.close();
    }
    ui.separator();
    for (label, value) in CONSTANTS {
        if ui.button(tr(label)).clicked() {
            set_constant(map, target, value);
            changed = true;
            ui.close();
        }
    }
    if let Some(SourceRef::Constant(mut value)) = source_of(map, target).cloned() {
        let constant = egui::DragValue::new(&mut value).range(0.0..=1.0).speed(CONSTANT_DRAG_SPEED).fixed_decimals(2);
        if ui.add(constant).changed() {
            set_constant(map, target, value);
            changed = true;
        }
    }
    ui.separator();
    ui.menu_button(tr("Layout"), |ui| {
        let current = Layout::of(map);
        for layout in Layout::ALL {
            if ui.selectable_label(layout == current, tr(layout.label())).clicked() && layout != current {
                set_layout(map, layout);
                changed = true;
                ui.close();
            }
        }
    });
    changed
}

/// The swatch's channels write the constant `value`
fn set_constant(map: &mut MapSpec, target: ChannelTarget, value: f32) {
    for (dest, source) in &mut map.channels {
        if target.dests().contains(dest) {
            *source = SourceRef::Constant(value);
        }
    }
}

/// The frame round a drop target, in the text colour: the accent is Export's alone
fn drop_highlight(ui: &Ui, rect: Rect) {
    let look = ExportLook::get(ui.ctx());
    ui.painter().rect_stroke(rect, CARD_RADIUS, egui::Stroke::new(DROP_HIGHLIGHT_STROKE_POINTS, look.text), egui::StrokeKind::Outside);
}

/// One channel swatch `id`: its letters, the source's colour, its channel's underline, a corner mark when inverted
fn swatch(ui: &mut Ui, id: egui::Id, rect: Rect, label: &str, name: &str, swatch_look: &SwatchLook) -> egui::Response {
    let look = ExportLook::get(ui.ctx());
    let enabled = ui.is_enabled();
    let response = ui.interact(rect, id, egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name));

    let painter = ui.painter();
    let fill = swatch_look.fill.unwrap_or(look.raised);
    painter.rect_filled(rect, 0.0, fill);
    if response.hovered() {
        painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, look.text_dim), egui::StrokeKind::Inside);
    }
    let lettering = look.lettering_on(fill);
    painter.text(pos2(rect.center().x, rect.top() + SWATCH_LABEL_Y), egui::Align2::CENTER_CENTER, label, drawn::font(false), lettering);
    if let Some(bar) = swatch_look.bar {
        let bottom = rect.bottom() - SWATCH_BAR_BOTTOM;
        painter.rect_filled(
            Rect::from_min_max(pos2(rect.left() + SWATCH_BAR_INSET, bottom), pos2(rect.right() - SWATCH_BAR_INSET, bottom + SWATCH_BAR_HEIGHT)),
            0.0,
            bar,
        );
    }
    if swatch_look.inverted {
        let corner = rect.right_top();
        let mark = vec![corner, corner - vec2(INVERTED_MARK_POINTS, 0.0), corner + vec2(0.0, INVERTED_MARK_POINTS)];
        painter.add(egui::Shape::convex_polygon(mark, lettering, egui::Stroke::NONE));
    }
    drawn::paint_focus(ui, &response);
    response
}

/// Clear channel: the swatch is empty again ("Empty (writes black)", white for alpha), not
/// inverted
fn clear_channel(map: &mut MapSpec, target: ChannelTarget) {
    for (dest, source) in &mut map.channels {
        if target.dests().contains(dest) {
            *source = SourceRef::Constant(if *dest == Dest::A { CLEARED_ALPHA } else { CLEARED_VALUE });
        }
    }
    map.inverted.retain(|dest| !target.dests().contains(dest));
}

/// Separate R, G and B: the RGB swatch splits into R, G and B; split ones that read one map collapse
/// into one RGB swatch reading that map's R, G and B. R, G and B reading different maps stay split,
/// so no map is dropped. True when the map changed.
fn split_or_collapse(map: &mut MapSpec, split: &mut bool, shows_split: bool) -> bool {
    if !shows_split || !rgb_one_map(map) {
        *split = true;
        return false;
    }

    *split = false;
    if rgb_collapsed(map) {
        return false;
    }

    let Some(red) = source_of(map, ChannelTarget::One(Dest::R)).map(|s| with_component(s, 0)) else { return false };
    if red.components() == 3 {
        drop_input(map, Dest::R, &red);
    } else {
        for (dest, source) in &mut map.channels {
            if is_rgb(*dest) {
                *source = red.clone();
            }
        }
    }
    true
}

/// The drop popup ("Which part of <map>?"): which channel of the dropped map the swatch reads; Escape or a click elsewhere
/// assigns nothing. True when the map changed.
fn source_channel_popup(ui: &Ui, pending: &mut Option<PendingDrop>, maps: &mut [MapSpec]) -> bool {
    let Some(waiting) = pending.clone() else { return false };
    let Some(map) = maps.get_mut(waiting.map) else {
        *pending = None;
        return false;
    };

    let name = map_lists::name_of(&waiting.source).map_or_else(String::new, |name| tr(name).to_string());
    let mut picked = None;
    let shown = egui::Popup::new(egui::Id::unique("exp_source_channel"), ui.ctx().clone(), egui::PopupAnchor::Position(waiting.position), ui.layer_id())
        .kind(egui::PopupKind::Menu)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .style(egui::containers::menu::menu_style)
        .open(true)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.label(RichText::new(trf("Which part of {map}?", &[("map", &name)])).strong());
            for (label, pick) in source_choices(&waiting.source, waiting.target) {
                match pick {
                    Some(pick) => {
                        if ui.button(tr(label)).clicked() {
                            picked = Some(pick);
                        }
                    }
                    None => {
                        ui.add_enabled(false, egui::Button::new(tr(label))).on_disabled_hover_text(tr(NOT_AVAILABLE));
                    }
                }
            }
        });
    // The app's shortcuts can consume Escape before the popup sees it, so read the raw event too; the runtime keeps an
    // Escape nothing of TextureCraft's has focus for as the window's cancel, which the framework reads
    let escaped = crate::framework::escape_pressed(ui.ctx())
        || ui.input(|input| input.raw.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. })));
    if picked.is_some() || escaped || shown.is_none_or(|menu| menu.response.should_close()) {
        *pending = None;
    }
    let Some(pick) = picked else { return false };

    apply_pick(map, waiting.target, &waiting.source, pick);
    true
}

/// The maps the selected template's outputs read, with their colours, for the panes' markers
fn marked_sources(app: &TextureCraftApp) -> Vec<(SourceRef, Color32)> {
    let draft = app.ui.export.draft.as_ref().map(std::borrow::Cow::Borrowed);
    let Some(preset) = draft.or_else(|| selected(app).map(std::borrow::Cow::Owned)) else { return Vec::new() };

    let sources = preset.maps.iter().flat_map(|map| map.channels.iter().map(|(_, source)| source));
    sources.filter_map(|source| source_color(source).map(|color| (source.clone(), color))).collect()
}

/// A pane's title at `left`, on the column titles' line
fn pane_title(ui: &mut Ui, left: f32, y: f32, title: &str) {
    let look = ExportLook::get(ui.ctx());
    drawn::label_at(ui, left + GROUP_INDENT, y, title, look.text, true);
}

/// A pane's entries scrolling in `area`, the scroll bar in its strip at the right
fn pane_list(ui: &mut Ui, area: Rect, id: &str, add: impl FnOnce(&mut Ui, f32)) {
    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
        egui::ScrollArea::vertical().id_salt(id).auto_shrink([false; 2]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui, area.width() - GUTTER);
        });
    });
}

/// The Input maps pane in `pane`: the input maps in their groups
fn input_maps(ui: &mut Ui, pane: Rect, top: f32, marked: &[(SourceRef, Color32)]) {
    let look = ExportLook::get(ui.ctx());
    pane_title(ui, pane.left(), top + HEADER_Y, tr("Input maps"));
    let list = Rect::from_min_max(pos2(pane.left(), top + LIST_TOP), pane.max);
    pane_list(ui, list, "exp_inputs", |ui, width| {
        for (group, entries) in INPUT_MAPS.iter() {
            let (rect, _) = ui.allocate_exact_size(vec2(width, GROUP_HEIGHT), egui::Sense::hover());
            drawn::label_at(ui, rect.left() + GROUP_INDENT, rect.center().y, tr(group), look.text_dim, false);
            map_entries(ui, width, entries, marked);
        }
    });
}

/// The side pane in `pane`: Mesh maps above Converted maps, a divider between them
fn mesh_and_converted_maps(ui: &mut Ui, pane: Rect, top: f32, marked: &[(SourceRef, Color32)]) {
    pane_title(ui, pane.left(), top + HEADER_Y, tr("Mesh maps"));
    let split = pane.center().y + SIDE_SPLIT_BELOW_MIDDLE;
    let mesh = Rect::from_min_max(pos2(pane.left(), top + LIST_TOP), pos2(pane.right(), split - MESH_BOTTOM_GAP));
    pane_list(ui, mesh, "exp_mesh_maps", |ui, width| map_entries(ui, width, &MESH_MAPS, marked));
    drawn::divider(ui, Rect::from_min_max(pos2(pane.left() - 1.0, split), pos2(pane.right(), split + 1.0)));
    pane_title(ui, pane.left(), split + CONVERTED_TITLE_Y, tr("Converted maps"));
    let converted = Rect::from_min_max(pos2(pane.left(), split + CONVERTED_ROWS_TOP), pane.max);
    pane_list(ui, converted, "exp_converted_maps", |ui, width| map_entries(ui, width, &CONVERTED_MAPS, marked));
}

fn map_entries(ui: &mut Ui, width: f32, entries: &[MapEntry], marked: &[(SourceRef, Color32)]) {
    for (name, source) in entries {
        let marker = source.as_ref().and_then(|source| marked.iter().find(|(used, _)| same_source(used, source)).map(|(_, color)| *color));
        map_entry(ui, width, tr(name), source.as_ref(), marker);
    }
}

/// A map list entry to drag onto a channel; a used map shows a square of its colour, a missing one is greyed
fn map_entry(ui: &mut Ui, width: f32, name: &str, source: Option<&SourceRef>, marker: Option<Color32>) {
    let look = ExportLook::get(ui.ctx());
    let available = source.is_some();
    let sense = if available { egui::Sense::drag() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(width, ENTRY_PITCH), sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ListItem, available, name));

    let row = Rect::from_min_max(pos2(rect.left() + ENTRY_INSET, rect.top()), pos2(rect.right(), rect.top() + ENTRY_HEIGHT));
    let painter = ui.painter();
    painter.rect_filled(row, 0.0, if available && response.hovered() { look.raised } else { look.control });
    if let Some(color) = marker {
        let square = Rect::from_min_size(pos2(rect.left() + MARKER_LEFT, row.center().y - MARKER_SIZE / 2.0), vec2(MARKER_SIZE, MARKER_SIZE));
        let edge = Color32::from_rgb(
            color.r().saturating_add(MARKER_EDGE_LIGHTEN),
            color.g().saturating_add(MARKER_EDGE_LIGHTEN),
            color.b().saturating_add(MARKER_EDGE_LIGHTEN),
        );
        painter.rect(square, MARKER_RADIUS, color, egui::Stroke::new(1.0, edge), egui::StrokeKind::Inside);
    }
    let color = if available { look.text } else { look.text_dim.gamma_multiply(UNAVAILABLE_FADE) };
    let shown = drawn::fitted(ui, name, row.right() - rect.left() - ENTRY_NAME_LEFT, false, false);
    drawn::text_at(ui, rect.left() + ENTRY_NAME_LEFT, row.center().y, &shown, color, false);
    let Some(source) = source else {
        response.on_hover_text(tr(NOT_AVAILABLE));
        return;
    };

    // How to use the entry is its tooltip, not a line under the cards
    let response = response.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(tr("Drop an input map onto a channel to assign it to that output."));
    response.dnd_set_drag_payload(source.clone());
    if response.dragged()
        && let Some(at) = response.interact_pointer_pos()
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let dragged = egui::Area::new(egui::Id::unique(("exp_map_drag", name))).order(egui::Order::Tooltip).fixed_pos(at + vec2(8.0, 8.0));
        dragged.show(ui.ctx(), |ui| chip(ui, name, true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(channel: Channel, component: u8) -> SourceRef {
        SourceRef::Channel { channel, component }
    }

    #[test]
    fn feedback_preview_follows_draft_maps_to_their_saved_overrides() {
        use crate::state::{MapExport, SetExport};
        use egui_kittest::Harness;

        let session = texturecraft_engine::sample_session(256, 64).unwrap();
        let mut harness = Harness::builder().with_context_setup(crate::framework::install_headless).build_eframe(|cc| TextureCraftApp::new(cc, session));
        let app = harness.state_mut();
        let ts = app.session.ts();
        let saved = app.session.export_preset("Unreal Engine (Packed)").unwrap();
        let mut draft = saved.clone();
        draft.maps = vec![saved.maps[1].clone(), new_map(Layout::Gray, "New"), saved.maps[0].clone()];
        let resized = MapExport { size: Some(128), ..MapExport::default() };
        let disabled = MapExport { enabled: false, ..MapExport::default() };
        app.ui.export.preset = saved.name.clone();
        app.ui.export.sets.insert(
            ts,
            SetExport {
                maps_template: saved.name,
                maps: [(0, resized), (1, disabled), (2, MapExport { size: Some(512), ..MapExport::default() })].into(),
                ..SetExport::default()
            },
        );
        app.ui.export.draft = Some(draft.clone());
        app.ui.export.draft_maps = vec![Some(1), None, Some(0)];

        let params = plan_params(app, &draft).unwrap();
        let outputs = planned_outputs(app, &params).unwrap();

        assert!(outputs[0].is_none(), "the disabled map stays disabled in its new position");
        assert_eq!(outputs[2].as_ref().unwrap().size, 128);
        assert_eq!(params["maps"].as_array().unwrap().len(), 2, "the new map and the removed map get no override");
    }

    #[test]
    fn tiled_raster_and_psd_previews_describe_the_whole_atlas() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable as _;
        use texturecraft_engine::Session;
        use texturecraft_engine::geom::primitives::udim_plane;

        let session = Session::new_project("Tiles", udim_plane(), "builtin:udim_plane", None, 128).unwrap();
        let mut harness = Harness::builder().with_context_setup(crate::framework::install_headless).build_eframe(|cc| TextureCraftApp::new(cc, session));
        let app = harness.state_mut();
        let mut preset = app.session.export_preset("Separate maps").unwrap();
        preset.maps.truncate(2);
        preset.maps[0].file = "Raster".into();
        preset.maps[0].format = FileFormat::Png;
        preset.maps[1].file = "Group/Layer.$udim".into();
        preset.maps[1].format = FileFormat::Psd;
        let mut ambiguous = preset.maps[0].clone();
        ambiguous.file = "Literal_UDIMTOKEN.$udim".into();
        preset.maps.push(ambiguous);
        let params = json!({"ts": 0, "preset": preset});

        let outputs = planned_outputs(app, &params).unwrap();
        let preview = Harness::builder().with_context_setup(crate::framework::install_headless).build_ui(|ui| {
            for (index, map) in preset.maps.iter().enumerate() {
                output_preview(ui, index, map, outputs[index].as_ref());
            }
        });

        assert!(preview.query_by_label("Atlas preview: Raster.<UDIM>.png").is_some());
        assert!(preview.query_by_label_contains("<UDIM>.psd").is_some());
        assert!(preview.query_by_label("Atlas preview: UV tile files").is_some());
        assert!(preview.query_by_label_contains("128 x 128").is_none(), "the tile size must not describe the whole atlas");
    }

    #[test]
    fn layouts_keep_sources_and_drops_fill_rgb() {
        let mut map = new_map(Layout::Rgb, "t");
        drop_input(&mut map, Dest::G, &ch(Channel::BaseColor, 0));
        assert_eq!(map.channels, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2))]);
        drop_input(&mut map, Dest::B, &ch(Channel::Metallic, 0));
        assert_eq!(map.channels.get(2), Some(&(Dest::B, ch(Channel::Metallic, 0))), "a gray input fills only its channel");
        set_layout(&mut map, Layout::Rgba);
        assert_eq!(Layout::of(&map), Layout::Rgba);
        assert_eq!(map.channels.get(3), Some(&(Dest::A, SourceRef::Constant(1.0))));
        set_layout(&mut map, Layout::GrayAlpha);
        assert_eq!(map.channels, vec![(Dest::L, ch(Channel::BaseColor, 0)), (Dest::A, SourceRef::Constant(1.0))]);
        set_layout(&mut map, Layout::Rgb);
        assert_eq!(map.channels.get(1), Some(&(Dest::G, ch(Channel::BaseColor, 1))), "gray to RGB spreads a color source");
        for l in [Layout::Rgb, Layout::Rgba, Layout::Gray, Layout::GrayAlpha] {
            let m = new_map(l, "t");
            assert_eq!(Layout::of(&m), l);
            ExportPreset { name: "t".into(), maps: vec![m] }.validate().unwrap();
        }
        assert!(same_source(&ch(Channel::Normal, 2), &ch(Channel::Normal, 0)));
        assert!(!same_source(&SourceRef::Constant(0.0), &SourceRef::Constant(1.0)));
    }

    #[test]
    fn inversion_follows_its_source_through_layouts_and_drops() {
        let mut map = new_map(Layout::Rgb, "t");
        map.inverted = vec![Dest::R];
        set_layout(&mut map, Layout::Gray);
        assert_eq!(map.inverted, vec![Dest::L], "the colour source keeps its inversion on L");
        set_layout(&mut map, Layout::Rgb);
        assert_eq!(map.inverted, vec![Dest::R, Dest::G, Dest::B], "and spreads it over R, G, B");
        set_layout(&mut map, Layout::Rgba);
        assert_eq!(map.inverted, vec![Dest::R, Dest::G, Dest::B], "alpha stays un-inverted");
        map.inverted = vec![Dest::G];
        set_layout(&mut map, Layout::GrayAlpha);
        assert!(map.inverted.is_empty(), "channels that leave take their inversion with them");
        let mut map = new_map(Layout::Rgb, "t");
        map.inverted = vec![Dest::B];
        drop_input(&mut map, Dest::B, &ch(Channel::Metallic, 0));
        assert_eq!(map.inverted, vec![Dest::B], "a dropped map keeps the channel's toggle, like picking from the list");
        ExportPreset { name: "t".into(), maps: vec![map] }.validate().unwrap();
    }

    #[test]
    fn split_and_collapse_keep_r_g_b_on_one_or_three_swatches() {
        let mut map = new_map(Layout::Rgb, "t");
        let mut split = false;
        assert_eq!(swatch_targets(&map, split), vec![ChannelTarget::Rgb], "an empty RGB map shows one swatch");

        assert!(!split_or_collapse(&mut map, &mut split, false), "splitting changes only the view");
        assert_eq!(swatch_targets(&map, split).len(), 3);

        map.channels = vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 0)), (Dest::B, ch(Channel::BaseColor, 1))];
        assert!(split_or_collapse(&mut map, &mut split, true));
        assert_eq!(swatch_targets(&map, split), vec![ChannelTarget::Rgb]);
        assert_eq!(map.channels.get(2), Some(&(Dest::B, ch(Channel::BaseColor, 2))), "collapsing reads the map's R, G and B");
    }

    #[test]
    fn r_g_b_reading_different_maps_or_inverted_apart_stay_split() {
        let differing = vec![(Dest::R, ch(Channel::Roughness, 0)), (Dest::G, ch(Channel::Metallic, 0)), (Dest::B, SourceRef::Constant(0.5))];
        let mut map = new_map(Layout::Rgb, "t");
        map.channels = differing.clone();
        let mut split = true;

        assert!(!split_or_collapse(&mut map, &mut split, true), "collapsing would drop maps");

        assert_eq!(map.channels, differing);
        assert_eq!(swatch_targets(&map, split).len(), 3);
        let mut color = new_map(Layout::Rgb, "t");
        drop_input(&mut color, Dest::R, &ch(Channel::BaseColor, 0));
        color.inverted = vec![Dest::G];
        assert_eq!(swatch_targets(&color, false).len(), 3, "one map inverted in G only shows R, G and B apart");
    }

    #[test]
    fn clear_channel_empties_the_swatch_and_its_inversion() {
        let mut map = new_map(Layout::Rgba, "t");
        drop_input(&mut map, Dest::R, &ch(Channel::BaseColor, 0));
        drop_input(&mut map, Dest::A, &ch(Channel::Opacity, 0));
        map.inverted = vec![Dest::R, Dest::G, Dest::B, Dest::A];

        clear_channel(&mut map, ChannelTarget::Rgb);
        clear_channel(&mut map, ChannelTarget::One(Dest::A));

        let cleared = [(Dest::R, 0.0), (Dest::G, 0.0), (Dest::B, 0.0), (Dest::A, 1.0)].map(|(dest, value)| (dest, SourceRef::Constant(value)));
        assert_eq!(map.channels, cleared);
        assert!(map.inverted.is_empty(), "a cleared channel writes its default, not its inverse");
        assert_eq!(swatch_tooltip(&map, &SourceRef::Constant(0.0), ChannelTarget::Rgb, false), "Empty (writes black)");
        assert_eq!(swatch_tooltip(&map, &SourceRef::Constant(1.0), ChannelTarget::One(Dest::A), false), "Empty (writes white)");
        assert_eq!(
            swatch_tooltip(&map, &SourceRef::Constant(0.0), ChannelTarget::Rgb, true),
            "Empty (writes white)",
            "inverted, the empty colour writes its inverse"
        );
        assert_eq!(swatch_tooltip(&map, &SourceRef::Constant(1.0), ChannelTarget::One(Dest::A), true), "Empty (writes black)");
    }

    #[test]
    fn the_drop_popup_offers_the_parts_of_the_map_the_swatch_can_take() {
        let offered = |source: &SourceRef, target| source_choices(source, target).into_iter().map(|(label, pick)| (label, pick.is_some())).collect::<Vec<_>>();

        assert_eq!(offered(&ch(Channel::Metallic, 0), ChannelTarget::Rgb), [("Gray value", true), ("Alpha channel", false)]);
        let color_on_gray = [("Gray value", false), ("Red channel", true), ("Green channel", true), ("Blue channel", true), ("Alpha channel", false)];
        assert_eq!(offered(&ch(Channel::BaseColor, 0), ChannelTarget::One(Dest::L)), color_on_gray);
        let color_on_rgb = offered(&ch(Channel::BaseColor, 0), ChannelTarget::Rgb);
        assert_eq!(color_on_rgb.first(), Some(&("Red, green and blue", true)), "only the RGB swatch offers all three");
        assert_eq!(color_on_rgb.get(1..), Some(color_on_gray.as_slice()));
    }

    #[test]
    fn rgb_channels_fill_r_g_b_and_one_channel_fills_every_swatch_channel() {
        let mut map = new_map(Layout::Rgb, "t");

        apply_pick(&mut map, ChannelTarget::Rgb, &ch(Channel::BaseColor, 0), SourcePick::AllRgb);
        assert!(rgb_spread(&map));
        assert_eq!(swatch_tooltip(&map, &ch(Channel::BaseColor, 0), ChannelTarget::Rgb, false), "Base color, RGB");

        apply_pick(&mut map, ChannelTarget::Rgb, &ch(Channel::BaseColor, 0), SourcePick::Component(0));
        assert_eq!(swatch_targets(&map, false), vec![ChannelTarget::Rgb], "R of one map in R, G and B stays one swatch");
        assert_eq!(swatch_tooltip(&map, &ch(Channel::BaseColor, 0), ChannelTarget::Rgb, false), "Base color, red channel");
    }

    #[test]
    fn new_rows_take_their_layout_name_with_a_number_when_taken() {
        let maps = vec![new_map(Layout::Rgb, "RGB"), new_map(Layout::Rgb, "RGB_1")];

        assert_eq!(unique_file(&maps, "Gray"), "Gray");
        assert_eq!(unique_file(&maps, "RGB"), "RGB_2");
    }
}
