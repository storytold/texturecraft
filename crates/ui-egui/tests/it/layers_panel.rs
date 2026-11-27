//! The Layers panel rows: thumbnails select and disclose their own effects without editing the
//! document, and the thumbnail images arrive from the worker.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};
use texturecraft_engine::compose::ThumbTarget;
use texturecraft_engine::core::{Channel, LayerId};
use texturecraft_engine::{StackTarget, execute};
use texturecraft_ui_egui::TextureCraftApp;

use crate::SETTLE_FRAMES;

fn id(v: Value, key: &str) -> u64 {
    v[key].as_u64().unwrap()
}

/// A session whose top layer, "Masked fill", has one content effect and a mask with one effect.
fn harness() -> (Harness<'static, TextureCraftApp>, LayerId) {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = LayerId(id(execute(&mut session, "layer.add.fill", &json!({"color": "#336699"})).unwrap(), "layer"));
    execute(&mut session, "layer.rename", &json!({"layer": layer.0, "name": "Masked fill"})).unwrap();
    execute(&mut session, "effect.add.filter", &json!({"target": "content", "filter": "invert", "name": "Effect A"})).unwrap();
    execute(&mut session, "mask.add.black", &Value::Null).unwrap();
    execute(&mut session, "effect.add.generator", &json!({"generator": "position", "level": 0.5, "name": "Effect M"})).unwrap();
    execute(&mut session, "layer.select", &json!({"layer": layer.0})).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    (h, layer)
}

/// Centre of the content (`mask` false) or mask thumbnail of the top-level, non-folder row
/// labelled `name`: after the 7-point edge, the 20-point eye column, then 36-point squares 4 apart.
fn thumb(h: &Harness<'_, TextureCraftApp>, name: &str, mask: bool) -> egui::Pos2 {
    let row = h.get_by_label(name).rect();
    let x = row.left() + 7.0 + 20.0 + 18.0 + if mask { 40.0 } else { 0.0 };
    egui::pos2(x, row.center().y)
}

/// The accessible name of the mask thumbnail of the layer "Masked fill"
const MASK_THUMBNAIL_NAME: &str = "Select the mask (Alt+click: show it in the viewport, Shift+click: switch it on or off) (Masked fill)";

/// Toggle the effect rows through the selected layer's content thumbnail.
pub(super) fn click_selected_content_thumbnail(h: &mut Harness<'_, TextureCraftApp>) {
    let layer = h.state().session.selected_layer().unwrap();
    let name = &h.state().session.doc.project.layers[&layer].name;
    h.get_by_label(&format!("Select the layer content ({name})")).click();
    h.run_steps(3);
}

fn click(h: &mut Harness<'_, TextureCraftApp>, at: egui::Pos2) {
    h.drag_at(at);
    h.drop_at(at);
    h.run_steps(3);
}

/// An instance without a mask of its own shows its source's mask thumbnail with the effect badge, so
/// that thumbnail opens the source mask's effects and each of those rows can be selected from the instance.
#[test]
fn an_instance_mask_thumbnail_opens_the_inherited_mask_effects() {
    let (mut h, source) = harness();
    let instance = LayerId(id(execute(&mut h.state_mut().session, "layer.instance", &json!({"layer": source.0})).unwrap(), "layer"));
    execute(&mut h.state_mut().session, "layer.rename", &json!({"layer": instance.0, "name": "Instance row"})).unwrap();
    execute(&mut h.state_mut().session, "layer.select", &json!({"layer": instance.0})).unwrap();
    h.run_steps(4);
    assert!(h.state().session.doc.project.layers[&instance].mask.is_none(), "the instance draws with its source's mask");
    assert!(h.query_by_label("Effect M").is_none(), "stacks start closed");

    let mask = thumb(&h, "Instance row", true);
    click(&mut h, mask);
    assert_eq!(h.state().session.selected_layer(), Some(instance));
    assert_eq!(h.state().session.target, StackTarget::Mask);
    assert!(h.query_by_label("Effect M").is_some(), "the inherited mask effect is listed");
    click(&mut h, mask);
    assert!(h.query_by_label("Effect M").is_none(), "the same thumbnail closes the stack");
    click(&mut h, mask);

    // Select the effect through the instance
    let row = h.get_by_label("Effect M").rect();
    click(&mut h, row.center());

    let StackTarget::Effect(effect) = h.state().session.target else { panic!("{:?}", h.state().session.target) };
    assert_eq!(h.state().session.doc.project.effect(effect).map(|(owner, _)| owner), Some(source));
    assert_eq!(h.state().session.selected_layer(), Some(instance));
    assert!(h.query_by_label("Effect M").is_some(), "selecting an effect leaves its stack open");
}

