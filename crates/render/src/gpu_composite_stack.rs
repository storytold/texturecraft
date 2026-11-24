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
//! - in gpu_composite.rs: the stack loop of `try_compile_with_brush` (with the folder offset it
//!   carries per draw), `compile_mask`, `content_filter`, `content_deposit`,
//!   `content_depends_on_channel`, `uniform_blend`, `blends_per_channel`, `channel_filter`,
//!   `placed`, `effect_needs_cpu`, `reads_image_input`, and the folder offset and anchored-mask
//!   lines of `try_compile_mask_preview` (P6 only reorders finished operations through
//!   `packages::order_layer_operations`, after P3 has merged);
//! - `shaders/composite_mask_effects.wgsl` and `shaders/composite_content_effects.wgsl` (P7's
//!   erase preview coordinates with P3 there);
//! - in crates/ui-egui/src/gpu.rs: `view_project` (which project a view compiles: the solo
//!   copy, `compose::solo_project`, for the material; the document for the mask view) and the
//!   aux-view status text of `gpu_composite_reason`.
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

use texturecraft_core::project::MAX_DEPTH;
use texturecraft_core::{
    Channel, ChannelKind, ColorSelection, CompareSource, ContentsOffset, Effect, EffectKind, Filter, ImageInput, LayerId, LayerKind, Project, Projection,
    Sampling, Source,
};

use super::packages::{STACK_OP_BITS, STACK_OPCODES};
use super::{
    CompositeProgram, FIRST_EXTENDED_PATTERN, FIRST_GENERATOR, NEIGHBOURHOOD_OPCODE, Operation, ProceduralInputs, bind_tile_source, channel_slot,
    source_operation,
};
use crate::gpu_image_source::ImageUse;
use crate::gpu_tile_pattern::TilePatterns;

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
fn stage_input_value(op: Operation, texel: LayerTexel) -> f32 { return base_content(op, texel).x; }
fn stack_soft_compare(op: Operation, mask: f32) -> f32 { return mask; }
fn stack_color_selection(effect: Operation, texel: LayerTexel) -> vec2<f32> { return vec2(0.0, 1.0); }";

/// `Operation::ext[2]` flag of a mask comparison or colour selection with a soft edge, an output
/// or a background (`composite_mask_effects.wgsl` then calls this package's functions).
pub(super) const SOFT_MASK_EFFECT: u32 = 0x0000_0100;
/// `Operation::ext[2]` flag of a colour selection reading an image as its ID map, whose record
/// is the first word of its side-table parameters (`ProceduralInputs::pack_image`).
const ID_MAP_IMAGE: u32 = 0x0000_0200;
/// The background value of a transparent colour selection, which covers only what it selects.
const TRANSPARENT_BACKGROUND: f32 = -1.0;
/// P3 opcode: a mask comparison with a side reading the layers below or this layer.
const LAYER_COMPARE: u32 = 144;
/// Width of a side code in `ids[3]` (the left side, then the right side above it).
const SIDE_BITS: u32 = 4;
/// The bits of one side code.
const SIDE_MASK: u32 = (1 << SIDE_BITS) - 1;
/// Side codes of a layer comparison (`stack_compare_side` in WGSL).
const MASK_SIDE: u32 = 0;
const CONSTANT_SIDE: u32 = 1;
const LAYERS_BELOW_SIDE: u32 = 2;
const THIS_LAYER_SIDE: u32 = 3;
/// Shift of the compared channel's kind in `ids[3]`.
const KIND_SHIFT: u32 = 8;
/// Kind codes of the compared channel.
const COLOR_KIND: u32 = 0;
const SCALAR_KIND: u32 = 1;
const SIGNED_KIND: u32 = 2;
const NORMAL_MAP_KIND: u32 = 3;

/// Lets a mask comparison ramp over `1 - hardness` past its edge (`CompareOp::mask`), the width
/// in `low.z`. A hard comparison, and a NaN hardness the CPU also treats as hard, stays as it is.
pub(super) fn soften_compare(op: &mut Operation, hardness: f32) {
    let width = (1.0 - hardness).clamp(0.0, 1.0);
    if width > 0.0 {
        op.low[2] = width;
        op.ext[2] |= SOFT_MASK_EFFECT;
    }
}

/// Lets a colour selection give its output with an edge ramping over `1 - hardness` past its
/// tolerance, on its background (`ColorSelection::value`), and read an image as its ID map
/// (`id_color`, the image's stored texel at the texel's place): the image record, output, width
/// and background go in the side table. A selection with full output, a hard edge and a black
/// background on the baked ID map stays as it is. None for a non-finite output or hardness (the
/// CPU selects nothing at a NaN hardness).
pub(super) fn soften_selection(op: &mut Operation, selection: &ColorSelection, inputs: &mut ProceduralInputs) -> Option<()> {
    let ColorSelection { output, hardness, background, id_map, .. } = selection;
    if !output.is_finite() || !hardness.is_finite() {
        return None;
    }

    let width = 1.0 - hardness.clamp(0.0, 1.0);
    let background_value = background.value();
    if *output == 1.0 && width <= 0.0 && background_value == Some(0.0) && id_map.is_none() {
        return Some(());
    }

    // The record word comes first, the parameters right after it
    let record = match id_map {
        Some(name) => {
            op.ext[2] |= ID_MAP_IMAGE;
            inputs.pack_image(name, ImageUse::Material)?
        }
        None => inputs.pack_words(&[0])?,
    };
    inputs.pack_params(&[*output, width, background_value.unwrap_or(TRANSPARENT_BACKGROUND)])?;
    op.ext[0] = record;
    op.ext[2] |= SOFT_MASK_EFFECT;
    Some(())
}

/// Turns a mask comparison into one reading the material of `channel` at the texel for its
/// layer sides (`compare_side`): the layers below (`stack_at`) or this layer's own content over
/// them (`this_layer_content`), through the program's stack read (`spatial[3]`). The sides and
/// the channel's kind go in `ids[3]`.
pub(super) fn read_layers(op: &mut Operation, left: CompareSource, right: CompareSource, channel: Channel) {
    let side = |source: CompareSource| match source {
        CompareSource::ThisMask => MASK_SIDE,
        CompareSource::Constant => CONSTANT_SIDE,
        CompareSource::LayersBelow => LAYERS_BELOW_SIDE,
        CompareSource::ThisLayer => THIS_LAYER_SIDE,
    };
    let kind = match channel.kind() {
        ChannelKind::Color => COLOR_KIND,
        ChannelKind::Scalar => SCALAR_KIND,
        ChannelKind::Signed => SIGNED_KIND,
        ChannelKind::NormalMap => NORMAL_MAP_KIND,
    };
    op.ids[1] = LAYER_COMPARE;
    op.ids[3] = side(left) | (side(right) << SIDE_BITS) | (kind << KIND_SHIFT);
    op.spatial[3] = channel_slot(channel) + 1;
    op.effects[1] = 0;
}

