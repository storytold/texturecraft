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
//! P6 also owns `BAKE_MAPS`, `BakedInputs` and `GpuCompositor::baked_input` in gpu_composite.rs.
//! The detail normal reads the baked Height and World Normal from the bake rows
//! (`ProceduralInputs::read_bake_rows`, P0h) and the surface tangent (`Surface.tangent`).
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

use texturecraft_core::{Channel, DIELECTRIC_REFLECTANCE, Effect, Filter, PbrValidate, finite_unit};

use super::packages::{LIGHTING_OP_BITS, LIGHTING_OPCODES};
use super::{CompositeProgram, Operation, ProceduralInputs, channel_slot};

/// Library filter ids P6 still leaves on the CPU with their defaults (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &[];

pub(super) const WGSL: &str = include_str!("shaders/composite_lighting.wgsl");
/// The same functions without P6's code, for programs that do not use it.
pub(super) const WGSL_STUB: &str = "fn lighting_scalar_filter(op: Operation, x: f32) -> f32 { return x; }
fn lighting_effect_color(op: Operation, c: vec3<f32>, channel: u32, surface: Surface, baked_ao: f32) -> vec3<f32> { return c; }
fn lighting_content_effect(effect: Operation, content: vec4<f32>, channel: u32, texel: LayerTexel) -> vec4<f32> {
    let filtered = lighting_effect_color(effect, content.rgb, channel, texel.surface, texel.baked.x);
    return vec4(content.rgb + (filtered - content.rgb) * effect.low.w, content.a);
}
fn lighting_mask_effect(effect: Operation, mask: f32, texel: LayerTexel) -> f32 { return lighting_scalar_filter(effect, mask); }";

/// Baked Lighting Stylized: sun, two lights and sky shading the base color with the layer's material
const STYLIZED_OPCODE: u32 = LIGHTING_OPCODES.start;
/// Baked Lighting Environment: the viewport environment's diffuse light on the base color
const ENVIRONMENT_OPCODE: u32 = LIGHTING_OPCODES.start + 1;
/// PBR Validate: the base color checked against the metal of the layer's material
const PBR_VALIDATE_OPCODE: u32 = LIGHTING_OPCODES.start + 2;
/// Baked Lighting Stylized's flags in `ids[2]`: Specular Glossiness, the diffuse and specular
/// outputs, and the detail normal (Mesh + Height + Normal)
const SPECULAR_GLOSSINESS: u32 = 1;
const LIT_DIFFUSE: u32 = 2;
const LIT_SPECULAR: u32 = 4;
const DETAIL_NORMAL: u32 = 8;
/// A smoothness that is not a number fades in as a step on the CPU (`fade_in`), as any value at or below its step limit does
const STEP_SMOOTHNESS: f32 = 0.0;
/// Baked Lighting Environment's Height Intensity per unit of relief (`ENVIRONMENT_HEIGHT_UNIT` in compose's eval.rs)
const ENVIRONMENT_HEIGHT_UNIT: f32 = 4.0;
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
pub(super) fn lower(filter: &Filter, _effect: &Effect, op: &mut Operation, inputs: &mut ProceduralInputs) -> Option<(u32, [f32; 3])> {
    match filter {
        Filter::BakedLighting { .. } => stylized(filter, op, inputs),
        Filter::EnvironmentLighting { horizontal, vertical, exposure, height_intensity, ambient_occlusion, specular_occlusion } => {
            let light = inputs.reserve_environment()?;
            let (sin_tilt, cos_tilt) = vertical.to_radians().sin_cos();
            let stops = Filter::ENVIRONMENT_EXPOSURE_STOPS;
            let values = [
                horizontal.to_radians(),
                sin_tilt,
                cos_tilt,
                exposure.clamp(-stops, stops).exp2(),
                height_intensity / ENVIRONMENT_HEIGHT_UNIT,
                finite_unit(*ambient_occlusion),
                finite_unit(*specular_occlusion),
                DIELECTRIC_REFLECTANCE,
            ];
            if values.iter().any(|value| !value.is_finite()) {
                return None;
            }

            // The environment's late-bound words first, then the settings
            let words: Vec<u32> = std::iter::once(light).chain(values.map(f32::to_bits)).collect();
            op.ext[0] = inputs.pack_words(&words)?;
            read_material(op, Channel::Metallic);
            inputs.read_bake_rows();
            Some((ENVIRONMENT_OPCODE, [0.0; 3]))
        }
        Filter::PbrValidate(validate) => {
            let flag = |on: bool, bit: u32| if on { bit } else { 0 };
            op.ids[2] = flag(validate.mode.checks_albedo(), CHECKS_ALBEDO) | flag(validate.mode.checks_metal(), CHECKS_METAL);
            let [red, green, blue] = PbrValidate::INVALID;
            op.high = [red, green, blue, METAL_THRESHOLD];
            read_material(op, Channel::Metallic);
            let range = [f32::from(validate.albedo_dark) / SRGB_STEPS, f32::from(validate.metal_low) / PERCENT, ALBEDO_BRIGHT_SRGB_STEPS / SRGB_STEPS];
            Some((PBR_VALIDATE_OPCODE, range))
        }
        _ => None,
    }
}

