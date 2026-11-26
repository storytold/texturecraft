//! Placing the symmetry planes by clicking the model: the next click sets the offsets so the planes pass through the clicked point

use egui::{Align2, Color32, CursorIcon, FontId, PointerButton, Response, Ui, pos2};
use texturecraft_engine::geom::glam::{Vec2, Vec3};

use super::movable_axes;
use crate::TextureCraftApp;
use crate::i18n::tr;

/// Distance from the view's top edge to the hint (points)
const HINT_INSET: f32 = 12.0;
const HINT_SIZE: f32 = 14.0;

/// The offsets that put the movable planes and axes through `point`, the others unchanged, kept inside the mesh bounds like the sliders
pub(crate) fn offsets_through(symmetry: &texturecraft_engine::paint::Symmetry, bounds: (Vec3, Vec3), point: Vec3) -> [f32; 3] {
    let center = (bounds.0 + bounds.1) * 0.5;
    let half_extent = (bounds.1 - bounds.0) * 0.5;
    let movable = movable_axes(symmetry);
    let mut offset = symmetry.offset;
    for axis in 0..3 {
        if movable[axis] {
            offset[axis] = (point[axis] - center[axis]).clamp(-half_extent[axis], half_extent[axis]);
        }
    }

    offset
}

/// While picking: a click on the model places the planes there and ends picking, and nothing paints.
/// Returns whether picking owns the view's input this frame.
pub(crate) fn input(app: &mut TextureCraftApp, ui: &Ui, response: &Response, rect: egui::Rect) -> bool {
    if !app.picking_symmetry_plane {
        return false;
    }

    // Picking is only for the Painting mode (the settings popup lives there)
    if app.ui.mode != crate::state::Mode::Painting {
        app.picking_symmetry_plane = false;
        return false;
    }

    if !movable_axes(&app.session.tool.brush.symmetry).contains(&true) {
        app.picking_symmetry_plane = false;
        return false;
    }

    ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
    ui.painter_at(rect).text(
        pos2(rect.center().x, rect.top() + HINT_INSET),
        Align2::CENTER_TOP,
        tr("Click the model to place the symmetry plane there (Esc cancels)"),
        FontId::proportional(HINT_SIZE),
        Color32::WHITE,
    );

    if response.clicked_by(PointerButton::Primary)
        && !ui.input(|input| input.modifiers.alt)
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let local = Vec2::new(pointer.x - rect.left(), pointer.y - rect.top());
        if let Some(hit) = app.session.pick(local, app.session.viewport) {
            let offset = offsets_through(&app.session.tool.brush.symmetry, app.session.mesh.bounds(), hit.pos);
            app.run("tool.brush", serde_json::json!({"symmetryOffset": offset}));
            app.picking_symmetry_plane = false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use texturecraft_engine::paint::Symmetry;

    use super::*;

    #[test]
    fn only_the_movable_axes_move_and_stay_inside_the_mesh() {
        let bounds = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 4.0, 6.0));
        let symmetry = Symmetry { x: true, z: true, offset: [0.0, 0.25, 0.0], ..Symmetry::default() };

        let offset = offsets_through(&symmetry, bounds, Vec3::new(1.5, 3.0, 9.0));

        assert_eq!(offset, [0.5, 0.25, 3.0], "x is picked minus center, y is untouched, z is clamped to the mesh");
    }

    /// Escape closes an open colour picker first and leaves Pick point armed; the next Escape cancels Pick point
    #[test]
    fn escape_closes_an_open_colour_picker_before_it_cancels_pick_point() {
        use egui_kittest::kittest::Queryable;

        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, _) = crate::frame_gizmo::tests::fill_app("triplanar");
        harness.state_mut().session.tool.brush.symmetry.x = true;
        harness.state_mut().picking_symmetry_plane = true;
        harness.get_by_role_and_label(egui::Role::Tab, "Material").click();
        harness.run_steps(25);
        harness.get_by_role_and_label(egui::Role::Button, "Base color").click();
        harness.run_steps(2);
        assert!(crate::color_picker::open_window(&harness.ctx).is_some(), "the picker is open");

        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);
        assert!(crate::color_picker::open_window(&harness.ctx).is_none(), "Escape closed the picker");
        assert!(harness.state().picking_symmetry_plane, "and left Pick point armed");

        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);
        assert!(!harness.state().picking_symmetry_plane, "the next Escape cancels Pick point");
    }
}
