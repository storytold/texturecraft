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
    let [a, b, _, d] = four(&mut s);
    execute(&mut s, "layer.select", &json!({"layers": [b.0, d.0]})).unwrap();
    let depth = s.history_labels().len();
    let r = execute(&mut s, "layer.remove", &Value::Null).unwrap();
    assert_eq!(r["removed"], json!([d.0, b.0]));
    assert_eq!(rows(&s)[..2], ["C", "A"]);
    assert_eq!(selected(&s), ["A"], "the layer under the lowest removed one is selected");
    assert_eq!(s.selected_layer(), Some(a));
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
fn group_puts_the_folder_at_the_topmost_selected_layer_with_gaps_between() {
    let mut s = session();
    execute(&mut s, "textureSet.select", &json!({"ts": 0})).unwrap();
    let [l1, l2, l3, l4, l5] = ["L1", "L2", "L3", "L4", "L5"].map(|n| add(&mut s, n));
    execute(&mut s, "layer.select", &json!({"layers": [l2.0, l4.0]})).unwrap();
    let folder = id(execute(&mut s, "layer.group", &Value::Null).unwrap(), "layer");
    assert_eq!(rows(&s)[..6], ["L5", "Folder 1", "L4", "L2", "L3", "L1"], "the folder sits where L4 was, L3 stays below it");
    assert_eq!(s.doc.project.layer(folder).unwrap().children(), [l2, l4], "L2 stays below L4 inside");
    let stack = &s.doc.project.texture_sets[0].stack;
    assert_eq!(stack[stack.len() - 4..], [l1, l3, folder, l5], "L5 stays above the folder");

    // Same result when the layers are picked in the other order
    s.undo().unwrap();
    execute(&mut s, "layer.select", &json!({"layers": [l4.0, l2.0]})).unwrap();
    execute(&mut s, "layer.group", &Value::Null).unwrap();
    assert_eq!(rows(&s)[..6], ["L5", "Folder 1", "L4", "L2", "L3", "L1"]);
}

#[test]
fn group_of_a_layer_inside_a_folder_and_one_outside_keeps_the_topmost_place() {
    let mut s = session();
    execute(&mut s, "textureSet.select", &json!({"ts": 0})).unwrap();
    let [x, y, z] = ["X", "Y", "Z"].map(|n| add(&mut s, n));
    let outer = id(execute(&mut s, "layer.add.group", &Value::Null).unwrap(), "layer");
    execute(&mut s, "layer.move", &json!({"layers": [y.0, z.0], "above": outer.0, "inside": true})).unwrap();
    // Rows from the top: Folder 1 (Z, Y), X
    assert_eq!(rows(&s)[..4], ["Folder 1", "Z", "Y", "X"]);

    // The topmost selected layer Z is inside the folder: the new folder replaces it there
    execute(&mut s, "layer.select", &json!({"layers": [z.0, x.0]})).unwrap();
    let folder = id(execute(&mut s, "layer.group", &Value::Null).unwrap(), "layer");
    assert_eq!(rows(&s)[..5], ["Folder 1", "Folder 2", "Z", "X", "Y"]);
    assert_eq!(s.doc.project.layer(outer).unwrap().children(), [y, folder], "Z's place in the outer folder");
    assert_eq!(s.doc.project.layer(folder).unwrap().children(), [x, z], "X keeps its place below Z");
    s.undo().unwrap();
    assert_eq!(rows(&s)[..4], ["Folder 1", "Z", "Y", "X"]);

    // X is outside and Y inside: Y is the topmost, so the folder appears inside the outer folder
    execute(&mut s, "layer.select", &json!({"layers": [x.0, y.0]})).unwrap();
    let folder = id(execute(&mut s, "layer.group", &Value::Null).unwrap(), "layer");
    assert_eq!(rows(&s)[..5], ["Folder 1", "Z", "Folder 2", "Y", "X"]);
    assert_eq!(s.doc.project.texture_sets[0].stack.last(), Some(&outer), "the outer folder stays on top");
    assert_eq!(s.doc.project.layer(outer).unwrap().children(), [folder, z]);
    assert_eq!(s.doc.project.layer(folder).unwrap().children(), [x, y]);
}

