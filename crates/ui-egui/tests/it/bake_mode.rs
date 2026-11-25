use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable, by};
use texturecraft_engine::core::{BakeResolution, BakeSettings, BakedMap, CageType, CurvatureMethod, HeightNormalization};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;
use texturecraft_ui_egui::state::Mode;

/// A bake-mode app with only `map` enabled and Mesh Map Settings in front
fn harness(map: Option<BakedMap>) -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    session.doc.project.texture_sets[0].bake = Default::default();
    session.doc.project.texture_sets[0].bake.common.resolution = BakeResolution::R64;
    session.doc.project.texture_sets[0].bake.maps.select(map);
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1500.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(4);
    harness.state_mut().ui.dock.focus(Panel::MeshMapSettings);
    harness.run_steps(3);
    harness
}

/// The maps the first texture set will bake
fn enabled_maps(harness: &Harness<'_, TextureCraftApp>) -> Vec<BakedMap> {
    harness.state().session.doc.project.texture_sets[0].bake.maps.enabled().collect()
}

/// Click a map's row in Mesh Map Bakers, which shows its settings
fn select_map(harness: &mut Harness<'_, TextureCraftApp>, name: &str) {
    harness.get_by_role_and_label(Role::Button, name).click();
    harness.run_steps(3);
}

/// Click a checkbox in Mesh Map Bakers: a map's row or Check All
fn click_checkbox(harness: &mut Harness<'_, TextureCraftApp>, name: &str) {
    harness.get_by_role_and_label(Role::CheckBox, name).click();
    harness.run_steps(3);
}

/// Scroll the widget labelled `label` into view and click it
fn click(harness: &mut Harness<'_, TextureCraftApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(3);
    harness.get_by_label(label).click();
    harness.run_steps(3);
}

/// Type `text` as the value of the slider labelled `label`: a click on its value turns it into a field with the value selected
fn type_slider_value(harness: &mut Harness<'_, TextureCraftApp>, label: &str, text: &str) {
    harness.get_by_role_and_label(Role::Label, &format!("{label} value")).click();
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text(text.into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
}

#[test]
fn mesh_map_edits_update_all_sets_without_copying_unedited_fields() {
    let mut harness = harness(Some(BakedMap::AmbientOcclusion));
    let session = &mut harness.state_mut().session;
    assert_eq!(session.set_count(), 2);
    let second = &mut session.doc.project.texture_sets[1].bake;
    *second = BakeSettings::default();
    second.common.is_enabled = false;
    second.common.resolution = BakeResolution::R128;
    second.common.resolution_height = Some(BakeResolution::R256);
    second.maps.ao.is_enabled = false;
    second.maps.ao.parameters.rays.rays = 16;
    second.maps.ao.parameters.ground_plane.offset = 3.0;
    second.maps.height.parameters.scaling_divisor = 2.0;
    let before = session.doc.project.clone();
    let history_steps = session.history_labels().len();

    click_checkbox(&mut harness, "Ground Plane");

    let mut expected = before.clone();
    for texture_set in &mut expected.texture_sets {
        texture_set.bake.maps.ao.parameters.ground_plane.enabled = true;
    }
    let session = &mut harness.state_mut().session;
    assert_eq!(session.doc.project, expected);
    assert_eq!(session.history_labels().len(), history_steps + 1);
    session.undo().unwrap();
    assert_eq!(session.doc.project, before);
    session.redo().unwrap();
    assert_eq!(session.doc.project, expected);
}

#[test]
fn common_settings_edits_update_all_sets_and_keep_their_map_selection() {
    let mut harness = harness(Some(BakedMap::AmbientOcclusion));
    let session = &mut harness.state_mut().session;
    let second = &mut session.doc.project.texture_sets[1].bake;
    *second = BakeSettings::default();
    second.common.is_enabled = false;
    second.common.resolution = BakeResolution::R128;
    second.maps.select([BakedMap::Height]);
    let before = session.doc.project.clone();

    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);
    click_checkbox(&mut harness, "Apply Diffusion");

    let mut expected = before.clone();
    for texture_set in &mut expected.texture_sets {
        texture_set.bake.common.apply_diffusion = true;
    }

    assert_eq!(harness.state().session.doc.project, expected);

    harness.get_all_by_value("64").next().unwrap().click();
    harness.run_steps(3);
    click(&mut harness, "256");
    for texture_set in &mut expected.texture_sets {
        texture_set.bake.common.resolution = BakeResolution::R256;
    }

    assert_eq!(harness.state().session.doc.project, expected);

    click_checkbox(&mut harness, "Use Low Poly Mesh as High Poly Mesh");
    for texture_set in &mut expected.texture_sets {
        texture_set.bake.high_poly.use_low_as_high = true;
    }

    assert_eq!(harness.state().session.doc.project, expected);
}

