//! Derived curvature detail, computed from the bake on the GPU as compose's `curvature_sizes.rs`
//! builds it: each UV tile's block of the bake on its own (a classic set is one block), padded
//! within the block, blurred at a radius of the block's width and coarsened in steps that divide
//! the block. A UV tile smaller than its block covers the bake through its block scale.

use bytemuck::{Pod, Zeroable};
use texturecraft_compose::curvature_sizes::LEVEL_RADII;
use wgpu::util::DeviceExt;

use crate::gpu_trace::{entry, storage};

/// The blurred levels: the curvature detail sizes, then the wider ones the wear generators' edges smoothness reaches
pub(crate) const SIZES: usize = LEVEL_RADII.len();
const LINE_WORKGROUP: u32 = 64;
const TILE_WORKGROUP: u32 = 8;
const BOX_PASSES: usize = 3;

pub(crate) struct CurvatureInput<'a> {
    /// Existing packed bake texture: the curvature scalar is f32 bits in its second word.
    pub(crate) baked: &'a wgpu::TextureView,
    pub(crate) bake_size: [u32; 2],
    /// One bit per working UV texel, each row starting on a new word. None means fully covered.
    pub(crate) coverage: Option<&'a wgpu::Buffer>,
    pub(crate) working_size: [u32; 2],
    pub(crate) needed: [bool; SIZES],
    /// The bake texels of one UV tile block (`bake_size` for a classic set)
    pub(crate) block: [u32; 2],
    /// A UV tile atlas's working block side and each block's tile scale, row by row, when a tile
    /// is smaller than its block (empty otherwise: the bake then covers what the working texels do)
    pub(crate) scales: (u32, &'a [f32]),
}

#[derive(Clone, Debug)]
pub(crate) struct CurvatureLevels {
    pub(crate) image: wgpu::TextureView,
    pub(crate) layout: wgpu::Buffer,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Layout {
    levels: [[u32; 4]; SIZES],
    bake_size: [u32; 4],
}

impl CurvatureLevels {
    fn new(device: &wgpu::Device, extent: [u32; 2], data: &Layout) -> Self {
        Self {
            image: texture(device, "tc_curvature_levels", extent),
            layout: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tc_curvature_layout"),
                contents: bytemuck::bytes_of(data),
                usage: wgpu::BufferUsages::UNIFORM,
            }),
        }
    }

    pub(crate) fn empty(device: &wgpu::Device) -> Self {
        Self::new(device, [1, 1], &Layout::zeroed())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    dimensions: [u32; 4],
    mode: [u32; 4],
    level: [u32; 4],
    /// The bake block's width and height, the working block's side and the block columns (0
    /// without tile scales)
    block: [u32; 4],
}

pub(crate) struct CurvatureGpu {
    layout: wgpu::BindGroupLayout,
    empty: wgpu::Buffer,
    scan_rows: wgpu::ComputePipeline,
    scan_columns: wgpu::ComputePipeline,
    pad: wgpu::ComputePipeline,
    blur: wgpu::ComputePipeline,
    coarsen: wgpu::ComputePipeline,
}

