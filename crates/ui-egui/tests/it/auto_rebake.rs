//! Auto-rebake in the app: a settings change rebakes the auto-rebake map alone, in the background

use egui_kittest::Harness;
use serde_json::json;
use texturecraft_engine::bake::BakeMapState;
use texturecraft_engine::core::BakedMap;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::Mode;

/// Frames far past the half-second settle wait, at the harness's quarter-second step
const PAST_THE_SETTLE_FRAMES: usize = 60;

/// A plane in Bake mode with its position and ID maps baked and Auto-rebake on for the position map
fn harness(resolution: u32) -> Harness<'static, TextureCraftApp> {
    let mesh = primitives::builtin("plane").unwrap();
    let mut session = Session::new_project("t", mesh, "builtin:plane", None, 128).unwrap();
    execute(&mut session, "bake.run", &json!({ "resolution": resolution, "maps": ["position", "id"], "dilation": 0 })).unwrap();
    execute(&mut session, "bake.autoRebake", &json!({ "map": "position" })).unwrap();

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(4);
    harness
}

fn change_settings(harness: &mut Harness<'static, TextureCraftApp>, params: serde_json::Value) {
    execute(&mut harness.state_mut().session, "bake.settings", &params).unwrap();
}

#[test]
fn a_settings_change_rebakes_only_the_auto_rebake_map_without_a_toast() {
    let mut harness = harness(64);
    let id_before = harness.state().session.baked(0).unwrap().maps[&BakedMap::Id].clone();
    change_settings(&mut harness, json!({ "positionMode": "boundingBox" }));

    super::step_until(&mut harness, "the rebake", |app| app.bake.is_some());
    super::step_until(&mut harness, "the end of the rebake", |app| app.bake.is_none());

    let app = harness.state();
    assert!(app.session.doc.project.texture_sets[0].bake_sources.contains_key(&BakedMap::Position), "the rebake records what it read");
    assert!(app.session.bake_changes(0, BakedMap::Position).is_empty(), "{:?}", app.session.bake_changes(0, BakedMap::Position));
    assert!(std::sync::Arc::ptr_eq(&app.session.baked(0).unwrap().maps[&BakedMap::Id], &id_before), "the ID map was not baked again");
    let rows: Vec<_> = app.session.bake_log.maps.iter().map(|row| (row.map, row.state.clone())).collect();
    assert_eq!(rows, [(BakedMap::Position, BakeMapState::Baked)]);
    assert!(app.toasts.is_empty(), "a rebake that went well is quiet");
}

#[test]
fn the_progress_footer_names_the_map_the_rebake_makes() {
    let mut harness = harness(64);
    change_settings(&mut harness, json!({ "positionMode": "boundingBox" }));

    super::step_until(&mut harness, "the rebake", |app| app.bake.as_ref().is_some_and(|job| job.current.is_some()));

    let stage = harness.state().session.bake_preview.as_ref().and_then(|preview| preview.stage());
    assert_eq!(stage.map(|stage| (stage.map, stage.total)), Some((BakedMap::Position, 1)), "not the set's first checked map, not all its maps");
}

#[test]
fn nothing_is_rebaked_outside_bake_mode() {
    let mut harness = harness(64);
    harness.state_mut().ui.mode = Mode::Painting;
    change_settings(&mut harness, json!({ "positionMode": "boundingBox" }));

    harness.run_steps(120);

    assert!(harness.state().bake.is_none());
    assert!(!harness.state().session.bake_changes(0, BakedMap::Position).is_empty());
}

#[test]
fn a_change_while_the_rebake_runs_cancels_it_so_the_newest_settings_win() {
    let mut harness = harness(64);
    execute(&mut harness.state_mut().session, "bake.run", &json!({ "maps": ["ambientOcclusion"], "ao": { "rays": 1 } })).unwrap();
    execute(&mut harness.state_mut().session, "bake.autoRebake", &json!({ "map": "ambientOcclusion" })).unwrap();

    // A slow rebake, so the next change lands while it runs
    change_settings(&mut harness, json!({ "resolution": 2048, "ao": { "rays": 256 } }));
    super::step_until(&mut harness, "the rebake", |app| app.bake.as_ref().is_some_and(|job| job.current.is_some()));
    change_settings(&mut harness, json!({ "ao": { "rays": 128 } }));
    harness.run_steps(1);

    let app = harness.state();
    assert!(app.bake.is_none(), "the outdated rebake stopped");
    let ambient_occlusion = app.session.bake_log.maps.iter().find(|row| row.map == BakedMap::AmbientOcclusion).map(|row| row.state.clone());
    assert_eq!(ambient_occlusion, Some(BakeMapState::Canceled));
}

