//! Compile the viewport's supported UV fills to a small GPU layer program.

use std::sync::{Arc, Weak};
use std::{borrow::Cow, collections::BTreeMap};

use bytemuck::{Pod, Zeroable};
use rayon::prelude::*;
use texturecraft_compose::pattern::pattern_pixels;
use texturecraft_compose::{BakedMaps, DisplayKind, ImageStore, PixelStore};
use texturecraft_core::{
    BakedMap, BlendMode, Channel, ChannelKind, Effect, EffectKind, Filter, Generator, GeneratorKind, LayerId, LayerKind, Mask, MaskBackground, NormalFormat,
    ProceduralKind, Project, Projection, Source,
};
use texturecraft_geom::{Mesh, NO_TRI, UvGBuffer, glam::Vec3};
use texturecraft_texel::{TILE, TileImage};

use crate::gpu_image_source::{DECAL_PROJECTION, IMAGE_SOURCE_OPCODE, MaterialImages};
use crate::gpu_neighbourhood::{FIELD_WORDS, NeighbourhoodFilter, NeighbourhoodGpu, NeighbourhoodRegion};
use crate::gpu_stencil::GpuStencil;
use crate::gpu_tile_pattern::{TILE_GENERATOR_OPCODE, TileImageAtlas, TilePatterns};
use crate::paint_window::TileWindow;
use wgpu::util::DeviceExt;

const MAX_PAINT_WORDS: usize = (64 << 20) / size_of::<u32>();
const LOOKUP_WORDS: u32 = 4096;
const NO_PAINT_IMAGE: u32 = u32::MAX;
const BAKED_LIGHTING_OPCODE: u32 = 35;
const NEIGHBOURHOOD_OPCODE: u32 = 36;
const NOOP_EFFECT_OPCODE: u32 = 39;
const MASK_STAGE: u32 = 1 << 31;
// Extended procedural sources follow the filter and generator opcodes.
const FIRST_GENERATOR: u32 = 40;
const FIRST_EXTENDED_PATTERN: u32 = 48;
pub(crate) const ROWS: u32 = 64;
const WORKGROUP: u32 = 8;
// Keep extreme Tile Generator overlap from monopolizing a single dispatch or submission.
const MAX_TILE_WORK_PER_BAND: u64 = 64 << 20;
const MAX_TILE_WORK_PER_SUBMISSION: u64 = 256 << 20;
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
    // UV rotation and scale, or the pigment source in W for live paint (1: brush, 2: stencil).
    projection: [f32; 4],
    // UV offset XY, procedural scale and contrast, or live-brush values and opacity.
    uv: [f32; 4],
    balance: [f32; 4],
    // Offsets and counts of the mask and content effects.
    effects: [u32; 4],
    // Sampling resolution, color-ramp offset and count, projection selector.
    procedural: [u32; 4],
    // First content field, content has fields, mask has fields.
    spatial: [u32; 4],
}

struct ProceduralInputs {
    resolution: u32,
    ramp_words: Vec<u32>,
    filters: Vec<(Filter, f32)>,
    materials: MaterialImages,
}

impl ProceduralInputs {
    fn new(resolution: u32) -> Self {
        Self { resolution, ramp_words: Vec::new(), filters: Vec::new(), materials: MaterialImages::default() }
    }

    fn pack_gradient(&mut self, colors: &[[f32; 3]]) -> Option<[u32; 2]> {
        if colors.iter().flatten().any(|value| !value.is_finite()) {
            return None;
        }
        let offset = LOOKUP_WORDS.checked_add(u32::try_from(self.ramp_words.len()).ok()?)?;
        let count = u32::try_from(colors.len()).ok()?;
        self.ramp_words.extend(colors.iter().flatten().map(|value| value.to_bits()));
        Some([offset, count])
    }
}

#[derive(Debug)]
struct FilterStage {
    layer: u32,
    preceding: u32,
    filter: NeighbourhoodFilter,
}

#[derive(Debug, Default)]
struct FilterPlan {
    stages: Vec<FilterStage>,
    reach: u32,
}

impl FilterPlan {
    fn compile(operations: &mut [Operation], layer_count: u32, inputs: &ProceduralInputs) -> Self {
        if inputs.filters.is_empty() {
            return Self::default();
        }
        let mut plan = Self::default();
        let (layers, effects) = operations.split_at_mut(layer_count as usize);
        let mut masks = BTreeMap::new();
        for (index, layer) in layers.iter_mut().enumerate() {
            let [mask, mask_count, content, content_count] = layer.effects;
            let mask_has_fields = mask_count > 0
                && *masks.entry(mask).or_insert_with(|| {
                    let before = plan.stages.len();
                    plan.add_stack(effects, mask, mask_count, index as u32, true, false, inputs);
                    plan.stages.len() != before
                });
            let first = plan.stages.len();
            plan.add_stack(effects, content, content_count, index as u32, false, layer.ids[0] == channel_slot(Channel::Height), inputs);
            layer.spatial = [first as u32, u32::from(first != plan.stages.len()), u32::from(mask_has_fields), 0];
        }
        plan
    }

    #[allow(clippy::too_many_arguments)]
    fn add_stack(&mut self, effects: &mut [Operation], start: u32, count: u32, layer: u32, mask: bool, signed: bool, inputs: &ProceduralInputs) {
        let first = self.stages.len();
        let mut reach: u32 = 0;
        for (preceding, effect) in effects.iter_mut().skip(start as usize).take(count as usize).enumerate() {
            if effect.ids[1] != NEIGHBOURHOOD_OPCODE {
                continue;
            }
            let (filter, opacity) = &inputs.filters[effect.ids[2] as usize];
            let Some(filter) = NeighbourhoodFilter::new(filter, inputs.resolution, inputs.resolution, *opacity, signed) else { continue };
            let slot = self.stages.len();
            effect.spatial[0] = if mask { slot } else { slot - first } as u32;
            let preceding = preceding as u32 | if mask { MASK_STAGE } else { 0 };
            self.stages.push(FilterStage { layer, preceding, filter });
            reach = reach.saturating_add(filter.reach());
        }
        self.reach = self.reach.max(reach);
    }

    fn expand(&self, region: Option<CompositeRegion>) -> Option<CompositeRegion> {
        region.map(|region| region.expanded(self.reach))
    }

    fn words(&self, area: u64) -> u64 {
        if self.stages.is_empty() { 0 } else { (self.stages.len() as u64 + 3) * u64::from(FIELD_WORDS) * area }
    }
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

