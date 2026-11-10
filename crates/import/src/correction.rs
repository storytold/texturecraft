//! The correction layer: the original's stored result only where the imported layers do not reproduce it
//! ([`crate::ImportOptions::original_correction`], off by default).
//!
//! The set's stack as imported (channel defaults included; the hidden reference contributes nothing) is composited per
//! channel and compared texel by texel with the stored result laid over the channel defaults (what the original shows).
//! Every texel on the mesh that differs by more than [`TEXEL_STEPS`] storage steps (any component) gets the original's
//! value in the correction layer at full coverage; every other texel stays empty, so the layers below show through.
//! With the correction on top the set looks as the original saved it; switched off, it shows what the layers reproduce,
//! and how much it covers is how far the import still is from editable. The correction is the original's pixels, not
//! something to paint on: it is locked, and its lock lists the share of texels it covers per channel.

use texturecraft_compose::eval::Composite;
use texturecraft_core::{Channel, ChannelFormat, ChannelKind};
use texturecraft_geom::{NO_TRI, UvGBuffer};
use texturecraft_texel::{TileImage, linear_to_srgb, to_u16, to_u16_signed};

use crate::resources::CacheImage;

/// The layer's name.
pub const CORRECTION_NAME: &str = "Correction to original";
/// The key of the correction's lock reason: it is the original's pixels.
pub const CORRECTION_KEY: &str = "feature:original-correction";
/// A texel differing by more than this many storage steps of its channel (any component) is corrected: above the one
/// step of rounding either way, so constants rounded differently don't count.
pub const TEXEL_STEPS: f32 = 1.5;
/// One storage step of a half float around 0.5 … 1, where height and normals live (11 significant bits).
const F16_STEP_NEAR_ONE: f32 = 1.0 / 2048.0;
const U8_STEP: f32 = 1.0 / 255.0;
const U16_STEP: f32 = 1.0 / 65535.0;

/// One storage step of `format`, in the stored encoding (0..1).
pub fn storage_step(format: ChannelFormat) -> f32 {
    match format {
        ChannelFormat::Srgb8 | ChannelFormat::L8 | ChannelFormat::Rgb8 => U8_STEP,
        ChannelFormat::L16F | ChannelFormat::Rgb16F => F16_STEP_NEAR_ONE,
        ChannelFormat::L16 | ChannelFormat::Rgb16 | ChannelFormat::L32F | ChannelFormat::Rgb32F => U16_STEP,
    }
}

/// One corrected channel: its paint layer tiles (premultiplied values plus coverage) and the share of the mesh's texels
/// it covers.
pub struct ChannelCorrection {
    pub channel: Channel,
    pub tiles: TileImage,
    pub share: f32,
}

/// The stored result `cache` (over `default` where it is uncovered) on every texel of the mesh where `ours` differs by
/// more than `tolerance` in the stored encoding. `None` when nothing is compared.
pub fn correct(ours: &Composite, gbuf: &UvGBuffer, channel: Channel, cache: &CacheImage, default: [f32; 3], tolerance: f32) -> Option<ChannelCorrection> {
    let comps = channel.components();
    let signed = channel.kind() == ChannelKind::Signed;
    let (w, h) = (cache.image.width.min(ours.resolution).min(gbuf.width), cache.image.height.min(ours.height).min(gbuf.height));
    let encode = |v: f32| if cache.srgb { linear_to_srgb(v.clamp(0.0, 1.0)) } else { v };
    let mut tiles = TileImage::new(ours.resolution, ours.height, comps + 1);
    let (mut on_mesh, mut differ) = (0usize, 0usize);
    let mut texel = vec![0u16; comps + 1];
    for y in 0..h {
        for x in 0..w {
            if gbuf.tri.get((y * gbuf.width + x) as usize).is_none_or(|t| *t == NO_TRI) {
                continue;
            }
            on_mesh += 1;
            let c = cache.image.get(x, y);
            let a = if c[3].is_finite() { c[3].clamp(0.0, 1.0) } else { 0.0 };
            let o = ours.value(channel, x, y);
            let mut stored = [0.0f32; 3];
            let mut off = false;
            for k in 0..comps.min(3) {
                let s = c[k] * a + (1.0 - a) * default[k];
                stored[k] = if s.is_finite() { s } else { 0.0 };
                let d = (encode(o[k]) - encode(stored[k])).abs();
                off |= !d.is_finite() || d > tolerance;
            }
            if !off {
                continue;
            }
            differ += 1;
            for (k, slot) in texel.iter_mut().enumerate() {
                *slot = match stored.get(k).filter(|_| k < comps) {
                    Some(v) if signed => to_u16_signed(*v),
                    Some(v) => to_u16(*v),
                    None => u16::MAX,
                };
            }
            tiles.set(x, y, &texel);
        }
    }
    (on_mesh > 0).then(|| ChannelCorrection { channel, tiles, share: differ as f32 / on_mesh as f32 })
}

/// What [`describe`] puts in front of the shares.
const DESCRIBE_PREFIX: &str = "the original's result where the layers differ: ";
/// How [`percent`] writes a share that is above 0 but would round to 0.0 %.
const TINY_PERCENT: &str = "<0.1%";

