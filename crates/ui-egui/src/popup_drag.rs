//! Dismisses popups when a primary press outside them becomes a drag

use std::sync::Arc;

use egui::{Event, Id, Order, PointerButton, ViewportId};

/// Installs dismissal before drawing popups in the main and native floating windows
pub(crate) fn install(ctx: &egui::Context) {
    ctx.on_begin_pass("dismiss popups on outside drag", Arc::new(|ui| update(ui.ctx())));
}

/// Tracks the primary press across frames, including the frame that releases it
fn update(ctx: &egui::Context) {
    let viewport = ctx.viewport_id();
    let press_id = Id::unique("popup-drag-press");
    let result_id = Id::unique("popup-drag-result");
    let (press, down, drag) = ctx.input(|input| {
        let pointer = &input.pointer;
        let press = pointer
            .button_pressed(PointerButton::Primary)
            .then(|| {
                input.events.iter().rev().find_map(|event| match event {
                    Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, .. } => Some(*pos),
                    _ => None,
                })
            })
            .flatten();
        let down = input.focused && pointer.button_down(PointerButton::Primary);
        let released_without_click = pointer.button_released(PointerButton::Primary) && !pointer.button_clicked(PointerButton::Primary);
        let drag = input.focused && !input.modifiers.alt && (released_without_click || (down && pointer.is_decidedly_dragging()));
        (press, down, drag)
    });

    let started_outside =
        press.map(|position| ctx.layer_id_at(position).is_none_or(|layer| !matches!(layer.order, Order::Foreground | Order::Tooltip | Order::Debug)));
    let dismiss = ctx.data_mut(|data| {
        let previous = data.get_temp::<(ViewportId, bool)>(press_id).filter(|(owner, _)| *owner == viewport);
        let outside = started_outside.or(previous.map(|(_, outside)| outside)).unwrap_or(false);
        if let Some(started_outside) = started_outside.filter(|_| down) {
            data.insert_temp(press_id, (viewport, started_outside));
        } else if !down && previous.is_some() {
            data.remove::<(ViewportId, bool)>(press_id);
        }

        let dismiss = outside && drag;
        if viewport == ViewportId::ROOT {
            data.insert_temp(result_id, dismiss);
        }

        dismiss
    });

    if dismiss && egui::Popup::is_any_open(ctx) {
        egui::Popup::close_all(ctx);
    }
}

/// Whether the main window's current pass has an outside drag for its app-owned popups
pub(crate) fn drag_outside_popups(ctx: &egui::Context) -> bool {
    let id = Id::unique("popup-drag-result");
    ctx.data(|data| data.get_temp::<bool>(id).unwrap_or(false))
}
