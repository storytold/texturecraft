//! Builds the whole-map generator fields on the GPU (P3b, `gpu_generator_p3b.rs`): the passes
//! of shaders/generator_p3b.wgsl, one array layer per field, kept while the program reads the
//! same fields of the same mesh, coverage and bake.

use std::collections::BTreeMap;
use std::sync::{Arc, Weak};

use texturecraft_compose::field_recipe as recipe;
use texturecraft_core::{BorderDistanceParameters, DrippingRustParameters, KindControls, UvCheckerParameters};
use texturecraft_geom::{Mesh, TrianglePartition, UvGBuffer};
use texturecraft_texel::linear_to_srgb;
use wgpu::util::DeviceExt;

use super::field_plan::FieldPlan;
use super::field_request::FieldRequest;
use super::params::FieldParams;
use crate::gpu_composite::udim::UdimLayout;
use crate::gpu_trace::{entry, storage};

const WORKGROUP: u32 = 8;
const LINE_WORKGROUP: u32 = 64;
/// The box passes per axis of a blur (compose's `BLUR_BOX_PASSES`)
const BOX_PASSES: usize = 3;
/// Scratch planes a Dripping Rust field needs: its values, the blurred sums, the cover, the blurred cover and a blur's middle pass
const SCRATCH_PLANES: usize = 5;
/// Taps per uniform vector (`Params.taps` in WGSL)
const TAPS_PER_VECTOR: usize = 2;

/// The fields built last: their mesh, coverage and occlusion bake, requests, material and UV tile layout, and their array view
type Built = (Weak<Mesh>, Weak<UvGBuffer>, Option<Weak<Vec<f32>>>, Vec<FieldRequest>, usize, Option<UdimLayout>, wgpu::TextureView);

struct Kernels {
    layout: wgpu::BindGroupLayout,
    random_color: wgpu::ComputePipeline,
    distance_rows: wgpu::ComputePipeline,
    distance_columns: wgpu::ComputePipeline,
    border_gradient: wgpu::ComputePipeline,
    box_pass: wgpu::ComputePipeline,
    keep_covered: wgpu::ComputePipeline,
    cover: wgpu::ComputePipeline,
    over_cover: wgpu::ComputePipeline,
    rust_seed: wgpu::ComputePipeline,
    drips: wgpu::ComputePipeline,
    checker: wgpu::ComputePipeline,
}

/// The compositor's whole-map field builder: its kernels once a program needs them, an empty
/// array for programs without fields, and the fields built last for each texture set's material
/// and mask preview
pub(crate) struct GeneratorFieldsGpu {
    kernels: Option<Kernels>,
    empty: wgpu::TextureView,
    built: BTreeMap<(usize, bool), Built>,
}

/// One build's fixed inputs: the device, the shared bindings and the base uniform
struct Pass<'a> {
    device: &'a wgpu::Device,
    kernels: &'a Kernels,
    surface_uv: &'a wgpu::TextureView,
    corners: &'a wgpu::Buffer,
    occlusion: &'a wgpu::TextureView,
    blank: &'a wgpu::TextureView,
    /// Each block's tile scale row by row, the block side they are kept by and the grid's columns (BlockScales; empty when every tile fills its block)
    scales: (&'a [f32], u32, u32),
    block_scales: &'a wgpu::Buffer,
    /// Radius 0 for every block, for the passes that do not blur
    no_radii: &'a wgpu::Buffer,
    base: FieldParams,
}

fn plane(device: &wgpu::Device, label: &str, (width, height): (u32, u32), layers: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: layers },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn layer_view(texture: &wgpu::Texture, layer: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2),
        base_array_layer: layer,
        array_layer_count: Some(1),
        ..Default::default()
    })
}

