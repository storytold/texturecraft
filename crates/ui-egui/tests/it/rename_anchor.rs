//! Renaming an effect row and mask anchor points, driven through the interface: the row's Rename action, Undo and Redo,
//! Save and Open, and the Add effect menu and Source picker that link a fill on a higher layer to an anchor in a mask.

use std::path::{Path, PathBuf};

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, EffectKind, LayerId, Source};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, StackTarget};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::file_dialogs::Answer;

const BLACK: [f32; 3] = [0.0; 3];
const WHITE: [f32; 3] = [1.0; 3];

fn id(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap()
}

fn harness_for(session: Session) -> Harness<'static, TextureCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));

    // English, so a first-run locale does not change the queried labels
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    h
}

/// A path in a new temporary folder, named after the test.
fn temp_file(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texturecraft-rename-anchor-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("project.tcraft")
}

fn save_to(h: &mut Harness<'_, TextureCraftApp>, file: &Path) {
    let path = file.to_path_buf();
    h.state_mut().file_dialogs.set_override(Some(Box::new(move |_| Answer::Paths(vec![path.clone()]))));
    let ctx = h.ctx.clone();
    texturecraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "file.saveAs");
    h.run_steps(3);
    assert!(file.exists(), "Save as wrote the project");
    assert!(!h.state().session.modified, "the project counts as saved");
}

fn reopen(h: &mut Harness<'_, TextureCraftApp>, file: &Path) {
    let path = file.to_path_buf();
    h.state_mut().file_dialogs.set_override(Some(Box::new(move |_| Answer::Paths(vec![path.clone()]))));
    let ctx = h.ctx.clone();
    texturecraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "file.open");
    h.run_steps(3);
    assert_eq!(h.state().session.path.as_deref(), Some(file.display().to_string().as_str()), "the saved project is open");
}

fn effect_name(h: &Harness<'_, TextureCraftApp>, effect: u64) -> String {
    h.state().session.doc.project.effect(LayerId(effect)).unwrap().1.name.clone()
}

/// A fill layer with one effect named "Effect A", selected, in the running interface.
fn effect_harness() -> (Harness<'static, TextureCraftApp>, u64) {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = id(&texturecraft_engine::execute(&mut session, "layer.add.fill", &Value::Null).unwrap(), "layer");
    let effect = id(
        &texturecraft_engine::execute(&mut session, "effect.add.filter", &json!({"target": "content", "filter": "invert", "name": "Effect A"})).unwrap(),
        "effect",
    );
    texturecraft_engine::execute(&mut session, "layer.select", &json!({"layer": layer})).unwrap();
    (harness_for(session), effect)
}

/// Choose Rename effect from the row's context menu and type `text` over the old name.
fn rename_from_the_menu(h: &mut Harness<'_, TextureCraftApp>, old: &str, text: &str) {
    h.get_by_label(old).click_secondary();
    h.run_steps(2);
    h.get_by_label("Rename effect").click();
    h.run_steps(3);
    assert!(h.state().ui.rename.is_some(), "the editor is open");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(3);
}

#[test]
fn renaming_an_effect_row_undoes_redoes_and_survives_save_and_reopen() {
    let (mut h, effect) = effect_harness();
    rename_from_the_menu(&mut h, "Effect A", "Pitted rust");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(h.state().ui.rename.is_none(), "Enter closes the editor");
    assert_eq!(effect_name(&h, effect), "Pitted rust");
    assert!(h.query_by_label("Pitted rust").is_some(), "the row shows the new name");
    assert!(h.query_by_label("Effect A").is_none(), "the old name is gone from the row");

    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(3);
    assert_eq!(effect_name(&h, effect), "Effect A", "Ctrl+Z restores the old name");
    assert!(h.query_by_label("Effect A").is_some(), "the row shows the old name again");

    h.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    h.run_steps(3);
    assert_eq!(effect_name(&h, effect), "Pitted rust", "Ctrl+Y redoes the rename");
    assert!(h.query_by_label("Pitted rust").is_some());

    let file = temp_file("rename");
    save_to(&mut h, &file);

    // Change the open project, so the reopen has to bring the saved name back
    h.state_mut().run("effect.set", json!({"effect": effect, "name": "Unsaved name"}));
    h.state_mut().session.modified = false;
    reopen(&mut h, &file);
    assert_eq!(effect_name(&h, effect), "Pitted rust", "the reopened project keeps the name");
    let layer = h.state().session.doc.project.effect(LayerId(effect)).unwrap().0;
    h.state_mut().run("layer.select", json!({"layer": layer.0}));
    h.run_steps(3);
    assert!(h.query_by_label("Pitted rust").is_some(), "the reopened stack shows the name");
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

#[test]
fn escape_in_the_effect_editor_keeps_the_name() {
    let (mut h, effect) = effect_harness();
    let history = h.state().session.history_labels().len();
    rename_from_the_menu(&mut h, "Effect A", "Never applied");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().ui.rename.is_none(), "Escape closes the editor");
    assert_eq!(effect_name(&h, effect), "Effect A");
    assert_eq!(h.state().session.history_labels().len(), history, "nothing was added to the history");
}

/// Composited base color at the middle texel of the sphere's texture.
fn base_color(s: &mut Session) -> [f32; 3] {
    s.update();
    s.caches[0].composite.value(Channel::BaseColor, 128, 128)
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 2e-3)
}

