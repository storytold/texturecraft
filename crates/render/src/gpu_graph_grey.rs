//! Packed grayscale graph passes. Whole images can feed later GPU nodes, while bounded stripes support larger jobs.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use texturecraft_graph::accelerator::{GreyInput, GreyPass, ResidentGrey16, ResidentGreyPass, ResidentGreyPixels};
use texturecraft_graph::context::Cancel;
use texturecraft_graph::tex::Grey16;
use wgpu::util::DeviceExt;

const BAND_BYTES: u64 = 8 << 20;
const WORKGROUP: u32 = 8;
const ENLARGE: u32 = 0;
const REDUCE: u32 = 1;
const LEVELS: u32 = 2;
const SHADER: &str = concat!(include_str!("shaders/graph_grey.wgsl"), "\n", include_str!("shaders/graph_compiled_grey.wgsl"));

pub(crate) struct GreyRasterizer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    resident_bytes: Arc<AtomicU64>,
}

impl GreyRasterizer {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self, String> {
        let pipeline = crate::gpu_trace::scoped(device, "graph grayscale shader", || {
            let module =
                device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("graph grayscale"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("graph grayscale"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            })
        })?;
        Ok(Self { device: device.clone(), queue: queue.clone(), pipeline, resident_bytes: Arc::default() })
    }

    fn resident_storage_limit(&self) -> u64 {
        let limits = self.device.limits();
        limits.max_buffer_size.min(limits.max_storage_buffer_binding_size)
    }

    fn reserve(&self, bytes: u64) -> Option<Arc<ResidentBytes>> {
        let storage = self.resident_storage_limit();
        if bytes > storage {
            return None;
        }
        let buffers = if self.device.adapter_info().device_type == wgpu::DeviceType::DiscreteGpu { 6 } else { 3 };
        let cap = (texturecraft_compose::graphs::budget::limit() / 8).min(storage.saturating_mul(buffers));
        self.resident_bytes.try_update(Ordering::Relaxed, Ordering::Relaxed, |used| used.checked_add(bytes).filter(|&used| used <= cap)).ok()?;
        Some(Arc::new(ResidentBytes { total: self.resident_bytes.clone(), bytes }))
    }

