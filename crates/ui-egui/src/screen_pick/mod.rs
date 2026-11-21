//! Eyedropper picks outside the app: while a colour bar's eyedropper waits, a left click anywhere on the desktop (another app, a
//! reference picture on another monitor) reads the one screen pixel under it at that moment. Nothing records the screen, and
//! clicks on the app's own windows keep picking from the model. Where the system allows neither, the pick stays on the model;
//! while a pick waits the pointer is the eyedropper, drawn over the app's window and, where the system allows, over every app

mod cursor_image;
mod cursor_texture;
#[cfg(windows)]
mod eyedropper_pointer;
#[cfg(windows)]
mod hook_state;
mod keeper_args;
#[cfg(windows)]
mod live_swatch;
mod painted_cursor;
mod pick_outcome;
mod pick_shared;
#[cfg(all(unix, not(target_os = "macos")))]
mod pixel_layout;
#[cfg(all(unix, not(target_os = "macos")))]
mod portal;
#[cfg(all(unix, not(target_os = "macos")))]
mod premultiplied_argb;
#[cfg(any(windows, test))]
mod release_wait;
#[cfg(all(unix, not(target_os = "macos")))]
mod shown_disc;
mod swatch_color;
mod swatch_picture;
mod swatch_side;
#[cfg(windows)]
mod windows_hook;
#[cfg(all(unix, not(target_os = "macos")))]
mod x11_grab;

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

pub use cursor_image::CursorImage;
#[cfg(windows)]
pub use eyedropper_pointer::keep_pointer;
pub use keeper_args::KEEPER_ARG;
pub use painted_cursor::paint as paint_cursor;
pub use pick_outcome::PickOutcome;
pub(crate) use pick_shared::PickShared;
pub(crate) use swatch_color::SwatchColor;
pub use swatch_picture::{swatch_reach, swatch_unit};
pub use swatch_side::SwatchSide;

use crate::TextureCraftApp;
use crate::color_picker::ColorPick;

/// A running eyedropper pick's wait for a click outside the app, for the colour bar `bar`
pub struct ScreenPick {
    bar: egui::Id,
    outcomes: Receiver<PickOutcome>,
    shared: Arc<PickShared>,
    /// Whether the app losing the focus ends the pick (not while the desktop's own picker has it)
    ends_on_focus_loss: bool,
}

impl ScreenPick {
    fn start(bar: egui::Id, ctx: &egui::Context) -> Self {
        let (sender, outcomes) = mpsc::channel();
        let shared = Arc::new(PickShared::default());

        let waiting = Arc::clone(&shared);
        let dpi = window_dpi(ctx);
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("eyedropper".into()).spawn(move || wait_for_click(&waiting, sender, ctx, dpi));
        if let Err(error) = spawned {
            log::warn!("the eyedropper picks on the model only: {error}");
        }

        ScreenPick { bar, outcomes, shared, ends_on_focus_loss: ends_on_focus_loss() }
    }

    /// A pick for `bar` whose click outside the app already ended as `outcome` (tests and agents stand in for the system)
    pub fn ended(bar: egui::Id, outcome: PickOutcome) -> Self {
        let (sender, outcomes) = mpsc::channel();
        let _ = sender.send(outcome);
        ScreenPick { bar, outcomes, shared: Arc::default(), ends_on_focus_loss: true }
    }

    /// A pick for `bar` still waiting for its click outside the app (tests stand in for the system)
    pub fn waiting(bar: egui::Id) -> Self {
        let (_, outcomes) = mpsc::channel();
        ScreenPick { bar, outcomes, shared: Arc::default(), ends_on_focus_loss: true }
    }

    /// Tells the waiting thread where the app's windows are and whether the pointer is over one of them
    fn share_windows(&self, ctx: &egui::Context) {
        let viewports = ctx.input(|input| input.raw.viewports.clone());
        let windows = viewports.values().filter_map(physical_rect).collect();
        self.shared.set_windows(windows);
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let pointer_inside = viewports.keys().any(|id| ctx.input_for(*id, |input| input.pointer.has_pointer()));
            self.shared.set_pointer_outside(!pointer_inside);
        }
    }
}

impl Drop for ScreenPick {
    fn drop(&mut self) {
        self.shared.stop();
    }
}

