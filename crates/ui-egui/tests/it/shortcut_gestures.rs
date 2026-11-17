//! The shortcut window's fixed list names the mouse gestures and hold keys of the painting tools.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use texturecraft_ui_egui::TextureCraftApp;

#[test]
fn the_shortcut_overview_lists_the_stencil_clone_line_and_path_gestures() {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    h.state_mut().ui.windows.shortcuts = true;
    h.run_steps(3);
    h.get_by_label("Viewport and brush (fixed)").click();
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
    ] {
        assert!(h.query_by_label(text).is_some(), "the overview lacks {text}");
    }
}
