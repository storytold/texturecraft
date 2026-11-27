//! The Baking Log panel: the latest bake, map by map

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_engine::bake::{BakeHighState, BakeLogHigh, BakeLogMap, BakeLogMatching, BakeMapState, NamePair};
use texturecraft_engine::core::{BakeResolution, BakedMap};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;
use texturecraft_ui_egui::state::Mode;

fn harness() -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    for texture_set in &mut session.doc.project.texture_sets {
        texture_set.bake = Default::default();
        texture_set.bake.common.resolution = BakeResolution::R64;
        texture_set.bake.maps.select([BakedMap::Id, BakedMap::Position]);
    }

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(4);
    harness
}

/// Run frames until the bake ends, then a few more for its result to show
fn run_to_the_end(harness: &mut Harness<'static, TextureCraftApp>) {
    super::step_until(harness, "the end of the bake", |app| app.bake.is_none());
    harness.run_steps(super::SETTLE_FRAMES);
}

/// Bake sets 0 and 1, where set 1 has no tile left to bake, so its bake fails
fn bake_with_set_1_failing(harness: &mut Harness<'static, TextureCraftApp>, failed_set_maps: &[BakedMap]) {
    let set = &mut harness.state_mut().session.doc.project.texture_sets[1];
    set.uv_tiles = vec![1001];
    set.bake_excluded_tiles.insert(1001);
    set.bake.maps.select(failed_set_maps.iter().copied());

    harness.state_mut().start_bake(vec![0, 1]);
    run_to_the_end(harness);
}

#[test]
fn the_baking_process_tab_lists_each_set_and_map_with_how_it_went() {
    let mut harness = harness();
    assert!(harness.state().ui.dock.is_visible(Panel::BakingLog), "the bake layout shows the Baking Log");

    bake_with_set_1_failing(&mut harness, &[BakedMap::Id]);

    let names: Vec<String> = harness.state().session.doc.project.texture_sets.iter().map(|set| set.name.clone()).collect();
    for map in ["ID", "Position"] {
        assert!(harness.query_by_label(&format!("{} {map}, Baked", names[0])).is_some(), "set 0's {map} row is ticked");
    }
    assert!(harness.query_by_label(&format!("{} ID, Not baked", names[1])).is_some(), "set 1's row failed");
}

#[test]
fn a_failed_rows_reason_is_read_without_hovering() {
    let mut harness = harness();

    bake_with_set_1_failing(&mut harness, &[BakedMap::Id]);

    let app = harness.state();
    let row = app.session.bake_log.maps.iter().find(|row| row.texture_set == 1).unwrap();
    let BakeMapState::Failed(reason) = &row.state else { panic!("set 1's bake failed: {:?}", row.state) };

    let label = format!("{} ID, Not baked", row.set_name);
    let node = harness.get_by_label(&label);
    assert_eq!(node.accesskit_node().description().as_deref(), Some(reason.as_str()), "the row's node carries its tooltip's reason");
}

#[test]
fn a_one_map_bake_lists_only_that_map() {
    let mut harness = harness();

    harness.state_mut().start_bake_of(vec![0], Some(BakedMap::Position));
    run_to_the_end(&mut harness);

    let name = harness.state().session.doc.project.texture_sets[0].name.clone();
    assert!(harness.query_by_label(&format!("{name} Position, Baked")).is_some(), "the baked map is ticked");
    assert!(harness.query_by_label_contains(&format!("{name} ID")).is_none(), "a checked map the bake left alone has no row");
}

#[test]
fn a_canceled_bake_keeps_the_maps_it_finished_and_warns_only_on_the_rest() {
    let mut harness = harness();
    let before = harness.state().session.baked(0).cloned().unwrap_or_default();

    harness.state_mut().start_bake(vec![0, 1]);

    // Leaving Bake mode closes the preview; the bake and the maps it finishes go on
    harness.state_mut().ui.mode = Mode::Painting;
    let state = |app: &TextureCraftApp, set: usize, map: BakedMap| {
        app.session.bake_log.maps.iter().find(|row| row.texture_set == set && row.map == map).map(|row| row.state.clone())
    };
    super::step_until(&mut harness, "set 0's first map", |app| state(app, 0, BakedMap::Id) == Some(BakeMapState::Baked));
    assert!(!harness.state().session.bake_preview_active());

    harness.state_mut().cancel_bake();

    let app = harness.state();
    assert_eq!(state(app, 0, BakedMap::Id), Some(BakeMapState::Baked), "a ticked map keeps its tick");
    for map in [BakedMap::Id, BakedMap::Position] {
        assert_eq!(state(app, 1, map), Some(BakeMapState::Canceled), "set 1 was still queued, so its {map:?} warns");
    }

    // The worker may have queued Position before cancellation
    let row = state(app, 0, BakedMap::Position);
    assert!(matches!(row, Some(BakeMapState::Baked | BakeMapState::Canceled)), "set 0's Position is ticked or, unfinished, warns: {row:?}");
    let after = app.session.baked(0).cloned().unwrap_or_default();
    let changed = |map: BakedMap| match (before.maps.get(&map), after.maps.get(&map)) {
        (Some(old), Some(new)) => !std::sync::Arc::ptr_eq(old, new),
        (old, new) => old.is_some() != new.is_some(),
    };
    for map in [BakedMap::Id, BakedMap::Position] {
        let ticked = state(app, 0, map) == Some(BakeMapState::Baked);
        assert_eq!(changed(map), ticked, "set 0's {map:?} is in the document exactly when its row is ticked");
    }
}

