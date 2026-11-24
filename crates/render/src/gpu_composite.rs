//! Compile the viewport's supported UV fills to a small GPU layer program.

use std::sync::{Arc, Weak};
use std::{borrow::Cow, collections::BTreeMap};

use bytemuck::{Pod, Zeroable};
use rayon::prelude::*;
use texturecraft_compose::pattern::pattern_pixels;
use texturecraft_compose::{BakedMaps, DisplayKind, ImageStore, PixelStore};
use texturecraft_core::{
    BakedMap, BlendMode, Channel, ChannelKind, ColorSelection, CompareOp, CompareSource, CurvatureDetail, CurvatureMode, Effect, EffectKind, Filter, Generator,
    GeneratorKind, LayerId, LayerKind, Mask, MaskBackground, NormalFormat, ProceduralKind, Project, Projection, Sampling, Source, ThresholdMode,
};
use texturecraft_geom::{Mesh, NO_TRI, UvGBuffer, glam::Vec3};
use texturecraft_texel::{TILE, TileImage};

use crate::gpu_curvature::{CurvatureGpu, CurvatureInput, CurvatureLevels};
use crate::gpu_image_source::{DECAL_PROJECTION, IMAGE_SOURCE_OPCODE, MaterialImages};
use crate::gpu_neighbourhood::{self, FIELD_WORDS, NeighbourhoodFilter, NeighbourhoodGpu, NeighbourhoodRegion};
use crate::gpu_stencil::GpuStencil;
use crate::gpu_tile_pattern::{TILE_GENERATOR_OPCODE, TileImageAtlas, TilePatterns};
use crate::paint_window::CompositePaintWindow;
use wgpu::util::DeviceExt;

#[path = "gpu_composite_groups.rs"]
mod groups;
#[path = "gpu_composite_pipelines.rs"]
mod pipelines;
// The GPU port packages (texturecraft #713): the hub chains every package's hooks, each package
// owns its module. See gpu_composite_packages.rs for the ownership table and shared layouts.
#[path = "gpu_composite_brush.rs"]
mod brush;
#[path = "gpu_composite_projection.rs"]
mod fill_projection;
#[path = "gpu_composite_lighting.rs"]
mod lighting;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "gpu_composite_oracle.rs"]
pub(crate) mod oracle;
#[path = "gpu_composite_packages.rs"]
mod packages;
#[path = "gpu_composite_point.rs"]
mod point;
#[path = "gpu_composite_stack.rs"]
mod stack;
#[path = "gpu_composite_udim.rs"]
mod udim;
use groups::{BEGIN_GROUP, END_GROUP, GroupPlan, StackWork};
use pipelines::CompositePipelines;

const MAX_PAINT_WORDS: usize = (64 << 20) / size_of::<u32>();
const LOOKUP_WORDS: u32 = 4096;
const NO_PAINT_IMAGE: u32 = u32::MAX;
const BAKED_LIGHTING_OPCODE: u32 = 35;
const NEIGHBOURHOOD_OPCODE: u32 = 36;
const COMPARE_OPCODE: u32 = 37;
const COLOR_SELECTION_OPCODE: u32 = 38;
const NOOP_EFFECT_OPCODE: u32 = 39;
const UV_TILE_PROJECTION: u32 = 10;
const MASK_STAGE: u32 = 1 << 31;
const GENERATOR_STAGE: u32 = 1 << 30;
const MATERIAL_ANCHOR_STAGE: u32 = 1 << 29;
const MASK_ANCHOR_STAGE: u32 = 1 << 28;
const ANCHOR_SOURCE: u32 = 15;
const ANCHOR_PUBLISH_MASK: u32 = 16;
const ANCHOR_PUBLISH_LAYER: u32 = 17;
// Opcodes are classified by range, in the shaders too (`is_fill_opcode` in
// composite_packages.wgsl): fills and patterns below 20 and from 48 below 128, built-in effects
// 20..40, generators 40..48, the GPU port packages' effects 128..192 (`packages::EXTENDED_EFFECTS`,
// one block per package), group markers 200 and 201. Never renumber an existing opcode.
// Extended procedural sources follow the filter and generator opcodes.
const FIRST_GENERATOR: u32 = 40;
const FIRST_EXTENDED_PATTERN: u32 = 48;
pub(crate) const ROWS: u32 = 64;
const WORKGROUP: u32 = 8;
// Keep extreme Tile Generator overlap from monopolizing an update, dispatch, or submission.
const MAX_TILE_WORK_PER_UPDATE: u64 = 8 << 30;
const MAX_TILE_WORK_PER_BAND: u64 = 64 << 20;
const MAX_TILE_WORK_PER_SUBMISSION: u64 = 256 << 20;
const BAKE_MAPS: [BakedMap; 4] = [BakedMap::AmbientOcclusion, BakedMap::Normal, BakedMap::Curvature, BakedMap::Thickness];
const MAX_ID_BAKE_BYTES: u64 = 1 << 30;
const MAX_ID_UPLOAD_BYTES: usize = 64 << 20;

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

#[derive(Debug)]
struct IdBake {
    size: [u32; 2],
    data: Arc<Vec<f32>>,
}

impl IdBake {
    fn new(baked: &BakedMaps) -> Option<Self> {
        let data = baked.maps.get(&BakedMap::Id)?;
        let height = baked.height(BakedMap::Id);
        (height > 0).then(|| Self { size: [baked.resolution.max(1), height], data: data.clone() })
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
    // UV offset XY, procedural scale and contrast, live-brush values and opacity, or curvature weights and mode in XY.
    uv: [f32; 4],
    balance: [f32; 4],
    // Offsets and counts of the mask and content effects.
    effects: [u32; 4],
    // Sampling resolution, color-ramp offset and count, projection selector.
    procedural: [u32; 4],
    // First content field, content has fields, mask has fields, content can add coverage.
    spatial: [u32; 4],
    // Geometry mask row (resolved to a lookup offset at upload), height operation plus one,
    // height-mask contrast and offset encoded as floats.
    constraints: [u32; 4],
    // The GPU port packages' word, zero everywhere else (layout in gpu_composite_packages.rs):
    // side-table offset (0: none), P1 projection flags, P2/P3/P6/P7 flag bytes, P8 word.
    ext: [u32; 4],
}

#[derive(Debug, Default)]
struct GeometryMasks {
    parts: Vec<Vec<String>>,
    words: Vec<u32>,
    stride: usize,
}

#[derive(Default)]
struct PaintInputs {
    images: Vec<TileImage>,
    indices: BTreeMap<(LayerId, Option<Channel>), u32>,
}

impl PaintInputs {
    fn register(&mut self, id: LayerId, channel: Option<Channel>, image: &TileImage) -> u32 {
        *self.indices.entry((id, channel)).or_insert_with(|| {
            let index = self.images.len() as u32;
            self.images.push(image.clone());
            index
        })
    }
}

impl GeometryMasks {
    /// Zero allows every part. A passthrough folder restricts every child to its own allowed parts.
    fn register(&mut self, inherited: u32, own: Option<&[String]>) -> u32 {
        let Some(own) = own else { return inherited };
        let mut names = match inherited.checked_sub(1).and_then(|index| self.parts.get(index as usize)) {
            Some(parent) => own.iter().filter(|name| parent.binary_search(name).is_ok()).cloned().collect(),
            None => own.to_vec(),
        };
        names.sort_unstable();
        names.dedup();
        if let Some(index) = self.parts.iter().position(|mask| *mask == names) {
            return index as u32 + 1;
        }
        self.parts.push(names);
        self.parts.len() as u32
    }

    fn resolve(&mut self, mesh: &Mesh) {
        if self.parts.is_empty() {
            return;
        }
        let triangles = mesh.triangle_count();
        self.stride = triangles.div_ceil(32).max(1);
        self.words.clear();
        self.words.resize(self.parts.len() * self.stride, u32::MAX);
        let parts = &mesh.submeshes;
        let ordered = parts.windows(2).all(|pair| pair[0].start.saturating_add(pair[0].count) <= pair[1].start);
        for (names, row) in self.parts.iter().zip(self.words.chunks_exact_mut(self.stride)) {
            if ordered {
                for part in parts.iter().filter(|part| names.binary_search(&part.name).is_err()) {
                    let first = part.start.div_ceil(3) as usize;
                    let end = (part.start.saturating_add(part.count).div_ceil(3) as usize).min(triangles);
                    if first >= end {
                        continue;
                    }
                    for (index, word) in row.iter_mut().enumerate().take(end.div_ceil(32)).skip(first / 32) {
                        let start_bit = first.saturating_sub(index * 32);
                        let end_bit = (end - index * 32).min(32);
                        *word &= !((u32::MAX << start_bit) & (u32::MAX >> (32 - end_bit)));
                    }
                }
            } else {
                // Match the CPU's lookup even for overlapping or out-of-order submeshes.
                for triangle in 0..triangles {
                    let first = triangle as u32 * 3;
                    let holds = |index: usize| parts.get(index).is_some_and(|part| part.start <= first && first < part.start.saturating_add(part.count));
                    let part = parts
                        .partition_point(|part| part.start <= first)
                        .checked_sub(1)
                        .filter(|index| holds(*index))
                        .or_else(|| (0..parts.len()).find(|index| holds(*index)));
                    if part.and_then(|index| parts.get(index)).is_some_and(|part| names.binary_search(&part.name).is_err()) {
                        row[triangle / 32] &= !(1 << (triangle % 32));
                    }
                }
            }
        }
    }
}

struct ProceduralInputs {
    resolution: u32,
    ramp_words: Vec<u32>,
    curvature_sizes: [bool; CurvatureDetail::BLURRED_SIZES],
    filters: Vec<(Filter, f32)>,
    materials: MaterialImages,
    generator_stages: Vec<(Option<NeighbourhoodFilter>, u32)>,
    generator_sources: Vec<Operation>,
    anchors: AnchorInputs,
    /// A neighbourhood stage a package lowering asked to run right before the effect's own
    /// operation ([`Self::precede_with_stage`]); the callers of `point_effect` push it first.
    preceding_stage: Option<Operation>,
}

#[derive(Default)]
struct AnchorInputs {
    visible: BTreeMap<LayerId, (u32, bool)>,
    reads: Vec<(u32, Channel)>,
    draw_ranges: Vec<std::ops::Range<u32>>,
    next: u32,
}

impl AnchorInputs {
    fn publish(&mut self, id: LayerId, mask: bool) -> u32 {
        let publish = self.next;
        self.next += 1;
        self.visible.insert(id, (publish, mask));
        publish
    }

    fn read(&mut self, id: LayerId, channel: Channel) -> (u32, bool) {
        let Some(&(publish, mask)) = self.visible.get(&id) else { return (0, false) };
        let channel = if mask { Channel::BaseColor } else { channel };
        let read = (publish, channel);
        let index = self.reads.iter().position(|existing| *existing == read).unwrap_or_else(|| {
            let index = self.reads.len();
            self.reads.push(read);
            index
        });
        (index as u32 + 1, mask)
    }
}

impl ProceduralInputs {
    fn new(resolution: u32) -> Self {
        Self {
            resolution,
            ramp_words: Vec::new(),
            curvature_sizes: [false; CurvatureDetail::BLURRED_SIZES],
            filters: Vec::new(),
            materials: MaterialImages::default(),
            generator_stages: Vec::new(),
            generator_sources: Vec::new(),
            anchors: AnchorInputs::default(),
            preceding_stage: None,
        }
    }

    /// Lets a package lowering (in `packages::lower_filter`) run the neighbourhood stage of
    /// `filter` right before the effect's own operation: one effect, two operations. This is the
    /// seam of the Gradient's `input.blur` (P4 defines the stage, P2 the gradient that follows).
    /// The point operation then reads the stage's output field in place of the content, so a
    /// stage that must keep the original colour (for the opacity mix) passes its derived value in
    /// a component or the sixth field word, as P4 defines it. None, and nothing queued, when the
    /// GPU has no stage for `filter` or its size reaches nothing.
    #[allow(dead_code)] // Used by the GPU port packages
    fn precede_with_stage(&mut self, filter: &Filter, opacity: f32) -> Option<()> {
        NeighbourhoodFilter::new(filter, self.resolution, self.resolution, opacity, None)?;
        let ids = [0, NEIGHBOURHOOD_OPCODE, u32::try_from(self.filters.len()).ok()?, 0];
        self.filters.push((filter.clone(), opacity));
        self.preceding_stage = Some(Operation { ids, low: [0.0, 0.0, 0.0, opacity.clamp(0.0, 1.0)], ..Operation::zeroed() });
        Some(())
    }

    fn pack_rgb(&mut self, colors: &[[f32; 3]]) -> Option<[u32; 2]> {
        if colors.iter().flatten().any(|value| !value.is_finite()) {
            return None;
        }
        let offset = LOOKUP_WORDS.checked_add(u32::try_from(self.ramp_words.len()).ok()?)?;
        let count = u32::try_from(colors.len()).ok()?;
        self.ramp_words.extend(colors.iter().flatten().map(|value| value.to_bits()));
        Some([offset, count])
    }

    /// Append finite `values` to the parameter side table (the words after the sRGB lookup in
    /// binding 2, shared with [`Self::pack_rgb`]) and return their absolute word offset for
    /// `Operation::ext[0]`. The shader reads them with `param(op, i)`. None for a non-finite value.
    #[allow(dead_code)] // Used by the GPU port packages
    fn pack_params(&mut self, values: &[f32]) -> Option<u32> {
        if values.iter().any(|value| !value.is_finite()) {
            return None;
        }
        let words: Vec<u32> = values.iter().map(|value| value.to_bits()).collect();
        self.pack_words(&words)
    }

    /// [`Self::pack_params`] for raw words (counts, packed flags, offsets); read with `param_word(op, i)`.
    #[allow(dead_code)] // Used by the GPU port packages
    fn pack_words(&mut self, words: &[u32]) -> Option<u32> {
        let offset = LOOKUP_WORDS.checked_add(u32::try_from(self.ramp_words.len()).ok()?)?;
        self.ramp_words.extend_from_slice(words);
        Some(offset)
    }

    fn pack_curvature(&mut self, detail: &CurvatureDetail, derive_levels: bool) -> Option<[f32; 2]> {
        let offset = LOOKUP_WORDS.checked_add(u32::try_from(self.ramp_words.len()).ok()?)?;
        if offset >= 1 << 24 || detail.weights.iter().any(|weight| !weight.is_finite()) {
            return None;
        }
        self.ramp_words.extend(detail.weights.map(|weight| weight.clamp(0.0, 1.0).to_bits()));
        for (size, needed) in self.curvature_sizes.iter_mut().zip(detail.blurred_sizes_in_use()) {
            *size |= derive_levels && needed;
        }
        let mode = match detail.mode {
            CurvatureMode::Edges => 1.0,
            CurvatureMode::Cavities => 2.0,
            CurvatureMode::Both => 3.0,
            CurvatureMode::Raw => 4.0,
        };
        Some([offset as f32, mode])
    }
}

#[derive(Debug)]
struct FilterStage {
    layer: u32,
    preceding: u32,
    filter: Option<NeighbourhoodFilter>,
    cell: u32,
    anchor: Option<(Channel, u32, u32)>,
    /// The operation (index plus one, 0: none) the prepare pass evaluates into the field's sixth
    /// word: an image input's gray (P3) or the layer's own Height (P5, Height To Normal).
    input: u32,
    /// The prepare pass reads the set periodically outside it (P4: Blur and Bevel `wrap`), and
    /// the fields of the program may start below zero ([`FilterPlan::wraps`]).
    wrap: bool,
}

#[derive(Debug, Default)]
struct FilterPlan {
    stages: Vec<FilterStage>,
    reach: u32,
    anchor_slots: Vec<u32>,
}

impl FilterPlan {
    fn compile(operations: &mut [Operation], layer_count: u32, inputs: &ProceduralInputs) -> Self {
        if inputs.filters.is_empty() && inputs.generator_stages.is_empty() && inputs.anchors.reads.is_empty() {
            return Self::default();
        }
        let mut plan = Self::default();
        let (layers, effects) = operations.split_at_mut(layer_count as usize);
        let mut masks = BTreeMap::new();
        plan.anchor_slots.resize(inputs.anchors.reads.len(), 0);
        for (index, layer) in layers.iter_mut().enumerate() {
            if layer.ids[1] == ANCHOR_PUBLISH_LAYER {
                plan.add_anchor(layer.ids[2], index as u32, MATERIAL_ANCHOR_STAGE, &inputs.anchors, Some((layer.effects[0], layer.effects[1])));
                continue;
            }
            let [mask, mask_count, content, content_count] = layer.effects;
            let mask_has_fields = mask_count > 0
                && *masks.entry(mask).or_insert_with(|| {
                    let before = plan.stages.len();
                    plan.add_stack(effects, mask, mask_count, index as u32, None, inputs);
                    plan.stages.len() != before
                });
            let first = plan.stages.len();
            plan.add_stack(effects, content, content_count, index as u32, Some(slot_kind(layer.ids[0])), inputs);
            layer.spatial[..3].copy_from_slice(&[first as u32, u32::from(first != plan.stages.len()), u32::from(mask_has_fields)]);
        }
        // An anchor can carry a filter's footprint into the next layer. Independent
        // channels within a layer share a footprint instead of adding to one another.
        if !inputs.anchors.reads.is_empty() {
            let mut draw_reach = BTreeMap::<(u32, bool), u32>::new();
            for stage in &plan.stages {
                let reach = draw_reach.entry((stage.layer, stage.preceding & MASK_STAGE != 0)).or_default();
                *reach =
                    reach.saturating_add(stage.filter.map_or(0, NeighbourhoodFilter::reach)).saturating_add(if stage.cell > 1 { stage.cell * 2 } else { 0 });
            }
            let across_layers = inputs.anchors.draw_ranges.iter().fold(0u32, |reach, range| {
                let layer = draw_reach.range((range.start, false)..(range.end, false)).map(|(_, reach)| *reach).max().unwrap_or(0);
                reach.saturating_add(layer)
            });
            plan.reach = plan.reach.max(across_layers);
        }
        plan
    }

