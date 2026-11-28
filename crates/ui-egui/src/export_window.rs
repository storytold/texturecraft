//! File ▸ Export textures: the SETTINGS, OUTPUT TEMPLATES and LIST OF EXPORTS tabs. The window
//! edits `app.ui.export` live; Cancel (and closing the window) goes back to the settings it
//! opened with, Save settings keeps them.

use egui::{RichText, Ui, vec2};
use egui_phosphor::regular as ic;
use serde_json::json;
use texturecraft_engine::formats::{ExportPreset, FileFormat};

use crate::TextureCraftApp;
#[cfg(not(target_arch = "wasm32"))]
use crate::file_dialogs::{Bound, FileDialog};
use crate::i18n::{n_, tr, trf};
use crate::state::{ConsoleLine, ExportSetPlan, ExportUi, FileType, MapExport, SIZE_OF_LARGEST_SET, SetExport};
use crate::theme::{Tokens, semibold};
use crate::unavailable::NOT_AVAILABLE;

pub(crate) mod drawn;
pub(crate) mod export_look;
mod inherited_settings;
mod map_row;
mod page_place;
mod tree_file;
pub(crate) use export_look::ExportLook;
use inherited_settings::InheritedSettings;
use map_row::MapRow;
use page_place::PagePlace;
use tree_file::TreeFile;

/// The reference window's size; a smaller screen gets a smaller window
const REFERENCE_WINDOW_SIZE_POINTS: egui::Vec2 = egui::vec2(1100.0, 740.0);
/// Space kept free around the content on a small screen: the title bar, the frame and a gap
const WINDOW_VIEWPORT_MARGIN_POINTS: f32 = 72.0;
/// The content runs nearly to the window's frame: one point inside its edge
const CONTENT_INSET: i8 = 1;
/// The widest dilation the slider offers, the cap `export.run` puts on its `dilation`
const MAX_DILATION_PIXELS: u32 = 256;

// Layout in points from the window content's top left corner, measured on the locked mocks

/// The tab row's height, down to the divider under it
pub(crate) const TAB_ROW_HEIGHT: f32 = 42.0;
/// The tabs' top edge
const TAB_TOP: f32 = 16.0;
/// The first tab's left edge
const TABS_LEFT: f32 = 7.5;
/// Middle of the controls at the tab row's right end, 9 points of air above the divider
const TAB_ROW_CONTROLS_Y: f32 = 19.5;
/// From the window's left edge to the buttons there
const LEFT_MARGIN: f32 = 7.5;
/// From the window's right edge to the buttons there
const RIGHT_MARGIN: f32 = 10.5;
/// The bottom bar's height under its divider
const FOOTER_HEIGHT: f32 = 46.0;
/// Middle of the bottom bar's buttons, from the window's bottom
const FOOTER_MIDDLE_FROM_BOTTOM: f32 = 22.5;
/// Width of the bottom bar's buttons, the same for all as in the reference
const FOOTER_BUTTON_WIDTH: f32 = 117.5;
/// Gap between the bottom bar's buttons
const FOOTER_BUTTON_GAP: f32 = 6.5;
/// Air between the texture count and Cancel
const COUNT_GAP: f32 = 14.5;
/// Open output directory: the air around its name
const OPEN_DIRECTORY_PADDING: f32 = 24.0;
/// Open output directory: its distance from the window's right edge
const OPEN_DIRECTORY_FROM_RIGHT: f32 = 12.0;
/// The strip at a list's right edge that holds its scroll bar
pub(crate) const GUTTER: f32 = 12.0;
/// Size of the menu (≡) and reset glyphs of the lists and pages
const MENU_ICON_SIZE: f32 = 14.0;

/// Width of the texture set list
const SET_LIST_WIDTH: f32 = 240.7;
/// Centre of the set list's menu, x
const SET_MENU_X: f32 = 227.5;
/// Centre of the set list's menu, y
const SET_MENU_Y: f32 = 57.0;
/// Top of the set list's first row, Global settings
const SET_ROWS_TOP: f32 = 70.0;
/// Height of a set list row
const SET_ROW_HEIGHT: f32 = 25.0;
/// Where a set row's check box starts
const SET_CHECK_LEFT: f32 = 18.8;
/// Where a set row's name starts
const SET_NAME_LEFT: f32 = 36.7;
/// Where Global settings starts, having no check box
const GLOBAL_NAME_LEFT: f32 = 16.0;
/// The override mark's distance from its row's end
const OVERRIDE_MARK_FROM_RIGHT: f32 = 14.0;
/// Side of a check box's click area
const CHECK_HIT_SIZE: f32 = 20.0;
/// How far a check box's click area reaches past its box on each side
const CHECK_HIT_OVERHANG: f32 = 3.5;

/// A page's label column
const LABEL_LEFT: f32 = 257.6;
/// Where a page's controls start
const CONTROLS_LEFT: f32 = 455.0;
/// Where a page's controls end, from the window's right edge
const CONTROLS_RIGHT_MARGIN: f32 = 112.8;
/// A page header's glyph, from the window's right edge
const PAGE_ICON_FROM_RIGHT: f32 = 22.6;
/// Middle of the Global settings page's header
const GLOBAL_HEADER_Y: f32 = 71.0;
/// Middle of its reset glyph
const GLOBAL_ICON_Y: f32 = 70.0;
/// Middle of its first parameter row
const GLOBAL_FIRST_ROW_Y: f32 = 115.0;
/// Pitch of its parameter rows
const GLOBAL_ROW_PITCH: f32 = 35.5;
/// From the last parameter row to the first tick
const CHECKS_GAP: f32 = 30.0;
/// Pitch of the ticks
const CHECK_PITCH: f32 = 24.0;
/// The page's least width before it scrolls
const MIN_PAGE_WIDTH: f32 = 640.0;
/// The Global settings page's height under the tab row
const GLOBAL_PAGE_HEIGHT: f32 = 376.0;
/// The ticks sit a point in from the labels
const CHECK_INDENT: f32 = 1.0;
/// A tick's label starts after its box and the gap
const CHECK_LABEL_ROOM: f32 = 18.0;
/// Width of the output directory's "…" and reset buttons
#[cfg(not(target_arch = "wasm32"))]
const BROWSE_WIDTH: f32 = 22.0;
/// Gap before them
#[cfg(not(target_arch = "wasm32"))]
const BROWSE_GAP: f32 = 4.0;
/// Room the file type list keeps for its chevron past its longest entry
const FILE_TYPE_ROOM: f32 = 36.0;
/// Gap between two controls on one row
const PAIR_GAP: f32 = 6.6;
/// Width of the dilation width's dropdown
const DILATION_WIDTH: f32 = 56.4;

/// Middle of a texture set page's header
const SET_HEADER_Y: f32 = 74.0;
/// Middle of its first override row
const SET_FIRST_ROW_Y: f32 = 121.0;
/// Pitch of its override rows
const SET_ROW_PITCH: f32 = 36.0;
/// Centre of the override pens
const PEN_CENTRE: f32 = 464.4;
/// Where an inherited value's text starts
const VALUE_LEFT: f32 = 479.4;
/// Where an override's control starts
const OVERRIDE_LEFT: f32 = 480.3;
/// The divider above Output maps
const SET_DIVIDER_Y: f32 = 257.0;
/// Middle of the Output maps header
const MAPS_HEADER_Y: f32 = 288.0;
/// Top of the first Output maps row
const MAPS_TOP: f32 = 318.0;
/// Pitch of the Output maps rows
const MAP_ROW_PITCH: f32 = 34.0;
/// Leave room for two map rows when the texture-set page scrolls vertically.
const MIN_SET_PAGE_HEIGHT: f32 = MAPS_TOP - TAB_ROW_HEIGHT + 2.0 * MAP_ROW_PITCH;
/// Where an Output maps row's tick starts
const MAP_CHECK_LEFT: f32 = 258.6;
/// Where its file name starts
const MAP_NAME_LEFT: f32 = 280.1;
/// The inherited bit depth's text, after the file type's
const BITS_TEXT_OFFSET: f32 = 32.0;
/// Width of a map's file type list
const MAP_FORMAT_WIDTH: f32 = 61.2;
/// Width of a map's own size list
const MAP_SIZE_WIDTH: f32 = 100.0;

/// The console's least width: the reference's 300 at TextureCraft's narrower window
pub(crate) const CONSOLE_WIDTH: f32 = 284.0;
/// The file tree's least width
const MIN_TREE_WIDTH: f32 = 300.0;
/// Width of the strip that folds the console
const STRIP_WIDTH: f32 = 17.9;
/// Gap between the file tree and the strip
const STRIP_GAP: f32 = 1.9;
/// Size of the strip's glyph
const STRIP_GLYPH_SIZE: f32 = 13.0;
/// The console text's indent
const CONSOLE_TEXT_INDENT: f32 = 18.0;
/// Middle of the console's header
const CONSOLE_HEADER_Y: f32 = 66.0;
/// Where the console's lines start
const CONSOLE_BODY_TOP: f32 = 89.0;
/// Air above and below the console's lines
const CONSOLE_PADDING: f32 = 6.0;
/// Air above the file tree's first row
const TREE_TOP_PADDING: f32 = 3.0;
/// Height of a file tree row
const TREE_ROW_HEIGHT: f32 = 21.0;
/// Centre of a set's chevron
const TREE_CHEVRON_X: f32 = 13.2;
/// Where a set's name starts
const TREE_NAME_INDENT: f32 = 26.3;
/// Where its files start
const TREE_FILE_INDENT: f32 = 45.1;
/// Gap after the name and size columns
const TREE_COLUMN_GAP: f32 = 28.0;
/// Gap after the file type column
const TREE_BITS_GAP: f32 = 14.0;

/// The output sizes the Size list offers, in texels per side
pub(crate) const SIZES: [u32; 7] = [128, 256, 512, 1024, 2048, 4096, 8192];

/// Templates the reference lists above the separator, in its order (one more carries a vendor
/// name and is left out). TextureCraft has some of them; the others are listed greyed.
const FIRST_GROUP: [&str; 7] = [
    "2D View",
    "Document channels + Normal + AO (No Alpha)",
    "Document channels + Normal + AO (With Alpha)",
    "Sketchfab",
    "USDz (Apple AR)",
    "glTF PBR Metal Roughness",
    "glTF PBR Metal Roughness + Displacement texture (experimental)",
];

/// First-group templates a texture set cannot override to (the reference leaves them out there)
const NOT_PER_SET: [&str; 4] = ["Sketchfab", "USDz (Apple AR)", "glTF PBR Metal Roughness", "glTF PBR Metal Roughness + Displacement texture (experimental)"];

/// The File type list in the reference order: (name, the format; None for the ones TextureCraft
/// cannot write yet, listed greyed)
pub(crate) const FILE_TYPES: [(&str, Option<FileFormat>); 14] = [
    ("bmp", Some(FileFormat::Bmp)),
    ("jpeg", Some(FileFormat::Jpeg)),
    ("jng", Some(FileFormat::Jng)),
    ("pgm", Some(FileFormat::Pgm)),
    ("png", Some(FileFormat::Png)),
    ("targa", Some(FileFormat::Tga)),
    ("tiff", Some(FileFormat::Tiff)),
    ("hdr", Some(FileFormat::Hdr)),
    ("exr", Some(FileFormat::Exr)),
    ("j2k", None),
    ("jp2", None),
    ("webp", Some(FileFormat::Webp)),
    ("psd", Some(FileFormat::Psd)),
    ("sbsar", None),
];

/// The bit depths `format` is written with, as the bit depth list offers them: (bits,
/// dithering)
pub(crate) fn depth_options(format: FileFormat) -> Vec<(u8, bool)> {
    format.bit_depths().iter().flat_map(|&d| if d == 8 && !format.holds_float(8) { vec![(8, false), (8, true)] } else { vec![(d, false)] }).collect()
}

