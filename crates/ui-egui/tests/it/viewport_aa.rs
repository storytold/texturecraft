//! The top bar's AA toggle switches the 3D view's anti-aliasing (4× MSAA and TAA together), remembers it in the
//! preferences for the next project, answers Alt+A, and leaves painting and Rendering mode
//! working; the camera button opens Export image.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::Mode;

#[test]
fn aa_toggle_switches_taa_persists_and_keeps_painting_working() {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    let aa = |h: &Harness<'_, TextureCraftApp>| {
        let d = &h.state().session.doc.project.display;
        assert_eq!(d.msaa, d.taa, "the toggle switches multisampling and the temporal refinement together");
        d.msaa
    };
    assert!(aa(&h), "on by default");

    h.get_by_label("Anti-aliasing").click();
    h.run_steps(3);
    assert!(!aa(&h), "off: one sample per pixel, no refinement");
    assert!(!h.state().ui.prefs.viewport_aa, "the choice is a preference");

    // Keyboard: Alt+A (the Viewport › Anti-aliasing shortcut) turns it back on, and off again.
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::A);
    h.run_steps(3);
    assert!(aa(&h));
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::A);
    h.run_steps(3);
    assert!(!aa(&h));

    // Painting still works with anti-aliasing off.
    let s = &mut h.state_mut().session;
    texturecraft_engine::execute(s, "layer.add.paint", &json!({})).unwrap();
    let before = s.history_labels().len();
    let (cx, cy) = (s.viewport.x / 2.0, s.viewport.y / 2.0);
    let r = texturecraft_engine::execute(s, "tool.stroke", &json!({"points": [[cx, cy], [cx + 20.0, cy + 5.0]]})).unwrap();
    assert!(s.history_labels().len() > before, "the stroke landed: {r}");
    h.run_steps(3);

    // A new project takes the preference (off), not the default (on).
    texturecraft_engine::execute(&mut h.state_mut().session, "project.new", &json!({"mesh": "sphere", "resolution": 128})).unwrap();
    h.run_steps(3);
    assert!(!aa(&h), "the new project follows the preference");

    // Rendering mode (path traced) runs with the toggle in either state.
    h.state_mut().ui.mode = Mode::Rendering;
    h.run_steps(4);
    h.state_mut().toggle_viewport_aa();
    h.run_steps(4);
    assert!(aa(&h) && h.state().ui.prefs.viewport_aa);
    h.state_mut().ui.mode = Mode::Painting;
    h.run_steps(3);
}

#[test]
fn camera_button_opens_export_image() {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    assert!(!h.state().ui.windows.image_export);
    h.get_by_label("Export image (resolution, render scale, transparency) (Ctrl+Shift+P)").click();
    h.run_steps(4);
    assert!(h.state().ui.windows.image_export);
    h.get_by_label("Render scale");
    h.get_by_label("Transparent background");
    // The settings persist with the preferences.
    let prefs = serde_json::to_value(&h.state().ui.prefs).unwrap();
    assert_eq!(prefs["image_export"]["scale"], 1);
    assert_eq!(prefs["viewport_aa"], true);
}
