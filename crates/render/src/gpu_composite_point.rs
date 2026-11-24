//! P2, point filters and gradients (plan section 2.2, without the gradient pre-blur, which is
//! P4's). Owned by the P2 agent; no other package edits this file or `shaders/composite_point.wgsl`.
//!
//! What P2 implements here: HSL Perceptive (OKLab), Posterize `alpha`, Histogram Scan
//! `invert_position`, Color Correct, Smoothstep, Height Adjust, Gradient Curve, Gradient Dynamic,
//! Grayscale Conversion (its Custom Input waits for P3's image inputs), and the Gradient's
//! `positions`, `quantity`, `linear: false`, `input` mode/balance/contrast/invert and `debug`.
//! Levels `affects` is P3's (per-channel stack plumbing).
//!
//! How:
//! - [`lower`] is called from `point_effect` for every filter its built-in arms do not take
//!   (the built-in arms keep their exact settings, so `Hsl { perceptive: true, .. }` reaches it).
//!   Return `(opcode, [three params])` with an opcode from `packages::POINT_OPCODES` (128..144),
//!   or None without touching `op`. The caller writes `op.low` = params + opacity and checks every
//!   word is finite. More numbers go in `op.high`, `op.projection` or the side table
//!   (`inputs.pack_params(&[..])`, offset in `op.ext[0]`, read with `param(op, i)` in WGSL);
//!   flags in `op.ext[2] & POINT_OP_BITS`.
//! - The WGSL file defines `point_scalar_filter` (scalar channels and masks) and
//!   `point_effect_color` (every content channel; `channel` is the destination slot), and the
//!   loop-level `point_content_effect` (content with coverage and the whole texel: Posterize
//!   alpha, the Grayscale alpha weight, Gradient debug) and `point_mask_effect`.
//! - Keep [`uses_shader`] true for every program with a P2 opcode.
//! - The Gradient's `input.blur` (with P4): [`lower`] calls
//!   `inputs.precede_with_stage(gradient, opacity)` with the gradient filter itself, so P4's
//!   `box_family::new` keys its stage on it, then returns the gradient's opcode with the
//!   `GRADIENT_BLURRED` flag. The stage runs right before the gradient operation (test
//!   `a_lowering_can_run_a_neighbourhood_stage_before_its_operation`). Its output keeps the
//!   original premultiplied texel in words 0-3, so the loop resumes from the unblurred content,
//!   and word 4 holds the blurred gray: the input's mode applied to the blurred straight colour,
//!   or the blurred first component on scalar and signed channels and masks. The WGSL reads it
//!   with `preceding_stage_word`, maps it through the gradient and mixes the result over the
//!   unblurred content by the opacity, keeping its coverage, as `map_blurred` does on the CPU.
//!   Until P4 builds that stage the call fails and the gradient stays on the CPU (its row in
//!   `still_on_cpu`).
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] and `still_on_cpu` in the tests below. Every port gets a CPU
//! oracle test (`crate::gpu_composite::oracle`) in colour, another content channel and the mask,
//! for each setting value, set to Disable too, at opacity below 1 with a non-Normal blend.

use texturecraft_core::{
    ConversionMode, CurvePreset, Effect, Filter, GradientInput, GrayscaleMode, POSTERIZE_LEVELS_MAX, POSTERIZE_LEVELS_MIN, finite_unit, lum,
};
use texturecraft_texel::linear_to_srgb;

use super::packages::{POINT_OP_BITS, POINT_OPCODES};
use super::{CompositeProgram, Operation, ProceduralInputs};

/// Library filter ids P2 still leaves on the CPU with their defaults (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &[];

// P2's opcodes (composite_point.wgsl switches on the same numbers)
const HSL_PERCEPTIVE: u32 = 128;
const POSTERIZE_ALPHA: u32 = 129;
const HISTOGRAM_SCAN_INVERTED: u32 = 130;
const COLOR_CORRECT: u32 = 131;
const SMOOTHSTEP: u32 = 132;
const HEIGHT_ADJUST: u32 = 133;
const GRADIENT_CURVE: u32 = 134;
const GRADIENT_DYNAMIC: u32 = 135;
const GRAYSCALE_CONVERSION: u32 = 136;
const GRADIENT: u32 = 137;

