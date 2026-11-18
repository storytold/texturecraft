//! Bake presets (save, load, delete), "Apply to All Texture Sets" and the summary toast when a bake finishes.

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_engine::core::{BakeAntialiasing, BakeResolution, BakedMap};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;
use texturecraft_ui_egui::state::Mode;

fn harness() -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    for texture_set in &mut session.doc.project.texture_sets {
        texture_set.bake = Default::default();
        texture_set.bake.common.resolution = BakeResolution::R64;
        texture_set.bake.maps.select([BakedMap::Id]);
    }

    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|context| TextureCraftApp::new(context, session));
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

fn open_menu(harness: &mut Harness<'_, TextureCraftApp>) {
    harness.get_by_label_contains("Presets").click();
    harness.run_steps(3);
}

fn save_preset(harness: &mut Harness<'_, TextureCraftApp>, name: &str) {
    open_menu(harness);
    harness.state_mut().ui.bake_preset_name = name.to_string();
    harness.run_steps(2);
    click(harness, "Save Preset");
}

fn antialiasing(harness: &Harness<'_, TextureCraftApp>, set: usize) -> BakeAntialiasing {
    harness.state().session.doc.project.texture_sets[set].bake.common.antialiasing
}

#[test]
fn a_saved_preset_loads_back_into_the_active_set_as_one_undo_step() {
    let mut harness = harness();
    harness.state_mut().session.doc.project.texture_sets[0].bake.common.antialiasing = BakeAntialiasing::Subsampling4x4;
    save_preset(&mut harness, "Final");
    assert_eq!(harness.state().ui.bake_presets.len(), 1);
    assert_eq!(harness.state().ui.bake_presets[0].name, "Final");
    assert!(harness.state().ui.bake_preset_name.is_empty(), "the name field is cleared");

    harness.state_mut().session.doc.project.texture_sets[0].bake.common.antialiasing = BakeAntialiasing::None;
    let steps = harness.state().session.history_labels().len();
    open_menu(&mut harness);
    click(&mut harness, "Final");

    assert_eq!(antialiasing(&harness, 0), BakeAntialiasing::Subsampling4x4);
    assert_eq!(antialiasing(&harness, 1), BakeAntialiasing::None, "only the active set loads it");
    assert_eq!(harness.state().session.history_labels().len(), steps + 1);
    harness.state_mut().session.undo().unwrap();
    assert_eq!(antialiasing(&harness, 0), BakeAntialiasing::None);
}

#[test]
fn loading_a_preset_keeps_the_sets_include_in_bake_flag() {
    let mut harness = harness();
    save_preset(&mut harness, "Plain");
    harness.state_mut().session.doc.project.texture_sets[0].bake.common.is_enabled = false;
    open_menu(&mut harness);
    click(&mut harness, "Plain");
    assert!(!harness.state().session.doc.project.texture_sets[0].bake.common.is_enabled);
}

#[test]
fn saving_under_an_existing_name_replaces_it_and_delete_removes_it() {
    let mut harness = harness();
    save_preset(&mut harness, "Fast");
    harness.state_mut().session.doc.project.texture_sets[0].bake.common.antialiasing = BakeAntialiasing::Subsampling2x2;
    save_preset(&mut harness, "Fast");
    assert_eq!(harness.state().ui.bake_presets.len(), 1);
    assert_eq!(harness.state().ui.bake_presets[0].settings.common.antialiasing, BakeAntialiasing::Subsampling2x2);

    open_menu(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Delete preset Fast").click();
    harness.run_steps(3);
    assert!(harness.state().ui.bake_presets.is_empty());
}

#[test]
fn presets_survive_the_saved_ui_state() {
    let mut harness = harness();
    save_preset(&mut harness, "Keep");
    let json = serde_json::to_string(&harness.state().ui).unwrap();
    let back: texturecraft_ui_egui::UiState = serde_json::from_str(&json).unwrap();
    assert_eq!(back.bake_presets, harness.state().ui.bake_presets);
}

#[test]
fn apply_to_all_copies_the_active_sets_settings_and_keeps_each_include_flag() {
    let mut harness = harness();
    {
        let session = &mut harness.state_mut().session;
        let first = &mut session.doc.project.texture_sets[0].bake;
        first.common.antialiasing = BakeAntialiasing::Subsampling8x8;
        first.maps.select([BakedMap::Id, BakedMap::Height]);
        session.doc.project.texture_sets[1].bake.common.is_enabled = false;
    }
    assert_ne!(antialiasing(&harness, 1), BakeAntialiasing::Subsampling8x8);

    open_menu(&mut harness);
    click(&mut harness, "Apply to All Texture Sets");

    let second = &harness.state().session.doc.project.texture_sets[1].bake;
    assert_eq!(second.common.antialiasing, BakeAntialiasing::Subsampling8x8);
    assert_eq!(second.maps.enabled().collect::<Vec<_>>(), vec![BakedMap::Id, BakedMap::Height]);
    assert!(!second.common.is_enabled, "Include in Bake stays as it was");
    harness.state_mut().session.undo().unwrap();
    assert_ne!(antialiasing(&harness, 1), BakeAntialiasing::Subsampling8x8, "one undo step");
}

fn run_to_the_end(harness: &mut Harness<'static, TextureCraftApp>) {
    for _ in 0..4000 {
        harness.run_steps(1);
        if harness.state().bake.is_none() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("the bake did not finish");
}

#[test]
fn a_finished_bake_toasts_maps_and_time_and_logs_them() {
    let mut harness = harness();
    harness.state_mut().start_bake(vec![0]);
    run_to_the_end(&mut harness);

    let toast = harness.state().toasts.last().expect("a summary toast");
    assert!(toast.text.starts_with("Baked 1 maps in "), "{}", toast.text);
    assert!(!toast.error);
    assert!(harness.state().session.log.last().unwrap().starts_with("Bake finished: 1 maps in "));
}

#[test]
fn a_failed_set_makes_an_error_toast_that_points_to_the_log() {
    let mut harness = harness();
    {
        let set = &mut harness.state_mut().session.doc.project.texture_sets[1];
        set.uv_tiles = vec![1001];
        set.bake_excluded_tiles.insert(1001);
    }
    harness.state_mut().start_bake(vec![0, 1]);
    run_to_the_end(&mut harness);

    let toast = harness.state().toasts.last().expect("a summary toast");
    assert!(toast.error);
    assert!(toast.text.contains("1 failed") && toast.text.contains("Log"), "{}", toast.text);
    let log = &harness.state().session.log;
    assert!(log.iter().any(|line| line.starts_with("Bake of texture set 1 failed")), "{log:?}");
}
