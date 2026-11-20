//! File > New project: BASIC SETTINGS (mesh file, template, resolution), ADVANCED SETTINGS (normal
//! map format, UV tiles, import settings, baked maps imported at creation, physical size, color
//! management), and Cancel / Create fixed below the scrolling form.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use egui::{RichText, Ui, vec2};
use serde_json::{Value, json};
use texturecraft_engine::core::NormalFormat;
use texturecraft_engine::template_cmds::template_label;

use crate::TextureCraftApp;
use crate::i18n::{tr, trf};
use crate::project_form::{
    FIELD_WIDTH_POINTS, ROW_INDENT_POINTS, auto_unwrap_row, check_row, collapsible, color_management, note, physical_size, row, sub_title, tangent_space_row,
    unavailable, uv_tile_radios,
};
use crate::state::{Mode, NewProject};
use crate::theme::{Tokens, caption};
use crate::widgets::primary_button;
use crate::windows::{dialog, file_dialogs_available};

/// Each normal map format with its command id and label
const NORMAL_FORMATS: [(NormalFormat, &str, &str); 2] = [(NormalFormat::OpenGl, "openGl", "OpenGL"), (NormalFormat::DirectX, "directX", "DirectX")];

/// Document resolutions offered (texture sets range from 128 to 8192 px)
const RESOLUTIONS: [u32; 7] = [128, 256, 512, 1024, 2048, 4096, 8192];

/// Width of the dialog's form
const DIALOG_WIDTH_POINTS: f32 = 560.0;

/// Leave space around the centered dialog on short viewports
const DIALOG_VIEWPORT_MARGIN_POINTS: f32 = 32.0;

/// Room for the title bar and the button row below the scrolling form
const DIALOG_CHROME_HEIGHT_POINTS: f32 = 96.0;

/// Width of the Add and Clear buttons
const FILE_BUTTON_WIDTH_POINTS: f32 = 96.0;

/// Height of the list of added baked map files (it scrolls when longer)
const FILE_LIST_HEIGHT_POINTS: f32 = 120.0;

/// Width of the Cancel and Create buttons
const BUTTON_WIDTH_POINTS: f32 = 90.0;

/// File > New project...: start a fresh form, carrying over failed imports from a closed form.
pub(crate) fn open(app: &mut TextureCraftApp) {
    let baked_maps = if app.ui.windows.new_project { Vec::new() } else { std::mem::take(&mut app.ui.new_project.baked_maps) };

    app.ui.new_project = NewProject { baked_maps, opening_id: next_request_id(), ..NewProject::default() };

    // The list opens on the template the last project was created from
    if let Some(template) = app.session.template(&app.ui.last_template)
        && template.name != texturecraft_engine::template_cmds::DEFAULT_TEMPLATE
    {
        app.ui.new_project.template = template.name;
        app.ui.new_project.resolution = template.resolution;
    }
    app.ui.windows.new_project = true;
}

fn next_request_id() -> u64 {
    static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
}

fn close(app: &mut TextureCraftApp) {
    app.ui.windows.new_project = false;
    app.ui.new_project.clear_mesh();
}

/// The New project dialog.
pub(crate) fn show(app: &mut TextureCraftApp, ctx: &egui::Context) {
    if !app.ui.windows.new_project {
        return;
    }

    let mut open = true;
    let (mut create, mut cancel) = (false, false);

    // A fixed height keeps the centered window still while its sections lay out
    let form_height = (ctx.content_rect().height() - DIALOG_VIEWPORT_MARGIN_POINTS - DIALOG_CHROME_HEIGHT_POINTS)
        .clamp(crate::project_form::MIN_FORM_HEIGHT_POINTS, crate::project_form::FORM_HEIGHT_POINTS);
    dialog("new_project", tr("New project"), &mut open).default_width(DIALOG_WIDTH_POINTS).show(ctx, |ui| {
        ui.set_width(DIALOG_WIDTH_POINTS);
        egui::ScrollArea::vertical().id_salt("np_form").min_scrolled_height(form_height).max_height(form_height).auto_shrink([false, true]).show(ui, |ui| {
            basic_settings(app, ui);
            ui.add_space(8.0);
            advanced_settings(app, ui);
        });
        ui.separator();
        let can_create = app.ui.new_project.file.is_some();
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                create = ui
                    .add_enabled(can_create, primary_button(ui, tr("Create")).min_size(vec2(BUTTON_WIDTH_POINTS, 0.0)))
                    .on_disabled_hover_text(tr("Select a 3D mesh first"))
                    .clicked();
                cancel = ui.add(egui::Button::new(tr("Cancel")).min_size(vec2(BUTTON_WIDTH_POINTS, 0.0))).clicked();
            });
        });
    });

    if cancel || !open {
        app.ui.new_project.baked_maps = Vec::new();
        close(app);
    } else if create && app.ui.windows.unsaved.is_none() {
        let opening_id = app.ui.new_project.opening_id;
        if !crate::windows::ask_to_save(app, crate::state::Unsaved::CreateProject(opening_id)) {
            finish_create(app, opening_id);
        }
    }
}