/// An instance with a mask of its own lists that mask's effects, not its source's.
#[test]
fn an_instance_with_its_own_mask_opens_its_own_mask_effects() {
    let (mut h, source) = harness();
    let instance = LayerId(id(execute(&mut h.state_mut().session, "layer.instance", &json!({"layer": source.0})).unwrap(), "layer"));
    execute(&mut h.state_mut().session, "layer.rename", &json!({"layer": instance.0, "name": "Instance row"})).unwrap();
    execute(&mut h.state_mut().session, "layer.select", &json!({"layer": instance.0})).unwrap();
    execute(&mut h.state_mut().session, "mask.add.black", &Value::Null).unwrap();
    execute(&mut h.state_mut().session, "effect.add.generator", &json!({"generator": "position", "level": 0.5, "name": "Own effect"})).unwrap();
    execute(&mut h.state_mut().session, "layer.select", &json!({"layer": instance.0})).unwrap();
    h.run_steps(4);

    let mask = thumb(&h, "Instance row", true);
    click(&mut h, mask);

    assert!(h.query_by_label("Own effect").is_some(), "the instance's own mask effect is listed");
    assert!(h.query_by_label("Effect M").is_none(), "the source's mask effect is not");
}

#[test]
fn mask_and_content_thumbnails_toggle_their_own_stacks() {
    let (mut h, layer) = harness();
    let content = thumb(&h, "Masked fill", false);
    let mask = thumb(&h, "Masked fill", true);
    assert!(h.query_by_label("Effect A").is_none() && h.query_by_label("Effect M").is_none(), "stacks start closed");

    click(&mut h, mask);
    assert_eq!(h.state().session.selected_layer(), Some(layer));
    assert_eq!(h.state().session.target, StackTarget::Mask);
    assert!(h.query_by_label("Effect M").is_some());
    click(&mut h, mask);
    assert!(h.query_by_label("Effect M").is_none(), "the same mask thumbnail closes its stack");
    click(&mut h, mask);
    assert!(h.query_by_label("Effect M").is_some());

    click(&mut h, content);
    assert_eq!(h.state().session.target, StackTarget::Content);
    assert!(h.query_by_label("Effect M").is_none(), "switching thumbnails closes the mask stack");
    assert!(h.query_by_label("Effect A").is_some());
    click(&mut h, content);
    assert!(h.query_by_label("Effect A").is_none(), "the same content thumbnail closes its stack");
    click(&mut h, content);
    let row = h.get_by_label("Effect A").rect();
    click(&mut h, row.center());
    assert!(matches!(h.state().session.target, StackTarget::Effect(_)));
    assert!(h.query_by_label("Effect A").is_some(), "selecting an effect leaves its stack open");
}

#[test]
fn keyboard_toggles_effects_without_editing_the_document() {
    let (mut h, layer) = harness();
    let session = &h.state().session;
    let (project, history, modified) = (session.doc.project.clone(), session.history_labels(), session.modified);
    let content = "Select the layer content (Masked fill)";
    assert_eq!(h.get_by_label(content).accesskit_node().data().is_expanded(), Some(false));

    // Use the standard button keys so both paths preserve focus
    h.get_by_label(content).focus();
    h.run_steps(3);
    assert!(h.get_by_label(content).is_focused());
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.query_by_label("Effect A").is_some());
    h.key_press(egui::Key::Space);
    h.run_steps(3);
    assert!(h.query_by_label("Effect A").is_none());
    assert_eq!(h.get_by_label(content).accesskit_node().data().is_expanded(), Some(false));

    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.get_by_label(content).is_focused());
    assert_eq!(h.get_by_label(content).accesskit_node().data().is_expanded(), Some(true));

    let session = &h.state().session;
    let empty = session.doc.project.texture_sets[session.ts()]
        .stack
        .iter()
        .rev()
        .find_map(|id| {
            let layer = session.doc.project.layers.get(id)?;
            (layer.mask.is_none() && layer.content_effects.is_empty()).then(|| (*id, layer.name.clone()))
        })
        .expect("sample contains an empty paint layer");
    let empty_thumbnail = format!("Select the layer content ({})", empty.1);
    h.get_by_label(&empty_thumbnail).focus();
    h.run_steps(3);
    assert!(h.get_by_label(&empty_thumbnail).is_focused());
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().session.selected_layer(), Some(empty.0));
    assert_ne!(empty.0, layer);
    assert!(h.query_by_label("Effect A").is_none() && h.query_by_label("Effect M").is_none(), "switching layers closes the old stack");
    assert_eq!(h.get_by_label(&empty_thumbnail).accesskit_node().data().is_expanded(), None);

    let session = &h.state().session;
    assert_eq!(session.doc.project, project);
    assert_eq!(session.history_labels(), history);
    assert_eq!(session.modified, modified);
}

