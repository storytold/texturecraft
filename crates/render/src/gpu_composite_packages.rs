//! Registration hub of the GPU port packages (texturecraft #713: nothing the viewport composites
//! stays on the CPU). Package 0 created this file and it is frozen: a package edits its own
//! module and WGSL file, never this one. Every hook below already calls each package, so a
//! package's work lands in its own file only.
//!
//! | Package | Rust module | WGSL file | Opcodes | `Operation::ext` | Uniform |
//! |---|---|---|---|---|---|
//! | P1 fill sampling and projection | `gpu_composite_projection.rs` | `composite_projection.wgsl` | none; selectors [`PROJECTION_SELECTORS`] in `procedural[3]` | `ext[1]` (all 32 bits) | `packages[0]` |
//! | P2 point filters and gradients | `gpu_composite_point.rs` | `composite_point.wgsl` | [`POINT_OPCODES`] | `ext[2]` [`POINT_OP_BITS`] | `packages[1]` |
//! | P3 stack plumbing and mask effects | `gpu_composite_stack.rs` | `composite_stack.wgsl` | [`STACK_OPCODES`] | `ext[2]` [`STACK_OP_BITS`] | `packages[2]` |
//! | P4 box and distance neighbourhood | `gpu_neighbourhood_box.rs` | `neighbourhood_box.wgsl` | neighbourhood kinds 16..32 | – | `packages[3]` (compose) |
//! | P5 gather and whole-set neighbourhood | `gpu_neighbourhood_gather.rs` | `neighbourhood_gather.wgsl` | neighbourhood kinds 32..56 | – | `packages[4]` (compose) |
//! | P6 lighting and material-reading | `gpu_composite_lighting.rs` | `composite_lighting.wgsl` | [`LIGHTING_OPCODES`] | `ext[2]` [`LIGHTING_OP_BITS`] | `packages[5]` |
//! | P7 brush and live stroke | `gpu_composite_brush.rs` | `composite_brush.wgsl` | none | `ext[2]` [`BRUSH_OP_BITS`] | `packages[6]` |
//! | P8 UDIM and baked layouts | `gpu_composite_udim.rs` | `composite_udim.wgsl` | none | `ext[3]` | `packages[7]` |
//!
//! Shared layouts defined here once:
//! - Effect opcodes are classified by range (see the comment at `FIRST_GENERATOR`). The packages'
//!   effects live in [`EXTENDED_EFFECTS`]; 160..192 stay spare. Existing opcodes never move.
//! - `Operation::ext[0]` is the offset of the operation's parameters in the side table
//!   (`ProceduralInputs::pack_params`/`pack_words`, read in WGSL with `param(op, i)` and
//!   `param_word(op, i)`). The table is the lookup buffer at binding 2, so it needs no binding of
//!   its own and is bounded by `max_storage_buffer_binding_size` like the colour ramps.
//! - The band uniform (`Params` in composite.wgsl) is [`UNIFORM_WORDS`] words: the 44 positional
//!   words, the field layout at [`FIELD_LAYOUT`] and one vec4 per package from word 48 (P1 first).
//!   Words 34, 35 and 43 stay unused.
//! - A neighbourhood field texel has five words, or six when a stage carries an input
//!   (`FilterStage::input`): P3 and P5 share that sixth word, nobody widens `FIELD_WORDS`.
//!
//! Binding budget (WebGPU defaults, which `Headless` requests): the composite bind group uses 6
//! of 8 storage buffers, up to 7 of 16 sampled textures, 3 of 12 uniform buffers and up to 16
//! bindings. New images go into existing atlases or texture arrays (`MaterialImages`,
//! `TileImageAtlas`), not new storage buffers.

use std::ops::Range;

use texturecraft_core::{Effect, Filter, TextureSet};

use super::{CompositeProgram, NEIGHBOURHOOD_OPCODE, NOOP_EFFECT_OPCODE, Operation, ProceduralInputs, brush, fill_projection, lighting, point, stack, udim};
use crate::gpu_neighbourhood::{box_family, gather};

