//! Reading RGBA8 textures back to the CPU: blocking (headless renders, screenshots) or without
//! stalling (the app polls until the copy is mapped).

use std::sync::{Arc, Mutex, PoisonError};

/// A texture copy on its way to the CPU.
pub struct Readback {
    buf: wgpu::Buffer,
    width: u32,
    height: u32,
    /// Bytes per buffer row (padded to the copy alignment).
    row: u32,
    /// Set by the map callback: whether mapping worked.
    mapped: Arc<Mutex<Option<bool>>>,
}

impl Readback {
    /// Encode the copy of mip 0 of `texture` (an RGBA8 format) into a mappable buffer.
    pub fn new(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) -> Readback {
        Readback::new_level(device, encoder, texture, 0)
    }

    /// [`Readback::new`] for mip `level` of `texture`.
    pub fn new_level(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture, level: u32) -> Readback {
        let (width, height) = ((texture.width() >> level).max(1), (texture.height() >> level).max(1));
        let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tc_read"),
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(height) },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        Readback { buf, width, height, row, mapped: Arc::new(Mutex::new(None)) }
    }

    /// Start mapping; call once, after the encoder holding the copy was submitted.
    pub fn map(&self) {
        let mapped = self.mapped.clone();
        self.buf.slice(..).map_async(wgpu::MapMode::Read, move |r| *mapped.lock().unwrap_or_else(PoisonError::into_inner) = Some(r.is_ok()));
    }

    /// The tightly packed pixels once mapped: None while pending, Some(None) when it failed.
    pub fn take(&self) -> Option<Option<(u32, u32, Vec<u8>)>> {
        let ok = (*self.mapped.lock().unwrap_or_else(PoisonError::into_inner))?;
        Some(ok.then(|| self.pixels()).flatten())
    }

    fn pixels(&self) -> Option<(u32, u32, Vec<u8>)> {
        let data = self.buf.slice(..).get_mapped_range().ok()?;
        let line = (self.width * 4) as usize;
        let mut out = Vec::with_capacity(line * self.height as usize);
        for y in 0..self.height as usize {
            let start = y * self.row as usize;
            out.extend_from_slice(data.get(start..start + line)?);
        }
        Some((self.width, self.height, out))
    }

    /// Copy, submit and wait (the web cannot block: there it returns None).
    pub fn blocking(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Option<(u32, u32, Vec<u8>)> {
        Readback::blocking_level(device, queue, texture, 0)
    }

    /// [`Readback::blocking`] for mip `level`.
    pub fn blocking_level(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture, level: u32) -> Option<(u32, u32, Vec<u8>)> {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_read") });
        let r = Readback::new_level(device, &mut enc, texture, level);
        queue.submit([enc.finish()]);
        r.map();
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        r.take().flatten()
    }
}