#[test]
fn secondary_rays_edit_from_another_set_updates_all_sets() {
    let mut harness = harness(Some(BakedMap::AmbientOcclusion));
    let session = &mut harness.state_mut().session;
    session.doc.project.texture_sets[1].bake = BakeSettings::default();
    session.doc.project.texture_sets[1].bake.maps.ao.parameters.rays.rays = 16;
    texturecraft_engine::execute(session, "textureSet.select", &serde_json::json!({"ts": 1})).unwrap();
    harness.run_steps(3);
    select_map(&mut harness, "Ambient Occlusion");
    let history_steps = harness.state().session.history_labels().len();

    type_slider_value(&mut harness, "Secondary Rays", "128");

    let session = &harness.state().session;
    assert_eq!(session.ts(), 1);
    for texture_set in &session.doc.project.texture_sets {
        assert_eq!(texture_set.bake.maps.ao.parameters.rays.rays, 128);
    }

    assert_eq!(session.history_labels().len(), history_steps + 1);
}

#[test]
fn map_selection_stays_per_set() {
    let mut harness = harness(Some(BakedMap::AmbientOcclusion));
    let second = harness.state().session.doc.project.texture_sets[1].bake.clone();

    click_checkbox(&mut harness, "Ambient Occlusion");
    assert!(enabled_maps(&harness).is_empty());
    assert_eq!(harness.state().session.doc.project.texture_sets[1].bake, second);

    click_checkbox(&mut harness, "Check All");
    assert_eq!(enabled_maps(&harness), BakedMap::ALL);
    assert_eq!(harness.state().session.doc.project.texture_sets[1].bake, second);
}

#[test]
fn bake_texture_set_visibility_is_independent_of_bake_selection() {
    let mut harness = harness(Some(BakedMap::AmbientOcclusion));
    let before: Vec<_> = harness.state().session.doc.project.texture_sets.iter().map(|texture_set| texture_set.bake.clone()).collect();
    let active_set = harness.state().session.ts();

    harness.get_all_by_label("Show or hide the texture set").nth(1).unwrap().click();
    harness.run_steps(3);

    let session = &harness.state().session;
    assert!(session.doc.project.texture_sets[0].visible);
    assert!(!session.doc.project.texture_sets[1].visible);
    assert_eq!(session.ts(), active_set);
    for (texture_set, settings) in session.doc.project.texture_sets.iter().zip(&before) {
        assert_eq!(&texture_set.bake, settings);
    }

    harness.get_all_by_label("Show or hide the texture set").nth(1).unwrap().click();
    harness.run_steps(3);
    assert!(harness.state().session.doc.project.texture_sets[1].visible);
}

#[test]
fn texture_set_resolution_controls_are_independent_from_bake_output() {
    let mut harness = harness(Some(BakedMap::AmbientOcclusion));
    let before = harness.state().session.doc.project.clone();
    let active_set = harness.state().session.ts();
    harness.get_all_by_value("256x256").nth(1).unwrap().click();
    harness.run_steps(3);
    click(&mut harness, "128x128");
    wait_for_resolution(&mut harness, 1, 128);

    let session = &harness.state().session;
    assert_eq!(session.ts(), active_set);
    assert_eq!(session.doc.project.texture_sets[0], before.texture_sets[0]);
    assert_eq!(session.doc.project.texture_sets[1].bake, before.texture_sets[1].bake);
    harness.state_mut().session.undo().unwrap();
    assert_eq!(harness.state().session.doc.project, before);
}

