//! What the fill screens' tests share: the sample app with a new fill selected, the fill as the document holds it, and real pointer clicks on its controls

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use texturecraft_engine::core::{Fill, LayerId, LayerKind};
use texturecraft_ui_egui::TextureCraftApp;

use crate::filter_support::click_at;
use crate::{BAKE_RESOLUTION, SETTLE_FRAMES, STARTUP_FRAMES, TEXTURE_RESOLUTION};

/// The app `window` big with a new fill layer (`settings` given to layer.add.fill, then projected as `projection`) selected in Properties, `tab` clicked, and the layer's id
pub(crate) fn fill_app(settings: Value, projection: Option<&str>, tab: Option<&str>, window: egui::Vec2) -> (Harness<'static, TextureCraftApp>, LayerId) {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let added = texturecraft_engine::execute(&mut session, "layer.add.fill", &settings).unwrap();
    let layer = added["layer"].as_u64().unwrap();
    if let Some(projection) = projection {
        texturecraft_engine::execute(&mut session, "layer.fill.projection", &json!({"layer": layer, "projection": projection})).unwrap();
    }
    let mut harness = Harness::builder().with_size(window).build_eframe(|cc| TextureCraftApp::new(cc, session));

    // The labels below are English; a first run would speak the system's language
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    if let Some(tab) = tab {
        click_node(&mut harness, Role::Tab, tab);
    }
    (harness, LayerId(layer))
}

/// The fill layer as the document holds it
pub(crate) fn fill_of(harness: &Harness<'_, TextureCraftApp>, layer: LayerId) -> Fill {
    match &harness.state().session.doc.project.layer(layer).unwrap().kind {
        LayerKind::Fill(fill) => fill.clone(),
        other => panic!("not a fill: {other:?}"),
    }
}

/// A real pointer click in the middle of the `role` named `label`, then frames for the edit to show
pub(crate) fn click_node(harness: &mut Harness<'_, TextureCraftApp>, role: Role, label: &str) {
    let at = harness.get_by_role_and_label(role, label).rect().center();
    click_at(harness, at);
    harness.run_steps(SETTLE_FRAMES);
}

/// A real pointer click in the middle of the widget named `label`, whatever its role
pub(crate) fn click_label(harness: &mut Harness<'_, TextureCraftApp>, label: &str) {
    let at = harness.get_by_label(label).rect().center();
    click_at(harness, at);
    harness.run_steps(SETTLE_FRAMES);
}
