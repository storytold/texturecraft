use serde_json::json;
use texturecraft_engine::{Session, execute, geom::primitives};

fn session() -> Session {
    Session::new_project("Swatches", primitives::sphere(1.0, 16, 8), "builtin:sphere", None, 128).unwrap()
}

#[test]
fn swatches_are_added_saved_with_the_project_removed_and_undone() {
    let mut s = session();
    let plain = s.to_project_bytes().unwrap();
    let saved: serde_json::Value = serde_json::to_value(&s.doc.project).unwrap();
    assert!(saved.get("swatches").is_none(), "a project without swatches saves as before");

    execute(&mut s, "swatch.add", &json!({"color": [0.8, 0.25, 0.07]})).unwrap();
    let added = execute(&mut s, "swatch.add", &json!({"color": [2.0, -1.0, 0.5]})).unwrap();
    assert_eq!(added["index"], 1);
    assert_eq!(s.doc.project.swatches, vec![[0.8, 0.25, 0.07], [1.0, 0.0, 0.5]], "kept in order, clamped to 0..1");
    assert_eq!(execute(&mut s, "project.inspect", &json!({})).unwrap()["swatches"][1], json!([1.0, 0.0, 0.5]));

    let reopened = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();
    assert_eq!(reopened.doc.project.swatches, s.doc.project.swatches, "saved per project");
    assert!(Session::from_project_bytes(&plain).unwrap().doc.project.swatches.is_empty());

    execute(&mut s, "swatch.remove", &json!({"index": 0})).unwrap();
    assert_eq!(s.doc.project.swatches, vec![[1.0, 0.0, 0.5]]);
    execute(&mut s, "edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.project.swatches.len(), 2, "a removal is one undo step");

    assert!(execute(&mut s, "swatch.add", &json!({})).is_err(), "a colour is required");
    assert!(execute(&mut s, "swatch.remove", &json!({"index": 2})).is_err(), "no such swatch");
    while s.doc.project.swatches.len() < texturecraft_core::MAX_SWATCHES {
        execute(&mut s, "swatch.add", &json!({"color": "#336699"})).unwrap();
    }
    assert!(execute(&mut s, "swatch.add", &json!({"color": "#336699"})).is_err(), "the list is capped");
}
