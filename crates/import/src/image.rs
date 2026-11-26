//! [`Rgba`]: the file's pixel buffers as plain float images (rows top first, straight values, alpha = coverage), and
//! the ways they leave the translator: as a paint layer's or mask's tiles ([`Rgba::layer_tiles`],
//! [`Rgba::mask_tiles`]) and as an image resource file ([`Rgba::resource_bytes`]).
//!
//! Texture-cache entries are premultiplied by coverage, colour entries in linear light before sRGB encoding; image
//! resources are straight. Reading the stored pixels is the reader's ([`SourceFormat`]); the colour maths is here.

use rayon::prelude::*;
use texturecraft_compose::ImageRes;
use texturecraft_core::{Channel, ChannelKind};
use texturecraft_import_reader::decode::pixels::{Alpha, RowOrder, SourceFormat};
use texturecraft_texel::{TILE, TILE_AREA, TileImage, srgb_to_linear, to_u16, to_u16_signed};

/// Largest image side read from a file: the reference app's maximum (8K); anything larger is refused as hostile.
pub const MAX_SIDE: u32 = 8192;

/// A float image: rows top first, `width × height` texels.
#[derive(Debug, Clone, PartialEq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub px: Vec<[f32; 4]>,
}

/// Why a buffer could not become an image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageError(pub String);

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Rgba {
    /// One value everywhere.
    pub fn solid(width: u32, height: u32, v: [f32; 4]) -> Rgba {
        Rgba { width, height, px: vec![v; width as usize * height as usize] }
    }

    pub fn get(&self, x: u32, y: u32) -> [f32; 4] {
        if x >= self.width {
            return [0.0; 4];
        }
        self.px.get(y as usize * self.width as usize + x as usize).copied().unwrap_or([0.0; 4])
    }

    /// Decodes a stored buffer of `format` (`raw` may be short: missing bytes read as zero). Premultiplied formats
    /// are made straight; sRGB cache entries are also made linear (`linear`: true for those), others keep the stored
    /// encoding.
    pub fn decode(format: SourceFormat, rows: RowOrder, width: u32, height: u32, raw: &[u8]) -> Result<Rgba, ImageError> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(ImageError(format!("image is {width}x{height}")));
        }
        let (w, h) = (width as usize, height as usize);
        let mut px = vec![[0.0f32; 4]; w * h];
        px.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            let src_y = rows.stored_row(y, h);
            for (x, p) in row.iter_mut().enumerate() {
                *p = straight(format.decode_at(raw, src_y * w + x), format.alpha());
            }
        });
        Ok(Rgba { width, height, px })
    }

    /// True if no texel has coverage below 1.
    pub fn fully_covered(&self) -> bool {
        self.px.iter().all(|p| p[3] >= FULL)
    }

    /// The tiles of a paint layer channel: premultiplied value(s) plus coverage, as the compositor reads them. Values
    /// are what TextureCraft holds for `channel` (linear colours); tiles without coverage stay unallocated.
    pub fn layer_tiles(&self, channel: Channel) -> TileImage {
        let comps = channel.components();
        let signed = channel.kind() == ChannelKind::Signed;
        self.tiles(comps + 1, |p, out| {
            let a = cover(p[3]);
            for (k, slot) in out.iter_mut().take(comps).enumerate() {
                let v = finite(p[k]) * a;
                *slot = if signed { to_u16_signed(v) } else { to_u16(v) };
            }
            if let Some(slot) = out.get_mut(comps) {
                *slot = to_u16(a);
            }
        })
    }

    /// The tiles of a mask paint effect: the premultiplied grey (red) plus coverage.
    pub fn mask_tiles(&self) -> TileImage {
        self.tiles(2, |p, out| {
            let a = cover(p[3]);
            if let [v, c] = out {
                *v = to_u16(finite(p[0]).clamp(0.0, 1.0) * a);
                *c = to_u16(a);
            }
        })
    }

    fn tiles(&self, comps: usize, texel: impl Fn([f32; 4], &mut [u16]) + Sync) -> TileImage {
        let mut img = TileImage::new(self.width, self.height, comps);
        let (tx_n, ty_n) = (img.tiles_x(), img.tiles_y());
        let coords: Vec<(u32, u32)> = (0..ty_n).flat_map(|ty| (0..tx_n).map(move |tx| (tx, ty))).collect();
        let built: Vec<(Tile, Option<Vec<u16>>)> = coords
            .into_par_iter()
            .map(|(tx, ty)| {
                let mut data = vec![0u16; TILE_AREA * comps];
                let mut any = false;
                for ly in 0..TILE {
                    for lx in 0..TILE {
                        let (x, y) = (tx * TILE + lx, ty * TILE + ly);
                        if x >= self.width || y >= self.height {
                            continue;
                        }
                        let p = self.get(x, y);
                        if p[3] <= 0.0 || !p[3].is_finite() {
                            continue;
                        }
                        any = true;
                        let o = (ly * TILE + lx) as usize * comps;
                        if let Some(out) = data.get_mut(o..o + comps) {
                            texel(p, out);
                        }
                    }
                }
                ((tx, ty), any.then_some(data))
            })
            .collect();
        for ((tx, ty), data) in built {
            img.set_tile(tx, ty, data.map(std::sync::Arc::new));
        }
        img
    }

    /// The image as the compositor samples a resource (`srgb`: integer values in the stored encoding).
    pub fn image_res(&self, srgb: bool) -> ImageRes {
        ImageRes::new(self.width, self.height, self.px.clone(), srgb)
    }

    /// The image as a resource file: 16-bit PNG for integer images (`srgb`: values in their stored encoding, as the
    /// compositor reads 8/16-bit files), EXR for float images (linear).
    pub fn resource_bytes(&self, srgb: bool) -> Result<Vec<u8>, ImageError> {
        let mut out = Vec::new();
        let cursor = &mut std::io::Cursor::new(&mut out);
        let err = |e: image::ImageError| ImageError(e.to_string());
        if srgb {
            let raw: Vec<u16> = self.px.iter().flat_map(|p| p.map(|v| to_u16(finite(v)))).collect();
            let img = image::ImageBuffer::<image::Rgba<u16>, _>::from_raw(self.width, self.height, raw).ok_or_else(|| ImageError("image size".into()))?;
            img.write_to(cursor, image::ImageFormat::Png).map_err(err)?;
        } else {
            let raw: Vec<f32> = self.px.iter().flat_map(|p| p.map(finite)).collect();
            let img = image::ImageBuffer::<image::Rgba<f32>, _>::from_raw(self.width, self.height, raw).ok_or_else(|| ImageError("image size".into()))?;
            img.write_to(cursor, image::ImageFormat::OpenExr).map_err(err)?;
        }
        Ok(out)
    }
}

