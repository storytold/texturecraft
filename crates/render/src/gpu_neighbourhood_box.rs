//! P4, the box family and distance neighbourhood filters (plan section 2.3). Owned by the P4
//! agent; no other package edits this file or `shaders/neighbourhood_box.wgsl`.
//!
//! What P4 implements: Blur and Bevel `wrap`, Highpass `space` (sRGB from linear or ACEScg) and
//! `alpha`, Pixelate `alpha: false`, the Gradient's `input.blur` pre-blur (a box stage, then P2's
//! gradient), Glow (`HaloGlow`), Drop Shadow, Mask Outline, and the distance filters Bevel
//! (`DistanceBevel`), Bevel Smooth and Directional Distance (separable distance transform with
//! exact integer envelope arithmetic: the CPU computes the crossings in f64). The Distance Map
//! input of Bevel Smooth and Directional Distance reads the field's sixth word (P3's inputs).
//!
//! How:
//! - [`supports`] and [`new`] take every filter the built-in stages reject (the built-in ones keep
//!   their exact settings, so `Blur { wrap: true, .. }` reaches them). Kinds: `BOX_KINDS`
//!   16..24 and `DISTANCE_KINDS` 24..32 (gpu_neighbourhood.rs), as `Kind(n)` constants in this file. `new` returns None
//!   only when the size reaches nothing (the effect then changes nothing, as on the CPU).
//!   `StageInputs::filter` builds the stage; `words` and `extra` carry further parameters.
//! - [`uses_box`], [`reach`] and [`work_per_texel`] answer for P4 kinds only; `uses_box` must
//!   agree with `box_family_uses_box` in the WGSL file. Large halos (Drop Shadow offset) belong
//!   in [`reach`].
//! - [`encode`] runs passes of P4's own (return true), else the box passes and the apply pass run;
//!   the WGSL functions `box_family_filtered`/`box_family_gathered` then compute the texel.
//!   New entry points go in [`Pipelines`].
//!
//! - The Gradient's `input.blur` (with P2): P4 provides the stage (a kind whose output keeps the
//!   original colour for the opacity mix and carries the blurred gray where P2's gradient reads
//!   it); P2's `lower` queues it with `ProceduralInputs::precede_with_stage` before the gradient.
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] and `still_on_cpu` in the tests below. Every port gets a CPU
//! oracle test (`crate::gpu_composite::oracle`, `Tolerance::NEIGHBOURHOOD`) on paint and in a
//! mask, chained, and with a partial region (the halo); distance filters also compare the raw
//! distance field exactly.

use texturecraft_core::Filter;

use super::{BOX_KINDS, Kind, NeighbourhoodFilter, NeighbourhoodGpu, NeighbourhoodRegion, StageInputs};

/// Library filter ids P4 still leaves on the CPU with their defaults (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &["bevel", "bevelSmooth", "directionalDistance", "glow", "dropShadow", "maskOutline"];

pub(super) const WGSL: &str = include_str!("shaders/neighbourhood_box.wgsl");

/// Pixelate with Alpha off: the cell's colour laid on each texel's own material alpha
/// (`PIXELATE_KEEPING_ALPHA` in the WGSL file).
const PIXELATE_KEEPING_ALPHA: Kind = Kind(BOX_KINDS.start);

/// Work per texel of one bilinear sample, as the built-in Pixelate counts it.
const BILINEAR_SAMPLE_WORK: u64 = 5;

/// Work per texel the built-in stages count for a box filter and its apply pass.
const BOX_FILTER_WORK: u64 = 8;

/// P4's own compute pipelines, created from the neighbourhood module.
pub(super) struct Pipelines;

