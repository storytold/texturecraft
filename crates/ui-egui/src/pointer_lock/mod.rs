//! Drags that hold the pointer (Move on model, number fields that scrub): hidden while held, its motion endless so no screen edge stops it, back where the press began when let go. egui sees a pointer that keeps going: the raw input is rewritten to that endless position, so every drag reads its usual positions and deltas

mod hold;
mod lock;
#[cfg(test)]
pub(crate) mod system_pointer;
#[cfg(test)]
mod tests;

use egui::{Context, CursorIcon, Event, Id, RawInput, ViewportCommand};

pub(crate) use hold::Hold;
use lock::Lock;

/// Points from where the press began within which a pointer move is the put-back arriving (the system rounds it to whole pixels)
const PUT_BACK_REACH: f32 = 1.0;

fn lock_id() -> Id {
    Id::unique("tc_pointer_lock")
}

fn asked_id() -> Id {
    Id::unique("tc_pointer_lock_asked")
}

fn hold_override_id() -> Id {
    Id::unique("tc_pointer_lock_hold")
}

/// A drag under way asks for the pointer this frame; it keeps it as long as it asks every frame
pub(crate) fn ask_for_hold(ctx: &Context) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| data.insert_temp(asked_id(), pass));
}

/// The app's raw input hook: while a drag holds the pointer, its moves become moves of the endless pointer egui sees
pub(crate) fn rewrite(ctx: &Context, raw: &mut RawInput) {
    let Some(mut lock) = ctx.data(|data| data.get_temp::<Lock>(lock_id())) else {
        return;
    };

    if lock.let_go {
        raw.events.insert(0, Event::PointerMoved(lock.anchor));
        ctx.data_mut(|data| data.remove::<Lock>(lock_id()));
        return;
    }

    let zoom = ctx.zoom_factor().max(f32::EPSILON);
    let events = std::mem::take(&mut raw.events);
    for event in events {
        match event {
            Event::PointerMoved(to) => {
                let put_back = lock.putting_back && to.distance(lock.anchor) <= PUT_BACK_REACH;
                lock.putting_back &= !put_back;
                if lock.hold.puts_back() && !put_back {
                    lock.at += to - lock.seen;
                    raw.events.push(Event::PointerMoved(lock.at));
                }
                lock.seen = to;
            }
            Event::MouseMoved(delta) => {
                if lock.hold == Hold::Locked {
                    lock.at += delta / zoom;
                    raw.events.push(Event::PointerMoved(lock.at));
                }
                raw.events.push(event);
            }
            Event::PointerButton { button, pressed, modifiers, .. } => raw.events.push(Event::PointerButton { pos: lock.at, button, pressed, modifiers }),
            // The pointer never really leaves: the drag goes on
            Event::PointerGone => {}
            other => raw.events.push(other),
        }
    }
    ctx.data_mut(|data| data.insert_temp(lock_id(), lock));
}

/// The end of the app's frame: a drag that asked this frame takes or keeps the pointer (hidden, put back), one that stopped asking lets it go
pub(crate) fn settle(ctx: &Context) {
    let asked = !crate::viewport::focus_lost(ctx) && ctx.data(|data| data.get_temp::<u64>(asked_id())) == Some(ctx.cumulative_pass_nr());
    let held = ctx.data(|data| data.get_temp::<Lock>(lock_id())).filter(|lock| !lock.let_go);
    let mut lock = match (asked, held) {
        (true, Some(lock)) => lock,
        (true, None) => {
            let hold = ctx.data(|data| data.get_temp::<Hold>(hold_override_id())).unwrap_or_else(|| Hold::of_platform(ctx));
            let (origin, latest) = ctx.input(|input| (input.pointer.press_origin(), input.pointer.latest_pos()));
            let (Some(seen), false) = (latest, hold == Hold::Free) else {
                return;
            };

            if hold == Hold::Locked {
                ctx.send_viewport_cmd(ViewportCommand::CursorGrab(egui::CursorGrab::Locked));
            }
            Lock { hold, anchor: origin.unwrap_or(seen), seen, at: seen, putting_back: false, let_go: false }
        }
        (false, Some(mut lock)) => {
            // Put back before the grab ends: Wayland moves only a locked pointer
            if !lock.putting_back && lock.seen.distance(lock.anchor) > PUT_BACK_REACH {
                ctx.send_viewport_cmd(ViewportCommand::CursorPosition(lock.anchor));
            }
            if lock.hold == Hold::Locked {
                ctx.send_viewport_cmd(ViewportCommand::CursorGrab(egui::CursorGrab::None));
            }
            lock.let_go = true;
            ctx.data_mut(|data| data.insert_temp(lock_id(), lock));
            ctx.request_repaint();
            return;
        }
        (false, None) => return,
    };

    ctx.set_cursor_icon(CursorIcon::None);
    if lock.hold.puts_back() && !lock.putting_back && lock.seen.distance(lock.anchor) > PUT_BACK_REACH {
        ctx.send_viewport_cmd(ViewportCommand::CursorPosition(lock.anchor));

        // A folding system reads every later move from where it put the pointer, its own move folded in
        if lock.hold == Hold::PutBackFolded {
            lock.seen = lock.anchor;
        } else {
            lock.putting_back = true;
        }
    }
    ctx.data_mut(|data| data.insert_temp(lock_id(), lock));
}

/// Tests pick how the pointer is held, as a platform would
#[cfg(test)]
pub(crate) fn hold_like(ctx: &Context, hold: Hold) {
    ctx.data_mut(|data| data.insert_temp(hold_override_id(), hold));
}