/// A bit depth as listed: "8 bits", "8 bits + dithering", "16 bits", "16f bits", "32f bits"
pub(crate) fn depth_label(format: Option<FileFormat>, bit_depth: u8, dithering: bool) -> String {
    match (bit_depth, dithering) {
        (8, true) => tr("8 bits + dithering").to_string(),
        (16, _) if format == Some(FileFormat::Exr) => tr("16f bits").to_string(),
        (32, _) => tr("32f bits").to_string(),
        (n, _) => trf("{n} bits", &[("n", &n)]),
    }
}

/// A file type list and its bit depth list in `rects`; `from_template` adds "Based on output template" (an empty `format`)
fn file_type_lists(
    ui: &mut Ui,
    id: (&str, usize, usize),
    from_template: bool,
    rects: [egui::Rect; 2],
    format: &mut String,
    bit_depth: &mut u8,
    dithering: &mut bool,
) {
    let parsed = FileFormat::from_extension(format);

    // A depth the type does not have (or none) becomes its default
    if let Some(f) = parsed
        && !depth_options(f).contains(&(*bit_depth, *dithering))
    {
        (*bit_depth, *dithering) = (f.default_depth(), false);
    }
    let shown = if format.is_empty() { tr("Based on output template").to_string() } else { file_type_name(format) };
    drawn::combo(ui, rects[0], ("type", id), shown, |ui| {
        if from_template {
            if ui.selectable_label(format.is_empty(), tr("Based on output template")).clicked() {
                format.clear();
                (*bit_depth, *dithering) = (0, false);
            }
            ui.separator();
        }
        for (name, f) in FILE_TYPES {
            match f {
                Some(f) => {
                    if ui.selectable_label(file_type_name(format) == name, name).clicked() && file_type_name(format) != name {
                        *format = name.to_string();
                        (*bit_depth, *dithering) = (f.default_depth(), false);
                    }
                }
                None => {
                    ui.add_enabled(false, egui::Button::selectable(false, name)).on_disabled_hover_text(tr(NOT_AVAILABLE));
                }
            }
        }
    });

    // A file type with one bit depth (or none) shows its bit depth list greyed
    let options = parsed.map(depth_options).unwrap_or_default();
    let shown = if parsed.is_some() { depth_label(parsed, *bit_depth, *dithering) } else { String::new() };
    if options.len() < 2 {
        drawn::disabled_combo(ui, rects[1], ("bits", id), &shown, tr("Bit depth"));
        return;
    }

    drawn::combo(ui, rects[1], ("bits", id), shown, |ui| {
        for (depth, dither) in options {
            if ui.selectable_label((*bit_depth, *dithering) == (depth, dither), depth_label(parsed, depth, dither)).clicked() {
                (*bit_depth, *dithering) = (depth, dither);
            }
        }
    });
}

/// The padding types, in the reference order: (`file.exportTextures` value, label)
pub(crate) const PADDINGS: [(&str, &str); 5] = [
    ("none", n_("No padding (passthrough)")),
    ("infinite", n_("Dilation infinite")),
    ("dilation", n_("Dilation + transparent")),
    ("dilationDefault", n_("Dilation + default background color")),
    ("diffusion", n_("Dilation + diffusion")),
];

pub(crate) fn padding_label(padding: &str) -> &'static str {
    PADDINGS.iter().find(|(v, _)| *v == padding).map_or("Dilation infinite", |(_, l)| l)
}

/// Paddings with a width: the others ignore the dilation
pub(crate) fn has_width(padding: &str) -> bool {
    matches!(padding, "dilation" | "dilationDefault" | "diffusion")
}

/// The name a stored file type is listed under ("" when the template decides)
fn file_type_name(format: &str) -> String {
    FileFormat::from_extension(format).map_or_else(|| format.to_string(), |f| f.name().to_string())
}

/// The label shown for a stored template name. Custom template names pass through unchanged.
pub(super) fn template_label<'a>(session: &texturecraft_engine::Session, name: &'a str) -> &'a str {
    use texturecraft_engine::preset_cmds::unqualified_preset_name;

    let label = match unqualified_preset_name(name) {
        "Unreal Engine Subsurface Profile (Packed)" => tr("Packed maps with a scattering mask"),
        "Redshift Standard Material (separate maps)" => tr("Separate coat, sheen and scattering maps"),
        "Roblox SurfaceAppearance (separate maps)" => tr("Four maps with opacity in color"),
        "Sketchfab Metalness (separate maps)" => tr("Six maps with a negative-Y normal"),
        _ => return name,
    };

    if session.has_user_export_preset(name) { name } else { label }
}

/// The template list: the first group (name, whether TextureCraft has it), then the others
/// (built-in and user templates) sorted by name.
pub(crate) fn template_groups(app: &TextureCraftApp, builtins: &[ExportPreset]) -> (Vec<(&'static str, bool)>, Vec<String>) {
    let first = FIRST_GROUP.iter().map(|name| (*name, builtins.iter().any(|p| p.name == *name))).collect();
    let mut rest: Vec<String> = builtins
        .iter()
        .filter(|preset| !FIRST_GROUP.contains(&preset.name.as_str()))
        .map(|preset| app.session.builtin_export_preset_id(&preset.name).into_owned())
        .collect();
    rest.extend(app.session.export_presets.list.iter().map(|p| p.name.clone()));
    rest.sort_by(|left, right| template_label(&app.session, left).cmp(template_label(&app.session, right)).then_with(|| left.cmp(right)));
    rest.dedup();
    (first, rest)
}

/// A template dropdown in `rect` over `first` and `rest`; true when `current` changed
fn template_combo(
    session: &texturecraft_engine::Session,
    ui: &mut Ui,
    rect: egui::Rect,
    id: &str,
    current: &mut String,
    first: &[(&str, bool)],
    rest: &[String],
) -> bool {
    let mut changed = false;
    drawn::combo(ui, rect, id, template_label(session, current).to_owned(), |ui| {
        for (name, have) in first {
            if *have {
                changed |= ui.selectable_value(current, (*name).to_string(), *name).on_hover_text(*name).changed();
            } else {
                ui.add_enabled(false, egui::Button::selectable(false, *name)).on_disabled_hover_text(tr(NOT_AVAILABLE));
            }
        }
        ui.separator().on_hover_text(tr("Some templates in this list are unavailable"));
        for name in rest {
            changed |= ui.selectable_value(current, name.clone(), template_label(session, name)).on_hover_text(name).changed();
        }
    });
    changed
}

/// The number shown for an output size
fn size_label(size: u32) -> String {
    match size {
        0 => tr("Based on each Texture Set's size").to_string(),
        SIZE_OF_LARGEST_SET => tr("Maximum size for all Texture Sets").to_string(),
        n => n.to_string(),
    }
}

/// The displayed block dimensions at an export width
fn scaled_block_size(width: u32, native: (u32, u32)) -> (u32, u32) {
    (width, texturecraft_engine::formats::export::scale_export_side(native.1, native.0, Some(width)))
}

/// Display both dimensions and mark the texture set's own size
fn set_size_label(size: (u32, u32), native: (u32, u32)) -> String {
    let (width, height) = size;
    if size == native { trf("{width}x{height} (Texture Set size)", &[("width", &width), ("height", &height)]) } else { format!("{width}x{height}") }
}

/// The largest texture set's resolution
fn largest_set(app: &TextureCraftApp) -> u32 {
    app.session.doc.project.texture_sets.iter().map(|t| t.block_resolution()).max().unwrap_or(1024)
}

/// Texture sets ticked in the export dialog.
fn export_selection(app: &TextureCraftApp) -> Vec<usize> {
    (0..app.session.set_count()).filter(|i| !app.ui.export.skip.contains(i)).collect()
}

/// Start exporting the selected texture sets: the window shows LIST OF EXPORTS, the console and
/// a progress bar, and writes one texture set per frame. Each set's settings and the output
/// folder are taken now, so changing them while it runs does not change what it writes.
#[cfg(not(target_arch = "wasm32"))]
fn start_export(app: &mut TextureCraftApp) {
    let selection = export_selection(app);
    if selection.is_empty() {
        return;
    }

    let dir = app.ui.export.dir.clone();
    let pending: Vec<(usize, serde_json::Value)> = selection
        .into_iter()
        .map(|set_index| {
            let mut params = set_params(app, set_index);
            params["dir"] = json!(dir);

            // Capture the template so edits during export cannot change the files
            let preset = params.get("preset").and_then(serde_json::Value::as_str).and_then(|name| app.session.export_preset(name));
            if let Some(preset) = preset.and_then(|preset| serde_json::to_value(preset).ok()) {
                params["preset"] = preset;
            }

            (set_index, params)
        })
        .collect();

    let preview: Vec<_> = pending.iter().filter_map(|(index, params)| plan_set(app, *index, params)).collect();
    let error = preview.iter().find_map(|plan| plan.files.as_ref().err().cloned()).or_else(|| name_clash(&preview)).or_else(|| {
        let paths = preview.iter().filter_map(|plan| plan.files.as_ref().ok()).flat_map(planned_names);
        texturecraft_engine::formats::export::validate_export_paths(paths).err().map(|error| trf("Export stopped: {error}", &[("error", &error)]))
    });

    let e = &mut app.ui.export;
    e.tab = 2;
    e.console = vec![ConsoleLine::Step(tr("Preparing texture export").into())];
    if let Some(error) = error {
        e.console.push(ConsoleLine::Step(error.clone()));
        app.toast(error, true);
        return;
    }

    e.job = Some(crate::state::ExportJob {
        total: pending.len(),
        pending,
        preview,
        directory: dir,
        usd_asset: e.usd_asset,
        shader_parameters: e.shader_parameters,
        ..Default::default()
    });
}

/// Two texture sets that would write a file of the same name (ignoring case, as Windows and macOS
/// do): the later one would replace the earlier one's file, so the export does not start.
#[cfg(not(target_arch = "wasm32"))]
fn name_clash(preview: &[ExportSetPlan]) -> Option<String> {
    let mut seen = std::collections::HashMap::new();
    for (index, set) in preview.iter().enumerate() {
        let Ok(files) = &set.files else { continue };
        for file in planned_names(files) {
            let (first_index, first_name) = *seen.entry(file.to_ascii_lowercase()).or_insert((index, &set.name));
            if first_index != index {
                return Some(trf(
                    "Texture sets {first} and {second} both export {file}: add $textureSet to the template's file names or rename a texture set",
                    &[("first", first_name), ("second", &set.name), ("file", &file)],
                ));
            }
        }
    }
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn planned_names(plan: &serde_json::Value) -> impl Iterator<Item = &str> {
    plan.as_array().into_iter().flatten().filter_map(|file| file.get("file").and_then(serde_json::Value::as_str))
}

fn planned_file_shapes(plan: &serde_json::Value) -> impl Iterator<Item = [&serde_json::Value; 3]> {
    plan.as_array().into_iter().flatten().map(|file| [&file["file"], &file["size"], &file["height"]])
}

/// In the browser the textures download (one zip for several files).
#[cfg(target_arch = "wasm32")]
fn start_export(app: &mut TextureCraftApp) {
    let jobs: Vec<(usize, serde_json::Value)> = export_selection(app).into_iter().map(|ts| (ts, set_params(app, ts))).collect();
    crate::web_files::export_textures(app, &jobs);
}

/// Write the next texture set of the running export, logging each step;
/// the last one finishes the export. A failed command (its error already shown) stops it.
fn export_step(app: &mut TextureCraftApp, frame: u64) {
    let Some(job) = app.ui.export.job.as_mut() else { return };
    if job.frame == frame {
        return;
    }

    job.frame = frame;
    if job.pending.is_empty() {
        let written = std::mem::take(&mut job.written);
        let console = &mut app.ui.export.console;
        console.push(ConsoleLine::Step(tr("All selected texture sets processed").into()));
        if written.is_empty() {
            console.push(ConsoleLine::Step(tr("No files were written").into()));
        } else {
            console.extend([
                ConsoleLine::Step(tr("Collecting the exported files").into()),
                ConsoleLine::Done(tr("Texture export complete").into()),
                ConsoleLine::Done(tr("Files saved").into()),
            ]);
            console.extend(written.into_iter().map(ConsoleLine::Done));
        }

        app.ui.export.job = None;
        return;
    }

    let (set_index, mut params) = job.pending.remove(0);
    let name = app.session.doc.project.texture_sets.get(set_index).map(|t| t.name.clone()).unwrap_or_default();

    // Project edits can invalidate the frozen filenames or output sizes
    params["ts"] = json!(set_index);
    if let Some(Ok(initial)) = job.preview.iter().find(|plan| plan.index == Some(set_index)).map(|plan| &plan.files) {
        let current = texturecraft_engine::execute(&mut app.session, "export.plan", &params);
        if !current.is_ok_and(|current| planned_file_shapes(&current).eq(planned_file_shapes(initial))) {
            let error = trf("Export stopped: the planned files for {set} changed", &[("set", &name)]);
            app.ui.export.job = None;
            app.ui.export.console.push(ConsoleLine::Step(error.clone()));
            app.toast(error, true);
            return;
        }
    }

    let set = || ConsoleLine::Set(name.clone());
    app.ui.export.console.extend([set(), set(), ConsoleLine::Step(tr("Rendering the selected maps").into())]);

    // Protect concrete files and UDIM references from earlier texture sets
    if let Some(job) = app.ui.export.job.as_ref().filter(|job| !job.written.is_empty()) {
        params["reservedNames"] = job.written.iter().chain(&job.material_udim_patterns).map(|name| json!(name)).collect();
    }

    let directory = std::path::PathBuf::from(params["dir"].as_str().unwrap_or_default());

    // The last command writes the USD asset from the params of every command, so a canceled or failed export writes none
    if let Some(job) = app.ui.export.job.as_mut().filter(|job| job.usd_asset) {
        job.sent.push(params.clone());
        if job.pending.is_empty() {
            params["usdAsset"] = json!(job.sent);
        }
    }

    // The last command writes the shader parameters, listing every texture set of the export
    if let Some(job) = app.ui.export.job.as_ref().filter(|job| job.shader_parameters && job.pending.is_empty()) {
        params["shaderParameters"] = job.preview.iter().filter_map(|plan| plan.index).collect();
    }

    let Some(response) = app.run("file.exportTextures", params) else {
        app.ui.export.job = None;
        return;
    };

    // A USD asset that could not be written also shows as an error toast
    let usd_failure = response_strings(&response, "warnings").find(|warning| warning.starts_with(texturecraft_engine::file_cmds::USD_ASSET_FAILED));
    if let Some(warning) = usd_failure {
        app.toast(warning.to_string(), true);
    }

    // Names relative to the output folder keep a template's folders
    let relative = |file: &str| {
        let path = std::path::Path::new(file);
        match path.strip_prefix(&directory) {
            Ok(inside) => inside.components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/"),
            Err(_) => path.file_name().map_or_else(|| file.to_string(), |name| name.to_string_lossy().into_owned()),
        }
    };
    let files: Vec<String> = response_strings(&response, "files").map(relative).collect();
    let mut lines = vec![set(), ConsoleLine::Step(tr("Saving texture files").into())];

    // The USD asset and the shader parameters are no textures: they stay in the list of written files only
    for file in files.iter().filter(|file| !file.to_ascii_lowercase().ends_with(".usd") && !file.to_ascii_lowercase().ends_with(".json")) {
        let stem = file.rsplit_once('.').map_or(file.as_str(), |(stem, _)| stem);
        lines.push(set());
        lines.push(ConsoleLine::Step(trf("Saved texture {file}", &[("file", &stem)])));
    }

    // The export's warnings go to the console and the log, each once per export
    let warnings = response_strings(&response, "warnings");
    for warning in warnings {
        let Some(job) = app.ui.export.job.as_mut() else { break };
        if job.shown_warnings.iter().any(|seen| seen == warning) {
            continue;
        }

        job.shown_warnings.push(warning.to_string());
        app.session.log.warn("MapExporter", warning);
        lines.push(set());
        lines.push(ConsoleLine::Step(trf("Warning: {message}", &[("message", &warning)])));
    }

    lines.extend([set(), ConsoleLine::Step(tr("Finished writing this texture set").into())]);
    app.ui.export.console.extend(lines);
    if let Some(job) = app.ui.export.job.as_mut() {
        job.written.extend(files);
        job.material_udim_patterns.extend(response_strings(&response, "materialUdimPatterns").map(str::to_owned));
    }
}

fn response_strings<'a>(response: &'a serde_json::Value, field: &str) -> impl Iterator<Item = &'a str> {
    response.get(field).and_then(|items| items.as_array()).into_iter().flatten().filter_map(|item| item.as_str())
}

