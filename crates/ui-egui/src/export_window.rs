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
use crate::theme::{Tokens, caption};
use crate::unavailable::NOT_AVAILABLE;
use crate::widgets::{icon_button, primary_button};

mod inherited_settings;
mod map_row;
use inherited_settings::InheritedSettings;
use map_row::MapRow;

/// Height of the window's tab body, above its buttons
const BODY_HEIGHT: f32 = 440.0;
/// Width of the texture set list
const LIST_WIDTH: f32 = 180.0;
const ICON: f32 = 20.0;
/// Width of the value column of the parameter rows
const VALUE_WIDTH: f32 = 360.0;

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
fn depth_options(format: FileFormat) -> Vec<(u8, bool)> {
    format.bit_depths().iter().flat_map(|&d| if d == 8 && !format.holds_float(8) { vec![(8, false), (8, true)] } else { vec![(d, false)] }).collect()
}

/// A bit depth as listed: "8 bits", "8 bits + dithering", "16 bits", "16f bits", "32f bits"
fn depth_label(format: Option<FileFormat>, bit_depth: u8, dithering: bool) -> String {
    match (bit_depth, dithering) {
        (8, true) => tr("8 bits + dithering").to_string(),
        (16, _) if format == Some(FileFormat::Exr) => tr("16f bits").to_string(),
        (32, _) => tr("32f bits").to_string(),
        (n, _) => trf("{n} bits", &[("n", &n)]),
    }
}

