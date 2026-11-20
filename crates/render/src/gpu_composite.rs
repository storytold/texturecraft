//! Compile the viewport's supported UV fills to a small GPU layer program.

use std::sync::{Arc, Weak};

use bytemuck::{Pod, Zeroable};
use rayon::prelude::*;
use texturecraft_compose::PixelStore;
use texturecraft_core::{BlendMode, Channel, Effect, EffectKind, Filter, LayerKind, MaskBackground, ProceduralKind, Project, Projection, Source};
use texturecraft_texel::{DilationMap, TILE, TILE_AREA, TileImage};
use wgpu::util::DeviceExt;

const MAX_LAYERS: usize = 128;
const MAX_EFFECTS: usize = 256;
const MAX_PAINT_WORDS: usize = (64 << 20) / size_of::<u32>();
pub(crate) const ROWS: u32 = 64;
const WORKGROUP: u32 = 8;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Operation {
    // Destination, source kind, noise seed, invert.
    ids: [u32; 4],
    // RGB low and layer weight.
    low: [f32; 4],
    high: [f32; 4],
    // UV rotation cosine/sine followed by projection XY scale.
    projection: [f32; 4],
    // UV offset XY, procedural scale, contrast.
    uv: [f32; 4],
    balance: [f32; 4],
    // Offsets and counts of the mask and content effects.
    effects: [u32; 4],
}

/// Inclusive origin and exclusive end of a material update, in document texels.
#[derive(Clone, Copy, Debug)]
pub struct CompositeRegion {
    pub(crate) min: [u32; 2],
    pub(crate) max: [u32; 2],
}

impl CompositeRegion {
    pub fn from_tiles(tiles: impl IntoIterator<Item = (u32, u32)>) -> Option<Self> {
        let mut tiles = tiles.into_iter();
        let (x, y) = tiles.next()?;
        let mut region = Self { min: [x * TILE, y * TILE], max: [(x + 1) * TILE, (y + 1) * TILE] };
        for (x, y) in tiles {
            region.min = [region.min[0].min(x * TILE), region.min[1].min(y * TILE)];
            region.max = [region.max[0].max((x + 1) * TILE), region.max[1].max((y + 1) * TILE)];
        }
        Some(region)
    }

    fn includes_tile(self, x: u32, y: u32) -> bool {
        let (x, y) = (x * TILE, y * TILE);
        x >= self.min[0] && x < self.max[0] && y >= self.min[1] && y < self.max[1]
    }

    pub fn expanded(self, texels: u32) -> Self {
        Self { min: self.min.map(|x| x.saturating_sub(texels)), max: self.max.map(|x| x.saturating_add(texels)) }
    }

    pub fn bounds(self) -> ([u32; 2], [u32; 2]) {
        (self.min, self.max)
    }

    pub(crate) fn viewport(self, shift: u32, size: (u32, u32)) -> Self {
        Self { min: [self.min[0] >> shift, self.min[1] >> shift], max: [(self.max[0] >> shift).min(size.0), (self.max[1] >> shift).min(size.1)] }
    }
}

#[derive(Debug)]
pub struct CompositeProgram {
    operations: Vec<Operation>,
    layer_count: u32,
    paint: Vec<u32>,
    pub(crate) region: Option<CompositeRegion>,
    pub(crate) coverage: Option<Arc<DilationMap>>,
    pub(crate) writes_height: bool,
    pub(crate) writes_normal: bool,
}

impl CompositeProgram {
    pub(crate) fn paint_bytes(&self) -> u64 {
        (self.paint.len() * size_of::<u32>()) as u64
    }

    /// Let the shader sample the nearest covered UV texel for island padding.
    pub fn set_coverage(&mut self, coverage: Option<Arc<DilationMap>>) {
        self.coverage = coverage;
    }

    /// A flat stack of sparse paint, solid and UV procedural fills, masks and point effects.
    /// Disabled layers need no work. Stacks outside this evaluator's domain use the CPU.
    pub fn compile(project: &Project, pixels: &PixelStore, ts: usize) -> Option<Self> {
        Self::compile_region(project, pixels, ts, None)
    }

