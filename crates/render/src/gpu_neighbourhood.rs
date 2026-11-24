//! GPU stages for the neighbourhood filters over a premultiplied texel field.
//!
//! The GPU port packages extend these stages from their own files: P4 (box family and distance
//! filters) in `gpu_neighbourhood_box.rs` with kinds 16..32, P5 (gather and whole-set filters)
//! in `gpu_neighbourhood_gather.rs` with kinds 32..56. Package 0 froze this file: every hook
//! below already routes a package's kinds to its module and its WGSL file.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use texturecraft_core::{ChannelKind, Filter, HighpassSpace};
use wgpu::util::DeviceExt;

#[path = "gpu_neighbourhood_box.rs"]
pub(crate) mod box_family;
#[path = "gpu_neighbourhood_gather.rs"]
pub(crate) mod gather;

/// Words of a field texel: premultiplied RGB, material alpha and UV coverage. A program whose
/// stages carry an input (`FilterStage::input`) adds a sixth word to every field; the shaders read
/// the stride from their parameters (`field_layout`), never from a literal.
pub(crate) const FIELD_WORDS: u32 = 5;
const LINE_WORKGROUP: u32 = 64;
const APPLY_WORKGROUP: u32 = 8;
const BOX_PASSES: usize = 3;

/// P4: the box family (wrap, Highpass space and alpha, Pixelate alpha, gradient pre-blur, Glow,
/// Drop Shadow, Mask Outline).
pub(crate) const BOX_KINDS: Range<u32> = 16..24;
/// P4: the distance filters (Bevel, Bevel Smooth, Directional Distance).
pub(crate) const DISTANCE_KINDS: Range<u32> = 24..32;
/// P5: the gather filters (FXAA, Kuwahara, Height To Normal, Warp, Blur Slope, Mirror, Transform).
pub(crate) const GATHER_KINDS: Range<u32> = 32..48;
/// P5: the whole-set filters (Color Match).
pub(crate) const WHOLE_SET_KINDS: Range<u32> = 48..56;

/// The stage a filter runs as; the value is `mode.y` in neighbourhood.wgsl. 0-10 are built in,
/// the packages own the ranges above and define their kinds as constants in their own module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Kind(pub(crate) u32);

impl Kind {
    const BLUR: Self = Self(0);
    const SHARPEN: Self = Self(1);
    const HIGH_PASS: Self = Self(2);
    const BEVEL: Self = Self(3);
    const GLOW: Self = Self(4);
    const DIRECTIONAL_BLUR: Self = Self(5);
    const WARP: Self = Self(6);
    const DIRECTIONAL_WARP: Self = Self(7);
    const EDGE: Self = Self(8);
    const SLOPE_BLUR: Self = Self(9);
    const MOSAIC: Self = Self(10);
    /// Test probe of the stage inputs: the input's colour (`input_color_at`) at the texel.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) const INPUT_PROBE: Self = Self(15);

    fn box_family(self) -> bool {
        BOX_KINDS.contains(&self.0) || DISTANCE_KINDS.contains(&self.0)
    }

