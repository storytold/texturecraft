//! GPU brush coverage and projection. Only finished stroke tiles return to the document.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, Weak};

use bytemuck::{Pod, Zeroable};
use texturecraft_compose::{ImageRes, Stencil};
use texturecraft_geom::{
    Bvh, Mesh, UvGBuffer,
    glam::{Mat4, Vec2},
};
use texturecraft_paint::{BrushAlpha, BrushSettings, EyeDepth, Stamp, UvStamp, alphas::AlphaImage};
use texturecraft_texel::{TILE, TileImage};
use wgpu::util::DeviceExt;

use crate::gpu_stencil::{self, GpuStencil};

type Tiles = BTreeSet<(u32, u32)>;
type TileDabs = BTreeMap<(u32, u32), Vec<u32>>;
type ProjectedTiles = BTreeMap<(u32, u32), Vec<[f32; 4]>>;
/// A finished stroke's alpha map and the tiles it wrote, which the next stroke of that size
/// clears and reuses instead of allocating a set-size map (a path repaints one per change)
type SpareAlpha = Arc<Mutex<Option<(wgpu::Texture, Tiles)>>>;
const ROW_BYTES: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
const TILE_BYTES: u64 = (ROW_BYTES * TILE) as u64;
const CHUNK: usize = 256;
const LIBRARY_ALPHA_KIND: u32 = 7;
const IMAGE_ALPHA_KIND: u32 = 8;
const PROJECTION_TILE_BYTES: u64 = (TILE * TILE) as u64 * size_of::<[f32; 4]>() as u64;
const PROJECTION_READBACK_TILES: usize = 64;