    fn add_anchor(&mut self, publication: u32, layer: u32, preceding: u32, inputs: &AnchorInputs, material: Option<(u32, u32)>) {
        for (read, &(producer, channel)) in inputs.reads.iter().enumerate().filter(|(_, (producer, _))| *producer == publication) {
            debug_assert_eq!(producer, publication);
            self.anchor_slots[read] = self.stages.len() as u32 + 1;
            self.stages.push(FilterStage {
                layer,
                preceding,
                filter: None,
                cell: 1,
                anchor: material.map(|(first, cut)| (channel, first, cut)),
                input: 0,
                wrap: false,
            });
        }
    }

    fn bind_anchors(&self, operations: &mut [Operation]) {
        for operation in operations.iter_mut().filter(|op| op.ids[1] == ANCHOR_SOURCE && op.ids[2] != 0) {
            operation.ids[2] = self.anchor_slots[operation.ids[2] as usize - 1];
        }
    }

    #[allow(clippy::too_many_arguments)]
    /// The stages of one effect stack: a mask's when `channel` is None, else the content of a
    /// channel of that kind (the stage context the packages' stages read).
    fn add_stack(&mut self, effects: &mut [Operation], start: u32, count: u32, layer: u32, channel: Option<ChannelKind>, inputs: &ProceduralInputs) {
        let mask = channel.is_none();
        let first = self.stages.len();
        let mut reach: u32 = 0;
        for (preceding, effect) in effects.iter_mut().skip(start as usize).take(count as usize).enumerate() {
            if mask && effect.ids[1] == ANCHOR_PUBLISH_MASK {
                self.add_anchor(effect.ids[2], layer, preceding as u32 | MASK_STAGE | MASK_ANCHOR_STAGE, &inputs.anchors, None);
                continue;
            }
            if (FIRST_GENERATOR..FIRST_EXTENDED_PATTERN).contains(&effect.ids[1]) && effect.spatial[1] != 0 {
                let (filter, cell) = inputs.generator_stages[effect.spatial[1] as usize - 1];
                effect.spatial[1] = self.stages.len() as u32 + 1;
                reach = reach.saturating_add(filter.map_or(0, NeighbourhoodFilter::reach));
                if cell > 1 {
                    reach = reach.saturating_add(cell * 2);
                }
                self.stages.push(FilterStage {
                    layer,
                    preceding: preceding as u32 | MASK_STAGE | GENERATOR_STAGE,
                    filter,
                    cell,
                    anchor: None,
                    input: 0,
                    wrap: false,
                });
                continue;
            }
            if effect.ids[1] != NEIGHBOURHOOD_OPCODE {
                continue;
            }
            let (filter, opacity) = &inputs.filters[effect.ids[2] as usize];
            let Some(filter) = NeighbourhoodFilter::new(filter, inputs.resolution, inputs.resolution, *opacity, channel) else { continue };
            let slot = self.stages.len();
            effect.spatial[0] = if mask { slot } else { slot - first } as u32;
            let preceding = preceding as u32 | if mask { MASK_STAGE } else { 0 };
            self.stages.push(FilterStage { layer, preceding, filter: Some(filter), cell: 1, anchor: None, input: 0, wrap: false });
            reach = reach.saturating_add(filter.reach());
        }
        self.reach = self.reach.max(reach);
    }

    fn expand(&self, region: Option<CompositeRegion>) -> Option<CompositeRegion> {
        region.map(|region| region.expanded(self.reach))
    }

    fn words(&self, area: u64) -> u64 {
        if self.stages.is_empty() { 0 } else { (self.stages.len() as u64 + 3) * u64::from(self.field_words()) * area }
    }

    /// Words per field texel: four premultiplied components and the UV coverage, plus a sixth
    /// word when a stage carries an input ([`FilterStage::input`]). One stride for every field
    /// of the program; the shaders read it from the uniform, never from a literal.
    /// Whether a stage wraps: the fields then extend by the halo past the set's edges, with an
    /// origin below zero, and the prepare pass fills the wrapping stages periodically there.
    fn wraps(&self) -> bool {
        self.stages.iter().any(|stage| stage.wrap)
    }

    fn field_words(&self) -> u32 {
        if self.stages.iter().any(|stage| stage.input != 0) { FIELD_WORDS + 1 } else { FIELD_WORDS }
    }
}

fn bind_generator_sources(operations: &mut Vec<Operation>, sources: &[Operation]) {
    let offset = operations.len() as u32;
    for op in operations.iter_mut().filter(|op| (FIRST_GENERATOR..FIRST_EXTENDED_PATTERN).contains(&op.ids[1])) {
        for source in &mut op.effects[2..] {
            if *source != 0 {
                *source += offset;
            }
        }
    }
    operations.extend_from_slice(sources);
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

    fn value_for_layer(&self, id: LayerId, channel: Channel) -> Option<([f32; 3], f32)> {
        let LiveBrushTarget::Layer { layer, values } = &self.target else { return None };
        if *layer != id {
            return None;
        }
        values.get(&channel).map(|&value| (value, self.opacity.clamp(0.0, 1.0)))
    }
}

#[derive(Debug)]
pub struct CompositeProgram {
    operations: Vec<Operation>,
    layer_count: u32,
    filters: FilterPlan,
    groups: GroupPlan,
    geometry: GeometryMasks,
    paint: Vec<TileImage>,
    ramp_words: Vec<u32>,
    curvature_sizes: [bool; CurvatureDetail::BLURRED_SIZES],
    tile_patterns: TilePatterns,
    material_images: MaterialImages,
    brush_alpha: Option<wgpu::TextureView>,
    brush_stencil: Option<GpuStencil>,
    pub(crate) region: Option<CompositeRegion>,
    coverage: Option<(Arc<UvGBuffer>, u32)>,
    surface: Option<(Arc<Mesh>, Arc<UvGBuffer>)>,
    needs_surface: bool,
    baked: Option<BakedInputs>,
    id_bake: Option<IdBake>,
    display_outputs: u32,
    glossiness: bool,
    mask_preview: bool,
    mask_layer: u32,
    mask_layers: BTreeMap<LayerId, u32>,
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

/// The kind of the channel in destination slot `slot` ([`channel_slot`]).
fn slot_kind(slot: u32) -> ChannelKind {
    Channel::ALL.into_iter().find(|&channel| channel_slot(channel) == slot).map_or(ChannelKind::Scalar, Channel::kind)
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
        // P1's projector depth grids, before the atlas reads them
        self.material_images.set_surface(mesh);
        if self.needs_surface || !self.geometry.parts.is_empty() {
            self.surface = Some((mesh.clone(), gbuf.clone()));
        }
        self.geometry.resolve(mesh);
        if self.mask_preview || ((!self.filters.stages.is_empty() || !self.geometry.parts.is_empty()) && self.coverage.is_none()) {
            self.set_coverage(gbuf.clone(), 0);
        }
        self.baked = baked.map(BakedInputs::new).transpose()?.flatten();
        self.id_bake = baked.filter(|_| self.operations.iter().any(|op| op.ids[1] == COLOR_SELECTION_OPCODE && op.procedural[2] != 0)).and_then(IdBake::new);
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
        packages::check_set(set, false)?;
        if let Some(brush) = &brush {
            brush.validate(set.size())?;
        }
        let mut operations: Vec<Operation> = Vec::new();
        let mut paint = PaintInputs::default();
        let mut tile_patterns = TilePatterns::default();
        let mut effects = Vec::new();
        let mut procedural = ProceduralInputs::new(set.resolution);
        let mut geometry = GeometryMasks::default();
        let mut groups = GroupPlan::default();
        let mut group_depth = 0;
        let mut mask_layers = BTreeMap::new();
        let mut non_default = [false; Channel::ALL.len()];
        let stencil_pigment = brush.as_ref().is_some_and(|brush| brush.stencil.is_some());
        let mut layers: Vec<_> = set.stack.iter().rev().map(|&id| StackWork::Layer(id, 0, 0)).collect();
        while let Some(work) = layers.pop() {
            let (id, depth, inherited_geometry) = match work {
                StackWork::Layer(id, depth, geometry) => (id, depth, geometry),
                StackWork::EndGroup(start) => {
                    let end = operations.len() as u32 + 1;
                    let weights = operations[start].ids[2];
                    operations[start].ids[3] = end;
                    operations.push(Operation { ids: [0, END_GROUP, weights, 0], ..Operation::zeroed() });
                    group_depth -= 1;
                    continue;
                }
            };
            let own = project.layers.get(&id).ok_or("A layer is missing from the material.")?;
            if !own.visible {
                continue;
            }
            let Some(layer) = project.content_layer(id) else { continue };
            let geometry_mask = geometry.register(inherited_geometry, own.geometry_mask.as_deref());
            let mask = project.layers.get(&project.mask_owner(id)).and_then(|owner| owner.mask.as_ref()).filter(|m| m.enabled);
            if let LayerKind::Group { children, contents, .. } = &layer.kind {
                if depth > texturecraft_core::project::MAX_DEPTH {
                    continue;
                }
                // A folder's Scale and Rotation (an instanced folder's too) re-place every fill inside it
                if !contents.is_none() {
                    return Err("A folder with Scale or Rotation uses CPU compositing.");
                }
                // A full-weight group sees the incoming stack and returns it exactly
                // as its children changed it. A geometry restriction must stay on the
                // group boundary when children publish anchors outside that restriction.
                if mask.is_none()
                    && set.channels.iter().all(|&(channel, _)| {
                        layer.blend(channel).is_some_and(|blend| blend.mode != BlendMode::Disable && blend.opacity.is_finite() && blend.opacity >= 1.0)
                    })
                    && (geometry_mask == 0 || !groups::children_publish_anchors(project, children, depth + 1))
                {
                    layers.extend(children.iter().rev().map(|&child| StackWork::Layer(child, depth + 1, geometry_mask)));
                    continue;
                }
                let mut weights = [0.0f32; Channel::ALL.len()];
                for &(channel, _) in &set.channels {
                    if let Some(blend) = layer.blend(channel).filter(|blend| blend.mode != BlendMode::Disable) {
                        if !blend.opacity.is_finite() {
                            return Err("A group has an invalid opacity.");
                        }
                        weights[channel_slot(channel) as usize] = blend.opacity.clamp(0.0, 1.0);
                    }
                }
                if weights.iter().all(|&opacity| opacity == 0.0)
                    && mask.is_none_or(|mask| !mask.effects.iter().any(|effect| effect.visible && matches!(effect.kind, EffectKind::Anchor)))
                {
                    continue;
                }
                let begin = effects.len() as u32;
                let background = compile_mask(mask, pixels, brush.as_ref(), (&mut paint, &mut procedural, &mut tile_patterns), &mut effects)?;
                let count = effects.len() as u32 - begin;
                let height = layer.height_mask;
                let height_params = if height.enabled && count != 0 {
                    if !height.contrast.is_finite() || !height.offset.is_finite() {
                        return Err("A group's height mask has invalid parameters.");
                    }
                    [height.contrast.to_bits(), height.offset.to_bits()]
                } else {
                    [0; 2]
                };
                let weight_offset = u32::try_from(procedural.ramp_words.len())
                    .ok()
                    .and_then(|len| LOOKUP_WORDS.checked_add(len))
                    .ok_or("The GPU group settings are too large.")?;
                procedural.ramp_words.extend(weights.map(f32::to_bits));
                let open = operations.len();
                let mut op = Operation {
                    ids: [0, BEGIN_GROUP, weight_offset, 0],
                    effects: [begin, count, 0, 0],
                    constraints: [geometry_mask, 0, height_params[0], height_params[1]],
                    ..Operation::zeroed()
                };
                op.balance[1] = background;
                operations.push(op);
                mask_layers.insert(id, open as u32);
                procedural.anchors.draw_ranges.push(open as u32..open as u32 + 1);
                group_depth += 1;
                groups.depth = groups.depth.max(group_depth);
                layers.push(StackWork::EndGroup(open));
                // The group tests its geometry before entering. Children still apply their own masks.
                layers.extend(children.iter().rev().map(|&child| StackWork::Layer(child, depth + 1, 0)));
                continue;
            }
            let mask_start = effects.len() as u32;
            let mask_background = compile_mask(mask, pixels, brush.as_ref(), (&mut paint, &mut procedural, &mut tile_patterns), &mut effects)?;
            let shared_content = effects.len() as u32;
            let mask_count = shared_content - mask_start;
            let has_content_anchors = layer.content_effects.iter().any(|effect| effect.visible && matches!(effect.kind, EffectKind::Anchor));
            if mask_background == 0.0 && mask_count == 0 && !has_content_anchors {
                continue;
            }
            let mut content_anchors = Vec::new();
            let mut visible_cut = 0u32;
            let per_channel = layer.content_effects.iter().any(|effect| effect.visible && content_depends_on_channel(effect));
            let mut templates = Vec::new();
            for effect in layer.content_effects.iter().filter(|e| e.visible) {
                if let Some(reason) = effect_needs_cpu(effect, true) {
                    return Err(reason);
                }
                if matches!(effect.kind, EffectKind::Anchor) {
                    content_anchors.push((effect.id, visible_cut));
                    continue;
                }
                visible_cut += 1;
                let op = if content_depends_on_channel(effect) {
                    None
                } else {
                    Some(content_filter(effect, &mut procedural).ok_or("A layer effect uses CPU compositing.")?)
                };
                // A lowering with a stage before its operation takes two operations
                let stage = procedural.preceding_stage.take();
                visible_cut += u32::from(stage.is_some());
                if per_channel {
                    templates.push((effect, stage, op));
                } else if let Some(op) = op {
                    effects.extend(stage);
                    effects.push(op);
                }
            }
            let fill = match &layer.kind {
                LayerKind::Fill(fill) => Some(fill),
                LayerKind::Paint => None,
                _ => return Err("A group or another layer type uses CPU compositing."),
            };
            let decal = fill.map(|fill| register_decal(fill, &mut procedural.materials)).transpose()?.flatten();

            let height = layer.height_mask;
            let height_params = if height.enabled && mask_count != 0 {
                if !height.contrast.is_finite() || !height.offset.is_finite() {
                    return Err("A layer's height mask has invalid parameters.");
                }
                [height.contrast.to_bits(), height.offset.to_bits()]
            } else {
                [0; 2]
            };
            let first_op = operations.len();
            let mut height_op = 0;
            for &(channel, _) in &set.channels {
                if mask_background == 0.0 && mask_count == 0 {
                    break;
                }
                let Some(blend) = layer.blend(channel) else { continue };
                if blend.mode == BlendMode::Disable || blend.opacity <= 0.0 {
                    continue;
                }
                if !blend.opacity.is_finite() {
                    return Err("A layer has an invalid opacity.");
                }
                let (painted, brush_value) = if fill.is_none() {
                    (sparse_channel(pixels, layer.id, channel)?, brush.as_ref().and_then(|brush| brush.value_for_layer(layer.id, channel)))
                } else {
                    (None, None)
                };
                let content_start = if per_channel { effects.len() as u32 } else { shared_content };
                let mut adds_coverage = false;
                for &(effect, stage, template) in &templates {
                    effects.extend(stage);
                    let op = if let Some(op) = template {
                        op
                    } else if let Some(op) = content_deposit(effect, channel, pixels, brush.as_ref(), (&mut paint, &mut procedural, &mut tile_patterns))? {
                        adds_coverage = true;
                        op
                    } else {
                        Operation { ids: [0, NOOP_EFFECT_OPCODE, 0, 0], ..Operation::zeroed() }
                    };
                    effects.push(op);
                }
                if fill.is_none() && painted.is_none() && brush_value.is_none() && !adds_coverage {
                    if per_channel {
                        effects.truncate(content_start as usize);
                    }
                    continue;
                }
                let content_count = effects.len() as u32 - content_start;
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
                    content_paint(layer.id, channel, painted, brush_value, stencil_pigment, &mut paint)
                } else {
                    fill.and_then(|fill| source_operation(source, &fill.projection, fill.sampling, default, channel.kind(), &mut procedural))
                        .ok_or("A fill source or projection uses CPU compositing.")?
                };
                if let Some(decal) = decal {
                    op.uv[0] = decal as f32;
                }
                bind_tile_source(&mut op, source, &mut tile_patterns);
                op.ids[0] = slot;
                op.low[3] = blend.opacity.min(1.0);
                op.balance[1] = mask_background;
                op.balance[2] = blend_code(blend.mode) as f32;
                op.effects = [mask_start, mask_count, content_start, content_count];
                op.spatial[3] = u32::from(adds_coverage);
                if height_params[0] != 0 && channel == Channel::Height {
                    height_op = operations.len() as u32 + 1;
                }
                operations.push(op);
            }
            packages::order_layer_operations(&mut operations[first_op..]);
            if height_op != 0 {
                // The reorder can move the Height operation that the height mask reads
                let height = operations[first_op..].iter().position(|op| op.ids[0] == channel_slot(Channel::Height));
                height_op = height.map_or(0, |index| (first_op + index) as u32 + 1);
            }
            for op in &mut operations[first_op..] {
                op.constraints = [geometry_mask, height_op, height_params[0], height_params[1]];
            }
            if first_op != operations.len() {
                mask_layers.insert(id, first_op as u32);
            } else if mask.as_ref().is_some_and(|mask| {
                mask.effects.iter().any(|effect| effect.visible && (!effect.kind.anchors_read().is_empty() || matches!(effect.kind, EffectKind::Anchor)))
            }) {
                // A layer with no display channels can still publish anchors in its mask.
                mask_layers.insert(id, first_op as u32);
                operations.push(Operation {
                    ids: [u32::MAX, ANCHOR_PUBLISH_MASK, 0, 0],
                    effects: [mask_start, mask_count, 0, 0],
                    balance: [0.0, mask_background, 0.0, 0.0],
                    constraints: [geometry_mask, 0, 0, 0],
                    ..Operation::zeroed()
                });
            }
            procedural.anchors.draw_ranges.push(first_op as u32..operations.len() as u32);
            if !content_anchors.is_empty() {
                let has_channels =
                    set.channels.iter().any(|&(channel, _)| layer.blend(channel).is_some_and(|blend| blend.mode != BlendMode::Disable && blend.opacity > 0.0));
                if !has_channels && mask.is_none_or(|mask| !mask.effects.iter().any(|effect| effect.visible && matches!(effect.kind, EffectKind::Anchor))) {
                    continue;
                }
            }
            for (anchor, cut) in content_anchors {
                let publication = procedural.anchors.publish(anchor, false);
                operations.push(Operation {
                    ids: [u32::MAX, ANCHOR_PUBLISH_LAYER, publication, 0],
                    effects: [first_op as u32, cut, 0, 0],
                    ..Operation::zeroed()
                });
            }
        }
        let mut display_outputs = display_bit(DisplayKind::BaseColor) | display_bit(DisplayKind::Orm);
        for channel in Channel::ALL {
            let slot = channel_slot(channel);
            if operations.iter().any(|op| {
                !groups::is_group(op)
                    && op.ids[0] == slot
                    && (op.ids[1] != 0 || op.low[..3] != channel.default_value() || op.effects[3] != 0 || op.balance[2] != 0.0)
            }) {
                display_outputs |= display_bit(DisplayKind::of_channel(channel));
            }
        }
        groups.finish(&operations);
        let layer_count = operations.len() as u32;
        if !operations.iter().any(|op| op.constraints[0] != 0) {
            geometry.parts.clear();
        }
        operations.extend(effects);
        let filters = FilterPlan::compile(&mut operations, layer_count, &procedural);
        bind_generator_sources(&mut operations, &procedural.generator_sources);
        filters.bind_anchors(&mut operations);
        let region = filters.expand(region);
        let (brush_alpha, brush_stencil) = brush.map_or((None, None), |brush| (Some(brush.alpha), brush.stencil));
        let needs_surface = brush_stencil.is_some() || needs_surface(&operations);
        Ok(Self {
            operations,
            layer_count,
            filters,
            groups,
            geometry,
            paint: paint.images,
            ramp_words: procedural.ramp_words,
            curvature_sizes: procedural.curvature_sizes,
            tile_patterns,
            material_images: procedural.materials,
            brush_alpha,
            brush_stencil,
            region,
            coverage: None,
            surface: None,
            needs_surface,
            baked: None,
            id_bake: None,
            display_outputs,
            glossiness: set.has_channel(Channel::Glossiness),
            mask_preview: false,
            mask_layer: 0,
            mask_layers,
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
        packages::check_set(set, true)?;
        // Folders around the layer (an instance's own, as the CPU places it, or its source's) re-place its mask fills
        let requested = owner;
        let owner = project.mask_owner(owner);
        if !project.contents_around(requested).is_none() || !project.contents_around(owner).is_none() {
            return Err("A mask inside a folder with Scale or Rotation uses CPU mask preview.");
        }
        let mask = project.layers.get(&owner).and_then(|layer| layer.mask.as_ref()).filter(|mask| mask.enabled);
        let mask = mask.ok_or("The layer has no enabled mask.")?;
        if mask.effects.iter().any(|effect| effect.visible && !effect.kind.anchors_read().is_empty()) {
            let mut program = Self::try_compile_with_brush(project, pixels, ts, region, brush)?;
            program.mask_layer = *program.mask_layers.get(&owner).ok_or("The mask's GPU anchor stack is missing.")?;
            program.mask_preview = true;
            program.display_outputs = display_bit(DisplayKind::BaseColor);
            return Ok(program);
        }
        if let Some(brush) = &brush {
            brush.validate(set.size())?;
        }
        let mut operations = vec![Operation::zeroed()];
        let mut paint = PaintInputs::default();
        let mut procedural = ProceduralInputs::new(set.resolution);
        let mut tile_patterns = TilePatterns::default();
        let background = compile_mask(Some(mask), pixels, brush.as_ref(), (&mut paint, &mut procedural, &mut tile_patterns), &mut operations)?;
        operations[0].effects[1] = operations.len() as u32 - 1;
        operations[0].balance[1] = background;
        let filters = FilterPlan::compile(&mut operations, 1, &procedural);
        bind_generator_sources(&mut operations, &procedural.generator_sources);
        filters.bind_anchors(&mut operations);
        let region = filters.expand(region);
        let (brush_alpha, brush_stencil) = brush.map_or((None, None), |brush| (Some(brush.alpha), brush.stencil));
        let needs_surface = brush_stencil.is_some() || needs_surface(&operations);
        Ok(Self {
            operations,
            layer_count: 1,
            filters,
            groups: GroupPlan::default(),
            geometry: GeometryMasks::default(),
            paint: paint.images,
            ramp_words: procedural.ramp_words,
            curvature_sizes: procedural.curvature_sizes,
            tile_patterns,
            material_images: procedural.materials,
            brush_alpha,
            brush_stencil,
            region,
            coverage: None,
            surface: None,
            needs_surface,
            baked: None,
            id_bake: None,
            // Reuse the first four-byte display slot for linear grayscale.
            display_outputs: display_bit(DisplayKind::BaseColor),
            glossiness: false,
            mask_preview: true,
            mask_layer: 0,
            mask_layers: BTreeMap::new(),
        })
    }
}

/// Filter library ids that still composite on the CPU with their library defaults, as
/// (package, id). The GPU port's library gate (crates/ui-egui/src/gpu_port_gate.rs) checks this
/// against the compiler; at the end of the port it is empty.
#[doc(hidden)]
pub fn library_filters_still_on_cpu() -> Vec<(&'static str, &'static str)> {
    packages::library_still_on_cpu()
}

fn needs_surface(operations: &[Operation]) -> bool {
    operations.iter().any(|op| {
        op.procedural[3] != 0
            || op.ids[1] == BAKED_LIGHTING_OPCODE
            || (FIRST_GENERATOR..FIRST_EXTENDED_PATTERN).contains(&op.ids[1])
            || packages::needs_surface(op)
    })
}

fn sparse_channel(pixels: &PixelStore, id: LayerId, channel: Channel) -> Result<Option<&TileImage>, &'static str> {
    let image = pixels.layer(id, channel).filter(|image| image.allocated_count() > 0);
    if image.is_some_and(|image| image.comps() != channel.components() + 1) {
        return Err("A paint layer has a layout that uses CPU compositing.");
    }
    Ok(image)
}

fn content_paint(
    id: LayerId,
    channel: Channel,
    image: Option<&TileImage>,
    brush_value: Option<([f32; 3], f32)>,
    stencil_pigment: bool,
    paint: &mut PaintInputs,
) -> Operation {
    let image_index = image.map_or(NO_PAINT_IMAGE, |image| paint.register(id, Some(channel), image));
    let mut op = Operation { ids: [channel_slot(channel), 10, image_index, (channel.components() + 1) as u32], ..Operation::zeroed() };
    if let Some(([r, g, b], opacity)) = brush_value {
        op.projection[3] = if stencil_pigment && channel == Channel::BaseColor { 2.0 } else { 1.0 };
        op.uv = [r, g, b, opacity];
    }
    op
}

fn content_depends_on_channel(effect: &Effect) -> bool {
    matches!(effect.kind, EffectKind::Material(_) | EffectKind::Paint | EffectKind::Path(_))
}

fn content_filter(effect: &Effect, procedural: &mut ProceduralInputs) -> Option<Operation> {
    let noop = Operation { ids: [0, NOOP_EFFECT_OPCODE, 0, 0], ..Operation::zeroed() };
    match &effect.kind {
        // A mask-only effect authored in this stack leaves its content alone.
        EffectKind::Fill { .. } | EffectKind::Generator(_) | EffectKind::ColorSelection(_) | EffectKind::Compare { .. } => Some(noop),
        // Levels limited to one channel would need a program per channel
        EffectKind::Levels(levels) if levels.affects.is_some() => None,
        // A blending effect set to Disable leaves the content as it is
        kind if kind.blends() && effect.blend == BlendMode::Disable && uniform_blend(effect) => Some(noop),
        _ => point_effect(effect, procedural),
    }
}

/// Whether the effect blends every channel alike: the GPU program reads one blend mode and opacity per effect
fn uniform_blend(effect: &Effect) -> bool {
    effect.channel_blends.is_empty() && effect.off_channels.is_empty()
}

/// Settings of a content effect the GPU program does not model: per-channel blends, channels
/// switched off, and image inputs its filter reads. A mask ignores per-channel blends (it blends
/// by [`Effect::blend_of`] with no channel), so a mask effect is checked with `content` false.
fn effect_needs_cpu(effect: &Effect, content: bool) -> Option<&'static str> {
    if content && !uniform_blend(effect) {
        return Some("An effect with per-channel blending uses CPU compositing.");
    }
    if reads_image_input(effect) {
        return Some("An effect with an image input uses CPU compositing.");
    }
    None
}