impl Pipelines {
    pub(super) fn new(_pipeline: &dyn Fn(&'static str) -> wgpu::ComputePipeline) -> Self {
        Self
    }
}

/// Whether a P4 stage evaluates `filter` as the CPU does.
pub(super) fn supports(filter: &Filter) -> bool {
    matches!(filter, Filter::Mosaic { alpha: false, .. })
}

/// The P4 stage for `filter`, None when its size reaches nothing.
pub(super) fn new(filter: &Filter, stage: StageInputs) -> Option<NeighbourhoodFilter> {
    match *filter {
        Filter::Mosaic { size, alpha: false } => Some(stage.filter(PIXELATE_KEEPING_ALPHA, stage.texels_at_2k(size))),
        _ => None,
    }
}

pub(super) fn uses_box(_filter: NeighbourhoodFilter) -> bool {
    false
}

pub(super) fn reach(filter: NeighbourhoodFilter) -> u32 {
    filter.radius.saturating_add(1)
}

pub(super) fn work_per_texel(filter: NeighbourhoodFilter) -> u64 {
    if filter.kind == PIXELATE_KEEPING_ALPHA { BILINEAR_SAMPLE_WORK } else { BOX_FILTER_WORK }
}

/// Encodes a P4 stage with passes of its own; false lets the box and apply passes run.
pub(super) fn encode(
    (_gpu, _pipelines): (&NeighbourhoodGpu, &Pipelines),
    _device: &wgpu::Device,
    _encoder: &mut wgpu::CommandEncoder,
    _scratch: &wgpu::Buffer,
    _region: NeighbourhoodRegion,
    _filter: NeighbourhoodFilter,
) -> bool {
    false
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::sync::Arc;

    use texturecraft_compose::{BakedMaps, ImageStore, PixelStore};
    use texturecraft_core::{
        BlendMode, Channel, ContentsOffset, Effect, EffectKind, Filter, GradientInput, HighpassSpace, Layer, LayerId, LayerKind, Mask, MaskBackground, Project,
        TextureSet,
    };
    use texturecraft_geom::primitives;
    use texturecraft_texel::{TileImage, to_u16, to_u16_signed};

    use crate::gpu_composite::CompositeRegion;
    use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu, compile_effects, effect, headless};

    /// Side of the oracle sets, in texels.
    const SIZE: u32 = 128;

    /// Cell centres are a hard decision: at most 0.1 % of the texels may flip.
    const CELL_CENTRES: Tolerance = Tolerance::NEIGHBOURHOOD.with_flips((SIZE * SIZE / 1000) as usize);

    /// P4's filters and settings that still composite on the CPU, in content and in masks. Port
    /// one: delete its row (this test fails while a ported row stays) and add its oracle test.
    fn still_on_cpu() -> Vec<Filter> {
        vec![
            Filter::DropShadow { angle: 45.0, distance: 0.02, size: 8.0, spread: 0.0, opacity: 1.0, color: [0.0; 3] },
            Filter::Blur { radius: 8.0, wrap: true },
            Filter::HighPass { radius: 8.0, space: HighpassSpace::SrgbFromLinear, alpha: false },
            Filter::GradientMap {
                colors: vec![[0.0; 3], [1.0; 3]],
                positions: Vec::new(),
                quantity: None,
                input: GradientInput { blur: 2.0, ..Default::default() },
                linear: true,
                debug: false,
            },
            Filter::Bevel { width: 16.0, wrap: true },
            Filter::HighPass { radius: 8.0, space: HighpassSpace::Working, alpha: true },
        ]
    }

    #[test]
    fn box_and_distance_filters_still_on_the_cpu_fall_back() {
        let none = ContentsOffset::NONE;
        for filter in still_on_cpu() {
            let (material, _) = compile_effects(Some(effect(EffectKind::Filter(filter.clone()))), None, none);
            assert!(material.is_err(), "content {filter:?}");
            let (material, mask) = compile_effects(None, Some(effect(EffectKind::Filter(filter.clone()))), none);
            assert!(material.is_err() && mask.is_err(), "mask {filter:?}");
        }
    }

    /// Pixelate with Alpha off compiles for the GPU, in content and in the mask.
    #[test]
    fn pixelate_keeping_alpha_compiles_on_the_gpu() {
        let filter = Filter::Mosaic { size: 96.0, alpha: false };
        let none = ContentsOffset::NONE;

        let (material, _) = compile_effects(Some(effect(EffectKind::Filter(filter.clone()))), None, none);
        let (mask_material, mask) = compile_effects(None, Some(effect(EffectKind::Filter(filter))), none);

        assert!(material.is_ok() && mask_material.is_ok() && mask.is_ok(), "{material:?} {mask_material:?} {mask:?}");
    }

