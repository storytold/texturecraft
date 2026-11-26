//! Windows: while an eyedropper pick waits for a click outside the app, the system's pointers are the eyedropper over every app
//! and monitor. A keeper process (this program started with `KEEPER_ARG`) swaps them and puts the user's own back when the pick
//! ends, and also when the app goes away for any reason: it watches the app's process, so the system pointer is never left
//! changed. The keeper is started with the system's start-up feedback off: Windows otherwise shows its "app starting" pointer
//! (an arrow with a busy circle) over every app for up to seven seconds after a program starts, until the program first calls
//! `GetMessage`, which a keeper never does, and that pointer showed instead of the eyedropper. Once the keeper has kept the
//! user's pointers and shown the eyedropper, the app swaps in the eyedropper with its live colour disc; the keeper still puts
//! the user's own back

use std::sync::atomic::{AtomicU64, Ordering};

use winsafe::guard::{CloseHandleGuard, CloseHandlePiGuard, DestroyCursorGuard};
use winsafe::prelude::Handle;
use winsafe::{HEVENT, HINSTANCE, HPROCESS, IdOcrStr, SIZE, STARTUPINFO, co};

use super::CursorImage;
use super::keeper_args::KeeperArgs;

/// How long the app waits for the keeper to put the user's pointers back, in milliseconds
const RESTORE_WAIT_MS: u32 = 2_000;
/// How often the keeper looks whether the app still runs, in milliseconds
const WATCH_MS: u32 = 50;

/// The system pointers other apps show; the animated busy pointers stay as they are (a copy would keep one frame)
const POINTERS: [co::OCR; 11] = [
    co::OCR::NORMAL,
    co::OCR::IBEAM,
    co::OCR::CROSS,
    co::OCR::UP,
    co::OCR::SIZENWSE,
    co::OCR::SIZENESW,
    co::OCR::SIZEWE,
    co::OCR::SIZENS,
    co::OCR::SIZEALL,
    co::OCR::NO,
    co::OCR::HAND,
];

/// The keeper's exit code for a command line it can't read
const EXIT_BAD_ARGS: i32 = 2;
/// The keeper's exit code when the app's process, its event or the eyedropper can't be had
const EXIT_UNAVAILABLE: i32 = 1;

/// Each pick's own stop event
static NEXT_PICK: AtomicU64 = AtomicU64::new(0);

/// The eyedropper as the system pointer, until this is dropped
pub(super) struct EyedropperPointer {
    keeper: CloseHandlePiGuard,
    stop: CloseHandleGuard<HEVENT>,
    /// The event the keeper sets once it has kept the user's pointers, held so it lives as long as the pick
    _shown: CloseHandleGuard<HEVENT>,
    /// That event's name
    pub(super) shown_event: String,
    /// The system's pointer cell at the app window's density, in pixels
    pub(super) cell: u32,
}

impl EyedropperPointer {
    /// Starts the keeper for a window at `dpi` dots per inch; `None` leaves the system pointer as it is
    pub(super) fn show(dpi: u32) -> Option<Self> {
        let program = std::env::current_exe().ok()?;
        let parent = std::process::id();

        // The pick's stop event, and the event the keeper sets once it has kept the user's pointers
        let stop_name = format!("Local\\texturecraft-eyedropper-{parent}-{}", NEXT_PICK.fetch_add(1, Ordering::Relaxed));
        let stop = HEVENT::CreateEvent(None, true, false, Some(&stop_name)).ok()?;
        let args = KeeperArgs { parent, stop: stop_name, dpi };
        let shown_event = args.shown_event();
        let shown = HEVENT::CreateEvent(None, true, false, Some(&shown_event)).ok()?;

        let cell = args.pointer_px(winsafe::GetSystemMetricsForDpi(co::SM::CXCURSOR, dpi).ok());
        let command_line = args.command_line(program.to_str()?);

        let mut startup = STARTUPINFO::default();

        // No "app starting" pointer for the keeper: it shows no window and never asks for input
        startup.dwFlags = co::STARTF::FORCEOFFFEEDBACK;
        let keeper = winsafe::CreateProcess(None, Some(&command_line), None, None, false, co::CREATE::NO_WINDOW, &[], None, &mut startup);
        Some(EyedropperPointer { keeper: keeper.ok()?, stop, _shown: shown, shown_event, cell })
    }
}

impl Drop for EyedropperPointer {
    fn drop(&mut self) {
        // A timed-out keeper still holds the system-cursor lock until restoration finishes
        let _ = self.stop.SetEvent();
        let _ = self.keeper.hProcess.WaitForSingleObject(Some(RESTORE_WAIT_MS));
    }
}

