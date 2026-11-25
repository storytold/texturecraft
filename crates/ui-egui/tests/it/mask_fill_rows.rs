//! A mask fill effect's Properties: Fill | Symmetry | Grayscale tabs, the fill's projection rows, and one Grayscale slot whose picker sets its source

use egui::{Event, Modifiers, PointerButton, Pos2, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::core::{Channel, EffectKind, LayerId, ProceduralKind, Projection, ResolutionMode, Source};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::move_on_model::MoveOnModel;

use crate::{BAKE_RESOLUTION, DRAG_STEP_FRAMES, SETTLE_FRAMES, STARTUP_FRAMES, TEXTURE_RESOLUTION};

/// Tall enough that Properties shows every section of a mask fill without scrolling
const WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 1600.0);

/// Moves of a mouse drag, one per frame
const DRAG_MOVES: usize = 40;

/// How far left a drag moves the projection on the model
const MOVE_DRAG: egui::Vec2 = egui::vec2(-60.0, 0.0);

/// The app with a paint layer whose mask holds one fill effect, that effect selected; the layer's and the effect's ids
fn app_with_mask_fill(projection: &str) -> (Harness<'static, TextureCraftApp>, LayerId, LayerId) {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let layer = LayerId(texturecraft_engine::execute(&mut session, "layer.add.paint", &json!({})).unwrap()["layer"].as_u64().unwrap());
    let added = texturecraft_engine::execute(&mut session, "effect.add.fill", &json!({"layer": layer.0, "target": "mask", "projection": projection})).unwrap();
    let effect = LayerId(added["effect"].as_u64().unwrap());
    texturecraft_engine::execute(&mut session, "layer.select", &json!({"layer": layer.0, "target": effect.0})).unwrap();
    let mut harness = Harness::builder().with_size(WINDOW_SIZE).build_eframe(|cc| TextureCraftApp::new(cc, session));

    // The labels below are English; a first run would speak the system's language
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    (harness, layer, effect)
}

fn mask_fill(harness: &Harness<'_, TextureCraftApp>, effect: LayerId) -> (Source, Projection) {
    match &harness.state().session.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::Fill { source, projection, .. } => (source.clone(), projection.clone()),
        other => panic!("not a mask fill: {other:?}"),
    }
}

fn click(harness: &mut Harness<'_, TextureCraftApp>, role: Role, label: &str) {
    harness.get_by_role_and_label(role, label).click();
    harness.run_steps(SETTLE_FRAMES);
}

/// Scrolls the `role` named `label` into view first, then clicks it
fn click_below(harness: &mut Harness<'_, TextureCraftApp>, role: Role, label: &str) {
    harness.get_by_role_and_label(role, label).scroll_to_me();
    harness.run_steps(SETTLE_FRAMES);
    click(harness, role, label);
}

#[test]
fn a_mask_fill_has_fill_symmetry_and_grayscale_tabs_and_its_slot_picks_the_source() {
    let (mut harness, _, effect) = app_with_mask_fill("uv");
    for tab in ["Fill", "Symmetry", "Grayscale"] {
        assert!(harness.query_by_role_and_label(Role::Tab, tab).is_some(), "the {tab} tab");
    }

    assert!(harness.query_by_role_and_label(Role::Tab, "Material").is_none(), "a mask fill has no material");
    assert!(harness.query_by_label("Move on model").is_some(), "the Fill section's projection rows");

    click(&mut harness, Role::ComboBox, "Grayscale");

    assert!(harness.query_by_role_and_label(Role::Tab, "Anchor points").is_some(), "the slot opens the source picker");

    click(&mut harness, Role::ListBoxOption, "Clouds");
    let (Source::Procedural { pattern, low, high, .. }, _) = mask_fill(&harness, effect) else { panic!("a procedural") };
    assert_eq!((pattern.kind, low, high), (ProceduralKind::Clouds, [0.0; 3], [1.0; 3]), "a gray pattern spans black to white");
    assert!(!egui::Popup::is_any_open(&harness.ctx), "a pick closes the picker");
    assert!(harness.query_by_label("Scale").is_some(), "the pattern's own rows follow the slot");

    for shown in ["Contrast", "Balance", "Random"] {
        assert!(harness.query_by_label(shown).is_some(), "a grayscale's {shown} (Seed's dice) shows with Scale, Attributes folded");
    }

    // Resolution: the pattern is made at the texture set's size until another is picked
    let resolution = |harness: &Harness<'_, TextureCraftApp>| match mask_fill(harness, effect).0 {
        Source::Procedural { pattern, .. } => (pattern.resolution.mode, pattern.resolution.custom, pattern.resolution.factor),
        other => panic!("a procedural: {other:?}"),
    };
    click_below(&mut harness, Role::ComboBox, "Texture set  ·  256");
    for caption in ["Relative", "Fixed"] {
        assert!(harness.query_by_role_and_label(Role::Label, caption).is_some(), "one list holds the {caption} sizes under their caption");
    }
    click(&mut harness, Role::ListBoxOption, "1024 × 1024");

    assert_eq!(resolution(&harness), (ResolutionMode::Custom, 1024, 0));

    click_below(&mut harness, Role::ComboBox, "1024 × 1024");
    click(&mut harness, Role::ListBoxOption, "×4  ·  1024");

    assert_eq!(resolution(&harness), (ResolutionMode::Auto, 1024, 2), "1024 is the texture set's ×4");

    // The slot's × puts the mask fill back to white
    click(&mut harness, Role::Button, "Clear");

    assert_eq!(mask_fill(&harness, effect).0, Source::Uniform([1.0; 3]));
}

