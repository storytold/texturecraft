//! A malformed colour param is an error, not the command's default colour, and changes nothing.
//! So is a malformed point or direction (`center`, `direction`, colour balance offsets).

use serde_json::{Value, json};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute, inspect};

/// A sphere project with a fill layer selected, so the layer and effect commands have a target.
fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 24, 12), "builtin:sphere", None, 128).unwrap();
    execute(&mut s, "layer.add.fill", &json!({})).unwrap();
    s
}

/// What a command could have changed: the session as inspected, and the tool's brush material.
fn state(s: &Session) -> Value {
    let mut inspected = inspect::session(s);
    if let Some(o) = inspected.as_object_mut() {
        o.remove("memoryBytes");
        o.remove("pendingTiles");
    }
    json!({"session": inspected, "maskValue": s.tool.mask_value, "material": format!("{:?} {:?}", s.tool.values, s.tool.enabled)})
}

#[test]
fn a_malformed_colour_is_an_error() {
    let rows: [(&str, &str, Value, &str); 21] = [
        ("fill layer, short form", "layer.add.fill", json!({"color": "#fff"}), "color"),
        ("fill layer, a word for a component", "layer.add.fill", json!({"color": [1, "bad", 0]}), "color"),
        ("content fill effect, not hex", "effect.add.fill", json!({"target": "content", "color": "#zzzzzz"}), "color"),
        ("fill source, a name", "layer.fill.set", json!({"channel": "baseColor", "color": "white"}), "color"),
        ("fill source, no color and a bad value", "layer.fill.set", json!({"channel": "baseColor", "value": "white"}), "value"),
        ("procedural low, too short", "layer.fill.set", json!({"channel": "baseColor", "procedural": "clouds", "low": [0.5, 0.5]}), "low"),
        ("procedural high, a name", "layer.fill.set", json!({"channel": "baseColor", "procedural": "clouds", "high": "white"}), "high"),
        ("brush, five digits", "tool.material", json!({"channel": "baseColor", "color": "c8102"}), "color"),
        ("brush, no color and a bad value", "tool.material", json!({"channel": "baseColor", "value": [1, "bad", 0]}), "value"),
        ("brush with a mask value, short form", "tool.material", json!({"maskValue": 0.25, "channel": "baseColor", "color": "#fff"}), "color"),
        ("ID colour mask, a name", "mask.add.colorSelection", json!({"color": "red"}), "color"),
        ("scatter, alpha", "display.set", json!({"scatterColor": "#c8102e80"}), "scatterColor"),
        ("scatter with an exposure, a null component", "display.set", json!({"exposure": 2, "scatterColor": [0.5, null, 0.5]}), "scatterColor"),
        ("replace colour filter, an object", "effect.add.filter", json!({"filter": "replaceColor", "to": {"r": 1}}), "to"),
        ("replace colour filter, bad from", "effect.add.filter", json!({"filter": "replaceColor", "from": "bad"}), "from"),
        ("replace colour filter, good from and bad to", "effect.add.filter", json!({"filter": "replaceColor", "from": "#000000", "to": "bad"}), "to"),
        ("colour balance, a hex offset", "effect.add.filter", json!({"filter": "colorBalance", "shadows": "#ffffff"}), "shadows"),
        ("text, three digits", "resource.text", json!({"text": "A", "color": "#12f"}), "color"),
        ("stencil centre, a word", "tool.stencil", json!({"center": [0.5, "x"]}), "center"),
        ("direction generator, hex", "effect.add.generator", json!({"generator": "direction", "direction": "#ffffff"}), "direction"),
        ("gradient map, not hex", "effect.add.filter", json!({"filter": "gradientMap", "colors": ["#000000", "#ffffzz"]}), "colors"),
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

/// `value` is the fallback for a missing `color`: with a good `color` it is not read at all.
#[test]
fn a_good_colour_is_not_failed_by_a_bad_value() {
    let rows: [(&str, &str, Value); 2] = [
        ("fill source", "layer.fill.set", json!({"channel": "baseColor", "color": "#c8102e", "value": "bad"})),
        ("brush", "tool.material", json!({"channel": "baseColor", "color": [0.5, 0.25, 1], "value": "bad"})),
    ];
    for (name, cmd, params) in rows {
        let mut s = session();
        if let Err(e) = execute(&mut s, cmd, &params) {
            panic!("{name}: {cmd} {params} failed: {e}");
        }
    }
}
