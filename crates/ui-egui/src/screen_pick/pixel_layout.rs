/// How the X server lays out one pixel of a `ZPixmap` image
#[derive(Clone, Copy, Debug)]
pub(crate) struct PixelLayout {
    pub(crate) bits_per_pixel: u8,
    pub(crate) least_significant_first: bool,
    /// The red, green and blue masks of the root window's visual
    pub(crate) masks: [u32; 3],
}

impl PixelLayout {
    /// The 8-bit colour of the first pixel in `data`, or `None` for a layout other than 24 or 32 bits per pixel
    pub(crate) fn decode(&self, data: &[u8]) -> Option<[u8; 3]> {
        let bytes = match self.bits_per_pixel {
            24 => 3,
            32 => 4,
            _ => return None,
        };
        let pixel = data.get(..bytes)?;
        let value = if self.least_significant_first {
            pixel.iter().rev().fold(0_u32, |value, byte| (value << 8) | u32::from(*byte))
        } else {
            pixel.iter().fold(0_u32, |value, byte| (value << 8) | u32::from(*byte))
        };
        Some(self.masks.map(|mask| channel(value, mask)))
    }
}

/// The channel `mask` selects in `value`, scaled to 8 bits
fn channel(value: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }

    let shift = mask.trailing_zeros();
    let max = mask >> shift;
    let raw = (value & mask) >> shift;
    u8::try_from(u64::from(raw) * 255 / u64::from(max)).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RGB_MASKS: [u32; 3] = [0x00ff_0000, 0x0000_ff00, 0x0000_00ff];

    #[test]
    fn a_32_bit_pixel_reads_in_the_servers_byte_order() {
        let little = PixelLayout { bits_per_pixel: 32, least_significant_first: true, masks: RGB_MASKS };
        let big = PixelLayout { least_significant_first: false, ..little };

        assert_eq!(little.decode(&[0x30, 0x20, 0x10, 0x00]), Some([0x10, 0x20, 0x30]));
        assert_eq!(big.decode(&[0x00, 0x10, 0x20, 0x30]), Some([0x10, 0x20, 0x30]));
    }

    #[test]
    fn narrow_masks_scale_to_8_bits() {
        let rgb565 = PixelLayout { bits_per_pixel: 24, least_significant_first: true, masks: [0xf800, 0x07e0, 0x001f] };

        assert_eq!(rgb565.decode(&[0xff, 0xff, 0x00]), Some([255, 255, 255]));
        assert_eq!(rgb565.decode(&[0x1f, 0x00, 0x00]), Some([0, 0, 255]));
    }

    #[test]
    fn an_unknown_layout_or_short_data_reads_nothing() {
        let layout = PixelLayout { bits_per_pixel: 16, least_significant_first: true, masks: RGB_MASKS };

        assert_eq!(layout.decode(&[0, 0, 0, 0]), None);
        assert_eq!(PixelLayout { bits_per_pixel: 32, ..layout }.decode(&[0, 0]), None);
    }
}
