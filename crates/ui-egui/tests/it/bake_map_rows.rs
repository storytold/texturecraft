//! The Mesh Map Bakers rows: a primary drag down the checkboxes gives each the state the first one took

use egui::{Event, Modifiers, PointerButton, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_engine::core::BakedMap;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;
use texturecraft_ui_egui::state::Mode;

/// The rows the drag crosses, from the press to the release
const DRAGGED: [(BakedMap, &str); 3] = [(BakedMap::Normal, "Normal"), (BakedMap::WorldNormal, "World Space Normal"), (BakedMap::Id, "ID")];

/// A bake-mode app with Mesh Map Bakers in front and every map checked
fn harness() -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    session.doc.project.texture_sets[0].bake.maps.select(BakedMap::ALL);
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1500.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(4);
    harness.state_mut().ui.dock.focus(Panel::MeshMapBakers);
    harness.run_steps(3);
    harness
}

fn checkbox_center(harness: &Harness<'_, TextureCraftApp>, name: &str) -> egui::Pos2 {
    harness.get_by_role_and_label(Role::CheckBox, name).rect().center()
}

fn enabled(harness: &Harness<'_, TextureCraftApp>) -> Vec<BakedMap> {
    let maps = &harness.state().session.doc.project.texture_sets[0].bake.maps;
    BakedMap::ALL.into_iter().filter(|map| maps.is_enabled(*map)).collect()
}

#[test]
fn a_drag_down_the_checkboxes_unchecks_every_row_it_crosses() {
    let mut harness = harness();
    let [from, through, to] = DRAGGED.map(|(_, name)| checkbox_center(&harness, name));

    harness.hover_at(from);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(2);
    for position in [through, to] {
        harness.hover_at(position);
        harness.run_steps(1);
    }
    harness.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(3);

    let expected: Vec<BakedMap> = BakedMap::ALL.into_iter().filter(|map| DRAGGED.iter().all(|(dragged, _)| dragged != map)).collect();
    assert_eq!(enabled(&harness), expected, "the three rows are unchecked, the release does not check the last one again");
}

#[test]
fn a_drag_that_jumps_past_a_row_between_two_frames_still_unchecks_it() {
    let mut harness = harness();
    let [from, _, to] = DRAGGED.map(|(_, name)| checkbox_center(&harness, name));

    harness.hover_at(from);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(2);
    harness.hover_at(to);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(3);

    let expected: Vec<BakedMap> = BakedMap::ALL.into_iter().filter(|map| DRAGGED.iter().all(|(dragged, _)| dragged != map)).collect();
    assert_eq!(enabled(&harness), expected, "World Space Normal, between Normal and ID, is unchecked too");
}

#[test]
fn holding_the_secondary_button_on_a_checkbox_leaves_it_alone() {
    let mut harness = harness();
    let at = checkbox_center(&harness, DRAGGED[1].1);

    harness.hover_at(at);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(3);
    harness.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(3);

    assert_eq!(enabled(&harness), BakedMap::ALL);
}

#[test]
fn a_click_on_one_checkbox_still_toggles_only_that_row() {
    let mut harness = harness();
    let (map, name) = DRAGGED[1];
    let at = checkbox_center(&harness, name);

    harness.hover_at(at);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(2);
    harness.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(3);

    let expected: Vec<BakedMap> = BakedMap::ALL.into_iter().filter(|other| *other != map).collect();
    assert_eq!(enabled(&harness), expected);
}

#[test]
fn keyboard_opens_an_unselected_maps_actions_without_selecting_it() {
    let mut harness = harness();
    let row = "Height";
    harness.get_by_role_and_label(Role::Button, row).focus();
    harness.run_steps(2);

    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::F10);
    harness.run_steps(3);

    assert!(harness.query_by_label("Check Height for More Texture Sets...").is_some());
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);

    assert!(harness.get_by_role_and_label(Role::Button, row).is_focused());
    harness.key_press(egui::Key::Menu);
    harness.run_steps(3);
    harness.get_by_label("Check Height for More Texture Sets...").focus();
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);

    assert!(harness.get_by_role_and_label(Role::Button, "Close").is_focused());
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);

    assert!(harness.get_by_role_and_label(Role::Button, row).is_focused());
    assert_eq!(harness.get_by_role_and_label(Role::Button, row).accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
}
