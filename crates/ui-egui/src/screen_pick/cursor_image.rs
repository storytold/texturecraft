//! The eyedropper pointer an armed pick shows everywhere: the panels' pipette icon (tip at the bottom left, bulb at the top
//! right) in white with a dark outline so it reads on any colour, its glass tube's inside dark like the glyph's, the same picture
//! in the app and, where the system lets the app set the pointer, over other apps; its hotspot is the tip's last pixel, so the
//! pixel under the tip is the one picked. The pointer is a system pointer's cell (32 pixels at 100 %) with the pipette drawn
//! in its bottom-left part, about as tall as the system's arrow in its cell, so it reads at the size every other pointer has

use super::swatch_picture::{disc_pixel, over, pipette_pixel};
use super::{SwatchSide, swatch_reach, swatch_unit};
use crate::drawn_icon::pipette;

/// The outline's width in pixels at the smallest pointer; larger pointers scale it
const OUTLINE_PX: f32 = 1.0;
/// The pointer size the outline width is given for
const OUTLINE_SIZE: f32 = 24.0;
/// The coverage from which a pixel counts as ink for the hotspot
const INK_ALPHA: u8 = 128;
/// A `.cur` file's header (6 bytes) and its one picture's entry (16 bytes)
#[cfg(any(windows, test))]
const CUR_HEADER_BYTES: u32 = 22;
/// The `.cur` file type in its header (an `.ico` file says 1)
#[cfg(any(windows, test))]
const CUR_TYPE: u16 = 2;
/// A `BITMAPINFOHEADER`'s size
#[cfg(any(windows, test))]
const BITMAP_HEADER_BYTES: u32 = 40;
/// The `.cur` picture's colour depth: 8-bit blue, green, red and alpha
#[cfg(any(windows, test))]
const CUR_BITS_PER_PIXEL: u16 = 32;
/// A bitmap's mask rows are padded to whole 32-bit words
#[cfg(any(windows, test))]
const MASK_WORD_BITS: u32 = 32;
/// A mask byte's first pixel is its high bit
#[cfg(any(windows, test))]
const MASK_FIRST_BIT: u8 = 0x80;
/// The part of the pointer's cell the pipette's square takes (the system's arrow is about 20 pixels tall in its 32-pixel cell)
const GLYPH_SHARE: f32 = 0.625;
/// The smallest and largest pointer drawn, in pixels
pub(super) const SIZES: std::ops::RangeInclusive<u32> = 8..=256;
/// The largest picture with a colour disc made, in pixels (the largest pointer the systems take)
pub(super) const MAX_PICTURE_PX: u32 = 256;

/// A square pointer picture in straight-alpha RGBA rows, top row first, with the pixel that is the pointer's position
#[derive(Clone, Debug, PartialEq)]
pub struct CursorImage {
    pub size: u32,
    pub rgba: Vec<u8>,
    pub hotspot: [u32; 2],
}

