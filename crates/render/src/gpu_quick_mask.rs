//! Compose the Quick Mask overlay from the viewport's current material and brush coverage.

use std::collections::BTreeMap;
use texturecraft_texel::{TILE, TileImage};
use wgpu::util::DeviceExt;

pub enum QuickMaskAlpha {
    Gpu(wgpu::TextureView),
    Cpu(TileImage),
}

pub struct QuickMaskOverlay {
    pub mask: TileImage,
    pub inverted: bool,
    pub stroke: Option<(QuickMaskAlpha, f32, bool)>,
}

pub(crate) struct QuickMaskGpu {
    pub(crate) pipeline: wgpu::RenderPipeline,
    empty: wgpu::TextureView,
    coverage: BTreeMap<usize, (TileImage, wgpu::TextureView)>,
}

impl QuickMaskGpu {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let texture_entry = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tc_quick_mask"),
            entries: &[
                texture_entry(0, wgpu::TextureSampleType::Float { filterable: false }),
                texture_entry(1, wgpu::TextureSampleType::Uint),
                texture_entry(2, wgpu::TextureSampleType::Uint),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tc_quick_mask"),
            bind_group_layouts: &[Some(&bindings)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tc_quick_mask"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/quick_mask.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tc_quick_mask"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, empty: coverage_texture(device, 1).create_view(&Default::default()), coverage: BTreeMap::new() }
    }

    pub(crate) fn retain_texture_sets(&mut self, count: usize) {
        self.coverage.retain(|&ts, _| ts < count);
    }

    pub(crate) fn bindings(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        ts: usize,
        size: u32,
        base: &wgpu::TextureView,
        overlay: &QuickMaskOverlay,
    ) -> wgpu::BindGroup {
        let mask = &overlay.mask;
        let committed = if mask.allocated_count() == 0 {
            self.coverage.remove(&ts);
            self.empty.clone()
        } else {
            if self.coverage.get(&ts).is_none_or(|(previous, _)| !previous.same_tiles(mask)) {
                self.coverage.remove(&ts);
                self.coverage.insert(ts, (mask.clone(), upload_coverage(device, queue, size, mask)));
            }
            self.coverage.get(&ts).map_or_else(|| self.empty.clone(), |(_, view)| view.clone())
        };
        let (stroke, opacity, erase) = match &overlay.stroke {
            Some((QuickMaskAlpha::Gpu(view), opacity, erase)) => (view.clone(), *opacity, *erase),
            Some((QuickMaskAlpha::Cpu(mask), opacity, erase)) => (upload_coverage(device, queue, size, mask), *opacity, *erase),
            None => (self.empty.clone(), 0.0, false),
        };
        let params = [size, u32::from(overlay.inverted), u32::from(erase), opacity.to_bits()];
        let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_quick_mask_params"),
            contents: bytemuck::cast_slice(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tc_quick_mask"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(base) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&committed) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&stroke) },
                wgpu::BindGroupEntry { binding: 3, resource: uniforms.as_entire_binding() },
            ],
        })
    }
}

fn coverage_texture(device: &wgpu::Device, size: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tc_quick_mask_coverage"),
        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R16Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn upload_coverage(device: &wgpu::Device, queue: &wgpu::Queue, size: u32, mask: &TileImage) -> wgpu::TextureView {
    let texture = coverage_texture(device, size);
    let (width, height) = (mask.width(), mask.height());
    let mut coverage = vec![0u16; (size * size) as usize];
    for (tx, ty) in mask.allocated() {
        let Some(tile) = mask.tile(tx, ty) else { continue };
        // Only visit samples in allocated tiles. Their source position matches the CPU overlay.
        let first_x = (tx * TILE * size).div_ceil(width);
        let last_x = (((tx + 1) * TILE).min(width) * size).div_ceil(width);
        let first_y = (ty * TILE * size).div_ceil(height);
        let last_y = (((ty + 1) * TILE).min(height) * size).div_ceil(height);
        for y in first_y..last_y {
            let row = y * height / size % TILE;
            for x in first_x..last_x {
                let column = x * width / size % TILE;
                coverage[(y * size + x) as usize] = tile[(row * TILE + column) as usize];
            }
        }
    }
    queue.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(&coverage),
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size * 2), rows_per_image: None },
        wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
    );
    texture.create_view(&Default::default())
}