    fn union(self, other: Self) -> Self {
        Self { min: [self.min[0].min(other.min[0]), self.min[1].min(other.min[1])], max: [self.max[0].max(other.max[0]), self.max[1].max(other.max[1])] }
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

/// The active stroke's GPU alpha, sampled in the corresponding paint layer or mask effect.
pub struct LiveBrush {
    pub target: LiveBrushTarget,
    pub opacity: f32,
    pub alpha: wgpu::TextureView,
    /// Supplies opacity modulation and Base Color for a projection stroke.
    pub stencil: Option<GpuStencil>,
}

pub enum LiveBrushTarget {
    Layer { layer: LayerId, values: BTreeMap<Channel, [f32; 3]> },
    Mask { effect: LayerId, value: f32 },
}

impl LiveBrush {
    fn validate(&self, size: (u32, u32)) -> Result<(), &'static str> {
        let alpha = self.alpha.texture();
        if (alpha.width(), alpha.height()) != size || alpha.format() != wgpu::TextureFormat::R16Uint {
            return Err("The live brush alpha does not match the texture set.");
        }
        let valid = match &self.target {
            LiveBrushTarget::Layer { values, .. } => values.values().flatten().all(|v| v.is_finite()),
            LiveBrushTarget::Mask { value, .. } => value.is_finite(),
        };
        if !valid || !self.opacity.is_finite() {
            return Err("The live brush has invalid paint values.");
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct CompositeProgram {
    operations: Vec<Operation>,
    layer_count: u32,
    filters: FilterPlan,
    paint: Vec<TileImage>,
    ramp_words: Vec<u32>,
    tile_patterns: TilePatterns,
    material_images: MaterialImages,
    brush_alpha: Option<wgpu::TextureView>,
    brush_stencil: Option<GpuStencil>,
    pub(crate) region: Option<CompositeRegion>,
    coverage: Option<(Arc<UvGBuffer>, u32)>,
    surface: Option<(Arc<Mesh>, Arc<UvGBuffer>)>,
    needs_surface: bool,
    baked: Option<BakedInputs>,
    display_outputs: u32,
    glossiness: bool,
    mask_preview: bool,
}

fn display_bit(kind: DisplayKind) -> u32 {
    1 << kind as u32
}

// Destination slots match the channel groups consumed by composite.wgsl.
fn channel_slot(channel: Channel) -> u32 {
    use Channel::*;
    match channel {
        BaseColor => 0,
        Opacity => 1,
        AmbientOcclusion => 2,
        Roughness => 3,
        Metallic => 4,
        SpecularLevel => 5,
        Height => 6,
        Normal => 7,
        SheenColor => 8,
        Specular => 9,
        Emissive => 10,
        CoatNormal => 11,
        SheenWeight => 12,
        SheenRoughness => 13,
        ScatterWeight => 14,
        CoatWeight => 15,
        CoatRoughness => 16,
        AnisotropyLevel => 17,
        AnisotropyAngle => 18,
        Transmission => 19,
        Glossiness => 20,
    }
}

impl CompositeProgram {
    pub fn is_neutral(&self) -> bool {
        !self.mask_preview
            && !self.glossiness
            && self.layer_count == 0
            && self.baked.as_ref().is_none_or(|baked| baked.maps[0].is_none() && baked.maps[1].is_none())
    }

    pub fn is_mask_preview(&self) -> bool {
        self.mask_preview
    }

    fn tile_work_per_pixel(&self) -> Result<u64, &'static str> {
        self.operations.iter().filter(|op| op.ids[1] == TILE_GENERATOR_OPCODE).try_fold(0u64, |work, op| {
            let tile_work = self.tile_patterns.work_per_sample(op.high[3] as usize).ok_or("A Tile Generator operation has no settings.")?;
            let planes = if op.procedural[3] == 1 { 3 } else { 1 };
            let samples = if op.procedural[0] == 0 { 1 } else { 4 };
            Ok(work.saturating_add(tile_work.saturating_mul(planes * samples)))
        })
    }

    pub(crate) fn writes_display(&self, kind: DisplayKind) -> bool {
        self.display_outputs & display_bit(kind) != 0
    }

    /// Let the shader sample the nearest covered UV texel for island padding.
    pub fn set_coverage(&mut self, coverage: Arc<UvGBuffer>, radius: u32) {
        self.coverage = Some((coverage, if self.mask_preview { 0 } else { radius.min(15) }));
    }

    /// Resolve named images used by authored fills, decals and Tile Generators.
    pub fn set_tile_images(&mut self, images: &ImageStore) {
        self.tile_patterns.set_images(images);
        self.material_images.set_images(images);
    }

    /// Mesh-space projections and generators share the viewport's surface and bake inputs.
    pub fn set_surface_inputs(&mut self, mesh: &Arc<Mesh>, gbuf: &Arc<UvGBuffer>, baked: Option<&BakedMaps>) -> Result<(), &'static str> {
        if self.needs_surface {
            self.surface = Some((mesh.clone(), gbuf.clone()));
        }
        if self.mask_preview || (!self.filters.stages.is_empty() && self.coverage.is_none()) {
            self.set_coverage(gbuf.clone(), 0);
        }
        self.baked = baked.map(BakedInputs::new).transpose()?.flatten();
        if !self.mask_preview && self.baked.as_ref().is_some_and(|baked| baked.maps[1].is_some()) {
            self.display_outputs |= display_bit(DisplayKind::Normal);
        }
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
        Self::try_compile_with_brush(project, pixels, ts, region, None)
    }

    pub fn try_compile_with_brush(
        project: &Project,
        pixels: &PixelStore,
        ts: usize,
        region: Option<CompositeRegion>,
        brush: Option<LiveBrush>,
    ) -> Result<Self, &'static str> {
        let set = project.texture_sets.get(ts).ok_or("The texture set is missing.")?;
        if !set.uv_tiles.is_empty() {
            return Err("UDIM texture sets use CPU compositing.");
        }
        if let Some(brush) = &brush {
            brush.validate(set.size())?;
        }
        let mut operations = Vec::new();
        let mut paint = Vec::new();
        let mut tile_patterns = TilePatterns::default();
        let mut effects = Vec::new();
        let mut procedural = ProceduralInputs::new(set.resolution);
        let mut non_default = [false; Channel::ALL.len()];
        let stencil_pigment = brush.as_ref().is_some_and(|brush| brush.stencil.is_some());
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
            let mask_background = compile_mask(mask, pixels, set.size(), brush.as_ref(), (&mut paint, &mut procedural, &mut tile_patterns), &mut effects)?;
            let content_start = effects.len() as u32;
            let mask_count = content_start - mask_start;
            if mask_background == 0.0 && mask_count == 0 {
                continue;
            }
            for effect in layer.content_effects.iter().filter(|e| e.visible) {
                effects.push(point_effect(effect, &mut procedural).ok_or("A layer effect uses CPU compositing.")?);
            }
            let content_count = effects.len() as u32 - content_start;
            let fill = match &layer.kind {
                LayerKind::Fill(fill) => Some(fill),
                LayerKind::Paint => None,
                _ => return Err("A group or another layer type uses CPU compositing."),
            };
            let decal = fill.and_then(|fill| match fill.projection {
                Projection::Decal(frame) => Some(procedural.materials.register_decal(frame, fill.sources.values())),
                _ => None,
            });

            let live_brush = brush.as_ref().and_then(|brush| match &brush.target {
                LiveBrushTarget::Layer { layer, values } if fill.is_none() && *layer == id => Some((values, brush.opacity.clamp(0.0, 1.0))),
                _ => None,
            });
            for &(channel, _) in &set.channels {
                let Some(blend) = layer.blend(channel) else { continue };
                if blend.mode == BlendMode::Disable || blend.opacity <= 0.0 {
                    continue;
                }
                if !blend.opacity.is_finite() {
                    return Err("A layer has an invalid opacity.");
                }
                let brush_value = live_brush.and_then(|(values, opacity)| values.get(&channel).map(|value| (*value, opacity)));
                let painted = if fill.is_none() {
                    let image = pixels.layer(id, channel).filter(|img| img.allocated_count() > 0);
                    if image.is_none() && brush_value.is_none() {
                        continue;
                    }
                    if image.is_some_and(|image| (image.width(), image.height()) != set.size() || image.comps() != channel.components() + 1) {
                        return Err("A paint layer has a layout that uses CPU compositing.");
                    }
                    image
                } else {
                    None
                };
                let default = channel.default_value();
                let source = fill.and_then(|fill| fill.sources.get(&channel));
                let slot = channel_slot(channel);
                // Adding a channel to the set also adds it to the earlier layers. Skip
                // their neutral values until the accumulator can contain something else.
                // After a real contribution, neutral fills can still reset the channel.
                if !non_default[slot as usize]
                    && painted.is_none()
                    && brush_value.is_none()
                    && content_count == 0
                    && matches!(blend.mode, BlendMode::Normal | BlendMode::Passthrough | BlendMode::Replace)
                    && source.is_none_or(|source| matches!(source, Source::Uniform(value) if *value == default))
                {
                    continue;
                }
                non_default[slot as usize] = true;
                let mut op = if fill.is_none() {
                    let image_index = painted.map_or(NO_PAINT_IMAGE, |image| {
                        let image_index = paint.len() as u32;
                        paint.push(image.clone());
                        image_index
                    });
                    Operation { ids: [slot, 10, image_index, (channel.components() + 1) as u32], ..Operation::zeroed() }
                } else {
                    fill.and_then(|fill| source_operation(source, &fill.projection, default, channel.kind(), &mut procedural))
                        .ok_or("A fill source or projection uses CPU compositing.")?
                };
                if let Some(decal) = decal {
                    op.uv[0] = decal as f32;
                }
                if let Some(([r, g, b], opacity)) = brush_value {
                    op.projection[3] = if stencil_pigment && channel == Channel::BaseColor { 2.0 } else { 1.0 };
                    op.uv = [r, g, b, opacity];
                }
                bind_tile_source(&mut op, source, &mut tile_patterns);
                op.ids[0] = slot;
                op.low[3] = blend.opacity.min(1.0);
                op.balance[1] = mask_background;
                op.balance[2] = blend_code(blend.mode) as f32;
                op.effects = [mask_start, mask_count, content_start, content_count];
                operations.push(op);
            }
        }
        let mut display_outputs = display_bit(DisplayKind::BaseColor) | display_bit(DisplayKind::Orm);
        for channel in Channel::ALL {
            let slot = channel_slot(channel);
            if operations
                .iter()
                .any(|op| op.ids[0] == slot && (op.ids[1] != 0 || op.low[..3] != channel.default_value() || op.effects[3] != 0 || op.balance[2] != 0.0))
            {
                display_outputs |= display_bit(DisplayKind::of_channel(channel));
            }
        }
        let layer_count = operations.len() as u32;
        operations.extend(effects);
        let filters = FilterPlan::compile(&mut operations, layer_count, &procedural);
        let region = filters.expand(region);
        let (brush_alpha, brush_stencil) = brush.map_or((None, None), |brush| (Some(brush.alpha), brush.stencil));
        let needs_surface = brush_stencil.is_some() || needs_surface(&operations);
        Ok(Self {
            operations,
            layer_count,
            filters,
            paint,
            ramp_words: procedural.ramp_words,
            tile_patterns,
            material_images: procedural.materials,
            brush_alpha,
            brush_stencil,
            region,
            coverage: None,
            surface: None,
            needs_surface,
            baked: None,
            display_outputs,
            glossiness: set.has_channel(Channel::Glossiness),
            mask_preview: false,
        })
    }

    /// Compose one layer's enabled mask into the standalone grayscale view.
    pub fn try_compile_mask_preview(
        project: &Project,
        pixels: &PixelStore,
        ts: usize,
        owner: LayerId,
        region: Option<CompositeRegion>,
        brush: Option<LiveBrush>,
    ) -> Result<Self, &'static str> {
        let set = project.texture_sets.get(ts).ok_or("The texture set is missing.")?;
        if !set.uv_tiles.is_empty() {
            return Err("UDIM texture sets use CPU mask preview.");
        }
        let mask = project.layers.get(&project.mask_owner(owner)).and_then(|layer| layer.mask.as_ref()).filter(|mask| mask.enabled);
        let mask = mask.ok_or("The layer has no enabled mask.")?;
        if let Some(brush) = &brush {
            brush.validate(set.size())?;
        }
        let mut operations = vec![Operation::zeroed()];
        let mut paint = Vec::new();
        let mut procedural = ProceduralInputs::new(set.resolution);
        let mut tile_patterns = TilePatterns::default();
        let background = compile_mask(Some(mask), pixels, set.size(), brush.as_ref(), (&mut paint, &mut procedural, &mut tile_patterns), &mut operations)?;
        operations[0].effects[1] = operations.len() as u32 - 1;
        operations[0].balance[1] = background;
        let filters = FilterPlan::compile(&mut operations, 1, &procedural);
        let region = filters.expand(region);
        let (brush_alpha, brush_stencil) = brush.map_or((None, None), |brush| (Some(brush.alpha), brush.stencil));
        let needs_surface = brush_stencil.is_some() || needs_surface(&operations);
        Ok(Self {
            operations,
            layer_count: 1,
            filters,
            paint,
            ramp_words: procedural.ramp_words,
            tile_patterns,
            material_images: procedural.materials,
            brush_alpha,
            brush_stencil,
            region,
            coverage: None,
            surface: None,
            needs_surface,
            baked: None,
            // Reuse the first four-byte display slot for linear grayscale.
            display_outputs: display_bit(DisplayKind::BaseColor),
            glossiness: false,
            mask_preview: true,
        })
    }
}

fn needs_surface(operations: &[Operation]) -> bool {
    operations.iter().any(|op| op.procedural[3] != 0 || op.ids[1] == BAKED_LIGHTING_OPCODE || (FIRST_GENERATOR..FIRST_EXTENDED_PATTERN).contains(&op.ids[1]))
}

fn compile_mask(
    mask: Option<&Mask>,
    pixels: &PixelStore,
    size: (u32, u32),
    brush: Option<&LiveBrush>,
    (paint, procedural, tile_patterns): (&mut Vec<TileImage>, &mut ProceduralInputs, &mut TilePatterns),
    effects: &mut Vec<Operation>,
) -> Result<f32, &'static str> {
    let Some(mask) = mask else { return Ok(1.0) };
    for effect in mask.effects.iter().filter(|e| e.visible) {
        let mut op = match &effect.kind {
            EffectKind::Fill { source, projection } => {
                let mut operation = source_operation(Some(source), projection, [0.0; 3], ChannelKind::Scalar, procedural)
                    .ok_or("A mask fill source or projection uses CPU compositing.")?;
                if let Projection::Decal(frame) = projection {
                    operation.uv[0] = procedural.materials.register_decal(*frame, std::iter::once(source)) as f32;
                }
                operation
            }
            EffectKind::Paint | EffectKind::Path(_) => {
                let live = brush.and_then(|brush| match &brush.target {
                    LiveBrushTarget::Mask { effect: id, value } if *id == effect.id && matches!(effect.kind, EffectKind::Paint) => {
                        Some((value.clamp(0.0, 1.0), brush.opacity.clamp(0.0, 1.0)))
                    }
                    _ => None,
                });
                let image = pixels.masks.get(&effect.id).filter(|im| im.allocated_count() > 0);
                if image.is_none() && live.is_none() {
                    continue;
                }
                if image.is_some_and(|image| (image.width(), image.height()) != size || image.comps() != 2) {
                    return Err("A painted mask has a layout that uses CPU compositing.");
                }
                let image_index = image.map_or(NO_PAINT_IMAGE, |image| {
                    let index = paint.len() as u32;
                    paint.push(image.clone());
                    index
                });
                let mut op = Operation { ids: [0, 10, image_index, 2], ..Operation::zeroed() };
                if let Some((value, opacity)) = live {
                    op.projection[3] = 1.0;
                    op.uv = [value, value, value, opacity];
                }
                op
            }
            EffectKind::Generator(generator) => generator_operation(generator).ok_or("A mask generator uses CPU compositing.")?,
            EffectKind::Filter(Filter::BakedLighting { .. }) => continue,
            _ => point_effect(effect, procedural).ok_or("A mask effect uses CPU compositing.")?,
        };
        if !effect.opacity.is_finite() {
            return Err("A mask effect has an invalid opacity.");
        }
        if let EffectKind::Fill { source, .. } = &effect.kind {
            bind_tile_source(&mut op, Some(source), tile_patterns);
        }
        op.low[3] = effect.opacity.clamp(0.0, 1.0);
        op.balance[2] = blend_code(effect.blend) as f32;
        effects.push(op);
    }
    Ok(if mask.background == MaskBackground::White { 1.0 } else { 0.0 })
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

fn bind_tile_source(op: &mut Operation, source: Option<&Source>, patterns: &mut TilePatterns) {
    if op.ids[1] == TILE_GENERATOR_OPCODE
        && let Some(Source::Procedural { pattern, .. }) = source
    {
        let settings = pattern.tile.as_deref().cloned().unwrap_or_default();
        op.high[3] = patterns.register(settings, pattern.seed) as f32;
    }
}

