//! Compile the viewport's supported UV fills to a small GPU layer program.

use std::collections::BTreeMap;
use std::sync::{Arc, Weak};

use bytemuck::{Pod, Zeroable};
use rayon::prelude::*;
use texturecraft_compose::{BakedMaps, PixelStore};
use texturecraft_core::{
    BakedMap, BlendMode, Channel, Effect, EffectKind, Filter, Generator, GeneratorKind, LayerKind, MaskBackground, ProceduralKind, Project, Projection, Source,
};
use texturecraft_geom::{Mesh, NO_TRI, UvGBuffer, glam::Vec3};
use texturecraft_texel::{TILE, TILE_AREA, TileImage};
use wgpu::util::DeviceExt;

const MAX_LAYERS: usize = 128;
const MAX_EFFECTS: usize = 256;
const MAX_PAINT_WORDS: usize = (64 << 20) / size_of::<u32>();
pub(crate) const ROWS: u32 = 64;
const WORKGROUP: u32 = 8;
const BAKE_MAPS: [BakedMap; 4] = [BakedMap::AmbientOcclusion, BakedMap::Normal, BakedMap::Curvature, BakedMap::Thickness];

#[derive(Debug)]
struct BakedInputs {
    width: u32,
    height: u32,
    maps: [Option<Arc<Vec<f32>>>; BAKE_MAPS.len()],
}

impl BakedInputs {
    fn new(baked: &BakedMaps) -> Result<Option<Self>, &'static str> {
        let maps = BAKE_MAPS.map(|map| baked.maps.get(&map).filter(|data| !data.is_empty()).cloned());
        let Some(index) = maps.iter().position(Option::is_some) else { return Ok(None) };
        let height = baked.height(BAKE_MAPS[index]);
        if height == 0 || baked.resolution == 0 || BAKE_MAPS.iter().zip(&maps).any(|(&map, data)| data.is_some() && baked.height(map) != height) {
            return Err("The baked maps have different layouts and use CPU compositing.");
        }
        Ok(Some(Self { width: baked.resolution, height, maps }))
    }
}

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
    coverage: Option<(Arc<UvGBuffer>, u32)>,
    surface: Option<(Arc<Mesh>, Arc<UvGBuffer>)>,
    needs_surface: bool,
    baked: Option<BakedInputs>,
    pub(crate) writes_height: bool,
    pub(crate) writes_normal: bool,
}

impl CompositeProgram {
    pub(crate) fn paint_bytes(&self) -> u64 {
        (self.paint.len() * size_of::<u32>()) as u64
    }

    pub fn is_neutral(&self) -> bool {
        self.layer_count == 0 && self.baked.as_ref().is_none_or(|baked| baked.maps[0].is_none() && baked.maps[1].is_none())
    }

    /// Let the shader sample the nearest covered UV texel for island padding.
    pub fn set_coverage(&mut self, coverage: Arc<UvGBuffer>, radius: u32) {
        self.coverage = Some((coverage, radius.min(15)));
    }

