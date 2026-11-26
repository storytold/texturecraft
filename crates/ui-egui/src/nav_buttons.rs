//! Orbit, pan and zoom buttons in the corner of the 3D and 2D views. Pressing one with the primary button (mouse or pen tip) and
//! dragging moves the view like the mouse shortcuts do, without painting, so a pen and tablet can navigate without a keyboard or wheel.

use crate::TextureCraftApp;
use crate::i18n::tr;
pub use crate::nav_button::NavButton;
pub use crate::nav_view::NavView;
use crate::theme::Tokens;
use crate::viewport::{DRAG_ZOOM_PER_POINT, UV_VIEW_MAX_ZOOM, UV_VIEW_MIN_ZOOM, UV_WHEEL_ZOOM_PER_POINT};
use egui::{Area, CursorIcon, Order, PointerButton, Rect, Sense, Ui, Vec2, pos2, vec2};

/// Button side, gap between buttons and the distance from the view's right edge, in points
const BUTTON_SIZE: f32 = 30.0;
const BUTTON_GAP: f32 = 4.0;
const EDGE_MARGIN: f32 = 8.0;

/// Distance from the view's top edge to the first button: below the row of pickers that sits at the top centre, so the two never overlap
const TOP_OFFSET: f32 = 40.0;

/// Room kept below the column for the 2D view's axis gizmo (bottom right) and the view's own corner
const BOTTOM_ROOM: f32 = 64.0;

/// Narrowest a view may be and still show the column
const MIN_VIEW_WIDTH: f32 = BUTTON_SIZE + 2.0 * EDGE_MARGIN + 20.0;

/// The 2D zoom a drag changes per point, as a multiple of the wheel's (the 3D drag zoom is also a few times its wheel's)
const UV_DRAG_ZOOM_PER_WHEEL: f32 = 4.0;

/// The column of buttons in the top right corner of a view of `rect`; `None` when the view is too small to hold it without covering other things
fn strip_rect(rect: Rect, view: NavView) -> Option<Rect> {
    let count = view.buttons().len() as f32;
    let size = vec2(BUTTON_SIZE, count * BUTTON_SIZE + (count - 1.0) * BUTTON_GAP);
    let fits = rect.width() >= MIN_VIEW_WIDTH && rect.height() >= TOP_OFFSET + size.y + BOTTOM_ROOM;
    fits.then(|| Rect::from_min_size(pos2(rect.right() - EDGE_MARGIN - BUTTON_SIZE, rect.top() + TOP_OFFSET), size))
}

/// Where `button` sits in a view of `rect`; `None` when the view has no such button or is too small
pub fn button_rect(rect: Rect, view: NavView, button: NavButton) -> Option<Rect> {
    let index = view.buttons().iter().position(|candidate| *candidate == button)?;
    let strip = strip_rect(rect, view)?;
    Some(Rect::from_min_size(strip.min + vec2(0.0, index as f32 * (BUTTON_SIZE + BUTTON_GAP)), vec2(BUTTON_SIZE, BUTTON_SIZE)))
}

/// Whether `point` is over the shown buttons of a view of `rect`
pub(crate) fn covers(app: &TextureCraftApp, rect: Rect, view: NavView, point: egui::Pos2) -> bool {
    app.ui.nav_buttons && strip_rect(rect, view).is_some_and(|strip| strip.expand(BUTTON_GAP).contains(point))
}

/// Draws the buttons over the view `rect` and applies a drag on one to the view's camera
pub(crate) fn show(app: &mut TextureCraftApp, ui: &Ui, rect: Rect, view: NavView) {
    if !app.ui.nav_buttons {
        return;
    }

    let Some(strip) = strip_rect(rect, view) else { return };
    let tokens = Tokens::get(ui.ctx());
    let area_id = ui.scope_id().with(("nav_buttons", view));

    // Their own layer above the view, so a press on a button never reaches the view's own painting or camera handling
    Area::new(area_id).order(Order::Middle).fixed_pos(strip.min).constrain_to(rect).show(ui.ctx(), |ui| {
        // The whole column and a gap-wide border own a press, so one landing between buttons never reaches the view
        ui.allocate_rect(strip.expand(BUTTON_GAP), Sense::click_and_drag());
        for &button in view.buttons() {
            let Some(button_rect) = button_rect(rect, view, button) else { continue };
            let response = ui.interact(button_rect, area_id.with(button), Sense::click_and_drag());
            let response = response.on_hover_text(tr(button.tooltip(view))).on_hover_cursor(CursorIcon::Grab);
            let pressed = response.dragged_by(PointerButton::Primary);
            if pressed {
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                apply(app, rect, view, button, response.drag_delta());
            }

            let fill = if pressed {
                tokens.tool_active
            } else if response.hovered() {
                tokens.row_hover
            } else {
                tokens.panel.gamma_multiply(0.85)
            };
            let color = if pressed { tokens.accent } else { tokens.text };
            ui.painter().rect_filled(button_rect, 4.0, fill);
            ui.painter().text(button_rect.center(), egui::Align2::CENTER_CENTER, button.icon(), egui::FontId::proportional(BUTTON_SIZE * 0.55), color);
        }
    });
}

/// Moves the view by a drag of `delta` points on `button`, at the speeds and in the directions of the mouse shortcuts
fn apply(app: &mut TextureCraftApp, rect: Rect, view: NavView, button: NavButton, delta: Vec2) {
    let size = rect.size();
    match (view, button) {
        (NavView::Scene, NavButton::Orbit) => crate::viewport::orbit(app, delta),
        (NavView::Scene, NavButton::Pan) => app.session.camera.pan(delta.x / size.y, delta.y / size.y),
        (NavView::Scene, NavButton::Zoom) => app.session.camera.zoom(-delta.y * DRAG_ZOOM_PER_POINT * app.ui.prefs.drag_zoom_scale()),
        (NavView::Uv, NavButton::Pan) => {
            app.ui.uv_pan[0] += delta.x / size.x * 2.0;
            app.ui.uv_pan[1] -= delta.y / size.y * 2.0;
        }
        (NavView::Uv, NavButton::Zoom) => {
            // About the view's centre: the wheel zooms about the cursor, which is on the button here
            let old_zoom = app.ui.uv_zoom;
            let scroll = -delta.y * UV_DRAG_ZOOM_PER_WHEEL;
            let new_zoom = (old_zoom * (1.0 + scroll * UV_WHEEL_ZOOM_PER_POINT)).clamp(UV_VIEW_MIN_ZOOM, UV_VIEW_MAX_ZOOM);
            for axis in 0..2 {
                app.ui.uv_pan[axis] *= new_zoom / old_zoom;
            }
            app.ui.uv_zoom = new_zoom;
        }
        (NavView::Uv, NavButton::Orbit) => {}
    }
}
