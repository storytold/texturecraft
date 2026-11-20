//! Decoding imported image resources (fills, alphas, stencils).

use craft_image_codecs::{ChannelLayout, EncodeOptions, Format, Image, PngCompression, SampleType};

use crate::{FormatError, MAX_IMAGE_SIZE, MAX_RESOURCE_TEXELS, Result};

/// A decoded image: RGBA as 0..1 floats in the file's own encoding (8/16-bit files stay
/// sRGB-encoded; `srgb` says so), row-major, top row first.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<[f32; 4]>,
    /// True for 8/16-bit images (sRGB-encoded); false for float images (linear).
    pub srgb: bool,
}

/// Decode PNG, JPEG, TGA, TIFF, BMP, WEBP, EXR or HDR bytes.
pub fn decode_image(bytes: &[u8]) -> Result<DecodedImage> {
    decode_image_within(bytes, MAX_RESOURCE_TEXELS)
}

/// [`decode_image`] for an image of at most `max_texels` (width × height), checked from the
/// header, before any pixel is decoded or allocated.
pub fn decode_image_within(bytes: &[u8], max_texels: u64) -> Result<DecodedImage> {
    decode_image_with_limits(bytes, max_texels, image::Limits::default())
}

/// Decode a cook output under both its declared dimensions and decoder allocation budget.
pub fn decode_cooked_image(bytes: &[u8], size: [u32; 2], max_decoder_bytes: u64) -> Result<DecodedImage> {
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(max_decoder_bytes);
    limits.max_image_width = Some(size[0]);
    limits.max_image_height = Some(size[1]);
    let texels = (u64::from(size[0]) * u64::from(size[1])).min(MAX_RESOURCE_TEXELS);
    decode_image_with_limits(bytes, texels, limits)
}

fn decode_image_with_limits(bytes: &[u8], max_texels: u64, limits: image::Limits) -> Result<DecodedImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| FormatError::Malformed { kind: "image", msg: error.to_string() })?;

    // TGA has no magic signature; its decoder validates the header when no format was recognized
    let format = reader.format().unwrap_or(image::ImageFormat::Tga);
    reader.set_format(format);
    reader.limits(limits);

    let (width, height) = image::ImageReader::with_format(std::io::Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|error| FormatError::Malformed { kind: "image", msg: error.to_string() })?;
    if width == 0 || height == 0 || width > MAX_IMAGE_SIZE || height > MAX_IMAGE_SIZE {
        return Err(FormatError::TooLarge(format!("image is {width}x{height} (limit {MAX_IMAGE_SIZE})")));
    }

    if u64::from(width) * u64::from(height) > max_texels {
        return Err(FormatError::TooLarge(format!("image is {width}x{height}, more than {max_texels} texels")));
    }

    let image = crate::guard("image", || reader.decode().map_err(|error| FormatError::Malformed { kind: "image", msg: error.to_string() }))?;
    let srgb = !matches!(image, image::DynamicImage::ImageRgb32F(_) | image::DynamicImage::ImageRgba32F(_));
    let float_image = image.into_rgba32f();
    let (width, height) = float_image.dimensions();

    // Reuse the float buffer when its allocation can be represented as RGBA pixels
    let rgba = match bytemuck::allocation::try_cast_vec(float_image.into_raw()) {
        Ok(rgba) => rgba,
        Err((_, samples)) => {
            let mut rgba = Vec::new();
            rgba.try_reserve_exact(samples.len() / 4).map_err(|error| FormatError::TooLarge(error.to_string()))?;
            rgba.extend_from_slice(samples.as_chunks::<4>().0);
            rgba
        }
    };

    Ok(DecodedImage { width, height, rgba, srgb })
}

/// PNG bytes of an `rgb` (sRGB) image whose alpha is `coverage` (`width × height`, top row first):
/// drawn text as an image resource.
pub fn coverage_png(width: u32, height: u32, coverage: &[u8], rgb: [u8; 3]) -> Result<Vec<u8>> {
    // Resource callers may supply a larger coverage buffer. Encode just the given extent.
    let coverage = usize::try_from(u64::from(width) * u64::from(height))
        .ok()
        .and_then(|count| coverage.get(..count))
        .ok_or_else(|| FormatError::Invalid(format!("{} coverage values for {width}x{height}", coverage.len())))?;
    let bytes = coverage.len().checked_mul(4).ok_or_else(|| FormatError::Invalid(format!("RGBA buffer does not fit {width}x{height}")))?;
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(bytes).map_err(|error| FormatError::Invalid(error.to_string()))?;
    rgba.extend(coverage.iter().flat_map(|&alpha| [rgb[0], rgb[1], rgb[2], alpha]));
    let image = Image::from_u8(width, height, ChannelLayout::Rgba, rgba).map_err(|error| FormatError::Invalid(error.to_string()))?;
    encode_resource_png(&image)
}

