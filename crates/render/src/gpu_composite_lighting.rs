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
//!   `lighting_content_effect`/`lighting_mask_effect`. The layer's other channels come from a
//!   stack read of its material (P0g: `spatial[3]`, `effects[1] = 1`, `stack_at` in WGSL).
//! - [`needs_surface`] keeps the surface inputs bound for P6 operations.
//! - [`order_layer_operations`] may move any operation of the layer; the caller then finds the
//!   Height operation again for the height mask (`height_op`) and writes the constraints.
//! - Lighting is skipped in masks on both sides (`compile_mask`), keep it so.
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] and `still_on_cpu` in the tests below. Every port gets a CPU
//! oracle test (`crate::gpu_composite::oracle`) on Base Color with the other channels present,
//! with and without bakes, chained after and before other filters, at opacity below 1.

use texturecraft_core::{Channel, Effect, Filter, PbrValidate};

use super::packages::{LIGHTING_OP_BITS, LIGHTING_OPCODES};
use super::{CompositeProgram, Operation, ProceduralInputs, channel_slot};

/// Library filter ids P6 still leaves on the CPU with their defaults (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &["bakedLighting", "bakedLightingEnvironment"];

pub(super) const WGSL: &str = include_str!("shaders/composite_lighting.wgsl");
/// The same functions without P6's code, for programs that do not use it.
pub(super) const WGSL_STUB: &str = "fn lighting_scalar_filter(op: Operation, x: f32) -> f32 { return x; }
fn lighting_effect_color(op: Operation, c: vec3<f32>, channel: u32, surface: Surface, baked_ao: f32) -> vec3<f32> { return c; }
fn lighting_content_effect(effect: Operation, content: vec4<f32>, channel: u32, texel: LayerTexel) -> vec4<f32> {
    let filtered = lighting_effect_color(effect, content.rgb, channel, texel.surface, texel.baked.x);
    return vec4(content.rgb + (filtered - content.rgb) * effect.low.w, content.a);
}
fn lighting_mask_effect(effect: Operation, mask: f32, texel: LayerTexel) -> f32 { return lighting_scalar_filter(effect, mask); }";

/// PBR Validate: the base color checked against the metal of the layer's material
const PBR_VALIDATE_OPCODE: u32 = LIGHTING_OPCODES.start + 2;
/// PBR Validate's mode flags in `ids[2]`: non-metals checked, metals checked
const CHECKS_ALBEDO: u32 = 1;
const CHECKS_METAL: u32 = 2;
/// `PbrValidate::check`'s brightest valid non-metal, in sRGB steps of 255
const ALBEDO_BRIGHT_SRGB_STEPS: f32 = 240.0;
/// `PbrValidate::check` counts metal from this value up
const METAL_THRESHOLD: f32 = 0.5;
/// Albedo Dark Range Threshold is in sRGB steps of 255, Metal Reflectance Range in percent of white
const SRGB_STEPS: f32 = 255.0;
const PERCENT: f32 = 100.0;

/// A filter as a P6 operation: (opcode, three parameters), or None.
pub(super) fn lower(filter: &Filter, _effect: &Effect, op: &mut Operation, _inputs: &mut ProceduralInputs) -> Option<(u32, [f32; 3])> {
    match filter {
        Filter::PbrValidate(validate) => {
            op.ids[2] = u32::from(validate.mode.checks_albedo()) * CHECKS_ALBEDO | u32::from(validate.mode.checks_metal()) * CHECKS_METAL;
            let [red, green, blue] = PbrValidate::INVALID;
            op.high = [red, green, blue, METAL_THRESHOLD];
            read_material(op, Channel::Metallic);
            let range = [f32::from(validate.albedo_dark) / SRGB_STEPS, f32::from(validate.metal_low) / PERCENT, ALBEDO_BRIGHT_SRGB_STEPS / SRGB_STEPS];
            Some((PBR_VALIDATE_OPCODE, range))
        }
        _ => None,
    }
}

/// Let `op` read `channel` of its layer's material at the texel: the stack below with the layer's
/// own operations before its Base Color laid on (`stack_at` in WGSL, P0g's stack read)
fn read_material(op: &mut Operation, channel: Channel) {
    op.spatial[3] = channel_slot(channel) + 1;
    op.effects[1] = 1;
}

/// Whether a P6 operation reads the surface inputs.
pub(super) fn needs_surface(op: &Operation) -> bool {
    LIGHTING_OPCODES.contains(&op.ids[1]) && op.ids[1] != PBR_VALIDATE_OPCODE
}