#[test]
fn a_failed_maps_arrow_opens_its_settings() {
    let mut harness = harness();

    // Three rows fit the docked log without scrolling
    harness.state_mut().session.doc.project.texture_sets[0].bake.maps.select([BakedMap::Id]);
    bake_with_set_1_failing(&mut harness, &[BakedMap::Id, BakedMap::Position]);
    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);

    assert_eq!(harness.state().session.ts(), 0);
    assert!(harness.query_by_label("Normalization Scale").is_none(), "Position's settings are not shown yet");

    let set_name = harness.state().session.doc.project.texture_sets[1].name.clone();
    harness.get_by_label(&format!("Open the {set_name} Position settings")).click();
    harness.run_steps(3);

    assert_eq!(harness.state().session.ts(), 1, "the failed map's texture set is the active one");
    assert!(harness.state().ui.dock.is_visible(Panel::MeshMapSettings), "the arrow fronts Mesh Map Settings");
    harness.get_by_label("Normalization Scale");
    assert!(
        harness.query_by_label(&format!("Open the {} ID settings", harness.state().session.doc.project.texture_sets[0].name)).is_none(),
        "a baked map has no arrow"
    );
}

/// Put a finished bake of set 0's ID and Position maps in the log, the Position map ending as `position`
fn log_a_bake(harness: &mut Harness<'static, TextureCraftApp>, position: BakeMapState) -> String {
    let set_name = harness.state().session.doc.project.texture_sets[0].name.clone();
    let row = |map, state| BakeLogMap { texture_set: 0, set_name: set_name.clone(), map, state };
    harness.state_mut().session.bake_log.maps = vec![row(BakedMap::Id, BakeMapState::Baked), row(BakedMap::Position, position)];
    harness.run_steps(3);
    set_name
}

#[test]
fn the_issues_filter_keeps_only_the_maps_with_a_problem() {
    let mut harness = harness();
    let set_name = log_a_bake(&mut harness, BakeMapState::Failed("The high poly file could not be read.".into()));
    harness.get_by_label(&format!("{set_name} ID, Baked"));

    harness.get_by_label("1, Show only rows with an issue").click();
    harness.run_steps(3);

    assert!(harness.query_by_label(&format!("{set_name} ID, Baked")).is_none(), "a baked map is filtered out");
    harness.get_by_label(&format!("{set_name} Position, Not baked"));
    assert!(harness.query_by_label("Turn off the issues filter to see every row.").is_none(), "an issue left: no empty text");
}

#[test]
fn the_issues_filter_says_so_when_nothing_went_wrong() {
    let mut harness = harness();
    let set_name = log_a_bake(&mut harness, BakeMapState::Baked);

    harness.get_by_label("Show only rows with an issue").click();
    harness.run_steps(3);

    assert!(harness.query_by_label(&format!("{set_name} ID, Baked")).is_none());
    harness.get_by_label("Every map so far baked without a problem.");
    harness.get_by_label("Turn off the issues filter to see every row.");

    harness.get_by_label("Show only rows with an issue").click();
    harness.run_steps(3);

    harness.get_by_label(&format!("{set_name} ID, Baked"));
    assert!(harness.query_by_label("Every map so far baked without a problem.").is_none(), "off again: every row is back");
}

