//! P4, the box family and distance neighbourhood filters (plan section 2.3). Owned by the P4
//! agent; no other package edits this file or `shaders/neighbourhood_box.wgsl`.
//!
//! What P4 implements: Blur and Bevel `wrap`, Highpass `space` (sRGB from linear or ACEScg) and
//! `alpha`, Pixelate `alpha: false`, the Gradient's `input.blur` pre-blur (a box stage, then P2's
//! gradient), Glow (`HaloGlow`), Drop Shadow, Mask Outline, and the distance filters Bevel
//! (`DistanceBevel`), Bevel Smooth and Directional Distance (separable distance transform with
//! exact integer envelope arithmetic: the CPU computes the crossings in f64). The Distance Map
//! input of Bevel Smooth and Directional Distance waits for P3's inputs: an effect with an image
//! input still composites on the CPU (`effect_needs_cpu`).
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
//! - Wrapping stages ([`wraps`]) set `FilterStage::wrap`: the prepare pass then fills their field
//!   periodically past the set's edges, so the box passes stay local.
//!
//! - The Gradient's `input.blur` (with P2): P4 provides the stage `PRE_BLUR`. Its output keeps the
//!   original premultiplied texel in words 0-3 for the gradient's opacity mix and the blurred gray
//!   in word 4 (`preceding_stage_word`): `GrayscaleMode::gray` of the blurred straight colour for
//!   colour, the blurred first component for other channels and masks. A normal map's gradient is
//!   the identity, so there the stage writes the final value, the blurred normal mixed in by the
//!   opacity, and word 4 stays 0. P2's `lower` queues it with
//!   `ProceduralInputs::precede_with_stage` before the gradient; [`supports`] stays false for it.
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] and `still_on_cpu` in the tests below. Every port gets a CPU
//! oracle test (`crate::gpu_composite::oracle`, `Tolerance::NEIGHBOURHOOD`) on paint and in a
//! mask, chained, and with a partial region (the halo); distance filters also compare the raw
//! distance field exactly.

use texturecraft_core::{
    Bevel, BevelBlur, BevelCorner, BevelDirection, BevelSmooth, ChannelKind, DirectionalDistance, Filter, GrayscaleMode, HighpassSpace, MaskOutline,
};

use super::{
    APPLY_WORKGROUP, BOX_KINDS, BOX_PASSES, DISTANCE_KINDS, Kind, LINE_WORKGROUP, NeighbourhoodFilter, NeighbourhoodGpu, NeighbourhoodRegion, Params,
    StageInputs,
};

/// Library filter ids P4 still leaves on the CPU with their defaults (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &[];

pub(super) const WGSL: &str = include_str!("shaders/neighbourhood_box.wgsl");

/// Pixelate with Alpha off: the cell's colour laid on each texel's own material alpha.
const PIXELATE_KEEPING_ALPHA: Kind = Kind(BOX_KINDS.start);
/// Blur with Wrap: the built-in box passes over a field the prepare pass fills periodically.
const WRAPPED_BLUR: Kind = Kind(BOX_KINDS.start + 1);
/// Bevel with Wrap, as [`WRAPPED_BLUR`].
const WRAPPED_BEVEL: Kind = Kind(BOX_KINDS.start + 2);
/// Highpass of colour in an sRGB space, or of the alpha too.
const HIGHPASS: Kind = Kind(BOX_KINDS.start + 3);
/// The Gradient's input blur, queued by P2 before the gradient: the original texel in words 0-3
/// of the output and the blurred gray in word 4 (a normal map's blur mixed in by the opacity).
const PRE_BLUR: Kind = Kind(BOX_KINDS.start + 4);
/// Glow (`HaloGlow`) and Drop Shadow.
const HALO: Kind = Kind(BOX_KINDS.start + 5);
/// Mask Outline.
const OUTLINE: Kind = Kind(BOX_KINDS.start + 6);
/// Bevel (`DistanceBevel`).
const DISTANCE_BEVEL: Kind = Kind(DISTANCE_KINDS.start);
/// Bevel Smooth.
const BEVEL_SMOOTH: Kind = Kind(DISTANCE_KINDS.start + 1);
/// Directional Distance.
const DIRECTIONAL_DISTANCE: Kind = Kind(DISTANCE_KINDS.start + 2);

/// Flag bits of `NeighbourhoodFilter::words[0]` per kind (the same constants in the WGSL file).
const HALO_INNER: u32 = 1;
const HALO_PRECISE: u32 = 2;
const OUTLINE_FROM_BLACK: u32 = 1;
const OUTLINE_COVERS_INSIDE: u32 = 2;
const OUTLINE_COVERS_OUTSIDE: u32 = 4;
const OUTLINE_BAND_OFF: u32 = 8;
const DISTANCE_ANGULAR: u32 = 1;
const DISTANCE_INWARD: u32 = 2;
const DISTANCE_NON_UNIFORM: u32 = 4;
const SMOOTH_IN: u32 = 8;
const SMOOTH_IN_OUT: u32 = 16;
/// A distance filter that reads past the set's edges from the far side (the shader ignores it).
const DISTANCE_WRAP: u32 = 256;

/// Filter sizes are texels at this many texels per side, as the CPU scales them.
const FILTER_SIZE_RESOLUTION: f32 = 2048.0;
/// The smallest radius a filter runs at, in texels.
const MIN_FILTER_RADIUS: f32 = 0.5;
/// How soft a full spread leaves a halo: its least hardness (`HALO_SOFTEST` in eval.rs).
const HALO_SOFTEST: f32 = 0.01;
/// What a threshold gives way by (`THRESHOLD_TOLERANCE` in core's unit_value.rs).
const THRESHOLD_TOLERANCE: f32 = f32::EPSILON;
/// The base a ramp curve's shape bends by (`CURVE_SHAPE_BASE` in core's ramp_curve.rs).
const CURVE_SHAPE_BASE: f32 = 4.0;
/// The most contrast Directional Distance reaches (`DirectionalDistance::HARDEST`).
const HARDEST_TRAIL_CONTRAST: f32 = 0.99;

/// Work per texel of one bilinear sample, as the built-in Pixelate counts it.
const BILINEAR_SAMPLE_WORK: u64 = 5;
/// Work per texel the built-in stages count for a box filter and its apply pass.
const BOX_FILTER_WORK: u64 = 8;
/// Work per texel of a pass of P4's own (prepare, apply, a distance line or the edges).
const OWN_PASS_WORK: u64 = 2;

/// P4's own compute pipelines, created from the neighbourhood module.
pub(super) struct Pipelines {
    prepare: wgpu::ComputePipeline,
    apply: wgpu::ComputePipeline,
    distance_rows: wgpu::ComputePipeline,
    distance_columns: wgpu::ComputePipeline,
    distance_edges: wgpu::ComputePipeline,
    directional_trail: wgpu::ComputePipeline,
}