/// The `file.exportTextures` / `export.plan` parameters of the window's global settings
pub(crate) fn export_params(app: &TextureCraftApp) -> serde_json::Value {
    let settings = &app.ui.export;
    let mut params = json!({"preset": settings.preset, "padding": settings.padding, "dilation": settings.dilation});
    if !settings.format.is_empty() {
        params["format"] = json!(settings.format);
    }
    if !settings.format.is_empty() && settings.bit_depth > 0 {
        params["bitDepth"] = json!(settings.bit_depth);
        params["dithering"] = json!(settings.dithering);
    }
    match settings.size {
        0 => {}
        SIZE_OF_LARGEST_SET => params["size"] = json!(largest_set(app)),
        size => params["size"] = json!(size),
    }
    if !settings.normal_format.is_empty() {
        params["normalFormat"] = json!(settings.normal_format);
    }
    if !settings.color_space.is_empty() {
        params["colorSpace"] = json!(settings.color_space);
    }

    // The browser downloads the textures only
    #[cfg(not(target_arch = "wasm32"))]
    if settings.materialx {
        params["materialX"] = json!(true);
    }

    params
}

/// The parameters exporting texture set `ts`: the global settings with the set's overrides
pub(crate) fn set_params(app: &TextureCraftApp, set_index: usize) -> serde_json::Value {
    let mut params = export_params(app);
    params["ts"] = json!(set_index);
    let Some(overrides) = app.ui.export.sets.get(&set_index) else { return params };

    if let Some(preset) = &overrides.preset {
        params["preset"] = json!(preset);
    }

    // An empty override inherits the template's file type
    match &overrides.file_type {
        Some(file_type) if file_type.format.is_empty() => {
            if let Some(params) = params.as_object_mut() {
                for key in ["format", "bitDepth", "dithering"] {
                    params.remove(key);
                }
            }
        }
        Some(file_type) => {
            params["format"] = json!(file_type.format);
            params["bitDepth"] = json!(file_type.bit_depth);
            params["dithering"] = json!(file_type.dithering);
        }
        None => {}
    }
    if let Some(size) = overrides.size {
        params["size"] = json!(size);
    }
    if let Some((padding, dilation)) = &overrides.padding {
        params["padding"] = json!(padding);
        params["dilation"] = json!(dilation);
    }

    // Ignore stale indices when a template loses maps
    if !overrides.maps.is_empty() && params.get("preset").and_then(serde_json::Value::as_str) == Some(overrides.maps_template.as_str()) {
        let map_count = app.session.export_preset(&overrides.maps_template).map_or(0, |preset| preset.maps.len());
        params["maps"] = overrides
            .maps
            .iter()
            .filter(|(map_index, _)| **map_index < map_count)
            .map(|(map_index, map)| {
                let mut settings = json!({"map": map_index, "enabled": map.enabled});
                if let Some(file_type) = &map.file_type {
                    settings["format"] = json!(file_type.format);
                    settings["bitDepth"] = json!(file_type.bit_depth);
                    settings["dithering"] = json!(file_type.dithering);
                }
                if let Some(size) = map.size {
                    settings["size"] = json!(size);
                }
                settings
            })
            .collect();
    }
    params
}

/// The template texture set `ts` exports with
fn set_template(app: &TextureCraftApp, ts: usize) -> String {
    app.ui.export.sets.get(&ts).and_then(|o| o.preset.clone()).unwrap_or_else(|| app.ui.export.preset.clone())
}

/// A file type and bit depth as an Output maps row shows them ("png   8 bits"), and the two apart
fn file_type_text(t: &FileType) -> String {
    let (format, bits) = file_type_parts(t);
    format!("{format}   {bits}")
}

fn file_type_parts(t: &FileType) -> (String, String) {
    (file_type_name(&t.format), depth_label(FileFormat::from_extension(&t.format), t.bit_depth, t.dithering))
}

/// The rows of texture set `ts`'s Output maps: the template's maps the set has sources for
fn map_rows(app: &TextureCraftApp, ts: usize) -> Vec<MapRow> {
    let mut params = set_params(app, ts);

    // Discover maps from their sources so invalid output sizes cannot hide the controls that fix them
    if let Some(object) = params.as_object_mut() {
        object.remove("maps");
        object.remove("size");
    }

    let Ok(options) = texturecraft_engine::file_cmds::export_options(&app.session, &params, "export.plan") else { return Vec::new() };
    let Ok(set) = app.session.doc.project.texture_set(ts) else { return Vec::new() };

    let mut rows = Vec::new();
    let mut psd_rows = Vec::new();
    let mut psd_depth = 8;
    let mut psd_dithering = false;
    for (map, spec) in options.preset.maps.iter().enumerate().filter(|(_, spec)| spec.is_available_for(set)) {
        let (format, bit_depth, dithering) = options.map_file_type(map, spec);
        let pattern = spec.file.replace("$channel", texturecraft_engine::formats::export::channel_label(spec));
        let row = MapRow { map, pattern, inherited: FileType { format: format.name().to_owned(), bit_depth, dithering } };
        if format == FileFormat::Psd {
            psd_depth = psd_depth.max(bit_depth);
            psd_dithering |= dithering;
            psd_rows.push(row);
        } else {
            rows.push(row);
        }
    }

    // All PSD rows inherit the document's file type and follow the standalone maps
    for mut row in psd_rows {
        row.inherited.bit_depth = psd_depth;
        row.inherited.dithering = psd_depth == 8 && psd_dithering;
        rows.push(row);
    }

    rows
}

/// The Output maps section of texture set `ts`'s page: its header, then its rows scrolling in `area`
fn output_maps(app: &mut TextureCraftApp, ui: &mut Ui, ts: usize, place: PagePlace, area: egui::Rect) {
    let look = ExportLook::get(ui.ctx());
    let Some(set) = app.session.doc.project.texture_sets.get(ts) else { return };
    let native = set.block_size();
    let rows = map_rows(app, ts);
    let template = set_template(app, ts);
    let overrides = app.ui.export.sets.entry(ts).or_default();
    if overrides.maps_template != template {
        overrides.maps.clear();
        overrides.maps_template = template;
    }

    let mut menu = None;
    drawn::label_at(ui, place.x(LABEL_LEFT), place.y(MAPS_HEADER_Y), tr("Output maps"), look.text, true);
    list_menu(ui, place.right_inset(PAGE_ICON_FROM_RIGHT, MAPS_HEADER_Y), tr("Output maps menu"), |ui| {
        for (i, label) in [tr("Check all"), tr("Uncheck all"), tr("Invert all")].into_iter().enumerate() {
            if ui.button(label).clicked() {
                menu = Some(i);
            }
        }
        ui.separator();
        for (i, label) in [tr("Override every item"), tr("Clear all overrides")].into_iter().enumerate() {
            if ui.button(label).clicked() {
                menu = Some(3 + i);
            }
        }
        ui.separator();
    });

    // The rows scroll under the header, the scroll bar in its strip at the window's right edge
    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
        egui::ScrollArea::vertical().id_salt(("exp_maps_scroll", ts)).auto_shrink([false; 2]).show(ui, |ui| {
            let height = rows.len() as f32 * MAP_ROW_PITCH;
            let (rows_rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::hover());
            for (index, row) in rows.iter().enumerate() {
                let y = rows_rect.top() + (index as f32 + 0.5) * MAP_ROW_PITCH;
                map_row(ui, place, y, ts, native, row, overrides.maps.entry(row.map).or_default());
            }
        });
    });

    for row in &rows {
        let map_export = overrides.maps.entry(row.map).or_default();
        match menu {
            Some(0) => map_export.enabled = true,
            Some(1) => map_export.enabled = false,
            Some(2) => map_export.enabled = !map_export.enabled,
            Some(3) if map_export.file_type.is_none() => map_export.file_type = Some(row.inherited.clone()),
            Some(4) => (map_export.file_type, map_export.size) = (None, None),
            _ => {}
        }
    }
    overrides.maps.retain(|_, map_export| *map_export != MapExport::default());
}