    /// Pixelate with Alpha off matches the CPU in Base Color, Roughness, Height and the mask: at
    /// the library's size, below full opacity under a Multiply layer, set to Disable (the mask
    /// applies it anyway), after a blur and before one, and in a partial update that reads the halo.
    #[test]
    fn pixelate_keeping_alpha_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let (baked, images) = (BakedMaps::default(), ImageStore::default());

        // A paint layer whose Base Color, Roughness and Height fill a rectangle with an alpha step, a painted mask, and the stack after both
        let painted_layer = |stack: &[(Filter, f32, BlendMode)], channel_blend: BlendMode| -> (Project, PixelStore, LayerId) {
            let channels = [Channel::BaseColor, Channel::Roughness, Channel::Height];
            let mut project = Project::new("Box family", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
            let owner = project.alloc_id();
            let mut layer = Layer::new(owner, "Paint", LayerKind::Paint, &channels);
            layer.channels.get_mut(&Channel::BaseColor).unwrap().mode = channel_blend;
            let mut mask = Mask::new(MaskBackground::Black);
            let mask_paint = project.alloc_id();
            mask.effects.push(Effect::new(mask_paint, EffectKind::Paint));
            for effects in [&mut layer.content_effects, &mut mask.effects] {
                for (filter, opacity, blend) in stack {
                    let mut added = Effect::new(project.alloc_id(), EffectKind::Filter(filter.clone()));
                    added.opacity = *opacity;
                    added.blend = *blend;
                    effects.push(added);
                }
            }
            layer.mask = Some(mask);
            project.insert_layer(0, layer, None, false).unwrap();

            // Cells across the alpha step and the rectangle's edges mix alphas and colours
            let size = (SIZE, SIZE);
            let mut pixels = PixelStore::default();
            let mut mask_pixels = TileImage::new(SIZE, SIZE, 2);
            for y in 22..103 {
                for x in 23..98 {
                    let alpha = if x >= 63 { 1.0 } else { 0.65 };
                    let tone = if y <= 64 { 0.8 } else { 0.4 };
                    let premultiplied = |value: f32| to_u16(value * alpha);
                    let color = [premultiplied(0.7 * tone), premultiplied(0.2 + 0.3 * tone), premultiplied(0.3), to_u16(alpha)];
                    pixels.layer_mut(owner, Channel::BaseColor, size).set(x, y, &color);
                    pixels.layer_mut(owner, Channel::Roughness, size).set(x, y, &[premultiplied(0.9 * tone), to_u16(alpha)]);
                    pixels.layer_mut(owner, Channel::Height, size).set(x, y, &[to_u16_signed((tone - 0.6) * alpha), to_u16(alpha)]);
                    mask_pixels.set(x, y, &[to_u16(tone), u16::MAX]);
                }
            }
            pixels.masks.insert(mask_paint, mask_pixels);
            (project, pixels, owner)
        };
        let pixelate = |size: f32| Filter::Mosaic { size, alpha: false };
        let blur = Filter::Blur { radius: 24.0, wrap: false };
        let normal = BlendMode::Normal;
        let cases = [
            ("library size", vec![(pixelate(Filter::pixel_size(32.0)), 1.0, normal)], normal),
            ("opacity under Multiply", vec![(pixelate(160.0), 0.7, normal)], BlendMode::Multiply),
            ("disabled", vec![(pixelate(96.0), 1.0, BlendMode::Disable)], normal),
            ("after a blur", vec![(blur.clone(), 1.0, normal), (pixelate(96.0), 0.85, normal), (Filter::Invert, 1.0, normal)], normal),
            ("before a blur", vec![(Filter::Invert, 1.0, normal), (pixelate(96.0), 0.85, normal), (blur, 1.0, normal)], normal),
        ];

        for (label, stack, channel_blend) in cases {
            let (project, pixels, owner) = painted_layer(&stack, channel_blend);
            let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
            case.mask_owner = Some(owner);
            case.region = Some(CompositeRegion { min: [61, 57], max: [75, 73] });

            assert_matches_cpu(&mut gpu, &case, CELL_CENTRES, &format!("Pixelate keeping alpha, {label}"));
        }
    }
}
