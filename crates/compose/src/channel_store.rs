//! What a channel's storage format keeps of a composited value

use std::sync::OnceLock;

use texturecraft_core::{ChannelFormat, lum};
use texturecraft_texel::{linear_to_srgb, srgb_to_linear};

/// What `value` keeps in `format`: 8-bit = 256 steps (sRGB8 gamma-spaced), L = grey, others = the tiles' 16 bits; `signed` = stored offset into 0..1
pub(crate) fn stored(format: ChannelFormat, signed: bool, mut value: [f32; 3], components: usize) -> [f32; 3] {
    if format.is_grey() && components == 3 {
        value = [lum(value); 3];
    }
    value.map(|component| stored_value(format, signed, component))
}

fn stored_value(format: ChannelFormat, signed: bool, component: f32) -> f32 {
    let value = if signed { component * 0.5 + 0.5 } else { component };
    let unit = |unit_value: f32| if signed { unit_value * 2.0 - 1.0 } else { unit_value };
    let step = |unit_value: f32| (unit_value.clamp(0.0, 1.0) * 255.0).round();
    match format {
        ChannelFormat::Rgb8 | ChannelFormat::L8 => unit(step(value) / 255.0),
        ChannelFormat::Srgb8 => unit(srgb8_linear().get(step(linear_to_srgb(value.clamp(0.0, 1.0))) as usize).copied().unwrap_or(0.0)),
        ChannelFormat::L16 | ChannelFormat::Rgb16 | ChannelFormat::L16F | ChannelFormat::Rgb16F | ChannelFormat::L32F | ChannelFormat::Rgb32F => component,
    }
}

/// The linear value of each 8-bit sRGB step, so storing a colour decodes no sRGB per texel
fn srgb8_linear() -> &'static [f32; 256] {
    static LINEAR: OnceLock<[f32; 256]> = OnceLock::new();
    LINEAR.get_or_init(|| std::array::from_fn(|step| srgb_to_linear(step as f32 / 255.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_bit_formats_keep_256_steps_and_sixteen_bit_ones_everything() {
        let x = 0.3_f32 + 1.0 / 1024.0;
        let eight = stored(ChannelFormat::L8, false, [x; 3], 1)[0];

        assert!((eight * 255.0 - (eight * 255.0).round()).abs() < 1e-4, "on a 1/255 step");
        assert!((eight - x).abs() > 1e-4 && (eight - x).abs() <= 0.5 / 255.0 + 1e-6);
        assert_eq!(stored(ChannelFormat::L16, false, [x; 3], 1)[0], x);
        assert_eq!(stored(ChannelFormat::Rgb32F, true, [-x; 3], 1)[0], -x);
    }

    #[test]
    fn srgb8_keeps_more_of_the_darks_than_rgb8() {
        let dark = 0.002_f32;
        let srgb = stored(ChannelFormat::Srgb8, false, [dark; 3], 3)[0];
        let linear = stored(ChannelFormat::Rgb8, false, [dark; 3], 3)[0];

        assert!((srgb - dark).abs() < (linear - dark).abs());
    }

    #[test]
    fn srgb8_keeps_the_same_steps_as_decoding_each_one() {
        for value in (0..=1000).map(|step| step as f32 / 1000.0) {
            let decoded = srgb_to_linear((linear_to_srgb(value) * 255.0).round() / 255.0);

            assert_eq!(stored(ChannelFormat::Srgb8, false, [value; 3], 3)[0], decoded, "at {value}");
        }
    }

    #[test]
    fn grey_formats_turn_colours_grey_and_signed_values_keep_their_sign() {
        let grey = stored(ChannelFormat::L16, false, [1.0, 0.0, 0.0], 3);

        assert!(grey[0] == grey[1] && grey[1] == grey[2] && grey[0] > 0.0 && grey[0] < 1.0);
        assert_eq!(stored(ChannelFormat::Rgb16, false, [1.0, 0.0, 0.0], 3), [1.0, 0.0, 0.0]);

        let low = stored(ChannelFormat::L8, true, [-0.5; 3], 1)[0];
        assert!(low < -0.49 && low > -0.51);
    }
}
