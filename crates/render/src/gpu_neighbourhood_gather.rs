//! P5, the gather and whole-set neighbourhood filters (plan section 2.3). Owned by the P5 agent;
//! no other package edits this file or `shaders/neighbourhood_gather.wgsl`.
//!
//! What P5 implements: FXAA, Anisotropic Kuwahara (structure tensor pass, then a sector gather),
//! Warp (`SourceWarp`) and Blur Slope (`SourceSlopeBlur`) with every mode, blending and source,
//! Mirror and Transform (whole-UV-tile reach), Color Match (the whole set's mean: two reduction
//! passes, then a point map) and Height To Normal (the layer's own Height in the field's sixth
//! word, `INPUT_LAYER`). A linked image input reaches the kernels through the field's input
//! words (`stage_linked`, `input_at`, `input_color_at`); [`bind_inputs`] says which input an
//! effect registers. An unlinked input reads its default.
//!
//! P5 also owns the whole-tile and whole-set field path in `GpuCompositor::compose`
//! (gpu_composite.rs: band planning and the prepare loop; no other package touches `compose`)
//! and may write `params.packages[4]` there. Stages reading a UV tile or the whole set
//! ([`whole_set`]) run in P0f's whole-set pre-pass (plan risk 1), so no band carries their reach.
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

use texturecraft_core::{
    AnisotropicKuwahara, ChannelKind, ColorMatch, Effect, Filter, HeightToNormal, ImageInput, Mirror, MirrorMode, RepeatMode, SlopeBlending, SourceColorMode, SourceSlopeBlur,
    SourceWarp, TargetColorMode, TileTransform, WarpBlending, WarpMode, WarpSource, WarpSourceMode, finite_unit, oklab,
};

use super::{
    APPLY_WORKGROUP, BUDGET, GATHER_KINDS, Kind, LINE_WORKGROUP, NeighbourhoodFilter, NeighbourhoodGpu, NeighbourhoodRegion, Params, StageInputs, WHOLE_SET_KINDS,
};

/// Library filter ids P5 still leaves on the CPU with their defaults (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &[];

pub(super) const WGSL: &str = include_str!("shaders/neighbourhood_gather.wgsl");

/// FXAA: the luma edge search of `fxaa_texel` in eval.rs.
const FXAA: Kind = Kind(GATHER_KINDS.start);
/// Anisotropic Kuwahara: an ellipse frame per texel from its own structure tensor pass, then the
/// sector gather (`kuwahara_texel` in eval.rs).
const KUWAHARA: Kind = Kind(GATHER_KINDS.start + 1);
/// Warp (`SourceWarp`): where its source sends a texel (`warp_texel.rs`).
const SOURCE_WARP: Kind = Kind(GATHER_KINDS.start + 2);
/// Blur Slope (`SourceSlopeBlur`): the reads downhill of its source (`slope_texel.rs`).
const SOURCE_SLOPE_BLUR: Kind = Kind(GATHER_KINDS.start + 3);
/// Mirror: the texel its UV tile's kept side gives (`mirror_texel` in eval.rs), whole-set.
const MIRROR: Kind = Kind(GATHER_KINDS.start + 4);
/// Transform: the bilinear read its affine gives in its UV tile (`transform_texel`), whole-set.
const TRANSFORM: Kind = Kind(GATHER_KINDS.start + 5);
/// Height To Normal: the Sobel normal of the layer's Height (`height_to_normal_texel` in eval.rs).
const HEIGHT_TO_NORMAL: Kind = Kind(GATHER_KINDS.start + 6);
/// Color Match: every colour moved from its source to its target in OKLab (`color_match` in
/// eval.rs); Average reads the whole set's mean, summed by two reduction passes of its own.
const COLOR_MATCH: Kind = Kind(WHOLE_SET_KINDS.start);

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

/// Flag bit of a Height To Normal stage's `words[0]` (the same constant in the WGSL file):
/// Overwrite Existing Normal.
const HEIGHT_TO_NORMAL_OVERWRITE: u32 = 1;

/// Height To Normal reads its 3 × 3 heights around a texel.
const HEIGHT_TO_NORMAL_READS: u64 = 9;

/// Flag bits of a Color Match stage's `words[0]` (the same constants in the WGSL file): the source
/// is the set's mean, and Custom Color Variation.
const COLOR_MATCH_AVERAGE: u32 = 1;
const COLOR_MATCH_CUSTOM_VARIATION: u32 = 2;
/// The source (Input) or the target (Image Input) a linked input gives at each texel.
const COLOR_MATCH_SOURCE_INPUT: u32 = 4;
const COLOR_MATCH_TARGET_INPUT: u32 = 8;

/// Work per texel of Color Match: its OKLab round trip, and its share of the mean's sums.
const COLOR_MATCH_WORK: u64 = 2 * SAMPLE_WORK;

/// Flag bit of a Kuwahara stage's `words[0]` (the same constant in the WGSL file): Extract
/// Direction Custom Input.
const KUWAHARA_CUSTOM_INPUT: u32 = 1;

/// Sample rows (and columns) on each side of the middle a Kuwahara ellipse reads at most
/// (`KUWAHARA_STEPS` in eval.rs).
const KUWAHARA_STEPS: u64 = 6;

/// Bit fields of a Warp's or Blur Slope's `words[0]` (the same constants in the WGSL file): the
/// warp mode, the blending, the directions, the source's mode, whether the source repeats within
/// the UV tile, and Blur Slope's read count.
const WARP_MODE_SHIFT: u32 = 0;
const BLENDING_SHIFT: u32 = 2;
const DIRECTION_COUNT_SHIFT: u32 = 4;
const SOURCE_MODE_SHIFT: u32 = 8;
const SOURCE_TILED: u32 = 1 << 10;
const READ_COUNT_SHIFT: u32 = 16;

/// Samples along a slope blur at most (`LINE_SAMPLES` in eval.rs).
const LINE_SAMPLES: u32 = 24;

/// The least steepness a source contrast below 1 keeps (`MIN_STEEPNESS` in core's gradient_input.rs).
const MIN_SOURCE_STEEPNESS: f32 = 1e-3;

