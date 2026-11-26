use std::collections::BTreeMap;
use std::sync::Arc;

use bytemuck::Zeroable;
use wgpu::util::DeviceExt;

use super::super::CompositeProgram;
use super::dummies::Dummies;
use super::field_buffers::FieldBuffers;
use super::micro_cache::MicroCache;
use super::micro_key::MicroKey;
use super::micro_maps::MicroMaps;
use super::params::Params;
use super::run::Run;
use super::slot_data::SlotData;
use super::{WORKGROUP, block_groups, dispatch, texture};
use crate::gpu_trace::{entry, storage};

/// The micro detail passes, and each texture set's last maps
pub(crate) struct MicroGpu {
    pub(super) layout: wgpu::BindGroupLayout,
    pub(super) sample: wgpu::ComputePipeline,
    pub(super) tilt: wgpu::ComputePipeline,
    pub(super) blur: wgpu::ComputePipeline,
    pub(super) accumulate: wgpu::ComputePipeline,
    pub(super) term: wgpu::ComputePipeline,
    pub(super) occlusion: wgpu::ComputePipeline,
    pub(super) adjust: wgpu::ComputePipeline,
    pub(super) cache: BTreeMap<usize, MicroCache>,
}

impl MicroGpu {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let float_array = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tc_micro_inputs"),
            entries: &[
                entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
                storage(1, true),
                storage(2, false),
                storage(3, false),
                storage(4, false),
                storage(5, false),
                storage(6, false),
                storage(7, false),
                entry(
                    8,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::R32Float,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                    },
                ),
                entry(
                    9,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(10, float_array),
                entry(
                    11,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba32Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                ),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tc_micro_pipeline"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::include_wgsl!("../shaders/generator_p2_micro.wgsl"));
        let pipeline = |name| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(name),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(name),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            sample: pipeline("sample_slots"),
            tilt: pipeline("tilt"),
            blur: pipeline("blur"),
            accumulate: pipeline("accumulate"),
            term: pipeline("term"),
            occlusion: pipeline("occlusion"),
            adjust: pipeline("adjust_bake"),
            layout,
            cache: BTreeMap::new(),
        }
    }

    /// The micro maps `program` reads on texture set `ts`, built on first use and kept while its
    /// requests, images and bake stay the same (None without micro details). `baked` is the
    /// program's bake texture, `supported` whether the composite binds micro maps.
    pub(crate) fn maps(
        slot: &mut Option<MicroGpu>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        (ts, supported): (usize, bool),
        program: &CompositeProgram,
        baked: &wgpu::TextureView,
    ) -> Result<Option<Arc<MicroMaps>>, String> {
        let plan = &program.micro;
        if plan.is_empty() {
            if let Some(gpu) = slot {
                gpu.cache.remove(&ts);
            }
            return Ok(None);
        }
        if !supported {
            return Err("Micro Details exceed the GPU adapter's binding limits.".into());
        }

        let bake = program.baked.as_ref();
        let width = bake.map_or(1, |baked| baked.width);
        let map_rows = |index: usize| bake.and_then(|baked| baked.maps[index].as_ref().map(|_| baked.heights[index])).unwrap_or(0);
        let (occlusion_rows, curvature_rows) = (map_rows(0), map_rows(2));
        let grid = program.udim_layout.as_ref().map_or([1, 1], |layout| [layout.grid.cols, layout.grid.rows]);
        let images = plan.images.iter().map(|pair| pair.each_ref().map(|image| image.as_ref().map(Arc::downgrade))).collect();
        let maps = [0, 2].map(|index| bake.and_then(|baked| baked.maps[index].as_ref().map(Arc::downgrade)));
        let key = MicroKey { requests: plan.requests.clone(), images, maps, size: [width, occlusion_rows, curvature_rows], grid, levels: plan.levels };
        let gpu = slot.get_or_insert_with(|| MicroGpu::new(device));
        if let Some(cache) = gpu.cache.get(&ts).filter(|cache| cache.key.matches(&key)) {
            return Ok(Some(cache.maps.clone()));
        }

        gpu.cache.remove(&ts);
        let maps = Arc::new(gpu.build(device, queue, program, baked, (width, occlusion_rows, curvature_rows), grid)?);
        gpu.cache.insert(ts, MicroCache { key, maps: maps.clone() });
        Ok(Some(maps))
    }

    pub(super) fn build(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        program: &CompositeProgram,
        baked: &wgpu::TextureView,
        (width, occlusion_rows, curvature_rows): (u32, u32, u32),
        grid: [u32; 2],
    ) -> Result<MicroMaps, String> {
        let plan = &program.micro;
        let limits = device.limits();
        // The row counts sit after the tallest map's rows, after one row where no map is baked
        let tallest = occlusion_rows.max(curvature_rows).max(1);
        let layers = (plan.requests.len() * 2) as u32;
        if width.max(tallest + 1) > limits.max_texture_dimension_2d || layers > limits.max_texture_array_layers {
            return Err("Micro Details exceed the GPU texture dimensions.".into());
        }

        let usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let maps = texture(device, "tc_micro_maps", [width, tallest + 1, layers], wgpu::TextureFormat::R32Float, usage);
        let maps_view = maps.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let dummy = Dummies::new(device);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_micro") });

        for (index, request) in plan.requests.iter().enumerate() {
            // Images not resolved yet (`set_tile_images`) read as not loaded
            let images = plan.images.get(index).cloned().unwrap_or_default();
            let slots = SlotData::new(device, request, &images, limits.max_storage_buffer_binding_size.min(limits.max_buffer_size))?;
            let layers = [(request.curvature, curvature_rows, 0), (request.occlusion, occlusion_rows, 1)];
            for (built, rows, offset) in layers {
                let layer = index as u32 * 2 + offset;

                // Each layer's own row count, after the tallest map's rows
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &maps,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x: 0, y: tallest, z: layer },
                        aspect: wgpu::TextureAspect::All,
                    },
                    bytemuck::bytes_of(&(rows as f32)),
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: None, rows_per_image: None },
                    wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                );
                if !built {
                    continue;
                }

                // A map not baked reads neutral, its one row: no curvature term, no occlusion
                if rows == 0 {
                    if offset == 1 {
                        queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &maps,
                                mip_level: 0,
                                origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                                aspect: wgpu::TextureAspect::All,
                            },
                            bytemuck::cast_slice(&vec![1.0f32; width as usize]),
                            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: None, rows_per_image: None },
                            wgpu::Extent3d { width, height: 1, depth_or_array_layers: 1 },
                        );
                    }
                    continue;
                }
                for group in block_groups([width, rows], grid) {
                    let run = Run { gpu: self, device, slots: &slots, dummy: &dummy, target: &maps_view, group: &group, layer };
                    run.fits(limits.max_storage_buffer_binding_size.min(limits.max_buffer_size))?;
                    if offset == 0 {
                        run.curvature(&mut encoder, request);
                    } else {
                        run.occlusion(&mut encoder, request);
                    }
                }
            }
        }

        // The detail sizes blur the bake with the claimed micro curvature added
        let adjusted = match plan.levels {
            Some(Some(index)) if curvature_rows > 0 => {
                let bake = texture(
                    device,
                    "tc_micro_bake",
                    [width, curvature_rows, 1],
                    wgpu::TextureFormat::Rgba32Uint,
                    wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                );
                let view = bake.create_view(&Default::default());
                let params = Params { place: [0, 0, index as u32 * 2, 0], ..Params::zeroed() };
                let bindings = self.bindings(device, &params, &dummy, (&dummy.maps_out, baked, &maps_view, &view));
                dispatch(&mut encoder, &self.adjust, &bindings, [width.div_ceil(WORKGROUP), curvature_rows.div_ceil(WORKGROUP)]);
                Some(view)
            }
            _ => None,
        };
        queue.submit([encoder.finish()]);
        Ok(MicroMaps { maps: maps_view, bake: adjusted })
    }

    /// A pass's bind group: `params`, the run's fields and the images (`maps_out`, `bake`,
    /// `maps_in`, `bake_out`)
    pub(super) fn bindings(
        &self,
        device: &wgpu::Device,
        params: &Params,
        fields: &dyn FieldBuffers,
        (maps_out, bake, maps_in, bake_out): (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView),
    ) -> wgpu::BindGroup {
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_micro_params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let buffers = fields.buffers();
        let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }];
        entries
            .extend(buffers.iter().enumerate().map(|(index, buffer)| wgpu::BindGroupEntry { binding: index as u32 + 1, resource: buffer.as_entire_binding() }));
        entries.extend([
            wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(maps_out) },
            wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(bake) },
            wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(maps_in) },
            wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(bake_out) },
        ]);
        device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("tc_micro_pass"), layout: &self.layout, entries: &entries })
    }
}