#[test]
fn visible_rows_get_their_thumbnails() {
    let (mut h, layer) = harness();
    let content = ThumbTarget::Content { layer, channel: Channel::BaseColor };
    let mask = ThumbTarget::Mask { layer };
    let effect = h.state().session.doc.project.layers[&layer].content_effects[0].id;
    let stage = ThumbTarget::Stage { effect, channel: Channel::BaseColor };
    click_selected_content_thumbnail(&mut h);
    let t0 = std::time::Instant::now();
    while ![content, mask, stage].iter().all(|target| h.state().layer_panel.service().is_fresh(*target)) {
        assert!(t0.elapsed().as_secs() < 60, "thumbnails never arrived");
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn deeply_nested_effect_visibility_is_reachable_in_a_narrow_panel() {
    use craft_behavior::dock::Node;
    use craft_behavior::dock::SplitSize;
    use texturecraft_ui_egui::docking::Dock;

    let (mut harness, layer) = harness();
    for _ in 0..6 {
        execute(&mut harness.state_mut().session, "layer.group", &Value::Null).unwrap();
    }

    let workspace = harness.state().ui.dock.workspace().clone();
    let mut layout = workspace.layout().clone();
    let Some(Node::Split { second, .. }) = layout.root.as_mut() else { panic!("default columns") };
    let Node::Split { size, second: column, .. } = second.as_mut() else { panic!("viewport and right column") };
    *size = SplitSize::Ratio(0.99);

    // Give Layers enough height to show the nested stack
    let Node::Split { second: below_sets, .. } = column.as_mut() else { panic!("texture sets and the rest") };
    let Node::Split { size: layers_share, .. } = below_sets.as_mut() else { panic!("layers and properties") };
    *layers_share = SplitSize::Ratio(0.9);
    let workspace = texturecraft_ui_egui::docking::Workspace::from_parts(layout, workspace.hidden().to_vec(), workspace.floated().to_vec());
    harness.state_mut().ui.dock = Dock::from_value(serde_json::to_value(workspace).unwrap());

    // Closing Properties lets the column shrink to the Layers minimum
    harness.state_mut().ui.dock.close(texturecraft_ui_egui::docking::Panel::Properties);
    harness.run_steps(4);

    harness.state_mut().run("layer.select", json!({"layer": layer.0})).unwrap();
    harness.run_steps(3);
    click_selected_content_thumbnail(&mut harness);
    let row = harness.get_by_label("Effect A").rect();
    assert!(row.width() < 180.0, "the test must use a compact panel: {row:?}");

    let indent = (row.width() - 7.0 - 22.0 - 21.6 - 8.0 - 20.0 - 6.0).max(0.0);
    click(&mut harness, egui::pos2(row.left() + 15.0 + indent, row.center().y));

    let content = &harness.state().session.doc.project.layers[&layer].content_effects;
    assert!(!content.iter().find(|effect| effect.name == "Effect A").unwrap().visible, "the compact eye toggles the effect instead of missing the panel");
}

/// Every painted shape of the last frame, folded groups flattened.
fn shapes(h: &Harness<'_, TextureCraftApp>) -> Vec<egui::Shape> {
    fn flatten(shape: &egui::Shape, out: &mut Vec<egui::Shape>) {
        match shape {
            egui::Shape::Vec(inner) => inner.iter().for_each(|shape| flatten(shape, out)),
            other => out.push(other.clone()),
        }
    }
    let mut out = Vec::new();
    h.output().shapes.iter().for_each(|clipped| flatten(&clipped.shape, &mut out));
    out
}

fn hidpi_harness() -> (Harness<'static, TextureCraftApp>, LayerId) {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = LayerId(id(execute(&mut session, "layer.add.fill", &json!({"color": "#336699"})).unwrap(), "layer"));
    execute(&mut session, "layer.rename", &json!({"layer": layer.0, "name": "Active one"})).unwrap();
    execute(&mut session, "layer.select", &json!({"layer": layer.0})).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .with_pixels_per_point(2.0)
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    (h, layer)
}

#[test]
fn the_active_layer_row_has_its_own_strong_fill_and_a_wide_edge_at_scale_two() {
    use texturecraft_ui_egui::dock::selected_layer_fill;
    use texturecraft_ui_egui::theme::{Accent, Brightness, Tokens};

    let (h, _) = hidpi_harness();
    assert_eq!(h.ctx.pixels_per_point(), 2.0);
    let tokens = Tokens::get(&h.ctx);
    let row = h.get_by_label("Active one").rect();
    let fill = selected_layer_fill(&tokens);
    assert_ne!(fill, tokens.row_selected, "the active layer row is more tinted than the plain selected shade");
    let painted = shapes(&h);
    let rects: Vec<_> = painted
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Rect(rect) => Some((rect.rect, rect.fill)),
            _ => None,
        })
        .collect();
    assert!(
        rects.iter().any(|(rect, color)| *color == fill && rect.width() >= row.width() - 1.0 && rect.contains(row.center())),
        "the whole active row gets the highlight fill"
    );
    assert!(
        rects.iter().any(|(rect, color)| *color == tokens.accent
            && (rect.width() - 4.0).abs() < 0.01
            && rect.left() <= row.left() + 0.5
            && rect.contains(egui::pos2(rect.center().x, row.center().y))),
        "a 4 point accent edge marks the row"
    );

    // Neighbouring rows keep the alternating shades
    assert!(rects.iter().all(|(rect, color)| *color != fill || rect.contains(row.center())), "only the active row carries the highlight");

    // The lettering keeps its contrast on the stronger fill in every theme
    let lum = |c: egui::Color32| {
        let f = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    };
    let contrast = |a: egui::Color32, b: egui::Color32| (lum(a).max(lum(b)) + 0.05) / (lum(a).min(lum(b)) + 0.05);
    for brightness in Brightness::ALL {
        for accent in Accent::ALL {
            let tokens = Tokens::for_brightness(brightness).with_accent(accent);
            let fill = selected_layer_fill(&tokens);
            assert!(contrast(tokens.text_strong, fill) >= 4.5, "{brightness:?} {accent:?}: name on the active row");
            assert!(
                contrast(tokens.text_dim, fill) >= (contrast(tokens.text_dim, tokens.row_selected) - 0.5).min(3.0),
                "{brightness:?} {accent:?}: subtitle on the active row is not much dimmer than on the plain selected shade"
            );
            assert!(contrast(fill, tokens.panel) >= 1.15, "{brightness:?} {accent:?}: the active row stands out from the panel");
        }
    }
}

