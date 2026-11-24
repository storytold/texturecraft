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

use texturecraft_core::{Filter, Mirror, MirrorMode, RepeatMode, TileTransform};

use super::{GATHER_KINDS, Kind, NeighbourhoodFilter, NeighbourhoodGpu, NeighbourhoodRegion, StageInputs};

/// Library filter ids P5 still leaves on the CPU with their defaults (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &["warp", "blurSlope", "anisotropicKuwahara", "colorMatch", "heightToNormal"];

pub(super) const WGSL: &str = include_str!("shaders/neighbourhood_gather.wgsl");

/// FXAA: the luma edge search of `fxaa_texel` in eval.rs.
const FXAA: Kind = Kind(GATHER_KINDS.start);
/// Mirror: the texel its UV tile's kept side gives (`mirror_texel` in eval.rs), whole-set.
const MIRROR: Kind = Kind(GATHER_KINDS.start + 4);
/// Transform: the bilinear read its affine gives in its UV tile (`transform_texel`), whole-set.
const TRANSFORM: Kind = Kind(GATHER_KINDS.start + 5);

/// Texels an FXAA edge search walks each way along an edge at most (`FXAA_SEARCH` in eval.rs).
const FXAA_SEARCH: u32 = 8;

/// Flag bit of an FXAA stage's `words[0]` (the same constant in the WGSL file): Affect Alpha Channel.
const FXAA_ALPHA: u32 = 1;

/// Flag bits of a Mirror stage's `words[0]` (the same constants in the WGSL file): which axes it
/// mirrors across, whether each copies its right (bottom) side, and Display Debug.
const MIRROR_X: u32 = 1;
const MIRROR_Y: u32 = 2;
const MIRROR_KEEP_RIGHT: u32 = 4;
const MIRROR_KEEP_BOTTOM: u32 = 8;
const MIRROR_DEBUG: u32 = 16;

/// Filter sizes are texels at this many texels per side, as the CPU scales them.
const FILTER_SIZE_RESOLUTION: f32 = 2048.0;
/// The smallest radius a filter runs at, in texels.
const MIN_FILTER_RADIUS: f32 = 0.5;

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
    matches!(filter, Filter::Fxaa { .. } | Filter::Mirror(_) | Filter::Transform(_))
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
        // A mirror or transform reads anywhere in its UV tile; one that changes nothing is skipped
        Filter::Mirror(mirror) => Some(mirror_stage(stage, mirror)),
        Filter::Transform(transform) if !transform.is_identity() => Some(transform_stage(stage, transform)),
        _ => None,
    }
}

/// A Mirror stage: the axes, the debug line's half width (a share of the tile) and the tile's
/// side in `extra`, its sides in `words[0]`. The CPU's block is the tile's side too.
fn mirror_stage(stage: StageInputs, mirror: &Mirror) -> NeighbourhoodFilter {
    let side = stage.resolution.max(1) as f32;
    let axis = |at: f32| if at.is_finite() { at.clamp(0.0, 1.0) } else { 0.5 };
    let (axes, keep_after, mirrored) = match mirror.mode {
        MirrorMode::AxisX => ([axis(mirror.offset[0]), 0.5], [mirror.invert[0], false], [true, false]),
        MirrorMode::AxisY => ([0.5, axis(mirror.offset[1])], [false, mirror.invert[1]], [false, true]),
        MirrorMode::Corner => {
            let (right, bottom) = mirror.corner.sides();
            ([0.5, 0.5], [right, bottom], [true, true])
        }
    };
    let line = (Mirror::DEBUG_HALF_WIDTH * side / FILTER_SIZE_RESOLUTION).max(MIN_FILTER_RADIUS) / side;

    let mut out = stage.filter(MIRROR, side);
    out.extra[..4].copy_from_slice(&[axes[0], axes[1], line, side]);
    out.words[0] = flag(mirrored[0], MIRROR_X)
        | flag(mirrored[1], MIRROR_Y)
        | flag(keep_after[0], MIRROR_KEEP_RIGHT)
        | flag(keep_after[1], MIRROR_KEEP_BOTTOM)
        | flag(mirror.debug, MIRROR_DEBUG);
    out
}

