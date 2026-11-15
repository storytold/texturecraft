//! Several selected layers, as in the reference app: Shift+click selects a range of rows,
//! Ctrl/Cmd+click adds or takes out one layer, and remove, duplicate, group, visibility and move
//! act on every selected layer in one undo step.

use serde_json::{Value, json};
use texturecraft_engine::core::{LayerId, LayerKind};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::layer_selection::stack_rows;
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 256).unwrap()
}

fn id(v: Value, key: &str) -> LayerId {
    LayerId(v[key].as_u64().unwrap())
}

fn add(s: &mut Session, name: &str) -> LayerId {
    id(execute(s, "layer.add.paint", &json!({ "name": name })).unwrap(), "layer")
}

/// Paint layers A (bottom) to D (top) on texture set 0, D selected.
fn four(s: &mut Session) -> [LayerId; 4] {
    execute(s, "textureSet.select", &json!({"ts": 0})).unwrap();
    [add(s, "A"), add(s, "B"), add(s, "C"), add(s, "D")]
}

/// The names of set 0's rows from the top (folder contents included).
fn rows(s: &Session) -> Vec<String> {
    stack_rows(&s.doc.project, 0, false).iter().map(|id| s.doc.project.layer(*id).unwrap().name.clone()).collect()
}

fn names(s: &Session, ids: &[LayerId]) -> Vec<String> {
    ids.iter().map(|id| s.doc.project.layer(*id).unwrap().name.clone()).collect()
}

fn selected(s: &Session) -> Vec<String> {
    names(s, &s.selected_layers())
}

fn click(s: &mut Session, layer: LayerId, how: &str) {
    let params = match how {
        "shift" => json!({"layer": layer.0, "range": true}),
        "ctrl" => json!({"layer": layer.0, "toggle": true}),
        _ => json!({"layer": layer.0}),
    };
    execute(s, "layer.select", &params).unwrap();
}

#[test]
fn shift_click_selects_the_rows_from_the_last_clicked_layer() {
    let mut s = session();
    let [a, b, _, d] = four(&mut s);
    click(&mut s, b, "plain");
    click(&mut s, d, "shift");
    assert_eq!(selected(&s), ["D", "C", "B"]);
    assert_eq!(s.selected_layer(), Some(d), "the clicked layer is the active one");

    // The range still starts at B, not at the active D
    click(&mut s, a, "shift");
    assert_eq!(selected(&s), ["B", "A"]);
    assert_eq!(s.selected_layer(), Some(a));
    assert_eq!(texturecraft_engine::inspect::session(&s)["selectedLayers"], json!([b.0, a.0]));

    // A plain click selects one layer again
    click(&mut s, d, "plain");
    assert_eq!(selected(&s), ["D"]);
}

#[test]
fn ctrl_click_adds_and_takes_out_one_layer() {
    let mut s = session();
    let [a, _, c, _] = four(&mut s);
    click(&mut s, a, "plain");
    click(&mut s, c, "ctrl");
    assert_eq!(selected(&s), ["C", "A"]);
    assert_eq!(s.selected_layer(), Some(c));
    click(&mut s, c, "ctrl");
    assert_eq!(selected(&s), ["A"], "a second Ctrl+click takes the layer out");
    assert_eq!(s.selected_layer(), Some(a), "the remaining layer becomes the active one");
    click(&mut s, a, "ctrl");
    assert_eq!(selected(&s), ["A"], "the last selected layer stays selected");
}

#[test]
fn ctrl_click_on_another_selected_layer_keeps_the_active_one_and_the_range_start() {
    let mut s = session();
    let [a, b, c, d] = four(&mut s);
    click(&mut s, b, "plain");
    click(&mut s, d, "shift");
    click(&mut s, c, "ctrl");
    assert_eq!(selected(&s), ["D", "B"]);
    assert_eq!(s.selected_layer(), Some(d), "D stays the active layer");
    click(&mut s, a, "shift");
    assert_eq!(selected(&s), ["B", "A"], "the range still starts at B");
}

#[test]
fn a_range_covers_the_shown_rows_only() {
    let mut s = session();
    let [a, b, c, d] = four(&mut s);
    execute(&mut s, "layer.select", &json!({"layers": [b.0, c.0]})).unwrap();
    let folder = id(execute(&mut s, "layer.group", &Value::Null).unwrap(), "layer");
    assert_eq!(rows(&s)[..5], ["D", "Folder 1", "C", "B", "A"]);

    click(&mut s, d, "plain");
    click(&mut s, a, "shift");
    assert_eq!(selected(&s), ["D", "Folder 1", "C", "B", "A"], "an open folder's layers are in the range");

    if let LayerKind::Group { collapsed, .. } = &mut s.doc.project.layer_mut(folder).unwrap().kind {
        *collapsed = true;
    }
    click(&mut s, d, "plain");
    click(&mut s, a, "shift");
    assert_eq!(selected(&s), ["D", "Folder 1", "A"], "a collapsed folder's layers are not");
}

#[test]
fn another_selected_layer_ends_the_multiple_selection() {
    let mut s = session();
    let [a, _, c, _] = four(&mut s);
    click(&mut s, a, "plain");
    click(&mut s, c, "ctrl");
    add(&mut s, "E");
    assert_eq!(selected(&s), ["E"], "a new layer is selected alone");
}