/// Whether a mask operation compares this layer's own content.
pub(super) fn reads_own_layer(op: &Operation) -> bool {
    let reads = |side: u32| side & SIDE_MASK == THIS_LAYER_SIDE;
    op.ids[1] == LAYER_COMPARE && (reads(op.ids[3]) || reads(op.ids[3] >> SIDE_BITS))
}

/// Whether a mask's operations compare this layer's content before a filter stage of the mask:
/// the prepare pass builds the mask's fields before the layer's content fields, so the content
/// that comparison reads would lack its filters (the hub orders them later).
pub(super) fn reads_own_layer_before_stage(mask: &[Operation]) -> bool {
    let Some(first) = mask.iter().position(reads_own_layer) else { return false };
    mask[first + 1..]
        .iter()
        .any(|op| op.ids[1] == NEIGHBOURHOOD_OPCODE || ((FIRST_GENERATOR..FIRST_EXTENDED_PATTERN).contains(&op.ids[1]) && op.spatial[1] != 0))
}

/// A folder mask's comparison: the CPU's This Layer of a folder is the stack below the folder
/// (a folder has no content of its own), so its sides read Layers Below.
pub(super) fn read_folder_layers(op: &mut Operation) {
    if op.ids[1] != LAYER_COMPARE {
        return;
    }

    for shift in [0, SIDE_BITS] {
        if (op.ids[3] >> shift) & SIDE_MASK == THIS_LAYER_SIDE {
            op.ids[3] = (op.ids[3] & !(SIDE_MASK << shift)) | (LAYERS_BELOW_SIDE << shift);
        }
    }
}

/// Gives a Bevel Smooth or Directional Distance stage its Distance Map (set, or its default
/// gray), which the stage evaluates into its field's input word. Other operations stay as they
/// are. None leaves the effect on the CPU.
pub(super) fn attach_distance_map(effect: &Effect, op: &mut Operation, inputs: &mut ProceduralInputs) -> Option<()> {
    let EffectKind::Filter(Filter::BevelSmooth(_) | Filter::DirectionalDistance(_)) = &effect.kind else { return Some(()) };
    if op.ids[1] != NEIGHBOURHOOD_OPCODE {
        return Some(());
    }

    op.effects[3] = register_image_input(effect, &ImageInput::DISTANCE_MAP, ChannelKind::Scalar, inputs)?;
    Some(())
}

/// Gives the Tile Generator sources among `sources` (the inputs `effect` just registered) their
/// tile settings, from the input of the effect's filter that reads a Tile Generator.
pub(super) fn bind_input_tiles(effect: &Effect, sources: &mut [Operation], patterns: &mut TilePatterns) {
    let EffectKind::Filter(filter) = &effect.kind else { return };
    let Some(source) = ImageInput::of(filter).iter().map(|input| effect.input(input)).find(|source| matches!(source, Source::Procedural { .. })) else {
        return;
    };

    for op in sources.iter_mut() {
        bind_tile_source(op, Some(&source), patterns);
    }
}

/// Register `effect`'s image input `input` (its source, else its default gray) as the source the
/// effect operation reads at the texel, read as a `kind` channel: the reference for the
/// operation's `effects[3]`. The CPU reads inputs with `Sampling::default()` and no projection,
/// so an image source waits for the sampling P1 adds (`source_operation` refuses it); a Tile
/// Generator gets its settings from [`bind_input_tiles`]. None leaves the effect on the CPU.
pub(super) fn register_image_input(effect: &Effect, input: &ImageInput, kind: ChannelKind, inputs: &mut ProceduralInputs) -> Option<u32> {
    let source = effect.input(input);
    let operation = source_operation(Some(&source), &Projection::default(), Sampling::default(), [0.0; 3], kind, inputs)?;
    Some(inputs.register_input(operation))
}

/// Whether the GPU evaluates the image inputs of `filter` (`ProceduralInputs::register_input`):
/// Grayscale Conversion's Custom Input (lowered in gpu_composite_point.rs) and the Distance Map
/// of Bevel Smooth and Directional Distance ([`attach_distance_map`], read by P4's kernels).
pub(super) fn evaluates_inputs(filter: &Filter) -> bool {
    matches!(
        filter,
        Filter::GrayscaleConversion(_)
            | Filter::BevelSmooth(_)
            | Filter::DirectionalDistance(_)
            | Filter::SourceWarp(_)
            | Filter::SourceSlopeBlur(_)
            | Filter::AnisotropicKuwahara(_)
            | Filter::ColorMatch(_)
    )
}

/// Gives a P5 gather stage (Warp and Blur Slope's Custom Noise, Kuwahara's Custom Input, Color
/// Match's Input or Image Input) the input it reads at its texels, and its filter the inputs it
/// does not (`gather::bind_inputs`). Other operations stay as they are. None leaves the effect on
/// the CPU.
pub(super) fn attach_gather_input(effect: &Effect, op: &mut Operation, inputs: &mut ProceduralInputs) -> Option<()> {
    let EffectKind::Filter(filter @ (Filter::SourceWarp(_) | Filter::SourceSlopeBlur(_) | Filter::AnisotropicKuwahara(_) | Filter::ColorMatch(_))) =
        &effect.kind
    else {
        return Some(());
    };
    if op.ids[1] != NEIGHBOURHOOD_OPCODE {
        return Some(());
    }

    let (bound, input) = crate::gpu_neighbourhood::gather::bind_inputs(effect, filter)?;
    inputs.filters.get_mut(op.ids[2] as usize)?.0 = bound;
    if let Some((input, kind)) = input {
        let source = effect.input(&input);
        let operation = source_operation(Some(&source), &Projection::default(), Sampling::default(), [0.0; 3], kind, inputs)?;
        op.effects[3] = if kind == ChannelKind::Color { inputs.register_color_input(operation) } else { inputs.register_input(operation) };
    }
    Some(())
}