/// A file type list and its bit depth list; `from_template` adds "Based on output template"
/// (an empty `format`), which leaves the bit depth to the template.
fn file_type_lists(ui: &mut Ui, id: (&str, usize, usize), from_template: bool, format: &mut String, bit_depth: &mut u8, dithering: &mut bool) {
    let parsed = FileFormat::from_extension(format);

    // A depth the type does not have (or none) becomes its default
    if let Some(f) = parsed
        && !depth_options(f).contains(&(*bit_depth, *dithering))
    {
        (*bit_depth, *dithering) = (f.default_depth(), false);
    }
    let shown = if format.is_empty() { tr("Based on output template").to_string() } else { file_type_name(format) };

    // Tall enough for the whole list
    let list = egui::ComboBox::from_id_salt(("type", id)).width(if from_template { 166.0 } else { 90.0 }).height(f32::INFINITY);
    list.selected_text(shown).show_ui(ui, |ui| {
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
    let options = parsed.map(depth_options).unwrap_or_default();
    let shown = if parsed.is_some() { depth_label(parsed, *bit_depth, *dithering) } else { String::new() };
    ui.add_enabled_ui(options.len() > 1, |ui| {
        egui::ComboBox::from_id_salt(("bits", id)).width(if from_template { 160.0 } else { 120.0 }).selected_text(shown).show_ui(ui, |ui| {
            for (d, dither) in options {
                if ui.selectable_label((*bit_depth, *dithering) == (d, dither), depth_label(parsed, d, dither)).clicked() {
                    (*bit_depth, *dithering) = (d, dither);
                }
            }
        });
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

/// The template list: the first group (name, whether TextureCraft has it), then the others
/// (built-in and user templates) sorted by name.
pub(crate) fn template_groups(app: &TextureCraftApp, builtins: &[ExportPreset]) -> (Vec<(&'static str, bool)>, Vec<String>) {
    let first = FIRST_GROUP.iter().map(|name| (*name, builtins.iter().any(|p| p.name == *name))).collect();
    let mut rest: Vec<String> = builtins.iter().map(|p| p.name.clone()).filter(|n| !FIRST_GROUP.contains(&n.as_str())).collect();
    rest.extend(app.session.export_presets.list.iter().map(|p| p.name.clone()));
    rest.sort();
    rest.dedup();
    (first, rest)
}

/// A template dropdown over `first` and `rest`; true when `current` changed
fn template_combo(ui: &mut Ui, id: &str, current: &mut String, first: &[(&str, bool)], rest: &[String]) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id).width(VALUE_WIDTH).selected_text(current.clone()).show_ui(ui, |ui| {
        for (name, have) in first {
            if *have {
                changed |= ui.selectable_value(current, (*name).to_string(), *name).on_hover_text(*name).changed();
            } else {
                ui.add_enabled(false, egui::Button::selectable(false, *name)).on_disabled_hover_text(tr(NOT_AVAILABLE));
            }
        }
        ui.separator().on_hover_text(tr("Some templates in this list are unavailable"));
        for name in rest {
            changed |= ui.selectable_value(current, name.clone(), name).on_hover_text(name).changed();
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

/// A size as a texture set page shows it: "1024x1024", marked when it is the set's own
fn set_size_label(size: u32, own: u32) -> String {
    if size == own { trf("{n}x{n} (Texture Set size)", &[("n", &size)]) } else { format!("{size}x{size}") }
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
        .map(|ts| {
            let mut p = set_params(app, ts);
            p["dir"] = json!(dir);

            // Capture the template so edits during export cannot change the files
            let preset = p.get("preset").and_then(serde_json::Value::as_str).and_then(|name| app.session.export_preset(name));
            if let Some(v) = preset.and_then(|t| serde_json::to_value(t).ok()) {
                p["preset"] = v;
            }
            (ts, p)
        })
        .collect();
    let preview: Vec<_> = pending.iter().filter_map(|(index, params)| plan_set(app, *index, params)).collect();
    let error = preview.iter().find_map(|plan| plan.files.as_ref().err().cloned()).or_else(|| name_clash(&preview));
    let e = &mut app.ui.export;
    e.tab = 2;
    e.console = vec![ConsoleLine::Step(tr("Preparing texture export").into())];
    if let Some(error) = error {
        e.console.push(ConsoleLine::Step(error.clone()));
        app.toast(error, true);
        return;
    }

    e.job = Some(crate::state::ExportJob { total: pending.len(), pending, preview, directory: dir, written: Vec::new(), frame: 0 });
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

fn planned_file_shapes(plan: &serde_json::Value) -> impl Iterator<Item = (&serde_json::Value, &serde_json::Value)> {
    plan.as_array().into_iter().flatten().map(|file| (&file["file"], &file["size"]))
}

/// In the browser the textures download (one zip for several files).
#[cfg(target_arch = "wasm32")]
fn start_export(app: &mut TextureCraftApp) {
    let jobs: Vec<(usize, serde_json::Value)> = export_selection(app).into_iter().map(|ts| (ts, set_params(app, ts))).collect();
    crate::web_files::export_textures(app, &jobs);
}

/// Write the next texture set of the running export, logging the steps as the reference does;
/// the last one finishes the export. A failed command (its error already shown) stops it.
fn export_step(app: &mut TextureCraftApp, frame: u64) {
    let Some(job) = app.ui.export.job.as_mut() else { return };
    if job.frame == frame {
        return;
    }

    job.frame = frame;
    if job.pending.is_empty() {
        let written = std::mem::take(&mut job.written);
        let mut lines = vec![
            ConsoleLine::Step(tr("All selected texture sets processed").into()),
            ConsoleLine::Step(tr("Collecting the exported files").into()),
            ConsoleLine::Done(tr("Texture export complete").into()),
            ConsoleLine::Done(tr("Files saved").into()),
        ];
        lines.extend(written.into_iter().map(ConsoleLine::Done));
        app.ui.export.console.extend(lines);
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

    let Some(response) = app.run("file.exportTextures", params) else {
        app.ui.export.job = None;
        return;
    };

    let files: Vec<String> = response
        .get("files")
        .and_then(|file| file.as_array())
        .into_iter()
        .flatten()
        .filter_map(|file| file.as_str())
        .map(|file| std::path::Path::new(file).file_name().map_or_else(|| file.to_string(), |n| n.to_string_lossy().into_owned()))
        .collect();
    let mut lines = vec![set(), ConsoleLine::Step(tr("Saving texture files").into())];
    for file in &files {
        let stem = file.rsplit_once('.').map_or(file.as_str(), |(stem, _)| stem);
        lines.push(set());
        lines.push(ConsoleLine::Step(trf("Saved texture {file}", &[("file", &stem)])));
    }
    lines.extend([set(), ConsoleLine::Step(tr("Finished writing this texture set").into())]);
    app.ui.export.console.extend(lines);
    if let Some(job) = app.ui.export.job.as_mut() {
        job.written.extend(files);
    }
}

/// The `file.exportTextures` / `export.plan` parameters of the window's global settings
pub(crate) fn export_params(app: &TextureCraftApp) -> serde_json::Value {
    let e = &app.ui.export;
    let mut p = json!({"preset": e.preset, "padding": e.padding, "dilation": e.dilation});
    if !e.format.is_empty() {
        p["format"] = json!(e.format);
    }
    if !e.format.is_empty() && e.bit_depth > 0 {
        p["bitDepth"] = json!(e.bit_depth);
        p["dithering"] = json!(e.dithering);
    }
    match e.size {
        0 => {}
        SIZE_OF_LARGEST_SET => p["size"] = json!(largest_set(app)),
        n => p["size"] = json!(n),
    }
    if !e.normal_format.is_empty() {
        p["normalFormat"] = json!(e.normal_format);
    }
    if !e.color_space.is_empty() {
        p["colorSpace"] = json!(e.color_space);
    }
    p
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

/// Shorten a long file name pattern at the front, as the reference's Output maps rows do
fn elide_front(text: &str) -> String {
    const SHOWN: usize = 26;
    let n = text.chars().count();
    if n <= SHOWN { text.to_string() } else { format!("…{}", text.chars().skip(n - (SHOWN - 1)).collect::<String>()) }
}

/// A file type and bit depth as an Output maps row shows them ("png   8 bits")
fn file_type_text(t: &FileType) -> String {
    format!("{}   {}", file_type_name(&t.format), depth_label(FileFormat::from_extension(&t.format), t.bit_depth, t.dithering))
}

/// The rows of texture set `ts`'s Output maps: the template's maps the set has sources for
fn map_rows(app: &mut TextureCraftApp, ts: usize) -> Vec<MapRow> {
    let mut p = set_params(app, ts);
    if let Some(o) = p.as_object_mut() {
        o.remove("maps");
    }
    let preset = app.session.export_preset(&set_template(app, ts));
    let Ok(plan) = texturecraft_engine::execute(&mut app.session, "export.plan", &p) else { return Vec::new() };
    let mut rows: Vec<MapRow> = Vec::new();
    for f in plan.as_array().into_iter().flatten() {
        let Some(map) = f.get("map").and_then(serde_json::Value::as_u64).map(|m| m as usize) else { continue };
        if rows.iter().any(|r| r.map == map) {
            continue;
        }

        let format = f.get("format").and_then(serde_json::Value::as_str).unwrap_or("png").to_string();
        let bit_depth = f.get("bitDepth").and_then(serde_json::Value::as_u64).unwrap_or(8) as u8;
        let dithering = f.get("dithering").and_then(serde_json::Value::as_bool).unwrap_or(false);
        let pattern = preset.as_ref().and_then(|p| p.maps.get(map)).map(|m| m.file.clone()).unwrap_or_default();
        rows.push(MapRow { map, pattern, inherited: FileType { format: file_type_name(&format), bit_depth, dithering } });
    }
    rows
}

/// The Output maps section of texture set `ts`'s page
fn output_maps(app: &mut TextureCraftApp, ui: &mut Ui, ts: usize) {
    let rows = map_rows(app, ts);
    let template = set_template(app, ts);
    let o = app.ui.export.sets.entry(ts).or_default();
    if o.maps_template != template {
        o.maps.clear();
        o.maps_template = template;
    }
    let mut menu = None;
    header(ui, tr("Output maps"), |ui| {
        list_menu(ui, |ui| {
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
    });
    egui::Grid::new(("exp_maps", ts)).num_columns(3).spacing(vec2(8.0, 6.0)).show(ui, |ui| {
        for row in &rows {
            let m = o.maps.entry(row.map).or_default();
            ui.checkbox(&mut m.enabled, "");
            ui.add(egui::Label::new(elide_front(&row.pattern)).selectable(false)).on_hover_text(&row.pattern);
            ui.horizontal(|ui| {
                if icon_button(ui, ic::PENCIL_SIMPLE, tr("Enable override"), m.file_type.is_some(), ICON).clicked() {
                    m.file_type = if m.file_type.is_some() { None } else { Some(row.inherited.clone()) };
                }
                match &mut m.file_type {
                    Some(t) => file_type_combo(ui, ("exp_map", ts, row.map), t),
                    None => {
                        ui.label(file_type_text(&row.inherited));
                    }
                }
            });
            ui.end_row();
        }
    });
    for row in &rows {
        let m = o.maps.entry(row.map).or_default();
        match menu {
            Some(0) => m.enabled = true,
            Some(1) => m.enabled = false,
            Some(2) => m.enabled = !m.enabled,
            Some(3) if m.file_type.is_none() => m.file_type = Some(row.inherited.clone()),
            Some(4) => m.file_type = None,
            _ => {}
        }
    }
    o.maps.retain(|_, m| *m != MapExport::default());
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

/// A header with a menu or tool at its right end
fn header(ui: &mut Ui, title: &str, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.label(caption(title));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
    });
}

/// The menu button of a list header
fn list_menu(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.menu_button(RichText::new(ic::LIST_CHECKS).size(15.0), add);
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

/// Output directory: the path as a button that opens the folder picker, and a reset
#[cfg(not(target_arch = "wasm32"))]
fn output_directory(app: &mut TextureCraftApp, ui: &mut Ui) {
    ui.label(tr("Output directory"));
    ui.horizontal(|ui| {
        let dir = app.ui.export.dir.clone();
        let button = egui::Button::new(dir.as_str()).truncate();
        if ui.add_sized(vec2(VALUE_WIDTH - 24.0, 20.0), button).on_hover_text(dir.as_str()).clicked() {
            app.ask_file(FileDialog::folder().directory(dir), Bound::App, |app, paths| {
                if let Some(d) = paths.first() {
                    app.ui.export.dir = d.display().to_string();
                }
            });
        }
        if ui.small_button("R").on_hover_text(tr("Reset to default")).clicked() {
            app.ui.export.dir = ExportUi::default().dir;
        }
    });
    ui.end_row();
}

/// The dilation width: a button showing it, with a slider in its menu (only for paddings that
/// have a width)
fn dilation_button(ui: &mut Ui, padding: &str, dilation: &mut u32) {
    let enabled = has_width(padding);

    // The menu stays open while the slider is used
    let config = egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    let r = ui.add_enabled_ui(enabled, |ui| {
        egui::containers::menu::MenuButton::new(dilation.to_string()).config(config).ui(ui, |ui| {
            ui.label(tr("Dilation (pixels)"));
            ui.add(egui::Slider::new(dilation, 0..=256).step_by(1.0));
        })
    });
    if enabled {
        r.response.on_hover_text(tr("Dilation in pixels"));
    } else {
        r.response.on_disabled_hover_text(tr("This padding mode does not use a dilation width"));
    }
}

/// A greyed reference checkbox with its tooltip and the "not available" hint
fn unavailable_checkbox(ui: &mut Ui, label: &str, tooltip: &str) {
    let mut off = false;
    ui.add_enabled(false, egui::Checkbox::new(&mut off, label)).on_disabled_hover_ui(|ui| {
        ui.label(tooltip);
        ui.label(tr(NOT_AVAILABLE));
    });
}

/// The Global settings page
fn global_page(app: &mut TextureCraftApp, ui: &mut Ui, builtins: &[ExportPreset]) {
    let mut reset = false;
    header(ui, tr("General Export Parameters"), |ui| {
        reset = icon_button(ui, ic::ARROW_COUNTER_CLOCKWISE, tr("Restore the default export parameters"), false, ICON).clicked();
    });
    if reset {
        let d = ExportUi::default();
        let e = &mut app.ui.export;
        (e.dir, e.format, e.bit_depth, e.dithering, e.size, e.padding, e.dilation, e.normal_format, e.color_space) =
            (d.dir, d.format, d.bit_depth, d.dithering, d.size, d.padding, d.dilation, d.normal_format, d.color_space);
        crate::export_templates::select(app, d.preset);
    }
    let (first, rest) = template_groups(app, builtins);
    egui::Grid::new("exp_grid").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
        // The browser downloads the files instead
        #[cfg(not(target_arch = "wasm32"))]
        output_directory(app, ui);
        ui.label(tr("Output template"));
        let mut picked = app.ui.export.preset.clone();
        if template_combo(ui, "exp_preset", &mut picked, &first, &rest) {
            crate::export_templates::select(app, picked);
        }
        ui.end_row();
        ui.label(tr("File type"));
        ui.horizontal(|ui| {
            let e = &mut app.ui.export;
            file_type_lists(ui, ("exp_global", 0, 0), true, &mut e.format, &mut e.bit_depth, &mut e.dithering);
        });
        ui.end_row();
        ui.label(tr("Size"));
        let e = &mut app.ui.export;
        egui::ComboBox::from_id_salt("exp_size").width(VALUE_WIDTH).selected_text(size_label(e.size)).show_ui(ui, |ui| {
            ui.selectable_value(&mut e.size, 0, size_label(0));
            ui.separator();
            ui.selectable_value(&mut e.size, SIZE_OF_LARGEST_SET, size_label(SIZE_OF_LARGEST_SET));
            for r in SIZES {
                ui.selectable_value(&mut e.size, r, size_label(r));
            }
        });
        ui.end_row();
        ui.label(tr("Padding"));
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("exp_pad").width(260.0).selected_text(tr(padding_label(&e.padding))).show_ui(ui, |ui| {
                for (v, l) in PADDINGS {
                    ui.selectable_value(&mut e.padding, v.to_string(), tr(l));
                }
            });
            dilation_button(ui, &e.padding, &mut e.dilation);
        });
        ui.end_row();
    });

    // Unavailable export controls stay disabled
    unavailable_checkbox(ui, tr("Export USD asset"), tr("Package the mesh, materials and textures together as USD"));
    unavailable_checkbox(ui, tr("Export shader settings"), tr("Save the settings of each shader instance in a JSON file"));
    ui.add_space(8.0);
    egui::Grid::new("exp_grid_more").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
        let e = &mut app.ui.export;
        ui.label(tr("Normal map format"));
        let nf = match e.normal_format.as_str() {
            "openGl" => "OpenGL",
            "directX" => "DirectX",
            _ => tr("From template / project"),
        };
        egui::ComboBox::from_id_salt("exp_nf").selected_text(nf).show_ui(ui, |ui| {
            for (v, l) in [("", tr("From template / project")), ("openGl", "OpenGL"), ("directX", "DirectX")] {
                ui.selectable_value(&mut e.normal_format, v.to_string(), l);
            }
        });
        ui.end_row();
        ui.label(tr("Color space"));
        let shown = if e.color_space.is_empty() { tr("Default") } else { e.color_space.as_str() };
        egui::ComboBox::from_id_salt("exp_cs").selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(&mut e.color_space, String::new(), tr("Default"));
            for name in ["sRGB", "Linear Rec.709", "ACEScg", "ACES2065-1"] {
                ui.selectable_value(&mut e.color_space, name.to_string(), name);
            }
        });
        ui.end_row();
    });
}

/// The left column: Global settings, then a row per texture set with its export tick; a click
/// on a row shows the set's page, "*" marks a set with overrides
fn set_list(app: &mut TextureCraftApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| list_menu(ui, |ui| set_checks_menu(app, ui)));
    });
    let header = ui.selectable_label(app.ui.export.page.is_none(), RichText::new(tr("Global settings")).strong());
    if header.clicked() {
        app.ui.export.page = None;
    }

    // Many texture sets scroll inside the list; the footer stays in view
    let list_height = BODY_HEIGHT - header.rect.height() - ui.spacing().item_spacing.y;
    egui::ScrollArea::vertical().id_salt("export_texture_sets").max_height(list_height.max(0.0)).show(ui, |ui| set_rows(app, ui));
}

/// A row per texture set: its export tick, its name and the override mark
fn set_rows(app: &mut TextureCraftApp, ui: &mut Ui) {
    for i in 0..app.session.set_count() {
        let name = app.session.doc.project.texture_sets.get(i).map(|t| t.name.clone()).unwrap_or_default();
        let mut on = !app.ui.export.skip.contains(&i);
        ui.horizontal(|ui| {
            if ui.checkbox(&mut on, "").changed() {
                if on {
                    app.ui.export.skip.retain(|x| *x != i);
                } else {
                    app.ui.export.skip.push(i);
                }
            }
            if ui.selectable_label(app.ui.export.page == Some(i), name).clicked() {
                app.ui.export.page = Some(i);
            }
            if app.ui.export.sets.get(&i).is_some_and(SetExport::any) {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label("*").on_hover_text(tr("Override(s) active"));
                });
            }
        });
    }
}

