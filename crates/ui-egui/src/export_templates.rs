//! Export dialog › Output templates: create and edit export presets. Built-in presets are
//! read-only; the selected user preset is edited as a draft that is saved through
//! `export.preset.save` whenever it validates, so a half-typed file name never reaches disk.
//! Maps dragged from the Input maps, Mesh maps and Converted maps panes onto an output map's
//! channel swatches fill its channels.

use egui::{Color32, RichText, Ui, vec2};
use egui_phosphor::regular as ic;
use serde_json::{Value, json};
use texturecraft_engine::core::{BakedMap, Channel};
use texturecraft_engine::formats::export::{FILE_TOKENS, MAX_PRESET_NAME, sanitize_file_stem};
use texturecraft_engine::formats::{Dest, ExportPreset, FileFormat, MapSpec, SourceRef, VirtualMap};

use crate::TextureCraftApp;
use crate::i18n::{n_, tr, trf};
use crate::theme::{Tokens, caption};
use crate::unavailable::NOT_AVAILABLE;
use crate::widgets::{chip, icon_button_without_tooltip};

mod channel_target;
mod map_drag;
mod map_lists;
mod pending_drop;
mod row_edit;
mod source_pick;
mod swatch_look;

pub use channel_target::ChannelTarget;
use map_drag::MapDrag;
use map_lists::{CONVERTED_MAPS, INPUT_MAPS, MESH_MAPS, MapEntry};
pub use pending_drop::PendingDrop;
use row_edit::RowEdit;
use source_pick::SourcePick;
use swatch_look::SwatchLook;

const ICON_POINTS: f32 = 20.0;
/// Width of the Presets list
const LIST_WIDTH_POINTS: f32 = 220.0;
/// The tab's width in the reference-size window: a narrower window narrows the side panes in
/// proportion, so the output rows keep their room
const REFERENCE_TAB_WIDTH_POINTS: f32 = 1085.0;
/// The middle column's least width, and room for the separators and their spacing
const MIN_OUTPUTS_WIDTH_POINTS: f32 = 200.0;
const SEPARATOR_ALLOWANCE_POINTS: f32 = 6.0;
/// Width of the Input maps pane
const INPUTS_WIDTH_POINTS: f32 = 170.0;
/// Width of the pane with Mesh maps above Converted maps
const SIDE_MAPS_WIDTH_POINTS: f32 = 165.0;
/// Share of the side pane's height that Mesh maps take; Converted maps get the rest
const MESH_MAPS_SHARE: f32 = 0.45;
const RENAME_ID: &str = "exp_rename";
/// The name "+" gives a new template, as in the reference
const NEW_PRESET: &str = "new_export_preset";

/// Height of a Presets list row, and where its icon and name start
const PRESET_ROW_HEIGHT_POINTS: f32 = 18.0;
const PRESET_ICON_INDENT_POINTS: f32 = 2.0;
const PRESET_TEXT_INDENT_POINTS: f32 = 20.0;

/// Height of a map list entry, the gap between entries, and where its marker and name sit
const ENTRY_HEIGHT_POINTS: f32 = 21.0;
const ENTRY_GAP_POINTS: f32 = 2.0;
const ENTRY_TEXT_INDENT_POINTS: f32 = 22.0;
const ENTRY_TEXT_SIZE_POINTS: f32 = 12.0;
const MARKER_INDENT_POINTS: f32 = 10.0;
const MARKER_SIZE_POINTS: f32 = 10.0;
/// How strongly a map TextureCraft does not have yet is dimmed
const UNAVAILABLE_FADE: f32 = 0.6;