impl Kernels {
    fn new(device: &wgpu::Device) -> Self {
        let float = |binding| {
            entry(
                binding,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            )
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tc_generator_fields"),
            entries: &[
                entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
                entry(
                    1,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                storage(2, true),
                storage(3, true),
                float(4),
                float(5),
                float(6),
                entry(
                    7,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::R32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                ),
                float(8),
                storage(9, true),
                storage(10, true),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tc_generator_fields"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let source = [include_str!("shaders/noise.wgsl"), include_str!("shaders/generator_p3b.wgsl")].join("\n");
        let module =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("tc_generator_fields"), source: wgpu::ShaderSource::Wgsl(source.into()) });
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
            random_color: pipeline("random_color"),
            distance_rows: pipeline("distance_rows"),
            distance_columns: pipeline("distance_columns"),
            border_gradient: pipeline("border_gradient"),
            box_pass: pipeline("box_pass"),
            keep_covered: pipeline("keep_covered"),
            cover: pipeline("cover"),
            over_cover: pipeline("over_cover"),
            rust_seed: pipeline("rust_seed"),
            drips: pipeline("drips"),
            checker: pipeline("checker"),
            layout,
        }
    }
}

impl Pass<'_> {
    /// Workgroups covering every texel
    fn texels(&self) -> (u32, u32) {
        (self.base.size[0].div_ceil(WORKGROUP), self.base.size[1].div_ceil(WORKGROUP))
    }

    /// The UV tile scale of the block holding working texel (`x`, `y`) (BlockScales::at)
    fn scale_at(&self, x: u32, y: u32) -> f32 {
        let (scales, side, columns) = self.scales;
        if scales.is_empty() {
            return 1.0;
        }

        let column = x / side.max(1);
        let index = (y / side.max(1)) as usize * columns as usize + column as usize;
        if column >= columns { 1.0 } else { scales.get(index).copied().unwrap_or(1.0) }
    }

    /// The box radius `radius_of` gives each working block's UV tile texels, row by row (blur_blocks reads them at the block's corner)
    fn block_radii(&self, radius_of: impl Fn(f32) -> usize) -> Vec<u32> {
        let [width, height, block_width, block_height] = self.base.size;
        (0..height.div_ceil(block_height))
            .flat_map(|row| (0..width.div_ceil(block_width)).map(move |column| (column * block_width, row * block_height)))
            .map(|(left, top)| radius_of(block_width as f32 * self.scale_at(left, top)) as u32)
            .collect()
    }

    /// Run `pipeline` reading `inputs` (unused ones blank) and the island table, writing `output`
    fn run(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        params: &FieldParams,
        (inputs, islands): ([Option<&wgpu::TextureView>; 3], &wgpu::Buffer),
        output: &wgpu::TextureView,
        groups: (u32, u32),
    ) {
        self.run_with_radii(encoder, pipeline, params, (inputs, islands, self.no_radii), output, groups);
    }

    /// [`Self::run`] with each block's box radius in `radii`
    fn run_with_radii(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        params: &FieldParams,
        (inputs, islands, radii): ([Option<&wgpu::TextureView>; 3], &wgpu::Buffer, &wgpu::Buffer),
        output: &wgpu::TextureView,
        groups: (u32, u32),
    ) {
        let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_generator_field_params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let input = |index: usize| wgpu::BindingResource::TextureView(inputs[index].unwrap_or(self.blank));
        let bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tc_generator_field_pass"),
            layout: &self.kernels.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(self.surface_uv) },
                wgpu::BindGroupEntry { binding: 2, resource: self.corners.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: islands.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: input(0) },
                wgpu::BindGroupEntry { binding: 5, resource: input(1) },
                wgpu::BindGroupEntry { binding: 6, resource: input(2) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(output) },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(self.occlusion) },
                wgpu::BindGroupEntry { binding: 9, resource: self.block_scales.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 10, resource: radii.as_entire_binding() },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        pass.dispatch_workgroups(groups.0, groups.1, 1);
    }

    /// Three box passes of each block's radius in `radii` along the block's rows, then its columns (blur_blocks): `source` into `target` through `middle`
    fn blur(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        islands: &wgpu::Buffer,
        radii: &wgpu::Buffer,
        (source, middle, target): (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView),
    ) {
        let mut read = source;
        for pass in 0..2 * BOX_PASSES {
            let axis = u32::from(pass >= BOX_PASSES);
            let params = FieldParams { words: [0, axis, 0, 0], ..self.base };
            let written = if pass % 2 == 0 { middle } else { target };
            self.run_with_radii(encoder, &self.kernels.box_pass, &params, ([Some(read), None, None], islands, radii), written, self.texels());
            read = written;
        }
    }

    /// The blocks' radii as a buffer, None when no block blurs
    fn radii_buffer(&self, radii: &[u32]) -> Option<wgpu::Buffer> {
        radii.iter().any(|&radius| radius > 0).then(|| crate::gpu_trace::buffer(self.device, "tc_generator_field_radii", bytemuck::cast_slice(radii)))
    }

    /// blur_over_cover: the values in plane `values` blurred over the covered texels by each
    /// block's radius in `radii`; the index of the plane holding the result (`values` itself
    /// when no block blurs)
    fn blur_over_cover(&self, encoder: &mut wgpu::CommandEncoder, islands: &wgpu::Buffer, radii: &[u32], planes: &[wgpu::TextureView], values: usize) -> usize {
        let Some(radii) = self.radii_buffer(radii) else { return values };

        let [sums, cover, blurred_cover, middle] = others(values);
        self.blur(encoder, islands, &radii, (&planes[values], &planes[middle], &planes[sums]));
        self.run(encoder, &self.kernels.cover, &self.base, ([None, None, None], islands), &planes[cover], self.texels());
        self.blur(encoder, islands, &radii, (&planes[cover], &planes[middle], &planes[blurred_cover]));
        let params = FieldParams { values: [[recipe::MIN_COVERAGE, 0.0, 0.0, 0.0], [0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4]], ..self.base };
        let inputs = [Some(&planes[values]), Some(&planes[sums]), Some(&planes[blurred_cover])];
        self.run(encoder, &self.kernels.over_cover, &params, (inputs, islands), &planes[cover], self.texels());
        cover
    }
}

