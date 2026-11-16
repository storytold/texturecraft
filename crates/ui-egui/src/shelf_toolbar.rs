//! The shelf's bottom toolbar: the side pane toggles on the left; refresh, reset query and import on the right

use egui::Ui;
use egui_phosphor::regular as ic;

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::widgets::icon_button;

/// The gap above the toolbar in points
const GAP_ABOVE: f32 = 4.0;

/// The toolbar's height for a `ui` (one icon button and the gap above it)
pub(crate) fn height(ui: &Ui) -> f32 {
    ui.spacing().interact_size.y + GAP_ABOVE
}

/// The toolbar
pub(crate) fn show(app: &mut TextureCraftApp, ui: &mut Ui) {
    let size = ui.spacing().interact_size.y;
    ui.add_space(GAP_ABOVE);
    ui.horizontal(|ui| {
        crate::shelf_panes::toggles(app, ui, size);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button(ui, ic::PLUS, tr("Import resources..."), false, size).clicked() {
                crate::menus::invoke(app, &ui.ctx().clone(), "resource.import");
            }
            if icon_button(ui, ic::X_CIRCLE, tr("Reset query"), false, size).clicked() {
                reset_query(app);
            }
            if icon_button(ui, ic::ARROWS_CLOCKWISE, tr("Refresh"), false, size).clicked() {
                refresh(app);
            }
        });
    });
}

/// Show every asset again: no search, no type picked, all libraries
pub(crate) fn reset_query(app: &mut TextureCraftApp) {
    app.ui.shelf_filter.clear();
    app.ui.shelf_tab = crate::shelf_tabs::NO_TYPE;
    app.ui.shelf_path.clear();
}

/// Read the user's saved assets from disk again and redraw their thumbnails
fn refresh(app: &mut TextureCraftApp) {
    app.library = None;
    app.brushes = None;
    app.thumbs.forget_saved_files();
}