impl CurvatureGpu {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let uint_image =
            wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tc_curvature_inputs"),
            entries: &[
                entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
                entry(1, uint_image),
                entry(
                    2,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::R32Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                ),
                storage(3, true),
                entry(4, uint_image),
                storage(5, true),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tc_curvature_pipeline"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::include_wgsl!("shaders/curvature_sizes.wgsl"));
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
        let empty = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_curvature_empty"),
            contents: &[0; 4],
            usage: wgpu::BufferUsages::STORAGE,
        });
        Self {
            layout,
            empty,
            scan_rows: pipeline("scan_rows"),
            scan_columns: pipeline("scan_columns"),
            pad: pipeline("pad_curvature"),
            blur: pipeline("blur_curvature"),
            coarsen: pipeline("coarsen_curvature"),
        }
    }

    pub(crate) fn build(&self, device: &wgpu::Device, queue: &wgpu::Queue, input: CurvatureInput<'_>) -> Result<CurvatureLevels, String> {
        let [width, height] = input.bake_size;
        let [working_width, working_height] = input.working_size;
        let dimensions = [width, height, working_width, working_height];
        let limits = device.limits();
        if !input.needed.contains(&true) {
            return Ok(CurvatureLevels::empty(device));
        }
        if dimensions.contains(&0) || width.max(height) > limits.max_texture_dimension_2d {
            return Err("Curvature bake dimensions do not fit the GPU.".into());
        }
        let coverage_bytes = u64::from(working_width.div_ceil(32)) * u64::from(working_height) * 4;
        if input.coverage.is_some_and(|coverage| coverage.size() < coverage_bytes) {
            return Err("The curvature UV coverage does not match its working size.".into());
        }
        let block = [input.block[0].clamp(1, width), input.block[1].clamp(1, height)];
        let columns = if input.scales.1.is_empty() { 0 } else { width.div_ceil(block[0]) };
        let block_words = [block[0], block[1], input.scales.0, columns];
        let (extent, layout, levels) = plan_atlas(input.bake_size, block, input.needed, limits.max_texture_dimension_2d)
            .ok_or("Curvature detail sizes do not fit the GPU texture dimensions.")?;
        crate::gpu_trace::scoped(device, "GPU curvature detail", || {
            let result = CurvatureLevels::new(device, extent, &layout);
            let first = texture(device, "tc_curvature_first", input.bake_size);
            let second = texture(device, "tc_curvature_second", input.bake_size);
            let third = input.coverage.map(|_| texture(device, "tc_curvature_padded_scratch", input.bake_size));
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_curvature") });
            let mut run = |pipeline: &wgpu::ComputePipeline,
                           (source, target): (&wgpu::TextureView, &wgpu::TextureView),
                           weights: &wgpu::Buffer,
                           mode: [u32; 4],
                           level: [u32; 4],
                           groups: [u32; 2]| {
                let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("tc_curvature_params"),
                    contents: bytemuck::bytes_of(&Params { dimensions, mode, level, block: block_words }),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("tc_curvature_stage"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(source) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(target) },
                        wgpu::BindGroupEntry { binding: 3, resource: input.coverage.unwrap_or(&self.empty).as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(input.baked) },
                        wgpu::BindGroupEntry { binding: 5, resource: weights.as_entire_binding() },
                    ],
                });
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &bindings, &[]);
                pass.dispatch_workgroups(groups[0], groups[1], 1);
            };
            // The coverage scan reads each block's tile scale where the edge weights go later
            let scales = (!input.scales.1.is_empty()).then(|| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("tc_curvature_block_scales"),
                    contents: bytemuck::cast_slice(input.scales.1),
                    usage: wgpu::BufferUsages::STORAGE,
                })
            });
            let scales = scales.as_ref().unwrap_or(&self.empty);
            let (original, ping_pong, encoding) = if let Some(third) = &third {
                run(&self.scan_rows, (input.baked, &first), scales, [0; 4], [0; 4], [height.div_ceil(LINE_WORKGROUP), 1]);
                run(&self.scan_columns, (&first, &second), &self.empty, [0; 4], [0; 4], [width.div_ceil(LINE_WORKGROUP), 1]);
                run(&self.pad, (&second, &first), &self.empty, [0; 4], [0; 4], [width.div_ceil(TILE_WORKGROUP), height.div_ceil(TILE_WORKGROUP)]);
                (&first, [&second, third], 1)
            } else {
                (input.baked, [&first, &second], 2)
            };
            for level in levels {
                let mut source = original;
                for pass in 0..BOX_PASSES * 2 {
                    let vertical = pass >= BOX_PASSES;
                    let line_count = if vertical { width } else { height };
                    let destination = ping_pong[pass % 2];
                    run(
                        &self.blur,
                        (source, destination),
                        &self.empty,
                        [level.radius, 0, u32::from(vertical), if pass == 0 { encoding } else { 0 }],
                        [0; 4],
                        [line_count.div_ceil(LINE_WORKGROUP), 1],
                    );
                    source = destination;
                }
                // Each block is blurred alone, so its edges lose support at its own sides
                let block_weights =
                    |length: u32, side: u32| (0..length).step_by(side as usize).flat_map(move |start| inside_weights(side.min(length - start), level.radius));
                let weights: Vec<_> = block_weights(width, block[0]).chain(block_weights(height, block[1])).collect();
                let weights = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("tc_curvature_edge_weights"),
                    contents: bytemuck::cast_slice(&weights),
                    usage: wgpu::BufferUsages::STORAGE,
                });
                run(
                    &self.coarsen,
                    (source, &result.image),
                    &weights,
                    [0, level.step, 0, 0],
                    level.rect,
                    [level.rect[2].div_ceil(TILE_WORKGROUP), level.rect[3].div_ceil(TILE_WORKGROUP)],
                );
            }
            queue.submit([encoder.finish()]);
            result
        })
    }
}

