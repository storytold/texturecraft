//! A brush tool's Properties explain themselves in tooltips: no line of instructions under the controls

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::execute;
use texturecraft_ui_egui::TextureCraftApp;

use crate::{BAKE_RESOLUTION, FRAME_SECONDS, PROPERTIES_WINDOW, STARTUP_FRAMES, TEXTURE_RESOLUTION};

fn with_tool(tool: &str) -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    execute(&mut session, "tool.select", &json!({"tool": tool})).unwrap();
    let mut harness = Harness::builder()
        .with_size(PROPERTIES_WINDOW)
        .with_step_dt(FRAME_SECONDS)
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    harness
}

/// The words of every label in the app
fn labels(harness: &Harness<'_, TextureCraftApp>) -> Vec<String> {
    harness.query_all_by_role(Role::Label).filter_map(|node| node.accesskit_node().value().map(|value| value.to_string())).collect()
}

#[test]
fn the_brush_section_has_no_shortcut_lines() {
    let harness = with_tool("paint");
    assert!(harness.query_by_role_and_label(Role::Slider, "Size").is_some(), "the Brush section is on screen");
    let labels = labels(&harness);
    let shortcut = labels.iter().find(|label| label.contains("drag"));
    assert!(shortcut.is_none(), "the view shortcuts are in the keymap, not under the brush: {shortcut:?}");
}

#[test]
fn the_smudge_section_has_no_explanation_lines() {
    let labels = labels(&with_tool("smudge"));
    assert!(labels.iter().any(|label| label == "Channels dragged"), "the Smudge section is on screen");
    let sentence = labels.iter().find(|label| label.ends_with('.'));
    assert!(sentence.is_none(), "what Smudge does is in its tooltips: {sentence:?}");
}

#[test]
fn the_clone_section_shows_its_source_state_without_instructions() {
    let labels = labels(&with_tool("clone"));
    assert!(labels.iter().any(|label| label == "Source: none"), "no source set yet reads as state");
    let sentence = labels.iter().find(|label| label.ends_with('.'));
    assert!(sentence.is_none(), "how to set the source is in the tooltip: {sentence:?}");
}