#[test]
fn the_high_poly_tab_lists_each_sets_high_poly_files_under_a_row_that_folds() {
    let mut harness = harness();
    harness.state_mut().session.add_high_poly_bytes("ball_high.obj", b"o Ball_high\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()).unwrap();
    harness.state_mut().start_bake(vec![0]);
    run_to_the_end(&mut harness);

    harness.get_by_label("High Poly Parameters").click();
    harness.run_steps(3);

    let set_name = harness.state().session.doc.project.texture_sets[0].name.clone();
    harness.get_by_label("ball_high.obj, Loaded");
    let set_row = format!("{set_name}, High poly files");
    assert_eq!(harness.get_by_label(&set_row).accesskit_node().data().is_expanded(), Some(true));

    harness.get_by_label(&set_row).click();
    harness.run_steps(3);

    assert!(harness.query_by_label("ball_high.obj, Loaded").is_none(), "a folded set hides its files");
    assert_eq!(harness.get_by_label(&set_row).accesskit_node().data().is_expanded(), Some(false), "the row says it is folded");
}

#[test]
fn the_high_poly_tab_counts_and_keeps_its_own_issues() {
    let mut harness = harness();
    let set_name = log_a_bake(&mut harness, BakeMapState::Baked);
    let row = |file: &str, state| BakeLogHigh { texture_set: 0, set_name: set_name.clone(), file: file.into(), state };
    harness.state_mut().session.bake_log.highs = vec![row("ball_high.obj", BakeHighState::Loaded), row("bolt_high.obj", BakeHighState::Canceled)];
    harness.get_by_label("High Poly Parameters").click();
    harness.run_steps(3);

    harness.get_by_label("1, Show only rows with an issue").click();
    harness.run_steps(3);

    assert!(harness.query_by_label("ball_high.obj, Loaded").is_none(), "a loaded file is filtered out");
    harness.get_by_label("bolt_high.obj, Canceled");
}

#[test]
fn the_matching_tab_lists_the_pairs_and_its_filter_keeps_the_parts_with_no_partner() {
    let mut harness = harness();
    log_a_bake(&mut harness, BakeMapState::Baked);
    let pair = |low: Option<&str>, high: Option<&str>| NamePair { low: low.map(Into::into), high: high.map(Into::into) };
    harness.state_mut().session.bake_log.matching = Some(BakeLogMatching {
        low_suffix: "_low".into(),
        high_suffix: "_high".into(),
        pairs: vec![pair(Some("Barrel_low"), Some("Barrel_high")), pair(Some("Strap_low"), None)],
    });
    harness.get_by_label("Matching by Name").click();
    harness.run_steps(3);
    harness.get_by_label("Barrel_low, Barrel_high");

    harness.get_by_label("1, Show only rows with an issue").click();
    harness.run_steps(3);

    assert!(harness.query_by_label("Barrel_low, Barrel_high").is_none(), "a pair is filtered out");
    harness.get_by_label("Strap_low, No match");
}

#[test]
fn the_matching_tab_says_when_no_set_matched_by_name() {
    let mut harness = harness();

    harness.get_by_label("Matching by Name").click();
    harness.run_steps(3);

    assert!(harness.query_by_label("Set Match to By Mesh Name to pair parts by name.").is_none(), "nothing baked yet: nothing to say");

    log_a_bake(&mut harness, BakeMapState::Baked);

    harness.get_by_label("Set Match to By Mesh Name to pair parts by name.");
}

#[test]
fn on_a_narrow_dock_the_tabs_scroll_clear_of_the_issues_filter() {
    // A window this narrow leaves the docked log less room than its three tabs and the filter take together
    const NARROW_WINDOW_WIDTH: f32 = 520.0;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(NARROW_WINDOW_WIDTH, 1000.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, texturecraft_engine::sample_session(256, 64).unwrap()));
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(4);

    let filter = harness.get_by_label("Show only rows with an issue").rect();
    let strip_start = harness.get_by_role_and_label(Role::Tab, "Baking Process").rect().left();
    let last_tab = harness.get_by_role_and_label(Role::Tab, "Matching by Name").rect();
    assert!(last_tab.right() > filter.left(), "unscrolled, the tabs {last_tab:?} run past the filter {filter:?}, so they must scroll");

    // Frames for the strip's scroll to the focused tab to finish
    const SCROLL_FRAMES: usize = 30;

    for tab in ["Baking Process", "High Poly Parameters", "Matching by Name"] {
        harness.get_by_role_and_label(Role::Tab, tab).focus();
        harness.run_steps(SCROLL_FRAMES);

        let rect = harness.get_by_role_and_label(Role::Tab, tab).rect();
        assert!(rect.right() <= filter.left(), "the focused {tab} tab {rect:?} ends left of the filter {filter:?}");
        assert!(rect.left() >= strip_start, "the focused {tab} tab {rect:?} starts inside the log, where the first tab starts unscrolled");
    }
}

#[test]
fn a_matching_name_cut_short_shows_in_full_on_hover() {
    const LONG_NAME: &str = "Left_shoulder_strap_buckle_with_its_rivets_and_its_stitching_low";

    // Frames past the tooltip delay
    const TOOLTIP_FRAMES: usize = 60;

    let mut harness = harness();
    log_a_bake(&mut harness, BakeMapState::Baked);
    let pair = NamePair { low: Some(LONG_NAME.into()), high: Some("Strap_high".into()) };
    harness.state_mut().session.bake_log.matching = Some(BakeLogMatching { low_suffix: "_low".into(), high_suffix: "_high".into(), pairs: vec![pair] });
    harness.get_by_label("Matching by Name").click();
    harness.run_steps(3);

    let label = format!("{LONG_NAME}, Strap_high");
    let row = harness.get_by_label(&label).rect();
    assert_eq!(harness.query_all_by_label(&label).count(), 1, "no tooltip before hovering");

    harness.hover_at(row.center());
    harness.run_steps(TOOLTIP_FRAMES);

    assert_eq!(harness.query_all_by_label(&label).count(), 2, "the tooltip carries both names in full");
}
