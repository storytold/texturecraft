//! Shift+click and Ctrl/Cmd+click on Layers panel rows select several layers, as in the
//! reference app, and the selected rows are highlighted.

use egui::accesskit::Toggled;
use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::core::LayerId;
use texturecraft_engine::execute;
use texturecraft_ui_egui::TextureCraftApp;

/// Paint layers "Row A" (bottom) to "Row D" (top) in the active texture set, "Row D" selected.
fn harness() -> (Harness<'static, TextureCraftApp>, [LayerId; 4]) {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let rows = ["Row A", "Row B", "Row C", "Row D"].map(|name| LayerId(execute(&mut session, "layer.add.paint", &json!({ "name": name })).unwrap()["layer"].as_u64().unwrap()));
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1200.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    (h, rows)
}

/// Click the row `name` while `modifiers` are held (pressed before and released after the click).
fn click(h: &mut Harness<'_, TextureCraftApp>, name: &str, modifiers: Modifiers) {
    let at: Pos2 = h.get_by_label(name).rect().center();
    h.event(Event::PointerMoved(at));
    h.event(Event::ModifiersChanged(modifiers));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers });
    h.run_steps(2);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

fn highlighted(h: &Harness<'_, TextureCraftApp>, name: &str) -> bool {
    // egui reports a selected list item as toggled on
    h.get_by_label(name).accesskit_node().toggled() == Some(Toggled::True)
}

#[test]
fn shift_click_selects_a_range_and_highlights_its_rows() {
    let (mut h, [a, b, c, d]) = harness();
    click(&mut h, "Row B", Modifiers::NONE);
    click(&mut h, "Row D", Modifiers::SHIFT);
    assert_eq!(h.state().session.selected_layers(), [d, c, b]);
    assert_eq!(h.state().session.selected_layer(), Some(d), "the clicked row is the active layer");
    for name in ["Row B", "Row C", "Row D"] {
        assert!(highlighted(&h, name), "{name} is highlighted");
    }
    assert!(!highlighted(&h, "Row A"));

    // A second Shift+click ranges from B again, and Delete removes both layers
    click(&mut h, "Row A", Modifiers::SHIFT);
    assert_eq!(h.state().session.selected_layers(), [b, a], "the range starts at the last plain click");
    h.key_press(egui::Key::Delete);
    h.run_steps(3);
    let project = &h.state().session.doc.project;
    assert!(project.layer(a).is_err() && project.layer(b).is_err());
    assert!(project.layer(c).is_ok() && project.layer(d).is_ok());
}

#[test]
fn ctrl_click_adds_and_takes_out_a_row() {
    let (mut h, [a, _, c, _]) = harness();
    click(&mut h, "Row A", Modifiers::NONE);
    click(&mut h, "Row C", Modifiers::COMMAND);
    assert_eq!(h.state().session.selected_layers(), [c, a]);
    assert!(highlighted(&h, "Row A") && highlighted(&h, "Row C") && !highlighted(&h, "Row B"));
    click(&mut h, "Row C", Modifiers::COMMAND);
    assert_eq!(h.state().session.selected_layers(), [a]);
    assert!(!highlighted(&h, "Row C"));
    assert!(h.state().ui.rename.is_none(), "two quick Ctrl+clicks do not start a rename");
}
