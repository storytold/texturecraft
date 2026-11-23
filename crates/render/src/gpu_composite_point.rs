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
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] and `still_on_cpu` in the tests below. Every port gets a CPU
//! oracle test (`crate::gpu_composite::oracle`) in colour, another content channel and the mask,
//! for each setting value, set to Disable too, at opacity below 1 with a non-Normal blend.

use texturecraft_core::{Effect, Filter};

use super::packages::{POINT_OP_BITS, POINT_OPCODES};
use super::{CompositeProgram, Operation, ProceduralInputs};

/// Library filter ids P2 still leaves on the CPU with their defaults (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &["hsl", "colorCorrect", "gradientMap", "gradientCurve", "gradientDynamic", "smoothstep", "heightAdjust", "grayscale"];

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
pub(super) fn lower(_filter: &Filter, _effect: &Effect, _op: &mut Operation, _inputs: &mut ProceduralInputs) -> Option<(u32, [f32; 3])> {
    None
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
    use texturecraft_core::{ContentsOffset, EffectKind, Filter};

    use crate::gpu_composite::oracle::{compile_effects, effect};

    /// P2's filters and settings that still composite on the CPU, in content and in masks. Port
    /// one: delete its row (this test fails while a ported row stays) and add its oracle test.
    fn still_on_cpu() -> Vec<Filter> {
        vec![
            Filter::Smoothstep { min: 0.2, max: 0.8 },
            Filter::Hsl { hue: 0.1, saturation: 0.0, lightness: 0.0, perceptive: true },
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
            Filter::HistogramScan { position: 0.4, contrast: 0.5, invert_position: true },
            Filter::Posterize { levels: 4.0, alpha: true },
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