/// A Transform stage: the affine's columns and shift and the tile's side in `extra`, whether it
/// repeats in `words[0]`.
fn transform_stage(stage: StageInputs, transform: &TileTransform) -> NeighbourhoodFilter {
    let side = stage.resolution.max(1) as f32;
    let (columns, shift) = transform.affine();

    let mut out = stage.filter(TRANSFORM, side);
    out.extra[..7].copy_from_slice(&[columns[0][0], columns[0][1], columns[1][0], columns[1][1], shift[0], shift[1], side]);
    out.words[0] = u32::from(transform.repeat == RepeatMode::HorizontallyAndVertically);
    out
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
        MIRROR => SAMPLE_WORK,
        // The four texels round the read, one at a time
        TRANSFORM => 4 * SAMPLE_WORK,
        _ => 8,
    }
}

/// Whether a stage runs in the whole-set pre-pass instead of in every band (see
/// `NeighbourhoodFilter::whole_set`): large reaches and whole-set reads.
pub(super) fn whole_set(filter: NeighbourhoodFilter, _resolution: u32) -> bool {
    // A mirror or transform reads anywhere in its UV tile
    matches!(filter.kind, MIRROR | TRANSFORM)
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
        BlendMode, Channel, ContentsOffset, Effect, EffectKind, Filter, FlipMode, HeightToNormal, Layer, LayerId, LayerKind, Mask, MaskBackground, Mirror,
        MirrorCorner, MirrorMode, Project, RepeatMode, TextureSet, TileTransform,
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
        let mut filters = vec![Filter::Fxaa { alpha: false }, Filter::Fxaa { alpha: true }];
        filters.extend(mirrors().into_iter().map(Filter::Mirror));
        filters.extend(transforms().into_iter().map(Filter::Transform));
        filters
    }

    /// The library's Mirror and one of every mode, side, corner and the debug axes.
    fn mirrors() -> Vec<Mirror> {
        let library = Mirror::LIBRARY;
        vec![
            library,
            Mirror { invert: [true, false], offset: [0.3, 0.5], ..library },
            Mirror { mode: MirrorMode::AxisY, offset: [0.5, 0.7], ..library },
            Mirror { mode: MirrorMode::AxisY, invert: [false, true], ..library },
            Mirror { mode: MirrorMode::Corner, corner: MirrorCorner::TopRight, ..library },
            Mirror { mode: MirrorMode::Corner, corner: MirrorCorner::BottomLeft, ..library },
            Mirror { mode: MirrorMode::Corner, corner: MirrorCorner::BottomRight, debug: true, ..library },
            Mirror { debug: true, offset: [0.4, 0.5], ..library },
        ]
    }

    /// Transforms that turn, scale, move and flip, repeating and not, tile safe and exact.
    fn transforms() -> Vec<TileTransform> {
        let library = TileTransform::LIBRARY;
        vec![
            TileTransform { rotation: 30.0, tile_safe: false, ..library },
            TileTransform { rotation: 30.0, ..library },
            TileTransform { scaling: [2.0, 0.5], offset: [0.1, -0.2], repeat: RepeatMode::None, ..library },
            TileTransform { scaling: [0.7, 1.3], rotation: 200.0, tile_safe: false, repeat: RepeatMode::None, ..library },
            TileTransform { flip: FlipMode::Horizontally, offset: [0.25, 0.0], ..library },
            TileTransform { flip: FlipMode::HorizontallyAndVertically, rotation: 90.0, ..library },
        ]
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

    /// Mirror matches the CPU in every mode, side, corner and with its debug axes, through the
    /// whole-set pre-pass: below full opacity under Multiply, disabled, between filters and in a
    /// partial update, whose bands read the whole UV tile's result.
    #[test]
    fn mirror_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let normal = BlendMode::Normal;
        let mut cases: Vec<(String, Stack, BlendMode)> =
            mirrors().into_iter().map(|mirror| (format!("{mirror:?}"), vec![(Filter::Mirror(mirror), 1.0, normal)], normal)).collect();
        let library = Filter::Mirror(Mirror::LIBRARY);
        cases.extend([
            ("Mirror under Multiply".to_string(), vec![(library.clone(), 0.7, normal)], BlendMode::Multiply),
            ("Mirror disabled".to_string(), vec![(library.clone(), 1.0, BlendMode::Disable)], normal),
            (
                "Mirror between filters".to_string(),
                vec![(Filter::Blur { radius: 24.0, wrap: false }, 1.0, normal), (library.clone(), 0.85, normal), (Filter::Invert, 1.0, normal)],
                normal,
            ),
            ("Mirror before a blur".to_string(), vec![(Filter::Invert, 1.0, normal), (library, 0.85, normal), (Filter::Blur { radius: 24.0, wrap: false }, 1.0, normal)], normal),
        ]);
        let cases: Vec<(&str, Stack, BlendMode)> = cases.iter().map(|(label, stack, blend)| (label.as_str(), stack.clone(), *blend)).collect();

        assert!(check_cases(&mut gpu, &cases, MIDDLE_PAINT, MIDDLE, Tolerance::NEIGHBOURHOOD).is_some());
    }

    /// Transform matches the CPU turning, scaling, moving and flipping, repeating and not, through
    /// the whole-set pre-pass: below full opacity under Multiply, disabled, between filters and in
    /// a partial update.
    #[test]
    fn transform_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let normal = BlendMode::Normal;
        let mut cases: Vec<(String, Stack, BlendMode)> =
            transforms().into_iter().map(|transform| (format!("{transform:?}"), vec![(Filter::Transform(transform), 1.0, normal)], normal)).collect();
        let turned = Filter::Transform(TileTransform { rotation: 30.0, tile_safe: false, ..TileTransform::LIBRARY });
        cases.extend([
            ("Transform under Multiply".to_string(), vec![(turned.clone(), 0.7, normal)], BlendMode::Multiply),
            ("Transform disabled".to_string(), vec![(turned.clone(), 1.0, BlendMode::Disable)], normal),
            (
                "Transform between filters".to_string(),
                vec![(Filter::Blur { radius: 24.0, wrap: false }, 1.0, normal), (turned.clone(), 0.85, normal), (Filter::Invert, 1.0, normal)],
                normal,
            ),
            ("Transform before a mirror".to_string(), vec![(turned, 1.0, normal), (Filter::Mirror(Mirror::LIBRARY), 0.9, normal)], normal),
        ]);
        let cases: Vec<(&str, Stack, BlendMode)> = cases.iter().map(|(label, stack, blend)| (label.as_str(), stack.clone(), *blend)).collect();

        assert!(check_cases(&mut gpu, &cases, MIDDLE_PAINT, MIDDLE, Tolerance::NEIGHBOURHOOD).is_some());
    }

    /// Mirror and Transform under a wrapping Blur, on paint across the set's edges: the pre-pass
    /// fields hold periodic copies, and each texel reads its tile in its own copy.
    #[test]
    fn mirror_and_transform_under_a_wrapping_blur_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let normal = BlendMode::Normal;
        let wrapped = Filter::Blur { radius: 160.0, wrap: true };
        let turned = Filter::Transform(TileTransform { rotation: 30.0, tile_safe: false, ..TileTransform::LIBRARY });
        let cases = [
            ("Mirror under a wrapping Blur", vec![(Filter::Mirror(Mirror::LIBRARY), 1.0, normal), (wrapped.clone(), 1.0, normal)], normal),
            ("Transform under a wrapping Blur", vec![(turned, 1.0, normal), (wrapped, 1.0, normal)], normal),
        ];

        assert!(check_cases(&mut gpu, &cases, EDGE_PAINT, CORNER, Tolerance::NEIGHBOURHOOD).is_some());
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
