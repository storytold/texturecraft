//! Floating windows and the dockable History panel

use egui::{RichText, Ui, vec2};
use egui_phosphor::regular as ic;
use serde_json::json;

use crate::TextureCraftApp;
#[cfg(not(target_arch = "wasm32"))]
use crate::file_dialogs::{Bound, FileDialog};
use crate::i18n::{tr, tr_history, trf};
#[cfg(not(target_arch = "wasm32"))]
use crate::state::Mode;
use crate::theme::Tokens;
use crate::widgets::primary_button;

pub fn show(app: &mut TextureCraftApp, ctx: &egui::Context) {
    crate::new_project_advanced::show(app, ctx);
    about(app, ctx);
    crate::settings_window::show(app, ctx);
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::project_config::show(app, ctx);
        crate::import_resources::show(app, ctx);
    }
    crate::keymap::window(app, ctx);
    crate::export_window::show(app, ctx);
    crate::mesh_export_options::show(app, ctx);
    #[cfg(target_arch = "wasm32")]
    export_mesh(app, ctx);
    crate::pbr_import::window(app, ctx);
    unsaved(app, ctx);
    crate::import_ui::show(app, ctx);
    reset_ui(app, ctx);
    crate::named_layouts::show(app, ctx);
}

/// Window ▸ Reset UI asks first: Escape closes the question, Enter answers Reset.
pub(crate) fn reset_ui_keys(app: &mut TextureCraftApp, input: &mut egui::InputState, nothing_focused: bool) {
    if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
        app.ui.windows.confirm_reset_ui = false;
    } else if nothing_focused && input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
        app.ui.adopt_layout(crate::UiState::default());
        app.ui.windows.confirm_reset_ui = false;
    }
}

fn reset_ui(app: &mut TextureCraftApp, ctx: &egui::Context) {
    if !app.ui.windows.confirm_reset_ui {
        return;
    }

    let mut open = true;
    let (mut reset, mut cancel) = (false, false);
    dialog("reset_ui", tr("Reset UI"), &mut open).show(ctx, |ui| {
        ui.label(tr("Reset the panels, view layout, zoom and every other view setting to their defaults? Settings and shortcuts stay. To reset only the panels, use Reset panel layout."));
        ui.add_space(8.0);

        // One row keeps the window to its contents; a bare right-to-left layout fills the screen height
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                reset = ui.add(primary_button(ui, tr("Reset"))).clicked();
                cancel = ui.button(tr("Cancel")).clicked();
            });
        });
    });

    if reset {
        app.ui.adopt_layout(crate::UiState::default());
    }
    if reset || cancel || !open {
        app.ui.windows.confirm_reset_ui = false;
    }
}

/// Hold `then` (quitting, opening or closing a project) behind the unsaved-changes dialog when
/// the project has unsaved changes; false when it can go ahead now.
pub(crate) fn ask_to_save(app: &mut TextureCraftApp, then: crate::state::Unsaved) -> bool {
    if app.session.modified {
        app.ui.windows.unsaved = Some(then);
    }
    app.session.modified
}