fn wait_for_resolution(harness: &mut Harness<'_, TextureCraftApp>, texture_set: usize, resolution: u32) {
    for _ in 0..100 {
        harness.run_steps(1);
        if harness.state().session.doc.project.texture_sets[texture_set].resolution == resolution {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("texture set resize did not finish");
}

#[test]
fn tile_resolution_overrides_can_be_changed_and_removed_in_bake_mode() {
    let session = texturecraft_engine::Session::new_project(
        "Tile resolution controls",
        texturecraft_engine::geom::primitives::udim_plane(),
        "builtin:udim_plane",
        None,
        256,
    )
    .unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1500.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(4);
    let before = harness.state().session.doc.project.clone();

    assert_eq!(harness.get_all_by_value("256x256").count(), 1);
    let resolution_position = harness.get_by_label("UV tile 1002 resolution").rect().center();
    harness.hover_at(resolution_position);
    harness.run_steps(3);
    assert_eq!(harness.get_all_by_value("256x256").count(), 2);
    harness.hover_at(egui::pos2(900.0, 500.0));
    harness.run_steps(3);
    assert_eq!(harness.get_all_by_value("256x256").count(), 1);
    harness.get_by_label("UV tile 1002 resolution").click();
    harness.run_steps(3);
    click(&mut harness, "128x128");
    harness.get_by_value("128x128*");
    let texture_set = &harness.state().session.doc.project.texture_sets[0];
    assert_eq!(texture_set.resolution, 256);
    assert_eq!(texture_set.tile_resolution(1001), 256);
    assert_eq!(texture_set.tile_resolution(1002), 128);
    assert_eq!(texture_set.bake, before.texture_sets[0].bake);

    harness.get_by_value("128x128*").click();
    harness.run_steps(3);
    click(&mut harness, "Remove resolution override");
    assert_eq!(harness.state().session.doc.project, before);
    assert!(harness.query_by_value("128x128*").is_none());
    assert_eq!(harness.get_all_by_value("256x256").count(), 1);
    harness.state_mut().session.undo().unwrap();
    harness.run_steps(3);
    harness.get_by_value("128x128*");

    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);
    harness.get_all_by_value("2048").next().unwrap().click();
    harness.run_steps(3);
    click(&mut harness, "64");
    let texture_set = &harness.state().session.doc.project.texture_sets[0];
    assert_eq!(texture_set.resolution, 256);
    assert_eq!(texture_set.tile_resolution(1002), 128);
    assert_eq!(texture_set.bake.common.resolution, BakeResolution::R64);
}

#[test]
fn bake_texture_sets_list_uv_tiles_and_their_resolutions() {
    let mut session =
        texturecraft_engine::Session::new_project("UV tile list", texturecraft_engine::geom::primitives::udim_plane(), "builtin:udim_plane", None, 256)
            .unwrap();
    texturecraft_engine::execute(&mut session, "textureSet.tile.resolution", &serde_json::json!({"tile": 1002, "resolution": 128})).unwrap();
    let before = session.doc.project.clone();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1500.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(4);

    harness.get_by_label("1001");
    harness.get_by_value("256x256");
    harness.get_by_label("1002");
    harness.get_by_value("128x128*");
    click_checkbox(&mut harness, "1002");
    assert!(harness.state().session.doc.project.texture_sets[0].bake_excluded_tiles.contains(&1002));
    assert!(!harness.get_by_label("Bake (7)").accesskit_node().is_disabled(), "7 checked maps on the one tile left");
    click_checkbox(&mut harness, "1001");
    assert!(harness.get_by_label("Bake (0)").accesskit_node().is_disabled());
    harness.state_mut().session.undo().unwrap();
    harness.state_mut().session.undo().unwrap();
    harness.run_steps(3);
    click(&mut harness, "UV tiles (UDIM) (2)");
    assert!(harness.query_by_label("1001").is_none());
    assert!(harness.query_by_label("1002").is_none());
    assert_eq!(harness.state().session.doc.project, before);
}

#[test]
fn restoring_map_defaults_updates_other_sets_even_when_active_values_are_default() {
    let mut harness = harness(Some(BakedMap::AmbientOcclusion));
    let session = &mut harness.state_mut().session;
    let second = &mut session.doc.project.texture_sets[1].bake;
    *second = BakeSettings::default();
    second.common.is_enabled = false;
    second.maps.ao.is_enabled = false;
    second.maps.ao.parameters.rays.rays = 16;
    second.maps.ao.parameters.ground_plane.offset = 3.0;
    let before = session.doc.project.clone();

    click(&mut harness, "Restore Defaults");

    let mut expected = before.clone();
    expected.texture_sets[1].bake.maps.ao.parameters = BakeSettings::default().maps.ao.parameters;
    let session = &mut harness.state_mut().session;
    assert_eq!(session.doc.project, expected);
    session.undo().unwrap();
    assert_eq!(session.doc.project, before);
}

#[test]
fn restoring_common_defaults_keeps_each_sets_inclusion_and_maps() {
    let mut harness = harness(Some(BakedMap::AmbientOcclusion));
    let session = &mut harness.state_mut().session;
    session.doc.project.texture_sets[0].bake.common = Default::default();
    let second = &mut session.doc.project.texture_sets[1].bake;
    *second = BakeSettings::default();
    second.common.is_enabled = false;
    second.common.resolution = BakeResolution::R128;
    second.common.resolution_height = Some(BakeResolution::R256);
    second.common.apply_diffusion = false;
    second.maps.select([BakedMap::Height]);
    second.high_poly.low_suffix = "_custom_low".into();
    let before = session.doc.project.clone();

    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Button, "Reset Common Settings").click();
    harness.run_steps(3);

    let mut expected = before.clone();
    expected.texture_sets[1].bake.common = Default::default();
    expected.texture_sets[1].bake.common.is_enabled = false;

    // The section holds the High-Poly Parameters too, so its reset covers them
    expected.texture_sets[1].bake.high_poly = Default::default();
    let session = &mut harness.state_mut().session;
    assert_eq!(session.doc.project, expected);
    session.undo().unwrap();

    assert_eq!(session.doc.project, before);
}

