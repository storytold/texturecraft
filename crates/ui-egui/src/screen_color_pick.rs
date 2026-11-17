//! An eyedropper button: after it is pressed, the next click anywhere in the window picks the color shown there

use std::sync::{Arc, Mutex};

use egui::{Id, Pos2, Ui};
use egui_phosphor::regular as icon;

use crate::widgets::icon_button;

/// Where the pick in progress is kept between frames
const PICK_ID: &str = "screen_color_pick";

/// Where a screenshot callback leaves the sampled sRGB color for the button that asked for it
type SampledSlot = Arc<Mutex<Option<(Id, [u8; 3])>>>;

/// The pick in progress: which button started it, and the color its screenshot sampled once that arrives
#[derive(Clone, Default)]
struct ScreenColorPick {
    target: Option<Id>,
    /// Filled by the screenshot callback with the target and the sRGB color under the click
    sampled: SampledSlot,
}

/// An eyedropper button for the linear RGB color `linear`, `size` points square; a picked color lands in `linear` a frame or two after the click
pub fn eyedropper(ui: &mut Ui, id: Id, linear: &mut [f32; 3], tooltip: &str, size: f32) {
    let key = Id::unique(PICK_ID);
    let mut pick: ScreenColorPick = ui.data(|data| data.get_temp(key)).unwrap_or_default();
    let picking = pick.target == Some(id);

    let button = icon_button(ui, icon::EYEDROPPER, tooltip, picking, size);
    if button.clicked() {
        pick.target = if picking { None } else { Some(id) };
    } else if picking {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        let (clicked, escaped, pointer) =
            ui.input(|input| (input.pointer.primary_clicked(), input.key_pressed(egui::Key::Escape), input.pointer.interact_pos()));
        if escaped {
            pick.target = None;
        } else if clicked && let Some(point) = pointer {
            request_sample(ui.ctx(), id, point, pick.sampled.clone());
            pick.target = None;
        }
    }

    let landed = pick.sampled.lock().ok().and_then(|mut sampled| sampled.take_if(|(target, _)| *target == id));
    if let Some((_, srgb)) = landed {
        *linear = srgb.map(|channel| texturecraft_engine::texel::srgb_to_linear(f32::from(channel) / 255.0));
    }

    ui.data_mut(|data| data.insert_temp(key, pick));
}

/// Screenshot the next frame and put the color at `point` (in points) into `sampled` for `target`
fn request_sample(ctx: &egui::Context, target: Id, point: Pos2, sampled: SampledSlot) {
    let pixels_per_point = ctx.pixels_per_point();
    let repaint = ctx.clone();
    let callback = egui::ScreenshotCallback::new(move |image| {
        if let (Some(color), Ok(mut slot)) = (pixel_at(&image, point, pixels_per_point), sampled.lock()) {
            *slot = Some((target, color));
        }
        repaint.request_repaint();
    });
    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(callback));
    ctx.request_repaint();
}

/// The sRGB color of `image` under `point` (in points at `pixels_per_point`); `None` outside it
fn pixel_at(image: &egui::ColorImage, point: Pos2, pixels_per_point: f32) -> Option<[u8; 3]> {
    let (x, y) = ((point.x * pixels_per_point).floor(), (point.y * pixels_per_point).floor());
    let [width, height] = image.size;
    if !(x >= 0.0 && y >= 0.0 && (x as usize) < width && (y as usize) < height) {
        return None;
    }

    let color = image.pixels.get(y as usize * width + x as usize)?;
    Some([color.r(), color.g(), color.b()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pixel_under_the_point_is_sampled_at_the_display_scale() {
        let mut image = egui::ColorImage::filled([4, 2], egui::Color32::BLACK);
        image.pixels[4 + 3] = egui::Color32::from_rgb(10, 20, 30);

        assert_eq!(pixel_at(&image, egui::pos2(1.6, 0.6), 2.0), Some([10, 20, 30]));
        assert_eq!(pixel_at(&image, egui::pos2(0.2, 0.2), 2.0), Some([0, 0, 0]));
        for outside in [egui::pos2(-0.1, 0.0), egui::pos2(2.0, 0.0), egui::pos2(0.0, 1.0), egui::pos2(f32::NAN, 0.0)] {
            assert_eq!(pixel_at(&image, outside, 2.0), None, "{outside:?}");
        }
    }
}