#[test]
fn the_layer_eye_looks_clearly_different_when_visible_and_hidden_at_scale_two() {
    let (mut h, layer) = hidpi_harness();
    let tokens = texturecraft_ui_egui::theme::Tokens::get(&h.ctx);
    let row = h.get_by_label("Active one").rect();
    let eye_center = egui::pos2(row.left() + 7.0 + 8.0, row.center().y);

    // What is painted on the eye: (glyph, glyph colour, chip fill behind it)
    let eye = |h: &Harness<'_, TextureCraftApp>| {
        let painted = shapes(h);
        let glyph = painted
            .iter()
            .find_map(|shape| match shape {
                egui::Shape::Text(text) if text.visual_bounding_rect().contains(eye_center) && !text.galley.text().trim().is_empty() => {
                    Some((text.galley.text().to_string(), text.galley.job.sections[0].format.color))
                }
                _ => None,
            })
            .expect("the eye glyph is painted");
        let chip = painted.iter().find_map(|shape| match shape {
            egui::Shape::Rect(rect) if rect.rect.contains(eye_center) && rect.rect.width() < 20.0 => Some(rect.fill),
            _ => None,
        });
        (glyph.0, glyph.1, chip)
    };
    let (on_glyph, on_color, on_chip) = eye(&h);
    execute(&mut h.state_mut().session, "layer.visible", &json!({"layer": layer.0})).unwrap();
    h.run_steps(3);
    let (off_glyph, off_color, off_chip) = eye(&h);
    assert_ne!(on_glyph, off_glyph, "a hidden layer shows the crossed-out eye");
    assert_ne!(on_color, off_color, "the glyph colour changes with the state");
    assert_eq!(on_color, tokens.text_strong);
    assert_eq!(off_color, tokens.text_dim);
    assert_eq!(on_chip, Some(tokens.accent_dim), "a visible layer's eye sits on an accent chip");
    assert_eq!(off_chip, None, "a hidden layer's eye has no chip");
}

