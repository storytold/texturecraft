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
    assert!(rects.iter().any(|(rect, color)| *color == fill && rect.width() >= row.width() - 1.0 && rect.contains(row.center())), "the whole active row gets the highlight fill");
    assert!(
        rects.iter().any(|(rect, color)| *color == tokens.accent && (rect.width() - 4.0).abs() < 0.01 && rect.left() <= row.left() + 0.5 && rect.contains(egui::pos2(rect.center().x, row.center().y))),
        "a 4 point accent edge marks the row"
    );
    // Neighbouring rows keep the alternating shades
    assert!(rects.iter().all(|(rect, color)| *color != fill || rect.contains(row.center())), "only the active row carries the highlight");

    // The lettering keeps its contrast on the stronger fill in every theme.
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
            assert!(contrast(tokens.text_dim, fill) >= 3.0, "{brightness:?} {accent:?}: subtitle on the active row");
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
