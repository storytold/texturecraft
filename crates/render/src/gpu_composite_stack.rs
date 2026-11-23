//! P3, stack plumbing and mask effects (plan section 2.4, without the UDIM and baked-layout
//! rows, which are P8's). Owned by the P3 agent; no other package edits this file or
//! `shaders/composite_stack.wgsl`.
//!
//! What P3 implements: per-channel blends and switched-off channels on content effects (every
//! new lighting, Color Match, PBR Validate and Height To Normal effect starts with channels off),
//! Levels `affects`, effect image inputs (a source evaluated into the stage field's sixth word),
//! Compare reading This Layer or Layers Below and Compare `hardness` below 1, Color Selection
//! `output`/`hardness`/`background` and its ID-map image, folder Scale and Rotation (also around
//! instances and in the mask preview), and the solo view.
//!
//! P3 also owns these shared regions outright (no other package edits them):
//! - in gpu_composite.rs: the stack loop of `try_compile_with_brush`, `compile_mask`,
//!   `content_filter`, `content_deposit`, `content_depends_on_channel`, `uniform_blend`,
//!   `effect_needs_cpu` and `reads_image_input` (P6 only reorders finished operations through
//!   `packages::order_layer_operations`, after P3 has merged);
//! - `shaders/composite_mask_effects.wgsl` and `shaders/composite_content_effects.wgsl` (P7's
//!   erase preview coordinates with P3 there);
//! - in crates/ui-egui/src/gpu.rs: `view_project` (which project a view compiles: the solo
//!   copy, `compose::solo_project`), the solo term of `gpu_composite_reason`, and the status
//!   text "The active view uses CPU compositing." for aux views (reword it: the material is
//!   hidden there, nothing composites on the CPU).
//!
//! How:
//! - New mask or content opcodes come from `packages::STACK_OPCODES` (144..152), handled in WGSL
//!   by `stack_scalar_filter`/`stack_effect_color` or the loop-level `stack_content_effect`/
//!   `stack_mask_effect` (or directly in composite_mask_effects.wgsl);
//!   flags go in `op.ext[2] & STACK_OP_BITS`, numbers in the side table (`pack_params`).
//! - Inputs: set `FilterStage::input` to the source operation's index plus one; the prepare pass
//!   then writes `stage_input_value(op, texel)` (this WGSL file) into the field's sixth word and
//!   every field of the program grows to six words (`FilterPlan::field_words`). Images go through
//!   `MaterialImages::register_image(name, ImageUse::Material)`.
//! - Folder offsets: call `ContentsOffset::apply` on the `Projection` before `source_operation`
//!   (P1 owns the projection words; this call site is P3's).
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] and the tests below. Every port gets a CPU oracle test
//! (`crate::gpu_composite::oracle`) per context and setting value, chained and with a region.

use super::CompositeProgram;
use super::packages::{STACK_OP_BITS, STACK_OPCODES};

/// Library filter ids P3 still leaves on the CPU (filters whose only blocker is a P3 setting are
/// listed under the package that ports the filter).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &[];

pub(super) const WGSL: &str = include_str!("shaders/composite_stack.wgsl");
/// The same functions without P3's code, for programs that do not use it.
pub(super) const WGSL_STUB: &str = "fn stack_scalar_filter(op: Operation, x: f32) -> f32 { return x; }
fn stack_effect_color(op: Operation, c: vec3<f32>, channel: u32, surface: Surface, baked_ao: f32) -> vec3<f32> { return c; }
fn stack_content_effect(effect: Operation, content: vec4<f32>, channel: u32, texel: LayerTexel) -> vec4<f32> {
    let filtered = stack_effect_color(effect, content.rgb, channel, texel.surface, texel.baked.x);
    return vec4(content.rgb + (filtered - content.rgb) * effect.low.w, content.a);
}
fn stack_mask_effect(effect: Operation, mask: f32, texel: LayerTexel) -> f32 { return stack_scalar_filter(effect, mask); }
fn stage_input_value(op: Operation, texel: LayerTexel) -> f32 { return base_content(op, texel).x; }";

/// Whether a program needs the full `composite_stack.wgsl`.
pub(super) fn uses_shader(program: &CompositeProgram) -> bool {
    program.filters.stages.iter().any(|stage| stage.input != 0)
        || program.operations.iter().any(|op| STACK_OPCODES.contains(&op.ids[1]) || op.ext[2] & STACK_OP_BITS != 0)
}

