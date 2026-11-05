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
    // Clicking an effect row still selects that effect.
    let r = h.get_by_label("Effect A").rect();
    click(&mut h, egui::pos2(r.right() - 40.0, r.center().y));
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
