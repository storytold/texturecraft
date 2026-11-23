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

use super::packages::{STACK_OP_BITS, STACK_OPCODES};
use super::CompositeProgram;

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

    use texturecraft_compose::PixelStore;
    use texturecraft_core::{
        BlendMode, Channel, ChannelBlend, ColorSelection, CompareOp, CompareSource, ContentsOffset, EffectKind, Fill, Filter, Layer, LayerKind, Levels, Mask,
        MaskBackground, Project, SelectionBackground, Source, TextureSet,
    };

    use crate::gpu_composite::oracle::{compile_effects, effect};
    use crate::gpu_composite::{CompositeProgram, effect_needs_cpu, reads_image_input};

    /// P3's effect settings that still composite on the CPU. Port one: delete its row or block
    /// (these tests fail while a ported case stays) and add its oracle test.
    #[test]
    fn stack_settings_still_on_the_cpu_fall_back() {
        let none = ContentsOffset::NONE;

        // Levels limited to one channel of the content
        let one_channel = Levels { affects: Some(Channel::Height), ..Levels::default() };
        assert!(compile_effects(Some(effect(EffectKind::Levels(one_channel))), None, none).0.is_err());

        // Per-channel blends and channels switched off on content effects
        let mut per_channel = effect(EffectKind::Filter(Filter::Invert));
        per_channel.channel_blends.insert(Channel::Height, ChannelBlend { mode: BlendMode::Multiply, opacity: 0.5 });
        let mut switched_off = effect(EffectKind::Filter(Filter::Invert));
        switched_off.off_channels.insert(Channel::Height);
        for effect in [per_channel, switched_off] {
            assert!(compile_effects(Some(effect.clone()), None, none).0.is_err(), "{effect:?}");
        }

        // Only an input the filter reads counts: Grayscale Conversion reads its Custom Input once switched on
        let conversion = texturecraft_core::GrayscaleConversion { custom_input: true, ..texturecraft_core::GrayscaleConversion::LIBRARY };
        let mut reads_input = effect(EffectKind::Filter(Filter::GrayscaleConversion(conversion)));
        reads_input.inputs.insert(texturecraft_core::ImageInput::CUSTOM_INPUT.id.into(), Source::Uniform([0.5; 3]));
        assert!(effect_needs_cpu(&reads_input, false).is_some() && reads_image_input(&reads_input));

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
        if let LayerKind::Group { contents, .. } = &mut project.layer_mut(inner).unwrap().kind {
            *contents = ContentsOffset::NONE;
        }
        assert!(CompositeProgram::try_compile_region(&project, &pixels, 0, None).is_ok(), "the same stack without the offset");

        // An instance inside such a folder draws its source's mask placed by the instance's folders
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
        assert!(CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, source, None, None).is_ok(), "the source itself sits outside the folder");
        assert!(CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, instance, None, None).is_err());
    }
}