const LONG_NAME: &str = "Weathered painted steel plate with scratched edges";

/// The app in a 1280 x 800 window with one layer named `LONG_NAME`.
fn narrow_harness() -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = id(execute(&mut session, "layer.add.fill", &json!({"color": "#336699"})).unwrap(), "layer");
    execute(&mut session, "layer.rename", &json!({"layer": layer, "name": LONG_NAME})).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    h
}

#[test]
fn layer_toolbar_stays_on_one_row_in_a_1280_by_800_window() {
    let h = narrow_harness();
    let buttons = ["Add paint layer", "Add fill layer", "Add folder"].map(|label| h.get_by_label(label).rect());

    let wand = h.get_by_label("Add effect").rect();
    let remove = h.query_all_by_label_contains("Remove layer").next().expect("the remove button").rect();
    for rect in buttons.into_iter().chain([remove]) {
        assert!((rect.center().y - wand.center().y).abs() < 1.0, "the toolbar wrapped: {rect:?} against {wand:?}");
    }
    let row = h.get_by_label(LONG_NAME).rect();
    assert!(remove.right() <= row.right() + 1.0, "the last button stays inside the panel: {remove:?} against {row:?}");
}

#[test]
fn a_layer_name_cut_short_shows_in_full_on_hover() {
    let mut h = narrow_harness();
    let row = h.get_by_label(LONG_NAME).rect();
    let before = h.query_all_by_label_contains(LONG_NAME).count();
    h.hover_at(egui::pos2(row.left() + 125.0, row.center().y));
    h.run_steps(30);
    assert_eq!(h.query_all_by_label_contains(LONG_NAME).count(), before + 1, "the tooltip carries the full name");
}

const LONG_EFFECT_NAME: &str = "Edge wear from curvature with dirt accumulation";

#[test]
fn an_effect_name_cut_short_shows_in_full_on_hover() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = id(execute(&mut session, "layer.add.fill", &json!({"color": "#336699"})).unwrap(), "layer");
    execute(&mut session, "effect.add.filter", &json!({"target": "content", "filter": "invert", "name": LONG_EFFECT_NAME})).unwrap();
    execute(&mut session, "layer.select", &json!({"layer": layer})).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);

    click_selected_content_thumbnail(&mut h);
    let row = h.get_by_label(LONG_EFFECT_NAME).rect();
    let before = h.query_all_by_label_contains(LONG_EFFECT_NAME).count();
    h.hover_at(egui::pos2(row.left() + 75.0, row.center().y));
    h.run_steps(30);
    assert_eq!(h.query_all_by_label_contains(LONG_EFFECT_NAME).count(), before + 1, "the tooltip carries the full effect name");
}

fn right_click(h: &mut Harness<'_, TextureCraftApp>, at: egui::Pos2) {
    h.hover_at(at);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
    }
    h.run_steps(3);
}

#[test]
fn thumbnail_menus_apply_actions_to_the_clicked_layer_and_stack() {
    let (mut h, clicked) = harness();
    let selected = {
        let session = &mut h.state_mut().session;
        let layer = LayerId(id(execute(session, "layer.add.fill", &json!({"name": "Another fill"})).unwrap(), "layer"));
        execute(session, "mask.add.white", &Value::Null).unwrap();
        layer
    };
    h.run_steps(3);
    assert_eq!(h.state().session.selected_layer(), Some(selected));

    let at = thumb(&h, "Masked fill", true);
    right_click(&mut h, at);
    assert!(h.query_by_label("Flatten layer").is_none());
    h.get_by_label("Toggle mask").click();
    h.run_steps(3);
    let project = &h.state().session.doc.project;
    assert!(!project.layers[&clicked].mask.as_ref().unwrap().enabled);
    assert!(project.layers[&selected].mask.as_ref().unwrap().enabled);
    assert_eq!(h.state().session.target, StackTarget::Mask);

    // The same layer's mask is still selected when its material menu opens.
    let at = thumb(&h, "Masked fill", false);
    right_click(&mut h, at);
    assert!(h.query_by_label("Toggle mask").is_none());
    // The same items as the toolbar's Add effect on the content: a generator can't go there, so it is greyed
    assert!(h.get_by_label_contains("Add generator").accesskit_node().is_disabled());
    h.get_by_label_contains("Add levels").click();
    h.run_steps(3);
    let layer = &h.state().session.doc.project.layers[&clicked];
    assert_eq!(layer.content_effects.len(), 2);
    assert_eq!(layer.mask.as_ref().unwrap().effects.len(), 1);
}

