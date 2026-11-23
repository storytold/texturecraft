use egui::{Modifiers, PointerButton, Pos2, Rect, Sense, Vec2, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::system_pointer::SystemPointer;
use super::{Hold, ask_for_hold, hold_like, settle};

/// A strip that scrubs: dragging it adds the pointer's moves to the state, holding the pointer while it does
fn scrub_strip(hold_as: Hold) -> Harness<'static, Vec2> {
    let mut harness = Harness::builder().with_size(vec2(400.0, 300.0)).build_ui_state(
        |ui, moved: &mut Vec2| {
            let response = ui.interact(Rect::from_min_size(pos2(10.0, 10.0), vec2(100.0, 24.0)), ui.scope_id().with("scrub"), Sense::drag());
            if response.dragged() {
                ask_for_hold(ui.ctx());
                *moved += response.drag_delta();
            }
            settle(ui.ctx());
        },
        Vec2::ZERO,
    );
    hold_like(&harness.ctx, hold_as);
    harness.run_steps(1);
    harness
}

const PRESS: Pos2 = pos2(60.0, 20.0);

/// Drags the strip 1500 points right (far past the 400-point window) and back up 40, then lets go
fn scrub_past_the_edge(harness: &mut Harness<'static, Vec2>) -> SystemPointer {
    let mut pointer = SystemPointer::at(harness, PRESS);
    pointer.button(harness, PointerButton::Primary, true, Modifiers::NONE);
    pointer.drags(harness, vec2(1500.0, -40.0), 30, Modifiers::NONE);
    assert!(pointer.hidden, "the pointer is hidden while the drag holds it");
    assert!((*harness.state() - vec2(1500.0, -40.0)).length() < 0.01, "every move counts, past the window's edge: {:?}", harness.state());
    pointer
}

#[test]
fn a_put_back_pointer_never_reaches_the_edge_and_comes_back_where_the_press_began() {
    let mut harness = scrub_strip(Hold::PutBack);
    let mut pointer = scrub_past_the_edge(&mut harness);

    assert!(pointer.put_backs >= 30, "the pointer went back after every move: {}", pointer.put_backs);
    assert!(pointer.at.distance(PRESS) < 1.0, "it never strayed from the press: {:?}", pointer.at);

    pointer.button(&mut harness, PointerButton::Primary, false, Modifiers::NONE);
    pointer.frame(&mut harness);

    assert!(!pointer.hidden, "let go, the pointer shows again");
    assert_eq!(pointer.at, PRESS, "where the press began");
    assert_eq!(harness.ctx.input(|input| input.pointer.latest_pos()), Some(PRESS), "egui sees it there too");

    let moved = *harness.state();
    pointer.moves(&mut harness, vec2(30.0, 0.0), Modifiers::NONE);

    assert_eq!(pointer.at, PRESS + vec2(30.0, 0.0), "a free pointer moves again");
    assert_eq!(*harness.state(), moved, "and drags nothing");
}

/// A system that folds the put-back into the next move (Windows) reads that move from the press point: the drag counts in full and never stalls
#[test]
fn a_folded_put_back_counts_every_move_from_the_press() {
    let mut harness = scrub_strip(Hold::PutBackFolded);
    let mut pointer = SystemPointer::at(&mut harness, PRESS);
    pointer.folds = true;
    pointer.button(&mut harness, PointerButton::Primary, true, Modifiers::NONE);
    pointer.drags(&mut harness, vec2(1500.0, -40.0), 30, Modifiers::NONE);

    assert!(pointer.hidden && pointer.put_backs >= 30, "hidden, and put back after every move: {pointer:?}");
    assert!((*harness.state() - vec2(1500.0, -40.0)).length() < 0.01, "every move counts: {:?}", harness.state());

    pointer.button(&mut harness, PointerButton::Primary, false, Modifiers::NONE);
    pointer.frame(&mut harness);

    assert!(!pointer.hidden && pointer.at == PRESS, "let go, it shows again where the press began: {pointer:?}");
}

#[test]
fn a_locked_pointer_reads_raw_motion_and_is_let_go_where_the_press_began() {
    let mut harness = scrub_strip(Hold::Locked);
    let mut pointer = scrub_past_the_edge(&mut harness);

    assert!(pointer.locked, "the system holds the pointer");
    assert_eq!(pointer.at, PRESS, "it never moved");

    pointer.button(&mut harness, PointerButton::Primary, false, Modifiers::NONE);
    pointer.frame(&mut harness);

    assert!(!pointer.locked && !pointer.hidden, "let go, the pointer is free and shows");
    assert_eq!(harness.ctx.input(|input| input.pointer.latest_pos()), Some(PRESS), "egui sees it where the press began");
}

#[test]
fn a_browser_pointer_is_not_held() {
    let mut harness = scrub_strip(Hold::Free);
    let mut pointer = SystemPointer::at(&mut harness, PRESS);
    pointer.button(&mut harness, PointerButton::Primary, true, Modifiers::NONE);
    pointer.drags(&mut harness, vec2(100.0, 0.0), 10, Modifiers::NONE);

    assert!(!pointer.hidden && pointer.put_backs == 0, "the pointer moves and shows");
    assert_eq!(*harness.state(), vec2(100.0, 0.0), "the drag reads it as it moves");
}

/// A number field in the Properties panel scrubs with the pointer held: hidden, kept where the press began, a drag far past the window's edge counted in full
#[test]
fn a_number_field_scrubs_past_the_edge_with_the_pointer_held() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut harness, layer) = crate::frame_gizmo::tests::fill_app("triplanar");
    hold_like(&harness.ctx, Hold::PutBack);
    let press = harness.get_by_label("Offset X").rect().center();
    let mut pointer = SystemPointer::at(&mut harness, press);
    pointer.button(&mut harness, PointerButton::Primary, true, Modifiers::NONE);
    let window = harness.ctx.content_rect().width();
    pointer.drags(&mut harness, vec2(2.0 * window, 0.0), 40, Modifiers::NONE);

    assert!(pointer.hidden && pointer.at.distance(press) < 1.0, "the pointer is hidden and kept on the field: {pointer:?}");

    pointer.button(&mut harness, PointerButton::Primary, false, Modifiers::NONE);
    pointer.frame(&mut harness);

    assert!(!pointer.hidden && pointer.at == press, "let go, it shows again on the field: {pointer:?}");

    let texturecraft_engine::core::Projection::Triplanar { offset, .. } = crate::frame_gizmo::tests::projection(&harness, layer) else {
        panic!("still tri-planar")
    };

    let expected = 2.0 * window * crate::fill_panel::OFFSET_FIELD_SPEED;
    assert!((offset[0] - expected).abs() < 0.05 * expected, "the whole drag counts, not only the part inside the window: {offset:?}, {expected}");
}