    pub fn compile_region(project: &Project, pixels: &PixelStore, ts: usize, region: Option<CompositeRegion>) -> Option<Self> {
        use Channel::*;
        let set = project.texture_sets.get(ts)?;
        if !set.uv_tiles.is_empty() || set.has_channel(Glossiness) {
            return None;
        }
        let mut operations = Vec::new();
        let mut paint = vec![0];
        let mut effects = Vec::new();
        for id in &set.stack {
            let layer = project.layers.get(id)?;
            if !layer.visible {
                continue;
            }
            if layer.geometry_mask.is_some() || layer.height_mask.enabled {
                return None;
            }
            let mask = layer.mask.as_ref().filter(|m| m.enabled);
            let mask_start = effects.len() as u32;
            let mask_background = mask.map_or(1.0, |m| if m.background == MaskBackground::White { 1.0 } else { 0.0 });
            if let Some(mask) = mask {
                for effect in mask.effects.iter().filter(|e| e.visible) {
                    let mut op = match &effect.kind {
                        EffectKind::Fill { source, projection } => source_operation(Some(source), projection, [0.0; 3])?,
                        EffectKind::Paint | EffectKind::Path(_) => {
                            let Some(image) = pixels.masks.get(&effect.id).filter(|im| im.allocated_count() > 0) else { continue };
                            if (image.width(), image.height()) != set.size() || image.comps() != 2 {
                                return None;
                            }
                            Operation { ids: [0, 10, pack_paint(&mut paint, image, region.map(|r| r.expanded(TILE)))?, 2], ..Operation::zeroed() }
                        }
                        _ => point_effect(effect)?,
                    };
                    if !effect.opacity.is_finite() {
                        return None;
                    }
                    op.low[3] = effect.opacity.clamp(0.0, 1.0);
                    op.balance[2] = blend_code(effect.blend) as f32;
                    effects.push(op);
                }
            }
            let content_start = effects.len() as u32;
            let mask_count = content_start - mask_start;
            if mask_background == 0.0 && mask_count == 0 {
                continue;
            }
            for effect in layer.content_effects.iter().filter(|e| e.visible) {
                effects.push(point_effect(effect)?);
            }
            let content_count = effects.len() as u32 - content_start;
            if effects.len() > MAX_EFFECTS {
                return None;
            }
            let fill = match &layer.kind {
                LayerKind::Fill(fill) => Some(fill),
                LayerKind::Paint => None,
                _ => return None,
            };
            if fill.is_some_and(|fill| {
                matches!(fill.projection, Projection::Decal(_))
                    || fill.sources.values().any(|source| !matches!(source, Source::Uniform(_) | Source::Procedural { .. }))
            }) {
                return None;
            }

            for &(channel, _) in &set.channels {
                let Some(blend) = layer.blend(channel) else { continue };
                if blend.mode == BlendMode::Disable || blend.opacity <= 0.0 {
                    continue;
                }
                if !blend.opacity.is_finite() {
                    return None;
                }
                let painted = if fill.is_none() {
                    let Some(image) = pixels.layer(*id, channel).filter(|img| img.allocated_count() > 0) else { continue };
                    if (image.width(), image.height()) != set.size() || image.comps() != channel.components() + 1 {
                        return None;
                    }
                    Some(image)
                } else {
                    None
                };
                let default = channel.default_value();
                let source = fill.and_then(|fill| fill.sources.get(&channel));
                let slot = match channel {
                    BaseColor => 0,
                    Opacity => 1,
                    AmbientOcclusion => 2,
                    Roughness => 3,
                    Metallic => 4,
                    SpecularLevel => 5,
                    Height => 6,
                    Normal => 7,
                    _ if painted.is_none()
                        && (channel == Normal || channel == CoatNormal || content_count == 0)
                        && matches!(blend.mode, BlendMode::Normal | BlendMode::Passthrough | BlendMode::Replace)
                        && (source.is_none() || matches!(source, Some(Source::Uniform(value)) if *value == default)) =>
                    {
                        continue;
                    }
                    _ => return None,
                };
                let mut op = if let Some(image) = painted {
                    Operation { ids: [slot, 10, pack_paint(&mut paint, image, region.map(|r| r.expanded(TILE)))?, image.comps() as u32], ..Operation::zeroed() }
                } else {
                    source_operation(source, &fill?.projection, default)?
                };
                op.ids[0] = slot;
                op.low[3] = blend.opacity.min(1.0);
                op.balance[1] = mask_background;
                op.balance[2] = blend_code(blend.mode) as f32;
                op.effects = [mask_start, mask_count, content_start, content_count];
                if operations.len() == MAX_LAYERS {
                    return None;
                }
                operations.push(op);
            }
        }
        if operations.is_empty() {
            return None;
        }
        let changes_channel = |slot, default: [f32; 3]| {
            operations
                .iter()
                .any(|op| op.ids[0] == slot && (op.ids[1] != 0 || op.low[..3] != default || (slot != 7 && op.effects[3] != 0) || op.balance[2] != 0.0))
        };
        let writes_height = changes_channel(6, Height.default_value());
        let writes_normal = changes_channel(7, Normal.default_value());
        let layer_count = operations.len() as u32;
        operations.extend(effects);
        Some(Self { operations, layer_count, paint, region, coverage: None, writes_height, writes_normal })
    }
}