pub(crate) fn finish_create(app: &mut TextureCraftApp, opening_id: u64) -> bool {
    if !app.ui.windows.new_project || app.ui.new_project.opening_id != opening_id || !create_project(app) {
        return false;
    }

    close(app);
    app.ui.last_template = app.ui.new_project.template.clone();
    app.ui.mode = Mode::Painting;
    apply_after_create(app);
    true
}

/// The `file.new` command for the form: the chosen mesh file with the settings the engine takes at
/// creation (the project is named after the file). None until a mesh file is chosen.
pub fn create_command(new_project: &NewProject) -> Option<Value> {
    let mesh = new_project.file.as_ref()?;
    Some(json!({
        "mesh": mesh,
        "resolution": new_project.resolution,
        "uvTiles": new_project.uv_tiles == Some(true),
        "template": new_project.template,
        "importCameras": new_project.import_cameras,
    }))
}

/// Create the project; false (with the error reported) when it failed.
#[cfg(not(target_arch = "wasm32"))]
fn create_project(app: &mut TextureCraftApp) -> bool {
    create_command(&app.ui.new_project).is_some_and(|params| app.run("file.new", params).is_some())
}

/// Create the project from the mesh file's bytes picked in the browser, as `file.new` does on the
/// desktop.
#[cfg(target_arch = "wasm32")]
fn create_project(app: &mut TextureCraftApp) -> bool {
    let new_project = &app.ui.new_project;
    let (Some(name), Some(bytes)) = (new_project.file.as_deref(), new_project.file_bytes.as_deref()) else { return false };

    let stem = std::path::Path::new(name).file_stem().and_then(|stem| stem.to_str()).unwrap_or("Untitled").to_string();
    let template = app.session.template(&new_project.template);
    match texturecraft_engine::Session::from_mesh_bytes_tiled(name, bytes, new_project.resolution, &stem, Some(new_project.uv_tiles == Some(true))) {
        Ok(mut next) => {
            if !new_project.import_cameras {
                next.doc.project.cameras.clear();
            }
            if let Some(template) = &template {
                next.apply_template(template);
            }
            app.session.switch_to(next);
            app.pbr_import = None;
            app.resize = None;
            app.layer_panel = Default::default();
            app.camera_glide = None;
            true
        }
        Err(error) => {
            app.toast(trf("Couldn't open {name}: {error}", &[("name", &name), ("error", &error)]), true);
            false
        }
    }
}

fn basic_settings(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tokens = Tokens::get(ui.ctx());
    ui.label(caption(&tr("Basic settings").to_uppercase()));
    let mut select = false;
    row(ui, tr("3D mesh"), Some(tr("The mesh file to paint. Create is available once one is selected.")), |ui| {
        select = ui.add(egui::Button::new(tr("Select...")).min_size(vec2(FILE_BUTTON_WIDTH_POINTS, 0.0))).clicked();
        match &app.ui.new_project.file {
            Some(path) => {
                ui.add(egui::Label::new(file_name(path)).truncate()).on_hover_text(path.as_str());
            }
            None => {
                ui.label(RichText::new(tr("No file selected")).italics().color(tokens.text_dim));
            }
        }
    });
    if select {
        ask_mesh_file(app, ui.ctx());
    }

    row(ui, tr("Project template"), Some(tr("Templates set the starting channels, resolution, normal map format and shader settings.")), |ui| {
        let new_project = &mut app.ui.new_project;
        let builtin = texturecraft_engine::template_cmds::builtin_templates();
        let user_templates = app.session.templates.list.iter();
        let listed: Vec<_> = builtin.iter().map(|template| (template, true)).chain(user_templates.map(|template| (template, false))).collect();
        let shown = listed
            .iter()
            .find(|(template, _)| template.name == new_project.template)
            .map_or_else(|| new_project.template.clone(), |(template, builtin)| template_label(&template.name, *builtin));
        egui::ComboBox::from_id_salt("np_template").width(FIELD_WIDTH_POINTS).selected_text(shown).show_ui(ui, |ui| {
            for (template, builtin) in listed {
                if ui.selectable_label(template.name == new_project.template, template_label(&template.name, builtin)).clicked() {
                    new_project.template = template.name.clone();
                    new_project.resolution = template.resolution;
                    new_project.normal_format = None;
                }
            }
        });
    });

    row(ui, tr("Document resolution (px)"), Some(tr("The texture size of every texture set. It can be changed later.")), |ui| {
        let resolution = &mut app.ui.new_project.resolution;
        egui::ComboBox::from_id_salt("np_res").width(FIELD_WIDTH_POINTS).selected_text(format!("{0} x {0}", *resolution)).show_ui(ui, |ui| {
            for size in RESOLUTIONS {
                ui.selectable_value(resolution, size, format!("{size} x {size}"));
            }
        });
    });
}