/// 16-bit single-channel PNG bytes of `values` (0..1, `width × height`, top row first): one map
/// split out of a packed image, or an inverted one.
pub fn gray16_png(width: u32, height: u32, values: impl Iterator<Item = f32>) -> Result<Vec<u8>> {
    encode_resource_png(&gray16_image(width, height, values)?)
}

/// A single-channel PNG and its decoded pixels, quantized identically to a saved/reloaded image.
pub fn gray16_resource(width: u32, height: u32, values: impl Iterator<Item = f32>) -> Result<(Vec<u8>, DecodedImage)> {
    let image = gray16_image(width, height, values)?;
    let png = encode_resource_png(&image)?;
    let rgba = image
        .data()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|bytes| {
            // Match image's integer-to-float expansion so reload is bit-identical
            const GRAY16_TO_FLOAT: f32 = 1.0 / u16::MAX as f32;
            let value = f32::from(u16::from_ne_bytes(*bytes)) * GRAY16_TO_FLOAT;
            [value, value, value, 1.0]
        })
        .collect();

    Ok((png, DecodedImage { width, height, rgba, srgb: true }))
}

fn encode_resource_png(image: &Image) -> Result<Vec<u8>> {
    let options = EncodeOptions { png_compression: PngCompression::Fast, ..EncodeOptions::default() };
    craft_image_codecs::encode(image, Format::Png, &options).map_err(|error| FormatError::Invalid(error.to_string()))
}