/// Reorders one layer's channel operations (in set order now) so that the channels P6 effects
/// read come before Base Color: a stack read of the layer's material (`effects[1] = 1`) captures
/// everything before the Base Color operation, as the CPU lays the other channels first
/// (`Texel::lit_content`). Each operation writes its own channel, so the order changes nothing else.
pub(super) fn order_layer_operations(operations: &mut [Operation]) {
    let base_color = channel_slot(Channel::BaseColor);
    if let Some(index) = operations.iter().position(|op| op.ids[0] == base_color) {
        operations[index..].rotate_left(1);
    }
}

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

    /// PBR Validate checks the base color against the metal of the layer's material: the metal
    /// below with the layer's own Metallic laid on, which needs its Base Color operation last.
    /// Painted rows of gray away from every range edge over metal that flips in the bottom
    /// quarter, under a mask: the library default, every mode and range at opacity below 1 with
    /// a non-Normal blend, Disable, every channel switched on, chained after Invert and before a
    /// Blur (a partial update covers its halo), and the mask, which PBR Validate leaves alone.
    #[test]
    fn pbr_validate_matches_the_cpu_on_the_layers_metal() {
        use std::sync::Arc;

        use texturecraft_compose::{BakedMaps, ImageStore, PixelStore};
        use texturecraft_core::{BlendMode, Channel, Effect, Layer, LayerKind, Mask, MaskBackground, PbrValidate, Project, Projection, Sampling, Source, TextureSet, ValidationMode};
        use texturecraft_geom::primitives;
        use texturecraft_texel::{TILE, to_u16};

        use crate::gpu_composite::CompositeRegion;
        use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu, headless};

        const SIZE: u32 = TILE * 2;
        // sRGB grays away from 30, 50 and 240 of 255 and from 60 and 70 percent, inverted too
        const GRAYS_SRGB: [f32; 5] = [0.05, 0.15, 0.3, 0.65, 0.99];
        let validate = Filter::PbrValidate(PbrValidate::LIBRARY);
        let (material, mask) = compile_effects(Some(effect(EffectKind::Filter(validate.clone()))), Some(effect(EffectKind::Filter(validate.clone()))), ContentsOffset::NONE);
        assert!(material.is_ok() && mask.is_ok(), "{material:?} {mask:?}");

        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let (baked, images) = (BakedMaps::default(), ImageStore::default());
        // Each effect: filter, opacity, blend, and whether it starts on Base Color only as the library adds it
        let build = |content: &[(Filter, f32, BlendMode, bool)]| {
            let mut project = Project::new("PBR Validate", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
            let below = project.alloc_id();
            project.insert_layer(0, Layer::new(below, "Below", LayerKind::Paint, &Channel::DEFAULT_SET), None, false).unwrap();
            let owner = project.alloc_id();
            let mut layer = Layer::new(owner, "Checked", LayerKind::Paint, &Channel::DEFAULT_SET);
            for (filter, opacity, blend, base_color_only) in content {
                let mut added = Effect::new(project.alloc_id(), EffectKind::Filter(filter.clone()));
                added.opacity = *opacity;
                added.blend = *blend;
                if *base_color_only {
                    added.off_channels = Channel::DEFAULT_SET.into_iter().filter(|&channel| channel != Channel::BaseColor).collect();
                }
                layer.content_effects.push(added);
            }
            let mut mask = Mask::new(MaskBackground::White);
            let fill = EffectKind::Fill { source: Source::Uniform([0.6; 3]), projection: Projection::default(), sampling: Sampling::SAVED_BEFORE };
            mask.effects.push(Effect::new(project.alloc_id(), fill));
            let mut masked = Effect::new(project.alloc_id(), EffectKind::Filter(Filter::PbrValidate(PbrValidate::LIBRARY)));
            masked.opacity = 0.63;
            masked.blend = BlendMode::Multiply;
            mask.effects.push(masked);
            layer.mask = Some(mask);
            project.insert_layer(0, layer, None, false).unwrap();
            (project, below, owner)
        };
        let (_, below, owner) = build(&[]);
        let linear = |srgb: f32| if srgb <= 0.040_45 { srgb / 12.92 } else { ((srgb + 0.055) / 1.055).powf(2.4) };
        let mut pixels = PixelStore::default();
        for y in 0..SIZE {
            for x in 0..SIZE {
                let left = x < SIZE / 2;
                let gray = to_u16(linear(GRAYS_SRGB[(y * GRAYS_SRGB.len() as u32 / SIZE) as usize]));
                pixels.layer_mut(owner, Channel::BaseColor, (SIZE, SIZE)).set(x, y, &[gray, gray, gray, u16::MAX]);
                pixels.layer_mut(below, Channel::Metallic, (SIZE, SIZE)).set(x, y, &[to_u16(if left { 1.0 } else { 0.0 }), u16::MAX]);
                // The layer's own metal flips the metal below, 0.6 over it under the mask
                if y >= SIZE * 3 / 4 {
                    pixels.layer_mut(owner, Channel::Metallic, (SIZE, SIZE)).set(x, y, &[to_u16(if left { 0.0 } else { 1.0 }), u16::MAX]);
                }
            }
        }

        let library = (validate.clone(), 1.0, BlendMode::Normal, true);
        let mut cases = vec![
            ("library default".to_string(), vec![library.clone()]),
            ("disabled".to_string(), vec![(validate.clone(), 0.77, BlendMode::Disable, true)]),
            ("every channel on".to_string(), vec![(validate.clone(), 0.77, BlendMode::Normal, false)]),
            // Invert on every channel: the material holds the inverted metal
            ("after Invert".to_string(), vec![(Filter::Invert, 1.0, BlendMode::Normal, false), library.clone()]),
        ];
        for mode in ValidationMode::ALL {
            for albedo_dark in PbrValidate::ALBEDO_DARK {
                for metal_low in PbrValidate::METAL_LOW {
                    let filter = Filter::PbrValidate(PbrValidate { mode, albedo_dark, metal_low });
                    cases.push((format!("{mode:?} {albedo_dark} {metal_low}"), vec![(filter, 0.77, BlendMode::Multiply, true)]));
                }
            }
        }
        for (label, content) in &cases {
            let (project, _, owner) = build(content);
            let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
            case.mask_owner = Some(owner);
            assert_matches_cpu(&mut gpu, &case, Tolerance::POINT, label);
        }

        // A filter after it: the stage's prepare pass reads the same material
        let (project, _, owner) = build(&[library, (Filter::Blur { radius: 24.0, wrap: false }, 1.0, BlendMode::Normal, true)]);
        let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
        case.mask_owner = Some(owner);
        case.region = Some(CompositeRegion { min: [61, 57], max: [75, 73] });
        assert_matches_cpu(&mut gpu, &case, Tolerance::NEIGHBOURHOOD, "before a Blur");
    }
}
