//! Pause Engine Computation (Shift+Esc) and its top bar button. The compositing and upload
//! skipping is covered by the `gpu` unit test.

use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use texturecraft_ui_egui::TextureCraftApp;

fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(6);
    h
}

fn shift_escape(h: &mut Harness<'_, TextureCraftApp>) {
    let modifiers = Modifiers::SHIFT;
    h.event(Event::Key { key: Key::Escape, physical_key: Some(Key::Escape), pressed: true, repeat: false, modifiers });
    h.run_steps(2);
}

#[test]
fn shift_escape_toggles_the_pause() {
    let mut h = harness();
    assert!(!h.state().session.paused);

    shift_escape(&mut h);
    assert!(h.state().session.paused, "Shift+Esc pauses");

    shift_escape(&mut h);
    assert!(!h.state().session.paused);
}

#[test]
fn the_top_bar_button_has_the_reference_tooltip_and_toggles() {
    let mut h = harness();
    let label =
        "Pause Engine Computation \u{2014} When the engine is paused, textures are not computed or updated in the layer stack or the viewport. (Shift+Escape)";
    h.get_by_label(label).click();
    h.run_steps(3);
    assert!(h.state().session.paused);
    h.get_by_label(label).click();
    h.run_steps(3);
    assert!(!h.state().session.paused);
}