impl Pipelines {
    pub(super) fn new(pipeline: &dyn Fn(&'static str) -> wgpu::ComputePipeline) -> Self {
        Self {
            prepare: pipeline("box_family_prepare"),
            apply: pipeline("box_family_apply"),
            distance_rows: pipeline("distance_rows"),
            distance_columns: pipeline("distance_columns"),
            distance_edges: pipeline("distance_edges"),
            directional_trail: pipeline("directional_trail"),
        }
    }
}

/// Whether a P4 stage evaluates `filter` as the CPU does.
/// The Gradient's input blur is no effect of its own: P2 queues [`new`]'s stage for it before the gradient.
pub(super) fn supports(filter: &Filter) -> bool {
    matches!(
        filter,
        Filter::Mosaic { alpha: false, .. }
            | Filter::Blur { wrap: true, .. }
            | Filter::Bevel { wrap: true, .. }
            | Filter::HighPass { .. }
            | Filter::HaloGlow { .. }
            | Filter::DropShadow { .. }
            | Filter::MaskOutline(_)
            | Filter::DistanceBevel(_)
            | Filter::BevelSmooth(_)
            | Filter::DirectionalDistance(_)
    )
}

/// The P4 stage for `filter`, None when its size reaches nothing.
pub(super) fn new(filter: &Filter, stage: StageInputs) -> Option<NeighbourhoodFilter> {
    // Sizes at 2K, scaled as neighbourhood_filters in eval.rs scales them
    let texels = |size: f32| (size * stage.resolution as f32 / FILTER_SIZE_RESOLUTION).max(MIN_FILTER_RADIUS);

    match filter {
        Filter::Mosaic { size, alpha: false } => Some(stage.filter(PIXELATE_KEEPING_ALPHA, texels(*size))),
        Filter::Blur { radius, wrap: true } => (*radius > 0.0).then(|| stage.filter(WRAPPED_BLUR, texels(*radius))),
        // Three box passes ramp an edge over about twice their radius
        Filter::Bevel { width, wrap: true } => Some(stage.filter(WRAPPED_BEVEL, texels(*width * 0.5))),
        Filter::HighPass { radius, space, alpha } => Some(highpass(stage, texels(*radius), *space, *alpha)),
        Filter::GradientMap { input, debug: false, .. } if input.blur > 0.0 => {
            let mut blur = stage.filter(PRE_BLUR, texels(input.blur));
            blur.words[0] = gray_mode(input.mode);
            Some(blur)
        }
        Filter::HaloGlow { on_mask, precise, width, spread, opacity, color } => {
            let flags = flag(*width < 0.0, HALO_INNER) | flag(*precise, HALO_PRECISE);
            Some(halo(stage, texels(width.abs() * Filter::HALO_REACH), flags, (*spread, *opacity), *color, *on_mask, [0; 2]))
        }
        Filter::DropShadow { angle, distance, size, spread, opacity, color } => {
            // Cast away from the light: `distance` of a UV tile, against the light's direction
            let [dx, dy] = direction(*angle);
            let reach = distance.clamp(-Filter::SHADOW_DISTANCE_MAX, Filter::SHADOW_DISTANCE_MAX) * stage.resolution as f32;
            let offset = [(-dx * reach).round() as i32, (-dy * reach).round() as i32];
            Some(halo(stage, texels(*size * Filter::HALO_REACH), 0, (*spread, *opacity), *color, true, offset))
        }
        Filter::MaskOutline(outline) => Some(mask_outline(stage, outline)),
        Filter::DistanceBevel(bevel) => Some(distance_bevel(stage, bevel)),
        Filter::BevelSmooth(smooth) => Some(bevel_smooth(stage, smooth)),
        Filter::DirectionalDistance(trail) => Some(directional_distance(stage, trail)),
        _ => None,
    }
}

fn flag(on: bool, bit: u32) -> u32 {
    if on { bit } else { 0 }
}

/// Unit texel-space vector of `angle` degrees, as `direction` in eval.rs.
fn direction(angle: f32) -> [f32; 2] {
    let (sine, cosine) = angle.to_radians().sin_cos();
    [cosine, -sine]
}

/// A threshold as `at_or_above_threshold` reads it, its tolerance taken off.
fn threshold(value: f32, fallback: f32) -> f32 {
    (if value.is_finite() { value.clamp(0.0, 1.0) } else { fallback }) - THRESHOLD_TOLERANCE
}

/// A ramp curve's offset or shape, as `RampCurve::new` bounds it.
fn bounded_curve(value: f32) -> f32 {
    if value.is_finite() { value.clamp(-1.0, 1.0) } else { 0.0 }
}

/// The exponent a ramp curve's shape bends by.
fn curve_exponent(shape: f32) -> f32 {
    CURVE_SHAPE_BASE.powf(-bounded_curve(shape))
}

/// `GrayscaleMode` as the WGSL `box_gray_of` numbers it.
fn gray_mode(mode: GrayscaleMode) -> u32 {
    match mode {
        GrayscaleMode::Desaturation => 0,
        GrayscaleMode::Luma => 1,
        GrayscaleMode::Average => 2,
        GrayscaleMode::Max => 3,
        GrayscaleMode::Min => 4,
    }
}

/// A Highpass stage: masks filter their value in the working space, content other than colour
/// keeps the working space (filter_region in eval.rs). The built-in stage takes what is left.
fn highpass(stage: StageInputs, texels: f32, space: HighpassSpace, alpha: bool) -> NeighbourhoodFilter {
    let space = if stage.channel == Some(ChannelKind::Color) { space } else { HighpassSpace::Working };
    let alpha = alpha && stage.channel.is_some();
    if space == HighpassSpace::Working && !alpha {
        return stage.filter(Kind::HIGH_PASS, texels);
    }

    let mut highpass = stage.filter(HIGHPASS, texels);
    let space_word = match space {
        HighpassSpace::Working => 0,
        HighpassSpace::SrgbFromLinear => 1,
        HighpassSpace::SrgbFromAcescg => 2,
    };
    highpass.words = [space_word, u32::from(alpha)];
    highpass
}

/// A Glow or Drop Shadow stage (halo in eval.rs): tint, hardness and strength in `extra`, the
/// shadow's offset in `direction` and its texels moved in `words[1]`.
fn halo(
    stage: StageInputs,
    texels: f32,
    flags: u32,
    (spread, strength): (f32, f32),
    color: [f32; 3],
    mask_white: bool,
    offset: [i32; 2],
) -> NeighbourhoodFilter {
    // Rec. 709 luminance, unlike lum's weights on purpose (as halo in eval.rs)
    let luminance = 0.2126 * color[0] + 0.7152 * color[1] + 0.0722 * color[2];
    let tint = match stage.channel {
        Some(ChannelKind::Color) => color,
        Some(_) => [luminance; 3],
        None if mask_white => [1.0; 3],
        None => [luminance; 3],
    };
    let hardness = (1.0 - spread.clamp(0.0, 1.0)).max(HALO_SOFTEST);

    let mut halo = stage.filter(HALO, texels);
    halo.extra[..5].copy_from_slice(&[tint[0], tint[1], tint[2], hardness, strength.clamp(0.0, 1.0)]);
    halo.direction = offset.map(|texels| texels as f32);
    let moved = offset[0].unsigned_abs().max(offset[1].unsigned_abs()).min(stage.longest_side);
    halo.words = [flags, moved];
    halo
}

/// A Mask Outline stage: threshold, blur and fade exponent in `extra`.
fn mask_outline(stage: StageInputs, outline: &MaskOutline) -> NeighbourhoodFilter {
    // Three box passes reach the band's width
    let texels = (outline.width * stage.resolution as f32 / BOX_PASSES as f32).max(MIN_FILTER_RADIUS);
    let flags = flag(outline.from_black, OUTLINE_FROM_BLACK)
        | flag(outline.position.covers(true), OUTLINE_COVERS_INSIDE)
        | flag(outline.position.covers(false), OUTLINE_COVERS_OUTSIDE)
        | flag(!outline.width.is_finite() || outline.width <= 0.0, OUTLINE_BAND_OFF);
    let blur = if outline.blur.is_finite() { outline.blur.clamp(0.0, 1.0) } else { 0.0 };

    let mut stage_filter = stage.filter(OUTLINE, texels);
    stage_filter.extra[..3].copy_from_slice(&[threshold(outline.threshold, MaskOutline::LIBRARY.threshold), blur, curve_exponent(outline.curve)]);
    stage_filter.words[0] = flags;
    stage_filter
}

/// A distance filter's stage: three box passes' reach covers the distance and its smoothing.
fn distance_stage(stage: StageInputs, kind: Kind, distance_reach: f32) -> NeighbourhoodFilter {
    stage.filter(kind, distance_reach / BOX_PASSES as f32 + 1.0)
}

/// A Bevel stage: the edge threshold and the reach in `extra`, the smoothing radius in `words[1]`.
fn distance_bevel(stage: StageInputs, bevel: &Bevel) -> NeighbourhoodFilter {
    let reach = bevel.reach(stage.resolution);
    let smoothing = bevel.smoothing_texels(stage.resolution);
    let flags = flag(bevel.corner == BevelCorner::Angular, DISTANCE_ANGULAR)
        | flag(bevel.inward(), DISTANCE_INWARD)
        | flag(bevel.blur == BevelBlur::NonUniform, DISTANCE_NON_UNIFORM)
        | flag(bevel.wrap, DISTANCE_WRAP);

    let mut stage_filter = distance_stage(stage, DISTANCE_BEVEL, reach + BOX_PASSES as f32 * smoothing);
    stage_filter.extra[..2].copy_from_slice(&[Bevel::EDGE, reach]);
    stage_filter.words = [flags, smoothing.round() as u32];
    stage_filter
}

/// A Bevel Smooth stage: threshold, reach and curve in `extra`, the smoothing radius in `words[1]`.
fn bevel_smooth(stage: StageInputs, smooth: &BevelSmooth) -> NeighbourhoodFilter {
    let reach = smooth.reach(stage.resolution);
    let smoothing = smooth.smoothing_texels(stage.resolution);
    let direction_flag = match smooth.direction {
        BevelDirection::Out => 0,
        BevelDirection::In => SMOOTH_IN,
        BevelDirection::InOut => SMOOTH_IN_OUT,
    };

    let mut stage_filter = distance_stage(stage, BEVEL_SMOOTH, reach + BOX_PASSES as f32 * smoothing);
    stage_filter.extra[..4].copy_from_slice(&[
        threshold(smooth.threshold, BevelSmooth::LIBRARY.threshold),
        reach,
        bounded_curve(smooth.curve_offset),
        curve_exponent(smooth.curve_shape),
    ]);
    stage_filter.words = [direction_flag | flag(smooth.wrap, DISTANCE_WRAP), smoothing.round() as u32];
    stage_filter
}

/// A Directional Distance stage: the steps of `trail_steps` in shape_distance.rs (count, length
/// and direction) and the trail's reach and contrast.
fn directional_distance(stage: StageInputs, trail: &DirectionalDistance) -> NeighbourhoodFilter {
    let reach = trail.reach(stage.resolution);
    let along = direction(trail.angle);

    // Steps of one texel along the direction's longer axis visit every column (or row) the trail crosses
    let longer_axis = along[0].abs().max(along[1].abs());
    let samples = if reach > 0.0 && longer_axis > 0.0 { (reach * longer_axis).ceil() as u32 } else { 0 };
    let step = if samples > 0 { reach / samples as f32 } else { 0.0 };
    let contrast = if trail.contrast.is_finite() { trail.contrast.clamp(0.0, HARDEST_TRAIL_CONTRAST) } else { 0.0 };

    let mut stage_filter = distance_stage(stage, DIRECTIONAL_DISTANCE, reach);
    stage_filter.extra[..4].copy_from_slice(&[DirectionalDistance::EDGE, reach, step, 1.0 - contrast]);
    stage_filter.direction = along;
    stage_filter.words = [samples, 0];
    stage_filter
}

/// Whether the stage reads past the set's edges from the far side (`FilterStage::wrap`).
pub(crate) fn wraps(filter: NeighbourhoodFilter) -> bool {
    match filter.kind {
        WRAPPED_BLUR | WRAPPED_BEVEL => true,
        DISTANCE_BEVEL | BEVEL_SMOOTH => filter.words[0] & DISTANCE_WRAP != 0,
        _ => false,
    }
}

pub(super) fn uses_box(filter: NeighbourhoodFilter) -> bool {
    matches!(filter.kind, WRAPPED_BLUR | WRAPPED_BEVEL | HIGHPASS | PRE_BLUR | HALO | OUTLINE)
}

pub(super) fn reach(filter: NeighbourhoodFilter) -> u32 {
    match filter.kind {
        PIXELATE_KEEPING_ALPHA => filter.radius.saturating_add(1),
        // A shadow also reads the shapes it was moved from
        HALO => filter.radius.saturating_mul(BOX_PASSES as u32).saturating_add(filter.words[1]),
        _ => filter.radius.saturating_mul(BOX_PASSES as u32),
    }
}

pub(super) fn work_per_texel(filter: NeighbourhoodFilter) -> u64 {
    match filter.kind {
        PIXELATE_KEEPING_ALPHA => BILINEAR_SAMPLE_WORK,
        HIGHPASS => BOX_FILTER_WORK + OWN_PASS_WORK,
        PRE_BLUR => BOX_FILTER_WORK + OWN_PASS_WORK,
        HALO => BOX_FILTER_WORK + OWN_PASS_WORK,
        OUTLINE => BOX_FILTER_WORK + OWN_PASS_WORK,
        // Rows, columns, edges, the smoothing's box passes and the apply pass
        DISTANCE_BEVEL | BEVEL_SMOOTH => BOX_FILTER_WORK + 4 * OWN_PASS_WORK,
        // Every texel walks the trail's steps
        DIRECTIONAL_DISTANCE => u64::from(filter.words[0]).saturating_add(2 * OWN_PASS_WORK),
        _ => BOX_FILTER_WORK,
    }
}

/// The three horizontal and three vertical box passes over the field at `first` (read on the
/// first pass as the original is), radius `radius`; the offset of the blurred field.
fn box_passes(
    run: &mut impl FnMut(&wgpu::ComputePipeline, [u32; 4], u32, bool, [u32; 2]),
    box_blur: &wgpu::ComputePipeline,
    region: NeighbourhoodRegion,
    first: u32,
    radius: u32,
) -> u32 {
    let [_, _, width, height] = region.rect;
    let mut source = first;
    for step in 0..2 * BOX_PASSES {
        let vertical = step >= BOX_PASSES;
        let line_count = if vertical { width } else { height };
        let target = region.blur[step % 2];
        run(box_blur, [first, source, target, u32::from(vertical)], radius, step == 0, [line_count.div_ceil(LINE_WORKGROUP), 1]);
        source = target;
    }
    source
}

/// Encodes a P4 stage with passes of its own; false lets the box and apply passes run.
pub(super) fn encode(
    (gpu, pipelines): (&NeighbourhoodGpu, &Pipelines),
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    scratch: &wgpu::Buffer,
    region: NeighbourhoodRegion,
    filter: NeighbourhoodFilter,
) -> bool {
    let [_, _, width, height] = region.rect;
    let texel_groups = [width.div_ceil(APPLY_WORKGROUP), height.div_ceil(APPLY_WORKGROUP)];
    let mut run = |pipeline: &wgpu::ComputePipeline, buffers: [u32; 4], radius: u32, first_box: bool, groups: [u32; 2]| {
        let params = Params {
            rect: region.rect,
            buffers,
            mode: [radius, filter.kind.0, filter.seed, u32::from(first_box)],
            weights: [filter.opacity, filter.amount, filter.frequency, filter.middle],
            direction: [filter.direction[0], filter.direction[1], 0.0, 0.0],
            field_layout: region.field_layout(filter.words),
            extra: filter.extra,
            stage: [filter.context, u32::from(region.wrap), 0, 0],
            bounds: region.bounds,
        };
        NeighbourhoodGpu::run(pipeline, device, encoder, scratch, &params, groups);
    };
    let (original, output) = (region.original, region.output);
    match filter.kind {
        // What the box passes blur goes to the output field first, which the apply pass overwrites
        HIGHPASS | HALO | OUTLINE => {
            run(&pipelines.prepare, [original, 0, output, 0], filter.radius, false, texel_groups);
            let blurred = box_passes(&mut run, &gpu.box_blur, region, output, filter.radius);
            run(&pipelines.apply, [original, blurred, output, 0], filter.radius, false, texel_groups);
        }
        PRE_BLUR => {
            let blurred = box_passes(&mut run, &gpu.box_blur, region, original, filter.radius);
            run(&pipelines.apply, [original, blurred, output, 0], filter.radius, false, texel_groups);
        }
        // Row distances in the first blur field, the envelopes' stacks in the second, the ramp in the output, then its smoothing
        DISTANCE_BEVEL | BEVEL_SMOOTH => {
            let [rows, stacks] = region.blur;
            run(&pipelines.distance_rows, [original, 0, rows, 0], filter.radius, false, [height.div_ceil(LINE_WORKGROUP), 1]);
            run(&pipelines.distance_columns, [original, stacks, rows, 0], filter.radius, false, [width.div_ceil(LINE_WORKGROUP), 1]);
            run(&pipelines.distance_edges, [original, rows, output, 0], filter.radius, false, texel_groups);
            let smoothing = filter.words[1];
            let ramp = if smoothing > 0 { box_passes(&mut run, &gpu.box_blur, region, output, smoothing) } else { output };
            run(&pipelines.apply, [original, ramp, output, 0], filter.radius, false, texel_groups);
        }
        DIRECTIONAL_DISTANCE => {
            let inside = region.blur[0];
            run(&pipelines.prepare, [original, 0, inside, 0], filter.radius, false, texel_groups);
            run(&pipelines.directional_trail, [original, inside, output, 0], filter.radius, false, texel_groups);
            run(&pipelines.apply, [original, output, output, 0], filter.radius, false, texel_groups);
        }
        // Pixelate gathers and the wrapping filters run the built-in passes
        _ => return false,
    }
    true
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::sync::Arc;

    use texturecraft_compose::distance_test_hooks;
    use texturecraft_compose::{BakedMaps, ImageRes, ImageStore, PixelStore};
    use texturecraft_core::{
        BakedMap, Bevel, BevelBlur, BevelCorner, BevelDirection, BevelSmooth, BlendMode, Channel, ChannelKind, ContentsOffset, DirectionalDistance, Effect,
        EffectKind, Filter, Generator, GeneratorKind, GradientInput, GrayscaleMode, HighpassSpace, Layer, LayerId, LayerKind, Mask, MaskBackground,
        MaskOutline, OutlinePosition, Project, Source, TextureSet,
    };
    use texturecraft_geom::glam::Vec2;
    use texturecraft_geom::primitives;
    use texturecraft_texel::{TileImage, to_u16, to_u16_signed};
    use wgpu::util::DeviceExt;

    use super::super::{FIELD_WORDS, NeighbourhoodFilter, NeighbourhoodGpu, NeighbourhoodRegion, Params, StageInputs};
    use crate::gpu_composite::CompositeRegion;
    use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu, compile_effects, effect, headless};
    use crate::headless::Headless;

