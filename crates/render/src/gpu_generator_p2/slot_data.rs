use std::sync::Arc;

use texturecraft_compose::ImageRes;
use wgpu::util::DeviceExt;

use super::micro_request::MicroRequest;
use super::micro_slot::MicroSlot;
use super::{READS_HEIGHT, READS_NORMAL, slot_size};

/// A request's slots for the passes: the height image's gray (the mean of its channels, as
/// `Slot::heights` reads it) then the normal image's red and green, and the uniform values
pub(super) struct SlotData {
    pub(super) buffer: wgpu::Buffer,
    pub(super) normal_start: u32,
    pub(super) sizes: [u32; 4],
    pub(super) uniforms: [f32; 4],
    pub(super) reads: u32,
}

impl SlotData {
    pub(super) fn new(device: &wgpu::Device, request: &MicroRequest, images: &[Option<Arc<ImageRes>>; 2], limit: u64) -> Result<Self, String> {
        let [height_image, normal_image] = images.each_ref().map(Option::as_ref);
        let mut words: Vec<f32> =
            height_image.map_or_else(Vec::new, |image| image.rgba.iter().map(|[red, green, blue, _]| (red + green + blue) / 3.0).collect());
        let normal_start = words.len() as u32;
        if let Some(image) = normal_image {
            words.extend(image.rgba.iter().flat_map(|[red, green, _, _]| [*red, *green]));
        }
        words.resize(words.len().max(4), 0.0);
        if (words.len() * 4) as u64 > limit {
            return Err("A Micro Details image exceeds the GPU storage-buffer limit.".into());
        }

        // An image slot whose image is not loaded reads black
        let uniform = |slot: &Option<MicroSlot>| match slot {
            Some(MicroSlot::Uniform(value)) => *value,
            _ => [0.0; 3],
        };
        let height = uniform(&request.height);
        let normal = uniform(&request.normal);
        let flip_green = matches!(request.normal, Some(MicroSlot::Image { flip_green: true, .. })) && normal_image.is_some();
        let [height_size, normal_size] = [slot_size(height_image), slot_size(normal_image)];
        let reads = if request.height.is_some() { READS_HEIGHT } else { 0 } | if request.normal.is_some() { READS_NORMAL } else { 0 };
        Ok(Self {
            buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tc_micro_slots"),
                contents: bytemuck::cast_slice(&words),
                usage: wgpu::BufferUsages::STORAGE,
            }),
            normal_start,
            sizes: [height_size[0], height_size[1], normal_size[0], normal_size[1]],
            uniforms: [height.iter().sum::<f32>() / 3.0, normal[0] * 2.0 - 1.0, normal[1] * 2.0 - 1.0, if flip_green { -1.0 } else { 1.0 }],
            reads,
        })
    }
}