fn source_operation(
    source: Option<&Source>,
    projection: &Projection,
    default: [f32; 3],
    channel: ChannelKind,
    inputs: &mut ProceduralInputs,
) -> Option<Operation> {
    let mut op = Operation::zeroed();
    match source {
        None => op.low[..3].copy_from_slice(&default),
        Some(Source::Uniform(value)) => op.low[..3].copy_from_slice(value),
        Some(Source::Image { name, normal_format }) => {
            let kind = match channel {
                ChannelKind::Color => 0,
                ChannelKind::Scalar => 1,
                ChannelKind::Signed => 2,
                ChannelKind::NormalMap => {
                    if *normal_format == NormalFormat::DirectX {
                        4
                    } else {
                        3
                    }
                }
            };
            op.ids = [0, IMAGE_SOURCE_OPCODE, inputs.materials.register_image(name), kind];
            match projection {
                Projection::Uv { scale, rotation, offset } => {
                    let (sin, cos) = rotation.to_radians().sin_cos();
                    op.projection = [cos, sin, scale[0], scale[1]];
                    op.uv[..2].copy_from_slice(offset);
                }
                Projection::Triplanar { scale, sharpness, offset } => {
                    op.projection = [offset[0], offset[1], offset[2], *scale];
                    op.procedural[3] = 1;
                    op.balance[3] = sharpness.max(1.0);
                }
                Projection::Planar { scale, axis, offset } => {
                    op.projection = [offset[0], offset[1], offset[2], *scale];
                    op.procedural[3] = 2 + u32::from((*axis).min(2));
                }
                Projection::Spherical { scale } => {
                    op.projection[3] = *scale;
                    op.procedural[3] = 5;
                }
                Projection::Cylindrical { scale, axis } => {
                    op.projection[3] = *scale;
                    op.procedural[3] = match axis {
                        0 => 6,
                        2 => 8,
                        _ => 7,
                    };
                }
                Projection::Decal(_) => {}
            }
        }
        Some(Source::Procedural { pattern, low, high, ramp }) => {
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
                ProceduralKind::Checker => 48,
                ProceduralKind::Stripes => 49,
                ProceduralKind::Bricks => 50,
                ProceduralKind::Tiles => 51,
                ProceduralKind::Weave => 52,
                ProceduralKind::Twill => 53,
                ProceduralKind::Leather => 54,
                ProceduralKind::Hammered => 55,
                ProceduralKind::Brushed => 56,
                ProceduralKind::Voronoi => 57,
                ProceduralKind::Smudges => 58,
                ProceduralKind::HexagonTiles => 60,
                ProceduralKind::WaveScales => 61,
                ProceduralKind::HerringbonePlanks => 62,
                ProceduralKind::HalftoneDots => 63,
                ProceduralKind::DiamondBrocade => 64,
                ProceduralKind::TruchetArcs => 65,
                ProceduralKind::SmoothFur => 66,
                ProceduralKind::MackerelSky => 67,
                ProceduralKind::Billow => 68,
                ProceduralKind::ReptileScales => 69,
                ProceduralKind::Bark => 70,
                ProceduralKind::Flesh => 71,
                ProceduralKind::Camouflage => 72,
                ProceduralKind::Knit => 73,
                ProceduralKind::BasketWeave => 74,
                ProceduralKind::GooseEye => 75,
                ProceduralKind::SlubSilk => 76,
                ProceduralKind::StripFlooring => 77,
                ProceduralKind::KnottyWood => 78,
                ProceduralKind::TileGenerator => TILE_GENERATOR_OPCODE,
            };
            op.ids = [0, kind, pattern.seed, u32::from(pattern.invert)];
            op.low[..3].copy_from_slice(low);
            op.high[..3].copy_from_slice(high);
            op.uv[2] = pattern.scale.max(1e-4);
            op.uv[3] = pattern.contrast.max(0.0);
            op.balance[0] = pattern.balance;
            op.procedural[0] = pattern_pixels(pattern, inputs.resolution, inputs.resolution).unwrap_or(0);
            if channel == ChannelKind::Color && !ramp.is_empty() {
                op.procedural[1..3].copy_from_slice(&inputs.pack_gradient(ramp)?);
            }
            match projection {
                Projection::Uv { scale, rotation, offset } => {
                    let (sin, cos) = rotation.to_radians().sin_cos();
                    op.projection = [cos, sin, scale[0], scale[1]];
                    op.uv[..2].copy_from_slice(offset);
                    if scale.iter().zip(offset).any(|(s, o)| (s.abs() * 2.0 + o.abs()) * op.uv[2] > 32_768.0) {
                        return None;
                    }
                }
                Projection::Triplanar { scale, offset, .. } | Projection::Planar { scale, offset, .. } => {
                    let frequency = *scale * op.uv[2];
                    if offset.iter().any(|o| frequency.abs() + o.abs() > 32_768.0) {
                        return None;
                    }
                    op.projection = [offset[0], offset[1], offset[2], frequency];
                    if let Projection::Planar { axis, .. } = projection {
                        op.procedural[3] = 2 + u32::from((*axis).min(2));
                    } else if let Projection::Triplanar { sharpness, .. } = projection {
                        op.procedural[3] = 1;
                        op.balance[3] = sharpness.max(1.0);
                    }
                }
                Projection::Spherical { scale } | Projection::Cylindrical { scale, .. } => {
                    let frequency = *scale * op.uv[2];
                    if frequency.abs() * 4.0 > 32_768.0 {
                        return None;
                    }
                    op.projection[3] = frequency;
                    op.procedural[3] = match projection {
                        Projection::Cylindrical { axis: 0, .. } => 6,
                        Projection::Cylindrical { axis: 2, .. } => 8,
                        Projection::Cylindrical { .. } => 7,
                        _ => 5,
                    };
                }
                Projection::Decal(_) => {}
            }
        }
        _ => return None,
    }
    if matches!(projection, Projection::Decal(_)) {
        op.procedural[3] = DECAL_PROJECTION;
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

fn point_effect(effect: &Effect, inputs: &mut ProceduralInputs) -> Option<Operation> {
    use Filter::*;
    if !effect.opacity.is_finite() {
        return None;
    }
    let mut op = Operation::zeroed();
    let (kind, params) = match &effect.kind {
        EffectKind::Levels(levels) => {
            op.high[..2].copy_from_slice(&[levels.out_low, levels.out_high]);
            (20, [levels.in_low, levels.in_high, levels.gamma])
        }
        EffectKind::Filter(filter) => match filter {
            Invert => (21, [0.0; 3]),
            Threshold { level } => (22, [*level, 0.0, 0.0]),
            ContrastLuminosity { contrast, luminosity } => (23, [*contrast, *luminosity, 0.0]),
            Posterize { levels } => (24, [*levels, 0.0, 0.0]),
            Clamp { min, max } => (25, [min.min(*max), min.max(*max), 0.0]),
            HistogramScan { position, contrast } => (26, [*position, *contrast, 0.0]),
            HistogramRange { range, position } => (27, [*range, *position, 0.0]),
            HistogramShift { offset } => (28, [*offset, 0.0, 0.0]),
            Grayscale => (29, [0.0; 3]),
            NormalInvert => (30, [0.0; 3]),
            Hsl { hue, saturation, lightness } => (31, [*hue, *saturation, *lightness]),
            ColorBalance { shadows, midtones, highlights } => {
                op.high[..3].copy_from_slice(midtones);
                op.projection[..3].copy_from_slice(highlights);
                (32, *shadows)
            }
            GradientMap { colors } => {
                op.procedural[1..3].copy_from_slice(&inputs.pack_gradient(colors)?);
                (33, [0.0; 3])
            }
            ReplaceColor { from, to, tolerance, softness } => {
                op.high[..3].copy_from_slice(to);
                op.projection[..2].copy_from_slice(&[tolerance.clamp(0.0, 1.0), softness.clamp(0.0, 1.0)]);
                (34, *from)
            }
            BakedLighting { azimuth, elevation, light, ambient, occlusion } => {
                let (sa, ca) = azimuth.to_radians().sin_cos();
                let (se, ce) = elevation.to_radians().sin_cos();
                op.high[..3].copy_from_slice(&[ce * sa, se, ce * ca]);
                (BAKED_LIGHTING_OPCODE, [*light, *ambient, occlusion.clamp(0.0, 1.0)])
            }
            filter if filter.is_neighbourhood() => {
                let spatial = NeighbourhoodFilter::new(filter, inputs.resolution, inputs.resolution, effect.opacity, false);
                if spatial.is_some() {
                    op.ids[2] = inputs.filters.len() as u32;
                    inputs.filters.push((filter.clone(), effect.opacity));
                }
                (if spatial.is_some() { NEIGHBOURHOOD_OPCODE } else { NOOP_EFFECT_OPCODE }, [0.0; 3])
            }
            _ => return None,
        },
        _ => return None,
    };
    op.ids[1] = kind;
    op.low = [params[0], params[1], params[2], effect.opacity.clamp(0.0, 1.0)];
    op.low.iter().chain(&op.high).chain(&op.projection).all(|x| x.is_finite()).then_some(op)
}

// Three 10-bit components followed by a validity bit.
fn encode_baked_normal(normal: [f32; 3]) -> u32 {
    if !normal.iter().all(|value| (0.0..=1.0).contains(value)) {
        return 0;
    }
    // An even scale preserves the flat normal's exact 0.5 components.
    let [x, y, z] = [normal[0], normal[1], normal[2].sqrt()].map(|value| (value * 1022.0 + 0.5) as u32);
    // Square-root encoding preserves precision near zero, where RNM is most sensitive.
    let z = z.max(u32::from(normal[2] > 0.0));
    x | (y << 10) | (z << 20) | (1 << 30)
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

fn write_input<T: Pod>(queue: &wgpu::Queue, texture: &wgpu::Texture, size: (u32, u32), row_layer: (u32, u32), texels: &[T]) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { origin: wgpu::Origin3d { x: 0, y: row_layer.0, z: row_layer.1 }, ..texture.as_image_copy() },
        bytemuck::cast_slice(texels),
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size.0 * size_of::<T>() as u32), rows_per_image: None },
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
    shader: wgpu::ShaderModule,
    bindings: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    filter_gpu: Option<(wgpu::ComputePipeline, NeighbourhoodGpu)>,
    lookup: wgpu::Buffer,
    uniform_coverage: wgpu::Buffer,
    coverage: Vec<(Weak<UvGBuffer>, wgpu::Buffer)>,
    surfaces: Vec<SurfaceCache>,
    baked: BTreeMap<usize, BakeCache>,
    default_surface: wgpu::TextureView,
    default_vertices: wgpu::Buffer,
    default_baked: wgpu::TextureView,
    default_brush_alpha: wgpu::TextureView,
    default_brush_stencil: GpuStencil,
    default_tile_images: wgpu::TextureView,
    tile_images: BTreeMap<usize, TileImageAtlas>,
    paint_upload: Option<wgpu::Buffer>,
}

