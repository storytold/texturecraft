//! The painting toggles that have default keys (symmetry L, lazy mouse D, resume previous stroke A, ignore stencil N)
//! and the switch that hides the stencil while a stroke is painted

use serde_json::{Value, json};
use texturecraft_engine::core::Channel;
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, ToolSettings, command_specs, execute};

const RES: f32 = 256.0;

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 256).unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    s.camera.yaw = 0.0;
    s.camera.pitch = 0.0;
    s.update();
    s
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    execute(s, id, &params).unwrap()
}

fn color_at(s: &Session, x: f32, y: f32) -> [f32; 3] {
    let hit = s.pick(Vec2::new(x, y), s.viewport).unwrap();
    s.caches[0].composite.value(Channel::BaseColor, (hit.uv.x * RES) as u32, (hit.uv.y * RES) as u32)
}

/// A green stencil on the Projection tool, painting red when the stencil is ignored
fn projection_with_green_stencil() -> Session {
    let mut s = session();
    run(&mut s, "layer.add.paint", Value::Null);
    s.update();
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 255, 0, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let name = s.import_resource_bytes("green.png", png).unwrap();
    run(&mut s, "tool.material", json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]}));
    run(&mut s, "tool.brush", json!({"size": 40, "hardness": 100, "opacity": 100}));
    run(&mut s, "tool.stencil", json!({"image": name, "scale": 2.0}));
    run(&mut s, "tool.projection", Value::Null);
    s
}

#[test]
fn the_toggles_have_the_reference_default_keys() {
    let key = |id: &str| command_specs().iter().find(|spec| spec.id == id).unwrap_or_else(|| panic!("{id} is a command")).shortcut;
    assert_eq!(key("tool.symmetry"), Some("L"));
    assert_eq!(key("tool.lazyMouse"), Some("D"));
    assert_eq!(key("tool.resumeStroke"), Some("A"));
    assert_eq!(key("tool.ignoreStencil"), Some("N"));

    // Invert greyscale is the existing mask value swap
    assert_eq!(key("tool.swapMaskValue"), Some("X"));
}

#[test]
fn no_two_commands_share_a_default_key() {
    let mut seen = std::collections::BTreeMap::new();
    for spec in command_specs() {
        if let Some(key) = spec.shortcut
            && let Some(other) = seen.insert(key, spec.id)
        {
            panic!("{key} is the default of both {other} and {}", spec.id);
        }
    }
}

#[test]
fn symmetry_toggles_and_keeps_its_settings() {
    let mut s = session();
    run(&mut s, "tool.brush", json!({"symmetryZ": true, "symmetryRadial": 4}));
    let before = s.tool.brush.symmetry;
    let on = s.tool.brush.symmetry.enabled;

    assert_eq!(run(&mut s, "tool.symmetry", Value::Null)["enabled"], !on);
    assert_eq!(s.tool.brush.symmetry.enabled, !on);
    assert_eq!(run(&mut s, "tool.symmetry", json!({"enabled": true}))["enabled"], true);
    assert_eq!(run(&mut s, "tool.symmetry", json!({"enabled": true}))["enabled"], true, "an explicit value does not flip");
    assert!(s.tool.brush.symmetry.enabled);
    run(&mut s, "tool.symmetry", json!({"enabled": false}));
    assert!(!s.tool.brush.symmetry.enabled);

    let mut after = s.tool.brush.symmetry;
    after.enabled = before.enabled;
    assert_eq!(after, before, "only the switch changed");
}

#[test]
fn lazy_mouse_toggles_and_remembers_its_distance() {
    let mut s = session();
    assert_eq!(s.tool.brush.lazy, 0.0);

    let on = run(&mut s, "tool.lazyMouse", Value::Null);
    assert_eq!(on["enabled"], true);
    assert_eq!(s.tool.brush.lazy, 20.0, "the first time it uses the default distance");

    run(&mut s, "tool.brush", json!({"lazy": 75}));
    assert_eq!(run(&mut s, "tool.lazyMouse", Value::Null)["enabled"], false);
    assert_eq!(s.tool.brush.lazy, 0.0);
    run(&mut s, "tool.lazyMouse", Value::Null);
    assert_eq!(s.tool.brush.lazy, 75.0, "on again: the distance from before");

    run(&mut s, "tool.lazyMouse", json!({"enabled": true}));
    assert_eq!(s.tool.brush.lazy, 75.0, "on while on changes nothing");
    run(&mut s, "tool.lazyMouse", json!({"enabled": false}));
    run(&mut s, "tool.lazyMouse", json!({"enabled": false}));
    assert_eq!(s.tool.brush.lazy, 0.0);
    run(&mut s, "tool.lazyMouse", json!({"enabled": true}));
    assert_eq!(s.tool.brush.lazy, 75.0, "off while off does not forget the distance");
}

