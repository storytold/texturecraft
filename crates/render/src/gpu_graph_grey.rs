//! Large compiled grayscale transforms and blends using bounded stripes of packed 16-bit samples.

use texturecraft_graph::accelerator::GreyPass;
use texturecraft_graph::context::Cancel;
use texturecraft_graph::tex::Grey16;

const BAND_BYTES: u64 = 8 << 20;
const WORKGROUP: u32 = 8;

pub(crate) struct GreyRasterizer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
}

impl GreyRasterizer {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self, String> {
        let pipeline = crate::gpu_trace::scoped(device, "graph grayscale shader", || {
            let module = device.create_shader_module(wgpu::include_wgsl!("shaders/graph_grey.wgsl"));
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("graph grayscale"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            })
        })?;
        Ok(Self { device: device.clone(), queue: queue.clone(), pipeline })
    }

    pub(crate) fn render(&self, pass: GreyPass<'_>, cancel: &Cancel) -> Result<Option<Grey16>, String> {
        let side = match &pass {
            GreyPass::Enlarge { side, .. } => *side,
            GreyPass::Blend { foreground, .. } => foreground.w,
        };
        if !(4..=8192).contains(&side) || !side.is_power_of_two() || cancel.is_cancelled() {
            return Ok(None);
        }
        let limit = self.device.limits();
        let budget = limit.max_buffer_size.min(limit.max_storage_buffer_binding_size).min(BAND_BYTES);
        // Doublings need an even band height so local row parity is also global row parity.
        let max_rows = (budget / (side as u64 * 2)).min(u64::from(limit.max_compute_workgroups_per_dimension * WORKGROUP));
        let band_rows = side.min(max_rows as usize / 2 * 2);
        if band_rows < 2 || (side as u32 / 2).div_ceil(WORKGROUP) > limit.max_compute_workgroups_per_dimension {
            return Ok(None);
        }
        let bands = Bands::new(self, band_rows, (side * band_rows * 2) as u64)?;
        match pass {
            GreyPass::Enlarge { source, side } => {
                if source.w < 2 || source.w >= side || source.h != source.w || !source.w.is_power_of_two() || source.px.len() != source.w * source.h {
                    return Ok(None);
                }
                let mut doubled = bands.run(source, None, 0, 0.0, source.w * 2, cancel)?;
                while doubled.w < side {
                    doubled = bands.run(&doubled, None, 0, 0.0, doubled.w * 2, cancel)?;
                }
                Ok(Some(doubled))
            }
            GreyPass::Blend { foreground, background, mode, opacity } => {
                if foreground.h != side || background.w != side || background.h != side || !(32..=34).contains(&mode) || !opacity.is_finite() {
                    return Ok(None);
                }
                bands.run(foreground, Some(background), mode, opacity, side, cancel).map(Some)
            }
        }
    }
}

struct Bands<'a> {
    renderer: &'a GreyRasterizer,
    rows: usize,
    inputs: [wgpu::Buffer; 2],
    result: wgpu::Buffer,
    readback: wgpu::Buffer,
    uniform: wgpu::Buffer,
    binding: wgpu::BindGroup,
}

impl<'a> Bands<'a> {
    fn new(renderer: &'a GreyRasterizer, rows: usize, bytes: u64) -> Result<Self, String> {
        let device = &renderer.device;
        crate::gpu_trace::scoped(device, "graph grayscale buffers", || {
            let buffer = |label, size, usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
            let input_usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
            let inputs = [buffer("graph grey foreground", bytes, input_usage), buffer("graph grey background", bytes, input_usage)];
            let result = buffer("graph grey output", bytes, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
            let readback = buffer("graph grey readback", bytes, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
            let uniform = buffer("graph grey parameters", 32, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
            let buffers = [&uniform, &inputs[0], &inputs[1], &result];
            let entries = buffers.map(wgpu::Buffer::as_entire_binding);
            let entries = entries.into_iter().enumerate().map(|(i, resource)| wgpu::BindGroupEntry { binding: i as u32, resource }).collect::<Vec<_>>();
            let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("graph grey inputs"),
                layout: &renderer.pipeline.get_bind_group_layout(0),
                entries: &entries,
            });
            Self { renderer, rows, inputs, result, readback, uniform, binding }
        })
    }

    fn run(&self, foreground: &Grey16, background: Option<&Grey16>, mode: u32, opacity: f32, side: usize, cancel: &Cancel) -> Result<Grey16, String> {
        let (device, queue) = (&self.renderer.device, &self.renderer.queue);
        let mut pixels = Vec::with_capacity(side * side);
        for y in (0..side).step_by(self.rows) {
            if cancel.is_cancelled() {
                return Err("GPU grayscale pass was cancelled".into());
            }
            let rows = (side - y).min(self.rows);
            if let Some(background) = background {
                let range = y * side..(y + rows) * side;
                let write = |buffer: &wgpu::Buffer, image: &Grey16| {
                    let src = image.px.get(range.clone()).ok_or("incomplete grayscale image")?;
                    queue.write_buffer(buffer, 0, bytemuck::cast_slice(src));
                    Ok::<(), String>(())
                };
                write(&self.inputs[0], foreground)?;
                write(&self.inputs[1], background)?;
            } else {
                upload_doubling_rows(queue, &self.inputs[0], foreground, y, rows)?;
            }
            let params = [side as u32, rows as u32, foreground.w as u32, mode, opacity.to_bits(), 0, 0, 0];
            queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&params));
            let bytes = (rows * side * 2) as u64;
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("graph grayscale band") });
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.renderer.pipeline);
                pass.set_bind_group(0, &self.binding, &[]);
                pass.dispatch_workgroups((side as u32 / 2).div_ceil(WORKGROUP), (rows as u32).div_ceil(WORKGROUP), 1);
            }
            encoder.copy_buffer_to_buffer(&self.result, 0, &self.readback, 0, bytes);
            let submission = queue.submit([encoder.finish()]);
            let slice = self.readback.slice(..bytes);
            let (sender, receiver) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
            device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: None }).map_err(|e| e.to_string())?;
            receiver.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
            let copied = (|| {
                let data = slice.get_mapped_range().map_err(|e| e.to_string())?;
                pixels.extend_from_slice(bytemuck::try_cast_slice::<u8, u16>(&data).map_err(|e| e.to_string())?);
                Ok::<(), String>(())
            })();
            self.readback.unmap();
            copied?;
        }
        let alpha = background.map_or(foreground.alpha, |b| b.alpha);
        Ok(Grey16 { w: side, h: side, px: pixels, div: 65535.0, alpha })
    }
}

/// Upload the sampled source rows plus one row of wrap padding on both ends.
fn upload_doubling_rows(queue: &wgpu::Queue, buffer: &wgpu::Buffer, source: &Grey16, first: usize, rows: usize) -> Result<(), String> {
    let (w, h) = (source.w, source.h);
    let mut source_row = (first / 2 + h - 1) % h;
    let mut dest_row = 0;
    let row_count = rows / 2 + 2;
    while dest_row < row_count {
        let take = (row_count - dest_row).min(h - source_row);
        let pixels = source.px.get(source_row * w..(source_row + take) * w).ok_or("incomplete grayscale image")?;
        queue.write_buffer(buffer, (dest_row * w * 2) as u64, bytemuck::cast_slice(pixels));
        dest_row += take;
        source_row = 0;
    }
    Ok(())
}