    /// Mesh-space projections and generators share the viewport's surface and bake inputs.
    pub fn set_surface_inputs(&mut self, mesh: &Arc<Mesh>, gbuf: &Arc<UvGBuffer>, baked: Option<&BakedMaps>) -> Result<(), &'static str> {
        if self.needs_surface {
            self.surface = Some((mesh.clone(), gbuf.clone()));
        }
        self.baked = baked.map(BakedInputs::new).transpose()?.flatten();
        self.writes_normal |= self.baked.as_ref().is_some_and(|baked| baked.maps[1].is_some());
        Ok(())
    }

    /// A flat stack of sparse paint, solid and UV procedural fills, masks and point effects.
    /// Disabled layers need no work. Stacks outside this evaluator's domain use the CPU.
    pub fn compile(project: &Project, pixels: &PixelStore, ts: usize) -> Option<Self> {
        Self::compile_region(project, pixels, ts, None)
    }

    pub fn compile_region(project: &Project, pixels: &PixelStore, ts: usize, region: Option<CompositeRegion>) -> Option<Self> {
        Self::try_compile_region(project, pixels, ts, region).ok()
    }

    /// Also provide the reason the viewport must select its CPU compositor.
    pub fn try_compile_region(project: &Project, pixels: &PixelStore, ts: usize, region: Option<CompositeRegion>) -> Result<Self, &'static str> {
        use Channel::*;
        let set = project.texture_sets.get(ts).ok_or("The texture set is missing.")?;
        if !set.uv_tiles.is_empty() {
            return Err("UDIM texture sets use CPU compositing.");
        }
        if set.has_channel(Glossiness) {
            return Err("The Glossiness channel uses CPU compositing.");
        }
        let mut operations = Vec::new();
        let mut paint = vec![0];
        let mut effects = Vec::new();
        let mut layers: Vec<_> = set.stack.iter().rev().map(|&id| (id, 0)).collect();
        while let Some((id, depth)) = layers.pop() {
            let layer = project.layers.get(&id).ok_or("A layer is missing from the material.")?;
            if !layer.visible {
                continue;
            }
            if layer.geometry_mask.is_some() || layer.height_mask.enabled {
                return Err("A geometry or height mask uses CPU compositing.");
            }
            let mask = layer.mask.as_ref().filter(|m| m.enabled);
            if let LayerKind::Group { children, .. } = &layer.kind {
                if depth >= texturecraft_core::project::MAX_DEPTH
                    || mask.is_some()
                    || !layer.content_effects.is_empty()
                    || set
                        .channels
                        .iter()
                        .any(|&(channel, _)| layer.blend(channel).is_none_or(|blend| blend.mode != BlendMode::Passthrough || blend.opacity != 1.0))
                {
                    return Err("A masked or modified group uses CPU compositing.");
                }
                layers.extend(children.iter().rev().map(|&child| (child, depth + 1)));
                continue;
            }
            let mask_start = effects.len() as u32;
            let mask_background = mask.map_or(1.0, |m| if m.background == MaskBackground::White { 1.0 } else { 0.0 });
            if let Some(mask) = mask {
                for effect in mask.effects.iter().filter(|e| e.visible) {
                    let mut op = match &effect.kind {
                        EffectKind::Fill { source, projection } => {
                            source_operation(Some(source), projection, [0.0; 3]).ok_or("A mask fill source or projection uses CPU compositing.")?
                        }
                        EffectKind::Paint | EffectKind::Path(_) => {
                            let Some(image) = pixels.masks.get(&effect.id).filter(|im| im.allocated_count() > 0) else { continue };
                            if (image.width(), image.height()) != set.size() || image.comps() != 2 {
                                return Err("A painted mask has a layout that uses CPU compositing.");
                            }
                            let offset =
                                pack_paint(&mut paint, image, region.map(|r| r.expanded(TILE))).ok_or("The painted material exceeds the GPU upload budget.")?;
                            Operation { ids: [0, 10, offset, 2], ..Operation::zeroed() }
                        }
                        EffectKind::Generator(generator) => generator_operation(generator).ok_or("A mask generator uses CPU compositing.")?,
                        _ => point_effect(effect).ok_or("A mask effect uses CPU compositing.")?,
                    };
                    if !effect.opacity.is_finite() {
                        return Err("A mask effect has an invalid opacity.");
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
                effects.push(point_effect(effect).ok_or("A layer effect uses CPU compositing.")?);
            }
            let content_count = effects.len() as u32 - content_start;
            if effects.len() > MAX_EFFECTS {
                return Err("The number of effects exceeds the GPU compositor limit.");
            }
            let fill = match &layer.kind {
                LayerKind::Fill(fill) => Some(fill),
                LayerKind::Paint => None,
                _ => return Err("A group or another layer type uses CPU compositing."),
            };
            if fill.is_some_and(|fill| {
                matches!(fill.projection, Projection::Decal(_))
                    || fill.sources.values().any(|source| !matches!(source, Source::Uniform(_) | Source::Procedural { .. }))
            }) {
                return Err("An image source or decal uses CPU compositing.");
            }

            for &(channel, _) in &set.channels {
                let Some(blend) = layer.blend(channel) else { continue };
                if blend.mode == BlendMode::Disable || blend.opacity <= 0.0 {
                    continue;
                }
                if !blend.opacity.is_finite() {
                    return Err("A layer has an invalid opacity.");
                }
                let painted = if fill.is_none() {
                    let Some(image) = pixels.layer(id, channel).filter(|img| img.allocated_count() > 0) else { continue };
                    if (image.width(), image.height()) != set.size() || image.comps() != channel.components() + 1 {
                        return Err("A paint layer has a layout that uses CPU compositing.");
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
                    _ => return Err("A channel in the layer stack uses CPU compositing."),
                };
                let mut op = if let Some(image) = painted {
                    let offset =
                        pack_paint(&mut paint, image, region.map(|r| r.expanded(TILE))).ok_or("The painted material exceeds the GPU upload budget.")?;
                    Operation { ids: [slot, 10, offset, image.comps() as u32], ..Operation::zeroed() }
                } else {
                    fill.and_then(|fill| source_operation(source, &fill.projection, default)).ok_or("A fill source or projection uses CPU compositing.")?
                };
                op.ids[0] = slot;
                op.low[3] = blend.opacity.min(1.0);
                op.balance[1] = mask_background;
                op.balance[2] = blend_code(blend.mode) as f32;
                op.effects = [mask_start, mask_count, content_start, content_count];
                if operations.len() == MAX_LAYERS {
                    return Err("The number of layer channels exceeds the GPU compositor limit.");
                }
                operations.push(op);
            }
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
        let needs_surface = operations.iter().any(|op| op.balance[3] > 0.0 || op.ids[1] >= 40);
        Ok(Self { operations, layer_count, paint, region, coverage: None, surface: None, needs_surface, baked: None, writes_height, writes_normal })
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
                ProceduralKind::Pores => 11,
                ProceduralKind::Chips => 12,
                ProceduralKind::Streaks => 13,
                _ => return None,
            };
            op.ids = [0, kind, pattern.seed, u32::from(pattern.invert)];
            op.low[..3].copy_from_slice(low);
            op.high[..3].copy_from_slice(high);
            op.uv[2] = pattern.scale.max(1e-4);
            op.uv[3] = pattern.contrast.max(0.0);
            op.balance[0] = pattern.balance;
            match projection {
                Projection::Uv { scale, rotation, offset } => {
                    let (sin, cos) = rotation.to_radians().sin_cos();
                    op.projection = [cos, sin, scale[0], scale[1]];
                    op.uv[..2].copy_from_slice(offset);
                    if scale.iter().zip(offset).any(|(s, o)| (s.abs() * 2.0 + o.abs()) * op.uv[2] > 32_768.0) {
                        return None;
                    }
                }
                Projection::Triplanar { scale, sharpness, offset } => {
                    let frequency = *scale * op.uv[2];
                    if offset.iter().any(|o| frequency.abs() + o.abs() > 32_768.0) {
                        return None;
                    }
                    op.projection = [offset[0], offset[1], offset[2], frequency];
                    op.balance[3] = sharpness.max(1.0);
                }
                _ => return None,
            }
        }
        _ => return None,
    }
    op.low.iter().chain(&op.high).chain(&op.projection).chain(&op.uv).chain(&op.balance).all(|v| v.is_finite()).then_some(op)
}

