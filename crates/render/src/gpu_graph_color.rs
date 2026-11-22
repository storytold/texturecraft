//! Compiled color graph operations with the same GPU storage budget as grayscale nodes.

use texturecraft_graph::accelerator::{ColorInput, ResidentColor16, ResidentColorOutput, ResidentColorPass, ResidentGrey16};
use texturecraft_graph::context::Cancel;
use wgpu::util::DeviceExt;

use crate::gpu_graph_grey::{GreyDevicePixels, GreyRasterizer};

const SHADER: &str =
    concat!(include_str!("shaders/graph_color.wgsl"), "\n", include_str!("shaders/graph_compiled_grey.wgsl"), "\n", include_str!("shaders/graph_blend.wgsl"),);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    extent: [u32; 4],
    formats: [u32; 4],
    alphas: [f32; 4],
    weights: [f32; 4],
    alpha_mode: [u32; 4],
}

pub(crate) struct ColorRasterizer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
}

impl ColorRasterizer {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self, String> {
        let pipeline = crate::gpu_trace::scoped(device, "graph color shader", || {
            let module =
                device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("graph color"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("graph color"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            })
        })?;
        Ok(Self { device: device.clone(), queue: queue.clone(), pipeline })
    }

    fn bind(&self, pool: &GreyRasterizer, input: ColorInput<'_>, area: usize) -> Result<Option<GreyDevicePixels>, String> {
        match input {
            ColorInput::Grey(image) => pool.bind_input(image),
            ColorInput::Host(image) if image.px.len() == area => pool.upload_pixels(bytemuck::cast_slice(&image.px)),
            ColorInput::Host(_) => Ok(None),
            ColorInput::Device(image) => {
                Ok(GreyDevicePixels::from_pixels(image.pixels.as_ref(), area as u64 * 8).filter(|pixels| pixels.belongs_to(&self.device)).cloned())
            }
        }
    }

    pub(crate) fn render(&self, pass: ResidentColorPass<'_>, pool: &GreyRasterizer, cancel: &Cancel) -> Result<Option<ResidentColorOutput>, String> {
        let (front, back, mask, op, mode, opacity, alpha_mode, weights, alpha) = match pass {
            ResidentColorPass::Blend { foreground, background, mask, mode, opacity, alpha_mode, alpha } => {
                (foreground, Some(background), mask, 0, mode, opacity, alpha_mode, [0.0; 4], alpha)
            }
            ResidentColorPass::Grayscale { source, weights, alpha } => (source, None, None, if alpha.is_some() { 1 } else { 2 }, 0, 0.0, 0, weights, alpha),
        };
        let [w, h] = input_size(front);
        if cancel.is_cancelled() || !(2..=8192).contains(&w) || !w.is_multiple_of(2) || !(1..=8192).contains(&h) {
            return Ok(None);
        }
        let group = [(w as u32 / 2).div_ceil(8), (h as u32).div_ceil(8)];
        let limit = self.device.limits().max_compute_workgroups_per_dimension;
        if group.into_iter().any(|g| g > limit) || back.into_iter().chain(mask).any(|input| input_size(input) != [w, h]) {
            return Ok(None);
        }
        let [first, second, third] = [front, back.unwrap_or(front), mask.unwrap_or(front)].map(input_format);
        let (Some((format_f, alpha_f)), Some((format_b, alpha_b)), Some((format_m, alpha_m))) = (first, second, third) else { return Ok(None) };
        let params = Params {
            extent: [w as u32, h as u32, op, mode],
            formats: [format_f, format_b, format_m, u32::from(mask.is_some())],
            alphas: [alpha_f, alpha_b, alpha_m, opacity],
            weights,
            alpha_mode: [alpha_mode, 0, 0, 0],
        };
        self.device.poll(wgpu::PollType::Poll).map_err(|error| error.to_string())?;
        let Some(front) = self.bind(pool, front, w * h)? else { return Ok(None) };
        let back = match back {
            Some(image) => {
                let Some(bound) = self.bind(pool, image, w * h)? else { return Ok(None) };
                Some(bound)
            }
            None => None,
        };
        let mask = match mask {
            Some(image) => {
                let Some(bound) = self.bind(pool, image, w * h)? else { return Ok(None) };
                Some(bound)
            }
            None => None,
        };
        let Some(output) = pool.allocate_resident(if op == 1 { w } else { w * 4 }, h)? else { return Ok(None) };
        if cancel.is_cancelled() {
            return Ok(None);
        }

        crate::gpu_trace::scoped(&self.device, "resident graph color pass", || {
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("resident graph color") });
            let settings = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("resident graph color parameters"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let buffers = [&settings, &front.buffer, &back.as_ref().unwrap_or(&front).buffer, &mask.as_ref().unwrap_or(&front).buffer, &output.buffer];
            let entries = std::array::from_fn::<_, 5, _>(|i| wgpu::BindGroupEntry { binding: i as u32, resource: buffers[i].as_entire_binding() });
            let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("resident graph color inputs"),
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &entries,
            });
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &binding, &[]);
            pass.dispatch_workgroups(group[0], group[1], 1);
            drop(pass);
            self.queue.submit([encoder.finish()]);
        })?;
        for pixels in [&front, output.as_ref()].into_iter().chain(back.as_ref()).chain(mask.as_ref()) {
            pixels.retain_until_completed(&self.queue);
        }
        Ok(Some(match (op, alpha) {
            (1, Some(alpha)) => ResidentColorOutput::Grey(ResidentGrey16 { w, h, div: 65535.0, alpha, pixels: output }),
            _ => ResidentColorOutput::Color(ResidentColor16 { w, h, alpha, pixels: output }),
        }))
    }
}