    fn gather(self) -> bool {
        GATHER_KINDS.contains(&self.0) || WHOLE_SET_KINDS.contains(&self.0)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NeighbourhoodFilter {
    pub(crate) kind: Kind,
    pub(crate) radius: u32,
    pub(crate) opacity: f32,
    pub(crate) amount: f32,
    pub(crate) frequency: f32,
    pub(crate) middle: f32,
    pub(crate) direction: [f32; 2],
    pub(crate) seed: u32,
    /// The packages' words: `params.field_layout.zw` and `params.extra` in WGSL.
    pub(crate) words: [u32; 2],
    pub(crate) extra: [f32; 8],
    /// What the stage filters ([`stage_context`]): `params.stage.x` in WGSL (`stage_context()`).
    pub(crate) context: u32,
}

#[cfg(all(test, not(target_arch = "wasm32")))]
thread_local! {
    /// Test seam: built-in stages reaching at least this many texels run in the whole-set
    /// pre-pass, so its tests need no package stage.
    pub(crate) static WHOLE_SET_TEST_REACH: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

/// `params.stage.x` of a stage filtering a mask, and of content by the channel's kind.
pub(crate) const STAGE_MASK: u32 = 0;
pub(crate) const STAGE_COLOR: u32 = 1;
pub(crate) const STAGE_SCALAR: u32 = 2;
pub(crate) const STAGE_SIGNED: u32 = 3;
pub(crate) const STAGE_NORMAL_MAP: u32 = 4;

/// The stage context word of `channel` (None: a mask).
pub(crate) fn stage_context(channel: Option<ChannelKind>) -> u32 {
    match channel {
        None => STAGE_MASK,
        Some(ChannelKind::Color) => STAGE_COLOR,
        Some(ChannelKind::Scalar) => STAGE_SCALAR,
        Some(ChannelKind::Signed) => STAGE_SIGNED,
        Some(ChannelKind::NormalMap) => STAGE_NORMAL_MAP,
    }
}

/// What a package's stage constructor gets: the CPU evaluator's working resolution, the longest
/// side, the effect's opacity and what the stage filters: `channel` is the content channel's
/// kind, None for a mask (and for the callers that only ask whether a stage exists).
#[allow(dead_code)] // Read by the GPU port packages
#[derive(Clone, Copy, Debug)]
pub(crate) struct StageInputs {
    pub(crate) resolution: u32,
    pub(crate) longest_side: u32,
    pub(crate) opacity: f32,
    pub(crate) channel: Option<ChannelKind>,
}

#[allow(dead_code)] // Used by the GPU port packages
impl StageInputs {
    /// Whether the stage filters a signed channel (Height): its high-pass middle is 0.
    pub(crate) fn signed(self) -> bool {
        self.channel == Some(ChannelKind::Signed)
    }

    /// Source texels for a size given at 2048 texels, at least half a texel, as the CPU scales it.
    pub(crate) fn texels_at_2k(self, at_2k: f32) -> f32 {
        (at_2k * (self.resolution.max(1) as f32 / 2048.0)).max(0.5)
    }

    /// A stage of `kind` reaching `texels` (rounded, 1 to the longest side) with neutral settings.
    pub(crate) fn filter(self, kind: Kind, texels: f32) -> NeighbourhoodFilter {
        NeighbourhoodFilter {
            kind,
            radius: texels.round().clamp(1.0, self.longest_side.max(1) as f32) as u32,
            opacity: self.opacity.clamp(0.0, 1.0),
            amount: 0.0,
            frequency: 0.0,
            middle: if self.signed() { 0.0 } else { 0.5 },
            direction: [0.0; 2],
            seed: 0,
            words: [0; 2],
            extra: [0.0; 8],
            context: stage_context(self.channel),
        }
    }
}

/// A work budget the kernels run under (`NeighbourhoodGpu::budget`)
struct Budget {
    queue: wgpu::Queue,
    /// The apply pipeline's address: only its dispatch reads its first row from `stage.z`
    apply: usize,
    work_per_texel: u64,
    limit: u64,
}

thread_local! {
    static BUDGET: std::cell::RefCell<Option<Budget>> = const { std::cell::RefCell::new(None) };
}

/// Ends the work budget of `NeighbourhoodGpu::budget` when dropped (also on an early return).
pub(crate) struct BudgetGuard;

impl Drop for BudgetGuard {
    fn drop(&mut self) {
        BUDGET.set(None);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
thread_local! {
    /// Test seam: Height To Normal and Color Match become the input probe stage
    /// (`Kind::INPUT_PROBE`), lowered by `packages::lower_filter`'s seam.
    pub(crate) static INPUT_PROBE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the GPU stages evaluate neighbourhood filter `filter` as the CPU does. Kinds without a
/// stage, and settings the stages do not model, make the caller composite on the CPU.
pub(crate) fn supports(filter: &Filter) -> bool {
    supports_built_in(filter) || box_family::supports(filter) || gather::supports(filter)
}

/// The built-in stages. Edge wrapping, Highpass colour space and alpha, and Mosaic leaving alpha
/// alone are P4's; P4 and P5 take every filter this rejects.
fn supports_built_in(filter: &Filter) -> bool {
    matches!(
        filter,
        Filter::Blur { wrap: false, .. }
            | Filter::Sharpen { .. }
            | Filter::HighPass { space: HighpassSpace::Working, alpha: false, .. }
            | Filter::Bevel { wrap: false, .. }
            | Filter::Glow { .. }
            | Filter::DirectionalBlur { .. }
            | Filter::Warp { .. }
            | Filter::DirectionalWarp { .. }
            | Filter::EdgeDetect { .. }
            | Filter::SlopeBlur { .. }
            | Filter::Mosaic { alpha: true, .. }
    )
}

impl NeighbourhoodFilter {
    /// The source-pixel parameters used by the CPU evaluator at the same working resolution.
    /// A nonpositive radius for an optional effect leaves the incoming field unchanged.
    /// `channel` is what the stage filters (None: a mask, or a caller that only asks whether a
    /// stage exists). None for a filter no stage evaluates. The packages' `new` may also build a
    /// stage for a filter their `supports` rejects: one a lowering queues with
    /// `ProceduralInputs::precede_with_stage` (the Gradient's pre-blur), which never becomes an
    /// effect's own stage.
    pub(crate) fn new(filter: &Filter, resolution: u32, longest_side: u32, opacity: f32, channel: Option<ChannelKind>) -> Option<Self> {
        let signed = channel == Some(ChannelKind::Signed);
        #[cfg(all(test, not(target_arch = "wasm32")))]
        if INPUT_PROBE.get() && matches!(filter, Filter::HeightToNormal(_) | Filter::ColorMatch(_)) {
            let stage = StageInputs { resolution, longest_side, opacity, channel };
            return Some(stage.filter(Kind::INPUT_PROBE, 1.0));
        }
        if !supports_built_in(filter) {
            let stage = StageInputs { resolution, longest_side, opacity, channel };
            return box_family::new(filter, stage).or_else(|| gather::new(filter, stage));
        }
        let scale = resolution.max(1) as f32 / 2048.0;
        let radius = |at_2k: f32| (at_2k * scale).max(0.5);
        let mut out = Self {
            kind: Kind::BLUR,
            radius: 1,
            opacity: opacity.clamp(0.0, 1.0),
            amount: 0.0,
            frequency: 0.0,
            middle: if signed { 0.0 } else { 0.5 },
            direction: [0.0; 2],
            seed: 0,
            words: [0; 2],
            extra: [0.0; 8],
            context: stage_context(channel),
        };
        let direction = |degrees: f32| {
            let (sin, cos) = degrees.to_radians().sin_cos();
            [cos, -sin]
        };
        let noise_frequency = |scale: f32| (if scale.is_finite() { scale.clamp(0.01, 1000.0) } else { 1.0 }) / resolution.max(1) as f32;
        // A size over its intensity divider (a divider of 0 counts as 1), as the CPU reads it
        let divided = |size: f32, divider: u16| size / f32::from(divider.max(1));

        let (kind, texels) = match *filter {
            Filter::Blur { radius: r, .. } if r > 0.0 => (Kind::BLUR, radius(r)),
            Filter::Sharpen { amount } => {
                out.amount = amount;
                (Kind::SHARPEN, (1.5 * scale).max(0.75))
            }
            Filter::HighPass { radius: r, .. } => (Kind::HIGH_PASS, radius(r)),
            Filter::Bevel { width, .. } => (Kind::BEVEL, radius(width * 0.5)),
            Filter::Glow { radius: r, amount } => {
                out.amount = amount;
                (Kind::GLOW, radius(r))
            }
            Filter::DirectionalBlur { radius: r, angle } if r > 0.0 => {
                out.direction = direction(angle);
                (Kind::DIRECTIONAL_BLUR, radius(r))
            }
            Filter::Warp { intensity, scale, seed, divider } if intensity > 0.0 => {
                out.frequency = noise_frequency(scale);
                out.seed = seed;
                (Kind::WARP, radius(divided(intensity, divider)))
            }
            Filter::DirectionalWarp { intensity, angle, scale, seed, divider } if intensity > 0.0 => {
                out.frequency = noise_frequency(scale);
                out.direction = direction(angle);
                out.seed = seed;
                (Kind::DIRECTIONAL_WARP, radius(divided(intensity, divider)))
            }
            Filter::EdgeDetect { width } => (Kind::EDGE, radius(width)),
            Filter::SlopeBlur { radius: r, scale, seed, divider } if r > 0.0 => {
                out.frequency = noise_frequency(scale);
                out.seed = seed;
                (Kind::SLOPE_BLUR, radius(divided(r, divider)))
            }
            Filter::Mosaic { size, .. } => (Kind::MOSAIC, radius(size)),
            _ => return None,
        };
        out.kind = kind;
        out.radius = texels.round().clamp(1.0, longest_side.max(1) as f32) as u32;
        Some(out)
    }

    /// Whether the stage runs in the whole-set pre-pass (once per update over the region the
    /// bands read, in stripes) instead of in every band: a package's stage with a large reach
    /// (P4: Bevel, Drop Shadow) or one that needs the whole set (P5: Color Match, Mirror,
    /// Transform). `resolution` is the set's working resolution.
    pub(crate) fn whole_set(self, resolution: u32) -> bool {
        #[cfg(all(test, not(target_arch = "wasm32")))]
        if WHOLE_SET_TEST_REACH.get().is_some_and(|reach| self.reach() >= reach) {
            return true;
        }
        if self.kind.box_family() {
            return box_family::whole_set(self, resolution);
        }
        if self.kind.gather() {
            return gather::whole_set(self, resolution);
        }
        false
    }

    /// Source texels the stage reads around an output texel.
    pub(crate) fn reach(self) -> u32 {
        if self.kind.box_family() {
            return box_family::reach(self);
        }
        if self.kind.gather() {
            return gather::reach(self);
        }
        if self.uses_box() { self.radius.saturating_mul(BOX_PASSES as u32) } else { self.radius.saturating_add(1) }
    }

    /// Whether the stage runs the three box passes before its apply pass (`uses_box_kind` in
    /// neighbourhood.wgsl must agree).
    /// The working fields its passes use besides the original and the output
    /// (`NeighbourhoodRegion::blur`): two for the box passes and every package kind (P4's
    /// distance passes, P5's frames and reductions; P5 declares fewer at its merge), none for a
    /// built-in gather. The whole-set pre-pass gives a stage only these.
    pub(crate) fn blur_fields(self) -> u32 {
        if self.kind.box_family() || self.kind.gather() || self.uses_box() { 2 } else { 0 }
    }

    pub(crate) fn uses_box(self) -> bool {
        if self.kind.box_family() {
            return box_family::uses_box(self);
        }
        if self.kind.gather() {
            return gather::uses_box(self);
        }
        matches!(self.kind, Kind::BLUR | Kind::SHARPEN | Kind::HIGH_PASS | Kind::BEVEL | Kind::GLOW)
    }

    pub(crate) fn work_per_texel(self) -> u64 {
        if self.kind.box_family() {
            return box_family::work_per_texel(self);
        }
        if self.kind.gather() {
            return gather::work_per_texel(self);
        }
        let samples = u64::from(self.radius.min(24));
        match self.kind {
            Kind::DIRECTIONAL_BLUR => 1 + 4 * (2 * samples + 1),
            Kind::SLOPE_BLUR => 5 + 4 * (samples + 1),
            Kind::EDGE => 17,
            Kind::WARP | Kind::DIRECTIONAL_WARP => 9,
            Kind::MOSAIC => 5,
            _ => 8,
        }
    }
}

/// Source-pixel region [left, top, width, height], with disjoint field offsets in u32 words.
#[derive(Clone, Copy)]
pub(crate) struct NeighbourhoodRegion {
    pub(crate) rect: [u32; 4],
    pub(crate) original: u32,
    pub(crate) blur: [u32; 2],
    pub(crate) output: u32,
    /// Words per field texel ([`FIELD_WORDS`], or one more with the input word).
    pub(crate) field_words: u32,
    /// Bit 0: the original field carries input words; bit 1: this stage has an input of its own
    /// (`stage_linked` in WGSL); bit 2: the fields carry the input's colour in words 5..8
    /// (`input_color_at`).
    pub(crate) input: u32,
    /// The stage wraps: it reads the field's periodic copies past the set's edges.
    pub(crate) wrap: bool,
    /// The set in field-local texels [x0, y0, x1, y1]: in a wrapping program the field holds
    /// periodic copies of the set past its edges, and a stage that does not wrap never reads
    /// across these bounds (`bounds` and the segment checks in neighbourhood.wgsl). Without a
    /// wrapping stage the field lies inside the set and the bounds reach past it.
    pub(crate) bounds: [i32; 4],
}

impl NeighbourhoodRegion {
    fn field_layout(self, words: [u32; 2]) -> [u32; 4] {
        [self.field_words, self.input, words[0], words[1]]
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    rect: [u32; 4],
    buffers: [u32; 4],
    mode: [u32; 4],
    weights: [f32; 4],
    direction: [f32; 4],
    field_layout: [u32; 4],
    extra: [f32; 8],
    stage: [u32; 4],
    bounds: [i32; 4],
}

pub(crate) struct NeighbourhoodGpu {
    box_blur: wgpu::ComputePipeline,
    apply: wgpu::ComputePipeline,
    cell_average: wgpu::ComputePipeline,
    cell_interpolate: wgpu::ComputePipeline,
    copy: wgpu::ComputePipeline,
    // The packages' own passes (distance transforms, structure tensors, reductions)
    box_family: box_family::Pipelines,
    gather: gather::Pipelines,
}

impl NeighbourhoodGpu {
    /// The neighbourhood shader with the packages' files.
    pub(crate) fn source() -> String {
        [include_str!("shaders/noise.wgsl"), include_str!("shaders/neighbourhood.wgsl"), box_family::WGSL, gather::WGSL].join("\n")
    }

    /// The entry points of the built-in passes (the packages add theirs in their `Pipelines`).
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) const ENTRY_POINTS: [&str; 5] = ["box_blur", "apply_neighbourhood", "average_cells", "interpolate_cells", "copy_field"];

    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let module = device
            .create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("tc_neighbourhood"), source: wgpu::ShaderSource::Wgsl(Self::source().into()) });
        let pipeline = |entry: &'static str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            box_blur: pipeline("box_blur"),
            apply: pipeline("apply_neighbourhood"),
            cell_average: pipeline("average_cells"),
            cell_interpolate: pipeline("interpolate_cells"),
            copy: pipeline("copy_field"),
            box_family: box_family::Pipelines::new(&pipeline),
            gather: gather::Pipelines::new(&pipeline),
        }
    }

    pub(crate) fn encode_cells(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scratch: &wgpu::Buffer,
        region: NeighbourhoodRegion,
        cell: u32,
    ) {
        let [left, top, width, height] = region.rect;

        // The field's origin is i32 bits and lies below zero in a program with a wrapping stage
        let cells_across = |origin: u32, size: u32| {
            let (origin, cell) = (i64::from(origin as i32), i64::from(cell.max(1)));
            ((origin + i64::from(size) + cell - 1).div_euclid(cell) - origin.div_euclid(cell)) as u32
        };
        let cells = [cells_across(left, width), cells_across(top, height)];
        let params = Params {
            rect: region.rect,
            buffers: [region.original, region.blur[0], region.output, 0],
            mode: [cell, 0, 0, 0],
            field_layout: region.field_layout([0; 2]),
            bounds: region.bounds,
            ..Params::zeroed()
        };
        Self::run(&self.cell_average, device, encoder, scratch, &params, cells.map(|side| side.div_ceil(APPLY_WORKGROUP)));
        Self::run(&self.cell_interpolate, device, encoder, scratch, &params, [width.div_ceil(APPLY_WORKGROUP), height.div_ceil(APPLY_WORKGROUP)]);
    }

    /// Copy `size` texels of the field at word `field` (`field_width` texels wide), from field
    /// texel `from`, into rows `row..` of the whole-set result at word `result` (`result_width`
    /// texels wide).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_copy(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scratch: &wgpu::Buffer,
        (field, field_width, from): (u32, u32, [u32; 2]),
        (result, result_width, row): (u32, u32, u32),
        size: [u32; 2],
        field_words: u32,
    ) {
        let params = Params {
            rect: [from[0], from[1], field_width, 0],
            buffers: [field, 0, result, 0],
            mode: [result_width, size[0], size[1], row],
            field_layout: [field_words, 0, 0, 0],
            ..Params::zeroed()
        };
        Self::run(&self.copy, device, encoder, scratch, &params, [size[0].div_ceil(APPLY_WORKGROUP), size[1].div_ceil(APPLY_WORKGROUP)]);
    }

    /// Run the kernels encoded until the guard drops under a work budget: an apply pass (the
    /// gathers' heavy part) whose texels times `work_per_texel` exceed `limit` is dispatched in
    /// row chunks, each submitted on its own (the whole-set pre-pass, whose fields span the set).
    /// The packages' own passes run whole.
    pub(crate) fn budget(&self, queue: &wgpu::Queue, work_per_texel: u64, limit: u64) -> BudgetGuard {
        BUDGET.set(Some(Budget { queue: queue.clone(), apply: std::ptr::from_ref(&self.apply) as usize, work_per_texel, limit }));
        BudgetGuard
    }

    fn run(
        pipeline: &wgpu::ComputePipeline,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scratch: &wgpu::Buffer,
        params: &Params,
        groups: [u32; 2],
    ) {
        let budget = BUDGET.with_borrow(|budget| {
            budget
                .as_ref()
                .filter(|budget| budget.apply == std::ptr::from_ref(pipeline) as usize)
                .map(|budget| (budget.queue.clone(), budget.work_per_texel, budget.limit))
        });
        if let Some((queue, work_per_texel, limit)) = budget {
            let row_work = u64::from(groups[0]) * u64::from(APPLY_WORKGROUP * APPLY_WORKGROUP) * work_per_texel.max(1);
            let rows = (limit / row_work.max(1)).clamp(1, u64::from(groups[1].max(1))) as u32;
            if rows < groups[1] {
                let mut first = 0;
                while first < groups[1] {
                    let count = rows.min(groups[1] - first);
                    let mut chunk = *params;
                    chunk.stage[2] = first * APPLY_WORKGROUP;
                    Self::dispatch(pipeline, device, encoder, scratch, &chunk, [groups[0], count]);
                    first += count;
                    let finished =
                        std::mem::replace(encoder, device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_neighbourhood") }));
                    queue.submit([finished.finish()]);
                }
                return;
            }
        }
        Self::dispatch(pipeline, device, encoder, scratch, params, groups);
    }

    fn dispatch(
        pipeline: &wgpu::ComputePipeline,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scratch: &wgpu::Buffer,
        params: &Params,
        groups: [u32; 2],
    ) {
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_neighbourhood_params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tc_neighbourhood_inputs"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: scratch.as_entire_binding() },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        pass.dispatch_workgroups(groups[0], groups[1], 1);
    }

    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scratch: &wgpu::Buffer,
        region: NeighbourhoodRegion,
        filter: NeighbourhoodFilter,
    ) {
        let [_, _, width, height] = region.rect;
        if width == 0 || height == 0 {
            return;
        }
        // A package encodes its kinds itself when they need passes of their own
        if (filter.kind.box_family() && box_family::encode((self, &self.box_family), device, encoder, scratch, region, filter))
            || (filter.kind.gather() && gather::encode((self, &self.gather), device, encoder, scratch, region, filter))
        {
            return;
        }
        let mut run = |pipeline: &wgpu::ComputePipeline, buffers: [u32; 4], first_box: bool, groups: [u32; 2]| {
            let params = Params {
                rect: region.rect,
                buffers,
                mode: [filter.radius, filter.kind.0, filter.seed, u32::from(first_box)],
                weights: [filter.opacity, filter.amount, filter.frequency, filter.middle],
                direction: [filter.direction[0], filter.direction[1], 0.0, 0.0],
                field_layout: region.field_layout(filter.words),
                extra: filter.extra,
                stage: [filter.context, u32::from(region.wrap), 0, 0],
                bounds: region.bounds,
            };
            Self::run(pipeline, device, encoder, scratch, &params, groups);
        };
        let mut source = region.original;
        if filter.uses_box() {
            for step in 0..2 * BOX_PASSES {
                let vertical = step >= BOX_PASSES;
                let line_count = if vertical { width } else { height };
                let target = region.blur[step % 2];
                run(&self.box_blur, [region.original, source, target, u32::from(vertical)], step == 0, [line_count.div_ceil(LINE_WORKGROUP), 1]);
                source = target;
            }
        }
        run(&self.apply, [region.original, source, region.output, 0], false, [width.div_ceil(APPLY_WORKGROUP), height.div_ceil(APPLY_WORKGROUP)]);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::sync::Arc;

    use texturecraft_compose::{BakedMaps, ImageStore, PixelStore};
    use texturecraft_core::{Channel, Effect, EffectKind, Filter, Layer, LayerKind, Mask, MaskBackground, Project, TextureSet};
    use texturecraft_geom::primitives;
    use texturecraft_texel::{TileImage, to_u16};

    use crate::gpu_composite::CompositeRegion;
    use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu, headless};

    const SIDE: u32 = 128;

    /// The material's display map `kind` composited on the GPU.
    #[allow(clippy::unwrap_used)] // A test helper: a project that does not build fails the test
    fn display(
        gpu: &mut crate::headless::Headless,
        project: &Project,
        mesh: &Arc<texturecraft_geom::Mesh>,
        kind: texturecraft_compose::DisplayKind,
    ) -> Vec<u8> {
        use crate::gpu_composite::CompositeProgram;
        let program = CompositeProgram::try_compile_region(project, &PixelStore::default(), 0, None);
        super::INPUT_PROBE.set(false);
        let mut program = program.unwrap();
        let gbuf = Arc::new(texturecraft_geom::UvGBuffer::new(mesh, 0, 64, 64));
        program.set_surface_inputs(mesh, &gbuf, None).unwrap();
        program.set_tile_images(&ImageStore::default());
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((64, 64), 0)], 1);
        gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
        let texture = gpu.renderer.display_textures()[0][kind as usize];
        crate::readback::Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap().2
    }

    /// A fill layer of `sources` with `content` effects on each channel's content.
    #[allow(clippy::unwrap_used)] // A test helper: a project that does not build fails the test
    fn fill_layer(sources: &[(Channel, [f32; 3])], content: &[(Channel, Filter)]) -> (Project, texturecraft_core::LayerId) {
        use std::collections::BTreeMap;

        use texturecraft_core::{Fill, Source};
        let channels: Vec<Channel> = sources.iter().map(|(channel, _)| *channel).collect();
        let mut project = Project::new("Inputs", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, 64));
        let owner = project.alloc_id();
        let fill =
            Fill { sources: sources.iter().map(|&(channel, value)| (channel, Source::Uniform(value))).collect::<BTreeMap<_, _>>(), ..Default::default() };
        let mut layer = Layer::new(owner, "Fill", LayerKind::Fill(fill), &channels);
        for (channel, filter) in content {
            let mut added = Effect::new(project.alloc_id(), EffectKind::Filter(filter.clone()));
            added.off_channels = channels.iter().copied().filter(|other| other != channel).collect();
            layer.content_effects.push(added);
        }
        project.insert_layer(0, layer, None, false).unwrap();
        (project, owner)
    }

    /// The whole-set pre-pass in a wrapping program matches the CPU: a wrapping stage below the
    /// whole-set stage, a stage that does not wrap below that, a wrapping stage above it, and a
    /// gather in row chunks of a tiny work budget. Each stack runs as its own pass, whose fields
    /// reach past the set only by its stages at and below its topmost wrapping stage.
    #[test]
    fn whole_set_passes_in_a_wrapping_program_match_the_cpu() {
        use super::WHOLE_SET_TEST_REACH;
        use crate::gpu_composite::{CompositeProgram, PRE_PASS_CHUNK_WINDOWS, PRE_PASS_TEST_BUDGET, PRE_PASS_TEST_PAINT_WORDS};

        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let (baked, images) = (BakedMaps::default(), ImageStore::default());
        let reach = |filter: &Filter| super::NeighbourhoodFilter::new(filter, SIDE, SIDE, 1.0, None).unwrap().reach();
        let wide = Filter::Blur { radius: 60.0, wrap: false };
        let wrap = Filter::Blur { radius: 4.0, wrap: true };
        let small = Filter::Blur { radius: 3.0, wrap: false };
        let warp = Filter::Warp { intensity: 40.0, scale: 3.0, seed: 5, divider: 1 };
        let cases = [
            ("a wrapping stage below", vec![wrap.clone(), wide.clone()], reach(&wide), reach(&wrap), None),
            ("a stage below a wrapping stage below", vec![small.clone(), wrap.clone(), wide.clone()], reach(&wide), reach(&small) + reach(&wrap), None),
            ("a wrapping stage above", vec![wide.clone(), wrap.clone()], reach(&wide), 0, None),
            ("a gather in row chunks", vec![warp.clone(), wrap.clone()], reach(&warp), 0, Some(4096)),
            ("paint in windows per chunk", vec![wrap.clone(), wide.clone()], reach(&wide), reach(&wrap), None),
        ];
        for (label, stack, whole_reach, past_edges, budget) in cases {
            let (project, pixels, owner) = striped(&stack);
            WHOLE_SET_TEST_REACH.set(Some(whole_reach));
            PRE_PASS_TEST_BUDGET.set(budget);
            // The whole set's paint (2 × 2 tiles of three images) does not fit, a tile row does
            let windows = label.starts_with("paint");
            PRE_PASS_TEST_PAINT_WORDS.set(windows.then_some(40_000));
            PRE_PASS_CHUNK_WINDOWS.set(0);
            let program = CompositeProgram::try_compile_region(&project, &pixels, 0, None).unwrap();
            let whole = program.whole_set_passes();
            assert!(!whole.is_empty() && whole.iter().all(|&(_, past)| past == past_edges), "{label}: {whole:?}, past the edges {past_edges}");
            let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
            case.mask_owner = Some(owner);
            case.region = Some(CompositeRegion { min: [0, 0], max: [16, 16] });
            assert_matches_cpu(&mut gpu, &case, Tolerance::TRANSCENDENTAL, label);
            assert_eq!(PRE_PASS_CHUNK_WINDOWS.get() > 0, windows, "{label}: paint windows per chunk");
            WHOLE_SET_TEST_REACH.set(None);
            PRE_PASS_TEST_BUDGET.set(None);
            PRE_PASS_TEST_PAINT_WORDS.set(None);
        }
    }

    /// The whole-set pre-pass of large programs plans and runs: at 4K one painted layer over a
    /// six-channel material with a whole-tile stage (a Blur reaching across the set stands in for
    /// Mirror) on Base Color, Roughness, Height and its mask, its paint too large for one upload;
    /// at 2K the same below a wrapping Blur. Prints the working and paint bytes of each update.
    #[test]
    #[ignore = "heavy: 2K and 4K whole-set programs on the GPU"]
    #[allow(clippy::unwrap_used)] // A heavy check: a program that does not run fails it
    fn whole_set_programs_run_at_2k_and_4k() {
        use super::WHOLE_SET_TEST_REACH;
        use crate::gpu_composite::CompositeProgram;

        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let channels = [Channel::BaseColor, Channel::Metallic, Channel::Roughness, Channel::Normal, Channel::Height, Channel::AmbientOcclusion];
        for (side, wrapping) in [(4096u32, false), (2048, true)] {
            // A built-in gather reaching across the set, standing in for Mirror
            let wide = Filter::Warp { intensity: 4096.0, scale: 3.0, seed: 9, divider: 1 };
            let reach = super::NeighbourhoodFilter::new(&wide, side, side, 1.0, None).unwrap().reach();
            let mut project = Project::new("Large", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, side));
            let set = &mut project.texture_sets[0];
            set.channels = channels.iter().map(|&channel| (channel, texturecraft_core::ChannelFormat::default_for(channel))).collect();
            // A full material below, the painted layer with Base Color, Roughness and Height
            let base = project.alloc_id();
            project.insert_layer(0, Layer::new(base, "Material", LayerKind::Fill(texturecraft_core::Fill::default()), &channels), None, false).unwrap();
            let painted = [Channel::BaseColor, Channel::Roughness, Channel::Height];
            let owner = project.alloc_id();
            let mut layer = Layer::new(owner, "Paint", LayerKind::Paint, &painted);
            let mut mask = Mask::new(MaskBackground::Black);
            let mask_paint = project.alloc_id();
            mask.effects.push(Effect::new(mask_paint, EffectKind::Paint));
            let mut stack = Vec::new();
            if wrapping {
                stack.push(Filter::Blur { radius: 8.0, wrap: true });
            }
            stack.push(wide);
            for filter in &stack {
                layer.content_effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(filter.clone())));
                mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(filter.clone())));
            }
            layer.mask = Some(mask);
            project.insert_layer(0, layer, None, false).unwrap();
            let mut pixels = PixelStore::default();
            for &channel in &painted {
                let image = pixels.layer_mut(owner, channel, (side, side));
                let comps = channel.components();
                for y in 0..side {
                    for x in 0..side {
                        let mut texel = [u16::MAX; 4];
                        texel[0] = to_u16(((x ^ y) % 97) as f32 / 97.0);
                        image.set(x, y, &texel[..comps + 1]);
                    }
                }
            }
            let mut mask_pixels = TileImage::new(side, side, 2);
            for y in 0..side {
                for x in 0..side {
                    mask_pixels.set(x, y, &[to_u16(((x + y) % 13) as f32 / 13.0), u16::MAX]);
                }
            }
            pixels.masks.insert(mask_paint, mask_pixels);
            WHOLE_SET_TEST_REACH.set(Some(reach));
            let program = CompositeProgram::try_compile_region(&project, &pixels, 0, None);
            WHOLE_SET_TEST_REACH.set(None);
            let mut program = program.unwrap();
            assert_eq!(program.whole_set_passes().len(), 4, "{side}: a pass per stack");
            let gbuf = Arc::new(texturecraft_geom::UvGBuffer::new(&mesh, 0, side, side));
            program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
            program.set_tile_images(&ImageStore::default());
            gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((side, side), 0)], 1);
            let started = std::time::Instant::now();
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
            gpu.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
            let [working, paint] = gpu.renderer.composite_update_bytes();
            let limits = gpu.device.limits();
            eprintln!(
                "{side} wrapping {wrapping}: working {} MiB, paint upload {} MiB, binding limit {} MiB, {:?}",
                working >> 20,
                paint >> 20,
                limits.max_storage_buffer_binding_size.min(limits.max_buffer_size) >> 20,
                started.elapsed()
            );
        }
    }

    /// A Height To Normal stage on a Normal operation reads its layer's own Height content (times
    /// its coverage) as its input, prepared after the Height's own stages; without a Height
    /// operation it keeps its effect's input. A colour input reaches the kernel as colour.
    #[test]
    fn stage_inputs_reach_the_kernels() {
        use texturecraft_compose::DisplayKind;
        use texturecraft_core::{ColorMatch, HeightToNormal};

        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let to_normal = Filter::HeightToNormal(HeightToNormal::LIBRARY);
        let blur = Filter::Blur { radius: 30.0, wrap: false };
        let compare = |gpu: &mut crate::headless::Headless, probed: &Project, reference: &Project, kind: DisplayKind, label: &str| {
            super::INPUT_PROBE.set(true);
            let actual = display(gpu, probed, &mesh, kind);
            let expected = display(gpu, reference, &mesh, kind);
            assert!(actual.iter().zip(&expected).all(|(a, b)| a.abs_diff(*b) <= 1), "{label}: {:?} vs {:?}", &actual[..8], &expected[..8]);
        };

        // The layer's blurred Height (0.3 everywhere) as the Normal stage's input
        let (probed, _) =
            fill_layer(&[(Channel::Normal, [0.5, 0.5, 1.0]), (Channel::Height, [0.3; 3])], &[(Channel::Height, blur), (Channel::Normal, to_normal.clone())]);
        let (reference, _) = fill_layer(&[(Channel::Normal, [0.3; 3]), (Channel::Height, [0.3; 3])], &[]);
        super::INPUT_PROBE.set(true);
        let program = crate::gpu_composite::CompositeProgram::try_compile_region(&probed, &PixelStore::default(), 0, None).unwrap();
        super::INPUT_PROBE.set(false);
        let (height, normal) = program.input_stages();
        assert!(height < normal, "the Height's stage first: {height} {normal}");
        compare(&mut gpu, &probed, &reference, DisplayKind::Normal, "Height input");

        // No Height operation: the stage keeps its effect's (absent) input
        let (alone, _) = fill_layer(&[(Channel::Normal, [0.5, 0.5, 1.0])], &[(Channel::Normal, to_normal)]);
        super::INPUT_PROBE.set(true);
        let program = crate::gpu_composite::CompositeProgram::try_compile_region(&alone, &PixelStore::default(), 0, None).unwrap();
        super::INPUT_PROBE.set(false);
        assert_eq!(program.input_of_last_stage(), (0, 0));

        // A colour input
        let source_color = [0.2, 0.5, 0.7];
        let (probed, _) =
            fill_layer(&[(Channel::BaseColor, [0.9; 3])], &[(Channel::BaseColor, Filter::ColorMatch(ColorMatch { source_color, ..ColorMatch::LIBRARY }))]);
        let (reference, _) = fill_layer(&[(Channel::BaseColor, source_color)], &[]);
        compare(&mut gpu, &probed, &reference, DisplayKind::BaseColor, "colour input");
    }

    /// A paint layer of Base Color and Roughness in stripes and steps that run across the set's
    /// edges, its mask painted alike, and `stack` after both.
    #[allow(clippy::unwrap_used)] // A test helper: a project that does not build fails the test
    fn striped(stack: &[Filter]) -> (Project, PixelStore, texturecraft_core::LayerId) {
        let channels = [Channel::BaseColor, Channel::Roughness];
        let mut project = Project::new("Wrapping", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIDE));
        let owner = project.alloc_id();
        let mut layer = Layer::new(owner, "Paint", LayerKind::Paint, &channels);
        let mut mask = Mask::new(MaskBackground::Black);
        let mask_paint = project.alloc_id();
        mask.effects.push(Effect::new(mask_paint, EffectKind::Paint));
        for effects in [&mut layer.content_effects, &mut mask.effects] {
            for filter in stack {
                effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(filter.clone())));
            }
        }
        layer.mask = Some(mask);
        project.insert_layer(0, layer, None, false).unwrap();
        let size = (SIDE, SIDE);
        let mut pixels = PixelStore::default();
        let mut mask_pixels = TileImage::new(SIDE, SIDE, 2);
        for y in 0..SIDE {
            for x in 0..SIDE {
                let tone = if (x / 9 + y / 13) % 2 == 0 { 0.85 } else { 0.2 } * if (5..SIDE - 7).contains(&x) { 1.0 } else { 0.5 };
                let other = (x as f32 / SIDE as f32 + 0.3 * (y % 17) as f32 / 17.0).fract();
                pixels.layer_mut(owner, Channel::BaseColor, size).set(x, y, &[to_u16(tone), to_u16(other), to_u16(0.5 * tone), u16::MAX]);
                pixels.layer_mut(owner, Channel::Roughness, size).set(x, y, &[to_u16(other), u16::MAX]);
                mask_pixels.set(x, y, &[to_u16(tone), u16::MAX]);
            }
        }
        pixels.masks.insert(mask_paint, mask_pixels);
        (project, pixels, owner)
    }

    /// Warp, Directional Warp, Slope Blur and Pixelate (built-in kinds 6, 7, 9 and 10) below a
    /// Blur with Wrap match the CPU, whose wrapped Blur reads the lower stage at the texel's home
    /// in the set: the noise and the cells follow the home position in every periodic copy.
    #[test]
    fn built_in_gathers_below_a_wrapping_blur_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let (baked, images) = (BakedMaps::default(), ImageStore::default());
        let wrap = Filter::Blur { radius: 2.0, wrap: true };
        let cases = [
            ("Warp", Filter::Warp { intensity: 6.0, scale: 3.0, seed: 7, divider: 1 }),
            ("Directional Warp", Filter::DirectionalWarp { intensity: 6.0, angle: 30.0, scale: 3.0, seed: 3, divider: 1 }),
            ("Slope Blur", Filter::SlopeBlur { radius: 2.0, scale: 2.0, seed: 4, divider: 1 }),
            ("Pixelate", Filter::Mosaic { size: 24.0, alpha: true }),
        ];
        // llvmpipe's noise turns Slope Blur's direction (a finite difference of 0.02) at a few
        // texels inside the set, where the stripes' hard edges make that a large step; the other
        // backends check its copies
        let gl = gpu.device.adapter_info().backend == wgpu::Backend::Gl;
        for (label, filter) in cases {
            if gl && label == "Slope Blur" {
                continue;
            }
            let (project, pixels, owner) = striped(&[filter, wrap.clone()]);
            let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
            case.mask_owner = Some(owner);
            case.region = Some(CompositeRegion { min: [0, 0], max: [16, 16] });
            assert_matches_cpu(&mut gpu, &case, Tolerance::TRANSCENDENTAL, &format!("{label} below a wrapping Blur"));
        }
    }
}