pub struct BrushInput {
    pub mesh: Arc<Mesh>,
    pub bvh: Arc<Bvh>,
    pub uv: Arc<UvGBuffer>,
    pub material: usize,
    pub depth: Option<Arc<EyeDepth>>,
    pub settings: BrushSettings,
    pub alpha_image: Option<Arc<ImageRes>>,
    pub stencil: Option<Stencil>,
    pub limit: Option<(TileImage, bool)>,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Parameters {
    camera: [[f32; 4]; 4],
    eye: [f32; 4],
    view_scale: [f32; 4],
    mesh: [f32; 4],
    ortho: [f32; 4],
    shape: [f32; 4],
    alpha_rotation: [f32; 4],
    flags: [u32; 4],
    stamp_blending: u32,
    padding: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Dab {
    center: [f32; 4],
    normal: [f32; 4],
    tangent: [f32; 4],
    bitangent: [f32; 4],
    seed: u32,
    padding: [u32; 3],
    /// A symmetry copy's source origin with w 1, zero for any other dab.
    source: [f32; 4],
    /// The columns of the copy's `StampSource::to_source`.
    to_source: [[f32; 4]; 3],
}

struct Visibility {
    nodes: wgpu::Buffer,
    triangles: wgpu::Buffer,
    depth: wgpu::TextureView,
}

pub(crate) struct BrushPipeline {
    shader: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    pipelines: BTreeMap<u32, wgpu::ComputePipeline>,
    rays: Option<(Weak<Bvh>, wgpu::Buffer, wgpu::Buffer)>,
    depth: Option<(Weak<EyeDepth>, wgpu::TextureView)>,
    stencil_image: Option<(Weak<ImageRes>, wgpu::TextureView)>,
    library_tip: Option<(Weak<AlphaImage>, wgpu::TextureView)>,
    image_tip: Option<(Weak<ImageRes>, wgpu::TextureView)>,
    blank_tip: wgpu::TextureView,
    projection_readback: Option<wgpu::ComputePipeline>,
    spare_alpha: SpareAlpha,
}

impl BrushPipeline {
    pub(crate) fn clear_visibility(&mut self) {
        self.rays = None;
        self.depth = None;
        self.stencil_image = None;
    }

    pub fn new(device: &wgpu::Device) -> Self {
        use crate::gpu_trace::{entry, storage};
        let texture_entry = |binding, sample_type| {
            entry(binding, wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false })
        };
        let inputs = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tc_brush_inputs"),
            entries: &[
                entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
                storage(1, true),
                storage(2, true),
                texture_entry(3, wgpu::TextureSampleType::Uint),
                storage(4, true),
                texture_entry(5, wgpu::TextureSampleType::Uint),
                texture_entry(6, wgpu::TextureSampleType::Float { filterable: false }),
                storage(7, true),
                storage(8, true),
                storage(9, false),
                texture_entry(10, wgpu::TextureSampleType::Float { filterable: false }),
            ],
        });
        let layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("tc_brush"), bind_group_layouts: &[Some(&inputs)], immediate_size: 0 });
        let source = concat!(include_str!("shaders/rays.wgsl"), "\n", include_str!("shaders/noise.wgsl"), "\n", include_str!("shaders/brush.wgsl"));
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("tc_brush"), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let blank_tip = texture(device, [1, 1], wgpu::TextureFormat::R32Float).create_view(&Default::default());
        Self {
            shader,
            layout,
            pipelines: BTreeMap::new(),
            rays: None,
            depth: None,
            stencil_image: None,
            library_tip: None,
            image_tip: None,
            blank_tip,
            projection_readback: None,
            spare_alpha: SpareAlpha::default(),
        }
    }

    fn pipeline(&mut self, device: &wgpu::Device, alpha_kind: u32) -> wgpu::ComputePipeline {
        // Specialize by shape so the simple brushes keep their smaller GPU kernels.
        self.pipelines
            .entry(alpha_kind)
            .or_insert_with(|| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("tc_brush"),
                    layout: Some(&self.layout),
                    module: &self.shader,
                    entry_point: Some("brush"),
                    compilation_options: wgpu::PipelineCompilationOptions { constants: &[("alpha_kind", f64::from(alpha_kind))], ..Default::default() },
                    cache: None,
                })
            })
            .clone()
    }

    fn library_tip(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, alpha: &Arc<AlphaImage>) -> wgpu::TextureView {
        let key = Arc::downgrade(alpha);
        if let Some((previous, view)) = &self.library_tip
            && previous.ptr_eq(&key)
        {
            return view.clone();
        }
        let base = texturecraft_paint::alphas::BASE as u32;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tc_brush_library_tip"),
            size: extent(base, base),
            mip_level_count: alpha.levels() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, (side, pixels)) in alpha.mip_pixels().enumerate() {
            let destination = wgpu::TexelCopyTextureInfo { mip_level: level as u32, ..texture.as_image_copy() };
            let layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(side * size_of::<f32>() as u32), rows_per_image: None };
            queue.write_texture(destination, bytemuck::cast_slice(pixels), layout, extent(side, side));
        }
        let view = texture.create_view(&Default::default());
        self.library_tip = Some((key, view.clone()));
        view
    }

    fn brush_tip(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, input: &BrushInput) -> Result<(wgpu::TextureView, u32), String> {
        if let Some(image) = &input.alpha_image {
            let key = Arc::downgrade(image);
            if let Some((previous, view)) = &self.image_tip
                && previous.ptr_eq(&key)
            {
                return Ok((view.clone(), IMAGE_ALPHA_KIND));
            }
            let view = gpu_stencil::upload_image(device, queue, image).map_err(|_| "The brush alpha exceeds the adapter's texture layout limits.")?;
            self.image_tip = Some((key, view.clone()));
            return Ok((view, IMAGE_ALPHA_KIND));
        }
        if let Some(alpha) = input.settings.library_alpha_image() {
            return Ok((self.library_tip(device, queue, &alpha), LIBRARY_ALPHA_KIND));
        }
        let kind = match input.settings.alpha {
            BrushAlpha::Round => 0,
            BrushAlpha::Square => 1,
            BrushAlpha::Spray => 2,
            BrushAlpha::Grunge => 3,
            BrushAlpha::Cloud => 4,
            BrushAlpha::Star => 5,
            BrushAlpha::Streaks => 6,
        };
        Ok((self.blank_tip.clone(), kind))
    }

    fn visibility(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, input: &BrushInput) -> Result<Visibility, String> {
        let key = Arc::downgrade(&input.bvh);
        if self.rays.as_ref().is_none_or(|(last, ..)| !last.ptr_eq(&key)) {
            let (nodes, triangles) = crate::gpu_trace::bvh_rows(&input.bvh, (0, 0), |_| ([0; 3], [0; 4]));
            let bytes = (nodes.len().max(triangles.len()) * 16) as u64;
            if bytes > device.limits().max_storage_buffer_binding_size.min(device.limits().max_buffer_size) {
                return Err("mesh visibility buffers exceed this adapter's limits".into());
            }
            self.rays = Some((key, storage(device, "tc_brush_nodes", &nodes), storage(device, "tc_brush_triangles", &triangles)));
        }
        let (_, nodes, triangles) = self.rays.as_ref().ok_or("brush meshes are missing")?;
        let depth = if let Some(depth) = &input.depth {
            let key = Arc::downgrade(depth);
            if self.depth.as_ref().is_none_or(|(last, _)| !last.ptr_eq(&key)) {
                let surface = texture(device, [depth.width, depth.height], wgpu::TextureFormat::R32Float);
                queue.write_texture(
                    surface.as_image_copy(),
                    bytemuck::cast_slice(&depth.dist),
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(depth.width * 4), rows_per_image: None },
                    extent(depth.width, depth.height),
                );
                self.depth = Some((key, surface.create_view(&Default::default())));
            }
            self.depth.as_ref().ok_or("brush depth is missing")?.1.clone()
        } else {
            texture(device, [1, 1], wgpu::TextureFormat::R32Float).create_view(&Default::default())
        };
        Ok(Visibility { nodes: nodes.clone(), triangles: triangles.clone(), depth })
    }

    pub fn begin(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        input: BrushInput,
        surface: (wgpu::TextureView, wgpu::Buffer),
    ) -> Result<GpuBrushStroke, String> {
        let size = [input.uv.width, input.uv.height];
        if size.into_iter().any(|edge| !edge.is_multiple_of(TILE) || edge > device.limits().max_texture_dimension_2d) {
            return Err("unsupported GPU brush texture size".into());
        }
        if input.limit.as_ref().is_some_and(|(mask, _)| [mask.width(), mask.height()] != size || mask.comps() != 1) {
            return Err("The quick mask does not match the GPU brush texture size.".into());
        }
        let visibility = self.visibility(device, queue, &input)?;
        let spare = self.spare_alpha.lock().ok().and_then(|mut spare| spare.take());
        let alpha = match spare {
            Some((alpha, written)) if [alpha.width(), alpha.height()] == size => {
                clear_tiles(queue, &alpha, &written);
                alpha
            }
            _ => texture(device, size, wgpu::TextureFormat::R16Uint),
        };
        let view = alpha.create_view(&Default::default());
        let (lo, hi) = input.mesh.bounds();
        let center = (lo + hi) * 0.5;
        let radius = (hi - lo).length().mul_add(0.5, 0.0).max(1e-4);
        self.stencil_image.take_if(|(source, _)| source.strong_count() == 0);
        self.image_tip.take_if(|(source, _)| source.strong_count() == 0);
        self.library_tip.take_if(|(source, _)| source.strong_count() == 0);
        let projection = if let Some(stencil) = &input.stencil {
            let key = Arc::downgrade(&stencil.image);
            if self.stencil_image.as_ref().is_none_or(|(previous, _)| !previous.ptr_eq(&key)) {
                self.stencil_image = None;
                self.stencil_image = Some((key, gpu_stencil::upload_image(device, queue, &stencil.image)?));
            }
            let (_, image) = self.stencil_image.as_ref().ok_or("projection image missing")?;
            let stencil = GpuStencil::new(device, image.clone(), stencil, center, radius);
            let readback = self.projection_readback.get_or_insert_with(|| gpu_stencil::readback_pipeline(device)).clone();
            Some((stencil, readback))
        } else {
            None
        };
        let settings = &input.settings;
        let (tip, kind) = self.brush_tip(device, queue, &input)?;
        let pipeline = self.pipeline(device, kind);
        let (sin, cos) = settings.alpha_params.rotation.to_radians().sin_cos();
        let mut params = Parameters {
            camera: Mat4::IDENTITY.to_cols_array_2d(),
            eye: [0.0; 4],
            view_scale: [0.0, settings.angle_fade.1.to_radians().cos(), settings.angle_fade.0.to_radians().cos(), 0.0],
            mesh: center.extend(radius).to_array(),
            ortho: [0.0; 4],
            shape: [settings.alpha_params.size_x, settings.alpha_params.size_y, settings.alpha_params.roundness, settings.alpha_params.contrast],
            alpha_rotation: [sin, cos, 0.0, 0.0],
            flags: [
                0,
                u32::from(settings.backface_culling),
                u32::from(settings.alpha_params.invert),
                input.limit.as_ref().map_or(0, |(_, invert)| 1 + u32::from(*invert)),
            ],
            stamp_blending: settings.stamp_blending as u32,
            padding: [0; 3],
        };
        if let Some(depth) = &input.depth {
            params.camera = depth.view_proj.to_cols_array_2d();
            params.eye = depth.eye.extend(depth.bias).to_array();
            params.view_scale[0] = (depth.height as f32 * depth.view_proj.row(1).truncate().length()).max(f32::EPSILON);
            params.view_scale[3] = 1.0;
            if let Some(to_eye) = depth.orthographic_to_eye {
                params.ortho = to_eye.extend(1.0).to_array();
            }
        }
        Ok(GpuBrushStroke {
            input,
            pipeline,
            device: device.clone(),
            queue: queue.clone(),
            surface,
            visibility,
            alpha,
            view,
            tip,
            params,
            scratch: None,
            projection,
            written: Tiles::new(),
            spare_alpha: self.spare_alpha.clone(),
        })
    }
}

