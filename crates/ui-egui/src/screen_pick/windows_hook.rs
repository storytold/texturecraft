//! Windows: a low-level mouse hook on its own thread takes a left click outside the app's windows (the click never reaches the
//! window under it), and the thread then reads the one pixel under it from the desktop through GDI; clicks on the app's own
//! windows pass on. The system pointer is the eyedropper with its colour disc until the wait ends

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use winsafe::prelude::Handle;
use winsafe::{HHOOK, HINSTANCE, HPROCESS, HWND, MSG, POINT, co};

use super::eyedropper_pointer::EyedropperPointer;
use super::hook_state::HookState;
use super::live_swatch::LiveSwatch;
use super::pick_shared::over_windows;
use super::release_wait::ReleaseWait;
use super::{PickOutcome, PickShared};

/// How often the hook thread wakes to see whether the pick ended in the app, in milliseconds
const STOP_CHECK_MS: u32 = 50;
/// The id asked for the stop-check timer (a thread timer gets its id from the system, which returns it)
const STOP_CHECK_TIMER_ID: usize = 1;

static HOOK: Mutex<Option<HookState>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// Held from showing the eyedropper until the user's pointers are back: a pick's keeper never copies the last one's eyedropper
static ONE_POINTER_KEEPER: Mutex<()> = Mutex::new(());

/// Waits on this thread for a left click outside the app until it lands or `shared` stops the pick (`dpi` sizes the pointer)
pub(super) fn run(shared: &Arc<PickShared>, outcomes: Sender<PickOutcome>, ctx: egui::Context, dpi: u32) {
    // Taken before this pick's state goes in: the last pick's hook reports to its own state until it is gone
    let keeper_turn = ONE_POINTER_KEEPER.lock().unwrap_or_else(PoisonError::into_inner);

    let unavailable = outcomes.clone();
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut hook) = HOOK.lock() {
        *hook = Some(HookState::new(generation, Arc::clone(shared), outcomes, ctx.clone()));
    }

    // A pick the app ended while the last one was finishing shows no pointer
    if shared.stopped() {
        clear_state(generation);
        return;
    }

    // Prepare the pointer before installing the hook so its message loop starts promptly
    let pointer = EyedropperPointer::show(dpi);
    let Ok(module) = HINSTANCE::GetModuleHandle(None) else { return finish_unavailable(generation, &unavailable, &ctx) };
    let Ok(mut hook) = HHOOK::SetWindowsHookEx(co::WH::MOUSE_LL, mouse_hook, Some(&module), None) else {
        return finish_unavailable(generation, &unavailable, &ctx);
    };

    // A thread timer wakes GetMessage so the loop sees a stop and a taken press; the hook procedure runs inside GetMessage
    let Ok(timer) = HWND::NULL.SetTimer(STOP_CHECK_TIMER_ID, STOP_CHECK_MS, None) else {
        // Without it a stop in the app would leave the hook in place until the next click
        let _ = hook.UnhookWindowsHookEx();
        return finish_unavailable(generation, &unavailable, &ctx);
    };

    let mut message = MSG::default();

    // The disc follows on a thread of its own: this one must answer the hook at once, or every mouse move stalls
    let disc = pointer.as_ref().map(|pointer| (pointer.shown_event.clone(), pointer.cell));
    std::thread::scope(|scope| {
        if let Some((shown_event, cell)) = &disc {
            let follow = std::thread::Builder::new().name("eyedropper disc".into());
            let _ = follow.spawn_scoped(scope, || LiveSwatch::follow(shown_event, *cell, shared, generation));
        }
        while waiting(shared, generation) && matches!(winsafe::GetMessage(&mut message, None, 0, 0), Ok(true)) {
            deliver_pick(generation);
        }

        // Remove the hook before joining the disc thread so desktop input can continue
        let _ = hook.UnhookWindowsHookEx();
        let _ = HWND::NULL.KillTimer(timer);

        // A press and its release can both come in one wait for a message; the state's end stops the disc thread too
        deliver_pick(generation);
        clear_state(generation);
    });

    // The user's own pointers are back before the next pick's keeper may start
    drop(pointer);
    drop(keeper_turn);
}

/// Ends pick `generation` where no hook can be set: the pick stays on the model
fn finish_unavailable(generation: u64, outcomes: &Sender<PickOutcome>, ctx: &egui::Context) {
    clear_state(generation);
    let _ = outcomes.send(PickOutcome::Unavailable);
    ctx.request_repaint();
}

/// Takes pick `generation`'s state away, if the hook still holds it
fn clear_state(generation: u64) {
    if let Ok(mut hook) = HOOK.lock()
        && hook.as_ref().is_some_and(|state| state.generation == generation)
    {
        *hook = None;
    }
}

