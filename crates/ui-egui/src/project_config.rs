//! Edit > Project Configuration...: the project's settings in sections. Today it has one, the
//! 3D mesh: pick another mesh file for the project, or read the current one again.

use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Key, Modifiers};
use serde_json::{Value, json};
use texturecraft_engine::core::NormalFormat;

use crate::TextureCraftApp;
use crate::dock::info_icon;
use crate::file_dialogs::{Bound, FileDialog};
use crate::i18n::{tr, trf};
use crate::state::ProjectConfig;
use crate::theme::caption;
use crate::widgets::primary_button;

/// Room for the Select... button and its info icon beside the path.
const SELECT_BUTTON_WIDTH_POINTS: f32 = 150.0;
/// Room for an info icon at the end of a row.
const INFO_WIDTH_POINTS: f32 = 28.0;
/// Keep a usable field when the dialog is narrowed.
const MIN_FIELD_WIDTH_POINTS: f32 = 60.0;
/// Initial width of the configuration dialog.
const DEFAULT_DIALOG_WIDTH_POINTS: f32 = 460.0;
/// Leave space around the centered dialog on short viewports.
const DIALOG_VIEWPORT_MARGIN_POINTS: f32 = 32.0;

/// Opens the window on the current project, with nothing chosen yet.
pub(crate) fn open(app: &mut TextureCraftApp) {
    // Layout resets must not reuse an identity held by a pending file picker
    static NEXT_OPENING_ID: AtomicU64 = AtomicU64::new(1);

    app.ui.windows.project_config = Some(ProjectConfig {
        mesh: None,
        reimport: false,
        preserve: true,
        normal_format: None,
        confirm: false,
        project: app.session.project_id,
        opening_id: NEXT_OPENING_ID.fetch_add(1, Ordering::Relaxed),
    });
}

/// `text` with its beginning replaced by an ellipsis until `fits` accepts it (the end of a path
/// is the part that tells files apart).
fn elide_start(text: &str, fits: impl Fn(&str) -> bool) -> String {
    if fits(text) {
        return text.to_string();
    }

    let chars: Vec<char> = text.chars().collect();
    let shown = |from: usize| format!("…{}", chars[from..].iter().collect::<String>());

    // Bisection finds the longest tail that fits because each shorter tail also fits
    let (mut low, mut high) = (1, chars.len());
    while low < high {
        let mid = low + (high - low) / 2;
        if fits(&shown(mid)) {
            high = mid;
        } else {
            low = mid + 1;
        }
    }

    shown(low)
}

/// The keys of the open window, read in `menus::shortcuts` before any panel draws: Escape closes
/// it (and goes no further, so it doesn't cancel a stroke), Enter confirms it unless a text field
/// or button had the keyboard (`nothing_focused`), whose Enter belongs to it. An open dropdown
/// (`popup_open`) takes both keys first: Escape closes only it, Enter doesn't press OK behind it.
pub(crate) fn keys_in(app: &mut TextureCraftApp, input: &mut egui::InputState, nothing_focused: bool, popup_open: bool) {
    if popup_open {
        return;
    }

    let escape = input.consume_key(Modifiers::NONE, Key::Escape);
    let enter = input.consume_key(Modifiers::NONE, Key::Enter);

    if escape {
        app.ui.windows.project_config = None;
    } else if enter
        && nothing_focused
        && let Some(config) = app.ui.windows.project_config.as_mut()
    {
        config.confirm = true;
    }
}

/// The command OK runs for the mesh: the project moves to `chosen`, or `reimport` (the project's
/// own file, when Reimport mesh is ticked) is read again. `preserve` is the Import settings box.
fn mesh_command(chosen: Option<&str>, reimport: Option<&str>, preserve: bool) -> Option<(&'static str, Value)> {
    match (chosen, reimport) {
        (Some(path), _) => Some(("file.changeMesh", json!({"path": path, "preserveStrokes": preserve}))),
        (None, Some(path)) => Some(("file.reimportMesh", json!({"path": path, "preserveStrokes": preserve}))),
        (None, None) => None,
    }
}

fn format_name(format: NormalFormat) -> &'static str {
    match format {
        NormalFormat::OpenGl => "OpenGL",
        NormalFormat::DirectX => "DirectX",
    }
}

