//! The folder in Properties: CONTENTS with Scale and Rotation over every fill inside (a drag is one undo step, the round reset drops them) and Randomize seeds (the reset also puts back the seeds it changed); an empty folder has no properties

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::core::{Channel, ContentsOffset, LayerId, LayerKind, Source};
use texturecraft_engine::execute;
use texturecraft_ui_egui::TextureCraftApp;

use crate::{BAKE_RESOLUTION, DRAG_STEP_FRAMES, FRAME_SECONDS, PROPERTIES_WINDOW, SETTLE_FRAMES, SLIDER_END_INSET, STARTUP_FRAMES, TEXTURE_RESOLUTION};

/// How near a dragged Scale and Rotation land to the track's value (a drag ends a pixel or so off it)
const SCALE_TOLERANCE: f32 = 0.05;
const ROTATION_TOLERANCE_DEGREES: f32 = 3.0;

/// Hovers a drag passes through on its way, so the slider follows it as a hand would
const DRAG_MOVES: usize = 3;

/// A folder holding a Clouds fill (or nothing), the folder selected: (harness, folder, fill)
fn folder_harness(with_fill: bool) -> (Harness<'static, TextureCraftApp>, LayerId, LayerId) {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let fill = LayerId(execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].as_u64().unwrap());
    execute(&mut session, "layer.fill.set", &json!({"layer": fill.0, "channel": "baseColor", "procedural": "Clouds", "seed": 7})).unwrap();
    let folder = if with_fill {
        LayerId(execute(&mut session, "layer.group", &json!({"layer": fill.0})).unwrap()["layer"].as_u64().unwrap())
    } else {
        LayerId(execute(&mut session, "layer.add.group", &json!({})).unwrap()["layer"].as_u64().unwrap())
    };
    execute(&mut session, "layer.select", &json!({"layer": folder.0, "target": "content"})).unwrap();
    let mut harness = Harness::builder().with_size(PROPERTIES_WINDOW).with_step_dt(FRAME_SECONDS).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    (harness, folder, fill)
}

fn offset(harness: &Harness<'_, TextureCraftApp>, folder: LayerId) -> ContentsOffset {
    harness.state().session.doc.project.contents_offset(folder)
}

/// Drags the slider labelled `label` from its middle to `to` (0 = its left end, 1 = its right end), hovering on the way
fn drag(harness: &mut Harness<'_, TextureCraftApp>, label: &str, to: f32) {
    let track = harness.get_by_role_and_label(Role::Slider, label).rect();
    let at = |t: f32| egui::pos2(track.left() + SLIDER_END_INSET + (track.width() - 2.0 * SLIDER_END_INSET) * t, track.center().y);
    harness.drag_at(at(0.5));
    harness.run_steps(DRAG_STEP_FRAMES);
    for step in 1..=DRAG_MOVES {
        harness.hover_at(at(0.5 + (to - 0.5) * step as f32 / DRAG_MOVES as f32));
        harness.run_steps(DRAG_STEP_FRAMES);
    }
    harness.drop_at(at(to));
    harness.run_steps(SETTLE_FRAMES);
}

#[test]
fn scale_and_rotation_drags_are_one_undo_step_each_and_the_reset_drops_them() {
    let (mut harness, folder, fill) = folder_harness(true);
    let steps = harness.state().session.history_labels().len();
    drag(&mut harness, "Scale", 1.0);
    let scaled = offset(&harness, folder);
    assert!((scaled.scale - 4.0).abs() < SCALE_TOLERANCE, "the right end is x4: {scaled:?}");
    assert_eq!(harness.state().session.history_labels().len(), steps + 1, "one undo step for the drag");

    drag(&mut harness, "Rotation", 0.75);
    let turned = offset(&harness, folder);
    assert!((turned.rotation - 90.0).abs() < ROTATION_TOLERANCE_DEGREES && turned.scale == scaled.scale, "{turned:?}");
    assert_eq!(harness.state().session.history_labels().len(), steps + 2);
    let LayerKind::Fill(own) = &harness.state().session.doc.project.layer(fill).unwrap().kind else { panic!("not a fill") };
    assert_eq!(own.projection, Default::default(), "the fill keeps its own projection");

    harness.get_by_role_and_label(Role::Button, "Contents: Reset to defaults").click();
    harness.run_steps(SETTLE_FRAMES);

    assert!(offset(&harness, folder).is_none(), "the reset drops the offset");
    assert_eq!(harness.state().session.history_labels().len(), steps + 3);
}

#[test]
fn randomize_seeds_is_one_undo_step_the_contents_reset_puts_them_back_and_an_empty_folder_has_no_properties() {
    let (mut harness, _, fill) = folder_harness(true);
    let seed = |harness: &Harness<'_, TextureCraftApp>| match &harness.state().session.doc.project.layer(fill).unwrap().kind {
        LayerKind::Fill(fill_layer) => match fill_layer.sources.get(&Channel::BaseColor) {
            Some(Source::Procedural { pattern, .. }) => pattern.seed,
            other => panic!("not a pattern: {other:?}"),
        },
        other => panic!("not a fill: {other:?}"),
    };
    let steps = harness.state().session.history_labels().len();
    harness.get_by_role_and_label(Role::Button, "Randomize seeds").click();
    harness.run_steps(SETTLE_FRAMES);

    assert_ne!(seed(&harness), 7);
    assert_eq!(harness.state().session.history_labels().len(), steps + 1);

    // The Contents reset lights up and puts the seed back, in one undo step
    let reset = harness.get_by_role_and_label(Role::Button, "Contents: Reset to defaults");
    assert!(!reset.accesskit_node().is_disabled(), "randomized seeds light the reset");

    reset.click();
    harness.run_steps(SETTLE_FRAMES);

    assert_eq!(seed(&harness), 7, "the seed from before Randomize seeds");
    assert_eq!(harness.state().session.history_labels().len(), steps + 2);
    assert!(harness.get_by_role_and_label(Role::Button, "Contents: Reset to defaults").accesskit_node().is_disabled(), "nothing left to reset");

    let (harness, _, _) = folder_harness(false);
    assert!(harness.query_by_label("No properties found").is_some());
    assert!(harness.query_by_role_and_label(Role::Slider, "Scale").is_none());
}

/// An empty folder's note starts at the Properties header name's left edge, as the note with no project open does (it was centred)
#[test]
fn an_empty_folders_note_starts_at_its_header_names_left_edge() {
    let (harness, _, _) = folder_harness(false);

    let painted_left = |wanted: &dyn Fn(&str) -> bool| {
        harness.output().shapes.iter().find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if wanted(text.galley.text()) => Some(text.pos.x),
            _ => None,
        })
    };
    let note = painted_left(&|text| text == "No properties found");
    let header = painted_left(&|text| text.starts_with("PROPERTIES"));

    assert!(note.is_some() && note == header, "the note starts at {note:?}, the header name at {header:?}");
}