pub struct GpuBrushStroke {
    input: BrushInput,
    pipeline: wgpu::ComputePipeline,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: (wgpu::TextureView, wgpu::Buffer),
    visibility: Visibility,
    alpha: wgpu::Texture,
    view: wgpu::TextureView,
    tip: wgpu::TextureView,
    params: Parameters,
    scratch: Option<wgpu::Buffer>,
    projection: Option<(GpuStencil, wgpu::ComputePipeline)>,
    /// The alpha tiles the stroke wrote, cleared when the next stroke reuses its map
    written: Tiles,
    spare_alpha: SpareAlpha,
}

impl Drop for GpuBrushStroke {
    fn drop(&mut self) {
        if let Ok(mut spare) = self.spare_alpha.lock() {
            *spare = Some((self.alpha.clone(), std::mem::take(&mut self.written)));
        }
    }
}

impl GpuBrushStroke {
    pub fn view(&self) -> wgpu::TextureView {
        self.view.clone()
    }

    pub fn stencil(&self) -> Option<GpuStencil> {
        self.projection.as_ref().map(|(stencil, _)| stencil.clone())
    }

    pub fn append_world(&mut self, stamps: &[Stamp]) -> Result<Tiles, String> {
        let mut bins = TileDabs::new();
        let input = &self.input;
        let size = Vec2::new(input.uv.width as f32, input.uv.height as f32);
        let mut dabs = Vec::new();
        for stamp in stamps {
            if !stamp.radius.is_finite() || stamp.radius <= 0.0 || !stamp.center.is_finite() {
                continue;
            }
            let (source, to_source) = match stamp.source {
                Some(source) => (source.origin.extend(1.0).to_array(), source.to_source.to_cols_array_2d().map(|[x, y, z]| [x, y, z, 0.0])),
                None => ([0.0; 4], [[0.0; 4]; 3]),
            };
            let index = dabs.len() as u32;
            dabs.push(Dab {
                center: stamp.center.extend(stamp.radius).to_array(),
                normal: stamp.normal.extend(stamp.depth).to_array(),
                tangent: stamp.tangent.extend(stamp.flow).to_array(),
                bitangent: stamp.bitangent.extend(stamp.hardness).to_array(),
                seed: stamp.seed,
                padding: [0; 3],
                source,
                to_source,
            });
            let extent = stamp.world_extent();
            let mut touched = Tiles::new();
            input.bvh.query_box(stamp.center - extent, stamp.center + extent, |triangle| {
                if input.mesh.material_of(triangle as usize) == Some(input.material)
                    && let Some((lo, hi)) = footprint(&input.mesh, triangle, stamp)
                {
                    add_tiles(&mut touched, lo * size, hi * size, size);
                }
            });
            for tile in touched {
                bins.entry(tile).or_default().push(index);
            }
        }
        self.dispatch(&bins, &dabs, false)?;
        Ok(bins.into_keys().collect())
    }

    pub fn append_uv(&mut self, stamps: &[UvStamp]) -> Result<Tiles, String> {
        let mut bins = TileDabs::new();
        let size = Vec2::new(self.input.uv.width as f32, self.input.uv.height as f32);
        let mut dabs = Vec::new();
        for stamp in stamps {
            if !stamp.radius.is_finite() || stamp.radius <= 0.0 || !stamp.center.is_finite() {
                continue;
            }
            let center = stamp.center * size;
            let radius = stamp.radius * size.x;
            let (sin, cos) = stamp.angle.to_radians().sin_cos();
            let index = dabs.len() as u32;
            dabs.push(Dab {
                center: [center.x, center.y, 0.0, radius],
                normal: [0.0; 4],
                tangent: [cos, sin, 0.0, stamp.flow],
                bitangent: [0.0, 0.0, 0.0, stamp.hardness],
                seed: stamp.seed,
                padding: [0; 3],
                source: [0.0; 4],
                to_source: [[0.0; 4]; 3],
            });
            let reach = Vec2::splat(radius * 1.42);
            let mut touched = Tiles::new();
            add_tiles(&mut touched, center - reach, center + reach, size);
            for tile in touched {
                bins.entry(tile).or_default().push(index);
            }
        }
        self.dispatch(&bins, &dabs, true)?;
        Ok(bins.into_keys().collect())
    }

