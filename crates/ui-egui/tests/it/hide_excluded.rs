//! The top bar's "Hide/ignore excluded geometry" toggle (Alt+H): off without a project; with a
//! layer whose geometry mask leaves out the lid in front of a body, it hides the lid and the view
//! picks the body behind.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::{Session, execute};
use texturecraft_ui_egui::{TextureCraftApp, menus, theme, toolbar};

const TOOLTIP: &str = "Hides geometry excluded in the geometry mask. While enabled, brush strokes paint through geometry. (Alt+H)";

fn harness(session: Session) -> Harness<'static, TextureCraftApp> {
    let context = eframe::CreationContext::_new_kittest(egui::Context::default());
    let mut app = TextureCraftApp::new(&context, session);
    app.ui.prefs.language = "en".into();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 200.0))
        .with_context_setup(|context| {
            theme::install(context);
            theme::apply(context, theme::Brightness::default(), theme::Accent::default());
        })
        .build_ui_state(|ui, app| toolbar::paint_bar(app, ui), app);
    harness.run_steps(4);
    harness
}

/// A "Lid" triangle in front of a bigger "Body" triangle; the selected layer shows on the body only.
fn lid_over_body() -> Session {
    let obj = "o Lid\nv 0 0 0.5\nv 1 0 0.5\nv 0 1 0.5\nvt 0 0\nvt 0.5 0\nvt 0 0.5\nf 1/1 2/2 3/3\n\
               o Body\nv 0 0 0\nv 2 0 0\nv 0 2 0\nvt 0.5 0.5\nvt 1 0.5\nvt 0.5 1\nf 4/4 5/5 6/6\n";
    let mut s = Session::from_mesh_bytes("lid.obj", obj.as_bytes(), 256, "lid").unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    (s.camera.yaw, s.camera.pitch) = (0.0, 0.0);
    let (lo, hi) = s.mesh.bounds();
    s.camera.frame(lo, hi);
    let layer = s.selected_layer().unwrap().0;
    execute(&mut s, "layer.set", &json!({"layer": layer, "geometryMask": ["Body"]})).unwrap();
    s
}

#[test]
fn the_toggle_hides_the_excluded_part_and_picking_reaches_the_body() {
    let mut harness = harness(lid_over_body());
    let n = harness.state().session.camera.project(texturecraft_engine::geom::glam::Vec3::new(0.2, 0.2, 0.0), 1.0).unwrap();
    let at = Vec2::new((n.x + 1.0) * 200.0, (1.0 - n.y) * 200.0);
    assert!(harness.state().session.excluded_parts().is_empty());
    assert_eq!(harness.state().session.pick(at, Vec2::new(400.0, 400.0)).unwrap().tri, 0, "the lid is in front");

    harness.get_by_label(TOOLTIP).click();
    harness.run_steps(3);
    assert!(harness.state().session.hide_excluded);
    assert_eq!(harness.state().session.excluded_parts(), [true, false], "the lid is hidden");
    assert_eq!(harness.state().session.pick(at, Vec2::new(400.0, 400.0)).unwrap().tri, 1, "the body behind is picked");

    harness.get_by_label(TOOLTIP).click();
    harness.run_steps(3);
    assert!(!harness.state().session.hide_excluded && harness.state().session.excluded_parts().is_empty(), "off restores the view");
}

#[test]
fn alt_h_runs_the_toggle() {
    let mut harness = harness(lid_over_body());
    let ctx = harness.ctx.clone();
    menus::invoke(harness.state_mut(), &ctx, "viewport.hideExcluded");
    assert!(harness.state().session.hide_excluded);
    assert_eq!(texturecraft_ui_egui::keymap::bindings().iter().find(|b| b.id == "viewport.hideExcluded").and_then(|b| b.default), Some("Alt+H"));
}

#[test]
fn the_toggle_is_disabled_without_a_project() {
    let mut harness = harness(Session::closed().unwrap());
    let node = harness.get_by_label(TOOLTIP);
    assert!(node.accesskit_node().is_disabled(), "no project: disabled");
    harness.run_steps(1);
    assert!(!harness.state().session.hide_excluded);
}
