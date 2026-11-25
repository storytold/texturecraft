//! A click on a simple shelf material sets the brush's material and adds no layer; a click on a smart material changes nothing

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_engine::materials::{brush_values, find};
use texturecraft_ui_egui::TextureCraftApp;

/// The shelf's Materials and Smart materials types
const MATERIALS_TYPE: usize = 0;
const SMART_TYPE: usize = 1;

/// The sample project with the shelf on type `tab`, filtered to `label`
fn shelf(tab: usize, label: &str) -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1920.0, 1032.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.shelf_tab = tab;
    harness.state_mut().ui.shelf_filter = label.to_lowercase();
    harness.run_steps(4);
    harness
}

fn stack_sizes(harness: &Harness<'_, TextureCraftApp>) -> Vec<usize> {
    harness.state().session.doc.project.texture_sets.iter().map(|set| set.stack.len()).collect()
}

#[test]
fn a_click_on_a_simple_material_sets_the_brush_material_and_adds_no_layer() {
    let label = find("gold_polished").unwrap().label;
    let mut harness = shelf(MATERIALS_TYPE, label);
    let (stacks, history, values) = (stack_sizes(&harness), harness.state().session.history_labels(), harness.state().session.tool.values.clone());
    assert_ne!(values[&texturecraft_engine::core::Channel::Metallic], [1.0; 3], "the brush starts non-metallic");

    harness.get_all_by_label(label).next().unwrap().click();
    harness.run_steps(3);

    let app = harness.state();
    assert_eq!(stack_sizes(&harness), stacks, "no layer");
    assert_eq!(app.session.history_labels(), history, "no history entry");
    for (channel, value) in brush_values("gold_polished").unwrap() {
        assert_eq!(app.session.tool.values[&channel], value, "{channel:?}");
    }
}

#[test]
fn a_click_on_a_smart_material_adds_no_layer_and_leaves_the_brush() {
    let label = find("iron_rusted").unwrap().label;
    let mut harness = shelf(SMART_TYPE, label);
    let (stacks, history, tool) = (stack_sizes(&harness), harness.state().session.history_labels(), harness.state().session.tool.clone());

    harness.get_all_by_label(label).next().unwrap().click();
    harness.run_steps(3);

    let app = harness.state();
    assert_eq!(stack_sizes(&harness), stacks, "no layer");
    assert_eq!(app.session.history_labels(), history, "no history entry");
    assert_eq!(serde_json::to_value(&app.session.tool).unwrap(), serde_json::to_value(&tool).unwrap(), "the brush is untouched");
}

#[test]
fn clicking_a_saved_material_loads_it_above_the_selected_layer() {
    let mut harness = shelf(SMART_TYPE, "SavedFixture");
    let layer = harness.state().session.selected_layer().unwrap();
    let bytes = harness.state().session.smart_material_bytes(layer, "Saved fixture").unwrap();
    let file = std::env::temp_dir().join(format!("SavedFixture-{}.tcmat", std::process::id()));
    std::fs::write(&file, bytes).unwrap();
    harness.state_mut().library = Some(vec![file.clone()]);
    harness.state_mut().ui.shelf_path = vec!["user".into()];
    harness.run_steps(3);
    let before = stack_sizes(&harness);

    harness.get_by_label_contains("SavedFixture-").click();
    harness.run_steps(3);
    std::fs::remove_file(file).unwrap();

    let ts = harness.state().session.ts();
    assert_eq!(stack_sizes(&harness)[ts], before[ts] + 1);
    assert_ne!(harness.state().session.selected_layer(), Some(layer));
}