fn generator_operation(generator: &Generator) -> Option<Operation> {
    if generator.detail.is_some()
        || generator.micro.is_some()
        || !generator.inputs.is_empty()
        || generator.resolution.can_reduce()
        || !generator.global.is_neutral()
    {
        return None;
    }
    let kind = match generator.kind {
        GeneratorKind::EdgeWear => 40,
        GeneratorKind::Dirt => 41,
        GeneratorKind::ThinAreas => 42,
        GeneratorKind::Curvature => 43,
        GeneratorKind::AmbientOcclusion => 44,
        GeneratorKind::Position => 45,
        GeneratorKind::Direction => 46,
    };
    let direction = Vec3::from(generator.direction).try_normalize().unwrap_or(Vec3::Y);
    let op = Operation {
        ids: [0, kind, generator.seed, u32::from(generator.invert)],
        low: [generator.level, generator.softness.max(Generator::MIN_SOFTNESS), generator.noise_amount, 0.0],
        high: [direction.x, direction.y, direction.z, 0.0],
        projection: [generator.noise_scale.max(1e-3), generator.offset, 0.0, 0.0],
        ..Operation::zeroed()
    };
    op.low.iter().chain(&op.projection).all(|x| x.is_finite()).then_some(op)
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

fn coverage_word(triangles: &[u32]) -> u32 {
    triangles.iter().enumerate().fold(0, |word, (bit, &triangle)| word | (u32::from(triangle != NO_TRI) << bit))
}

fn input_texture(device: &wgpu::Device, label: &str, size: (u32, u32), layers: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: layers },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn bake_view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() })
}