/// First word of the field layout in the band uniform (`Params.field_layout`): words per field
/// texel, input word present, the prepare pass's input operation plus one, and the prepare
/// pass's stage flags (bit 0: the stage wraps, `FilterStage::wrap`).
pub(super) const FIELD_LAYOUT: usize = 44;
/// First word of the package vec4s (`Params.packages`): P1 at 48, P2 at 52, … P8 at 76.
const PACKAGE_SLOTS: usize = 48;
/// Words of the band uniform.
pub(super) const UNIFORM_WORDS: usize = PACKAGE_SLOTS + 8 * 4;

/// Opcodes of the packages' effects (`is_extended_effect` in WGSL).
pub(super) const EXTENDED_EFFECTS: Range<u32> = 128..192;
/// P2: point filters and gradients.
pub(super) const POINT_OPCODES: Range<u32> = 128..144;
/// P3: stack plumbing and mask effects (soft compare, colour selection output, …).
pub(super) const STACK_OPCODES: Range<u32> = 144..152;
/// P6: lighting and material-reading effects.
pub(super) const LIGHTING_OPCODES: Range<u32> = 152..160;
/// P1: projection selectors in `procedural[3]` (warp, copies with symmetry, cropped or culled
/// variants). 0-10 are taken: UV, triplanar, planar ×3, spherical, cylindrical ×3, decal, UV tile.
pub(super) const PROJECTION_SELECTORS: Range<u32> = 16..32;
/// Flag bytes of `Operation::ext[2]`, one per package.
pub(super) const POINT_OP_BITS: u32 = 0x0000_00ff;
pub(super) const STACK_OP_BITS: u32 = 0x0000_ff00;
pub(super) const LIGHTING_OP_BITS: u32 = 0x00ff_0000;
pub(super) const BRUSH_OP_BITS: u32 = 0xff00_0000;

/// A filter `point_effect` has no built-in arm for (or a setting its arm does not take): the
/// packages' lowering, or None to composite on the CPU. A package returns None without touching
/// `op` for every filter it does not evaluate. `op.low` is overwritten by the caller with the
/// three returned parameters and the opacity; other parameters go in `op.high`, `op.projection`
/// or the side table. A lowering may also queue a neighbourhood stage to run right before its
/// operation (`inputs.precede_with_stage`, the Gradient's input blur: P4's stage, P2's gradient).
pub(super) fn lower_filter(filter: &Filter, effect: &Effect, op: &mut Operation, inputs: &mut ProceduralInputs) -> Option<(u32, [f32; 3])> {
    #[cfg(all(test, not(target_arch = "wasm32")))]
    if STAGED_TEST_LOWERING.get() && matches!(filter, Filter::Smoothstep { .. }) {
        inputs.precede_with_stage(&Filter::Blur { radius: 24.0, wrap: false }, 1.0)?;
        return Some((21, [0.0; 3]));
    }
    let mut lowered = point::lower(filter, effect, op, inputs);
    if lowered.is_none() {
        lowered = lighting::lower(filter, effect, op, inputs);
    }
    if lowered.is_none() {
        // A stage queued by a lowering that gave up runs for nothing
        inputs.preceding_stage = None;
    }
    debug_assert!(
        lowered.is_none_or(|(kind, _)| EXTENDED_EFFECTS.contains(&kind) || kind == NOOP_EFFECT_OPCODE || kind == NEIGHBOURHOOD_OPCODE),
        "a package lowered {filter:?} outside its opcode block: {lowered:?}"
    );
    lowered
}

#[cfg(all(test, not(target_arch = "wasm32")))]
thread_local! {
    /// Test seam: lower Smoothstep as a Blur stage followed by Invert, the shape of a lowering
    /// that runs a neighbourhood stage before its operation.
    pub(super) static STAGED_TEST_LOWERING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether an operation needs the surface inputs (positions, normals, bakes) besides the
/// projection selector, generators and Baked Lighting checked by the caller.
pub(super) fn needs_surface(op: &Operation) -> bool {
    lighting::needs_surface(op)
}

/// Reorders a layer's finished channel operations (P6: Base Color after the channels lighting
/// reads). The caller then finds the Height operation again for the height mask; the per-layer
/// constraints are written after this call.
pub(super) fn order_layer_operations(operations: &mut [Operation]) {
    lighting::order_layer_operations(operations);
}

/// Whether the GPU program evaluates texture set `set` (P8: UDIM sets); `mask_preview` picks the message.
pub(super) fn check_set(set: &TextureSet, mask_preview: bool) -> Result<(), &'static str> {
    udim::check_set(set, mask_preview)
}