/// The close decision, once per frame in `logic` (which also runs while the window is minimized):
/// a close with unsaved changes waits on the dialog, in a window brought back so it can be
/// answered. A close nothing holds open quits through the platform's own path (craft-quit).
pub(crate) fn close_request(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let closing = ctx.input(|i| i.viewport().close_requested());
    // A stroke still open at the close, or released in this frame, is a change to ask about.
    if closing
        && app.session.stroke_active()
        && let Err(error) = app.session.end_stroke()
    {
        app.toast(error.to_string(), true);
    }
    if closing && ask_to_save(app, crate::state::Unsaved::Quit) {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        if ctx.input(|i| i.viewport().minimized == Some(true)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }
    craft_quit::on_close_request(ctx);
}

/// Quitting, opening or closing a project with unsaved changes asks whether to save them first
/// (in the browser, Save downloads the project).
pub(crate) fn unsaved(app: &mut TextureCraftApp, ctx: &egui::Context) {
    use crate::state::Unsaved;

    let Some(pending) = app.ui.windows.unsaved.clone() else { return };

    #[cfg(not(target_arch = "wasm32"))]
    let (waiting_for_save, saved_by_dialog) = (app.saving_for_prompt, std::mem::take(&mut app.prompt_saved));
    #[cfg(target_arch = "wasm32")]
    let (waiting_for_save, saved_by_dialog) = (false, false);

    let mut open = true;
    let mut save_first = None;
    let mut cancel = false;
    let question_args: [(&str, &dyn std::fmt::Display); 1] = [("name", &app.session.doc.project.name)];
    let question = if pending == Unsaved::Quit {
        trf("Save the changes to {name} before closing?", &question_args)
    } else {
        trf("Save the changes to {name} first?", &question_args)
    };
    dialog("unsaved", tr("Unsaved changes"), &mut open).show(ctx, |ui| {
        ui.label(question);
        ui.add_space(8.0);

        // One row keeps the window to its contents; a bare right-to-left layout fills the screen height
        ui.horizontal(|ui| {
            // The answers wait while the Save dialog is open
            ui.add_enabled_ui(!waiting_for_save, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(primary_button(ui, tr("Save"))).clicked() {
                        save_first = Some(true);
                    }
                    if ui.button(tr("Don't save")).clicked() {
                        save_first = Some(false);
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        });
    });
    if !open || cancel {
        app.ui.windows.unsaved = None;
        #[cfg(not(target_arch = "wasm32"))]
        {
            app.saving_for_prompt = false;
        }
    }

    let discarding = save_first == Some(false) && app.session.modified;
    match save_first {
        Some(true) => {
            // An untitled project is saved from a dialog that answers on a later frame
            #[cfg(not(target_arch = "wasm32"))]
            {
                app.saving_for_prompt = app.session.path.is_none();
            }
            file_action(app, ctx, "file.save");
        }
        // Going ahead anyway: the changes are let go
        Some(false) => app.session.modified = false,
        None if saved_by_dialog => {}
        None => return,
    }

    // A canceled or failed save keeps the project and this dialog open
    if app.session.modified {
        return;
    }

    app.ui.windows.unsaved = None;
    let import_was_active = crate::import_ui::active(app);
    match pending {
        Unsaved::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        Unsaved::Command(id) => crate::menus::invoke(app, ctx, &id),
        Unsaved::CreateProject(opening_id) => {
            if !crate::new_project_advanced::finish_create(app, opening_id) && discarding {
                app.session.modified = true;
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        Unsaved::Open(path) => open_path(app, &path),
        Unsaved::Load(name, bytes) => crate::web_files::load(app, &name, std::sync::Arc::unwrap_or_clone(bytes)),
    }
    // Conversion can still be cancelled or fail without replacing the project.
    if discarding && !import_was_active && crate::import_ui::active(app) {
        app.session.modified = true;
    }
}

/// A titled tool window; `id` keeps its place across UI languages.
pub(crate) fn window<'a>(id: &str, title: &str, open: &'a mut bool) -> egui::Window<'a> {
    egui::Window::new(RichText::new(title).font(crate::theme::semibold(13.0)).strong())
        .id(egui::Id::unique(id))
        .open(open)
        .resizable(true)
        .collapsible(false)
        .default_width(300.0)
}

/// A centered dialog; `id` keeps its place across UI languages.
pub(crate) fn dialog<'a>(id: &str, title: &str, open: &'a mut bool) -> egui::Window<'a> {
    egui::Window::new(title.to_owned())
        .id(egui::Id::unique(id))
        .open(open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
}

/// The History panel body (a dock tab)
pub(crate) fn history(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tokens = Tokens::get(ui.ctx());
    let (done, undone) = (app.session.history_labels(), app.session.redo_labels());

    // A click on a step goes back (or forward) to it: steps to undo, or to redo when negative
    let mut jump = 0i64;

    // Bottom up: the Undo and Redo row keeps its place under a list that fills the rest of the panel
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.horizontal(|ui| {
            if ui.add_enabled(app.session.can_undo(), egui::Button::new(format!("{} {}", ic::ARROW_COUNTER_CLOCKWISE, tr("Undo")))).clicked() {
                app.run("edit.undo", json!({}));
            }
            if ui.add_enabled(app.session.can_redo(), egui::Button::new(format!("{} {}", ic::ARROW_CLOCKWISE, tr("Redo")))).clicked() {
                app.run("edit.redo", json!({}));
            }
        });

        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let done_count = done.len() as i64;
                let rows = std::iter::once(RichText::new(tr("Project opened")).color(tokens.text_dim))
                    .chain(done.iter().map(|label| RichText::new(tr_history(label))))
                    .chain(undone.iter().map(|label| RichText::new(trf("{action} (undone)", &[("action", &tr_history(label))])).color(tokens.text_dim)));
                for (row, text) in rows.enumerate() {
                    // Each row is the state after that many steps
                    let steps = done_count - row as i64;
                    if ui.selectable_label(steps == 0, text).on_hover_text(tr("Return to this step")).clicked() {
                        // Point history is consumed before the document rows shown here
                        jump = if steps >= 0 {
                            app.session.history_undo_steps(steps as usize) as i64
                        } else {
                            -(app.session.history_redo_steps((-steps) as usize) as i64)
                        };
                    }
                }
            });
        });
    });

    if jump > 0 {
        app.run("edit.undo", json!({ "steps": jump }));
    } else if jump < 0 {
        app.run("edit.redo", json!({ "steps": -jump }));
    }
}

