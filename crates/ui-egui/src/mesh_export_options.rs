//! File > Export Mesh asks for the mesh export options first; Export then leads to the file choice

use egui::Ui;

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::theme::Tokens;
use crate::unavailable;

/// Indent of an option's checkbox under its choice
const OPTION_INDENT: f32 = 20.0;

/// Width of the Cancel and Export buttons
const BUTTON_WIDTH: f32 = 110.0;

/// The reference dialog's content width
const DIALOG_WIDTH: f32 = 500.0;

/// Width of an info icon's tooltip, as in the reference
const INFO_TOOLTIP_WIDTH: f32 = 250.0;

/// Gap between the two option groups, as in the reference
const GROUP_GAP: f32 = 25.0;

/// The prompt, while File > Export Mesh waits on it
pub(crate) fn show(app: &mut TextureCraftApp, ctx: &egui::Context) {
    if app.ui.windows.mesh_export_options != Some(app.session.project_id) {
        app.ui.windows.mesh_export_options = None;
        return;
    }

    let mut open = true;
    let (mut export, mut cancel) = (false, false);
    crate::windows::dialog(ctx, "mesh_export_options", tr("Mesh Export Options"), &mut open).show(ctx, |ui| {
        ui.set_width(DIALOG_WIDTH);

        // TextureCraft exports the mesh as it is: triangulated, without displacement
        ui.add(egui::RadioButton::new(true, tr("Without displacement/tessellation")));
        option(
            ui,
            true,
            tr("Apply triangulation"),
            &[
                tr("Splits every face with more than three corners into triangles; triangles stay as they are."),
                tr("TextureCraft keeps meshes as triangles, so the exported mesh is always triangulated."),
            ],
        );
        ui.add_space(GROUP_GAP);
        unavailable::greyed(ui, |ui| ui.add(egui::RadioButton::new(false, tr("With displacement/tessellation"))));
        option(
            ui,
            true,
            tr("Recompute vertex normals"),
            &[
                tr("Replaces the mesh's vertex normals with newly computed ones in the exported file."),
                tr("TextureCraft cannot do this yet: it writes the vertex normals the mesh has in the project."),
            ],
        );
        ui.add_space(8.0);

        // One row keeps the window to its contents; a bare right-to-left layout fills the screen height
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let size = egui::vec2(BUTTON_WIDTH, ui.spacing().interact_size.y);
                export = ui.add(egui::Button::new(tr("Export")).min_size(size)).clicked();
                cancel = ui.add(egui::Button::new(tr("Cancel")).min_size(size)).clicked();
            });
        });
    });

    if export {
        app.ui.windows.mesh_export_options = None;
        crate::windows::file_action(app, ctx, "file.exportMesh");
    } else if cancel || !open {
        app.ui.windows.mesh_export_options = None;
    }
}

/// A greyed option under a choice, ticked as the reference shows it, with an info icon at the
/// row's right end whose tooltip has the paragraphs `info`
fn option(ui: &mut Ui, checked: bool, label: &str, info: &[&str]) {
    ui.horizontal(|ui| {
        ui.add_space(OPTION_INDENT);
        unavailable::checkbox(ui, checked, label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let icon = ui.label(egui::RichText::new(egui_phosphor::regular::INFO).color(Tokens::get(ui.ctx()).text_dim));
            icon.on_hover_ui(|ui| {
                ui.set_max_width(INFO_TOOLTIP_WIDTH);
                for (k, paragraph) in info.iter().enumerate() {
                    if k > 0 {
                        ui.add_space(ui.text_style_height(&egui::TextStyle::Body));
                    }
                    ui.label(*paragraph);
                }
            });
        });
    });
}