#[test]
fn focused_mask_thumbnail_opens_and_dismisses_its_menu_from_the_keyboard() {
    let (mut harness, _) = harness();

    harness.get_by_label(MASK_THUMBNAIL_NAME).focus();
    harness.run_steps(3);
    assert!(harness.get_by_label(MASK_THUMBNAIL_NAME).is_focused());

    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::F10);
    harness.run_steps(3);
    assert!(harness.query_by_label("Toggle mask").is_some());

    harness.key_press(egui::Key::ArrowDown);
    harness.run_steps(3);
    assert!(harness.get_by_label("Toggle mask").is_focused());

    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);

    assert!(harness.query_by_label("Toggle mask").is_none());
    assert!(harness.get_by_label(MASK_THUMBNAIL_NAME).is_focused());
}

/// Click the eye of the top-level row `name` with `modifiers` held (the eye sits in the first 20 points after the 7-point edge).
fn click_eye(h: &mut Harness<'_, TextureCraftApp>, name: &str, modifiers: egui::Modifiers) {
    let row = h.get_by_label(name).rect();
    h.event(egui::Event::ModifiersChanged(modifiers));
    click(h, egui::pos2(row.left() + 17.0, row.center().y));
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(2);
}

#[test]
fn the_context_menu_solos_a_layer_without_touching_the_document() {
    let (mut h, layer) = harness();
    let (project, history, modified) = (h.state().session.doc.project.clone(), h.state().session.history_labels(), h.state().session.modified);
    let row = h.get_by_label("Masked fill").rect();
    right_click(&mut h, row.center());
    h.get_by_label("Solo layer").click();
    h.run_steps(3);
    assert_eq!(h.state().session.solo, Some(texturecraft_engine::Solo { layer, ignore_mask: false }));
    assert_eq!(h.state().session.doc.project, project, "soloing is not an edit");
    assert_eq!((h.state().session.history_labels(), h.state().session.modified), (history.clone(), modified));

    // The entry is lit while the layer is soloed; clicking it again ends the solo
    right_click(&mut h, row.center());
    h.get_by_label("Solo layer").click();
    h.run_steps(3);
    assert_eq!(h.state().session.solo, None);

    right_click(&mut h, row.center());
    h.get_by_label("Solo layer without its mask").click();
    h.run_steps(3);
    assert_eq!(h.state().session.solo, Some(texturecraft_engine::Solo { layer, ignore_mask: true }));
    right_click(&mut h, row.center());
    h.get_by_label("Solo layer without its mask").click();
    h.run_steps(3);
    assert_eq!(h.state().session.solo, Some(texturecraft_engine::Solo { layer, ignore_mask: false }), "the second entry only switches the mask back on");
    assert_eq!((h.state().session.doc.project.clone(), h.state().session.history_labels()), (project, history));
}

#[test]
fn alt_click_on_the_eye_solos_and_a_plain_click_still_hides() {
    let (mut h, layer) = harness();
    click_eye(&mut h, "Masked fill", egui::Modifiers::ALT);
    assert_eq!(h.state().session.solo, Some(texturecraft_engine::Solo { layer, ignore_mask: false }));
    assert!(h.state().session.doc.project.layers[&layer].visible, "Alt+click does not touch the layer's own eye");
    click_eye(&mut h, "Masked fill", egui::Modifiers::ALT);
    assert_eq!(h.state().session.solo, None, "Alt+click on the soloed layer ends the solo");
    click_eye(&mut h, "Masked fill", egui::Modifiers::ALT | egui::Modifiers::SHIFT);
    assert_eq!(h.state().session.solo, Some(texturecraft_engine::Solo { layer, ignore_mask: true }), "Alt+Shift also ignores the mask");
    click_eye(&mut h, "Masked fill", egui::Modifiers::ALT);
    assert_eq!(h.state().session.solo, None);
    click_eye(&mut h, "Masked fill", egui::Modifiers::NONE);
    assert!(!h.state().session.doc.project.layers[&layer].visible, "a plain click hides the layer as before");
    assert_eq!(h.state().session.solo, None);
}

fn alt_click(h: &mut Harness<'_, TextureCraftApp>, at: egui::Pos2) {
    click_with(h, at, egui::Modifiers::ALT);
}