/// Browser File › Export Mesh: pick the download format
#[cfg(target_arch = "wasm32")]
fn export_mesh(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let formats = [
        ("glb", "glTF binary (.glb)", tr("Textures embedded; for game engines and most viewers")),
        ("usdz", "USDZ (.usdz)", tr("Textures packaged; for USD tools and AR Quick Look")),
        ("obj", "Wavefront OBJ (.obj)", tr("Geometry and UVs only")),
    ];
    if !app.ui.windows.export_mesh {
        return;
    }

    let mut open = true;
    let mut picked = None;
    dialog("export_mesh", tr("Export mesh"), &mut open).show(ctx, |ui| {
        ui.label(crate::theme::caption(tr("Download as")));
        for (extension, label, hint) in formats {
            if ui.add_sized(vec2(220.0, ui.spacing().interact_size.y), egui::Button::new(label)).on_hover_text(hint).clicked() {
                picked = Some(extension);
            }
        }
    });

    if let Some(extension) = picked {
        crate::web_files::export_mesh(app, extension);
        open = false;
    }
    app.ui.windows.export_mesh = open;
}

fn about(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let tk = Tokens::get(ctx);
    let mut open = app.ui.windows.about;
    dialog("about", tr("About TextureCraft"), &mut open).show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("TextureCraft").size(22.0).strong().color(tk.text_strong));
            ui.label(trf("Version {version}", &[("version", &texturecraft_engine::VERSION)]));
            ui.add_space(6.0);
            ui.label(tr("3D texture painting: layers, masks, smart materials, mesh map baking and export."));
            ui.label(tr("Free and open source (MIT OR Apache-2.0), written in Rust."));
            ui.add_space(6.0);
            ui.hyperlink_to(tr("Join the ArtCraft community on Discord"), "https://discord.gg/artcraft");
            ui.hyperlink_to("getartcraft.com/apps/texturecraft", "https://getartcraft.com/apps/texturecraft");
            ui.add_space(6.0);
            ui.label(RichText::new(tr("Made by the ArtCraft team and community.")).color(tk.text_dim));
        });
    });
    app.ui.windows.about = open;
}

/// Render the session log as text, preserving each entry's occurrence time, severity and source.
fn log_text(entries: &[texturecraft_engine::LogEntry]) -> String {
    use std::fmt::Write as _;

    const UTC_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.6fZ";
    let mut text = String::from("TextureCraft log, exported ");
    let _ = chrono::Utc::now().naive_utc().format(UTC_FORMAT).write_to(&mut text);
    text.push('\n');

    for entry in entries {
        let timestamp = entry.time.naive_utc().format(UTC_FORMAT);
        for line in entry.message.split('\n') {
            text.push('[');
            let _ = timestamp.write_to(&mut text);
            let _ = writeln!(text, "] {:5} [{}] {line}", entry.level.as_str(), entry.source);
        }
    }

    text
}

