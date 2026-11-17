//! A click on a simple shelf material sets the brush's material and adds no layer; a smart material still adds its layers

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
fn a_click_on_a_smart_material_still_adds_its_layers() {
    let label = find("iron_rusted").unwrap().label;
    let mut harness = shelf(SMART_TYPE, label);
    let (stacks, values) = (stack_sizes(&harness), harness.state().session.tool.values.clone());

    harness.get_all_by_label(label).next().unwrap().click();
    harness.run_steps(3);

    let app = harness.state();
    assert_eq!(stack_sizes(&harness).iter().sum::<usize>(), stacks.iter().sum::<usize>() + 1, "one folder");
    assert_eq!(app.session.tool.values, values, "the brush is untouched");
}
