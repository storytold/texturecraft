//! The shelf's type row: one icon per type (none picked shows every type), greyed without matches, one dropdown when narrow

use egui::{Ui, vec2};

use crate::TextureCraftApp;
use crate::i18n::{n_, tr};
use crate::shelf::{TABS, Tab};
use crate::widgets::{icon_button, icon_dropdown_button};

/// The gap between the row's icons in points
const ICON_GAP_POINTS: f32 = 2.0;
/// The size of the dot that marks the collapsed row's dropdown while a type is picked, in points
const FILTER_DOT_RADIUS_POINTS: f32 = 2.5;

/// The dropdown's entry after the types, greyed until fonts are assets
const FONTS: &str = n_("Fonts");
/// The dropdown's last entry: no type picked
const SHOW_ALL: &str = n_("Show all");

/// `app.ui.shelf_tab` while no type is picked: every type shows
pub(crate) const NO_TYPE: usize = TABS.len();

/// The row, then `trailing` at its right end; `counts[i]` is how many assets of `TABS[i]` match the search
pub(crate) fn row(app: &mut TextureCraftApp, ui: &mut Ui, counts: &[usize], trailing: impl FnOnce(&mut TextureCraftApp, &mut Ui, f32)) {
    let size = ui.spacing().interact_size.y;

    // Reserve a slot for the thumbnail size control after the type buttons
    let row_width = (TABS.len() + 1) as f32 * (size + ICON_GAP_POINTS);
    let mut picked = app.ui.shelf_tab.min(NO_TYPE);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(ICON_GAP_POINTS, ICON_GAP_POINTS);
        if ui.available_width() >= row_width {
            // Leave the active type enabled so it can clear the filter
            for (index, (_, icon, label)) in TABS.iter().enumerate() {
                let selected = index == picked;
                let enabled = selected || counts.get(index).is_some_and(|count| *count > 0);
                if ui.add_enabled_ui(enabled, |ui| icon_button(ui, icon, tr(label), selected, size)).inner.clicked() {
                    picked = if selected { NO_TYPE } else { index };
                }
            }
        } else {
            collapsed(ui, &mut picked, counts, size);
        }

        let remaining_width = ui.available_width() - size;
        ui.add_space(remaining_width.max(0.0));
        trailing(app, ui, size);
    });
    app.ui.shelf_tab = picked;
}

/// One dropdown listing the types, dotted while one is picked
fn collapsed(ui: &mut Ui, picked: &mut usize, counts: &[usize], size: f32) {
    let filtering = *picked != NO_TYPE;
    let label = tr("Filter by type");
    let response = icon_dropdown_button(ui, egui_phosphor::regular::FUNNEL_SIMPLE, label, filtering, size).on_hover_text(label);

    if filtering {
        let tokens = crate::theme::Tokens::get(ui.ctx());
        let dot = response.rect.left_top() + vec2(size - FILTER_DOT_RADIUS_POINTS, FILTER_DOT_RADIUS_POINTS);
        ui.painter().circle_filled(dot, FILTER_DOT_RADIUS_POINTS, tokens.accent);
    }

    egui::Popup::menu(&response).show(|ui| {
        for (index, (_, _, label)) in TABS.iter().enumerate() {
            let selected = index == *picked;
            let enabled = selected || counts.get(index).is_some_and(|count| *count > 0);
            if ui.add_enabled(enabled, egui::Button::selectable(selected, tr(label))).clicked() {
                *picked = if selected { NO_TYPE } else { index };
                ui.close();
            }
        }

        ui.add_enabled(false, egui::Button::selectable(false, tr(FONTS))).on_disabled_hover_text(tr(crate::unavailable::NOT_AVAILABLE));
        ui.separator();

        if ui.button(tr(SHOW_ALL)).clicked() {
            *picked = NO_TYPE;
            ui.close();
        }
    });
}

/// Whether `tab` (an index into [`TABS`], or [`NO_TYPE`]) shows assets of type `kind`
pub(crate) fn shows(tab: usize, kind: Tab) -> bool {
    TABS.get(tab).is_none_or(|(shown, _, _)| *shown == kind)
}