/// The scratch planes other than `plane`, in order
fn others(plane: usize) -> [usize; SCRATCH_PLANES - 1] {
    let mut others = [0; SCRATCH_PLANES - 1];
    for (slot, index) in others.iter_mut().zip((0..SCRATCH_PLANES).filter(|&index| index != plane)) {
        *slot = index;
    }
    others
}

/// The island table of a field: Random Color's gray in x, Random Colors' light sRGB colour in yzw
fn island_table(device: &wgpu::Device, request: &FieldRequest, islands: usize) -> wgpu::Buffer {
    let rows: Vec<[f32; 4]> = (0..islands.max(1) as u32)
        .map(|island| match &request.controls {
            KindControls::UvRandomColor(random) => [recipe::random_gray(random, request.seed, island), 0.0, 0.0, 0.0],
            KindControls::UvChecker(checker) if checker.random_colors => {
                let [red, green, blue] = recipe::island_color(request.seed, island);
                [0.0, red, green, blue]
            }
            _ => [0.0; 4],
        })
        .collect();
    crate::gpu_trace::buffer(device, "tc_generator_field_islands", bytemuck::cast_slice(&rows))
}

impl GeneratorFieldsGpu {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let empty = plane(device, "tc_generator_fields_empty", (1, 1), 1)
            .create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        Self { kernels: None, empty, built: BTreeMap::new() }
    }

    /// Drop the fields of texture sets from `count` on
    pub(crate) fn retain_texture_sets(&mut self, count: usize) {
        self.built.retain(|&(ts, _), _| ts < count);
    }

    /// Drop the fields of texture set `ts`
    pub(crate) fn release(&mut self, ts: usize) {
        self.built.retain(|&(built, _), _| built != ts);
    }

    /// Drop the fields whose mesh or UV coverage is gone
    pub(crate) fn release_unused(&mut self) {
        self.built.retain(|_, built| built.0.strong_count() != 0 && built.1.strong_count() != 0);
    }

    /// The fields `plan` asks for as one array, a layer each (an empty array without any)
    pub(crate) fn fields(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        key: (usize, bool),
        plan: &FieldPlan,
        layout: Option<&UdimLayout>,
        surface_uv: &wgpu::TextureView,
    ) -> Result<wgpu::TextureView, String> {
        if plan.requests.is_empty() {
            self.built.remove(&key);
            return Ok(self.empty.clone());
        }

        let (mesh, gbuf) = plan.surface.as_ref().ok_or("The whole-map generator fields are missing their UV surface.")?;
        let occlusion = plan.occlusion.as_ref().map(|(map, ..)| Arc::downgrade(map));
        let same = |built: &&Built| {
            built.0.ptr_eq(&Arc::downgrade(mesh))
                && built.1.ptr_eq(&Arc::downgrade(gbuf))
                && match (&built.2, &occlusion) {
                    (Some(old), Some(new)) => old.ptr_eq(new),
                    (None, None) => true,
                    _ => false,
                }
                && built.3 == plan.requests
                && built.4 == plan.material
                && built.5.as_ref() == layout
        };
        if let Some(built) = self.built.get(&key).filter(same) {
            return Ok(built.6.clone());
        }

        self.built.remove(&key);
        let view = self.build(device, queue, plan, layout, surface_uv)?;
        self.built.insert(key, (Arc::downgrade(mesh), Arc::downgrade(gbuf), occlusion, plan.requests.clone(), plan.material, layout.cloned(), view.clone()));
        Ok(view)
    }

    fn build(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        plan: &FieldPlan,
        layout: Option<&UdimLayout>,
        surface_uv: &wgpu::TextureView,
    ) -> Result<wgpu::TextureView, String> {
        let grid = layout.map(|layout| layout.grid);
        let (mesh, gbuf) = plan.surface.as_ref().ok_or("The whole-map generator fields are missing their UV surface.")?;
        let size = (gbuf.width, gbuf.height);
        let limits = device.limits();
        let layers = plan.requests.len() as u32;
        if size.0.max(size.1) > limits.max_texture_dimension_2d || layers > limits.max_texture_array_layers {
            return Err("The whole-map generator fields exceed the GPU texture limits.".into());
        }

        // Each triangle's corners and UV island, as the CPU's TrianglePartition numbers them
        let islands = TrianglePartition::uv_islands(mesh, plan.material);
        let mut corners: Vec<[u32; 8]> = Vec::with_capacity(mesh.indices.len());
        for (triangle, indices) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
            let island = islands.index_of(triangle as u32).map_or(u32::MAX, |island| island as u32);
            for &index in indices {
                let position = mesh.positions.get(index as usize).copied().unwrap_or_default();
                let uv = mesh.uvs.get(index as usize).copied().unwrap_or_default();
                corners.push([position.x.to_bits(), position.y.to_bits(), position.z.to_bits(), uv.x.to_bits(), uv.y.to_bits(), island, 0, 0]);
            }
        }
        let corner_bytes = (corners.len().max(1) * size_of::<[u32; 8]>()) as u64;
        if corner_bytes > limits.max_storage_buffer_binding_size {
            return Err("The mesh exceeds the GPU's whole-map generator field limits.".into());
        }
        let corners =
            crate::gpu_trace::buffer(device, "tc_generator_field_corners", if corners.is_empty() { &[0; 32] } else { bytemuck::cast_slice(&corners) });

        // The occlusion bake's first component, read nearest as BakedMaps::sample_working reads it
        let (occlusion, bake) = match &plan.occlusion {
            Some((map, width, height, components)) if (*width).max(*height) <= limits.max_texture_dimension_2d => {
                let values: Vec<f32> = map.chunks(*components).map(|texel| texel.first().copied().unwrap_or(1.0)).collect();
                let values = values.get(..(*width * *height) as usize).ok_or("The occlusion bake is shorter than its size.")?;
                let texture = plane(device, "tc_generator_field_occlusion", (*width, *height), 1);
                queue.write_texture(
                    texture.as_image_copy(),
                    bytemuck::cast_slice(values),
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: None },
                    wgpu::Extent3d { width: *width, height: *height, depth_or_array_layers: 1 },
                );
                (texture.create_view(&Default::default()), [*width, *height, 1, 0])
            }
            Some(_) => return Err("The occlusion bake exceeds the GPU texture limits.".into()),
            None => (plane(device, "tc_generator_field_no_occlusion", (1, 1), 1).create_view(&Default::default()), [0; 4]),
        };

        let block = grid.map_or(size, |grid| (size.0 / grid.cols.max(1), size.1 / grid.rows.max(1)));
        let block = (block.0.clamp(1, size.0.max(1)), block.1.clamp(1, size.1.max(1)));
        let grid_words = grid.map_or([0.0; 4], |grid| [grid.cols as f32, grid.rows as f32, grid.col as f32, grid.row as f32]);

        // A UV tile with a resolution of its own fills a share of its block (TextureSet::block_scales)
        let scaled = layout.filter(|layout| layout.scaled);
        let scales = scaled.map_or((&[][..], 0, 0), |layout| (layout.scales.as_slice(), layout.block, layout.grid.cols));
        let bake = [bake[0], bake[1], bake[2], scales.1];
        let block_scales =
            crate::gpu_trace::buffer(device, "tc_generator_field_block_scales", if scales.0.is_empty() { &[0; 4] } else { bytemuck::cast_slice(scales.0) });
        let block_count = (size.0.div_ceil(block.0) * size.1.div_ceil(block.1)) as usize;
        let no_radii = crate::gpu_trace::buffer(device, "tc_generator_field_no_radii", bytemuck::cast_slice(&vec![0u32; block_count.max(1)]));
        let base = FieldParams { size: [size.0, size.1, block.0, block.1], words: [0; 4], grid: grid_words, bake, values: [[0.0; 4]; 5], taps: [[0.0; 4]; 16] };
        let blank = plane(device, "tc_generator_field_blank", (1, 1), 1).create_view(&Default::default());
        let output = plane(device, "tc_generator_fields", size, layers);
        let kernels = self.kernels.get_or_insert_with(|| Kernels::new(device));
        let pass = Pass {
            device,
            kernels,
            surface_uv,
            corners: &corners,
            occlusion: &occlusion,
            blank: &blank,
            scales,
            block_scales: &block_scales,
            no_radii: &no_radii,
            base,
        };

        crate::gpu_trace::scoped(device, "GPU whole-map generator fields", || {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_generator_fields") });
            let needs_planes = plan.requests.iter().any(|request| matches!(request.controls, KindControls::BorderDistance(_) | KindControls::DrippingRust(_)));
            let planes: Vec<wgpu::TextureView> = if needs_planes {
                (0..SCRATCH_PLANES).map(|_| plane(device, "tc_generator_field_scratch", size, 1).create_view(&Default::default())).collect()
            } else {
                Vec::new()
            };
            let needs_distances = plan.requests.iter().any(|request| match &request.controls {
                KindControls::BorderDistance(_) => true,
                KindControls::UvChecker(checker) => checker.border_width > 0.0,
                _ => false,
            });
            let distances = needs_distances.then(|| {
                let rows = plane(device, "tc_generator_field_row_distances", size, 1).create_view(&Default::default());
                let distances = plane(device, "tc_generator_field_distances", size, 1).create_view(&Default::default());
                let table = island_table(device, &plan.requests[0], 0);
                let blocks = size.0.div_ceil(block.0);
                pass.run(&mut encoder, &kernels.distance_rows, &base, ([None, None, None], &table), &rows, ((size.1 * blocks).div_ceil(LINE_WORKGROUP), 1));
                pass.run(&mut encoder, &kernels.distance_columns, &base, ([Some(&rows), None, None], &table), &distances, pass.texels());
                distances
            });
            for (layer, request) in plan.requests.iter().enumerate() {
                let target = layer_view(&output, layer as u32);
                let table = island_table(device, request, islands.groups.len());
                let tables = &table;
                match &request.controls {
                    KindControls::BorderDistance(border) => border_distance(&pass, &mut encoder, tables, border, (distances.as_ref(), &planes), &target),
                    KindControls::DrippingRust(rust) => dripping_rust(&pass, &mut encoder, tables, (rust, request.seed, mesh), &planes, &target),
                    KindControls::UvRandomColor(_) => {
                        pass.run(&mut encoder, &kernels.random_color, &base, ([None, None, None], tables), &target, pass.texels())
                    }
                    KindControls::UvChecker(checker) => uv_checker(&pass, &mut encoder, tables, checker, distances.as_ref(), &target),
                    _ => return Err("A whole-map generator field has controls the GPU does not build.".to_string()),
                }
            }
            queue.submit([encoder.finish()]);
            Ok(output.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() }))
        })?
    }
}