/// Starts or ends the wait for a click outside the app with the eyedropper pick, and hands a colour picked there to its bar
pub(crate) fn sync(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let Some(bar) = app.picking_color else {
        app.screen_pick = None;
        return;
    };

    if app.screen_pick.as_ref().is_none_or(|pick| pick.bar != bar) {
        // The last pick stops before the next starts, so its pointer goes back first
        drop(app.screen_pick.take());
        app.screen_pick = app.screen_picking.then(|| ScreenPick::start(bar, ctx));
    }

    let Some(pick) = app.screen_pick.as_ref() else { return };

    pick.share_windows(ctx);
    match pick.outcomes.try_recv() {
        Ok(PickOutcome::Picked(color)) => {
            ColorPick::deliver(ctx, bar, color.map(egui::ecolor::linear_f32_from_gamma_u8));
            app.picking_color = None;
            app.screen_pick = None;
        }
        Ok(PickOutcome::Cancelled) => {
            app.picking_color = None;
            app.screen_pick = None;
        }

        // The pick stays on the model; the finished wait is kept so it does not start again
        Ok(PickOutcome::Unavailable) | Err(_) => {
            // Another app came forward (Alt+Tab, the Start menu): the pick ends there and the pointer is the user's again
            if pick.ends_on_focus_loss && ctx.input(|input| input.viewport().focused) == Some(false) {
                app.picking_color = None;
                app.screen_pick = None;
            }
        }
    }
}

/// Whether the app losing the focus ends a pick here (not on Wayland, where the desktop's colour picker takes the focus)
fn ends_on_focus_loss() -> bool {
    cfg!(windows) || (cfg!(all(unix, not(target_os = "macos"))) && std::env::var_os("WAYLAND_DISPLAY").is_none())
}

/// The highest window DPI read, far past any display, so a broken scale cannot size a huge pointer
const MAX_WINDOW_DPI: f32 = 4096.0;

/// The app window's dots per inch, which size the eyedropper as the system's pointers are sized there
fn window_dpi(ctx: &egui::Context) -> u32 {
    let scale = ctx.input(|input| input.viewport().native_pixels_per_point).filter(|scale| scale.is_finite() && *scale > 0.0);
    (scale.unwrap_or(1.0) * keeper_args::BASE_DPI as f32).round().clamp(1.0, MAX_WINDOW_DPI) as u32
}

/// A window's outer rectangle as `[left, top, right, bottom]` in physical screen pixels, where the system reports one
fn physical_rect(viewport: &egui::ViewportInfo) -> Option<[i32; 4]> {
    let rect = viewport.outer_rect?;
    let scale = viewport.native_pixels_per_point?;
    let edges = [rect.left(), rect.top(), rect.right(), rect.bottom()].map(|edge| edge * scale);
    if edges.iter().any(|edge| !edge.is_finite()) {
        return None;
    }

    Some(edges.map(|edge| edge.round() as i32))
}

/// The system's way to read a click outside the app, on the pick's own thread
fn wait_for_click(shared: &Arc<PickShared>, outcomes: Sender<PickOutcome>, ctx: egui::Context, dpi: u32) {
    #[cfg(windows)]
    windows_hook::run(shared, outcomes, ctx, dpi);
    #[cfg(not(windows))]
    let _ = dpi;

    #[cfg(all(unix, not(target_os = "macos")))]
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        portal::run(shared, &outcomes, &ctx);
    } else if x11_grab::run(shared, &outcomes, &ctx).is_none() {
        let _ = outcomes.send(PickOutcome::Unavailable);
    }

    // macOS: reading another app's pixels needs the screen-recording permission; the pick stays on the model
    #[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
    {
        let _ = (shared, ctx);
        let _ = outcomes.send(PickOutcome::Unavailable);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_reports_its_rectangle_in_physical_pixels() {
        let viewport = egui::ViewportInfo {
            outer_rect: Some(egui::Rect::from_min_size(egui::pos2(-960.0, 100.0), egui::vec2(800.0, 600.0))),
            native_pixels_per_point: Some(1.5),
            ..Default::default()
        };

        assert_eq!(physical_rect(&viewport), Some([-1440, 150, -240, 1050]));
        assert_eq!(physical_rect(&egui::ViewportInfo::default()), None, "no position (Wayland): no rectangle");
    }

    #[test]
    fn a_click_is_the_apps_own_only_on_one_of_its_windows() {
        let windows = [[0, 0, 100, 50], [-300, 0, -100, 200]];

        assert!(pick_shared::over_windows(&windows, 0, 0));
        assert!(pick_shared::over_windows(&windows, -101, 199));
        assert!(!pick_shared::over_windows(&windows, 100, 10), "the right edge is outside");
        assert!(!pick_shared::over_windows(&windows, 500, 500));
    }
}
