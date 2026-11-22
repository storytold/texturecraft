//! The safety net: each imported stack composited with TextureCraft's own compositor and compared with the original's
//! stored result (the texture cache), and the per-set decision of what the project shows.
//!
//! - **Real stack**: every channel of the imported layers is exact by its parts and the composite agrees with the stored
//!   result. The layers are the result; the stored result stays as a hidden reference ("Original result (reference)").
//! - **Stored result disagrees**: exact by its parts, but the composite differs beyond rounding (a stale cache, or a
//!   rule TextureCraft's compositor computes otherwise). Arranged like the real stack.
//! - **Not reproduced**: something is not computable (compiled graphs, strokes, …) or approximated. The layers show what
//!   can be computed (the ones that cannot are locked); the stored result is the hidden reference, or shown on top as
//!   "Original result" when the import options ask for it.
//! - **Best effort**: not reproduced and no stored result to fall back on.
//!
//! Differences are in 8-bit steps of the stored encoding (sRGB for colour entries stored sRGB), on the texels of the
//! mesh only (the rest of the stored result is padding).

use texturecraft_compose::eval::{Composite, Inputs, all_tiles, composite_tiles, prepare_texture_set_graphs};
use texturecraft_compose::{BakedMaps, GraphStore, ImageStore, PixelStore};
use texturecraft_core::{Channel, Project};
use texturecraft_geom::{Mesh, NO_TRI, UvGBuffer};
use texturecraft_texel::linear_to_srgb;

use crate::resources::CacheImage;

/// Mean difference at or below this many 8-bit steps counts as reproduced (rounding level).
pub const EXACT_STEPS: f32 = 0.5;
/// Mean difference above which an exact stack counts as disagreeing with the stored result: above rounding, so 8-bit
/// constants rounded the other way (one step) still agree.
pub const AGREEMENT_STEPS: f32 = 1.0;

/// One channel's composite against the stored result.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Score {
    /// Mean absolute difference over the mesh's texels and the channel's components, in 8-bit steps.
    pub mean: f32,
    pub max: f32,
    /// Texels compared.
    pub texels: usize,
}

impl Score {
    pub fn reproduced(&self) -> bool {
        self.mean <= EXACT_STEPS
    }
}

/// What a texture set shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    RealStack,
    Disagrees,
    NotReproduced,
    BestEffort,
}

impl Strategy {
    pub fn id(self) -> &'static str {
        match self {
            Strategy::RealStack => "real-stack",
            Strategy::Disagrees => "stored-result-disagrees",
            Strategy::NotReproduced => "not-reproduced",
            Strategy::BestEffort => "best-effort",
        }
    }

    /// `exact`: every channel is exact by its parts; `agrees`: the composite agrees with the stored result (true when
    /// there is none to compare with).
    pub fn choose(exact: bool, has_result: bool, agrees: bool) -> Strategy {
        match (exact, has_result) {
            (true, true) if !agrees => Strategy::Disagrees,
            (true, _) => Strategy::RealStack,
            (false, true) => Strategy::NotReproduced,
            (false, false) => Strategy::BestEffort,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Strategy::RealStack => "the layers reproduce the original and are the result; the original's stored result kept hidden as a reference",
            Strategy::Disagrees => {
                "every layer is computable, but the composite differs from the original's stored result; the stored result kept hidden as a reference"
            }
            Strategy::NotReproduced => "not every layer is computable: the layers show what can be computed; the original's stored result kept as a reference",
            Strategy::BestEffort => "not every layer is computable and the file holds no stored result: the layers show what can be computed",
        }
    }
}

/// Everything the compositor reads for one set.
pub struct SetScene<'a> {
    pub project: &'a Project,
    pub ts: usize,
    pub mesh: &'a Mesh,
    pub pixels: &'a PixelStore,
    pub baked: &'a BakedMaps,
    pub images: &'a ImageStore,
    pub graphs: &'a GraphStore,
}

/// Composites `channels` of the set (its stack as it stands).
pub fn composite(scene: &SetScene, channels: &[Channel]) -> Option<(Composite, UvGBuffer)> {
    with_inputs(scene, |inputs| {
        let (w, h) = (inputs.gbuf.width, inputs.gbuf.height);
        let mut out = Composite::with_size(channels, w, h);
        composite_tiles(inputs, &mut out, &all_tiles(w, h));
        out
    })
}

/// Prepare graph outputs and their layer inputs before the viewport composes the material.
pub fn prepare_graphs(scene: &SetScene) {
    with_inputs(scene, prepare_texture_set_graphs);
}