#[test]
fn a_canceled_rebake_holds_back_only_its_own_texture_set() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    execute(&mut session, "bake.run", &json!({ "ts": "all", "resolution": 32, "maps": ["position"], "dilation": 0 })).unwrap();
    for set in [0, 1] {
        execute(&mut session, "bake.autoRebake", &json!({ "ts": set, "map": "position" })).unwrap();
    }
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(4);
    let rebaking = |app: &TextureCraftApp, set: usize| app.bake.as_ref().and_then(|job| job.rebake.as_ref()).is_some_and(|rebake| rebake.0 == set);

    change_settings(&mut harness, json!({ "ts": "all", "positionMode": "boundingBox" }));
    super::step_until(&mut harness, "the first set's rebake", |app| rebaking(app, 0));
    harness.state_mut().cancel_bake();

    super::step_until(&mut harness, "the second set's rebake", |app| rebaking(app, 1));
}

#[test]
fn a_skew_stroke_rebakes_the_auto_rebake_map_inside_the_stroke() {
    let mut harness = harness(64);
    let plane = primitives::builtin("plane").unwrap();
    let mut high = String::from("o Part_high\n");
    for position in &plane.positions {
        high += &format!("v {} {} {}\n", position.x, position.y + 0.1, position.z);
    }
    for triangle in plane.indices.as_chunks::<3>().0 {
        high += &format!("f {} {} {}\n", triangle[0] + 1, triangle[1] + 1, triangle[2] + 1);
    }
    let session = &mut harness.state_mut().session;
    session.add_high_poly_bytes("high.obj", high.into_bytes()).unwrap();
    execute(session, "bake.run", &json!({ "maps": ["position"] })).unwrap();
    harness.run_steps(4);
    assert!(harness.state().bake.is_none(), "up to date after the bake");

    execute(&mut harness.state_mut().session, "bake.skew.paint", &json!({ "points": [[0.5, 0.5]], "radius": 0.05, "size": 64 })).unwrap();
    super::step_until(&mut harness, "the rebake", |app| app.bake.is_some());
    super::step_until(&mut harness, "the end of the rebake", |app| app.bake.is_none());

    let app = harness.state();
    assert!(app.session.auto_rebake_painted().is_empty(), "the rebake took the stroke");
    let rows: Vec<_> = app.session.bake_log.maps.iter().map(|row| (row.map, row.state.clone())).collect();
    assert_eq!(rows, [(BakedMap::Position, BakeMapState::Baked)]);
    assert!(app.toasts.is_empty(), "a rebake that went well is quiet");
}

#[test]
fn the_rows_icon_turns_auto_rebake_on_for_its_map_and_off_again() {
    use egui_kittest::kittest::Queryable;

    let mut harness = harness(64);
    let auto_rebake = |harness: &Harness<'static, TextureCraftApp>| harness.state().session.doc.project.texture_sets[0].auto_rebake;
    assert_eq!(auto_rebake(&harness), Some(BakedMap::Position));

    assert!(harness.query_by_label("Auto-rebake Normal").is_none(), "off: shown only on a hovered, focused or selected row");
    harness.get_by_role_and_label(egui::Role::Button, "Normal").hover();
    harness.run_steps(2);
    harness.get_by_label("Auto-rebake Normal");

    // ID is the selected row (the first checked map), which shows its icons
    harness.get_by_label("Auto-rebake ID").click();
    harness.run_steps(2);
    assert_eq!(auto_rebake(&harness), Some(BakedMap::Id), "one map per set: ID replaces Position");

    harness.get_by_label("Auto-rebake ID").click();
    harness.run_steps(2);
    assert_eq!(auto_rebake(&harness), None);
}

#[test]
fn no_rebake_starts_while_the_pointer_is_held_so_a_paused_drag_stays_one_undo_step() {
    let mut harness = harness(64);
    change_settings(&mut harness, json!({ "positionMode": "boundingBox" }));

    // Held still well past the settle time, as a hand pausing mid-drag, in the window's corner where no control takes the press
    let corner = harness.ctx.content_rect().right_bottom() - egui::vec2(10.0, 10.0);
    harness.drag_at(corner);
    harness.run_steps(120);
    assert!(harness.state().bake.is_none(), "the edit is not over while the press lasts");

    harness.drop_at(corner);
    super::step_until(&mut harness, "the rebake after the release", |app| app.bake.is_some());
}

#[test]
fn a_rebake_canceled_in_one_project_still_runs_in_the_next_project_opened() {
    let mut harness = harness(64);
    change_settings(&mut harness, json!({ "positionMode": "boundingBox" }));
    super::step_until(&mut harness, "the rebake", |app| app.bake.is_some());
    harness.state_mut().cancel_bake();
    harness.run_steps(PAST_THE_SETTLE_FRAMES);
    assert!(harness.state().bake.is_none(), "a canceled rebake is not tried again in its project");

    // The same project opened again: its out-of-date map reads what the canceled one read
    let mesh = primitives::builtin("plane").unwrap();
    let mut next = Session::new_project("t", mesh, "builtin:plane", None, 128).unwrap();
    execute(&mut next, "bake.run", &json!({ "resolution": 64, "maps": ["position", "id"], "dilation": 0 })).unwrap();
    execute(&mut next, "bake.autoRebake", &json!({ "map": "position" })).unwrap();
    execute(&mut next, "bake.settings", &json!({ "positionMode": "boundingBox" })).unwrap();
    harness.state_mut().session.switch_to(next);

    super::step_until(&mut harness, "the rebake in the opened project", |app| app.bake.is_some());
}
