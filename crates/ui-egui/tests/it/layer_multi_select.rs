//! Shift+click and Ctrl/Cmd+click on Layers panel rows select several layers, as in the
//! reference app, and the selected rows are highlighted.

use egui::accesskit::Toggled;
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::core::{LayerId, LayerKind};
use texturecraft_engine::execute;
use texturecraft_ui_egui::TextureCraftApp;

/// Paint layers "Row A" (bottom) to "Row D" (top) in the active texture set, "Row D" selected.
fn harness() -> (Harness<'static, TextureCraftApp>, [LayerId; 4]) {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let rows = ["Row A", "Row B", "Row C", "Row D"]
        .map(|name| LayerId(execute(&mut session, "layer.add.paint", &json!({ "name": name })).unwrap()["layer"].as_u64().unwrap()));
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

/// Click the eye of the row `name` (it sits at the left of a top-level row, after the 7-point edge).
fn click_eye(h: &mut Harness<'_, TextureCraftApp>, name: &str) {
    let row = h.get_by_label(name).rect();
    let eye = egui::pos2(row.left() + 7.0 + 8.0, row.center().y);
    h.drag_at(eye);
    h.drop_at(eye);
    h.run_steps(3);
}

fn visible(h: &Harness<'_, TextureCraftApp>, layer: LayerId) -> bool {
    h.state().session.doc.project.layer(layer).unwrap().visible
}

#[test]
fn the_eye_of_a_selected_row_shows_or_hides_that_layer_only() {
    let (mut h, [a, _, c, _]) = harness();
    click(&mut h, "Row A", Modifiers::NONE);
    click(&mut h, "Row C", Modifiers::COMMAND);
    click_eye(&mut h, "Row C");
    assert!(!visible(&h, c), "C is hidden");
    assert!(visible(&h, a), "A, also selected, stays shown");
    assert_eq!(h.state().session.selected_layers(), [c, a], "the selection stays");
    click_eye(&mut h, "Row C");
    assert!(visible(&h, c), "a second click shows C again");
}

#[test]
fn the_eye_of_an_unselected_row_acts_on_its_own_layer_and_keeps_the_selection() {
    let (mut h, [a, b, c, _]) = harness();
    click(&mut h, "Row A", Modifiers::NONE);
    click(&mut h, "Row C", Modifiers::COMMAND);
    click_eye(&mut h, "Row B");
    assert!(!visible(&h, b), "B is hidden");
    assert!(visible(&h, a) && visible(&h, c), "the selected layers stay shown");
    assert_eq!(h.state().session.selected_layers(), [c, a], "the selection stays");
}

#[test]
fn ctrl_a_over_the_layers_panel_selects_every_shown_row_but_not_the_layers_in_a_collapsed_folder() {
    let (mut h, [a, b, c, d]) = harness();
    let session = &mut h.state_mut().session;
    let folder = LayerId(execute(session, "layer.add.group", &json!(null)).unwrap()["layer"].as_u64().unwrap());
    execute(session, "layer.move", &json!({"layers": [a.0, b.0], "above": folder.0, "inside": true})).unwrap();
    if let LayerKind::Group { collapsed, .. } = &mut session.doc.project.layer_mut(folder).unwrap().kind {
        *collapsed = true;
    }
    execute(session, "layer.select", &json!({"layer": d.0})).unwrap();
    h.run_steps(3);
    let at = h.get_by_label("Row C").rect().center();
    h.event(Event::PointerMoved(at));
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(3);
    let session = &h.state().session;
    let every_row = texturecraft_engine::layer_selection::stack_rows(&session.doc.project, 0, true);
    for layer in [c, d, folder] {
        assert!(session.is_layer_selected(layer), "layer {} is selected", layer.0);
    }
    assert_eq!(session.selected_layers().len(), every_row.len(), "every shown row is selected");
    assert!(!session.is_layer_selected(a) && !session.is_layer_selected(b), "the collapsed folder's layers are not");
    assert_eq!(h.state().session.selected_layer(), Some(d), "the active layer stays");
}

#[test]
fn ctrl_a_leaves_text_fields_and_other_panels_alone() {
    let (mut h, [_, _, c, d]) = harness();

    // Pointer outside every panel: nothing is selected
    h.event(Event::PointerGone);
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(3);
    assert_eq!(h.state().session.selected_layers(), [d], "no pointer over the Layers panel");

    // A rename field over the panel keeps Ctrl+A for its own text
    h.state_mut().ui.rename = Some((c.0, "Row C".into()));
    h.run_steps(3);
    let at = h.get_by_label("Row B").rect().center();
    h.event(Event::PointerMoved(at));
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(3);
    assert_eq!(h.state().session.selected_layers().len(), 1, "Ctrl+A in the rename field selects text, not layers");
}

/// Right-click the row `name` and choose the menu item whose label starts with `item`.
fn row_menu(h: &mut Harness<'_, TextureCraftApp>, name: &str, item: &str) {
    h.get_by_label(name).click_secondary();
    h.run_steps(3);
    h.get_by(|node| node.label().is_some_and(|label| label.starts_with(item))).click();
    h.run_steps(3);
}

#[test]
fn the_row_menu_of_a_selected_row_acts_on_the_whole_selection() {
    let (mut h, [a, b, c, _]) = harness();
    click(&mut h, "Row A", Modifiers::NONE);
    click(&mut h, "Row C", Modifiers::COMMAND);
    row_menu(&mut h, "Row C", "Duplicate layer(s)");
    let names: Vec<String> = h.state().session.selected_layers().iter().map(|id| h.state().session.doc.project.layer(*id).unwrap().name.clone()).collect();
    assert_eq!(names, ["Row C copy", "Row A copy"], "both selected layers are duplicated");

    click(&mut h, "Row A", Modifiers::NONE);
    click(&mut h, "Row C", Modifiers::COMMAND);
    row_menu(&mut h, "Row A", "Group layer(s)");
    let folder = h.state().session.selected_layer().unwrap();
    assert_eq!(h.state().session.doc.project.layer(folder).unwrap().children(), [a, c], "both go into the folder");

    // On a row outside the selection, only that row
    row_menu(&mut h, "Row B", "Remove layer(s)");
    let project = &h.state().session.doc.project;
    assert!(project.layer(b).is_err());
    assert!(project.layer(a).is_ok() && project.layer(c).is_ok());
}

#[test]
fn the_row_menu_of_an_unselected_row_acts_on_that_row_only() {
    let (mut h, [a, b, c, _]) = harness();
    click(&mut h, "Row A", Modifiers::NONE);
    click(&mut h, "Row C", Modifiers::COMMAND);
    row_menu(&mut h, "Row B", "Duplicate layer(s)");
    let project = &h.state().session.doc.project;
    assert_eq!(project.layers.values().filter(|layer| layer.name.ends_with(" copy")).count(), 1);
    assert!(project.layers.values().any(|layer| layer.name == "Row B copy"));
    assert!(project.layer(a).is_ok() && project.layer(b).is_ok() && project.layer(c).is_ok());
}
