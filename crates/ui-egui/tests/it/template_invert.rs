//! Export dialog › Output templates: each output channel has an Invert toggle that is saved in the template.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::core::Channel;
use texturecraft_engine::formats::{Dest, ExportPreset, FileFormat, MapSpec, SourceRef};
use texturecraft_ui_egui::TextureCraftApp;

const TEMPLATE: &str = "Packed test";

mod library;

use library::Library;

fn harness() -> (Harness<'static, TextureCraftApp>, Library) {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();

    // The user's template library is a folder; use a temporary one, not the real one
    let dir = std::env::temp_dir().join(format!("texturecraft-template-invert-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    h.state_mut().session.load_export_presets(dir.clone());
    h.run_steps(4);
    (h, Library(dir))
}

/// The channel toggles of the open dialog (the dialog sits left of the properties panel, which has its own Invert controls)
fn toggles<'h>(h: &'h Harness<'_, TextureCraftApp>) -> Vec<egui_kittest::Node<'h>> {
    h.get_all_by_label("Invert").filter(|n| n.rect().max.x < DIALOG_RIGHT_EDGE).collect()
}

const DIALOG_RIGHT_EDGE: f32 = 1200.0;

fn template() -> ExportPreset {
    let ch = |channel| SourceRef::Channel { channel, component: 0 };
    ExportPreset {
        name: TEMPLATE.into(),
        maps: vec![MapSpec {
            file: "$mesh_$textureSet_Packed".into(),
            format: FileFormat::Png,
            bit_depth: 8,
            channels: vec![(Dest::R, ch(Channel::Roughness)), (Dest::G, ch(Channel::Metallic)), (Dest::B, SourceRef::Constant(0.5))],
            inverted: Vec::new(),
        }],
    }
}

#[test]
fn invert_toggle_changes_the_saved_template() {
    let (mut h, _library) = harness();
    let app = h.state_mut();
    app.run("export.preset.save", json!({ "preset": serde_json::to_value(template()).unwrap() })).expect("save template");
    app.ui.export.preset = TEMPLATE.into();
    app.ui.export.tab = 1;
    app.ui.windows.export = true;
    h.run_steps(4);

    let saved = |h: &Harness<'_, TextureCraftApp>| h.state().session.export_preset(TEMPLATE).unwrap().maps[0].inverted.clone();
    assert!(saved(&h).is_empty());

    // One Invert toggle per channel: R, G, B
    assert_eq!(toggles(&h).len(), 3);
    toggles(&h)[1].click();
    h.run_steps(4);
    assert_eq!(saved(&h), vec![Dest::G], "the second channel (G) is inverted");
    toggles(&h)[1].click();
    h.run_steps(4);
    assert!(saved(&h).is_empty(), "a second click turns it off");
}