/// A tile's coordinates.
type Tile = (u32, u32);

/// Coverage at or above this counts as full.
const FULL: f32 = 0.999;

fn finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

fn cover(a: f32) -> f32 {
    if a.is_finite() { a.clamp(0.0, 1.0) } else { 0.0 }
}

/// Straight values of a stored pixel; sRGB cache entries come out linear. Uncovered pixels are all zero.
fn straight(p: [f32; 4], alpha: Alpha) -> [f32; 4] {
    let a = p[3];
    match alpha {
        Alpha::Straight => p,
        _ if a.is_nan() || a <= 0.0 => [0.0; 4],
        Alpha::Premultiplied => [p[0] / a, p[1] / a, p[2] / a, a],
        Alpha::PremultipliedLinearSrgb => {
            let c = |v: f32| (srgb_to_linear(v) / a).min(1.0);
            [c(p[0]), c(p[1]), c(p[2]), a]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_texel::{from_u16, from_u16_signed, linear_to_srgb};

    #[test]
    fn cache_pixels_are_made_straight_and_flipped() {
        // 1×2 RG8, bottom row first: bottom (128, 255) full coverage, top (64, 128) half coverage.
        let img = Rgba::decode(SourceFormat::CacheRg8, RowOrder::BottomFirst, 1, 2, &[128, 255, 64, 128]).unwrap();
        let top = img.get(0, 0);
        assert!((top[0] - 0.5).abs() < 0.01 && (top[3] - 0.5).abs() < 0.01, "{top:?}");
        assert!((img.get(0, 1)[0] - 128.0 / 255.0).abs() < 1e-6);
        assert!(!img.fully_covered());
        let lin = linear_to_srgb(0.5 * srgb_to_linear(0.8));
        let enc = (lin * 255.0).round() as u8;
        let srgb = Rgba::decode(SourceFormat::CacheSrgb8Alpha8, RowOrder::TopFirst, 1, 1, &[enc, 0, 0, 128]).unwrap();
        assert!((srgb.get(0, 0)[0] - srgb_to_linear(0.8)).abs() < 0.02, "linear, straight: {:?}", srgb.get(0, 0));
        assert!(Rgba::decode(SourceFormat::Lum8, RowOrder::TopFirst, 0, 4, &[]).is_err());
        assert!(Rgba::decode(SourceFormat::Lum8, RowOrder::TopFirst, MAX_SIDE + 1, 1, &[]).is_err());
        // short buffers read as zero
        let short = Rgba::decode(SourceFormat::Lum16, RowOrder::TopFirst, 2, 2, &[0xff, 0xff, 0xff]).unwrap();
        assert_eq!(short.get(0, 0)[0], 1.0);
        assert_eq!(short.get(1, 1)[0], 0.0);
    }

    #[test]
    fn layer_tiles_are_premultiplied_and_sparse() {
        let mut img = Rgba::solid(130, 70, [0.0; 4]);
        img.px[0] = [0.4, 0.4, 0.4, 0.5];
        let t = img.layer_tiles(Channel::Roughness);
        assert_eq!((t.width(), t.height(), t.comps(), t.allocated_count()), (130, 70, 2, 1));
        let v = t.get(0, 0);
        assert!((from_u16(v[0]) - 0.2).abs() < 1e-4 && (from_u16(v[1]) - 0.5).abs() < 1e-4);
        let mut h = Rgba::solid(2, 2, [-0.5, -0.5, -0.5, 1.0]);
        h.px[3] = [f32::NAN, 0.0, 0.0, 1.0];
        let ht = h.layer_tiles(Channel::Height);
        assert!((from_u16_signed(ht.get(0, 0)[0]) + 0.5).abs() < 1e-4);
        assert!(from_u16_signed(ht.get(1, 1)[0]).abs() < 1e-4, "NaN reads as zero");
        let m = Rgba::solid(2, 2, [0.25, 0.0, 0.0, 1.0]).mask_tiles();
        assert!((from_u16(m.get(1, 1)[0]) - 0.25).abs() < 1e-4);
    }

    #[test]
    fn resources_round_trip_through_files() {
        let img = Rgba { width: 2, height: 1, px: vec![[0.25, 0.5, 1.0, 1.0], [0.0, 0.0, 0.0, 0.5]] };
        let png = img.resource_bytes(true).unwrap();
        let back = image::load_from_memory(&png).unwrap().into_rgba32f();
        assert!((back.get_pixel(0, 0)[1] - 0.5).abs() < 1e-4);
        let exr = Rgba { width: 1, height: 1, px: vec![[2.0, 0.5, 0.0, 1.0]] }.resource_bytes(false).unwrap();
        let back = image::load_from_memory(&exr).unwrap().into_rgba32f();
        assert!((back.get_pixel(0, 0)[0] - 2.0).abs() < 1e-4, "float images keep values above 1");
    }
}
