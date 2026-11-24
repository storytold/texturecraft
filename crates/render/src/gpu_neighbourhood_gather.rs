//! P5, the gather and whole-set neighbourhood filters (plan section 2.3). Owned by the P5 agent;
//! no other package edits this file or `shaders/neighbourhood_gather.wgsl`.
//!
//! What P5 implements: FXAA, Anisotropic Kuwahara (structure tensor pass, then a sector gather),
//! Height To Normal (the layer's own Height in the field's sixth word while filtering Normal),
//! Warp (`SourceWarp`) and Blur Slope (`SourceSlopeBlur`) with every mode, blending and source,
//! Mirror and Transform (whole-UV-tile reach), and Color Match (the whole set's mean: a reduction
//! pass, then a point map). Custom inputs read the sixth word that P3's inputs write.
//!
//! P5 also owns the whole-tile and whole-set field path in `GpuCompositor::compose`
//! (gpu_composite.rs: band planning and the prepare loop; no other package touches `compose`)
//! and may write `params.packages[4]` there. Whole-set fields do not fit the band scratch (plan
//! risk 1): plan a pre-pass that writes the stage result into a texture.
//!
//! How:
//! - [`supports`] and [`new`] take every filter the built-in stages reject. Kinds: `GATHER_KINDS`
//!   32..48 and `WHOLE_SET_KINDS` 48..56 (gpu_neighbourhood.rs), as `Kind(n)` constants in this
//!   file. `new` returns None only when the size reaches nothing. `StageInputs::filter` builds the
//!   stage; `words` and `extra` carry further parameters.
//! - [`uses_box`], [`reach`] and [`work_per_texel`] answer for P5 kinds only; `uses_box` must
//!   agree with `gather_family_uses_box` in the WGSL file.
//! - [`encode`] runs passes of P5's own (return true), else the apply pass calls
//!   `gather_family_gathered`. New entry points go in [`Pipelines`].
//! - Every kernel works out its positions at the texel's home in the set and reads them in the
//!   texel's copy (`gather_begin` in the WGSL file): in a program with a wrapping stage the
//!   fields hold periodic copies past the set's edges, and noise, tiles and mirrors follow the
//!   home position as the CPU's wrapped lower stages do.
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] and `still_on_cpu` in the tests below. Every port gets a CPU
//! oracle test (`crate::gpu_composite::oracle`, `Tolerance::TRANSCENDENTAL`, normals by dot
//! product) on paint and in a mask, chained, and with a partial region.

use texturecraft_core::Filter;

use super::{GATHER_KINDS, Kind, NeighbourhoodFilter, NeighbourhoodGpu, NeighbourhoodRegion, StageInputs};

/// Library filter ids P5 still leaves on the CPU with their defaults (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &["warp", "blurSlope", "anisotropicKuwahara", "mirror", "transform", "colorMatch", "heightToNormal"];

pub(super) const WGSL: &str = include_str!("shaders/neighbourhood_gather.wgsl");

/// FXAA: the luma edge search of `fxaa_texel` in eval.rs.
const FXAA: Kind = Kind(GATHER_KINDS.start);

/// Texels an FXAA edge search walks each way along an edge at most (`FXAA_SEARCH` in eval.rs).
const FXAA_SEARCH: u32 = 8;

/// Flag bit of an FXAA stage's `words[0]` (the same constant in the WGSL file): Affect Alpha Channel.
const FXAA_ALPHA: u32 = 1;

/// Work per texel of one bilinear read, as the built-in gathering stages count it.
const SAMPLE_WORK: u64 = 4;

/// Reads of an FXAA texel at most: the 3 × 3 lumas, both edge walks and the blended read.
const FXAA_READS: u64 = 9 + 2 * FXAA_SEARCH as u64 + 1;

/// P5's own compute pipelines, created from the neighbourhood module.
pub(super) struct Pipelines;

