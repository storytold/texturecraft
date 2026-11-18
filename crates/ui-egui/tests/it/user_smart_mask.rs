//! Saving a layer's mask as a user smart mask from its context menu, and the shelf listing and applying it

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};
use texturecraft_engine::core::LayerId;
use texturecraft_engine::execute;
use texturecraft_ui_egui::TextureCraftApp;

const WINDOW: egui::Vec2 = egui::vec2(1920.0, 1032.0);
/// The Smart masks type's and the Smart materials type's positions in the shelf's type row (its table is crate-private)
const SMART_MASKS_TYPE: usize = 2;
const SMART_MATERIALS_TYPE: usize = 1;

fn harness(prepare: impl FnOnce(&mut texturecraft_engine::Session)) -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    prepare(&mut session);
    let mut harness = Harness::builder().with_size(WINDOW).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(4);
    harness
}

/// A new fill layer called `name`, with a mask of two effects when `masked`
fn add_layer(session: &mut texturecraft_engine::Session, name: &str, masked: bool) -> LayerId {
    let layer = LayerId(execute(session, "layer.add.fill", &Value::Null).unwrap()["layer"].as_u64().unwrap());
    execute(session, "layer.rename", &json!({"layer": layer.0, "name": name})).unwrap();
    if masked {
        execute(session, "mask.add.black", &json!({"layer": layer.0})).unwrap();
        execute(session, "effect.add.generator", &json!({"layer": layer.0, "generator": "edgeWear"})).unwrap();
        execute(session, "effect.add.filter", &json!({"layer": layer.0, "target": "mask", "filter": "invert"})).unwrap();
    }
    layer
}

fn mask_effect_count(harness: &Harness<'_, TextureCraftApp>, layer: LayerId) -> usize {
    harness.state().session.doc.project.layer(layer).unwrap().mask.as_ref().map_or(0, |mask| mask.effects.len())
}

fn menu_item_disabled(name: &str) -> bool {
    let mut harness = harness(|session| {
        add_layer(session, "Plain layer", false);
        add_layer(session, "Masked layer", true);
    });
    harness.get_all_by_label(name).next().unwrap().click_secondary();
    harness.run_steps(3);
    harness.get_all_by_label("Create smart mask").last().unwrap().accesskit_node().is_disabled()
}

#[test]
fn create_smart_mask_is_in_the_menu_of_a_layer_with_a_mask() {
    assert!(!menu_item_disabled("Masked layer"), "a mask can be saved");
}

#[test]
fn create_smart_mask_is_off_for_a_layer_without_a_mask() {
    assert!(menu_item_disabled("Plain layer"), "there is no mask to save");
}

#[test]
fn a_saved_smart_mask_is_listed_with_the_smart_masks_and_applies_to_the_selected_layer() {
    let mut saved = texturecraft_engine::sample_session(256, 64).unwrap();
    let source = add_layer(&mut saved, "Source", true);
    let dir = std::env::temp_dir().join(format!("tc-ui-smart-mask-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Stencil.tcmask");
    std::fs::write(&file, saved.smart_mask_bytes(source, "Stencil").unwrap()).unwrap();

    let mut harness = harness(|_| {});
    let target = harness.state().session.selected_layer().unwrap();
    let before = mask_effect_count(&harness, target);
    harness.state_mut().library = Some(vec![file.clone()]);
    harness.state_mut().ui.shelf_path = vec!["user".into()];
    harness.state_mut().ui.shelf_tab = SMART_MATERIALS_TYPE;
    harness.run_steps(3);
    assert!(harness.query_by_label_contains("Stencil").is_none(), "it is not a smart material");

    harness.state_mut().ui.shelf_tab = SMART_MASKS_TYPE;
    harness.run_steps(3);
    harness.get_by_label_contains("Stencil").click();
    harness.run_steps(3);
    std::fs::remove_dir_all(&dir).unwrap();

    assert_ne!(before, 2, "the layer did not already have the two effects");
    assert_eq!(mask_effect_count(&harness, target), 2, "the click replaces the selected layer's mask with the saved one");
}