/// Help ▸ Export log...: saves the whole log to a text file chosen in a save dialog.
pub(crate) fn export_log(app: &mut TextureCraftApp) {
    let text = log_text(app.session.log.entries());
    #[cfg(not(target_arch = "wasm32"))]
    app.ask_file(FileDialog::save().filter(tr("Text file"), &["txt"]).file_name("texturecraft-log.txt"), Bound::App, move |app, paths| {
        let Some(path) = paths.first() else { return };
        match std::fs::write(path, &text) {
            Ok(()) => app.toast(trf("Exported the log to {path}", &[("path", &path.display().to_string())]), false),
            Err(e) => app.toast(trf("Could not export the log: {error}", &[("error", &e.to_string())]), true),
        }
    });
    #[cfg(target_arch = "wasm32")]
    match crate::web_files::download("texturecraft-log.txt", text.as_bytes()) {
        Ok(()) => app.toast(tr("Exported the log"), false),
        Err(e) => app.toast(trf("Could not export the log: {error}", &[("error", &e)]), true),
    }
}

// ---------------------------------------------------------------------------------------------
// Files (wired to texturecraft-formats once it lands)

pub fn file_dialogs_available() -> bool {
    cfg!(not(target_arch = "wasm32"))
}

/// Remember a project path in File ▸ Recent projects.
pub fn remember(app: &mut TextureCraftApp, path: &str) {
    app.ui.recent.retain(|p| p != path);
    app.ui.recent.insert(0, path.to_string());
    app.ui.recent.truncate(10);
}

/// Start a save picker in its library folder when available.
#[cfg(not(target_arch = "wasm32"))]
fn in_save_folder(app: &mut TextureCraftApp, dialog: FileDialog, dir: std::path::PathBuf) -> FileDialog {
    // Native pickers need an existing initial folder, but a different destination can still be writable
    match std::fs::create_dir_all(&dir) {
        Ok(()) => dialog.directory(dir),
        Err(error) => {
            app.session.log.warn(texturecraft_engine::log::LIBRARY_SOURCE, format!("Cannot open save folder {}: {error}", dir.display()));
            dialog
        }
    }
}

/// Window › Save UI Layout › Painting / Rendering / Baking mode: the panels, windows and view
/// settings as that mode shows them, as a JSON file.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_layout(app: &mut TextureCraftApp, mode: crate::state::Mode) {
    let dir = crate::user_dir("layouts");
    let name = match mode {
        Mode::Painting => "painting-mode.json",
        Mode::Rendering => "rendering-mode.json",
        Mode::Baking => "baking-mode.json",
    };
    let dialog = in_save_folder(app, FileDialog::save().file_name(name).filter(tr("UI layout"), &["json"]), dir);
    app.ask_file(dialog, Bound::App, move |app, paths| {
        let Some(path) = paths.first() else { return };

        // The file carries both docks; the mode's own is the one in use when it is loaded
        let mut layout = crate::UiState { recent: Vec::new(), named_layouts: Vec::new(), mode, ..app.ui.clone() };
        layout.sync_mode_dock();
        let result = serde_json::to_vec_pretty(&layout)
            .map_err(|error| error.to_string())
            .and_then(|bytes| texturecraft_engine::formats::write_atomic(path, &bytes).map_err(|error| error.to_string()))
            .map(|_| trf("Saved UI layout {path}", &[("path", &path.display())]));
        match result {
            Ok(msg) => app.toast(msg, false),
            Err(e) => app.toast(trf("UI layout: {error}", &[("error", &e)]), true),
        }
    });
}

