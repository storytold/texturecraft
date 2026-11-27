use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::core::LayerId;
use texturecraft_engine::execute;

use super::anchor_link::layer_anchor_links;
use super::broken_anchor_order::{broken_anchor_orders, fix_anchor_order_command};
use super::highlighted_anchor;
use crate::TextureCraftApp;
use crate::i18n::TEST_LANGUAGE;

const WINDOW_SIZE: egui::Vec2 = egui::vec2(1400.0, 900.0);
const STARTUP_FRAMES: usize = 3;
const SETTLE_FRAMES: usize = 3;

fn anchor_harness(moved_above: bool) -> (Harness<'static, TextureCraftApp>, LayerId, LayerId, LayerId) {
    let mut session = texturecraft_engine::sample_session(128, 128).unwrap();
    let lower = LayerId(execute(&mut session, "layer.add.fill", &json!({"name": "Base", "color": [0.4, 0.4, 0.4]})).unwrap()["layer"].as_u64().unwrap());
    let anchor = LayerId(execute(&mut session, "effect.add.anchor", &json!({"layer": lower.0, "name": "Base Anchor"})).unwrap()["effect"].as_u64().unwrap());
    let reader = LayerId(execute(&mut session, "layer.add.fill", &json!({"name": "Reader"})).unwrap()["layer"].as_u64().unwrap());
    execute(&mut session, "layer.fill.set", &json!({"layer": reader.0, "channel": "baseColor", "anchor": anchor.0})).unwrap();
    if moved_above {
        execute(&mut session, "layer.move", &json!({"layer": lower.0, "above": reader.0, "inside": false})).unwrap();
    }

    let mut harness =
        Harness::builder().with_size(WINDOW_SIZE).with_context_setup(crate::framework::install_headless).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    (harness, lower, anchor, reader)
}

#[test]
fn broken_anchor_order_detects_anchor_above_reader_and_fix_command_restores_below() {
    let _language = TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut session = texturecraft_engine::sample_session(128, 128).unwrap();
    let lower = LayerId(execute(&mut session, "layer.add.fill", &json!({"name": "Anchor Owner"})).unwrap()["layer"].as_u64().unwrap());
    let anchor = LayerId(execute(&mut session, "effect.add.anchor", &json!({"layer": lower.0, "name": "Height Stamp"})).unwrap()["effect"].as_u64().unwrap());
    let reader = LayerId(execute(&mut session, "layer.add.fill", &json!({"name": "Top Wear"})).unwrap()["layer"].as_u64().unwrap());
    execute(&mut session, "layer.fill.set", &json!({"layer": reader.0, "channel": "baseColor", "anchor": anchor.0})).unwrap();

    assert!(broken_anchor_orders(&session.doc.project, 0).is_empty(), "valid below order reports no warnings");
    let valid_links = layer_anchor_links(&session.doc.project, reader);
    assert_eq!(valid_links.len(), 1);
    assert!(!valid_links[0].broken);

    // Move the anchor layer above its reader so the link breaks
    execute(&mut session, "layer.move", &json!({"layer": lower.0, "above": reader.0, "inside": false})).unwrap();
    let broken = broken_anchor_orders(&session.doc.project, 0);
    assert_eq!(broken.len(), 1);
    assert_eq!(broken[0].anchor_name, "Height Stamp");
    assert_eq!(broken[0].lowest_reader, reader);
    assert!(layer_anchor_links(&session.doc.project, reader)[0].broken);

    let fix = fix_anchor_order_command(&session.doc.project, &broken[0]);
    execute(&mut session, "layer.move", &fix).unwrap();
    assert!(broken_anchor_orders(&session.doc.project, 0).is_empty(), "fix command restores the anchor below its reader");
    assert!(!layer_anchor_links(&session.doc.project, reader)[0].broken);
}

#[test]
fn reading_layer_shows_anchor_badge_and_hover_highlights_anchor_point() {
    let _language = TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut harness, _, anchor, _) = anchor_harness(false);

    let badge = harness.get_by_role_and_label(Role::Button, "Linked to anchor: Base Anchor\nClick to select");
    badge.hover();
    harness.run_steps(SETTLE_FRAMES);

    assert_eq!(highlighted_anchor(&harness.ctx), Some(anchor), "hovering the badge highlights the linked anchor");
}

#[test]
fn out_of_order_anchor_warning_banner_fixes_layer_order() {
    let _language = TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut harness, _, _, reader) = anchor_harness(true);

    assert_eq!(broken_anchor_orders(&harness.state().session.doc.project, 0).len(), 1);
    harness.get_by_role_and_label(Role::Button, "Fix").click();
    harness.run_steps(SETTLE_FRAMES);

    assert!(broken_anchor_orders(&harness.state().session.doc.project, 0).is_empty(), "Fix button restores the anchor below its reader");
    assert!(harness.query_by_role_and_label(Role::Button, "Fix").is_none());
    assert!(!layer_anchor_links(&harness.state().session.doc.project, reader)[0].broken);
}
