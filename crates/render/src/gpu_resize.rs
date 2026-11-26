//! Bilinear GPU resampling of the document's sparse, premultiplied paint tiles.

use std::sync::Arc;

use texturecraft_texel::{TILE, TILE_AREA, TileImage};
use wgpu::util::DeviceExt;

use crate::paint_window::TileWindow;

const OUTPUT_BYTES: u64 = 8 << 20;
const INPUT_BYTES: u64 = 32 << 20;
/// Keeps both center mapping and Q16 weight rounding within a WGSL u32.
const MAX_INTEGER_RESIZE_SIDE: u32 = 32_768;

pub struct PaintResampler {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    input: Option<wgpu::Buffer>,
    result: Option<wgpu::Buffer>,
    readback: Option<wgpu::Buffer>,
}

impl PaintResampler {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self, String> {
        let pipeline = crate::gpu_trace::scoped(device, "paint resize shader", || {
            let module = device.create_shader_module(wgpu::include_wgsl!("shaders/paint_resize.wgsl"));
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("tc_paint_resize"),
                layout: None,
                module: &module,
                entry_point: Some("resize"),
                compilation_options: Default::default(),
                cache: None,
            })
        })?;
        Ok(Self { device: device.clone(), queue: queue.clone(), pipeline, input: None, result: None, readback: None })
    }

    pub fn resize(&mut self, src: &TileImage, size: (u32, u32), empty: &[u16], cancelled: impl Fn() -> bool) -> Result<TileImage, String> {
        if (src.width(), src.height()) == size {
            return Ok(src.clone());
        }
        let size = [size.0.max(1), size.1.max(1)];
        if src.width().max(src.height()).max(size[0]).max(size[1]) > MAX_INTEGER_RESIZE_SIDE {
            return Err("Paint resize dimensions exceed the GPU integer resampling limit".into());
        }
        let mut out = TileImage::new(size[0], size[1], src.comps());
        if src.allocated_count() == 0 {
            return Ok(out);
        }
        if !matches!(src.comps(), 2 | 4) {
            return Err("paint resize needs two or four components".into());
        }

        let limits = self.device.limits();
        let limit = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
        let tile_bytes = (TILE_AREA * src.comps() * 2) as u64;
        let tiles_per_job = (limit.min(OUTPUT_BYTES) / tile_bytes).max(1) as u32;
        let columns = out.tiles_x().min(tiles_per_job.isqrt().max(1));
        let rows = (tiles_per_job / columns).max(1);
        let mut work = Vec::new();
        for y in (0..out.tiles_y()).step_by(rows as usize) {
            for x in (0..out.tiles_x()).step_by(columns as usize) {
                work.push(TileWindow { origin: [x, y], size: [columns.min(out.tiles_x() - x), rows.min(out.tiles_y() - y)] });
            }
        }
        let mut packed = Vec::new();
        while let Some(dest) = work.pop() {
            if cancelled() {
                return Err("Paint resize was cancelled".into());
            }
            let (lo, hi) = source_footprint(src, size, dest);
            if !src.has_allocated_texels(lo, hi) {
                continue;
            }
            let source = TileWindow::covering(lo, hi, [src.width(), src.height()]);
            if let Some(words) = source.words(std::slice::from_ref(src), (limit.min(INPUT_BYTES) / 4) as usize) {
                source.pack(std::slice::from_ref(src), words, &mut packed);
                self.resize_block(src, &mut out, source, dest, empty, &packed)?;
                continue;
            }
            let axis = usize::from(dest.size[1] >= dest.size[0]);
            let half = dest.size[axis] / 2;
            if half == 0 {
                return Err("source tiles for one resize block exceed the GPU upload budget".into());
            }
            let (mut first, mut second) = (dest, dest);
            first.size[axis] = half;
            second.origin[axis] += half;
            second.size[axis] -= half;
            work.extend([second, first]);
        }
        Ok(out)
    }

    fn resize_block(
        &mut self,
        src: &TileImage,
        out: &mut TileImage,
        source: TileWindow,
        dest: TileWindow,
        empty: &[u16],
        packed: &[u32],
    ) -> Result<(), String> {
        let words_per_pixel = (src.comps() / 2) as u32;
        let row_bytes = u64::from(dest.size[0] * dest.size[1]) * (TILE_AREA * src.comps() * 2) as u64;
        let (device, queue) = (&self.device, &self.queue);
        let submission = crate::gpu_trace::scoped(device, "GPU paint resize", || {
            let input =
                buffer(device, &mut self.input, "tc_resize_input", (packed.len() * 4) as u64, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
            let result = buffer(device, &mut self.result, "tc_resize_result", row_bytes, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
            let readback = buffer(device, &mut self.readback, "tc_resize_readback", row_bytes, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
            let cleared = |i: usize| u32::from(empty.get(i).copied().unwrap_or(0));
            let params = [
                src.width(),
                src.height(),
                out.width(),
                out.height(),
                source.origin[0],
                source.origin[1],
                source.size[0],
                source.size[1],
                dest.origin[0],
                dest.origin[1],
                dest.size[0],
                dest.size[1],
                words_per_pixel,
                cleared(0) | (cleared(1) << 16),
                cleared(2) | (cleared(3) << 16),
                0,
            ];
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tc_resize_params"),
                contents: bytemuck::cast_slice(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            queue.write_buffer(&input, 0, bytemuck::cast_slice(packed));
            let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tc_resize_inputs"),
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: input.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: result.as_entire_binding() },
                ],
            });
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_paint_resize") });
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bindings, &[]);
                pass.dispatch_workgroups(dest.size[0] * (TILE / 8), dest.size[1] * (TILE / 8), 1);
            }
            encoder.copy_buffer_to_buffer(&result, 0, &readback, 0, row_bytes);
            (queue.submit([encoder.finish()]), readback)
        })?;
        let slice = submission.1.slice(..row_bytes);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device.poll(wgpu::PollType::Wait { submission_index: Some(submission.0), timeout: None }).map_err(|e| e.to_string())?;
        receiver.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
        let mapped = (|| {
            let data = slice.get_mapped_range().map_err(|e| e.to_string())?;
            let samples = bytemuck::try_cast_slice::<u8, u16>(&data).map_err(|e| e.to_string())?;
            for (slot, tile_data) in samples.chunks_exact(TILE_AREA * src.comps()).enumerate() {
                let x = dest.origin[0] + slot as u32 % dest.size[0];
                let y = dest.origin[1] + slot as u32 / dest.size[0];
                let tile = TileWindow { origin: [x, y], size: [1, 1] };
                let (lo, hi) = source_footprint(src, [out.width(), out.height()], tile);
                if src.has_allocated_texels(lo, hi) {
                    out.set_tile(x, y, Some(Arc::new(tile_data.iter().copied().map(u16::from_le).collect())));
                }
            }
            Ok(())
        })();
        submission.1.unmap();
        mapped
    }
}