/// Window › Load UI Layout: read a layout file saved by Save UI Layout.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_layout(app: &mut TextureCraftApp) {
    let dir = crate::user_dir("layouts");
    let dialog = FileDialog::open().title(tr("Select a UI layout file.")).filter(tr("UI layout"), &["json"]).directory(dir);
    app.ask_file(dialog, Bound::App, move |app, paths| {
        let Some(path) = paths.first() else { return };
        let result = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| serde_json::from_slice::<crate::UiState>(&bytes).map_err(|e| e.to_string()))
            .map(|layout| {
                app.ui.adopt_layout(layout);
                trf("Loaded UI layout {path}", &[("path", &path.display())])
            });
        match result {
            Ok(msg) => app.toast(msg, false),
            Err(e) => app.toast(trf("UI layout: {error}", &[("error", &e)]), true),
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
/// The template name a chosen file name stands for: only a trailing ".json" is dropped, so
/// "Metal v1.2" stays whole
pub(crate) fn template_name_of(file_name: &str) -> &str {
    match file_name.len().checked_sub(5) {
        Some(at) if file_name.is_char_boundary(at) && file_name[at..].eq_ignore_ascii_case(".json") => &file_name[..at],
        _ => file_name,
    }
}

/// File > More save options > Save as template...: the project's settings as a template in the
/// user templates folder. It saves without a confirmation and shows up in New project at once.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_as_template(app: &mut TextureCraftApp) {
    let dir = app.session.templates.dir.clone().unwrap_or_else(|| crate::user_dir("templates"));
    let dialog = in_save_folder(app, FileDialog::save().title(tr("Save template")).filter(tr("Project template"), &["json"]), dir);
    app.ask_file(dialog, Bound::Project, |app, paths| {
        let Some(path) = paths.first() else { return };

        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            app.toast(tr("The template file name must be valid text"), true);
            return;
        };

        let name = template_name_of(file_name);
        let path = if name == file_name { path.with_file_name(format!("{file_name}.json")) } else { path.clone() };
        if let Some(path) = path.to_str() {
            app.run("project.template.save", json!({"name": name, "path": path}));
        } else {
            app.toast(tr("The template path must be valid text"), true);
        }
    });
}

#[cfg(target_arch = "wasm32")]
pub fn save_as_template(app: &mut TextureCraftApp) {
    app.toast(tr("Saving templates to files is available in the desktop app"), false);
}

/// The browser has no file paths; layouts persist with the rest of the UI state.
#[cfg(target_arch = "wasm32")]
pub fn save_layout(app: &mut TextureCraftApp, _mode: crate::state::Mode) {
    app.toast(tr("Saving UI layouts to files is available in the desktop app"), false);
}

#[cfg(target_arch = "wasm32")]
pub fn load_layout(app: &mut TextureCraftApp) {
    app.toast(tr("Saving UI layouts to files is available in the desktop app"), false);
}

/// File menu actions that need dialogs. Returns true when handled.
#[cfg(not(target_arch = "wasm32"))]
pub fn file_action(app: &mut TextureCraftApp, _ctx: &egui::Context, id: &str) -> bool {
    match id {
        "file.open" => {
            app.ask_file(FileDialog::open().filter(tr("TextureCraft project or mesh"), crate::web_files::OPEN_EXTS), Bound::App, |app, paths| {
                if let Some(path) = paths.first() {
                    open_path(app, &path.display().to_string());
                }
            });
            true
        }
        "file.save" if app.session.path.is_some() => {
            if let Some(result) = app.run("file.save", json!({})) {
                let saved_path = result["path"].as_str().unwrap_or_default();
                app.toast(trf("Saved {path}", &[("path", &saved_path)]), false);
            }
            true
        }
        "file.save" | "file.saveAs" => {
            ask_save_project(app, "file.saveAs");
            true
        }
        "file.saveCopy" => {
            ask_save_project(app, "file.saveCopy");
            true
        }
        "material.load" => {
            app.ask_file(FileDialog::open_many().filter(tr("Materials"), crate::web_files::MATERIAL_EXTS), Bound::Project, |app, paths| {
                for path in paths {
                    app.run("material.load", json!({"path": path.display().to_string()}));
                }
            });
            true
        }
        "file.importResources" | "resource.import" => {
            crate::import_resources::open(app);
            true
        }
        "file.importProject" => {
            let ext = texturecraft_engine::import_cmds::PROJECT_EXT;
            app.ask_file(FileDialog::open().filter(tr("Project to convert"), &[ext]), Bound::App, |app, paths| {
                if let Some(p) = paths.first() {
                    open_path(app, &p.display().to_string());
                }
            });
            true
        }
        "file.importMaterialX" => {
            app.ask_file(FileDialog::open().filter(tr("MaterialX materials"), &["mtlx"]), Bound::Project, |app, paths| {
                if let Some(path) = paths.first()
                    && let Some(result) = app.run("file.importMaterialX", json!({"path": path.display().to_string()}))
                {
                    let name = result["material"].as_str().unwrap_or_default();
                    for warning in result["warnings"].as_array().into_iter().flatten().filter_map(serde_json::Value::as_str) {
                        app.session.log.warn(texturecraft_engine::log::LIBRARY_SOURCE, format!("MaterialX {name}: {warning}"));
                    }
                    app.toast(trf("Imported {name}", &[("name", &name)]), false);
                    app.ui.dock.focus(crate::docking::Panel::Layers);
                }
            });
            true
        }
        "file.importPbrFolder" => {
            crate::pbr_import::pick(app);
            true
        }
        "file.exportMesh" => {
            let name = format!("{}.obj", texturecraft_engine::formats::export::sanitize_file_stem(&app.session.doc.project.name));
            let dialog = FileDialog::save()
                .filter(tr("Wavefront OBJ with MTL, textures alongside"), &["obj"])
                .filter(tr("glTF binary with textures"), &["glb"])
                .filter(tr("USDZ with textures"), &["usdz"])
                .filter(tr("USD text, textures alongside"), &["usda"])
                .filter(tr("USD binary, textures alongside"), &["usdc"])
                .file_name(name);
            app.ask_file(dialog, Bound::Project, |app, paths| {
                if let Some(path) = paths.first()
                    && let Some(result) = app.run("file.exportMesh", json!({"path": path.display().to_string()}))
                {
                    let saved_path = result["path"].as_str().unwrap_or_default();
                    app.toast(trf("Exported mesh to {path}", &[("path", &saved_path)]), false);
                }
            });
            true
        }
        "file.reimportMesh" => {
            // Ask for a replacement only when the mesh source is missing
            if let Some(path) = app.session.remembered_mesh_path().map(str::to_string) {
                reimport_mesh(app, path);
                return true;
            }

            let dialog = FileDialog::open().filter(tr("3D meshes"), &texturecraft_engine::formats::mesh_import::MESH_EXTENSIONS);
            app.ask_file(dialog, Bound::Project, |app, paths| {
                if let Some(path) = paths.first() {
                    reimport_mesh(app, path.display().to_string());
                }
            });
            true
        }
        _ => false,
    }
}