/// The collapsible ADVANCED SETTINGS section (open at first)
fn advanced_settings(app: &mut TextureCraftApp, ui: &mut Ui) {
    collapsible(ui, "np_advanced_settings", &tr("Advanced settings").to_uppercase(), true, None, |ui| {
        row(
            ui,
            tr("Normal map format"),
            Some(tr("Green channel convention of normal maps: OpenGL (Y+) or DirectX (Y-). Imported and exported normal maps follow it.")),
            |ui| normal_format_combo(app, ui),
        );
        tangent_space_row(ui);

        ui.add_space(4.0);
        sub_title(ui, tr("UV Tile settings (UDIMs)"), Some(tr("Paint meshes whose UVs span several UV tiles (UDIMs)")));
        note(ui, tr("You can enable or disable UV tiles later in Texture Set Settings."));
        let new_project = &mut app.ui.new_project;
        let mut uv_tiles = new_project.uv_tiles == Some(true);
        if check_row(ui, None, |ui| ui.checkbox(&mut uv_tiles, tr("Use UV Tile workflow")), |_| {}).changed() {
            new_project.uv_tiles = Some(uv_tiles);
        }
        uv_tile_radios(ui, uv_tiles, tr("Tick Use UV Tile workflow first"));

        ui.add_space(4.0);
        sub_title(ui, tr("Import settings"), None);
        check_row(ui, Some(tr("Keep the cameras stored in the mesh file")), |ui| ui.checkbox(&mut new_project.import_cameras, tr("Import cameras")), |_| {});
        auto_unwrap_row(ui);

        ui.add_space(4.0);
        collapsible(ui, "np_import_baked_maps", tr("Import baked maps"), true, None, |ui| {
            if baked_maps(&mut app.ui.new_project, ui) {
                ask_baked_map_files(app);
            }
        });
        physical_size(ui, "np_physical_size", true);
        color_management(ui, "np_color_management", &mut app.ui.new_project.color_management);
    });
}

fn normal_format_combo(app: &mut TextureCraftApp, ui: &mut Ui) {
    let new_project = &mut app.ui.new_project;
    let template_format = app.session.template(&new_project.template).map(|template| template.normal_format).unwrap_or_default();
    let current = new_project.normal_format.unwrap_or(template_format);
    let current_label = NORMAL_FORMATS.iter().find(|(format, _, _)| *format == current).map_or("", |(_, _, label)| label);
    egui::ComboBox::from_id_salt("np_normal_format").width(FIELD_WIDTH_POINTS).selected_text(current_label).show_ui(ui, |ui| {
        for (format, _, label) in NORMAL_FORMATS {
            if ui.selectable_label(format == current, label).clicked() {
                new_project.normal_format = Some(format);
            }
        }
    });
}

