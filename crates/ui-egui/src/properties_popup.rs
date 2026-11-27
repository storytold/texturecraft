//! The Properties panel's contents in a popup at the cursor: a right click in the 3D or 2D view opens it

use egui::{PointerButton, Pos2, Rect, Ui};

use crate::TextureCraftApp;
use crate::property_sheet::SectionScope;
use crate::state::Mode;
use crate::theme::Tokens;

/// The popup's width, about the docked Properties panel's
const PROPERTIES_POPUP_WIDTH: f32 = 300.0;

/// Share of the view's height the popup fills before its contents scroll
const PROPERTIES_POPUP_MAX_HEIGHT_SHARE: f32 = 0.7;

/// Gap the popup keeps from the view's edges, so its border stays visible beside a split line
const PROPERTIES_POPUP_VIEW_MARGIN: f32 = 4.0;

/// The Properties popup over a view
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropertiesPopup {
    /// Shown with its top-left corner at `at`, kept inside the view `bounds`
    Open { at: Pos2, bounds: Rect },
    /// Closed by a press outside it: until every button is up, that press neither paints nor reopens it
    Closing,
}

/// The popup's area, whose rect egui remembers between frames
pub fn area_id() -> egui::Id {
    egui::Id::unique("properties_popup")
}

/// Opens the popup on a plain right click in a view; right drags and modified clicks keep their gestures
pub(crate) fn open_on_right_click(app: &mut TextureCraftApp, ui: &Ui, response: &egui::Response, view: Rect, gesture_key_held: bool) {
    let free = app.ui.mode == Mode::Painting
        && app.properties_popup.is_none()
        && !app.session.stroke_active()
        && app.moving_on_model.is_none()
        && !gesture_key_held
        && !crate::viewport::focus_picking(ui.ctx())
        && crate::generator_point_pick::armed(ui.ctx()).is_none()
        && ui.input(|input| input.modifiers.is_none());

    if !free || !response.clicked_by(PointerButton::Secondary) {
        return;
    }

    if let Some(at) = response.interact_pointer_pos() {
        app.properties_popup = Some(PropertiesPopup::Open { at, bounds: view });
    }
}

/// Draws the open popup and closes it on a press outside it (but not an eyedropper pick's) or when the app leaves Painting
pub(crate) fn show(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let (any_down, pressed_at) = ctx.input(|input| (input.pointer.any_down(), input.pointer.press_origin().filter(|_| input.pointer.any_pressed())));

    match app.properties_popup {
        _ if app.ui.mode != Mode::Painting => app.properties_popup = None,
        Some(PropertiesPopup::Closing) if !any_down => app.properties_popup = None,
        Some(PropertiesPopup::Open { .. }) => {
            let shown = ctx.memory(|memory| memory.area_rect(area_id()));
            let in_picker = |at: egui::Pos2| crate::color_picker::open_window(ctx).is_some_and(|picker| picker.contains(at));
            let outside = pressed_at.is_some_and(|at| shown.is_some_and(|rect| !rect.contains(at)) && !in_picker(at));

            // A child picker or an eyedropper press can land outside the panel
            if outside && !egui::Popup::is_any_open(ctx) && app.picking_color.is_none() {
                app.properties_popup = Some(PropertiesPopup::Closing);
            }
        }
        _ => {}
    }

    let Some(PropertiesPopup::Open { at, bounds }) = app.properties_popup else { return };

    let tokens = Tokens::get(ctx);
    egui::Area::new(area_id()).order(egui::Order::Foreground).fixed_pos(at).constrain_to(bounds.shrink(PROPERTIES_POPUP_VIEW_MARGIN)).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).fill(tokens.panel).inner_margin(egui::Margin::ZERO).show(ui, |ui| {
            ui.set_width(PROPERTIES_POPUP_WIDTH);

            // The area's remembered size would otherwise cap the scroll height
            ui.set_max_height(bounds.height() * PROPERTIES_POPUP_MAX_HEIGHT_SHARE);

            // A floating bar would cover the sliders' right-aligned values
            ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();

            // The docked panel's jump tabs on the top edge stay put while the sections scroll under them
            let scope = SectionScope::Popup;
            if let Some(tabs) = crate::dock::properties_fill_tabs(app) {
                crate::property_sheet::section_tabs(ui, scope, ui.style().visuals.menu_corner_radius.nw, &tabs);
            }
            crate::property_sheet::in_scope(ui, scope, |ui| {
                // A popup that fits its rows keeps its size; one that scrolls goes on until its last section's top
                let area = egui::ScrollArea::vertical().id_salt("properties_popup_scroll");
                crate::dock::properties_scrolled(app, ui, area, crate::past_end::PastEnd::WhenScrolling);
            });
        });
    });
}
