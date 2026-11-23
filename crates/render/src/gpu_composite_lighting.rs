//! P6, lighting and material-reading effects (plan section 2.5). Owned by the P6 agent; no other
//! package edits this file or `shaders/composite_lighting.wgsl`. P6 starts after P3 has merged
//! (both touch a layer's channel operations).
//!
//! What P6 implements: Baked Lighting Stylized (`stylized: Some`, the library default), Baked
//! Lighting Environment (SH coefficients computed on the CPU) and PBR Validate. They read the
//! layer's other channels at the texel, so the layer's Base Color operation must run after them
//! ([`order_layer_operations`]); filters after a lighting effect must see the lit Base Color in
//! the stage prepare pass (chained oracle cases).
//!
//! P6 also owns `BAKE_MAPS`, `BakedInputs` and `GpuCompositor::baked_input` in gpu_composite.rs
//! (baked Height and World Normal are not in the GPU baked array yet). Add maps as layers of the
//! existing baked texture or an array, not as new bindings (`gpu_composite_packages.rs` lists the
//! binding budget).
//!
//! How:
//! - [`lower`] is called from `point_effect` for every filter the built-in arms do not take
//!   (`BakedLighting { stylized: None, .. }` stays built in). Return `(opcode, [three params])`
//!   with an opcode from `packages::LIGHTING_OPCODES` (152..160), or None without touching `op`.
//!   About 30 stylized parameters go in the side table (`inputs.pack_params`, offset in
//!   `op.ext[0]`); flags in `op.ext[2] & LIGHTING_OP_BITS`.
//! - The WGSL file defines `lighting_scalar_filter`, `lighting_effect_color` and the loop-level
//!   `lighting_content_effect`/`lighting_mask_effect`. Reading the layer's other channels needs
//!   the channel accumulator in `compose()` (composite.wgsl), an open hook P6 adds after P3.
//! - [`needs_surface`] keeps the surface inputs bound for P6 operations.
//! - [`order_layer_operations`] may move any operation of the layer; the caller then finds the
//!   Height operation again for the height mask (`height_op`) and writes the constraints.
//! - Lighting is skipped in masks on both sides (`compile_mask`), keep it so.
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] and `still_on_cpu` in the tests below. Every port gets a CPU
//! oracle test (`crate::gpu_composite::oracle`) on Base Color with the other channels present,
//! with and without bakes, chained after and before other filters, at opacity below 1.

use texturecraft_core::{Effect, Filter};

use super::packages::{LIGHTING_OP_BITS, LIGHTING_OPCODES};
use super::{CompositeProgram, Operation, ProceduralInputs};

/// Library filter ids P6 still leaves on the CPU with their defaults (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &["bakedLighting", "bakedLightingEnvironment", "pbrValidate"];

pub(super) const WGSL: &str = include_str!("shaders/composite_lighting.wgsl");
/// The same functions without P6's code, for programs that do not use it.
pub(super) const WGSL_STUB: &str = "fn lighting_scalar_filter(op: Operation, x: f32) -> f32 { return x; }
fn lighting_effect_color(op: Operation, c: vec3<f32>, channel: u32, surface: Surface, baked_ao: f32) -> vec3<f32> { return c; }
fn lighting_content_effect(effect: Operation, content: vec4<f32>, channel: u32, texel: LayerTexel) -> vec4<f32> {
    let filtered = lighting_effect_color(effect, content.rgb, channel, texel.surface, texel.baked.x);
    return vec4(content.rgb + (filtered - content.rgb) * effect.low.w, content.a);
}
fn lighting_mask_effect(effect: Operation, mask: f32, texel: LayerTexel) -> f32 { return lighting_scalar_filter(effect, mask); }";

/// A filter as a P6 operation: (opcode, three parameters), or None.
pub(super) fn lower(_filter: &Filter, _effect: &Effect, _op: &mut Operation, _inputs: &mut ProceduralInputs) -> Option<(u32, [f32; 3])> {
    None
}

/// Whether a P6 operation reads the surface inputs.
pub(super) fn needs_surface(op: &Operation) -> bool {
    LIGHTING_OPCODES.contains(&op.ids[1])
}

/// Reorders one layer's channel operations (in set order now) so that the channels P6 effects
/// read come before Base Color.
pub(super) fn order_layer_operations(_operations: &mut [Operation]) {}

/// Whether a program needs the full `composite_lighting.wgsl`.
pub(super) fn uses_shader(program: &CompositeProgram) -> bool {
    program.operations.iter().any(|op| LIGHTING_OPCODES.contains(&op.ids[1]) || op.ext[2] & LIGHTING_OP_BITS != 0)
}

/// P6's vec4 of the band uniform (`params.packages[5]`).
pub(super) fn band_uniform(_program: &CompositeProgram) -> [u32; 4] {
    [0; 4]
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use texturecraft_core::{ContentsOffset, EffectKind, Filter, StylizedLighting};

    use crate::gpu_composite::oracle::{compile_effects, effect};

    /// P6's filters that still composite on the CPU in content (a mask skips lighting on both
    /// sides). Port one: delete its row (this test fails while a ported row stays) and add its
    /// oracle test.
    fn still_on_cpu() -> Vec<Filter> {
        vec![
            Filter::BakedLighting {
                azimuth: 0.0,
                elevation: 45.0,
                light: 1.0,
                ambient: 1.0,
                occlusion: 0.0,
                stylized: Some(Box::new(StylizedLighting::library())),
            },
            Filter::EnvironmentLighting {
                horizontal: 0.0,
                vertical: 0.0,
                exposure: 0.0,
                height_intensity: 1.0,
                ambient_occlusion: 1.0,
                specular_occlusion: 1.0,
            },
        ]
    }

    #[test]
    fn lighting_still_on_the_cpu_falls_back() {
        for filter in still_on_cpu() {
            assert!(compile_effects(Some(effect(EffectKind::Filter(filter.clone()))), None, ContentsOffset::NONE).0.is_err(), "content {filter:?}");
        }
    }
}
