//! The screen-pinned image and camera shared by live painting and stroke readback.

use bytemuck::{Pod, Zeroable};
#[cfg(not(target_arch = "wasm32"))]
use texturecraft_compose::{ImageRes, Stencil};
#[cfg(not(target_arch = "wasm32"))]
use texturecraft_geom::glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Placement {
    view_proj: [[f32; 4]; 4],
    center_half: [f32; 4],
    rotation_flags: [f32; 4],
}

/// GPU resources for an immutable projection image and its placement at stroke start.
#[derive(Clone, Debug)]
pub struct GpuStencil {
    pub(crate) image: wgpu::TextureView,
    pub(crate) uniforms: wgpu::Buffer,
}

impl GpuStencil {
    fn from_placement(device: &wgpu::Device, image: wgpu::TextureView, placement: Placement) -> Self {
        let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_stencil_placement"),
            contents: bytemuck::bytes_of(&placement),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        Self { image, uniforms }
    }

    pub(crate) fn empty(device: &wgpu::Device) -> Self {
        Self::from_placement(device, image_texture(device, 1, 1).create_view(&Default::default()), Placement::zeroed())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn new(device: &wgpu::Device, image: wgpu::TextureView, stencil: &Stencil, center: Vec3, radius: f32) -> Self {
        let (sin, cos) = (-stencil.rotation).sin_cos();
        let [x, y] = stencil.center;
        let [w, h] = stencil.half;
        // The surface buffer contains (world - center) / radius. Fold the inverse
        // into the captured camera once, so each GPU sample can use that buffer.
        let world = Mat4::from_scale_rotation_translation(Vec3::splat(radius), texturecraft_geom::glam::Quat::IDENTITY, center);
        let placement = Placement {
            view_proj: (stencil.view_proj * world).to_cols_array_2d(),
            center_half: [x, y, w.max(1e-6), h.max(1e-6)],
            rotation_flags: [sin, cos, u32::from(stencil.tile) as f32, u32::from(stencil.image.srgb) as f32],
        };
        Self::from_placement(device, image, placement)
    }
}

fn image_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tc_stencil_image"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn upload_image(device: &wgpu::Device, queue: &wgpu::Queue, image: &ImageRes) -> Result<wgpu::TextureView, String> {
    let (width, height) = (image.width, image.height);
    let limit = device.limits().max_texture_dimension_2d;
    if width == 0 || height == 0 || width.max(height) > limit {
        return Err("The projection image exceeds the adapter's texture layout limits.".into());
    }
    let texture = image_texture(device, width, height);
    for top in (0..height).step_by(64) {
        let rows = (height - top).min(64);
        let first = top as usize * width as usize;
        let end = first + rows as usize * width as usize;
        let padded;
        let rgba = if let Some(rgba) = image.rgba.get(first..end) {
            rgba
        } else {
            // Missing texels have the same transparent value as ImageRes::sample.
            let mut pixels = vec![[0.0; 4]; end - first];
            if let Some(rest) = image.rgba.get(first..) {
                pixels[..rest.len()].copy_from_slice(rest);
            }
            padded = pixels;
            &padded
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { origin: wgpu::Origin3d { x: 0, y: top, z: 0 }, ..texture.as_image_copy() },
            bytemuck::cast_slice(rgba),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 16), rows_per_image: None },
            wgpu::Extent3d { width, height: rows, depth_or_array_layers: 1 },
        );
    }
    Ok(texture.create_view(&Default::default()))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn readback_pipeline(device: &wgpu::Device) -> wgpu::ComputePipeline {
    use crate::gpu_trace::{entry, storage};
    let texture =
        |binding, sample_type| entry(binding, wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false });
    let inputs = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("tc_projection_readback_inputs"),
        entries: &[
            texture(0, wgpu::TextureSampleType::Uint),
            texture(1, wgpu::TextureSampleType::Uint),
            storage(2, true),
            texture(3, wgpu::TextureSampleType::Float { filterable: false }),
            entry(4, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
            storage(5, true),
            storage(6, false),
        ],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("tc_projection_readback"),
        bind_group_layouts: &[Some(&inputs)],
        immediate_size: 0,
    });
    let source = concat!(include_str!("shaders/stencil_projection.wgsl"), "\n", include_str!("shaders/stencil_readback.wgsl"));
    let module =
        device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("tc_projection_readback"), source: wgpu::ShaderSource::Wgsl(source.into()) });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("tc_projection_readback"),
        layout: Some(&layout),
        module: &module,
        entry_point: Some("read_projection"),
        compilation_options: Default::default(),
        cache: None,
    })
}