/// An output map row: padding, gap below it, grip width, file name field width, icon size
const ROW_MARGIN_POINTS: f32 = 6.0;
const ROW_GAP_POINTS: f32 = 4.0;
const GRIP_WIDTH_POINTS: f32 = 5.0;
const NAME_FIELD_WIDTH_POINTS: f32 = 150.0;
const ROW_ICON_POINTS: f32 = 16.0;
/// Width the "Output maps" title and the Create buttons need on one row
const OUTPUT_MAPS_ROW_WIDTH_POINTS: f32 = 420.0;
/// The narrowest file name field, the "$" button and the format, bit depth and icon columns at a
/// row's right end, and the gaps between a row's controls (besides the swatches')
const MIN_NAME_FIELD_WIDTH_POINTS: f32 = 60.0;
const DOLLAR_BUTTON_WIDTH_POINTS: f32 = 22.0;
const ROW_LISTS_WIDTH_POINTS: f32 = 100.0;
const ROW_GAPS: f32 = 4.0;
/// The accent frame around a drop target: its stroke and its gap to the target
const DROP_HIGHLIGHT_STROKE_POINTS: f32 = 1.5;
const DROP_HIGHLIGHT_GAP_POINTS: f32 = 2.0;
/// Side of the corner mark of an inverted swatch
const INVERTED_MARK_POINTS: f32 = 8.0;

/// A channel swatch: its side, the label's size and offset from the top, and the bars that show
/// which channels of the source it reads
const SWATCH_SIZE_POINTS: f32 = 32.0;
const SWATCH_TEXT_SIZE_POINTS: f32 = 11.0;
const SWATCH_TEXT_TOP_POINTS: f32 = 3.0;
const SWATCH_BAR_WIDTH_POINTS: f32 = 18.0;
const SWATCH_BAR_HEIGHT_POINTS: f32 = 2.0;
const SWATCH_BAR_BOTTOM_POINTS: f32 = 9.0;

/// Swatch colours: one hue per map, spread by the golden ratio so neighbours differ, muted to
/// keep the white label readable
const HUE_STEP: f32 = 0.618_034;
const SWATCH_SATURATION: f32 = 0.45;
const SWATCH_VALUE_DARK: f32 = 0.42;
const SWATCH_VALUE_LIGHT: f32 = 0.62;

/// The bars of the R, G and B channels a swatch reads
const COMPONENT_COLORS: [Color32; 3] = [Color32::from_rgb(220, 70, 70), Color32::from_rgb(80, 190, 90), Color32::from_rgb(80, 120, 230)];

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
/// Width of the Naming pattern list
const NAMING_PATTERN_WIDTH_POINTS: f32 = 150.0;

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

