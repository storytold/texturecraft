//! A press outside the popups that turns into a drag closes them: egui closes a popup only on a click, so a paint stroke or slider drag left it open

use egui::{Order, PointerButton};

/// Whether a primary press outside every popup has just been held or moved past a click: it starts a stroke, a slider drag or a panel move
pub(crate) fn drag_outside_popups(ctx: &egui::Context) -> bool {
    let (dragging, origin, orbiting) = ctx.input(|input| {
        let dragging = input.pointer.button_down(PointerButton::Primary) && input.pointer.is_decidedly_dragging();
        (dragging, input.pointer.press_origin(), input.modifiers.alt)
    });

    // Alt+drag orbits the camera, which keeps popups open like the colour picker; a drag in a popup or a dropdown it opened stays in it
    let over_popup = |at| ctx.layer_id_at(at).is_some_and(|layer| matches!(layer.order, Order::Foreground | Order::Tooltip | Order::Debug));
    dragging && !orbiting && origin.is_some_and(|at| !over_popup(at))
}

/// Closes egui's open popup, menu or dropdown when [`drag_outside_popups`]; a plain click outside is egui's own close
pub(crate) fn close_on_drag_outside(ctx: &egui::Context) {
    if egui::Popup::is_any_open(ctx) && drag_outside_popups(ctx) {
        egui::Popup::close_all(ctx);
    }
}
