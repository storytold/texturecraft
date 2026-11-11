//! The shelf's type row: one icon per type (none picked shows every type), greyed without matches, one dropdown when narrow

use egui::{Ui, vec2};

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::shelf::{TABS, Tab};
use crate::widgets::{icon_button, icon_dropdown_button};

/// The gap between the row's icons in points
const ICON_GAP: f32 = 2.0;
/// The size of the dot that marks the collapsed row's dropdown while a type is picked, in points
const FILTER_DOT_RADIUS: f32 = 2.5;

/// `app.ui.shelf_tab` while no type is picked: every type shows
pub(crate) const NO_TYPE: usize = TABS.len();

/// The row, then `trailing` at its right end; `counts[i]` is how many assets of `TABS[i]` match the search
pub(crate) fn row(app: &mut TextureCraftApp, ui: &mut Ui, counts: &[usize], trailing: impl FnOnce(&mut TextureCraftApp, &mut Ui, f32)) {
    let size = ui.spacing().interact_size.y;
    // The type icons and one icon after them
    let row_width = (TABS.len() + 1) as f32 * (size + ICON_GAP);
    let mut picked = app.ui.shelf_tab.min(NO_TYPE);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(ICON_GAP, ICON_GAP);
        if ui.available_width() >= row_width {
            for (i, (_, icon, label)) in TABS.iter().enumerate() {
                // The picked type stays clickable so it can be unpicked
                let empty = i != picked && counts.get(i).is_none_or(|count| *count == 0);
                if ui.add_enabled_ui(!empty, |ui| icon_button(ui, icon, tr(label), i == picked, size)).inner.clicked() {
                    picked = if i == picked { NO_TYPE } else { i };
                }
            }
        } else {
            collapsed(ui, &mut picked, counts, size);
        }

        let rest = ui.available_width() - size;
        ui.add_space(rest.max(0.0));
        trailing(app, ui, size);
    });
    app.ui.shelf_tab = picked;
}

/// One dropdown listing the types, dotted while one is picked
fn collapsed(ui: &mut Ui, picked: &mut usize, counts: &[usize], size: f32) {
    let filtering = *picked != NO_TYPE;
    let response = icon_dropdown_button(ui, egui_phosphor::regular::FUNNEL_SIMPLE, tr("Filter by type"), filtering, size);
    let response = response.on_hover_text(tr("Filter by type"));
    if filtering {
        let tokens = crate::theme::Tokens::get(ui.ctx());
        let dot = response.rect.left_top() + vec2(size - FILTER_DOT_RADIUS, FILTER_DOT_RADIUS);
        ui.painter().circle_filled(dot, FILTER_DOT_RADIUS, tokens.accent);
    }

    egui::Popup::menu(&response).show(|ui| {
        for (i, (_, icon, label)) in TABS.iter().enumerate() {
            let empty = i != *picked && counts.get(i).is_none_or(|count| *count == 0);
            if ui.add_enabled(!empty, egui::Button::selectable(i == *picked, format!("{icon}  {}", tr(label)))).clicked() {
                *picked = if i == *picked { NO_TYPE } else { i };
                ui.close();
            }
        }
    });
}

/// Whether `tab` (an index into [`TABS`], or [`NO_TYPE`]) shows assets of type `kind`
pub(crate) fn shows(tab: usize, kind: Tab) -> bool {
    TABS.get(tab).is_none_or(|(shown, _, _)| *shown == kind)
}