#[test]
fn select_all_takes_every_layer_of_the_set_with_folder_contents() {
    let mut s = session();
    let [a, b, c, d] = four(&mut s);
    let folder = id(execute(&mut s, "layer.add.group", &Value::Null).unwrap(), "layer");
    execute(&mut s, "layer.move", &json!({"layers": [a.0, b.0], "above": folder.0, "inside": true})).unwrap();
    if let LayerKind::Group { collapsed, .. } = &mut s.doc.project.layer_mut(folder).unwrap().kind {
        *collapsed = true;
    }
    click(&mut s, c, "plain");
    let depth = s.history_labels().len();
    let r = execute(&mut s, "layer.select", &json!({"all": true})).unwrap();
    assert_eq!(selected(&s), ["Folder 1", "B", "A", "D", "C"]);
    assert_eq!(r["layers"].as_array().unwrap().len(), 5);
    assert_eq!(s.selected_layer(), Some(c), "the active layer stays");
    assert_eq!(s.history_labels().len(), depth, "selecting is no undo step");
    assert!(s.is_layer_selected(a) && s.is_layer_selected(d));

    // With nothing selected the top row becomes the active layer
    execute(&mut s, "layer.select", &json!({"layer": d.0})).unwrap();
    execute(&mut s, "layer.select", &json!({"all": true})).unwrap();
    assert_eq!(s.selected_layer(), Some(d), "the active layer stays when it is picked again");
}

#[test]
fn visibility_of_several_layers_follows_the_layer_given_as_lead() {
    let mut s = session();
    let [a, b, c, _] = four(&mut s);
    execute(&mut s, "layer.visible", &json!({"layer": a.0, "visible": false})).unwrap();
    // The active layer is C (shown), but the eye of A (hidden) was clicked: all are shown
    execute(&mut s, "layer.select", &json!({"layers": [a.0, b.0, c.0], "layer": c.0})).unwrap();
    execute(&mut s, "layer.visible", &json!({"layers": [a.0, b.0, c.0], "layer": a.0})).unwrap();
    let shown = |s: &Session| [a, b, c].map(|l| s.doc.project.layer(l).unwrap().visible);
    assert_eq!(shown(&s), [true; 3], "A was hidden, so all are shown");
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

    // Into a folder
    let folder = id(execute(&mut s, "layer.add.group", &Value::Null).unwrap(), "layer");
    execute(&mut s, "layer.move", &json!({"layers": [d.0, b.0], "above": folder.0, "inside": true})).unwrap();
    assert_eq!(s.doc.project.layer(folder).unwrap().children(), [b, d]);
    assert!(execute(&mut s, "layer.move", &json!({"layers": [a.0, c.0], "above": c.0})).is_err(), "not above one of themselves");
}

#[test]
fn selected_layers_moved_to_an_index_keep_their_order() {
    for (pair, expected) in [("B", ["C", "D", "B", "A", "Layer 1"]), ("A", ["C", "D", "A", "B", "Layer 1"])] {
        let mut s = session();
        let [a, b, _, d] = four(&mut s);
        let lower = if pair == "A" { a } else { b };
        execute(&mut s, "layer.move", &json!({"layers": [d.0, lower.0], "index": 2})).unwrap();
        assert_eq!(rows(&s)[..5], expected, "D and {pair} to index 2");
    }
}