/// One Output maps row at `y`: tick, file name cut at the front, override pen, file type, the map's own size
fn map_row(ui: &mut Ui, place: PagePlace, y: f32, ts: usize, native: (u32, u32), row: &MapRow, map_export: &mut MapExport) {
    let look = ExportLook::get(ui.ctx());
    let check_left = place.x(MAP_CHECK_LEFT);
    let hit = egui::Rect::from_center_size(egui::pos2(check_left + CHECK_HIT_SIZE / 2.0 - CHECK_HIT_OVERHANG, y), vec2(CHECK_HIT_SIZE, CHECK_HIT_SIZE));
    let name_left = place.x(MAP_NAME_LEFT);
    let shown = drawn::fitted(ui, &row.pattern, place.x(CONTROLS_LEFT) - name_left, true, false);
    drawn::check(ui, ui.scope_id().with(("exp_map_tick", ts, row.map)), hit, egui::pos2(check_left, y), &mut map_export.enabled, None, &shown);
    let color = if map_export.enabled { look.text } else { look.text_dim };
    drawn::label_at(ui, name_left, y, &shown, color, false).on_hover_text(&row.pattern);

    let pen = egui::pos2(place.x(PEN_CENTRE), y);
    let name = trf("Enable override for map {number} ({pattern})", &[("number", &(row.map + 1)), ("pattern", &row.pattern)]);
    if drawn::icon_button(ui, pen, ic::PENCIL_SIMPLE, drawn::CHEVRON_SIZE, &name, true, map_export.file_type.is_some()).clicked() {
        map_export.file_type = if map_export.file_type.is_some() { None } else { Some(row.inherited.clone()) };
    }

    // The map's size list keeps the row's right end; the file type and bit depth come before it
    let right = place.right - CONTROLS_RIGHT_MARGIN;
    let size_rect = drawn::control_rect(right - MAP_SIZE_WIDTH, right, y);
    match &mut map_export.file_type {
        Some(FileType { format, bit_depth, dithering }) => {
            let format_rect = drawn::control_rect(place.x(OVERRIDE_LEFT), place.x(OVERRIDE_LEFT) + MAP_FORMAT_WIDTH, y);
            let bits_rect = drawn::control_rect(format_rect.right() + PAIR_GAP, size_rect.left() - PAIR_GAP, y);
            file_type_lists(ui, ("exp_map", ts, row.map), false, [format_rect, bits_rect], format, bit_depth, dithering);
        }
        None => inherited_file_type(ui, place, y, &row.inherited),
    }
    map_size_combo(ui, size_rect, ("exp_map_size", ts, row.map), native, &mut map_export.size);
}

/// A map's inherited file type: "png" and "8 bits" in their columns, one text to assistive technology
fn inherited_file_type(ui: &mut Ui, place: PagePlace, y: f32, inherited: &FileType) {
    let look = ExportLook::get(ui.ctx());
    let (format, bits) = file_type_parts(inherited);
    let format_rect = drawn::text_at(ui, place.x(VALUE_LEFT), y, &format, look.text, false);
    let bits_rect = drawn::text_at(ui, place.x(VALUE_LEFT + BITS_TEXT_OFFSET), y, &bits, look.text, false);
    let text = file_type_text(inherited);
    let response =
        ui.interact(format_rect.union(bits_rect), ui.scope_id().with(("exp_map_inherited", place.y(0.0).to_bits(), y.to_bits())), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &text));
}

/// An output map's size in `rect`: the export's, or its own, which wins over it
fn map_size_combo(ui: &mut Ui, rect: egui::Rect, id: (&str, usize, usize), native: (u32, u32), size: &mut Option<u32>) {
    let label = |width| {
        let (_, height) = scaled_block_size(width, native);
        format!("{width}x{height}")
    };
    let shown = size.map_or_else(|| tr("Export size").to_string(), label);
    drawn::combo(ui, rect, id, shown, |ui| {
        ui.selectable_value(size, None, tr("Export size"));
        ui.separator();
        for side in SIZES {
            ui.selectable_value(size, Some(side), label(side));
        }
    })
    .response
    .on_hover_text(tr("This map's own size, which wins over the export size"));
}

/// Count the files planned across all selected texture sets.
fn selected_texture_count(app: &mut TextureCraftApp) -> usize {
    let mut count = 0;
    for ts in export_selection(app) {
        let params = set_params(app, ts);
        let Ok(plan) = texturecraft_engine::execute(&mut app.session, "export.plan", &params) else { return 0 };
        count += plan.as_array().map_or(0, Vec::len);
    }
    count
}

/// The menu button (≡) of a list or page header centred on `centre`, called `name` for assistive technology; its menu holds `add`
fn list_menu(ui: &mut Ui, centre: egui::Pos2, name: &str, add: impl FnOnce(&mut Ui)) {
    let response = drawn::icon_button(ui, centre, ic::LIST, MENU_ICON_SIZE, name, true, false);
    egui::Popup::menu(&response).show(add);
}

/// Check all, Uncheck all, Invert all over the texture sets
fn set_checks_menu(app: &mut TextureCraftApp, ui: &mut Ui) {
    let sets = app.session.set_count();
    if ui.button(tr("Check all")).clicked() {
        app.ui.export.skip.clear();
    }
    if ui.button(tr("Uncheck all")).clicked() {
        app.ui.export.skip = (0..sets).collect();
    }
    if ui.button(tr("Invert all")).clicked() {
        let skipped = std::mem::take(&mut app.ui.export.skip);
        app.ui.export.skip = (0..sets).filter(|i| !skipped.contains(i)).collect();
    }
    ui.separator();
}

/// A parameter's name at the label column, its middle at `y`
fn row_label(ui: &mut Ui, place: PagePlace, y: f32, text: &str) {
    let look = ExportLook::get(ui.ctx());
    drawn::label_at(ui, place.x(LABEL_LEFT), y, text, look.text, false);
}

/// Output directory at `y`: the path as a field that opens the folder picker, "…" and a reset
#[cfg(not(target_arch = "wasm32"))]
fn output_directory(app: &mut TextureCraftApp, ui: &mut Ui, place: PagePlace, y: f32) {
    row_label(ui, place, y, tr("Output directory"));
    let right = place.right - CONTROLS_RIGHT_MARGIN;
    let dir = app.ui.export.dir.clone();
    let field = drawn::control_rect(place.x(CONTROLS_LEFT), right - BROWSE_WIDTH - BROWSE_GAP, y);
    let path = drawn::field_button(ui, field, ui.scope_id().with("exp_dir"), &dir).on_hover_text(dir.as_str());
    let browse_rect = drawn::control_rect(right - BROWSE_WIDTH, right, y);
    let browse_name = tr("Choose the output folder");
    let browse = drawn::button_with_id(ui, ui.scope_id().with("exp_dir_browse"), browse_rect, "…", browse_name, false, true).on_hover_text(browse_name);
    if path.clicked() || browse.clicked() {
        app.ask_file(FileDialog::folder().directory(dir), Bound::App, |app, paths| {
            if let Some(directory) = paths.first() {
                app.ui.export.dir = directory.display().to_string();
            }
        });
    }
    let reset = drawn::control_rect(right + BROWSE_GAP, right + BROWSE_GAP + BROWSE_WIDTH, y);
    let reset_id = ui.scope_id().with("exp_dir_reset");
    let reset_name = tr("Reset to default");
    if drawn::button_with_id(ui, reset_id, reset, ic::ARROW_COUNTER_CLOCKWISE, reset_name, false, true).on_hover_text(reset_name).clicked() {
        app.ui.export.dir = ExportUi::default().dir;
    }
}

/// The dilation width in `rect`: a dropdown with a slider in its menu, greyed for paddings without a width
fn dilation_button(ui: &mut Ui, rect: egui::Rect, id_salt: &str, padding: &str, dilation: &mut u32) {
    let enabled = has_width(padding);
    let response = drawn::combo_button(ui, rect, ui.scope_id().with(("exp_dilation", id_salt)), &dilation.to_string(), enabled);
    if !enabled {
        response.on_hover_text(tr("This padding mode does not use a dilation width"));
        return;
    }

    // The menu stays open while the slider is used
    let response = response.on_hover_text(tr("Dilation in pixels"));
    egui::Popup::menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.label(tr("Dilation (pixels)"));
        ui.add(egui::Slider::new(dilation, 0..=MAX_DILATION_PIXELS).step_by(1.0));
    });
}

/// A greyed checkbox with its tooltip and the "not available" hint
#[cfg(target_arch = "wasm32")]
fn unavailable_checkbox(ui: &mut Ui, label: &str, tooltip: &str) {
    let mut off = false;
    ui.add_enabled(false, egui::Checkbox::new(&mut off, label)).on_disabled_hover_ui(|ui| {
        ui.label(tooltip);
        ui.label(tr(NOT_AVAILABLE));
    });
}

/// A tick of the global page at `y` with its label after it
fn page_check(ui: &mut Ui, place: PagePlace, y: f32, on: &mut bool, label: &str) -> egui::Response {
    let left = place.x(LABEL_LEFT + CHECK_INDENT);
    let width = CHECK_LABEL_ROOM + drawn::text_width(ui, label, false);
    let hit = egui::Rect::from_min_max(egui::pos2(left, y - CHECK_HIT_SIZE / 2.0), egui::pos2(left + width, y + CHECK_HIT_SIZE / 2.0));
    drawn::check(ui, ui.scope_id().with(("exp_check", label)), hit, egui::pos2(left, y), on, Some(label), label)
}

