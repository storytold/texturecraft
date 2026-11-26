//! An angle as a dial beside a slider, as the reference app shows angles: drag the dial to point it, or set the slider

use egui::{Sense, Stroke, Ui, vec2};

use crate::i18n::tr;
use crate::theme::Tokens;
use crate::widgets::slider_edit;

const DIAL_SIZE: f32 = 36.0;
const DIAL_RIM_WIDTH: f32 = 1.5;
const POINTER_WIDTH: f32 = 2.0;
const HUB_RADIUS: f32 = 2.0;
/// Gap between the dial and its slider
const DIAL_GAP: f32 = 8.0;
const FULL_TURN: f32 = 360.0;

/// `label` over a dial and an "Angle" slider for `degrees` (0 at the right, counterclockwise): `Some(merge)` on every frame that changes it, as [`slider_edit`]
pub(crate) fn angle_dial(ui: &mut Ui, label: &str, degrees: &mut f32) -> Option<bool> {
    ui.label(label);
    ui.horizontal(|ui| {
        let dial_edit = dial(ui, degrees);
        ui.add_space(DIAL_GAP);
        let slider = ui.vertical(|ui| slider_edit(ui, tr("Angle"), degrees, 0.0..=FULL_TURN, 0)).inner;
        dial_edit.or(slider)
    })
    .inner
}

/// The dial: a disc whose pointer follows a drag around its center
fn dial(ui: &mut Ui, degrees: &mut f32) -> Option<bool> {
    let tokens = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(DIAL_SIZE, DIAL_SIZE), Sense::click_and_drag());
    let center = rect.center();
    let mut edit = None;
    if let Some(pointer) = response.interact_pointer_pos().filter(|_| response.dragged() || response.clicked()) {
        let offset = pointer - center;
        if offset.length() > HUB_RADIUS {
            // Screen y grows downwards, angles grow counterclockwise
            *degrees = (-offset.y).atan2(offset.x).to_degrees().rem_euclid(FULL_TURN);
            edit = Some(response.dragged() && !response.drag_started());
        }
    }

    let radius = DIAL_SIZE * 0.5 - DIAL_RIM_WIDTH;
    let painter = ui.painter();
    let rim = if response.hovered() || response.dragged() { tokens.button_border } else { tokens.field_border };
    painter.circle(center, radius, tokens.field, Stroke::new(DIAL_RIM_WIDTH, rim));
    let (sin, cos) = degrees.to_radians().sin_cos();
    painter.line_segment([center, center + vec2(cos, -sin) * (radius - DIAL_RIM_WIDTH)], Stroke::new(POINTER_WIDTH, tokens.accent));
    painter.circle_filled(center, HUB_RADIUS, tokens.text);
    edit
}