impl GpuCompositor {
    pub(crate) fn supports(device: &wgpu::Device) -> bool {
        let limits = device.limits();
        limits.max_storage_buffers_per_shader_stage >= 6
            && limits.max_sampled_textures_per_shader_stage >= 5
            && limits.max_uniform_buffers_per_shader_stage >= 2
            && limits.max_bindings_per_bind_group >= 13
            && limits.max_compute_invocations_per_workgroup >= WORKGROUP * WORKGROUP
            && limits.max_compute_workgroup_size_x >= WORKGROUP
            && limits.max_compute_workgroup_size_y >= WORKGROUP
    }

    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let binding = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None };
        let buffer = |ty| wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None };
        let uniform = buffer(wgpu::BufferBindingType::Uniform);
        let read = buffer(wgpu::BufferBindingType::Storage { read_only: true });
        let write = buffer(wgpu::BufferBindingType::Storage { read_only: false });
        let texture = |sample_type, view_dimension| wgpu::BindingType::Texture { sample_type, view_dimension, multisampled: false };
        let uint = texture(wgpu::TextureSampleType::Uint, wgpu::TextureViewDimension::D2);
        let array = texture(wgpu::TextureSampleType::Uint, wgpu::TextureViewDimension::D2Array);
        let float = texture(wgpu::TextureSampleType::Float { filterable: false }, wgpu::TextureViewDimension::D2);
        let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tc_gpu_composite_bindings"),
            entries: &[
                binding(0, uniform),
                binding(1, read),
                binding(2, read),
                binding(3, write),
                binding(4, read),
                binding(5, read),
                binding(6, uint),
                binding(7, read),
                binding(8, uint),
                binding(9, uint),
                binding(10, array),
                binding(11, float),
                binding(12, uniform),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tc_gpu_composite_layout"),
            bind_group_layouts: &[Some(&bindings)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tc_gpu_composite"),
            source: wgpu::ShaderSource::Wgsl(
                concat!(
                    include_str!("shaders/noise.wgsl"),
                    "\n",
                    include_str!("shaders/procedural_extra.wgsl"),
                    "\n",
                    include_str!("shaders/tile_generator.wgsl"),
                    "\n",
                    include_str!("shaders/material_image.wgsl"),
                    "\n",
                    include_str!("shaders/stencil_projection.wgsl"),
                    "\n",
                    include_str!("shaders/composite.wgsl")
                )
                .into(),
            ),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("tc_gpu_composite"),
            layout: Some(&pipeline_layout),
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
        let default_baked = input_texture(device, "tc_composite_empty_bakes", (1, 1), 1, wgpu::TextureFormat::Rgba32Uint).create_view(&Default::default());
        let default_brush_alpha =
            input_texture(device, "tc_composite_empty_brush_alpha", (1, 1), 1, wgpu::TextureFormat::R16Uint).create_view(&Default::default());
        Self {
            pipeline,
            shader: module,
            bindings,
            pipeline_layout,
            filter_gpu: None,
            lookup,
            uniform_coverage,
            coverage: Vec::new(),
            surfaces: Vec::new(),
            baked: BTreeMap::new(),
            default_surface,
            default_vertices,
            default_baked,
            default_brush_alpha,
            default_brush_stencil: GpuStencil::empty(device),
            default_tile_images: TileImageAtlas::empty_view(device),
            tile_images: BTreeMap::new(),
            paint_upload: None,
        }
    }

    fn surface_inputs(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, program: &CompositeProgram) -> Result<(wgpu::TextureView, wgpu::Buffer), String> {
        let Some((mesh, gbuf)) = &program.surface else {
            if program.needs_surface {
                return Err("Missing UV surface inputs for GPU layer composition".into());
            }
            return Ok((self.default_surface.clone(), self.default_vertices.clone()));
        };
        self.uv_surface(device, queue, mesh, gbuf)
    }

    pub(crate) fn uv_surface(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mesh: &Arc<Mesh>,
        gbuf: &Arc<UvGBuffer>,
    ) -> Result<(wgpu::TextureView, wgpu::Buffer), String> {
        self.release_unused_surfaces();
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
                let uv = mesh.uvs.get(index).copied().unwrap_or_default();
                [pos.extend(uv.x).to_array(), normal.extend(uv.y).to_array()]
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
            write_input(queue, &texture, (gbuf.width, tri.len() as u32 / gbuf.width), (band as u32 * ROWS, 0), &pixels);
        }
        let view = texture.create_view(&Default::default());
        self.surfaces.push(SurfaceCache { gbuf: key, view: view.clone(), vertices: vertices.clone() });
        Ok((view, vertices))
    }

    pub(crate) fn retain_texture_sets(&mut self, count: usize) {
        self.release_unused_surfaces();
        self.baked.retain(|&ts, _| ts < count);
        self.tile_images.retain(|&ts, _| ts < count);
    }

    pub(crate) fn release_unused_surfaces(&mut self) {
        self.tile_images.retain(|_, atlas| atlas.is_live());
        self.surfaces.retain(|cache| cache.gbuf.strong_count() != 0);
    }

    pub(crate) fn clear_baked_inputs(&mut self) {
        self.baked.clear();
    }

    pub(crate) fn release_inputs(&mut self, ts: usize) {
        self.baked.remove(&ts);
        self.tile_images.remove(&ts);
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
        // Keep all three scalar inputs at f32 precision for narrow generator ramps. The
        // normal uses the remaining word, keeping the atlas at 16 bytes per texel.
        let texture = input_texture(device, "tc_composite_bakes", size, 1, wgpu::TextureFormat::Rgba32Uint);
        let value = |map: usize, i: usize, fallback: f32| baked.maps[map].as_ref().and_then(|data| data.get(i).copied()).unwrap_or(fallback);
        for top in (0..size.1).step_by(ROWS as usize) {
            let end = (top + ROWS).min(size.1);
            let indices = top as usize * size.0 as usize..end as usize * size.0 as usize;
            let texels: Vec<[u32; 4]> = indices
                .into_par_iter()
                .map(|i| {
                    let normal = if baked.maps[1].is_some() {
                        encode_baked_normal([value(1, i * 3, 0.5), value(1, i * 3 + 1, 0.5), value(1, i * 3 + 2, 1.0)])
                    } else {
                        0
                    };
                    [value(0, i, 1.0).to_bits(), value(2, i, 0.5).to_bits(), value(3, i, 1.0).to_bits(), normal]
                })
                .collect();
            write_input(queue, &texture, (size.0, end - top), (top, 0), &texels);
        }
        let view = texture.create_view(&Default::default());
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

    fn tile_atlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, ts: usize, program: &CompositeProgram) -> Result<Option<&TileImageAtlas>, String> {
        let gray: Vec<_> = program.tile_patterns.image_sources().collect();
        let color: Vec<_> = program.material_images.sources().collect();
        if gray.is_empty() && color.is_empty() {
            self.tile_images.remove(&ts);
            return Ok(None);
        }
        if self.tile_images.get(&ts).is_none_or(|atlas| !atlas.matches(&gray, &color)) {
            self.tile_images.remove(&ts);
            let atlas = TileImageAtlas::new(device, queue, &gray, &color)?;
            self.tile_images.insert(ts, atlas);
        }
        Ok(self.tile_images.get(&ts))
    }

    /// Compose bands into reusable GPU scratch space, then copy them into display textures.
    /// Indexed/SRGB and floating-point maps use the same formats as the CPU uploader.
    pub(crate) fn compose(
        &mut self,
        gpu: (&wgpu::Device, &wgpu::Queue),
        program: &CompositeProgram,
        target: (usize, (u32, u32), u32, Option<CompositeRegion>),
        textures: &[&wgpu::Texture],
        submitted: &mut Option<CompositeRegion>,
    ) -> Result<(), String> {
        let (device, queue) = gpu;
        let (ts, source_size, shift, region) = target;
        let base = textures.first().ok_or("The GPU output textures are missing.")?;
        let extent = (base.width(), base.height());
        let region = region.map(|r| r.viewport(shift, extent)).unwrap_or(CompositeRegion { min: [0; 2], max: [extent.0, extent.1] });
        let width = region.max[0].saturating_sub(region.min[0]);
        let height = region.max[1].saturating_sub(region.min[1]);
        if width == 0 || height == 0 {
            return Ok(());
        }
        let tile_work = program.tile_work_per_pixel()?;
        let radius = program.coverage.as_ref().map_or(0, |(_, radius)| *radius);
        let halo = radius.saturating_add(program.filters.reach);
        let limits = device.limits();
        let scratch_limit = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
        let (mut band_width, mut band_rows) = composite_band_size(width, tile_work);
        let filtered = !program.filters.stages.is_empty();
        if filtered {
            if limits.max_compute_workgroup_size_x < 64 {
                return Err("The GPU cannot run the neighbourhood filter workgroups.".into());
            }
            if tile_work == 0 {
                band_rows = height.min(256);
            }
            self.filter_gpu.get_or_insert_with(|| {
                let prepare = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("tc_spatial_source"),
                    layout: Some(&self.pipeline_layout),
                    module: &self.shader,
                    entry_point: Some("prepare_filter"),
                    compilation_options: Default::default(),
                    cache: None,
                });
                (prepare, NeighbourhoodGpu::new(device))
            });
        }
        let coverage = self.coverage_buffer(device, program.coverage.as_ref().map(|(map, _)| map))?;
        let (surface, vertices) = self.surface_inputs(device, queue, program)?;
        let baked = self.baked_input(device, queue, ts, program.baked.as_ref())?;
        let mut words_per_pixel = 0;
        let copies = DisplayKind::ALL.map(|kind| {
            if !program.writes_display(kind) {
                return None;
            }
            let bytes_per_texel = kind.bytes_per_texel() as u32;
            let offset = words_per_pixel;
            words_per_pixel += u64::from(bytes_per_texel / 4);
            Some((offset, bytes_per_texel))
        });
        let field_area = |width: u32, rows: u32| {
            let side = |pixels: u32, limit: u32| (pixels << shift).saturating_add(halo.saturating_mul(2)).min(limit);
            u64::from(side(width, source_size.0)) * u64::from(side(rows, source_size.1))
        };
        let display_words = |width: u32, rows: u32| u64::from(width.next_multiple_of(64) * rows) * words_per_pixel;
        let required = |width, rows| (display_words(width, rows) + program.filters.words(field_area(width, rows))) * 4;
        let filter_work: u64 = program.filters.stages.iter().map(|stage| stage.filter.work_per_texel()).sum();
        // A filtered band includes its source halo. Split its output while preserving all of that input.
        while required(band_width, band_rows) > scratch_limit {
            if band_width == 1 && band_rows == 1 {
                return Err("Filter inputs for one GPU texel exceed the adapter's working memory limit.".into());
            }
            if band_width >= band_rows {
                band_width = band_width.div_ceil(2);
            } else {
                band_rows = band_rows.div_ceil(2);
            }
        }
        let pitch = band_width.next_multiple_of(64);
        let submit_budget = if filtered || band_width < width || band_rows < ROWS { MAX_TILE_WORK_PER_SUBMISSION } else { u64::MAX };
        let area = pitch * band_rows;
        let stride = u64::from(area) * 4;
        let field_base = (display_words(band_width, band_rows)) as u32;
        let scratch = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tc_composite_scratch"),
            size: required(band_width, band_rows),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let atlas = self.tile_atlas(device, queue, ts, program)?;
        let tile_images = atlas.map(|atlas| atlas.view.clone());
        let (operations, lookup) = if program.tile_patterns.is_empty() && program.ramp_words.is_empty() && program.material_images.is_empty() {
            (Cow::Borrowed(program.operations.as_slice()), self.lookup.clone())
        } else {
            let mut words = texturecraft_texel::srgb_lut().map(u32::from).to_vec();
            words.extend(&program.ramp_words);
            let offsets = program.tile_patterns.append_words(&mut words, atlas);
            let mesh = program.surface.as_ref().map(|(mesh, _)| mesh.as_ref());
            let (images, decals) = program.material_images.append_words(&mut words, atlas, mesh);
            let limits = device.limits();
            // The operation stores the offset as an exactly represented float.
            if ((!offsets.is_empty() || !decals.is_empty()) && words.len() >= 1 << 24)
                || (words.len() * 4) as u64 > limits.max_storage_buffer_binding_size.min(limits.max_buffer_size)
            {
                return Err("Procedural settings exceed the GPU parameter buffer limit.".into());
            }
            let mut operations = program.operations.clone();
            for op in operations.iter_mut().filter(|op| op.ids[1] == TILE_GENERATOR_OPCODE) {
                let index = op.high[3] as usize;
                op.high[3] = *offsets.get(index).ok_or("A Tile Generator operation has no settings.")? as f32;
            }
            for op in &mut operations {
                if op.ids[1] == IMAGE_SOURCE_OPCODE {
                    op.ids[2] = *images.get(op.ids[2] as usize).ok_or("An image source has no binding.")?;
                }
                if op.procedural[3] == DECAL_PROJECTION {
                    op.uv[0] = *decals.get(op.uv[0] as usize).ok_or("A decal has no frame.")? as f32;
                }
            }
            let lookup = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tc_composite_tile_patterns"),
                contents: bytemuck::cast_slice(&words),
                usage: wgpu::BufferUsages::STORAGE,
            });
            (Cow::Owned(operations), lookup)
        };
        let tile_images = tile_images.as_ref().unwrap_or(&self.default_tile_images);
        let brush_alpha = program.brush_alpha.as_ref().unwrap_or(&self.default_brush_alpha);
        let brush_stencil = program.brush_stencil.as_ref().unwrap_or(&self.default_brush_stencil);
        let empty_operation = Operation::zeroed();
        let limits = device.limits();
        if (operations.len() * size_of::<Operation>()) as u64 > limits.max_storage_buffer_binding_size.min(limits.max_buffer_size) {
            return Err("The layer program exceeds the adapter's storage-buffer limit".into());
        }
        let ops = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_composite_operations"),
            contents: if operations.is_empty() { bytemuck::bytes_of(&empty_operation) } else { bytemuck::cast_slice(operations.as_ref()) },
            usage: wgpu::BufferUsages::STORAGE,
        });
        let max_words = MAX_PAINT_WORDS.min((limits.max_storage_buffer_binding_size.min(limits.max_buffer_size) / 4) as usize);
        let windows = paint_windows(program, region, source_size, shift, halo, max_words)?;
        let mut packed = Vec::new();
        let mut pending_bytes = 0;
        let mut last_submission = None;
        let make_encoder = || device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_composite") });
        let submit = |encoder: wgpu::CommandEncoder, bounds: CompositeRegion, submitted: &mut Option<CompositeRegion>| {
            let submission = queue.submit([encoder.finish()]);
            *submitted = Some(submitted.map_or(bounds, |region| region.union(bounds)));
            submission
        };
        for (chunk, window, words) in windows {
            let bytes = (words * 4) as u64;
            window.pack(&program.paint, words, &mut packed);
            if pending_bytes + bytes > (max_words * 4) as u64 {
                device.poll(wgpu::PollType::Wait { submission_index: last_submission.take(), timeout: None }).map_err(|e| e.to_string())?;
                pending_bytes = 0;
            }
            let upload_size = bytes.next_power_of_two().min((max_words * 4) as u64);
            if self.paint_upload.as_ref().is_none_or(|buffer| buffer.size() < bytes) {
                self.paint_upload = Some(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("tc_composite_paint"),
                    size: upload_size,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            }
            let paint = self.paint_upload.as_ref().ok_or("paint upload buffer is missing")?;
            queue.write_buffer(paint, 0, bytemuck::cast_slice(&packed));
            let mut encoder = make_encoder();
            let mut work = 0u64;
            let mut bounds: Option<CompositeRegion> = None;
            let bands = (chunk.min[1]..chunk.max[1])
                .step_by(band_rows as usize)
                .flat_map(|top| (chunk.min[0]..chunk.max[0]).step_by(band_width as usize).map(move |left| (left, top)));
            for (left, top) in bands {
                let width = band_width.min(chunk.max[0] - left);
                let rows = band_rows.min(chunk.max[1] - top);
                let source_work = u64::from(width).saturating_mul(u64::from(rows)).saturating_mul(tile_work);
                let band_work = source_work.saturating_add(field_area(width, rows).saturating_mul(filter_work));
                if let Some(written) = bounds.take_if(|_| work.saturating_add(band_work) > submit_budget) {
                    let previous = std::mem::replace(&mut encoder, make_encoder());
                    submit(previous, written, submitted);
                    work = 0;
                }
                let field = CompositeRegion { min: [left << shift, top << shift], max: [(left + width) << shift, (top + rows) << shift] }.expanded(halo);
                let field_rect = [field.min[0], field.min[1], field.max[0].min(source_size.0) - field.min[0], field.max[1].min(source_size.1) - field.min[1]];
                let values = [
                    width,
                    rows,
                    top,
                    area,
                    source_size.0,
                    source_size.1,
                    1 << shift,
                    if program.mask_preview { 2 } else { u32::from(program.glossiness) },
                    program.layer_count,
                    pitch,
                    program.display_outputs,
                    left,
                    u32::from(program.coverage.is_some()),
                    source_size.0.div_ceil(32),
                    radius,
                    u32::from(program.surface.is_some()) | (u32::from(program.baked.is_some()) << 1) | (u32::from(program.brush_stencil.is_some()) << 2),
                    window.origin[0],
                    window.origin[1],
                    window.size[0],
                    window.size[1],
                    field_rect[0],
                    field_rect[1],
                    field_rect[2],
                    field_rect[3],
                    field_base,
                    0,
                    0,
                    0,
                ];
                let bindings = |values: &[u32]| {
                    let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("tc_composite_band"),
                        contents: bytemuck::cast_slice(values),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("tc_composite_band"),
                        layout: &self.bindings,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: ops.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 2, resource: lookup.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 3, resource: scratch.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 4, resource: paint.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 5, resource: coverage.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&surface) },
                            wgpu::BindGroupEntry { binding: 7, resource: vertices.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&baked) },
                            wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(brush_alpha) },
                            wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(tile_images) },
                            wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(&brush_stencil.image) },
                            wgpu::BindGroupEntry { binding: 12, resource: brush_stencil.uniforms.as_entire_binding() },
                        ],
                    })
                };
                if filtered && let Some((prepare, kernels)) = &self.filter_gpu {
                    let fields = program.filters.stages.len() as u32;
                    let field_words = field_rect[2] * field_rect[3] * FIELD_WORDS;
                    let offset = |slot| field_base + slot * field_words;
                    for (output, stage) in program.filters.stages.iter().enumerate() {
                        let mut source = values;
                        source[25..].copy_from_slice(&[fields, stage.layer, stage.preceding]);
                        let source_bindings = bindings(&source);
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("tc_spatial_source"), timestamp_writes: None });
                        pass.set_pipeline(prepare);
                        pass.set_bind_group(0, &source_bindings, &[]);
                        pass.dispatch_workgroups(field_rect[2].div_ceil(WORKGROUP), field_rect[3].div_ceil(WORKGROUP), 1);
                        drop(pass);
                        let region = NeighbourhoodRegion {
                            rect: field_rect,
                            original: offset(fields),
                            blur: [offset(fields + 1), offset(fields + 2)],
                            output: offset(output as u32),
                        };
                        kernels.encode(device, &mut encoder, &scratch, region, stage.filter);
                    }
                }
                let bindings = bindings(&values);
                {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("tc_composite"), timestamp_writes: None });
                    pass.set_pipeline(&self.pipeline);
                    pass.set_bind_group(0, &bindings, &[]);
                    pass.dispatch_workgroups(width.div_ceil(WORKGROUP), rows.div_ceil(WORKGROUP), 1);
                }
                for (texture, copy) in textures.iter().copied().zip(copies) {
                    let Some((offset, bytes_per_texel)) = copy else { continue };
                    encoder.copy_buffer_to_texture(
                        wgpu::TexelCopyBufferInfo {
                            buffer: &scratch,
                            layout: wgpu::TexelCopyBufferLayout { offset: stride * offset, bytes_per_row: Some(pitch * bytes_per_texel), rows_per_image: None },
                        },
                        wgpu::TexelCopyTextureInfo { origin: wgpu::Origin3d { x: left, y: top, z: 0 }, ..texture.as_image_copy() },
                        wgpu::Extent3d { width, height: rows, depth_or_array_layers: 1 },
                    );
                }
                let band = CompositeRegion { min: [left, top], max: [left + width, top + rows] };
                bounds = Some(bounds.map_or(band, |region| region.union(band)));
                work = work.saturating_add(band_work);
            }
            if let Some(written) = bounds {
                last_submission = Some(submit(encoder, written, submitted));
            }
            pending_bytes += bytes;
        }
        Ok(())
    }
}

