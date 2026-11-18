//! The Texture Sets list: rename by double-click and from the row menu, the row menu, one size
//! for all sets, and the next / previous set commands.

use egui::{Event, Key, Modifiers, PointerButton};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_ui_egui::TextureCraftApp;

fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).with_step_dt(1.0 / 60.0).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    h
}

fn names(h: &Harness<'_, TextureCraftApp>) -> Vec<String> {
    h.state().session.doc.project.texture_sets.iter().map(|t| t.name.clone()).collect()
}

fn sizes(h: &Harness<'_, TextureCraftApp>) -> Vec<u32> {
    h.state().session.doc.project.texture_sets.iter().map(|t| t.resolution).collect()
}

fn row_center(h: &Harness<'_, TextureCraftApp>, name: &str) -> egui::Pos2 {
    // The row's label is only the name: pick the list row, not the settings panel's name
    h.get_all_by_label(name).map(|n| n.rect()).min_by(|a, b| a.min.x.total_cmp(&b.min.x)).unwrap().center()
}

fn click(h: &mut Harness<'_, TextureCraftApp>, pos: egui::Pos2, button: PointerButton) {
    h.hover_at(pos);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
}

fn type_name(h: &mut Harness<'_, TextureCraftApp>, text: &str) {
    h.run_steps(3);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text(text.into()));
    h.run_steps(3);
}

#[test]
fn double_click_renames_a_set() {
    let mut h = harness();
    let name = names(&h)[1].clone();
    let at = row_center(&h, &name);
    // Two clicks a few frames apart: a double-click
    click(&mut h, at, PointerButton::Primary);
    click(&mut h, at, PointerButton::Primary);
    assert_eq!(h.state().ui.set_rename.as_ref().map(|r| r.0), Some(1), "double-click opens the editor on that row");
    type_name(&mut h, "Chrome base");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(h.state().ui.set_rename.is_none());
    assert_eq!(names(&h)[1], "Chrome base");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(3);
    assert_eq!(names(&h)[1], name, "Ctrl+Z undoes the rename");
}

#[test]
fn escape_cancels_the_rename() {
    let mut h = harness();
    let name = names(&h)[0].clone();
    h.state_mut().ui.set_rename = Some((0, name.clone()));
    type_name(&mut h, "Nope");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().ui.set_rename.is_none());
    assert_eq!(names(&h)[0], name);
}

#[test]
fn the_row_menu_renames_and_selects_the_row() {
    let mut h = harness();
    let name = names(&h)[1].clone();
    let at = row_center(&h, &name);
    click(&mut h, at, PointerButton::Secondary);
    h.get_by_label("Rename").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.set_rename.as_ref().map(|r| r.0), Some(1));
    assert_eq!(h.state().session.ts(), 1, "Rename selects the row");
    type_name(&mut h, "From the menu");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(names(&h)[1], "From the menu");
}

#[test]
fn the_row_menu_hides_and_isolates_a_set() {
    let mut h = harness();
    let name = names(&h)[1].clone();
    let at = row_center(&h, &name);
    click(&mut h, at, PointerButton::Secondary);
    h.get_by_label("Hide").click();
    h.run_steps(3);
    assert!(!h.state().session.doc.project.texture_sets[1].visible);
    let at = row_center(&h, &name);
    click(&mut h, at, PointerButton::Secondary);
    h.get_by_label("Show only this set").click();
    h.run_steps(3);
    let visible: Vec<bool> = h.state().session.doc.project.texture_sets.iter().map(|t| t.visible).collect();
    assert_eq!(visible, vec![false, true]);
}

#[test]
fn same_size_for_all_sets_from_the_row_menu() {
    let mut h = harness();
    // Make the sets differ first
    h.state_mut().run("textureSet.resolution", serde_json::json!({"ts": 1, "resolution": 128}));
    assert_ne!(sizes(&h)[0], sizes(&h)[1]);
    let name = names(&h)[0].clone();
    let at = row_center(&h, &name);
    click(&mut h, at, PointerButton::Secondary);
    h.get_by_label("Same size for all sets").click();
    h.run_steps(2);
    h.get_by_label("512").click();
    for _ in 0..200 {
        h.run_steps(2);
        if h.state().resize.is_none() && sizes(&h).iter().all(|r| *r == 512) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(sizes(&h), vec![512; sizes(&h).len()]);
}

#[test]
fn next_and_previous_set_are_bindable_menu_commands() {
    let mut h = harness();
    for id in ["textureSet.next", "textureSet.previous"] {
        assert!(texturecraft_ui_egui::menus::CATALOG.iter().any(|i| i.id == id), "{id} is in the menu catalog");
        assert!(texturecraft_ui_egui::keymap::bindings().iter().any(|b| b.id == id && b.default.is_none()), "{id} can get a key, and has none by default");
    }
    let ctx = h.ctx.clone();
    texturecraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "textureSet.next");
    assert_eq!(h.state().session.ts(), 1);
    texturecraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "textureSet.previous");
    assert_eq!(h.state().session.ts(), 0);
}