/// Whether the effect's filter reads one of the image inputs set on it now (inputs a filter it
/// was swapped from left behind, or that its mode does not read, change nothing)
fn reads_image_input(effect: &Effect) -> bool {
    let EffectKind::Filter(filter) = &effect.kind else { return false };
    texturecraft_core::ImageInput::of(filter).iter().any(|input| effect.inputs.contains_key(input.id))
}

fn content_deposit(
    effect: &Effect,
    channel: Channel,
    pixels: &PixelStore,
    brush: Option<&LiveBrush>,
    (paint, procedural, tile_patterns): (&mut PaintInputs, &mut ProceduralInputs, &mut TilePatterns),
) -> Result<Option<Operation>, &'static str> {
    if !effect.opacity.is_finite() {
        return Err("A content effect has an invalid opacity.");
    }
    // A deposit set to Disable leaves the content and its coverage as they are
    if effect.opacity <= 0.0 || effect.blend == BlendMode::Disable {
        return Ok(None);
    }
    let mut op = if let EffectKind::Material(fill) = &effect.kind {
        let Some(source) = fill.sources.get(&channel) else { return Ok(None) };
        let mut op = source_operation(Some(source), &fill.projection, fill.sampling, channel.default_value(), channel.kind(), procedural)
            .ok_or("A content material source or projection uses CPU compositing.")?;
        if let Some(decal) = register_decal(fill, &mut procedural.materials)? {
            op.uv[0] = decal as f32;
        }
        bind_tile_source(&mut op, Some(source), tile_patterns);
        op.ids[0] = channel_slot(channel);
        op
    } else {
        let image = sparse_channel(pixels, effect.id, channel)?;
        let value = brush.and_then(|brush| brush.value_for_layer(effect.id, channel));
        if image.is_none() && value.is_none() {
            return Ok(None);
        }
        content_paint(effect.id, channel, image, value, brush.is_some_and(|brush| brush.stencil.is_some()), paint)
    };
    op.low[3] = effect.opacity.min(1.0);
    op.balance[2] = blend_code(effect.blend) as f32;
    Ok(Some(op))
}

fn compile_mask(
    mask: Option<&Mask>,
    pixels: &PixelStore,
    brush: Option<&LiveBrush>,
    (paint, procedural, tile_patterns): (&mut PaintInputs, &mut ProceduralInputs, &mut TilePatterns),
    effects: &mut Vec<Operation>,
) -> Result<f32, &'static str> {
    let Some(mask) = mask else { return Ok(1.0) };
    for effect in mask.effects.iter().filter(|e| e.visible) {
        if let Some(reason) = effect_needs_cpu(effect, false) {
            return Err(reason);
        }
        let mut op = match &effect.kind {
            EffectKind::Fill { source, projection, sampling } => {
                let mut operation = source_operation(Some(source), projection, *sampling, [0.0; 3], ChannelKind::Scalar, procedural)
                    .ok_or("A mask fill source or projection uses CPU compositing.")?;
                if let Some(frame) = projection.frame() {
                    operation.uv[0] = procedural.materials.register_decal(*frame, std::iter::once(source), *sampling) as f32;
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
                if image.is_some_and(|image| image.comps() != 2) {
                    return Err("A painted mask has a layout that uses CPU compositing.");
                }
                let image_index = image.map_or(NO_PAINT_IMAGE, |image| paint.register(effect.id, None, image));
                let mut op = Operation { ids: [0, 10, image_index, 2], ..Operation::zeroed() };
                if let Some((value, opacity)) = live {
                    op.projection[3] = 1.0;
                    op.uv = [value, value, value, opacity];
                }
                op
            }
            EffectKind::Generator(generator) => generator_operation(generator, procedural, tile_patterns).ok_or("A mask generator uses CPU compositing.")?,
            EffectKind::ColorSelection(selection) => {
                // The GPU selects hard, on black, from the baked ID map
                let ColorSelection { colors, tolerance, output, hardness, background, id_map } = selection;
                if *output != 1.0 || *hardness != 1.0 || *background != texturecraft_core::SelectionBackground::Black || id_map.is_some() {
                    return Err("A color selection's output, hardness, background or ID map uses CPU compositing.");
                }
                let [offset, count] = procedural.pack_rgb(colors).ok_or("A color selection has invalid colors.")?;
                if !tolerance.is_finite() {
                    return Err("A color selection has invalid tolerance.");
                }
                Operation { ids: [0, COLOR_SELECTION_OPCODE, 0, 0], low: [*tolerance, 0.0, 0.0, 0.0], procedural: [0, offset, count, 0], ..Operation::zeroed() }
            }
            EffectKind::Anchor => {
                let publication = procedural.anchors.publish(effect.id, true);
                Operation { ids: [0, ANCHOR_PUBLISH_MASK, publication, 0], ..Operation::zeroed() }
            }
            EffectKind::Compare { op, value, tolerance, left, right, channel: _, hardness } => {
                // The GPU compares the mask so far with a constant, hard; the channel is read only by layer sides
                if *left != CompareSource::ThisMask || *right != CompareSource::Constant || *hardness != 1.0 {
                    return Err("A mask comparison reading layers or with a soft edge uses CPU compositing.");
                }
                if !value.is_finite() || !tolerance.is_finite() {
                    return Err("A mask comparison has invalid parameters.");
                }
                let compare = match op {
                    CompareOp::LesserThan => 0,
                    CompareOp::WithinTolerance => 1,
                    CompareOp::GreaterThan => 2,
                };
                Operation { ids: [0, COMPARE_OPCODE, compare, 0], low: [*value, *tolerance, 0.0, 0.0], ..Operation::zeroed() }
            }
            // Lighting reads the base color, which a mask does not have
            EffectKind::Filter(Filter::BakedLighting { .. } | Filter::EnvironmentLighting { .. }) => continue,
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
        // A lowering with a stage before its operation (`ProceduralInputs::precede_with_stage`)
        effects.extend(procedural.preceding_stage.take());
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

/// The frame record of a placed planar or warp fill (its first image's alpha also controls the
/// non-image channels), as `op.uv[0]` of its operations.
fn register_decal(fill: &texturecraft_core::Fill, materials: &mut MaterialImages) -> Result<Option<u32>, &'static str> {
    let Some(frame) = fill.projection.frame() else { return Ok(None) };
    Ok(Some(materials.register_decal(*frame, fill.sources.values(), fill.sampling)))
}

fn source_operation(
    source: Option<&Source>,
    projection: &Projection,
    sampling: Sampling,
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
            op.ids = [0, IMAGE_SOURCE_OPCODE, inputs.materials.register_image(name, fill_projection::image_use(sampling, channel)), kind];
            fill_projection::place_image(projection, sampling, &mut op, inputs)?;
        }
        Some(Source::Anchor { anchor, channel: read_channel, ramp }) => {
            let (reference, is_mask) = inputs.anchors.read(*anchor, *read_channel);
            let gray =
                matches!(read_channel.kind(), ChannelKind::Color | ChannelKind::NormalMap) && matches!(channel, ChannelKind::Scalar | ChannelKind::Signed);
            op.ids = [0, ANCHOR_SOURCE, reference, u32::from(gray) | (u32::from(read_channel.kind() == ChannelKind::Signed && !is_mask) << 1)];
            op.low[..3].copy_from_slice(&read_channel.default_value());
            if channel == ChannelKind::Color && !ramp.is_empty() {
                op.procedural[1..3].copy_from_slice(&inputs.pack_rgb(ramp)?);
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
                op.procedural[1..3].copy_from_slice(&inputs.pack_rgb(ramp)?);
            }
            fill_projection::place_procedural(projection, sampling, &mut op, inputs)?;
        }
    }
    // Coverage of every source kind (wrap, crop, backface, depth culling, decals)
    fill_projection::place_cover(projection, sampling, &mut op, inputs)?;
    op.low.iter().chain(&op.high).chain(&op.projection).chain(&op.uv).chain(&op.balance).all(|v| v.is_finite()).then_some(op)
}