impl Pipelines {
    pub(super) fn new(_pipeline: &dyn Fn(&'static str) -> wgpu::ComputePipeline) -> Self {
        Self
    }
}

fn flag(on: bool, bit: u32) -> u32 {
    if on { bit } else { 0 }
}

/// Whether a P5 stage evaluates `filter` as the CPU does.
pub(super) fn supports(filter: &Filter) -> bool {
    matches!(filter, Filter::Fxaa { .. })
}

/// The P5 stage for `filter`, None when its size reaches nothing.
pub(super) fn new(filter: &Filter, stage: StageInputs) -> Option<NeighbourhoodFilter> {
    match filter {
        // The edge walk and the bilinear read past its end
        Filter::Fxaa { alpha } => {
            let mut fxaa = stage.filter(FXAA, FXAA_SEARCH as f32 + 1.0);
            fxaa.words[0] = flag(*alpha, FXAA_ALPHA);
            Some(fxaa)
        }
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
    match filter.kind {
        FXAA => SAMPLE_WORK * FXAA_READS,
        _ => 8,
    }
}

/// Whether a stage runs in the whole-set pre-pass instead of in every band (see
/// `NeighbourhoodFilter::whole_set`): large reaches and whole-set reads.
pub(super) fn whole_set(_filter: NeighbourhoodFilter, _resolution: u32) -> bool {
    false
}

/// Encodes a P5 stage with passes of its own; false lets the apply pass run.
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
        BlendMode, Channel, ContentsOffset, Effect, EffectKind, Filter, HeightToNormal, Layer, LayerId, LayerKind, Mask, MaskBackground, Project, TextureSet,
    };
    use texturecraft_geom::primitives;
    use texturecraft_texel::{TileImage, to_u16, to_u16_signed};