    /// Side of the oracle sets, in texels.
    const SIZE: u32 = 128;
    /// Side of the sets whose smoothing reaches two texels and more.
    const SMOOTHING_SIZE: u32 = 256;

    /// Cell centres are a hard decision: at most 0.1 % of the texels may flip.
    const CELL_CENTRES: Tolerance = Tolerance::NEIGHBOURHOOD.with_flips((SIZE * SIZE / 1000) as usize);

    /// The library's Glow and Drop Shadow (filter_lib.rs in the engine).
    const LIBRARY_GLOW: Filter = Filter::HaloGlow { on_mask: false, precise: false, width: 0.15, spread: 0.0, opacity: 1.0, color: [0.871, 0.578, 0.188] };
    const LIBRARY_DROP_SHADOW: Filter = Filter::DropShadow { angle: 90.0, distance: 0.1, size: 0.2, spread: 0.0, opacity: 0.5, color: [0.0; 3] };

    /// A stack of (filter, effect opacity, effect blend).
    type Stack = Vec<(Filter, f32, BlendMode)>;

    /// The oracle's layer: paint in the middle or across the set's edges, the set's side, and
    /// whether the mesh leaves a border of the set uncovered.
    type Fixture = (bool, u32, bool);

    /// Paint in the middle of a 128 texel set on a mesh covering it.
    const MIDDLE_PAINT: Fixture = (false, SIZE, false);
    /// Paint across the set's edges.
    const EDGE_PAINT: Fixture = (true, SIZE, false);