#[test]
fn output_axes_unlock_independently_and_relock_to_width() {
    let mut harness = harness(None);
    harness.get_by_label("Texture Sets");
    harness.get_by_label("Mesh Map Settings");
    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);
    let width = harness.state().session.doc.project.texture_sets[0].bake.common.resolution.pixels().to_string();
    assert!(harness.get_all_by_value(&width).nth(1).unwrap().accesskit_node().is_disabled());

    click(&mut harness, "Lock Output Size");
    assert!(!harness.get_all_by_value(&width).nth(1).unwrap().accesskit_node().is_disabled());
    harness.get_all_by_value(&width).next().unwrap().click();
    harness.run_steps(3);
    click(&mut harness, "128");
    let settings = &harness.state().session.doc.project.texture_sets[0].bake.common;
    assert_eq!(settings.resolution, BakeResolution::R128);
    assert_eq!(settings.resolution_height.map(BakeResolution::pixels), Some(width.parse().unwrap()));

    click(&mut harness, "Lock Output Size");
    assert_eq!(harness.state().session.doc.project.texture_sets[0].bake.common.output_size(), (128, 128));
    assert!(harness.get_all_by_value("128").nth(1).unwrap().accesskit_node().is_disabled());
    harness.state_mut().ui.dock.focus(Panel::MeshMapSettings);
    harness.run_steps(3);
    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);
    assert!(harness.get_all_by_value("128").nth(1).unwrap().accesskit_node().is_disabled());

    harness.state_mut().ui.mode = Mode::Painting;
    harness.run_steps(3);
    assert!(!harness.state().ui.dock.is_open(Panel::CommonSettings));
    assert!(harness.state().ui.dock.is_open(Panel::Assets));
}

#[test]
fn height_controls_follow_manual_normalization_and_the_row_checkbox_enables_height() {
    let mut harness = harness(None);
    assert!(harness.query_by_label("Normalization").is_none());
    select_map(&mut harness, "Height");
    assert!(!harness.state().session.doc.project.texture_sets[0].bake.maps.height.is_enabled, "selecting is not enabling");
    assert!(harness.get_by_label("Scaling Divisor").accesskit_node().is_disabled());
    click(&mut harness, "Normalization");
    click(&mut harness, "Manual");

    assert!(!harness.get_by_label("Scaling Divisor").accesskit_node().is_disabled());
    assert_eq!(harness.state().session.doc.project.texture_sets[0].bake.maps.height.parameters.normalization, HeightNormalization::Manual);

    click_checkbox(&mut harness, "Height");
    assert!(harness.state().session.doc.project.texture_sets[0].bake.maps.height.is_enabled);
    click(&mut harness, "Restore Defaults");

    assert!(harness.state().session.doc.project.texture_sets[0].bake.maps.height.is_enabled, "Restore Defaults keeps the map enabled");
    assert_eq!(harness.state().session.doc.project.texture_sets[0].bake.maps.height.parameters.normalization, HeightNormalization::RelativeToLowMesh);
    assert!(harness.get_by_label("Scaling Divisor").accesskit_node().is_disabled());
    click_checkbox(&mut harness, "Height");
    assert!(!harness.state().session.doc.project.texture_sets[0].bake.maps.height.is_enabled);
}

