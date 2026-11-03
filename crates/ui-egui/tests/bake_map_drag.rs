//! Bake mode: pressing a mesh map's checkbox and dragging down the list sets every row the
//! pointer passes over to the state the first one got.

use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_engine::core::BakedMap;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::Mode;

fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1500.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    // The sample set bakes at 64, below the smallest resolution the settings accept.
    h.state_mut().session.doc.project.texture_sets[0].bake.resolution = 256;
    h.state_mut().ui.mode = Mode::Baking;
    h.run_steps(4);
    h
}

fn maps(h: &Harness<'_, TextureCraftApp>) -> Vec<BakedMap> {
    let ts = h.state().session.ts();
    BakedMap::ALL.into_iter().filter(|m| h.state().session.doc.project.texture_sets[ts].bake.maps.contains(m)).collect()
}

/// The checkbox of the row labelled `label`.
fn checkbox(h: &Harness<'_, TextureCraftApp>, label: &str) -> Pos2 {
    let y = h.get_by_label(label).rect().center().y;
    let boxes = h.get_all_by_role(Role::CheckBox);
    boxes.map(|b| b.rect()).find(|r| (r.center().y - y).abs() < 4.0).expect("the row has a checkbox").center()
}

fn press(h: &mut Harness<'_, TextureCraftApp>, at: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
}

fn drag(h: &mut Harness<'_, TextureCraftApp>, from: &str, to: &str) {
    let (a, b) = (checkbox(h, from), checkbox(h, to));
    h.hover_at(a);
    h.run_steps(1);
    press(h, a, true);
    h.run_steps(2);
    for k in 1..=6 {
        h.hover_at(a + (b - a) * (k as f32 / 6.0));
        h.run_steps(1);
    }
    press(h, b, false);
    h.run_steps(3);
}

#[test]
fn dragging_down_the_list_sets_every_row_to_the_state_of_the_first() {
    let mut h = harness();
    let before = maps(&h);
    let (first, last) = (BakedMap::ALL[1], BakedMap::ALL[4]);
    let (from, to) = (first.label(), last.label());
    let between: Vec<BakedMap> = BakedMap::ALL[1..=4].to_vec();
    let on = before.contains(&first);
    // The first row's state flips and the others follow it, whatever they were.
    drag(&mut h, from, to);
    let after = maps(&h);
    for m in &between {
        assert_eq!(after.contains(m), !on, "{m:?} follows the first row");
    }
    for m in BakedMap::ALL.iter().filter(|m| !between.contains(m)) {
        assert_eq!(after.contains(m), before.contains(m), "{m:?} is not passed over");
    }
    // Dragging back up over the same rows with the opposite state restores them.
    drag(&mut h, to, from);
    let again = maps(&h);
    for m in &between {
        assert_eq!(again.contains(m), on, "{m:?} follows the last row");
    }
}

#[test]
fn a_plain_click_still_toggles_one_map() {
    let mut h = harness();
    let before = maps(&h);
    let m = BakedMap::ALL[2];
    let at = checkbox(&h, m.label());
    h.hover_at(at);
    h.run_steps(1);
    press(&mut h, at, true);
    h.run_steps(1);
    press(&mut h, at, false);
    h.run_steps(3);
    let after = maps(&h);
    assert_ne!(after.contains(&m), before.contains(&m));
    assert_eq!(after.len() as i32 - before.len() as i32, if before.contains(&m) { -1 } else { 1 });
}