/// The Output templates tab: template list, the selected template's outputs, then the Input maps
/// pane and the pane with Mesh maps above Converted maps.
pub fn tab(app: &mut TextureCraftApp, ui: &mut Ui, builtins: &[ExportPreset]) {
    sync_draft(app);
    let height = ui.available_height();
    let names = crate::export_window::template_groups(app, builtins).1;
    let scale = (ui.available_width() / REFERENCE_TAB_WIDTH_POINTS).min(1.0);
    let (list_width, inputs_width, side_width) = (LIST_WIDTH_POINTS * scale, INPUTS_WIDTH_POINTS * scale, SIDE_MAPS_WIDTH_POINTS * scale);
    let mut list_action = None;
    ui.vertical(|ui| {
        ui.set_width(list_width);
        ui.set_height(height);
        list_action = preset_list(app, ui, &names);
    });
    ui.separator();
    ui.vertical(|ui| {
        // Leave room for the two map panes and three separators
        let panes = inputs_width + side_width + 8.0 * ui.spacing().item_spacing.x + SEPARATOR_ALLOWANCE_POINTS;
        ui.set_width((ui.available_width() - panes).max(MIN_OUTPUTS_WIDTH_POINTS));
        ui.set_height(height);
        outputs(app, ui);
    });

    // The panes mark the maps the selected template's outputs read
    let marked = marked_sources(app, &Tokens::get(ui.ctx()));
    ui.separator();
    ui.vertical(|ui| {
        ui.set_width(inputs_width);
        ui.set_height(height);
        input_maps(ui, &marked);
    });
    ui.separator();
    ui.vertical(|ui| {
        ui.set_width(side_width);
        ui.set_height(height);
        mesh_and_converted_maps(ui, height, &marked);
    });
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

/// "Naming pattern" with its info icon, at the right end of the tab row (laid out right to left)
pub(crate) fn naming_pattern(ui: &mut Ui) {
    let tk = Tokens::get(ui.ctx());
    ui.label(RichText::new(ic::INFO).color(tk.text_dim)).on_hover_ui(|ui| {
        ui.label(tr("Choose how the input maps are named in this editor."));
        ui.label(tr("The active texture set’s shader can affect the available map names."));
    });
    egui::ComboBox::from_id_salt("exp_naming").width(NAMING_PATTERN_WIDTH_POINTS).selected_text(NAMING_PATTERNS[0]).show_ui(ui, |ui| {
        for p in NAMING_PATTERNS {
            let _ = ui.selectable_label(true, p);
        }
    });
    ui.label(tr("Naming pattern"));
}

/// The Presets list: the templates sorted by name (built-in and user); a right click offers
/// Rename, Duplicate and Remove, F2 renames
fn preset_list(app: &mut TextureCraftApp, ui: &mut Ui, names: &[String]) -> Option<ListAction> {
    let mut act = None;
    let sel = app.ui.export.preset.clone();
    let user = app.ui.export.draft.is_some();
    ui.horizontal(|ui| {
        ui.label(caption(tr("Presets")));

        // Copy, "−" and "+" from left to right, without tooltips as in the reference
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button_without_tooltip(ui, ic::PLUS, tr("New template"), false, ICON_POINTS).clicked() {
                act = Some(ListAction::New);
            }
            if ui.add_enabled_ui(user, |ui| icon_button_without_tooltip(ui, ic::MINUS, tr("Delete the selected template"), false, ICON_POINTS)).inner.clicked()
            {
                act = Some(ListAction::Remove(sel.clone()));
            }
            if icon_button_without_tooltip(ui, ic::COPY, tr("Duplicate the selected template"), false, ICON_POINTS).clicked() {
                act = Some(ListAction::Duplicate(sel.clone()));
            }

            // TextureCraft's template files: a focusable menu, as the row's right click is pointer only
            let actions = icon_button_without_tooltip(ui, ic::DOTS_THREE, tr("Template actions"), false, ICON_POINTS);
            let opened_now = actions.clicked();
            egui::Popup::menu(&actions).id(ui.make_persistent_id("exp_template_actions")).show(|ui| template_file_items(ui, &mut act, true, opened_now));
        });
    });
    ui.add_space(2.0);
    if std::mem::take(&mut app.ui.export.rename_key) && user {
        act = Some(ListAction::StartRename(sel.clone()));
    }
    egui::ScrollArea::vertical().id_salt("exp_presets").auto_shrink([false; 2]).show(ui, |ui| {
        for name in names {
            let is_user = app.session.export_presets.list.iter().any(|p| p.name == *name);
            match app.ui.export.renaming.as_mut().filter(|_| *name == sel) {
                Some(buf) => {
                    let r = ui.add(egui::TextEdit::singleline(buf).id(egui::Id::unique(RENAME_ID)).desired_width(f32::INFINITY));
                    if r.lost_focus() {
                        act = Some(if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            ListAction::Select(name.clone())
                        } else {
                            ListAction::Rename(buf.clone())
                        });
                    }
                }
                None => {
                    let r = preset_row(ui, name, *name == sel).on_hover_text(name);
                    if r.double_clicked() && is_user {
                        act = Some(ListAction::StartRename(name.clone()));
                    } else if r.clicked() {
                        act = Some(ListAction::Select(name.clone()));
                    }
                    r.context_menu(|ui| {
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
                        template_file_items(ui, &mut act, *name == sel, false);
                    });
                }
            }
        }
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

/// A Presets list row: a document icon and the name (cut with "…" when long), the selected one in
/// the accent colour
fn preset_row(ui: &mut Ui, name: &str, selected: bool) -> egui::Response {
    let tokens = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), PRESET_ROW_HEIGHT_POINTS), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, name));

    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, tokens.radius, tokens.row_hover);
    }
    let font = egui::TextStyle::Body.resolve(ui.style());
    painter.text(rect.left_center() + vec2(PRESET_ICON_INDENT_POINTS, 0.0), egui::Align2::LEFT_CENTER, ic::FILE_TEXT, font.clone(), tokens.text_dim);
    let color = if selected { tokens.accent } else { tokens.text };
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - PRESET_TEXT_INDENT_POINTS);
    let galley = painter.layout_job(job);
    let top_left = egui::pos2(rect.left() + PRESET_TEXT_INDENT_POINTS, rect.center().y - galley.size().y / 2.0);
    painter.galley(top_left, galley, color);
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
                let name = format!("{}_copy", p.name).chars().take(MAX_PRESET_NAME).collect();
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
    for p in storage.and_then(|s| eframe::get_value::<Vec<ExportPreset>>(s, STORAGE_KEY)).unwrap_or_default() {
        if let Ok(v) = serde_json::to_value(&p) {
            app.run("export.preset.save", json!({ "preset": v, "unique": true }));
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

/// The selected template's output maps (editable for user templates).
fn outputs(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tokens = Tokens::get(ui.ctx());
    let (mut preset, editable) = match (&app.ui.export.draft, selected(app)) {
        (Some(d), _) => (d.clone(), true),
        (None, Some(b)) => (b, false),
        (None, None) => {
            ui.label(RichText::new(tr("Select an output template")).color(tokens.text_dim));
            return;
        }
    };
    let preview = preview_names(app, &preset);
    let mut edit = RowEdit::default();

    // The selected template in a strip, "Current preset:" in bold
    egui::Frame::new().fill(tokens.row).inner_margin(ROW_MARGIN_POINTS).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Current preset:")).strong().color(tokens.text_strong));
            ui.label(&preset.name);
            if !editable {
                ui.label(RichText::new(ic::LOCK_SIMPLE).color(tokens.text_dim)).on_hover_text(tr("Built-in templates are read-only: duplicate one to edit it"));
            }
        });
    });

    // "Output maps", with the Create buttons at the right end of the same row, or of the next one
    // when the column is too narrow for both
    let create_buttons = |ui: &mut Ui, edit: &mut RowEdit| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(editable, |ui| {
                for (label, layout, split, file) in CREATE.iter().rev() {
                    if ui.button(tr(label)).clicked() {
                        edit.action = Some(MapAction::Add(*layout, *split, file));
                    }
                }
                ui.label(RichText::new(tr("Create:")).color(tokens.text_dim));
            });
        });
    };
    if ui.available_width() >= OUTPUT_MAPS_ROW_WIDTH_POINTS {
        ui.horizontal(|ui| {
            ui.label(caption(tr("Output maps")));
            create_buttons(ui, &mut edit);
        });
    } else {
        ui.label(caption(tr("Output maps")));
        ui.horizontal(|ui| create_buttons(ui, &mut edit));
    }
    if let Err(e) = &preview {
        ui.label(RichText::new(trf("Not saved: {error}", &[("error", e)])).color(tokens.danger).size(11.0));
    }
    let mut changed = false;
    let map_count = preset.maps.len();
    let mut split = app.ui.export.draft_split.clone();
    split.resize(map_count, false);

    // Leave room for the footer text and the framed count below
    let footer = 2.0 * ui.text_style_height(&egui::TextStyle::Body) + 3.0 * ui.spacing().item_spacing.y + 2.0 * ROW_MARGIN_POINTS;
    egui::ScrollArea::vertical().id_salt("exp_outputs").auto_shrink([false; 2]).max_height((ui.available_height() - footer).max(60.0)).show(ui, |ui| {
        ui.add_enabled_ui(editable, |ui| {
            // Every row takes the column's width, not the width a wider row before it left
            let row_width = ui.available_width();
            for ((index, map), row_split) in preset.maps.iter_mut().enumerate().zip(split.iter_mut()) {
                let file = preview.as_ref().ok().and_then(|names| names.get(index).cloned().flatten());
                let planned = match file {
                    Some(file) => format!("{} {file}", ic::ARROW_RIGHT),
                    None => tr("Not exported: no source in this texture set").to_string(),
                };
                changed |= ui.allocate_ui(vec2(row_width, 0.0), |ui| output_row(ui, index, map_count, map, row_split, &planned, &mut edit)).inner;
                ui.add_space(ROW_GAP_POINTS);
            }
        });
    });
    ui.label(RichText::new(tr("Drop an input map onto a channel to assign it to that output.")).color(tokens.text_dim));

    // Count only maps that have sources in the current set
    if let Ok(names) = &preview {
        let exported = names.iter().flatten().count();
        egui::Frame::new().fill(tokens.field).inner_margin(ROW_MARGIN_POINTS).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(trf("{n} output maps will be exported.", &[("n", &exported)]));
        });
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
    let e = &mut app.ui.export;
    e.draft_split = split;
    e.pending_drop = pending;
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

