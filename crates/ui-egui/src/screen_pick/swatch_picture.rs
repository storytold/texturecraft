//! The eyedropper pointer's colour disc, drawn beside the pipette's tip by `CursorImage::eyedropper_with_swatch`: a disc of the
//! colour under the tip in a crisp white outline and a faint dark outer ring, so it reads on white and on black alike; one
//! picture, so the disc moves with the pointer itself and never lags or flickers

use super::CursorImage;
use crate::color_picker::component_byte;

/// The pointer cell the disc's sizes are given for (a system pointer at 100 %), in pixels
const NOMINAL_CELL_PX: f32 = 32.0;
/// The coloured disc's radius at the nominal cell (the disc with its outline is 40 pixels across)
const FILL_RADIUS_PX: f32 = 18.0;
/// The white outline's width at the nominal cell
const OUTLINE_PX: f32 = 2.0;
/// The dark outer ring's width at the nominal cell
const RING_PX: f32 = 1.0;
/// The least the ring is wide in pixels, so a small pointer's disc still has its edge on white
const MIN_RING_PX: f32 = 1.0;
/// How dark the outer ring is: faint, only enough to edge the white outline on a white ground
const RING_ALPHA: f32 = 0.45;

/// Pixels to a nominal pointer pixel for a `cell`-pixel pointer
pub fn swatch_unit(cell: u32) -> f32 {
    CursorImage::clamped_size(cell) as f32 / NOMINAL_CELL_PX
}

/// How far the disc reaches from its centre with its outline and ring, in pixels, `unit` pixels to a nominal pixel
pub fn swatch_reach(unit: f32) -> f32 {
    (FILL_RADIUS_PX + OUTLINE_PX + RING_PX) * unit
}

/// The pipette's pixel at `(x, y)` of its cell, clear outside it
pub(super) fn pipette_pixel(pipette: &CursorImage, x: f32, y: f32) -> [u8; 4] {
    if x < 0.0 || y < 0.0 {
        return [0; 4];
    }

    let (x, y) = (x as u32, y as u32);
    if x >= pipette.size || y >= pipette.size {
        return [0; 4];
    }

    let index = ((y * pipette.size + x) * 4) as usize;
    pipette.rgba.get(index..index + 4).and_then(|pixel| pixel.try_into().ok()).unwrap_or([0; 4])
}

/// The disc's straight-alpha pixel `distance` pixels from its centre: the colour, the white outline round it, the faint ring
pub(super) fn disc_pixel(distance: f32, unit: f32, color: [u8; 3]) -> [u8; 4] {
    let fill_radius = FILL_RADIUS_PX * unit;
    let outline_radius = fill_radius + OUTLINE_PX * unit;
    let ring_radius = outline_radius + (RING_PX * unit).max(MIN_RING_PX);

    // Each edge's coverage over the pixel's width, for smooth edges
    let coverage = |radius: f32| (radius - distance + 0.5).clamp(0.0, 1.0);

    let ring = [0, 0, 0, component_byte(RING_ALPHA * coverage(ring_radius))];
    let outline = over([u8::MAX, u8::MAX, u8::MAX, component_byte(coverage(outline_radius))], ring);
    over([color[0], color[1], color[2], component_byte(coverage(fill_radius))], outline)
}