/// Baked Lighting Stylized as `StylizedShading::new` works its settings out, in the side table:
/// three lights of seven words (direction toward it, intensity, color), the sky intensity, the
/// sky, horizon and ground colors, the AO, cavity and edges amounts, the two smoothnesses, the
/// dielectric reflectance and the relief. Highlights read the material's Metallic and Roughness,
/// or Glossiness and Specular; None for Baked Lighting saved before the stylized settings.
fn stylized(filter: &Filter, op: &mut Operation, inputs: &mut ProceduralInputs) -> Option<(u32, [f32; 3])> {
    let (settings, light) = filter.stylized_lighting()?;
    let lights = std::iter::once((light.sun.direction(), light.sun.intensity, settings.sun_color))
        .chain(settings.lights.iter().map(|extra| (extra.direction(), extra.intensity, extra.color)));
    let mut values: Vec<f32> = lights.flat_map(|(direction, intensity, color)| direction.into_iter().chain([intensity]).chain(color)).collect();
    let smoothness = |value: f32| if value.is_nan() { STEP_SMOOTHNESS } else { value };
    let relief = (!settings.mesh_only).then_some(settings.height_to_normal);
    values.push(light.sky_intensity);
    values.extend(settings.sky_color.into_iter().chain(settings.horizon_color).chain(settings.ground_color));
    values.extend([
        finite_unit(light.diffuse_ao),
        finite_unit(settings.specular_ao),
        finite_unit(settings.diffuse_cavity),
        finite_unit(settings.specular_cavity),
        finite_unit(settings.edges_intensity),
        smoothness(settings.cavity_smoothness),
        smoothness(settings.edges_smoothness),
        finite_unit(settings.dielectric_reflectance),
        relief.unwrap_or(0.0),
    ]);
    op.ext[0] = inputs.pack_params(&values)?;

    let output = settings.output;
    let flag = |on: bool, bit: u32| if on { bit } else { 0 };
    op.ids[2] = flag(settings.specular_glossiness, SPECULAR_GLOSSINESS)
        | flag(output.diffuse(), LIT_DIFFUSE)
        | flag(output.specular(), LIT_SPECULAR)
        | flag(relief.is_some(), DETAIL_NORMAL);

    // Only highlights read the material, as the CPU lights a layer by it (`lights_material`)
    if output.specular() {
        let (first, second) = if settings.specular_glossiness { (Channel::Glossiness, Channel::Specular) } else { (Channel::Metallic, Channel::Roughness) };
        read_material(op, first);
        op.spatial[2] = channel_slot(second) + 1;
    }
    if relief.is_some() {
        inputs.read_bake_rows();
    }
    Some((STYLIZED_OPCODE, [0.0; 3]))
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
    use texturecraft_core::{ContentsOffset, EffectKind, Filter};

    use crate::gpu_composite::oracle::{compile_effects, effect};

    /// P6's filters that still composite on the CPU in content (a mask skips lighting on both
    /// sides). Port one: delete its row (this test fails while a ported row stays) and add its
    /// oracle test.
    fn still_on_cpu() -> Vec<Filter> {
        Vec::new()
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
        use texturecraft_core::{
            BlendMode, Channel, Effect, Layer, LayerKind, Mask, MaskBackground, PbrValidate, Project, Projection, Sampling, Source, TextureSet, ValidationMode,
        };
        use texturecraft_geom::primitives;
        use texturecraft_texel::{TILE, to_u16};

        use crate::gpu_composite::CompositeRegion;
        use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu, headless};

        const SIZE: u32 = TILE * 2;
        // sRGB grays away from 30, 50 and 240 of 255 and from 60 and 70 percent, inverted too
        const GRAYS_SRGB: [f32; 5] = [0.05, 0.15, 0.3, 0.65, 0.99];
        let validate = Filter::PbrValidate(PbrValidate::LIBRARY);
        let (material, mask) =
            compile_effects(Some(effect(EffectKind::Filter(validate.clone()))), Some(effect(EffectKind::Filter(validate.clone()))), ContentsOffset::NONE);
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

    /// One content effect of the lit layer: filter, opacity, blend, and whether it starts on Base
    /// Color only, as the library adds lighting
    type LitEffect = (texturecraft_core::Filter, f32, texturecraft_core::BlendMode, bool);

    const LIT_SIZE: u32 = texturecraft_texel::TILE * 2;
    /// Bakes at half the set's resolution: a texel's bake sample never lies on a bake texel's edge
    const LIT_BAKE_SIZE: usize = 64;

    /// A sphere's set with Glossiness and Specular: a fill of the whole material below, and a lit
    /// fill laying its own Metallic, Roughness and Specular over it under a 0.7 mask that also
    /// holds a Baked Lighting (masks skip it). `edit` changes the project (its display settings).
    fn lit_project(effects: &[LitEffect], edit: impl FnOnce(&mut texturecraft_core::Project)) -> (texturecraft_core::Project, texturecraft_core::LayerId) {
        use std::collections::BTreeMap;

        use texturecraft_core::{
            Channel, ChannelFormat, Effect, Fill, Layer, LayerKind, Mask, MaskBackground, Project, Projection, Sampling, Source, StylizedLighting, TextureSet,
        };

        let mut project = Project::new("Lighting", "builtin:sphere");
        let mut set = TextureSet::new("Sphere", 0, LIT_SIZE);
        set.channels.extend([Channel::Glossiness, Channel::Specular].map(|channel| (channel, ChannelFormat::default_for(channel))));
        project.texture_sets.push(set);
        let channels: Vec<Channel> = project.texture_sets[0].channels.iter().map(|&(channel, _)| channel).collect();
        let below = BTreeMap::from([
            (Channel::BaseColor, Source::Uniform([0.3; 3])),
            (Channel::Metallic, Source::Uniform([0.7; 3])),
            (Channel::Roughness, Source::Uniform([0.35; 3])),
            (Channel::Glossiness, Source::Uniform([0.6; 3])),
            (Channel::Specular, Source::Uniform([0.3, 0.5, 0.2])),
        ]);
        let below_id = project.alloc_id();
        let material =
            project.insert_layer(0, Layer::new(below_id, "Material", LayerKind::Fill(Fill { sources: below, ..Default::default() }), &channels), None, false);
        assert!(material.is_ok(), "{material:?}");
        let lit = BTreeMap::from([
            (Channel::BaseColor, Source::Uniform([0.62, 0.45, 0.3])),
            (Channel::Metallic, Source::Uniform([0.2; 3])),
            (Channel::Roughness, Source::Uniform([0.55; 3])),
            (Channel::Specular, Source::Uniform([0.6, 0.4, 0.2])),
        ]);
        let owner = project.alloc_id();
        let mut layer = Layer::new(owner, "Lit", LayerKind::Fill(Fill { sources: lit, ..Default::default() }), &channels);
        for (filter, opacity, blend, base_color_only) in effects {
            let mut added = Effect::new(project.alloc_id(), texturecraft_core::EffectKind::Filter(filter.clone()));
            added.opacity = *opacity;
            added.blend = *blend;
            if *base_color_only {
                added.off_channels = channels.iter().copied().filter(|&channel| channel != Channel::BaseColor).collect();
            }
            layer.content_effects.push(added);
        }
        let mut mask = Mask::new(MaskBackground::White);
        let fill =
            texturecraft_core::EffectKind::Fill { source: Source::Uniform([0.7; 3]), projection: Projection::default(), sampling: Sampling::SAVED_BEFORE };
        mask.effects.push(Effect::new(project.alloc_id(), fill));
        let skipped = stylized_lighting(StylizedLighting::library());
        mask.effects.push(Effect::new(project.alloc_id(), texturecraft_core::EffectKind::Filter(skipped)));
        layer.mask = Some(mask);
        let lit_layer = project.insert_layer(0, layer, None, false);
        assert!(lit_layer.is_ok(), "{lit_layer:?}");
        edit(&mut project);
        (project, owner)
    }

    /// Baked Lighting Stylized with a sun from the side and some occlusion
    fn stylized_lighting(settings: texturecraft_core::StylizedLighting) -> texturecraft_core::Filter {
        texturecraft_core::Filter::BakedLighting {
            azimuth: 30.0,
            elevation: 50.0,
            light: 1.0,
            ambient: 1.0,
            occlusion: 0.4,
            stylized: Some(Box::new(settings)),
        }
    }

    /// Smooth AO, a curvature ramp that never lands on flat, and, when asked, a wavy Height and a
    /// World Normal that is zero (the mesh's normal) along the left edge
    fn lit_bakes(height: bool, world_normal: bool) -> texturecraft_compose::BakedMaps {
        use std::collections::BTreeMap;
        use std::f32::consts::TAU;
        use std::sync::Arc;

        use texturecraft_core::BakedMap;

        let at = |index: usize| ((index % LIT_BAKE_SIZE) as f32 / LIT_BAKE_SIZE as f32, (index / LIT_BAKE_SIZE) as f32 / LIT_BAKE_SIZE as f32);
        let texels = 0..LIT_BAKE_SIZE * LIT_BAKE_SIZE;
        let mut maps = BTreeMap::new();
        maps.insert(BakedMap::AmbientOcclusion, Arc::new(texels.clone().map(|index| 0.35 + 0.6 * at(index).0 * at(index).1).collect::<Vec<_>>()));
        maps.insert(BakedMap::Curvature, Arc::new(texels.clone().map(|index| 0.21 + 0.6 * at(index).0).collect::<Vec<_>>()));
        if height {
            let wave = |(u, v): (f32, f32)| 0.5 + 0.15 * (u * TAU * 2.0).sin() * (v * TAU).cos();
            maps.insert(BakedMap::Height, Arc::new(texels.clone().map(|index| wave(at(index))).collect::<Vec<_>>()));
        }
        if world_normal {
            let packed = |(u, v): (f32, f32)| if u < 0.1 { [0.5; 3] } else { [0.5 + 0.25 * (v * TAU).sin(), 0.85, 0.5 + 0.25 * (u * TAU).cos()] };
            maps.insert(BakedMap::WorldNormal, Arc::new(texels.flat_map(|index| packed(at(index))).collect::<Vec<_>>()));
        }
        texturecraft_compose::BakedMaps { resolution: LIT_BAKE_SIZE as u32, maps }
    }

    /// Composite `project` against the CPU with `baked`, the lit layer's mask preview too, and a
    /// partial update when `region` is set
    fn assert_lit_matches(
        gpu: &mut crate::headless::Headless,
        (project, owner): &(texturecraft_core::Project, texturecraft_core::LayerId),
        baked: &texturecraft_compose::BakedMaps,
        region: bool,
        label: &str,
    ) {
        use std::sync::Arc;

        use texturecraft_compose::{ImageStore, PixelStore};
        use texturecraft_geom::primitives;

        use crate::gpu_composite::CompositeRegion;
        use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu};

        let mesh = Arc::new(primitives::sphere(1.0, 24, 12));
        let (pixels, images) = (PixelStore::default(), ImageStore::default());
        let mut case = OracleCase::new(project, &pixels, baked, &images, &mesh);
        case.mask_owner = Some(*owner);
        if region {
            case.region = Some(CompositeRegion { min: [61, 57], max: [75, 73] });
        }
        assert_matches_cpu(gpu, &case, Tolerance::TRANSCENDENTAL, label);
    }

    /// Baked Lighting Stylized lights the base color as the CPU does: its sun, extra lights and sky
    /// on the sphere's normals tilted by the baked World Normal and Height along the surface
    /// tangent, with highlights from the material's Metallic and Roughness or Glossiness and
    /// Specular (the layer's own laid over the stack below), cavities and edges from the baked
    /// curvature. The library default, each output, Specular Glossiness, the mesh's own normals,
    /// extra lights with cavity, edges and specular AO, a smoothness that is not a number, opacity
    /// below 1 with a non-Normal blend, Disable, every channel on, missing Height or World Normal
    /// or no bakes, and chained after Invert and before a Blur with a partial update.
    #[test]
    fn baked_lighting_stylized_matches_the_cpu() {
        use texturecraft_core::{BlendMode, DirectionalLight, LightingOutput, StylizedLighting};

        use crate::gpu_composite::oracle::headless;

        let library = StylizedLighting::library();
        assert!(compile_effects(Some(effect(EffectKind::Filter(stylized_lighting(library.clone())))), None, ContentsOffset::NONE).0.is_ok());

        let Some(mut gpu) = headless() else { return };
        let detailed = StylizedLighting {
            lights: [
                DirectionalLight { azimuth: 120.0, elevation: 20.0, intensity: 0.6, color: [0.4, 0.6, 1.0] },
                DirectionalLight { azimuth: 250.0, elevation: 60.0, intensity: 0.3, color: [1.0, 0.5, 0.3] },
            ],
            diffuse_cavity: 0.6,
            specular_cavity: 0.4,
            specular_ao: 0.5,
            cavity_smoothness: 0.25,
            edges_intensity: 0.7,
            edges_smoothness: 0.3,
            height_to_normal: 2.5,
            ..library.clone()
        };
        let settings = [
            ("library default", library.clone()),
            ("diffuse", StylizedLighting { output: LightingOutput::Diffuse, ..detailed.clone() }),
            ("specular", StylizedLighting { output: LightingOutput::Specular, ..detailed.clone() }),
            ("specular glossiness", StylizedLighting { specular_glossiness: true, ..detailed.clone() }),
            ("mesh only", StylizedLighting { mesh_only: true, ..detailed.clone() }),
            ("extra lights, cavity and edges", detailed.clone()),
            ("smoothness not a number", StylizedLighting { cavity_smoothness: f32::NAN, edges_smoothness: 0.0, ..detailed.clone() }),
        ];
        let baked = lit_bakes(true, true);
        for (label, settings) in settings {
            assert_lit_matches(&mut gpu, &lit_project(&[(stylized_lighting(settings), 1.0, BlendMode::Normal, true)], |_| {}), &baked, false, label);
        }

        let filter = stylized_lighting(detailed);
        let blends = [
            ("opacity and Multiply", (filter.clone(), 0.77, BlendMode::Multiply, true)),
            ("disabled", (filter.clone(), 0.77, BlendMode::Disable, true)),
            ("every channel on", (filter.clone(), 0.77, BlendMode::Normal, false)),
        ];
        for (label, lit) in blends {
            assert_lit_matches(&mut gpu, &lit_project(&[lit], |_| {}), &baked, false, label);
        }

        let lit = (filter, 1.0, BlendMode::Normal, true);
        for (label, baked) in
            [("no Height", lit_bakes(false, true)), ("no World Normal", lit_bakes(true, false)), ("no bakes", texturecraft_compose::BakedMaps::default())]
        {
            assert_lit_matches(&mut gpu, &lit_project(std::slice::from_ref(&lit), |_| {}), &baked, false, label);
        }

        // Invert on every channel changes the material it reads; a Blur after it reads the lit base color in its prepare pass
        let chain = [(Filter::Invert, 1.0, BlendMode::Normal, false), lit.clone(), (Filter::Blur { radius: 24.0, wrap: false }, 1.0, BlendMode::Normal, true)];
        assert_lit_matches(&mut gpu, &lit_project(&chain, |_| {}), &baked, true, "after Invert and before a Blur");
    }

    /// Baked Lighting Environment lights the base color with the viewport environment's diffuse
    /// light as the CPU does: turned and tilted, exposed, on the detail normal at its Height
    /// Intensity, occluded by the baked AO, with the material's metal. Defaults, every setting
    /// changed, a turned viewport environment and another environment, opacity below 1 with a
    /// non-Normal blend, Disable, missing bakes, and chained after Invert and before a Blur with a
    /// partial update.
    #[test]
    fn baked_lighting_environment_matches_the_cpu() {
        use texturecraft_core::BlendMode;

        use crate::gpu_composite::oracle::headless;

        let environment = |horizontal, vertical, exposure, height_intensity, ambient_occlusion, specular_occlusion| Filter::EnvironmentLighting {
            horizontal,
            vertical,
            exposure,
            height_intensity,
            ambient_occlusion,
            specular_occlusion,
        };
        let defaults = environment(0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
        assert!(compile_effects(Some(effect(EffectKind::Filter(defaults.clone()))), None, ContentsOffset::NONE).0.is_ok());

        let Some(mut gpu) = headless() else { return };
        let baked = lit_bakes(true, true);
        let changed = environment(75.0, 30.0, -1.5, 2.5, 0.4, 0.8);
        let on_base = |filter: &Filter| (filter.clone(), 1.0, BlendMode::Normal, true);
        assert_lit_matches(&mut gpu, &lit_project(&[on_base(&defaults)], |_| {}), &baked, false, "defaults");
        assert_lit_matches(&mut gpu, &lit_project(&[on_base(&changed)], |_| {}), &baked, false, "every setting");
        let turned = lit_project(&[on_base(&changed)], |project| project.display.env_rotation = 70.0);
        assert_lit_matches(&mut gpu, &turned, &baked, false, "turned environment");
        let night = lit_project(&[on_base(&changed)], |project| project.display.environment = "Night".into());
        assert_lit_matches(&mut gpu, &night, &baked, false, "another environment");

        for (label, lit) in
            [("opacity and Multiply", (changed.clone(), 0.77, BlendMode::Multiply, true)), ("disabled", (changed.clone(), 0.77, BlendMode::Disable, true))]
        {
            assert_lit_matches(&mut gpu, &lit_project(&[lit], |_| {}), &baked, false, label);
        }
        for (label, baked) in
            [("no Height", lit_bakes(false, true)), ("no World Normal", lit_bakes(true, false)), ("no bakes", texturecraft_compose::BakedMaps::default())]
        {
            assert_lit_matches(&mut gpu, &lit_project(&[on_base(&changed)], |_| {}), &baked, false, label);
        }

        let chain =
            [(Filter::Invert, 1.0, BlendMode::Normal, false), on_base(&changed), (Filter::Blur { radius: 24.0, wrap: false }, 1.0, BlendMode::Normal, true)];
        assert_lit_matches(&mut gpu, &lit_project(&chain, |_| {}), &baked, true, "after Invert and before a Blur");
    }
}