/// The band uniform's package vec4s.
pub(super) fn write_band_uniform(program: &CompositeProgram, values: &mut [u32; UNIFORM_WORDS]) {
    // P4 and P5 run in the neighbourhood shader with their own parameters; their vec4s stay zero
    // unless P5 writes its own from the prepare loop in `compose`.
    let slots = [
        fill_projection::band_uniform(program),
        point::band_uniform(program),
        stack::band_uniform(program),
        [0; 4],
        [0; 4],
        lighting::band_uniform(program),
        brush::band_uniform(program),
        udim::band_uniform(program),
    ];
    for (target, slot) in values[PACKAGE_SLOTS..].as_chunks_mut::<4>().0.iter_mut().zip(slots) {
        *target = slot;
    }
}

/// Which package WGSL files a program compiles in full; the others compile as stubs so that
/// Mesa does not inline unused code (gpu_composite_pipelines.rs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ShaderFlags {
    projection: bool,
    point: bool,
    stack: bool,
    lighting: bool,
    brush: bool,
    udim: bool,
}

impl ShaderFlags {
    pub(super) fn of(program: &CompositeProgram) -> Self {
        Self {
            projection: fill_projection::uses_shader(program),
            point: point::uses_shader(program),
            stack: stack::uses_shader(program),
            lighting: lighting::uses_shader(program),
            brush: brush::uses_shader(program),
            udim: udim::uses_shader(program),
        }
    }

    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(super) const ALL: Self = Self { projection: true, point: true, stack: true, lighting: true, brush: true, udim: true };
    #[cfg(all(test, not(target_arch = "wasm32")))]
    const NONE: Self = Self { projection: false, point: false, stack: false, lighting: false, brush: false, udim: false };
    /// Each package's file in full, the others as stubs.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(super) const EACH: [Self; 6] = [
        Self { projection: true, ..Self::NONE },
        Self { point: true, ..Self::NONE },
        Self { stack: true, ..Self::NONE },
        Self { lighting: true, ..Self::NONE },
        Self { brush: true, ..Self::NONE },
        Self { udim: true, ..Self::NONE },
    ];
}

/// The packages' WGSL for the composite shader: this hub's dispatcher, then each package file
/// or its stub.
pub(super) fn shader_sources(flags: ShaderFlags) -> [&'static str; 7] {
    let pick = |full: bool, source: &'static str, stub: &'static str| if full { source } else { stub };
    [
        include_str!("shaders/composite_packages.wgsl"),
        pick(flags.projection, fill_projection::WGSL, fill_projection::WGSL_STUB),
        pick(flags.point, point::WGSL, point::WGSL_STUB),
        pick(flags.stack, stack::WGSL, stack::WGSL_STUB),
        pick(flags.lighting, lighting::WGSL, lighting::WGSL_STUB),
        pick(flags.brush, brush::WGSL, brush::WGSL_STUB),
        pick(flags.udim, udim::WGSL, udim::WGSL_STUB),
    ]
}