/// `the original's result where the layers differ: base color 12.5%, roughness 0.0%`, for the lock and the report.
pub fn describe(shares: &[(String, f32)]) -> String {
    let parts: Vec<String> = shares.iter().map(|(c, share)| format!("{c} {}", percent(*share))).collect();
    format!("{DESCRIBE_PREFIX}{}", parts.join(", "))
}

fn percent(share: f32) -> String {
    let p = share * 100.0;
    if p > 0.0 && p < 0.1 { TINY_PERCENT.to_string() } else { format!("{p:.1}%") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::Rgba;
    use texturecraft_compose::{BakedMaps, GraphStore, ImageStore, PixelStore};
    use texturecraft_core::{Fill, Layer, LayerKind, Project, Source, TextureSet};
    use texturecraft_geom::Mesh;
    use texturecraft_texel::from_u16;

    use crate::safety_net::{SetScene, composite};

    fn quad() -> Mesh {
        let mut m = Mesh::from_parts(
            vec![glam::Vec3::new(0.0, 0.0, 0.0), glam::Vec3::new(1.0, 0.0, 0.0), glam::Vec3::new(1.0, 1.0, 0.0), glam::Vec3::new(0.0, 1.0, 0.0)],
            None,
            vec![glam::Vec2::new(0.0, 0.0), glam::Vec2::new(1.0, 0.0), glam::Vec2::new(1.0, 1.0), glam::Vec2::new(0.0, 1.0)],
            vec![0, 1, 2, 0, 2, 3],
            "m",
        )
        .unwrap();
        m.compute_tangents();
        m
    }

    /// A 64×64 roughness set filled with `ours`, composited, and the corrections against `cache`.
    fn run(ours: f32, cache: &CacheImage) -> (Option<ChannelCorrection>, Project) {
        let mesh = quad();
        let mut p = Project::new("t", "mesh.glb");
        let mut set = TextureSet::new("S", 0, 64);
        set.channels = vec![(Channel::Roughness, ChannelFormat::L8)];
        p.texture_sets.push(set);
        let mut fill = Fill::default();
        fill.sources.insert(Channel::Roughness, Source::Uniform([ours; 3]));
        let l = Layer::new(p.alloc_id(), "f", LayerKind::Fill(fill), &[Channel::Roughness]);
        p.insert_layer(0, l, None, false).unwrap();
        let (pixels, baked, images) = (PixelStore::default(), BakedMaps::default(), ImageStore::default());
        let scene = SetScene { project: &p, ts: 0, mesh: &mesh, pixels: &pixels, baked: &baked, images: &images, graphs: GraphStore::empty() };
        let (c, gbuf) = composite(&scene, &[Channel::Roughness]).unwrap();
        (correct(&c, &gbuf, Channel::Roughness, cache, [0.3; 3], TEXEL_STEPS * storage_step(ChannelFormat::L8)), p)
    }

    fn cache(f: impl Fn(u32, u32) -> f32) -> CacheImage {
        let mut image = Rgba::solid(64, 64, [0.0; 4]);
        for y in 0..64 {
            for x in 0..64 {
                let v = f(x, y);
                image.px[(y * 64 + x) as usize] = [v, v, v, 1.0];
            }
        }
        CacheImage { bit: 0, image, srgb: false, missing_bytes: 0 }
    }

    #[test]
    fn only_differing_texels_are_corrected_with_the_original_value() {
        let (c, _) = run(0.3, &cache(|x, y| if (x, y) == (20, 31) { 0.9 } else { 0.3 }));
        let c = c.unwrap();
        assert_eq!(c.tiles.allocated_count(), 1, "sparse: one tile");
        let at = |x, y| c.tiles.get(x, y).iter().take(2).map(|v| from_u16(*v)).collect::<Vec<_>>();
        assert!((at(20, 31)[0] - 0.9).abs() < 1e-3 && at(20, 31)[1] == 1.0);
        assert_eq!(at(21, 31), [0.0, 0.0], "agreeing texels are left to the layers below");
        assert!(c.share > 0.0 && c.share < 0.001, "{}", c.share);
        assert_eq!(describe(&[("roughness".into(), c.share)]), "the original's result where the layers differ: roughness <0.1%");
    }

    #[test]
    fn a_stack_that_reproduces_the_original_needs_nothing_and_rounding_is_tolerated() {
        // one 8-bit step off everywhere: within the tolerance
        let (c, _) = run(0.3, &cache(|_, _| 0.3 + 1.0 / 255.0));
        let c = c.unwrap();
        assert_eq!((c.tiles.allocated_count(), c.share), (0, 0.0));
        // uncovered stored texels show the channel default (0.3): nothing to correct either
        let mut uncovered = cache(|_, _| 0.0);
        uncovered.image.px.iter_mut().for_each(|p| p[3] = 0.0);
        assert_eq!(run(0.3, &uncovered).0.unwrap().share, 0.0);
        assert_eq!(run(0.5, &uncovered).0.unwrap().share, 1.0);
    }

    #[test]
    fn steps_follow_the_storage() {
        assert_eq!(storage_step(ChannelFormat::Srgb8), 1.0 / 255.0);
        assert!(storage_step(ChannelFormat::L16F) < storage_step(ChannelFormat::L8));
        assert!(storage_step(ChannelFormat::L16) < storage_step(ChannelFormat::L16F));
        assert_eq!(percent(0.125), "12.5%");
        assert_eq!(percent(0.0), "0.0%");
    }
}
