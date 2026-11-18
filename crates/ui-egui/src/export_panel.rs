//! The Export panel on the rail: where to send the project, then Export Mesh and Export Textures

use egui::{RichText, Ui};
use egui_phosphor::regular as ic;

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::theme::{Tokens, caption};
use crate::unavailable;
use crate::widgets::primary_button;

/// The SEND TO rows: none has a counterpart in TextureCraft yet
pub const SEND_TO: [&str; 3] = ["Export to image editor", "Send to compositing app", "Send to 3D staging app"];

/// Width of the Export Mesh and Export Textures buttons
const EXPORT_BUTTON_WIDTH: f32 = 176.0;

/// Size of the Configure button beside a SEND TO row
const CONFIGURE_SIZE: f32 = 20.0;

/// Space between the SEND TO and EXPORT parts
const PART_GAP: f32 = 12.0;

/// The Export panel body (a rail popup or dock tab); its buttons need an open project
pub(crate) fn body(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tokens = Tokens::get(ui.ctx());
    ui.label(caption(&tr("Send to").to_uppercase()));
    for (index, target) in SEND_TO.into_iter().enumerate() {
        ui.horizontal(|ui| {
            unavailable::greyed(ui, |ui| ui.add(egui::Button::new(tr(target)).frame(false)));

            // The reference's second row has a Configure button
            if index == 1 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    unavailable::greyed(ui, |ui| {
                        ui.add(
                            egui::Button::new(RichText::new(ic::GEAR).color(tokens.text_dim)).frame(false).min_size(egui::vec2(CONFIGURE_SIZE, CONFIGURE_SIZE)),
                        )
                    });
                });
            }
        });
    }

    ui.add_space(PART_GAP);
    ui.label(caption(&tr("Export").to_uppercase()));
    let project_open = app.session.project_open;
    ui.vertical_centered(|ui| {
        let size = [EXPORT_BUTTON_WIDTH, ui.spacing().interact_size.y];
        if ui.add_enabled(project_open, egui::Button::new(tr("Export Mesh...")).min_size(size.into())).clicked() {
            app.ui.windows.mesh_export_options = true;
            app.ui.rail_panel = None;
        }
        if ui.add_enabled(project_open, primary_button(ui, tr("Export Textures...")).min_size(size.into())).clicked() {
            app.ui.windows.export = true;
            app.ui.rail_panel = None;
        }
    });
}
