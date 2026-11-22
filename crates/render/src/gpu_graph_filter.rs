//! Resident spatial filters for packed grayscale graphs.

use std::sync::Arc;

use texturecraft_graph::accelerator::{FilterOperation, GreyInput, ResidentFilter, ResidentGrey16};
use texturecraft_graph::context::Cancel;
use wgpu::util::DeviceExt;

use crate::gpu_graph_grey::GreyRasterizer;

const WORKGROUP: u32 = 8;
const DIRECTIONAL_WARP: u32 = 0;
const GRADIENT_WARP: u32 = 1;
const SHORT_BOX: u32 = 2;
const SHORT_LINE: u32 = 3;
const SHARPEN: u32 = 4;
const SHADER: &str = concat!(include_str!("shaders/graph_filter.wgsl"), "\n", include_str!("shaders/graph_compiled_grey.wgsl"));

pub(crate) struct FilterRasterizer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
}

impl FilterRasterizer {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self, String> {
        let pipeline = crate::gpu_trace::scoped(device, "graph grayscale filter shader", || {
            let module = device
                .create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("graph grayscale filters"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("graph grayscale filter"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            })
        })?;
        Ok(Self { device: device.clone(), queue: queue.clone(), pipeline })
    }

    pub(crate) fn render(&self, filter: ResidentFilter<'_>, pool: &GreyRasterizer, cancel: &Cancel) -> Result<Option<ResidentGrey16>, String> {
        let [w, h] = filter.size;
        let [groups_x, groups_y] = [(w as u32 / 2).div_ceil(WORKGROUP), (h as u32).div_ceil(WORKGROUP)];
        let limit = self.device.limits().max_compute_workgroups_per_dimension;
        if cancel.is_cancelled() || groups_x == 0 || groups_y == 0 || groups_x.max(groups_y) > limit {
            return Ok(None);
        }
        let source = filter.source;
        let (sw, sh, divisor, alpha) = source.info();
        let [wrap_x, wrap_y] = filter.tile.map(u32::from);
        let mut params = Params {
            source: [sw as u32, sh as u32, divisor as u32, 0],
            output: [w as u32, h as u32, filter.div as u32, 0],
            map: [0; 4],
            control: [0, 0, wrap_x, wrap_y],
            values: [0.0; 4],
            direction: [0.0; 4],
        };
        let mut passes = Vec::with_capacity(2);
        let map = match filter.operation {
            FilterOperation::DirectionalWarp { map, intensity, offset, direction, aspect } => {
                params.control[0] = DIRECTIONAL_WARP;
                params.values = [intensity, offset, aspect[0], aspect[1]];
                params.direction = [direction[0], direction[1], 0.0, 0.0];
                Some(map)
            }
            FilterOperation::GradientWarp { map, displacement } => {
                params.control[0] = GRADIENT_WARP;
                params.values = [displacement[0], displacement[1], 0.0, 0.0];
                Some(map)
            }
            FilterOperation::Box { radius } => {
                params.control[0] = SHORT_BOX;
                for (axis, radius) in radius.into_iter().enumerate() {
                    if radius <= 0.5 && params.source[2] == params.output[2] {
                        continue;
                    }
                    params.control[1] = axis as u32;
                    params.values[0] = radius;
                    let edge = (f64::from(radius) - 1.5).max(0.0);
                    params.values[1] = (edge / (1.0 + edge) * 256.0).round() as f32;
                    passes.push(params);
                    params.source = params.output;
                }
                None
            }
            FilterOperation::ShortLine { major, slope, radius } => {
                params.control[0] = SHORT_LINE;
                params.control[1] = major as u32;
                params.values = [radius, slope, 0.0, 0.0];
                None
            }
            FilterOperation::Sharpen { weights } => {
                params.control[0] = SHARPEN;
                params.values = [weights[0], weights[1], 0.0, 0.0];
                None
            }
        };
        if let Some(map) = map {
            let (mw, mh, div, _) = map.info();
            params.map = [mw as u32, mh as u32, div as u32, 0];
        }
        if !matches!(filter.operation, FilterOperation::Box { .. }) {
            passes.push(params);
        }

        self.device.poll(wgpu::PollType::Poll).map_err(|error| error.to_string())?;
        let Some(input) = pool.bind_input(source)? else { return Ok(None) };
        let external_map = match map {
            Some(map) if !same_input(source, map) => {
                let Some(bound) = pool.bind_input(map)? else { return Ok(None) };
                Some(bound)
            }
            _ => None,
        };
        if passes.is_empty() {
            return Ok(Some(ResidentGrey16 { w, h, div: filter.div, alpha, pixels: Arc::new(input) }));
        }
        let mut outputs = Vec::with_capacity(passes.len());
        for _ in &passes {
            let Some(output) = pool.allocate_resident(w, h)? else { return Ok(None) };
            outputs.push(output);
        }
        if cancel.is_cancelled() {
            return Ok(None);
        }

        crate::gpu_trace::scoped(&self.device, "resident graph spatial filter", || {
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("graph grayscale filters") });
            let mut front = &input.buffer;
            let map = &external_map.as_ref().unwrap_or(&input).buffer;
            let layout = self.pipeline.get_bind_group_layout(0);
            for (params, output) in passes.iter().zip(&outputs) {
                let settings = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("graph filter settings"),
                    contents: bytemuck::bytes_of(params),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let buffers = [&settings, front, map, &output.buffer];
                let entries = std::array::from_fn::<_, 4, _>(|i| wgpu::BindGroupEntry { binding: i as u32, resource: buffers[i].as_entire_binding() });
                let bind_group =
                    self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("graph filter inputs"), layout: &layout, entries: &entries });
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("graph grayscale filter"), timestamp_writes: None });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(groups_x, groups_y, 1);
                front = &output.buffer;
            }
            self.queue.submit([encoder.finish()]);
        })?;
        input.retain_until_completed(&self.queue);
        if let Some(map) = external_map {
            map.retain_until_completed(&self.queue);
        }
        for pixels in &outputs {
            pixels.retain_until_completed(&self.queue);
        }
        Ok(outputs.pop().map(|pixels| ResidentGrey16 { w, h, div: filter.div, alpha, pixels }))
    }
}