fn with_inputs<T>(scene: &SetScene, prepare: impl FnOnce(&Inputs) -> T) -> Option<(T, UvGBuffer)> {
    let set = scene.project.texture_set(scene.ts).ok()?;
    let (w, h) = set.size();
    let gbuf = UvGBuffer::new(scene.mesh, set.material, w, h);
    let inp = Inputs {
        project: scene.project,
        ts: scene.ts,
        mesh: scene.mesh,
        gbuf: &gbuf,
        pixels: scene.pixels,
        baked: scene.baked,
        block_scales: set.block_scales(),
        preview: None,
        images: scene.images,
        graphs: scene.graphs,
        bounds: scene.mesh.bounds(),
    };
    Some((prepare(&inp), gbuf))
}

/// Our composite of `channel` against a stored result image of the same size, on the mesh's texels; `default` is what
/// the original shows where the stored result is uncovered.
pub fn score(ours: &Composite, gbuf: &UvGBuffer, channel: Channel, cache: &CacheImage, default: [f32; 3]) -> Score {
    let comps = channel.components();
    let (w, h) = (cache.image.width.min(ours.resolution), cache.image.height.min(ours.height));
    let encode = |v: f32| if cache.srgb { linear_to_srgb(v.clamp(0.0, 1.0)) } else { v };
    let (mut sum, mut max, mut n) = (0.0f64, 0.0f32, 0usize);
    let mut texels = 0usize;
    for y in 0..h {
        for x in 0..w {
            let on_mesh = gbuf.tri.get((y * gbuf.width + x) as usize).is_some_and(|t| *t != NO_TRI);
            if !on_mesh {
                continue;
            }
            texels += 1;
            let c = cache.image.get(x, y);
            let a = if c[3].is_finite() { c[3].clamp(0.0, 1.0) } else { 0.0 };
            let o = ours.value(channel, x, y);
            for k in 0..comps {
                let stored = c[k] * a + (1.0 - a) * default[k];
                let d = (encode(o[k]) - encode(stored)).abs() * 255.0;
                let d = if d.is_finite() { d } else { 255.0 };
                sum += f64::from(d);
                max = max.max(d);
                n += 1;
            }
        }
    }
    Score { mean: if n == 0 { 0.0 } else { (sum / n as f64) as f32 }, max, texels }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::Rgba;
    use texturecraft_core::{Fill, Layer, LayerKind, Source, TextureSet};

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

    #[test]
    fn a_stack_that_reproduces_the_original_scores_zero_and_others_do_not() {
        let mesh = quad();
        let mut p = Project::new("t", "mesh.glb");
        let mut set = TextureSet::new("S", 0, 128);
        set.channels = vec![(Channel::Roughness, texturecraft_core::ChannelFormat::L8)];
        p.texture_sets.push(set);
        let mut fill = Fill::default();
        fill.sources.insert(Channel::Roughness, Source::Uniform([0.3; 3]));
        let l = Layer::new(p.alloc_id(), "f", LayerKind::Fill(fill), &[Channel::Roughness]);
        p.insert_layer(0, l, None, false).unwrap();
        let (pixels, baked, images) = (PixelStore::default(), BakedMaps::default(), ImageStore::default());
        let scene = SetScene { project: &p, ts: 0, mesh: &mesh, pixels: &pixels, baked: &baked, images: &images, graphs: GraphStore::empty() };
        let (ours, gbuf) = composite(&scene, &[Channel::Roughness]).unwrap();
        let cache = |v: f32, a: f32| CacheImage { bit: 128, image: Rgba::solid(128, 128, [v, v, v, a]), srgb: false, missing_bytes: 0 };
        let same = score(&ours, &gbuf, Channel::Roughness, &cache(0.3, 1.0), [0.3; 3]);
        assert!(same.reproduced() && same.texels > 16000, "{same:?}");
        let off = score(&ours, &gbuf, Channel::Roughness, &cache(0.9, 1.0), [0.3; 3]);
        assert!(off.mean > 100.0, "{off:?}");
        // uncovered texels of the stored result show the channel default (roughness 0.3)
        assert!(score(&ours, &gbuf, Channel::Roughness, &cache(0.0, 0.0), [0.3; 3]).reproduced());
    }

    #[test]
    fn strategies_follow_exactness_and_agreement() {
        assert_eq!(Strategy::choose(true, true, true), Strategy::RealStack);
        assert_eq!(Strategy::choose(true, true, false), Strategy::Disagrees);
        assert_eq!(Strategy::choose(false, true, true), Strategy::NotReproduced);
        assert_eq!(Strategy::choose(false, false, true), Strategy::BestEffort);
        assert_eq!(Strategy::choose(true, false, true), Strategy::RealStack);
    }
}
