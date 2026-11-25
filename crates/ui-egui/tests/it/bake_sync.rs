//! Sharing bake settings between texture sets from the Mesh Map Bakers panel

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_engine::core::{BakeResolution, BakeSyncGroup, BakedMap};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;
use texturecraft_ui_egui::state::Mode;

/// A bake-mode app on the two-set sample with Mesh Map Bakers in front
fn harness() -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    for set in &mut session.doc.project.texture_sets {
        set.bake = Default::default();
        set.bake.common.resolution = BakeResolution::R64;
    }
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1500.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(4);
    harness.state_mut().ui.dock.focus(Panel::MeshMapBakers);
    harness.run_steps(3);
    harness
}

fn click(harness: &mut Harness<'_, TextureCraftApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run_steps(3);
}

fn second_name(harness: &Harness<'_, TextureCraftApp>) -> String {
    harness.state().session.doc.project.texture_sets[1].name.clone()
}

#[test]
fn the_common_settings_sync_dialog_takes_a_set_out_of_the_group() {
    let mut harness = harness();
    click(&mut harness, "Synchronize Common Settings");
    harness.get_by_label("Common Settings Sync");
    harness.get_by_role_and_label(Role::CheckBox, "Deselect All");

    let second = second_name(&harness);
    harness.get_by_role_and_label(Role::CheckBox, &second).click();
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::CheckBox, "Select All");
    click(&mut harness, "OK");

    let project = &harness.state().session.doc.project;
    assert!(harness.query_by_label("Common Settings Sync").is_none());
    assert!(!project.texture_sets[1].bake.sync.common);
    assert_eq!(project.bake_synced_sets(0, BakeSyncGroup::Common), [0]);
    assert_eq!(project.bake_synced_sets(1, BakeSyncGroup::Map(BakedMap::Normal)), [0, 1], "map sync stays");
}

#[test]
fn cancel_leaves_the_group_as_it_was() {
    let mut harness = harness();
    click(&mut harness, "Synchronize Common Settings");
    let second = second_name(&harness);
    harness.get_by_role_and_label(Role::CheckBox, &second).click();
    harness.run_steps(3);
    click(&mut harness, "Cancel");

    assert!(harness.state().session.doc.project.texture_sets[1].bake.sync.common);
}

#[test]
fn common_settings_edits_reach_only_the_sets_sharing_them() {
    let mut harness = harness();
    harness.state_mut().session.doc.project.texture_sets[1].bake.sync.common = false;
    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::CheckBox, "Apply Diffusion").click();
    harness.run_steps(3);

    let sets = &harness.state().session.doc.project.texture_sets;
    assert_ne!(sets[0].bake.common.apply_diffusion, sets[1].bake.common.apply_diffusion);
}

#[test]
fn the_list_menu_applies_the_checked_maps_to_all_sets() {
    let mut harness = harness();
    harness.state_mut().session.doc.project.texture_sets[0].bake.maps.select([BakedMap::Normal, BakedMap::Height]);
    harness.run_steps(3);
    click(&mut harness, "Mesh Map Bakers Menu");
    click(&mut harness, "Apply Checked to All Texture Sets");
    let enabled: Vec<BakedMap> = harness.state().session.doc.project.texture_sets[1].bake.maps.enabled().collect();
    assert_eq!(enabled, [BakedMap::Normal, BakedMap::Height]);
}

#[test]
fn a_rows_menu_checks_the_map_for_sets_picked_in_the_dialog() {
    let mut harness = harness();
    for set in &mut harness.state_mut().session.doc.project.texture_sets {
        set.bake.maps.select([]);
    }
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Button, "Height").click_secondary();
    harness.run_steps(3);
    click(&mut harness, "Check Height for More Texture Sets...");
    harness.get_by_label("Check Height for More Texture Sets");

    let first = harness.state().session.doc.project.texture_sets[0].name.clone();
    assert!(harness.query_by_role_and_label(Role::CheckBox, &first).is_none(), "the active set is the source, not a target");
    let second = second_name(&harness);
    harness.get_by_role_and_label(Role::CheckBox, &second).click();
    harness.run_steps(3);
    click(&mut harness, "OK");

    let sets = &harness.state().session.doc.project.texture_sets;
    assert!(!sets[0].bake.maps.height.is_enabled && sets[1].bake.maps.height.is_enabled);
}

#[test]
fn replacing_the_project_discards_the_open_sync_picker() {
    let mut harness = harness();
    click(&mut harness, "Synchronize Common Settings");
    harness.get_by_label("Common Settings Sync");

    harness.state_mut().session = texturecraft_engine::sample_session(256, 64).unwrap();
    harness.run_steps(3);

    assert!(harness.query_by_label("Common Settings Sync").is_none());
}

#[test]
fn keyboard_opens_the_sync_picker_and_returns_to_its_button() {
    let mut harness = harness();
    let opener = "Synchronize Common Settings";
    harness.get_by_role_and_label(Role::Button, opener).focus();
    harness.run_steps(2);

    let node = harness.get_by_role_and_label(Role::Button, opener);
    assert!(node.is_focused());
    let center = node.rect().center();
    assert!(
        harness
            .output()
            .shapes
            .iter()
            .any(|clipped| { matches!(&clipped.shape, egui::Shape::Rect(shape) if shape.stroke.width > 1.0 && shape.rect.center().distance(center) < 1.0) }),
        "the focused sync control has a visible outline"
    );

    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);

    assert!(harness.get_by_role_and_label(Role::Button, "Close").is_focused());
    harness.key_press(egui::Key::Tab);
    harness.run_steps(2);

    assert!(!harness.get_by_role_and_label(Role::Button, "Close").is_focused());
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);

    assert!(harness.query_by_label("Common Settings Sync").is_none());
    assert!(harness.get_by_role_and_label(Role::Button, opener).is_focused());
}