/// The file each map produces for the current texture set with the dialog's settings (None:
/// the set has none of the map's sources), via `export.plan`.
fn preview_names(app: &mut TextureCraftApp, preset: &ExportPreset) -> Result<Vec<Option<String>>, String> {
    let mut p = crate::export_window::set_params(app, app.session.ts());
    p["preset"] = serde_json::to_value(preset).map_err(|e| e.to_string())?;

    // The set's map overrides apply only to its own template
    if let Some(p) = p.as_object_mut() {
        p.remove("maps");
    }
    let plan = texturecraft_engine::execute(&mut app.session, "export.plan", &p).map_err(|e| e.to_string())?;
    let mut names = vec![None; preset.maps.len()];
    for f in plan.as_array().into_iter().flatten() {
        let map = f.get("map").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
        if let (Some(slot), Some(file)) = (map.and_then(|i| names.get_mut(i)), f.get("file").and_then(Value::as_str)) {
            *slot = Some(file.to_string());
        }
    }
    Ok(names)
}

/// One output map row: drag grip, file name, "$", the channel swatches, then file type, bit depth,
/// duplicate and remove. `planned` is the file it writes for the current texture set.
fn output_row(ui: &mut Ui, index: usize, count: usize, map: &mut MapSpec, split: &mut bool, planned: &str, edit: &mut RowEdit) -> bool {
    let tokens = Tokens::get(ui.ctx());
    let mut changed = false;
    let frame = egui::Frame::new().fill(tokens.row).corner_radius(tokens.radius).inner_margin(ROW_MARGIN_POINTS).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);

            // The name field gives way in a narrow window, so the swatches and lists stay in the row
            let swatches = swatch_targets(map, *split).len() as f32;
            let spacing = ui.spacing().item_spacing.x;
            let others = GRIP_WIDTH_POINTS + DOLLAR_BUTTON_WIDTH_POINTS + swatches * (SWATCH_SIZE_POINTS + spacing) + ROW_LISTS_WIDTH_POINTS;
            let name_width = (ui.available_width() - others - ROW_GAPS * spacing).clamp(MIN_NAME_FIELD_WIDTH_POINTS, NAME_FIELD_WIDTH_POINTS);

            grip(ui, index, count, edit);
            let name = ui.add(egui::TextEdit::singleline(&mut map.file).desired_width(name_width)).on_hover_ui(|ui| {
                ui.label(tr("File name without extension; variables insert names"));
                ui.label(planned);
            });
            changed |= name.changed();
            changed |= token_menu(ui, &mut map.file);
            changed |= channel_swatches(ui, index, map, split, edit);

            // The lists and icons at the row's right end: file format and duplicate, bit depth and remove
            ui.add_space((ui.available_width() - ROW_LISTS_WIDTH_POINTS).max(0.0));
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    changed |= format_menu(ui, map);
                    if icon_button_without_tooltip(ui, ic::COPY, tr("Duplicate this output"), false, ROW_ICON_POINTS).clicked() {
                        edit.action = Some(MapAction::Duplicate(index));
                    }
                });
                ui.horizontal(|ui| {
                    changed |= depth_menu(ui, map);
                    if icon_button_without_tooltip(ui, ic::X, tr("Remove this output"), false, ROW_ICON_POINTS).clicked() {
                        edit.action = Some(MapAction::Remove(index));
                    }
                });
            });
        });
    });

    // Another row's grip dropped here moves that row here (the type check keeps a map list drop)
    let row = frame.response;
    let row_dragged_over = row.dnd_hover_payload::<MapDrag>().is_some_and(|from| from.0 != index);
    if row_dragged_over {
        drop_highlight(ui, row.rect, &tokens);
    }
    if row_dragged_over && let Some(from) = row.dnd_release_payload::<MapDrag>() {
        edit.action = Some(MapAction::Move { from: from.0, to: index });
    }
    changed
}

