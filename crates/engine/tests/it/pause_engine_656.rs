//! Pause Engine Computation: the flag toggles, edits still reach the document and history, and
//! anything that needs results (export) composites regardless.

use serde_json::{Value, json};
use texturecraft_engine::{execute, sample_session};

#[test]
fn pause_toggles_and_reports_pending_tiles() {
    let mut s = sample_session(128, 64).unwrap();
    s.update();
    assert!(!s.paused);

    let on = execute(&mut s, "viewport.pauseEngine", &Value::Null).unwrap();
    assert_eq!(on["paused"], true);
    assert!(s.paused);

    execute(&mut s, "layer.add.fill", &json!({})).unwrap();
    let still = execute(&mut s, "viewport.pauseEngine", &json!({"paused": true})).unwrap();
    assert_eq!(still["paused"], true, "an explicit value does not toggle");
    assert!(still["pendingTiles"].as_u64().unwrap() > 0, "the edit waits for the engine");

    let off = execute(&mut s, "viewport.pauseEngine", &json!({"paused": false})).unwrap();
    assert_eq!(off["paused"], false);
}

#[test]
fn edits_while_paused_keep_history_and_export_is_current() {
    let mut s = sample_session(128, 64).unwrap();
    s.update();
    execute(&mut s, "viewport.pauseEngine", &Value::Null).unwrap();

    let layers = s.doc.project.layers.len();
    execute(&mut s, "layer.add.fill", &json!({})).unwrap();
    assert_eq!(s.doc.project.layers.len(), layers + 1, "the document changed while paused");
    assert!(s.pending_tiles() > 0);

    // Export composites what it needs even when paused
    let opts = texturecraft_engine::file_cmds::export_options(&s, &json!({"preset": "Document channels + Normal + AO", "format": "png"}), "t").unwrap();
    let files = s.export_files(&[0], &opts).unwrap();
    assert!(!files.is_empty());
    assert_eq!(s.pending_tiles(), 0, "export brought the textures up to date");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert_eq!(s.doc.project.layers.len(), layers, "the edit is one history step");
}