#[test]
fn resume_and_ignore_stencil_toggle() {
    let mut s = session();
    assert!(!s.tool.resume_stroke && !s.tool.ignore_stencil);

    assert_eq!(run(&mut s, "tool.resumeStroke", Value::Null)["enabled"], true);
    assert!(s.tool.resume_stroke);
    assert_eq!(run(&mut s, "tool.resumeStroke", Value::Null)["enabled"], false);
    run(&mut s, "tool.resumeStroke", json!({"enabled": true}));
    assert!(s.tool.resume_stroke);

    assert_eq!(run(&mut s, "tool.ignoreStencil", Value::Null)["enabled"], true);
    assert!(s.tool.ignore_stencil);
    run(&mut s, "tool.ignoreStencil", json!({"enabled": false}));
    assert!(!s.tool.ignore_stencil);

    let state = run(&mut s, "tool.state", Value::Null);
    assert_eq!(state["resume_stroke"], true);
    assert_eq!(state["ignore_stencil"], false);
}

#[test]
fn toggles_are_not_history_steps() {
    let mut s = projection_with_green_stencil();
    run(&mut s, "tool.stroke", json!({"points": [[200, 200]]}));
    let label = s.undo_label().map(str::to_owned);

    for id in ["tool.symmetry", "tool.lazyMouse", "tool.resumeStroke", "tool.ignoreStencil"] {
        run(&mut s, id, Value::Null);
    }
    run(&mut s, "tool.stencil", json!({"hideWhilePainting": false}));

    assert_eq!(s.undo_label().map(str::to_owned), label, "the stroke is still the last step");
}

#[test]
fn the_projection_tool_paints_its_stencil_unless_it_is_ignored() {
    let mut s = projection_with_green_stencil();
    run(&mut s, "tool.stroke", json!({"points": [[200, 200]]}));
    s.update();
    let stencilled = color_at(&s, 200.0, 200.0);
    assert!(stencilled[1] > 0.85 && stencilled[0] < 0.15, "the stencil's green: {stencilled:?}");

    run(&mut s, "edit.undo", Value::Null);
    s.update();
    run(&mut s, "tool.ignoreStencil", json!({"enabled": true}));
    run(&mut s, "tool.stroke", json!({"points": [[200, 200]]}));
    s.update();
    let plain = color_at(&s, 200.0, 200.0);
    assert!(plain[0] > 0.85 && plain[1] < 0.15, "the brush's red: {plain:?}");

    // The ignored stroke is one history step like any other
    run(&mut s, "edit.undo", Value::Null);
    s.update();
    let undone = color_at(&s, 200.0, 200.0);
    assert!(!(undone[0] > 0.85 && undone[1] < 0.15), "undo takes the stroke back: {undone:?}");
}

#[test]
fn ignoring_the_stencil_lets_the_projection_tool_paint_without_one() {
    let mut s = session();
    run(&mut s, "layer.add.paint", Value::Null);
    s.update();
    run(&mut s, "tool.projection", Value::Null);
    assert!(execute(&mut s, "tool.stroke", &json!({"points": [[200, 200]]})).is_err(), "no stencil, no stroke");

    run(&mut s, "tool.ignoreStencil", json!({"enabled": true}));
    run(&mut s, "tool.stroke", json!({"points": [[200, 200]]}));
}

#[test]
fn the_stencil_is_hidden_while_painting_by_default_and_the_choice_is_kept() {
    let mut s = session();
    assert!(s.tool.hide_stencil_while_painting, "on by default");

    let answer = run(&mut s, "tool.stencil", json!({"hideWhilePainting": false}));
    assert_eq!(answer["hideWhilePainting"], false);
    assert!(!s.tool.hide_stencil_while_painting, "it works without a stencil image");

    let settings = ToolSettings::of(&s.tool);
    let mut other = session();
    settings.apply(&mut other.tool);
    assert!(!other.tool.hide_stencil_while_painting, "kept for the next project");

    // Settings saved before the switch existed start with it on
    let mut old = serde_json::to_value(ToolSettings::of(&session().tool)).unwrap();
    old.as_object_mut().unwrap().remove("hide_stencil_while_painting");
    let old: ToolSettings = serde_json::from_value(old).unwrap();
    assert!(old.hide_stencil_while_painting);
}