/// Reads and sends the pixel under the press pick `generation` took, once, out of the hook procedure and its lock
fn deliver_pick(generation: u64) {
    let Some((point, outcomes, ctx)) = take_pending_pick(generation) else { return };

    let outcome = screen_pixel(point).map_or(PickOutcome::Unavailable, PickOutcome::Picked);
    let _ = outcomes.send(outcome);
    ctx.request_repaint();
}

/// The press pick `generation` took and not yet read, with where to send its colour
fn take_pending_pick(generation: u64) -> Option<(POINT, Sender<PickOutcome>, egui::Context)> {
    let mut hook = HOOK.lock().ok()?;
    let state = hook.as_mut().filter(|state| state.generation == generation)?;
    let point = state.pending_pick.take()?;
    Some((point, state.outcomes.clone(), state.ctx.clone()))
}

/// Whether pick `generation` still waits: for its click until the app ends the pick, then for the release of the press it took
pub(super) fn waiting(shared: &PickShared, generation: u64) -> bool {
    let Ok(mut hook) = HOOK.lock() else { return false };
    let Some(state) = hook.as_mut().filter(|state| state.generation == generation) else { return false };

    // A hook the system removed never sees the release: the button being up ends that wait
    if state.release_wait.as_mut().is_some_and(|release_wait| release_wait.over(pick_button_down(), Instant::now())) {
        state.release_wait = None;
    }

    state.release_wait.is_some() || (!state.done && !shared.stopped())
}

/// Whether the button that sends left clicks is physically down (the system reports the physical buttons, swapped or not)
fn pick_button_down() -> bool {
    let button = if winsafe::GetSystemMetrics(co::SM::SWAPBUTTON) != 0 { co::VK::RBUTTON } else { co::VK::LBUTTON };
    winsafe::GetAsyncKeyState(button)
}

extern "system" fn mouse_hook(code: i32, wparam: usize, lparam: isize) -> isize {
    // A negative code must go straight to the next hook
    if code >= 0 && take(wparam, lparam) {
        return 1;
    }

    winsafe::CallNextHookEx(code, wparam, lparam)
}

/// Whether the mouse message `message` is the pick's own: a left press outside the app (noted, then taken) or its release
fn take(message: usize, event_address: isize) -> bool {
    let Ok(mut hook) = HOOK.lock() else { return false };
    let Some(state) = hook.as_mut() else { return false };

    let message = u32::try_from(message).unwrap_or_default();
    if message == co::WM::LBUTTONUP.raw() && state.release_wait.is_some() {
        state.release_wait = None;
        return true;
    }

    if message != co::WM::LBUTTONDOWN.raw() || state.done || state.shared.stopped() {
        return false;
    }

    // MSLLHOOKSTRUCT starts with the event's POINT, in physical screen coordinates
    let mut coordinates = [0; 8];
    if HPROCESS::GetCurrentProcess().ReadProcessMemory(event_address as *mut _, &mut coordinates) != Ok(coordinates.len()) {
        return false;
    }

    let [x0, x1, x2, x3, y0, y1, y2, y3] = coordinates;
    let point = POINT { x: i32::from_ne_bytes([x0, x1, x2, x3]), y: i32::from_ne_bytes([y0, y1, y2, y3]) };
    if over_own_window(point) || over_windows(&state.shared.windows(), point.x, point.y) {
        return false;
    }

    // Only noted here: the system removes a hook that answers too slowly, so the pixel is read after this returns
    state.pending_pick = Some(point);
    state.release_wait = Some(ReleaseWait::new(Instant::now()));
    state.done = true;
    true
}

/// Whether the window under `point` belongs to this process (the app's main window, a floating panel, a dialog)
pub(super) fn over_own_window(point: POINT) -> bool {
    HWND::WindowFromPoint(point).is_some_and(|window| window.GetWindowThreadProcessId().1 == winsafe::GetCurrentProcessId())
}

/// The colour of the one desktop pixel at `point` (physical pixels across every monitor: the app is per-monitor DPI aware)
pub(super) fn screen_pixel(point: POINT) -> Option<[u8; 3]> {
    let monitor = xcap::Monitor::from_point(point.x, point.y).ok()?;
    let x = u32::try_from(point.x.checked_sub(monitor.x().ok()?)?).ok()?;
    let y = u32::try_from(point.y.checked_sub(monitor.y().ok()?)?).ok()?;
    let pixel = monitor.capture_region(x, y, 1, 1).ok()?;
    let [red, green, blue, _] = pixel.get_pixel_checked(0, 0)?.0;
    Some([red, green, blue])
}