/// Asks where to save the project and runs `cmd` with the answer. Each caller is its own dialog
/// request, so Save As and Save a Copy queue separately.
#[cfg(not(target_arch = "wasm32"))]
#[track_caller]
fn ask_save_project(app: &mut TextureCraftApp, cmd: &'static str) {
    let name = crate::web_files::project_file_name(app);
    let dialog = FileDialog::save().filter(tr("TextureCraft project"), &[texturecraft_engine::core::PROJECT_EXT]).file_name(name);
    // Save a Copy leaves the project unsaved, so it never answers the unsaved-changes prompt.
    let for_prompt = app.saving_for_prompt && cmd == "file.saveAs";
    let asked = app.ask_file(dialog, Bound::Project, move |app, paths| {
        let mut saved = false;
        if let Some(p) = paths.first() {
            let path = p.display().to_string();
            saved = app.run(cmd, json!({"path": path})).is_some();
            if saved {
                remember(app, &path);
                app.toast(trf("Saved {path}", &[("path", &path)]), false);
            }
        }
        // The answer ends the wait. A canceled or failed save leaves the prompt open for an answer.
        if for_prompt {
            app.saving_for_prompt = false;
            app.prompt_saved = saved && !app.session.modified;
        }
    });
    // A Save As already on its way answers for itself: the prompt's buttons stay usable.
    if for_prompt && !asked {
        app.saving_for_prompt = false;
    }
}

/// Reimport the mesh; painted strokes follow the surface when the new mesh's UVs differ.
#[cfg(not(target_arch = "wasm32"))]
fn reimport_mesh(app: &mut TextureCraftApp, path: String) {
    app.run("file.reimportMesh", json!({"path": path, "preserveStrokes": true}));
}

/// Open a project or mesh by path (menus, Recent projects, drag and drop).
#[cfg(not(target_arch = "wasm32"))]
pub fn open_path(app: &mut TextureCraftApp, path: &str) {
    if crate::import_ui::active(app) || ask_to_save(app, crate::state::Unsaved::Open(path.into())) {
        return;
    }
    // a project file of the reference app is converted, behind the import's notice
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == texturecraft_engine::import_cmds::PROJECT_EXT {
        crate::import_ui::ask(app, texturecraft_engine::import_cmds::ImportTask::path(path));
        return;
    }
    if app.run("file.open", json!({"path": path})).is_some() {
        remember(app, path);
        app.ui.mode = Mode::Painting;
    }
}