    /// The middle of the paint, where the tone and alpha steps meet.
    const MIDDLE: CompositeRegion = CompositeRegion { min: [61, 57], max: [75, 73] };
    /// A corner of the set, whose halo reaches across its edges.
    const CORNER: CompositeRegion = CompositeRegion { min: [120, 2], max: [128, 18] };
    /// [`MIDDLE`] in a [`SMOOTHING_SIZE`] set.
    const SMOOTHING_MIDDLE: CompositeRegion = CompositeRegion { min: [122, 114], max: [150, 146] };

    /// P4's filters and settings that still composite on the CPU, in content and in masks. Port
    /// one: delete its row (this test fails while a ported row stays) and add its oracle test.
    /// The Gradient's input blur compiles once P2's gradient queues P4's stage before it.
    fn still_on_cpu() -> Vec<Filter> {
        vec![Filter::GradientMap {
            colors: vec![[0.0; 3], [1.0; 3]],
            positions: Vec::new(),
            quantity: None,
            input: GradientInput { blur: 2.0, ..Default::default() },
            linear: true,
            debug: false,
        }]
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

    /// Every filter and setting P4 ported compiles for the GPU, in content and in the mask.
    #[test]
    fn box_and_distance_filters_compile_on_the_gpu() {
        let none = ContentsOffset::NONE;
        let ported = [
            Filter::Mosaic { size: 96.0, alpha: false },
            Filter::Blur { radius: 8.0, wrap: true },
            Filter::Bevel { width: 16.0, wrap: true },
            Filter::HighPass { radius: 8.0, space: HighpassSpace::SrgbFromLinear, alpha: false },
            Filter::HighPass { radius: 8.0, space: HighpassSpace::SrgbFromAcescg, alpha: true },
            Filter::HighPass { radius: 8.0, space: HighpassSpace::Working, alpha: true },
            LIBRARY_GLOW,
            LIBRARY_DROP_SHADOW,
            Filter::MaskOutline(MaskOutline::LIBRARY),
            Filter::DistanceBevel(Bevel::LIBRARY),
            Filter::DistanceBevel(Bevel { wrap: true, ..Bevel::LIBRARY }),
            Filter::BevelSmooth(BevelSmooth::LIBRARY),
            Filter::DirectionalDistance(DirectionalDistance::LIBRARY),
        ];

        for filter in ported {
            let (material, _) = compile_effects(Some(effect(EffectKind::Filter(filter.clone()))), None, none);
            let (mask_material, mask) = compile_effects(None, Some(effect(EffectKind::Filter(filter.clone()))), none);

            assert!(material.is_ok() && mask_material.is_ok() && mask.is_ok(), "{filter:?}: {material:?} {mask_material:?} {mask:?}");
        }
    }

    /// A paint layer with Base Color, Roughness and Height painted in two tones (inside and outside
    /// the shapes at 0.5) and two alphas, a mask painted with the same tones, and `stack` after the
    /// content and after the mask's paint, on a set `side` texels across. The paint lies in the
    /// middle, or in the corners across the set's edges. Base Color blends by `channel_blend`.
    fn painted_project(
        stack: &[(Filter, f32, BlendMode)],
        channel_blend: BlendMode,
        (across_edges, side): (bool, u32),
    ) -> Option<(Project, PixelStore, LayerId)> {
        let channels = [Channel::BaseColor, Channel::Roughness, Channel::Height];
        let mut project = Project::new("Box family", "builtin:plane");
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

        // Cells, blurs and distances across the tone and alpha steps and the paint's edges, laid out at 128 texels across
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

    /// The plane, or the plane's UVs shrunk so a border of the set lies off the mesh.
    fn oracle_mesh(uncovered_border: bool) -> Arc<texturecraft_geom::Mesh> {
        let mut mesh = primitives::plane(1.0, 1);
        if uncovered_border {
            for uv in &mut mesh.uvs {
                *uv = *uv * 0.8 + Vec2::splat(0.1);
            }
        }
        Arc::new(mesh)
    }

    /// Composites `project` on the CPU and the GPU: every map, the mask preview of `owner` and a
    /// partial update of `region`.
    fn check_project(
        gpu: &mut Headless,
        (project, pixels, owner): (&Project, &PixelStore, LayerId),
        uncovered_border: bool,
        region: CompositeRegion,
        tolerance: Tolerance,
        label: &str,
    ) {
        let mesh = oracle_mesh(uncovered_border);
        let (baked, images) = (BakedMaps::default(), ImageStore::default());
        let mut case = OracleCase::new(project, pixels, &baked, &images, &mesh);
        case.mask_owner = Some(owner);
        case.region = Some(region);

        assert_matches_cpu(gpu, &case, tolerance, label);
    }

    /// Composites each (label, stack, Base Color blend) on the CPU and the GPU on `fixture`'s layer.
    /// None when a project could not be built.
    fn check_cases(gpu: &mut Headless, cases: &[(&str, Stack, BlendMode)], fixture: Fixture, region: CompositeRegion, tolerance: Tolerance) -> Option<()> {
        let (across_edges, side, uncovered_border) = fixture;
        for (label, stack, channel_blend) in cases {
            let (project, pixels, owner) = painted_project(stack, *channel_blend, (across_edges, side))?;
            check_project(gpu, (&project, &pixels, owner), uncovered_border, region, tolerance, label);
        }
        Some(())
    }

    /// Pixelate with Alpha off matches the CPU in Base Color, Roughness, Height and the mask: at
    /// the library's size, below full opacity under a Multiply layer, set to Disable (the mask
    /// applies it anyway), after a blur and before one, and in a partial update that reads the halo.
    #[test]
    fn pixelate_keeping_alpha_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let pixelate = |size: f32| Filter::Mosaic { size, alpha: false };
        let blur = Filter::Blur { radius: 24.0, wrap: false };
        let normal = BlendMode::Normal;
        let cases = [
            ("Pixelate, library size", vec![(pixelate(Filter::pixel_size(32.0)), 1.0, normal)], normal),
            ("Pixelate under Multiply", vec![(pixelate(160.0), 0.7, normal)], BlendMode::Multiply),
            ("Pixelate disabled", vec![(pixelate(96.0), 1.0, BlendMode::Disable)], normal),
            ("Pixelate after a blur", vec![(blur.clone(), 1.0, normal), (pixelate(96.0), 0.85, normal), (Filter::Invert, 1.0, normal)], normal),
            ("Pixelate before a blur", vec![(Filter::Invert, 1.0, normal), (pixelate(96.0), 0.85, normal), (blur, 1.0, normal)], normal),
        ];

        assert!(check_cases(&mut gpu, &cases, MIDDLE_PAINT, MIDDLE, CELL_CENTRES).is_some());
    }

    /// Blur and Bevel with Wrap match the CPU on paint across the set's edges, whole and in a
    /// corner's partial update: below full opacity under Multiply, disabled, between filters, and
    /// after another blur, whose field the wrapping stage reads past the set's edges.
    #[test]
    fn wrapping_blur_and_bevel_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let blur = Filter::Blur { radius: 160.0, wrap: true };
        let normal = BlendMode::Normal;
        let cases = [
            ("wrapped Blur", vec![(blur.clone(), 1.0, normal)], normal),
            ("wrapped Blur under Multiply", vec![(blur.clone(), 0.7, normal)], BlendMode::Multiply),
            ("wrapped Blur disabled", vec![(blur.clone(), 1.0, BlendMode::Disable)], normal),
            (
                "wrapped Blur between filters",
                vec![(Filter::Invert, 1.0, normal), (blur.clone(), 0.8, normal), (Filter::Clamp { min: 0.18, max: 0.86 }, 1.0, normal)],
                normal,
            ),
            ("wrapped Blur after a blur", vec![(Filter::Blur { radius: 24.0, wrap: false }, 1.0, normal), (blur, 1.0, normal)], normal),
            ("wrapped Bevel", vec![(Filter::Bevel { width: 320.0, wrap: true }, 0.9, normal)], normal),
        ];

        assert!(check_cases(&mut gpu, &cases, EDGE_PAINT, CORNER, Tolerance::NEIGHBOURHOOD).is_some());
    }

