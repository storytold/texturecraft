use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

use bytemuck::Zeroable;
use texturecraft_core::LayerId;
use wgpu::util::DeviceExt;

use super::super::CompositeProgram;
use super::capture_layout::CaptureLayout;
use super::dummies::Dummies;
use super::field_buffers::FieldBuffers;
use super::micro_cache::MicroCache;
use super::micro_key::MicroKey;
use super::micro_maps::MicroMaps;
use super::micro_slot::MicroSlot;
use super::params::Params;
use super::run::{Run, Scratch};
use super::slot_data::SlotData;
use super::{LINE_WORKGROUP, WORKGROUP, block_groups, dispatch, single_blocks, texture};
use crate::gpu_curvature::CurvatureLevels;
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
                entry(
                    12,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
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
    /// program's bake texture, `supported` whether the composite binds micro maps, `captures`
    /// the anchor heights its capture programs composited (maps reading them are built anew).
    pub(crate) fn maps(
        slot: &mut Option<MicroGpu>,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        (ts, supported): (usize, bool),
        program: &CompositeProgram,
        (baked, captures): (&wgpu::TextureView, &[(LayerId, wgpu::TextureView)]),
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
        let key = MicroKey {
            requests: plan.requests.clone(),
            images,
            maps,
            size: [width, occlusion_rows, curvature_rows],
            grid,
            level_sets: plan.level_sets.clone(),
            gbuf: program.surface.as_ref().map(|(_, gbuf)| Arc::downgrade(gbuf)),
            needed: program.curvature_sizes,
        };
        let gpu = slot.get_or_insert_with(|| MicroGpu::new(device));
        if let Some(cache) = gpu.cache.get(&ts).filter(|cache| captures.is_empty() && cache.key.matches(&key)) {
            return Ok(Some(cache.maps.clone()));
        }

        gpu.cache.remove(&ts);
        let maps = Arc::new(gpu.build((device, queue), program, (baked, captures), (width, occlusion_rows, curvature_rows), grid)?);

        // Anchor heights change with the stack below them, which the key does not see
        if captures.is_empty() {
            gpu.cache.insert(ts, MicroCache { key, maps: maps.clone() });
        }
        Ok(Some(maps))
    }

    pub(super) fn build(
        &self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        program: &CompositeProgram,
        (baked, captures): (&wgpu::TextureView, &[(LayerId, wgpu::TextureView)]),
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
        let capture_layout = capture_layout(program, captures);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_micro") });
        let scratch = Scratch::default();

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
                // An anchor's heights differ in every block: each is computed alone
                let capture = match request.height {
                    Some(MicroSlot::Anchor(anchor)) => captures.iter().find(|(captured, _)| *captured == anchor).map(|(_, heights)| heights),
                    _ => None,
                };
                let groups = block_groups([width, rows], grid);
                let groups = if capture.is_some() { single_blocks(groups) } else { groups };
                for group in groups {
                    let capture = capture.map(|heights| (heights, &capture_layout, [width, rows]));
                    let run = Run { gpu: self, device, slots: &slots, dummy: &dummy, target: &maps_view, group: &group, layer, capture, scratch: &scratch };
                    run.fits(limits.max_storage_buffer_binding_size.min(limits.max_buffer_size))?;
                    if offset == 0 {
                        run.curvature(&mut encoder, request);
                    } else {
                        run.occlusion(&mut encoder, request);
                    }
                }
            }
        }

        // Each micro curvature the detail sizes read, added to the bake they blur
        let read_curvature = if curvature_rows > 0 && program.curvature_sizes.contains(&true) { &plan.level_sets } else { &BTreeSet::new() };
        let mut bakes = Vec::new();
        for &index in read_curvature {
            let bake = texture(
                device,
                "tc_micro_bake",
                [width, curvature_rows, 1],
                wgpu::TextureFormat::Rgba32Uint,
                wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            let view = bake.create_view(&Default::default());
            let params = Params { place: [0, 0, index as u32 * 2, 0], ..Params::zeroed() };
            let bindings = self.bindings(device, &params, &dummy, (&dummy.maps_out, baked, &maps_view, &view, &dummy.capture));
            dispatch(&mut encoder, &self.adjust, &bindings, [width.div_ceil(WORKGROUP), curvature_rows.div_ceil(WORKGROUP)]);
            bakes.push((index, view));
        }
        queue.submit([encoder.finish()]);
        Ok(MicroMaps { maps: maps_view, bakes, levels: OnceLock::new() })
    }

    /// Copy each micro curvature's detail sizes (`sets`, in the order of `maps.bakes`) into one
    /// atlas the composite reads (`generator_p2_levels_read.wgsl`): row 0 names each request's
    /// header row and the layout's level count, then each set's levels and its header row
    pub(crate) fn store_levels(&self, device: &wgpu::Device, queue: &wgpu::Queue, maps: &MicroMaps, sets: &[CurvatureLevels]) -> Result<(), String> {
        let Some(first) = sets.first() else { return Ok(()) };

        // Every set's layout has the same vec4s: its levels, then the bake's size
        let vec4s = (first.layout.size() / 16) as u32;
        let requests = maps.bakes.iter().map(|(index, _)| index + 1).max().unwrap_or(0) as u32;
        let extents: Vec<wgpu::Extent3d> = sets.iter().map(|set| set.image.texture().size()).collect();
        let width = extents.iter().map(|extent| extent.width).max().unwrap_or(1).max(4 * vec4s).max(requests + 1);
        let height = extents.iter().fold(1u32, |rows, extent| rows.saturating_add(extent.height + 1));
        if width.max(height) > device.limits().max_texture_dimension_2d {
            return Err("Micro curvature detail sizes exceed the GPU texture dimensions.".into());
        }

        let usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let atlas = texture(device, "tc_micro_levels", [width, height, 1], wgpu::TextureFormat::R32Uint, usage);
        let view = atlas.create_view(&Default::default());
        let (layout, copy, header) = levels_pipelines(device, vec4s);
        let mut directory = vec![0u32; width as usize];
        directory[width as usize - 1] = vec4s - 1;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_micro_levels") });
        let mut row = 1;
        for (((index, _), set), extent) in maps.bakes.iter().zip(sets).zip(&extents) {
            let header_row = row + extent.height;
            directory[*index] = header_row;
            let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tc_micro_levels_params"),
                contents: bytemuck::bytes_of(&[row, header_row, 0, 0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tc_micro_levels"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&set.image) },
                    wgpu::BindGroupEntry { binding: 1, resource: set.layout.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 3, resource: params.as_entire_binding() },
                ],
            });
            dispatch(&mut encoder, &copy, &bindings, [extent.width.div_ceil(WORKGROUP), extent.height.div_ceil(WORKGROUP)]);
            dispatch(&mut encoder, &header, &bindings, [(4 * vec4s).div_ceil(LINE_WORKGROUP), 1]);
            row = header_row + 1;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &atlas, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&directory),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: None, rows_per_image: None },
            wgpu::Extent3d { width, height: 1, depth_or_array_layers: 1 },
        );
        queue.submit([encoder.finish()]);
        let _ = maps.levels.set(view);
        Ok(())
    }

    /// A pass's bind group: `params`, the run's fields and the images (`maps_out`, `bake`,
    /// `maps_in`, `bake_out`)
    pub(super) fn bindings(
        &self,
        device: &wgpu::Device,
        params: &Params,
        fields: &dyn FieldBuffers,
        (maps_out, bake, maps_in, bake_out, capture): (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView),
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
            wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(capture) },
        ]);
        device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("tc_micro_pass"), layout: &self.layout, entries: &entries })
    }
}

