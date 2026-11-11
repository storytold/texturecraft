//! The shelf greys tools that can't act on the selection; the top bar and Properties follow it

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};
use texturecraft_engine::{Session, StackTarget, Tool, execute};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::Mode;
use texturecraft_ui_egui::theme::Brightness;

fn add_layer(session: &mut Session, command: &str) -> u64 {
    execute(session, command, &json!({})).unwrap()["layer"].as_u64().unwrap()
}

fn select(session: &mut Session, layer: u64, target: Value) {
    execute(session, "layer.select", &json!({"layer": layer, "target": target})).unwrap();
}

fn select_nothing(session: &mut Session) {
    let set = session.ts();
    session.selected.remove(&set);
    session.target = StackTarget::Content;
}

fn app_harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.run_steps(4);
    harness
}

/// Selects a new fill layer's content with `tool` equipped
fn select_fill_content(harness: &mut Harness<'_, TextureCraftApp>, tool: Tool) {
    let session = &mut harness.state_mut().session;
    let fill = add_layer(session, "layer.add.fill");
    select(session, fill, json!("content"));
    session.tool.tool = tool;
    harness.run_steps(4);
}

fn shelf_button_disabled(harness: &Harness<'_, TextureCraftApp>, label: &str) -> bool {
    harness.get_by_label(label).accesskit_node().is_disabled()
}

fn shelf_button_selected(harness: &Harness<'_, TextureCraftApp>, label: &str) -> bool {
    harness.get_by_label(label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

fn shows_text(harness: &Harness<'_, TextureCraftApp>, text: &str) -> bool {
    harness.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(found) if found.galley.text() == text))
}

#[test]
fn a_fill_layer_greys_the_paint_tools_and_shows_its_own_properties() {
    let mut harness = app_harness();
    let session = &mut harness.state_mut().session;
    let paint = add_layer(session, "layer.add.paint");
    select(session, paint, json!("content"));
    harness.run_steps(4);

    assert!(!shelf_button_disabled(&harness, "Paint (1)"));
    assert!(shelf_button_selected(&harness, "Paint (1)"));
    assert!(shows_text(&harness, "Size"), "the brush controls are in the top bar");

    select_fill_content(&mut harness, Tool::PolygonFill);

    assert!(shelf_button_disabled(&harness, "Paint (1)"));
    assert!(shelf_button_disabled(&harness, "Polygon fill (4)"));
    assert!(!shelf_button_selected(&harness, "Polygon fill (4)"), "the equipped tool doesn't show as selected while greyed");
    assert!(!shelf_button_disabled(&harness, "Geometry mask (8)"));
    assert!(shows_text(&harness, "Properties - Fill"), "a fill layer shows its fill whatever the tool");
    assert!(!shows_text(&harness, "Size"), "a tool that can't act leaves the top bar");
}

#[test]
fn shortcuts_skip_the_tools_the_shelf_greys() {
    let mut harness = app_harness();
    select_fill_content(&mut harness, Tool::Paint);

    harness.key_press(egui::Key::Num2);
    harness.run_steps(2);

    assert_eq!(harness.state().session.tool.tool, Tool::Paint, "Eraser can't act on a fill layer");

    harness.key_press(egui::Key::P);
    harness.run_steps(2);

    assert_eq!(harness.state().session.tool.tool, Tool::ColorPicker);
}

/// Drag with the left button across the middle of the 3D view, as a stroke would
fn drag_across_the_view(harness: &mut Harness<'_, TextureCraftApp>) {
    const STEPS: usize = 4;
    const STEP_POINTS: f32 = 15.0;

    let center = harness.state().viewport_rect.center();
    let modifiers = egui::Modifiers::NONE;
    harness.hover_at(center);
    harness.event(egui::Event::PointerButton { pos: center, button: egui::PointerButton::Primary, pressed: true, modifiers });
    harness.run_steps(2);

    for step in 1..=STEPS {
        harness.hover_at(center + egui::vec2(STEP_POINTS * step as f32, 0.0));
        harness.run_steps(1);
    }

    let end = center + egui::vec2(STEP_POINTS * STEPS as f32, 0.0);
    harness.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers });
    harness.run_steps(3);
}

