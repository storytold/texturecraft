//! The anchor point in Properties: its name typed in place, and the References band listing what reads it (a click selects that)

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::core::LayerId;
use texturecraft_engine::{StackTarget, execute};
use texturecraft_ui_egui::TextureCraftApp;

use crate::{BAKE_RESOLUTION, FRAME_SECONDS, PROPERTIES_WINDOW, REDRAW_FRAMES, SETTLE_FRAMES, STARTUP_FRAMES, TEXTURE_RESOLUTION};

/// A fill with an anchor point in its mask, read by the mask fill of a layer above, the anchor point selected: (harness, anchor, reading layer, reading effect)
fn anchor_harness() -> (Harness<'static, TextureCraftApp>, LayerId, LayerId, LayerId) {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let low = execute(&mut session, "layer.add.fill", &json!({"roughness": 0.2})).unwrap()["layer"].clone();
    execute(&mut session, "layer.rename", &json!({"layer": low, "name": "Cavity Dirt"})).unwrap();
    execute(&mut session, "effect.add.fill", &json!({"layer": low, "target": "mask", "value": 0.4})).unwrap();
    let anchor = LayerId(execute(&mut session, "effect.add.anchor", &json!({"layer": low, "target": "mask"})).unwrap()["effect"].as_u64().unwrap());
    let top = execute(&mut session, "layer.add.fill", &json!({"roughness": 0.6})).unwrap()["layer"].clone();
    execute(&mut session, "layer.rename", &json!({"layer": top, "name": "Rust"})).unwrap();
    execute(&mut session, "mask.add.black", &json!({"layer": top})).unwrap();
    let reader = LayerId(execute(&mut session, "effect.add.fill", &json!({"layer": top, "anchor": anchor.0})).unwrap()["effect"].as_u64().unwrap());
    execute(&mut session, "layer.select", &json!({"layer": low, "target": anchor.0})).unwrap();
    let mut harness = Harness::builder().with_size(PROPERTIES_WINDOW).with_step_dt(FRAME_SECONDS).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    (harness, anchor, LayerId(top.as_u64().unwrap()), reader)
}

fn anchor_name(harness: &Harness<'_, TextureCraftApp>, anchor: LayerId) -> String {
    harness.state().session.doc.project.effect(anchor).unwrap().1.name.clone()
}

/// Clicks the name field, selects its text and types `text`
fn type_name(harness: &mut Harness<'_, TextureCraftApp>, text: &str) {
    harness.get_by_role_and_label(Role::TextInput, "Anchor point name").click();
    harness.run_steps(SETTLE_FRAMES);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.event(egui::Event::Text(text.into()));
    harness.run_steps(SETTLE_FRAMES);
}

#[test]
fn the_name_field_renames_on_enter_in_one_undo_step_and_escape_keeps_it() {
    let (mut harness, anchor, _, _) = anchor_harness();
    assert_eq!(anchor_name(&harness, anchor), "Cavity Dirt mask", "a mask anchor point is named after its layer");

    let steps = harness.state().session.history_labels().len();
    type_name(&mut harness, "Edges");

    assert_eq!(anchor_name(&harness, anchor), "Cavity Dirt mask", "nothing changes while typing");

    harness.key_press(Key::Enter);
    harness.run_steps(SETTLE_FRAMES);

    assert_eq!(anchor_name(&harness, anchor), "Edges");
    assert_eq!(harness.state().session.history_labels().len(), steps + 1, "one undo step");

    type_name(&mut harness, "Never applied");
    harness.key_press(Key::Escape);
    harness.run_steps(SETTLE_FRAMES);

    assert_eq!(anchor_name(&harness, anchor), "Edges", "Escape keeps the name");
}

#[test]
fn a_click_on_a_reference_selects_what_reads_the_anchor_point() {
    let (mut harness, _, top, reader) = anchor_harness();
    assert!(harness.state().session.selected_layer() != Some(top));

    harness.get_by_role_and_label(Role::Button, "Rust Mask fill").click();
    harness.run_steps(REDRAW_FRAMES);

    assert_eq!(harness.state().session.selected_layer(), Some(top));
    assert_eq!(harness.state().session.target, StackTarget::Effect(reader));
}

/// A reader's name longer than the band is cut short before its kind, and hovering the row shows it in full
#[test]
fn a_long_reference_name_is_cut_short_and_shows_in_full_on_hover() {
    const LONG_NAME: &str = "Rust along every lower edge of the hull plates where rain water collects and runs";

    // Frames past the tooltip delay
    const TOOLTIP_FRAMES: usize = 60;
    let (mut harness, _, top, _) = anchor_harness();
    harness.state_mut().run("layer.rename", json!({"layer": top.0, "name": LONG_NAME}));
    harness.run_steps(REDRAW_FRAMES);
    let row = harness.get_by_role_and_label(Role::Button, &format!("{LONG_NAME} Mask fill")).rect();
    let before = harness.query_all_by_label(LONG_NAME).count();

    harness.hover_at(row.center());
    harness.run_steps(TOOLTIP_FRAMES);

    // The row's own label is its name and kind; the Layers row shows the name too
    assert_eq!(harness.query_all_by_label(LONG_NAME).count(), before + 1, "the tooltip carries the full name");
}

#[test]
fn a_name_typed_then_left_with_shift_tab_is_kept() {
    let (mut harness, anchor, _, _) = anchor_harness();
    let steps = harness.state().session.history_labels().len();
    type_name(&mut harness, "Edges");

    // The focus moves back to a control drawn before the field: the field never sees itself focused again
    harness.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
    harness.run_steps(SETTLE_FRAMES);

    assert_eq!(anchor_name(&harness, anchor), "Edges", "the typed name is kept");
    assert_eq!(harness.state().session.history_labels().len(), steps + 1, "in one undo step");
}