fn generator_operation(generator: &Generator, inputs: &mut ProceduralInputs, patterns: &mut TilePatterns) -> Option<Operation> {
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
    let mut op = Operation {
        ids: [0, kind, generator.seed, u32::from(generator.invert)],
        low: [generator.level, generator.softness.max(Generator::MIN_SOFTNESS), generator.noise_amount, 0.0],
        high: [direction.x, direction.y, direction.z, 0.0],
        projection: [generator.noise_scale.max(1e-3), generator.offset, generator.global.balance, generator.global.contrast],
        ..Operation::zeroed()
    };
    if let Some(source) = generator.kind.inputs().first().and_then(|map| generator.inputs.get(map)) {
        let mut source_op = source_operation(Some(source), &Projection::default(), Sampling::SAVED_BEFORE, [0.0; 3], ChannelKind::Scalar, inputs)?;
        bind_tile_source(&mut source_op, Some(source), patterns);
        inputs.generator_sources.push(source_op);
        op.effects[3] = inputs.generator_sources.len() as u32;
    }
    if let Some(micro) = generator.micro {
        let source = Source::Anchor { anchor: micro.anchor, channel: Channel::Height, ramp: Vec::new() };
        let source = source_operation(Some(&source), &Projection::default(), Sampling::SAVED_BEFORE, [0.0; 3], ChannelKind::Signed, inputs)?;
        inputs.generator_sources.push(source);
        op.effects[2] = inputs.generator_sources.len() as u32;
        op.uv[2] = if micro.amount.is_finite() { micro.amount.clamp(0.0, 1.0) } else { 0.0 };
    }
    let resolution = inputs.resolution;
    let cell = generator.resolution.cell(resolution, resolution);
    op.spatial[2] = u32::from(generator.global.blur > 0.0 || generator.resolution.can_reduce());
    let blur = (generator.global.blur_radius_at(resolution).round() > 0.0)
        .then(|| Filter::Blur { radius: generator.global.blur_radius(), wrap: false })
        .and_then(|blur| NeighbourhoodFilter::new(&blur, resolution, resolution, 1.0, None));
    if let (Some(_), _) | (None, 2..) = (blur, cell) {
        inputs.generator_stages.push((blur, cell));
        op.spatial[1] = inputs.generator_stages.len() as u32;
    }
    if matches!(generator.kind, GeneratorKind::Curvature | GeneratorKind::EdgeWear)
        && let Some(detail) = &generator.detail
    {
        op.uv[..2].copy_from_slice(&inputs.pack_curvature(detail, op.effects[3] == 0)?);
    }
    op.low.iter().chain(&op.projection).all(|x| x.is_finite()).then_some(op)
}

/// One effect as a GPU point operation, or a neighbourhood stage. None for any kind or setting
/// the GPU program does not evaluate as the CPU does, so the stack composites on the CPU.
fn point_effect(effect: &Effect, inputs: &mut ProceduralInputs) -> Option<Operation> {
    use Filter::*;
    // Per-channel blends are checked by the content caller; a mask ignores them
    if !effect.opacity.is_finite() || reads_image_input(effect) {
        return None;
    }
    let mut op = Operation::zeroed();
    let (kind, params) = match &effect.kind {
        EffectKind::Levels(levels) => {
            // Unclamped Levels keep values outside the input range on the line through it
            op.high[..3].copy_from_slice(&[levels.out_low, levels.out_high, f32::from(u8::from(levels.clamp))]);
            (20, [levels.in_low, levels.in_high, levels.gamma])
        }
        EffectKind::Filter(filter) => match filter {
            Invert => (21, [0.0; 3]),
            Threshold { level, mode } => {
                // The comparison in ids.z: 0 at or above (projects saved before the modes), 1 above, 2 below, 3 at or below
                op.ids[2] = match mode {
                    ThresholdMode::GreaterOrEqual => 0,
                    ThresholdMode::Greater => 1,
                    ThresholdMode::Lower => 2,
                    ThresholdMode::LowerOrEqual => 3,
                };
                (22, [*level, 0.0, 0.0])
            }
            ContrastLuminosity { contrast, luminosity } => (23, [*contrast, *luminosity, 0.0]),
            Posterize { levels, alpha: false } => (24, [*levels, 0.0, 0.0]),
            Clamp { min, max } => (25, [min.min(*max), min.max(*max), 0.0]),
            HistogramScan { position, contrast, invert_position: false } => (26, [*position, *contrast, 0.0]),
            HistogramRange { range, position } => (27, [*range, *position, 0.0]),
            HistogramShift { offset } => (28, [*offset, 0.0, 0.0]),
            Grayscale => (29, [0.0; 3]),
            NormalInvert => (30, [0.0; 3]),
            Hsl { hue, saturation, lightness, perceptive: false } => (31, [*hue, *saturation, *lightness]),
            ColorBalance { shadows, midtones, highlights } => {
                op.high[..3].copy_from_slice(midtones);
                op.projection[..3].copy_from_slice(highlights);
                (32, *shadows)
            }
            // An evenly spaced linear ramp read from the plain gray
            GradientMap { colors, positions, quantity: None, input, linear: true, debug: false } if positions.is_empty() && input.is_unchanged() => {
                op.procedural[1..3].copy_from_slice(&inputs.pack_rgb(colors)?);
                (33, [0.0; 3])
            }
            ReplaceColor { from, to, tolerance, softness } => {
                op.high[..3].copy_from_slice(to);
                op.projection[..2].copy_from_slice(&[tolerance.clamp(0.0, 1.0), softness.clamp(0.0, 1.0)]);
                (34, *from)
            }
            BakedLighting { azimuth, elevation, light, ambient, occlusion, stylized: None } => {
                let (sa, ca) = azimuth.to_radians().sin_cos();
                let (se, ce) = elevation.to_radians().sin_cos();
                op.high[..3].copy_from_slice(&[ce * sa, se, ce * ca]);
                (BAKED_LIGHTING_OPCODE, [*light, *ambient, occlusion.clamp(0.0, 1.0)])
            }
            // Invert switched off and a filter not chosen yet change nothing
            InvertOff | Empty => (NOOP_EFFECT_OPCODE, [0.0; 3]),
            filter if gpu_neighbourhood::supports(filter) => {
                // A known stage whose size reaches nothing leaves the field as the CPU does
                let spatial = NeighbourhoodFilter::new(filter, inputs.resolution, inputs.resolution, effect.opacity, None);
                if spatial.is_some() {
                    op.ids[2] = inputs.filters.len() as u32;
                    inputs.filters.push((filter.clone(), effect.opacity));
                }
                (if spatial.is_some() { NEIGHBOURHOOD_OPCODE } else { NOOP_EFFECT_OPCODE }, [0.0; 3])
            }
            // Every other filter, and new settings of the ones above: a GPU port package lowers it, or the stack composites on the CPU
            filter => packages::lower_filter(filter, effect, &mut op, inputs)?,
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
    vertices: Option<wgpu::Buffer>,
}

struct BakeCache {
    size: (u32, u32),
    maps: [Option<Weak<Vec<f32>>>; BAKE_MAPS.len()],
    view: wgpu::TextureView,
}

struct IdBakeCache {
    size: [u32; 2],
    data: Weak<Vec<f32>>,
    view: wgpu::TextureView,
}

struct CurvatureCache {
    gbuf: Weak<UvGBuffer>,
    map: Weak<Vec<f32>>,
    size: [u32; 2],
    needed: [bool; CurvatureDetail::BLURRED_SIZES],
    levels: CurvatureLevels,
}

struct CurvatureResources {
    gpu: Option<CurvatureGpu>,
    empty: CurvatureLevels,
    cache: BTreeMap<usize, CurvatureCache>,
}

pub(crate) struct GpuCompositor {
    pipelines: CompositePipelines,
    bindings: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    filter_gpu: Option<NeighbourhoodGpu>,
    lookup: wgpu::Buffer,
    uniform_coverage: wgpu::Buffer,
    coverage: Vec<(Weak<UvGBuffer>, wgpu::Buffer)>,
    surfaces: Vec<SurfaceCache>,
    baked: BTreeMap<usize, BakeCache>,
    id_bakes: BTreeMap<usize, IdBakeCache>,
    curvature: Option<CurvatureResources>,
    default_surface: wgpu::TextureView,
    default_vertices: wgpu::Buffer,
    default_baked: wgpu::TextureView,
    default_id_bake: Option<wgpu::TextureView>,
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
        let limits = device.limits();
        let supports_curvature = limits.max_sampled_textures_per_shader_stage >= 6
            && limits.max_uniform_buffers_per_shader_stage >= 3
            && limits.max_bindings_per_bind_group >= 15
            && limits.max_storage_textures_per_shader_stage >= 1
            && limits.max_compute_workgroup_size_x >= 64;
        let supports_id = limits.max_sampled_textures_per_shader_stage >= 6 + u32::from(supports_curvature) && limits.max_bindings_per_bind_group >= 16;
        let mut entries = vec![
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
        ];
        if supports_curvature {
            entries.extend([binding(13, uint), binding(14, uniform)]);
        }
        if supports_id {
            entries.push(binding(15, uint));
        }
        let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("tc_gpu_composite_bindings"), entries: &entries });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tc_gpu_composite_layout"),
            bind_group_layouts: &[Some(&bindings)],
            immediate_size: 0,
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
            pipelines: CompositePipelines::new(supports_curvature, supports_id),
            bindings,
            pipeline_layout,
            filter_gpu: None,
            lookup,
            uniform_coverage,
            coverage: Vec::new(),
            surfaces: Vec::new(),
            baked: BTreeMap::new(),
            id_bakes: BTreeMap::new(),
            curvature: supports_curvature.then(|| CurvatureResources { gpu: None, empty: CurvatureLevels::empty(device), cache: BTreeMap::new() }),
            default_surface,
            default_vertices,
            default_baked,
            default_id_bake: supports_id
                .then(|| input_texture(device, "tc_composite_empty_id_bake", (1, 1), 1, wgpu::TextureFormat::Rgba32Uint).create_view(&Default::default())),
            default_brush_alpha,
            default_brush_stencil: GpuStencil::empty(device),
            default_tile_images: TileImageAtlas::empty_view(device),
            tile_images: BTreeMap::new(),
            paint_upload: None,
        }
    }

    fn surface_inputs(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, program: &CompositeProgram) -> Result<(wgpu::TextureView, wgpu::Buffer), String> {
        let Some((mesh, gbuf)) = &program.surface else {
            if program.needs_surface || !program.geometry.parts.is_empty() {
                return Err("Missing UV surface inputs for GPU layer composition".into());
            }
            return Ok((self.default_surface.clone(), self.default_vertices.clone()));
        };
        if program.needs_surface {
            return self.uv_surface(device, queue, mesh, gbuf);
        }

        // A geometry-only mask needs triangle IDs but no barycentrics or surface vertices.
        self.release_unused_surfaces();
        let key = Arc::downgrade(gbuf);
        if let Some(cache) = self.surfaces.iter().find(|cache| cache.gbuf.ptr_eq(&key)) {
            return Ok((cache.view.clone(), self.default_vertices.clone()));
        }
        let size = (gbuf.width, gbuf.height);
        if size.0.max(size.1) > device.limits().max_texture_dimension_2d {
            return Err("UV triangle data exceeds the GPU limits".into());
        }
        let texture = input_texture(device, "tc_composite_uv_triangles", size, 1, wgpu::TextureFormat::R32Uint);
        for (band, triangles) in gbuf.tri.chunks((size.0 * ROWS) as usize).enumerate() {
            write_input(queue, &texture, (size.0, triangles.len() as u32 / size.0), (band as u32 * ROWS, 0), triangles);
        }
        let view = texture.create_view(&Default::default());
        self.surfaces.push(SurfaceCache { gbuf: key, view: view.clone(), vertices: None });
        Ok((view, self.default_vertices.clone()))
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
        if let Some((view, vertices)) =
            self.surfaces.iter().find(|cache| cache.gbuf.ptr_eq(&key)).and_then(|cache| cache.vertices.as_ref().map(|vertices| (&cache.view, vertices)))
        {
            return Ok((view.clone(), vertices.clone()));
        }
        let limits = device.limits();
        let vertex_bytes = mesh.indices.len() as u64 * 32;
        if gbuf.width.max(gbuf.height) > limits.max_texture_dimension_2d || vertex_bytes > limits.max_storage_buffer_binding_size.min(limits.max_buffer_size) {
            return Err("UV surface data exceeds the GPU limits".into());
        }
        self.surfaces.retain(|cache| !cache.gbuf.ptr_eq(&key));
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
        self.surfaces.push(SurfaceCache { gbuf: key, view: view.clone(), vertices: Some(vertices.clone()) });
        Ok((view, vertices))
    }

    pub(crate) fn retain_texture_sets(&mut self, count: usize) {
        self.release_unused_surfaces();
        self.baked.retain(|&ts, _| ts < count);
        self.id_bakes.retain(|&ts, _| ts < count);
        if let Some(curvature) = &mut self.curvature {
            curvature.cache.retain(|&ts, _| ts < count);
        }
        self.tile_images.retain(|&ts, _| ts < count);
    }

    pub(crate) fn release_unused_surfaces(&mut self) {
        self.tile_images.retain(|_, atlas| atlas.is_live());
        self.surfaces.retain(|cache| cache.gbuf.strong_count() != 0);
        self.id_bakes.retain(|_, cache| cache.data.strong_count() != 0);
        if let Some(curvature) = &mut self.curvature {
            curvature.cache.retain(|_, cache| cache.map.strong_count() != 0 && cache.gbuf.strong_count() != 0);
        }
    }

    pub(crate) fn clear_baked_inputs(&mut self) {
        self.baked.clear();
        self.id_bakes.clear();
        if let Some(curvature) = &mut self.curvature {
            curvature.cache.clear();
        }
    }

    pub(crate) fn release_inputs(&mut self, ts: usize) {
        self.baked.remove(&ts);
        self.id_bakes.remove(&ts);
        if let Some(curvature) = &mut self.curvature {
            curvature.cache.remove(&ts);
        }
        self.tile_images.remove(&ts);
    }

    fn id_bake_input(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, ts: usize, id_bake: Option<&IdBake>) -> Result<Option<wgpu::TextureView>, String> {
        let Some(id_bake) = id_bake else {
            self.id_bakes.remove(&ts);
            return Ok(self.default_id_bake.clone());
        };
        if self.default_id_bake.is_none() {
            return Err("Color selection exceeds the adapter's sampled-texture limit.".into());
        }
        let data = Arc::downgrade(&id_bake.data);
        if let Some(cache) = self.id_bakes.get(&ts).filter(|cache| cache.size == id_bake.size && cache.data.ptr_eq(&data)) {
            return Ok(Some(cache.view.clone()));
        }

        let [width, height] = id_bake.size;
        // Pack the RGB floats across all four lanes of each GPU texel. This avoids wasting
        // one word per source texel and allows complete rows to upload directly from the bake.
        let packed_height = (u64::from(height) * 3).div_ceil(4) as u32;
        let dimensions = device.limits().max_texture_dimension_2d;
        if width.max(packed_height) > dimensions || u64::from(width) * u64::from(packed_height) * 16 > MAX_ID_BAKE_BYTES {
            return Err("The ID map exceeds the adapter's GPU texture budget.".into());
        }
        self.id_bakes.remove(&ts);
        let texture = input_texture(device, "tc_composite_id_bake", (width, packed_height), 1, wgpu::TextureFormat::Rgba32Uint);
        let row_words = width as usize * 4;
        let word_count = width as usize * height as usize * 3;
        let mut tail_row = Vec::new();
        let mut pending = 0;
        for (band, source) in id_bake.data[..word_count].chunks(row_words * ROWS as usize).enumerate() {
            let rows = source.len() / row_words;
            let top = band as u32 * ROWS;
            let (full, tail) = source.split_at(rows * row_words);
            if rows > 0 {
                let full: &[[f32; 4]] = bytemuck::cast_slice(full);
                write_input(queue, &texture, (width, rows as u32), (top, 0), full);
            }
            if !tail.is_empty() {
                tail_row.extend_from_slice(tail);
                tail_row.resize(row_words, 0.0);
                let padded: &[[f32; 4]] = bytemuck::cast_slice(&tail_row);
                write_input(queue, &texture, (width, 1), (top + rows as u32, 0), padded);
            }
            pending += std::mem::size_of_val(source);
            if pending >= MAX_ID_UPLOAD_BYTES {
                let submission = queue.submit([]);
                #[cfg(not(target_arch = "wasm32"))]
                device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: None }).map_err(|error| error.to_string())?;
                #[cfg(target_arch = "wasm32")]
                let _ = submission;
                pending = 0;
            }
        }
        let view = texture.create_view(&Default::default());
        self.id_bakes.insert(ts, IdBakeCache { size: id_bake.size, data, view: view.clone() });
        Ok(Some(view))
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

    fn curvature_inputs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ts: usize,
        program: &CompositeProgram,
        baked_view: &wgpu::TextureView,
    ) -> Result<Option<CurvatureLevels>, String> {
        let source = program.baked.as_ref().and_then(|baked| baked.maps[2].as_ref().map(|map| (baked, map)));
        let Some((baked, map)) = source else {
            return Ok(self.curvature.as_mut().map(|curvature| {
                curvature.cache.remove(&ts);
                curvature.empty.clone()
            }));
        };
        let needed = program.curvature_sizes;
        if !needed.contains(&true) {
            return Ok(self.curvature.as_ref().map(|curvature| curvature.empty.clone()));
        }
        let Some(curvature) = &self.curvature else {
            return Err("Curvature detail sizes exceed the GPU adapter's binding or dispatch limits.".into());
        };
        let (_, gbuf) = program.surface.as_ref().ok_or("Curvature detail is missing its UV coverage.")?;
        let key = Arc::downgrade(gbuf);
        let map = Arc::downgrade(map);
        let size = [baked.width, baked.height];
        if let Some(cache) = curvature.cache.get(&ts).filter(|cache| cache.gbuf.ptr_eq(&key) && cache.map.ptr_eq(&map) && cache.size == size)
            && needed.iter().zip(cache.needed).all(|(&required, present)| !required || present)
        {
            return Ok(Some(cache.levels.clone()));
        }
        let coverage = (gbuf.covered_count() != gbuf.tri.len()).then(|| self.coverage_buffer(device, Some(gbuf))).transpose()?;
        let curvature = self.curvature.as_mut().ok_or("The GPU curvature adapter is unavailable.")?;
        curvature.cache.remove(&ts);
        let gpu = curvature.gpu.get_or_insert_with(|| CurvatureGpu::new(device));
        let levels = gpu.build(
            device,
            queue,
            CurvatureInput { baked: baked_view, bake_size: size, coverage: coverage.as_ref(), working_size: [gbuf.width, gbuf.height], needed },
        )?;
        curvature.cache.insert(ts, CurvatureCache { gbuf: key, map, size, needed, levels: levels.clone() });
        Ok(Some(levels))
    }

    fn tile_atlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, ts: usize, program: &CompositeProgram) -> Result<Option<&TileImageAtlas>, String> {
        let gray: Vec<_> = program.tile_patterns.image_sources().collect();
        let color: Vec<_> = program.material_images.atlas_images().collect();
        let raw = program.material_images.raw_planes();
        if gray.is_empty() && color.is_empty() && raw.is_empty() {
            self.tile_images.remove(&ts);
            return Ok(None);
        }
        if self.tile_images.get(&ts).is_none_or(|atlas| !atlas.matches(&gray, &color, raw)) {
            self.tile_images.remove(&ts);
            let atlas = TileImageAtlas::new(device, queue, &gray, &color, raw)?;
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
        let selection_work = if program.id_bake.is_some() {
            program.operations.iter().filter(|op| op.ids[1] == COLOR_SELECTION_OPCODE).map(|op| u64::from(op.procedural[2])).sum()
        } else {
            0
        };
        let pattern_work = program.tile_work_per_pixel()?;
        check_tile_update_budget(width, height, pattern_work)?;
        let tile_work = pattern_work.saturating_add(program.groups.work_per_pixel()).saturating_add(selection_work);
        let grouped = program.groups.depth != 0;
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
            self.filter_gpu.get_or_insert_with(|| NeighbourhoodGpu::new(device));
        }
        let (pipeline, prepare) = self.pipelines.get(device, &self.pipeline_layout, program);
        let coverage = self.coverage_buffer(device, program.coverage.as_ref().map(|(map, _)| map))?;
        let (surface, vertices) = self.surface_inputs(device, queue, program)?;
        let baked = self.baked_input(device, queue, ts, program.baked.as_ref())?;
        let curvature = self.curvature_inputs(device, queue, ts, program, &baked)?;
        let id_bake = self.id_bake_input(device, queue, ts, program.id_bake.as_ref())?;
        let [id_width, id_height] = program.id_bake.as_ref().map_or([0; 2], |id| id.size);
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
        // A wrapping stage's fields extend past the set's edges (FilterPlan::wraps)
        let wraps = program.filters.wraps();
        let field_area = |width: u32, rows: u32| {
            let side = |pixels: u32, limit: u32| {
                let side = (pixels << shift).saturating_add(halo.saturating_mul(2));
                if wraps { side } else { side.min(limit) }
            };
            u64::from(side(width, source_size.0)) * u64::from(side(rows, source_size.1))
        };
        let band_area = |width: u32, rows: u32| u64::from(width.next_multiple_of(64) * rows);
        let display_words = |width, rows| band_area(width, rows) * words_per_pixel;
        let group_start = |width, rows| display_words(width, rows) + program.filters.words(field_area(width, rows));
        let required = |width, rows| (group_start(width, rows) + program.groups.words(band_area(width, rows))) * 4;
        let filter_work: u64 =
            program.filters.stages.iter().map(|stage| stage.filter.map_or(1, NeighbourhoodFilter::work_per_texel) + if stage.cell > 1 { 6 } else { 0 }).sum();
        // A filtered band includes its source halo. Folder snapshots use only its output area.
        while required(band_width, band_rows) > scratch_limit {
            if band_width == 1 && band_rows == 1 {
                return Err("Composite inputs for one GPU texel exceed the adapter's working memory limit.".into());
            }
            if band_width >= band_rows {
                band_width = band_width.div_ceil(2);
            } else {
                band_rows = band_rows.div_ceil(2);
            }
        }
        let pitch = band_width.next_multiple_of(64);
        let texel_words = program.filters.field_words();
        let submit_budget = if grouped || filtered || band_width < width || band_rows < ROWS { MAX_TILE_WORK_PER_SUBMISSION } else { u64::MAX };
        let area = pitch * band_rows;
        let stride = u64::from(area) * 4;
        let field_base = (display_words(band_width, band_rows)) as u32;
        let group_base = group_start(band_width, band_rows) as u32;
        let scratch = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tc_composite_scratch"),
            size: required(band_width, band_rows),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let atlas = self.tile_atlas(device, queue, ts, program)?;
        let tile_images = atlas.map(|atlas| atlas.view.clone());
        let (operations, lookup) =
            if program.tile_patterns.is_empty() && program.ramp_words.is_empty() && program.geometry.words.is_empty() && program.material_images.is_empty() {
                (Cow::Borrowed(program.operations.as_slice()), self.lookup.clone())
            } else {
                let mut words = texturecraft_texel::srgb_lut().map(u32::from).to_vec();
                words.extend(&program.ramp_words);
                let geometry_start = words.len();
                words.extend(&program.geometry.words);
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
                for op in operations.iter_mut().filter(|op| op.constraints[0] != 0) {
                    let row = (op.constraints[0] - 1) as usize;
                    op.constraints[0] =
                        u32::try_from(geometry_start + row * program.geometry.stride).map_err(|_| "The GPU geometry mask offset is too large.")?;
                }
                for op in operations.iter_mut().filter(|op| op.ids[1] == TILE_GENERATOR_OPCODE) {
                    let index = op.high[3] as usize;
                    op.high[3] = *offsets.get(index).ok_or("A Tile Generator operation has no settings.")? as f32;
                }
                for op in &mut operations {
                    if op.ids[1] == IMAGE_SOURCE_OPCODE {
                        op.ids[2] = *images.get(op.ids[2] as usize).ok_or("An image source has no binding.")?;
                    }
                    // Placed planars and warps (the first of P1's selectors) read their frame record
                    if op.procedural[3] == DECAL_PROJECTION || op.procedural[3] == packages::PROJECTION_SELECTORS.start {
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
            let [paint_x, paint_y, paint_columns, paint_rows] = window.params();
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
                let mut field_rect =
                    [field.min[0], field.min[1], field.max[0].min(source_size.0) - field.min[0], field.max[1].min(source_size.1) - field.min[1]];
                if wraps {
                    // The whole halo on every side; the origin can lie below zero (i32 bits)
                    let start = |pixel: u32| (i64::from(pixel << shift) - i64::from(halo)) as i32 as u32;
                    field_rect = [start(left), start(top), (width << shift).saturating_add(halo * 2), (rows << shift).saturating_add(halo * 2)];
                }
                let mut values = [0; packages::UNIFORM_WORDS];
                values[..packages::FIELD_LAYOUT].copy_from_slice(&[
                    width,
                    rows,
                    top,
                    area,
                    source_size.0,
                    source_size.1,
                    1 << shift,
                    u32::from(program.glossiness),
                    program.layer_count,
                    pitch,
                    program.display_outputs,
                    left,
                    u32::from(program.coverage.is_some()),
                    source_size.0.div_ceil(32),
                    radius,
                    u32::from(program.needs_surface)
                        | (u32::from(program.baked.is_some()) << 1)
                        | (u32::from(program.brush_stencil.is_some()) << 2)
                        | (u32::from(!program.geometry.words.is_empty()) << 3),
                    paint_x,
                    paint_y,
                    paint_columns,
                    paint_rows,
                    field_rect[0],
                    field_rect[1],
                    field_rect[2],
                    field_rect[3],
                    field_base,
                    group_base,
                    program.groups.words_per_level,
                    program.groups.channels,
                    0,
                    0,
                    field_rect[2],
                    field_rect[3],
                    id_width,
                    id_height,
                    0,
                    0,
                    0,
                    0,
                    0,
                    program.mask_layer,
                    0,
                    0,
                    0,
                    0,
                ]);
                // Words per field texel and whether the sixth (input) word is present; the
                // prepare pass adds its stage's input operation in the third word.
                values[packages::FIELD_LAYOUT..packages::FIELD_LAYOUT + 2].copy_from_slice(&[texel_words, u32::from(texel_words > FIELD_WORDS)]);
                packages::write_band_uniform(program, &mut values);
                let bindings = |values: &[u32]| {
                    let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("tc_composite_band"),
                        contents: bytemuck::cast_slice(values),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                    let mut entries = [
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
                        wgpu::BindGroupEntry {
                            binding: 13,
                            resource: wgpu::BindingResource::TextureView(curvature.as_ref().map_or(&baked, |levels| &levels.image)),
                        },
                        wgpu::BindGroupEntry {
                            binding: 14,
                            resource: curvature.as_ref().map_or_else(|| lookup.as_entire_binding(), |levels| levels.layout.as_entire_binding()),
                        },
                        wgpu::BindGroupEntry { binding: 15, resource: wgpu::BindingResource::TextureView(id_bake.as_ref().unwrap_or(&baked)) },
                    ];
                    let mut count = if curvature.is_some() { 15 } else { 13 };
                    if id_bake.is_some() {
                        entries.swap(count, 15);
                        count += 1;
                    }
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("tc_composite_band"),
                        layout: &self.bindings,
                        entries: &entries[..count],
                    })
                };
                if let (Some(prepare), Some(kernels)) = (&prepare, &self.filter_gpu) {
                    let fields = program.filters.stages.len() as u32;
                    let field_words = field_rect[2] * field_rect[3] * texel_words;
                    let offset = |slot| field_base + slot * field_words;
                    let (source_width, source_rows) =
                        if tile_work == 0 { (field_rect[2], field_rect[3]) } else { composite_band_size(field_rect[2], tile_work) };
                    for (output, stage) in program.filters.stages.iter().enumerate() {
                        let mut source = values;
                        let direct = stage.filter.is_none() && stage.cell == 1;
                        let slot = if direct { output as u32 } else { fields };
                        source[25..28].copy_from_slice(&[slot, stage.layer, stage.preceding]);
                        source[packages::FIELD_LAYOUT + 2] = stage.input;
                        source[packages::FIELD_LAYOUT + 3] = u32::from(stage.wrap);
                        if let Some((channel, first, cut)) = stage.anchor {
                            source[36..39].copy_from_slice(&[channel_slot(channel), first, cut]);
                            source[40..43].copy_from_slice(&channel.default_value().map(f32::to_bits));
                        }
                        for y in (0..field_rect[3]).step_by(source_rows as usize) {
                            for x in (0..field_rect[2]).step_by(source_width as usize) {
                                let columns = source_width.min(field_rect[2] - x);
                                let rows = source_rows.min(field_rect[3] - y);
                                let work_here = u64::from(columns).saturating_mul(u64::from(rows)).saturating_mul(tile_work);
                                if work > 0 && work.saturating_add(work_here) > submit_budget {
                                    let previous = std::mem::replace(&mut encoder, make_encoder());
                                    last_submission = Some(match bounds.take() {
                                        Some(written) => submit(previous, written, submitted),
                                        None => queue.submit([previous.finish()]),
                                    });
                                    work = 0;
                                }
                                source[28..32].copy_from_slice(&[x, y, columns, rows]);
                                let source_bindings = bindings(&source);
                                let mut pass =
                                    encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("tc_spatial_source"), timestamp_writes: None });
                                pass.set_pipeline(prepare);
                                pass.set_bind_group(0, &source_bindings, &[]);
                                pass.dispatch_workgroups(columns.div_ceil(WORKGROUP), rows.div_ceil(WORKGROUP), 1);
                                work = work.saturating_add(work_here);
                            }
                        }
                        let region = NeighbourhoodRegion {
                            rect: field_rect,
                            original: offset(fields),
                            blur: [offset(fields + 1), offset(fields + 2)],
                            output: offset(output as u32),
                            field_words: texel_words,
                            input: u32::from(texel_words > FIELD_WORDS),
                        };
                        if stage.cell > 1 {
                            let target = if stage.filter.is_some() { region.original } else { region.output };
                            kernels.encode_cells(device, &mut encoder, &scratch, NeighbourhoodRegion { output: target, ..region }, stage.cell);
                        }
                        if let Some(filter) = stage.filter {
                            kernels.encode(device, &mut encoder, &scratch, region, filter);
                        }
                    }
                }
                let bindings = bindings(&values);
                {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("tc_composite"), timestamp_writes: None });
                    pass.set_pipeline(&pipeline);
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