#[test]
fn clicking_a_row_shows_that_maps_settings_without_changing_what_bakes() {
    let mut harness = harness(Some(BakedMap::Curvature));
    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);
    let before = enabled_maps(&harness);

    select_map(&mut harness, "Ambient Occlusion");
    assert!(harness.state().ui.dock.is_visible(Panel::MeshMapSettings), "selecting a map fronts its settings");
    harness.get_by_label("Attenuation");
    assert!(harness.query_by_label("Method").is_none());
    select_map(&mut harness, "Opacity");
    harness.get_by_label("No Parameters");
    assert!(harness.query_by_label("Attenuation").is_none());
    assert_eq!(enabled_maps(&harness), before);

    harness.get_by_role_and_label(Role::Button, "Common Settings").click();
    harness.run_steps(3);

    assert!(harness.state().ui.dock.is_visible(Panel::CommonSettings));
    assert_eq!(enabled_maps(&harness), before);
}

#[test]
fn check_all_enables_every_map_then_disables_them_all() {
    let mut harness = harness(Some(BakedMap::Curvature));
    click_checkbox(&mut harness, "Check All");
    assert_eq!(enabled_maps(&harness), BakedMap::ALL);
    click_checkbox(&mut harness, "Check All");
    assert!(enabled_maps(&harness).is_empty());
}

#[test]
fn the_breadcrumb_switches_the_shown_map() {
    let mut harness = harness(Some(BakedMap::Curvature));
    harness.get_by_label("Sampling Radius");
    assert_eq!(harness.get_all_by_role_and_label(Role::Button, "Position").count(), 1, "only the list row before the menu opens");

    harness.get_by_label("Switch Mesh Map").click();
    harness.run_steps(3);

    assert_eq!(harness.get_all_by_role_and_label(Role::Button, "Position").count(), 2, "the map menu lists Position");

    // The menu draws its items after the list rows
    harness.get_all_by_role_and_label(Role::Button, "Position").last().unwrap().click();
    harness.run_steps(3);
    harness.get_by_label("Normalization Scale");
    assert!(harness.query_by_label("Sampling Radius").is_none());
    assert_eq!(enabled_maps(&harness), [BakedMap::Curvature]);
}

#[test]
fn a_long_texture_set_name_leaves_the_map_switcher_and_restore_defaults_in_the_breadcrumb() {
    let mut harness = harness(Some(BakedMap::Curvature));
    harness.state_mut().session.doc.project.texture_sets[0].name = "W".repeat(200);
    harness.run_steps(3);

    let switcher = harness.get_by_label("Switch Mesh Map").rect();
    let restore = harness.get_by_role_and_label(Role::Button, "Restore Defaults").rect();
    assert!(switcher.right() <= restore.left(), "the map switcher stays left of Restore Defaults");
}

#[test]
fn curvature_controls_follow_the_method_and_grey_out_by_their_switches() {
    let mut harness = harness(Some(BakedMap::Curvature));
    let curvature = |harness: &Harness<'_, TextureCraftApp>| harness.state().session.doc.project.texture_sets[0].bake.maps.curvature.parameters;

    assert_eq!(curvature(&harness).method, CurvatureMethod::Mesh);
    assert!(harness.query_by_label("Details").is_none());
    harness.get_by_label("Sampling Radius");
    for bound in ["Value Bounds Min", "Value Bounds Max"] {
        assert!(harness.get_by_label(bound).accesskit_node().is_disabled(), "{bound} is greyed while tonemapping is automatic");
    }

    click(&mut harness, "Auto Tonemapping (per UV Tile)");
    for bound in ["Value Bounds Min", "Value Bounds Max"] {
        assert!(!harness.get_by_label(bound).accesskit_node().is_disabled());
    }

    assert!(!curvature(&harness).mesh.auto_tonemapping);

    click(&mut harness, "Method");
    click(&mut harness, "Generate from Normal Map");

    assert_eq!(curvature(&harness).method, CurvatureMethod::NormalMap);
    assert!(harness.query_by_label("Sampling Radius").is_none());
    harness.get_by_label("Details");

    assert!(!harness.get_by_label("Seams Intensity").accesskit_node().is_disabled());
    click(&mut harness, "Enable Seams");

    assert!(harness.get_by_label("Seams Intensity").accesskit_node().is_disabled());
}