fn gray16_image(width: u32, height: u32, values: impl Iterator<Item = f32>) -> Result<Image> {
    let texels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || width > MAX_IMAGE_SIZE || height > MAX_IMAGE_SIZE || texels > MAX_RESOURCE_TEXELS {
        return Err(FormatError::TooLarge(format!("image is {width}x{height} (limit {MAX_RESOURCE_TEXELS} texels)")));
    }

    let expected = usize::try_from(texels).map_err(|_| FormatError::TooLarge(format!("image is {width}x{height}")))?;
    // Build the shared image's native-endian buffer directly to avoid a second copy of the samples.
    let data = values
        .take(expected + 1)
        .flat_map(|value| {
            let sample = if value.is_finite() { (value.clamp(0.0, 1.0) * f32::from(u16::MAX)).round() as u16 } else { 0 };
            sample.to_ne_bytes()
        })
        .collect();
    Image::from_raw(width, height, ChannelLayout::Gray, SampleType::U16, data)
        .map_err(|_| FormatError::Invalid(format!("pixel count does not match {width}x{height}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_png_and_rejects_garbage() {
        let mut png = Vec::new();
        let image = image::RgbaImage::from_fn(4, 2, |x, _| image::Rgba([x as u8 * 60, 0, 0, 255]));
        image.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();

        let decoded = decode_image(&png).unwrap();

        assert_eq!((decoded.width, decoded.height), (4, 2));
        assert!(decoded.srgb);
        assert!((decoded.rgba[1][0] - 60.0 / 255.0).abs() < 1e-3);
        assert!(decode_image(b"not an image").is_err());
        assert!(decode_image(&png[..png.len() / 2]).is_err());

        let text = decode_image(&coverage_png(2, 1, &[0, 255], [255; 3]).unwrap()).unwrap();

        assert_eq!(text.rgba, vec![[1.0, 1.0, 1.0, 0.0], [1.0; 4]]);
        assert!(coverage_png(2, 2, &[0], [255; 3]).is_err());
    }

    #[test]
    fn coverage_png_preserves_rgba8_samples_and_ignores_excess_coverage() {
        let coverage = [0, 17, 128, 255, 231];
        let png = coverage_png(2, 2, &coverage, [19, 83, 207]).unwrap();
        let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap();

        assert_eq!(decoded.color(), image::ColorType::Rgba8);
        let rgba = decoded.as_rgba8().unwrap();
        assert_eq!(rgba.dimensions(), (2, 2));
        assert_eq!(rgba.as_raw(), &[19, 83, 207, 0, 19, 83, 207, 17, 19, 83, 207, 128, 19, 83, 207, 255]);
        assert!(matches!(coverage_png(2, 2, &coverage[..3], [19, 83, 207]), Err(FormatError::Invalid(_))));
    }

    #[test]
    fn gray16_png_preserves_asymmetric_samples_in_row_order() {
        let samples = [0x0001u16, 0x0100, 0x1234, 0xabcd, 0xff00, 0xffff];
        let values = samples.into_iter().map(|value| f32::from(value) / f32::from(u16::MAX));
        let png = gray16_png(3, 2, values).unwrap();
        let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap();

        assert_eq!(decoded.color(), image::ColorType::L16);
        let gray = decoded.as_luma16().unwrap();
        assert_eq!(gray.dimensions(), (3, 2));
        assert_eq!(gray.as_raw(), &samples);
    }

    #[test]
    fn coverage_png_preserves_samples_above_four_mebibytes() {
        let (width, height) = (1024u32, 1025u32);
        let coverage: Vec<u8> = (0..width * height).map(|index| ((index * 37 + index / width) % 256) as u8).collect();
        // Exercise the shared encoder's parallel path, selected by raw image byte count.
        assert!(coverage.len() * 4 > 4 << 20);
        let png = coverage_png(width, height, &coverage, [23, 97, 211]).unwrap();
        let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap();

        assert_eq!(decoded.color(), image::ColorType::Rgba8);
        let rgba = decoded.as_rgba8().unwrap();
        assert_eq!(rgba.dimensions(), (width, height));
        for (index, (pixel, alpha)) in rgba.pixels().zip(&coverage).enumerate() {
            assert_eq!(pixel.0, [23, 97, 211, *alpha], "pixel {index}");
        }
    }

    #[test]
    fn gray16_round_trips_and_rejects_a_wrong_count() {
        let png = gray16_png(2, 1, [0.0, 1.0].into_iter()).unwrap();
        let decoded = decode_image(&png).unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!((decoded.rgba[0][0], decoded.rgba[1][0]), (0.0, 1.0));
        assert!(gray16_png(2, 2, [0.5].into_iter()).is_err());

        // NaN becomes 0, out-of-range values clamp
        let decoded = decode_image(&gray16_png(3, 1, [f32::NAN, 9.0, -9.0].into_iter()).unwrap()).unwrap();
        assert_eq!((decoded.rgba[0][0], decoded.rgba[1][0], decoded.rgba[2][0]), (0.0, 1.0, 0.0));
    }

    #[test]
    fn generated_resource_matches_reload_for_every_gray16_value() {
        let values = (0..=u16::MAX).map(|value| f32::from(value) / f32::from(u16::MAX));

        let (png, decoded) = gray16_resource(256, 256, values).unwrap();

        let reloaded = decode_image(&png).unwrap();
        assert_eq!((decoded.width, decoded.height, decoded.srgb), (reloaded.width, reloaded.height, reloaded.srgb));
        for (index, (cached, saved)) in decoded.rgba.iter().zip(&reloaded.rgba).enumerate() {
            assert_eq!(cached, saved, "pixel {index}");
        }
        let extremes = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 2.0, 0.5];
        let (png, decoded) = gray16_resource(6, 1, extremes.into_iter()).unwrap();

        let reloaded = decode_image(&png).unwrap();
        assert_eq!((decoded.width, decoded.height, decoded.srgb), (reloaded.width, reloaded.height, reloaded.srgb));
        for (index, (cached, saved)) in decoded.rgba.iter().zip(&reloaded.rgba).enumerate() {
            assert_eq!(cached, saved, "pixel {index}");
        }
        assert!(gray16_resource(2, 1, [0.5].into_iter()).is_err());
    }

    #[test]
    fn gray16_stops_after_one_excess_pixel() {
        let mut pixels_read = 0;
        let values = std::iter::from_fn(|| {
            pixels_read += 1;
            assert!(pixels_read <= 2, "iterator must stop after the first excess pixel");
            Some(0.5)
        });

        assert!(gray16_png(1, 1, values).is_err());
    }

    #[test]
    fn tga_without_a_magic_signature_decodes_within_the_texel_budget() {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([128, 51, 255, 255])).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Tga).unwrap();

        let image = decode_image_within(&bytes, 16).unwrap();

        assert_eq!((image.width, image.height), (4, 4));
        assert!((image.rgba[0][1] - 0.2).abs() < 1e-6);
        assert!(image.srgb);

        let header = &bytes[..18];

        assert!(matches!(decode_image_within(header, 15), Err(FormatError::TooLarge(_))));
        assert!(decode_image(header).is_err());
    }

    #[test]
    fn decoding_within_a_texel_limit_refuses_big_images_from_the_header() {
        let png = {
            let mut bytes = Vec::new();
            image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3])).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
            bytes
        };

        assert_eq!(decode_image_within(&png, 64).unwrap().width, 8);
        assert!(matches!(decode_image_within(&png, 63), Err(FormatError::TooLarge(_))));
        assert!(decode_image(&png).is_ok());
    }

    #[test]
    fn the_default_decoder_refuses_more_than_the_resource_texel_budget_from_the_header() {
        // The edge limit permits 16384² but the default resource budget does not
        let mut header = [0u8; 18];
        header[2] = 2;
        header[12..14].copy_from_slice(&16_384u16.to_le_bytes());
        header[14..16].copy_from_slice(&16_384u16.to_le_bytes());
        header[16] = 24;

        assert!(matches!(decode_image(&header), Err(FormatError::TooLarge(_))));
    }
}