fn check_tile_update_budget(width: u32, height: u32, work_per_pixel: u64) -> Result<(), &'static str> {
    let estimated_work = (u64::from(width) * u64::from(height)).saturating_mul(work_per_pixel);
    if estimated_work > MAX_TILE_WORK_PER_UPDATE {
        return Err("Tile Generator overlap exceeds the GPU compositor's interactive work budget.");
    }
    Ok(())
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
) -> Result<Vec<(CompositeRegion, CompositePaintWindow, usize)>, String> {
    let mut work = vec![region];
    let mut windows = Vec::new();
    while let Some(chunk) = work.pop() {
        let source = CompositeRegion { min: chunk.min.map(|p| p << shift), max: chunk.max.map(|p| p << shift) }.expanded(radius);
        let window = CompositePaintWindow::covering(source.min, source.max, [size.0, size.1], &program.paint);
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
    use texturecraft_core::{Fill, HighpassSpace, Layer, Levels, Mask, OutputResolution, Procedural, TextureSet};
    use texturecraft_geom::primitives;
    use texturecraft_texel::{srgb_lut, to_u16};

    #[test]
    fn mask_and_content_anchors_match_cpu_in_material_and_mask_views() {
        use texturecraft_compose::eval::{Composite, Inputs, composite_tiles, mask_image};
        use texturecraft_compose::tile_bytes;
        use texturecraft_core::MicroDetail;

        let Ok(mut gpu) = Headless::new() else { return };
        const SIZE: u32 = 64;
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let mut project = Project::new("Anchor stages", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
        let channels = [Channel::BaseColor, Channel::Height];
        let folder = project.alloc_id();
        let mut group = Layer::new(folder, "Translucent folder", LayerKind::folder(), &channels);
        group.channels.get_mut(&Channel::Height).unwrap().opacity = 0.45;
        project.insert_layer(0, group, None, false).unwrap();
        let lower = project.alloc_id();
        let mask_anchor = project.alloc_id();
        let height_anchor = project.alloc_id();
        let source = BTreeMap::from([(Channel::Height, Source::Uniform([0.8; 3]))]);
        let mut layer = Layer::new(lower, "Donor", LayerKind::Fill(Fill { sources: source, ..Fill::default() }), &channels);
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(
            project.alloc_id(),
            EffectKind::Fill { source: Source::Uniform([0.6; 3]), projection: Projection::default(), sampling: Sampling::SAVED_BEFORE },
        ));
        mask.effects.push(Effect::new(mask_anchor, EffectKind::Anchor));
        mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Invert)));
        layer.mask = Some(mask);
        layer.content_effects.push(Effect::new(height_anchor, EffectKind::Anchor));
        layer.content_effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Invert)));
        project.insert_layer(0, layer, Some(folder), true).unwrap();

        let after_group = project.alloc_id();
        let after_group_anchor = project.alloc_id();
        let mut bookmark = Layer::new(after_group, "Mixed height", LayerKind::Fill(Fill::default()), &channels);
        bookmark.channels.get_mut(&Channel::Height).unwrap().mode = BlendMode::Disable;
        bookmark.content_effects.push(Effect::new(after_group_anchor, EffectKind::Anchor));
        project.insert_layer(0, bookmark, None, false).unwrap();

        let owner = project.alloc_id();
        let source = BTreeMap::from([(
            Channel::BaseColor,
            Source::Anchor { anchor: mask_anchor, channel: Channel::BaseColor, ramp: vec![[0.1, 0.15, 0.3], [0.8, 0.45, 0.2]] },
        )]);
        let mut layer = Layer::new(owner, "Reader", LayerKind::Fill(Fill { sources: source, ..Fill::default() }), &channels);
        let mut mask = Mask::new(MaskBackground::Black);
        let mut generator = Generator::new(GeneratorKind::AmbientOcclusion);
        generator.micro = Some(MicroDetail { anchor: height_anchor, amount: 0.8 });
        generator.inputs.insert(BakedMap::AmbientOcclusion, Source::Anchor { anchor: mask_anchor, channel: Channel::BaseColor, ramp: Vec::new() });
        generator.global.blur = 0.7;
        mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Generator(generator)));
        let mut mix_height = Effect::new(
            project.alloc_id(),
            EffectKind::Fill {
                source: Source::Anchor { anchor: after_group_anchor, channel: Channel::Height, ramp: Vec::new() },
                projection: Projection::default(),
                sampling: Sampling::SAVED_BEFORE,
            },
        );
        mix_height.opacity = 0.3;
        mask.effects.push(mix_height);
        layer.mask = Some(mask);
        project.insert_layer(0, layer, None, false).unwrap();

        let pixels = PixelStore::default();
        let baked = BakedMaps::default();
        let images = ImageStore::default();
        let inputs = Inputs {
            project: &project,
            ts: 0,
            mesh: &mesh,
            gbuf: &gbuf,
            pixels: &pixels,
            baked: &baked,
            images: &images,
            block_scales: project.texture_sets[0].block_scales(),
            preview: None,
            bounds: mesh.bounds(),
        };
        let mut oracle = Composite::new(&channels, SIZE);
        composite_tiles(&inputs, &mut oracle, &[(0, 0)]);
        let expected = tile_bytes(&oracle, &baked, DisplayKind::BaseColor, 0, 0);
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        let mut program = CompositeProgram::try_compile_region(&project, &pixels, 0, None).unwrap();
        program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
        gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
        let texture = gpu.renderer.display_textures()[0][DisplayKind::BaseColor as usize];
        let (_, _, actual) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap();
        assert!(actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 2));

        let expected = mask_image(&inputs, owner, SIZE).unwrap();
        let mut program = CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, owner, None, None).unwrap();
        program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
        gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
        let (_, _, actual) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.aux_texture(0).unwrap()).unwrap();
        assert!(actual.as_chunks::<4>().0.iter().zip(expected).all(|(pixel, value)| pixel[0].abs_diff(value) <= 2));
    }

    #[test]
    fn restricted_passthrough_groups_publish_child_anchors_on_both_parts() {
        use texturecraft_compose::eval::{Composite, Inputs, composite_tiles};
        use texturecraft_compose::tile_bytes;
        use texturecraft_geom::SubMesh;

        let Ok(mut gpu) = Headless::new() else { return };
        let mut mesh = primitives::plane(1.0, 1);
        let original = mesh.submeshes[0].clone();
        mesh.submeshes[0].count = 3;
        mesh.submeshes.push(SubMesh { name: "Other part".into(), start: 3, count: 3, ..original.clone() });
        let mesh = Arc::new(mesh);
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, TILE, TILE));
        let probes = [0, 1].map(|id| gbuf.tri.iter().position(|&tri| tri == id).unwrap());

        let mut project = Project::new("Part-restricted anchors", "builtin:plane");
        let mut set = TextureSet::new("Plane", 0, TILE);
        set.channels.push((Channel::Opacity, texturecraft_core::ChannelFormat::default_for(Channel::Opacity)));
        let channels = set.channel_list();
        project.texture_sets.push(set);
        let folder = project.alloc_id();
        let mut group = Layer::new(folder, "One part", LayerKind::folder(), &channels);
        for blend in group.channels.values_mut() {
            blend.mode = BlendMode::Passthrough;
        }
        group.geometry_mask = Some(vec![original.name]);
        project.insert_layer(0, group, None, false).unwrap();

        let donor = project.alloc_id();
        let mask_anchor = project.alloc_id();
        let content_anchor = project.alloc_id();
        let material_anchor = project.alloc_id();
        let mut donor = Layer::new(
            donor,
            "Donor",
            LayerKind::Fill(Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.95; 3]))]), ..Default::default() }),
            &channels,
        );
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(
            project.alloc_id(),
            EffectKind::Fill { source: Source::Uniform([0.8; 3]), projection: Projection::default(), sampling: Sampling::SAVED_BEFORE },
        ));
        mask.effects.push(Effect::new(mask_anchor, EffectKind::Anchor));
        donor.mask = Some(mask);
        donor.content_effects.push(Effect::new(content_anchor, EffectKind::Anchor));
        donor.content_effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Invert)));
        donor.content_effects.push(Effect::new(material_anchor, EffectKind::Anchor));
        project.insert_layer(0, donor, Some(folder), true).unwrap();

        let reader = project.alloc_id();
        let sources = [(Channel::Opacity, mask_anchor), (Channel::Roughness, content_anchor), (Channel::Metallic, material_anchor)]
            .into_iter()
            .map(|(target, anchor)| (target, Source::Anchor { anchor, channel: Channel::BaseColor, ramp: Vec::new() }))
            .collect();
        project.insert_layer(0, Layer::new(reader, "Reader", LayerKind::Fill(Fill { sources, ..Default::default() }), &channels), None, false).unwrap();

        let pixels = PixelStore::default();
        let baked = BakedMaps::default();
        let images = ImageStore::default();
        let mut cpu = Composite::new(&channels, TILE);
        composite_tiles(
            &Inputs {
                project: &project,
                ts: 0,
                mesh: &mesh,
                gbuf: &gbuf,
                pixels: &pixels,
                baked: &baked,
                images: &images,
                block_scales: project.texture_sets[0].block_scales(),
                preview: None,
                bounds: mesh.bounds(),
            },
            &mut cpu,
            &[(0, 0)],
        );
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((TILE, TILE), 0)], 1);
        let mut program = CompositeProgram::try_compile_region(&project, &pixels, 0, None).unwrap();
        program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
        gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
        for kind in [DisplayKind::BaseColor, DisplayKind::Orm] {
            let expected = tile_bytes(&cpu, &baked, kind, 0, 0);
            let texture = gpu.renderer.display_textures()[0][kind as usize];
            let (_, _, actual) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap();
            for (part, texel) in probes.into_iter().enumerate() {
                let (actual, expected) = (&actual[texel * 4..][..4], &expected[texel * 4..][..4]);
                assert!(actual.iter().zip(expected).all(|(a, b)| a.abs_diff(*b) <= 2), "{kind:?}, part {part}: GPU {actual:?}, CPU {expected:?}");
            }
        }
    }

    #[test]
    fn generator_global_blur_and_resolution_match_cpu_with_an_image_override() {
        use texturecraft_compose::{
            ImageRes,
            eval::{Inputs, mask_image},
        };

        let Ok(mut gpu) = Headless::new() else { return };
        const SIZE: u32 = 128;
        let mut mesh = primitives::plane(1.0, 1);
        for uv in &mut mesh.uvs {
            *uv = *uv * 0.8 + texturecraft_geom::glam::Vec2::splat(0.1);
        }
        let mesh = Arc::new(mesh);
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let mut project = Project::new("Generator ramp", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
        let owner = project.alloc_id();
        let mut layer = Layer::new(owner, "Surface", LayerKind::Fill(Fill::default()), &[Channel::BaseColor]);
        let mut generator = Generator::new(GeneratorKind::Dirt);
        generator.noise_amount = 0.0;
        generator.invert = true;
        generator.offset = 0.07;
        generator.global.blur = 0.85;
        generator.global.balance = 0.58;
        generator.global.contrast = 0.23;
        generator.resolution.factor = -3;
        generator.inputs.insert(BakedMap::AmbientOcclusion, Source::image("AO override"));
        let pixels = PixelStore::default();
        let baked = BakedMaps::default();
        let images = ImageStore::from([("AO override".into(), Arc::new(ImageRes::new(2, 2, vec![[0.15; 4], [0.7; 4], [0.85; 4], [0.3; 4]], false)))]);
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(
            project.alloc_id(),
            EffectKind::Fill { source: Source::Uniform([0.58; 3]), projection: Projection::default(), sampling: Sampling::SAVED_BEFORE },
        ));
        let mut effect = Effect::new(project.alloc_id(), EffectKind::Generator(generator));
        effect.blend = BlendMode::LinearDodge;
        effect.opacity = 0.83;
        mask.effects.push(effect);
        let mut subtract = Effect::new(
            project.alloc_id(),
            EffectKind::Fill { source: Source::Uniform([0.22; 3]), projection: Projection::default(), sampling: Sampling::SAVED_BEFORE },
        );
        subtract.blend = BlendMode::Subtract;
        mask.effects.push(subtract);
        mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 32.0, wrap: false })));
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
        let expected = mask_image(&inputs, owner, SIZE).unwrap();
        let mut program = CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, owner, None, None).unwrap();
        program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
        program.set_tile_images(&images);
        assert_eq!(program.filters.stages[0].cell, 8);
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
        let texture = gpu.renderer.aux_texture(0).unwrap();
        let (_, _, actual) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap();
        for (index, (&expected, pixel)) in expected.iter().zip(actual.as_chunks::<4>().0).enumerate() {
            assert!(pixel[0].abs_diff(expected) <= 2, "generator mask texel {index}: {} instead of {expected}", pixel[0]);
        }
    }

    #[test]
    fn content_material_and_sparse_sublayers_match_cpu_before_and_during_a_stroke() {
        use texturecraft_compose::{Composite, ImageRes, Inputs, PaintTarget, StrokePreview, composite_tiles, tile_bytes};
        use texturecraft_core::Decal;
        use texturecraft_texel::to_u16_signed;

        let Ok(mut gpu) = Headless::new() else { return };
        const SIZE: u32 = TILE;
        let size = (SIZE, SIZE);
        let channels = [Channel::BaseColor, Channel::Height];
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let mut project = Project::new("Content effects", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
        let owner = project.alloc_id();
        let mut layer = Layer::new(owner, "Empty paint layer with content", LayerKind::Paint, &channels);
        layer.channels.get_mut(&Channel::BaseColor).unwrap().mode = BlendMode::Replace;
        let material = Fill {
            sources: BTreeMap::from([(Channel::BaseColor, Source::image("Decal")), (Channel::Height, Source::Uniform([-0.29; 3]))]),
            projection: Projection::decal(Decal {
                center: [-0.14, 0.0, 0.13],
                normal: [0.0, 1.0, 0.0],
                up: [0.0, 0.0, 1.0],
                size: [0.55, 0.63],
                rotation: 13.0,
                offset: [0.01, 0.0],
                depth: 0.08,
                symmetry: None,
                flip: [false, true],
            }),
            sampling: Sampling::SAVED_BEFORE,
            ..Fill::default()
        };
        let mut material = Effect::new(project.alloc_id(), EffectKind::Material(material));
        material.opacity = 0.73;
        let mut paint = Effect::new(project.alloc_id(), EffectKind::Paint);
        paint.opacity = 0.78;
        paint.blend = BlendMode::Screen;
        let paint_id = paint.id;
        let blur = Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 96.0, wrap: false }));
        let mut path = Effect::new(project.alloc_id(), EffectKind::Path(Vec::new()));
        path.opacity = 0.48;
        path.blend = BlendMode::Subtract;
        let path_id = path.id;
        layer.content_effects = vec![material, paint, blur, path];
        project.insert_layer(0, layer, None, false).unwrap();

        let mut pixels = PixelStore::default();
        let color = pixels.layer_mut(paint_id, Channel::BaseColor, size);
        for y in 13..42 {
            for x in 24..47 {
                color.set(x, y, &[to_u16(0.24), to_u16(0.08), to_u16(0.11), to_u16(0.45)]);
            }
        }
        let height = pixels.layer_mut(path_id, Channel::Height, size);
        for y in 24..49 {
            for x in 8..37 {
                height.set(x, y, &[to_u16_signed(0.3), to_u16(0.63)]);
            }
        }
        let images = ImageStore::from([(
            "Decal".into(),
            Arc::new(ImageRes::new(2, 2, vec![[0.18, 0.26, 0.63, 0.14], [0.36, 0.8, 0.16, 0.82], [0.72; 4], [0.39; 4]], true)),
        )]);
        let alpha = input_texture(&gpu.device, "test content stroke", size, 1, wgpu::TextureFormat::R16Uint);
        let mut preview = StrokePreview {
            target: PaintTarget::Layer(paint_id),
            alpha: TileImage::new(SIZE, SIZE, 1),
            opacity: 0.8,
            erase: false,
            values: BTreeMap::from([(Channel::BaseColor, [0.11, 0.46, 0.22]), (Channel::Height, [0.42; 3])]),
            mask_gray: texturecraft_compose::MaskGray::Value(1.0),
            stencil: None,
            carried: BTreeMap::new(),
            carried_mask: None,
        };
        let mut dense_alpha = vec![0; (SIZE * SIZE) as usize];
        for y in 29..53 {
            for x in 36..59 {
                let value = to_u16(0.62);
                preview.alpha.set(x, y, &[value]);
                dense_alpha[(y * SIZE + x) as usize] = value;
            }
        }
        write_input(&gpu.queue, &alpha, size, (0, 0), &dense_alpha);
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[(size, 0)], 1);
        let baked = BakedMaps::default();
        for live in [false, true] {
            let inputs = Inputs {
                project: &project,
                ts: 0,
                mesh: &mesh,
                gbuf: &gbuf,
                pixels: &pixels,
                baked: &baked,
                block_scales: project.texture_sets[0].block_scales(),
                preview: live.then_some(&preview),
                images: &images,
                bounds: mesh.bounds(),
            };
            let mut cpu = Composite::new(&channels, SIZE);
            composite_tiles(&inputs, &mut cpu, &[(0, 0)]);
            let live_brush = live.then(|| LiveBrush {
                target: LiveBrushTarget::Layer { layer: paint_id, values: preview.values.clone() },
                opacity: preview.opacity,
                alpha: alpha.create_view(&Default::default()),
                stencil: None,
            });
            let mut program = CompositeProgram::try_compile_with_brush(&project, &pixels, 0, None, live_brush).unwrap();
            assert_eq!(program.filters.stages.len(), channels.len());
            program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
            program.set_tile_images(&images);
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
            for kind in [DisplayKind::BaseColor, DisplayKind::Height] {
                let expected = tile_bytes(&cpu, &baked, kind, 0, 0);
                let texture = gpu.renderer.display_textures()[0][kind as usize];
                let (_, _, actual) = Readback::blocking_level(&gpu.device, &gpu.queue, texture, 0).unwrap();
                assert_eq!(actual.len(), expected.len());
                if kind == DisplayKind::Height {
                    for (actual, expected) in actual.as_chunks::<2>().0.iter().zip(expected.as_chunks::<2>().0) {
                        let actual = half::f16::from_le_bytes(*actual).to_f32();
                        let expected = half::f16::from_le_bytes(*expected).to_f32();
                        assert!((actual - expected).abs() <= 0.003, "stroke={live}: GPU height {actual}, CPU {expected}");
                    }
                } else {
                    assert!(actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 2), "stroke={live}: GPU color differs from CPU");
                }
            }
        }
    }

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
            Filter::Blur { radius: 64.0, wrap: false },
            Filter::Sharpen { amount: 0.85 },
            Filter::HighPass { radius: 64.0, space: HighpassSpace::Working, alpha: false },
            Filter::Bevel { width: 128.0, wrap: false },
            Filter::Glow { radius: 64.0, amount: 0.65 },
            Filter::DirectionalBlur { radius: 64.0, angle: 34.0 },
            Filter::Warp { intensity: 64.0, scale: 6.0, seed: 11, divider: 1 },
            Filter::Warp { intensity: 640.0, scale: 6.0, seed: 11, divider: 10 },
            Filter::DirectionalWarp { intensity: 64.0, angle: 40.0, scale: 6.0, seed: 11, divider: 1 },
            Filter::EdgeDetect { width: 52.0 },
            Filter::SlopeBlur { radius: 64.0, scale: 6.0, seed: 11, divider: 1 },
            Filter::Mosaic { size: 64.0, alpha: true },
        ];
        // A filter set to Disable leaves content alone; a mask filter applies whatever its blend
        let cases = filters.into_iter().map(|filter| (filter, BlendMode::Normal, BlendMode::Normal)).chain([
            (Filter::Blur { radius: 64.0, wrap: false }, BlendMode::Replace, BlendMode::Normal),
            (Filter::Blur { radius: 64.0, wrap: false }, BlendMode::Normal, BlendMode::Disable),
        ]);
        for (filter, blend, effect_blend) in cases {
            let mut project = Project::new("Neighbourhood filters", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
            let owner = project.alloc_id();
            let mut layer = Layer::new(owner, "Paint", LayerKind::Paint, &channels);
            layer.channels.get_mut(&Channel::BaseColor).unwrap().mode = blend;
            let mut mask = Mask::new(MaskBackground::Black);
            let mask_paint = project.alloc_id();
            mask.effects.push(Effect::new(mask_paint, EffectKind::Paint));
            for stack in [&mut layer.content_effects, &mut mask.effects] {
                let mut effect = Effect::new(project.alloc_id(), EffectKind::Filter(filter.clone()));
                effect.opacity = 0.8;
                effect.blend = effect_blend;
                stack.push(effect);
                stack.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Clamp { min: 0.18, max: 0.86 })));
                stack.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 24.0, wrap: false })));
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
            // A disabled content filter is no stage
            assert_eq!(program.filters.stages.len(), if effect_blend == BlendMode::Disable { 3 } else { 4 });
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
            let texture = gpu.renderer.display_textures()[0][DisplayKind::BaseColor as usize];
            let (_, _, full) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap();
            for (tx, ty) in tiles {
                let expected = tile_bytes(&oracle, &baked, DisplayKind::BaseColor, tx, ty);
                for (y, row) in expected.as_chunks::<{ (TILE * 4) as usize }>().0.iter().enumerate() {
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
    fn tile_generator_budget_rejects_full_overlap_and_allows_smaller_gpu_updates() {
        use texturecraft_core::{TileGenerator, TileSizeMode};

        let mut patterns = TilePatterns::default();
        let normal = TileGenerator { amount: [64, 64], ..TileGenerator::default() };
        let overlapping = TileGenerator { size_mode: TileSizeMode::Absolute, size_absolute: [1.0; 2], ..normal.clone() };
        let normal = patterns.register(normal, 0);
        let overlapping_index = patterns.register(overlapping.clone(), 0);
        let regular_work = patterns.work_per_sample(normal).unwrap();
        let overlapping_work = patterns.work_per_sample(overlapping_index).unwrap();

        assert_eq!(check_tile_update_budget(8192, 8192, regular_work), Ok(()));
        assert_eq!(composite_band_size(8192, regular_work), (8192, ROWS));
        assert!(check_tile_update_budget(8192, 8192, overlapping_work).is_err());
        assert_eq!(check_tile_update_budget(256, 256, overlapping_work), Ok(()));
        let (width, rows) = composite_band_size(256, overlapping_work);
        assert!(rows < ROWS);
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
            projection: Projection::Triplanar { scale: 1.0, sharpness: 6.0, offset: [0.0; 3], uv: Default::default(), projector: Default::default() },
            ..Default::default()
        };
        let layer = Layer::new(project.alloc_id(), "Overlapping tiles", LayerKind::Fill(fill), &[Channel::BaseColor]);
        project.insert_layer(0, layer, None, false).unwrap();
        let program = CompositeProgram::compile(&project, &PixelStore::default(), 0).unwrap();
        assert_eq!(program.tile_work_per_pixel().unwrap(), overlapping_work * 12, "the three planes each need four samples");
    }

    // The CPU fallback tables of the GPU port live with each package (`still_on_cpu` in
    // gpu_composite_point.rs, gpu_composite_stack.rs, gpu_composite_lighting.rs,
    // gpu_neighbourhood_box.rs, gpu_neighbourhood_gather.rs, …); the stacks that must stay on the
    // GPU are checked in gpu_composite_packages.rs.

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
            Filter::even_gradient(vec![[0.04, 0.13, 0.7], [0.85, 0.56, 0.22], [0.76, 0.11, 0.34]]),
            Filter::even_gradient(Vec::new()),
            Filter::ReplaceColor { from: [0.2, 0.43, 0.72], to: [0.62, 0.35, 0.33], tolerance: 0.09, softness: 0.28 },
            Filter::BakedLighting { azimuth: 30.0, elevation: 60.0, light: 0.6, ambient: 0.35, occlusion: 0.72, stylized: None },
            Filter::Threshold { level: 0.4, mode: ThresholdMode::GreaterOrEqual },
            // Each mode at the roughness and the mask's exact values, where the modes part
            Filter::Threshold { level: 0.41, mode: ThresholdMode::GreaterOrEqual },
            Filter::Threshold { level: 0.41, mode: ThresholdMode::Greater },
            Filter::Threshold { level: 0.41, mode: ThresholdMode::Lower },
            Filter::Threshold { level: 0.41, mode: ThresholdMode::LowerOrEqual },
            Filter::Threshold { level: 0.36, mode: ThresholdMode::GreaterOrEqual },
            Filter::Threshold { level: 0.36, mode: ThresholdMode::Greater },
            Filter::Threshold { level: 0.36, mode: ThresholdMode::Lower },
            Filter::Threshold { level: 0.36, mode: ThresholdMode::LowerOrEqual },
            Filter::Posterize { levels: 4.0, alpha: false },
            Filter::Hsl { hue: 0.2, saturation: 0.3, lightness: -0.1, perceptive: false },
            Filter::HistogramScan { position: 0.45, contrast: 0.6, invert_position: false },
            Filter::InvertOff,
            Filter::Empty,
        ];
        // Levels from older saves clamp; new ones keep values outside the input range on the line through it
        let levels = |clamp| Levels { in_low: 0.38, in_high: 0.52, gamma: 1.6, out_low: 0.1, out_high: 0.85, affects: None, clamp };
        let kinds = filters.into_iter().map(EffectKind::Filter).chain([EffectKind::Levels(levels(true)), EffectKind::Levels(levels(false))]);
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        // Each once more set to Disable: content filters then change nothing, mask filters apply anyway
        for (filter, disabled) in kinds.flat_map(|kind| [(kind.clone(), false), (kind, true)]) {
            let mut project = Project::new("Point filters", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
            let owner = project.alloc_id();
            let fill = Fill { sources: sources.clone(), ..Default::default() };
            let mut layer = Layer::new(owner, "Surface", LayerKind::Fill(fill), &channels);
            let mut effect = Effect::new(project.alloc_id(), filter.clone());
            effect.opacity = 0.77;
            if disabled {
                effect.blend = BlendMode::Disable;
            }
            layer.content_effects.push(effect);
            let mut mask = Mask::new(MaskBackground::Black);
            mask.effects.push(Effect::new(
                project.alloc_id(),
                EffectKind::Fill { source: Source::Uniform([0.36; 3]), projection: Projection::default(), sampling: Sampling::SAVED_BEFORE },
            ));
            let mut effect = Effect::new(project.alloc_id(), filter.clone());
            effect.opacity = 0.63;
            effect.blend = if disabled { BlendMode::Disable } else { BlendMode::Multiply };
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
                        assert!((actual - expected).abs() <= 0.001, "{filter:?} disabled {disabled} {kind:?}: GPU {actual} vs CPU {expected}");
                    }
                } else {
                    assert!(actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1), "{filter:?} disabled {disabled} {kind:?}");
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
                assert!(r.abs_diff(expected) <= 1 && r == g && r == b && a == 255, "{filter:?} disabled {disabled} mask: GPU {r} vs CPU {expected}");
            }
        }
    }

    #[test]
    fn color_selection_matches_raw_rgb_in_material_and_mask_views() {
        use texturecraft_compose::eval::{Composite, Inputs, composite_tiles, mask_image};
        use texturecraft_compose::tile_bytes;

        let Ok(mut gpu) = Headless::new() else { return };
        const SIZE: u32 = TILE;
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let mut project = Project::new("ID color selection", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
        let owner = project.alloc_id();
        let color = Channel::BaseColor;
        let fill = Fill { sources: BTreeMap::from([(color, Source::Uniform([0.89, 0.19, 0.04]))]), ..Default::default() };
        let mut layer = Layer::new(owner, "Surface", LayerKind::Fill(fill), &[color]);
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(
            project.alloc_id(),
            EffectKind::Fill { source: Source::Uniform([0.75; 3]), projection: Projection::default(), sampling: Sampling::SAVED_BEFORE },
        ));
        let raw = [0.40101, 0.69217, 0.05289];
        let within = [0.375; 3];
        let exact = [0.21875, 0.78125, 0.09375];
        let effect_kind = |tolerance| EffectKind::ColorSelection(ColorSelection::new(vec![raw, within, exact], tolerance));
        let mut select = Effect::new(project.alloc_id(), effect_kind(1.0 / 64.0));
        select.blend = BlendMode::Multiply;
        select.opacity = 0.75;
        mask.effects.push(select);
        layer.mask = Some(mask);
        project.insert_layer(0, layer, None, false).unwrap();

        let colors = [raw, [0.390625; 3], [0.390626, 0.375, 0.375], exact, [0.1, 0.9, 0.2], raw, [0.8, 0.1, 0.9], [0.359375; 3], [0.0; 3]];
        let make_bake =
            |colors: [[f32; 3]; 9]| BakedMaps { resolution: 3, maps: BTreeMap::from([(BakedMap::Id, Arc::new(colors.into_iter().flatten().collect()))]) };
        let bake = make_bake(colors);
        let mut changed = colors;
        changed[0] = [0.0; 3];
        let changed = make_bake(changed);
        let empty = BakedMaps::default();
        let pixels = PixelStore::default();
        let images = ImageStore::default();
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        for (baked, tolerance) in [(&bake, 1.0 / 64.0), (&bake, 0.0), (&changed, 0.0), (&empty, 0.0)] {
            project.layer_mut(owner).unwrap().mask.as_mut().unwrap().effects[1].kind = effect_kind(tolerance);
            let inputs = Inputs {
                project: &project,
                ts: 0,
                mesh: &mesh,
                gbuf: &gbuf,
                pixels: &pixels,
                baked,
                block_scales: project.texture_sets[0].block_scales(),
                preview: None,
                images: &images,
                bounds: mesh.bounds(),
            };
            let mut oracle = Composite::new(&[color], SIZE);
            composite_tiles(&inputs, &mut oracle, &[(0, 0)]);
            let expected = tile_bytes(&oracle, baked, DisplayKind::BaseColor, 0, 0);
            let mut program = CompositeProgram::try_compile_region(&project, &pixels, 0, None).unwrap();
            program.set_surface_inputs(&mesh, &gbuf, Some(baked)).unwrap();
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
            let (_, _, actual) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.display_textures()[0][0]).unwrap();
            assert_eq!(actual.len(), expected.len());
            assert!(actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1));

            let expected = mask_image(&inputs, owner, SIZE).unwrap();
            let mut mask = CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, owner, None, None).unwrap();
            mask.set_surface_inputs(&mesh, &gbuf, Some(baked)).unwrap();
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &mask).unwrap();
            let (_, _, actual) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.aux_texture(0).unwrap()).unwrap();
            assert_eq!(actual.len(), expected.len() * 4);
            for (expected, &[r, g, b, a]) in expected.into_iter().zip(actual.as_chunks::<4>().0) {
                assert!(r.abs_diff(expected) <= 1 && r == g && r == b && a == 255);
            }
            let gray = |x: u32, y: u32| actual[((y * SIZE + x) * 4) as usize];
            assert_eq!(gray(SIZE / 2, SIZE / 6) > gray(5 * SIZE / 6, SIZE / 6), tolerance > 0.0);
            assert_eq!(gray(SIZE / 6, SIZE / 6) > gray(5 * SIZE / 6, SIZE / 6), std::ptr::eq(baked, &bake));
        }
    }

    #[test]
    fn linked_paint_resamples_the_source_and_uses_the_instance_mask_override() {
        use texturecraft_compose::eval::{Composite, Inputs, all_tiles, composite_tiles, mask_image};
        use texturecraft_compose::tile_bytes;
        use texturecraft_texel::to_u16_signed;

        let Ok(mut gpu) = Headless::new() else { return };
        const SOURCE: u32 = 128;
        const SIZE: u32 = 256;
        let channels = [Channel::BaseColor, Channel::Height];
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let mut project = Project::new("Linked paint", "builtin:plane");
        project.texture_sets.extend([TextureSet::new("Source", 0, SOURCE), TextureSet::new("Instance", 1, SIZE)]);
        let source = project.alloc_id();
        let source_mask = project.alloc_id();
        let owner = project.alloc_id();
        let override_mask = project.alloc_id();
        let mut layer = Layer::new(source, "Source layer", LayerKind::Paint, &channels);
        layer.visible = false;
        layer.geometry_mask = Some(Vec::new());
        let mut inherited = Mask::new(MaskBackground::Black);
        inherited.effects.push(Effect::new(source_mask, EffectKind::Paint));
        layer.mask = Some(inherited);
        project.insert_layer(0, layer, None, false).unwrap();
        let instance = Layer::new(owner, "Linked layer", LayerKind::Instance { source }, &[]);
        project.insert_layer(1, instance, None, false).unwrap();

        let mut pixels = PixelStore::default();
        for y in 55..74 {
            for x in 59..69 {
                pixels.layer_mut(source, Channel::BaseColor, (SOURCE, SOURCE)).set(x, y, &[38_000, 3_000, 2_000, 45_000]);
                pixels.layer_mut(source, Channel::Height, (SOURCE, SOURCE)).set(x, y, &[to_u16_signed(-0.36), 45_000]);
                if x < 64 {
                    pixels.mask_mut(source_mask, (SOURCE, SOURCE)).set(x, y, &[51_000, 62_000]);
                }
            }
        }
        for y in 114..144 {
            for x in 117..125 {
                pixels.mask_mut(override_mask, (SIZE, SIZE)).set(x, y, &[0, 65_535]);
            }
        }
        let mut own = Mask::new(MaskBackground::White);
        own.effects.push(Effect::new(override_mask, EffectKind::Paint));
        let mut disabled = own.clone();
        disabled.enabled = false;

        let baked = BakedMaps::default();
        let images = ImageStore::default();
        let tiles = all_tiles(SIZE, SIZE);
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SOURCE, SOURCE), 0), ((SIZE, SIZE), 1)], 2);
        for (case, mask) in [None, Some(own), Some(disabled)].into_iter().enumerate() {
            project.layer_mut(owner).unwrap().mask = mask;
            let inputs = Inputs {
                project: &project,
                ts: 1,
                mesh: &mesh,
                gbuf: &gbuf,
                pixels: &pixels,
                baked: &baked,
                block_scales: project.texture_sets[1].block_scales(),
                preview: None,
                images: &images,
                bounds: mesh.bounds(),
            };
            let mut oracle = Composite::new(&channels, SIZE);
            composite_tiles(&inputs, &mut oracle, &tiles);
            let mut program = CompositeProgram::try_compile_region(&project, &pixels, 1, None).unwrap();
            program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 1, &program).unwrap();
            for kind in [DisplayKind::BaseColor, DisplayKind::Height] {
                let texture = gpu.renderer.display_textures()[1][kind as usize];
                let (_, _, actual) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap();
                let bytes = kind.bytes_per_texel();
                for &(x, y) in &tiles {
                    let expected = tile_bytes(&oracle, &baked, kind, x, y);
                    for (row, expected) in expected.chunks_exact(TILE as usize * bytes).enumerate() {
                        let offset = ((y * TILE + row as u32) * SIZE + x * TILE) as usize * bytes;
                        let actual = &actual[offset..offset + expected.len()];
                        if bytes == 8 {
                            for (actual, expected) in actual.as_chunks::<2>().0.iter().zip(expected.as_chunks::<2>().0) {
                                let actual = half::f16::from_le_bytes(*actual).to_f32();
                                let expected = half::f16::from_le_bytes(*expected).to_f32();
                                assert!((actual - expected).abs() < 0.001, "linked height case {case}: GPU {actual} vs CPU {expected}");
                            }
                        } else {
                            assert!(actual.iter().zip(expected).all(|(a, b)| a.abs_diff(*b) <= 1), "linked base color case {case} tile ({x}, {y}) row {row}");
                        }
                    }
                }
                if kind == DisplayKind::BaseColor {
                    let red = |x, y| actual[((y * SIZE + x) * 4) as usize];
                    assert_eq!(red(122, 128).cmp(&red(134, 128)), [std::cmp::Ordering::Greater, std::cmp::Ordering::Less, std::cmp::Ordering::Equal][case]);
                }
            }
            let mask = CompositeProgram::try_compile_mask_preview(&project, &pixels, 1, owner, None, None);
            if let Some(expected) = mask_image(&inputs, owner, SIZE) {
                let mut mask = mask.unwrap();
                mask.set_surface_inputs(&mesh, &gbuf, None).unwrap();
                gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 1, &mask).unwrap();
                let (_, _, actual) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.aux_texture(1).unwrap()).unwrap();
                assert_eq!(actual.len(), expected.len() * 4);
                for (expected, &[r, g, b, a]) in expected.into_iter().zip(actual.as_chunks::<4>().0) {
                    assert!(r.abs_diff(expected) <= 1 && r == g && r == b && a == 255, "linked mask case {case}");
                }
            } else {
                assert!(mask.is_err());
            }
        }
    }

    #[test]
    fn geometry_and_height_masks_with_compare_match_cpu() {
        use texturecraft_compose::display::tile_bytes;
        use texturecraft_compose::eval::{Composite, Inputs, composite_tiles, mask_image};
        use texturecraft_core::HeightMask;
        use texturecraft_geom::{SubMesh, glam::Vec2};

        let Ok(mut gpu) = Headless::new() else { return };
        const SIZE: u32 = TILE;
        let channels = [Channel::BaseColor, Channel::Height];
        let mut mesh = primitives::plane(1.0, 2);
        for uv in &mut mesh.uvs {
            *uv = *uv * 0.75 + Vec2::splat(0.125);
        }
        let original = mesh.submeshes[0].clone();
        mesh.submeshes = [("Shared part", 0, 3), ("Folder only", 3, 3), ("Layer only", 6, original.count - 6)]
            .into_iter()
            .map(|(name, start, count)| SubMesh { name: name.into(), start, count, ..original.clone() })
            .collect();
        let mesh = Arc::new(mesh);
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let [shared, folder_only, layer_only, uncovered] = [0, 1, 2, NO_TRI].map(|id| gbuf.tri.iter().position(|&tri| tri == id).unwrap());

        let mut project = Project::new("Layer mask constraints", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
        let folder_id = project.alloc_id();
        let mut folder = Layer::new(folder_id, "Folder", LayerKind::folder(), &project.texture_sets[0].channel_list());
        for blend in folder.channels.values_mut() {
            blend.mode = BlendMode::Passthrough;
        }
        folder.geometry_mask = Some(vec!["Shared part".into(), "Folder only".into()]);
        project.insert_layer(0, folder, None, false).unwrap();

        let owner = project.alloc_id();
        let comparison = project.alloc_id();
        let fill = Fill {
            sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.8, 0.45, 0.1])), (Channel::Height, Source::Uniform([0.35; 3]))]),
            ..Default::default()
        };
        let mut layer = Layer::new(owner, "Surface", LayerKind::Fill(fill), &channels);
        layer.height_mask = HeightMask { enabled: true, contrast: 0.9, offset: 0.1 };
        layer.geometry_mask = Some(vec!["Shared part".into(), "Layer only".into()]);
        layer.content_effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Invert)));
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(
            project.alloc_id(),
            EffectKind::Fill { source: Source::Uniform([0.375; 3]), projection: Projection::default(), sampling: Sampling::SAVED_BEFORE },
        ));
        let mut compare = Effect::new(
            comparison,
            EffectKind::Compare {
                op: CompareOp::LesserThan,
                value: 0.5,
                tolerance: 0.0,
                left: CompareSource::ThisMask,
                right: CompareSource::Constant,
                channel: Channel::Height,
                hardness: 1.0,
            },
        );
        compare.opacity = 0.5;
        compare.blend = BlendMode::Multiply;
        mask.effects.push(compare);
        layer.mask = Some(mask);
        project.insert_layer(0, layer, Some(folder_id), true).unwrap();

        let pixels = PixelStore::default();
        let baked = BakedMaps::default();
        let images = ImageStore::default();
        let cases = [
            (CompareOp::LesserThan, 0.5, 0.0, true, true),
            (CompareOp::LesserThan, 0.375, 0.0, false, true),
            (CompareOp::GreaterThan, 0.25, 0.0, true, true),
            (CompareOp::GreaterThan, 0.375, 0.0, false, true),
            (CompareOp::WithinTolerance, 0.25, 0.125, true, true),
            (CompareOp::WithinTolerance, 0.25, 0.0625, false, true),
            (CompareOp::WithinTolerance, 0.25, 0.125, true, false),
        ];
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        let mut red_with_height = 0;
        for (op, value, tolerance, passes, height_enabled) in cases {
            let layer = project.layer_mut(owner).unwrap();
            layer.channels.get_mut(&Channel::Height).unwrap().mode = if height_enabled { BlendMode::Normal } else { BlendMode::Disable };
            layer.mask.as_mut().unwrap().effects[1].kind = EffectKind::Compare {
                op,
                value,
                tolerance,
                left: CompareSource::ThisMask,
                right: CompareSource::Constant,
                channel: Channel::Height,
                hardness: 1.0,
            };
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
            let expected = tile_bytes(&oracle, &baked, DisplayKind::BaseColor, 0, 0);
            let mut program = CompositeProgram::try_compile_region(&project, &pixels, 0, None).unwrap();
            program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
            let texture = gpu.renderer.display_textures()[0][DisplayKind::BaseColor as usize];
            let (_, _, actual) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap();
            assert_eq!(actual.len(), expected.len());
            assert!(actual.iter().zip(&expected).all(|(a, b)| a.abs_diff(*b) <= 1), "material: {op:?} {value} ± {tolerance}, height {height_enabled}");
            let red = |index| actual[index * 4];
            assert_eq!(red(folder_only), red(uncovered));
            assert_eq!(red(layer_only), red(uncovered));
            assert!(red(shared) < red(uncovered));
            if passes && height_enabled {
                red_with_height = red(shared);
            }
            if !height_enabled {
                assert!(red(shared) < red_with_height, "signed height after Invert reduces the mask's effect on red");
            }

            let expected = mask_image(&inputs, owner, SIZE).unwrap();
            let mut mask = CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, owner, None, None).unwrap();
            mask.set_surface_inputs(&mesh, &gbuf, None).unwrap();
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &mask).unwrap();
            let (_, _, actual) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.aux_texture(0).unwrap()).unwrap();
            assert_eq!(actual.len(), expected.len() * 4);
            for (&expected, &[r, g, b, a]) in expected.iter().zip(actual.as_chunks::<4>().0) {
                assert!(r.abs_diff(expected) <= 1 && r == g && r == b && a == 255, "mask: {op:?} {value} ± {tolerance}");
            }
            // The comparison multiplies: a pass keeps the fill's 0.375, a fail halves it at opacity 0.5
            assert_eq!(expected[shared], if passes { 96 } else { 48 });
            assert_eq!(expected[shared], expected[layer_only]);
            assert_eq!(expected[uncovered], 0);
        }
    }

    #[test]
    fn nested_group_masks_blend_children_over_the_incoming_material() {
        use texturecraft_compose::display::tile_bytes;
        use texturecraft_compose::eval::{Composite, Inputs, composite_tiles};
        use texturecraft_core::HeightMask;

        let Ok(mut gpu) = Headless::new() else { return };
        const SIZE: u32 = TILE;
        let channels = [Channel::BaseColor, Channel::Roughness, Channel::Normal, Channel::Height];
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let mut project = Project::new("Nested material folders", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
        let below = project.alloc_id();
        let below_sources = BTreeMap::from([
            (Channel::BaseColor, Source::Uniform([0.28, 0.47, 0.63])),
            (Channel::Roughness, Source::Uniform([0.26; 3])),
            (Channel::Height, Source::Uniform([0.32; 3])),
        ]);
        let fill = |id, name, sources| Layer::new(id, name, LayerKind::Fill(Fill { sources, ..Fill::default() }), &channels);
        project.insert_layer(0, fill(below, "Under the folders", below_sources), None, false).unwrap();

        let outer = project.alloc_id();
        let inner = project.alloc_id();
        let group = |id, name| Layer::new(id, name, LayerKind::folder(), &channels);
        let mut folder = group(outer, "Masked folder");
        for blend in folder.channels.values_mut() {
            // The CPU uses the group's channel opacity with the completed inner
            // stack. Modes other than Disable do not blend it a second time.
            blend.mode = BlendMode::Screen;
            blend.opacity = 0.68;
        }
        folder.channels.get_mut(&Channel::Roughness).unwrap().mode = BlendMode::Disable;
        folder.height_mask = HeightMask { enabled: true, contrast: 0.4, offset: 0.1 };
        folder.geometry_mask = Some(vec![mesh.submeshes[0].name.clone()]);
        let mut mask = Mask::new(MaskBackground::Black);
        let position = Generator { direction: [1.0, 0.0, 0.0], softness: 0.18, ..Generator::new(GeneratorKind::Position) };
        mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Generator(position)));
        mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 32.0, wrap: false })));
        folder.mask = Some(mask);
        project.insert_layer(0, folder, None, false).unwrap();

        let mut folder = group(inner, "Translucent inner folder");
        for blend in folder.channels.values_mut() {
            blend.mode = BlendMode::Passthrough;
            blend.opacity = 0.54;
        }
        project.insert_layer(0, folder, Some(outer), true).unwrap();

        let child = project.alloc_id();
        let child_sources = BTreeMap::from([
            (Channel::BaseColor, Source::Uniform([0.35, 0.72, 0.48])),
            (Channel::Roughness, Source::Uniform([0.84; 3])),
            (Channel::Normal, Source::Uniform([0.66, 0.5, 0.97])),
            (Channel::Height, Source::Uniform([-0.52; 3])),
        ]);
        let mut child = fill(child, "Multiply over the material below", child_sources);
        child.channels.get_mut(&Channel::BaseColor).unwrap().mode = BlendMode::Multiply;
        project.insert_layer(0, child, Some(inner), true).unwrap();

        let pixels = PixelStore::default();
        let baked = BakedMaps::default();
        let images = ImageStore::default();
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
        let mut program = CompositeProgram::try_compile_region(&project, &pixels, 0, None).unwrap();
        assert_eq!(program.groups.depth, 2);
        program.set_surface_inputs(&mesh, &gbuf, Some(&baked)).unwrap();
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
        for kind in [DisplayKind::BaseColor, DisplayKind::Orm, DisplayKind::Normal, DisplayKind::Height] {
            let expected = tile_bytes(&oracle, &baked, kind, 0, 0);
            let texture = gpu.renderer.display_textures()[0][kind as usize];
            let (_, _, actual) = Readback::blocking_level(&gpu.device, &gpu.queue, texture, 0).unwrap();
            assert_eq!(actual.len(), expected.len());
            if kind.bytes_per_texel() == 8 {
                for (a, b) in actual.as_chunks::<2>().0.iter().zip(expected.as_chunks::<2>().0) {
                    let a = half::f16::from_le_bytes(*a).to_f32();
                    let b = half::f16::from_le_bytes(*b).to_f32();
                    assert!((a - b).abs() <= 0.001, "{kind:?} GPU {a}, CPU {b}");
                }
            } else {
                for (index, (a, b)) in actual.iter().zip(&expected).enumerate() {
                    assert!(a.abs_diff(*b) <= 1, "{kind:?} at byte {index}: GPU {a}, CPU {b}");
                }
            }
        }
        assert!(oracle.value(Channel::BaseColor, SIZE - 2, SIZE / 2)[0] < oracle.value(Channel::BaseColor, 1, SIZE / 2)[0]);
        assert!((oracle.value(Channel::Roughness, SIZE - 2, SIZE / 2)[0] - 0.26).abs() < 1e-4);
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
        let filter = Filter::BakedLighting { azimuth: 0.0, elevation: 20.0, light: 0.45, ambient: 0.45, occlusion: 1.0, stylized: None };
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
    fn curvature_detail_modes_match_cpu_with_scaled_uv_coverage_and_missing_bake() {
        use texturecraft_compose::eval::{Inputs, mask_image};

        let Ok(mut gpu) = Headless::new() else { return };
        const SIZE: u32 = 64;
        const BAKE_SIZE: u32 = 256;
        let mut mesh = primitives::plane(1.0, 1);
        for uv in &mut mesh.uvs {
            *uv = *uv * 0.75 + texturecraft_geom::glam::Vec2::splat(0.125);
        }
        let mesh = Arc::new(mesh);
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let curvature = (0..BAKE_SIZE * BAKE_SIZE)
            .map(|i| {
                let x = i % BAKE_SIZE;
                let y = i / BAKE_SIZE;
                if !(32..224).contains(&x) || !(32..224).contains(&y) {
                    1.0
                } else {
                    let phase = (x as f32 - 128.0) * 0.065 + y as f32 * 0.017;
                    0.5 + 0.03 * phase.sin()
                }
            })
            .collect();
        let baked = BakedMaps { resolution: BAKE_SIZE, maps: BTreeMap::from([(BakedMap::Curvature, Arc::new(curvature))]) };
        let absent = BakedMaps::default();
        let pixels = PixelStore::default();
        let images = ImageStore::default();
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((SIZE, SIZE), 0)], 1);
        // Changing the mode or generator shares the blurred bake. Removing the bake
        // at the end must go back to neutral curvature even after these cached reads.
        for bake in [&baked, &absent] {
            for kind in [GeneratorKind::Curvature, GeneratorKind::EdgeWear] {
                let mut mode_checks = Vec::new();
                for mode in CurvatureMode::ALL {
                    let mut project = Project::new("Curvature details", "builtin:plane");
                    project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
                    let owner = project.alloc_id();
                    let mut layer = Layer::new(owner, "Curvature mask", LayerKind::Fill(Fill::default()), &[Channel::BaseColor]);
                    let mut mask = Mask::new(MaskBackground::Black);
                    let generator = Generator {
                        detail: Some(CurvatureDetail { mode, weights: [0.16, 0.03, 0.16, 0.1, 0.28, 0.36, 0.46] }),
                        level: 0.74,
                        softness: 0.4,
                        noise_amount: 0.43,
                        noise_scale: 0.85,
                        seed: 27,
                        ..Generator::new(kind)
                    };
                    mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Generator(generator)));
                    layer.mask = Some(mask);
                    project.insert_layer(0, layer, None, false).unwrap();
                    let mut program = CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, owner, None, None).unwrap();
                    let inputs = Inputs {
                        project: &project,
                        ts: 0,
                        mesh: &mesh,
                        gbuf: &gbuf,
                        pixels: &pixels,
                        baked: bake,
                        block_scales: project.texture_sets[0].block_scales(),
                        preview: None,
                        images: &images,
                        bounds: mesh.bounds(),
                    };
                    let expected = mask_image(&inputs, owner, SIZE).unwrap();
                    program.set_surface_inputs(&mesh, &gbuf, Some(bake)).unwrap();
                    gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
                    let texture = gpu.renderer.aux_texture(0).unwrap();
                    let (_, _, rgba) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap();
                    assert_eq!(rgba.len(), expected.len() * 4);
                    for (index, (&cpu, texel)) in expected.iter().zip(rgba.as_chunks::<4>().0).enumerate() {
                        assert!(texel[0].abs_diff(cpu) <= 2, "{kind:?} {mode:?} curvature at {index}, bake {}: GPU {}, CPU {cpu}", bake.resolution, texel[0]);
                    }
                    if bake.resolution == BAKE_SIZE {
                        mode_checks.push(expected);
                    }
                }
                for (index, mode) in mode_checks.iter().enumerate() {
                    assert!(mode_checks[..index].iter().all(|prior| prior != mode));
                }
            }
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
        let triplanar =
            Projection::Triplanar { scale: 0.65, sharpness: 6.0, offset: [-0.23, 0.12, 0.43], uv: Default::default(), projector: Default::default() };
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
            Projection::Planar { scale: 0.8, axis: 1, offset: [0.03, 0.0, 0.34], uv: Default::default(), projector: Default::default(), placed: None },
            Projection::Spherical { scale: 0.9, uv: Default::default(), projector: Default::default(), offset: [0.0; 3] },
            Projection::Cylindrical { scale: 0.7, axis: 2, uv: Default::default(), projector: Default::default(), offset: [0.0; 3], angle: 360.0 },
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
                mask.effects
                    .push(Effect::new(project.alloc_id(), EffectKind::Fill { source, projection: projection.clone(), sampling: Sampling::SAVED_BEFORE }));
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
        let image = |rgba| Arc::new(ImageRes::new(2, 2, rgba, false));
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
                Projection::Triplanar { scale: 0.65, sharpness: 6.0, offset: [-0.23, 0.12, 0.43], uv: Default::default(), projector: Default::default() }
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
                mask.effects
                    .push(Effect::new(project.alloc_id(), EffectKind::Fill { source, projection: projection.clone(), sampling: Sampling::SAVED_BEFORE }));
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
        let image = |rgba, srgb| Arc::new(ImageRes::new(2, 2, rgba, srgb));
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
            Projection::Triplanar { scale: 0.63, sharpness: 2.1, offset: [0.14, -0.27, 0.31], uv: Default::default(), projector: Default::default() },
            Projection::Planar { scale: 1.4, axis: 0, offset: [0.04, 0.23, 0.18], uv: Default::default(), projector: Default::default(), placed: None },
            Projection::Planar { scale: 0.9, axis: 1, offset: [-0.18, -0.34, 1.03], uv: Default::default(), projector: Default::default(), placed: None },
            Projection::Planar { scale: 1.1, axis: 2, offset: [0.65, -0.26, -0.31], uv: Default::default(), projector: Default::default(), placed: None },
            Projection::Spherical { scale: 1.09, uv: Default::default(), projector: Default::default(), offset: [0.0; 3] },
            Projection::Cylindrical { scale: 1.21, axis: 0, uv: Default::default(), projector: Default::default(), offset: [0.0; 3], angle: 360.0 },
            Projection::Cylindrical { scale: 2.19, axis: 1, uv: Default::default(), projector: Default::default(), offset: [0.0; 3], angle: 360.0 },
            Projection::Cylindrical { scale: 0.87, axis: 2, uv: Default::default(), projector: Default::default(), offset: [0.0; 3], angle: 360.0 },
            Projection::decal(decal),
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
                sampling: Sampling::SAVED_BEFORE,
                ..Fill::default()
            };
            let owner = project.alloc_id();
            let mut layer = Layer::new(owner, "Material images", LayerKind::Fill(fill), &channels);
            let mut mask = Mask::new(MaskBackground::Black);
            let mask_source = Source::image("normal");
            mask.effects.push(Effect::new(
                project.alloc_id(),
                EffectKind::Fill { source: mask_source, projection: projection.clone(), sampling: Sampling::SAVED_BEFORE },
            ));
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
            groups: GroupPlan::default(),
            geometry: GeometryMasks::default(),
            operations: Vec::new(),
            layer_count: 0,
            paint: vec![paint],
            ramp_words: Vec::new(),
            curvature_sizes: [false; CurvatureDetail::BLURRED_SIZES],
            tile_patterns: TilePatterns::default(),
            material_images: MaterialImages::default(),
            brush_alpha: None,
            brush_stencil: None,
            region: None,
            coverage: None,
            surface: None,
            needs_surface: false,
            baked: None,
            id_bake: None,
            display_outputs: 0,
            glossiness: false,
            mask_preview: false,
            mask_layer: 0,
            mask_layers: BTreeMap::new(),
        };
        let region = CompositeRegion { min: [0, 0], max: [128, 128] };
        let limit = 6 * 64 * 64 * 2 + 32;
        let windows = paint_windows(&program, region, (256, 256), 1, 8, limit).unwrap();
        assert!(windows.len() > 1);
        let mut coverage = vec![0; 128 * 128];
        for (chunk, window, words) in windows {
            assert!(words <= limit);
            let [origin_x, origin_y, columns, rows] = window.params();
            let mut packed = Vec::new();
            window.pack(&program.paint, words, &mut packed);
            assert_eq!(packed.len(), words);
            for y in chunk.min[1]..chunk.max[1] {
                for x in chunk.min[0]..chunk.max[0] {
                    coverage[(y * 128 + x) as usize] += 1;
                    for sy in [y.saturating_mul(2).saturating_sub(8), (2 * y + 8).min(255)] {
                        for sx in [x.saturating_mul(2).saturating_sub(8), (2 * x + 8).min(255)] {
                            assert!(sx / 64 >= origin_x && sy / 64 >= origin_y);
                            let (tx, ty) = (sx / 64 - origin_x, sy / 64 - origin_y);
                            assert!(tx < columns && ty < rows);
                            let slot = (ty * columns + tx) as usize;
                            assert_ne!(packed[packed[0] as usize + slot], 0);
                        }
                    }
                }
            }
        }
        assert!(coverage.into_iter().all(|count| count == 1));
    }
}