/// UV Border Distance (uv_border_distance.rs build): the gradient, blurred with the empty space by Smoothness
fn border_distance(
    pass: &Pass,
    encoder: &mut wgpu::CommandEncoder,
    islands: &wgpu::Buffer,
    border: &BorderDistanceParameters,
    (distances, planes): (Option<&wgpu::TextureView>, &[wgpu::TextureView]),
    target: &wgpu::TextureView,
) {
    let reach_per_texel = border.distance * recipe::REACH_PER_DISTANCE;
    let radii = pass.block_radii(|tile_texels| {
        if border.smoothness > 0.0 {
            recipe::box_radius(border.smoothness * recipe::SMOOTHNESS_SIGMA_2K * tile_texels / recipe::REFERENCE_TILE_TEXELS)
        } else {
            0
        }
    });
    let params = FieldParams { values: [[reach_per_texel, 0.0, 0.0, 0.0], [0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4]], ..pass.base };
    let inputs = [distances, None, None];
    let Some(radii) = pass.radii_buffer(&radii) else {
        pass.run(encoder, &pass.kernels.border_gradient, &params, (inputs, islands), target, pass.texels());
        return;
    };

    let (gradient, middle, blurred) = (&planes[0], &planes[1], &planes[2]);
    pass.run(encoder, &pass.kernels.border_gradient, &params, (inputs, islands), gradient, pass.texels());
    pass.blur(encoder, islands, &radii, (gradient, middle, blurred));
    pass.run(encoder, &pass.kernels.keep_covered, &pass.base, ([Some(blurred), None, None], islands), target, pass.texels());
}