// Bytecode values used by composite.wgsl. Keep these stable if BlendMode is reordered.
fn blend_code(mode: BlendMode) -> u32 {
    use BlendMode::*;
    match mode {
        Normal | Passthrough => 0,
        Disable => 1,
        Replace => 2,
        Multiply => 3,
        Divide => 4,
        InverseDivide => 5,
        Darken => 6,
        Lighten => 7,
        LinearDodge => 8,
        Subtract => 9,
        InverseSubtract => 10,
        Difference => 11,
        Exclusion => 12,
        SignedAddition | LinearLight => 13,
        Overlay => 14,
        Screen => 15,
        LinearBurn => 16,
        ColorBurn => 17,
        ColorDodge => 18,
        SoftLight => 19,
        HardLight => 20,
        VividLight => 21,
        PinLight => 22,
        Tint => 23,
        Saturation => 24,
        Color => 25,
        Value => 26,
        NormalMapCombine => 27,
        NormalMapDetail => 28,
        NormalMapInverseDetail => 29,
    }
}

fn source_operation(source: Option<&Source>, projection: &Projection, default: [f32; 3]) -> Option<Operation> {
    let mut op = Operation::zeroed();
    match source {
        None => op.low[..3].copy_from_slice(&default),
        Some(Source::Uniform(value)) => op.low[..3].copy_from_slice(value),
        Some(Source::Procedural { pattern, low, high, ramp }) => {
            if pattern.resolution.can_reduce() || !ramp.is_empty() {
                return None;
            }
            let Projection::Uv { scale, rotation, offset } = projection else { return None };
            let kind = match pattern.kind {
                ProceduralKind::Clouds => 1,
                ProceduralKind::Perlin => 2,
                ProceduralKind::Noise => 3,
                ProceduralKind::Cells => 4,
                ProceduralKind::Grunge => 5,
                ProceduralKind::Scratches => 6,
                ProceduralKind::Wood => 7,
                ProceduralKind::Marble => 8,
                ProceduralKind::Gradient => 9,
                _ => return None,
            };
            let (sin, cos) = rotation.to_radians().sin_cos();
            op.ids = [0, kind, pattern.seed, u32::from(pattern.invert)];
            op.low[..3].copy_from_slice(low);
            op.high[..3].copy_from_slice(high);
            op.projection = [cos, sin, scale[0], scale[1]];
            op.uv = [offset[0], offset[1], pattern.scale.max(1e-4), pattern.contrast.max(0.0)];
            op.balance[0] = pattern.balance;
            if scale.iter().zip(offset).any(|(s, o)| (s.abs() * 2.0 + o.abs()) * op.uv[2] > 32_768.0) {
                return None;
            }
        }
        _ => return None,
    }
    op.low.iter().chain(&op.high).chain(&op.projection).chain(&op.uv).chain(&op.balance).all(|v| v.is_finite()).then_some(op)
}

fn point_effect(effect: &Effect) -> Option<Operation> {
    use Filter::*;
    if !effect.opacity.is_finite() {
        return None;
    }
    let (kind, params, out_low, out_high) = match &effect.kind {
        EffectKind::Levels(levels) => (20, [levels.in_low, levels.in_high, levels.gamma], levels.out_low, levels.out_high),
        EffectKind::Filter(filter) => match filter {
            Invert => (21, [0.0; 3], 0.0, 0.0),
            Threshold { level } => (22, [*level, 0.0, 0.0], 0.0, 0.0),
            ContrastLuminosity { contrast, luminosity } => (23, [*contrast, *luminosity, 0.0], 0.0, 0.0),
            Posterize { levels } => (24, [*levels, 0.0, 0.0], 0.0, 0.0),
            Clamp { min, max } => (25, [min.min(*max), min.max(*max), 0.0], 0.0, 0.0),
            HistogramScan { position, contrast } => (26, [*position, *contrast, 0.0], 0.0, 0.0),
            HistogramRange { range, position } => (27, [*range, *position, 0.0], 0.0, 0.0),
            HistogramShift { offset } => (28, [*offset, 0.0, 0.0], 0.0, 0.0),
            Grayscale => (29, [0.0; 3], 0.0, 0.0),
            NormalInvert => (30, [0.0; 3], 0.0, 0.0),
            Hsl { hue, saturation, lightness } => (31, [*hue, *saturation, *lightness], 0.0, 0.0),
            _ => return None,
        },
        _ => return None,
    };
    let low = [params[0], params[1], params[2], effect.opacity.clamp(0.0, 1.0)];
    low.iter().chain(&[out_low, out_high]).all(|x| x.is_finite()).then_some(Operation {
        ids: [0, kind, 0, 0],
        low,
        high: [out_low, out_high, 0.0, 0.0],
        ..Operation::zeroed()
    })
}

