//! The Layers panel rows: content and mask thumbnails select their own target, effect groups
//! fold without touching the document, and the thumbnails arrive from the worker.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use texturecraft_engine::compose::ThumbTarget;
use texturecraft_engine::core::{Channel, LayerId};
use texturecraft_engine::{StackTarget, execute};
use texturecraft_ui_egui::TextureCraftApp;

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
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1200.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
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

/// The `label` header that follows the "Masked fill" row (other layers have groups too).
fn header(h: &mut Harness<'_, TextureCraftApp>, label: &str) {
    let top = h.get_by_label("Masked fill").rect().top();
    let at = h.get_all_by_label(label).map(|n| n.rect()).filter(|r| r.top() > top).min_by(|a, b| a.top().total_cmp(&b.top())).unwrap().center();
    click(h, at);
}

fn click(h: &mut Harness<'_, TextureCraftApp>, at: egui::Pos2) {
    h.drag_at(at);
    h.drop_at(at);
    h.run_steps(3);
}

#[test]
fn mask_and_content_thumbnails_select_their_own_target() {
    let (mut h, layer) = harness();
    let at = thumb(&h, "Masked fill", true);
    click(&mut h, at);
    assert_eq!(h.state().session.selected_layer(), Some(layer));
    assert_eq!(h.state().session.target, StackTarget::Mask, "the mask thumbnail routes to the mask");
    let at = thumb(&h, "Masked fill", false);
    click(&mut h, at);
    assert_eq!(h.state().session.target, StackTarget::Content, "the content thumbnail routes back");
    // Clicking an effect row's name still selects that effect (its right end holds the blend mode and opacity)
    let row = h.get_by_label("Effect A").rect();
    click(&mut h, row.center());
    assert!(matches!(h.state().session.target, StackTarget::Effect(_)));
}

#[test]
fn folding_an_effect_group_hides_its_rows_without_changing_the_document() {
    let (mut h, _) = harness();
    let (project, undo) = (h.state().session.doc.project.clone(), h.state().session.revision);
    assert!(h.query_by_label("Effect A").is_some() && h.query_by_label("Effect M").is_some());
    header(&mut h, "Content effects");
    assert!(h.query_by_label("Effect A").is_none(), "folded content effects are hidden");
    assert!(h.query_by_label("Effect M").is_some(), "the mask group stays open");
    header(&mut h, "Mask effects");
    assert!(h.query_by_label("Effect M").is_none());
    assert_eq!(h.state().session.doc.project, project);
    assert_eq!(h.state().session.revision, undo, "folding is not an edit");
    header(&mut h, "Content effects");
    assert!(h.query_by_label("Effect A").is_some(), "unfolds again");
}

#[test]
fn visible_rows_get_their_thumbnails() {
    let (mut h, layer) = harness();
    let content = ThumbTarget::Content { layer, channel: Channel::BaseColor };
    let mask = ThumbTarget::Mask { layer };
    let t0 = std::time::Instant::now();
    while !(h.state().layer_panel.service().is_fresh(content) && h.state().layer_panel.service().is_fresh(mask)) {
        assert!(t0.elapsed().as_secs() < 60, "thumbnails never arrived");
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // Rows, mask, and every effect stage on screen were asked for: well under the cache size.
    assert!(h.state().layer_panel.service().cached() >= 4);
}

#[test]
fn deeply_nested_effect_visibility_is_reachable_in_a_narrow_panel() {
    use craft_ui::docking::Node;
    use craft_ui::layout::SplitSize;
    use texturecraft_ui_egui::docking::Dock;

    let (mut h, layer) = harness();
    for _ in 0..6 {
        execute(&mut h.state_mut().session, "layer.group", &Value::Null).unwrap();
    }
    let mut workspace = h.state().ui.dock.workspace().clone();
    let Some(Node::Split { second, .. }) = workspace.layout.root.as_mut() else { panic!("default columns") };
    let Node::Split { size, .. } = second.as_mut() else { panic!("viewport and right column") };
    *size = SplitSize::Ratio(0.99);
    h.state_mut().ui.dock = Dock::from_value(serde_json::to_value(workspace).unwrap());
    h.run_steps(4);

    let row = h.get_by_label("Effect A").rect();
    assert!(row.width() < 180.0, "the test must use a compact panel: {row:?}");
    let indent = (row.width() - 7.0 - 22.0 - 21.6 - 8.0 - 20.0 - 6.0).max(0.0);
    click(&mut h, egui::pos2(row.left() + 15.0 + indent, row.center().y));

    let content = &h.state().session.doc.project.layers[&layer].content_effects;
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
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_pixels_per_point(2.0).build_eframe(|cc| TextureCraftApp::new(cc, session));
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
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    h
}

#[test]
fn layer_toolbar_stays_on_one_row_in_a_1280_by_800_window() {
    let h = narrow_harness();
    let buttons = ["Add paint layer", "Add fill layer", "Add folder"].map(|label| h.get_by_label(label).rect());

    // The effect menu button is the wand left of the paint button (effect rows draw wands too)
    let wand = h
        .query_all_by_label(egui_phosphor::regular::MAGIC_WAND)
        .map(|n| n.rect())
        .filter(|r| r.right() < buttons[0].left())
        .max_by(|a, b| a.right().total_cmp(&b.right()))
        .expect("the effect menu button");
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
    execute(&mut session, "layer.select", &json!({"layer": layer})).unwrap();
    execute(&mut session, "effect.add.filter", &json!({"target": "content", "filter": "invert", "name": LONG_EFFECT_NAME})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);

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
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
    h.hover_at(at);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::ALT });
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
    let (mut h, _) = harness();
    let at = thumb(&h, "Masked fill", false);
    alt_click(&mut h, at);
    assert_eq!(h.state().session.view, texturecraft_engine::ViewMode::Material);
}
