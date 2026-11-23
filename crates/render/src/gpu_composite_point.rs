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
//! - The Gradient's `input.blur` (with P4): from [`lower`], call
//!   `inputs.precede_with_stage(&stage_filter, opacity)` with P4's stage filter, then return the
//!   gradient's opcode. The stage runs right before the gradient operation and its output field
//!   replaces the content the gradient reads (test `a_lowering_can_run_a_neighbourhood_stage_before_its_operation`).
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] and `still_on_cpu` in the tests below. Every port gets a CPU
//! oracle test (`crate::gpu_composite::oracle`) in colour, another content channel and the mask,
//! for each setting value, set to Disable too, at opacity below 1 with a non-Normal blend.

use texturecraft_core::{CurvePreset, Effect, Filter, POSTERIZE_LEVELS_MAX, POSTERIZE_LEVELS_MIN};

use super::packages::{POINT_OP_BITS, POINT_OPCODES};
use super::{CompositeProgram, Operation, ProceduralInputs};

/// Library filter ids P2 still leaves on the CPU with their defaults (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &["gradientMap", "gradientDynamic", "grayscale"];

// P2's opcodes (the same constants head composite_point.wgsl)
const HSL_PERCEPTIVE: u32 = 128;
const POSTERIZE_ALPHA: u32 = 129;
const HISTOGRAM_SCAN_INVERTED: u32 = 130;
const COLOR_CORRECT: u32 = 131;
const SMOOTHSTEP: u32 = 132;
const HEIGHT_ADJUST: u32 = 133;
const GRADIENT_CURVE: u32 = 134;

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
pub(super) fn lower(filter: &Filter, _effect: &Effect, op: &mut Operation, _inputs: &mut ProceduralInputs) -> Option<(u32, [f32; 3])> {
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
        _ => None,
    }
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
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use texturecraft_compose::{BakedMaps, ImageStore, PixelStore};
    use texturecraft_core::{
        BlendMode, Channel, ContentsOffset, CurvePreset, Effect, EffectKind, Fill, Filter, Layer, LayerId, LayerKind, Mask, MaskBackground, Project, Source,
        TextureSet,
    };
    use texturecraft_geom::primitives;
    use texturecraft_texel::{TILE, to_u16, to_u16_signed};

    use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu, compile_effects, effect, headless};

    /// The oracle's texture set is one tile across
    const SIZE: u32 = TILE;

    /// The opacity of the filter under test, below 1 so the mix is exercised
    const EFFECT_OPACITY: f32 = 0.77;

    /// A painted layer over a uniform fill, carrying `effects` (filter, opacity) in its content and
    /// in its painted mask, each set to `blend`: (project, pixels, the painted layer). The paint
    /// sweeps every channel and the mask across the tile at a coverage that rises down it.
    fn painted_case(effects: &[(Filter, f32)], blend: BlendMode) -> (Project, PixelStore, LayerId) {
        let channels = [Channel::BaseColor, Channel::Roughness, Channel::Height, Channel::Normal];
        let mut project = Project::new("Point filters", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
        let sources = BTreeMap::from([
            (Channel::BaseColor, Source::Uniform([0.47, 0.3, 0.58])),
            (Channel::Roughness, Source::Uniform([0.41; 3])),
            (Channel::Height, Source::Uniform([-0.34; 3])),
            (Channel::Normal, Source::Uniform([0.37, 0.68, 0.89])),
        ]);
        let below = Layer::new(project.alloc_id(), "Below", LayerKind::Fill(Fill { sources, ..Default::default() }), &channels);
        project.insert_layer(0, below, None, false).unwrap();

        let owner = project.alloc_id();
        let painted_mask = project.alloc_id();
        let mut layer = Layer::new(owner, "Painted", LayerKind::Paint, &channels);
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
        project.insert_layer(0, layer, None, false).unwrap();

        // Premultiplied paint, as the store keeps it
        let mut pixels = PixelStore::default();
        let size = (SIZE, SIZE);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let (across, down) = ((x as f32 + 0.5) / SIZE as f32, (y as f32 + 0.5) / SIZE as f32);
                let coverage = 0.25 + 0.75 * down;
                let premultiplied = |value: f32| to_u16(value * coverage);
                let alpha = to_u16(coverage);
                let color = [premultiplied(across), premultiplied(0.15 + 0.7 * down), premultiplied(0.9 - 0.6 * across), alpha];
                pixels.layer_mut(owner, Channel::BaseColor, size).set(x, y, &color);
                pixels.layer_mut(owner, Channel::Roughness, size).set(x, y, &[premultiplied(0.05 + 0.6 * across + 0.3 * down), alpha]);
                pixels.layer_mut(owner, Channel::Height, size).set(x, y, &[to_u16_signed((-0.8 + 1.2 * across + 0.4 * down) * coverage), alpha]);
                let normal = [premultiplied(0.35 + 0.3 * across), premultiplied(0.35 + 0.3 * down), premultiplied(0.92), alpha];
                pixels.layer_mut(owner, Channel::Normal, size).set(x, y, &normal);
                pixels.mask_mut(painted_mask, size).set(x, y, &[to_u16(0.05 + 0.6 * across + 0.3 * (1.0 - down)), u16::MAX]);
            }
        }
        (project, pixels, owner)
    }

    /// Each filter in content (colour and the other channels) and in the mask matches the CPU:
    /// alone, set to Disable, and between two filters the GPU already evaluates
    fn assert_filters_match_cpu(filters: &[Filter], tolerance: Tolerance) {
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
                let (project, pixels, owner) = painted_case(&effects, blend);
                let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
                case.mask_owner = Some(owner);
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

        // OKLab takes cube roots
        assert_filters_match_cpu(&filters, Tolerance::TRANSCENDENTAL);
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
            .chain([CurvePreset::PeakOut, CurvePreset::Peak, CurvePreset::PeakIn, CurvePreset::Triangular])
            .map(curve)
            .collect();
        filters.push(Filter::GradientCurve { preset: CurvePreset::Round, invert: true, balance: 0.3, contrast: 0.4 });
        filters.push(Filter::GradientCurve { preset: CurvePreset::Ease, invert: false, balance: 0.65, contrast: 0.8 });
        let sines = [CurvePreset::Bump, CurvePreset::DoubleBump].map(curve);
        let steps = [curve(CurvePreset::Stairs), Filter::GradientCurve { preset: CurvePreset::Round, invert: false, balance: 0.5, contrast: 1.0 }];

        assert_filters_match_cpu(&filters, Tolerance::POINT);
        assert_filters_match_cpu(&sines, Tolerance::TRANSCENDENTAL);
        // Stairs and full contrast are hard decisions; the painted values sit away from them
        assert_filters_match_cpu(&steps, Tolerance::POINT.with_flips(4));
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
        vec![
            Filter::GradientMap {
                colors: vec![[0.0; 3], [1.0; 3]],
                positions: vec![0.0, 0.3],
                quantity: None,
                input: Default::default(),
                linear: true,
                debug: false,
            },
            Filter::GradientMap {
                colors: vec![[0.0; 3], [1.0; 3], [0.5; 3]],
                positions: Vec::new(),
                quantity: Some(2),
                input: Default::default(),
                linear: true,
                debug: false,
            },
            Filter::GradientMap {
                colors: vec![[0.0; 3], [1.0; 3]],
                positions: Vec::new(),
                quantity: None,
                input: Default::default(),
                linear: false,
                debug: false,
            },
            Filter::GradientMap {
                colors: vec![[0.0; 3], [1.0; 3]],
                positions: Vec::new(),
                quantity: None,
                input: Default::default(),
                linear: true,
                debug: true,
            },
        ]
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