/// P3's vec4 of the band uniform (`params.packages[2]`).
pub(super) fn band_uniform(_program: &CompositeProgram) -> [u32; 4] {
    [0; 4]
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use texturecraft_compose::{BakedMaps, ImageStore, PixelStore};
    use texturecraft_core::{
        BlendMode, Channel, ChannelBlend, ColorSelection, CompareOp, CompareSource, ContentsOffset, Effect, EffectKind, Fill, Filter, Layer, LayerId, LayerKind,
        Levels, Mask, MaskBackground, Project, Projection, Sampling, SelectionBackground, Source, TextureSet,
    };
    use texturecraft_geom::primitives;
    use texturecraft_texel::{TILE, to_u16, to_u16_signed};

    use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu, compile_effects, effect, headless};
    use crate::gpu_composite::{CompositeProgram, CompositeRegion, effect_needs_cpu, reads_image_input};

    /// P3's effect settings that still composite on the CPU. Port one: delete its row or block
    /// (these tests fail while a ported case stays) and add its oracle test.
    #[test]
    fn stack_settings_still_on_the_cpu_fall_back() {
        let none = ContentsOffset::NONE;

        // Only an input the filter reads counts: Grayscale Conversion reads its Custom Input once switched on
        let conversion = texturecraft_core::GrayscaleConversion { custom_input: true, ..texturecraft_core::GrayscaleConversion::LIBRARY };
        let mut reads_input = effect(EffectKind::Filter(Filter::GrayscaleConversion(conversion)));
        reads_input.inputs.insert(texturecraft_core::ImageInput::CUSTOM_INPUT.id.into(), Source::Uniform([0.5; 3]));
        assert!(effect_needs_cpu(&reads_input).is_some() && reads_image_input(&reads_input));

        // Mask comparisons reading layers or with a soft edge, and soft or ID-map colour selections
        let compare = |left, hardness| EffectKind::Compare {
            op: CompareOp::GreaterThan,
            value: 0.5,
            tolerance: 0.0,
            left,
            right: CompareSource::Constant,
            channel: Channel::Height,
            hardness,
        };
        let mut soft_selection = ColorSelection::new(vec![[1.0, 0.0, 0.0]], 0.1);
        soft_selection.hardness = 0.5;
        let mut white_selection = ColorSelection::new(vec![[1.0, 0.0, 0.0]], 0.1);
        white_selection.background = SelectionBackground::White;
        let mut gray_selection = ColorSelection::new(vec![[1.0, 0.0, 0.0]], 0.1);
        gray_selection.output = 0.5;
        let mut id_map_selection = ColorSelection::new(vec![[1.0, 0.0, 0.0]], 0.1);
        id_map_selection.id_map = Some("id.png".into());
        let layer_on_right = EffectKind::Compare {
            op: CompareOp::GreaterThan,
            value: 0.5,
            tolerance: 0.0,
            left: CompareSource::ThisMask,
            right: CompareSource::ThisLayer,
            channel: Channel::Height,
            hardness: 1.0,
        };
        for kind in [
            compare(CompareSource::LayersBelow, 1.0),
            compare(CompareSource::ThisMask, 0.5),
            layer_on_right,
            EffectKind::ColorSelection(soft_selection),
            EffectKind::ColorSelection(white_selection),
            EffectKind::ColorSelection(gray_selection),
            EffectKind::ColorSelection(id_map_selection),
        ] {
            let (material, mask) = compile_effects(None, Some(effect(kind.clone())), none);
            assert!(material.is_err() && mask.is_err(), "{kind:?}");
        }
    }

    #[test]
    fn folder_offsets_still_on_the_cpu_fall_back() {
        let channels = [Channel::BaseColor, Channel::Height];

        // A folder's Scale and Rotation re-place the fills inside it
        let (material, mask) = compile_effects(None, None, ContentsOffset::new(2.0, 30.0));
        assert!(material.is_err() && mask.is_err(), "{material:?} {mask:?}");

        // A folder nested in one without an offset, and an instance of a fill inside such a folder
        let mut project = Project::new("Nested offsets", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, 64));
        let fill = Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.6, 0.3, 0.2]))]), ..Default::default() };
        let source = project.alloc_id();
        project.insert_layer(0, Layer::new(source, "Source", LayerKind::Fill(fill), &channels), None, false).unwrap();
        let outer = project.alloc_id();
        project.insert_layer(0, Layer::new(outer, "Outer", LayerKind::folder(), &channels), None, false).unwrap();
        let inner = project.alloc_id();
        let mut group = Layer::new(inner, "Inner", LayerKind::folder(), &channels);
        if let LayerKind::Group { contents, .. } = &mut group.kind {
            *contents = ContentsOffset::new(0.5, -20.0);
        }
        project.insert_layer(0, group, Some(outer), true).unwrap();
        let pixels = PixelStore::default();
        assert!(CompositeProgram::try_compile_region(&project, &pixels, 0, None).is_err(), "a nested folder with an offset");
        let instance = project.alloc_id();
        project.insert_layer(0, Layer::new(instance, "Instance", LayerKind::Instance { source }, &channels), Some(inner), true).unwrap();
        assert!(CompositeProgram::try_compile_region(&project, &pixels, 0, None).is_err(), "an instance inside it");

        // An instance inside such a folder draws its source's mask placed by the instance's folders
        // (the source's own preview stays on the GPU: gpu_composite_packages.rs)
        let mut project = Project::new("Instanced mask", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, 64));
        let source = project.alloc_id();
        let fill = Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.6, 0.3, 0.2]))]), ..Default::default() };
        let mut layer = Layer::new(source, "Source", LayerKind::Fill(fill), &channels);
        layer.mask = Some(Mask::new(MaskBackground::White));
        project.insert_layer(0, layer, None, false).unwrap();
        let folder_id = project.alloc_id();
        let mut group = Layer::new(folder_id, "Folder", LayerKind::folder(), &channels);
        if let LayerKind::Group { contents, .. } = &mut group.kind {
            *contents = ContentsOffset::new(2.0, 30.0);
        }
        project.insert_layer(0, group, None, false).unwrap();
        let instance = project.alloc_id();
        project.insert_layer(0, Layer::new(instance, "Instance", LayerKind::Instance { source }, &channels), Some(folder_id), true).unwrap();
        let pixels = PixelStore::default();
        assert!(CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, instance, None, None).is_err());
    }

    /// A paint layer with colour, roughness and height painted in a block (so a filter's halo
    /// shows), `content` on its content and `mask` after a gray fill in its black mask.
    fn painted_layer(content: Vec<Effect>, mask: Vec<Effect>) -> (Project, PixelStore, LayerId) {
        const SIZE: u32 = TILE * 2;
        let channels = [Channel::BaseColor, Channel::Roughness, Channel::Height];
        let mut project = Project::new("Per-channel effects", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
        let owner = project.alloc_id();
        let mut layer = Layer::new(owner, "Surface", LayerKind::Paint, &channels);
        layer.content_effects = content;
        let mut layer_mask = Mask::new(MaskBackground::Black);
        let fill = EffectKind::Fill { source: Source::Uniform([0.36; 3]), projection: Projection::default(), sampling: Sampling::SAVED_BEFORE };
        layer_mask.effects.push(Effect::new(project.alloc_id(), fill));
        layer_mask.effects.extend(mask);
        layer.mask = Some(layer_mask);
        project.insert_layer(0, layer, None, false).unwrap();

        let mut pixels = PixelStore::default();
        for y in 22..103 {
            for x in 23..98 {
                let gain = if x >= 63 { 1.0 } else { 0.65 };
                pixels.layer_mut(owner, Channel::BaseColor, (SIZE, SIZE)).set(x, y, &[to_u16(0.7 * gain), to_u16(0.2 * gain), to_u16(0.3 * gain), to_u16(gain)]);
                pixels.layer_mut(owner, Channel::Roughness, (SIZE, SIZE)).set(x, y, &[to_u16(0.41 * gain), to_u16(gain)]);
                pixels.layer_mut(owner, Channel::Height, (SIZE, SIZE)).set(x, y, &[to_u16_signed(-0.34 * gain), to_u16(gain)]);
            }
        }
        (project, pixels, owner)
    }

    /// Content effects laid on each channel apart (per-channel blends, switched-off channels,
    /// Levels remapping one channel) match the CPU on colour and the other channels, alone and
    /// chained, around a neighbourhood stage with a partial update, and in the mask, which
    /// ignores the per-channel settings.
    #[test]
    fn content_effects_laid_on_each_channel_apart_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let (baked, images) = (BakedMaps::default(), ImageStore::default());
        let with = |kind: EffectKind, opacity: f32, blends: &[(Channel, BlendMode, f32)], off: &[Channel]| {
            let mut effect = effect(kind);
            effect.opacity = opacity;
            effect.channel_blends.extend(blends.iter().map(|&(channel, mode, opacity)| (channel, ChannelBlend { mode, opacity })));
            effect.off_channels.extend(off.iter().copied());
            effect
        };
        let levels = |affects| Levels { in_low: 0.2, in_high: 0.8, gamma: 1.6, out_low: 0.1, out_high: 0.85, affects, clamp: false };
        let invert = || EffectKind::Filter(Filter::Invert);
        let contrast = EffectKind::Filter(Filter::ContrastLuminosity { contrast: 0.4, luminosity: 0.05 });
        let material = EffectKind::Material(Fill {
            sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.2, 0.6, 0.35])), (Channel::Roughness, Source::Uniform([0.8; 3]))]),
            ..Default::default()
        });
        let blur = EffectKind::Filter(Filter::Blur { radius: 24.0, wrap: false });

        let cases: Vec<(&str, Vec<Effect>, Tolerance)> = vec![
            ("per-channel multiply", vec![with(invert(), 0.77, &[(Channel::Height, BlendMode::Multiply, 0.5)], &[])], Tolerance::POINT),
            ("switched off", vec![with(invert(), 0.77, &[], &[Channel::Roughness])], Tolerance::POINT),
            ("disabled on colour", vec![with(invert(), 1.0, &[(Channel::BaseColor, BlendMode::Disable, 1.0)], &[])], Tolerance::POINT),
            ("levels on height", vec![with(EffectKind::Levels(levels(Some(Channel::Height))), 0.6, &[], &[])], Tolerance::POINT),
            ("levels on colour", vec![with(EffectKind::Levels(levels(Some(Channel::BaseColor))), 1.0, &[], &[])], Tolerance::POINT),
            ("library levels", vec![with(EffectKind::Levels(Levels { affects: Some(Channel::BaseColor), ..Levels::default() }), 1.0, &[], &[])], Tolerance::POINT),
            // Levels does not blend, so Disable leaves it on
            ("levels disabled", vec![with(EffectKind::Levels(levels(Some(Channel::Roughness))), 0.7, &[(Channel::Roughness, BlendMode::Disable, 0.7)], &[])], Tolerance::POINT),
            (
                "chained",
                vec![
                    with(contrast.clone(), 0.8, &[(Channel::Roughness, BlendMode::Normal, 0.3)], &[]),
                    with(EffectKind::Levels(levels(Some(Channel::BaseColor))), 0.9, &[], &[]),
                    with(invert(), 0.5, &[], &[Channel::Height]),
                ],
                Tolerance::POINT,
            ),
            (
                "material deposit",
                vec![with(material, 0.8, &[(Channel::BaseColor, BlendMode::Multiply, 0.6)], &[Channel::Roughness]), with(invert(), 0.4, &[], &[])],
                Tolerance::POINT,
            ),
            (
                "neighbourhood stage",
                vec![
                    with(blur, 0.9, &[(Channel::Roughness, BlendMode::Normal, 0.3), (Channel::BaseColor, BlendMode::Disable, 1.0)], &[Channel::Height]),
                    with(contrast, 1.0, &[], &[]),
                ],
                Tolerance::NEIGHBOURHOOD,
            ),
        ];
        for (label, content, tolerance) in cases {
            // The mask takes the same filters (a material is content only)
            let mask = content.iter().filter(|effect| !matches!(effect.kind, EffectKind::Material(_))).cloned().collect();
            let (project, pixels, owner) = painted_layer(content, mask);
            let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
            case.mask_owner = Some(owner);
            case.region = Some(CompositeRegion { min: [61, 57], max: [75, 73] });
            assert_matches_cpu(&mut gpu, &case, tolerance, label);
        }
    }
}