/// Filter sizes are texels at this many texels per side, as the CPU scales them.
const FILTER_SIZE_RESOLUTION: f32 = 2048.0;
/// The smallest radius a filter runs at, in texels.
const MIN_FILTER_RADIUS: f32 = 0.5;

/// Work per texel of one bilinear read, as the built-in gathering stages count it.
const SAMPLE_WORK: u64 = 4;

/// Reads of an FXAA texel at most: the 3 × 3 lumas, both edge walks and the blended read.
const FXAA_READS: u64 = 9 + 2 * FXAA_SEARCH as u64 + 1;

/// Source reads of a warp or slope blur texel at most: four slope probes, each at its blur's corners.
const SOURCE_READS: u64 = 4 * 4;

/// Gray reads of a Kuwahara texel's structure tensor: a 3 × 3 grid, each with its four sides.
const KUWAHARA_TENSOR_READS: u64 = 9 * 4;

/// Work per Kuwahara ellipse read: the read and its sectors' weights.
const KUWAHARA_SAMPLE_WORK: u64 = 3 * SAMPLE_WORK;

/// P5's own compute pipelines, created from the neighbourhood module.
pub(super) struct Pipelines {
    /// Anisotropic Kuwahara's structure tensor: each texel's ellipse frame.
    kuwahara_frame: wgpu::ComputePipeline,
    /// Color Match's mean: each row's sums, then the rows' total.
    color_match_rows: wgpu::ComputePipeline,
    color_match_total: wgpu::ComputePipeline,
}

impl Pipelines {
    pub(super) fn new(pipeline: &dyn Fn(&'static str) -> wgpu::ComputePipeline) -> Self {
        Self {
            kuwahara_frame: pipeline("gather_kuwahara_frame"),
            color_match_rows: pipeline("gather_color_match_rows"),
            color_match_total: pipeline("gather_color_match_total"),
        }
    }
}

fn flag(on: bool, bit: u32) -> u32 {
    if on { bit } else { 0 }
}

/// Whether a P5 stage evaluates `filter` as the CPU does.
pub(super) fn supports(filter: &Filter) -> bool {
    matches!(
        filter,
        Filter::Fxaa { .. }
            | Filter::AnisotropicKuwahara(_)
            | Filter::SourceWarp(_)
            | Filter::SourceSlopeBlur(_)
            | Filter::Mirror(_)
            | Filter::Transform(_)
            | Filter::ColorMatch(_)
            | Filter::HeightToNormal(_)
    )
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
        // Both the direction search and the ellipse can reach twice the radius
        Filter::AnisotropicKuwahara(kuwahara) if kuwahara.radius > 0.0 => Some(kuwahara_stage(stage, kuwahara)),
        Filter::SourceWarp(warp) if warp.move_texels(stage.resolution) > 0.0 => Some(source_warp(stage, warp)),
        Filter::SourceSlopeBlur(slope) if slope.move_texels(stage.resolution) > 0.0 => Some(source_slope_blur(stage, slope)),
        Filter::ColorMatch(matcher) => Some(color_match_stage(stage, matcher)),
        // The slopes come from the heights right around a texel
        Filter::HeightToNormal(to_normal) => Some(height_to_normal_stage(stage, to_normal)),
        // A mirror or transform reads anywhere in its UV tile; one that changes nothing is skipped
        Filter::Mirror(mirror) => Some(mirror_stage(stage, mirror)),
        Filter::Transform(transform) if !transform.is_identity() => Some(transform_stage(stage, transform)),
        _ => None,
    }
}

/// An Anisotropic Kuwahara stage: the radius, the tensor's reach as a share of it, the sector and
/// sharpness exponents, the Anisotropy and the Angle (radians) in `extra`, the custom input in
/// `words[0]`.
fn kuwahara_stage(stage: StageInputs, kuwahara: &AnisotropicKuwahara) -> NeighbourhoodFilter {
    let radius = (kuwahara.radius.min(AnisotropicKuwahara::MAX_RADIUS) * stage.resolution as f32 / FILTER_SIZE_RESOLUTION).max(MIN_FILTER_RADIUS);
    let tensor_reach = kuwahara.tensor_reach();
    let anisotropy = if kuwahara.anisotropy.is_finite() { kuwahara.anisotropy.clamp(0.0, 1.0) } else { AnisotropicKuwahara::LIBRARY.anisotropy };

    let mut out = stage.filter(KUWAHARA, radius * (2.0 + tensor_reach) + 1.0);
    out.extra[..6].copy_from_slice(&[
        radius,
        tensor_reach,
        kuwahara.sector_exponent(),
        kuwahara.sharpness_exponent(),
        anisotropy,
        kuwahara.angle.to_radians(),
    ]);
    out.words[0] = flag(kuwahara.custom_input, KUWAHARA_CUSTOM_INPUT);
    out
}

/// A Color Match stage. It moves colours only: other channels and masks pass, without the mean.
/// `extra` holds the source's and the target's OKLab references (polar with Custom Color
/// Variation, as `PreparedColorMatch` prepares them) beside the Hue and Chroma limits, `amount` the
/// Luma limit. Average sums the whole set's mean on the GPU; where nothing is covered the CPU falls
/// back to the target, whose reference then stands in for the source's.
fn color_match_stage(stage: StageInputs, matcher: &ColorMatch) -> NeighbourhoodFilter {
    let average = matcher.source_mode == SourceColorMode::Average && stage.channel == Some(ChannelKind::Color);

    // An image input not linked reads its default (filter_region in eval.rs); a linked one is
    // read at the texel ([`bind_inputs`] laid the others in)
    let source = match matcher.source_mode {
        SourceColorMode::Input => [ImageInput::SOURCE_INPUT.default; 3],
        SourceColorMode::Average | SourceColorMode::Parameter => matcher.source_color,
    };
    let target = match matcher.target_mode {
        TargetColorMode::Parameter => matcher.target_color,
        TargetColorMode::ImageInput => [ImageInput::TARGET_INPUT.default; 3],
    };
    let reference = |color: [f32; 3]| {
        let lab = oklab::from_linear(color.map(|component| component.max(0.0)));
        if matcher.custom_variation { [lab[0], lab[1].hypot(lab[2]), lab[2].atan2(lab[1])] } else { lab }
    };
    let limit = |value: f32, most: f32| if value.is_finite() { value.clamp(0.0, most) } else { 0.0 };
    let source = reference(if average { target } else { source });
    let target = reference(target);

    // Average reads the whole set; the other modes and channels the texel alone
    let texels = if average { stage.longest_side as f32 } else { 1.0 };
    let mut out = stage.filter(COLOR_MATCH, texels);
    out.extra = [
        source[0],
        source[1],
        source[2],
        limit(matcher.hue, ColorMatch::MAX_HUE_DEGREES).to_radians(),
        target[0],
        target[1],
        target[2],
        limit(matcher.chroma, ColorMatch::MAX_CHROMA),
    ];
    out.amount = limit(matcher.luma, ColorMatch::MAX_LUMA);
    out.words[0] = flag(average, COLOR_MATCH_AVERAGE)
        | flag(matcher.custom_variation, COLOR_MATCH_CUSTOM_VARIATION)
        | flag(matcher.source_mode == SourceColorMode::Input, COLOR_MATCH_SOURCE_INPUT)
        | flag(matcher.target_mode == TargetColorMode::ImageInput, COLOR_MATCH_TARGET_INPUT);
    out
}