impl CursorImage {
    /// The eyedropper in a pointer cell `size` pixels square (clamped to a sane range), the pipette in its bottom-left part
    pub fn eyedropper(size: u32) -> Self {
        let size = CursorImage::clamped_size(size);
        let scale = (size as f32 * GLYPH_SHARE).round();
        let top = size as f32 - scale;
        let outline = OUTLINE_PX * (scale / OUTLINE_SIZE).max(1.0);
        let mut rgba = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let point = [(x as f32 + 0.5) / scale, (y as f32 + 0.5 - top) / scale];
                let inside = pipette::silhouette(point) * scale;
                let fill = (0.5 - inside).clamp(0.0, 1.0).min((0.5 - pipette::glyph(point) * scale).clamp(0.0, 1.0));
                let alpha = (0.5 - (inside - outline)).clamp(0.0, 1.0);
                let grey = (fill * 255.0).round() as u8;
                rgba.extend_from_slice(&[grey, grey, grey, (alpha * 255.0).round() as u8]);
            }
        }
        let hotspot = tip(&rgba, size);
        CursorImage { size, rgba, hotspot }
    }

    /// The eyedropper in a `cell`-pixel pointer with a disc of `color` (8-bit sRGB) on `side` of its tip; its hotspot is the tip
    pub fn eyedropper_with_swatch(cell: u32, color: [u8; 3], side: SwatchSide) -> Self {
        let pipette = CursorImage::eyedropper(cell);
        let unit = swatch_unit(cell);
        let reach = swatch_reach(unit);
        let [tip_x, tip_y] = pipette.hotspot.map(|edge| edge as f32 + 0.5);
        let [offset_x, offset_y] = side.offset();
        let center = [tip_x + offset_x * unit, tip_y + offset_y * unit];

        // The picture spans the pipette's cell and the disc, as a square
        let left = (center[0] - reach).floor().min(0.0);
        let top = (center[1] - reach).floor().min(0.0);
        let right = (center[0] + reach).ceil().max(pipette.size as f32);
        let bottom = (center[1] + reach).ceil().max(pipette.size as f32);
        let side_px = (right - left).max(bottom - top) as u32;
        if side_px > MAX_PICTURE_PX && cell > 1 {
            let smaller = (cell as f32 * MAX_PICTURE_PX as f32 / side_px as f32).floor() as u32;
            return CursorImage::eyedropper_with_swatch(smaller.min(cell - 1), color, side);
        }

        let mut rgba = Vec::with_capacity((side_px * side_px * 4) as usize);
        for y in 0..side_px {
            for x in 0..side_px {
                let [cell_x, cell_y] = [x as f32 + left, y as f32 + top];
                let under = pipette_pixel(&pipette, cell_x, cell_y);
                let distance = ((cell_x + 0.5 - center[0]).powi(2) + (cell_y + 0.5 - center[1]).powi(2)).sqrt();
                rgba.extend_from_slice(&over(disc_pixel(distance, unit, color), under));
            }
        }

        let hotspot = [pipette.hotspot[0] + (-left) as u32, pipette.hotspot[1] + (-top) as u32];
        CursorImage { size: side_px, rgba, hotspot }
    }

    /// A pointer size kept to the range drawn
    pub fn clamped_size(size: u32) -> u32 {
        size.clamp(*SIZES.start(), *SIZES.end())
    }

    /// The picture as a Windows `.cur` file: 32-bit colour with alpha, its hotspot in the file's header
    #[cfg(any(windows, test))]
    pub fn cur_file(&self) -> Vec<u8> {
        let side = self.size;
        let mask_row = side.div_ceil(MASK_WORD_BITS) * (MASK_WORD_BITS / u8::BITS);
        let pixel_bytes = side * side * 4;
        let image_bytes = BITMAP_HEADER_BYTES + pixel_bytes + mask_row * side;
        let mut file = Vec::with_capacity((CUR_HEADER_BYTES + image_bytes) as usize);

        // The file's header: one cursor picture, a byte size of 0 standing for 256
        for word in [0_u16, CUR_TYPE, 1] {
            file.extend_from_slice(&word.to_le_bytes());
        }
        let byte_side = u8::try_from(side).unwrap_or(0);
        file.extend_from_slice(&[byte_side, byte_side, 0, 0]);
        for word in self.hotspot.map(|edge| u16::try_from(edge).unwrap_or(0)) {
            file.extend_from_slice(&word.to_le_bytes());
        }
        file.extend_from_slice(&image_bytes.to_le_bytes());
        file.extend_from_slice(&CUR_HEADER_BYTES.to_le_bytes());

        // The bitmap's header: twice the height, the colour picture over its mask
        let side_signed = i32::try_from(side).unwrap_or(0);
        for word in [BITMAP_HEADER_BYTES as i32, side_signed, side_signed * 2] {
            file.extend_from_slice(&word.to_le_bytes());
        }
        file.extend_from_slice(&1_u16.to_le_bytes());
        file.extend_from_slice(&CUR_BITS_PER_PIXEL.to_le_bytes());
        for word in [0, pixel_bytes + mask_row * side, 0, 0, 0, 0] {
            file.extend_from_slice(&word.to_le_bytes());
        }

        // Rows bottom up: blue, green, red, alpha; then the mask, set where the picture is clear
        for row in self.rgba.chunks_exact(side as usize * 4).rev() {
            for &[red, green, blue, alpha] in row.as_chunks::<4>().0 {
                file.extend_from_slice(&[blue, green, red, alpha]);
            }
        }
        for row in self.rgba.chunks_exact(side as usize * 4).rev() {
            let mut bits = vec![0_u8; mask_row as usize];
            for (x, &[.., alpha]) in row.as_chunks::<4>().0.iter().enumerate() {
                if alpha == 0
                    && let Some(byte) = bits.get_mut(x / u8::BITS as usize)
                {
                    *byte |= MASK_FIRST_BIT >> (x % u8::BITS as usize);
                }
            }
            file.extend_from_slice(&bits);
        }
        file
    }

    /// The alpha of the pixel at `(x, y)`, none outside the picture
    pub fn alpha(&self, x: u32, y: u32) -> Option<u8> {
        let index = (y.checked_mul(self.size)?.checked_add(x)? as usize).checked_mul(4)?.checked_add(3)?;
        (x < self.size).then(|| self.rgba.get(index).copied()).flatten()
    }
}