    fn dispatch(&mut self, tiles: &TileDabs, dabs: &[Dab], uv: bool) -> Result<(), String> {
        if tiles.is_empty() || dabs.is_empty() {
            return Ok(());
        }
        let mut params = self.params;
        params.flags[0] = u32::from(uv);
        self.written.extend(tiles.keys());
        let coordinates: Vec<_> = tiles.iter().collect();
        let (device, queue) = (&self.device, &self.queue);
        crate::gpu_trace::scoped(device, "GPU brush dispatch", || {
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tc_brush_parameters"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let dabs = storage(device, "tc_brush_dabs", dabs);
            let bytes = coordinates.len().min(CHUNK) as u64 * TILE_BYTES;
            if self.scratch.as_ref().is_none_or(|buffer| buffer.size() < bytes) {
                self.scratch = Some(buffer(device, "tc_brush_scratch", bytes.next_power_of_two(), wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC));
            }
            let scratch = self.scratch.as_ref().ok_or("brush scratch buffer missing")?;
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_brush") });
            for chunk in coordinates.chunks(CHUNK) {
                // Origins, dab offsets/counts and optional quick-mask offsets.
                let mut cells = vec![0; chunk.len() * 5];
                for (tile, (&(x, y), indices)) in chunk.iter().copied().enumerate() {
                    let first = cells.len() as u32;
                    cells.extend(indices);
                    let mask = self.input.limit.as_ref().and_then(|(mask, _)| mask.tile(x, y));
                    let limit = mask.map_or(0, |mask| {
                        let offset = cells.len() as u32;
                        cells.extend(mask.as_chunks::<2>().0.iter().map(|&[lo, hi]| u32::from(lo) | (u32::from(hi) << 16)));
                        offset
                    });
                    cells[tile * 5..tile * 5 + 5].copy_from_slice(&[x * TILE, y * TILE, first, indices.len() as u32, limit]);
                }
                let cells = storage(device, "tc_brush_tiles", &cells);
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("tc_brush_inputs"),
                    layout: &self.pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: self.visibility.nodes.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: self.visibility.triangles.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.surface.0) },
                        wgpu::BindGroupEntry { binding: 4, resource: self.surface.1.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&self.view) },
                        wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&self.visibility.depth) },
                        wgpu::BindGroupEntry { binding: 7, resource: dabs.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 8, resource: cells.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 9, resource: scratch.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&self.tip) },
                    ],
                });
                {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(&self.pipeline);
                    pass.set_bind_group(0, &bind, &[]);
                    pass.dispatch_workgroups(TILE / 16, TILE / 8, chunk.len() as u32);
                }
                for (i, (&(x, y), _)) in chunk.iter().copied().enumerate() {
                    encoder.copy_buffer_to_texture(buffer_copy(scratch, i), tile_copy(&self.alpha, x * TILE, y * TILE), extent(TILE, TILE));
                }
            }
            queue.submit([encoder.finish()]);
            Ok::<_, String>(())
        })?
    }

    pub fn read_alpha(&self, tiles: &Tiles) -> Result<TileImage, String> {
        let mut alpha = TileImage::new(self.input.uv.width, self.input.uv.height, 1);
        if tiles.is_empty() {
            return Ok(alpha);
        }
        let (device, queue) = (&self.device, &self.queue);
        let read = crate::gpu_trace::scoped(device, "GPU brush readback buffer", || {
            buffer(device, "tc_brush_read_alpha", tiles.len().min(CHUNK) as u64 * TILE_BYTES, wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ)
        })?;
        let coords: Vec<_> = tiles.iter().copied().collect();
        for chunk in coords.chunks(CHUNK) {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_brush_read_alpha") });
            for (i, &(x, y)) in chunk.iter().enumerate() {
                encoder.copy_texture_to_buffer(tile_copy(&self.alpha, x * TILE, y * TILE), buffer_copy(&read, i), extent(TILE, TILE));
            }
            let index = queue.submit([encoder.finish()]);
            let slice = read.slice(..chunk.len() as u64 * TILE_BYTES);
            let (send, receive) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
            device.poll(wgpu::PollType::Wait { submission_index: Some(index), timeout: None }).map_err(|e| e.to_string())?;
            receive.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
            {
                let mapped = slice.get_mapped_range().map_err(|e| e.to_string())?;
                for (bytes, &(x, y)) in mapped.as_chunks::<{ TILE_BYTES as usize }>().0.iter().zip(chunk) {
                    let samples: Vec<_> = bytes
                        .as_chunks::<{ ROW_BYTES as usize }>()
                        .0
                        .iter()
                        .flat_map(|row| row[..(TILE * 2) as usize].as_chunks::<2>().0.iter().map(|&value| u16::from_le_bytes(value)))
                        .collect();
                    if samples.iter().any(|&a| a != 0) {
                        alpha.set_tile(x, y, Some(Arc::new(samples)));
                    }
                }
            }
            read.unmap();
        }
        Ok(alpha)
    }
    /// Read the projected linear RGB and image alpha of the touched tiles.
    /// The stroke's geometric coverage remains in the separate alpha map.
    pub fn read_projection(&self, tiles: &Tiles) -> Result<ProjectedTiles, String> {
        let Some((stencil, pipeline)) = &self.projection else { return Err("The stroke has no GPU projection.".into()) };
        if tiles.is_empty() {
            return Ok(BTreeMap::new());
        }
        let (device, queue) = (&self.device, &self.queue);
        let limits = device.limits();
        let tiles_per_batch = PROJECTION_READBACK_TILES
            .min((limits.max_storage_buffer_binding_size.min(limits.max_buffer_size) / PROJECTION_TILE_BYTES) as usize)
            .min(limits.max_compute_workgroups_per_dimension as usize);
        if tiles_per_batch == 0 {
            return Err("The GPU projection readback exceeds the adapter's buffer limits.".into());
        }
        let coords: Vec<_> = tiles.iter().copied().collect();
        if coords.iter().any(|&(x, y)| x >= self.input.uv.width / TILE || y >= self.input.uv.height / TILE) {
            return Err("A GPU projection tile is outside the texture set.".into());
        }
        let bytes = tiles.len().min(tiles_per_batch) as u64 * PROJECTION_TILE_BYTES;
        let (output, read) = crate::gpu_trace::scoped(device, "GPU projection readback buffers", || {
            (
                buffer(device, "tc_projected_tiles", bytes, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC),
                buffer(device, "tc_read_projected_tiles", bytes, wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ),
            )
        })?;
        let layout = pipeline.get_bind_group_layout(0);
        let mut samples = BTreeMap::new();
        for chunk in coords.chunks(tiles_per_batch) {
            let bytes = chunk.len() as u64 * PROJECTION_TILE_BYTES;
            let origins: Vec<_> = chunk.iter().map(|&(x, y)| [x * TILE, y * TILE]).collect();
            let submitted = crate::gpu_trace::scoped(device, "GPU projection readback", || {
                let tile_origins = storage(device, "tc_projected_tile_origins", &origins);
                let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("tc_projected_tiles"),
                    layout: &layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.view) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.surface.0) },
                        wgpu::BindGroupEntry { binding: 2, resource: self.surface.1.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&stencil.image) },
                        wgpu::BindGroupEntry { binding: 4, resource: stencil.uniforms.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 5, resource: tile_origins.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 6, resource: output.as_entire_binding() },
                    ],
                });
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_read_projected_tiles") });
                {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, &bindings, &[]);
                    pass.dispatch_workgroups(TILE / 8, TILE / 8, chunk.len() as u32);
                }
                encoder.copy_buffer_to_buffer(&output, 0, &read, 0, Some(bytes));
                queue.submit([encoder.finish()])
            })?;
            let slice = read.slice(..bytes);
            let (sender, receiver) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
            device.poll(wgpu::PollType::Wait { submission_index: Some(submitted), timeout: None }).map_err(|e| e.to_string())?;
            receiver.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
            {
                let mapping = slice.get_mapped_range().map_err(|e| e.to_string())?;
                let rgba: &[[f32; 4]] = bytemuck::try_cast_slice(&mapping).map_err(|e| e.to_string())?;
                for (&tile, texels) in chunk.iter().zip(rgba.as_chunks::<{ texturecraft_texel::TILE_AREA }>().0) {
                    samples.insert(tile, texels.to_vec());
                }
            }
            read.unmap();
        }
        Ok(samples)
    }
}