// A gradient's flags in `Operation::ext[2]` (within `POINT_OP_BITS`)
const GRADIENT_EVEN_LINEAR: u32 = 1;
const GRADIENT_SRGB: u32 = 2;
const GRADIENT_NEUTRAL_INPUT: u32 = 4;
const GRADIENT_DEBUG: u32 = 8;
const GRADIENT_BLURRED: u32 = 16;

// The grays `point_gray` in composite_point.wgsl reads
const GRAY_DESATURATION: u32 = 0;
const GRAY_LUMA: u32 = 1;
const GRAY_AVERAGE: u32 = 2;
const GRAY_MAX: u32 = 3;
const GRAY_MIN: u32 = 4;
const GRAY_WEIGHTS: u32 = 5;

pub(super) const WGSL: &str = include_str!("shaders/composite_point.wgsl");
/// The same functions without P2's code, for programs that do not use it.
pub(super) const WGSL_STUB: &str = "fn point_scalar_filter(op: Operation, x: f32) -> f32 { return x; }
fn point_effect_color(op: Operation, c: vec3<f32>, channel: u32, surface: Surface, baked_ao: f32) -> vec3<f32> { return c; }
fn point_content_effect(effect: Operation, content: vec4<f32>, channel: u32, texel: LayerTexel) -> vec4<f32> {
    let filtered = point_effect_color(effect, content.rgb, channel, texel.surface, texel.baked.x);
    return vec4(content.rgb + (filtered - content.rgb) * effect.low.w, content.a);
}
fn point_mask_effect(effect: Operation, mask: f32, texel: LayerTexel) -> f32 { return point_scalar_filter(effect, mask); }";

/// A filter as a P2 point operation: (opcode, three parameters), or None.
pub(super) fn lower(filter: &Filter, effect: &Effect, op: &mut Operation, inputs: &mut ProceduralInputs) -> Option<(u32, [f32; 3])> {
    match filter {
        Filter::Hsl { hue, saturation, lightness, perceptive: true } => {
            // The hue turn as the CPU computes it, so the shader only rotates
            let (sine, cosine) = (hue * std::f32::consts::TAU).sin_cos();
            op.high[..2].copy_from_slice(&[sine, cosine]);
            Some((HSL_PERCEPTIVE, [*hue, *saturation, *lightness]))
        }
        Filter::Posterize { levels, alpha: true } => {
            // The steps between the levels, as `posterize` rounds them
            let steps = (levels.clamp(POSTERIZE_LEVELS_MIN, POSTERIZE_LEVELS_MAX).round() - 1.0).max(1.0);
            Some((POSTERIZE_ALPHA, [steps, 0.0, 0.0]))
        }
        Filter::HistogramScan { position, contrast, invert_position: true } => Some((HISTOGRAM_SCAN_INVERTED, [1.0 - position, *contrast, 0.0])),
        Filter::ColorCorrect { shadows, midtones, highlights } => {
            op.high[..3].copy_from_slice(midtones);
            op.projection[..3].copy_from_slice(highlights);
            Some((COLOR_CORRECT, *shadows))
        }
        Filter::Smoothstep { min, max } => {
            // The bounds in order, as `smoothstep` reads them
            let (low, high) = (min.min(*max), min.max(*max));
            Some((SMOOTHSTEP, [if low.is_finite() { low } else { 0.0 }, if high.is_finite() { high } else { 1.0 }, 0.0]))
        }
        Filter::HeightAdjust { invert, offset, multiply } => {
            let offset = if offset.is_finite() { *offset } else { 0.0 };
            let multiply = if multiply.is_finite() { *multiply } else { 1.0 };
            Some((HEIGHT_ADJUST, [f32::from(u8::from(*invert)), offset, multiply]))
        }
        Filter::GradientCurve { preset, invert, balance, contrast } => {
            op.ids[2] = curve_shape(*preset);
            Some((GRADIENT_CURVE, [*balance, *contrast, f32::from(u8::from(*invert))]))
        }
        Filter::GradientDynamic { color, .. } => {
            // Scalar channels and masks read the colour's luminance
            op.high[0] = finite_unit(lum(*color));
            Some((GRADIENT_DYNAMIC, color.map(finite_unit)))
        }
        // A Custom Input waits for the image inputs of the stack plumbing (P3)
        Filter::GrayscaleConversion(conversion) if !conversion.custom_input => {
            op.ids[2] = match conversion.mode {
                ConversionMode::Desaturation => GRAY_DESATURATION,
                ConversionMode::ChannelsWeights => GRAY_WEIGHTS,
                ConversionMode::Average => GRAY_AVERAGE,
                ConversionMode::Max => GRAY_MAX,
                ConversionMode::Min => GRAY_MIN,
            };
            op.high = conversion.weights;
            Some((GRAYSCALE_CONVERSION, [conversion.balance, conversion.contrast, f32::from(u8::from(conversion.invert))]))
        }
        Filter::GradientMap { .. } => lower_gradient(filter, effect, op, inputs),
        _ => None,
    }
}

