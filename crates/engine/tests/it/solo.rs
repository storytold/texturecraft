//! Solo: a view-only preview of one layer. The composite shows that layer alone (optionally at
//! full coverage); saving, exporting and the history never see it.

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, LayerId};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, Solo, execute};

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 128).unwrap();
    s.update();
    s
}

fn add_fill(s: &mut Session, color: [f32; 3], roughness: f32) -> LayerId {
    let r = execute(s, "layer.add.fill", &json!({"color": color, "roughness": roughness})).unwrap();
    LayerId(r["layer"].as_u64().unwrap())
}

/// A red fill below a green fill that a black mask hides everywhere, and the green one.
fn masked_stack() -> (Session, LayerId, LayerId) {
    let mut s = session();
    let red = add_fill(&mut s, [1.0, 0.0, 0.0], 0.2);
    let green = add_fill(&mut s, [0.0, 1.0, 0.0], 0.8);
    execute(&mut s, "mask.add.black", &json!({"layer": green.0})).unwrap();
    s.update();
    (s, red, green)
}

/// A texel the mesh covers.
fn covered(s: &Session) -> (u32, u32) {
    let g = &s.caches[0].gbuf;
    let i = g.tri.iter().position(|t| *t != texturecraft_engine::geom::NO_TRI).unwrap();
    (i as u32 % g.width, i as u32 / g.width)
}

fn base(s: &mut Session) -> [f32; 3] {
    s.update();
    let (x, y) = covered(s);
    s.caches[0].composite.value(Channel::BaseColor, x, y)
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 2e-3)
}

#[test]
fn solo_shows_only_that_layer_and_ignoring_the_mask_shows_it_everywhere() {
    let (mut s, red, green) = masked_stack();
    assert!(close(base(&mut s), [1.0, 0.0, 0.0]), "the black mask hides the green layer");

    s.set_solo(Some(Solo { layer: green, ignore_mask: false }));
    assert!(close(base(&mut s), Channel::BaseColor.default_value()), "alone and masked away: nothing is left");
    s.set_solo(Some(Solo { layer: green, ignore_mask: true }));
    assert!(close(base(&mut s), [0.0, 1.0, 0.0]), "the mask is ignored: the whole material shows");
    s.set_solo(Some(Solo { layer: red, ignore_mask: true }));
    assert!(close(base(&mut s), [1.0, 0.0, 0.0]), "the other layer alone");
}

#[test]
fn the_solo_composite_equals_the_layer_alone_without_its_mask() {
    let (mut s, _, green) = masked_stack();
    s.set_solo(Some(Solo { layer: green, ignore_mask: true }));
    s.update();

    // The same layer in a project of its own, without mask and without the layer below
    let mut alone = session();
    add_fill(&mut alone, [0.0, 1.0, 0.0], 0.8);
    alone.update();
    assert!(s.caches[0].composite == alone.caches[0].composite, "every channel, every texel");
}

#[test]
fn the_solo_changes_neither_the_document_nor_the_history_nor_the_saved_file() {
    let (mut s, _, green) = masked_stack();
    let (project, labels, saved) =
        (s.doc.project.clone(), s.history_labels(), Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap().doc.project);
    s.set_solo(Some(Solo { layer: green, ignore_mask: true }));
    s.update();
    assert_eq!(s.doc.project, project);
    assert_eq!(s.history_labels(), labels);
    assert_eq!(Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap().doc.project, saved, "the file holds every layer and the mask");
}

#[test]
fn exports_hold_the_whole_stack_while_a_layer_is_soloed_and_the_solo_stays() {
    let (mut s, _, green) = masked_stack();
    let opts = texturecraft_engine::file_cmds::export_options(&s, &json!({"preset": "Document channels + Normal + AO", "format": "png"}), "t").unwrap();
    let whole = s.export_files(&[0], &opts).unwrap();
    s.set_solo(Some(Solo { layer: green, ignore_mask: true }));
    s.update();
    let revision = s.caches[0].revision;
    let soloed = s.export_files(&[0], &opts).unwrap();
    assert_eq!(s.pending_tiles(), 0, "export leaves the viewport cache ready to draw");
    assert_eq!(s.caches[0].revision, revision);
    assert_eq!(soloed.len(), whole.len());
    assert!(soloed.iter().zip(&whole).all(|(a, b)| a.name == b.name && a.bytes == b.bytes), "the files are the whole stack's");
    assert_eq!(s.solo, Some(Solo { layer: green, ignore_mask: true }), "the view keeps showing the solo");
    assert!(close(base(&mut s), [0.0, 1.0, 0.0]), "and the viewport composite is the solo again");
}

#[test]
fn turning_the_solo_off_restores_the_original_composite() {
    let (mut s, _, green) = masked_stack();
    let before = s.caches[0].composite.clone();
    s.set_solo(Some(Solo { layer: green, ignore_mask: true }));
    s.update();
    assert!(s.caches[0].composite != before);
    s.set_solo(None);
    s.update();
    assert!(s.caches[0].composite == before);
}

#[test]
fn edits_while_soloed_show_in_the_solo_and_in_the_whole_stack_afterwards() {
    let (mut s, red, _) = masked_stack();
    s.set_solo(Some(Solo { layer: red, ignore_mask: false }));
    s.update();
    execute(&mut s, "layer.fill.set", &json!({"layer": red.0, "channel": "baseColor", "color": [0.0, 0.0, 1.0]})).unwrap();
    assert!(close(base(&mut s), [0.0, 0.0, 1.0]), "the edited layer, alone");
    s.set_solo(None);
    assert!(close(base(&mut s), [0.0, 0.0, 1.0]), "the stack still hides the green layer behind its mask: blue shows");
}

#[test]
fn removing_the_soloed_layer_ends_the_solo() {
    let (mut s, _, green) = masked_stack();
    s.set_solo(Some(Solo { layer: green, ignore_mask: true }));
    execute(&mut s, "layer.remove", &json!({"layer": green.0})).unwrap();
    s.update();
    assert_eq!(s.solo, None);
    assert!(close(base(&mut s), [1.0, 0.0, 0.0]), "the whole stack again");
}

#[test]
fn the_view_solo_command_sets_reports_and_ends_the_solo() {
    let (mut s, _, green) = masked_stack();
    let r = execute(&mut s, "view.solo", &json!({"layer": green.0, "ignoreMask": true})).unwrap();
    assert_eq!(r, json!({"layer": green.0, "ignoreMask": true}));
    assert_eq!(texturecraft_engine::inspect::session(&s)["solo"], json!({"layer": green.0, "ignoreMask": true}));

    // The selected layer is the default
    execute(&mut s, "view.solo", &json!({"layer": null})).unwrap();
    assert_eq!(s.solo, None);
    execute(&mut s, "layer.select", &json!({"layer": green.0})).unwrap();
    let r = execute(&mut s, "view.solo", &json!({})).unwrap();
    assert_eq!(r, json!({"layer": green.0, "ignoreMask": false}));
    assert!(execute(&mut s, "view.solo", &json!({"layer": 999_999})).is_err());
    assert_eq!(texturecraft_engine::inspect::session(&s)["solo"]["layer"], json!(green.0));
    execute(&mut s, "view.solo", &json!({"layer": null})).unwrap();
    assert_eq!(texturecraft_engine::inspect::session(&s)["solo"], Value::Null);
}