/// The grip at a row's left: drag it onto another row to move the map there; the arrow keys and
/// its right-click menu move it one place
fn grip(ui: &mut Ui, index: usize, count: usize, edit: &mut RowEdit) {
    let tokens = Tokens::get(ui.ctx());
    let enabled = ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(vec2(GRIP_WIDTH_POINTS, SWATCH_SIZE_POINTS), egui::Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, tr("Reorder")));
    let color = if response.hovered() || response.dragged() { tokens.text } else { tokens.text_dim };
    ui.painter().rect_filled(rect, 1.0, color);
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

/// The "$" button: the file name variables in the reference's order, then $channel
fn token_menu(ui: &mut Ui, file: &mut String) -> bool {
    let mut changed = false;
    let menu = ui.menu_button("$", |ui| {
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
    menu.response.on_hover_text(tr("Insert a variable"));
    changed
}

/// The file type menu of an output map ("png˅")
fn format_menu(ui: &mut Ui, map: &mut MapSpec) -> bool {
    let mut changed = false;
    let menu = ui.menu_button(RichText::new(format!("{} {}", map.format.extension(), ic::CARET_DOWN)), |ui| {
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
    menu.response.on_hover_text(tr("File format"));
    changed
}

/// The bit depth menu of an output map ("8 bits˅"); templates do not keep dithering yet
fn depth_menu(ui: &mut Ui, map: &mut MapSpec) -> bool {
    let format = map.format;
    let mut changed = false;
    let shown = crate::export_window::depth_label(Some(format), map.bit_depth, false);
    let menu = ui.menu_button(RichText::new(format!("{shown} {}", ic::CARET_DOWN)), |ui| {
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
    menu.response.on_hover_text(tr("Bit depth"));
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

/// The colour of a map on swatches and list markers (None for constants)
fn source_color(source: &SourceRef, tokens: &Tokens) -> Option<Color32> {
    let channels = Channel::ALL.len();
    let converted = VirtualMap::ALL.len();
    let index = match source {
        SourceRef::Channel { channel, .. } => Channel::ALL.iter().position(|c| c == channel)?,
        SourceRef::Virtual { map, .. } => channels + VirtualMap::ALL.iter().position(|m| m == map)?,
        SourceRef::Baked { map, .. } => channels + converted + BakedMap::ALL.iter().position(|m| m == map)?,
        SourceRef::Constant(_) => return None,
    };
    let hue = (index as f32 * HUE_STEP).fract();
    let value = if tokens.dark { SWATCH_VALUE_DARK } else { SWATCH_VALUE_LIGHT };
    Some(egui::ecolor::Hsva::new(hue, SWATCH_SATURATION, value, 1.0).into())
}

/// The bars under a swatch's letters: red, green and blue for a colour map's RGB, else the one
/// channel read (white for gray)
fn swatch_bars(map: &MapSpec, source: &SourceRef, target: ChannelTarget, tokens: &Tokens) -> Vec<Color32> {
    match source {
        SourceRef::Constant(_) => Vec::new(),
        _ if source.components() == 3 && target == ChannelTarget::Rgb && rgb_spread(map) => COMPONENT_COLORS.to_vec(),
        _ if source.components() == 3 => COMPONENT_COLORS.get(usize::from(component_of(source))).copied().into_iter().collect(),
        _ => vec![tokens.text_strong],
    }
}

/// An output map's channel swatches: drop targets for the map lists. A click opens Clear channel
/// (and Separate R, G and B on R, G, B), a right click TextureCraft's own tools.
fn channel_swatches(ui: &mut Ui, index: usize, map: &mut MapSpec, split: &mut bool, edit: &mut RowEdit) -> bool {
    let tokens = Tokens::get(ui.ctx());
    let mut changed = false;
    let shows_split = *split || !rgb_collapsed(map);
    for (slot, target) in swatch_targets(map, *split).into_iter().enumerate() {
        let Some(source) = source_of(map, target).cloned() else { continue };

        let label = target_label(target);
        let inverted = target.dests().iter().all(|dest| map.inverted.contains(dest));
        let tooltip = swatch_tooltip(map, &source, target, inverted);
        let mut name = trf("{channel}: {tooltip}", &[("channel", &label), ("tooltip", &tooltip)]);
        if inverted {
            name = trf("{swatch}, inverted", &[("swatch", &name)]);
        }
        let look = SwatchLook { fill: source_color(&source, &tokens), bars: swatch_bars(map, &source, target, &tokens), inverted };
        let response = swatch(ui, label, &name, &look).on_hover_ui(|ui| {
            ui.label(&tooltip);
            if inverted {
                ui.label(tr("Inverted"));
            }
        });

        // A map dragged over the swatch, or dropped on it; a row's grip passes through to its row
        let map_dragged_over = response.dnd_hover_payload::<SourceRef>().is_some();
        if map_dragged_over {
            drop_highlight(ui, response.rect.expand(DROP_HIGHLIGHT_GAP_POINTS), &tokens);
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

/// The accent frame around a drop target under a dragged map or row
fn drop_highlight(ui: &Ui, rect: egui::Rect, tokens: &Tokens) {
    ui.painter().rect_stroke(rect, tokens.radius, egui::Stroke::new(DROP_HIGHLIGHT_STROKE_POINTS, tokens.accent), egui::StrokeKind::Outside);
}

/// One channel swatch: `label` on top, filled with the source's colour, bars below for the
/// channels it reads, a corner mark when inverted
fn swatch(ui: &mut Ui, label: &str, name: &str, look: &SwatchLook) -> egui::Response {
    let tokens = Tokens::get(ui.ctx());
    let enabled = ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(vec2(SWATCH_SIZE_POINTS, SWATCH_SIZE_POINTS), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name));

    let painter = ui.painter();
    let border = if response.hovered() { tokens.accent } else { tokens.button_border };
    painter.rect(rect, tokens.radius, look.fill.unwrap_or(tokens.button), egui::Stroke::new(1.0, border), egui::StrokeKind::Inside);
    let label_font = egui::FontId::proportional(SWATCH_TEXT_SIZE_POINTS);
    painter.text(rect.center_top() + vec2(0.0, SWATCH_TEXT_TOP_POINTS), egui::Align2::CENTER_TOP, label, label_font, tokens.text_strong);
    let bar_width = SWATCH_BAR_WIDTH_POINTS / look.bars.len().max(1) as f32;
    let left = rect.center().x - SWATCH_BAR_WIDTH_POINTS / 2.0;
    for (k, color) in look.bars.iter().enumerate() {
        let min = egui::pos2(left + k as f32 * bar_width, rect.bottom() - SWATCH_BAR_BOTTOM_POINTS);
        painter.rect_filled(egui::Rect::from_min_size(min, vec2(bar_width, SWATCH_BAR_HEIGHT_POINTS)), 0.0, *color);
    }
    if look.inverted {
        let corner = rect.right_top();
        let mark = vec![corner, corner - vec2(INVERTED_MARK_POINTS, 0.0), corner + vec2(0.0, INVERTED_MARK_POINTS)];
        painter.add(egui::Shape::convex_polygon(mark, tokens.text_strong, egui::Stroke::NONE));
    }
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
    // The app can consume Escape before the popup sees it, so read the raw event
    let escaped = ui.input(|input| input.raw.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. })));
    if picked.is_some() || escaped || shown.is_none_or(|menu| menu.response.should_close()) {
        *pending = None;
    }
    let Some(pick) = picked else { return false };

    apply_pick(map, waiting.target, &waiting.source, pick);
    true
}

/// The maps the selected template's outputs read, with their colours, for the panes' markers
fn marked_sources(app: &TextureCraftApp, tokens: &Tokens) -> Vec<(SourceRef, Color32)> {
    let Some(preset) = app.ui.export.draft.clone().or_else(|| selected(app)) else { return Vec::new() };

    let sources = preset.maps.iter().flat_map(|map| map.channels.iter().map(|(_, source)| source));
    sources.filter_map(|source| source_color(source, tokens).map(|color| (source.clone(), color))).collect()
}

/// The Input maps pane: the input maps in their groups
fn input_maps(ui: &mut Ui, marked: &[(SourceRef, Color32)]) {
    ui.label(caption(tr("Input maps")));
    egui::ScrollArea::vertical().id_salt("exp_inputs").auto_shrink([false; 2]).show(ui, |ui| {
        for (group, entries) in INPUT_MAPS.iter() {
            ui.label(RichText::new(tr(group)).italics());
            map_entries(ui, entries, marked, false);
        }
    });
}

/// The side pane: Mesh maps above Converted maps (in the highlight colour, as in the reference)
fn mesh_and_converted_maps(ui: &mut Ui, height: f32, marked: &[(SourceRef, Color32)]) {
    ui.label(caption(tr("Mesh maps")));
    egui::ScrollArea::vertical()
        .id_salt("exp_mesh_maps")
        .auto_shrink([false; 2])
        .max_height(height * MESH_MAPS_SHARE)
        .show(ui, |ui| map_entries(ui, &MESH_MAPS, marked, false));
    ui.separator();
    ui.label(caption(tr("Converted maps")));
    egui::ScrollArea::vertical().id_salt("exp_converted_maps").auto_shrink([false; 2]).show(ui, |ui| map_entries(ui, &CONVERTED_MAPS, marked, true));
}

fn map_entries(ui: &mut Ui, entries: &[MapEntry], marked: &[(SourceRef, Color32)], converted: bool) {
    let tokens = Tokens::get(ui.ctx());
    let text_color = if converted { tokens.warning } else { tokens.text };
    for (name, source) in entries {
        let marker = source.as_ref().and_then(|source| marked.iter().find(|(used, _)| same_source(used, source)).map(|(_, color)| *color));
        map_entry(ui, tr(name), source.as_ref(), text_color, marker);
    }
}

/// A map list entry to drag onto an output channel, with the colour marker of an output map that
/// reads it; a map TextureCraft does not have yet is greyed and cannot be dragged
fn map_entry(ui: &mut Ui, name: &str, source: Option<&SourceRef>, text_color: Color32, marker: Option<Color32>) {
    let tokens = Tokens::get(ui.ctx());
    let available = source.is_some();
    let sense = if available { egui::Sense::drag() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ENTRY_HEIGHT_POINTS), sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ListItem, available, name));

    let painter = ui.painter();
    let row = rect.shrink2(vec2(0.0, ENTRY_GAP_POINTS / 2.0));
    painter.rect_filled(row, 0.0, if available && response.hovered() { tokens.row_hover } else { tokens.row });
    if let Some(color) = marker {
        painter.rect_filled(
            egui::Rect::from_center_size(row.left_center() + vec2(MARKER_INDENT_POINTS, 0.0), egui::Vec2::splat(MARKER_SIZE_POINTS)),
            1.0,
            color,
        );
    }
    let color = if available { text_color } else { tokens.text_dim.gamma_multiply(UNAVAILABLE_FADE) };
    painter.text(
        row.left_center() + vec2(ENTRY_TEXT_INDENT_POINTS, 0.0),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(ENTRY_TEXT_SIZE_POINTS),
        color,
    );
    let Some(source) = source else {
        response.on_hover_text(tr(NOT_AVAILABLE));
        return;
    };

    let response = response.on_hover_cursor(egui::CursorIcon::Grab);
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