/// Dripping Rust (dripping_rust.rs build): the rust from the occlusion and grunge, spread over the cover, run down in drips and softened
fn dripping_rust(
    pass: &Pass,
    encoder: &mut wgpu::CommandEncoder,
    islands: &wgpu::Buffer,
    (rust, seed, mesh): (&DrippingRustParameters, u32, &Mesh),
    planes: &[wgpu::TextureView],
    target: &wgpu::TextureView,
) {
    let Some((_, repeats, mean, spread)) = recipe::rust_grunge() else { return };

    let shape = [recipe::RUST_FLOOR, recipe::RUST_GAIN, recipe::RUST_OCCLUSION, recipe::RUST_POWER];
    let seed_params =
        FieldParams { words: [0, 0, seed, 0], values: [shape, [repeats, mean, spread, recipe::GRUNGE_SPREAD], [0.0; 4], [0.0; 4], [0.0; 4]], ..pass.base };
    pass.run(encoder, &pass.kernels.rust_seed, &seed_params, ([None, None, None], islands), &planes[0], pass.texels());
    let radii = |sigma_2k: f32, smoothness: f32| {
        pass.block_radii(|tile_texels| recipe::box_radius(smoothness * sigma_2k * tile_texels / recipe::REFERENCE_TILE_TEXELS))
    };
    let mut field = pass.blur_over_cover(encoder, islands, &radii(recipe::SPREADING_SIGMA_2K, rust.spreading_smoothness), planes, 0);

    if rust.drips_samples > 0 && rust.drips_intensity > 0.0 {
        let axis = usize::from(rust.axis.min(2));
        let (low, high) = mesh.bounds();
        let low = low.to_array()[axis];
        let extent = (high.to_array()[axis] - low).max(f32::EPSILON);
        let reach_per_texel = rust.drips_intensity * recipe::DRIP_LENGTH_2K;
        let mut taps = [[0.0; 4]; 16];
        for (index, (fraction, fade)) in recipe::drip_taps(rust.drips_samples).into_iter().enumerate() {
            let slot = &mut taps[index / TAPS_PER_VECTOR];
            slot[(index % TAPS_PER_VECTOR) * 2] = fraction;
            slot[(index % TAPS_PER_VECTOR) * 2 + 1] = fade;
        }
        let params = FieldParams {
            words: [axis as u32, rust.drips_samples, 0, 0],
            values: [[low, extent, reach_per_texel, recipe::REFERENCE_TILE_TEXELS], [0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4]],
            taps,
            ..pass.base
        };
        let dripped = usize::from(field == 0);
        pass.run(encoder, &pass.kernels.drips, &params, ([Some(&planes[field]), None, None], islands), &planes[dripped], pass.texels());
        field = dripped;
    }

    let field = pass.blur_over_cover(encoder, islands, &radii(recipe::DRIPS_SIGMA_2K, rust.drips_smoothness), planes, field);
    let params = FieldParams { values: [[0.0, 1.0, 0.0, 0.0], [0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4]], ..pass.base };
    pass.run(encoder, &pass.kernels.keep_covered, &params, ([Some(&planes[field]), None, None], islands), target, pass.texels());
}