    /// A Blur with Wrap after a mask generator's blurred ramp at a coarser resolution (its cells)
    /// matches the CPU near the set's edges.
    #[test]
    fn a_wrapping_blur_after_a_blurred_generator_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let stack = vec![(Filter::Blur { radius: 160.0, wrap: true }, 1.0, BlendMode::Normal)];
        let (mut project, pixels, owner) = painted_project(&stack, BlendMode::Normal, (true, SIZE)).unwrap();
        let mut generator = Generator::new(GeneratorKind::Dirt);
        generator.global.blur = 0.85;
        generator.resolution.factor = -3;
        generator.inputs.insert(BakedMap::AmbientOcclusion, Source::image("Occlusion"));
        let mut generated = Effect::new(project.alloc_id(), EffectKind::Generator(generator));
        generated.blend = BlendMode::Multiply;
        project.layers.get_mut(&owner).and_then(|layer| layer.mask.as_mut()).unwrap().effects.insert(1, generated);

        // An occlusion that changes across the set, so the generator's cells differ
        let occlusion: Vec<[f32; 4]> = (0..64).map(|texel| [0.1 + 0.8 * ((texel % 8 + texel / 8 * 3) % 8) as f32 / 7.0; 4]).collect();
        let images = ImageStore::from([("Occlusion".into(), Arc::new(ImageRes::new(8, 8, occlusion, false)))]);
        let mesh = oracle_mesh(false);
        let baked = BakedMaps::default();
        let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
        case.mask_owner = Some(owner);
        case.region = Some(CORNER);

