//! GPU stages for the neighbourhood filters over a premultiplied texel field.

use bytemuck::{Pod, Zeroable};
use texturecraft_core::{Filter, HighpassSpace};
use wgpu::util::DeviceExt;

pub(crate) const FIELD_WORDS: u32 = 5;
const LINE_WORKGROUP: u32 = 64;
const APPLY_WORKGROUP: u32 = 8;
const BOX_PASSES: usize = 3;

#[repr(u32)]
#[derive(Clone, Copy, Debug)]
enum Kind {
    Blur = 0,
    Sharpen = 1,
    HighPass = 2,
    Bevel = 3,
    Glow = 4,
    DirectionalBlur = 5,
    Warp = 6,
    DirectionalWarp = 7,
    Edge = 8,
    SlopeBlur = 9,
    Mosaic = 10,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NeighbourhoodFilter {
    kind: Kind,
    radius: u32,
    opacity: f32,
    amount: f32,
    frequency: f32,
    middle: f32,
    direction: [f32; 2],
    seed: u32,
}

/// Whether the GPU stages evaluate neighbourhood filter `filter` as the CPU does. Kinds without a
/// stage, and settings the stages do not model (edge wrapping, Highpass colour space and alpha,
/// Mosaic leaving alpha alone), make the caller composite on the CPU.
pub(crate) fn supports(filter: &Filter) -> bool {
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
    /// None as well for a filter [`supports`] rejects.
    pub(crate) fn new(filter: &Filter, resolution: u32, longest_side: u32, opacity: f32, signed: bool) -> Option<Self> {
        if !supports(filter) {
            return None;
        }
        let scale = resolution.max(1) as f32 / 2048.0;
        let radius = |at_2k: f32| (at_2k * scale).max(0.5);
        let mut out = Self {
            kind: Kind::Blur,
            radius: 1,
            opacity: opacity.clamp(0.0, 1.0),
            amount: 0.0,
            frequency: 0.0,
            middle: if signed { 0.0 } else { 0.5 },
            direction: [0.0; 2],
            seed: 0,
        };
        let direction = |degrees: f32| {
            let (sin, cos) = degrees.to_radians().sin_cos();
            [cos, -sin]
        };
        let noise_frequency = |scale: f32| (if scale.is_finite() { scale.clamp(0.01, 1000.0) } else { 1.0 }) / resolution.max(1) as f32;
        // A size over its intensity divider (a divider of 0 counts as 1), as the CPU reads it
        let divided = |size: f32, divider: u16| size / f32::from(divider.max(1));

        let (kind, texels) = match *filter {
            Filter::Blur { radius: r, .. } if r > 0.0 => (Kind::Blur, radius(r)),
            Filter::Sharpen { amount } => {
                out.amount = amount;
                (Kind::Sharpen, (1.5 * scale).max(0.75))
            }
            Filter::HighPass { radius: r, .. } => (Kind::HighPass, radius(r)),
            Filter::Bevel { width, .. } => (Kind::Bevel, radius(width * 0.5)),
            Filter::Glow { radius: r, amount } => {
                out.amount = amount;
                (Kind::Glow, radius(r))
            }
            Filter::DirectionalBlur { radius: r, angle } if r > 0.0 => {
                out.direction = direction(angle);
                (Kind::DirectionalBlur, radius(r))
            }
            Filter::Warp { intensity, scale, seed, divider } if intensity > 0.0 => {
                out.frequency = noise_frequency(scale);
                out.seed = seed;
                (Kind::Warp, radius(divided(intensity, divider)))
            }
            Filter::DirectionalWarp { intensity, angle, scale, seed, divider } if intensity > 0.0 => {
                out.frequency = noise_frequency(scale);
                out.direction = direction(angle);
                out.seed = seed;
                (Kind::DirectionalWarp, radius(divided(intensity, divider)))
            }
            Filter::EdgeDetect { width } => (Kind::Edge, radius(width)),
            Filter::SlopeBlur { radius: r, scale, seed, divider } if r > 0.0 => {
                out.frequency = noise_frequency(scale);
                out.seed = seed;
                (Kind::SlopeBlur, radius(divided(r, divider)))
            }
            Filter::Mosaic { size, .. } => (Kind::Mosaic, radius(size)),
            _ => return None,
        };
        out.kind = kind;
        out.radius = texels.round().clamp(1.0, longest_side.max(1) as f32) as u32;
        Some(out)
    }

    pub(crate) fn reach(self) -> u32 {
        if self.uses_box() { self.radius.saturating_mul(BOX_PASSES as u32) } else { self.radius.saturating_add(1) }
    }

    pub(crate) fn uses_box(self) -> bool {
        matches!(self.kind, Kind::Blur | Kind::Sharpen | Kind::HighPass | Kind::Bevel | Kind::Glow)
    }

    pub(crate) fn work_per_texel(self) -> u64 {
        let samples = u64::from(self.radius.min(24));
        match self.kind {
            Kind::DirectionalBlur => 1 + 4 * (2 * samples + 1),
            Kind::SlopeBlur => 5 + 4 * (samples + 1),
            Kind::Edge => 17,
            Kind::Warp | Kind::DirectionalWarp => 9,
            Kind::Mosaic => 5,
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
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    rect: [u32; 4],
    buffers: [u32; 4],
    mode: [u32; 4],
    weights: [f32; 4],
    direction: [f32; 4],
}

pub(crate) struct NeighbourhoodGpu {
    box_blur: wgpu::ComputePipeline,
    apply: wgpu::ComputePipeline,
    cell_average: wgpu::ComputePipeline,
    cell_interpolate: wgpu::ComputePipeline,
}

impl NeighbourhoodGpu {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tc_neighbourhood"),
            source: wgpu::ShaderSource::Wgsl(concat!(include_str!("shaders/noise.wgsl"), "\n", include_str!("shaders/neighbourhood.wgsl")).into()),
        });
        let pipeline = |entry| {
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
        let params = Params { rect: region.rect, buffers: [region.original, region.blur[0], region.output, 0], mode: [cell, 0, 0, 0], ..Params::zeroed() };
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
        let mut run = |pipeline: &wgpu::ComputePipeline, buffers: [u32; 4], first_box: bool, groups: [u32; 2]| {
            let params = Params {
                rect: region.rect,
                buffers,
                mode: [filter.radius, filter.kind as u32, filter.seed, u32::from(first_box)],
                weights: [filter.opacity, filter.amount, filter.frequency, filter.middle],
                direction: [filter.direction[0], filter.direction[1], 0.0, 0.0],
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
