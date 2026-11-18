//! Display Settings > Post effects: a switch and the glow, vignette, sharpen and film grain amounts, with a reset on the header.

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::core::project::DisplaySettings;
use texturecraft_ui_egui::TextureCraftApp;

const SECTION: &str = "Post effects";
const SWITCH: &str = "Post effects (viewport and Rendering mode)";
const SLIDERS: [&str; 4] = ["Glow", "Vignette", "Sharpen", "Film grain"];
const SETTLE_FRAMES: usize = 3;

/// The four amounts in the order of [`SLIDERS`]
fn amounts(d: &DisplaySettings) -> [f32; 4] {
    [d.bloom, d.vignette, d.sharpen, d.grain]
}

fn display(harness: &Harness<'_, TextureCraftApp>) -> DisplaySettings {
    harness.state().session.doc.project.display.clone()
}

/// The app with Display Settings open from the rail and the Post effects section unfolded
fn app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(4);
    harness
        .get_all_by_role(Role::Button)
        .find(|b| b.accesskit_node().label().is_some_and(|l| l == "Display Settings" || l.starts_with("Display Settings (")))
        .unwrap()
        .click();
    harness.run_steps(SETTLE_FRAMES);
    assert!(harness.query_by_label(SWITCH).is_none(), "the section starts folded");
    // The section's header, not the page's Post effects tab (a small icon) above it, scrolled into the page first
    let widest = |a: &egui_kittest::Node<'_>, b: &egui_kittest::Node<'_>| a.rect().width().total_cmp(&b.rect().width());
    harness.get_all_by_label(SECTION).max_by(widest).unwrap().scroll_to_me();
    harness.run_steps(SETTLE_FRAMES);
    harness.get_all_by_label(SECTION).max_by(widest).unwrap().click();
    harness.run_steps(SETTLE_FRAMES * 3);
    harness.hover_at(egui::pos2(5.0, 5.0));
    harness.run_steps(SETTLE_FRAMES);
    harness
}

fn reset_icon<'t>(harness: &'t Harness<'_, TextureCraftApp>) -> egui_kittest::Node<'t> {
    harness.get_by_role_and_label(Role::Button, "Post effects: Reset to defaults")
}

fn set_switch(harness: &mut Harness<'_, TextureCraftApp>, on: bool) {
    if display(harness).post_effects != on {
        harness.get_by_label(SWITCH).scroll_to_me();
        harness.run_steps(SETTLE_FRAMES);
        harness.get_by_label(SWITCH).click();
        harness.run_steps(SETTLE_FRAMES);
    }
    assert_eq!(display(harness).post_effects, on);
}

/// Drags the slider labelled `label` from its left end to `x` pixels in
fn drag(harness: &mut Harness<'_, TextureCraftApp>, label: &str, x: f32) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(SETTLE_FRAMES);
    let l = harness.get_by_label(label).rect();
    let y = l.bottom() + 10.0;
    harness.drag_at(egui::pos2(l.left() + 6.0, y));
    harness.run_steps(2);
    harness.hover_at(egui::pos2(l.left() + x, y));
    harness.run_steps(2);
    harness.drop_at(egui::pos2(l.left() + x, y));
    harness.run_steps(SETTLE_FRAMES);
}

#[test]
fn the_defaults_are_off() {
    let harness = app();
    let d = display(&harness);
    assert!(!d.post_effects);
    assert_eq!(amounts(&d), [0.0; 4]);
    assert!(reset_icon(&harness).accesskit_node().is_disabled(), "nothing to reset yet");
}

#[test]
fn each_slider_sets_only_its_own_amount() {
    for (i, label) in SLIDERS.into_iter().enumerate() {
        let mut harness = app();
        set_switch(&mut harness, true);
        drag(&mut harness, label, 120.0);
        let got = amounts(&display(&harness));
        assert!(got[i] > 0.05 && got[i] < 1.0, "{label} follows the drag: {got:?}");
        for (j, v) in got.into_iter().enumerate() {
            if j != i {
                assert_eq!(v, 0.0, "{label} leaves {} alone", SLIDERS[j]);
            }
        }
    }
}

#[test]
fn sliders_do_nothing_while_the_switch_is_off_and_switching_off_keeps_the_amounts() {
    let mut harness = app();
    drag(&mut harness, "Glow", 120.0);
    assert_eq!(display(&harness).bloom, 0.0, "the amounts are locked while the effects are off");

    set_switch(&mut harness, true);
    drag(&mut harness, "Glow", 120.0);
    let glow = display(&harness).bloom;
    assert!(glow > 0.05);

    set_switch(&mut harness, false);
    let d = display(&harness);
    assert!(!d.post_effects, "the renderer reads this switch, so every effect is off");
    assert_eq!(d.bloom, glow, "the amount is kept for the next time");
    drag(&mut harness, "Film grain", 120.0);
    assert_eq!(display(&harness).grain, 0.0);
}

#[test]
fn the_values_agree_with_the_view_command_and_survive_save_and_open() {
    let mut harness = app();
    set_switch(&mut harness, true);
    for label in SLIDERS {
        drag(&mut harness, label, 120.0);
    }
    let shown = display(&harness);
    assert!(amounts(&shown).iter().all(|v| *v > 0.05));

    // The command reports the same document values the panel changed
    let session = &mut harness.state_mut().session;
    let reported = texturecraft_engine::execute(session, "display.set", &json!({})).unwrap();
    assert_eq!(reported["post_effects"], json!(true));
    for (key, want) in [("bloom", shown.bloom), ("vignette", shown.vignette), ("sharpen", shown.sharpen), ("grain", shown.grain)] {
        assert_eq!(reported[key].as_f64().unwrap() as f32, want, "{key}");
    }

    let dir = std::env::temp_dir().join(format!("texturecraft-display-post-effects-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("p.tcraft").display().to_string();
    texturecraft_engine::execute(session, "file.saveAs", &json!({"path": path})).unwrap();

    // Wipe the values, then open the saved file
    texturecraft_engine::execute(session, "display.set", &json!({"postEffects": false, "bloom": 0, "vignette": 0, "sharpen": 0, "grain": 0})).unwrap();
    texturecraft_engine::execute(session, "file.open", &json!({"path": path})).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    harness.run_steps(SETTLE_FRAMES);
    let loaded = display(&harness);
    assert!(loaded.post_effects);
    assert_eq!(amounts(&loaded), amounts(&shown));
}

#[test]
fn reset_puts_back_only_the_post_effects() {
    let mut harness = app();
    harness.state_mut().session.doc.project.display =
        DisplaySettings { post_effects: true, bloom: 0.5, vignette: 0.4, sharpen: 0.3, grain: 0.2, env_opacity: 0.5, ..DisplaySettings::default() };
    harness.run_steps(SETTLE_FRAMES);
    assert!(!reset_icon(&harness).accesskit_node().is_disabled());
    reset_icon(&harness).scroll_to_me();
    harness.run_steps(SETTLE_FRAMES);
    reset_icon(&harness).focus();
    harness.run_steps(SETTLE_FRAMES);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(SETTLE_FRAMES);
    let d = display(&harness);
    assert_eq!(DisplaySettings { env_opacity: 0.5, ..DisplaySettings::default() }, d, "post effects are back to the defaults, the rest is untouched");
    assert!(reset_icon(&harness).accesskit_node().is_disabled());
}
