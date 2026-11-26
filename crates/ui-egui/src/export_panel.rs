//! The Export panel on the rail: Export Mesh and Export Textures

use egui::Ui;

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::theme::{Tokens, caption};
use crate::widgets::primary_button;

/// Space around each button's text, so the buttons are as wide as their text as in the reference
const EXPORT_BUTTON_PADDING: egui::Vec2 = egui::vec2(18.0, 5.0);

/// Gap between the Export Mesh and Export Textures buttons
const EXPORT_BUTTON_GAP: f32 = 12.0;

/// The Export panel body (a rail popup or dock tab); its buttons need an open project
pub(crate) fn body(app: &mut TextureCraftApp, ui: &mut Ui) {
    ui.label(caption(&tr("Export").to_uppercase()));
    let project_open = app.session.project_open;
    let tokens = Tokens::get(ui.ctx());
    ui.vertical_centered(|ui| {
        ui.spacing_mut().button_padding = EXPORT_BUTTON_PADDING;
        ui.spacing_mut().item_spacing.y = EXPORT_BUTTON_GAP;

        // Fully rounded: Export Mesh outlined, Export Textures filled
        let pill = ui.spacing().interact_size.y + 2.0 * EXPORT_BUTTON_PADDING.y;
        let outlined = egui::Button::new(tr("Export Mesh...")).fill(egui::Color32::TRANSPARENT).stroke(egui::Stroke::new(1.0, tokens.button_border));
        if ui.add_enabled(project_open, outlined.corner_radius(pill / 2.0)).clicked() {
            app.ui.windows.mesh_export_options = Some(app.session.project_id);
            app.ui.rail_panel = None;
        }
        if ui.add_enabled(project_open, primary_button(ui, tr("Export Textures...")).corner_radius(pill / 2.0)).clicked() {
            app.ui.windows.export = true;
            app.ui.rail_panel = None;
        }
    });
}