#[test]
fn a_tool_that_cant_act_ignores_the_view() {
    let mut harness = app_harness();
    let session = &mut harness.state_mut().session;
    let paint = add_layer(session, "layer.add.paint");
    select(session, paint, json!("content"));
    harness.run_steps(4);

    let steps_before = harness.state().session.history_labels().len();
    drag_across_the_view(&mut harness);

    assert!(harness.state().session.history_labels().len() > steps_before, "the drag reaches the mesh and paints a paint layer");

    for tool in [Tool::Paint, Tool::PolygonFill, Tool::Path] {
        select_fill_content(&mut harness, tool);
        let steps_before = harness.state().session.history_labels().len();
        drag_across_the_view(&mut harness);

        let state = harness.state();
        assert!(state.toasts.is_empty(), "{tool:?} on a fill layer: no error toast, got {:?}", state.toasts.first().map(|toast| &toast.text));
        assert_eq!(state.session.history_labels().len(), steps_before, "{tool:?} on a fill layer changes nothing");
        assert!(state.session.tool.path.points.is_empty(), "{tool:?} places no path points");
    }
}

#[test]
fn bake_painting_turns_on_the_shelf_paint_tool() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    session.add_high_poly_bytes("High.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(4);
    assert!(shelf_button_disabled(&harness, "Paint (1)"), "nothing is being painted yet");

    harness.state_mut().ui.bake_paint_brush.on = true;
    harness.run_steps(4);

    assert!(harness.state().ui.bake_paint_brush.on, "the high poly allows bake painting");
    assert!(!shelf_button_disabled(&harness, "Paint (1)"), "Paint is the bake brush while it paints");
    assert!(shelf_button_disabled(&harness, "Eraser (2)"));
    assert!(shelf_button_disabled(&harness, "Polygon fill (4)"));
}

#[test]
fn nothing_selected_asks_for_a_layer() {
    let mut harness = app_harness();
    select_nothing(&mut harness.state_mut().session);
    harness.run_steps(4);

    assert!(shows_text(&harness, "Select a layer to paint on"));
    assert!(shelf_button_disabled(&harness, "Geometry mask (8)"));
}

#[test]
fn baking_keeps_three_greyed_tools_and_rendering_none() {
    let mut harness = app_harness();
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(4);

    for label in ["Paint (1)", "Eraser (2)", "Polygon fill (4)"] {
        assert!(shelf_button_disabled(&harness, label), "{label}");
    }
    assert!(harness.query_by_label("Path (7)").is_none());

    harness.state_mut().ui.mode = Mode::Rendering;
    harness.run_steps(4);

    assert!(harness.query_by_label("Paint (1)").is_none());
}

#[test]
#[ignore = "offscreen GPU snapshots; set TC_UI_SNAPSHOTS to an output directory"]
fn tool_availability_snapshots() {
    let Some(directory) = std::env::var_os("TC_UI_SNAPSHOTS").map(std::path::PathBuf::from) else {
        return;
    };

    std::fs::create_dir_all(&directory).unwrap();
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).wgpu().build_eframe(|context| TextureCraftApp::new(context, session));
    let save = |harness: &mut Harness<'_, TextureCraftApp>, name: &str| {
        for (brightness, suffix) in [(Brightness::MediumDark, "medium"), (Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            harness.run_steps(8);
            harness.render().unwrap().save(directory.join(format!("{name}-{suffix}.png"))).unwrap();
        }
    };

    let session = &mut harness.state_mut().session;
    let paint = add_layer(session, "layer.add.paint");
    select(session, paint, json!("content"));
    session.tool.tool = Tool::Paint;
    save(&mut harness, "paint-layer");

    let session = &mut harness.state_mut().session;
    let fill = add_layer(session, "layer.add.fill");
    select(session, fill, json!("content"));
    session.tool.tool = Tool::PolygonFill;
    save(&mut harness, "fill-content-polygon-fill");

    let session = &mut harness.state_mut().session;
    execute(session, "effect.add.filter", &json!({"layer": paint, "target": "content", "filter": "blur"})).unwrap();
    let filter = session.doc.project.layer(texturecraft_engine::core::LayerId(paint)).unwrap().content_effects.last().unwrap().id.0;
    select(session, paint, json!(filter));
    session.tool.tool = Tool::Paint;
    save(&mut harness, "filter");

    let session = &mut harness.state_mut().session;
    execute(session, "mask.add.black", &json!({"layer": paint})).unwrap();
    select(session, paint, json!("mask"));
    session.tool.tool = Tool::Smudge;
    save(&mut harness, "mask-smudge");

    let session = &mut harness.state_mut().session;
    let folder = add_layer(session, "layer.add.group");
    select(session, folder, json!("content"));
    session.tool.tool = Tool::Paint;
    save(&mut harness, "folder");

    select_nothing(&mut harness.state_mut().session);
    save(&mut harness, "nothing-selected");

    harness.state_mut().ui.mode = Mode::Baking;
    save(&mut harness, "baking");

    harness.state_mut().ui.mode = Mode::Rendering;
    save(&mut harness, "rendering");
}