fn composite_band_size(width: u32, tile_work_per_pixel: u64) -> (u32, u32) {
    let texels = MAX_TILE_WORK_PER_BAND / tile_work_per_pixel.max(1);
    // Leave room for a full workgroup of rows when a costly pattern needs narrower bands.
    let width = width.min((texels / u64::from(WORKGROUP)).max(1) as u32);
    let rows = (texels / u64::from(width)).clamp(1, u64::from(ROWS)) as u32;
    (width, rows)
}

/// Split only when the paint for this viewport window exceeds the device budget. Include a
/// source halo because the UV padding shader can choose a texel from an adjoining window.
fn paint_windows(
    program: &CompositeProgram,
    region: CompositeRegion,
    size: (u32, u32),
    shift: u32,
    radius: u32,
    limit: usize,
) -> Result<Vec<(CompositeRegion, TileWindow, usize)>, String> {
    let mut work = vec![region];
    let mut windows = Vec::new();
    while let Some(chunk) = work.pop() {
        let source = CompositeRegion { min: chunk.min.map(|p| p << shift), max: chunk.max.map(|p| p << shift) }.expanded(radius);
        let window = TileWindow::covering(source.min, source.max, [size.0, size.1]);
        if let Some(words) = window.words(&program.paint, limit) {
            windows.push((chunk, window, words));
            continue;
        }
        let width = chunk.max[0] - chunk.min[0];
        let height = chunk.max[1] - chunk.min[1];
        let axis = usize::from(height >= width);
        let length = chunk.max[axis] - chunk.min[axis];
        if length < 2 {
            return Err("Paint inputs for one GPU texel exceed the adapter's upload budget".into());
        }
        let mut first = chunk;
        let mut second = chunk;
        let middle = chunk.min[axis] + length / 2;
        first.max[axis] = middle;
        second.min[axis] = middle;
        work.push(second);
        work.push(first);
    }
    Ok(windows)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod procedural_tests {
    use super::*;
    use crate::{headless::Headless, readback::Readback};
    use texturecraft_compose::PatternContext;
    use texturecraft_compose::pattern::{Coords, procedural_in};
    use texturecraft_core::{Fill, Layer, Mask, OutputResolution, Procedural, TextureSet};
    use texturecraft_geom::primitives;
    use texturecraft_texel::{srgb_lut, to_u16};

    #[test]
    fn neighbourhood_filter_chains_match_cpu_on_paint_and_masks() {
        use texturecraft_compose::eval::{Composite, Inputs, composite_tiles, mask_image};
        use texturecraft_compose::tile_bytes;

        let Ok(mut gpu) = Headless::new() else { return };
        const SIZE: u32 = 128;
        let size = (SIZE, SIZE);
        let tiles = [(0, 0), (1, 0), (0, 1), (1, 1)];
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let baked = BakedMaps::default();
        let images = ImageStore::default();
        let channels = [Channel::BaseColor];
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[(size, 0)], 1);

        let filters = [
            Filter::Blur { radius: 64.0 },
            Filter::Sharpen { amount: 0.85 },
            Filter::HighPass { radius: 64.0 },
            Filter::Bevel { width: 128.0 },
            Filter::Glow { radius: 64.0, amount: 0.65 },
            Filter::DirectionalBlur { radius: 64.0, angle: 34.0 },
            Filter::Warp { intensity: 64.0, scale: 6.0, seed: 11 },
            Filter::DirectionalWarp { intensity: 64.0, angle: 40.0, scale: 6.0, seed: 11 },
            Filter::EdgeDetect { width: 52.0 },
            Filter::SlopeBlur { radius: 64.0, scale: 6.0, seed: 11 },
            Filter::Mosaic { size: 64.0 },
        ];
        for filter in filters {
            let mut project = Project::new("Neighbourhood filters", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
            let owner = project.alloc_id();
            let mut layer = Layer::new(owner, "Paint", LayerKind::Paint, &channels);
            layer.channels.get_mut(&Channel::BaseColor).unwrap().mode = BlendMode::Replace;
            let mut mask = Mask::new(MaskBackground::Black);
            let mask_paint = project.alloc_id();
            mask.effects.push(Effect::new(mask_paint, EffectKind::Paint));
            for stack in [&mut layer.content_effects, &mut mask.effects] {
                let mut effect = Effect::new(project.alloc_id(), EffectKind::Filter(filter.clone()));
                effect.opacity = 0.8;
                stack.push(effect);
                stack.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Clamp { min: 0.18, max: 0.86 })));
                stack.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 24.0 })));
            }
            layer.mask = Some(mask);
            project.insert_layer(0, layer, None, false).unwrap();
            let mut pixels = PixelStore::default();
            let mut mask_image_pixels = TileImage::new(SIZE, SIZE, 2);
            let layer_paint = pixels.layer_mut(owner, Channel::BaseColor, size);
            for y in 22..103 {
                for x in 23..98 {
                    let gain = if x >= 63 { 1.0 } else { 0.65 };
                    layer_paint.set(x, y, &[to_u16(0.7 * gain), to_u16(0.2 * gain), to_u16(0.3 * gain), to_u16(gain)]);
                    mask_image_pixels.set(x, y, &[to_u16(if y <= 64 { 0.8 } else { 0.4 }), u16::MAX]);
                }
            }
            pixels.masks.insert(mask_paint, mask_image_pixels);

            let inputs = Inputs {
                project: &project,
                ts: 0,
                mesh: &mesh,
                gbuf: &gbuf,
                pixels: &pixels,
                baked: &baked,
                block_scales: project.texture_sets[0].block_scales(),
                preview: None,
                images: &images,
                bounds: mesh.bounds(),
            };
            let mut oracle = Composite::new(&channels, SIZE);
            composite_tiles(&inputs, &mut oracle, &tiles);
            let mut program = CompositeProgram::try_compile_region(&project, &pixels, 0, None).unwrap();
            program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
            assert_eq!(program.filters.stages.len(), 4);
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
            let texture = gpu.renderer.display_textures()[0][DisplayKind::BaseColor as usize];
            let (_, _, full) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap();
            for (tx, ty) in tiles {
                let expected = tile_bytes(&oracle, &baked, DisplayKind::BaseColor, tx, ty);
                for (y, row) in expected.chunks_exact((TILE * 4) as usize).enumerate() {
                    let start = ((ty * TILE + y as u32) * SIZE * 4 + tx * TILE * 4) as usize;
                    let actual = &full[start..start + row.len()];
                    for (x, (gpu, cpu)) in actual.as_chunks::<4>().0.iter().zip(row.as_chunks::<4>().0).enumerate() {
                        let largest = gpu.iter().zip(cpu).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
                        assert!(largest <= 2, "{filter:?} material ({}, {}): GPU {gpu:?}, CPU {cpu:?}", tx * TILE + x as u32, ty * TILE + y as u32);
                    }
                }
            }
            program.region = Some(CompositeRegion { min: [61, 57], max: [75, 73] });
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
            let texture = gpu.renderer.display_textures()[0][DisplayKind::BaseColor as usize];
            let (_, _, partial) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap();
            assert!(partial.iter().zip(&full).all(|(a, b)| a.abs_diff(*b) <= 1), "{filter:?} partial update changed the result");

            let expected = mask_image(&inputs, owner, SIZE).unwrap();
            let mut mask = CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, owner, None, None).unwrap();
            mask.set_surface_inputs(&mesh, &gbuf, None).unwrap();
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &mask).unwrap();
            let texture = gpu.renderer.aux_texture(0).unwrap();
            let (_, _, actual) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap();
            for (index, (&expected, pixel)) in expected.iter().zip(actual.as_chunks::<4>().0).enumerate() {
                assert!(pixel[0].abs_diff(expected) <= 2, "{filter:?} mask texel {index}: {} instead of {expected}", pixel[0]);
            }
        }
    }

    #[test]
    fn tile_generator_band_size_budgets_overlapping_work_at_8k() {
        use texturecraft_core::{TileGenerator, TileSizeMode};

        let mut patterns = TilePatterns::default();
        let normal = TileGenerator { amount: [64, 64], ..TileGenerator::default() };
        let overlapping = TileGenerator { size_mode: TileSizeMode::Absolute, size_absolute: [1.0; 2], ..normal.clone() };
        let normal = patterns.register(normal, 0);
        let overlapping_index = patterns.register(overlapping.clone(), 0);
        let regular_work = patterns.work_per_sample(normal).unwrap();
        let overlapping_work = patterns.work_per_sample(overlapping_index).unwrap();

        assert_eq!(composite_band_size(8192, regular_work), (8192, ROWS));
        let (width, rows) = composite_band_size(8192, overlapping_work);
        assert!(width < 8192 && rows < ROWS);
        assert!(u64::from(width) * u64::from(rows) * overlapping_work <= MAX_TILE_WORK_PER_BAND);

        let mut project = Project::new("Tile Generator budget", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, 8192));
        let pattern = Procedural {
            resolution: OutputResolution { factor: -1, ..Default::default() },
            tile: Some(Box::new(overlapping)),
            seed: 0,
            ..Procedural::new(ProceduralKind::TileGenerator)
        };
        let source = Source::Procedural { pattern, low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() };
        let fill = Fill {
            sources: BTreeMap::from([(Channel::BaseColor, source)]),
            projection: Projection::Triplanar { scale: 1.0, sharpness: 6.0, offset: [0.0; 3] },
            ..Default::default()
        };
        let layer = Layer::new(project.alloc_id(), "Overlapping tiles", LayerKind::Fill(fill), &[Channel::BaseColor]);
        project.insert_layer(0, layer, None, false).unwrap();
        let program = CompositeProgram::compile(&project, &PixelStore::default(), 0).unwrap();
        assert_eq!(program.tile_work_per_pixel().unwrap(), overlapping_work * 12, "the three planes each need four samples");
    }

    #[test]
    fn point_filters_match_cpu_for_material_channels_and_mask_preview() {
        use texturecraft_compose::display::tile_bytes;
        use texturecraft_compose::eval::{Composite, Inputs, composite_tiles, mask_image};

        let Ok(mut gpu) = Headless::new() else { return };
        const SIZE: u32 = TILE;
        let mut mesh = primitives::plane(1.0, 1);
        for (normal, position) in mesh.normals.iter_mut().zip(&mesh.positions) {
            if position.x > 0.0 {
                *normal = Vec3::Z;
            }
        }
        let mesh = Arc::new(mesh);
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let baked = BakedMaps { resolution: 2, maps: BTreeMap::from([(BakedMap::AmbientOcclusion, Arc::new(vec![0.25, 0.7, 0.9, 0.45]))]) };
        let images = ImageStore::default();
        let pixels = PixelStore::default();
        let channels = [Channel::BaseColor, Channel::Roughness, Channel::Height, Channel::Normal];
        let sources = BTreeMap::from([
            (Channel::BaseColor, Source::Uniform([0.47, 0.3, 0.58])),
            (Channel::Roughness, Source::Uniform([0.41; 3])),
            (Channel::Height, Source::Uniform([-0.34; 3])),
            (Channel::Normal, Source::Uniform([0.37, 0.68, 0.89])),
        ]);
        let filters = [
            Filter::ColorBalance { shadows: [0.27, 0.06, -0.13], midtones: [-0.11, 0.23, 0.09], highlights: [0.08, 0.21, -0.08] },
            Filter::GradientMap { colors: vec![[0.04, 0.13, 0.7], [0.85, 0.56, 0.22], [0.76, 0.11, 0.34]] },
            Filter::GradientMap { colors: Vec::new() },
            Filter::ReplaceColor { from: [0.2, 0.43, 0.72], to: [0.62, 0.35, 0.33], tolerance: 0.09, softness: 0.28 },
            Filter::BakedLighting { azimuth: 30.0, elevation: 60.0, light: 0.6, ambient: 0.35, occlusion: 0.72 },
        ];
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        for filter in filters {
            let mut project = Project::new("Point filters", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
            let owner = project.alloc_id();
            let fill = Fill { sources: sources.clone(), ..Default::default() };
            let mut layer = Layer::new(owner, "Surface", LayerKind::Fill(fill), &channels);
            let mut effect = Effect::new(project.alloc_id(), EffectKind::Filter(filter.clone()));
            effect.opacity = 0.77;
            layer.content_effects.push(effect);
            let mut mask = Mask::new(MaskBackground::Black);
            mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Fill { source: Source::Uniform([0.36; 3]), projection: Projection::default() }));
            let mut effect = Effect::new(project.alloc_id(), EffectKind::Filter(filter.clone()));
            effect.opacity = 0.63;
            effect.blend = BlendMode::Multiply;
            mask.effects.push(effect);
            layer.mask = Some(mask);
            project.insert_layer(0, layer, None, false).unwrap();

            let inputs = Inputs {
                project: &project,
                ts: 0,
                mesh: &mesh,
                gbuf: &gbuf,
                pixels: &pixels,
                baked: &baked,
                block_scales: project.texture_sets[0].block_scales(),
                preview: None,
                images: &images,
                bounds: mesh.bounds(),
            };
            let mut oracle = Composite::new(&channels, SIZE);
            composite_tiles(&inputs, &mut oracle, &[(0, 0)]);
            let mut material = CompositeProgram::try_compile_region(&project, &pixels, 0, None).unwrap();
            material.set_surface_inputs(&mesh, &gbuf, Some(&baked)).unwrap();
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &material).unwrap();
            for kind in [DisplayKind::BaseColor, DisplayKind::Orm, DisplayKind::Height, DisplayKind::Normal] {
                let expected = tile_bytes(&oracle, &baked, kind, 0, 0);
                let texture = gpu.renderer.display_textures()[0][kind as usize];
                let (_, _, actual) = Readback::blocking_level(&gpu.device, &gpu.queue, texture, 0).unwrap();
                assert_eq!(actual.len(), expected.len());
                if kind.bytes_per_texel() == 8 {
                    for (actual, expected) in actual.as_chunks::<2>().0.iter().zip(expected.as_chunks::<2>().0) {
                        let actual = half::f16::from_le_bytes(*actual).to_f32();
                        let expected = half::f16::from_le_bytes(*expected).to_f32();
                        assert!((actual - expected).abs() <= 0.001, "{filter:?} {kind:?}: GPU {actual} vs CPU {expected}");
                    }
                } else {
                    assert!(actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1), "{filter:?} {kind:?}");
                }
            }

            let expected = mask_image(&inputs, owner, SIZE).unwrap();
            let mut mask = CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, owner, None, None).unwrap();
            mask.set_surface_inputs(&mesh, &gbuf, Some(&baked)).unwrap();
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &mask).unwrap();
            let texture = gpu.renderer.aux_texture(0).unwrap();
            let (_, _, rgba) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap();
            assert_eq!(rgba.len(), expected.len() * 4);
            for (&expected, &[r, g, b, a]) in expected.iter().zip(rgba.as_chunks::<4>().0) {
                assert!(r.abs_diff(expected) <= 1 && r == g && r == b && a == 255, "{filter:?} mask: GPU {r} vs CPU {expected}");
            }
        }
    }

    #[test]
    fn baked_lighting_uses_the_covered_padding_source_with_or_without_ao() {
        use texturecraft_compose::eval::{Composite, Inputs, composite_tiles, pad_tiles};
        use texturecraft_compose::tile_bytes;
        use texturecraft_geom::glam::Vec2;
        use texturecraft_texel::DilationMap;

        let Ok(mut gpu) = Headless::new() else { return };
        const SIZE: u32 = TILE;
        const PADDING: u32 = 4;
        let mut mesh = primitives::plane(1.0, 1);
        mesh.normals.fill(Vec3::Z);
        for uv in &mut mesh.uvs {
            *uv = *uv * 0.5 + Vec2::splat(0.25);
        }
        let mesh = Arc::new(mesh);
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let padding = DilationMap::new(&gbuf.coverage_mask(), SIZE, SIZE, Some(PADDING));
        let gutter = ((SIZE / 2) * SIZE + SIZE / 4 - 1) as usize;
        assert!(!gbuf.covered(gutter) && padding.source(SIZE / 4 - 1, SIZE / 2).is_some());

        let mut project = Project::new("Padded baked lighting", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
        let channels = [Channel::BaseColor];
        let owner = project.alloc_id();
        let fill = Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.8, 0.45, 0.1]))]), ..Default::default() };
        let mut layer = Layer::new(owner, "Lit surface", LayerKind::Fill(fill), &channels);
        let filter = Filter::BakedLighting { azimuth: 0.0, elevation: 20.0, light: 0.45, ambient: 0.45, occlusion: 1.0 };
        layer.content_effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(filter)));
        project.insert_layer(0, layer, None, false).unwrap();

        // The gutter is in a different baked AO cell from its covered source.
        let ao = BakedMaps { resolution: 4, maps: BTreeMap::from([(BakedMap::AmbientOcclusion, Arc::new([0.05, 0.6, 0.4, 0.2].repeat(4)))]) };
        let empty = BakedMaps::default();
        let pixels = PixelStore::default();
        let images = ImageStore::default();
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        let mut reds = [0; 2];
        for (i, baked) in [None, Some(&ao)].into_iter().enumerate() {
            let inputs = Inputs {
                project: &project,
                ts: 0,
                mesh: &mesh,
                gbuf: &gbuf,
                pixels: &pixels,
                baked: baked.unwrap_or(&empty),
                block_scales: project.texture_sets[0].block_scales(),
                preview: None,
                images: &images,
                bounds: mesh.bounds(),
            };
            let mut oracle = Composite::new(&channels, SIZE);
            composite_tiles(&inputs, &mut oracle, &[(0, 0)]);
            pad_tiles(&mut oracle, &padding, &[(0, 0)]);
            let expected = tile_bytes(&oracle, inputs.baked, DisplayKind::BaseColor, 0, 0);

            let mut program = CompositeProgram::try_compile_region(&project, &pixels, 0, None).unwrap();
            program.set_surface_inputs(&mesh, &gbuf, baked).unwrap();
            program.set_coverage(gbuf.clone(), PADDING);
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
            let texture = gpu.renderer.display_textures()[0][DisplayKind::BaseColor as usize];
            let (_, _, actual) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap();
            assert_eq!(actual.len(), expected.len());
            assert!(actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1), "covered and padded lighting must match the CPU, AO={}", baked.is_some());
            reds[i] = actual[gutter * 4];
        }
        assert!(reds[0] > reds[1], "missing AO leaves the ambient light at full strength in the gutter");
    }

    #[test]
    fn narrow_gpu_generator_ramp_preserves_baked_scalar_precision() {
        use texturecraft_compose::eval::{Inputs, mask_image};

        let Ok(mut gpu) = Headless::new() else { return };
        const SIZE: u32 = 64;
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let mut project = Project::new("Narrow mask ramp", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
        let owner = project.alloc_id();
        let mut layer = Layer::new(owner, "AO mask", LayerKind::Fill(Fill::default()), &[Channel::BaseColor]);
        let mut mask = Mask::new(MaskBackground::Black);
        let generator = Generator { softness: Generator::MIN_SOFTNESS, ..Generator::new(GeneratorKind::AmbientOcclusion) };
        mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Generator(generator)));
        layer.mask = Some(mask);
        project.insert_layer(0, layer, None, false).unwrap();
        let baked = BakedMaps { resolution: 1, maps: BTreeMap::from([(BakedMap::AmbientOcclusion, Arc::new(vec![0.5002]))]) };
        let pixels = PixelStore::default();
        let inputs = Inputs {
            project: &project,
            ts: 0,
            mesh: &mesh,
            gbuf: &gbuf,
            pixels: &pixels,
            baked: &baked,
            block_scales: project.texture_sets[0].block_scales(),
            preview: None,
            images: &ImageStore::default(),
            bounds: mesh.bounds(),
        };
        let expected = mask_image(&inputs, owner, SIZE).unwrap();
        let mut program = CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, owner, None, None).unwrap();
        program.set_surface_inputs(&mesh, &gbuf, Some(&baked)).unwrap();

        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
        let texture = gpu.renderer.aux_texture(0).unwrap();
        let (_, _, rgba) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap();
        assert!(expected.iter().any(|&value| value > 150), "fixture distinguishes f32 from half-rounded 0.5");
        for (&cpu, texel) in expected.iter().zip(rgba.as_chunks::<4>().0) {
            assert!(texel[0].abs_diff(cpu) <= 1, "GPU mask {} differs from CPU {cpu}", texel[0]);
        }
    }

    #[test]
    fn steep_baked_normal_keeps_its_direction_with_painted_detail() {
        let Ok(mut gpu) = Headless::new() else { return };
        const SIZE: u32 = 64;
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let mut project = Project::new("Steep baked normal", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
        let owner = project.alloc_id();
        let detail = [0.75, 0.5, 0.9330127];
        let fill = Fill { sources: BTreeMap::from([(Channel::Normal, Source::Uniform(detail))]), ..Default::default() };
        let layer = Layer::new(owner, "Surface detail", LayerKind::Fill(fill), &[Channel::Normal]);
        project.insert_layer(0, layer, None, false).unwrap();
        let steep = [0.5158094, 0.5, 0.00025];
        let baked = BakedMaps { resolution: 1, maps: BTreeMap::from([(BakedMap::Normal, Arc::new(steep.to_vec()))]) };
        let pixels = PixelStore::default();
        let mut program = CompositeProgram::compile(&project, &pixels, 0).unwrap();
        program.set_surface_inputs(&mesh, &gbuf, Some(&baked)).unwrap();

        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
        let texture = gpu.renderer.display_textures()[0][DisplayKind::Normal as usize];
        let (width, _, bytes) = Readback::blocking_level(&gpu.device, &gpu.queue, texture, 0).unwrap();
        let center = (width * (SIZE / 2) + SIZE / 2) as usize * DisplayKind::Normal.bytes_per_texel();
        let actual = Vec3::from_array([0, 2, 4].map(|offset| half::f16::from_le_bytes(bytes[center + offset..][..2].try_into().unwrap()).to_f32()));
        let expected = Vec3::from_array(texturecraft_core::blend::rnm(steep, detail)) * 2.0 - Vec3::ONE;
        assert!(actual.dot(expected) > 0.99, "GPU {actual} vs CPU {expected}");
    }

    #[test]
    fn material_patterns_match_cpu_as_fills_and_masks() {
        use ProceduralKind::*;
        const SIZE: u32 = 128;
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let (lo, hi) = mesh.bounds();
        let mut gpu = Headless::new().ok();
        if let Some(gpu) = &mut gpu {
            gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        }
        let uv = Projection::Uv { scale: [1.15, 0.76], rotation: 13.0, offset: [-0.51, 0.12] };
        let triplanar = Projection::Triplanar { scale: 0.65, sharpness: 6.0, offset: [-0.23, 0.12, 0.43] };
        let cases = ProceduralKind::ALL
            .iter()
            .copied()
            .map(|kind| Procedural { scale: 9.7, seed: 73, contrast: 1.1, balance: -0.03, invert: true, ..Procedural::new(kind) })
            .flat_map(|pattern| [(pattern.clone(), uv.clone(), false, Vec::new()), (pattern, triplanar.clone(), true, Vec::new())]);
        let ramp = vec![[0.01, 0.42, 0.13], [0.73, 0.05, 0.38], [0.4, 0.71, 0.85]];
        let reduced_pattern =
            Procedural { scale: 3.2, seed: 11, resolution: OutputResolution { factor: -2, ..OutputResolution::default() }, ..Procedural::new(Clouds) };
        let reduced = [
            uv.clone(),
            triplanar.clone(),
            Projection::Planar { scale: 0.8, axis: 1, offset: [0.03, 0.0, 0.34] },
            Projection::Spherical { scale: 0.9 },
            Projection::Cylindrical { scale: 0.7, axis: 2 },
        ]
        .into_iter()
        .flat_map(|projection| [(reduced_pattern.clone(), projection.clone(), false, ramp.clone()), (reduced_pattern.clone(), projection, true, ramp.clone())]);
        for (pattern, projection, in_mask, ramp) in cases.chain(reduced) {
            let kind = pattern.kind;
            let context = PatternContext { pixels: pattern_pixels(&pattern, SIZE, SIZE), images: None };
            let source = Source::Procedural { pattern: pattern.clone(), low: [0.0; 3], high: [1.0; 3], ramp: ramp.clone() };
            let mut project = Project::new("Procedural material", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
            let fill = Fill {
                sources: BTreeMap::from([(Channel::BaseColor, if in_mask { Source::Uniform([0.9; 3]) } else { source.clone() })]),
                projection: projection.clone(),
                ..Fill::default()
            };
            let mut layer = Layer::new(project.alloc_id(), "Procedural fill", LayerKind::Fill(fill), &[Channel::BaseColor]);
            if in_mask {
                let mut mask = Mask::new(MaskBackground::Black);
                mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Fill { source, projection: projection.clone() }));
                layer.mask = Some(mask);
            }
            project.insert_layer(0, layer, None, false).unwrap();
            let mut program = CompositeProgram::try_compile_region(&project, &PixelStore::default(), 0, None).unwrap();
            if let Some(gpu) = &mut gpu {
                program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
                gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
                let (_, _, rgba) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.display_textures()[0][0]).unwrap();
                for index in (0..rgba.len() / 4).step_by(37) {
                    let surface = gbuf.surface(&mesh, index).unwrap();
                    let coords = Coords { uv: surface.uv, pos: (surface.pos - (lo + hi) * 0.5) / ((hi - lo).length() * 0.5), nrm: surface.nrm };
                    let value = procedural_in(&pattern, &projection, &coords, &context);
                    let linear = if in_mask { [0.5 + (0.9 - 0.5) * value; 3] } else { texturecraft_core::gradient(&ramp, value).unwrap_or([value; 3]) };
                    let expected = linear.map(|linear| srgb_lut()[usize::from(to_u16(linear) >> 4)]);
                    for (channel, expected) in expected.into_iter().enumerate() {
                        let actual = rgba[index * 4 + channel];
                        assert!(
                            actual.abs_diff(expected) <= 3,
                            "{kind:?}, {projection:?}, mask={in_mask}, texel {index}, channel {channel}, expected {expected} got {actual}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn tile_generator_matches_cpu_with_shaped_and_authored_tiles() {
        use texturecraft_compose::{ImageRes, ImageStore, PatternContext, pattern::procedural_in};
        use texturecraft_core::{TileBlending, TileGenerator, TileInputDistribution, TileShape, TileSizeMode};

        const SIZE: u32 = 128;
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let (lo, hi) = mesh.bounds();
        let image = |rgba| Arc::new(ImageRes { width: 2, height: 2, rgba, srgb: false });
        let images = ImageStore::from([
            ("tile".into(), image(vec![[1.0, 0.0, 0.0, 1.0], [1.0; 4], [0.0; 4], [0.4; 4]])),
            ("another".into(), image(vec![[0.3; 4], [0.6; 4], [0.2; 4], [0.9; 4]])),
            ("background".into(), image(vec![[0.1; 4], [0.2; 4], [0.4; 4], [0.3; 4]])),
        ]);
        let context = PatternContext { images: Some(&images), ..Default::default() };
        let mut gpu = Headless::new().ok();
        if let Some(gpu) = &mut gpu {
            gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        }
        for (index, shape) in TileShape::ALL.into_iter().enumerate() {
            let in_mask = index % 2 != 0;
            let projection = if in_mask {
                Projection::Triplanar { scale: 0.65, sharpness: 6.0, offset: [-0.23, 0.12, 0.43] }
            } else {
                Projection::Uv { scale: [1.15, 0.76], rotation: 13.0, offset: [-0.51, 0.12] }
            };
            let tile = TileGenerator {
                shape,
                amount: [5, 3],
                shape_angle: 13.0,
                shape_scale: 0.93,
                shape_size: [0.81, 0.93],
                shape_tiling: (index % 2 + 1) as u32,
                shape_diagonal: index % 3 == 0,
                inputs: vec!["tile".into(), "absent".into(), "another".into()],
                background_image: Some("background".into()),
                input_distribution: [TileInputDistribution::ByNumber, TileInputDistribution::Random][index % 2],
                size_mode: TileSizeMode::ALL[index % TileSizeMode::ALL.len()],
                middle_size: [0.42, 0.6],
                interstice: [0.11, 0.32],
                size_pixel: [429.0, 700.0],
                size_absolute: [0.3, 0.39],
                size_random: [0.12, 0.31],
                position_random: [0.14, 0.38],
                global_offset: [0.1, 0.35],
                scale: 1.3,
                scale_random: 0.37,
                offset: 0.31,
                offset_random: 0.4,
                vertical_offset: index % 3 != 0,
                quarter_turns: 1,
                quarter_turn_random: 0.4,
                quincunx_flip: true,
                symmetry_random: 0.76,
                rotation: 13.0,
                rotation_random: 0.29,
                luminance_by_scale: true,
                luminance_random: 0.13,
                color: 0.87,
                opacity: 0.93,
                reverse_order: index % 2 != 0,
                random_mask: 0.13,
                blending: TileBlending::ALL[index % TileBlending::ALL.len()],
                position: 0.56,
                contrast: 0.15,
                ..Default::default()
            };
            let pattern = Procedural { tile: Some(Box::new(tile)), seed: 73, ..Procedural::new(ProceduralKind::TileGenerator) };
            let source = Source::Procedural { pattern: pattern.clone(), low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() };
            let mut project = Project::new("Tile Generator", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
            let fill = Fill {
                sources: BTreeMap::from([(Channel::BaseColor, if in_mask { Source::Uniform([0.9; 3]) } else { source.clone() })]),
                projection: projection.clone(),
                ..Fill::default()
            };
            let mut layer = Layer::new(project.alloc_id(), "Tile Generator", LayerKind::Fill(fill), &[Channel::BaseColor]);
            if in_mask {
                let mut mask = Mask::new(MaskBackground::Black);
                mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Fill { source, projection: projection.clone() }));
                layer.mask = Some(mask);
            }
            project.insert_layer(0, layer, None, false).unwrap();
            let mut program = CompositeProgram::try_compile_region(&project, &PixelStore::default(), 0, None).unwrap();
            program.set_tile_images(&images);
            if let Some(gpu) = &mut gpu {
                program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
                gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
                let (_, _, rgba) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.display_textures()[0][0]).unwrap();
                for index in (0..rgba.len() / 4).step_by(37) {
                    let surface = gbuf.surface(&mesh, index).unwrap();
                    let coords = Coords { uv: surface.uv, pos: (surface.pos - (lo + hi) * 0.5) / ((hi - lo).length() * 0.5), nrm: surface.nrm };
                    let value = procedural_in(&pattern, &projection, &coords, &context);
                    let linear = if in_mask { 0.5 + (0.9 - 0.5) * value } else { value };
                    let expected = srgb_lut()[usize::from(to_u16(linear) >> 4)];
                    let actual = rgba[index * 4];
                    assert!(actual.abs_diff(expected) <= 3, "{shape:?}, mask={in_mask}, texel {index}: {actual} instead of {expected}");
                }
            }
        }
    }

    #[test]
    fn authored_material_images_match_cpu_projections_and_decal_coverage() {
        use texturecraft_compose::{
            BakedMaps, ImageRes,
            display::tile_bytes,
            eval::{Composite, Inputs, composite_tiles, mask_image},
        };
        use texturecraft_core::{Decal, DecalFlip, DecalSymmetry};

        const SIZE: u32 = TILE;
        let Ok(mut gpu) = Headless::new() else { return };
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        let mut mesh = primitives::plane(1.0, 1);
        mesh.normals.fill(Vec3::new(0.3, 0.9, -0.2).normalize());
        let mesh = Arc::new(mesh);
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let image = |rgba, srgb| Arc::new(ImageRes { width: 2, height: 2, rgba, srgb });
        let bytes = [[18, 206, 44, 17], [191, 51, 20, 237], [44, 12, 99, 0], [213, 221, 40, 110]].map(|pixel| pixel.map(|v| v as f32 / 255.0));
        let shorts = [[15324, 60403, 21177, 65535], [33499, 2177, 5828, 0], [21643, 8005, 11999, 9001], [57132, 39801, 6021, 32777]]
            .map(|pixel| pixel.map(|v| v as f32 / 65535.0));
        let floats = [[0.1321, 0.223, 0.13, 1.0], [1.34, 0.11, 0.15, 0.7], [-0.051, 0.41, 0.622, 0.12], [0.09, 0.221, 0.94, 0.0]];
        let images = ImageStore::from([
            ("color".into(), image(bytes.to_vec(), true)),
            ("normal".into(), image(shorts.to_vec(), true)),
            ("float".into(), image(floats.to_vec(), false)),
        ]);
        let decal = Decal {
            center: [0.26, -0.06, 0.06],
            normal: [0.0, 1.0, 0.0],
            up: [0.0, 0.0, 1.0],
            size: [0.51, 0.71],
            rotation: 12.0,
            offset: [0.018, 0.021],
            depth: 0.073,
            symmetry: Some(DecalSymmetry { axis: 0, flip: DecalFlip::V, offset: 0.014 }),
            flip: [true, false],
        };
        let projections = [
            Projection::Uv { scale: [1.21, -0.78], rotation: 39.0, offset: [-0.54, 0.16] },
            Projection::Triplanar { scale: 0.63, sharpness: 2.1, offset: [0.14, -0.27, 0.31] },
            Projection::Planar { scale: 1.4, axis: 0, offset: [0.04, 0.23, 0.18] },
            Projection::Planar { scale: 0.9, axis: 1, offset: [-0.18, -0.34, 1.03] },
            Projection::Planar { scale: 1.1, axis: 2, offset: [0.65, -0.26, -0.31] },
            Projection::Spherical { scale: 1.09 },
            Projection::Cylindrical { scale: 1.21, axis: 0 },
            Projection::Cylindrical { scale: 2.19, axis: 1 },
            Projection::Cylindrical { scale: 0.87, axis: 2 },
            Projection::Decal(decal),
        ];
        let channels = [Channel::BaseColor, Channel::Roughness, Channel::Height, Channel::Normal];
        for projection in projections {
            let mut project = Project::new("Image source parity", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
            let fill = Fill {
                sources: BTreeMap::from([
                    (Channel::BaseColor, Source::image("color")),
                    (Channel::Roughness, Source::image("float")),
                    (Channel::Height, Source::image("float")),
                    (Channel::Normal, Source::Image { name: "normal".into(), normal_format: NormalFormat::DirectX }),
                ]),
                projection: projection.clone(),
                ..Fill::default()
            };
            let owner = project.alloc_id();
            let mut layer = Layer::new(owner, "Material images", LayerKind::Fill(fill), &channels);
            let mut mask = Mask::new(MaskBackground::Black);
            let mask_source = Source::image("normal");
            mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Fill { source: mask_source, projection: projection.clone() }));
            layer.mask = Some(mask);
            project.insert_layer(0, layer, None, false).unwrap();
            let pixels = PixelStore::default();
            let baked = BakedMaps::default();
            let inputs = Inputs {
                project: &project,
                ts: 0,
                mesh: &mesh,
                gbuf: &gbuf,
                pixels: &pixels,
                baked: &baked,
                block_scales: project.texture_sets[0].block_scales(),
                preview: None,
                images: &images,
                bounds: mesh.bounds(),
            };
            let mut cpu = Composite::new(&channels, SIZE);
            composite_tiles(&inputs, &mut cpu, &[(0, 0)]);
            let mut program = CompositeProgram::try_compile_region(&project, &pixels, 0, None).unwrap();
            program.set_tile_images(&images);
            program.set_surface_inputs(&mesh, &gbuf, Some(&baked)).unwrap();
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
            for kind in [DisplayKind::BaseColor, DisplayKind::Orm, DisplayKind::Height, DisplayKind::Normal] {
                let expected = tile_bytes(&cpu, &baked, kind, 0, 0);
                let texture = gpu.renderer.display_textures()[0][kind as usize];
                let (_, _, actual) = Readback::blocking_level(&gpu.device, &gpu.queue, texture, 0).unwrap();
                assert_eq!(actual.len(), expected.len());
                if kind.bytes_per_texel() == 8 {
                    for (actual, expected) in actual.as_chunks::<2>().0.iter().zip(expected.as_chunks::<2>().0) {
                        let actual = half::f16::from_le_bytes(*actual).to_f32();
                        let expected = half::f16::from_le_bytes(*expected).to_f32();
                        assert!((actual - expected).abs() <= 0.003, "{projection:?} {kind:?}: GPU {actual} vs CPU {expected}");
                    }
                } else {
                    assert!(actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 2), "{projection:?} {kind:?}");
                }
            }
            let expected = mask_image(&inputs, owner, SIZE).unwrap();
            let mut mask = CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, owner, None, None).unwrap();
            mask.set_tile_images(&images);
            mask.set_surface_inputs(&mesh, &gbuf, Some(&baked)).unwrap();
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &mask).unwrap();
            let texture = gpu.renderer.aux_texture(0).unwrap();
            let (_, _, actual) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap();
            assert_eq!(actual.len(), expected.len() * 4);
            for (expected, &[r, g, b, a]) in expected.into_iter().zip(actual.as_chunks::<4>().0) {
                assert!(r.abs_diff(expected) <= 2 && r == g && r == b && a == 255, "{projection:?} mask: GPU {r} vs CPU {expected}");
            }
        }
    }
}