/// A gradient the built-in opcode 33 does not take: placed stops, a Color Quantity, sRGB
/// blending, a changed input or the debug view. Its stops go in the side table (evenly spaced
/// linear ones as opcode 33 packs them), the input's gray in `ids[2]` and its balance, contrast
/// and invert in the parameters.
fn lower_gradient(filter: &Filter, effect: &Effect, op: &mut Operation, inputs: &mut ProceduralInputs) -> Option<(u32, [f32; 3])> {
    let Filter::GradientMap { positions, input, linear, debug, .. } = filter else { return None };
    let linear = *linear;
    let colors = filter.ramp_colors();
    let even = positions.is_empty() && linear;
    let offset = if even {
        inputs.pack_rgb(colors)?[0]
    } else {
        let words: Vec<f32> =
            sorted_stops(colors, positions, linear).into_iter().flat_map(|(position, [red, green, blue])| [position, red, green, blue]).collect();
        inputs.pack_params(&words)?
    };
    let count = u32::try_from(colors.len()).ok()?;

    // The input's blur runs as the box family's stage (P4) right before the gradient; the debug view reads no input
    let blurred = input.blur > 0.0 && !debug;
    if blurred {
        inputs.precede_with_stage(filter, effect.opacity)?;
    }

    let neutral = GradientInput::default();
    let unshaped = input.balance == neutral.balance && input.contrast == neutral.contrast && input.invert == neutral.invert;
    let mut flags = if even { GRADIENT_EVEN_LINEAR } else { 0 };
    flags |= if linear { 0 } else { GRADIENT_SRGB };
    flags |= if unshaped { GRADIENT_NEUTRAL_INPUT } else { 0 };
    flags |= if *debug { GRADIENT_DEBUG } else { 0 };
    flags |= if blurred { GRADIENT_BLURRED } else { 0 };

    if even {
        op.procedural[1] = offset;
    } else {
        op.ext[0] = offset;
    }
    op.procedural[2] = count;
    op.ext[2] = flags;
    op.ids[2] = match input.mode {
        GrayscaleMode::Desaturation => GRAY_DESATURATION,
        GrayscaleMode::Luma => GRAY_LUMA,
        GrayscaleMode::Average => GRAY_AVERAGE,
        GrayscaleMode::Max => GRAY_MAX,
        GrayscaleMode::Min => GRAY_MIN,
    };
    Some((GRADIENT, [input.balance, input.contrast, f32::from(u8::from(input.invert))]))
}

/// A ramp's stops as the CPU places them (`ramp_stops` in core: a missing or non-finite place
/// evenly spaced, every place clamped to 0..1), sRGB-encoded when the ramp blends in sRGB, in
/// place order. The stable sort keeps the list order at a shared place, which decides the stop
/// the CPU reads there.
fn sorted_stops(colors: &[[f32; 3]], positions: &[f32], linear: bool) -> Vec<(f32, [f32; 3])> {
    // A copy of core's `ramp_stops` (gradient_input.rs), which core keeps private
    let last = colors.len().saturating_sub(1);
    let mut stops: Vec<(f32, [f32; 3])> = colors
        .iter()
        .enumerate()
        .map(|(index, color)| {
            let even = if last == 0 { 0.0 } else { index as f32 / last as f32 };
            let position = positions.get(index).copied().filter(|position| position.is_finite()).unwrap_or(even);
            let position = if position <= 0.0 { 0.0 } else { position.min(1.0) };
            (position, if linear { *color } else { color.map(linear_to_srgb) })
        })
        .collect();
    stops.sort_by(|left, right| left.0.total_cmp(&right.0));
    stops
}

