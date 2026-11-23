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
    /// 1 when the original field carries the input word.
    pub(crate) input: u32,
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
}

pub(crate) struct NeighbourhoodGpu {
    box_blur: wgpu::ComputePipeline,
    apply: wgpu::ComputePipeline,
    cell_average: wgpu::ComputePipeline,
    cell_interpolate: wgpu::ComputePipeline,
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
    pub(crate) const ENTRY_POINTS: [&str; 4] = ["box_blur", "apply_neighbourhood", "average_cells", "interpolate_cells"];

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
        let cells = [(left + width).div_ceil(cell) - left / cell, (top + height).div_ceil(cell) - top / cell];
        let params = Params {
            rect: region.rect,
            buffers: [region.original, region.blur[0], region.output, 0],
            mode: [cell, 0, 0, 0],
            field_layout: region.field_layout([0; 2]),
            ..Params::zeroed()
        };
        Self::run(&self.cell_average, device, encoder, scratch, &params, cells.map(|side| side.div_ceil(APPLY_WORKGROUP)));
        Self::run(&self.cell_interpolate, device, encoder, scratch, &params, [width.div_ceil(APPLY_WORKGROUP), height.div_ceil(APPLY_WORKGROUP)]);
    }

    fn run(
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
                stage: [filter.context, 0, 0, 0],
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