fn source_footprint(src: &TileImage, size: [u32; 2], dest: TileWindow) -> ([u32; 2], [u32; 2]) {
    src.resize_footprint(size, std::array::from_fn(|axis| dest.origin[axis]..dest.origin[axis] + dest.size[axis]))
}

fn buffer(device: &wgpu::Device, slot: &mut Option<wgpu::Buffer>, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    if let Some(buffer) = slot.as_ref().filter(|buffer| buffer.size() >= size) {
        return buffer.clone();
    }
    let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
    *slot = Some(buffer.clone());
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bilinear_resize_preserves_sparse_color_and_signed_channel_tiles() {
        let Ok(gpu) = crate::headless::Headless::new() else { return };
        eprintln!("Paint resize adapter: {:?}", gpu.device.adapter_info());
        let mut accelerator = PaintResampler::new(&gpu.device, &gpu.queue).unwrap();
        for (components, empty) in [(4, &[0, 0, 0, 0][..]), (2, &[32768, 0][..])] {
            let mut image = TileImage::new(192, 128, components);
            image.set(63, 63, &[4096, 61440, 1000, 65535]);
            image.set(64, 64, &[60000, 30000, 5000, 65535]);
            image.set(191, 127, &[65535, 1, 65534, 32768]);
            for size in [(384, 256), (96, 64), (87, 135), (1, 1), (257, 1), (MAX_INTEGER_RESIZE_SIDE, 1)] {
                let actual = accelerator.resize(&image, size, empty, || false).unwrap();
                let expected = image.resampled(size.0, size.1, empty);
                assert_eq!(actual.allocated().collect::<Vec<_>>(), expected.allocated().collect::<Vec<_>>());
                for (x, y) in expected.allocated() {
                    let gpu = actual.tile(x, y).unwrap();
                    let cpu = expected.tile(x, y).unwrap();
                    assert_eq!(gpu, cpu, "components {components}, size {size:?}, tile {x}/{y}");
                }
            }
        }
    }
}