    use crate::gpu_composite::CompositeRegion;
    use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu, compile_effects, effect, headless};
    use crate::headless::Headless;

    /// Side of the oracle sets, in texels.
    const SIZE: u32 = 128;

    /// The middle of the paint, where the tone and alpha steps meet.
    const MIDDLE: CompositeRegion = CompositeRegion { min: [61, 57], max: [75, 73] };

    /// A corner of the set, whose halo reaches across its edges.
    const CORNER: CompositeRegion = CompositeRegion { min: [120, 2], max: [128, 18] };

    /// Edge searches and blend decisions are hard: at most 0.1 % of the texels may flip.
    const GATHER_DECISIONS: Tolerance = Tolerance::TRANSCENDENTAL.with_flips((SIZE * SIZE / 1000) as usize);

    /// A stack of (filter, effect opacity, effect blend).
    type Stack = Vec<(Filter, f32, BlendMode)>;

    /// The oracle's layer: paint in the middle or across the set's edges, and the set's side.
    type Fixture = (bool, u32);

    /// Paint in the middle of a 128 texel set.
    const MIDDLE_PAINT: Fixture = (false, SIZE);

    /// Paint across the set's edges.
    const EDGE_PAINT: Fixture = (true, SIZE);

    /// P5's filters that still composite on the CPU, in content and in masks. Port one: delete
    /// its row (this test fails while a ported row stays) and add its oracle test.
    fn still_on_cpu() -> Vec<Filter> {
        vec![Filter::HeightToNormal(HeightToNormal::LIBRARY)]
    }

    #[test]
    fn gather_filters_still_on_the_cpu_fall_back() {
        let none = ContentsOffset::NONE;
        for filter in still_on_cpu() {
            let (material, _) = compile_effects(Some(effect(EffectKind::Filter(filter.clone()))), None, none);
            assert!(material.is_err(), "content {filter:?}");
            let (material, mask) = compile_effects(None, Some(effect(EffectKind::Filter(filter.clone()))), none);
            assert!(material.is_err() && mask.is_err(), "mask {filter:?}");
        }
    }

    /// Every filter and setting P5 ported, as content and in the mask.
    fn ported() -> Vec<Filter> {
        vec![Filter::Fxaa { alpha: false }, Filter::Fxaa { alpha: true }]
    }

    /// Every filter and setting P5 ported compiles for the GPU, in content and in the mask.
    #[test]
    fn gather_filters_compile_on_the_gpu() {
        let none = ContentsOffset::NONE;
        for filter in ported() {
            let (material, _) = compile_effects(Some(effect(EffectKind::Filter(filter.clone()))), None, none);
            let (mask_material, mask) = compile_effects(None, Some(effect(EffectKind::Filter(filter.clone()))), none);

            assert!(material.is_ok() && mask_material.is_ok() && mask.is_ok(), "{filter:?}: {material:?} {mask_material:?} {mask:?}");
        }
    }

    /// A paint layer with Base Color, Roughness and Height painted in two tones and two alphas, a
    /// mask painted with the same tones, and `stack` after the content and after the mask's paint,
    /// on a set `side` texels across. The paint lies in the middle, or in the corners across the
    /// set's edges. Base Color blends by `channel_blend`.
    fn painted_project(stack: &[(Filter, f32, BlendMode)], channel_blend: BlendMode, (across_edges, side): Fixture) -> Option<(Project, PixelStore, LayerId)> {
        let channels = [Channel::BaseColor, Channel::Roughness, Channel::Height];
        let mut project = Project::new("Gather family", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, side));
        let owner = project.alloc_id();
        let mut layer = Layer::new(owner, "Paint", LayerKind::Paint, &channels);
        layer.channels.get_mut(&Channel::BaseColor)?.mode = channel_blend;
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
        project.insert_layer(0, layer, None, false).ok()?;

        // Edges, tone and alpha steps laid out at 128 texels across, in colours away from gray
        let size = (side, side);
        let scale = side / SIZE;
        let mut pixels = PixelStore::default();
        let mut mask_pixels = TileImage::new(side, side, 2);
        for y in 0..side {
            for x in 0..side {
                let (at_x, at_y) = (x / scale, y / scale);
                let painted =
                    if across_edges { !(26..104).contains(&at_x) && !(30..100).contains(&at_y) } else { (23..98).contains(&at_x) && (22..103).contains(&at_y) };
                if !painted {
                    continue;
                }
                let (alpha, tone) = if across_edges {
                    (if at_y < 64 { 1.0 } else { 0.65 }, if at_x < 64 { 0.8 } else { 0.4 })
                } else {
                    (if at_x >= 63 { 1.0 } else { 0.65 }, if at_y <= 64 { 0.8 } else { 0.4 })
                };
                // A diagonal stripe gives the edge searches slanted edges to follow
                let tone = if (at_x + 2 * at_y) % 37 < 5 { tone * 0.5 + 0.1 } else { tone };
                let premultiplied = |value: f32| to_u16(value * alpha);
                let color = [premultiplied(tone), premultiplied(0.9 * tone + 0.05), premultiplied(0.6 * tone), to_u16(alpha)];
                pixels.layer_mut(owner, Channel::BaseColor, size).set(x, y, &color);
                pixels.layer_mut(owner, Channel::Roughness, size).set(x, y, &[premultiplied(tone), to_u16(alpha)]);
                pixels.layer_mut(owner, Channel::Height, size).set(x, y, &[to_u16_signed((tone - 0.6) * alpha), to_u16(alpha)]);
                mask_pixels.set(x, y, &[to_u16(tone), u16::MAX]);
            }
        }
        pixels.masks.insert(mask_paint, mask_pixels);
        Some((project, pixels, owner))
    }

    /// Composites each (label, stack, Base Color blend) on the CPU and the GPU on `fixture`'s
    /// layer: every map, the mask preview and a partial update of `region`. None when a project
    /// could not be built.
    fn check_cases(gpu: &mut Headless, cases: &[(&str, Stack, BlendMode)], fixture: Fixture, region: CompositeRegion, tolerance: Tolerance) -> Option<()> {
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let (baked, images) = (BakedMaps::default(), ImageStore::default());
        for (label, stack, channel_blend) in cases {
            let (project, pixels, owner) = painted_project(stack, *channel_blend, fixture)?;
            let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
            case.mask_owner = Some(owner);
            case.region = Some(region);

            assert_matches_cpu(gpu, &case, tolerance, label);
        }
        Some(())
    }

    /// FXAA matches the CPU in Base Color, Roughness, Height and the mask: the library's (alpha
    /// kept), smoothing the alpha too, below full opacity under a Multiply layer, set to Disable
    /// (the mask applies it anyway), after a blur and before one, and in a partial update.
    #[test]
    fn fxaa_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let fxaa = |alpha: bool| Filter::Fxaa { alpha };
        let blur = Filter::Blur { radius: 24.0, wrap: false };
        let normal = BlendMode::Normal;
        let cases = [
            ("FXAA, library", vec![(fxaa(false), 1.0, normal)], normal),
            ("FXAA with alpha", vec![(fxaa(true), 1.0, normal)], normal),
            ("FXAA under Multiply", vec![(fxaa(false), 0.7, normal)], BlendMode::Multiply),
            ("FXAA disabled", vec![(fxaa(true), 1.0, BlendMode::Disable)], normal),
            ("FXAA after a blur", vec![(blur.clone(), 1.0, normal), (fxaa(true), 0.85, normal), (Filter::Invert, 1.0, normal)], normal),
            ("FXAA before a blur", vec![(Filter::Invert, 1.0, normal), (fxaa(false), 0.85, normal), (blur, 1.0, normal)], normal),
        ];

        assert!(check_cases(&mut gpu, &cases, MIDDLE_PAINT, MIDDLE, GATHER_DECISIONS).is_some());
    }

    /// FXAA before a wrapping Blur, on paint across the set's edges: the wrapping program's field
    /// holds periodic copies, and FXAA reads each texel's own copy as the CPU's wrapped lower
    /// stage does.
    #[test]
    fn fxaa_under_a_wrapping_blur_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let normal = BlendMode::Normal;
        let stack = vec![(Filter::Fxaa { alpha: true }, 1.0, normal), (Filter::Blur { radius: 160.0, wrap: true }, 1.0, normal)];

        assert!(check_cases(&mut gpu, &[("FXAA under a wrapping Blur", stack, normal)], EDGE_PAINT, CORNER, GATHER_DECISIONS).is_some());
    }
}