/// A Height To Normal stage: the slope's strength in `extra`, Overwrite in `words[0]`. It reads
/// the layer's Height from its input word (`INPUT_LAYER`), else a flat height of 1 as the CPU
/// does without one; only Normal content changes.
fn height_to_normal_stage(stage: StageInputs, to_normal: &HeightToNormal) -> NeighbourhoodFilter {
    let mut out = stage.filter(HEIGHT_TO_NORMAL, 1.0);
    out.extra[0] = to_normal.strength(stage.resolution);
    out.words[0] = flag(to_normal.overwrite, HEIGHT_TO_NORMAL_OVERWRITE);
    out
}

/// `filter` with the inputs it does not read at its texels laid in (an unlinked input reads its
/// default, a uniform one of Color Match is its colour), and the input it reads at its texels
/// with the kind to read it as (Color Match's in colour). The caller registers that input
/// (`register_input`, or `register_color_input` for colour) in the operation's `effects[3]` and
/// stores the returned filter in place of the effect's. None when two inputs are read at the
/// texels (Color Match's Input and Image Input both linked to non-uniform sources): one stage
/// carries one input, so that effect stays on the CPU.
#[allow(dead_code)] // Called by the stack plumbing once it registers the gather filters' inputs
pub(crate) fn bind_inputs(effect: &Effect, filter: &Filter) -> Option<(Filter, Option<(ImageInput, ChannelKind)>)> {
    let linked = |input: &ImageInput| effect.inputs.get(input.id);
    match filter {
        Filter::ColorMatch(matcher) => {
            let mut matcher = *matcher;
            let mut texel_inputs = Vec::new();
            if matcher.source_mode == SourceColorMode::Input {
                match linked(&ImageInput::SOURCE_INPUT) {
                    None => (matcher.source_mode, matcher.source_color) = (SourceColorMode::Parameter, [ImageInput::SOURCE_INPUT.default; 3]),
                    Some(texturecraft_core::Source::Uniform(color)) => (matcher.source_mode, matcher.source_color) = (SourceColorMode::Parameter, *color),
                    Some(_) => texel_inputs.push(ImageInput::SOURCE_INPUT),
                }
            }
            if matcher.target_mode == TargetColorMode::ImageInput {
                match linked(&ImageInput::TARGET_INPUT) {
                    None => (matcher.target_mode, matcher.target_color) = (TargetColorMode::Parameter, [ImageInput::TARGET_INPUT.default; 3]),
                    Some(texturecraft_core::Source::Uniform(color)) => (matcher.target_mode, matcher.target_color) = (TargetColorMode::Parameter, *color),
                    Some(_) => texel_inputs.push(ImageInput::TARGET_INPUT),
                }
            }
            let input = match texel_inputs.as_slice() {
                [] => None,
                [input] => Some((*input, ChannelKind::Color)),
                _ => return None,
            };
            Some((Filter::ColorMatch(matcher), input))
        }
        filter => {
            let input = ImageInput::of(filter).into_iter().find(|input| linked(input).is_some());
            Some((filter.clone(), input.map(|input| (input, ChannelKind::Scalar))))
        }
    }
}

/// Unit texel-space vector of `angle` degrees, as `direction` in eval.rs.
fn direction(angle: f32) -> [f32; 2] {
    let (sine, cosine) = angle.to_radians().sin_cos();
    [cosine, -sine]
}

/// Whether a warp's source repeats within the UV tile: it then reads anywhere in it.
fn tiled_source(source: &WarpSource) -> bool {
    source.mode != WarpSourceMode::DefaultNoise && source.tiling_factor() != 1.0
}

/// The texels a warp or slope blur reads around a texel (`reach` its own), the whole set where
/// its source repeats within the UV tile (`filter_reach` in eval.rs).
fn source_texels(reach: f32, source: &WarpSource, stage: StageInputs) -> f32 {
    let texels = reach.max(MIN_FILTER_RADIUS);
    if tiled_source(source) { texels.max(stage.longest_side as f32) } else { texels }
}

/// The source of a warp or slope blur: its mode bits, and its reads in `frequency` (the noise's
/// scale per texel, or the Tiling), `middle` (the Balance), `direction` (the slope probe and half
/// the blur) and `words[1]` (the Contrast's steepness as f32 bits, 0 for a step).
fn source_settings(out: &mut NeighbourhoodFilter, source: &WarpSource, resolution: u32) -> u32 {
    let mode = match source.mode {
        WarpSourceMode::DefaultNoise => 0,
        WarpSourceMode::PreviousInput => 1,
        WarpSourceMode::CustomNoise => 2,
    };
    let noise_scale = source.noise_scale();
    let noise_scale = if noise_scale.is_finite() { noise_scale.clamp(Filter::NOISE_SCALE_MIN, Filter::NOISE_SCALE_MAX) } else { 1.0 };
    let contrast = finite_unit(source.contrast);

    out.frequency = if source.mode == WarpSourceMode::DefaultNoise { noise_scale / resolution.max(1) as f32 } else { source.tiling_factor() };
    out.middle = finite_unit(source.balance);
    out.direction = [SourceWarp::probe_texels(resolution), source.blur_texels(resolution) / 2.0];
    out.words[1] = if contrast == 1.0 { 0 } else { (1.0 - contrast).max(MIN_SOURCE_STEEPNESS).to_bits() };
    (mode << SOURCE_MODE_SHIFT) | flag(tiled_source(source), SOURCE_TILED)
}