/// The shape `point_curve` in composite_point.wgsl draws for a Gradient Curve preset
fn curve_shape(preset: CurvePreset) -> u32 {
    match preset {
        CurvePreset::Round => 0,
        CurvePreset::Pinch => 1,
        CurvePreset::Stairs => 2,
        CurvePreset::EaseOut => 3,
        CurvePreset::Ease => 4,
        CurvePreset::EaseIn => 5,
        CurvePreset::PeakOut => 6,
        CurvePreset::Peak => 7,
        CurvePreset::PeakIn => 8,
        CurvePreset::Bump => 9,
        CurvePreset::DoubleBump => 10,
        CurvePreset::Triangular => 11,
    }
}

/// Whether a program needs the full `composite_point.wgsl`.
pub(super) fn uses_shader(program: &CompositeProgram) -> bool {
    program.operations.iter().any(|op| POINT_OPCODES.contains(&op.ids[1]) || op.ext[2] & POINT_OP_BITS != 0)
}

/// P2's vec4 of the band uniform (`params.packages[1]`).
pub(super) fn band_uniform(_program: &CompositeProgram) -> [u32; 4] {
    [0; 4]
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::sync::Arc;

    use texturecraft_compose::{BakedMaps, ImageStore, PixelStore};
    use texturecraft_core::{
        BlendMode, Channel, ChannelFormat, ChannelKind, ContentsOffset, ConversionMode, CurvePreset, Effect, EffectKind, Fill, Filter, GradientInput,
        GrayscaleConversion, GrayscaleMode, Layer, LayerId, LayerKind, Mask, MaskBackground, Project, Source, TextureSet,
    };
    use texturecraft_geom::primitives;
    use texturecraft_texel::{TILE, to_u16, to_u16_signed};

    use crate::gpu_composite::CompositeRegion;
    use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu, compile_effects, effect, headless};

    /// The oracle's texture set is one tile across
    const SIZE: u32 = TILE;

    /// The opacity of the filter under test, below 1 so the mix is exercised
    const EFFECT_OPACITY: f32 = 0.77;

    /// Every kind of content channel: colours in Base Color and a colour lobe, a scalar, the
    /// signed Height, and both normal maps
    const CHANNELS: [Channel; 6] = [Channel::BaseColor, Channel::Emissive, Channel::Roughness, Channel::Height, Channel::Normal, Channel::CoatNormal];

    /// A painted layer over a uniform fill in `channels`, carrying `effects` (filter, opacity) in
    /// its content and in its painted mask, each set to `blend`: (project, pixels, the painted
    /// layer). The paint sweeps every channel and the mask across the tile at a coverage that
    /// rises down it.
    fn painted_case(effects: &[(Filter, f32)], blend: BlendMode, channels: &[Channel]) -> (Project, PixelStore, LayerId) {
        let mut project = Project::new("Point filters", "builtin:plane");
        let mut set = TextureSet::new("Plane", 0, SIZE);
        for &channel in channels {
            if !set.channels.iter().any(|&(existing, _)| existing == channel) {
                set.channels.push((channel, ChannelFormat::default_for(channel)));
            }
        }
        project.texture_sets.push(set);
        let below = |channel: Channel| match channel.kind() {
            ChannelKind::Color => [0.47, 0.3, 0.58],
            ChannelKind::Scalar => [0.41; 3],
            ChannelKind::Signed => [-0.34; 3],
            ChannelKind::NormalMap => [0.37, 0.68, 0.89],
        };
        let sources = channels.iter().map(|&channel| (channel, Source::Uniform(below(channel)))).collect();
        let below = Layer::new(project.alloc_id(), "Below", LayerKind::Fill(Fill { sources, ..Default::default() }), channels);
        assert!(project.insert_layer(0, below, None, false).is_ok());

        let owner = project.alloc_id();
        let painted_mask = project.alloc_id();
        let mut layer = Layer::new(owner, "Painted", LayerKind::Paint, channels);
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(painted_mask, EffectKind::Paint));
        for (filter, opacity) in effects {
            for stack in [&mut layer.content_effects, &mut mask.effects] {
                let mut added = Effect::new(project.alloc_id(), EffectKind::Filter(filter.clone()));
                added.opacity = *opacity;
                added.blend = blend;
                stack.push(added);
            }
        }
        layer.mask = Some(mask);
        assert!(project.insert_layer(0, layer, None, false).is_ok());

        // Premultiplied paint, as the store keeps it
        let mut pixels = PixelStore::default();
        let size = (SIZE, SIZE);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let (across, down) = ((x as f32 + 0.5) / SIZE as f32, (y as f32 + 0.5) / SIZE as f32);
                let coverage = 0.25 + 0.75 * down;
                let premultiplied = |value: f32| to_u16(value * coverage);
                let alpha = to_u16(coverage);
                for &channel in channels {
                    let texel = match channel.kind() {
                        ChannelKind::Color => vec![premultiplied(across), premultiplied(0.15 + 0.7 * down), premultiplied(0.9 - 0.6 * across), alpha],
                        ChannelKind::Scalar => vec![premultiplied(0.05 + 0.6 * across + 0.3 * down), alpha],
                        ChannelKind::Signed => vec![to_u16_signed((-0.8 + 1.2 * across + 0.4 * down) * coverage), alpha],
                        ChannelKind::NormalMap => vec![premultiplied(0.35 + 0.3 * across), premultiplied(0.35 + 0.3 * down), premultiplied(0.92), alpha],
                    };
                    pixels.layer_mut(owner, channel, size).set(x, y, &texel);
                }
                pixels.mask_mut(painted_mask, size).set(x, y, &[to_u16(0.05 + 0.6 * across + 0.3 * (1.0 - down)), u16::MAX]);
            }
        }
        (project, pixels, owner)
    }

    /// Each filter in every content channel and in the mask matches the CPU: alone, set to
    /// Disable, and between two filters the GPU already evaluates
    fn assert_filters_match_cpu(filters: &[Filter], tolerance: Tolerance) {
        assert_contexts_match_cpu(filters, &CHANNELS, true, tolerance, None);
    }

    /// [`assert_filters_match_cpu`] in `channels`, with the mask preview when `mask_preview`, and
    /// recomposited in `region` too when it has one
    fn assert_contexts_match_cpu(filters: &[Filter], channels: &[Channel], mask_preview: bool, tolerance: Tolerance, region: Option<CompositeRegion>) {
        for filter in filters {
            let added = || Some(effect(EffectKind::Filter(filter.clone())));
            let (material, mask) = compile_effects(added(), added(), ContentsOffset::NONE);
            assert!(material.is_ok() && mask.is_ok(), "{filter:?}: {material:?} {mask:?}");
        }

        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let (baked, images) = (BakedMaps::default(), ImageStore::default());
        let before = Filter::ContrastLuminosity { contrast: 0.2, luminosity: 0.05 };
        let after = Filter::Hsl { hue: 0.1, saturation: 0.2, lightness: -0.05, perceptive: false };
        for filter in filters {
            let alone = vec![(filter.clone(), EFFECT_OPACITY)];
            let chained = vec![(before.clone(), 0.9), (filter.clone(), EFFECT_OPACITY), (after.clone(), 0.85)];
            let stacks =
                [(alone.clone(), BlendMode::Multiply, "alone"), (alone, BlendMode::Disable, "set to Disable"), (chained, BlendMode::Multiply, "chained")];
            for (effects, blend, context) in stacks {
                let (project, pixels, owner) = painted_case(&effects, blend, channels);
                let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
                case.mask_owner = mask_preview.then_some(owner);
                case.region = region;
                assert_matches_cpu(&mut gpu, &case, tolerance, &format!("{filter:?} {context}"));
            }
        }
    }

    #[test]
    fn hsl_perceptive_matches_the_cpu() {
        let filters = [
            // The library default
            Filter::Hsl { hue: 0.0, saturation: 0.0, lightness: 0.0, perceptive: true },
            Filter::Hsl { hue: 0.2, saturation: 0.3, lightness: -0.1, perceptive: true },
            Filter::Hsl { hue: -0.35, saturation: -0.6, lightness: 0.4, perceptive: true },
            Filter::Hsl { hue: 0.45, saturation: 0.8, lightness: 0.25, perceptive: true },
            Filter::Hsl { hue: 1.3, saturation: -1.0, lightness: -1.0, perceptive: true },
        ];

        // OKLab takes cube roots in colours only; the other channels and the mask shift a lightness
        assert_contexts_match_cpu(&filters, &[Channel::BaseColor, Channel::Emissive], false, Tolerance::TRANSCENDENTAL, None);
        assert_contexts_match_cpu(&filters, &[Channel::Roughness, Channel::Height, Channel::Normal, Channel::CoatNormal], true, Tolerance::POINT, None);
    }

    #[test]
    fn posterize_apply_to_alpha_steps_the_coverage_as_the_cpu_does() {
        let filters = [
            Filter::Posterize { levels: 4.0, alpha: true },
            Filter::Posterize { levels: 8.0, alpha: true },
            // Rounds to three levels
            Filter::Posterize { levels: 2.6, alpha: true },
        ];

        // Steps are hard decisions; the painted values sit away from them
        assert_filters_match_cpu(&filters, Tolerance::POINT.with_flips(4));
    }

    #[test]
    fn histogram_scan_invert_position_matches_the_cpu() {
        let soft = [
            Filter::HistogramScan { position: 0.4, contrast: 0.5, invert_position: true },
            Filter::HistogramScan { position: 0.7, contrast: 0.0, invert_position: true },
            Filter::HistogramScan { position: 0.35, contrast: 0.95, invert_position: true },
        ];
        let hard = [Filter::HistogramScan { position: 0.45, contrast: 1.0, invert_position: true }];

        assert_filters_match_cpu(&soft, Tolerance::POINT);
        // Full contrast is a threshold; the painted values sit away from it
        assert_filters_match_cpu(&hard, Tolerance::POINT.with_flips(4));
    }

    #[test]
    fn color_correct_matches_the_cpu() {
        let filters = [
            // The library default
            Filter::ColorCorrect { shadows: [0.0, 0.0, 0.5], midtones: [0.0, 0.0, 0.5], highlights: [0.0, 0.0, 0.5] },
            Filter::ColorCorrect { shadows: [0.3, -0.1, 0.2], midtones: [-0.2, 0.15, 0.7], highlights: [0.1, 0.05, 0.4] },
            // Contrast and luminosity beyond their range clamp
            Filter::ColorCorrect { shadows: [1.4, 0.6, 1.0], midtones: [-1.0, -0.3, 0.0], highlights: [0.5, -1.2, 0.9] },
        ];

        assert_filters_match_cpu(&filters, Tolerance::POINT);
    }

    #[test]
    fn smoothstep_matches_the_cpu() {
        let soft = [
            // The library default
            Filter::Smoothstep { min: 0.0, max: 1.0 },
            Filter::Smoothstep { min: 0.2, max: 0.8 },
            // Swapped bounds read in order
            Filter::Smoothstep { min: 0.7, max: 0.3 },
        ];
        let step = [Filter::Smoothstep { min: 0.55, max: 0.55 }];

        assert_filters_match_cpu(&soft, Tolerance::POINT);
        // Equal bounds are a step; the painted values sit away from it
        assert_filters_match_cpu(&step, Tolerance::POINT.with_flips(4));
    }

    #[test]
    fn gradient_curve_presets_match_the_cpu() {
        // Each preset at the library's balance, contrast and invert (Round is the library default)
        let curve = |preset| Filter::GradientCurve { preset, invert: false, balance: 0.5, contrast: 0.0 };
        let mut filters: Vec<Filter> = [CurvePreset::Round, CurvePreset::Pinch, CurvePreset::EaseOut, CurvePreset::Ease, CurvePreset::EaseIn]
            .into_iter()
            .chain([CurvePreset::PeakOut, CurvePreset::Peak, CurvePreset::PeakIn, CurvePreset::Bump, CurvePreset::DoubleBump, CurvePreset::Triangular])
            .map(curve)
            .collect();
        filters.push(Filter::GradientCurve { preset: CurvePreset::Round, invert: true, balance: 0.3, contrast: 0.4 });
        filters.push(Filter::GradientCurve { preset: CurvePreset::Ease, invert: false, balance: 0.65, contrast: 0.8 });
        let steps = [curve(CurvePreset::Stairs), Filter::GradientCurve { preset: CurvePreset::Round, invert: false, balance: 0.5, contrast: 1.0 }];

        assert_filters_match_cpu(&filters, Tolerance::POINT);
        // Stairs and full contrast are hard decisions; the painted values sit away from them
        assert_filters_match_cpu(&steps, Tolerance::POINT.with_flips(4));
    }

    #[test]
    fn gradient_dynamic_matches_the_cpu() {
        let filters = [
            // The library default
            Filter::GradientDynamic { vertical: false, position: 0.0, color: [0.6; 3] },
            Filter::GradientDynamic { vertical: true, position: 0.3, color: [0.2, 0.7, 0.4] },
            // Components beyond 0..1 clamp
            Filter::GradientDynamic { vertical: false, position: 0.0, color: [1.3, -0.2, 0.5] },
        ];

        assert_filters_match_cpu(&filters, Tolerance::POINT);
    }

    #[test]
    fn grayscale_conversion_matches_the_cpu() {
        let library = GrayscaleConversion::LIBRARY;
        let mut conversions: Vec<GrayscaleConversion> = ConversionMode::ALL.into_iter().map(|mode| GrayscaleConversion { mode, ..library }).collect();
        // Alpha weighs the content's coverage, and 1 in a mask
        conversions.push(GrayscaleConversion { weights: [0.2, 0.3, 0.1, 0.4], ..library });
        conversions.push(GrayscaleConversion { invert: true, balance: 0.6, contrast: 0.3, ..library });
        let filters: Vec<Filter> = conversions.into_iter().map(Filter::GrayscaleConversion).collect();
        let hard = [Filter::GrayscaleConversion(GrayscaleConversion { contrast: 1.0, mode: ConversionMode::Average, ..library })];

        assert_filters_match_cpu(&filters, Tolerance::POINT);
        // Full contrast is a step at mid gray; the painted values sit away from it
        assert_filters_match_cpu(&hard, Tolerance::POINT.with_flips(4));
    }

    #[test]
    fn gradients_with_stops_quantity_srgb_and_input_match_the_cpu() {
        let three = [[0.05, 0.1, 0.6], [0.9, 0.4, 0.1], [0.3, 0.8, 0.2]];
        let ramp = |colors: &[[f32; 3]], positions: &[f32], quantity, input, linear| Filter::GradientMap {
            colors: colors.to_vec(),
            positions: positions.to_vec(),
            quantity,
            input,
            linear,
            debug: false,
        };
        let luma = GradientInput::default();
        // The library default: black, mid gray and white placed at 0, 0.5 and 1 of ten colours, the average gray
        let mut library_colors = vec![[0.0; 3], [0.214; 3]];
        library_colors.resize(10, [1.0; 3]);
        let mut library_positions = vec![0.0, 0.5];
        library_positions.resize(10, 1.0);
        let mut filters = vec![
            ramp(&library_colors, &library_positions, Some(3), GradientInput::LIBRARY, true),
            ramp(&three, &[0.0, 0.3, 0.9], None, luma, true),
            // Missing, non-finite and out-of-range places
            ramp(&three, &[-0.2, f32::NAN], None, luma, true),
            ramp(&[[0.2, 0.3, 0.4], [0.9, 0.1, 0.5], [0.1, 0.7, 0.3], [0.6, 0.6, 0.1]], &[], Some(2), luma, true),
            ramp(&three, &[], None, luma, false),
            ramp(&three, &[0.1, 0.75, 1.4], Some(2), luma, false),
            ramp(&[[0.4, 0.2, 0.7]], &[], None, luma, false),
            // No colours leave colours as they are and grays as gray
            ramp(&[], &[], None, GradientInput::LIBRARY, true),
        ];
        let shaped = |mode| GradientInput { mode, balance: 0.6, contrast: 0.3, invert: true, ..luma };
        filters.extend(GrayscaleMode::ALL.into_iter().map(|mode| ramp(&three, &[], None, shaped(mode), true)));
        let hard = [
            // A shared place is a jump in the ramp; the later stop in the list wins there
            ramp(&[[0.8, 0.1, 0.1], [0.1, 0.8, 0.1], [0.1, 0.1, 0.8], [0.6, 0.6, 0.6]], &[0.8, 0.2, 0.2, 0.5], None, luma, false),
            ramp(&three, &[], None, GradientInput { contrast: 1.0, ..GradientInput::LIBRARY }, true),
        ];

        assert_filters_match_cpu(&filters, Tolerance::POINT);
        // The jump and full contrast are hard decisions; the painted values sit away from them
        assert_filters_match_cpu(&hard, Tolerance::POINT.with_flips(4));
    }

    #[test]
    fn the_gradient_debug_view_matches_the_cpu() {
        let three = [[0.05, 0.1, 0.6], [0.9, 0.4, 0.1], [0.3, 0.8, 0.2]];
        let debug = |positions: &[f32], input, linear| Filter::GradientMap {
            colors: three.to_vec(),
            positions: positions.to_vec(),
            quantity: None,
            input,
            linear,
            debug: true,
        };
        let mut filters = vec![
            debug(&[], GradientInput::default(), true),
            debug(&[0.0, 0.3, 0.9], GradientInput::LIBRARY, true),
            debug(&[0.1, 0.75, 1.0], GradientInput::default(), false),
            // The debug view reads no input, so its blur changes nothing
            debug(&[], GradientInput { blur: 4.0, ..GradientInput::default() }, true),
        ];
        filters.push(Filter::GradientMap {
            colors: Vec::new(),
            positions: Vec::new(),
            quantity: None,
            input: GradientInput::default(),
            linear: true,
            debug: true,
        });

        assert_filters_match_cpu(&filters, Tolerance::POINT);
    }

    #[test]
    fn gradients_after_their_input_blur_match_the_cpu() {
        let three = [[0.05, 0.1, 0.6], [0.9, 0.4, 0.1], [0.3, 0.8, 0.2]];
        let blurred =
            |input, linear| Filter::GradientMap { colors: three.to_vec(), positions: vec![0.0, 0.4, 1.0], quantity: None, input, linear, debug: false };
        let filters = [
            blurred(GradientInput { blur: 4.0, ..GradientInput::LIBRARY }, true),
            blurred(GradientInput { blur: 12.0, balance: 0.6, contrast: 0.3, invert: true, ..GradientInput::default() }, false),
        ];
        // A region inside the tile, so the blur's halo reaches past it
        let region = CompositeRegion { min: [21, 17], max: [43, 39] };

        // The stage carries one blurred gray, so the normal maps, which the CPU blurs themselves, stay out
        assert_contexts_match_cpu(
            &filters,
            &[Channel::BaseColor, Channel::Emissive, Channel::Roughness, Channel::Height],
            true,
            Tolerance::NEIGHBOURHOOD,
            Some(region),
        );
    }

    #[test]
    fn height_adjust_matches_the_cpu() {
        let filters = [
            // The library default
            Filter::HeightAdjust { invert: false, offset: 0.0, multiply: 1.0 },
            Filter::HeightAdjust { invert: true, offset: 0.1, multiply: 1.5 },
            Filter::HeightAdjust { invert: false, offset: -0.2, multiply: 0.6 },
        ];

        assert_filters_match_cpu(&filters, Tolerance::POINT);
    }

    /// P2's filters and settings that still composite on the CPU, in content and in masks. Port
    /// one: delete its row (this test fails while a ported row stays) and add its oracle test.
    fn still_on_cpu() -> Vec<Filter> {
        vec![Filter::GrayscaleConversion(GrayscaleConversion { custom_input: true, ..GrayscaleConversion::LIBRARY })]
    }

    #[test]
    fn point_filters_still_on_the_cpu_fall_back() {
        let none = ContentsOffset::NONE;
        for filter in still_on_cpu() {
            let (material, _) = compile_effects(Some(effect(EffectKind::Filter(filter.clone()))), None, none);
            assert!(material.is_err(), "content {filter:?}");
            let (material, mask) = compile_effects(None, Some(effect(EffectKind::Filter(filter.clone()))), none);
            assert!(material.is_err() && mask.is_err(), "mask {filter:?}");
        }
    }
}
