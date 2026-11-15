//! Points and directions (`tool.stencil` `center`, the direction generator's `direction`, color
//! balance offsets) are lists of numbers: anything else is an error that changes nothing.

use serde_json::{Value, json};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, StencilState, execute, inspect};

/// A sphere project with a fill layer selected and a stencil on the brush.
fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 24, 12), "builtin:sphere", None, 128).unwrap();
    execute(&mut s, "layer.add.fill", &json!({})).unwrap();
    s.tool.stencil = Some(StencilState::new("stencil.png"));
    s
}

/// What a command could have changed: the session as inspected, and the brush's stencil.
fn state(s: &Session) -> Value {
    let mut inspected = inspect::session(s);
    if let Some(o) = inspected.as_object_mut() {
        o.remove("memoryBytes");
        o.remove("pendingTiles");
    }
    json!({"session": inspected, "stencil": s.tool.stencil})
}

#[test]
fn a_malformed_point_or_direction_is_an_error() {
    let rows: [(&str, &str, Value, &str); 7] = [
        ("stencil centre, a word", "tool.stencil", json!({"center": "left"}), "center"),
        ("stencil centre, a word inside", "tool.stencil", json!({"center": [0.5, "x"]}), "center"),
        ("stencil centre, a hex colour", "tool.stencil", json!({"center": "#ffffff"}), "center"),
        ("stencil centre with a scale, three numbers", "tool.stencil", json!({"scale": 2, "center": [0.5, 0.5, 0.5]}), "center"),
        ("direction, a hex colour", "effect.add.generator", json!({"generator": "direction", "direction": "#808080"}), "direction"),
        ("direction, one number", "effect.add.generator", json!({"generator": "direction", "direction": 0.5}), "direction"),
        ("colour balance, a hex offset", "effect.add.filter", json!({"filter": "colorBalance", "target": "content", "shadows": "#ffffff"}), "shadows"),
    ];
    for (name, cmd, params, key) in rows {
        let mut s = session();
        let before = state(&s);
        match execute(&mut s, cmd, &params) {
            Ok(r) => panic!("{name}: {cmd} {params} gave {r}"),
            Err(e) => assert!(e.to_string().contains(&format!("`{cmd}`")) && e.to_string().contains(&format!("`{key}`")), "{name}: {e}"),
        }
        assert_eq!(state(&s), before, "{name}: a failed {cmd} changed the session");
    }
}

#[test]
fn points_and_directions_given_as_numbers_still_work() {
    let rows: [(&str, &str, Value); 3] = [
        ("stencil centre", "tool.stencil", json!({"center": [0.25, -0.5]})),
        ("direction", "effect.add.generator", json!({"generator": "direction", "direction": [0, 1, 0]})),
        ("colour balance offsets", "effect.add.filter", json!({"filter": "colorBalance", "target": "content", "shadows": [0.1, 0, -0.1], "midtones": null})),
    ];
    for (name, cmd, params) in rows {
        let mut s = session();
        if let Err(e) = execute(&mut s, cmd, &params) {
            panic!("{name}: {cmd} {params} failed: {e}");
        }
    }
    let mut s = session();
    execute(&mut s, "tool.stencil", &json!({"center": [0.25, -0.5]})).unwrap();
    assert_eq!(s.tool.stencil.map(|st| st.center), Some([0.25, -0.5]));
}