/// UV Checker (uv_islands.rs checker_field): the checker and grid per UV tile, the islands' borders drawn in
fn uv_checker(
    pass: &Pass,
    encoder: &mut wgpu::CommandEncoder,
    islands: &wgpu::Buffer,
    checker: &UvCheckerParameters,
    distances: Option<&wgpu::TextureView>,
    target: &wgpu::TextureView,
) {
    let square = recipe::REFERENCE_TILE_TEXELS / (recipe::SQUARES_PER_GRID_TILE * checker.grid_tile.max(1) as f32);
    let minor = square / recipe::MINOR_LINES_PER_SQUARE;
    let [light_red, light_green, light_blue] = checker.checker_color.map(linear_to_srgb);
    let [border_red, border_green, border_blue] = checker.border_color.map(linear_to_srgb);
    let values = [
        [light_red, light_green, light_blue, f32::from(u8::from(checker.random_colors))],
        [square, minor, recipe::LINE_HALF_WIDTH_2K, checker.grid_opacity.clamp(0.0, 1.0)],
        [border_red, border_green, border_blue, checker.border_width * recipe::BORDER_TEXELS_PER_WIDTH_2K],
        [recipe::DARK_SQUARE_SHADE, recipe::MINOR_LINE_SHADE, recipe::MAJOR_LINE_SHADE, 0.0],
        [recipe::REFERENCE_TILE_TEXELS, 0.0, 0.0, 0.0],
    ];
    let bordered = checker.border_width > 0.0 && distances.is_some();
    let params = FieldParams { words: [u32::from(bordered), 0, 0, 0], values, ..pass.base };
    pass.run(encoder, &pass.kernels.checker, &params, ([distances.filter(|_| bordered), None, None], islands), target, pass.texels());
}