/// A texture set page row: the label, the override toggle, then the inherited value as text or,
/// overridden, `edit`; true when the toggle was clicked
fn override_row(ui: &mut Ui, label: &str, on: bool, inherited: &str, edit: impl FnOnce(&mut Ui)) -> bool {
    ui.label(label);
    let mut toggled = false;
    ui.horizontal(|ui| {
        toggled = icon_button(ui, ic::PENCIL_SIMPLE, tr("Enable override"), on, ICON).clicked();
        if on {
            edit(ui);
        } else {
            ui.label(inherited);
        }
    });
    ui.end_row();
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

/// A file type dropdown (without "Based on output template") and its bit depth
fn file_type_combo(ui: &mut Ui, id: (&str, usize, usize), t: &mut FileType) {
    file_type_lists(ui, id, false, &mut t.format, &mut t.bit_depth, &mut t.dithering);
}

/// A texture set's page: its overrides of the global settings
fn set_page(app: &mut TextureCraftApp, ui: &mut Ui, ts: usize, builtins: &[ExportPreset]) {
    let Some(set) = app.session.doc.project.texture_sets.get(ts) else { return };
    let own = set.block_resolution();
    let mut from = InheritedSettings::for_texture_set(app, own);

    // "Based on output template": the override starts from what the template writes
    if app.ui.export.format.is_empty() && app.ui.export.sets.get(&ts).is_none_or(|o| o.file_type.is_none()) {
        from.file_type = template_file_type(app, ts);
    }
    let (first, rest) = template_groups(app, builtins);
    let first: Vec<(&str, bool)> = first.into_iter().filter(|(n, _)| !NOT_PER_SET.contains(n)).collect();
    let global = &app.ui.export;
    let inherited_file_type = if global.format.is_empty() { tr("Based on output template").to_string() } else { file_type_text(&from.file_type) };
    let inherited_size = match global.size {
        0 => set_size_label(own, own),
        _ => set_size_label(from.size, own),
    };
    let inherited_padding = tr(padding_label(&global.padding)).to_string();

    let o = app.ui.export.sets.entry(ts).or_default();
    let mut all = None;
    header(ui, tr("General Export Parameters"), |ui| {
        list_menu(ui, |ui| {
            if ui.button(tr("Override every item")).clicked() {
                all = Some(true);
            }
            if ui.button(tr("Clear all overrides")).clicked() {
                all = Some(false);
            }
            ui.separator();
        });
    });
    let mut toggled = None;
    egui::Grid::new(("exp_set_grid", ts)).num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
        let on = o.preset.is_some();
        if override_row(ui, tr("Output template"), on, &from.preset, |ui| {
            if let Some(p) = &mut o.preset {
                template_combo(ui, "exp_set_preset", p, &first, &rest);
            }
        }) {
            toggled = Some(0);
        }
        let on = o.file_type.is_some();
        if override_row(ui, tr("File type"), on, &inherited_file_type, |ui| {
            if let Some(t) = &mut o.file_type {
                file_type_combo(ui, ("exp_set", ts, usize::MAX), t);
            }
        }) {
            toggled = Some(1);
        }
        let on = o.size.is_some();
        if override_row(ui, tr("Size"), on, &inherited_size, |ui| {
            if let Some(size) = &mut o.size {
                egui::ComboBox::from_id_salt("exp_set_size").width(VALUE_WIDTH).selected_text(set_size_label(*size, own)).show_ui(ui, |ui| {
                    for r in SIZES {
                        ui.selectable_value(size, r, set_size_label(r, own));
                    }
                });
            }
        }) {
            toggled = Some(2);
        }
        let on = o.padding.is_some();
        if override_row(ui, tr("Padding"), on, &inherited_padding, |ui| {
            if let Some((padding, dilation)) = &mut o.padding {
                egui::ComboBox::from_id_salt("exp_set_pad").width(260.0).selected_text(tr(padding_label(padding))).show_ui(ui, |ui| {
                    for (v, l) in PADDINGS {
                        ui.selectable_value(padding, v.to_string(), tr(l));
                    }
                });
                dilation_button(ui, padding, dilation);
            }
        }) {
            toggled = Some(3);
        }
    });
    if let Some(row) = toggled {
        toggle(o, row, &from);
    }
    match all {
        Some(true) => {
            for row in 0..4 {
                let on = [o.preset.is_some(), o.file_type.is_some(), o.size.is_some(), o.padding.is_some()];
                if !on.get(row).copied().unwrap_or(true) {
                    toggle(o, row, &from);
                }
            }
        }
        Some(false) => {
            o.preset = None;
            o.file_type = None;
            o.size = None;
            o.padding = None;
        }
        None => {}
    }
    ui.add_space(8.0);
    ui.separator();
    egui::ScrollArea::vertical().id_salt("exp_maps_scroll").auto_shrink([false; 2]).show(ui, |ui| output_maps(app, ui, ts));
    if app.ui.export.sets.get(&ts).is_some_and(SetExport::is_default) {
        app.ui.export.sets.remove(&ts);
    }
}

