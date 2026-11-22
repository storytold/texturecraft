//! The system's pointer in tests: the mouse moves it, and it obeys the app's put-backs, grabs and hidden cursor as a window system does

use egui::{CursorGrab, CursorIcon, Event, Modifiers, PointerButton, Pos2, Vec2, ViewportCommand, ViewportId};
use egui_kittest::Harness;

/// Where the system's pointer is and how the app holds it
#[derive(Clone, Copy, Debug)]
pub(crate) struct SystemPointer {
    pub(crate) at: Pos2,
    pub(crate) locked: bool,
    pub(crate) hidden: bool,
    pub(crate) put_backs: usize,
    /// The system folds a put-back into the next move, as Windows does, instead of reporting it as a move of its own
    pub(crate) folds: bool,
}

impl SystemPointer {
    /// The pointer resting at `at` over the window
    pub(crate) fn at<S>(harness: &mut Harness<'_, S>, at: Pos2) -> SystemPointer {
        let mut pointer = SystemPointer { at, locked: false, hidden: false, put_backs: 0, folds: false };
        harness.input_mut().events.push(Event::PointerMoved(at));
        pointer.frame(harness);
        pointer
    }

    /// One frame in which the mouse moves by `by` with `modifiers` held: a held pointer reports raw motion only, a free one moves
    pub(crate) fn moves<S>(&mut self, harness: &mut Harness<'_, S>, by: Vec2, modifiers: Modifiers) {
        let input = harness.input_mut();
        input.events.push(Event::ModifiersChanged(modifiers));
        input.events.push(Event::MouseMoved(by));
        if !self.locked {
            self.at += by;
            input.events.push(Event::PointerMoved(self.at));
        }
        self.frame(harness);
    }

    /// The mouse moves by `by` over `frames` frames
    pub(crate) fn drags<S>(&mut self, harness: &mut Harness<'_, S>, by: Vec2, frames: usize, modifiers: Modifiers) {
        for _ in 0..frames {
            self.moves(harness, by / frames as f32, modifiers);
        }
    }

    /// One frame in which `button` goes down or up where the pointer is
    pub(crate) fn button<S>(&mut self, harness: &mut Harness<'_, S>, button: PointerButton, pressed: bool, modifiers: Modifiers) {
        let input = harness.input_mut();
        input.events.push(Event::ModifiersChanged(modifiers));
        input.events.push(Event::PointerButton { pos: self.at, button, pressed, modifiers });
        self.frame(harness);
    }

    /// One frame of everything the system sent since the last: the app's raw input hook rewrites it, the app runs, and the pointer does what the app asked
    pub(crate) fn frame<S>(&mut self, harness: &mut Harness<'_, S>) {
        let ctx = harness.ctx.clone();
        super::rewrite(&ctx, harness.input_mut());
        harness.step();
        let output = harness.output();
        self.hidden = output.platform_output.cursor_icon == CursorIcon::None;
        let commands = output.viewport_output.get(&ViewportId::ROOT).map(|viewport| viewport.commands.clone()).unwrap_or_default();
        for command in commands {
            match command {
                ViewportCommand::CursorPosition(to) => {
                    self.at = to;
                    self.put_backs += 1;
                    if !self.folds {
                        harness.input_mut().events.push(Event::PointerMoved(to));
                    }
                }
                ViewportCommand::CursorGrab(grab) => self.locked = grab == CursorGrab::Locked,
                _ => {}
            }
        }
    }
}