/// The Global settings page, laid out from `place`
fn global_page(app: &mut TextureCraftApp, ui: &mut Ui, place: PagePlace, builtins: &[ExportPreset]) {
    let look = ExportLook::get(ui.ctx());
    drawn::label_at(ui, place.x(LABEL_LEFT), place.y(GLOBAL_HEADER_Y), tr("General Export Parameters"), look.text, true);
    let reset_at = place.right_inset(PAGE_ICON_FROM_RIGHT, GLOBAL_ICON_Y);
    let reset_tip = tr("Restore the default export parameters");
    if drawn::icon_button(ui, reset_at, ic::ARROW_COUNTER_CLOCKWISE, MENU_ICON_SIZE, reset_tip, true, false).on_hover_text(reset_tip).clicked() {
        let defaults = ExportUi::default();
        let settings = &mut app.ui.export;
        (
            settings.dir,
            settings.format,
            settings.bit_depth,
            settings.dithering,
            settings.size,
            settings.padding,
            settings.dilation,
            settings.normal_format,
            settings.color_space,
            settings.usd_asset,
            settings.materialx,
            settings.shader_parameters,
        ) = (
            defaults.dir,
            defaults.format,
            defaults.bit_depth,
            defaults.dithering,
            defaults.size,
            defaults.padding,
            defaults.dilation,
            defaults.normal_format,
            defaults.color_space,
            defaults.usd_asset,
            defaults.materialx,
            defaults.shader_parameters,
        );
        crate::export_templates::select(app, defaults.preset);
    }

    // One row per parameter at the reference's pitch; the browser has no output directory
    let mut rows = (0..).map(|k| place.y(GLOBAL_FIRST_ROW_Y + k as f32 * GLOBAL_ROW_PITCH));
    let mut next_row = || rows.next().unwrap_or_default();
    let (left, right) = (place.x(CONTROLS_LEFT), place.right - CONTROLS_RIGHT_MARGIN);
    #[cfg(not(target_arch = "wasm32"))]
    output_directory(app, ui, place, next_row());

    let y = next_row();
    row_label(ui, place, y, tr("Output template"));
    let (first, rest) = template_groups(app, builtins);
    let mut picked = app.ui.export.preset.clone();
    if template_combo(&app.session, ui, drawn::control_rect(left, right, y), "exp_preset", &mut picked, &first, &rest) {
        crate::export_templates::select(app, picked);
    }

    // The file type list fits its longest entry, the bit depth list fills the rest
    let y = next_row();
    row_label(ui, place, y, tr("File type"));
    let type_right = left + drawn::text_width(ui, tr("Based on output template"), false) + FILE_TYPE_ROOM;
    let rects = [drawn::control_rect(left, type_right, y), drawn::control_rect(type_right + PAIR_GAP, right, y)];
    let settings = &mut app.ui.export;
    file_type_lists(ui, ("exp_global", 0, 0), true, rects, &mut settings.format, &mut settings.bit_depth, &mut settings.dithering);

    let y = next_row();
    row_label(ui, place, y, tr("Size"));
    drawn::combo(ui, drawn::control_rect(left, right, y), "exp_size", size_label(settings.size), |ui| {
        ui.selectable_value(&mut settings.size, 0, size_label(0));
        ui.separator();
        ui.selectable_value(&mut settings.size, SIZE_OF_LARGEST_SET, size_label(SIZE_OF_LARGEST_SET));
        for size in SIZES {
            ui.selectable_value(&mut settings.size, size, size_label(size));
        }
    });

    let y = next_row();
    row_label(ui, place, y, tr("Padding"));
    let padding_rect = drawn::control_rect(left, right - DILATION_WIDTH - PAIR_GAP, y);
    drawn::combo(ui, padding_rect, "exp_pad", tr(padding_label(&settings.padding)), |ui| {
        for (value, label) in PADDINGS {
            ui.selectable_value(&mut settings.padding, value.to_string(), tr(label));
        }
    });
    dilation_button(ui, drawn::control_rect(right - DILATION_WIDTH, right, y), "global", &settings.padding, &mut settings.dilation);

    let y = next_row();
    row_label(ui, place, y, tr("Normal map format"));
    let normal_label = match settings.normal_format.as_str() {
        "openGl" => "OpenGL",
        "directX" => "DirectX",
        _ => tr("From template / project"),
    };
    drawn::combo(ui, drawn::control_rect(left, right, y), "exp_nf", normal_label, |ui| {
        for (value, label) in [("", tr("From template / project")), ("openGl", "OpenGL"), ("directX", "DirectX")] {
            ui.selectable_value(&mut settings.normal_format, value.to_string(), label);
        }
    });

    let y = next_row();
    row_label(ui, place, y, tr("Color space"));
    let shown = if settings.color_space.is_empty() { tr("Default").to_string() } else { settings.color_space.clone() };
    let color_space = drawn::combo(ui, drawn::control_rect(left, right, y), "exp_cs", shown, |ui| {
        ui.selectable_value(&mut settings.color_space, String::new(), tr("Default"));
        for name in ["sRGB", "Linear Rec.709", "ACEScg", "ACES2065-1"] {
            ui.selectable_value(&mut settings.color_space, name.to_string(), name);
        }
    });
    color_space.response.on_hover_text(tr("Color maps only: normal, roughness, metallic and other data maps stay linear"));

    // The extra files, ticked under the parameters
    let mut checks = (0..).map(|k| y + CHECKS_GAP + k as f32 * CHECK_PITCH);
    let mut next_check = || checks.next().unwrap_or_default();
    let usd_tip = tr("Also write <mesh>.usd next to the textures: the whole mesh, with a material per texture set that reads the exported textures");
    #[cfg(not(target_arch = "wasm32"))]
    page_check(ui, place, next_check(), &mut settings.usd_asset, tr("Export USD asset")).on_hover_text(usd_tip);
    #[cfg(not(target_arch = "wasm32"))]
    page_check(ui, place, next_check(), &mut settings.materialx, tr("Export MaterialX"));
    #[cfg(target_arch = "wasm32")]
    {
        let at = |y: f32| {
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::pos2(place.x(LABEL_LEFT), y - CHECK_HIT_SIZE / 2.0), vec2(right, CHECK_HIT_SIZE)))
        };
        ui.scope_builder(at(next_check()), |ui| unavailable_checkbox(ui, tr("Export USD asset"), usd_tip));
        ui.scope_builder(at(next_check()), |ui| crate::unavailable::checkbox(ui, false, tr("Export MaterialX")));
    }
    page_check(ui, place, next_check(), &mut settings.shader_parameters, tr("Export shader settings"))
        .on_hover_text(tr("Save the settings of each shader instance in a JSON file"));
}

/// The texture set list in `area`: Global settings, then a tick and a page per texture set, "*" on one with overrides
fn set_list(app: &mut TextureCraftApp, ui: &mut Ui, area: egui::Rect) {
    let top = area.top() - TAB_ROW_HEIGHT;
    list_menu(ui, egui::pos2(area.left() + SET_MENU_X, top + SET_MENU_Y), tr("Texture set list menu"), |ui| set_checks_menu(app, ui));
    let rows_right = area.right() - GUTTER;
    let header = egui::Rect::from_min_max(egui::pos2(area.left(), top + SET_ROWS_TOP), egui::pos2(rows_right, top + SET_ROWS_TOP + SET_ROW_HEIGHT));
    if list_row(ui, header, tr("Global settings"), app.ui.export.page.is_none()).clicked() {
        app.ui.export.page = None;
    }

    // Many texture sets scroll under Global settings, the scroll bar in its strip at the list's right
    let rows = egui::Rect::from_min_max(egui::pos2(area.left(), header.bottom()), area.max);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rows), |ui| {
        egui::ScrollArea::vertical().id_salt("export_texture_sets").auto_shrink([false; 2]).show(ui, |ui| set_rows(app, ui, rows.width() - GUTTER));
    });
}

/// A list row across `rect`, its name in bold
fn list_row(ui: &mut Ui, rect: egui::Rect, text: &str, selected: bool) -> egui::Response {
    let look = ExportLook::get(ui.ctx());
    let response = ui.interact(rect, ui.scope_id().with(("exp_list_row", text)), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, text));
    paint_row(ui, rect, selected, response.hovered());
    drawn::text_at(ui, rect.left() + GLOBAL_NAME_LEFT, rect.center().y, text, look.text, true);
    drawn::paint_focus(ui, &response);
    response
}

/// A list row's face: the selected one in the frame colour with a light edge; a hairline under every row
fn paint_row(ui: &Ui, rect: egui::Rect, selected: bool, hovered: bool) {
    let look = ExportLook::get(ui.ctx());
    let painter = ui.painter();
    if selected {
        painter.rect(rect, 0.0, look.chrome, egui::Stroke::new(1.0, look.outline), egui::StrokeKind::Inside);
    } else if hovered {
        painter.rect_filled(rect, 0.0, look.raised);
    }
    painter.rect_filled(egui::Rect::from_min_max(rect.left_bottom(), rect.right_bottom() + vec2(0.0, 1.0)), 0.0, look.chrome);
}

/// A row per texture set, `width` wide: its export tick, its name and the override mark
fn set_rows(app: &mut TextureCraftApp, ui: &mut Ui, width: f32) {
    let look = ExportLook::get(ui.ctx());
    ui.spacing_mut().item_spacing.y = 0.0;
    for i in 0..app.session.set_count() {
        let name = app.session.doc.project.texture_sets.get(i).map(|t| t.name.clone()).unwrap_or_default();
        let (row, _) = ui.allocate_exact_size(vec2(width, SET_ROW_HEIGHT), egui::Sense::hover());
        let y = row.center().y;

        // The tick leaves the set in or out, the name after it opens the set's page
        let name_area = egui::Rect::from_min_max(egui::pos2(row.left() + SET_NAME_LEFT - drawn::CHECK_LABEL_GAP, row.top()), row.max);
        let selected = app.ui.export.page == Some(i);
        paint_row(ui, row, selected, ui.rect_contains_pointer(name_area));
        let mut on = !app.ui.export.skip.contains(&i);
        let tick_area = egui::Rect::from_min_max(row.min, egui::pos2(name_area.left(), row.bottom()));
        let tick_name = trf("Include {set}", &[("set", &name)]);
        if drawn::check(ui, ui.scope_id().with(("exp_set_tick", i)), tick_area, egui::pos2(row.left() + SET_CHECK_LEFT, y), &mut on, None, &tick_name).changed()
        {
            if on {
                app.ui.export.skip.retain(|x| *x != i);
            } else {
                app.ui.export.skip.push(i);
            }
        }

        let page = ui.interact(name_area, ui.scope_id().with(("exp_set_row", i)), egui::Sense::click());
        page.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &name));
        if page.clicked() {
            app.ui.export.page = Some(i);
        }

        let color = if on { look.text } else { look.text_dim };
        drawn::text_at(ui, row.left() + SET_NAME_LEFT, y, &name, color, true);
        drawn::paint_focus(ui, &page);
        if app.ui.export.sets.get(&i).is_some_and(SetExport::any) {
            drawn::label_at(ui, row.right() - OVERRIDE_MARK_FROM_RIGHT, y, "*", look.text_dim, true).on_hover_text(tr("Override(s) active"));
        }
    }
}

/// A texture set page row at `y`: label, pen, then the inherited value or `edit`; true when the pen was clicked
fn override_row(ui: &mut Ui, place: PagePlace, y: f32, label: &str, on: bool, inherited: &str, edit: impl FnOnce(&mut Ui, egui::Rect)) -> bool {
    let look = ExportLook::get(ui.ctx());
    row_label(ui, place, y, label);
    let pen = egui::pos2(place.x(PEN_CENTRE), y);
    let name = trf("Enable override for {setting}", &[("setting", &label)]);
    let toggled = drawn::icon_button(ui, pen, ic::PENCIL_SIMPLE, drawn::CHEVRON_SIZE, &name, true, on).clicked();
    if on {
        edit(ui, drawn::control_rect(place.x(OVERRIDE_LEFT), place.right - CONTROLS_RIGHT_MARGIN, y));
    } else {
        drawn::label_at(ui, place.x(VALUE_LEFT), y, inherited, look.text, false);
    }
    toggled
}

/// The file type texture set `ts` gets from its output template: the one its maps share, or an
/// empty one when they differ (the override keeps the template's file types until one is picked)
fn template_file_type(app: &mut TextureCraftApp, ts: usize) -> FileType {
    let rows = map_rows(app, ts);
    match rows.split_first() {
        Some((first, rest)) if rest.iter().all(|r| r.inherited == first.inherited) => first.inherited.clone(),
        _ => FileType { format: String::new(), bit_depth: 0, dithering: false },
    }
}

/// Switch one override of `o` on (from what the set inherits) or off
fn toggle(o: &mut SetExport, row: usize, from: &InheritedSettings) {
    match row {
        0 => o.preset = if o.preset.is_some() { None } else { Some(from.preset.clone()) },
        1 => o.file_type = if o.file_type.is_some() { None } else { Some(from.file_type.clone()) },
        2 => o.size = if o.size.is_some() { None } else { Some(from.size) },
        _ => o.padding = if o.padding.is_some() { None } else { Some(from.padding.clone()) },
    }
}