/// Filter library ids (crates/engine/src/filter_lib.rs `FILTERS`) that still composite on the CPU
/// with their library defaults, as (package, id). The library gate in
/// crates/ui-egui/src/gpu_port_gate.rs fails when this disagrees with the compiler.
pub(super) fn library_still_on_cpu() -> Vec<(&'static str, &'static str)> {
    [
        ("P1", fill_projection::LIBRARY_STILL_ON_CPU),
        ("P2", point::LIBRARY_STILL_ON_CPU),
        ("P3", stack::LIBRARY_STILL_ON_CPU),
        ("P4", box_family::LIBRARY_STILL_ON_CPU),
        ("P5", gather::LIBRARY_STILL_ON_CPU),
        ("P6", lighting::LIBRARY_STILL_ON_CPU),
        ("P7", brush::LIBRARY_STILL_ON_CPU),
        ("P8", udim::LIBRARY_STILL_ON_CPU),
    ]
    .into_iter()
    .flat_map(|(package, ids)| ids.iter().map(move |id| (package, *id)))
    .collect()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::collections::BTreeMap;

    use texturecraft_compose::PixelStore;
    use texturecraft_core::{
        BlendMode, Channel, ChannelBlend, ContentsOffset, EffectKind, Fill, Filter, Layer, LayerKind, Mask, MaskBackground, Project, Source, TextureSet,
    };

    use super::{EXTENDED_EFFECTS, LIGHTING_OPCODES, POINT_OPCODES, STACK_OPCODES};
    use crate::gpu_composite::CompositeProgram;
    use crate::gpu_composite::oracle::{compile_effects, effect};

    /// Stacks the GPU evaluates must keep compiling while the packages land. Each package keeps
    /// its own table of what still falls back (`still_on_cpu` in its test module).
    #[test]
    fn effects_with_a_gpu_evaluation_stay_on_the_gpu() {
        let none = ContentsOffset::NONE;

        // A stack the GPU evaluates, including a filter not chosen yet
        let (material, mask) = compile_effects(Some(effect(EffectKind::Filter(Filter::Empty))), Some(effect(EffectKind::Filter(Filter::Invert))), none);
        assert!(material.is_ok() && mask.is_ok(), "{material:?} {mask:?}");

        // A mask blends by the shared blend alone (no channel), so the CPU ignores per-channel blends and switched-off channels there
        let mut per_channel = effect(EffectKind::Filter(Filter::Invert));
        per_channel.channel_blends.insert(Channel::Height, ChannelBlend { mode: BlendMode::Multiply, opacity: 0.5 });
        let mut switched_off = effect(EffectKind::Filter(Filter::Invert));
        switched_off.off_channels.insert(Channel::Height);
        for effect in [per_channel, switched_off] {
            let (material, mask) = compile_effects(None, Some(effect.clone()), none);
            assert!(material.is_ok() && mask.is_ok(), "mask {effect:?}: {material:?} {mask:?}");
        }

        // An input left behind by the filter it replaced reads nothing
        let mut with_input = effect(EffectKind::Filter(Filter::Invert));
        with_input.inputs.insert("mask".into(), Source::Uniform([0.5; 3]));
        assert!(compile_effects(Some(with_input.clone()), None, none).0.is_ok(), "{with_input:?}");
        assert!(compile_effects(None, Some(with_input.clone()), none).1.is_ok(), "{with_input:?}");

        // An instance in nested folders without Scale or Rotation
        let channels = [Channel::BaseColor, Channel::Height];
        let fill = || Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.6, 0.3, 0.2]))]), ..Default::default() };
        let mut project = Project::new("Nested folders", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, 64));
        let source = project.alloc_id();
        project.insert_layer(0, Layer::new(source, "Source", LayerKind::Fill(fill()), &channels), None, false).unwrap();
        let outer = project.alloc_id();
        project.insert_layer(0, Layer::new(outer, "Outer", LayerKind::folder(), &channels), None, false).unwrap();
        let inner = project.alloc_id();
        project.insert_layer(0, Layer::new(inner, "Inner", LayerKind::folder(), &channels), Some(outer), true).unwrap();
        let instance = project.alloc_id();
        project.insert_layer(0, Layer::new(instance, "Instance", LayerKind::Instance { source }, &channels), Some(inner), true).unwrap();
        let pixels = PixelStore::default();
        assert!(CompositeProgram::try_compile_region(&project, &pixels, 0, None).is_ok(), "the same stack without the offset");

        // The source of an instance placed in a folder with Scale and Rotation sits outside it
        let mut project = Project::new("Instanced mask", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, 64));
        let source = project.alloc_id();
        let mut layer = Layer::new(source, "Source", LayerKind::Fill(fill()), &channels);
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
        assert!(CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, source, None, None).is_ok(), "the source itself sits outside the folder");
    }

    /// A lowering that runs a neighbourhood stage before its operation (the seam of the
    /// Gradient's input blur) compiles to the same program as the two effects one after the
    /// other, in content and in the mask, for the material and the mask preview.
    #[test]
    fn a_lowering_can_run_a_neighbourhood_stage_before_its_operation() {
        let compile = |effects: &[(Filter, f32)]| {
            let channels = [Channel::BaseColor, Channel::Height];
            let mut project = Project::new("Staged lowering", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, 64));
            let owner = project.alloc_id();
            let fill = Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.6, 0.3, 0.2]))]), ..Default::default() };
            let mut layer = Layer::new(owner, "Surface", LayerKind::Fill(fill), &channels);
            let mut mask = Mask::new(MaskBackground::White);
            for (filter, opacity) in effects {
                for stack in [&mut layer.content_effects, &mut mask.effects] {
                    let mut added = effect(EffectKind::Filter(filter.clone()));
                    added.opacity = *opacity;
                    stack.push(added);
                }
            }
            layer.mask = Some(mask);
            project.insert_layer(0, layer, None, false).unwrap();
            let pixels = PixelStore::default();
            let material = CompositeProgram::try_compile_region(&project, &pixels, 0, None).unwrap();
            let preview = CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, owner, None, None).unwrap();
            [material, preview].map(|program| (bytemuck::cast_slice::<_, u8>(&program.operations).to_vec(), program.filters.stages.len()))
        };
        super::STAGED_TEST_LOWERING.set(true);
        let staged = compile(&[(Filter::Smoothstep { min: 0.0, max: 1.0 }, 0.77)]);
        super::STAGED_TEST_LOWERING.set(false);
        let separate = compile(&[(Filter::Blur { radius: 24.0, wrap: false }, 1.0), (Filter::Invert, 0.77)]);
        // A content stage per channel operation and one for the shared mask; the preview has the mask stage
        assert!(staged[0].1 >= 2 && staged[1].1 == 1, "{:?}", (staged[0].1, staged[1].1));
        assert_eq!(staged, separate);
    }

    /// Every stage knows what it filters (P0c): colour and Height content and the mask, while the
    /// built-in stages read only `middle` (0 on Height) as before, and no stage wraps yet.
    #[test]
    fn stages_carry_their_context_and_built_in_settings_stay() {
        use crate::gpu_neighbourhood::{STAGE_COLOR, STAGE_MASK, STAGE_SIGNED};

        let channels = [Channel::BaseColor, Channel::Height];
        let mut project = Project::new("Stage context", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, 64));
        let owner = project.alloc_id();
        let sources = BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.6, 0.3, 0.2])), (Channel::Height, Source::Uniform([-0.3; 3]))]);
        let mut layer = Layer::new(owner, "Surface", LayerKind::Fill(Fill { sources, ..Default::default() }), &channels);
        layer.content_effects.push(effect(EffectKind::Filter(Filter::HighPass {
            radius: 64.0,
            space: texturecraft_core::HighpassSpace::Working,
            alpha: false,
        })));
        let mut mask = Mask::new(MaskBackground::White);
        mask.effects.push(effect(EffectKind::Filter(Filter::Blur { radius: 64.0, wrap: false })));
        layer.mask = Some(mask);
        project.insert_layer(0, layer, None, false).unwrap();
        let program = CompositeProgram::try_compile_region(&project, &PixelStore::default(), 0, None).unwrap();
        let stages: Vec<_> = program.filters.stages.iter().filter_map(|stage| stage.filter.map(|filter| (filter.context, filter.middle))).collect();
        assert!(!program.filters.wraps());
        assert_eq!(stages.len(), 3, "{stages:?}");
        assert!(stages.contains(&(STAGE_MASK, 0.5)), "{stages:?}");
        assert!(stages.contains(&(STAGE_COLOR, 0.5)), "{stages:?}");
        assert!(stages.contains(&(STAGE_SIGNED, 0.0)), "{stages:?}");
    }

    #[test]
    fn package_opcode_blocks_are_disjoint_and_inside_the_extended_range() {
        let blocks = [POINT_OPCODES, STACK_OPCODES, LIGHTING_OPCODES];
        for (index, block) in blocks.iter().enumerate() {
            assert!(EXTENDED_EFFECTS.start <= block.start && block.end <= EXTENDED_EFFECTS.end, "{block:?}");
            for other in &blocks[index + 1..] {
                assert!(block.end <= other.start || other.end <= block.start, "{block:?} overlaps {other:?}");
            }
        }
        // Group markers and the Tile Generator stay outside the effects
        for kind in [crate::gpu_tile_pattern::TILE_GENERATOR_OPCODE, super::super::BEGIN_GROUP, super::super::END_GROUP] {
            assert!(!EXTENDED_EFFECTS.contains(&kind), "{kind}");
        }
    }
}
