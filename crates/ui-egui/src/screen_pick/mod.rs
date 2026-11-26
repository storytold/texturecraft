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
#[path = "screen_pick.rs"]
mod pick;
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
use std::sync::mpsc::Sender;

pub use cursor_image::CursorImage;
#[cfg(windows)]
pub use eyedropper_pointer::keep_pointer;
pub use keeper_args::KEEPER_ARG;
pub use painted_cursor::paint as paint_cursor;
pub use pick::ScreenPick;
pub use pick_outcome::PickOutcome;
pub(crate) use pick_shared::PickShared;
pub(crate) use swatch_color::SwatchColor;
pub use swatch_picture::{swatch_reach, swatch_unit};
pub use swatch_side::SwatchSide;

use crate::TextureCraftApp;
use crate::color_picker::ColorPick;

/// Starts or ends the wait for a click outside the app with the eyedropper pick, and hands a colour picked there to its bar
pub(crate) fn sync(app: &mut TextureCraftApp, ctx: &egui::Context) {
    if app.eyedropper_project_id != app.session.project_id {
        if let Some(bar) = app.picking_color.take() {
            ColorPick::delivered(ctx, bar);
        }
        app.screen_pick = None;
        app.eyedropper_project_id = app.session.project_id;
    }

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
fn ends_on_focus_loss(ctx: &egui::Context) -> bool {
    cfg!(windows) || (cfg!(all(unix, not(target_os = "macos"))) && !crate::uses_wayland(ctx))
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
    if crate::uses_wayland(&ctx) {
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