/// A Warp stage: the move in `amount`, the directions (Warp mode: the slope's measuring span) in
/// `extra`, the source as [`source_settings`] lays it out.
fn source_warp(stage: StageInputs, warp: &SourceWarp) -> NeighbourhoodFilter {
    let resolution = stage.resolution;
    let mode = match warp.mode {
        WarpMode::Warp => 0,
        WarpMode::Directional => 1,
        WarpMode::MultiDirectional => 2,
    };
    let blending = match warp.blending {
        WarpBlending::Average => 0,
        WarpBlending::Max => 1,
        WarpBlending::Min => 2,
        WarpBlending::Chain => 3,
    };
    let directions: Vec<[f32; 2]> = warp.angles().map(direction).collect();

    let mut out = stage.filter(SOURCE_WARP, source_texels(warp.reach(resolution), &warp.source, stage));
    if warp.mode == WarpMode::Warp {
        // The texels the slope is measured across, in UV tiles (slope_move in core's source_warp.rs)
        out.extra[0] = 2.0 * SourceWarp::probe_texels(resolution) / resolution.max(1) as f32;
    }
    for (slot, along) in out.extra.chunks_exact_mut(2).zip(&directions) {
        slot.copy_from_slice(along);
    }
    out.amount = warp.move_texels(resolution);
    out.seed = warp.seed;
    let source = source_settings(&mut out, &warp.source, resolution);
    out.words[0] = (mode << WARP_MODE_SHIFT) | (blending << BLENDING_SHIFT) | ((directions.len() as u32) << DIRECTION_COUNT_SHIFT) | source;
    out
}

/// A Blur Slope stage: the reach downhill in `amount`, the blending and the read count in
/// `words[0]`, the source as [`source_settings`] lays it out.
fn source_slope_blur(stage: StageInputs, slope: &SourceSlopeBlur) -> NeighbourhoodFilter {
    let resolution = stage.resolution;
    let reach = slope.move_texels(resolution);
    let reads = slope.samples((reach.ceil() as u32).clamp(1, LINE_SAMPLES));
    let blending = match slope.blending {
        SlopeBlending::Blur => 0,
        SlopeBlending::Min => 1,
        SlopeBlending::Max => 2,
    };

    let mut out = stage.filter(SOURCE_SLOPE_BLUR, source_texels(slope.reach(resolution), &slope.source, stage));
    out.amount = reach;
    out.seed = slope.seed;
    let source = source_settings(&mut out, &slope.source, resolution);
    out.words[0] = (blending << BLENDING_SHIFT) | (reads << READ_COUNT_SHIFT) | source;
    out
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
        KUWAHARA => SAMPLE_WORK * KUWAHARA_TENSOR_READS + KUWAHARA_SAMPLE_WORK * (2 * KUWAHARA_STEPS + 1).pow(2),
        // The source read at the four corners of its blur round the texel and its slope probes
        SOURCE_WARP => SAMPLE_WORK * (SOURCE_READS + u64::from(SourceWarp::DIRECTIONS[2])),
        SOURCE_SLOPE_BLUR => SAMPLE_WORK * (SOURCE_READS + u64::from(filter.words[0] >> READ_COUNT_SHIFT) + 1),
        MIRROR => SAMPLE_WORK,
        COLOR_MATCH => COLOR_MATCH_WORK,
        HEIGHT_TO_NORMAL => SAMPLE_WORK * HEIGHT_TO_NORMAL_READS,
        // The four texels round the read, one at a time
        TRANSFORM => 4 * SAMPLE_WORK,
        _ => SAMPLE_WORK,
    }
}

/// The working fields a P5 stage's own passes use besides the original and the output
/// (`NeighbourhoodFilter::blur_fields`): Kuwahara's ellipse frames, Color Match's row sums and mean.
pub(super) fn blur_fields(filter: NeighbourhoodFilter) -> u32 {
    match filter.kind {
        KUWAHARA => 1,
        COLOR_MATCH => 2,
        _ => 0,
    }
}

/// Whether a stage runs in the whole-set pre-pass instead of in every band (see
/// `NeighbourhoodFilter::whole_set`): large reaches and whole-set reads.
pub(super) fn whole_set(filter: NeighbourhoodFilter, _resolution: u32) -> bool {
    match filter.kind {
        // A mirror or transform reads anywhere in its UV tile
        MIRROR | TRANSFORM => true,
        // So does a warp whose source repeats within the tile
        SOURCE_WARP | SOURCE_SLOPE_BLUR => filter.words[0] & SOURCE_TILED != 0,
        // Average moves every colour by the whole set's mean
        COLOR_MATCH => filter.words[0] & COLOR_MATCH_AVERAGE != 0,
        _ => false,
    }
}