#[cfg(test)]
mod paint_window_tests {
    use super::*;

    #[test]
    fn a_small_paint_budget_splits_the_output_without_losing_padding_sources() {
        let mut paint = TileImage::new(256, 256, 4);
        let pixels = Arc::new(vec![65535; 64 * 64 * 4]);
        for y in 0..4 {
            for x in 0..4 {
                paint.set_tile(x, y, Some(pixels.clone()));
            }
        }
        let program = CompositeProgram {
            filters: FilterPlan::default(),
            operations: Vec::new(),
            layer_count: 0,
            paint: vec![paint],
            ramp_words: Vec::new(),
            tile_patterns: TilePatterns::default(),
            material_images: MaterialImages::default(),
            brush_alpha: None,
            brush_stencil: None,
            region: None,
            coverage: None,
            surface: None,
            needs_surface: false,
            baked: None,
            display_outputs: 0,
            glossiness: false,
            mask_preview: false,
        };
        let region = CompositeRegion { min: [0, 0], max: [128, 128] };
        let limit = 6 * 64 * 64 * 2 + 32;
        let windows = paint_windows(&program, region, (256, 256), 1, 8, limit).unwrap();
        assert!(windows.len() > 1);
        let mut coverage = vec![0; 128 * 128];
        for (chunk, window, words) in windows {
            assert!(words <= limit);
            let mut packed = Vec::new();
            window.pack(&program.paint, words, &mut packed);
            assert_eq!(packed.len(), words);
            for y in chunk.min[1]..chunk.max[1] {
                for x in chunk.min[0]..chunk.max[0] {
                    coverage[(y * 128 + x) as usize] += 1;
                    for sy in [y.saturating_mul(2).saturating_sub(8), (2 * y + 8).min(255)] {
                        for sx in [x.saturating_mul(2).saturating_sub(8), (2 * x + 8).min(255)] {
                            let origin = window.origin;
                            assert!(sx / 64 >= origin[0] && sy / 64 >= origin[1]);
                            let (tx, ty) = (sx / 64 - origin[0], sy / 64 - origin[1]);
                            assert!(tx < window.size[0] && ty < window.size[1]);
                            let slot = (ty * window.size[0] + tx) as usize;
                            assert_ne!(packed[packed[0] as usize + slot], 0);
                        }
                    }
                }
            }
        }
        assert!(coverage.into_iter().all(|count| count == 1));
    }
}