    pub(crate) fn allocate_resident(&self, w: usize, h: usize) -> Result<Option<Arc<GreyDevicePixels>>, String> {
        let Some(bytes) = packed_bytes(w, h) else { return Ok(None) };
        let Some(reservation) = self.reserve(bytes) else { return Ok(None) };
        let buffer = crate::gpu_trace::scoped(&self.device, "resident graph grayscale buffer", || {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("resident graph grayscale buffer"),
                size: bytes,
                usage: RESIDENT_BUFFER_USAGE,
                mapped_at_creation: false,
            })
        })?;
        Ok(Some(Arc::new(self.resident_pixels(buffer, reservation))))
    }

    fn resident_pixels(&self, buffer: wgpu::Buffer, reservation: Arc<ResidentBytes>) -> GreyDevicePixels {
        GreyDevicePixels { device: self.device.clone(), queue: self.queue.clone(), buffer, reservation }
    }

    pub(crate) fn bind_input(&self, input: GreyInput<'_>) -> Result<Option<GreyDevicePixels>, String> {
        let host = match input {
            GreyInput::Host(host) => host,
            GreyInput::Device(image) => {
                return Ok(GreyDevicePixels::from_resident(image).filter(|pixels| pixels.device == self.device).cloned());
            }
        };
        let Some(bytes) = packed_bytes(host.w, host.h).filter(|&bytes| bytes / 2 == host.px.len() as u64) else { return Ok(None) };
        let Some(reservation) = self.reserve(bytes) else { return Ok(None) };
        let buffer = crate::gpu_trace::scoped(&self.device, "resident graph grayscale upload", || {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("resident graph grayscale upload"),
                contents: bytemuck::cast_slice(&host.px),
                usage: RESIDENT_BUFFER_USAGE,
            })
        })?;
        Ok(Some(self.resident_pixels(buffer, reservation)))
    }

    pub(crate) fn render_resident(&self, pass: ResidentGreyPass<'_>, cancel: &Cancel) -> Result<Option<ResidentGrey16>, String> {
        let (foreground, background, side, mode, opacity) = match pass {
            ResidentGreyPass::Enlarge { source, side } => (source, None, side, ENLARGE, 0.0),
            ResidentGreyPass::Reduce { source, side } => (source, None, side, REDUCE, 0.0),
            ResidentGreyPass::Levels { source, curve, div } => return self.render_levels(source, &curve, div, cancel),
            ResidentGreyPass::Blend { foreground, background, mode, opacity } => {
                if !(32..=34).contains(&mode) || !opacity.is_finite() {
                    return Ok(None);
                }
                let (side, _) = grey_input_shape(&foreground);
                (foreground, Some(background), side, mode, opacity)
            }
        };
        let (source_side, alpha) = grey_input_shape(&foreground);
        if cancel.is_cancelled() || !(2..=8192).contains(&side) || !side.is_power_of_two() || source_side == 0 {
            return Ok(None);
        }
        let alpha = if let Some(background) = &background {
            let (background_side, alpha) = grey_input_shape(background);
            if background_side != side {
                return Ok(None);
            }
            alpha
        } else {
            if (mode == ENLARGE && source_side >= side) || (mode == REDUCE && source_side <= side) {
                return Ok(None);
            }
            alpha
        };
        let limit = self.device.limits();
        let largest_output_side = if mode == REDUCE { source_side / 2 } else { side };
        if (largest_output_side as u32).div_ceil(WORKGROUP) > limit.max_compute_workgroups_per_dimension {
            return Ok(None);
        }
        // Poll without waiting so completed submissions can release reservations before trying to reserve more.
        self.device.poll(wgpu::PollType::Poll).map_err(|error| error.to_string())?;
        let Some(front) = self.bind_input(foreground)? else { return Ok(None) };
        let back = match background {
            Some(image) => {
                let Some(bound) = self.bind_input(image)? else { return Ok(None) };
                Some(bound)
            }
            None => None,
        };
        let mut stages = Vec::new();
        let next_side = |size| match mode {
            ENLARGE => size * 2,
            REDUCE => size / 2,
            _ => side,
        };
        let mut size = next_side(source_side);
        loop {
            let Some(output) = self.allocate_resident(size, size)? else { return Ok(None) };
            stages.push((size, output));
            if size == side {
                break;
            }
            size = next_side(size);
        }
        if cancel.is_cancelled() {
            return Ok(None);
        }

        crate::gpu_trace::scoped(&self.device, "resident graph grayscale pass", || {
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("resident graph grayscale") });
            let mut input = (source_side, &front);
            for (size, output) in &stages {
                let back = &back.as_ref().unwrap_or(input.1).buffer; // Resizing does not read this binding.
                self.encode_resident(&mut encoder, input, back, (*size, output), mode, opacity);
                input = (*size, output);
            }
            self.queue.submit([encoder.finish()]);
        })?;
        front.retain_until_completed(&self.queue);
        if let Some(back) = back {
            back.retain_until_completed(&self.queue);
        }
        for (_, pixels) in &stages {
            pixels.retain_until_completed(&self.queue);
        }
        Ok(stages.pop().map(|(_, pixels)| ResidentGrey16 { w: side, h: side, div: 65535.0, alpha, pixels }))
    }

    fn render_levels(&self, source: GreyInput<'_>, curve: &[u16], div: f32, cancel: &Cancel) -> Result<Option<ResidentGrey16>, String> {
        let (w, h, source_div, alpha) = source.info();
        let limit = self.device.limits().max_compute_workgroups_per_dimension as usize;
        if cancel.is_cancelled() || w != h || curve.len() != source_div as usize + 1 || w.div_ceil(WORKGROUP as usize) > limit {
            return Ok(None);
        }
        self.device.poll(wgpu::PollType::Poll).map_err(|error| error.to_string())?;
        let Some(front) = self.bind_input(source)? else { return Ok(None) };
        let Some(output) = self.allocate_resident(w, h)? else { return Ok(None) };
        crate::gpu_trace::scoped(&self.device, "resident graph levels pass", || {
            let table = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("graph levels lookup table"),
                contents: bytemuck::cast_slice(curve),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("resident graph levels") });
            self.encode_resident(&mut encoder, (w, &front), &table, (w, &output), LEVELS, 0.0);
            self.queue.submit([encoder.finish()]);
        })?;
        front.retain_until_completed(&self.queue);
        output.retain_until_completed(&self.queue);
        Ok(Some(ResidentGrey16 { w, h, div, alpha, pixels: output }))
    }

    fn encode_resident(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        (source_side, front): (usize, &GreyDevicePixels),
        back: &wgpu::Buffer,
        (side, output): (usize, &GreyDevicePixels),
        mode: u32,
        opacity: f32,
    ) {
        let params = [side as u32, side as u32, source_side as u32, mode, opacity.to_bits(), 1, 0, 0];
        let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("resident graph grayscale parameters"),
            contents: bytemuck::cast_slice(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let entries = [&uniform, &front.buffer, back, &output.buffer]
            .into_iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry { binding: binding as u32, resource: buffer.as_entire_binding() })
            .collect::<Vec<_>>();
        let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("resident graph grayscale inputs"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let side = side as u32;
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &binding, &[]);
        pass.dispatch_workgroups((side / 2).div_ceil(WORKGROUP), side.div_ceil(WORKGROUP), 1);
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

const RESIDENT_BUFFER_USAGE: wgpu::BufferUsages = wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_SRC).union(wgpu::BufferUsages::COPY_DST);