/// Width of the LIST OF EXPORTS console
const CONSOLE_WIDTH: f32 = 300.0;

/// Green of the console's success lines
fn success(tk: &Tokens) -> egui::Color32 {
    if tk.dark { egui::Color32::from_rgb(110, 200, 120) } else { egui::Color32::from_rgb(30, 130, 50) }
}

/// The LIST OF EXPORTS tab: a tree of the files each ticked texture set exports (file, size,
/// file type, bit depth; the full path as tooltip), and the console
fn list_tab(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tk = Tokens::get(ui.ctx());
    let hidden = app.ui.export.console_hidden;
    let tree_width = ui.available_width() - if hidden { 24.0 } else { CONSOLE_WIDTH + 24.0 };
    ui.vertical(|ui| {
        ui.set_width(tree_width);
        egui::ScrollArea::vertical().id_salt("exp_list").auto_shrink([false; 2]).show(ui, |ui| export_tree(app, ui, &tk));
    });

    // The thin toggle at the console's left edge
    let toggle = if hidden { ic::CARET_DOUBLE_LEFT } else { ic::CARET_DOUBLE_RIGHT };
    let r = ui.add_sized(vec2(14.0, ui.available_height()), egui::Button::new(toggle).frame(false));
    let r = if hidden { r } else { r.on_hover_text(tr("Hide Console")) };
    if r.clicked() {
        app.ui.export.console_hidden = !hidden;
    }
    if !hidden {
        ui.vertical(|ui| {
            ui.set_width(CONSOLE_WIDTH);
            ui.label(caption(&tr("Console").to_uppercase()).strong());
            ui.separator();
            egui::ScrollArea::vertical().id_salt("exp_console").auto_shrink([false; 2]).stick_to_bottom(true).show(ui, |ui| {
                for line in &app.ui.export.console {
                    match line {
                        ConsoleLine::Step(t) => {
                            ui.label(t);
                        }
                        ConsoleLine::Done(t) => {
                            ui.label(RichText::new(t).color(success(&tk)));
                        }
                        ConsoleLine::Set(name) => {
                            ui.label(set_line(ui, name, &tk));
                        }
                    }
                }
            });
        });
    }
}

