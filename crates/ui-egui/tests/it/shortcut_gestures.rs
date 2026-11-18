//! The shortcut window's fixed list names the mouse gestures and hold keys of the painting tools.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;

#[test]
fn the_gesture_grid_of_the_shortcut_window_lists_the_stencil_clone_line_and_path_gestures() {
    let mut h = Harness::builder()
        .with_size(egui::vec2(700.0, 900.0))
        .with_context_setup(texturecraft_ui_egui::theme::install)
        .build_ui(texturecraft_ui_egui::keymap::fixed_gestures_grid);
    h.run_steps(3);
    for text in [
        "Shift + LMB",
        "Paint a straight line from where the last stroke ended",
        "S + LMB drag",
        "Projection: rotate the stencil",
        "S + RMB drag",
        "S + MMB drag",
        "V + LMB",
        "Clone: pick the point to copy from",
        "Path: add a point; on the curve it inserts one",
        "Path: close it (needs 3 points)",
        "Path: apply it with the brush",
        "Path: remove the selected or last point",
        "Path: cancel it",
        "Shift + RMB drag",
        "Alt + Shift + LMB drag",
        "Camera snap rotate",
    ] {
        assert!(h.query_by_label(text).is_some(), "the overview lacks {text}");
    }
}