fn packed_bytes(w: usize, h: usize) -> Option<u64> {
    // A row is addressable both as pairs of u16 on the GPU and by byte offset when read back.
    if w == 0 || h == 0 || !w.is_multiple_of(2) {
        return None;
    }
    (w as u64).checked_mul(h as u64)?.checked_mul(2)
}

fn grey_input_shape(input: &GreyInput<'_>) -> (usize, f32) {
    let (w, h, div, alpha) = input.info();
    (if (2..=8192).contains(&w) && w == h && w.is_power_of_two() && div == 65535.0 { w } else { 0 }, alpha)
}

#[derive(Debug)]
struct ResidentBytes {
    total: Arc<AtomicU64>,
    bytes: u64,
}

impl Drop for ResidentBytes {
    fn drop(&mut self) {
        self.total.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

/// GPU storage for a grayscale node, in row order with two u16 samples per word.
#[derive(Clone, Debug)]
pub(crate) struct GreyDevicePixels {
    pub(crate) buffer: wgpu::Buffer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    reservation: Arc<ResidentBytes>,
}

impl GreyDevicePixels {
    pub(crate) fn from_resident(image: &ResidentGrey16) -> Option<&Self> {
        let pixels = image.pixels.as_any().downcast_ref::<Self>()?;
        (pixels.buffer.size() == packed_bytes(image.w, image.h)?).then_some(pixels)
    }

    pub(crate) fn retain_until_completed(&self, queue: &wgpu::Queue) {
        let reservation = self.reservation.clone();
        queue.on_submitted_work_done(move || drop(reservation));
    }
}

impl ResidentGreyPixels for GreyDevicePixels {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn read(&self, cancel: &Cancel) -> Result<Vec<u16>, String> {
        if cancel.is_cancelled() {
            return Err("GPU grayscale readback was cancelled".into());
        }
        let size = self.buffer.size();
        let band = size.min(BAND_BYTES);
        let staging = crate::gpu_trace::scoped(&self.device, "resident graph grayscale readback", || {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("resident graph grayscale readback"),
                size: band,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        })?;
        let mut pixels = Vec::with_capacity((size / 2) as usize);
        for offset in (0..size).step_by(band as usize) {
            if cancel.is_cancelled() {
                return Err("GPU grayscale readback was cancelled".into());
            }
            let bytes = (size - offset).min(band);
            let submission = crate::gpu_trace::scoped(&self.device, "resident graph grayscale copy", || {
                let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("resident graph grayscale copy") });
                encoder.copy_buffer_to_buffer(&self.buffer, offset, &staging, 0, bytes);
                self.queue.submit([encoder.finish()])
            })?;
            read_words(&self.device, &staging, bytes, submission, &mut pixels)?;
        }
        Ok(pixels)
    }
}

fn read_words(device: &wgpu::Device, staging: &wgpu::Buffer, bytes: u64, submission: wgpu::SubmissionIndex, pixels: &mut Vec<u16>) -> Result<(), String> {
    let slice = staging.slice(..bytes);
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
    staging.unmap();
    copied
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
            read_words(device, &self.readback, bytes, submission, &mut pixels)?;
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