/// "Texture set <name>" with the name in bold
fn set_line(ui: &Ui, name: &str, tk: &Tokens) -> egui::text::LayoutJob {
    let text = tr("Texture set {set}");
    let (before, after) = text.split_once("{set}").unwrap_or((text, ""));
    let font = egui::TextStyle::Body.resolve(ui.style());
    let plain = egui::TextFormat::simple(font.clone(), tk.text);
    let mut job = egui::text::LayoutJob::default();
    job.append(before, 0.0, plain.clone());
    job.append(name, 0.0, egui::TextFormat::simple(font, tk.text_strong));
    job.append(after, 0.0, plain);
    job
}

/// One expandable node per ticked texture set, a row per file
fn export_tree(app: &mut TextureCraftApp, ui: &mut Ui, tk: &Tokens) {
    if let Some(job) = &app.ui.export.job
        && !job.preview.is_empty()
    {
        for (index, set) in job.preview.iter().enumerate() {
            export_tree_set(ui, index, set, &job.directory, tk);
        }
        return;
    }

    for ts in export_selection(app) {
        let params = set_params(app, ts);
        if let Some(set) = plan_set(app, ts, &params) {
            export_tree_set(ui, ts, &set, &app.ui.export.dir, tk);
        }
    }
}

fn plan_set(app: &mut TextureCraftApp, index: usize, params: &serde_json::Value) -> Option<ExportSetPlan> {
    let set = app.session.doc.project.texture_sets.get(index)?;
    let name = set.name.clone();
    let files = texturecraft_engine::execute(&mut app.session, "export.plan", params).map_err(|error| error.to_string());

    Some(ExportSetPlan { index: Some(index), name, files })
}