fn write_input(queue: &wgpu::Queue, texture: &wgpu::Texture, size: (u32, u32), row_layer: (u32, u32), bytes: &[u8]) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { origin: wgpu::Origin3d { x: 0, y: row_layer.0, z: row_layer.1 }, ..texture.as_image_copy() },
        bytes,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size.0 * 8), rows_per_image: None },
        wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
    );
}

struct SurfaceCache {
    gbuf: Weak<UvGBuffer>,
    view: wgpu::TextureView,
    vertices: wgpu::Buffer,
}

struct BakeCache {
    size: (u32, u32),
    maps: [Option<Weak<Vec<f32>>>; BAKE_MAPS.len()],
    view: wgpu::TextureView,
}

pub(crate) struct GpuCompositor {
    pipeline: wgpu::ComputePipeline,
    lookup: wgpu::Buffer,
    uniform_coverage: wgpu::Buffer,
    coverage: Vec<(Weak<UvGBuffer>, wgpu::Buffer)>,
    surfaces: Vec<SurfaceCache>,
    baked: BTreeMap<usize, BakeCache>,
    default_surface: wgpu::TextureView,
    default_vertices: wgpu::Buffer,
    default_baked: wgpu::TextureView,
}

impl GpuCompositor {
    pub(crate) fn supports(device: &wgpu::Device) -> bool {
        let limits = device.limits();
        limits.max_storage_buffers_per_shader_stage >= 6
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
        let default_surface = input_texture(device, "tc_composite_empty_surface", (1, 1), 1, wgpu::TextureFormat::Rg32Uint).create_view(&Default::default());
        let default_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_composite_empty_vertices"),
            contents: &[0; 32],
            usage: wgpu::BufferUsages::STORAGE,
        });
        let default_baked = bake_view(&input_texture(device, "tc_composite_empty_bakes", (1, 1), 2, wgpu::TextureFormat::Rgba16Float));
        Self {
            pipeline,
            lookup,
            uniform_coverage,
            coverage: Vec::new(),
            surfaces: Vec::new(),
            baked: BTreeMap::new(),
            default_surface,
            default_vertices,
            default_baked,
        }
    }

    fn surface_inputs(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, program: &CompositeProgram) -> Result<(wgpu::TextureView, wgpu::Buffer), String> {
        let Some((mesh, gbuf)) = &program.surface else {
            if program.needs_surface {
                return Err("Missing UV surface inputs for GPU layer composition".into());
            }
            return Ok((self.default_surface.clone(), self.default_vertices.clone()));
        };
        self.surfaces.retain(|cache| cache.gbuf.strong_count() != 0);
        let key = Arc::downgrade(gbuf);
        if let Some(cache) = self.surfaces.iter().find(|cache| cache.gbuf.ptr_eq(&key)) {
            return Ok((cache.view.clone(), cache.vertices.clone()));
        }
        let limits = device.limits();
        let vertex_bytes = mesh.indices.len() as u64 * 32;
        if gbuf.width.max(gbuf.height) > limits.max_texture_dimension_2d || vertex_bytes > limits.max_storage_buffer_binding_size.min(limits.max_buffer_size) {
            return Err("UV surface data exceeds the GPU limits".into());
        }
        let (lo, hi) = mesh.bounds();
        let center = (lo + hi) * 0.5;
        let radius = ((hi - lo).length() * 0.5).max(1e-4);
        let vertex_data: Vec<[[f32; 4]; 2]> = mesh
            .indices
            .iter()
            .map(|&index| {
                let index = index as usize;
                let pos = (mesh.positions.get(index).copied().unwrap_or_default() - center) / radius;
                let normal = mesh.normals.get(index).copied().unwrap_or(Vec3::Y);
                [pos.extend(0.0).to_array(), normal.extend(0.0).to_array()]
            })
            .collect();
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_composite_surface_vertices"),
            contents: bytemuck::cast_slice(&vertex_data),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let texture = input_texture(device, "tc_composite_uv_surface", (gbuf.width, gbuf.height), 1, wgpu::TextureFormat::Rg32Uint);
        let stride = (gbuf.width * ROWS) as usize;
        for (band, (tri, bary)) in gbuf.tri.chunks(stride).zip(gbuf.bary.chunks(stride)).enumerate() {
            let pixels: Vec<[u32; 2]> = tri
                .par_iter()
                .zip(bary)
                .map(|(&tri, &[u, v])| {
                    let pack = |x: f32| u32::from(texturecraft_texel::to_u16(x));
                    [tri, pack(u) | (pack(v) << 16)]
                })
                .collect();
            write_input(queue, &texture, (gbuf.width, tri.len() as u32 / gbuf.width), (band as u32 * ROWS, 0), bytemuck::cast_slice(&pixels));
        }
        let view = texture.create_view(&Default::default());
        self.surfaces.push(SurfaceCache { gbuf: key, view: view.clone(), vertices: vertices.clone() });
        Ok((view, vertices))
    }

    pub(crate) fn retain_texture_sets(&mut self, count: usize) {
        self.baked.retain(|&ts, _| ts < count);
    }

    fn baked_input(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, ts: usize, baked: Option<&BakedInputs>) -> Result<wgpu::TextureView, String> {
        let Some(baked) = baked else {
            self.baked.remove(&ts);
            return Ok(self.default_baked.clone());
        };
        let size = (baked.width, baked.height);
        let refs = baked.maps.each_ref().map(|map| map.as_ref().map(Arc::downgrade));
        if let Some(cache) = self.baked.get(&ts).filter(|cache| {
            size == cache.size
                && refs.iter().zip(&cache.maps).all(|(a, b)| match (a, b) {
                    (Some(a), Some(b)) => a.ptr_eq(b),
                    (None, None) => true,
                    _ => false,
                })
        }) {
            return Ok(cache.view.clone());
        }
        if size.0.max(size.1) > device.limits().max_texture_dimension_2d {
            return Err("Baked maps exceed the GPU texture dimensions".into());
        }
        self.baked.remove(&ts);
        let texture = input_texture(device, "tc_composite_bakes", size, 2, wgpu::TextureFormat::Rgba16Float);
        let value = |map: usize, i: usize, fallback: f32| baked.maps[map].as_ref().and_then(|data| data.get(i).copied()).unwrap_or(fallback);
        for top in (0..size.1).step_by(ROWS as usize) {
            let end = (top + ROWS).min(size.1);
            let indices = top as usize * size.0 as usize..end as usize * size.0 as usize;
            let scalars: Vec<[u16; 4]> = indices
                .clone()
                .into_par_iter()
                .map(|i| [value(0, i, 1.0), value(2, i, 0.5), value(3, i, 1.0), 0.0].map(|v| half::f16::from_f32(v).to_bits()))
                .collect();
            write_input(queue, &texture, (size.0, end - top), (top, 0), bytemuck::cast_slice(&scalars));
            let normals: Vec<[u16; 4]> = indices
                .into_par_iter()
                .map(|i| {
                    let n = [value(1, i * 3, 0.5), value(1, i * 3 + 1, 0.5), value(1, i * 3 + 2, 1.0)];
                    let valid = n.iter().all(|v| (0.0..=1.0).contains(v));
                    [n[0], n[1], n[2], if valid { 1.0 } else { 0.0 }].map(|v| half::f16::from_f32(v).to_bits())
                })
                .collect();
            write_input(queue, &texture, (size.0, end - top), (top, 1), bytemuck::cast_slice(&normals));
        }
        let view = bake_view(&texture);
        self.baked.insert(ts, BakeCache { size, maps: refs, view: view.clone() });
        Ok(view)
    }

    fn coverage_buffer(&mut self, device: &wgpu::Device, map: Option<&Arc<UvGBuffer>>) -> Result<wgpu::Buffer, String> {
        let Some(map) = map else { return Ok(self.uniform_coverage.clone()) };
        let key = Arc::downgrade(map);
        self.coverage.retain(|(cached, _)| cached.strong_count() != 0);
        if let Some((_, buffer)) = self.coverage.iter().find(|(cached, _)| key.ptr_eq(cached)) {
            return Ok(buffer.clone());
        }
        let width = map.width as usize;
        let words_per_row = width.div_ceil(32);
        let count = words_per_row * map.height as usize;
        let limits = device.limits();
        if (count * 4) as u64 > limits.max_storage_buffer_binding_size.min(limits.max_buffer_size) {
            return Err("UV padding data exceeds the adapter's storage-buffer limit".into());
        }
        // Each word packs 32 coverage samples. The same buffer serves every display resolution.
        let words: Vec<u32> = if width.is_multiple_of(32) {
            map.tri.par_chunks(32).map(coverage_word).collect()
        } else {
            (0..count)
                .into_par_iter()
                .map(|i| {
                    let row = i / words_per_row * width;
                    let start = row + i % words_per_row * 32;
                    coverage_word(&map.tri[start..(start + 32).min(row + width)])
                })
                .collect()
        };
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_composite_coverage"),
            contents: bytemuck::cast_slice(&words),
            usage: wgpu::BufferUsages::STORAGE,
        });
        self.coverage.push((key, buffer.clone()));
        Ok(buffer)
    }

    /// Compose bands into reusable GPU scratch space, then copy them into display textures.
    /// Indexed/SRGB and floating-point maps use the same formats as the CPU uploader.
    pub(crate) fn encode(
        &mut self,
        gpu: (&wgpu::Device, &wgpu::Queue),
        encoder: &mut wgpu::CommandEncoder,
        program: &CompositeProgram,
        target: (usize, (u32, u32), u32),
        textures: [&wgpu::Texture; 4],
    ) -> Result<(), String> {
        let (device, queue) = gpu;
        let (ts, source_size, shift) = target;
        let extent = (textures[0].width(), textures[0].height());
        let coverage = self.coverage_buffer(device, program.coverage.as_ref().map(|(map, _)| map))?;
        let (surface, vertices) = self.surface_inputs(device, queue, program)?;
        let baked = self.baked_input(device, queue, ts, program.baked.as_ref())?;
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
        let empty_operation = Operation::zeroed();
        let ops = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_composite_operations"),
            contents: if program.operations.is_empty() { bytemuck::bytes_of(&empty_operation) } else { bytemuck::cast_slice(&program.operations) },
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
                source_size.0.div_ceil(32),
                program.coverage.as_ref().map_or(0, |(_, radius)| *radius),
                u32::from(program.surface.is_some())
                    | (u32::from(program.baked.is_some()) << 1)
                    | (u32::from(program.baked.as_ref().is_some_and(|baked| baked.maps[1].is_some())) << 2),
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
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&surface) },
                    wgpu::BindGroupEntry { binding: 7, resource: vertices.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&baked) },
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