#[test]
fn cage_and_skew_controls_require_a_high_poly_and_grey_out_with_a_reason() {
    let mut harness = harness(None);
    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);

    // Without a high poly everything that needs one shows greyed, as in the reference
    for needs_high in ["Match", "Max Frontal Distance", "Edge Protection"] {
        assert!(harness.get_by_label(needs_high).accesskit_node().is_disabled(), "{needs_high} is greyed");
    }
    harness.get_by_label("The cage applies once a high poly mesh is loaded.");
    harness.get_by_label("Skew Correction applies once a high poly mesh is loaded.");

    harness.state_mut().session.add_high_poly_bytes("High.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()).unwrap();
    harness.run_steps(3);

    assert!(!harness.get_by_label("Max Frontal Distance").accesskit_node().is_disabled());
    assert!(harness.query_by_label("The cage applies once a high poly mesh is loaded.").is_none());
    harness.get_by_role_and_label(Role::Button, "Reset Skew Correction");
    harness.state_mut().session.doc.project.texture_sets[0].bake.cage.average_normals = false;
    harness.run_steps(3);

    assert!(harness.get_by_label("Edge Protection").accesskit_node().is_disabled(), "the section shows greyed");
    harness.get_by_label("Skew Correction needs Average Normals turned on in the cage settings.");
    harness.state_mut().session.doc.project.texture_sets[0].bake.high_poly.use_low_as_high = true;
    harness.run_steps(3);
    harness.get_by_label("Skew Correction is not available while the low poly mesh is used as the high poly mesh.");
    harness.get_by_label("The cage is not used while the low poly mesh is used as the high poly mesh.");
    assert!(harness.get(by().label_contains("Paint Cage")).accesskit_node().is_disabled(), "the low poly as its own high poly has no cage to paint");
    let settings = &mut harness.state_mut().session.doc.project.texture_sets[0].bake;
    (settings.cage.average_normals, settings.high_poly.use_low_as_high) = (true, false);
    harness.run_steps(3);

    assert!(!harness.get_by_label("Edge Protection").accesskit_node().is_disabled());

    click(&mut harness, "Cage Type");
    click(&mut harness, "Automatic (Experimental)");

    assert_eq!(harness.state().session.doc.project.texture_sets[0].bake.cage.kind, CageType::Automatic);
    for distance_based_only in ["Max Frontal Distance", "Average Normals"] {
        assert!(harness.query_by_label(distance_based_only).is_none(), "the automatic cage has no {distance_based_only}");
    }

    assert!(harness.query(by().label_contains("Paint Cage")).is_none(), "the automatic cage is not painted");
    assert!(
        harness.get_by_role_and_label(Role::CheckBox, "Matching Error").accesskit_node().is_disabled(),
        "greyed with the automatic cage, as in the reference"
    );
    harness.get_by_label("Skew Correction needs a distance-based cage: the automatic cage is fitted to the high poly as it is.");

    click(&mut harness, "Cage Type");
    click(&mut harness, "Custom File");

    assert_eq!(harness.state().session.doc.project.texture_sets[0].bake.cage.kind, CageType::CustomFile);
    harness.get_by_label("Cage Scene");
    assert!(harness.query_by_label("Max Frontal Distance").is_none());
    assert!(harness.get_by_label("Edge Protection").accesskit_node().is_disabled(), "the section shows greyed");
    harness.get_by_label("Skew Correction needs a distance-based cage: a custom cage file sets the ray directions itself.");
    harness.get_by_label("Load Cage");
}

#[test]
fn output_sizes_run_from_32_to_8192_as_in_the_reference() {
    let mut harness = harness(None);
    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);
    harness.get_all_by_value("64").next().unwrap().click();
    harness.run_steps(3);
    for offered in ["32", "8192"] {
        harness.get_by_label(offered);
    }

    assert!(harness.query_by_label("16").is_none());
}

