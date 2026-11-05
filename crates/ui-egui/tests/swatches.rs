//! The swatch strip under the brush colour: click applies, drag reorders, and the global swatches
//! reach the preferences.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::core::Channel;
use texturecraft_engine::core::swatch;
use texturecraft_ui_egui::TextureCraftApp;

fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    let app = h.state_mut();
    app.run("tool.paint", json!({}));
    app.run("tool.material", json!({"channel": "baseColor", "color": "#808080"}));
    for (name, c) in [("Brick", "#a03020"), ("Moss", "#406020")] {
        app.run("swatch.add", json!({"color": c, "name": name}));
    }
    // The colour editor is on the Properties panel's Material tab.
    h.state_mut().ui.props_tab = texturecraft_ui_egui::state::PropsTab::Material;
    h.run_steps(4);
    h
}

fn centre(h: &Harness<'_, TextureCraftApp>, label: &str) -> egui::Pos2 {
    h.get_by_label(label).rect().center()
}

#[test]
fn clicking_a_swatch_sets_the_brush_colour() {
    let mut h = harness();
    let at = centre(&h, "project Moss  #406020");
    h.drag_at(at);
    h.drop_at(at);
    h.run_steps(3);
    assert_eq!(swatch::to_hex(h.state().session.tool.values[&Channel::BaseColor]), "#406020");
}

#[test]
fn dragging_a_swatch_reorders_the_list() {
    let mut h = harness();
    let from = centre(&h, "project Moss  #406020");
    let to = centre(&h, "project Brick  #a03020");
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=6 {
        h.hover_at(from + (to - from) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(3);
    let names: Vec<&str> = h.state().session.doc.project.swatches.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Moss", "Brick"]);
}

#[test]
fn global_swatches_and_recent_colours_reach_the_preferences() {
    let mut h = harness();
    h.state_mut().run("swatch.add", json!({"scope": "global", "color": "#ffffff", "name": "Paper"}));
    h.state_mut().run("swatch.apply", json!({"index": 0}));
    h.run_steps(3);
    let prefs = &h.state().ui.prefs;
    assert_eq!(prefs.swatches.len(), 1);
    assert_eq!(prefs.swatches[0].name, "Paper");
    assert_eq!(swatch::to_hex(prefs.recent_colors[0]), "#a03020");
}