/// The levels atlas's bind group layout, and its copy and header pipelines for layouts of `vec4s`
/// vec4s (`generator_p2_levels.wgsl`)
fn levels_pipelines(device: &wgpu::Device, vec4s: u32) -> (wgpu::BindGroupLayout, wgpu::ComputePipeline, wgpu::ComputePipeline) {
    let uniform = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("tc_micro_levels_inputs"),
        entries: &[
            entry(
                0,
                wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            ),
            entry(1, uniform),
            entry(
                2,
                wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::R32Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
            ),
            entry(3, uniform),
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("tc_micro_levels_pipeline"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let source = format!("const LAYOUT_VEC4S: u32 = {vec4s}u;\n{}", include_str!("../shaders/generator_p2_levels.wgsl"));
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("tc_micro_levels"), source: wgpu::ShaderSource::Wgsl(source.into()) });
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
    let (copy, header) = (pipeline("copy_levels"), pipeline("write_header"));
    (layout, copy, header)
}

/// Where `captures`' anchor heights lie in `program`'s working atlas
fn capture_layout(program: &CompositeProgram, captures: &[(LayerId, wgpu::TextureView)]) -> CaptureLayout {
    let working = captures.first().map_or([1, 1], |(_, heights)| {
        let size = heights.texture().size();
        [size.width, size.height]
    });
    match program.udim_layout.as_ref().filter(|layout| layout.scaled) {
        Some(layout) => CaptureLayout { working, block: layout.block, scales: layout.scales.clone(), columns: layout.grid.cols },
        None => CaptureLayout { working, block: 1, scales: Vec::new(), columns: 1 },
    }
}