#[test]
fn layers_in_and_out_of_a_folder_dropped_below_the_stack_stay_together_at_its_bottom() {
    let mut s = session();
    let [_, b, _, d] = four(&mut s);
    let folder = id(execute(&mut s, "layer.group", &json!({"layer": b.0})).unwrap(), "layer");
    execute(&mut s, "layer.move", &json!({"layers": [d.0, b.0], "index": 0})).unwrap();
    assert_eq!(s.doc.project.texture_sets[0].stack[..2], [b, d], "B at the bottom, D right above it");
    assert!(s.doc.project.layer(folder).unwrap().children().is_empty(), "B left its folder");
}

#[test]
fn a_move_that_changes_nothing_is_not_an_undo_step() {
    let mut s = session();
    let [a, b, c, _] = four(&mut s);
    let depth = s.history_labels().len();
    execute(&mut s, "layer.move", &json!({"layer": b.0, "above": a.0})).unwrap();
    execute(&mut s, "layer.move", &json!({"layers": [c.0, b.0], "above": a.0})).unwrap();
    assert_eq!(s.history_labels().len(), depth, "B already sits above A, and C above B");
    execute(&mut s, "layer.move", &json!({"layer": a.0, "above": c.0})).unwrap();
    assert_eq!(s.history_labels().len(), depth + 1);
}

#[test]
fn an_explicit_layer_acts_on_that_layer_only_while_several_are_selected() {
    let select = |s: &mut Session, b: LayerId, d: LayerId| execute(s, "layer.select", &json!({"layers": [b.0, d.0]})).unwrap();

    let mut s = session();
    let [_, b, _, d] = four(&mut s);
    select(&mut s, b, d);
    execute(&mut s, "layer.remove", &json!({"layer": b.0})).unwrap();
    assert_eq!(rows(&s)[..3], ["D", "C", "A"], "remove");

    let mut s = session();
    let [_, b, _, d] = four(&mut s);
    select(&mut s, b, d);
    execute(&mut s, "layer.duplicate", &json!({"layer": b.0})).unwrap();
    assert_eq!(rows(&s)[..5], ["D", "C", "B copy", "B", "A"], "duplicate");

    let mut s = session();
    let [_, b, _, d] = four(&mut s);
    select(&mut s, b, d);
    let folder = id(execute(&mut s, "layer.group", &json!({"layer": b.0})).unwrap(), "layer");
    assert_eq!(s.doc.project.layer(folder).unwrap().children(), [b], "group");

    let mut s = session();
    let [_, b, _, d] = four(&mut s);
    select(&mut s, b, d);
    execute(&mut s, "layer.visible", &json!({"layer": b.0})).unwrap();
    assert!(!s.doc.project.layer(b).unwrap().visible && s.doc.project.layer(d).unwrap().visible, "visible");
}

#[test]
fn undo_and_redo_end_the_multiple_selection() {
    let mut s = session();
    let [_, b, ..] = four(&mut s);
    let e = add(&mut s, "E");
    execute(&mut s, "layer.select", &json!({"layers": [b.0, e.0]})).unwrap();
    s.undo().unwrap();
    s.redo().unwrap();
    assert_eq!(selected(&s), ["E"], "E is back alone, B is not picked again");
    add(&mut s, "F");
    assert_eq!(s.selected_layers().len(), 1);
    assert!(!s.is_layer_selected(b));
}

#[test]
fn reselecting_an_earlier_active_layer_does_not_bring_back_its_multiple_selection() {
    let mut s = session();
    let [_, b, c, d] = four(&mut s);
    click(&mut s, c, "plain");
    let effect = execute(&mut s, "effect.add.filter", &json!({"target": "content", "filter": "invert"})).unwrap()["effect"].as_u64().unwrap();
    execute(&mut s, "effect.copy", &json!({ "effect": effect })).unwrap();
    execute(&mut s, "layer.select", &json!({"layers": [b.0, d.0], "layer": d.0})).unwrap();
    add(&mut s, "E");
    // Pasting the effect onto D selects D again, alone
    execute(&mut s, "effect.paste", &json!({"layer": d.0})).unwrap();
    assert_eq!(selected(&s), ["D"]);
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