/// A drag as a mouse sends it: press, hold still a frame, then small moves one per frame until released; `watch` looks at the app after each move
fn mouse_drag_watching(harness: &mut Harness<'_, TextureCraftApp>, from: Pos2, to: Pos2, mut watch: impl FnMut(&Harness<'_, TextureCraftApp>)) {
    let button = |pos: Pos2, pressed: bool| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    harness.event(Event::PointerMoved(from));
    harness.run_steps(1);
    harness.event(button(from, true));
    harness.run_steps(DRAG_STEP_FRAMES);
    for step in 1..=DRAG_MOVES {
        harness.event(Event::PointerMoved(from.lerp(to, step as f32 / DRAG_MOVES as f32)));
        harness.run_steps(1);
        watch(harness);
    }
    harness.event(button(to, false));
    harness.run_steps(DRAG_STEP_FRAMES);
}

#[test]
fn a_mask_fills_projection_moves_on_the_model_and_resets_with_its_section() {
    let (mut harness, layer, effect) = app_with_mask_fill("uv");

    // A real drag that starts on the held button slides the mask fill's image
    let button = harness.get_by_role_and_label(Role::Button, "Move on model").rect().center();
    let mut held = None;
    mouse_drag_watching(&mut harness, button, button + MOVE_DRAG, |harness| held = held.or(harness.state().moving_on_model));

    assert_eq!(held, Some(MoveOnModel { layer, effect: Some(effect) }));
    assert_eq!(harness.state().moving_on_model, None, "letting go ends it");
    let Projection::Uv { offset, .. } = mask_fill(&harness, effect).1 else { panic!("still UV") };
    assert!(offset[0] != 0.0 && offset[1] == 0.0, "a drag across slides the image across: {offset:?}");

    click(&mut harness, Role::Button, "Fill: Reset to defaults");

    assert_eq!(mask_fill(&harness, effect).1, Projection::default(), "the Fill section's reset reaches the mask fill");
}

/// A mask fill reading a content anchor point picks the channel it reads in its Grayscale card
#[test]
fn a_mask_fill_reading_a_content_anchor_picks_its_channel() {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let low = texturecraft_engine::execute(&mut session, "layer.add.fill", &json!({"roughness": 0.2})).unwrap()["layer"].clone();
    let anchor = texturecraft_engine::execute(&mut session, "effect.add.anchor", &json!({"layer": low, "target": "content"})).unwrap()["effect"].clone();
    let top = texturecraft_engine::execute(&mut session, "layer.add.paint", &json!({})).unwrap()["layer"].clone();
    let added = texturecraft_engine::execute(&mut session, "effect.add.fill", &json!({"layer": top, "target": "mask", "anchor": anchor})).unwrap();
    let effect = LayerId(added["effect"].as_u64().unwrap());
    texturecraft_engine::execute(&mut session, "layer.select", &json!({"layer": top, "target": effect.0})).unwrap();
    let mut harness = Harness::builder().with_size(WINDOW_SIZE).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    let Source::Anchor { channel, .. } = mask_fill(&harness, effect).0 else { panic!("an anchor source") };
    assert_eq!(channel, Channel::Height, "a new anchor source reads height");

    click_below(&mut harness, Role::ComboBox, "Height");
    click(&mut harness, Role::ListBoxOption, "Roughness");

    let Source::Anchor { channel, .. } = mask_fill(&harness, effect).0 else { panic!("still an anchor source") };
    assert_eq!(channel, Channel::Roughness, "the pick reads the anchor's roughness");
}

/// An instance showing its source's mask fill lists only the anchor points below both: one between the source and the instance is refused by the source's mask
#[test]
fn an_inherited_mask_fill_lists_only_anchors_below_its_owner() {
    const BETWEEN: &str = "Edge wear between";
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    const BELOW: &str = "Edge wear below";
    let low = texturecraft_engine::execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    texturecraft_engine::execute(&mut session, "effect.add.anchor", &json!({"layer": low, "target": "content", "name": BELOW})).unwrap();
    let source = texturecraft_engine::execute(&mut session, "layer.add.paint", &json!({})).unwrap()["layer"].clone();
    let added = texturecraft_engine::execute(&mut session, "effect.add.fill", &json!({"layer": source, "target": "mask"})).unwrap();
    let effect = LayerId(added["effect"].as_u64().unwrap());
    let between = texturecraft_engine::execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    let anchor = texturecraft_engine::execute(&mut session, "effect.add.anchor", &json!({"layer": between, "target": "content"})).unwrap()["effect"].clone();
    texturecraft_engine::execute(&mut session, "effect.set", &json!({"effect": anchor, "name": BETWEEN})).unwrap();
    let instance = texturecraft_engine::execute(&mut session, "layer.instance", &json!({"layer": source})).unwrap()["layer"].clone();
    texturecraft_engine::execute(&mut session, "layer.select", &json!({"layer": instance})).unwrap();
    session.target = texturecraft_engine::StackTarget::Effect(effect);
    let mut harness = Harness::builder().with_size(WINDOW_SIZE).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);

    click(&mut harness, Role::ComboBox, "Grayscale");
    click(&mut harness, Role::Tab, "Anchor points");

    let offered: Vec<_> = harness.get_all_by_role(Role::ListBoxOption).filter_map(|row| row.accesskit_node().label()).collect();
    assert!(offered.iter().any(|label| label.contains(BELOW)), "the anchor below the source is offered");
    assert!(!offered.iter().any(|label| label.contains(BETWEEN)), "the anchor above the source is excluded");
}