#[derive(Clone, Copy)]
struct Level {
    radius: u32,
    step: u32,
    rect: [u32; 4],
}

/// The levels' places in one atlas: each size's radius (texels at 2K, scaled to the block's
/// width) and coarsening step (dividing the block, so no cell spans two UV tiles).
fn plan_atlas(size: [u32; 2], block: [u32; 2], needed: [bool; SIZES], limit: u32) -> Option<([u32; 2], Layout, Vec<Level>)> {
    let radii: Vec<_> =
        LEVEL_RADII.iter().zip(needed).map(|(&radius, needed)| needed.then_some((radius * block[0] as f32 / 2048.0).round().max(1.0) as u32)).collect();
    let initial_width = radii.iter().flatten().map(|&radius| size[0].div_ceil(level_step(block, radius))).max()?;
    if initial_width > limit {
        return None;
    }
    std::iter::successors(Some(initial_width), |&width| (width < limit).then_some(width.saturating_mul(2).min(limit)))
        .filter_map(|width| pack_levels((size, block), &radii, width, limit))
        .min_by_key(|(extent, ..)| u64::from(extent[0]) * u64::from(extent[1]))
}

fn pack_levels((size, block): ([u32; 2], [u32; 2]), radii: &[Option<u32>], row_width: u32, limit: u32) -> Option<([u32; 2], Layout, Vec<Level>)> {
    let mut layout = Layout { levels: [[0; 4]; SIZES], bake_size: [size[0], size[1], block[0], block[1]] };
    let mut levels = Vec::<Level>::new();
    let mut extent = [1, 1];
    let (mut x, mut y, mut row_height) = (0, 0, 0);
    for (&radius, slot) in radii.iter().zip(&mut layout.levels) {
        let Some(radius) = radius else { continue };
        if let Some(previous) = levels.iter().find(|level| level.radius == radius) {
            *slot = [previous.rect[0], previous.rect[1], previous.step, 0];
            continue;
        }
        let step = level_step(block, radius);
        let [width, height] = size.map(|edge| edge.div_ceil(step));
        if x + width > row_width {
            x = 0;
            y += row_height;
            row_height = 0;
        }
        if y + height > limit {
            return None;
        }
        *slot = [x, y, step, 0];
        levels.push(Level { radius, step, rect: [x, y, width, height] });
        x += width;
        row_height = row_height.max(height);
        extent = [extent[0].max(x), extent[1].max(y + row_height)];
    }
    Some((extent, layout, levels))
}

fn level_step(block: [u32; 2], radius: u32) -> u32 {
    let mut step = (radius / 2).max(1).next_power_of_two();
    while block.iter().any(|edge| !edge.is_multiple_of(step)) {
        step /= 2;
    }
    step
}

// The box filter reads zero beyond the bake. These dimension-only parameters
// normalize its loss of support at the border without a second blurred image.
fn inside_weights(length: u32, radius: u32) -> impl Iterator<Item = f32> {
    let length = length as usize;
    let radius = radius as usize;
    let divisor = (radius * 2 + 1) as f64;
    let mut weights = vec![1.0f64; length];
    let mut prefix = vec![0.0f64; length + 1];
    for _ in 0..BOX_PASSES {
        for (index, weight) in weights.iter().enumerate() {
            prefix[index + 1] = prefix[index] + *weight;
        }
        for (index, weight) in weights.iter_mut().enumerate() {
            *weight = (prefix[(index + radius + 1).min(length)] - prefix[index.saturating_sub(radius)]) / divisor;
        }
    }
    weights.into_iter().map(|weight| weight as f32)
}

fn texture(device: &wgpu::Device, label: &str, [width, height]: [u32; 2]) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
}
