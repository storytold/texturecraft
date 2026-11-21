//! Windows: the eyedropper's colour disc over other apps, part of the system pointer's picture, so it moves with the pointer
//! itself; each refresh reads the one desktop pixel under the tip and swaps in a new picture only when its colour or side changed

use std::time::Duration;

use winsafe::guard::CloseHandleGuard;
use winsafe::prelude::Handle;
use winsafe::{HEVENT, HMONITOR, POINT, co};

use super::eyedropper_pointer::swap_in;
use super::windows_hook::{over_own_window, screen_pixel, waiting};
use super::{CursorImage, PickShared, SwatchSide, pick_shared::over_windows, swatch_reach, swatch_unit};

/// How often the disc reads the colour under the tip (one display frame at 60 Hz)
const REFRESH: Duration = Duration::from_millis(16);

/// The disc the system pointer shows now
pub(super) struct LiveSwatch {
    /// Set once the keeper has kept the user's pointers: no picture is swapped in before, or theirs would be lost
    keeper_ready: CloseHandleGuard<HEVENT>,
    /// The system's pointer cell, in pixels
    cell: u32,
    shown: Option<([u8; 3], SwatchSide)>,
}

impl LiveSwatch {
    /// Keeps the disc on the colour under the tip until pick `generation` ends (`shown_event` names the keeper's ready event)
    pub(super) fn follow(shown_event: &str, cell: u32, shared: &PickShared, generation: u64) {
        let Ok(keeper_ready) = HEVENT::NULL.OpenEvent(co::EVENT_RIGHTS::SYNCHRONIZE, false, shown_event) else { return };

        let mut swatch = LiveSwatch { keeper_ready, cell, shown: None };
        while waiting(shared, generation) {
            swatch.refresh(shared);
            std::thread::sleep(REFRESH);
        }
    }

    /// Shows the colour under the tip in the pointer's disc, while the pointer is off the app (the app draws its own there)
    fn refresh(&mut self, shared: &PickShared) {
        let Ok(point) = winsafe::GetCursorPos() else { return };

        if over_own_window(point) || over_windows(&shared.windows(), point.x, point.y) {
            return;
        }

        if !matches!(self.keeper_ready.WaitForSingleObject(Some(0)), Ok(co::WAIT::OBJECT_0)) {
            return;
        }

        let Some(color) = screen_pixel(point) else { return };

        let look = (color, side(point, self.cell));
        if self.shown != Some(look) && swap_in(&CursorImage::eyedropper_with_swatch(self.cell, look.0, look.1)) {
            self.shown = Some(look);
        }
    }
}

/// The side of the tip whose disc stays on the monitor the pointer is on
fn side(point: POINT, cell: u32) -> SwatchSide {
    let unit = swatch_unit(cell);
    let tip = [point.x as f32 + 0.5, point.y as f32 + 0.5];
    let Ok(info) = HMONITOR::MonitorFromPoint(point, co::MONITOR::DEFAULTTONEAREST).GetMonitorInfo() else { return SwatchSide::BelowRight };

    let screen = info.rcMonitor;
    let bounds = [screen.left, screen.top, screen.right, screen.bottom].map(|edge| edge as f32);
    SwatchSide::fitting(tip, unit, swatch_reach(unit), bounds)
}