/// A primary click at `position` with `modifiers` held
fn click_with(h: &mut Harness<'_, TextureCraftApp>, position: egui::Pos2, modifiers: egui::Modifiers) {
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.hover_at(position);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed, modifiers });
        h.run_steps(2);
    }

    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(3);
}

#[test]
fn alt_click_on_a_mask_thumbnail_shows_that_mask_alone_without_changing_the_document() {
    let (mut h, layer) = harness();
    let history = h.state().session.history_labels().len();
    let project = h.state().session.doc.project.clone();

    let at = thumb(&h, "Masked fill", true);
    click(&mut h, at);
    assert_eq!(h.state().session.view, texturecraft_engine::ViewMode::Material, "a plain click only selects");

    alt_click(&mut h, at);
    assert_eq!(h.state().session.view, texturecraft_engine::ViewMode::Mask(layer));
    let gray = h.state().session.mask_view_rgba(layer, 64).unwrap();
    assert!(gray.chunks(4).all(|px| px[0] == px[1] && px[1] == px[2] && px[3] == 255), "the mask is shown in gray");
    assert!(gray.chunks(4).any(|px| px[0] != gray[0]), "the position generator shapes the mask");
    assert_eq!(h.state().session.history_labels().len(), history, "view only: no history step");
    assert_eq!(h.state().session.doc.project, project, "view only: the project is unchanged");

    execute(&mut h.state_mut().session, "viewport.displayMaterial", &Value::Null).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().session.view, texturecraft_engine::ViewMode::Material, "Display material (M) returns");
}

#[test]
fn alt_click_on_the_content_thumbnail_leaves_the_view_alone() {
    let (mut h, layer) = harness();
    let content_thumbnail = thumb(&h, "Masked fill", false);
    let mask_thumbnail = thumb(&h, "Masked fill", true);

    alt_click(&mut h, content_thumbnail);

    assert_eq!(h.state().session.view, texturecraft_engine::ViewMode::Material);

    alt_click(&mut h, mask_thumbnail);

    assert_eq!(h.state().session.view, texturecraft_engine::ViewMode::Mask(layer));
    assert_eq!(h.state().session.target, StackTarget::Mask);

    alt_click(&mut h, content_thumbnail);

    assert_eq!(h.state().session.view, texturecraft_engine::ViewMode::Mask(layer));
    assert_eq!(h.state().session.target, StackTarget::Content);
}

/// Whether a bypassed mask's strike is painted across the thumbnail at `position`
fn mask_strike_shown(h: &Harness<'_, TextureCraftApp>, position: egui::Pos2) -> bool {
    let danger = texturecraft_ui_egui::theme::Tokens::get(&h.ctx).danger;
    shapes(h).iter().any(|shape| matches!(shape, egui::Shape::LineSegment { points, stroke } if stroke.color == danger && egui::Rect::from_two_pos(points[0], points[1]).contains(position)))
}

#[test]
fn shift_click_on_a_mask_thumbnail_switches_the_mask_off_and_on_in_one_undo_step_each() {
    let (mut h, layer) = harness();
    let at = thumb(&h, "Masked fill", true);
    let target = h.state().session.target;
    let history = h.state().session.history_labels().len();
    let mask = h.state().session.doc.project.layers[&layer].mask.clone().unwrap();

    assert!(!mask_strike_shown(&h, at));
    assert_eq!(h.get_by_label(MASK_THUMBNAIL_NAME).accesskit_node().description().as_deref(), Some("Mask on"));

    click_with(&mut h, at, egui::Modifiers::SHIFT);

    let bypassed = h.state().session.doc.project.layers[&layer].mask.as_ref().unwrap();
    assert!(!bypassed.enabled);
    assert_eq!(bypassed.effects, mask.effects, "the mask keeps its effects while bypassed");
    assert_eq!(h.state().session.history_labels().len(), history + 1);
    assert_eq!(h.state().session.target, target, "Shift+click toggles instead of selecting the mask");
    assert!(mask_strike_shown(&h, at), "the disabled indicator shows while the mask is bypassed");

    let thumbnail = h.get_by_label(MASK_THUMBNAIL_NAME);
    assert!(thumbnail.is_focused());
    assert_eq!(thumbnail.accesskit_node().description().as_deref(), Some("Mask off"));

    click_with(&mut h, at, egui::Modifiers::SHIFT);

    assert_eq!(h.state().session.doc.project.layers[&layer].mask, Some(mask), "toggling back restores the mask as it was");
    assert_eq!(h.state().session.history_labels().len(), history + 2);
    assert!(!mask_strike_shown(&h, at));
    assert_eq!(h.get_by_label(MASK_THUMBNAIL_NAME).accesskit_node().description().as_deref(), Some("Mask on"));

    h.state_mut().session.undo().unwrap();

    assert!(!h.state().session.doc.project.layers[&layer].mask.as_ref().unwrap().enabled, "one undo undoes one toggle");
}