/// An index for each document tile followed by the texels of allocated tiles. Two u16
/// components fit in each word. A missing tile leaves the accumulated material untouched.
fn pack_paint(dst: &mut Vec<u32>, image: &TileImage, region: Option<CompositeRegion>) -> Option<u32> {
    let table = dst.len();
    let count = image.tiles_x() as usize * image.tiles_y() as usize;
    let words_per_tile = TILE_AREA * image.comps() / 2;
    if table.checked_add(count)? > MAX_PAINT_WORDS {
        return None;
    }
    dst.resize(table + count, 0);
    for (x, y) in image.allocated().filter(|&(x, y)| region.is_none_or(|r| r.includes_tile(x, y))) {
        if dst.len().checked_add(words_per_tile)? > MAX_PAINT_WORDS {
            return None;
        }
        let texels = image.tile(x, y)?;
        let (pairs, remainder) = texels.as_chunks::<2>();
        if pairs.len() != words_per_tile || !remainder.is_empty() {
            return None;
        }
        let offset = dst.len() as u32;
        dst[table + (y * image.tiles_x() + x) as usize] = offset;
        dst.extend(pairs.iter().map(|&[a, b]| u32::from(a) | (u32::from(b) << 16)));
    }
    Some(table as u32)
}

pub(crate) struct GpuCompositor {
    pipeline: wgpu::ComputePipeline,
    lookup: wgpu::Buffer,
    uniform_coverage: wgpu::Buffer,
    coverage: Option<(Weak<DilationMap>, u32, wgpu::Buffer)>,
}