fn input_size(input: ColorInput<'_>) -> [usize; 2] {
    match input {
        ColorInput::Grey(image) => {
            let (w, h, ..) = image.info();
            [w, h]
        }
        ColorInput::Host(image) => [image.w, image.h],
        ColorInput::Device(image) => [image.w, image.h],
    }
}

fn input_format(input: ColorInput<'_>) -> Option<(u32, f32)> {
    match input {
        ColorInput::Grey(image) => {
            let (_, _, div, alpha) = image.info();
            match div {
                65535.0 => Some((0, alpha)),
                255.0 => Some((1, alpha)),
                _ => None,
            }
        }
        ColorInput::Host(_) => Some((2, 0.0)),
        ColorInput::Device(_) => Some((3, 0.0)),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };

    use texturecraft_graph::{context::EvalContext, graph::Graph, profile::Profiler, tex::Tex};

    #[derive(Default)]
    struct Nodes(Mutex<BTreeMap<String, usize>>);

    impl Profiler for Nodes {
        fn enabled(&self) -> bool {
            false
        }

        fn span(&self, _: &str) -> Box<dyn std::any::Any + Send> {
            Box::new(())
        }

        fn resident_node(&self, _: u32, kind: &str) {
            *self.0.lock().unwrap().entry(kind.into()).or_default() += 1;
        }

        fn resident_readback(&self, _: u32, _: usize) {
            *self.0.lock().unwrap().entry("readback".into()).or_default() += 1;
        }
    }

    #[test]
    fn compiled_color_blends_and_grayscale_stay_resident() {
        let Ok(gpu) = crate::headless::Headless::new() else { return };
        let nodes = Arc::new(Nodes::default());
        let ctx = EvalContext {
            accelerator: Some(Arc::new(crate::gpu_graph::GraphAccelerator::new(&gpu.device, &gpu.queue))),
            profiler: Some(nodes.clone()),
            ..Default::default()
        };
        let graph = Graph::parse(
            "format 3\nsize 1024\nmeta compiled \"1\"\n\
             node 1 input idx=0 format=\"u16_compiled\"\nnode 2 input idx=1 gray=1 format=\"u8_compiled\"\n\
             node 3 input idx=2 format=\"u16_compiled\"\n\
             node 4 blend fg=@2 bg=@1 mask=@3 mode=\"add_sub_compiled\" opacity=0.62 format=\"u16_compiled\"\n\
             node 5 blend fg=@4 bg=@1 mode=\"switch\" alpha_mode=1 opacity=0.34 format=\"u16_compiled\"\n\
             node 6 grayscale_conversion in=@5 r=0.299 g=0.587 b=0.114 a=0.02 limit=1 format=\"u16_compiled\"\n\
             node 7 input idx=1 gray=1 format=\"u16_compiled\"\n\
             node 8 blend fg=@7 bg=@6 mode=\"multiply\" opacity=0.38 format=\"u16_compiled\"\noutput out 8\n",
        )
        .unwrap();
        for varying_alpha in [false, true] {
            let inputs = [
                Some(Tex::from_fn(1024, 1024, |u, v| [u, v, (u + v) * 0.5, if varying_alpha { u } else { 1.0 }])),
                Some(Tex::gray_fn(1024, 1024, |u, v| 0.5 + 0.5 * (u * 32.0).sin() * (v * 33.0).cos())),
                Some(Tex::from_fn(1024, 1024, |u, v| [v, 1.0 - u, 1.0 - v, u])),
            ];
            nodes.0.lock().unwrap().clear();
            let cpu = &graph.evaluate_with_inputs(None, &inputs).unwrap()["out"];
            let gpu = &graph.evaluate_in(&ctx, None, &inputs).unwrap()["out"];
            let seen = nodes.0.lock().unwrap();
            assert_eq!(seen.get("blend"), Some(&3), "varying alpha={varying_alpha}, {seen:?}");
            assert_eq!(seen.get("grayscale_conversion"), Some(&1));
            assert_eq!(seen.get("readback"), Some(&1));
            let max =
                cpu.px.iter().zip(&gpu.px).flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| ((a - b).abs() * 65535.0).round() as u32)).max().unwrap_or_default();
            assert!(max <= 1, "varying alpha={varying_alpha}, {max} u16 steps");
        }
    }
}