#[test]
fn delete_removes_every_selected_layer_in_one_undo_step() {
    let mut s = session();
    let [_, b, _, d] = four(&mut s);
    execute(&mut s, "layer.select", &json!({"layers": [b.0, d.0]})).unwrap();
    let depth = s.history_labels().len();
    let r = execute(&mut s, "layer.remove", &Value::Null).unwrap();
    assert_eq!(r["removed"], json!([d.0, b.0]));
    assert_eq!(rows(&s)[..2], ["C", "A"]);
    assert_eq!(s.selected_layers().len(), 1);
    assert_eq!(s.history_labels().len(), depth + 1, "one undo step");
    s.undo().unwrap();
    assert_eq!(rows(&s)[..4], ["D", "C", "B", "A"]);
}

#[test]
fn a_folder_and_its_own_layer_selected_remove_together() {
    let mut s = session();
    let [a, b, _, _] = four(&mut s);
    let folder = id(execute(&mut s, "layer.group", &json!({"layer": b.0})).unwrap(), "layer");
    execute(&mut s, "layer.select", &json!({"layers": [folder.0, b.0, a.0]})).unwrap();
    execute(&mut s, "layer.remove", &Value::Null).unwrap();
    assert_eq!(rows(&s)[..2], ["D", "C"]);
    assert!(s.doc.project.layer(b).is_err() && s.doc.project.layer(a).is_err());
}

#[test]
fn duplicate_copies_every_selected_layer_above_itself() {
    let mut s = session();
    let [_, b, _, d] = four(&mut s);
    execute(&mut s, "layer.select", &json!({"layers": [d.0, b.0], "layer": b.0})).unwrap();
    execute(&mut s, "layer.duplicate", &Value::Null).unwrap();
    assert_eq!(rows(&s)[..6], ["D copy", "D", "C", "B copy", "B", "A"]);
    assert_eq!(selected(&s), ["D copy", "B copy"], "the copies are selected");
    assert_eq!(names(&s, &[s.selected_layer().unwrap()]), ["B copy"], "the active layer's copy is the active one");
    s.undo().unwrap();
    assert_eq!(rows(&s)[..4], ["D", "C", "B", "A"]);
}

#[test]
fn group_puts_the_selected_layers_into_one_folder_in_their_order() {
    let mut s = session();
    let [a, _, c, _] = four(&mut s);
    execute(&mut s, "layer.select", &json!({"layers": [a.0, c.0]})).unwrap();
    let folder = id(execute(&mut s, "layer.group", &Value::Null).unwrap(), "layer");
    assert_eq!(rows(&s)[..5], ["D", "Folder 1", "C", "A", "B"], "the folder takes the top layer's place");
    assert_eq!(s.doc.project.layer(folder).unwrap().children(), [a, c], "bottom to top inside the folder");
    assert_eq!(s.selected_layers(), [folder]);
    s.undo().unwrap();
    assert_eq!(rows(&s)[..4], ["D", "C", "B", "A"]);
}

#[test]
fn visibility_toggles_every_selected_layer_the_way_of_the_active_one() {
    let mut s = session();
    let [a, b, c, _] = four(&mut s);
    execute(&mut s, "layer.visible", &json!({"layer": a.0, "visible": false})).unwrap();
    execute(&mut s, "layer.select", &json!({"layers": [a.0, b.0, c.0], "layer": c.0})).unwrap();
    let depth = s.history_labels().len();
    execute(&mut s, "layer.visible", &Value::Null).unwrap();
    let shown = |s: &Session| [a, b, c].map(|l| s.doc.project.layer(l).unwrap().visible);
    assert_eq!(shown(&s), [false; 3], "C was shown, so all are hidden");
    assert_eq!(s.history_labels().len(), depth + 1, "one undo step");
    execute(&mut s, "layer.visible", &json!({"layers": [a.0, b.0], "visible": true})).unwrap();
    assert_eq!(shown(&s), [true, true, false]);
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(shown(&s), [false, true, true]);
}

#[test]
fn selected_layers_move_together_keeping_their_order() {
    let mut s = session();
    let [a, b, c, d] = four(&mut s);
    execute(&mut s, "layer.move", &json!({"layers": [a.0, c.0], "above": d.0})).unwrap();
    assert_eq!(rows(&s)[..4], ["C", "A", "D", "B"]);
    s.undo().unwrap();
    assert_eq!(rows(&s)[..4], ["D", "C", "B", "A"]);

    // Into a folder, and to the bottom of the stack
    let folder = id(execute(&mut s, "layer.add.group", &Value::Null).unwrap(), "layer");
    execute(&mut s, "layer.move", &json!({"layers": [d.0, b.0], "above": folder.0, "inside": true})).unwrap();
    assert_eq!(s.doc.project.layer(folder).unwrap().children(), [b, d]);
    execute(&mut s, "layer.move", &json!({"layers": [d.0, b.0], "index": 0})).unwrap();
    assert_eq!(s.doc.project.layer(folder).unwrap().children(), [b, d], "`index` moves within each layer's own list");
    assert!(execute(&mut s, "layer.move", &json!({"layers": [a.0, c.0], "above": c.0})).is_err(), "not above one of themselves");
}

#[test]
fn bad_layer_lists_are_errors() {
    let mut s = session();
    let [a, ..] = four(&mut s);
    let other = id(execute(&mut s, "textureSet.select", &json!({"ts": 1})).and_then(|_| execute(&mut s, "layer.add.paint", &Value::Null)).unwrap(), "layer");
    for command in ["layer.select", "layer.remove", "layer.duplicate", "layer.group", "layer.visible", "layer.move"] {
        for layers in [json!("x"), json!([]), json!([u64::MAX]), json!([a.0, other.0]), json!([a.0, "x"])] {
            assert!(execute(&mut s, command, &json!({"layers": layers, "above": null})).is_err(), "{command} {layers}");
        }
    }
    assert!(execute(&mut s, "layer.select", &json!({"layers": [a.0], "layer": other.0})).is_err(), "the active layer must be listed");
}