impl GpuCompositor {
    pub(crate) fn supports(device: &wgpu::Device) -> bool {
        let limits = device.limits();
        limits.max_storage_buffers_per_shader_stage >= 5
            && limits.max_compute_invocations_per_workgroup >= WORKGROUP * WORKGROUP
            && limits.max_compute_workgroup_size_x >= WORKGROUP
            && limits.max_compute_workgroup_size_y >= WORKGROUP
    }

    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tc_gpu_composite"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/composite.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("tc_gpu_composite"),
            layout: None,
            module: &module,
            entry_point: Some("compose"),
            compilation_options: Default::default(),
            cache: None,
        });
        let values = texturecraft_texel::srgb_lut().map(u32::from);
        let lookup = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_composite_srgb"),
            contents: bytemuck::cast_slice(&values),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let uniform_coverage = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_composite_uniform_coverage"),
            contents: &[0; 4],
            usage: wgpu::BufferUsages::STORAGE,
        });
        Self { pipeline, lookup, uniform_coverage, coverage: None }
    }

    fn coverage_buffer(&mut self, device: &wgpu::Device, map: Option<&Arc<DilationMap>>, shift: u32, size: (u32, u32)) -> Result<wgpu::Buffer, String> {
        let Some(map) = map else { return Ok(self.uniform_coverage.clone()) };
        let key = Arc::downgrade(map);
        if let Some((cached, last_shift, buffer)) = &self.coverage
            && key.ptr_eq(cached)
            && shift == *last_shift
        {
            return Ok(buffer.clone());
        }
        let (w, h) = (size.0 as usize, size.1 as usize);
        let count = (w * h).div_ceil(2);
        let limits = device.limits();
        if (count * 4) as u64 > limits.max_storage_buffer_binding_size.min(limits.max_buffer_size) {
            return Err("UV padding data exceeds the adapter's storage-buffer limit".into());
        }
        let sample = |i: usize| -> Option<u32> {
            if i >= w * h {
                return Some(0xffff);
            }
            let (x, y) = (((i % w) as u32) << shift, ((i / w) as u32) << shift);
            let Some((sx, sy)) = map.source(x, y) else { return Some(0xffff) };
            let (dx, dy) = (sx as i32 - x as i32, sy as i32 - y as i32);
            if !(-31..=31).contains(&dx) || !(-31..=31).contains(&dy) {
                return None;
            }
            Some(((dx + 31) as u32) | (((dy + 31) as u32) << 6))
        };
        // Store two offset pairs per word. The source atlas already owns its exact, dense
        // padding map. A reduced viewport only uploads offsets for the texels it will sample.
        let words: Vec<u32> = (0..count)
            .into_par_iter()
            .map(|i| Some(sample(2 * i)? | (sample(2 * i + 1)? << 16)))
            .collect::<Option<_>>()
            .ok_or("UV padding exceeds the shader's offset range")?;
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_composite_coverage"),
            contents: bytemuck::cast_slice(&words),
            usage: wgpu::BufferUsages::STORAGE,
        });
        self.coverage = Some((key, shift, buffer.clone()));
        Ok(buffer)
    }

    /// Compose bands into reusable GPU scratch space, then copy them into display textures.
    /// Indexed/SRGB and floating-point maps use the same formats as the CPU uploader.
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        program: &CompositeProgram,
        source_size: (u32, u32),
        shift: u32,
        textures: [&wgpu::Texture; 4],
    ) -> Result<(), String> {
        let extent = (textures[0].width(), textures[0].height());
        let coverage = self.coverage_buffer(device, program.coverage.as_ref(), shift, extent)?;
        let region = program.region.map(|r| r.viewport(shift, extent)).unwrap_or(CompositeRegion { min: [0; 2], max: [extent.0, extent.1] });
        let [left, start_y] = region.min;
        let width = region.max[0].saturating_sub(left);
        let height = region.max[1].saturating_sub(start_y);
        if width == 0 || height == 0 {
            return Ok(());
        }
        let bands = height.div_ceil(ROWS);
        let pitch = width.next_multiple_of(64);
        let area = pitch * ROWS;
        let stride = u64::from(area) * 4;
        let scratch = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tc_composite_scratch"),
            size: stride
                * if program.writes_normal {
                    6
                } else if program.writes_height {
                    4
                } else {
                    2
                },
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let ops = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_composite_operations"),
            contents: bytemuck::cast_slice(&program.operations),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let paint = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_composite_paint"),
            contents: bytemuck::cast_slice(&program.paint),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let layout = self.pipeline.get_bind_group_layout(0);
        for band in 0..bands {
            let top = band * ROWS;
            let rows = ROWS.min(height - top);
            debug_assert_eq!(TILE, 64);
            let values = [
                width,
                rows,
                top + start_y,
                area,
                source_size.0,
                source_size.1,
                1 << shift,
                u32::from(program.writes_height),
                program.layer_count,
                pitch,
                u32::from(program.writes_normal),
                left,
                u32::from(program.coverage.is_some()),
                extent.0,
                0,
                0,
            ];
            let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tc_composite_band"),
                contents: bytemuck::cast_slice(&values),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tc_composite_band"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: ops.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: self.lookup.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: scratch.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: paint.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: coverage.as_entire_binding() },
                ],
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("tc_composite"), timestamp_writes: None });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bindings, &[]);
                pass.dispatch_workgroups(width.div_ceil(WORKGROUP), rows.div_ceil(WORKGROUP), 1);
            }
            let copies = [(0, 4, true), (1, 4, true), (2, 8, program.writes_height), (4, 8, program.writes_normal)];
            for (texture, (offset, bytes_per_texel, needed)) in textures.into_iter().zip(copies) {
                if !needed {
                    continue;
                }
                encoder.copy_buffer_to_texture(
                    wgpu::TexelCopyBufferInfo {
                        buffer: &scratch,
                        layout: wgpu::TexelCopyBufferLayout { offset: stride * offset, bytes_per_row: Some(pitch * bytes_per_texel), rows_per_image: None },
                    },
                    wgpu::TexelCopyTextureInfo { origin: wgpu::Origin3d { x: left, y: start_y + top, z: 0 }, ..texture.as_image_copy() },
                    wgpu::Extent3d { width, height: rows, depth_or_array_layers: 1 },
                );
            }
        }
        Ok(())
    }
}