fn same_input(a: GreyInput<'_>, b: GreyInput<'_>) -> bool {
    match (a, b) {
        (GreyInput::Host(a), GreyInput::Host(b)) => std::ptr::eq(a, b),
        (GreyInput::Device(a), GreyInput::Device(b)) => std::ptr::eq(a, b),
        _ => false,
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    source: [u32; 4],
    output: [u32; 4],
    map: [u32; 4],
    control: [u32; 4],
    values: [f32; 4],
    direction: [f32; 4],
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    use texturecraft_graph::{context::EvalContext, graph::Graph, profile::Profiler, tex::Tex};

    #[derive(Default)]
    struct Nodes(AtomicUsize);

    impl Profiler for Nodes {
        fn enabled(&self) -> bool {
            false
        }

        fn span(&self, _: &str) -> Box<dyn std::any::Any + Send> {
            Box::new(())
        }

        fn resident_node(&self, _: u32, _: &str) {
            self.0.fetch_add(1, Relaxed);
        }
    }

    #[test]
    fn imported_grayscale_filters_match_the_cpu() {
        let Ok(gpu) = crate::headless::Headless::new() else { return };
        let nodes = Arc::new(Nodes::default());
        let context = EvalContext {
            accelerator: Some(Arc::new(crate::gpu_graph::GraphAccelerator::new(&gpu.device, &gpu.queue))),
            profiler: Some(nodes.clone()),
            ..Default::default()
        };
        let [size, map_size] = [256, 128];
        let make_inputs = |map_size| {
            [
                Some(Tex::gray_fn(size, size, |u, v| 0.5 + 0.45 * (u * 18.0).sin() * (v * 20.0).cos())),
                Some(Tex::gray_fn(map_size, map_size, |u, v| 0.5 + 0.5 * (u * 9.0).sin() * (v * 15.0).cos())),
            ]
        };
        for (filter, div, options, gradient) in [
            ("dir_warp", 65535, "map=@2 angle=0.166 intensity=5", false),
            ("dir_warp", 65535, "map=@2 angle=-0.124 intensity=0.9 map_offset=0.5 tile_x=0 tile_y=0", false),
            ("grad_warp", 65535, "map=@2 intensity=0.8 aspect_x=0.5 tile_x=0", true),
            ("box_blur", 65535, "intensity=2.17 aspect_y=0.33", false),
            ("box_blur", 255, "intensity=1.7 tile_x=0 tile_y=0", false),
            ("line_blur", 65535, "intensity=1.3 angle=0.125", false),
            ("line_blur", 65535, "intensity=2.1 angle=0.25", false),
            ("laplacian_sharpen", 255, "intensity=0.4", false),
        ] {
            nodes.0.store(0, Relaxed);
            let format = if div == 255 { "u8_compiled" } else { "u16_compiled" };
            let map_log2 = if gradient { map_size.ilog2() } else { size.ilog2() };
            let graph = Graph::parse(&format!(
                "format 3\nsize {size}\nmeta compiled \"1\"\nnode 1 input idx=0 gray=1 format=\"{format}\" osize_mode=1\n\
                 node 2 input idx=1 gray=1 format=\"{format}\" osize_log2_x={map_log2} osize_log2_y={map_log2}\n\
                 node 3 {filter} in=@1 format=\"{format}\" {options}\noutput out 3\n"
            ))
            .unwrap();
            let inputs = make_inputs(if gradient { map_size } else { size });
            let cpu = &graph.evaluate_with_inputs(None, &inputs).unwrap()["out"];
            let gpu = &graph.evaluate_in(&context, None, &inputs).unwrap()["out"];
            assert_eq!(nodes.0.load(Relaxed), 1, "{filter} {options}");
            let errors = cpu.px.iter().zip(&gpu.px).map(|(a, b)| ((a[0] - b[0]).abs() * div as f32).round() as u32);
            let (max, sum) = errors.fold((0, 0), |(max, sum), error| (max.max(error), sum + error));
            let mean = f64::from(sum) / cpu.px.len() as f64;
            // A slope across two differently sized grids can cross one 1/256 interpolation weight.
            let max_steps = if gradient { 16 } else { 1 };
            assert!(max <= max_steps && mean < 0.05, "{filter} {options}: errors {max} maximum, {mean} mean quantization steps");
        }
    }
}