/// A texture set's page from `place`: its overrides, then its output maps scrolling down to `bottom`
fn set_page(app: &mut TextureCraftApp, ui: &mut Ui, ts: usize, place: PagePlace, bottom: f32, builtins: &[ExportPreset]) {
    let Some(set) = app.session.doc.project.texture_sets.get(ts) else { return };
    let native = set.block_size();
    let mut from = InheritedSettings::for_texture_set(app, native.0);

    let (first, rest) = template_groups(app, builtins);
    let first: Vec<(&str, bool)> = first.into_iter().filter(|(name, _)| !NOT_PER_SET.contains(name)).collect();
    let global = &app.ui.export;
    let inherited_file_type = if global.format.is_empty() { tr("Based on output template").to_string() } else { file_type_text(&from.file_type) };
    let inherited_size = set_size_label(scaled_block_size(from.size, native), native);
    let inherited_padding = tr(padding_label(&global.padding)).to_string();

    let look = ExportLook::get(ui.ctx());
    let overrides = app.ui.export.sets.entry(ts).or_default();
    let mut all = None;
    drawn::label_at(ui, place.x(LABEL_LEFT), place.y(SET_HEADER_Y), tr("General Export Parameters"), look.text, true);
    list_menu(ui, place.right_inset(PAGE_ICON_FROM_RIGHT, SET_HEADER_Y), tr("General Export Parameters menu"), |ui| {
        if ui.button(tr("Override every item")).clicked() {
            all = Some(true);
        }
        if ui.button(tr("Clear all overrides")).clicked() {
            all = Some(false);
        }
        ui.separator();
    });

    let row_y = |row: usize| place.y(SET_FIRST_ROW_Y + row as f32 * SET_ROW_PITCH);
    let mut toggled = None;
    if override_row(ui, place, row_y(0), tr("Output template"), overrides.preset.is_some(), template_label(&app.session, &from.preset), |ui, rect| {
        if let Some(preset) = &mut overrides.preset {
            template_combo(&app.session, ui, rect, "exp_set_preset", preset, &first, &rest);
        }
    }) {
        toggled = Some(0);
    }
    if override_row(ui, place, row_y(1), tr("File type"), overrides.file_type.is_some(), &inherited_file_type, |ui, rect| {
        if let Some(file_type) = &mut overrides.file_type {
            let type_rect = rect.with_max_x(rect.left() + MAP_FORMAT_WIDTH);
            let rects = [type_rect, rect.with_min_x(type_rect.right() + PAIR_GAP)];
            file_type_lists(ui, ("exp_set", ts, usize::MAX), false, rects, &mut file_type.format, &mut file_type.bit_depth, &mut file_type.dithering);
        }
    }) {
        toggled = Some(1);
    }
    if override_row(ui, place, row_y(2), tr("Size"), overrides.size.is_some(), &inherited_size, |ui, rect| {
        if let Some(size) = &mut overrides.size {
            drawn::combo(ui, rect, "exp_set_size", set_size_label(scaled_block_size(*size, native), native), |ui| {
                for side in SIZES {
                    ui.selectable_value(size, side, set_size_label(scaled_block_size(side, native), native));
                }
            });
        }
    }) {
        toggled = Some(2);
    }
    if override_row(ui, place, row_y(3), tr("Padding"), overrides.padding.is_some(), &inherited_padding, |ui, rect| {
        if let Some((padding, dilation)) = &mut overrides.padding {
            drawn::combo(ui, rect.with_max_x(rect.right() - DILATION_WIDTH - PAIR_GAP), "exp_set_pad", tr(padding_label(padding)), |ui| {
                for (value, label) in PADDINGS {
                    ui.selectable_value(padding, value.to_string(), tr(label));
                }
            });
            dilation_button(ui, rect.with_min_x(rect.right() - DILATION_WIDTH), "set", padding, dilation);
        }
    }) {
        toggled = Some(3);
    }
    if toggled.is_some() || all.is_some() {
        // A new file-type override inherits the effective formats of the current template
        if (toggled == Some(1) || all == Some(true)) && overrides.file_type.is_none() && app.ui.export.format.is_empty() {
            from.file_type = template_file_type(app, ts);
        }

        let overrides = app.ui.export.sets.entry(ts).or_default();
        if let Some(row) = toggled {
            toggle(overrides, row, &from);
        }
        match all {
            Some(true) => {
                for row in 0..4 {
                    let on = [overrides.preset.is_some(), overrides.file_type.is_some(), overrides.size.is_some(), overrides.padding.is_some()];
                    if !on.get(row).copied().unwrap_or(true) {
                        toggle(overrides, row, &from);
                    }
                }
            }
            Some(false) => {
                overrides.preset = None;
                overrides.file_type = None;
                overrides.size = None;
                overrides.padding = None;
            }
            None => {}
        }
    }

    // A divider across the page, then the output maps
    let page_left = place.x(SET_LIST_WIDTH + 1.0);
    drawn::divider(ui, egui::Rect::from_min_max(egui::pos2(page_left, place.y(SET_DIVIDER_Y)), egui::pos2(place.right, place.y(SET_DIVIDER_Y) + 1.0)));
    let maps_area = egui::Rect::from_min_max(egui::pos2(page_left, place.y(MAPS_TOP)), egui::pos2(place.right, bottom));
    output_maps(app, ui, ts, place, maps_area);
    if app.ui.export.sets.get(&ts).is_some_and(SetExport::is_default) {
        app.ui.export.sets.remove(&ts);
    }
}

/// Green of the console's success lines
fn success(tk: &Tokens) -> egui::Color32 {
    if tk.dark { egui::Color32::from_rgb(110, 200, 120) } else { egui::Color32::from_rgb(30, 130, 50) }
}

/// The LIST OF EXPORTS tab in `body`: the file tree with each file's path as tooltip, the fold strip and the console
fn list_tab(app: &mut TextureCraftApp, ui: &mut Ui, body: egui::Rect) {
    let look = ExportLook::get(ui.ctx());
    let tk = Tokens::get(ui.ctx());
    let hidden = app.ui.export.console_hidden;

    // The console keeps at least its least width, the tree at least its own
    let room = body.width() - STRIP_WIDTH - STRIP_GAP - 1.0;
    let console_width = app.ui.export.console_width.min(room - MIN_TREE_WIDTH).max(CONSOLE_WIDTH);
    let console_left = if hidden { body.right() + 1.0 } else { body.right() - console_width };
    let strip = egui::Rect::from_min_max(egui::pos2(console_left - 1.0 - STRIP_WIDTH, body.top() + 1.0), egui::pos2(console_left - 1.0, body.bottom()));
    let tree = egui::Rect::from_min_max(egui::pos2(body.left(), body.top() + 1.0), egui::pos2(strip.left() - STRIP_GAP, body.bottom()));
    ui.painter().rect_filled(tree, 0.0, look.chrome);
    ui.scope_builder(egui::UiBuilder::new().max_rect(tree), |ui| {
        egui::ScrollArea::both().id_salt("exp_list").auto_shrink([false; 2]).show(ui, |ui| export_tree(app, ui, &tk));
    });

    // The strip at the console's left edge: a click hides or shows the console, a drag resizes it
    drawn::divider(ui, egui::Rect::from_min_max(strip.right_top(), egui::pos2(strip.right() + 1.0, strip.bottom())));
    let response = ui.interact(strip, ui.scope_id().with("exp_console_strip"), egui::Sense::click_and_drag());
    let tooltip = if hidden { tr("Show Console") } else { tr("Hide Console") };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tooltip));
    let glyph = if hidden { ic::CARET_DOUBLE_LEFT } else { ic::CARET_DOUBLE_RIGHT };
    let glyph_color = if response.hovered() { look.text } else { look.text_dim };
    drawn::glyph(ui, strip.center(), glyph, STRIP_GLYPH_SIZE, glyph_color);
    drawn::paint_focus(ui, &response);
    let response = response.on_hover_text(tooltip);
    if !hidden && (response.hovered() || response.dragged()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if response.dragged() && !hidden {
        app.ui.export.console_width = (console_width - response.drag_delta().x).min(room - MIN_TREE_WIDTH).max(CONSOLE_WIDTH);
    } else if response.clicked() {
        app.ui.export.console_hidden = !hidden;
    }
    if hidden {
        return;
    }

    // The console: its header on the window colour, its lines on the frame colour below
    let top = body.top() - TAB_ROW_HEIGHT;
    let console = egui::Rect::from_min_max(egui::pos2(console_left, body.top() + 1.0), body.max);
    drawn::label_at(ui, console.left() + CONSOLE_TEXT_INDENT, top + CONSOLE_HEADER_Y, tr("Console"), look.text, true);
    let lines = egui::Rect::from_min_max(egui::pos2(console.left(), top + CONSOLE_BODY_TOP), console.max);
    ui.painter().rect_filled(lines, 0.0, look.chrome);
    let inner = lines.shrink2(vec2(0.0, CONSOLE_PADDING)).with_min_x(lines.left() + CONSOLE_TEXT_INDENT);
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        let console = egui::ScrollArea::vertical().id_salt("exp_console").auto_shrink([false; 2]).stick_to_bottom(true);
        console.show(ui, |ui| {
            for (index, line) in app.ui.export.console.iter().enumerate() {
                let response = match line {
                    ConsoleLine::Step(text) => ui.label(text),
                    ConsoleLine::Done(text) => ui.label(RichText::new(text).color(success(&tk))),
                    ConsoleLine::Set(name) => ui.label(set_line(ui, name, &tk)),
                };

                if index + 1 == app.ui.export.console.len() {
                    ui.ctx().accesskit_node_builder(response.id, |node| node.set_live(egui::accesskit::Live::Polite));
                }
            }
        });
    });
}

/// "Texture set <name>" with the name in bold
fn set_line(ui: &Ui, name: &str, tk: &Tokens) -> egui::text::LayoutJob {
    let text = tr("Texture set {set}");
    let (before, after) = text.split_once("{set}").unwrap_or((text, ""));
    let font = egui::TextStyle::Body.resolve(ui.style());
    let plain = egui::TextFormat::simple(font.clone(), tk.text);
    let mut job = egui::text::LayoutJob::default();
    job.append(before, 0.0, plain.clone());
    job.append(name, 0.0, egui::TextFormat::simple(semibold(font.size), tk.text_strong));
    job.append(after, 0.0, plain);
    job
}

/// One foldable node per ticked texture set, a row per file; the columns start after the longest file name
fn export_tree(app: &mut TextureCraftApp, ui: &mut Ui, tk: &Tokens) {
    let (sets, directory): (Vec<(usize, ExportSetPlan)>, String) = match &app.ui.export.job {
        Some(job) if !job.preview.is_empty() => (job.preview.iter().cloned().enumerate().collect(), job.directory.clone()),
        _ => {
            let planned = export_selection(app).into_iter().filter_map(|ts| {
                let params = set_params(app, ts);
                plan_set(app, ts, &params).map(|set| (ts, set))
            });
            (planned.collect(), app.ui.export.dir.clone())
        }
    };
    let rows: Vec<Vec<TreeFile>> = sets.iter().map(|(_, set)| tree_files(set, &directory)).collect();
    let longest = |text: fn(&TreeFile) -> &str| rows.iter().flatten().map(|file| drawn::text_width(ui, text(file), false)).fold(0.0, f32::max);
    let columns = {
        let size = TREE_FILE_INDENT + longest(|f| &f.name) + TREE_COLUMN_GAP;
        let format = size + longest(|f| &f.size) + TREE_COLUMN_GAP;
        [size, format, format + longest(|f| &f.format) + TREE_BITS_GAP]
    };

    // Each set takes its header row, its files when open and a blank row
    let open: Vec<bool> = sets.iter().map(|(index, _)| ui.data(|d| d.get_temp::<bool>(tree_id(ui, *index))).unwrap_or(true)).collect();
    let row_count: usize = sets
        .iter()
        .zip(&rows)
        .zip(&open)
        .map(|(((_, set), files), open)| 1 + 1 + if *open { files.len().max(usize::from(set.files.is_err())) } else { 0 })
        .sum();
    let width = ui.available_width().max(columns[2] + longest(|file| &file.bits) + TREE_BITS_GAP);
    let height = TREE_TOP_PADDING + row_count as f32 * TREE_ROW_HEIGHT;
    let (area, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());

    let mut y = area.top() + TREE_TOP_PADDING;
    for (((index, set), files), open) in sets.iter().zip(&rows).zip(open) {
        let header = egui::Rect::from_min_max(egui::pos2(area.left(), y), egui::pos2(area.right(), y + TREE_ROW_HEIGHT));
        if tree_header(ui, header, &set.name, open, tree_id(ui, *index)) {
            ui.data_mut(|d| d.insert_temp(tree_id(ui, *index), !open));
        }
        y += TREE_ROW_HEIGHT;
        if open {
            if let Err(error) = &set.files {
                drawn::label_at(ui, area.left() + TREE_FILE_INDENT, y + TREE_ROW_HEIGHT / 2.0, error, tk.danger, false);
                y += TREE_ROW_HEIGHT;
            }
            for file in files {
                tree_file(ui, area.left(), y + TREE_ROW_HEIGHT / 2.0, columns, file);
                y += TREE_ROW_HEIGHT;
            }
        }
        y += TREE_ROW_HEIGHT;
    }
}