pub(crate) fn show(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let Some(mut config) = app.ui.windows.project_config.clone() else { return };

    // Choices for another project cannot be applied here
    if config.project != app.session.project_id {
        app.ui.windows.project_config = None;
        return;
    }

    let project = &app.session.doc.project;
    let current_mesh_path = project.mesh_path.clone();

    // Show the imported name when there is no file path
    let mesh_label = if current_mesh_path.is_empty() { project.mesh_file.clone() } else { current_mesh_path.clone() };
    let source_file_available = app.session.remembered_mesh_path().is_some();
    let current_normal_format = project.normal_format;

    // Keep each texture set's tiles together for the read-only list
    let texture_set_tiles: Vec<(String, Option<String>)> = project
        .texture_sets
        .iter()
        .map(|set| (set.name.clone(), (!set.uv_tiles.is_empty()).then(|| set.uv_tiles.iter().map(u32::to_string).collect::<Vec<_>>().join(", "))))
        .collect();

    // Re-selecting the current path cancels an earlier choice
    let chosen_mesh = config.mesh.as_ref().filter(|chosen| **chosen != current_mesh_path).cloned();

    let mut open = true;
    let (mut ok, mut cancel, mut select) = (std::mem::take(&mut config.confirm), false, false);

    let dialog = crate::windows::dialog("project_config", tr("Project configuration"), &mut open)
        .default_width(DEFAULT_DIALOG_WIDTH_POINTS)
        .max_height((ctx.content_rect().height() - DIALOG_VIEWPORT_MARGIN_POINTS).max(0.0))
        .vscroll(true);

    dialog.show(ctx, |ui| {
        // Keep the project sections in the order users expect
        ui.label(caption(tr("3D MESH")));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(tr("3D mesh"));
            info_icon(ui, tr("The mesh file the project is built on. Hover the path to see it in full."));

            let display_path = chosen_mesh.clone().unwrap_or_else(|| mesh_label.clone());
            let font = egui::TextStyle::Body.resolve(ui.style());
            let path_width_points = (ui.available_width() - SELECT_BUTTON_WIDTH_POINTS).max(MIN_FIELD_WIDTH_POINTS);
            let measure = |text: &str| ui.painter().layout_no_wrap(text.to_string(), font.clone(), egui::Color32::WHITE).rect.width() <= path_width_points;

            if display_path.is_empty() {
                ui.weak(tr("Built-in mesh"));
            } else {
                ui.label(elide_start(&display_path, measure)).on_hover_text(&display_path);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                info_icon(ui, tr("Choose another mesh file for the project. It is read when you press OK."));
                select = ui.button(tr("Select...")).clicked();
            });
        });
        if chosen_mesh.is_none() && !current_mesh_path.is_empty() && !source_file_available {
            ui.colored_label(ui.visuals().error_fg_color, tr("File not found"));
        }

        ui.add_space(4.0);

        // A selected replacement always reads the mesh again
        // A missing source file cannot be reimported
        let mut reimport = config.reimport || chosen_mesh.is_some();
        let can_reimport_source = chosen_mesh.is_none() && source_file_available;
        let reimport_checkbox = ui
            .horizontal(|ui| {
                let checkbox = ui.add_enabled(can_reimport_source, egui::Checkbox::new(&mut reimport, tr("Reimport mesh")));
                info_icon(ui, tr("Read the mesh file again, keeping the layers and painted strokes"));
                checkbox
            })
            .inner;

        if can_reimport_source {
            config.reimport = reimport;
        } else if chosen_mesh.is_none() {
            reimport_checkbox.on_disabled_hover_text(tr("There is no mesh file to read again. Select one."));
        }

        ui.add_space(8.0);
        ui.label(caption(tr("Import settings")));

        // Import settings matter only when a mesh will be read
        let importing = chosen_mesh.is_some() || (config.reimport && source_file_available);
        ui.horizontal(|ui| {
            ui.add_enabled(importing, egui::Checkbox::new(&mut config.preserve, tr("Preserve stroke positions on mesh")));
            info_icon(ui, tr("Painted strokes follow the surface when the new mesh's UVs differ"));
        });

        ui.add_space(12.0);
        ui.label(caption(tr("PROJECT SETTINGS")));
        ui.add_space(4.0);
        let shown_format = config.normal_format.unwrap_or(current_normal_format);
        ui.horizontal(|ui| {
            ui.label(tr("Normal map format"));
            egui::ComboBox::from_id_salt(tr("Normal map format"))
                .width((ui.available_width() - INFO_WIDTH_POINTS).max(MIN_FIELD_WIDTH_POINTS))
                .wrap_mode(egui::TextWrapMode::Truncate)
                .selected_text(format_name(shown_format))
                .show_ui(ui, |ui| {
                    for format in [NormalFormat::OpenGl, NormalFormat::DirectX] {
                        if ui.selectable_label(format == shown_format, format_name(format)).clicked() {
                            config.normal_format = (format != current_normal_format).then_some(format);
                        }
                    }
                })
                .response
                .on_hover_text(tr(
                    "For the whole project: how normal map images set as mesh maps are read (set them again after a change), and the default for export",
                ));
            info_icon(ui, tr("Green channel convention of normal maps: OpenGL (Y+) or DirectX (Y-). Imported and exported normal maps follow it."));
        });
        ui.add_space(4.0);

        // Texture Set Settings owns UV tile changes
        ui.horizontal(|ui| {
            ui.label(tr("UV Tile settings (UDIMs)"));
            info_icon(ui, tr("The UV tiles (UDIMs) of each texture set, shown for reference. Change them in Texture Set Settings."));
        });
        if texture_set_tiles.iter().all(|(_, tile_text)| tile_text.is_none()) {
            ui.label(egui::RichText::new(tr("No UV tiles")).weak());
        }
        for (name, tile_text) in texture_set_tiles.iter().filter(|(_, tile_text)| tile_text.is_some()) {
            let tile_list = tile_text.as_deref().unwrap_or_default();
            ui.label(
                egui::RichText::new(if texture_set_tiles.len() > 1 {
                    trf("{set}: {tiles}", &[("set", name), ("tiles", &tile_list)])
                } else {
                    tile_list.to_string()
                })
                .weak(),
            );
        }
        ui.label(egui::RichText::new(tr("Change them in Texture Set Settings")).small().weak());

        ui.add_space(12.0);
        ui.label(egui::RichText::new(tr("Choosing or reimporting a mesh recomputes all channels.")).small().weak());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ok |= ui.add(primary_button(ui, tr("OK"))).clicked();
                cancel = ui.button(tr("Cancel")).clicked();
            });
        });
    });

    if select {
        let dialog = FileDialog::open().filter(tr("3D meshes"), &texturecraft_engine::formats::mesh_import::MESH_EXTENSIONS);
        let opening_id = config.opening_id;
        app.ask_file(dialog, Bound::Project, move |app, paths| {
            let Some(path) = paths.first().map(|p| p.display().to_string()) else { return };

            let current_mesh_path = app.session.doc.project.mesh_path.clone();
            if let Some(config) = app.ui.windows.project_config.as_mut().filter(|config| config.opening_id == opening_id) {
                // Re-selecting the source file drops an earlier choice
                config.mesh = (path != current_mesh_path).then_some(path);
            }
        });
    }

    // Apply the mesh first so a failed mesh change leaves the format untouched
    let mut mesh_applied = false;
    let done = ok && {
        let reimport = config.reimport.then(|| app.session.remembered_mesh_path().map(str::to_string)).flatten();
        mesh_applied = if config.reimport && chosen_mesh.is_none() && reimport.is_none() {
            // Drop a stale reimport tick so the user can retry the remaining settings
            app.toast(tr("File not found"), true);
            config.reimport = false;
            false
        } else {
            match mesh_command(chosen_mesh.as_deref(), reimport.as_deref(), config.preserve) {
                Some((command, params)) => app.run(command, params).is_some(),
                None => true,
            }
        };

        // Another panel may have set the same format while this dialog was open
        let format = config.normal_format.filter(|format| *format != app.session.doc.project.normal_format);
        mesh_applied
            && match format {
                Some(format) => {
                    app.run("textureSet.settings", json!({"normalFormat": if format == NormalFormat::DirectX { "directX" } else { "openGl" }})).is_some()
                }
                None => true,
            }
    };

    if done || cancel || !open {
        app.ui.windows.project_config = None;
    } else if let Some(stored) = app.ui.windows.project_config.as_mut() {
        // A file answer arrives later through the stored state
        (stored.reimport, stored.preserve, stored.normal_format, stored.confirm) = (config.reimport, config.preserve, config.normal_format, false);

        // A format retry must not reimport an already changed mesh
        if ok && mesh_applied {
            (stored.mesh, stored.reimport) = (None, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{elide_start, mesh_command};

    #[test]
    fn a_path_that_fits_is_unchanged() {
        assert_eq!(elide_start("/a/b.obj", |s| s.chars().count() <= 20), "/a/b.obj");
    }

    #[test]
    fn a_long_path_loses_its_beginning() {
        let shown = elide_start("/home/me/projects/models/hero.obj", |s| s.chars().count() <= 12);

        assert_eq!(shown, "…ls/hero.obj", "the longest tail that fits, not a shorter one");
    }

    #[test]
    fn the_import_setting_goes_to_the_command() {
        for preserve in [true, false] {
            let (command, params) = mesh_command(Some("/a/b.obj"), None, preserve).unwrap();

            assert_eq!((command, params["preserveStrokes"].as_bool()), ("file.changeMesh", Some(preserve)));

            let (command, params) = mesh_command(None, Some("/a/c.obj"), preserve).unwrap();

            assert_eq!((command, params["preserveStrokes"].as_bool(), params["path"].as_str()), ("file.reimportMesh", Some(preserve), Some("/a/c.obj")));
        }

        assert!(mesh_command(None, None, true).is_none(), "nothing to read");
    }
}