// Clip the triangle to the dab's enclosing volume before binning its projected texels.
fn footprint(mesh: &Mesh, triangle: u32, stamp: &Stamp) -> Option<(Vec2, Vec2)> {
    let corners = mesh.tri(triangle as usize)?;
    let mut polygon = Vec::with_capacity(9);
    for index in corners {
        polygon.push((*mesh.positions.get(index)? - stamp.center, *mesh.uvs.get(index)?));
    }
    let mut next = Vec::with_capacity(9);
    let r = stamp.radius;
    for (axis, limit) in [(stamp.tangent, r * 1.42), (stamp.bitangent, r * 1.42), (stamp.normal, r * stamp.depth)] {
        for axis in [axis, -axis] {
            next.clear();
            let mut prev = *polygon.last()?;
            let mut d0 = prev.0.dot(axis) - limit;
            for &curr in &polygon {
                let d1 = curr.0.dot(axis) - limit;
                if (d0 <= 0.0) != (d1 <= 0.0) {
                    let t = d0 / (d0 - d1);
                    next.push((prev.0.lerp(curr.0, t), prev.1.lerp(curr.1, t)));
                }
                if d1 <= 0.0 {
                    next.push(curr);
                }
                prev = curr;
                d0 = d1;
            }
            std::mem::swap(&mut polygon, &mut next);
        }
    }
    let (_, first) = *polygon.first()?;
    Some(polygon.iter().fold((first, first), |(lo, hi), &(_, uv)| (lo.min(uv), hi.max(uv))))
}

fn add_tiles(tiles: &mut Tiles, lo: Vec2, hi: Vec2, size: Vec2) {
    let lo = (lo - Vec2::ONE).floor().max(Vec2::ZERO).as_uvec2() / TILE;
    let hi = (hi + Vec2::ONE).ceil().min(size).max(Vec2::ZERO).as_uvec2();
    for y in lo.y..hi.y.div_ceil(TILE) {
        for x in lo.x..hi.x.div_ceil(TILE) {
            tiles.insert((x, y));
        }
    }
}

/// Zero `tiles` of a reused alpha map, so a new stroke starts with no coverage.
fn clear_tiles(queue: &wgpu::Queue, alpha: &wgpu::Texture, tiles: &Tiles) {
    let zeros = vec![0_u8; (TILE * TILE) as usize * size_of::<u16>()];
    let layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(TILE * size_of::<u16>() as u32), rows_per_image: None };
    for &(x, y) in tiles {
        queue.write_texture(tile_copy(alpha, x * TILE, y * TILE), &zeros, layout, extent(TILE, TILE));
    }
}

fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d { width, height, depth_or_array_layers: 1 }
}

fn texture(device: &wgpu::Device, size: [u32; 2], format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tc_brush_map"),
        size: extent(size[0], size[1]),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn buffer(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false })
}

fn storage<T: Pod>(device: &wgpu::Device, label: &str, data: &[T]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(data),
        usage: wgpu::BufferUsages::STORAGE,
    })
}

fn tile_copy(texture: &wgpu::Texture, x: u32, y: u32) -> wgpu::TexelCopyTextureInfo<'_> {
    wgpu::TexelCopyTextureInfo { origin: wgpu::Origin3d { x, y, z: 0 }, ..texture.as_image_copy() }
}