fn plan_set(app: &mut TextureCraftApp, index: usize, params: &serde_json::Value) -> Option<ExportSetPlan> {
    let set = app.session.doc.project.texture_sets.get(index)?;
    let name = set.name.clone();
    let files = texturecraft_engine::execute(&mut app.session, "export.plan", params).map_err(|error| error.to_string());

    Some(ExportSetPlan { index: Some(index), name, files })
}

/// Where a texture set's node keeps whether it is open
fn tree_id(ui: &Ui, index: usize) -> egui::Id {
    ui.scope_id().with(("exp_tree", index))
}

/// The texts of each file `set` plans: name, size, file type, bit depth and its whole path
fn tree_files(set: &ExportSetPlan, directory: &str) -> Vec<TreeFile> {
    let Ok(files) = &set.files else { return Vec::new() };

    let to_text = |output: &serde_json::Value| {
        let file = output.get("file").and_then(serde_json::Value::as_str).unwrap_or_default();
        let format = FileFormat::from_extension(output.get("format").and_then(serde_json::Value::as_str).unwrap_or_default());
        let depth = output.get("bitDepth").and_then(serde_json::Value::as_u64).unwrap_or(8) as u8;
        let dithering = output.get("dithering").and_then(serde_json::Value::as_bool).unwrap_or(false);
        let dimension = |key: &str| output[key].as_u64().unwrap_or_default() as u32;
        let size = (dimension("size"), dimension("height"));
        let source_size = (dimension("sourceSize"), dimension("sourceHeight"));

        TreeFile {
            name: file.to_string(),
            size: set_size_label(size, source_size),
            format: format.map_or("", FileFormat::name).to_string(),
            bits: depth_label(format, depth, dithering),
            path: std::path::Path::new(directory).join(file).display().to_string(),
        }
    };
    files.as_array().into_iter().flatten().map(to_text).collect()
}

/// A texture set's node row: the chevron and its name in bold; true when clicked
fn tree_header(ui: &mut Ui, rect: egui::Rect, name: &str, open: bool, id: egui::Id) -> bool {
    let look = ExportLook::get(ui.ctx());
    let response = ui.interact(rect, id, egui::Sense::click());

    // Expose the hand-drawn header as a disclosure control
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::DisclosureTriangle, true, name));

    let y = rect.center().y;
    let chevron = if open {
        ic::CARET_DOWN
    } else if ui.style().right_to_left {
        ic::CARET_LEFT
    } else {
        ic::CARET_RIGHT
    };
    drawn::glyph(ui, egui::pos2(rect.left() + TREE_CHEVRON_X, y), chevron, drawn::CHEVRON_SIZE, look.text);
    drawn::text_at(ui, rect.left() + TREE_NAME_INDENT, y, name, look.text, true);
    drawn::paint_focus(ui, &response);

    if response.has_focus() && rect.width() > ui.clip_rect().width() {
        let direction = ui.input_mut(|input| {
            let start = input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft);
            let end = input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight);
            i8::from(start) - i8::from(end)
        });
        if direction != 0 {
            ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            ui.scroll_with_delta(vec2(f32::from(direction) * ui.clip_rect().width() / 2.0, 0.0));
        }
    }

    response.clicked()
}

/// A file's row at `y`: its name, then size, file type and bit depth in `columns`, each with the whole path as tooltip
fn tree_file(ui: &mut Ui, left: f32, y: f32, columns: [f32; 3], file: &TreeFile) {
    let look = ExportLook::get(ui.ctx());
    let texts = [(TREE_FILE_INDENT, &file.name), (columns[0], &file.size), (columns[1], &file.format), (columns[2], &file.bits)];
    for (x, text) in texts {
        drawn::label_at(ui, left + x, y, text, look.text, false).on_hover_text(&file.path);
    }
}

/// Show the output folder in the system's file manager (created first when missing)
#[cfg(not(target_arch = "wasm32"))]
fn open_output_directory(app: &mut TextureCraftApp) {
    let settings = &app.ui.export;
    let directory = settings.job.as_ref().filter(|job| !job.preview.is_empty()).map_or(&settings.dir, |job| &job.directory);
    let dir = std::path::PathBuf::from(directory);
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|()| app.file_dialogs.show_folder(&dir)) {
        app.toast(trf("Could not open {folder}: {error}", &[("folder", &dir.display()), ("error", &e)]), true);
    }
}

/// Keep the texture set ticks and overrides with their texture sets: dropped for another project,
/// moved with a texture set by name, dropped with a removed one. A running export follows its
/// texture sets the same way; another project stops it.
fn follow_texture_sets(app: &mut TextureCraftApp) {
    let project = &app.session.doc.project;
    let owner_names = || std::iter::once(project.name.as_str()).chain(project.texture_sets.iter().map(|set| set.name.as_str()));
    let id = app.session.project_id;
    let e = &mut app.ui.export;
    if e.owner_id == Some(id) && e.owner.iter().map(String::as_str).eq(owner_names()) {
        return;
    }

    let owner: Vec<String> = owner_names().map(str::to_owned).collect();

    // Legacy settings adopt the first project without discarding saved exclusions
    if e.owner.is_empty() && e.opened_with.is_none() {
        e.owner = owner;
        e.owner_id = Some(id);
        return;
    }

    let same_project = match e.owner_id {
        Some(was) => was == id,
        None => e.owner.first() == owner.first(),
    };
    let names = owner.get(1..).unwrap_or_default();
    let old = e.owner.get(1..).unwrap_or_default();
    let new_indices: Vec<Option<usize>> = old
        .iter()
        .enumerate()
        .map(|(index, name)| {
            if !same_project {
                return None;
            }

            // When names repeat, a name alone cannot identify a moved set
            if old.iter().filter(|other| *other == name).nth(1).is_some() {
                return (names.get(index) == Some(name)).then_some(index);
            }

            let mut matches = names.iter().enumerate().filter(|(_, other)| *other == name);
            let (new_index, _) = matches.next()?;
            if matches.next().is_none() { Some(new_index) } else { (names.get(index) == Some(name)).then_some(index) }
        })
        .collect();
    let moved = |index: usize| new_indices.get(index).copied().flatten();
    let follow = |e: &mut ExportUi| {
        e.sets = std::mem::take(&mut e.sets).into_iter().filter_map(|(i, o)| moved(i).map(|j| (j, o))).collect();
        e.skip = e.skip.iter().filter_map(|i| moved(*i)).collect();
        e.page = e.page.and_then(moved);
    };
    follow(e);
    if let Some(before) = e.opened_with.as_deref_mut() {
        follow(before);
        (before.owner, before.owner_id) = (owner.clone(), Some(id));
    }
    if let Some(job) = e.job.as_mut() {
        if same_project {
            let queued = job.pending.len();
            job.pending = std::mem::take(&mut job.pending).into_iter().filter_map(|(i, p)| moved(i).map(|j| (j, p))).collect();
            for set in &mut job.preview {
                set.index = set.index.and_then(moved);
            }

            // The sets already written follow too (the USD asset reads their params); a removed one leaves
            job.sent = std::mem::take(&mut job.sent)
                .into_iter()
                .filter_map(|mut params| {
                    let Some(index) = params.get("ts").and_then(serde_json::Value::as_u64) else { return Some(params) };
                    let index = moved(usize::try_from(index).ok()?)?;
                    params["ts"] = json!(index);
                    Some(params)
                })
                .collect();

            // Removed sets leave the count too, so the bar keeps its pace
            job.total -= queued - job.pending.len();
        } else {
            e.job = None;
            e.console.push(ConsoleLine::Step(tr("Export stopped: another project was opened").into()));
        }
    }
    e.owner = owner;
    e.owner_id = Some(id);
}

/// The bit depth the window shows is the one exported: "Based on output template" has none (older
/// settings could keep one there), and a file type gets one of its own depths.
fn keep_file_type_visible(e: &mut ExportUi) {
    match FileFormat::from_extension(&e.format) {
        None => (e.format, e.bit_depth, e.dithering) = (String::new(), 0, false),
        Some(f) if !depth_options(f).contains(&(e.bit_depth, e.dithering)) => (e.bit_depth, e.dithering) = (f.default_depth(), false),
        Some(_) => {}
    }
}

/// Template `old` was renamed to `new` (None: removed): texture set overrides that use it follow,
/// in the settings the window opened with too, so Cancel restores a template that exists
pub(crate) fn template_renamed(e: &mut ExportUi, old: &str, new: Option<&str>) {
    let follow = |e: &mut ExportUi| {
        if e.preset == old
            && let Some(n) = new
        {
            e.preset = n.to_string();
        }
        for o in e.sets.values_mut() {
            if o.preset.as_deref() == Some(old) {
                o.preset = new.map(str::to_string);
            }
            if o.maps_template == old {
                match new {
                    Some(n) => o.maps_template = n.to_string(),
                    None => {
                        o.maps.clear();
                        o.maps_template.clear();
                    }
                }
            }
        }
        e.sets.retain(|_, o| !o.is_default());
    };
    follow(e);
    if let Some(before) = e.opened_with.as_deref_mut() {
        follow(before);
    }
}

/// The maps of template `template` were saved in a new order: `origin` gives each map's index
/// before. The texture set overrides of its maps follow them (removed maps lose theirs), in the
/// settings the window opened with too.
pub(crate) fn template_maps_moved(e: &mut ExportUi, template: &str, origin: &[Option<usize>]) {
    let follow = |e: &mut ExportUi| {
        for o in e.sets.values_mut().filter(|o| o.maps_template == template) {
            o.maps = std::mem::take(&mut o.maps).into_iter().filter_map(|(i, m)| origin.iter().position(|x| *x == Some(i)).map(|j| (j, m))).collect();
        }
        e.sets.retain(|_, o| !o.is_default());
    };
    follow(e);
    if let Some(before) = e.opened_with.as_deref_mut() {
        follow(before);
    }
}

/// Go back to the settings the window opened with
fn cancel(app: &mut TextureCraftApp) {
    if let Some(before) = app.ui.export.opened_with.take() {
        let mut before = *before;
        before.draft = None;
        before.renaming = None;
        before.ask = None;
        before.pending_drop = None;

        // A template deleted since keeps the default instead
        if app.session.export_preset(&before.preset).is_none() {
            before.preset = ExportUi::default().preset;
        }
        for o in before.sets.values_mut() {
            if o.preset.as_deref().is_some_and(|p| app.session.export_preset(p).is_none()) {
                o.preset = None;
            }
        }
        app.ui.export = before;
    }
    app.ui.windows.export = false;
}