/// The baked map hint, buttons, file list and naming scheme; true when Add was clicked.
fn baked_maps(new_project: &mut NewProject, ui: &mut Ui) -> bool {
    let tokens = Tokens::get(ui.ctx());
    let mut add = false;
    ui.horizontal(|ui| {
        ui.add_space(ROW_INDENT_POINTS);
        let buttons_width = 2.0 * FILE_BUTTON_WIDTH_POINTS + ui.spacing().item_spacing.x;
        let hint_width = (ui.available_width() - buttons_width - ui.spacing().item_spacing.x).max(0.0);
        ui.allocate_ui_with_layout(vec2(hint_width, 0.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(hint_width);
            ui.add(egui::Label::new(tr("Import mesh normal maps and baked maps for all materials.")).wrap());
        });
        let add_button = ui.add_enabled(file_dialogs_available(), egui::Button::new(tr("Add")).min_size(vec2(FILE_BUTTON_WIDTH_POINTS, 0.0)));
        add = unavailable(add_button).clicked();
        if ui.add_enabled(!new_project.baked_maps.is_empty(), egui::Button::new(tr("Clear")).min_size(vec2(FILE_BUTTON_WIDTH_POINTS, 0.0))).clicked() {
            new_project.baked_maps.clear();
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(ROW_INDENT_POINTS);
        egui::Frame::new().fill(tokens.field).corner_radius(2.0).inner_margin(4.0).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("np_baked_map_list").max_height(FILE_LIST_HEIGHT_POINTS).auto_shrink([false, false]).show(ui, |ui| {
                ui.vertical(|ui| {
                    for path in &new_project.baked_maps {
                        ui.add(egui::Label::new(file_name(path)).truncate()).on_hover_text(path.as_str());
                    }
                });
            });
        });
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(ROW_INDENT_POINTS);
        ui.label(RichText::new(tr("Normal map filename: <texture set>_normal")).color(tokens.text_dim))
            .on_hover_text(tr("Name each file after its texture set, like Body_normal.png"));
    });
    add
}

/// List picked baked map files (files already listed are skipped)
pub fn add_baked_map_files(new_project: &mut NewProject, paths: Vec<String>) {
    for path in paths {
        if !new_project.baked_maps.contains(&path) {
            new_project.baked_maps.push(path);
        }
    }
}

/// After creation: set the normal format, import the listed baked maps (one undo step), toast the outcome; a failed import keeps the list
fn apply_after_create(app: &mut TextureCraftApp) {
    let chosen_format = app.ui.new_project.normal_format.filter(|format| *format != app.session.doc.project.normal_format);
    if let Some((_, id, _)) = chosen_format.and_then(|chosen| NORMAL_FORMATS.iter().find(|(format, _, _)| *format == chosen)) {
        app.run("textureSet.settings", json!({ "normalFormat": id }));
    }

    if app.ui.new_project.baked_maps.is_empty() {
        return;
    }

    let files: Vec<Value> = app.ui.new_project.baked_maps.iter().map(|path| json!({ "path": path })).collect();
    let Some(result) = app.run("textureSet.meshMap.import", json!({ "files": files })) else {
        return;
    };

    let assigned = result["assigned"].as_array().into_iter().flatten();
    let maps: BTreeSet<(u64, &str)> = assigned.filter_map(|entry| Some((entry["ts"].as_u64()?, entry["map"].as_str()?))).collect();
    let not_imported: Vec<String> = result["unmatched"].as_array().into_iter().flatten().filter_map(Value::as_str).map(file_name).collect();
    let mut text = trf("Baked maps imported: {count}", &[("count", &maps.len())]);
    if !not_imported.is_empty() {
        text.push('\n');
        text.push_str(&trf("Not imported: {files}", &[("files", &not_imported.join(", "))]));
    }
    app.toast(text, !not_imported.is_empty());
    app.ui.new_project.baked_maps.clear();
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |name| name.to_string_lossy().into_owned())
}

#[cfg(not(target_arch = "wasm32"))]
fn ask_mesh_file(app: &mut TextureCraftApp, _ctx: &egui::Context) {
    use crate::file_dialogs::{Bound, FileDialog};
    let dialog = FileDialog::open().filter(tr("3D meshes"), &texturecraft_engine::formats::mesh_import::MESH_EXTENSIONS);
    let opening_id = app.ui.new_project.opening_id;
    app.ask_file(dialog, Bound::App, move |app, paths| {
        if app.ui.windows.new_project
            && app.ui.new_project.opening_id == opening_id
            && let Some(path) = paths.first()
        {
            app.ui.new_project.file = Some(path.display().to_string());
            app.ui.new_project.file_bytes = None;
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn ask_mesh_file(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let request_id = next_request_id();
    app.ui.new_project.mesh_request_id = request_id;
    crate::web_files::pick_new_project_mesh(ctx, request_id);
}

#[cfg(not(target_arch = "wasm32"))]
fn ask_baked_map_files(app: &mut TextureCraftApp) {
    use crate::file_dialogs::{Bound, FileDialog};

    let opening_id = app.ui.new_project.opening_id;
    app.ask_file(FileDialog::open_many().filter(tr("Images"), crate::web_files::IMAGE_EXTS), Bound::App, move |app, files| {
        if app.ui.windows.new_project && app.ui.new_project.opening_id == opening_id {
            add_baked_map_files(&mut app.ui.new_project, files.into_iter().map(|path| path.display().to_string()).collect());
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn ask_baked_map_files(_app: &mut TextureCraftApp) {}