/// Encodes a P5 stage with passes of its own; false lets the apply pass run.
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
    let params = |buffers: [u32; 4]| Params {
        rect: region.rect,
        buffers,
        mode: [filter.radius, filter.kind.0, filter.seed, 0],
        weights: [filter.opacity, filter.amount, filter.frequency, filter.middle],
        direction: [filter.direction[0], filter.direction[1], 0.0, 0.0],
        field_layout: region.field_layout(filter.words),
        extra: filter.extra,
        stage: [filter.context, u32::from(region.wrap), 0, 0],
        bounds: region.bounds,
    };
    let (original, output) = (region.original, region.output);
    match filter.kind {
        // Each texel's ellipse frame in the first blur field, which the apply pass reads
        KUWAHARA => {
            let frame = region.blur[0];
            let row_work = u64::from(width) * u64::from(APPLY_WORKGROUP) * SAMPLE_WORK * KUWAHARA_TENSOR_READS;
            let frames = params([original, 0, frame, 0]);
            dispatch_in_rows((device, &mut *encoder, scratch), &pipelines.kuwahara_frame, &frames, texel_groups, (APPLY_WORKGROUP, row_work));
            NeighbourhoodGpu::run(&gpu.apply, device, encoder, scratch, &params([original, frame, output, 0]), texel_groups);
        }
        // Each row's sums in the first blur field, their total's mean in the second
        COLOR_MATCH if filter.words[0] & COLOR_MATCH_AVERAGE != 0 => {
            let [rows, mean] = region.blur;
            let row_work = u64::from(width) * u64::from(LINE_WORKGROUP);
            let sums = params([original, 0, rows, 0]);
            dispatch_in_rows((device, &mut *encoder, scratch), &pipelines.color_match_rows, &sums, [1, height.div_ceil(LINE_WORKGROUP)], (LINE_WORKGROUP, row_work));
            NeighbourhoodGpu::run(&pipelines.color_match_total, device, encoder, scratch, &params([original, rows, mean, 0]), [1, 1]);
            NeighbourhoodGpu::run(&gpu.apply, device, encoder, scratch, &params([original, mean, output, 0]), texel_groups);
        }
        // The other kinds gather in the apply pass
        _ => return false,
    }
    true
}