/// Shows `image` as every swapped system pointer (only once the keeper has kept the user's own); whether it could
pub(super) fn swap_in(image: &CursorImage) -> bool {
    let Some(picture) = load(image) else { return false };

    swap_all(&picture);
    true
}

/// The keeper process's work and exit code: the eyedropper as the system pointer until the pick ends or the app is gone
pub fn keep_pointer() -> i32 {
    let Some(keeper) = KeeperArgs::parse_os(std::env::args_os().skip(2)) else { return EXIT_BAD_ARGS };
    let Ok(app) = HPROCESS::OpenProcess(co::PROCESS::SYNCHRONIZE, false, keeper.parent) else { return EXIT_UNAVAILABLE };
    let Ok(stop) = HEVENT::NULL.OpenEvent(co::EVENT_RIGHTS::SYNCHRONIZE, false, &keeper.stop) else { return EXIT_UNAVAILABLE };
    if ended(stop.WaitForSingleObject(Some(0))) {
        return 0;
    }

    // The keeper owns this lock through restoration, even if the app times out waiting for it
    let lock_path = std::env::temp_dir().join("texturecraft-system-cursors.lock");
    let Ok(owner) = std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(lock_path) else { return EXIT_UNAVAILABLE };
    if owner.try_lock().is_err() {
        return EXIT_UNAVAILABLE;
    }

    let size = keeper.pointer_px(winsafe::GetSystemMetricsForDpi(co::SM::CXCURSOR, keeper.dpi).ok());
    let Some(eyedropper) = load(&CursorImage::eyedropper(size)) else { return EXIT_UNAVAILABLE };
    let originals: Vec<_> = POINTERS.iter().filter_map(|&pointer| Some((pointer, current(pointer)?))).collect();

    // A pointer whose copy failed could never be put back: the user's pointers then stay as they are
    if !every_pointer_kept(originals.len()) {
        return EXIT_UNAVAILABLE;
    }

    swap_all(&eyedropper);

    // The app may change the picture from now on: the user's own pointers are kept
    if let Ok(shown) = HEVENT::NULL.OpenEvent(co::EVENT_RIGHTS::MODIFY_STATE, false, &keeper.shown_event()) {
        let _ = shown.SetEvent();
    }

    // Ends when the app sets the stop event (the pick ended) or its process ends (closed, killed, crashed)
    while !ended(stop.WaitForSingleObject(Some(0))) && !ended(app.WaitForSingleObject(Some(WATCH_MS))) {}

    // The user's own pointers back; the system owns each one it took
    for (pointer, mut original) in originals {
        if original.SetSystemCursor(pointer).is_ok() {
            let _ = original.leak();
        }
    }

    0
}

/// Whether a copy of each pointer swapped was kept (`kept` copies), so each can be put back
fn every_pointer_kept(kept: usize) -> bool {
    kept == POINTERS.len()
}

/// `picture` as each of the system pointers other apps show
fn swap_all(picture: &DestroyCursorGuard) {
    for pointer in POINTERS {
        if let Ok(mut copy) = picture.CopyCursor()
            && copy.SetSystemCursor(pointer).is_ok()
        {
            // The system owns the copy now
            let _ = copy.leak();
        }
    }
}

/// Whether a wait says its object is signalled (or can't be waited on, which ends the keeper just as well)
fn ended(wait: winsafe::SysResult<co::WAIT>) -> bool {
    !matches!(wait, Ok(co::WAIT::TIMEOUT))
}

/// A copy of the system pointer `pointer` as it is now
fn current(pointer: co::OCR) -> Option<DestroyCursorGuard> {
    let mut shared = HINSTANCE::NULL.LoadImageCursor(IdOcrStr::Ocr(pointer), SIZE::default(), co::LR::SHARED | co::LR::DEFAULTSIZE).ok()?;
    let copy = shared.CopyCursor().ok();

    // A shared pointer belongs to the system and is never destroyed
    let _ = shared.leak();
    copy
}

/// `image` as a Windows pointer (loaded from a `.cur` file, the one safe way to make a coloured pointer)
fn load(image: &CursorImage) -> Option<DestroyCursorGuard> {
    let file = std::env::temp_dir().join(format!("texturecraft-eyedropper-{}.cur", std::process::id()));
    std::fs::write(&file, image.cur_file()).ok()?;
    let loaded = HINSTANCE::NULL.LoadImageCursor(IdOcrStr::from_str(&file.to_string_lossy()), SIZE::default(), co::LR::LOADFROMFILE);
    let _ = std::fs::remove_file(&file);
    loaded.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pointers_are_swapped_only_when_each_one_can_be_put_back() {
        assert!(every_pointer_kept(POINTERS.len()), "every pointer kept: the eyedropper shows");
        assert!(!every_pointer_kept(POINTERS.len() - 1), "one copy failed: the user's pointers stay as they are");
    }
}
