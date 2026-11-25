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
/// First word of the whole-set pre-pass table (`Params.whole_slots`, `whole_bases`,
/// `whole_rect`), after the package vec4s.
pub(super) const WHOLE_SET: usize = PACKAGE_SLOTS + 8 * 4;
/// Words of the band uniform.
pub(super) const UNIFORM_WORDS: usize = MASK_VIEW + 4;
/// The lit Base Color's mask views (`mask_view` in composite.wgsl, `FilterPlan::add_lit_mask_views`).
pub(super) const MASK_VIEW: usize = WHOLE_COMPACT + 4;
/// The whole-set pre-pass's compact slots (`whole_compact` in composite.wgsl): the first stage
/// slot of the pass, its stage count, the program's first working field, and 1 when active.
pub(super) const WHOLE_COMPACT: usize = WHOLE_SET + 3 * 4;

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
    #[cfg(all(test, not(target_arch = "wasm32")))]
    if let (Some(probe), Filter::Smoothstep { min, max }) = (PROBE_LOWERING.get(), filter) {
        match (probe, PROBE_SELECTOR.get()) {
            (188 | 189, _) => op.spatial[3] = *min as u32 + 1,
            (190, _) => op.ext[0] = inputs.pack_image("probe", crate::gpu_image_source::ImageUse::Material)?,
            (_, 0) => op.effects[3] = inputs.register_input(Operation { low: [*max; 4], ..bytemuck::Zeroable::zeroed() }),
            (_, 5) => {
                op.spatial[3] = *min as u32 + 1;
                op.spatial[2] = *max as u32 + 1;
                op.ext[0] = inputs.pack_words(&[5])?;
            }
            (_, 6) => {
                let environment = inputs.reserve_environment()?;
                op.ext[0] = inputs.pack_words(&[6, environment, *max as u32])?;
            }
            (_, 9) => {
                op.ext[0] = inputs.pack_words(&[9])?;
                inputs.pack_image("probe", crate::gpu_image_source::ImageUse::Material)?;
            }
            (_, selector) => {
                if selector >= 7 {
                    inputs.read_bake_rows();
                }
                op.ext[0] = inputs.pack_words(&[selector])?;
            }
        }
        return Some((probe, [0.0; 3]));
    }
    // Test seam: Height To Normal and Color Match as the input probe stage, Color Match with
    // colour inputs of its source and target colours (the second one in `effects[2]`)
    #[cfg(all(test, not(target_arch = "wasm32")))]
    if crate::gpu_neighbourhood::INPUT_PROBE.get() && matches!(filter, Filter::HeightToNormal(_) | Filter::ColorMatch(_)) {
        if let Filter::ColorMatch(matching) = filter {
            let source = |color: [f32; 3]| Operation { low: [color[0], color[1], color[2], 1.0], ..bytemuck::Zeroable::zeroed() };
            op.effects[3] = inputs.register_color_input(source(matching.source_color));
            op.effects[2] = inputs.register_color_input(source(matching.target_color));
        }
        op.ids[2] = inputs.filters.len() as u32;
        inputs.filters.push((filter.clone(), effect.opacity));
        return Some((super::NEIGHBOURHOOD_OPCODE, [0.0; 3]));
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
    /// Test seam: lower Smoothstep as a probe of the P0g hooks (opcodes 188..192 in
    /// composite_packages.wgsl): 188 this layer, 189 the stack below (channel slot `min`),
    /// 190 an image named "probe" read nearest, 191 an image input of uniform `max`.
    pub(super) static PROBE_LOWERING: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
    /// Test seam: with probe 191, a P0h hook in place of the image input (`probe_late_hooks` in
    /// composite_packages.wgsl): 1..4 the surface tangent, 5 the second stack read (channel slots
    /// `min` and `max`), 6 environment word `max`, 7 the baked Height row, 8 the ambient
    /// occlusion with the bake rows present, 9 the image "probe" bilinear and clamped.
    pub(super) static PROBE_SELECTOR: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Whether an operation needs the surface inputs (positions, normals, bakes) besides the
/// projection selector, generators and Baked Lighting checked by the caller.
pub(super) fn needs_surface(op: &Operation) -> bool {
    #[cfg(all(test, not(target_arch = "wasm32")))]
    if op.ids[1] == 191 && op.ext[0] != 0 {
        return true;
    }
    lighting::needs_surface(op)
}

/// Whether a mask effect operation reads its own layer's content (`this_layer_content` in WGSL):
/// the layer's content stacks are then prepared before its mask stack, so the mask's prepare
/// pass reads prepared content fields. Content stacks never read the mask.
pub(super) fn mask_reads_own_layer(op: &Operation) -> bool {
    #[cfg(all(test, not(target_arch = "wasm32")))]
    if op.ids[1] == 188 {
        return true;
    }
    stack::reads_own_layer(op)
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
pub(super) fn shader_sources(flags: ShaderFlags) -> [&'static str; 8] {
    let pick = |full: bool, source: &'static str, stub: &'static str| if full { source } else { stub };
    // The hooks' test probes only in the tests: the driver compiles every function the shader holds
    let probes = if cfg!(test) {
        include_str!("shaders/composite_probes.wgsl")
    } else {
        "const PROBE_OPCODES_START: u32 = 188u;\nfn probe_mask_effect(effect: Operation, texel: LayerTexel) -> f32 { return 0.0; }"
    };
    [
        include_str!("shaders/composite_packages.wgsl"),
        probes,
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

    /// Without a wrapping stage the field lies inside the set, so the set bounds a stage that
    /// does not wrap stays within reach past the whole field; with one, they are the set itself.
    #[test]
    fn set_bounds_cover_the_field_unless_the_program_wraps() {
        let size = (128, 96);
        for rect in [[0, 0, 128, 96], [32, 16, 64, 48], [96, 64, 32, 32]] {
            let [x0, y0, x1, y1] = super::super::field_bounds(rect, size);
            assert!(x0 <= 0 && y0 <= 0 && x1 >= rect[2] as i32 && y1 >= rect[3] as i32, "{rect:?}");
            assert_eq!([x1 - x0, y1 - y0], [128, 96], "the bounds are the set, {rect:?}");
        }
        // A wrapping program's field starts 30 texels before the set's corner
        let start = (-30i32) as u32;
        assert_eq!(super::super::field_bounds([start, start, 76, 76], size), [30, 30, 158, 126]);
    }

    /// A stack whose upper Blur wraps over a lower one that does not (P0e): the wrapping stage
    /// reads the lower field's periodic copies past the set's edges and the lower stage stays
    /// inside the set, as the CPU widens lower regions across a crossed edge. Staged before P4:
    /// the GPU program compiles the stack without wrap and marks the stage, the CPU evaluates
    /// `wrap: true`. Content on paint near the corner (full and a corner region), and a mask with
    /// a blurred generator and painting near the edge.
    #[test]
    fn a_wrapping_stage_over_a_lower_stage_matches_the_cpu_at_the_corner() {
        use std::sync::Arc;

        use texturecraft_compose::eval::{Composite, Inputs, composite_tiles, mask_image};
        use texturecraft_compose::{BakedMaps, DisplayKind, ImageRes, ImageStore, tile_bytes};
        use texturecraft_core::{BakedMap, Effect, GeneratorKind};
        use texturecraft_geom::{UvGBuffer, primitives};
        use texturecraft_texel::{TILE, TileImage, to_u16};

        use crate::gpu_composite::CompositeRegion;
        use crate::gpu_composite::oracle::headless;
        use crate::readback::Readback;

        let Some(mut gpu) = headless() else { return };
        const SIZE: u32 = 128;
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let baked = BakedMaps::default();
        let images = ImageStore::from([("AO override".into(), Arc::new(ImageRes::new(2, 2, vec![[0.15; 4], [0.7; 4], [0.85; 4], [0.3; 4]], false)))]);
        let build = |wrap: bool| {
            let mut project = Project::new("Wrapped dependency", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
            let owner = project.alloc_id();
            let mut layer = Layer::new(owner, "Paint", LayerKind::Paint, &[Channel::BaseColor]);
            layer.content_effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 24.0, wrap: false })));
            layer.content_effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 160.0, wrap })));
            let mut mask = Mask::new(MaskBackground::Black);
            let mask_paint = project.alloc_id();
            mask.effects.push(Effect::new(mask_paint, EffectKind::Paint));
            let mut generator = crate::gpu_composite::oracle::earlier_generator(GeneratorKind::Dirt);
            generator.noise_amount = 0.0;
            generator.global.blur = 0.85;
            generator.inputs.insert(BakedMap::AmbientOcclusion, Source::image("AO override"));
            let mut ramp = Effect::new(project.alloc_id(), EffectKind::Generator(generator));
            ramp.blend = BlendMode::LinearDodge;
            ramp.opacity = 0.6;
            mask.effects.push(ramp);
            mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 160.0, wrap })));
            layer.mask = Some(mask);
            project.insert_layer(0, layer, None, false).unwrap();
            (project, owner, mask_paint)
        };
        let (cpu_project, owner, mask_paint) = build(true);
        let (gpu_project, _, _) = build(false);
        let mut pixels = PixelStore::default();
        let paint = pixels.layer_mut(owner, Channel::BaseColor, (SIZE, SIZE));
        let mut mask_pixels = TileImage::new(SIZE, SIZE, 2);
        for y in 0..SIZE {
            for x in 0..SIZE {
                // Shapes against the left and top edges and in the far corner
                let near_edge = (x < 9 && (20..70).contains(&y)) || (y < 7 && (30..90).contains(&x)) || (x > 116 && y > 112);
                if near_edge {
                    paint.set(x, y, &[to_u16(0.8), to_u16(0.25), to_u16(0.4), u16::MAX]);
                    mask_pixels.set(x, y, &[to_u16(0.9), u16::MAX]);
                }
            }
        }
        pixels.masks.insert(mask_paint, mask_pixels);

        let inputs = Inputs {
            project: &cpu_project,
            ts: 0,
            mesh: &mesh,
            gbuf: &gbuf,
            pixels: &pixels,
            baked: &baked,
            block_scales: cpu_project.texture_sets[0].block_scales(),
            preview: None,
            images: &images,
            bounds: mesh.bounds(),
        };
        // The stages of the 160 blur wrap; the generator's blur and the 24 blur do not
        let wide = crate::gpu_neighbourhood::NeighbourhoodFilter::new(&Filter::Blur { radius: 160.0, wrap: false }, SIZE, SIZE, 1.0, None).unwrap().radius;
        let mark = |program: &mut CompositeProgram| {
            for stage in &mut program.filters.stages {
                // Resolution cells do not read signed origins yet (brief section 1.2)
                assert_eq!(stage.cell, 1);
                stage.wrap = stage.filter.is_some_and(|filter| filter.radius == wide) && stage.preceding & super::super::GENERATOR_STAGE == 0;
            }
            assert!(program.filters.wraps());
        };
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);

        let tiles: Vec<_> = (0..SIZE / TILE).flat_map(|y| (0..SIZE / TILE).map(move |x| (x, y))).collect();
        let mut oracle = Composite::new(&[Channel::BaseColor], SIZE);
        composite_tiles(&inputs, &mut oracle, &tiles);
        let mut program = CompositeProgram::try_compile_region(&gpu_project, &pixels, 0, None).unwrap();
        mark(&mut program);
        program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
        program.set_tile_images(&images);
        let read = |gpu: &crate::headless::Headless| {
            let texture = gpu.renderer.display_textures()[0][DisplayKind::BaseColor as usize];
            Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap().2
        };
        gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
        let full = read(&gpu);
        for &(tx, ty) in &tiles {
            let expected = tile_bytes(&oracle, &baked, DisplayKind::BaseColor, tx, ty);
            for (y, row) in expected.as_chunks::<{ TILE as usize * 4 }>().0.iter().enumerate() {
                let start = ((ty * TILE + y as u32) * SIZE + tx * TILE) as usize * 4;
                for (x, (gpu, cpu)) in full[start..start + row.len()].as_chunks::<4>().0.iter().zip(row.as_chunks::<4>().0).enumerate() {
                    let largest = gpu.iter().zip(cpu).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
                    assert!(largest <= 2, "material ({}, {}): GPU {gpu:?}, CPU {cpu:?}", tx * TILE + x as u32, ty * TILE + y as u32);
                }
            }
        }
        program.region = Some(CompositeRegion { min: [0, 0], max: [16, 16] });
        gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
        assert!(read(&gpu).iter().zip(&full).all(|(a, b)| a.abs_diff(*b) <= 1), "the corner region changed the result");

        let expected = mask_image(&inputs, owner, SIZE).unwrap();
        let mut mask = CompositeProgram::try_compile_mask_preview(&gpu_project, &pixels, 0, owner, None, None).unwrap();
        mark(&mut mask);
        mask.set_surface_inputs(&mesh, &gbuf, None).unwrap();
        mask.set_tile_images(&images);
        gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &mask).unwrap();
        let (_, _, actual) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.aux_texture(0).unwrap()).unwrap();
        for (index, (&expected, pixel)) in expected.iter().zip(actual.as_chunks::<4>().0).enumerate() {
            assert!(pixel[0].abs_diff(expected) <= 2, "mask texel {index}: {} instead of {expected}", pixel[0]);
        }
    }

    /// The whole-set pre-pass (P0f): a stage marked whole-set (test seam: built-in stages
    /// reaching 30 texels or more) runs once per update with the lower stages of its stack, and
    /// the bands read its result instead of their own field. Paint content [Blur 24, Blur 160,
    /// Blur 24] and a painted mask with Blur 160, against the CPU: full, a partial update of a
    /// corner region, and with the Blur 160 stages wrapping (staged as in the P0e test).
    #[test]
    fn whole_set_stages_match_the_cpu_through_the_pre_pass() {
        use std::sync::Arc;

        use texturecraft_compose::eval::{Composite, Inputs, composite_tiles, mask_image};
        use texturecraft_compose::{BakedMaps, DisplayKind, ImageStore, tile_bytes};
        use texturecraft_core::Effect;
        use texturecraft_geom::{UvGBuffer, primitives};
        use texturecraft_texel::{TILE, TileImage, to_u16};

        use crate::gpu_composite::CompositeRegion;
        use crate::gpu_composite::oracle::headless;
        use crate::gpu_neighbourhood::WHOLE_SET_TEST_REACH;
        use crate::readback::Readback;

        let Some(mut gpu) = headless() else { return };
        const SIZE: u32 = 128;
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let (baked, images) = (BakedMaps::default(), ImageStore::default());
        let build = |wrap: bool| {
            let mut project = Project::new("Whole-set stage", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
            let owner = project.alloc_id();
            let mut layer = Layer::new(owner, "Paint", LayerKind::Paint, &[Channel::BaseColor]);
            for filter in [Filter::Blur { radius: 24.0, wrap: false }, Filter::Blur { radius: 160.0, wrap }, Filter::Blur { radius: 24.0, wrap: false }] {
                layer.content_effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(filter)));
            }
            let mut mask = Mask::new(MaskBackground::Black);
            let mask_paint = project.alloc_id();
            mask.effects.push(Effect::new(mask_paint, EffectKind::Paint));
            mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 160.0, wrap })));
            layer.mask = Some(mask);
            project.insert_layer(0, layer, None, false).unwrap();
            (project, owner, mask_paint)
        };
        let wide = crate::gpu_neighbourhood::NeighbourhoodFilter::new(&Filter::Blur { radius: 160.0, wrap: false }, SIZE, SIZE, 1.0, None).unwrap();
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        let tiles: Vec<_> = (0..SIZE / TILE).flat_map(|y| (0..SIZE / TILE).map(move |x| (x, y))).collect();
        for wrap in [false, true] {
            let (cpu_project, owner, mask_paint) = build(wrap);
            let (gpu_project, _, _) = build(false);
            let mut pixels = PixelStore::default();
            let paint = pixels.layer_mut(owner, Channel::BaseColor, (SIZE, SIZE));
            let mut mask_pixels = TileImage::new(SIZE, SIZE, 2);
            for y in 0..SIZE {
                for x in 0..SIZE {
                    if (x < 9 && (20..70).contains(&y))
                        || (y < 7 && (30..90).contains(&x))
                        || ((50..60).contains(&x) && (60..75).contains(&y))
                        || (x > 116 && y > 112)
                    {
                        paint.set(x, y, &[to_u16(0.8), to_u16(0.25), to_u16(0.4), u16::MAX]);
                        mask_pixels.set(x, y, &[to_u16(0.9), u16::MAX]);
                    }
                }
            }
            pixels.masks.insert(mask_paint, mask_pixels);
            let inputs = Inputs {
                project: &cpu_project,
                ts: 0,
                mesh: &mesh,
                gbuf: &gbuf,
                pixels: &pixels,
                baked: &baked,
                block_scales: cpu_project.texture_sets[0].block_scales(),
                preview: None,
                images: &images,
                bounds: mesh.bounds(),
            };
            let compile = |region: Option<CompositeRegion>, mask: bool| {
                WHOLE_SET_TEST_REACH.set(Some(wide.reach()));
                let program = if mask {
                    CompositeProgram::try_compile_mask_preview(&gpu_project, &pixels, 0, owner, region, None)
                } else {
                    CompositeProgram::try_compile_region(&gpu_project, &pixels, 0, region)
                };
                WHOLE_SET_TEST_REACH.set(None);
                let mut program = program.unwrap();
                for stage in &mut program.filters.stages {
                    stage.wrap = wrap && stage.filter.is_some_and(|filter| filter.radius == wide.radius);
                }
                let whole = program.filters.whole_set().expect("the Blur 160 stages run in the pre-pass");
                assert!(whole.outputs.iter().all(|&output| program.filters.stages[output].filter.is_some_and(|filter| filter.radius == wide.radius)));
                assert!(program.filters.band_reach() < program.filters.reach, "the bands no longer carry the pre-pass's reach");
                program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
                program
            };
            let read = |gpu: &crate::headless::Headless| {
                let texture = gpu.renderer.display_textures()[0][DisplayKind::BaseColor as usize];
                Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap().2
            };

            let mut oracle = Composite::new(&[Channel::BaseColor], SIZE);
            composite_tiles(&inputs, &mut oracle, &tiles);
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &compile(None, false)).unwrap();
            let full = read(&gpu);
            for &(tx, ty) in &tiles {
                let expected = tile_bytes(&oracle, &baked, DisplayKind::BaseColor, tx, ty);
                for (y, row) in expected.as_chunks::<{ TILE as usize * 4 }>().0.iter().enumerate() {
                    let start = ((ty * TILE + y as u32) * SIZE + tx * TILE) as usize * 4;
                    for (x, (gpu, cpu)) in full[start..start + row.len()].as_chunks::<4>().0.iter().zip(row.as_chunks::<4>().0).enumerate() {
                        let largest = gpu.iter().zip(cpu).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
                        assert!(largest <= 2, "wrap {wrap} material ({}, {}): GPU {gpu:?}, CPU {cpu:?}", tx * TILE + x as u32, ty * TILE + y as u32);
                    }
                }
            }
            // A partial update: the pre-pass covers only what the corner region's bands read
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &compile(Some(CompositeRegion { min: [0, 0], max: [16, 16] }), false)).unwrap();
            assert!(read(&gpu).iter().zip(&full).all(|(a, b)| a.abs_diff(*b) <= 1), "wrap {wrap}: the corner region changed the result");

            let expected = mask_image(&inputs, owner, SIZE).unwrap();
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &compile(None, true)).unwrap();
            let (_, _, actual) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.aux_texture(0).unwrap()).unwrap();
            for (index, (&expected, pixel)) in expected.iter().zip(actual.as_chunks::<4>().0).enumerate() {
                assert!(pixel[0].abs_diff(expected) <= 2, "wrap {wrap} mask texel {index}: {} instead of {expected}", pixel[0]);
            }
        }
    }

    /// Timing and working memory of a large-reach stage at 2K and 4K, in the bands and in the
    /// whole-set pre-pass (test seam). `cargo test … -- --ignored --nocapture bench_whole_set`.
    #[test]
    #[ignore = "GPU benchmark"]
    fn bench_whole_set_stage() {
        use std::sync::Arc;
        use std::time::Instant;

        use texturecraft_core::Effect;
        use texturecraft_geom::{UvGBuffer, primitives};
        use texturecraft_texel::to_u16;

        use crate::gpu_composite::oracle::headless;
        use crate::gpu_neighbourhood::{NeighbourhoodFilter, WHOLE_SET_TEST_REACH};

        let Some(mut gpu) = headless() else { return };
        let limits = gpu.device.limits();
        println!(
            "BENCH adapter {}: storage binding {} MiB, buffer {} MiB",
            gpu.adapter_name,
            limits.max_storage_buffer_binding_size >> 20,
            limits.max_buffer_size >> 20
        );
        let mesh = Arc::new(primitives::plane(1.0, 1));
        for size in [2048u32, 4096] {
            let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, size, size));
            let mut project = Project::new("Whole-set bench", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, size));
            let owner = project.alloc_id();
            let mut layer = Layer::new(owner, "Paint", LayerKind::Paint, &[Channel::BaseColor]);
            // A Bevel-like reach: about 411 texels at 4K
            let blur = Filter::Blur { radius: 68.5, wrap: false };
            layer.content_effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(blur.clone())));
            project.insert_layer(0, layer, None, false).unwrap();
            let mut pixels = PixelStore::default();
            let paint = pixels.layer_mut(owner, Channel::BaseColor, (size, size));
            for y in (0..size).step_by(7) {
                for x in (0..size).step_by(5) {
                    paint.set(x, y, &[to_u16(0.8), to_u16(0.25), to_u16(0.4), u16::MAX]);
                }
            }
            let reach = NeighbourhoodFilter::new(&blur, size, size, 1.0, Some(texturecraft_core::ChannelKind::Color)).unwrap().reach();
            gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((size, size), 0)], 1);
            for whole in [false, true] {
                WHOLE_SET_TEST_REACH.set(whole.then_some(reach));
                let mut program = CompositeProgram::try_compile_region(&project, &pixels, 0, None).unwrap();
                WHOLE_SET_TEST_REACH.set(None);
                program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
                let mut times = Vec::new();
                for _ in 0..4 {
                    let started = Instant::now();
                    gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
                    gpu.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
                    times.push(started.elapsed().as_secs_f64() * 1000.0);
                }
                let result_mb = if whole { (u64::from(size) * u64::from(size) * 5 * 4) >> 20 } else { 0 };
                println!(
                    "BENCH {size}² reach {reach} whole-set {whole}: first {:.0} ms, warm {:.0} ms (result {result_mb} MiB)",
                    times[0],
                    times[1..].iter().copied().fold(f64::MAX, f64::min)
                );
            }
        }
    }

    /// A stack whose reach is half a 4K set (a Drop Shadow's) composites in bands on an adapter
    /// whose full limits allow it, without the pre-pass. `-- --ignored large_halo`.
    #[test]
    #[ignore = "4K GPU memory"]
    fn a_large_halo_stack_composites_at_4k_with_the_adapter_limits() {
        use std::sync::Arc;

        use texturecraft_core::Effect;
        use texturecraft_geom::{UvGBuffer, primitives};
        use texturecraft_texel::to_u16;

        use crate::gpu_composite::oracle::headless;

        let Some(mut gpu) = headless() else { return };
        const SIZE: u32 = 4096;
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let mut project = Project::new("Large halo", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
        let owner = project.alloc_id();
        let mut layer = Layer::new(owner, "Paint", LayerKind::Paint, &[Channel::BaseColor]);
        // Three box passes of 683 texels: a reach of 2049, half the set
        layer.content_effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 341.5, wrap: false })));
        project.insert_layer(0, layer, None, false).unwrap();
        let mut pixels = PixelStore::default();
        pixels.layer_mut(owner, Channel::BaseColor, (SIZE, SIZE)).set(2000, 2000, &[to_u16(0.8), to_u16(0.25), to_u16(0.4), u16::MAX]);
        let mut program = CompositeProgram::try_compile_region(&project, &pixels, 0, None).unwrap();
        assert!(program.filters.whole_set().is_none() && program.filters.reach >= SIZE / 2, "{}", program.filters.reach);
        program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
    }

    /// The P0g hooks reach the shader (test probes, opcodes 188..192): a mask effect reads the
    /// stack below its layer and its layer's own content (both against a mask of that value),
    /// an image by nearest texel and an image input, through the mask preview.
    #[test]
    fn stack_reads_images_and_inputs_reach_the_shader() {
        use std::sync::Arc;

        use texturecraft_compose::{BakedMaps, DisplayKind, ImageRes, ImageStore};
        use texturecraft_core::{Effect, Projection, Sampling};
        use texturecraft_geom::{UvGBuffer, primitives};

        use crate::gpu_composite::oracle::headless;
        use crate::readback::Readback;

        let Some(mut gpu) = headless() else { return };
        const SIZE: u32 = 64;
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let baked = BakedMaps::default();
        let images = ImageStore::from([(
            "probe".into(),
            Arc::new(ImageRes::new(2, 2, [51.0, 102.0, 204.0, 153.0].map(|v: f32| [v / 255.0, 0.0, 0.0, 1.0]).to_vec(), false)),
        )]);
        let roughness = super::super::channel_slot(Channel::Roughness);
        let channels = [Channel::BaseColor, Channel::Roughness];
        // A lower fill of roughness 0.3, an upper white fill of roughness 0.8 masked by `mask`
        let build = |mask: EffectKind| {
            let mut project = Project::new("Probes", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
            let lower = Fill {
                sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.1, 0.4, 0.2])), (Channel::Roughness, Source::Uniform([0.3; 3]))]),
                ..Default::default()
            };
            let lower_id = project.alloc_id();
            project.insert_layer(0, Layer::new(lower_id, "Lower", LayerKind::Fill(lower), &channels), None, false).unwrap();
            let owner = project.alloc_id();
            let upper = Fill {
                sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([1.0; 3])), (Channel::Roughness, Source::Uniform([0.8; 3]))]),
                ..Default::default()
            };
            let mut layer = Layer::new(owner, "Upper", LayerKind::Fill(upper), &channels);
            let mut layer_mask = Mask::new(MaskBackground::Black);
            layer_mask.effects.push(Effect::new(project.alloc_id(), mask));
            layer.mask = Some(layer_mask);
            project.insert_layer(0, layer, None, false).unwrap();
            (project, owner)
        };
        let uniform =
            |value: f32| EffectKind::Fill { source: Source::Uniform([value; 3]), projection: Projection::default(), sampling: Sampling::SAVED_BEFORE };
        let probe = |kind: u32, min: f32, max: f32| {
            super::PROBE_LOWERING.set(Some(kind));
            let filter = EffectKind::Filter(Filter::Smoothstep { min, max });
            (build(filter), kind)
        };
        let compile = |project: &Project, owner, mask_preview: bool| {
            let pixels = PixelStore::default();
            let mut program = if mask_preview {
                CompositeProgram::try_compile_mask_preview(project, &pixels, 0, owner, None, None).unwrap()
            } else {
                CompositeProgram::try_compile_region(project, &pixels, 0, None).unwrap()
            };
            super::PROBE_LOWERING.set(None);
            program.set_surface_inputs(&mesh, &gbuf, Some(&baked)).unwrap();
            program.set_tile_images(&images);
            program
        };
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        let base_color = |gpu: &mut crate::headless::Headless, program: &CompositeProgram| {
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, program).unwrap();
            let texture = gpu.renderer.display_textures()[0][DisplayKind::BaseColor as usize];
            Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap().2
        };
        let mask_gray = |gpu: &mut crate::headless::Headless, program: &CompositeProgram| {
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, program).unwrap();
            let (_, _, rgba) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.aux_texture(0).unwrap()).unwrap();
            rgba.as_chunks::<4>().0.iter().map(|texel| texel[0]).collect::<Vec<u8>>()
        };

        // The stack below (roughness 0.3) and this layer's roughness (0.8) as the upper layer's mask
        for (kind, value) in [(189, 0.3), (188, 0.8)] {
            let ((project, owner), _) = probe(kind, roughness as f32, 0.0);
            let program = compile(&project, owner, false);
            assert!(program.filters.stages.iter().any(|stage| stage.anchor == Some((Channel::Roughness, u32::MAX, 0))), "probe {kind}: a capture stage");
            let (reference, owner) = build(uniform(value));
            let expected = base_color(&mut gpu, &compile(&reference, owner, false));
            let actual = base_color(&mut gpu, &program);
            assert!(actual.iter().zip(&expected).all(|(a, b)| a.abs_diff(*b) <= 1), "probe {kind}: {:?} vs {:?}", &actual[..4], &expected[..4]);
        }

        // The probe image by nearest texel: 51, 102 on the first row, 204, 153 on the second
        let ((project, owner), _) = probe(190, 0.0, 0.0);
        let gray = mask_gray(&mut gpu, &compile(&project, owner, true));
        let half = SIZE as usize / 2;
        for (x, y, expected) in [(5, 5, 51u8), (half + 5, 5, 102), (5, half + 5, 204), (half + 5, half + 5, 153)] {
            assert_eq!(gray[y * SIZE as usize + x], expected, "image texel at ({x}, {y})");
        }

        // An image input of 0.65
        let ((project, owner), _) = probe(191, 0.0, 0.65);
        let gray = mask_gray(&mut gpu, &compile(&project, owner, true));
        assert!(gray.iter().all(|&value| value.abs_diff(166) <= 1), "{:?}", &gray[..4]);
    }

    /// The P0h hooks reach the shader (probe 191 with a selector, through the mask preview): the
    /// surface tangent equals the CPU g-buffer's at every covered texel of a sphere, a second
    /// stack read gets its own channel, the environment words come from the reader the program
    /// was given, and the bake rows hold the Height below the built-in maps without moving them.
    #[test]
    fn late_bound_hooks_reach_the_shader() {
        use std::sync::Arc;

        use texturecraft_compose::environment_sh::EnvironmentSh;
        use texturecraft_compose::{BakedMaps, ImageStore};
        use texturecraft_core::{BakedMap, Effect};
        use texturecraft_geom::{Mesh, UvGBuffer, primitives};

        use crate::gpu_composite::oracle::headless;
        use crate::readback::Readback;

        let Some(mut gpu) = headless() else { return };
        const SIZE: u32 = 64;
        let channels = [Channel::BaseColor, Channel::Roughness, Channel::Metallic];
        // A lower fill of roughness 0.3 and metallic 0.6, an upper white fill masked by `mask`
        let project_with = |mask: EffectKind| {
            let mut project = Project::new("Probes", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
            let lower = Fill {
                sources: BTreeMap::from([
                    (Channel::BaseColor, Source::Uniform([0.1, 0.4, 0.2])),
                    (Channel::Roughness, Source::Uniform([0.3; 3])),
                    (Channel::Metallic, Source::Uniform([0.6; 3])),
                ]),
                ..Default::default()
            };
            let lower_id = project.alloc_id();
            project.insert_layer(0, Layer::new(lower_id, "Lower", LayerKind::Fill(lower), &channels), None, false).unwrap();
            let owner = project.alloc_id();
            let upper = Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([1.0; 3]))]), ..Default::default() };
            let mut layer = Layer::new(owner, "Upper", LayerKind::Fill(upper), &channels);
            let mut layer_mask = Mask::new(MaskBackground::Black);
            layer_mask.effects.push(Effect::new(project.alloc_id(), mask));
            layer.mask = Some(layer_mask);
            project.insert_layer(0, layer, None, false).unwrap();
            (project, owner)
        };
        // The probe as the upper layer's mask, compiled for the mask preview or the material
        let compile_probe = |selector: u32, min: f32, max: f32, preview: bool| {
            let (project, owner) = project_with(EffectKind::Filter(Filter::Smoothstep { min, max }));
            super::PROBE_LOWERING.set(Some(191));
            super::PROBE_SELECTOR.set(selector);
            let pixels = PixelStore::default();
            let program = if preview {
                CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, owner, None, None)
            } else {
                CompositeProgram::try_compile_region(&project, &pixels, 0, None)
            };
            super::PROBE_LOWERING.set(None);
            super::PROBE_SELECTOR.set(0);
            program.unwrap()
        };
        let build = |selector: u32, min: f32, max: f32| compile_probe(selector, min, max, true);
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        let mask_gray = |gpu: &mut crate::headless::Headless, program: &CompositeProgram| {
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, program).unwrap();
            let (_, _, rgba) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.aux_texture(0).unwrap()).unwrap();
            rgba.as_chunks::<4>().0.iter().map(|texel| texel[0]).collect::<Vec<u8>>()
        };
        let to_gray = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        let inputs = |mesh: &Arc<Mesh>, program: &mut CompositeProgram, baked: Option<&BakedMaps>| {
            let gbuf = Arc::new(UvGBuffer::new(mesh, 0, SIZE, SIZE));
            program.set_surface_inputs(mesh, &gbuf, baked).unwrap();
            program.set_tile_images(&ImageStore::default());
            gbuf
        };

        // The tangent's x, y, z and sign on a sphere, against the CPU surface at each covered texel
        let sphere = Arc::new(primitives::sphere(1.0, 16, 8));
        for component in 0..4 {
            let mut program = build(component as u32 + 1, 0.0, 0.0);
            let gbuf = inputs(&sphere, &mut program, None);
            let gray = mask_gray(&mut gpu, &program);
            let mut checked = 0;
            for (index, &actual) in gray.iter().enumerate() {
                let Some(surface) = gbuf.surface(&sphere, index) else { continue };
                let expected = to_gray(0.5 + 0.5 * surface.tangent[component]);
                assert!(actual.abs_diff(expected) <= 2, "tangent component {component} at texel {index}: {actual} vs {expected} ({:?})", surface.tangent);
                checked += 1;
            }
            assert!(checked > (SIZE * SIZE / 2) as usize, "covered texels: {checked}");
        }

        let plane = Arc::new(primitives::plane(1.0, 1));
        let uniform = |gray: &[u8], expected: u8, what: &str| assert!(gray.iter().all(|&value| value.abs_diff(expected) <= 1), "{what}: {:?}", &gray[..4]);

        // The second stack read: Metallic (0.6) besides Roughness, each in its own capture stage,
        // as the material's mask against a mask of 0.6
        let roughness = super::super::channel_slot(Channel::Roughness);
        let metallic = super::super::channel_slot(Channel::Metallic);
        let mut program = compile_probe(5, roughness as f32, metallic as f32, false);
        let captures: Vec<_> = program.filters.stages.iter().filter_map(|stage| stage.anchor.map(|(channel, _, _)| channel)).collect();
        assert_eq!(captures, [Channel::Roughness, Channel::Metallic]);
        inputs(&plane, &mut program, None);
        let (reference, _) = project_with(EffectKind::Fill {
            source: Source::Uniform([0.6; 3]),
            projection: texturecraft_core::Projection::default(),
            sampling: texturecraft_core::Sampling::SAVED_BEFORE,
        });
        let mut expected = CompositeProgram::try_compile_region(&reference, &PixelStore::default(), 0, None).unwrap();
        inputs(&plane, &mut expected, None);
        let base_color = |gpu: &mut crate::headless::Headless, program: &CompositeProgram| {
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, program).unwrap();
            let texture = gpu.renderer.display_textures()[0][texturecraft_compose::DisplayKind::BaseColor as usize];
            Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap().2
        };
        let expected = base_color(&mut gpu, &expected);
        let actual = base_color(&mut gpu, &program);
        assert!(actual.iter().zip(&expected).all(|(a, b)| a.abs_diff(*b) <= 1), "second stack read: {:?} vs {:?}", &actual[..4], &expected[..4]);

        // Environment words from a stub reader: coefficient word 13 and the rotation (word 27)
        let stub = EnvironmentSh::new(std::array::from_fn(|i| std::array::from_fn(|k| (3 * i + k) as f32 / 40.0)), 0.6);
        for (word, expected) in [(13, 13.0 / 40.0), (27, 0.6)] {
            let mut program = build(6, 0.0, word as f32);
            assert_eq!(program.environment(), Some(("Studio Soft", 0.0)), "the compiled project's environment");
            inputs(&plane, &mut program, None);
            program.set_environment(&stub);
            uniform(&mask_gray(&mut gpu, &program), to_gray(expected), "environment word");
        }

        // The bake rows: the Height (0.4) in the second block, the ambient occlusion (0.7) still
        // where `baked_at` reads it, and 0.25 from the probe without a baked Height
        let texels = (SIZE * SIZE) as usize;
        let mut baked = BakedMaps { resolution: SIZE, ..Default::default() };
        baked.maps.insert(BakedMap::AmbientOcclusion, Arc::new(vec![0.7; texels]));
        let without_height = baked.clone();
        baked.maps.insert(BakedMap::Height, Arc::new(vec![0.4; texels]));
        for (selector, maps, expected) in [(7, &baked, 0.4), (8, &baked, 0.7), (7, &without_height, 0.25), (8, &without_height, 0.7)] {
            let mut program = build(selector, 0.0, 0.0);
            inputs(&plane, &mut program, Some(maps));
            uniform(&mask_gray(&mut gpu, &program), to_gray(expected), &format!("bake rows, selector {selector}"));
        }

        // A 2 × 2 image read bilinearly without wrapping, as the CPU's BilinearSharp single image
        let image = Arc::new(texturecraft_compose::ImageRes::new(2, 2, [51.0, 102.0, 204.0, 153.0].map(|v: f32| [v / 255.0, 0.0, 0.0, 1.0]).to_vec(), false));
        let images = ImageStore::from([("probe".into(), image.clone())]);
        let mut program = build(9, 0.0, 0.0);
        let gbuf = Arc::new(UvGBuffer::new(&plane, 0, SIZE, SIZE));
        program.set_surface_inputs(&plane, &gbuf, None).unwrap();
        program.set_tile_images(&images);
        let gray = mask_gray(&mut gpu, &program);
        let sampling = texturecraft_core::Sampling {
            filtering: texturecraft_core::Filtering::BilinearSharp,
            wrap: texturecraft_core::UvWrap::None,
            ..texturecraft_core::Sampling::SAVED_BEFORE
        };
        for (index, &actual) in gray.iter().enumerate() {
            let (x, y) = (index as u32 % SIZE, index as u32 / SIZE);
            let [red, ..] = image.sample_filtered((x as f32 + 0.5) / SIZE as f32, (y as f32 + 0.5) / SIZE as f32, sampling, 1.0);
            assert!(actual.abs_diff(to_gray(red)) <= 1, "bilinear at ({x}, {y}): {actual} vs {}", to_gray(red));
        }
    }

    /// A mask that reads its own layer (probe 188, `this_layer_content`) comes after that
    /// layer's content stacks; any other mask stays before them.
    #[test]
    fn a_mask_reading_its_layer_follows_the_content_stacks() {
        use texturecraft_core::Effect;

        let stages = |own: bool| {
            let mut project = Project::new("Order", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, 64));
            let owner = project.alloc_id();
            let channels = [Channel::BaseColor, Channel::Roughness];
            let mut layer = Layer::new(owner, "Layer", LayerKind::Fill(Fill::default()), &channels);
            layer.content_effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 4.0, wrap: false })));
            let mut mask = Mask::new(MaskBackground::White);
            if own {
                let slot = super::super::channel_slot(Channel::Roughness) as f32;
                mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Smoothstep { min: slot, max: 0.0 })));
            }
            mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 4.0, wrap: false })));
            layer.mask = Some(mask);
            project.insert_layer(0, layer, None, false).unwrap();
            super::PROBE_LOWERING.set(own.then_some(188));
            let program = CompositeProgram::try_compile_region(&project, &PixelStore::default(), 0, None);
            super::PROBE_LOWERING.set(None);
            let program = program.unwrap();
            let kinds: Vec<bool> =
                program.filters.stages.iter().filter(|stage| stage.filter.is_some()).map(|stage| stage.preceding & super::super::MASK_STAGE != 0).collect();
            let layers = &program.operations[..program.layer_count as usize];
            (kinds, layers.iter().map(|op| op.spatial[..3].to_vec()).collect::<Vec<_>>())
        };
        let (plain, _) = stages(false);
        assert_eq!(plain.first(), Some(&true), "the mask first: {plain:?}");
        let (reading, layers) = stages(true);
        assert_eq!(reading.last(), Some(&true), "the mask last: {reading:?}");
        assert!(reading[..reading.len() - 1].iter().all(|mask| !mask), "{reading:?}");
        assert!(layers.iter().all(|spatial| spatial[1] == 1 && spatial[2] != 0), "{layers:?}");
    }

    /// Programs that do not ask for the bake rows upload the built-in maps alone, and a mask's
    /// PBR Validate lowers to nothing (the CPU leaves the mask as it is), with no stack read.
    #[test]
    fn bake_rows_only_when_asked_and_pbr_validate_in_masks() {
        use std::sync::Arc;

        use texturecraft_compose::BakedMaps;
        use texturecraft_core::{BakedMap, Effect, PbrValidate};

        let mut baked = BakedMaps { resolution: 4, ..Default::default() };
        baked.maps.insert(BakedMap::AmbientOcclusion, Arc::new(vec![0.7; 16]));
        baked.maps.insert(BakedMap::Height, Arc::new(vec![0.4; 16]));
        let plain = super::super::BakedInputs::new(&baked, false).unwrap().unwrap();
        assert!(plain.blocks() == 1 && plain.uniform_words() == [0, 0] && plain.rows.iter().all(Option::is_none));
        let rows = super::super::BakedInputs::new(&baked, true).unwrap().unwrap();
        assert!(rows.blocks() == 2 && rows.uniform_words() == [4, 1]);

        let mut project = Project::new("Validate", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, 64));
        let owner = project.alloc_id();
        let mut layer = Layer::new(owner, "Layer", LayerKind::Fill(Fill::default()), &[Channel::BaseColor, Channel::Metallic]);
        let mut mask = Mask::new(MaskBackground::White);
        mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::PbrValidate(PbrValidate::LIBRARY))));
        layer.mask = Some(mask);
        project.insert_layer(0, layer, None, false).unwrap();
        let pixels = PixelStore::default();
        for program in [
            CompositeProgram::try_compile_region(&project, &pixels, 0, None).unwrap(),
            CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, owner, None, None).unwrap(),
        ] {
            assert!(program.filters.stages.is_empty(), "no stack read: {:?}", program.filters.stages);
            assert!(program.operations.iter().all(|op| !EXTENDED_EFFECTS.contains(&op.ids[1])));
        }
    }

    /// A neighbourhood stage carries its effect's registered image input into the field's sixth
    /// word (P0g): the reference is rebound to the source's operation and the fields grow.
    #[test]
    fn a_neighbourhood_stage_takes_its_registered_input() {
        let mut inputs = super::super::ProceduralInputs::new(64);
        let blur = Filter::Blur { radius: 64.0, wrap: false };
        inputs.filters.push((blur, 1.0));
        let source = inputs.register_input(super::super::Operation { low: [0.4; 4], ..bytemuck::Zeroable::zeroed() });
        let layer = super::super::Operation { ids: [0, 0, 0, 0], effects: [0, 0, 0, 1], ..bytemuck::Zeroable::zeroed() };
        let stage = super::super::Operation { ids: [0, super::super::NEIGHBOURHOOD_OPCODE, 0, 0], effects: [0, 0, 0, source], ..bytemuck::Zeroable::zeroed() };
        let mut operations = vec![layer, stage];
        super::super::bind_input_sources(&mut operations, 1, &inputs.input_sources);
        let plan = super::super::FilterPlan::compile(&mut operations, 1, &inputs);
        assert_eq!(operations.len(), 3);
        assert_eq!(plan.stages[0].input, 3, "the source's operation index plus one");
        assert_eq!(plan.field_words(), crate::gpu_neighbourhood::FIELD_WORDS + 1);
    }

    /// A 4K whole-set stage with a Drop Shadow's reach (563 texels) in four channels plans its
    /// stripes beside the four results within a 2 GiB binding: the stripe fields are clamped to
    /// the set, and stripes shrink before planning fails.
    #[test]
    fn a_large_reach_whole_set_stage_plans_at_4k() {
        use crate::gpu_neighbourhood::WHOLE_SET_TEST_REACH;

        const SIZE: u32 = 4096;
        let channels = [Channel::BaseColor, Channel::Roughness, Channel::Metallic, Channel::Height];
        let mut project = Project::new("Large reach", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
        let owner = project.alloc_id();
        let sources = channels.map(|channel| (channel, Source::Uniform([0.4; 3])));
        let mut layer = Layer::new(owner, "Surface", LayerKind::Fill(Fill { sources: sources.into(), ..Default::default() }), &channels);
        // Three box passes of 188 texels at 4K: a reach of 564
        layer.content_effects.push(effect(EffectKind::Filter(Filter::Blur { radius: 94.0, wrap: false })));
        project.insert_layer(0, layer, None, false).unwrap();
        WHOLE_SET_TEST_REACH.set(Some(563));
        let program = CompositeProgram::try_compile_region(&project, &PixelStore::default(), 0, None);
        WHOLE_SET_TEST_REACH.set(None);
        let program = program.unwrap();
        let whole = program.filters.whole_set().expect("the Blur runs in the pre-pass");
        assert_eq!((whole.outputs.len(), whole.reach), (4, 564));
        let full = super::super::CompositeRegion { min: [0, 0], max: [SIZE, SIZE] };
        let pass = super::super::WholeSetPass::plan(whole, (full, 0), (SIZE, SIZE), program.filters.band_reach(), false, 5, program.filters.stages.len());
        let limit = 2047u64 << 20;
        let results = pass.result_bytes();
        let pass = pass.with_stripes(limit - results).expect("the stripes fit beside the results");
        assert!(pass.stripe_bytes() + results <= limit && pass.stripe_rows >= 1);
        // Every stripe's field stays inside the set
        for stack in &pass.stages.passes {
            let rect = pass.field_rect(stack, 0, pass.stripe_rows);
            assert!(rect[0] == 0 && rect[1] == 0 && rect[2] == SIZE && rect[3] <= SIZE, "{rect:?}");
        }
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