fn buffer_copy(buffer: &wgpu::Buffer, slot: usize) -> wgpu::TexelCopyBufferInfo<'_> {
    wgpu::TexelCopyBufferInfo {
        buffer,
        layout: wgpu::TexelCopyBufferLayout { offset: slot as u64 * TILE_BYTES, bytes_per_row: Some(ROW_BYTES), rows_per_image: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::headless::Headless;
    use texturecraft_geom::{OrbitCamera, glam::Vec3, primitives};
    use texturecraft_paint::{RasterCtx, Shape, StampBlending, StrokeCanvas, rasterize, rasterize_uv};

    #[test]
    fn gpu_projection_readback_matches_screen_stencil_sampling() {
        use texturecraft_geom::glam::Vec4;

        let Ok(mut gpu) = crate::headless::Headless::new() else { return };
        const SIZE: u32 = 128;
        let mesh = Arc::new(texturecraft_geom::primitives::plane(1.0, 1));
        let uv = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let bvh = Arc::new(Bvh::new(&mesh));
        for (tile, srgb) in [(false, true), (true, false)] {
            let stencil = Stencil {
                image: Arc::new(ImageRes::new(2, 2, vec![[0.3, 0.2, 0.1, 0.0], [0.0, 0.8, 1.4, 0.4], [0.9, 0.6, 0.1, 0.8], [0.04, 0.3, 0.5, 1.0]], srgb)),
                view_proj: Mat4::from_cols(Vec4::X, Vec4::Z, Vec4::Y, Vec4::W),
                center: [0.08, -0.025],
                half: [0.18, 0.23],
                rotation: 0.37,
                tile,
            };
            let input = BrushInput {
                mesh: mesh.clone(),
                bvh: bvh.clone(),
                uv: uv.clone(),
                material: 0,
                depth: None,
                settings: BrushSettings::default(),
                alpha_image: None,
                stencil: Some(stencil.clone()),
                limit: None,
            };
            let mut stroke = gpu.renderer.start_gpu_brush(&gpu.device, &gpu.queue, input).unwrap();
            let dab = UvStamp { center: Vec2::splat(0.5), radius: 0.41, angle: 0.0, flow: 0.8, hardness: 0.7, seed: 5, source: None };
            let tiles = stroke.append_uv(&[dab]).unwrap();
            let coverage = stroke.read_alpha(&tiles).unwrap();
            let projected = stroke.read_projection(&tiles).unwrap();
            assert_eq!(projected.keys().copied().collect::<Tiles>(), tiles);
            let mut skipped = 0;
            let mut sampled = 0;
            let mut clipped = 0;
            for ((tx, ty), samples) in projected {
                for (i, gpu) in samples.into_iter().enumerate() {
                    let x = tx * TILE + (i as u32 % TILE);
                    let y = ty * TILE + (i as u32 / TILE);
                    if coverage.get(x, y)[0] == 0 {
                        assert_eq!(gpu, [0.0; 4]);
                        skipped += 1;
                        continue;
                    }
                    let cpu = uv.surface(&mesh, (y * SIZE + x) as usize).and_then(|surface| stencil.sample_world(surface.pos));
                    if let Some(([r, g, b], alpha)) = cpu.filter(|&(_, alpha)| alpha > 0.0) {
                        assert!(gpu.iter().zip([r, g, b, alpha]).all(|(gpu, cpu)| (gpu - cpu).abs() <= 1e-5), "{x}, {y}: {gpu:?} vs {cpu:?}");
                        sampled += 1;
                    } else {
                        assert_eq!(gpu, [0.0; 4], "{x}, {y}: projected outside the image");
                        clipped += 1;
                    }
                }
            }
            assert!(sampled > 0 && skipped > 0);
            assert!(tile || clipped > 0);
        }
    }

    #[test]
    fn a_new_stroke_reuses_the_alpha_map_without_the_old_coverage() {
        let Ok(mut gpu) = Headless::new() else { return };
        let size = 128;
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let bvh = Arc::new(Bvh::new(&mesh));
        let uv = Arc::new(UvGBuffer::new(&mesh, 0, size, size));
        let input = || BrushInput {
            mesh: mesh.clone(),
            bvh: bvh.clone(),
            uv: uv.clone(),
            material: 0,
            depth: None,
            settings: BrushSettings::default(),
            alpha_image: None,
            stencil: None,
            limit: None,
        };
        let dab = |center: Vec2| UvStamp { center, radius: 0.1, angle: 0.0, flow: 0.9, hardness: 0.8, seed: 0, source: None };
        let mut first = gpu.renderer.start_gpu_brush(&gpu.device, &gpu.queue, input()).unwrap();
        let first_tiles = first.append_uv(&[dab(Vec2::splat(0.25))]).unwrap();
        let first_map = first.alpha.clone();
        drop(first);

        let mut second = gpu.renderer.start_gpu_brush(&gpu.device, &gpu.queue, input()).unwrap();
        let cleared = second.read_alpha(&first_tiles).unwrap();
        let second_dab = dab(Vec2::splat(0.7));
        let second_tiles = second.append_uv(&[second_dab]).unwrap();
        let actual = second.read_alpha(&second_tiles).unwrap();
        let mut cpu = StrokeCanvas::new(size, size);
        let shape = Shape::new(BrushAlpha::Round, None, None, BrushSettings::default().alpha_params);
        rasterize_uv(&mut cpu, &uv, shape, &[second_dab]);

        assert!(second.alpha == first_map, "the second stroke reuses the first one's map");
        assert_eq!(cleared.allocated().count(), 0, "the reused map starts without the old coverage");
        assert_alpha_pattern(&cpu.alpha, &actual, BrushAlpha::Round, "reused map");
    }

    #[test]
    fn procedural_alphas_keep_their_seeded_patterns_in_both_views() {
        let Ok(mut gpu) = Headless::new() else { return };
        let size = 128;
        let mesh = Arc::new(primitives::plane(1.0, 4));
        let bvh = Arc::new(Bvh::new(&mesh));
        let uv = Arc::new(UvGBuffer::new(&mesh, 0, size, size));
        for alpha in [BrushAlpha::Spray, BrushAlpha::Grunge, BrushAlpha::Cloud, BrushAlpha::Star, BrushAlpha::Streaks] {
            for (seed, invert) in [(19, false), (37, true)] {
                let mut settings = BrushSettings { alpha, backface_culling: false, ..Default::default() };
                settings.alpha_params.size_x = 0.75;
                settings.alpha_params.size_y = 0.9;
                settings.alpha_params.contrast = 1.3;
                settings.alpha_params.rotation = -27.0;
                settings.alpha_params.invert = invert;
                let shape = Shape::new(alpha, None, None, settings.alpha_params);
                let input = || BrushInput {
                    mesh: mesh.clone(),
                    bvh: bvh.clone(),
                    uv: uv.clone(),
                    material: 0,
                    depth: None,
                    settings: settings.clone(),
                    alpha_image: None,
                    stencil: None,
                    limit: None,
                };

                let uv_dab = UvStamp { center: Vec2::splat(0.48), radius: 0.29, angle: 35.0, flow: 0.8, hardness: 0.6, seed, source: None };
                let mut cpu = StrokeCanvas::new(size, size);
                rasterize_uv(&mut cpu, &uv, shape, &[uv_dab]);
                let mut stroke = gpu.renderer.start_gpu_brush(&gpu.device, &gpu.queue, input()).unwrap();
                let tiles = stroke.append_uv(&[uv_dab]).unwrap();
                let actual = stroke.read_alpha(&tiles).unwrap();
                assert_alpha_pattern(&cpu.alpha, &actual, alpha, "2D");

                let world_dab = Stamp {
                    center: Vec3::new(-0.03, 0.0, 0.02),
                    normal: Vec3::Y,
                    tangent: Vec3::X,
                    bitangent: Vec3::Z,
                    radius: 0.29,
                    flow: 0.8,
                    hardness: 0.6,
                    depth: 1.0,
                    seed,
                    source: None,
                };
                let context = RasterCtx {
                    mesh: &mesh,
                    bvh: &bvh,
                    gbuf: &uv,
                    material: 0,
                    depth: None,
                    eye: Vec3::Y,
                    backface_culling: false,
                    angle_fade: settings.angle_fade,
                    shape,
                };
                let mut cpu = StrokeCanvas::new(size, size);
                rasterize(&mut cpu, &context, &[world_dab]);
                let mut stroke = gpu.renderer.start_gpu_brush(&gpu.device, &gpu.queue, input()).unwrap();
                let tiles = stroke.append_world(&[world_dab]).unwrap();
                let actual = stroke.read_alpha(&tiles).unwrap();
                assert_alpha_pattern(&cpu.alpha, &actual, alpha, "3D");
            }
        }
    }

    fn assert_alpha_pattern(cpu: &TileImage, gpu: &TileImage, alpha: BrushAlpha, view: &str) {
        let (mut covered, mut differing) = (0, 0);
        for y in 0..cpu.height() {
            for x in 0..cpu.width() {
                let expected = cpu.get(x, y)[0];
                let observed = gpu.get(x, y)[0];
                covered += usize::from(expected != 0 || observed != 0);
                differing += usize::from(expected.abs_diff(observed) > 128);
            }
        }
        assert!(covered > 0, "{alpha:?} must stamp in {view}");
        // Barycentric quantization and different floating-point math can move a few texels
        // across a shape edge or the spray threshold.
        assert!(differing * 100 <= covered, "{alpha:?} in {view}: {differing}/{covered} alpha values differ");
    }

    fn world_brush_coverage(gpu: &mut Headless, mesh: Mesh, settings: BrushSettings, camera: Option<OrbitCamera>, dabs: &[Stamp]) -> (TileImage, TileImage) {
        let size = 128;
        let mesh = Arc::new(mesh);
        let bvh = Arc::new(Bvh::new(&mesh));
        let uv = Arc::new(UvGBuffer::new(&mesh, 0, size, size));
        let depth = camera.map(|camera| Arc::new(EyeDepth::render(&mesh, &camera, size, size)));
        let context = RasterCtx {
            mesh: &mesh,
            bvh: &bvh,
            gbuf: &uv,
            material: 0,
            depth: depth.as_deref(),
            eye: depth.as_ref().map_or(Vec3::ZERO, |depth| depth.eye),
            backface_culling: settings.backface_culling,
            angle_fade: settings.angle_fade,
            shape: Shape::new(settings.alpha, None, None, settings.alpha_params),
        };
        let mut cpu = StrokeCanvas::new(size, size);
        rasterize(&mut cpu, &context, dabs);

        let input = BrushInput { mesh, bvh, uv, material: 0, depth, settings, alpha_image: None, stencil: None, limit: None };
        let mut stroke = gpu.renderer.start_gpu_brush(&gpu.device, &gpu.queue, input).unwrap();
        let tiles = stroke.append_world(dabs).unwrap();
        (cpu.alpha, stroke.read_alpha(&tiles).unwrap())
    }

    #[test]
    fn tilted_world_dab_coverage_is_independent_of_uv_tile_placement() {
        let Ok(mut gpu) = Headless::new() else { return };
        let one_over_sqrt3 = 1.0 / 3.0_f32.sqrt();
        let one_over_sqrt6 = 1.0 / 6.0_f32.sqrt();
        let dab = Stamp {
            center: Vec3::ZERO,
            normal: Vec3::new(one_over_sqrt3, 2.0 * one_over_sqrt6, 0.0),
            tangent: Vec3::new(one_over_sqrt3, -one_over_sqrt6, std::f32::consts::FRAC_1_SQRT_2),
            bitangent: Vec3::new(one_over_sqrt3, -one_over_sqrt6, -std::f32::consts::FRAC_1_SQRT_2),
            radius: 1.0,
            flow: 1.0,
            hardness: 0.98,
            depth: 1.0,
            seed: 0,
            source: None,
        };
        // The second island fits in the rotated dab, but lies outside the old 1.42-radius world box.
        let positions =
            [Vec3::ZERO, Vec3::X * 1.6].into_iter().flat_map(|center| [center, center + Vec3::X * 0.01, center + Vec3::Y * 0.01]).collect::<Vec<_>>();
        for far_island_x in [24, 88] {
            let uvs = [8.0, far_island_x as f32]
                .into_iter()
                .flat_map(|x| [Vec2::new(x, 8.0), Vec2::new(x + 8.0, 8.0), Vec2::new(x, 16.0)].map(|uv| uv / 128.0))
                .collect();
            let mesh = Mesh::from_parts(positions.clone(), Some(vec![dab.normal; 6]), uvs, vec![0, 1, 2, 3, 4, 5], "Tilted islands").unwrap();
            let settings = BrushSettings { alpha: BrushAlpha::Square, backface_culling: false, ..Default::default() };
            let (cpu, actual) = world_brush_coverage(&mut gpu, mesh, settings, None, &[dab]);
            let expected = cpu.get(far_island_x + 2, 10)[0];
            assert!(expected > 0, "the dab reaches the far island at UV x={far_island_x}");
            assert_eq!(actual.get(far_island_x + 2, 10)[0], expected, "GPU coverage at UV x={far_island_x}");
        }
    }

    #[test]
    fn cancelled_surface_normal_uses_each_dabs_normal() {
        let Ok(mut gpu) = Headless::new() else { return };
        let mesh = Mesh::from_parts(
            vec![Vec3::new(-0.25, -0.25, 0.0), Vec3::new(0.75, -0.25, 0.0), Vec3::new(-0.25, 0.75, 0.0)],
            Some(vec![Vec3::Z, Vec3::NEG_Z, Vec3::NEG_Z]),
            [Vec2::new(16.5, 16.5), Vec2::new(80.5, 16.5), Vec2::new(16.5, 80.5)].map(|uv| uv / 128.0).to_vec(),
            vec![0, 1, 2],
            "Opposing normals",
        )
        .unwrap();
        let front = Stamp {
            center: Vec3::ZERO,
            normal: Vec3::Z,
            tangent: Vec3::X,
            bitangent: Vec3::Y,
            radius: 1.0,
            flow: 0.5,
            hardness: 0.9,
            depth: 1.0,
            seed: 0,
            source: None,
        };
        let back = Stamp { normal: Vec3::NEG_Z, bitangent: Vec3::NEG_Y, ..front };
        let settings = BrushSettings { backface_culling: true, ..Default::default() };
        let camera = OrbitCamera { pitch: 0.0, ..OrbitCamera::default() };
        let (cpu, actual) = world_brush_coverage(&mut gpu, mesh, settings, Some(camera), &[back, front]);
        // Exact weights (1/2, 1/4, 1/4) cancel here. Only the front-facing dab should paint.
        let expected = cpu.get(32, 32)[0];
        assert!(expected > 0 && expected < u16::MAX);
        assert_eq!(actual.get(32, 32)[0], expected);
    }

    /// Coverage within one 8-bit step, except visibility decisions flipped on at most 0.1 % of
    /// the covered texels.
    fn assert_coverage_matches(cpu: &TileImage, gpu: &TileImage, label: &str) {
        const EIGHT_BIT_STEP: u16 = 257;
        let (mut covered, mut differing) = (0, 0);
        for y in 0..cpu.height() {
            for x in 0..cpu.width() {
                let (expected, observed) = (cpu.get(x, y)[0], gpu.get(x, y)[0]);
                covered += usize::from(expected != 0 || observed != 0);
                differing += usize::from(expected.abs_diff(observed) > EIGHT_BIT_STEP);
            }
        }
        assert!(covered > 0, "{label}: nothing painted");
        assert!(differing * 1000 <= covered, "{label}: {differing}/{covered} texels differ");
    }

    #[test]
    fn symmetry_copies_see_the_texel_from_their_source_stamp() {
        use texturecraft_geom::glam::Mat3;
        use texturecraft_paint::StampSource;

        let Ok(mut gpu) = Headless::new() else { return };
        let camera = OrbitCamera { pitch: 1.2, ..OrbitCamera::default() };

        // A stroke and its copy across the X plane, with culling and the camera's visibility
        let mirror_x = StampSource { origin: Vec3::ZERO, to_source: Mat3::from_diagonal(Vec3::new(-1.0, 1.0, 1.0)) };
        let stroke = Stamp {
            center: Vec3::new(-0.25, 0.0, 0.1),
            normal: Vec3::Y,
            tangent: Vec3::X,
            bitangent: Vec3::Z,
            radius: 0.15,
            flow: 0.8,
            hardness: 0.7,
            depth: 1.0,
            seed: 3,
            source: None,
        };
        let copy = Stamp { center: Vec3::new(0.25, 0.0, 0.1), tangent: Vec3::NEG_X, bitangent: Vec3::Y.cross(Vec3::NEG_X), source: Some(mirror_x), ..stroke };
        let settings = BrushSettings { backface_culling: true, ..Default::default() };
        let (cpu, actual) = world_brush_coverage(&mut gpu, primitives::plane(1.0, 4), settings.clone(), Some(camera), &[stroke, copy]);
        assert_coverage_matches(&cpu, &actual, "mirrored plane");
        assert!(actual.get(96, 77)[0] > 0, "the copy paints across the plane");

        // A copy whose source faces away from the camera paints nothing, though its own texels face it
        let mirror_y = StampSource { origin: Vec3::ZERO, to_source: Mat3::from_diagonal(Vec3::new(1.0, -1.0, 1.0)) };
        let away = Stamp { center: Vec3::new(0.25, 0.0, 0.1), source: Some(mirror_y), ..stroke };
        let (cpu, actual) = world_brush_coverage(&mut gpu, primitives::plane(1.0, 4), settings.clone(), Some(camera), &[stroke, away]);
        assert_coverage_matches(&cpu, &actual, "copy of a back face");
        assert_eq!(actual.get(96, 77)[0], 0, "the copy sees the texel from its back-facing source");

        // A copy whose source point lies under another surface, inside the stamp depth
        let corners = [Vec3::new(-1.0, 0.03, -1.0), Vec3::new(1.0, 0.03, -1.0), Vec3::new(1.0, 0.03, 1.0), Vec3::new(-1.0, 0.03, 1.0)];
        let source_corners = corners.map(|point| Vec3::new(point.x, -0.02, point.z));
        let uvs = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y].into_iter().chain([Vec2::ZERO; 4]).collect();
        let indices = [0, 2, 1, 0, 3, 2, 4, 6, 5, 4, 7, 6].to_vec();
        let mesh = Mesh::from_parts(corners.into_iter().chain(source_corners).collect(), None, uvs, indices, "Asymmetric mirror").unwrap();
        let copy = Stamp { center: Vec3::new(0.0, 0.03, 0.0), radius: 0.3, flow: 1.0, hardness: 0.9, source: Some(mirror_y), ..stroke };
        let looking_down = OrbitCamera { pitch: 1.45, ..OrbitCamera::default() };
        let settings = BrushSettings { backface_culling: false, ..settings };
        let (cpu, actual) = world_brush_coverage(&mut gpu, mesh, settings, Some(looking_down), &[copy]);
        assert_coverage_matches(&cpu, &actual, "source behind its surface");
        assert!(actual.get(64, 64)[0] > 0, "the copy paints the visible texel");
    }

    #[test]
    fn world_stroke_stamp_blending_paints_both_sides_of_a_uv_seam() {
        let Ok(mut gpu) = Headless::new() else { return };
        let positions = vec![
            Vec3::new(-1.0, 0.0, -1.0),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(-1.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, -1.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 1.0),
        ];
        let uvs = [[0.05, 0.35], [0.65, 0.95]].into_iter().flat_map(|us| [0.1, 0.9].into_iter().flat_map(move |v| us.map(|u| Vec2::new(u, v)))).collect();
        let mesh = Arc::new(Mesh::from_parts(positions, Some(vec![Vec3::Y; 8]), uvs, vec![0, 2, 1, 1, 2, 3, 4, 6, 5, 5, 6, 7], "Seam").unwrap());
        let camera = OrbitCamera { pitch: 1.2, ..OrbitCamera::default() };
        let bvh = Arc::new(Bvh::new(&mesh));
        let uv = Arc::new(UvGBuffer::new(&mesh, 0, 128, 128));
        let depth = Arc::new(EyeDepth::render(&mesh, &camera, 128, 128));
        let dab = Stamp {
            center: Vec3::ZERO,
            normal: Vec3::Y,
            tangent: Vec3::X,
            bitangent: Vec3::Z,
            radius: 0.2,
            flow: 0.5,
            hardness: 0.8,
            depth: 1.0,
            seed: 0,
            source: None,
        };
        let pixel_coverage = [StampBlending::Lighten, StampBlending::Normal, StampBlending::Add].map(|stamp_blending| {
            // Every mode sees the same stamp on each island across separate GPU dispatches.
            let input = BrushInput {
                bvh: bvh.clone(),
                uv: uv.clone(),
                depth: Some(depth.clone()),
                mesh: mesh.clone(),
                material: 0,
                settings: BrushSettings { stamp_blending, ..BrushSettings::default() },
                alpha_image: None,
                stencil: None,
                limit: None,
            };
            let mut stroke = gpu.renderer.start_gpu_brush(&gpu.device, &gpu.queue, input).unwrap();
            let tiles = stroke.append_world(&[dab]).unwrap();
            let first = stroke.read_alpha(&tiles).unwrap();
            stroke.append_world(&[dab]).unwrap();
            let second = stroke.read_alpha(&tiles).unwrap();
            for x in [42, 85] {
                let single = first.get(x, 64)[0];
                assert!(single > 0, "both islands receive the same dab");
                let a = texturecraft_texel::from_u16(single);
                let expected = texturecraft_texel::to_u16(stamp_blending.accumulate(a, dab.flow));
                let blended = second.get(x, 64)[0];
                assert!(blended.abs_diff(expected) <= 1, "GPU {stamp_blending:?}: {blended} differs from {expected}");
            }
            assert_eq!(second.get(64, 64)[0], 0, "the gap between islands stays clear");
            assert_eq!(second.get(20, 64)[0], 0, "paint stays near the seam on the surface");
            second.get(42, 64)[0]
        });
        let [lighten, normal, add] = pixel_coverage;
        assert!(lighten < normal && normal < add);
    }
}