/// A black layer "Low" whose mask is a 0.4 fill: it shows at 0.5 * 0.6 over the default base color.
fn low_layer(s: &mut Session) -> u64 {
    let low = id(&texturecraft_engine::execute(s, "layer.add.fill", &json!({"color": BLACK})).unwrap(), "layer");
    texturecraft_engine::execute(s, "layer.rename", &json!({"layer": low, "name": "Low"})).unwrap();
    texturecraft_engine::execute(s, "mask.add.black", &Value::Null).unwrap();
    texturecraft_engine::execute(s, "effect.add.fill", &json!({"value": 0.4})).unwrap();
    low
}

/// A white layer with a black mask, selected on its mask.
fn high_layer(s: &mut Session) -> u64 {
    let high = id(&texturecraft_engine::execute(s, "layer.add.fill", &json!({"color": WHITE})).unwrap(), "layer");
    texturecraft_engine::execute(s, "mask.add.black", &Value::Null).unwrap();
    high
}

fn sphere_session() -> Session {
    Session::new_project("anchors", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 256).unwrap()
}

fn mask_anchor_of(h: &Harness<'_, TextureCraftApp>, layer: u64) -> Option<u64> {
    let layer = h.state().session.doc.project.layer(LayerId(layer)).unwrap();
    layer.mask.iter().flat_map(|m| m.effects.iter()).find(|e| matches!(e.kind, EffectKind::Anchor)).map(|e| e.id.0)
}

fn reader_source(h: &Harness<'_, TextureCraftApp>, reader: u64) -> Source {
    let layer = h.state().session.doc.project.layer(LayerId(reader)).unwrap();
    let effect = layer.mask.as_ref().unwrap().effects.iter().find(|e| matches!(e.kind, EffectKind::Fill { .. })).expect("the mask has a fill");
    let EffectKind::Fill { source, .. } = &effect.kind else { unreachable!() };
    source.clone()
}

#[test]
fn a_fill_on_a_higher_layer_reads_a_mask_anchor_added_in_the_interface() {
    let mut session = sphere_session();
    let low = low_layer(&mut session);
    let mut h = harness_for(session);

    // The wand adds to the mask, since the mask's fill is the selected target of the Low layer
    assert!(matches!(h.state().session.target, StackTarget::Effect(_)));
    h.get_by_label(egui_phosphor::regular::MAGIC_WAND).click();
    h.run_steps(3);
    assert!(h.query_by_label("Add to the mask").is_some(), "the menu offers the mask");
    h.get_by_label("Add anchor point").click();
    h.run_steps(3);
    let anchor = mask_anchor_of(&h, low).expect("the anchor point is in the mask of Low");
    assert!(h.state().session.doc.project.layer(LayerId(low)).unwrap().content_effects.is_empty(), "the anchor point is not on the layer content");

    // A higher layer with a mask fill, added through the same menu
    let high = high_layer(&mut h.state_mut().session);
    h.run_steps(3);
    h.get_by_label(egui_phosphor::regular::MAGIC_WAND).click();
    h.run_steps(3);
    h.get_by_label("Add fill").click();
    h.run_steps(3);
    assert!(matches!(reader_source(&h, high), Source::Uniform(_)), "a new fill starts as a uniform value");

    // The Anchor points tab lists the mask anchor of the layer below
    h.get_by_role_and_label(Role::ComboBox, "Grayscale").scroll_to_me();
    h.run_steps(30);
    h.get_by_role_and_label(Role::ComboBox, "Grayscale").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Tab, "Anchor points").click();
    h.run_steps(3);

    let label = format!("Low / mask / {}", effect_name(&h, anchor));
    h.get_by_role_and_label(Role::Button, &label).click();
    h.run_steps(3);

    let Source::Anchor { anchor: read, .. } = reader_source(&h, high) else { panic!("the fill reads an anchor: {:?}", reader_source(&h, high)) };
    assert_eq!(read.0, anchor, "the fill reads the anchor of Low");

    // The higher layer shows the mask value recorded at the anchor: 0.4
    let low_shown = 0.5 * 0.6;
    let expected = low_shown + (1.0 - low_shown) * 0.4;
    assert!(close(base_color(&mut h.state_mut().session), [expected; 3]), "{:?}", base_color(&mut h.state_mut().session));

    // The engine alone gives the same result for the same stack
    let mut reference = sphere_session();
    low_layer(&mut reference);
    let ref_anchor = id(&texturecraft_engine::execute(&mut reference, "effect.add.anchor", &json!({"target": "mask"})).unwrap(), "effect");
    high_layer(&mut reference);
    texturecraft_engine::execute(&mut reference, "effect.add.fill", &json!({"anchor": ref_anchor})).unwrap();
    assert!(close(base_color(&mut h.state_mut().session), base_color(&mut reference)));

    // Save and reopen keep the link and the result
    let file = temp_file("anchor");
    save_to(&mut h, &file);

    // Unlink the fill in the open project, so the reopen has to bring the saved link back
    let fill = h
        .state()
        .session
        .doc
        .project
        .layer(LayerId(high))
        .unwrap()
        .mask
        .as_ref()
        .unwrap()
        .effects
        .iter()
        .find(|e| matches!(e.kind, EffectKind::Fill { .. }))
        .unwrap()
        .id
        .0;
    h.state_mut().run("effect.set", json!({"effect": fill, "value": 1.0}));
    assert!(matches!(reader_source(&h, high), Source::Uniform(_)));
    h.state_mut().session.modified = false;
    reopen(&mut h, &file);
    assert_eq!(mask_anchor_of(&h, low), Some(anchor), "the anchor point is still in the mask");
    let Source::Anchor { anchor: read, .. } = reader_source(&h, high) else { panic!("the reopened fill reads an anchor") };
    assert_eq!(read.0, anchor, "the reopened fill still reads that anchor");
    assert!(close(base_color(&mut h.state_mut().session), [expected; 3]), "{:?}", base_color(&mut h.state_mut().session));
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}