        assert_matches_cpu(&mut gpu, &case, Tolerance::NEIGHBOURHOOD, "wrapped Blur after a generator's cells");
    }

    /// Highpass in either sRGB space and of the alpha matches the CPU (colour spaces apply to Base
    /// Color only, the alpha to content only), below full opacity, disabled and between filters.
    #[test]
    fn highpass_spaces_and_alpha_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let highpass = |space, alpha| Filter::HighPass { radius: 96.0, space, alpha };
        let normal = BlendMode::Normal;
        let cases = [
            ("Highpass sRGB from linear", vec![(highpass(HighpassSpace::SrgbFromLinear, false), 1.0, normal)], normal),
            ("Highpass sRGB from ACEScg", vec![(highpass(HighpassSpace::SrgbFromAcescg, false), 0.8, normal)], BlendMode::Multiply),
            ("Highpass alpha", vec![(highpass(HighpassSpace::Working, true), 1.0, normal)], normal),
            ("Highpass sRGB and alpha", vec![(highpass(HighpassSpace::SrgbFromLinear, true), 0.7, normal)], normal),
            ("Highpass disabled", vec![(highpass(HighpassSpace::SrgbFromAcescg, true), 1.0, BlendMode::Disable)], normal),
            (
                "Highpass between filters",
                vec![
                    (Filter::Blur { radius: 24.0, wrap: false }, 1.0, normal),
                    (highpass(HighpassSpace::SrgbFromLinear, true), 0.9, normal),
                    (Filter::Invert, 1.0, normal),
                ],
                normal,
            ),
        ];

        assert!(check_cases(&mut gpu, &cases, MIDDLE_PAINT, MIDDLE, Tolerance::NEIGHBOURHOOD).is_some());
    }

    /// A stage's inputs at 2K texels across, as a material channel of `channel` kind or a mask.
    fn stage_at_2k(channel: Option<ChannelKind>, opacity: f32) -> StageInputs {
        StageInputs { resolution: 2048, longest_side: 2048, opacity, channel }
    }

    /// The scratch buffer of four fields (original, two blur fields, output) of `texels`
    /// (premultiplied value and coverage) and their region.
    fn fields_on_gpu(gpu: &Headless, texels: &[([f32; 4], f32)], (width, height): (u32, u32)) -> (wgpu::Buffer, NeighbourhoodRegion) {
        let field = width * height * FIELD_WORDS;
        let mut words = vec![0u32; 4 * field as usize];
        for (texel, (value, coverage)) in words.as_chunks_mut::<{ FIELD_WORDS as usize }>().0.iter_mut().zip(texels) {
            texel[..4].copy_from_slice(&value.map(f32::to_bits));
            texel[4] = coverage.to_bits();
        }
        let scratch = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_box_family_test_fields"),
            contents: bytemuck::cast_slice(&words),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let region = NeighbourhoodRegion {
            rect: [0, 0, width, height],
            original: 0,
            blur: [field, 2 * field],
            output: 3 * field,
            field_words: FIELD_WORDS,
            input: 0,
            wrap: false,
            bounds: [0, 0, width as i32, height as i32],
        };
        (scratch, region)
    }

    /// The four fields' words, texel by texel, after the passes in `encoder`.
    fn read_fields(gpu: &Headless, mut encoder: wgpu::CommandEncoder, scratch: &wgpu::Buffer) -> Vec<[u32; 5]> {
        let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tc_box_family_test_read"),
            size: scratch.size(),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(scratch, 0, &staging, 0, Some(scratch.size()));
        gpu.queue.submit([encoder.finish()]);
        staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = gpu.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        // A failed mapping reads no texels, which the tests' comparisons then report
        let Ok(bytes) = staging.slice(..).get_mapped_range() else { return Vec::new() };
        bytemuck::cast_slice::<u8, u32>(&bytes).as_chunks::<5>().0.to_vec()
    }

    /// The fields after running the stage `filter` over `texels` as the compositor does.
    fn run_stage(gpu: &Headless, texels: &[([f32; 4], f32)], size: (u32, u32), filter: NeighbourhoodFilter) -> Vec<[u32; 5]> {
        let (scratch, region) = fields_on_gpu(gpu, texels, size);
        let kernels = NeighbourhoodGpu::new(&gpu.device);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        kernels.encode(&gpu.device, &mut encoder, &scratch, region, filter);
        read_fields(gpu, encoder, &scratch)
    }

    /// The pre-blur stage keeps the original texel in words 0-3 and leaves in word 4 the gray of
    /// the blurred colour (the Blur stage's) by each of the gradient's modes, its first component
    /// in other channels and masks; a normal map gets the blurred normal mixed in by the opacity.
    #[test]
    fn the_pre_blur_stage_keeps_the_texel_and_leaves_the_blurred_gray() {
        let Some(gpu) = headless() else { return };
        let (width, height) = (40u32, 24u32);
        let texels: Vec<([f32; 4], f32)> = (0..width * height)
            .map(|texel| {
                let (x, y) = ((texel % width) as f32, (texel / width) as f32);
                let alpha = if x < 20.0 { 1.0 } else { 0.55 };
                let color = [0.2 + 0.02 * x, 0.9 - 0.03 * y, if (x + y) % 7.0 < 3.0 { 0.8 } else { 0.1 }];
                let covered = !(x > 30.0 && y < 6.0);
                if covered { ([color[0] * alpha, color[1] * alpha, color[2] * alpha, alpha], 1.0) } else { ([0.0; 4], 0.0) }
            })
            .collect();
        let field = (width * height) as usize;
        let straight = |value: &[f32]| if value[3] > 1e-5 { [value[0] / value[3], value[1] / value[3], value[2] / value[3]] } else { [0.0; 3] };
        let opacity = 0.6;
        let color = Some(ChannelKind::Color);
        let contexts = [
            (color, GrayscaleMode::Desaturation),
            (color, GrayscaleMode::Luma),
            (color, GrayscaleMode::Average),
            (color, GrayscaleMode::Max),
            (color, GrayscaleMode::Min),
            (Some(ChannelKind::Scalar), GrayscaleMode::Average),
            (Some(ChannelKind::Signed), GrayscaleMode::Min),
            (Some(ChannelKind::NormalMap), GrayscaleMode::Luma),
            (None, GrayscaleMode::Luma),
        ];

        for (channel, mode) in contexts {
            let gradient = Filter::GradientMap {
                colors: vec![[0.0; 3], [1.0; 3]],
                positions: Vec::new(),
                quantity: None,
                input: GradientInput { blur: 3.0, mode, ..Default::default() },
                linear: true,
                debug: false,
            };
            let stage = super::new(&gradient, stage_at_2k(channel, opacity)).unwrap();
            let blur = NeighbourhoodFilter::new(&Filter::Blur { radius: 3.0, wrap: false }, 2048, 2048, 1.0, channel).unwrap();

            let staged = run_stage(&gpu, &texels, (width, height), stage);
            let blurred = run_stage(&gpu, &texels, (width, height), blur);

            for (index, ((original, output), blurred)) in texels.iter().zip(&staged[3 * field..]).zip(&blurred[3 * field..]).enumerate() {
                if original.1 <= 0.0 {
                    continue;
                }
                let value: Vec<f32> = output[..4].iter().map(|word| f32::from_bits(*word)).collect();
                let average: Vec<f32> = blurred[..4].iter().map(|word| f32::from_bits(*word)).collect();
                let gray = f32::from_bits(output[4]);
                let label = format!("{channel:?} {mode:?} texel {index}");
                if channel == Some(ChannelKind::NormalMap) {
                    let (before, after) = (straight(&original.0), straight(&average));
                    for component in 0..3 {
                        let mixed = (before[component] + (after[component] - before[component]) * opacity) * original.0[3];
                        assert!((value[component] - mixed).abs() < 1e-5, "{label}: {value:?}");
                    }
                    continue;
                }
                assert_eq!(value, original.0.to_vec(), "{label}: the original texel");
                let expected = match channel {
                    Some(ChannelKind::Color) => mode.gray(straight(&average)),
                    Some(_) => straight(&average)[0],
                    None => average[0],
                };
                assert!((gray - expected).abs() < 1e-5, "{label}: {gray} against {expected}");
            }
        }
    }

    /// Glow and Drop Shadow match the CPU: the library's, an inner precise glow on a mask in white,
    /// a soft spread shadow cast the other way in colour, below full opacity, disabled and chained.
    #[test]
    fn glow_and_drop_shadow_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let inner_glow = Filter::HaloGlow { on_mask: true, precise: true, width: -0.6, spread: 0.4, opacity: 0.8, color: [0.2, 0.7, 0.4] };
        let outer_glow = Filter::HaloGlow { on_mask: false, precise: true, width: 0.8, spread: 0.0, opacity: 1.0, color: [0.9, 0.5, 0.1] };
        let shadow = Filter::DropShadow { angle: 30.0, distance: -0.05, size: 0.6, spread: 0.5, opacity: 0.9, color: [0.2, 0.4, 0.9] };
        let normal = BlendMode::Normal;
        let cases = [
            ("library Glow", vec![(LIBRARY_GLOW, 1.0, normal)], normal),
            ("library Drop Shadow", vec![(LIBRARY_DROP_SHADOW, 1.0, normal)], normal),
            ("inner precise Glow", vec![(inner_glow, 0.75, normal)], BlendMode::Multiply),
            ("outer precise Glow", vec![(outer_glow.clone(), 1.0, normal)], normal),
            ("spread Drop Shadow", vec![(shadow.clone(), 1.0, normal)], normal),
            ("Drop Shadow disabled", vec![(shadow, 1.0, BlendMode::Disable)], normal),
            (
                "Glow between filters",
                vec![(Filter::Invert, 1.0, normal), (outer_glow, 0.8, normal), (Filter::Blur { radius: 24.0, wrap: false }, 1.0, normal)],
                normal,
            ),
        ];

        assert!(check_cases(&mut gpu, &cases, MIDDLE_PAINT, MIDDLE, Tolerance::NEIGHBOURHOOD).is_some());
    }

    /// Mask Outline matches the CPU at every position, on black and over the values, sharp and
    /// faded with a curve, below full opacity, disabled and chained.
    #[test]
    fn mask_outline_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let outline = |position, from_black, blur, curve| Filter::MaskOutline(MaskOutline { position, from_black, threshold: 0.5, width: 0.08, blur, curve });
        let normal = BlendMode::Normal;
        let cases = [
            ("library Mask Outline", vec![(Filter::MaskOutline(MaskOutline::LIBRARY), 1.0, normal)], normal),
            ("Mask Outline outside", vec![(outline(OutlinePosition::Outside, true, 0.0, 0.0), 1.0, normal)], normal),
            ("Mask Outline inside, over the values", vec![(outline(OutlinePosition::Inside, false, 0.5, 0.4), 0.7, normal)], BlendMode::Multiply),
            ("Mask Outline both, faded", vec![(outline(OutlinePosition::Both, true, 0.8, -0.6), 1.0, normal)], normal),
            ("Mask Outline disabled", vec![(outline(OutlinePosition::Both, true, 0.0, 0.0), 1.0, BlendMode::Disable)], normal),
            (
                "Mask Outline after a blur",
                vec![(Filter::Blur { radius: 24.0, wrap: false }, 1.0, normal), (outline(OutlinePosition::Outside, false, 0.3, 0.0), 0.9, normal)],
                normal,
            ),
        ];

        assert!(check_cases(&mut gpu, &cases, MIDDLE_PAINT, MIDDLE, Tolerance::NEIGHBOURHOOD).is_some());
    }

    /// Bevel and Bevel Smooth match the CPU: the library's, every corner, blur, direction and side,
    /// curves, below full opacity, disabled and chained.
    #[test]
    fn bevel_and_bevel_smooth_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let angular = Bevel { corner: BevelCorner::Angular, smoothing: 2.0, blur: BevelBlur::Uniform, ..Bevel::LIBRARY };
        let outward = Bevel { distance: 0.08, smoothing: 1.0, ..Bevel::LIBRARY };
        let inward_smooth = BevelSmooth { direction: BevelDirection::In, curve_offset: 0.2, curve_shape: 0.5, ..BevelSmooth::LIBRARY };
        let across_smooth = BevelSmooth { direction: BevelDirection::InOut, smoothing: 0.0, curve_shape: -0.7, ..BevelSmooth::LIBRARY };
        let normal = BlendMode::Normal;
        let cases = [
            ("library Bevel", vec![(Filter::DistanceBevel(Bevel::LIBRARY), 1.0, normal)], normal),
            ("angular uniform Bevel", vec![(Filter::DistanceBevel(angular), 0.8, normal)], BlendMode::Multiply),
            ("outward Bevel", vec![(Filter::DistanceBevel(outward), 1.0, normal)], normal),
            ("Bevel disabled", vec![(Filter::DistanceBevel(Bevel::LIBRARY), 1.0, BlendMode::Disable)], normal),
            ("library Bevel Smooth", vec![(Filter::BevelSmooth(BevelSmooth::LIBRARY), 1.0, normal)], normal),
            ("inward curved Bevel Smooth", vec![(Filter::BevelSmooth(inward_smooth), 0.7, normal)], normal),
            ("Bevel Smooth across the edge", vec![(Filter::BevelSmooth(across_smooth), 1.0, normal)], normal),
            (
                "Bevel between filters",
                vec![(Filter::Blur { radius: 24.0, wrap: false }, 1.0, normal), (Filter::DistanceBevel(outward), 0.9, normal), (Filter::Invert, 1.0, normal)],
                normal,
            ),
        ];

        assert!(check_cases(&mut gpu, &cases, MIDDLE_PAINT, MIDDLE, Tolerance::NEIGHBOURHOOD).is_some());
    }

    /// The smoothing of Bevel (both blurs) and Bevel Smooth over two texels and more matches the CPU
    /// on a mesh that leaves the set's border uncovered, where Bevel Smooth's Distance Map reads 0.
    #[test]
    fn distance_smoothing_matches_the_cpu_off_the_mesh_too() {
        let Some(mut gpu) = headless() else { return };
        let non_uniform = Bevel { smoothing: 4.0, ..Bevel::LIBRARY };
        let uniform = Bevel { smoothing: 5.0, blur: BevelBlur::Uniform, corner: BevelCorner::Angular, ..Bevel::LIBRARY };
        let smooth = BevelSmooth { smoothing: 8.0, ..BevelSmooth::LIBRARY };
        let normal = BlendMode::Normal;
        let cases = [
            ("smoothed non-uniform Bevel", vec![(Filter::DistanceBevel(non_uniform), 1.0, normal)], normal),
            ("smoothed uniform Bevel", vec![(Filter::DistanceBevel(uniform), 0.8, normal)], BlendMode::Multiply),
            ("smoothed Bevel Smooth", vec![(Filter::BevelSmooth(smooth), 1.0, normal)], normal),
            (
                "smoothed Bevel after a blur",
                vec![(Filter::Blur { radius: 24.0, wrap: false }, 1.0, normal), (Filter::DistanceBevel(non_uniform), 0.9, normal)],
                normal,
            ),
        ];

        assert!(check_cases(&mut gpu, &cases, (false, SMOOTHING_SIZE, true), SMOOTHING_MIDDLE, Tolerance::NEIGHBOURHOOD).is_some());
    }

    /// Bevel and Bevel Smooth with Wrap match the CPU on paint across the set's edges, and a Bevel
    /// without it matches in a program whose wrapping Blur widens every field past the edges.
    #[test]
    fn wrapping_and_mixed_distance_filters_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let normal = BlendMode::Normal;
        let smoothed = Bevel { smoothing: 1.0, ..Bevel::LIBRARY };
        let cases = [
            ("wrapped Bevel", vec![(Filter::DistanceBevel(Bevel { wrap: true, ..smoothed }), 1.0, normal)], normal),
            ("wrapped Bevel Smooth", vec![(Filter::BevelSmooth(BevelSmooth { wrap: true, ..BevelSmooth::LIBRARY }), 0.8, normal)], normal),
            (
                "Bevel after a wrapped Blur",
                vec![(Filter::Blur { radius: 64.0, wrap: true }, 1.0, normal), (Filter::DistanceBevel(smoothed), 1.0, normal)],
                normal,
            ),
        ];

        assert!(check_cases(&mut gpu, &cases, EDGE_PAINT, CORNER, Tolerance::NEIGHBOURHOOD).is_some());
    }

    /// The fields after only P4's own `passes` of `filter` (entry point, then the original, the
    /// field read and the field written by index: 0 original, 1 and 2 the blur fields, 3 the output).
    fn run_passes(gpu: &Headless, texels: &[([f32; 4], f32)], size: (u32, u32), filter: NeighbourhoodFilter, passes: &[(&str, [usize; 3])]) -> Vec<[u32; 5]> {
        let (scratch, region) = fields_on_gpu(gpu, texels, size);
        let offsets = [region.original, region.blur[0], region.blur[1], region.output];
        let kernels = NeighbourhoodGpu::new(&gpu.device);
        let pipelines = &kernels.box_family;
        let texel_groups = [size.0.div_ceil(8), size.1.div_ceil(8)];
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        for (entry, [original, read, written]) in passes {
            let (pipeline, groups) = match *entry {
                "distance_rows" => (&pipelines.distance_rows, [size.1.div_ceil(64), 1]),
                "distance_columns" => (&pipelines.distance_columns, [size.0.div_ceil(64), 1]),
                "distance_edges" => (&pipelines.distance_edges, texel_groups),
                "directional_trail" => (&pipelines.directional_trail, texel_groups),
                _ => (&pipelines.prepare, texel_groups),
            };
            let params = Params {
                rect: region.rect,
                buffers: [offsets[*original], offsets[*read], offsets[*written], 0],
                mode: [filter.radius, filter.kind.0, filter.seed, 0],
                weights: [filter.opacity, filter.amount, filter.frequency, filter.middle],
                direction: [filter.direction[0], filter.direction[1], 0.0, 0.0],
                field_layout: region.field_layout(filter.words),
                extra: filter.extra,
                stage: [filter.context, 0, 0, 0],
                bounds: region.bounds,
            };
            NeighbourhoodGpu::run(pipeline, &gpu.device, &mut encoder, &scratch, &params, groups);
        }
        read_fields(gpu, encoder, &scratch)
    }

    /// Shapes with straight edges, corners, a disc, single texels and gaps over a field wider than tall.
    fn shapes(width: usize, height: usize) -> Vec<bool> {
        let mut seed = 12345u32;
        (0..width * height)
            .map(|texel| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let (x, y) = (texel % width, texel / width);
                let block = (9..31).contains(&x) && (5..22).contains(&y);
                let disc = (x as f32 - 52.0).hypot(y as f32 - 30.0) < 11.5;
                block || disc || (seed >> 16) % 97 < 3
            })
            .collect()
    }

    /// The shapes as a mask field: inside 1, outside 0, every texel covered.
    fn mask_field(inside: &[bool]) -> Vec<([f32; 4], f32)> {
        inside.iter().map(|&inside| ([if inside { 1.0 } else { 0.0 }, 0.0, 0.0, 1.0], 1.0)).collect()
    }

    /// The edge distances of the GPU's integer envelopes and exact square roots equal the CPU's
    /// f64 ones bit for bit, straight and chessboard.
    #[test]
    fn edge_distances_equal_the_cpu_exactly() {
        let Some(gpu) = headless() else { return };
        let (width, height) = (77usize, 53usize);
        let inside = shapes(width, height);
        let field = mask_field(&inside);
        let passes = [("distance_rows", [0, 0, 1]), ("distance_columns", [0, 2, 1]), ("distance_edges", [0, 1, 3])];

        for corner in [BevelCorner::Round, BevelCorner::Angular] {
            let bevel = Filter::DistanceBevel(Bevel { corner, ..Bevel::LIBRARY });
            let filter = super::new(&bevel, stage_at_2k(None, 1.0)).unwrap();

            let fields = run_passes(&gpu, &field, (width as u32, height as u32), filter, &passes);

            let gpu_distances: Vec<u32> = fields[3 * width * height..].iter().map(|texel| texel[1]).collect();
            let cpu_distances: Vec<u32> =
                distance_test_hooks::edge_distances(&inside, (width, height), corner == BevelCorner::Angular).iter().map(|d| d.to_bits()).collect();
            assert_eq!(gpu_distances, cpu_distances, "{corner:?}");
        }
    }

    /// Directional Distance matches the CPU: the library's, at an angle with contrast and a
    /// shorter reach, below full opacity, disabled and chained.
    #[test]
    fn directional_distance_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let trail = DirectionalDistance { angle: 30.0, contrast: 0.6, multiplier: 0.5, ..DirectionalDistance::LIBRARY };
        let normal = BlendMode::Normal;
        let cases = [
            ("library Directional Distance", vec![(Filter::DirectionalDistance(DirectionalDistance::LIBRARY), 1.0, normal)], normal),
            ("hard short Directional Distance", vec![(Filter::DirectionalDistance(trail), 0.85, normal)], BlendMode::Multiply),
            ("Directional Distance disabled", vec![(Filter::DirectionalDistance(trail), 1.0, BlendMode::Disable)], normal),
            (
                "Directional Distance after a blur",
                vec![(Filter::Blur { radius: 24.0, wrap: false }, 1.0, normal), (Filter::DirectionalDistance(trail), 0.9, normal)],
                normal,
            ),
        ];

        assert!(check_cases(&mut gpu, &cases, MIDDLE_PAINT, MIDDLE, Tolerance::NEIGHBOURHOOD).is_some());
    }

    /// Every texel's trail finds the same shape at the same share of the reach as the CPU's.
    #[test]
    fn trail_distances_equal_the_cpu_exactly() {
        let Some(gpu) = headless() else { return };
        let (width, height) = (77usize, 53usize);
        let inside = shapes(width, height);
        let field = mask_field(&inside);
        let passes = [("box_family_prepare", [0, 0, 1]), ("directional_trail", [0, 1, 3])];

        for angle in [0.0, 17.0, 45.0, 90.0, 200.0, 333.0] {
            for distance in [0.0045, 0.02] {
                let trail = Filter::DirectionalDistance(DirectionalDistance { angle, distance, ..DirectionalDistance::LIBRARY });
                let filter = super::new(&trail, stage_at_2k(None, 1.0)).unwrap();
                let (reach, step) = (filter.extra[1], filter.extra[2]);

                let fields = run_passes(&gpu, &field, (width as u32, height as u32), filter, &passes);

                let found: Vec<Option<f32>> = fields[3 * width * height..]
                    .iter()
                    .map(|texel| {
                        let sample = f32::from_bits(texel[1]);
                        (sample >= 0.0).then(|| if reach > 0.0 { sample * step / reach } else { 0.0 })
                    })
                    .collect();
                let expected = distance_test_hooks::trail_distances(&inside, (width, height), filter.direction, reach);
                assert_eq!(found, expected, "{angle}° {distance}");
            }
        }
    }

    /// Composite time of Glow, Drop Shadow and Bevel (the library's) at 2K and 4K, the GPU's (median
    /// of five after a warm-up) against the CPU's, on the oracle's paint:
    /// `cargo test -p texturecraft-render --release --lib bench_box_and_distance_filters -- --ignored --nocapture`
    #[test]
    #[ignore = "benchmark"]
    fn bench_box_and_distance_filters() {
        use std::time::{Duration, Instant};

        use texturecraft_compose::eval::{Composite, Inputs, composite_tiles};
        use texturecraft_geom::UvGBuffer;
        use texturecraft_texel::TILE;

        use crate::gpu_composite::CompositeProgram;

        let Some(mut gpu) = headless() else { return };
        let mesh = oracle_mesh(false);
        let (baked, images) = (BakedMaps::default(), ImageStore::default());
        let filters = [("Glow", LIBRARY_GLOW), ("Drop Shadow", LIBRARY_DROP_SHADOW), ("Bevel", Filter::DistanceBevel(Bevel::LIBRARY))];
        for side in [2048, 4096] {
            gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((side, side), 0)], 1);
            let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, side, side));
            for (label, filter) in &filters {
                let (project, pixels, _) = painted_project(&[(filter.clone(), 1.0, BlendMode::Normal)], BlendMode::Normal, (false, side)).unwrap();
                let mut program = CompositeProgram::try_compile_region(&project, &pixels, 0, None).unwrap();
                program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
                // A halo too large for the adapter's working memory is reported, not timed
                if let Err(error) = gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program) {
                    println!("{label} at {side}: GPU [{}] {error}", gpu.adapter_name);
                    continue;
                }
                let mut gpu_times: Vec<Duration> = (0..5)
                    .map(|_| {
                        let start = Instant::now();
                        let _ = gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program);
                        let _ = gpu.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
                        start.elapsed()
                    })
                    .collect();
                gpu_times.sort_unstable();

                let channels: Vec<Channel> = project.texture_sets[0].channels.iter().map(|&(channel, _)| channel).collect();
                let inputs = Inputs {
                    project: &project,
                    ts: 0,
                    mesh: &mesh,
                    gbuf: &gbuf,
                    pixels: &pixels,
                    baked: &baked,
                    block_scales: project.texture_sets[0].block_scales(),
                    preview: None,
                    images: &images,
                    bounds: mesh.bounds(),
                };
                let tiles: Vec<(u32, u32)> = (0..side / TILE).flat_map(|y| (0..side / TILE).map(move |x| (x, y))).collect();
                let mut oracle = Composite::new(&channels, side);
                let start = Instant::now();
                composite_tiles(&inputs, &mut oracle, &tiles);
                let cpu_time = start.elapsed();

                println!(
                    "{label} at {side}: GPU [{}] median {:?} (range {:?}..{:?}), CPU {:?}",
                    gpu.adapter_name, gpu_times[2], gpu_times[0], gpu_times[4], cpu_time
                );
            }
        }
    }
}
