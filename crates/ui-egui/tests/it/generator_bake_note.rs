//! The generator's note when a mesh map it reads is not baked or out of date, and its Bake button

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::core::{BakedMap, LayerId};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, ViewMode, execute};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;

/// A plane whose bake makes only the position map, with a curvature generator selected in a fill layer's mask
fn harness() -> Harness<'static, TextureCraftApp> {
    harness_with("curvature")
}

/// A plane whose bake makes only the position map, with a `generator` selected in a fill layer's mask
fn harness_with(generator: &str) -> Harness<'static, TextureCraftApp> {
    let mesh = primitives::builtin("plane").unwrap();
    let mut session = Session::new_project("t", mesh, "builtin:plane", None, 128).unwrap();
    execute(&mut session, "bake.settings", &json!({ "resolution": 64, "maps": ["position"], "dilation": 0 })).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({ "color": [0.8, 0.3, 0.1] })).unwrap()["layer"].as_u64().unwrap();
    execute(&mut session, "mask.add.black", &json!({})).unwrap();
    execute(&mut session, "effect.add.generator", &json!({ "generator": generator })).unwrap();
    session.view = ViewMode::Mask(LayerId(layer));
    session.update();

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.dock.close(Panel::Layers);
    harness.state_mut().ui.dock.close(Panel::TextureSetSettings);
    harness.state_mut().ui.dock.focus(Panel::Properties);
    harness.run_steps(6);
    harness
}

/// Run frames until the bake ends, then a few more for its result to show
fn run_to_the_end(harness: &mut Harness<'static, TextureCraftApp>) {
    super::step_until(harness, "the end of the bake", |app| app.bake.is_none());
    harness.run_steps(super::SETTLE_FRAMES);
}

#[test]
fn the_note_shows_while_the_curvature_map_is_missing_or_out_of_date_only() {
    let mut harness = harness();
    let description = harness.get_by_label("Curvature map not baked").accesskit_node().description();
    assert!(description.is_some_and(|reasons| !reasons.is_empty()), "why the note shows is read without hovering");

    execute(&mut harness.state_mut().session, "bake.run", &json!({ "maps": ["curvature"] })).unwrap();
    harness.run_steps(3);

    assert!(harness.query_by_label("Curvature map not baked").is_none(), "baked: no note");
    assert!(harness.query_by_label("Curvature map out of date").is_none(), "baked: no note");

    execute(&mut harness.state_mut().session, "bake.settings", &json!({ "dilation": 4 })).unwrap();
    harness.run_steps(3);

    harness.get_by_label("Curvature map out of date");
}

#[test]
fn the_notes_bake_button_turns_the_map_on_and_bakes_it_in_the_background() {
    let mut harness = harness();
    assert!(!harness.state().session.doc.project.texture_sets[0].bake.maps.is_enabled(BakedMap::Curvature));

    harness.get_by_label("Bake").click();
    harness.run_steps(1);

    assert!(harness.state().session.doc.project.texture_sets[0].bake.maps.is_enabled(BakedMap::Curvature), "turned on");
    assert!(harness.state().bake.is_some(), "a background bake started");

    run_to_the_end(&mut harness);

    assert!(harness.state().session.baked(0).is_some_and(|baked| baked.maps.contains_key(&BakedMap::Curvature)));
    assert!(harness.query_by_label("Curvature map not baked").is_none(), "baked: no note");
}

#[test]
fn rust_runoff_notes_the_occlusion_it_reads_though_no_input_replaces_it() {
    let harness = harness_with("Dripping Rust");
    harness.get_by_label("Ambient Occlusion map not baked");
}