fn export_tree_set(ui: &mut Ui, index: usize, set: &ExportSetPlan, directory: &str, tokens: &Tokens) {
    egui::CollapsingHeader::new(&set.name).id_salt(("exp_tree", index)).default_open(true).show(ui, |ui| match &set.files {
        Ok(files) => {
            egui::Grid::new(("exp_tree_grid", index)).num_columns(4).spacing(vec2(12.0, 4.0)).show(ui, |ui| {
                for output in files.as_array().into_iter().flatten() {
                    let file = output.get("file").and_then(serde_json::Value::as_str).unwrap_or_default();
                    let format = FileFormat::from_extension(output.get("format").and_then(serde_json::Value::as_str).unwrap_or_default());
                    let depth = output.get("bitDepth").and_then(serde_json::Value::as_u64).unwrap_or(8) as u8;
                    let dithering = output.get("dithering").and_then(serde_json::Value::as_bool).unwrap_or(false);
                    let size = output["size"].as_u64().unwrap_or_default() as u32;
                    let source_size = output["sourceSize"].as_u64().unwrap_or_default() as u32;
                    let path = std::path::Path::new(directory).join(file).display().to_string();
                    ui.label(file).on_hover_text(&path);
                    ui.label(set_size_label(size, source_size)).on_hover_text(&path);
                    ui.label(format.map_or("", FileFormat::name)).on_hover_text(&path);
                    ui.label(depth_label(format, depth, dithering)).on_hover_text(&path);
                    ui.end_row();
                }
            });
        }
        Err(error) => {
            ui.label(RichText::new(error).color(tokens.danger));
        }
    });
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

pub fn show(app: &mut TextureCraftApp, ctx: &egui::Context) {
    if !app.ui.windows.export {
        // The window opens on SETTINGS again; an export stops with it
        let e = &mut app.ui.export;
        e.job = None;
        e.ask = None;
        e.opened_with = None;
        e.tab = 0;
        e.page = None;
        return;
    }

    // Settings saved under the default template's old name
    if let Some(b) = texturecraft_engine::formats::builtin_preset(&app.ui.export.preset)
        && b.name != app.ui.export.preset
    {
        app.ui.export.preset = b.name;
    }
    keep_file_type_visible(&mut app.ui.export);
    follow_texture_sets(app);
    if app.ui.export.opened_with.is_none() {
        let mut now = app.ui.export.clone();
        now.draft = None;
        app.ui.export.opened_with = Some(Box::new(now));
    }

    let mut open = true;
    let mut do_export = false;
    let mut canceled = false;
    let presets = texturecraft_engine::formats::builtin_presets();
    crate::windows::dialog("export_textures", tr("Export textures"), &mut open).fixed_size(vec2(900.0, 520.0)).show(ctx, |ui| {
        // Nothing to export without a project
        if !app.session.project_open {
            ui.label(tr("No project is open"));
            return;
        }

        ui.horizontal(|ui| {
            for (i, t) in [tr("Settings"), tr("Output templates"), tr("List of exports")].iter().enumerate() {
                if ui.selectable_label(app.ui.export.tab == i as u8, caption(&t.to_uppercase())).clicked() {
                    app.ui.export.tab = i as u8;
                }
            }

            // The file list opens the destination folder
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if app.ui.export.tab == 2 {
                    // The browser downloads the files instead
                    #[cfg(target_arch = "wasm32")]
                    crate::unavailable::greyed(ui, |ui| ui.button(tr("Open output directory")));
                    #[cfg(not(target_arch = "wasm32"))]
                    if ui.button(tr("Open output directory")).clicked() {
                        open_output_directory(app);
                    }
                } else if ui.button(tr("Include all")).on_hover_text(tr("Select every texture set, UV tile and output map")).clicked() {
                    app.ui.export.skip.clear();
                    for o in app.ui.export.sets.values_mut() {
                        o.maps.values_mut().for_each(|m| m.enabled = true);
                    }
                    app.ui.export.sets.retain(|_, o| !o.is_default());
                }
            });
        });
        ui.separator();
        ui.allocate_ui_with_layout(vec2(ui.available_width(), BODY_HEIGHT), egui::Layout::left_to_right(egui::Align::Min), |ui| {
            ui.set_height(BODY_HEIGHT);
            match app.ui.export.tab {
                1 => {
                    crate::export_templates::tab(app, ui, &presets);
                    return;
                }
                2 => {
                    list_tab(app, ui);
                    return;
                }
                _ => {}
            }

            ui.vertical(|ui| {
                ui.set_width(LIST_WIDTH);
                set_list(app, ui);
            });
            ui.separator();
            ui.vertical(|ui| match app.ui.export.page {
                Some(ts) if ts < app.session.set_count() => set_page(app, ui, ts, &presets),
                _ => global_page(app, ui, &presets),
            });
        });
        ui.add_space(8.0);
        ui.separator();

        // While exporting: the progress and Cancel Export
        if let Some(job) = &app.ui.export.job {
            let done = job.total - job.pending.len();
            let progress = if job.total == 0 { 1.0 } else { done as f32 / job.total as f32 };
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr("Cancel Export")).clicked() {
                        app.ui.export.job = None;
                    }
                    let percent = format!("{:.0}%", progress * 100.0);
                    ui.add(egui::ProgressBar::new(progress).text(percent));
                });
            });
            return;
        }

        let selected = selected_texture_count(app);
        ui.horizontal(|ui| {
            // Not on LIST OF EXPORTS
            if app.ui.export.tab != 2 && ui.button(tr("Reset all")).on_hover_text(tr("Restore the export defaults and select every texture")).clicked() {
                // Keep ownership so Cancel can restore the opening settings
                let e = &mut app.ui.export;
                let (opened_with, owner, owner_id) = (e.opened_with.take(), std::mem::take(&mut e.owner), e.owner_id);
                app.ui.export = ExportUi { opened_with, owner, owner_id, ..ExportUi::default() };
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Nothing to write without a ticked texture set
                let any = !export_selection(app).is_empty();
                if ui.add_enabled(any, primary_button(ui, tr("Export"))).on_hover_text(tr("Write the selected textures using the settings above")).clicked() {
                    do_export = true;
                }

                // Saving closes the window without starting an export
                if ui.button(tr("Save settings")).on_hover_text(tr("Keep these settings for the next export")).clicked() {
                    app.ui.windows.export = false;
                }
                if ui.button(tr("Cancel")).on_hover_text(tr("Restore the settings from when you opened this window")).clicked() {
                    canceled = true;
                }
                ui.label(trf("{n} textures selected for export", &[("n", &selected)]));
            });
        });
    });
    crate::export_templates::ask(app, ctx);

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