/// In the browser, Open and Import use the file picker and Save downloads the project.
#[cfg(target_arch = "wasm32")]
pub fn file_action(app: &mut TextureCraftApp, ctx: &egui::Context, id: &str) -> bool {
    match id {
        "file.open" => crate::web_files::open(ctx),
        "file.importProject" => crate::web_files::import_project(ctx),
        "file.importResources" | "resource.import" => crate::web_files::import_images(ctx),
        "material.load" => crate::web_files::import_material(ctx),
        "file.save" | "file.saveAs" | "file.saveCopy" => crate::web_files::save_project(app),
        "file.exportMesh" => app.ui.windows.export_mesh = true,
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_template_name_keeps_dots_and_loses_only_json() {
        assert_eq!(super::template_name_of("Metal v1.2.json"), "Metal v1.2");
        assert_eq!(super::template_name_of("Metal v1.2"), "Metal v1.2");
        assert_eq!(super::template_name_of("wood.2k.JSON"), "wood.2k");
        assert_eq!(super::template_name_of("plain"), "plain");
    }

    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};

    use super::*;
    use crate::state::Unsaved;

    fn harness() -> Harness<'static, TextureCraftApp> {
        let session = texturecraft_engine::sample_session(256, 64).unwrap();
        let mut h = Harness::builder().with_size(vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));

        // Match English labels independently of the system locale
        h.state_mut().ui.prefs.language = "en".into();
        h.run_steps(2);
        h
    }

    #[test]
    fn feedback_export_scroll_reaches_the_last_set_with_the_footer_visible() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut harness = harness();
        let project = &mut harness.state_mut().session.doc.project;
        for index in 1..40 {
            project.texture_sets.push(texturecraft_engine::core::TextureSet::new(format!("Export set {index}"), index, 32));
        }
        let last_index = project.texture_sets.len() - 1;
        harness.state_mut().ui.windows.export = true;
        harness.run_steps(5);
        let footer = harness.get_by_role_and_label(egui::Role::Button, "Reset all").rect();
        let first = harness.get_by_label("Export set 1");
        first.hover();
        harness.run_steps(2);

        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -2000.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
        });
        harness.run_steps(15);
        let last = harness.get_by_label("Export set 39");
        last.click();
        harness.run_steps(4);

        assert_eq!(harness.state().ui.export.page, Some(last_index), "the last texture set is clickable after scrolling");
        let footer_after = harness.get_by_role_and_label(egui::Role::Button, "Reset all").rect();
        assert_eq!(footer_after, footer, "scrolling keeps the footer in place");
        assert!(footer_after.bottom() < 960.0, "the footer is inside the window");

        if let Some(directory) = std::env::var_os("TC_UI_SNAPSHOTS") {
            let path = std::path::PathBuf::from(directory).join("export-texture-set-scroll.png");
            harness.render().unwrap().save(path).unwrap();
        }
    }

    #[test]
    fn a_canceled_save_dialog_does_not_answer_the_prompt() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut h = harness();
        let app = h.state_mut();
        app.file_dialogs.set_override(Some(Box::new(|_| crate::file_dialogs::Answer::Cancelled)));
        app.ui.windows.unsaved = Some(Unsaved::Quit);
        // Saved elsewhere, by a script, while the dialog was open.
        app.session.modified = false;
        app.saving_for_prompt = true;
        ask_save_project(app, "file.saveAs");
        h.run_steps(3);
        assert_eq!(h.state().ui.windows.unsaved, Some(Unsaved::Quit), "the prompt still waits for an answer");
    }

    #[test]
    fn the_prompts_buttons_wait_for_its_save_dialog() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut h = harness();
        let app = h.state_mut();
        app.ui.windows.unsaved = Some(Unsaved::Quit);
        app.session.modified = true;
        h.run_steps(2);
        for label in ["Save", "Don't save", "Cancel"] {
            assert!(!h.get_by_label(label).accesskit_node().is_disabled(), "{label} answers while no dialog is open");
        }
        h.state_mut().saving_for_prompt = true;
        h.run_steps(2);
        for label in ["Save", "Don't save", "Cancel"] {
            assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{label} waits for the Save dialog");
        }
    }

    #[test]
    fn a_save_a_copy_answer_does_not_end_the_prompts_wait() {
        // The harness draws the UI, and the i18n tests switch the process-wide language to ones its fonts cannot draw
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut h = harness();
        let app = h.state_mut();
        app.file_dialogs.set_override(Some(Box::new(|_| crate::file_dialogs::Answer::Cancelled)));
        app.saving_for_prompt = true;
        ask_save_project(app, "file.saveCopy");
        h.run_steps(3);
        assert!(h.state().saving_for_prompt, "only the prompt's own Save As ends the wait");
    }

    #[test]
    fn a_save_as_already_on_its_way_ends_the_prompts_wait() {
        // The harness draws the UI, and the i18n tests switch the process-wide language to ones its fonts cannot draw
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut h = harness();
        let app = h.state_mut();

        // One call site, as File › Save As and the prompt's Save share one
        for waiting in [false, true] {
            app.saving_for_prompt = waiting;
            ask_save_project(app, "file.saveAs");
        }
        assert_eq!(app.file_dialogs.pending(), 1);
        assert!(!app.saving_for_prompt, "the prompt's buttons work again");
    }

    /// A change that lands after the Save dialog saved (a bake finishing that frame) keeps the
    /// question open with buttons that still answer.
    #[test]
    fn a_change_after_the_dialog_saved_leaves_the_prompt_usable() {
        // The harness keeps the process-wide language at English while this asserts on labels
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut h = harness();
        let app = h.state_mut();
        app.ui.windows.unsaved = Some(Unsaved::Quit);
        app.session.modified = true;
        app.prompt_saved = true;
        h.run_steps(2);
        assert_eq!(h.state().ui.windows.unsaved, Some(Unsaved::Quit), "the project has unsaved changes again");
        assert!(!h.state().saving_for_prompt);
        h.get_by_label("Cancel").click();
        h.run_steps(2);
        assert!(h.state().ui.windows.unsaved.is_none());
    }

    /// The History panel runs each step name through the translator (skipped when this build
    /// embeds no catalog that translates "Remove layer"; `i18n` tests cover the template matching)
    #[test]
    fn history_rows_are_shown_in_the_interface_language() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(code) = crate::i18n::LANGUAGES.iter().map(|l| l.code).find(|code| {
            crate::i18n::set_language(code);
            *code != "en" && crate::i18n::tr("Remove layer") != "Remove layer"
        }) else {
            crate::i18n::set_language("en");
            return;
        };

        let translated = crate::i18n::tr("Remove layer").to_string();
        let mut h = harness();
        {
            let app = h.state_mut();
            let layer = app.session.selected_layer().unwrap();
            texturecraft_engine::execute(&mut app.session, "layer.remove", &json!({"layer": layer.0})).unwrap();
            app.ui.dock.focus(crate::docking::Panel::History);
            app.ui.prefs.language = code.into();
        }
        h.run_steps(4);
        let shown = h.query_all_by_label(&translated).count();
        let untranslated = h.query_all_by_label("Remove layer").count();
        h.state_mut().ui.prefs.language = "en".into();
        h.run_steps(4);
        assert!(shown > 0 && untranslated == 0, "{code}: translated rows {shown}, English rows {untranslated}");
        assert!(h.query_all_by_label("Remove layer").count() > 0);
    }
}

#[cfg(test)]
mod log_export_tests {
    use super::*;

    #[test]
    fn log_text_keeps_distinct_recorded_times_and_levels() {
        use texturecraft_engine::{LogEntry, LogLevel};

        let entries = [
            LogEntry {
                time: chrono::DateTime::from_timestamp(1_709_164_799, 0).unwrap(),
                source: "Importer".into(),
                level: LogLevel::Info,
                message: "Loaded".into(),
            },
            LogEntry {
                time: chrono::DateTime::from_timestamp(1_709_164_800, 0).unwrap(),
                source: "Library agent".into(),
                level: LogLevel::Warning,
                message: "Skipped".into(),
            },
        ];

        let text = log_text(&entries);

        assert_eq!(
            text.lines().skip(1).collect::<Vec<_>>(),
            ["[2024-02-28T23:59:59.000000Z] INFO  [Importer] Loaded", "[2024-02-29T00:00:00.000000Z] WARN  [Library agent] Skipped"]
        );
    }
}