/// Dispatches a P5 pass whose workgroup rows cover `group_rows` texel rows each (the kernel adds
/// `params.stage.z`, the first row): whole, or under a work budget (`NeighbourhoodGpu::budget`,
/// the whole-set pre-pass) in row chunks of at most the budget's work, each submitted on its own.
/// `group_row_work` is the work of one workgroup row.
fn dispatch_in_rows(
    (device, encoder, scratch): (&wgpu::Device, &mut wgpu::CommandEncoder, &wgpu::Buffer),
    pipeline: &wgpu::ComputePipeline,
    params: &Params,
    groups: [u32; 2],
    (group_rows, group_row_work): (u32, u64),
) {
    let budget = BUDGET.with_borrow(|budget| budget.as_ref().map(|budget| (budget.queue.clone(), budget.limit)));
    let Some((queue, limit)) = budget else {
        NeighbourhoodGpu::dispatch(pipeline, device, encoder, scratch, params, groups);
        return;
    };

    let rows = (limit / group_row_work.max(1)).clamp(1, u64::from(groups[1].max(1))) as u32;
    let mut first = 0;
    while first < groups[1] {
        let count = rows.min(groups[1] - first);
        let mut chunk = *params;
        chunk.stage[2] = first * group_rows;
        NeighbourhoodGpu::dispatch(pipeline, device, encoder, scratch, &chunk, [groups[0], count]);
        first += count;
        if rows < groups[1] {
            let finished = std::mem::replace(encoder, device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_gather") }));
            queue.submit([finished.finish()]);
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::sync::Arc;

    use texturecraft_compose::{BakedMaps, ImageStore, PixelStore};
    use texturecraft_core::{
        AnisotropicKuwahara, BlendMode, Channel, ColorMatch, ContentsOffset, Effect, EffectKind, Filter, FlipMode, HeightToNormal, Layer, LayerId, LayerKind,
        Mask, MaskBackground, Mirror, MirrorCorner, MirrorMode, Project, RepeatMode, SlopeBlending, SourceColorMode, SourceSlopeBlur, SourceWarp,
        TargetColorMode, TextureSet, TileTransform, WarpBlending, WarpMode, WarpSource, WarpSourceMode,
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

    /// Side of the Kuwahara sets: the largest radius reaches more than a texel there.
    const KUWAHARA_SIZE: u32 = 256;

    /// [`MIDDLE`] in a [`KUWAHARA_SIZE`] set.
    const KUWAHARA_MIDDLE: CompositeRegion = CompositeRegion { min: [122, 114], max: [150, 146] };

    /// Sector weights fall to zero at the sectors' edges: at most 0.1 % of the texels may flip.
    const KUWAHARA_DECISIONS: Tolerance = Tolerance::TRANSCENDENTAL.with_flips((KUWAHARA_SIZE * KUWAHARA_SIZE / 1000) as usize);

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
        Vec::new()
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
        filters.extend(kuwaharas().into_iter().map(Filter::AnisotropicKuwahara));
        filters.extend(color_matches().into_iter().map(Filter::ColorMatch));
        filters.extend(warps().into_iter().map(Filter::SourceWarp));
        filters.extend(slope_blurs().into_iter().map(Filter::SourceSlopeBlur));
        filters.extend(mirrors().into_iter().map(Filter::Mirror));
        filters.extend(transforms().into_iter().map(Filter::Transform));
        filters.extend(height_to_normals().into_iter().map(Filter::HeightToNormal));
        filters
    }

    /// The library's Height To Normal, laid over the Normal, by Normal Intensity and steeper.
    fn height_to_normals() -> Vec<HeightToNormal> {
        let library = HeightToNormal::LIBRARY;
        vec![
            library,
            HeightToNormal { overwrite: false, ..library },
            HeightToNormal { world_units: false, intensity: 20.0, overwrite: false, ..library },
            HeightToNormal { height_depth: 60.0, surface_size: 120.0, ..library },
        ]
    }

    /// The library's Color Match, each source and target mode (inputs unlinked), with and without
    /// Custom Color Variation, and with narrow limits.
    fn color_matches() -> Vec<ColorMatch> {
        let library = ColorMatch::LIBRARY;
        let blue = [0.1, 0.4, 0.6];
        vec![
            library,
            ColorMatch { custom_variation: false, target_color: [0.2, 0.5, 0.7], ..library },
            ColorMatch { source_mode: SourceColorMode::Parameter, source_color: [0.6, 0.3, 0.2], target_color: blue, custom_variation: false, ..library },
            ColorMatch { source_mode: SourceColorMode::Parameter, source_color: [0.6, 0.3, 0.2], target_color: blue, hue: 20.0, chroma: 0.05, luma: 0.1, ..library },
            ColorMatch { source_mode: SourceColorMode::Input, target_color: blue, ..library },
            ColorMatch { target_mode: TargetColorMode::ImageInput, ..library },
            ColorMatch { target_color: blue, hue: 0.0, chroma: 0.0, luma: 0.0, ..library },
        ]
    }

    /// The library's Anisotropic Kuwahara and each setting away from it.
    fn kuwaharas() -> Vec<AnisotropicKuwahara> {
        let library = AnisotropicKuwahara::LIBRARY;
        let widest = AnisotropicKuwahara { radius: AnisotropicKuwahara::MAX_RADIUS, ..library };
        vec![
            library,
            widest,
            AnisotropicKuwahara { custom_input: true, ..widest },
            AnisotropicKuwahara { smoothness: 1.0, sharpness: 1.0, ..widest },
            AnisotropicKuwahara { smoothness: 0.0, sharpness: 0.6, ..widest },
            AnisotropicKuwahara { tensor_smoothness: 0.6, anisotropy: 0.0, angle: 30.0, ..widest },
            AnisotropicKuwahara { anisotropy: 0.5, angle: -75.0, ..widest },
        ]
    }

    /// A warp moving 6.4 texels at 128 texels across, from its default noise.
    const WIDE_WARP: SourceWarp = SourceWarp { intensity: 0.5, ..SourceWarp::LIBRARY };

    /// Each source mode, shaped (blurred, balanced, contrasted, a hard step) and repeating.
    fn sources() -> Vec<WarpSource> {
        let library = WarpSource::LIBRARY;
        let previous = WarpSource { mode: WarpSourceMode::PreviousInput, ..library };
        vec![
            library,
            WarpSource { blur: 0.4, tiling: 3.0, ..library },
            previous,
            WarpSource { blur: 0.5, balance: 0.6, contrast: 0.3, ..previous },
            WarpSource { contrast: 1.0, ..previous },
            WarpSource { tiling: 2.0, ..previous },
            WarpSource { mode: WarpSourceMode::CustomNoise, ..library },
        ]
    }

    /// The library's Warp, every source in Warp mode, and every mode and blending.
    fn warps() -> Vec<SourceWarp> {
        let mut warps = vec![SourceWarp::LIBRARY];
        warps.extend(sources().into_iter().map(|source| SourceWarp { source, ..WIDE_WARP }));
        let multi = SourceWarp { mode: WarpMode::MultiDirectional, angle: 30.0, ..WIDE_WARP };
        warps.extend([
            SourceWarp { mode: WarpMode::Directional, angle: 30.0, ..WIDE_WARP },
            SourceWarp { mode: WarpMode::Directional, source: WarpSource { mode: WarpSourceMode::PreviousInput, ..WarpSource::LIBRARY }, ..WIDE_WARP },
            multi,
            SourceWarp { blending: WarpBlending::Max, ..multi },
            SourceWarp { blending: WarpBlending::Min, directions: 2, ..multi },
            SourceWarp { blending: WarpBlending::Chain, ..multi },
            SourceWarp { directions: 3, ..multi },
        ]);
        warps
    }

    /// The library's Blur Slope, every source, every blending and fewer reads.
    fn slope_blurs() -> Vec<SourceSlopeBlur> {
        let library = SourceSlopeBlur::LIBRARY;
        let mut slopes: Vec<SourceSlopeBlur> = sources().into_iter().map(|source| SourceSlopeBlur { source, ..library }).collect();
        slopes.extend([
            SourceSlopeBlur { blending: SlopeBlending::Min, ..library },
            SourceSlopeBlur { blending: SlopeBlending::Max, source: WarpSource { mode: WarpSourceMode::PreviousInput, ..WarpSource::LIBRARY }, ..library },
            SourceSlopeBlur { quality: 0.4, intensity: 0.5, ..library },
        ]);
        slopes
    }

    /// The library's Mirror and one of every mode, side, corner and the debug axes.
    fn mirrors() -> Vec<Mirror> {
        let library = Mirror::LIBRARY;
        vec![
            library,
            Mirror { invert: [true, false], offset: [0.3, 0.5], ..library },
            Mirror { mode: MirrorMode::AxisY, offset: [0.5, 0.7], ..library },
            Mirror { mode: MirrorMode::AxisY, invert: [false, true], ..library },
            Mirror { mode: MirrorMode::Corner, corner: MirrorCorner::TopLeft, ..library },
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
            TileTransform { flip: FlipMode::Vertically, offset: [0.0, 0.3], ..library },
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

    /// Labelled single-filter cases, plus opacity below 1 under Multiply, Disable and chains for
    /// `chained`: each filter alone at full opacity, as content and in the mask.
    fn filter_cases(filters: impl IntoIterator<Item = Filter>, chained: &Filter) -> Vec<(String, Stack, BlendMode)> {
        let normal = BlendMode::Normal;
        let blur = Filter::Blur { radius: 24.0, wrap: false };
        let mut cases: Vec<(String, Stack, BlendMode)> = filters.into_iter().map(|filter| (format!("{filter:?}"), vec![(filter, 1.0, normal)], normal)).collect();
        cases.extend([
            (format!("{chained:?} under Multiply"), vec![(chained.clone(), 0.7, normal)], BlendMode::Multiply),
            (format!("{chained:?} disabled"), vec![(chained.clone(), 1.0, BlendMode::Disable)], normal),
            (format!("{chained:?} after a blur"), vec![(blur.clone(), 1.0, normal), (chained.clone(), 0.85, normal), (Filter::Invert, 1.0, normal)], normal),
            (format!("{chained:?} before a blur"), vec![(Filter::Invert, 1.0, normal), (chained.clone(), 0.85, normal), (blur, 1.0, normal)], normal),
        ]);
        cases
    }

    /// Composites `cases` as [`check_cases`] does, with owned labels.
    fn check_labelled(gpu: &mut Headless, cases: &[(String, Stack, BlendMode)], fixture: Fixture, region: CompositeRegion, tolerance: Tolerance) -> Option<()> {
        let cases: Vec<(&str, Stack, BlendMode)> = cases.iter().map(|(label, stack, blend)| (label.as_str(), stack.clone(), *blend)).collect();
        check_cases(gpu, &cases, fixture, region, tolerance)
    }

    /// Color Match matches the CPU in Base Color (other channels and the mask pass): Average from
    /// the whole set's mean, summed by its reduction passes in the whole-set pre-pass, and the
    /// other modes, with and without Custom Color Variation: below full opacity under Multiply,
    /// disabled, between filters (the blur below runs in the pre-pass too) and in a partial update.
    #[test]
    fn color_match_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let cases = filter_cases(color_matches().into_iter().map(Filter::ColorMatch), &Filter::ColorMatch(ColorMatch::LIBRARY));

        assert!(check_labelled(&mut gpu, &cases, MIDDLE_PAINT, MIDDLE, Tolerance::TRANSCENDENTAL).is_some());
    }

    /// Color Match's Average over a layer that covers nothing falls back to its target, as the CPU
    /// does: the mean's passes leave no texel changed, every map and the mask match.
    #[test]
    fn color_match_over_nothing_covered_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let normal = BlendMode::Normal;
        let stack = vec![(Filter::ColorMatch(ColorMatch::LIBRARY), 1.0, normal)];
        let (project, _, owner) = painted_project(&stack, normal, MIDDLE_PAINT).unwrap();
        let (pixels, baked, images) = (PixelStore::default(), BakedMaps::default(), ImageStore::default());
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
        case.mask_owner = Some(owner);
        case.region = Some(MIDDLE);

        assert_matches_cpu(&mut gpu, &case, Tolerance::TRANSCENDENTAL, "Color Match over nothing covered");
    }

    /// Color Match and Anisotropic Kuwahara under a wrapping Blur, on paint across the set's
    /// edges: the mean counts each texel of the set once, and the ellipses read each texel's copy.
    #[test]
    fn color_match_and_kuwahara_under_a_wrapping_blur_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let normal = BlendMode::Normal;
        let wrapped = Filter::Blur { radius: 160.0, wrap: true };
        let widest = AnisotropicKuwahara { radius: AnisotropicKuwahara::MAX_RADIUS, ..AnisotropicKuwahara::LIBRARY };
        let cases = [
            ("Color Match under a wrapping Blur", vec![(Filter::ColorMatch(ColorMatch::LIBRARY), 1.0, normal), (wrapped.clone(), 1.0, normal)], normal),
            ("Kuwahara under a wrapping Blur", vec![(Filter::AnisotropicKuwahara(widest), 1.0, normal), (wrapped, 1.0, normal)], normal),
        ];

        assert!(check_cases(&mut gpu, &cases, EDGE_PAINT, CORNER, GATHER_DECISIONS).is_some());
    }

    /// Anisotropic Kuwahara matches the CPU through its structure tensor pass, at the library's
    /// settings and each setting away from them (the custom input unlinked): below full opacity
    /// under Multiply, disabled, between filters and in a partial update.
    #[test]
    fn anisotropic_kuwahara_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let widest = AnisotropicKuwahara { radius: AnisotropicKuwahara::MAX_RADIUS, ..AnisotropicKuwahara::LIBRARY };
        let cases = filter_cases(kuwaharas().into_iter().map(Filter::AnisotropicKuwahara), &Filter::AnisotropicKuwahara(widest));

        assert!(check_labelled(&mut gpu, &cases, (false, KUWAHARA_SIZE), KUWAHARA_MIDDLE, KUWAHARA_DECISIONS).is_some());
    }

    /// Warp matches the CPU from every source (default noise, the values it filters and an
    /// unlinked Custom Noise; blurred, balanced, contrasted, stepped and repeating in the tile,
    /// whose stages run in the whole-set pre-pass) in Warp mode, and in every mode and blending:
    /// below full opacity under Multiply, disabled, between filters and in a partial update. A
    /// stepped source is a hard decision.
    #[test]
    fn warp_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let cases = filter_cases(warps().into_iter().map(Filter::SourceWarp), &Filter::SourceWarp(WIDE_WARP));

        assert!(check_labelled(&mut gpu, &cases, MIDDLE_PAINT, MIDDLE, GATHER_DECISIONS).is_some());
    }

    /// Blur Slope matches the CPU from every source, in every blending and with fewer reads: below
    /// full opacity under Multiply, disabled, between filters and in a partial update.
    #[test]
    fn blur_slope_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let cases = filter_cases(slope_blurs().into_iter().map(Filter::SourceSlopeBlur), &Filter::SourceSlopeBlur(SourceSlopeBlur::LIBRARY));

        assert!(check_labelled(&mut gpu, &cases, MIDDLE_PAINT, MIDDLE, GATHER_DECISIONS).is_some());
    }

    /// Warp and Blur Slope under a wrapping Blur, on paint across the set's edges: their noise and
    /// repeating sources follow each texel's home position, as the CPU's wrapped lower stages do.
    #[test]
    fn warp_and_blur_slope_under_a_wrapping_blur_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let normal = BlendMode::Normal;
        let wrapped = Filter::Blur { radius: 160.0, wrap: true };
        let repeating = WarpSource { mode: WarpSourceMode::PreviousInput, tiling: 2.0, ..WarpSource::LIBRARY };
        let cases = [
            ("Warp under a wrapping Blur", vec![(Filter::SourceWarp(WIDE_WARP), 1.0, normal), (wrapped.clone(), 1.0, normal)], normal),
            (
                "repeating Warp under a wrapping Blur",
                vec![(Filter::SourceWarp(SourceWarp { source: repeating, ..WIDE_WARP }), 1.0, normal), (wrapped.clone(), 1.0, normal)],
                normal,
            ),
            ("Blur Slope under a wrapping Blur", vec![(Filter::SourceSlopeBlur(SourceSlopeBlur::LIBRARY), 1.0, normal), (wrapped, 1.0, normal)], normal),
        ];

        assert!(check_cases(&mut gpu, &cases, EDGE_PAINT, CORNER, GATHER_DECISIONS).is_some());
    }

    /// Mirror matches the CPU in every mode, side, corner and with its debug axes, through the
    /// whole-set pre-pass: below full opacity under Multiply, disabled, between filters and in a
    /// partial update, whose bands read the whole UV tile's result.
    #[test]
    fn mirror_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let cases = filter_cases(mirrors().into_iter().map(Filter::Mirror), &Filter::Mirror(Mirror::LIBRARY));

        assert!(check_labelled(&mut gpu, &cases, MIDDLE_PAINT, MIDDLE, Tolerance::NEIGHBOURHOOD).is_some());
    }

    /// Transform matches the CPU turning, scaling, moving and flipping, repeating and not, through
    /// the whole-set pre-pass: below full opacity under Multiply, disabled, between filters, before
    /// a mirror and in a partial update.
    #[test]
    fn transform_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let normal = BlendMode::Normal;
        let turned = Filter::Transform(TileTransform { rotation: 30.0, tile_safe: false, ..TileTransform::LIBRARY });
        let mut cases = filter_cases(transforms().into_iter().map(Filter::Transform), &turned);
        cases.push(("Transform before a mirror".to_string(), vec![(turned, 1.0, normal), (Filter::Mirror(Mirror::LIBRARY), 0.9, normal)], normal));

        assert!(check_labelled(&mut gpu, &cases, MIDDLE_PAINT, MIDDLE, Tolerance::NEIGHBOURHOOD).is_some());
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

    /// A paint layer with Base Color, Normal and (`with_height`) Height painted in shapes and two
    /// alphas, and `stack` on its content, each effect switched on only for its channel (None:
    /// every channel), and after its mask's paint.
    fn normal_project(stack: &[(Filter, f32, BlendMode, Option<Channel>)], with_height: bool) -> Option<(Project, PixelStore, LayerId)> {
        let channels: Vec<Channel> =
            [Channel::BaseColor, Channel::Normal, Channel::Height].into_iter().filter(|&channel| with_height || channel != Channel::Height).collect();
        let mut project = Project::new("Height To Normal", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
        let owner = project.alloc_id();
        let mut layer = Layer::new(owner, "Paint", LayerKind::Paint, &channels);
        let mut mask = Mask::new(MaskBackground::Black);
        let mask_paint = project.alloc_id();
        mask.effects.push(Effect::new(mask_paint, EffectKind::Paint));
        for (filter, opacity, blend, only) in stack {
            let mut added = Effect::new(project.alloc_id(), EffectKind::Filter(filter.clone()));
            added.opacity = *opacity;
            added.blend = *blend;
            if let Some(only) = only {
                added.off_channels = channels.iter().copied().filter(|channel| channel != only).collect();
            }
            layer.content_effects.push(added.clone());
            added.id = project.alloc_id();
            mask.effects.push(added);
        }
        layer.mask = Some(mask);
        project.insert_layer(0, layer, None, false).ok()?;

        // Raised shapes with soft and hard edges over a tilted normal, two alphas
        let size = (SIZE, SIZE);
        let mut pixels = PixelStore::default();
        let mut mask_pixels = TileImage::new(SIZE, SIZE, 2);
        for y in 0..SIZE {
            for x in 0..SIZE {
                if !(20..108).contains(&x) || !(18..110).contains(&y) {
                    continue;
                }

                let alpha = if x >= 64 { 1.0 } else { 0.7 };
                let ring = ((x as f32 - 64.0).hypot(y as f32 - 60.0) / 9.0).sin() * 0.4;
                let height = if (40..80).contains(&x) && (30..50).contains(&y) { 0.6 } else { ring };
                let premultiplied = |value: f32| to_u16(value * alpha);
                pixels.layer_mut(owner, Channel::BaseColor, size).set(x, y, &[premultiplied(0.7), premultiplied(0.4), premultiplied(0.2), to_u16(alpha)]);
                pixels.layer_mut(owner, Channel::Normal, size).set(x, y, &[premultiplied(0.62), premultiplied(0.44), premultiplied(0.93), to_u16(alpha)]);
                if with_height {
                    pixels.layer_mut(owner, Channel::Height, size).set(x, y, &[to_u16_signed(height * alpha), to_u16(alpha)]);
                }
                mask_pixels.set(x, y, &[to_u16(0.5 + height * 0.5), u16::MAX]);
            }
        }
        pixels.masks.insert(mask_paint, mask_pixels);
        Some((project, pixels, owner))
    }

    /// Height To Normal matches the CPU on the Normal from the layer's own Height (other channels
    /// and the mask pass): every setting, below full opacity under Multiply, disabled, after a
    /// Blur of the Height alone (the Height's stage prepared first), between filters, without a
    /// Height (a flat height) and in a partial update. Normals compare by direction.
    #[test]
    fn height_to_normal_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let (baked, images) = (BakedMaps::default(), ImageStore::default());
        let normal = BlendMode::Normal;
        let library = Filter::HeightToNormal(HeightToNormal::LIBRARY);
        let laid_over = Filter::HeightToNormal(HeightToNormal { overwrite: false, ..HeightToNormal::LIBRARY });
        let blur = Filter::Blur { radius: 24.0, wrap: false };
        let mut cases: Vec<(String, Vec<(Filter, f32, BlendMode, Option<Channel>)>, bool)> = height_to_normals()
            .into_iter()
            .map(|to_normal| (format!("{to_normal:?}"), vec![(Filter::HeightToNormal(to_normal), 1.0, normal, None)], true))
            .collect();
        cases.extend([
            ("under Multiply".to_string(), vec![(laid_over.clone(), 0.7, BlendMode::Multiply, None)], true),
            ("disabled".to_string(), vec![(library.clone(), 1.0, BlendMode::Disable, None)], true),
            ("after a Blur of the Height".to_string(), vec![(blur.clone(), 1.0, normal, Some(Channel::Height)), (library.clone(), 1.0, normal, None)], true),
            (
                "between filters".to_string(),
                vec![(blur.clone(), 1.0, normal, Some(Channel::Normal)), (laid_over.clone(), 0.85, normal, None), (blur, 1.0, normal, None)],
                true,
            ),
            ("without a Height".to_string(), vec![(laid_over, 1.0, normal, None)], false),
            ("without a Height, overwriting".to_string(), vec![(library, 1.0, normal, None)], false),
        ]);
        for (label, stack, with_height) in &cases {
            let (project, pixels, owner) = normal_project(stack, *with_height).unwrap();
            let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
            case.mask_owner = Some(owner);
            case.region = Some(MIDDLE);

            assert_matches_cpu(&mut gpu, &case, Tolerance::TRANSCENDENTAL, &format!("Height To Normal {label}"));
        }
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
