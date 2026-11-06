//! Bake mode: while a bake runs the 3D view shows the maps landing on the model, the previous view
//! mode comes back when the bake ends, is cancelled or Bake mode is left, and painting is paused.

use egui_kittest::Harness;
use texturecraft_engine::ViewMode;
use texturecraft_engine::core::{BakeResolution, BakedMap};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::Mode;

const MAPS: [BakedMap; 3] = [BakedMap::Id, BakedMap::AmbientOcclusion, BakedMap::Height];

fn harness() -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    for texture_set in &mut session.doc.project.texture_sets {
        texture_set.bake = Default::default();
        texture_set.bake.common.resolution = BakeResolution::R64;
        texture_set.bake.maps.ao.parameters.rays.rays = 8;
        texture_set.bake.maps.select(MAPS);
    }
    session.view = ViewMode::UvChecker;
    let mut harness = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(3);
    harness
}

/// Frames until the bake is gone, with the view mode after each one.
fn run_to_the_end(harness: &mut Harness<'static, TextureCraftApp>) -> Vec<ViewMode> {
    let mut seen = Vec::new();
    for _ in 0..4000 {
        harness.run_steps(1);
        seen.push(harness.state().session.view);
        if harness.state().bake.is_none() {
            return seen;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("the bake did not finish");
}

#[test]
fn the_view_shows_each_map_as_it_lands_and_the_previous_mode_comes_back() {
    let mut harness = harness();
    harness.state_mut().start_bake(vec![0]);
    // Right away: the first map shows, still neutral
    assert_eq!(harness.state().session.view, ViewMode::MeshMap(MAPS[0]));
    harness.run_steps(1);
    assert!(harness.state().bake_status().is_some_and(|status| status.contains("/3")), "the status line names the stage");
    let seen = run_to_the_end(&mut harness);
    // Every requested map was shown at some frame, in request order
    let mut shown: Vec<BakedMap> = Vec::new();
    for view in seen {
        if let ViewMode::MeshMap(map) = view
            && shown.last() != Some(&map)
        {
            shown.push(map);
        }
    }
    assert_eq!(shown, MAPS);
    assert_eq!(harness.state().session.view, ViewMode::UvChecker, "the mode in use before the bake is back");
    assert!(!harness.state().session.bake_preview_active());
    assert!(MAPS.iter().all(|map| harness.state().session.doc.baked[&0].maps.contains_key(map)));
}

#[test]
fn cancelling_restores_the_view_and_leaves_the_document_alone() {
    let mut harness = harness();
    let before = harness.state().session.doc.baked.get(&0).cloned();
    harness.state_mut().start_bake(vec![0]);
    harness.run_steps(2);
    assert!(harness.state().session.bake_preview_active());
    let (at, viewport) = (texturecraft_engine::geom::glam::Vec2::new(5.0, 5.0), texturecraft_engine::geom::glam::Vec2::new(100.0, 100.0));
    assert!(harness.state_mut().session.begin_stroke(at, viewport, 1.0).is_err(), "painting is paused");
    harness.state_mut().cancel_bake();
    harness.run_steps(2);
    assert_eq!(harness.state().session.view, ViewMode::UvChecker);
    assert!(!harness.state().session.bake_preview_active() && harness.state().bake.is_none());
    assert_eq!(harness.state().session.doc.baked.get(&0).cloned(), before);
}

#[test]
fn leaving_bake_mode_closes_the_preview_but_the_bake_goes_on() {
    let mut harness = harness();
    harness.state_mut().start_bake(vec![0]);
    harness.run_steps(2);
    harness.state_mut().ui.mode = Mode::Painting;
    harness.run_steps(2);
    assert_eq!(harness.state().session.view, ViewMode::UvChecker);
    assert!(!harness.state().session.bake_preview_active());
    run_to_the_end(&mut harness);
    assert!(harness.state().session.doc.baked[&0].maps.contains_key(&BakedMap::Height), "the bake finished and was applied");
    assert_eq!(harness.state().session.view, ViewMode::UvChecker);
}

#[test]
fn the_painting_message_is_a_translated_key() {
    assert_eq!(texturecraft_ui_egui::PAINTING_PAUSED, texturecraft_engine::bake_preview::PAINTING_PAUSED);
}