#[test]
fn shift_click_on_a_mask_thumbnail_works_with_no_layer_selected() {
    let (mut harness, layer) = harness();
    let row = harness.get_by_label("Masked fill").rect();
    let mask_thumbnail = thumb(&harness, "Masked fill", true);
    let history = harness.state().session.history_labels().len();

    click_with(&mut harness, row.center(), egui::Modifiers::COMMAND);

    assert!(harness.state().session.selected_layers().is_empty());
    assert!(harness.state().session.doc.project.layers[&layer].mask.as_ref().unwrap().enabled);

    click_with(&mut harness, mask_thumbnail, egui::Modifiers::SHIFT);

    assert!(!harness.state().session.doc.project.layers[&layer].mask.as_ref().unwrap().enabled);
    assert!(harness.state().session.selected_layers().is_empty());
    assert_eq!(harness.state().session.history_labels().len(), history + 1);
}

#[test]
fn the_mask_toggle_shortcut_requires_a_selected_layer() {
    let (mut harness, layer) = harness();
    let row = harness.get_by_label("Masked fill").rect();
    harness.state_mut().ui.keymap.insert("mask.toggle".into(), "F7".into());

    harness.key_press(egui::Key::F7);
    harness.run_steps(3);

    assert!(!harness.state().session.doc.project.layers[&layer].mask.as_ref().unwrap().enabled);

    click_with(&mut harness, row.center(), egui::Modifiers::COMMAND);
    assert!(harness.state().session.selected_layers().is_empty());
    let toast_count = harness.state().toasts.len();

    harness.key_press(egui::Key::F7);
    harness.run_steps(3);

    assert_eq!(harness.state().toasts.len(), toast_count, "the shortcut is inactive when it has no target");
    assert!(!harness.state().session.doc.project.layers[&layer].mask.as_ref().unwrap().enabled);
}

#[test]
fn clicking_the_material_thumbnail_leaves_the_mask_view() {
    let (mut h, layer) = harness();
    let mask_thumbnail = thumb(&h, "Masked fill", true);
    let material_thumbnail = thumb(&h, "Masked fill", false);

    alt_click(&mut h, mask_thumbnail);

    assert_eq!(h.state().session.view, texturecraft_engine::ViewMode::Mask(layer));

    click(&mut h, material_thumbnail);

    assert_eq!(h.state().session.view, texturecraft_engine::ViewMode::Material);
    assert_eq!(h.state().session.target, StackTarget::Content, "the layer content is selected");
}

/// Pointer moves of the opacity cell drag, one per frame
const OPACITY_DRAG_MOVES: usize = 4;

/// How far left each move of the opacity cell drag goes, in points
const OPACITY_DRAG_STEP: f32 = 10.0;

/// A drag on a row's opacity cell is one undo step, however many frames it spans
#[test]
fn dragging_a_row_opacity_adds_one_undo_step() {
    let (mut h, layer) = harness();
    let row = h.get_by_label("Masked fill").rect();
    let cell = h.get_all_by_role(egui::accesskit::Role::SpinButton).map(|node| node.rect()).find(|cell| row.contains(cell.center())).unwrap();
    let opacity = |h: &Harness<'_, TextureCraftApp>| h.state().session.doc.project.layers[&layer].blend(Channel::BaseColor).unwrap().opacity;
    let (before, steps) = (opacity(&h), h.state().session.history_labels().len());

    // A leftward drag lowers it from 100 %, a frame per move as a real drag arrives
    let from = cell.center();
    h.hover_at(from);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    let step_left = |step: usize| from - egui::vec2(OPACITY_DRAG_STEP * step as f32, 0.0);
    for step in 1..=OPACITY_DRAG_MOVES {
        h.hover_at(step_left(step));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton {
        pos: step_left(OPACITY_DRAG_MOVES),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(SETTLE_FRAMES);

    assert!(opacity(&h) < before, "the drag lowers the opacity: {}", opacity(&h));
    assert_eq!(h.state().session.history_labels().len(), steps + 1, "every frame of the drag merges into one step");
}
