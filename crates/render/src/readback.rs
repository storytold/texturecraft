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
        let (width, height) = (texture.width(), texture.height());
        Readback::encode(device, encoder, texture, 0, width, height)
    }

    /// [`Readback::new`] for mip `level` of `texture`; None when `texture` has no such level.
    pub fn new_level(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture, level: u32) -> Option<Readback> {
        if level >= texture.mip_level_count() {
            return None;
        }

        let (width, height) = ((texture.width() >> level).max(1), (texture.height() >> level).max(1));
        Some(Readback::encode(device, encoder, texture, level, width, height))
    }

    /// Encode the copy of mip `level` (`width` × `height`, an existing level) of `texture`.
    fn encode(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture, level: u32, width: u32, height: u32) -> Readback {
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

    /// [`Readback::blocking`] for mip `level` (None also when `texture` has no such level).
    pub fn blocking_level(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture, level: u32) -> Option<(u32, u32, Vec<u8>)> {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_read") });
        let r = Readback::new_level(device, &mut enc, texture, level)?;
        queue.submit([enc.finish()]);
        r.map();
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        r.take().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::headless::Headless;

    /// An 8×4 RGBA8 texture with its full mip chain (8×4, 4×2, 2×1, 1×1), each level filled with
    /// its own grey `level * 50 + 10`.
    fn levelled(h: &Headless) -> wgpu::Texture {
        let texture = h.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tc_read_test"),
            size: wgpu::Extent3d { width: 8, height: 4, depth_or_array_layers: 1 },
            mip_level_count: 4,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for level in 0..4 {
            let (w, h_) = ((8 >> level).max(1), (4 >> level).max(1));
            let grey = u8::try_from(level * 50 + 10).unwrap();
            h.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &[grey, grey, grey, 255].repeat((w * h_) as usize),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h_) },
                wgpu::Extent3d { width: w, height: h_, depth_or_array_layers: 1 },
            );
        }
        texture
    }

    #[test]
    fn level_zero_is_the_plain_readback() {
        let Ok(h) = Headless::new() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };

        let tex = levelled(&h);
        let plain = Readback::blocking(&h.device, &h.queue, &tex).unwrap();
        assert_eq!(Readback::blocking_level(&h.device, &h.queue, &tex, 0), Some(plain.clone()));
        assert_eq!(plain, (8, 4, [10, 10, 10, 255].repeat(32)));
    }

    #[test]
    fn reads_every_level_up_to_the_last() {
        let Ok(h) = Headless::new() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };

        let tex = levelled(&h);
        assert_eq!(Readback::blocking_level(&h.device, &h.queue, &tex, 1), Some((4, 2, [60, 60, 60, 255].repeat(8))));
        assert_eq!(Readback::blocking_level(&h.device, &h.queue, &tex, 3), Some((1, 1, vec![160, 160, 160, 255])));
    }

    #[test]
    fn a_level_past_the_chain_is_none() {
        let Ok(h) = Headless::new() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };

        let tex = levelled(&h);
        assert_eq!(Readback::blocking_level(&h.device, &h.queue, &tex, 4), None);
        assert_eq!(Readback::blocking_level(&h.device, &h.queue, &tex, 40), None);
        let mut enc = h.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        assert!(Readback::new_level(&h.device, &mut enc, &tex, 4).is_none());
    }
}