/// The folder offsets of every draw of layer `target` in `stack`, walked as the material program
/// walks it: visible layers, instances through their sources, folders' children under
/// `around.then(contents)`.
pub(super) fn draw_offsets(project: &Project, stack: &[LayerId], target: LayerId) -> Vec<ContentsOffset> {
    let mut offsets = Vec::new();
    let mut todo: Vec<_> = stack.iter().rev().map(|&id| (id, 0, ContentsOffset::NONE)).collect();
    while let Some((id, depth, around)) = todo.pop() {
        if !project.layers.get(&id).is_some_and(|layer| layer.visible) {
            continue;
        }

        if id == target {
            offsets.push(around);
        }
        if let Some(LayerKind::Group { children, contents, .. }) = project.content_layer(id).map(|layer| &layer.kind)
            && depth <= MAX_DEPTH
        {
            todo.extend(children.iter().rev().map(|&child| (child, depth + 1, around.then(*contents))));
        }
    }
    offsets
}

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

    use texturecraft_compose::{BakedMaps, ImageRes, ImageStore, PixelStore};
    use texturecraft_core::{
        BakedMap, BlendMode, Channel, ChannelBlend, ColorSelection, CompareOp, CompareSource, ContentsOffset, Effect, EffectKind, Fill, Filter, Layer, LayerId,
        LayerKind, Levels, Mask, MaskBackground, NormalFormat, Project, Projection, Sampling, SelectionBackground, Source, TextureSet,
    };
    use texturecraft_geom::primitives;
    use texturecraft_texel::{TILE, to_u16, to_u16_signed};

    use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu, compile_effects, effect, headless};
    use crate::gpu_composite::{CompositeProgram, CompositeRegion};

    /// P3's effect settings that still composite on the CPU. Port one: delete its row or block
    /// (these tests fail while a ported case stays) and add its oracle test.
    #[test]
    fn stack_settings_still_on_the_cpu_fall_back() {
        let none = ContentsOffset::NONE;

        // An image as Custom Input waits for the Bilinear HQ sampling of P1
        let conversion = texturecraft_core::GrayscaleConversion { custom_input: true, ..texturecraft_core::GrayscaleConversion::LIBRARY };
        let mut image_input = effect(EffectKind::Filter(Filter::GrayscaleConversion(conversion)));
        let image = Source::Image { name: "Pattern".into(), normal_format: NormalFormat::default() };
        image_input.inputs.insert(texturecraft_core::ImageInput::CUSTOM_INPUT.id.into(), image);
        assert!(compile_effects(Some(image_input.clone()), None, none).0.is_err());
        let (material, mask) = compile_effects(None, Some(image_input), none);
        assert!(material.is_err() && mask.is_err());

        // A mask filter stage after This Layer: the prepare pass builds the mask's fields before the
        // layer's content fields, so the filtered content is not there yet (the hub orders them later)
        let (mut project, pixels, owner) = compare_stack(vec![own_layer_compare(), effect(EffectKind::Filter(Filter::Blur { radius: 24.0, wrap: false }))]);
        project.layer_mut(owner).unwrap().content_effects.push(effect(EffectKind::Filter(Filter::Blur { radius: 12.0, wrap: false })));
        assert!(CompositeProgram::try_compile_region(&project, &pixels, 0, None).is_err());
        assert!(CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, owner, None, None).is_err());
    }

    /// A fill whose mask reads an anchor published by the layer below and an image fill, inside a
    /// folder with Scale and Rotation: (project, owner, folder).
    #[allow(clippy::unwrap_used)] // A test helper: a project that does not build fails the test
    fn anchored_mask_in_offset_folder() -> (Project, LayerId, LayerId) {
        let channels = [Channel::BaseColor, Channel::Height];
        let mut project = Project::new("Anchored mask", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, TILE * 2));
        let publisher = project.alloc_id();
        let fill = Fill { sources: BTreeMap::from([(Channel::Height, Source::Uniform([0.4; 3]))]), ..Default::default() };
        let mut below = Layer::new(publisher, "Below", LayerKind::Fill(fill), &channels);
        let anchor = Effect::new(project.alloc_id(), EffectKind::Anchor);
        let anchor_id = anchor.id;
        below.content_effects.push(anchor);
        project.insert_layer(0, below, None, false).unwrap();
        let folder_id = project.alloc_id();
        let mut group = Layer::new(folder_id, "Folder", LayerKind::folder(), &channels);
        if let LayerKind::Group { contents, .. } = &mut group.kind {
            *contents = ContentsOffset::new(2.0, 30.0);
        }
        project.insert_layer(0, group, None, false).unwrap();
        let owner = project.alloc_id();
        let fill = Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.6, 0.3, 0.2]))]), ..Default::default() };
        let mut layer = Layer::new(owner, "Surface", LayerKind::Fill(fill), &channels);
        let mut mask = Mask::new(MaskBackground::Black);
        let anchored = Source::Anchor { anchor: anchor_id, channel: Channel::Height, ramp: Vec::new() };
        mask.effects.push(effect(EffectKind::Fill { source: anchored, projection: Projection::default(), sampling: Sampling::SAVED_BEFORE }));
        let image = Source::Image { name: "Pattern".into(), normal_format: NormalFormat::default() };
        let projection = Projection::Uv { scale: [2.0, 1.0], rotation: 15.0, offset: [0.1, 0.0] };
        let mut placed = effect(EffectKind::Fill { source: image, projection, sampling: Sampling::SAVED_BEFORE });
        placed.blend = BlendMode::Multiply;
        mask.effects.push(placed);
        layer.mask = Some(mask);
        project.insert_layer(0, layer, Some(folder_id), true).unwrap();
        (project, owner, folder_id)
    }

    /// A 4×4 linear image with distinct texels, for fills whose placement must show.
    fn pattern_images() -> ImageStore {
        let texels = (0..16).map(|index| [0.1 + 0.05 * index as f32, 0.9 - 0.04 * index as f32, (index % 5) as f32 * 0.2, 1.0]).collect();
        ImageStore::from([("Pattern".into(), Arc::new(ImageRes::new(4, 4, texels, false)))])
    }

    #[test]
    fn folder_offsets_still_on_the_cpu_fall_back() {
        // A mask reading an anchor previews through the material program, which keeps the owner's
        // last draw: with the owner also drawn by an instanced folder under another offset, that
        // draw need not sit where the requested layer does
        let (mut project, owner, folder_id) = anchored_mask_in_offset_folder();
        let pixels = PixelStore::default();
        assert!(CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, owner, None, None).is_ok(), "drawn once");

        let channels = [Channel::BaseColor, Channel::Height];
        let around = project.alloc_id();
        let mut group = Layer::new(around, "Around", LayerKind::folder(), &channels);
        if let LayerKind::Group { contents, .. } = &mut group.kind {
            *contents = ContentsOffset::new(0.5, 45.0);
        }
        project.insert_layer(0, group, None, false).unwrap();
        let instanced = project.alloc_id();
        let instance = Layer::new(instanced, "Instanced folder", LayerKind::Instance { source: folder_id }, &channels);
        project.insert_layer(0, instance, Some(around), true).unwrap();
        assert!(CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, owner, None, None).is_err(), "drawn under two offsets");
    }

    /// An anchored mask inside a folder with Scale and Rotation, drawn once, previews on the GPU
    /// through the material program as the CPU's `mask_image` places it.
    #[test]
    fn an_anchored_mask_inside_an_offset_folder_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let (baked, images, pixels) = (BakedMaps::default(), pattern_images(), PixelStore::default());
        let (project, owner, _) = anchored_mask_in_offset_folder();

        let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
        case.mask_owner = Some(owner);
        assert_matches_cpu(&mut gpu, &case, Tolerance::BILINEAR, "anchored mask in an offset folder");
    }

    /// A paint layer with colour, roughness and height painted in a block (so a filter's halo
    /// shows), `content` on its content and `mask` after a gray fill in its black mask.
    #[allow(clippy::unwrap_used)] // A test helper: a project that does not build fails the test
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
                pixels.layer_mut(owner, Channel::BaseColor, (SIZE, SIZE)).set(
                    x,
                    y,
                    &[to_u16(0.7 * gain), to_u16(0.2 * gain), to_u16(0.3 * gain), to_u16(gain)],
                );
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

        // Set to Disable as a whole yet on for Height: the content filters Height alone, the mask filters anyway
        let mut disabled_but_height = with(invert(), 0.8, &[(Channel::Height, BlendMode::Normal, 1.0)], &[]);
        disabled_but_height.blend = BlendMode::Disable;

        let cases: Vec<(&str, Vec<Effect>, Tolerance)> = vec![
            ("per-channel multiply", vec![with(invert(), 0.77, &[(Channel::Height, BlendMode::Multiply, 0.5)], &[])], Tolerance::POINT),
            ("switched off", vec![with(invert(), 0.77, &[], &[Channel::Roughness])], Tolerance::POINT),
            ("disabled on colour", vec![with(invert(), 1.0, &[(Channel::BaseColor, BlendMode::Disable, 1.0)], &[])], Tolerance::POINT),
            ("disabled but on height", vec![disabled_but_height], Tolerance::POINT),
            ("levels on height", vec![with(EffectKind::Levels(levels(Some(Channel::Height))), 0.6, &[], &[])], Tolerance::POINT),
            ("levels on colour", vec![with(EffectKind::Levels(levels(Some(Channel::BaseColor))), 1.0, &[], &[])], Tolerance::POINT),
            (
                "library levels",
                vec![with(EffectKind::Levels(Levels { affects: Some(Channel::BaseColor), ..Levels::default() }), 1.0, &[], &[])],
                Tolerance::POINT,
            ),
            // Levels does not blend, so Disable leaves it on
            (
                "levels disabled",
                vec![with(EffectKind::Levels(levels(Some(Channel::Roughness))), 0.7, &[(Channel::Roughness, BlendMode::Disable, 0.7)], &[])],
                Tolerance::POINT,
            ),
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

    /// A lowering with a stage before its operation (the Gradient's input blur seam) laid on each
    /// channel apart: a channel set to Disable keeps a NOOP in the stage's place, so every channel
    /// has the same operations and the content cut of the anchor above holds for the layer that
    /// reads it. The staged program equals the one of a Blur and an Invert with the same per-channel
    /// settings, which matches the CPU in colour, the other channels and the mask.
    #[test]
    fn a_staged_lowering_keeps_every_channel_aligned_for_the_anchor_above() {
        let build = |staged: bool| {
            let per_channel = |kind: EffectKind, opacity: f32| {
                let mut effect = effect(kind);
                effect.opacity = opacity;
                effect.channel_blends.insert(Channel::Height, ChannelBlend { mode: BlendMode::Disable, opacity: 1.0 });
                effect
            };
            let mut content = if staged {
                vec![per_channel(EffectKind::Filter(Filter::Smoothstep { min: 0.0, max: 1.0 }), 0.77)]
            } else {
                vec![per_channel(EffectKind::Filter(Filter::Blur { radius: 24.0, wrap: false }), 1.0), per_channel(EffectKind::Filter(Filter::Invert), 0.77)]
            };
            content.push(effect(EffectKind::Levels(Levels { gamma: 1.4, affects: Some(Channel::Roughness), ..Levels::default() })));
            let anchor = Effect::new(LayerId(950), EffectKind::Anchor);
            content.push(anchor);
            content.push(effect(EffectKind::Filter(Filter::Invert)));
            let (mut project, pixels, owner) = painted_layer(content, Vec::new());

            // A fill above reads the anchor's roughness, cut after the effects below it
            let reader = project.alloc_id();
            let source = Source::Anchor { anchor: LayerId(950), channel: Channel::Roughness, ramp: Vec::new() };
            let fill = Fill { sources: BTreeMap::from([(Channel::Roughness, source)]), ..Default::default() };
            project.insert_layer(0, Layer::new(reader, "Reader", LayerKind::Fill(fill), &[Channel::Roughness]), None, false).unwrap();
            (project, pixels, owner)
        };
        let compile = |project: &Project, pixels: &PixelStore| {
            let program = CompositeProgram::try_compile_region(project, pixels, 0, None).unwrap();
            let counts: Vec<u32> = program.operations[..program.layer_count as usize].iter().filter(|op| op.ids[1] == 10).map(|op| op.effects[3]).collect();
            (bytemuck::cast_slice::<_, u8>(&program.operations).to_vec(), program.filters.stages.len(), counts)
        };

        let (staged_project, staged_pixels, _) = build(true);
        super::super::packages::STAGED_TEST_LOWERING.set(true);
        let staged = compile(&staged_project, &staged_pixels);
        super::super::packages::STAGED_TEST_LOWERING.set(false);
        let (project, pixels, owner) = build(false);
        let separate = compile(&project, &pixels);

        assert_eq!(staged.2.len(), 3, "a paint operation per channel");
        assert!(staged.2.iter().all(|&count| count == staged.2[0]), "every channel has the same operations: {:?}", staged.2);
        assert_eq!(staged, separate);

        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let (baked, images) = (BakedMaps::default(), ImageStore::default());
        let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
        case.mask_owner = Some(owner);
        case.region = Some(CompositeRegion { min: [61, 57], max: [75, 73] });
        assert_matches_cpu(&mut gpu, &case, Tolerance::NEIGHBOURHOOD, "staged lowering per channel");
    }

    /// A fill layer whose mask is painted as a ramp across the set (0 at the left, 1 at the
    /// right, with a step down the middle row) followed by `effects`, as (project, pixels, owner).
    #[allow(clippy::unwrap_used)] // A test helper: a project that does not build fails the test
    fn ramp_mask_layer(effects: Vec<Effect>) -> (Project, PixelStore, LayerId) {
        const SIZE: u32 = TILE * 2;
        let mut project = Project::new("Mask effects", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
        let owner = project.alloc_id();
        let fill = Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.89, 0.19, 0.04]))]), ..Default::default() };
        let mut layer = Layer::new(owner, "Surface", LayerKind::Fill(fill), &[Channel::BaseColor]);
        let mut mask = Mask::new(MaskBackground::Black);
        let paint = project.alloc_id();
        mask.effects.push(Effect::new(paint, EffectKind::Paint));
        mask.effects.extend(effects);
        layer.mask = Some(mask);
        project.insert_layer(0, layer, None, false).unwrap();

        let mut pixels = PixelStore::default();
        let ramp = pixels.mask_mut(paint, (SIZE, SIZE));
        for y in 0..SIZE {
            for x in 0..SIZE {
                let step = if y >= SIZE / 2 { 0.13 } else { 0.0 };
                ramp.set(x, y, &[to_u16((x as f32 + 0.5) / SIZE as f32 - step), u16::MAX]);
            }
        }
        (project, pixels, owner)
    }

    /// Mask comparisons with a soft edge ramp as the CPU's `CompareOp::mask` does, for every
    /// comparison, at opacity below 1, with a non-Normal blend and set to Disable, chained.
    #[test]
    fn soft_mask_comparisons_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let (baked, images) = (BakedMaps::default(), ImageStore::default());
        let compare = |op, hardness, blend, opacity| {
            let mut effect = effect(EffectKind::Compare {
                op,
                value: 0.45,
                tolerance: 0.12,
                left: CompareSource::ThisMask,
                right: CompareSource::Constant,
                channel: Channel::Height,
                hardness,
            });
            effect.blend = blend;
            effect.opacity = opacity;
            effect
        };
        let invert = effect(EffectKind::Filter(Filter::Invert));

        let mut cases = Vec::new();
        for op in [CompareOp::LesserThan, CompareOp::WithinTolerance, CompareOp::GreaterThan] {
            for hardness in [0.6, 0.0, -2.0] {
                cases.push(vec![compare(op, hardness, BlendMode::Normal, 1.0)]);
            }
            cases.push(vec![compare(op, 0.7, BlendMode::Multiply, 0.8)]);
            cases.push(vec![compare(op, 0.7, BlendMode::Disable, 0.8)]);
            cases.push(vec![invert.clone(), compare(op, 0.5, BlendMode::Screen, 0.9), invert.clone()]);
        }
        for effects in cases {
            let label = format!("{effects:?}");
            let (project, pixels, owner) = ramp_mask_layer(effects);
            let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
            case.mask_owner = Some(owner);
            assert_matches_cpu(&mut gpu, &case, Tolerance::POINT, &label);
        }
    }

    /// Colour selections with an output, a soft edge and each background match the CPU's
    /// `ColorSelection::value` (a transparent one covers only what it selects), at opacity below
    /// 1, with a non-Normal blend, chained, and with no colours picked.
    #[test]
    fn soft_colour_selections_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let images = ImageStore::default();

        // ID colours on, at and just past the tolerance of the picked ones, and far from them; the
        // hard selections read IDs clear of the tolerance, where one ulp cannot flip a whole cell
        let picked = vec![[0.40101, 0.69217, 0.05289], [0.375; 3], [0.21875, 0.78125, 0.09375]];
        let ids = |boundary: bool| {
            let [at, past, below] =
                if boundary { [[0.390625; 3], [0.390626, 0.375, 0.375], [0.359375; 3]] } else { [[0.38; 3], [0.42, 0.375, 0.375], [0.37; 3]] };
            let colors =
                [[0.40101, 0.69217, 0.05289], at, past, [0.21875, 0.78125, 0.09375], [0.1, 0.9, 0.2], [0.8, 0.1, 0.9], below, [0.0; 3], [0.3, 0.6, 0.1]];
            BakedMaps { resolution: 3, maps: BTreeMap::from([(BakedMap::Id, Arc::new(colors.into_iter().flatten().collect()))]) }
        };
        let (boundary, clear) = (ids(true), ids(false));
        let selection = |colors: Vec<[f32; 3]>, output, hardness, background, blend, opacity| {
            let mut selection = ColorSelection::new(colors, 1.0 / 64.0);
            selection.output = output;
            selection.hardness = hardness;
            selection.background = background;
            let mut effect = effect(EffectKind::ColorSelection(selection));
            effect.blend = blend;
            effect.opacity = opacity;
            effect
        };
        let invert = effect(EffectKind::Filter(Filter::Invert));

        let mut cases = Vec::new();
        for background in [SelectionBackground::Black, SelectionBackground::White, SelectionBackground::Gray, SelectionBackground::Transparent] {
            cases.push(vec![selection(picked.clone(), 1.0, 1.0, background, BlendMode::Normal, 1.0)]);
            cases.push(vec![selection(picked.clone(), 0.6, 0.5, background, BlendMode::Normal, 1.0)]);
            cases.push(vec![selection(picked.clone(), 0.8, 0.2, background, BlendMode::Multiply, 0.75)]);
            cases.push(vec![selection(Vec::new(), 0.7, 0.5, background, BlendMode::Normal, 1.0)]);
        }
        cases.push(vec![selection(picked.clone(), 0.6, 0.5, SelectionBackground::Gray, BlendMode::Disable, 0.9)]);
        cases.push(vec![invert.clone(), selection(picked.clone(), 0.4, 0.3, SelectionBackground::Transparent, BlendMode::Screen, 0.9), invert]);
        for effects in cases {
            let label = format!("{effects:?}");
            let hard = effects.iter().any(|effect| matches!(&effect.kind, EffectKind::ColorSelection(selection) if selection.hardness >= 1.0));
            let baked = if hard { &clear } else { &boundary };
            let (project, pixels, owner) = ramp_mask_layer(effects);
            let mut case = OracleCase::new(&project, &pixels, baked, &images, &mesh);
            case.mask_owner = Some(owner);
            assert_matches_cpu(&mut gpu, &case, Tolerance::POINT, &label);
        }
    }

    /// Fills inside folders with Scale and Rotation match the CPU: nested folders, a folder with
    /// a mask of its own (drawn as a group), a material deposit, a blur on the content and in the
    /// mask of a layer drawn under several offsets (an instance in another such folder and an
    /// instanced folder), in the material with a partial update and in both mask previews.
    #[test]
    fn fills_inside_folders_with_scale_and_rotation_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let baked = BakedMaps::default();
        let channels = [Channel::BaseColor, Channel::Roughness];
        let images = pattern_images();
        let image = || Source::Image { name: "Pattern".into(), normal_format: NormalFormat::default() };
        let own = Projection::Uv { scale: [2.0, 1.0], rotation: 15.0, offset: [0.1, 0.0] };
        let folder = |project: &mut Project, offset: ContentsOffset, parent: Option<LayerId>| {
            let id = project.alloc_id();
            let mut group = Layer::new(id, "Folder", LayerKind::folder(), &channels);
            if let LayerKind::Group { contents, .. } = &mut group.kind {
                *contents = offset;
            }
            project.insert_layer(0, group, parent, parent.is_some()).unwrap();
            id
        };

        let mut project = Project::new("Folder offsets", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, TILE * 2));
        let outer = folder(&mut project, ContentsOffset::new(2.0, 30.0), None);
        let inner = folder(&mut project, ContentsOffset::new(1.5, -10.0), Some(outer));
        let owner = project.alloc_id();
        let sources = BTreeMap::from([(Channel::BaseColor, image()), (Channel::Roughness, image())]);
        let fill = Fill { sources, projection: own.clone(), sampling: Sampling::SAVED_BEFORE, ..Default::default() };
        let mut layer = Layer::new(owner, "Surface", LayerKind::Fill(fill), &channels);
        let deposit = Fill { sources: BTreeMap::from([(Channel::BaseColor, image())]), sampling: Sampling::SAVED_BEFORE, ..Default::default() };
        let mut deposit = effect(EffectKind::Material(deposit));
        deposit.opacity = 0.6;
        deposit.blend = BlendMode::Multiply;
        layer.content_effects.push(deposit);

        // A blur on the content and in the mask: each draw of the owner keeps its own fields
        let blur = || effect(EffectKind::Filter(Filter::Blur { radius: 24.0, wrap: false }));
        layer.content_effects.push(blur());
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(effect(EffectKind::Fill { source: image(), projection: own.clone(), sampling: Sampling::SAVED_BEFORE }));
        mask.effects.push(blur());
        layer.mask = Some(mask);
        project.insert_layer(0, layer, Some(inner), true).unwrap();

        // A masked folder at part opacity draws as a group, its mask under the folders around it
        let masked = folder(&mut project, ContentsOffset::new(0.5, 45.0), Some(outer));
        let masked_layer = project.layer_mut(masked).unwrap();
        masked_layer.channels.insert(Channel::BaseColor, ChannelBlend { mode: BlendMode::Normal, opacity: 0.8 });
        let mut group_mask = Mask::new(MaskBackground::Black);
        group_mask.effects.push(effect(EffectKind::Fill { source: image(), projection: own, sampling: Sampling::SAVED_BEFORE }));
        masked_layer.mask = Some(group_mask);
        let instance = project.alloc_id();
        project.insert_layer(0, Layer::new(instance, "Instance", LayerKind::Instance { source: owner }, &channels), Some(masked), true).unwrap();

        // The inner folder instanced at the top, inside a folder of its own
        let around = folder(&mut project, ContentsOffset::new(3.0, -60.0), None);
        let instanced = project.alloc_id();
        project.insert_layer(0, Layer::new(instanced, "Instanced folder", LayerKind::Instance { source: inner }, &channels), Some(around), true).unwrap();

        let pixels = PixelStore::default();
        for mask_owner in [owner, instance] {
            let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
            case.mask_owner = Some(mask_owner);
            case.region = Some(CompositeRegion { min: [61, 57], max: [75, 73] });
            assert_matches_cpu(&mut gpu, &case, Tolerance::NEIGHBOURHOOD, &format!("folder offsets, mask of {mask_owner:?}"));
        }
    }

    /// How a layer-comparison project is laid out: whether a layer lies below, how many folders
    /// hold the compared layer, whether the innermost folder's mask (not the layer's) holds the
    /// effects, the compared layer's content effects, and whether the layer below publishes an
    /// anchor ([`BELOW_ANCHOR`]).
    type CompareLayout = (bool, usize, bool, Vec<Effect>, bool);

    /// The anchor the layer below publishes when the layout asks for it.
    const BELOW_ANCHOR: LayerId = LayerId(960);

    /// [`compare_stack_in`] with a layer below and the effects in the compared layer's mask.
    fn compare_stack(effects: Vec<Effect>) -> (Project, PixelStore, LayerId) {
        compare_stack_in(effects, (true, 0, false, Vec::new(), false))
    }

    /// A layer below with colour, roughness, height and normal ramping across the set under a
    /// blur, and above it a paint layer painted in a block (inside `folders` folders, the inner
    /// one at part opacity), with a black mask holding a gray fill and `effects` on the layer or on
    /// the innermost folder: (project, pixels, the mask's owner).
    #[allow(clippy::unwrap_used)] // A test helper: a project that does not build fails the test
    fn compare_stack_in(effects: Vec<Effect>, (below, folders, folder_mask, content, anchor): CompareLayout) -> (Project, PixelStore, LayerId) {
        const SIZE: u32 = TILE * 2;
        let channels = [Channel::BaseColor, Channel::Roughness, Channel::Height, Channel::Normal];
        let mut project = Project::new("Layer comparisons", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
        let below_id = project.alloc_id();
        if below {
            let mut layer = Layer::new(below_id, "Below", LayerKind::Paint, &channels);
            layer.content_effects.push(effect(EffectKind::Filter(Filter::Blur { radius: 24.0, wrap: false })));
            if anchor {
                layer.content_effects.push(Effect::new(BELOW_ANCHOR, EffectKind::Anchor));
            }
            project.insert_layer(0, layer, None, false).unwrap();
        }
        let mut mask = Mask::new(MaskBackground::Black);
        let fill = EffectKind::Fill { source: Source::Uniform([0.36; 3]), projection: Projection::default(), sampling: Sampling::SAVED_BEFORE };
        mask.effects.push(Effect::new(project.alloc_id(), fill));
        mask.effects.extend(effects);

        // The folders, the innermost at part opacity
        let mut parent = None;
        for depth in 0..folders {
            let folder = project.alloc_id();
            let mut group = Layer::new(folder, "Folder", LayerKind::folder(), &channels);
            if depth + 1 == folders {
                group.channels.insert(Channel::BaseColor, ChannelBlend { mode: BlendMode::Normal, opacity: 0.8 });
                if folder_mask {
                    group.mask = Some(mask.clone());
                }
            }
            project.insert_layer(0, group, parent, parent.is_some()).unwrap();
            parent = Some(folder);
        }
        let owner = project.alloc_id();
        let mut layer = Layer::new(owner, "Compared", LayerKind::Paint, &channels);
        layer.content_effects = content;
        if !folder_mask {
            layer.mask = Some(mask);
        }
        project.insert_layer(0, layer, parent, parent.is_some()).unwrap();

        let mut pixels = PixelStore::default();
        for y in 0..SIZE {
            for x in 0..SIZE {
                let t = (x as f32 + 0.5) / SIZE as f32;
                if below {
                    pixels.layer_mut(below_id, Channel::BaseColor, (SIZE, SIZE)).set(x, y, &[to_u16(t), to_u16(0.5 * t + 0.2), to_u16(1.0 - t), u16::MAX]);
                    pixels.layer_mut(below_id, Channel::Roughness, (SIZE, SIZE)).set(x, y, &[to_u16(t), u16::MAX]);
                    pixels.layer_mut(below_id, Channel::Height, (SIZE, SIZE)).set(x, y, &[to_u16_signed(1.2 * t - 0.6), u16::MAX]);
                    pixels.layer_mut(below_id, Channel::Normal, (SIZE, SIZE)).set(x, y, &[to_u16(t), to_u16(0.5), to_u16(1.0 - 0.4 * t), u16::MAX]);
                }
            }
        }
        for y in 22..103 {
            for x in 23..98 {
                let gain = if x >= 63 { 1.0 } else { 0.65 };
                pixels.layer_mut(owner, Channel::BaseColor, (SIZE, SIZE)).set(
                    x,
                    y,
                    &[to_u16(0.7 * gain), to_u16(0.2 * gain), to_u16(0.3 * gain), to_u16(gain)],
                );
                pixels.layer_mut(owner, Channel::Roughness, (SIZE, SIZE)).set(x, y, &[to_u16(0.8 * gain), to_u16(gain)]);
                pixels.layer_mut(owner, Channel::Height, (SIZE, SIZE)).set(x, y, &[to_u16_signed(0.5 * gain), to_u16(gain)]);
                pixels.layer_mut(owner, Channel::Normal, (SIZE, SIZE)).set(x, y, &[to_u16(0.3 * gain), to_u16(0.6 * gain), to_u16(0.9 * gain), to_u16(gain)]);
            }
        }
        let mask_owner = if folder_mask { parent.unwrap() } else { owner };
        (project, pixels, mask_owner)
    }

    /// A soft comparison of this layer's roughness with a constant.
    fn own_layer_compare() -> Effect {
        compare_effect(CompareOp::GreaterThan, (CompareSource::ThisLayer, CompareSource::Constant), Channel::Roughness, 0.6, BlendMode::Normal, 1.0)
    }

    /// A comparison with the constant 0.45 and tolerance 0.12.
    fn compare_effect(op: CompareOp, (left, right): (CompareSource, CompareSource), channel: Channel, hardness: f32, blend: BlendMode, opacity: f32) -> Effect {
        let mut effect = effect(EffectKind::Compare { op, value: 0.45, tolerance: 0.12, left, right, channel, hardness });
        effect.blend = blend;
        effect.opacity = opacity;
        effect
    }

    /// Composite each mask stack laid out by `layout` and compare it with the CPU in every map,
    /// a partial region around the blur and the mask preview (a hard comparison may flip where a
    /// side lies within an ulp of the constant).
    fn assert_compare_cases(cases: Vec<Vec<Effect>>, layout: impl Fn() -> CompareLayout) {
        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let (baked, images) = (BakedMaps::default(), ImageStore::default());
        for effects in cases {
            let label = format!("{effects:?}");
            let (project, pixels, mask_owner) = compare_stack_in(effects, layout());
            let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
            case.mask_owner = Some(mask_owner);
            case.region = Some(CompositeRegion { min: [61, 57], max: [75, 73] });
            assert_matches_cpu(&mut gpu, &case, Tolerance::NEIGHBOURHOOD.with_flips(16), &label);
        }
    }

    const SIDES: [(CompareSource, CompareSource); 4] = [
        (CompareSource::LayersBelow, CompareSource::Constant),
        (CompareSource::ThisLayer, CompareSource::Constant),
        (CompareSource::ThisMask, CompareSource::LayersBelow),
        (CompareSource::LayersBelow, CompareSource::ThisLayer),
    ];

    /// Mask comparisons reading the layers below or this layer over them match the CPU's
    /// `compare_side` on a colour, a scalar, a signed and a normal channel, on either side, for
    /// every comparison, soft and hard, at opacity below 1, with a non-Normal blend, set to
    /// Disable, chained, the library's default (This Layer against Layers Below on Height), around
    /// the blur below with a partial update, and in the mask preview through the material program.
    #[test]
    fn mask_comparisons_reading_layers_match_the_cpu() {
        let invert = effect(EffectKind::Filter(Filter::Invert));
        let mut cases = Vec::new();
        for channel in [Channel::BaseColor, Channel::Roughness, Channel::Height, Channel::Normal] {
            for pair in SIDES {
                cases.push(vec![compare_effect(CompareOp::GreaterThan, pair, channel, 0.6, BlendMode::Normal, 1.0)]);
            }
            for op in [CompareOp::LesserThan, CompareOp::WithinTolerance, CompareOp::GreaterThan] {
                cases.push(vec![compare_effect(op, SIDES[1], channel, 1.0, BlendMode::Normal, 1.0)]);
                cases.push(vec![compare_effect(op, SIDES[3], channel, 0.3, BlendMode::Multiply, 0.8)]);
            }
        }
        cases.push(vec![compare_effect(CompareOp::GreaterThan, SIDES[0], Channel::Roughness, 0.5, BlendMode::Disable, 0.7)]);
        cases.push(vec![invert.clone(), compare_effect(CompareOp::WithinTolerance, SIDES[1], Channel::Height, 0.4, BlendMode::Screen, 0.9), invert]);
        let library = (CompareSource::ThisLayer, CompareSource::LayersBelow);
        cases.push(vec![compare_effect(CompareOp::GreaterThan, library, Channel::Height, 0.9, BlendMode::Normal, 1.0)]);
        assert_compare_cases(cases, || (true, 0, false, Vec::new(), false));
    }

    /// Layer comparisons with nothing below (the channel's default), on a layer inside a folder,
    /// and on a layer with content effects (a point filter and a blur, without a mask filter after
    /// the comparison) match the CPU.
    #[test]
    fn mask_comparisons_reading_layers_in_other_stacks_match_the_cpu() {
        let cases = || {
            [Channel::BaseColor, Channel::Height]
                .into_iter()
                .flat_map(|channel| SIDES.map(|pair| vec![compare_effect(CompareOp::GreaterThan, pair, channel, 0.6, BlendMode::Normal, 1.0)]))
                .collect::<Vec<_>>()
        };
        assert_compare_cases(cases(), || (false, 0, false, Vec::new(), false));
        assert_compare_cases(cases(), || (true, 2, false, Vec::new(), false));
        let content = || vec![effect(EffectKind::Filter(Filter::Invert)), effect(EffectKind::Filter(Filter::Blur { radius: 12.0, wrap: false }))];
        assert_compare_cases(cases(), || (true, 0, false, content(), false));
    }

    /// A folder's mask comparing This Layer reads the stack below the folder, as the CPU does (a
    /// folder has no content of its own), and Layers Below likewise: Base Color and Height, a
    /// folder nested in another, chained, around the blur below with a partial update.
    #[test]
    fn folder_mask_comparisons_read_the_stack_below_as_the_cpu_does() {
        let invert = effect(EffectKind::Filter(Filter::Invert));
        let mut cases = Vec::new();
        for channel in [Channel::BaseColor, Channel::Height] {
            for pair in SIDES {
                cases.push(vec![compare_effect(CompareOp::GreaterThan, pair, channel, 0.6, BlendMode::Normal, 1.0)]);
            }
        }
        cases.push(vec![invert.clone(), compare_effect(CompareOp::LesserThan, SIDES[1], Channel::Height, 0.4, BlendMode::Multiply, 0.9), invert]);
        assert_compare_cases(cases.clone(), || (true, 1, true, Vec::new(), false));
        assert_compare_cases(cases, || (true, 2, true, Vec::new(), false));
    }

    /// This Layer read before a mask filter stage on a layer whose content has its own filter
    /// stage: enabled once the hub prepares content stacks before the mask stack that reads them.
    #[test]
    #[ignore = "needs hub ordering"]
    fn a_mask_stage_after_this_layer_reads_the_filtered_content() {
        let blur = |radius| effect(EffectKind::Filter(Filter::Blur { radius, wrap: false }));
        assert_compare_cases(vec![vec![own_layer_compare(), blur(24.0)]], || (true, 0, false, vec![blur(12.0)], false));
    }

    /// Colour selections reading an image as their ID map match the CPU's `id_color` (the image's
    /// stored texel at the texel's place): hard and soft, on each background, at opacity below 1
    /// with a non-Normal blend, set to Disable, chained, from a 3×3 sRGB image, and with the image
    /// missing (nothing selected).
    #[test]
    fn colour_selections_reading_an_id_map_image_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let baked = BakedMaps::default();

        // Byte-exact ID colours: picked ones, ones a little and well inside the tolerance, and far ones
        let byte = |k: u8| f32::from(k) / 255.0;
        let colors = [[200, 40, 10], [10, 180, 90], [60, 60, 220], [201, 41, 11], [12, 178, 92], [100, 100, 100], [0, 0, 0], [255, 255, 255]];
        let texels = |count: usize| (0..count).map(|index| colors[index % colors.len()]).map(|[r, g, b]| [byte(r), byte(g), byte(b), 1.0]).collect();
        let images = ImageStore::from([
            ("Ids".into(), Arc::new(ImageRes::new(4, 4, texels(16), false))),
            ("Ids sRGB".into(), Arc::new(ImageRes::new(3, 3, texels(9), true))),
        ]);
        let picked = vec![[200, 40, 10], [10, 180, 90], [60, 60, 220]].into_iter().map(|[r, g, b]| [byte(r), byte(g), byte(b)]).collect::<Vec<_>>();
        let selection = |name: &str, output, hardness, background, blend, opacity| {
            let mut selection = ColorSelection::new(picked.clone(), 1.0 / 64.0);
            selection.id_map = Some(name.into());
            selection.output = output;
            selection.hardness = hardness;
            selection.background = background;
            let mut effect = effect(EffectKind::ColorSelection(selection));
            effect.blend = blend;
            effect.opacity = opacity;
            effect
        };
        let invert = effect(EffectKind::Filter(Filter::Invert));

        let mut cases = Vec::new();
        for background in [SelectionBackground::Black, SelectionBackground::White, SelectionBackground::Gray, SelectionBackground::Transparent] {
            cases.push(vec![selection("Ids", 1.0, 1.0, background, BlendMode::Normal, 1.0)]);
            cases.push(vec![selection("Ids", 0.6, 0.3, background, BlendMode::Multiply, 0.75)]);
            cases.push(vec![selection("Ids sRGB", 0.7, 0.4, background, BlendMode::Normal, 1.0)]);
            cases.push(vec![selection("Missing", 0.8, 1.0, background, BlendMode::Normal, 1.0)]);
        }
        cases.push(vec![selection("Ids", 0.6, 0.5, SelectionBackground::White, BlendMode::Disable, 0.8)]);
        cases.push(vec![invert.clone(), selection("Ids", 0.4, 0.5, SelectionBackground::Gray, BlendMode::Screen, 0.9), invert]);
        for effects in cases {
            let label = format!("{effects:?}");
            let (project, pixels, owner) = ramp_mask_layer(effects);
            let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
            case.mask_owner = Some(owner);
            assert_matches_cpu(&mut gpu, &case, Tolerance::POINT, &label);
        }
    }

    /// Grayscale Conversion with its Custom Input switched on reads the input's colour (set, or its
    /// default gray) in place of the channel, its alpha weighed as 1, as the CPU's
    /// `GrayscaleConversion::channel_value` does: on colour, the other channels and the mask, for
    /// each mode, at opacity below 1 with a non-Normal blend, set to Disable, switched off on a
    /// channel, chained, and from inputs varying per texel (an anchor and a Tile Generator).
    #[test]
    fn grayscale_conversion_reads_its_custom_input_as_the_cpu_does() {
        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let (baked, images) = (BakedMaps::default(), ImageStore::default());
        let conversion = |mode, input: Option<Source>, blend, opacity, off: &[Channel]| {
            let conversion = texturecraft_core::GrayscaleConversion { custom_input: true, mode, ..texturecraft_core::GrayscaleConversion::LIBRARY };
            let mut effect = effect(EffectKind::Filter(Filter::GrayscaleConversion(conversion)));
            if let Some(source) = input {
                effect.inputs.insert(texturecraft_core::ImageInput::CUSTOM_INPUT.id.into(), source);
            }
            effect.blend = blend;
            effect.opacity = opacity;
            effect.off_channels.extend(off.iter().copied());
            effect
        };
        let contrast = effect(EffectKind::Filter(Filter::ContrastLuminosity { contrast: 0.4, luminosity: 0.05 }));
        let color = || Some(Source::Uniform([0.8, 0.3, 0.1]));
        let anchored = || Some(Source::Anchor { anchor: BELOW_ANCHOR, channel: Channel::BaseColor, ramp: Vec::new() });
        let tiles = || {
            let tile = texturecraft_core::TileGenerator { amount: [6, 6], ..texturecraft_core::TileGenerator::default() };
            let pattern = texturecraft_core::Procedural {
                tile: Some(Box::new(tile)),
                seed: 73,
                ..texturecraft_core::Procedural::new(texturecraft_core::ProceduralKind::TileGenerator)
            };
            Some(Source::Procedural { pattern, low: [0.1, 0.2, 0.3], high: [0.9, 0.7, 0.5], ramp: Vec::new() })
        };
        use texturecraft_core::ConversionMode::*;

        let mut cases = Vec::new();
        for mode in texturecraft_core::ConversionMode::ALL {
            cases.push((vec![conversion(mode, color(), BlendMode::Normal, 1.0, &[])], Tolerance::POINT));
        }
        cases.push((vec![conversion(Desaturation, None, BlendMode::Normal, 1.0, &[])], Tolerance::POINT));
        cases.push((vec![conversion(ChannelsWeights, color(), BlendMode::Multiply, 0.7, &[])], Tolerance::POINT));
        cases.push((vec![conversion(Average, color(), BlendMode::Disable, 0.9, &[])], Tolerance::POINT));
        cases.push((vec![conversion(Max, color(), BlendMode::Normal, 0.8, &[Channel::Roughness])], Tolerance::POINT));
        cases.push((vec![contrast.clone(), conversion(Min, color(), BlendMode::Normal, 0.6, &[]), contrast], Tolerance::POINT));
        cases.push((vec![conversion(Desaturation, anchored(), BlendMode::Normal, 0.9, &[])], Tolerance::NEIGHBOURHOOD));

        // The tiles' hard edges may flip where a texel centre lies within an ulp of one
        cases.push((vec![conversion(Average, tiles(), BlendMode::Normal, 1.0, &[])], Tolerance::POINT.with_flips(16)));
        for (content, tolerance) in cases {
            let label = format!("{content:?}");
            let (project, pixels, owner) = compare_stack_in(content.clone(), (true, 0, false, content, true));
            let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
            case.mask_owner = Some(owner);
            assert_matches_cpu(&mut gpu, &case, tolerance, &label);
        }
    }
}