/// Escape closes the window as Cancel does after shortcut routing checks higher-priority inputs.
pub(crate) fn keys_in(app: &mut TextureCraftApp, input: &mut egui::InputState) {
    if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
        app.ui.export.job = None;
        cancel(app);
    }
}

/// The bottom bar: Export, Save settings, Cancel and the texture count, Reset all on SETTINGS; while exporting, progress
fn footer(app: &mut TextureCraftApp, ui: &mut Ui, window: egui::Rect, y: f32, do_export: &mut bool, canceled: &mut bool) {
    let slot = |k: f32| {
        let right = window.right() - RIGHT_MARGIN - k * (FOOTER_BUTTON_WIDTH + FOOTER_BUTTON_GAP);
        drawn::control_rect(right - FOOTER_BUTTON_WIDTH, right, y)
    };
    if let Some(job) = &app.ui.export.job {
        let done = job.total - job.pending.len();
        let progress = if job.total == 0 { 1.0 } else { done as f32 / job.total as f32 };
        if drawn::button(ui, slot(0.0), tr("Cancel Export"), false, true).clicked() {
            app.ui.export.job = None;
        }
        let bar = drawn::control_rect(window.left() + LEFT_MARGIN, slot(0.0).left() - FOOTER_BUTTON_GAP, y);
        let look = ExportLook::get(ui.ctx());
        let response = ui.put(bar, egui::ProgressBar::new(progress).fill(look.outline).text(format!("{:.0}%", progress * 100.0)));
        ui.ctx().accesskit_node_builder(response.id, |node| node.set_live(egui::accesskit::Live::Polite));
        return;
    }

    // OUTPUT TEMPLATES shows no count; a template edit that does not validate holds Save and Export
    let templates_tab = app.ui.export.tab == 1;
    let draft_valid = app.ui.export.draft.as_ref().is_none_or(|draft| draft.validate().is_ok());
    let any = !export_selection(app).is_empty() && draft_valid;
    let export = drawn::button(ui, slot(0.0), tr("Export"), true, any);
    if export.on_hover_text(tr("Write the selected textures, and the extra files ticked in the settings")).clicked() {
        *do_export = true;
    }

    // Saving closes the window without starting an export
    let save = drawn::button(ui, slot(1.0), tr("Save settings"), false, draft_valid);
    if save.on_hover_text(tr("Keep these settings for the next export and close the window")).clicked() {
        app.ui.windows.export = false;
    }
    let cancel = drawn::button(ui, slot(2.0), tr("Cancel"), false, true);
    if cancel.on_hover_text(tr("Close the window and go back to the settings it opened with")).clicked() {
        *canceled = true;
    }
    if !templates_tab {
        let look = ExportLook::get(ui.ctx());
        let count = trf("{n} textures selected for export", &[("n", &selected_texture_count(app))]);
        let left = slot(2.0).left() - COUNT_GAP - drawn::text_width(ui, &count, false);
        drawn::label_at(ui, left, y, &count, look.text_dim, false);
    }

    let reset_rect = drawn::control_rect(window.left() + LEFT_MARGIN, window.left() + LEFT_MARGIN + FOOTER_BUTTON_WIDTH, y);
    if app.ui.export.tab == 0
        && drawn::button(ui, reset_rect, tr("Reset all"), false, true).on_hover_text(tr("Restore the export defaults and select every texture")).clicked()
    {
        // Keep ownership so Cancel can restore the opening settings
        let e = &mut app.ui.export;
        let (opened_with, owner, owner_id) = (e.opened_with.take(), std::mem::take(&mut e.owner), e.owner_id);
        app.ui.export = ExportUi { opened_with, owner, owner_id, ..ExportUi::default() };
    }
}

/// The SETTINGS tab in `body`: the texture set list, a divider, then the selected page
fn settings_tab(app: &mut TextureCraftApp, ui: &mut Ui, body: egui::Rect, presets: &[ExportPreset]) {
    let list = body.with_max_x(body.left() + SET_LIST_WIDTH);
    set_list(app, ui, list);
    drawn::divider(ui, egui::Rect::from_min_max(list.right_top(), list.right_bottom() + vec2(1.0, 0.0)));
    let page = body.with_min_x(list.right() + 1.0);
    match app.ui.export.page {
        Some(ts) if ts < app.session.set_count() => {
            ui.scope_builder(egui::UiBuilder::new().max_rect(page), |ui| {
                egui::ScrollArea::both().id_salt("export_texture_set_page").auto_shrink([false; 2]).show(ui, |ui| {
                    let size = vec2(ui.available_width().max(MIN_PAGE_WIDTH), ui.available_height().max(MIN_SET_PAGE_HEIGHT));
                    let (content, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                    let place = PagePlace { left: content.left() - list.width() - 1.0, top: content.top() - TAB_ROW_HEIGHT, right: content.right() };
                    set_page(app, ui, ts, place, content.bottom(), presets);
                });
            });
        }

        // A short or narrow window scrolls the page, so every control stays reachable
        _ => {
            ui.scope_builder(egui::UiBuilder::new().max_rect(page), |ui| {
                egui::ScrollArea::both().id_salt("export_settings_page").auto_shrink([false; 2]).show(ui, |ui| {
                    let size = vec2(ui.available_width().max(MIN_PAGE_WIDTH), GLOBAL_PAGE_HEIGHT);
                    let (content, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                    let place = PagePlace { left: content.left() - list.width() - 1.0, top: content.top() - TAB_ROW_HEIGHT, right: content.right() };
                    global_page(app, ui, place, presets);
                });
            });
        }
    }
}

pub fn show(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let showing_templates = app.ui.export.tab == 1;
    if !app.ui.windows.export {
        crate::export_templates::clear_map_previews(ctx);

        // The window opens on SETTINGS again; an export stops with it
        let e = &mut app.ui.export;
        e.job = None;
        e.ask = None;
        e.pending_drop = None;
        e.opened_with = None;
        e.tab = 0;
        e.page = None;
        return;
    }

    // Settings saved under the default template's old name
    if !app.session.has_user_export_preset(&app.ui.export.preset)
        && let Some(b) = texturecraft_engine::formats::builtin_preset(&app.ui.export.preset)
        && b.name != app.ui.export.preset
    {
        app.ui.export.preset = b.name;
    }
    keep_file_type_visible(&mut app.ui.export);
    follow_texture_sets(app);

    // A map waiting for its source channel belongs to the OUTPUT TEMPLATES tab
    if app.ui.export.tab != 1 {
        app.ui.export.pending_drop = None;
    }
    if app.ui.export.opened_with.is_none() {
        let mut now = app.ui.export.clone();
        now.draft = None;
        app.ui.export.opened_with = Some(Box::new(now));
    }

    let mut open = true;
    let mut do_export = false;
    let mut canceled = false;
    let presets = texturecraft_engine::formats::builtin_preset_catalog();
    let look = ExportLook::get(ctx);

    // The reference size includes the frame and title bar
    let style = ctx.global_style();
    let title_frame = egui::Frame::window(&style);
    let frame = title_frame.inner_margin(CONTENT_INSET);
    let title_height = ctx.fonts_mut(|fonts| fonts.row_height(&egui::TextStyle::Heading.resolve(&style))) + style.spacing.item_spacing.y;
    let chrome = vec2(frame.total_margin().sum().x, title_frame.total_margin().sum().y + title_height);
    let window_size =
        (REFERENCE_WINDOW_SIZE_POINTS + chrome).min(ctx.content_rect().size() - vec2(WINDOW_VIEWPORT_MARGIN_POINTS, WINDOW_VIEWPORT_MARGIN_POINTS));
    let dialog =
        crate::windows::dialog(ctx, "export_textures", tr("Export textures"), &mut open).title_frame(title_frame).frame(frame).default_size(window_size);
    dialog.show(ctx, |ui| {
        // Nothing to export without a project
        if !app.session.project_open {
            ui.label(tr("No project is open"));
            return;
        }

        look.apply(ui.style_mut());
        let window = ui.max_rect();
        ui.allocate_rect(window, egui::Sense::hover());
        let body =
            egui::Rect::from_min_max(egui::pos2(window.left(), window.top() + TAB_ROW_HEIGHT), egui::pos2(window.right(), window.bottom() - FOOTER_HEIGHT));

        // The tabs, and the tab's own control at the row's right end
        let tab_strip = egui::Rect::from_min_max(egui::pos2(window.left(), window.top() + TAB_TOP), egui::pos2(window.right(), body.top()));
        let labels = [tr("Settings"), tr("Output templates"), tr("List of exports")];
        if let Some(tab) = drawn::tabs(ui, tab_strip, window.left() + TABS_LEFT, &labels, usize::from(app.ui.export.tab)) {
            app.ui.export.tab = tab as u8;
        }
        let right = window.right() - RIGHT_MARGIN;
        let row_y = window.top() + TAB_ROW_CONTROLS_Y;
        match app.ui.export.tab {
            1 => crate::export_templates::naming_pattern(ui, window.right(), row_y),
            2 => {
                let width = drawn::text_width(ui, tr("Open output directory"), false) + OPEN_DIRECTORY_PADDING;
                let rect = drawn::control_rect(window.right() - OPEN_DIRECTORY_FROM_RIGHT - width, window.right() - OPEN_DIRECTORY_FROM_RIGHT, row_y);

                // The browser downloads the files instead
                #[cfg(target_arch = "wasm32")]
                drawn::button(ui, rect, tr("Open output directory"), false, false).on_hover_text(tr(NOT_AVAILABLE));
                #[cfg(not(target_arch = "wasm32"))]
                if drawn::button(ui, rect, tr("Open output directory"), false, true).clicked() {
                    open_output_directory(app);
                }
            }
            _ => {
                let rect = drawn::control_rect(right - FOOTER_BUTTON_WIDTH, right, row_y);
                if drawn::button(ui, rect, tr("Include all"), false, true).on_hover_text(tr("Select every texture set, UV tile and output map")).clicked() {
                    app.ui.export.skip.clear();
                    for o in app.ui.export.sets.values_mut() {
                        o.maps.values_mut().for_each(|m| m.enabled = true);
                    }
                    app.ui.export.sets.retain(|_, o| !o.is_default());
                }
            }
        }

        let footer_y = window.bottom() - FOOTER_MIDDLE_FROM_BOTTOM;
        drawn::divider(ui, egui::Rect::from_min_max(body.left_bottom(), body.right_bottom() + vec2(0.0, 1.0)));
        footer(app, ui, window, footer_y, &mut do_export, &mut canceled);

        // The tabs lay out from the body; a child clips what an overflow would draw past it
        let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body));
        body_ui.shrink_clip_rect(body);
        let ui = &mut body_ui;
        match app.ui.export.tab {
            1 => crate::export_templates::tab(app, ui, body, presets),
            2 => list_tab(app, ui, body),
            _ => settings_tab(app, ui, body, presets),
        }
    });
    crate::export_templates::ask(app, ctx);

    // Keep text focus before egui clears it on Escape
    app.ui.export.field_had_keyboard = ctx.text_edit_focused();

    if (showing_templates || app.ui.export.tab == 1) && (canceled || !open || !app.ui.windows.export || app.ui.export.tab != 1 || do_export) {
        crate::export_templates::clear_map_previews(ctx);
    }

    // Dismissal must run before the next write
    if canceled || !open {
        app.ui.export.job = None;
        cancel(app);
        return;
    }

    // Write the next set after the window handles cancellation
    if app.ui.export.job.is_some() && !do_export {
        export_step(app, ctx.cumulative_frame_nr());
        ctx.request_repaint();
    }
    if do_export {
        start_export(app);
        if let Some(job) = app.ui.export.job.as_mut() {
            job.frame = ctx.cumulative_frame_nr();
        }
        ctx.request_repaint();
    }
}