/// The ink pixel furthest toward the bottom-left corner: the pipette's tip
fn tip(rgba: &[u8], size: u32) -> [u32; 2] {
    let ink = rgba.as_chunks::<4>().0.iter().enumerate().filter(|(_, [.., alpha])| *alpha >= INK_ALPHA);
    let corner = ink.map(|(index, _)| [index as u32 % size, index as u32 / size]).max_by_key(|&[x, y]| (i64::from(y) - i64::from(x), y));
    corner.unwrap_or([0, size.saturating_sub(1)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hotspot_is_the_tips_last_ink_pixel_at_every_size() {
        for size in [16, 24, 32, 48, 64] {
            let image = CursorImage::eyedropper(size);
            let [hx, hy] = image.hotspot;

            assert!(image.alpha(hx, hy).is_some_and(|alpha| alpha >= INK_ALPHA), "{size}: the hotspot pixel is ink");
            assert!(hx < size / 4 && hy >= size * 3 / 4, "{size}: the tip is at the bottom left: {:?}", image.hotspot);

            for y in 0..size {
                for x in 0..size {
                    let ink = image.alpha(x, y).is_some_and(|alpha| alpha >= INK_ALPHA);
                    let further = i64::from(y) - i64::from(x) > i64::from(hy) - i64::from(hx);
                    assert!(!(ink && further), "{size}: ink at ({x}, {y}) lies beyond the tip");
                }
            }
        }
    }

    #[test]
    fn the_pipette_is_white_inside_a_dark_outline_on_a_clear_ground() {
        let image = CursorImage::eyedropper(32);
        let pixel = |x: u32, y: u32| image.rgba.get(((y * 32 + x) * 4) as usize..((y * 32 + x) * 4 + 4) as usize).map(<[u8]>::to_vec);

        let [bulb_x, bulb_y] = pipette::square(0.85, 0.0).map(|edge| (edge * 20.0) as u32);
        assert_eq!(pixel(bulb_x, bulb_y + 12), Some(vec![255, 255, 255, 255]), "the bulb is white");
        assert_eq!(pixel(31, 31).and_then(|pixel| pixel.get(3).copied()), Some(0), "the bottom-right corner is clear");

        let [hx, hy] = image.hotspot;
        assert!(pixel(hx, hy).and_then(|pixel| pixel.first().copied()).is_some_and(|grey| grey < 128), "the tip's edge is the dark outline");
        assert_eq!(CursorImage::eyedropper(0).size, 8, "a degenerate size is clamped");
    }

    #[test]
    fn the_pipette_is_as_tall_as_the_systems_arrow_in_its_cell_at_every_size() {
        for size in [32, 48, 64] {
            let image = CursorImage::eyedropper(size);
            let ink: Vec<[u32; 2]> =
                (0..size).flat_map(|y| (0..size).map(move |x| [x, y])).filter(|&[x, y]| image.alpha(x, y).is_some_and(|a| a > 0)).collect();
            let span = |axis: usize| {
                let along = ink.iter().map(|pixel| pixel[axis]);
                along.clone().max().unwrap_or(0) - along.min().unwrap_or(0) + 1
            };
            let most = (size as f32 * 0.69).ceil() as u32;

            assert_eq!(image.size, size, "the pointer's cell is the system pointer's size");
            assert!(span(0) <= most && span(1) <= most, "{size}: the pipette takes {}x{} of the cell, at most {most}", span(0), span(1));
            assert!(span(0) >= size / 2 && span(1) >= size / 2, "{size}: and still reads: {}x{}", span(0), span(1));
        }
    }

    #[test]
    fn the_pointer_is_the_panels_pipette_white_with_its_glass_tube_hollow() {
        let size = 48;
        let image = CursorImage::eyedropper(size);
        let glyph_px = (size as f32 * GLYPH_SHARE).round() as u32;
        let top = size - glyph_px;
        let glyph = crate::drawn_icon::DrawnIcon::Pipette.coverage(glyph_px);
        let at = |[x, y]: [f32; 2]| ((y * glyph_px as f32) as u32 + top, (x * glyph_px as f32) as u32);
        let grey = |point: [f32; 2]| {
            let (row, column) = at(point);
            image.rgba.get(((row * size + column) * 4) as usize).copied()
        };

        let tube = pipette::square(0.4, 0.0);
        assert!(grey(tube).is_some_and(|grey| grey < 64), "the tube's inside is dark, as the glyph leaves it clear");
        assert!(image.alpha(at(tube).1, at(tube).0).is_some_and(|alpha| alpha == u8::MAX), "and still opaque, so the pointer reads");
        assert_eq!(grey(pipette::square(0.85, 0.0)), Some(u8::MAX), "the bulb is white");

        for (index, coverage) in glyph.iter().enumerate() {
            let (row, column) = (index as u32 / glyph_px + top, index as u32 % glyph_px);
            let alpha = image.alpha(column, row).unwrap_or(0);
            assert!(*coverage == 0 || alpha > 0, "every pixel of the glyph's ink is part of the pointer ({index})");
        }
    }

    #[test]
    fn the_cur_files_hotspot_is_the_tip_and_its_pixel_is_ink() {
        let image = CursorImage::eyedropper(32);
        let file = image.cur_file();
        let word = |at: usize| file.get(at..at + 2).map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]));

        assert_eq!([word(2), word(4)], [Some(CUR_TYPE), Some(1)], "one cursor picture");
        assert_eq!([word(10), word(12)], [Some(image.hotspot[0] as u16), Some(image.hotspot[1] as u16)], "the hotspot is the tip");
        assert_eq!(file.len(), (CUR_HEADER_BYTES + BITMAP_HEADER_BYTES) as usize + 32 * 32 * 4 + 4 * 32, "header, bitmap header, colour rows and mask rows");

        let [hx, hy] = image.hotspot;
        let bottom_up_row = (31 - hy) as usize;
        let alpha = file.get((CUR_HEADER_BYTES + BITMAP_HEADER_BYTES) as usize + (bottom_up_row * 32 + hx as usize) * 4 + 3).copied();
        assert!(alpha.is_some_and(|alpha| alpha >= INK_ALPHA), "the file's hotspot pixel is the tip's ink: {alpha:?}");
    }
}
