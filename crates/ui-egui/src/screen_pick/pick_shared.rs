use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(all(unix, not(target_os = "macos")))]
use std::task::Waker;

/// What the frame loop tells the thread that waits for a click outside the app
#[derive(Default)]
pub(crate) struct PickShared {
    /// The pick ended in the app (Escape, a click on the model, its eyedropper again)
    stop: AtomicBool,
    /// The app's windows as `[left, top, right, bottom]` in physical screen pixels, where clicks stay the app's own
    windows: Mutex<Vec<[i32; 4]>>,
    /// The pointer is over none of the app's windows
    #[cfg(all(unix, not(target_os = "macos")))]
    pointer_outside: AtomicBool,
    #[cfg(all(unix, not(target_os = "macos")))]
    portal_waker: Mutex<Option<Waker>>,
}

impl PickShared {
    pub(crate) fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        #[cfg(all(unix, not(target_os = "macos")))]
        if let Some(waker) = self.take_portal_waker() {
            waker.wake();
        }
    }

    #[cfg(any(windows, all(unix, not(target_os = "macos"))))]
    pub(crate) fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    pub(crate) fn set_windows(&self, windows: Vec<[i32; 4]>) {
        if let Ok(mut known) = self.windows.lock() {
            *known = windows;
        }
    }

    /// The app's windows, or none while they are unknown (a system without global window positions)
    #[cfg(any(windows, all(unix, not(target_os = "macos")), test))]
    pub(crate) fn windows(&self) -> Vec<[i32; 4]> {
        self.windows.lock().map(|known| known.clone()).unwrap_or_default()
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub(crate) fn set_pointer_outside(&self, outside: bool) {
        self.pointer_outside.store(outside, Ordering::Relaxed);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub(crate) fn pointer_outside(&self) -> bool {
        self.pointer_outside.load(Ordering::Relaxed)
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub(super) fn watch_portal_stop(&self, waker: &Waker) -> bool {
        let mut waiting = self.portal_waker.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.stopped() {
            return true;
        }

        if waiting.as_ref().is_none_or(|registered| !registered.will_wake(waker)) {
            *waiting = Some(waker.clone());
        }

        false
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub(super) fn take_portal_waker(&self) -> Option<Waker> {
        self.portal_waker.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take()
    }
}

/// Whether the screen pixel `(x, y)` lies on one of `windows`
#[cfg(any(windows, all(unix, not(target_os = "macos")), test))]
pub(crate) fn over_windows(windows: &[[i32; 4]], x: i32, y: i32) -> bool {
    windows.iter().any(|&[left, top, right, bottom]| x >= left && x < right && y >= top && y < bottom)
}