/// `top` over `bottom`, both straight-alpha RGBA
pub(super) fn over(top: [u8; 4], bottom: [u8; 4]) -> [u8; 4] {
    let top_alpha = f32::from(top[3]) / 255.0;
    let bottom_alpha = f32::from(bottom[3]) / 255.0 * (1.0 - top_alpha);
    let alpha = top_alpha + bottom_alpha;
    if alpha <= 0.0 {
        return [0; 4];
    }

    let mix = |channel: usize| component_byte((f32::from(top[channel]) * top_alpha + f32::from(bottom[channel]) * bottom_alpha) / alpha / 255.0);
    [mix(0), mix(1), mix(2), component_byte(alpha)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen_pick::SwatchSide;
    use crate::screen_pick::cursor_image::MAX_PICTURE_PX;

    fn pixel(image: &CursorImage, x: u32, y: u32) -> [u8; 4] {
        pipette_pixel(image, x as f32, y as f32)
    }

    /// The disc's centre in `image` for `side`, from its hotspot
    fn disc_center(image: &CursorImage, side: SwatchSide, unit: f32) -> [u32; 2] {
        let [dx, dy] = side.offset();
        [(image.hotspot[0] as f32 + 0.5 + dx * unit) as u32, (image.hotspot[1] as f32 + 0.5 + dy * unit) as u32]
    }

    #[test]
    fn the_disc_shows_the_colour_in_a_white_outline_and_a_faint_dark_ring() {
        let color = [200, 40, 90];
        let image = CursorImage::eyedropper_with_swatch(32, color, SwatchSide::BelowRight);
        let [cx, cy] = disc_center(&image, SwatchSide::BelowRight, 1.0);

        assert_eq!(pixel(&image, cx, cy), [200, 40, 90, 255], "the colour fills the disc");
        assert_eq!(pixel(&image, cx + 19, cy), [255, 255, 255, 255], "a white outline round it");
        let ring = pixel(&image, cx + 21, cy);
        assert!(ring[0] < 64 && ring[3] > 30 && ring[3] < 128, "a faint dark ring outside the outline: {ring:?}");
        assert_eq!(pixel(&image, cx + 22, cy)[3], 0, "and nothing past it");
        let blended = pixel(&image, cx + 18, cy);
        assert!(blended[1] > 40 && blended[1] < 255 && blended[3] == 255, "the colour blends into the outline: {blended:?}");
    }

    #[test]
    fn the_disc_is_40_pixels_across_at_100_percent_and_scales_with_the_pointer() {
        for (cell, unit) in [(32, 1.0), (48, 1.5), (64, 2.0)] {
            let image = CursorImage::eyedropper_with_swatch(cell, [10, 200, 10], SwatchSide::BelowRight);
            let [cx, cy] = disc_center(&image, SwatchSide::BelowRight, unit);

            // The faint ring stays below this alpha; the outline and the colour reach it
            let across = (0..image.size).filter(|&x| pixel(&image, x, cy)[3] >= 160).count();

            let expected = 40.0 * unit;
            assert!((across as f32 - expected).abs() <= 1.0, "{cell}: {across} px across, want {expected}");
            assert_eq!(pixel(&image, cx, cy)[1], 200);
        }
    }

    #[test]
    fn the_disc_covers_neither_the_picked_pixel_nor_the_pipette_on_any_side() {
        for cell in [24, 32, 48, 64] {
            let plain = CursorImage::eyedropper(cell);
            let unit = swatch_unit(cell);
            for side in SwatchSide::ALL {
                let image = CursorImage::eyedropper_with_swatch(cell, [255, 0, 255], side);
                let shift = [image.hotspot[0] - plain.hotspot[0], image.hotspot[1] - plain.hotspot[1]];
                let [cx, cy] = disc_center(&image, side, unit);
                let reach = swatch_reach(unit);

                for y in 0..image.size {
                    for x in 0..image.size {
                        let in_disc = ((x as f32 + 0.5 - cx as f32).powi(2) + (y as f32 + 0.5 - cy as f32).powi(2)).sqrt() <= reach + 1.0;
                        let pipette_ink = x >= shift[0] && y >= shift[1] && plain.alpha(x - shift[0], y - shift[1]).is_some_and(|alpha| alpha > 0);
                        assert!(!(in_disc && pipette_ink), "{cell} {side:?}: the disc covers the pipette at ({x}, {y})");
                    }
                }
                assert_eq!(pixel(&image, image.hotspot[0], image.hotspot[1]), pixel(&plain, plain.hotspot[0], plain.hotspot[1]), "{cell} {side:?}");
            }
        }
    }

    #[test]
    fn a_huge_pointer_still_fits_the_largest_picture() {
        let image = CursorImage::eyedropper_with_swatch(256, [1, 2, 3], SwatchSide::AboveRight);

        assert!(image.size <= MAX_PICTURE_PX, "{}", image.size);
        assert!(image.alpha(image.hotspot[0], image.hotspot[1]).is_some_and(|alpha| alpha > 0), "the hotspot is still the tip");
    }
}
