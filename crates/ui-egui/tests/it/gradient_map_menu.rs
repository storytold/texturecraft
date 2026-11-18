//! The Gradient Map item of a fill's Base color source menu turns color stops on and off.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::core::{Channel, LayerId, LayerKind};
use texturecraft_ui_egui::TextureCraftApp;

/// The Base color channel's name in Properties (the rightmost "Base color" label).
fn base_color<'a>(h: &'a Harness<'_, TextureCraftApp>) -> egui_kittest::Node<'a> {
    h.get_all_by_label("Base color").max_by(|a, b| a.rect().right().total_cmp(&b.rect().right())).unwrap()
}

/// Opens the Base color source menu and clicks its Gradient Map item.
fn click_gradient_map(h: &mut Harness<'_, TextureCraftApp>) {
    base_color(h).scroll_to_me();

    // The scroll animates for about twenty frames
    h.run_steps(30);
    let row = base_color(h).rect().center().y;
    let menu = h.get_all_by_label(egui_phosphor::regular::DOTS_THREE).find(|n| (n.rect().center().y - row).abs() < 6.0).unwrap();
    let at = menu.rect().center();
    menu.click();
    h.run_steps(2);

    // The filter of the same name may show elsewhere (the shelf): the menu's item is the nearest
    let item = h.get_all_by_label("Gradient Map").min_by(|a, b| a.rect().center().distance(at).total_cmp(&b.rect().center().distance(at))).unwrap();
    item.click();
    h.run_steps(3);
}

fn ramp(h: &Harness<'_, TextureCraftApp>, layer: LayerId) -> Vec<[f32; 3]> {
    let LayerKind::Fill(fill) = &h.state().session.doc.project.layers[&layer].kind else { panic!("a fill layer") };
    fill.source(Channel::BaseColor).ramp().to_vec()
}

#[test]
fn the_base_color_menu_turns_a_procedural_gradient_map_on_and_off() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let out = texturecraft_engine::execute(&mut session, "layer.add.fill", &json!({})).unwrap();
    let layer = LayerId(out["layer"].as_u64().unwrap());
    let set = json!({"layer": layer.0, "channel": "baseColor", "procedural": "Clouds", "low": [0.0, 0.25, 0.5], "high": [1.0, 0.75, 0.5]});
    texturecraft_engine::execute(&mut session, "layer.fill.set", &set).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));

    // The labels below are English; a first run would speak the system's language
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);

    // The stops start at low, halfway and high, so the fill looks the same until they change
    click_gradient_map(&mut h);
    assert_eq!(ramp(&h, layer), vec![[0.0, 0.25, 0.5], [0.5; 3], [1.0, 0.75, 0.5]]);
    click_gradient_map(&mut h);
    assert!(ramp(&h, layer).is_empty(), "the item turns the stops off again");
}