#[test]
fn a_loaded_cage_can_be_removed_while_the_cage_does_not_apply() {
    let mut harness = harness(None);
    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    let session = &mut harness.state_mut().session;
    session.add_high_poly_bytes("High.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()).unwrap();
    let mesh = session.mesh.clone();
    session.doc.bake_meshes.cage =
        Some(texturecraft_engine::BakeMesh { name: "Cage.obj".into(), folder: None, bytes: Default::default(), mesh, base_color: Vec::new() });
    let settings = &mut session.doc.project.texture_sets[0].bake;
    (settings.cage.kind, settings.high_poly.use_low_as_high) = (CageType::CustomFile, true);
    harness.run_steps(3);

    assert!(harness.get_by_label("Load Cage").accesskit_node().is_disabled(), "the cage does not apply");
    assert!(!harness.get_by_label("Remove Cage").accesskit_node().is_disabled());
    click(&mut harness, "Remove Cage");

    assert!(harness.state().session.doc.bake_meshes.cage.is_none());
}

#[test]
fn the_view_menu_picks_the_neutral_or_the_painted_material_in_bake_mode() {
    let mut harness = harness(None);

    // Mesh Map Settings in front would show its map's preview instead of the material
    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(2);

    assert!(harness.state().ui.bake_visualization.neutral_shown, "bake mode starts on the neutral material");
    harness.get_by_value("Neutral Material").click();
    harness.run_steps(3);

    assert!(harness.query_by_label("UV checker").is_none(), "bake mode lists lighting and mesh maps only");
    harness.get_by_label("Material").click();
    harness.run_steps(3);

    assert!(!harness.state().ui.bake_visualization.neutral_shown);
    harness.get_by_value("Material");
}

#[test]
fn texture_set_names_show_in_full_beside_their_resolution() {
    let mut harness = harness(None);
    harness.state_mut().session.doc.project.texture_sets[0].name = "LowerTorso".into();
    harness.state_mut().ui.dock.focus(Panel::TextureSetList);
    harness.run_steps(3);

    // "LowerTorso" is about 75 points wide; cut short it collapses to an ellipsis of about 20
    const FULL_NAME_MIN_WIDTH: f32 = 60.0;

    // The breadcrumb names the set too; the list row is the top-most
    let row = harness.get_all_by_label("LowerTorso").min_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap();
    let width = row.rect().width();
    assert!(width >= FULL_NAME_MIN_WIDTH, "the name is not cut short: {width}");
}

#[test]
fn the_high_poly_scene_is_a_list_with_add_and_remove() {
    let mut harness = harness(None);
    harness.state_mut().session.add_high_poly_bytes("Gunhigh.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()).unwrap();
    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);
    harness.get_by_label("Add High-Poly Meshes");
    harness.get_by_label("Gunhigh.obj");

    // Each file's row has its own ×
    harness.get_by_label("Remove").click();
    harness.run_steps(3);

    assert!(harness.state().session.doc.bake_meshes.high.is_empty());
    assert!(harness.query_by_label("Remove").is_none());
}

#[test]
fn the_eye_greys_the_baking_meshes_and_holds_the_project_mesh() {
    let mut harness = harness(None);
    assert!(!harness.get_by_label("Mesh").accesskit_node().is_disabled(), "the eye starts off");
    harness.get_by_label("Hide Baking Meshes").click();
    harness.run_steps(3);

    for greyed in ["Mesh", "Matching Error", "Cage Surface", "Cage Wireframe Opacity"] {
        assert!(harness.get_by_label(greyed).accesskit_node().is_disabled(), "{greyed} is greyed");
    }
    assert!(harness.state().ui.bake_visualization.project_mesh.shown);

    // The header, not its "Reset Project Mesh (LP)" button further right
    let header = harness.get_all_by_label_contains("Project Mesh (LP)").min_by(|a, b| a.rect().left().total_cmp(&b.rect().left())).unwrap();
    header.click();
    harness.run_steps(3);
    assert!(harness.get_by_label("Project Mesh").accesskit_node().is_disabled(), "the project mesh stays on");
    assert!(!harness.get_by_label("Roughness").accesskit_node().is_disabled(), "the neutral material stays editable");
}

#[test]
fn picking_material_while_painting_keeps_bake_modes_neutral_material() {
    let mut harness = harness(None);
    harness.state_mut().ui.mode = Mode::Painting;
    harness.run_steps(3);

    // Painting mode shows the 3D and 2D views, each with a view menu; the 3D view's comes first
    harness.get_all_by_value("Material").next().unwrap().click();
    harness.run_steps(3);

    // The menu entry, not the dropdown that shows the same word
    let entry = harness.get_all_by_label("Material").find(|node| node.accesskit_node().role() != Role::ComboBox).unwrap();
    entry.click();
    harness.run_steps(3);

    assert!(harness.state().ui.bake_visualization.neutral_shown, "bake mode still opens on the neutral material");
}
