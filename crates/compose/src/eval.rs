//! The layer-stack evaluator: composites a texture set's stack into one image per channel,
//! tile by tile in parallel. This is the reference implementation (the oracle) for the viewport
//! and for export.

pub(crate) mod thumbnail;

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use rayon::prelude::*;
use texturecraft_core::{
    BakedMap, BlendMode, BlockScales, Channel, ChannelKind, CompareOp, CurvatureDetail, CurvatureMode, Effect, EffectKind, Fill, Filter, Generator,
    GeneratorKind, GeometryMask, GraphInput, GraphRef, HeightMask, Layer, LayerId, LayerKind, Mask, MaskBackground, MicroDetail, NormalFormat,
    PlacedProjection, Project, Projection, Source, TextureSet, TileGrid, blend1, blend3, lum, mix3, per_unit,
    rules::{BlendRules, mix_over},
};
use texturecraft_geom::glam::{Vec2, Vec3};
use texturecraft_geom::{Mesh, NO_TRI, UvGBuffer};
use texturecraft_texel::{DilationMap, TILE, TILE_AREA, TileImage, from_u16, noise, to_u16, to_u16_signed};

use crate::below::{Band, BelowCache, Level, StackKey};
use crate::curvature_sizes::CurvatureSizes;
use crate::field_cache::FieldCache;
use crate::graphs::{self, Cooked, GraphStore, GraphUse, RenderEnv};
use crate::held_fields::HeldFields;
use crate::pattern::{Coords, decal_uv_symmetric, image_samples_with_rules, pattern_pixels, procedural_in, world_noise};
use crate::pattern_context::PatternContext;
use crate::placed::{self, Taps};
use crate::store::{BakedMaps, ImageStore, PixelStore, decode};
use texturecraft_graph::tex::Tex;

const NCH: usize = Channel::ALL.len();

/// Stack values being composited, per channel, with the coverage each holds. Only the measured blend rules
/// ([`BlendRules::Measured`]) read the coverage: there a layer's mode applies only where something is below, and a
/// folder that does not pass through composites its children from nothing.
#[derive(Clone, Copy)]
struct Acc {
    v: [[f32; 3]; NCH],
    cov: [f32; NCH],
}

impl Acc {
    /// The channel defaults, covering nothing.
    fn defaults() -> Acc {
        Acc { v: Channel::ALL.map(Channel::default_value), cov: [0.0; NCH] }
    }

    fn get(&self, ch: usize) -> Option<&[f32; 3]> {
        self.v.get(ch)
    }

    /// What an anchor point inside a mask records: the mask's value on every channel, fully covered.
    fn uniform(value: f32) -> Acc {
        Acc { v: [[value; 3]; NCH], cov: [1.0; NCH] }
    }

    fn get_mut(&mut self, ch: usize) -> Option<&mut [f32; 3]> {
        self.v.get_mut(ch)
    }

    /// What an effect layer's channel `c` starts from ([`starts_from_below`]): the stack's shown value and its
    /// coverage. None on other channels.
    fn below(&self, c: &ChanPlan) -> Option<([f32; 3], f32)> {
        if !c.from_below {
            return None;
        }
        Some((*self.v.get(c.ch)?, self.cov.get(c.ch).copied().unwrap_or(0.0)))
    }
}

/// What a live stroke paints into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PaintTarget {
    /// A paint layer's channels.
    Layer(LayerId),
    /// A paint effect inside a mask.
    Mask(LayerId),
}

/// A screen-pinned image painted through the camera (projection tool / stencil).
#[derive(Clone, Debug)]
pub struct Stencil {
    pub image: std::sync::Arc<crate::store::ImageRes>,
    /// Camera view-projection captured when the stroke started.
    pub view_proj: texturecraft_geom::glam::Mat4,
    /// Image placement in normalized device coordinates: center, half extents, rotation.
    pub center: [f32; 2],
    pub half: [f32; 2],
    pub rotation: f32,
    /// Repeat the image beyond its bounds (otherwise nothing is painted outside it).
    pub tile: bool,
}

impl Stencil {
    /// Linear color and alpha of the stencil at world point `p` (None outside the image).
    pub fn sample_world(&self, p: Vec3) -> Option<([f32; 3], f32)> {
        let c = self.view_proj * p.extend(1.0);
        if c.w <= 1e-6 {
            return None;
        }
        let (dx, dy) = (c.x / c.w - self.center[0], c.y / c.w - self.center[1]);
        let (sn, cs) = (-self.rotation).sin_cos();
        let lx = (dx * cs - dy * sn) / self.half[0].max(1e-6);
        let ly = (dx * sn + dy * cs) / self.half[1].max(1e-6);
        if !self.tile && (lx.abs() > 1.0 || ly.abs() > 1.0) {
            return None;
        }
        let s = self.image.sample(lx * 0.5 + 0.5, 0.5 - ly * 0.5);
        let rgb = if self.image.srgb { [s[0], s[1], s[2]].map(texturecraft_texel::srgb_to_linear) } else { [s[0], s[1], s[2]] };
        Some((rgb, s[3].clamp(0.0, 1.0)))
    }
}

/// The channel under which a clone or smudge stroke on a mask carries its mask values (the mask
/// has no channels of its own; any scalar channel works).
pub const MASK_CHANNEL: Channel = Channel::Roughness;

/// The stroke being drawn, shown before it is committed.
#[derive(Clone, Debug)]
pub struct StrokePreview {
    pub target: PaintTarget,
    /// Stroke alpha per texel (1 component, 0..1).
    pub alpha: TileImage,
    /// Stroke opacity cap.
    pub opacity: f32,
    pub erase: bool,
    /// Values painted per channel (layer targets).
    pub values: BTreeMap<Channel, [f32; 3]>,
    /// Gray value painted into a mask.
    pub mask_value: f32,
    /// Projection tool: base color (and alpha) come from a screen-pinned image.
    pub stencil: Option<Stencil>,
    /// Clone and smudge: straight values per channel carried by the stroke (replace `values`).
    pub carried: BTreeMap<Channel, TileImage>,
    /// Mask coverage sampled by a mask pickup.
    pub carried_mask: Option<TileImage>,
}

/// Everything the evaluator reads.
pub struct Inputs<'a> {
    pub project: &'a Project,
    pub ts: usize,
    pub mesh: &'a Mesh,
    pub gbuf: &'a UvGBuffer,
    pub pixels: &'a PixelStore,
    pub baked: &'a BakedMaps,
    pub block_scales: BlockScales,
    pub preview: Option<&'a StrokePreview>,
    /// Imported image resources (fill sources).
    pub images: &'a ImageStore,
    /// Graph texts and cooked graph outputs (graph sources and effects).
    pub graphs: &'a GraphStore,
    /// Mesh bounds, for scale-independent procedural coordinates.
    pub bounds: (Vec3, Vec3),
}

impl Inputs<'_> {
    /// The UV tile grid of a UV-tiled set (the mesh's UVs for this set are in atlas space).
    fn tile_grid(&self) -> Option<TileGrid> {
        self.project.texture_set(self.ts).ok().and_then(TextureSet::tile_grid)
    }

    /// Texels per atlas block across (the whole width for a classic set): filter radii scale
    /// with it, and with the tile's share of its block.
    fn tile_res(&self) -> u32 {
        self.gbuf.width / self.tile_grid().map_or(1, |g| g.cols.max(1))
    }

    /// The current UV tile can occupy only part of its atlas block
    fn tile_res_at(&self, x: u32, y: u32) -> u32 {
        ((self.tile_res() as f32 * self.block_scales.at(x, y)).round() as u32).max(1)
    }

    /// What graphs of this set are rendered with.
    fn render_env(&self) -> RenderEnv<'_> {
        RenderEnv { baked: self.baked, images: self.images, side: RenderEnv::side_for((self.gbuf.width, self.gbuf.height)) }
    }

    /// The cook graph `g` shows for layer or effect `id` (read for `channel`); None for a graph that reads the stack
    /// (computed with the neighbourhood fields) or that could not be rendered.
    fn static_cook(&self, id: LayerId, channel: Option<Channel>, g: &GraphRef) -> Option<Arc<Cooked>> {
        if g.reads_stack() {
            return None;
        }
        self.graphs.cook_for(GraphUse { set: self.ts, id, channel }, g, &self.render_env(), &[])
    }
}

/// The composited channels of one texture set.
#[derive(Clone, Debug, PartialEq)]
pub struct Composite {
    /// Width in texels (the resolution of a classic, square set).
    pub resolution: u32,
    pub height: u32,
    pub channels: BTreeMap<Channel, TileImage>,
    /// The set's [`TextureSet::block_scales`]: height slopes on the display follow each tile.
    pub block_scales: BlockScales,
}

impl Composite {
    /// A square composite.
    pub fn new(channels: &[Channel], resolution: u32) -> Composite {
        Composite::with_size(channels, resolution, resolution)
    }

    pub fn with_size(channels: &[Channel], width: u32, height: u32) -> Composite {
        Composite {
            resolution: width,
            height,
            channels: channels.iter().map(|c| (*c, TileImage::new(width, height, c.components()))).collect(),
            block_scales: BlockScales::default(),
        }
    }

    /// Channel value at a texel (the channel default when absent).
    pub fn value(&self, c: Channel, x: u32, y: u32) -> [f32; 3] {
        let Some(img) = self.channels.get(&c) else { return c.default_value() };
        let raw = img.get(x, y);
        let k = c.kind();
        if !img.is_allocated(x / TILE, y / TILE) {
            return c.default_value();
        }
        if c.components() == 3 { [decode(k, raw[0]), decode(k, raw[1]), decode(k, raw[2])] } else { [decode(k, raw[0]); 3] }
    }
}

/// Every tile of a `width × height` image.
pub fn all_tiles(width: u32, height: u32) -> Vec<(u32, u32)> {
    let (nx, ny) = (width.div_ceil(TILE), height.div_ceil(TILE));
    (0..ny).flat_map(|y| (0..nx).map(move |x| (x, y))).collect()
}

/// Tiles that hold at least one covered texel.
pub fn covered_tiles(gbuf: &UvGBuffer) -> Vec<(u32, u32)> {
    let (tw, th) = (gbuf.width.div_ceil(TILE), gbuf.height.div_ceil(TILE));
    let mut out = Vec::new();
    for ty in 0..th {
        for tx in 0..tw {
            let any = (ty * TILE..((ty + 1) * TILE).min(gbuf.height))
                .any(|y| (tx * TILE..((tx + 1) * TILE).min(gbuf.width)).any(|x| gbuf.tri.get((y * gbuf.width + x) as usize).is_some_and(|t| *t != NO_TRI)));
            if any {
                out.push((tx, ty));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Per-tile plan: the stack flattened with each layer's tile slices looked up once per tile.

struct ChanPlan<'a> {
    ch: usize,
    kind: ChannelKind,
    comps: usize,
    blend: BlendMode,
    opacity: f32,
    data: Option<Cow<'a, [u16]>>,
    /// Visible content effects, holding this channel's tile for paint effects (empty, and
    /// allocation free, for layers without content effects).
    effects: Vec<EffectPlan<'a>>,
    /// A content neighbourhood filter precomputed over the region: continue the effects after
    /// effect `.0` starting from the field's value.
    start: Option<(usize, &'a Field)>,
    /// The cook of a fill's graph source on this channel.
    graph: Option<Arc<Cooked>>,
    /// An effect layer's channel ([`starts_from_below`]): its content starts from the stack below it.
    from_below: bool,
    /// The project's blend rules.
    rules: BlendRules,
    /// The channel stores 0..1 (the measured rules clamp blends to it before the opacity).
    unit: bool,
}

enum Content<'a> {
    Paint { preview: bool },
    Fill(&'a Fill),
    Group(Vec<Node<'a>>),
}

struct EffectPlan<'a> {
    /// The effect id (an anchor point records under it).
    id: LayerId,
    kind: &'a EffectKind,
    blend: BlendMode,
    opacity: f32,
    data: Option<Cow<'a, [u16]>>,
    preview: bool,
    /// The curvature detail sizes a generator with detail reads.
    curvature_sizes: Option<Arc<CurvatureSizes>>,
    /// The cook of a graph effect, or of a fill effect's graph source on the channel planned.
    graph: Option<Arc<Cooked>>,
    /// The effect acts on the content channel planned ([`Effect::channels`]); always true in a mask.
    acts: bool,
}

struct MaskPlan<'a> {
    background: f32,
    effects: Vec<EffectPlan<'a>>,
    /// A neighbourhood filter (blur, sharpen) precomputed over the region: continue the stack
    /// after effect `.0` starting from the field's value.
    start: Option<(usize, &'a Field)>,
}

/// A layer's mask (one value per texel) or one content channel (straight value and coverage)
/// after its last neighbourhood filter, over a texel region.
pub struct Field {
    x0: u32,
    y0: u32,
    w: u32,
    h: u32,
    comps: usize,
    data: FieldData,
    /// A mask's anchor points below the filter, each with the mask value it recorded per texel of the region
    /// (unfiltered): a mask continued from the field records them again.
    marks: Marks,
    /// The same whatever tiles are composited, and exact (its graph was cooked): a composite may keep it for every
    /// block (see [`FieldCache`]).
    shared: bool,
}

/// Anchor points inside a mask with one recorded value per texel of a region.
type Marks = Vec<(LayerId, Vec<f32>)>;

/// A field's values: floats while it is made; once it covers the whole set, 16-bit steps of 0..1 when every value
/// lies there (see [`Field::compact`]).
enum FieldData {
    F32(Vec<f32>),
    U16(Vec<u16>),
}

impl FieldData {
    fn bytes(&self) -> usize {
        match self {
            FieldData::F32(d) => d.len() * 4,
            FieldData::U16(d) => d.len() * 2,
        }
    }
}

impl Field {
    /// Bytes the field holds.
    pub(crate) fn bytes(&self) -> usize {
        self.data.bytes() + self.marks.iter().map(|(_, v)| v.len()).sum::<usize>() * std::mem::size_of::<f32>()
    }

    /// Holds the values as 16-bit steps of 0..1 when every value lies there (a field that covers the whole set, one
    /// per layer and channel that reads the stack through a graph, is 268 MB at 4096 as floats): half the memory, each
    /// value within 1/131070 of what it was. A field with any value outside 0..1 stays floats.
    fn compact(&mut self) {
        if let FieldData::F32(d) = &self.data
            && d.iter().all(|v| (0.0..=1.0).contains(v))
        {
            self.data = FieldData::U16(d.iter().map(|v| (v * 65535.0).round() as u16).collect());
        }
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        if x < self.x0 || y < self.y0 || x >= self.x0 + self.w || y >= self.y0 + self.h {
            return None;
        }
        Some(((y - self.y0) * self.w + (x - self.x0)) as usize)
    }

    /// The texel's `comps` values (zeros past them).
    fn get(&self, x: u32, y: u32) -> Option<[f32; 4]> {
        let i = self.index(x, y)? * self.comps;
        let mut out = [0.0; 4];
        match &self.data {
            FieldData::F32(d) => out.iter_mut().zip(d.get(i..i + self.comps)?).for_each(|(o, v)| *o = *v),
            FieldData::U16(d) => out.iter_mut().zip(d.get(i..i + self.comps)?).for_each(|(o, v)| *o = f32::from(*v) / 65535.0),
        }
        Some(out)
    }

    /// The mask anchor points recorded below the filter at texel (x, y), with their values.
    fn marks_at(&self, x: u32, y: u32) -> impl Iterator<Item = (LayerId, f32)> + '_ {
        let i = self.index(x, y);
        self.marks.iter().filter_map(move |(id, values)| Some((*id, *values.get(i?)?)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldKey {
    Mask(LayerId),
    Content(LayerId, Channel),
}

pub type Fields = HashMap<FieldKey, Arc<Field>>;

/// How a neighbourhood filter reads around a texel.
#[derive(Clone, Copy)]
enum Op {
    /// Box blurs (≈ Gaussian), sharpening by the amount when given.
    Blur(Option<f32>),
    /// The value minus its box blur, around the channel's mid value.
    HighPass,
    /// Values ramped down towards their shape's edge, from the box blur.
    Bevel,
    /// The value plus its box blur times the amount.
    Glow(f32),
    /// The value at the center of its cell (`r` texels across).
    Mosaic,
    /// Average along a direction (unit vector in texels).
    Line([f32; 2]),
    /// Read at an offset given by a noise vector field, or by noise along a direction.
    Warp { scale: f32, seed: u32, dir: Option<[f32; 2]> },
    /// Gradient magnitude from central differences.
    Edge,
    /// Average downhill along a noise height field.
    Slope { scale: f32, seed: u32 },
    /// A graph reading the stack: rendered from the whole texture set ([`graph_region`]).
    Graph,
}

/// The last neighbourhood filter of an effect stack (see [`neighbourhood_filters`]).
fn neighbourhood(effects: &[Effect], res: u32) -> Option<(usize, f32, Op)> {
    neighbourhood_filters(effects, res, None).last()
}

/// Below this a coverage weight counts as no coverage at all.
const MIN_COVERAGE_WEIGHT: f32 = 1e-6;
/// Filter sizes are texels at this many texels per side (2K), scaled to the resolution they run at.
const FILTER_SIZE_RESOLUTION: f32 = 2048.0;
/// The smallest radius a filter runs at, in texels.
const MIN_FILTER_RADIUS: f32 = 0.5;
/// Sharpen's blur radius at [`FILTER_SIZE_RESOLUTION`], and the smallest it runs at, in texels.
const SHARPEN_RADIUS: f32 = 1.5;
const MIN_SHARPEN_RADIUS: f32 = 0.75;

/// The neighbourhood filters of an effect stack, bottom → top: index among the visible effects, radius in texels (`res` texels per UV tile across) and what it does.
fn neighbourhood_filters(effects: &[Effect], res: u32, channel: Option<Channel>) -> impl Iterator<Item = (usize, f32, Op)> + '_ {
    let texels = move |size: f32| (size * res as f32 / FILTER_SIZE_RESOLUTION).max(MIN_FILTER_RADIUS);
    effects.iter().filter(|effect| effect.visible).enumerate().filter(move |(_, effect)| channel.is_none_or(|channel| effect.acts_on(channel))).filter_map(
        move |(index, effect)| match &effect.kind {
            EffectKind::Filter(Filter::Blur { radius }) if *radius > 0.0 => Some((index, texels(*radius), Op::Blur(None))),
            EffectKind::Filter(Filter::Sharpen { amount }) => {
                Some((index, (SHARPEN_RADIUS * res as f32 / FILTER_SIZE_RESOLUTION).max(MIN_SHARPEN_RADIUS), Op::Blur(Some(*amount))))
            }
            EffectKind::Filter(Filter::DirectionalBlur { radius, angle }) if *radius > 0.0 => Some((index, texels(*radius), Op::Line(direction(*angle)))),
            EffectKind::Filter(Filter::Warp { intensity, scale, seed }) if *intensity > 0.0 => {
                Some((index, texels(*intensity), Op::Warp { scale: *scale, seed: *seed, dir: None }))
            }
            EffectKind::Filter(Filter::DirectionalWarp { intensity, angle, scale, seed }) if *intensity > 0.0 => {
                Some((index, texels(*intensity), Op::Warp { scale: *scale, seed: *seed, dir: Some(direction(*angle)) }))
            }
            EffectKind::Filter(Filter::HighPass { radius }) => Some((index, texels(*radius), Op::HighPass)),
            EffectKind::Filter(Filter::EdgeDetect { width }) => Some((index, texels(*width), Op::Edge)),
            // Three box passes ramp an edge over about twice their radius
            EffectKind::Filter(Filter::Bevel { width }) => Some((index, texels(*width * 0.5), Op::Bevel)),
            EffectKind::Filter(Filter::Glow { radius, amount }) => Some((index, texels(*radius), Op::Glow(*amount))),
            EffectKind::Filter(Filter::Mosaic { size }) => Some((index, texels(*size), Op::Mosaic)),
            // Independent of `res`, so every caller agrees on which effects are stages; the reach covers the cells, then the blur around them
            EffectKind::Generator(generator) if generator.global.blur > 0.0 || generator.resolution.can_reduce() => {
                Some((index, texels(generator.global.blur_radius()) + generator.resolution.largest_cell(res) as f32, Op::Blur(None)))
            }
            EffectKind::Filter(Filter::SlopeBlur { radius, scale, seed }) if *radius > 0.0 => {
                Some((index, texels(*radius), Op::Slope { scale: *scale, seed: *seed }))
            }
            // reads the whole set: the radius is clamped to the set's longest side
            EffectKind::Graph { graph, .. } if graph.reads_stack() => Some((index, f32::MAX, Op::Graph)),
            _ => None,
        },
    )
}

/// Whether any mask or content effect in a texture set reads neighbouring texels. Includes
/// filters, blurred generators and reduced-resolution generators. Hidden layers and disabled
/// masks count too, so partial invalidation remains conservative.
pub fn stack_reads_neighbours(project: &Project, texture_set_index: usize) -> bool {
    const NEIGHBOUR_PROBE_RESOLUTION: u32 = 2048;

    let Some(set) = project.texture_sets.get(texture_set_index) else {
        return false;
    };

    project.render_tree(&set.stack).iter().filter_map(|id| project.layers.get(id)).any(|layer| {
        neighbourhood(&layer.content_effects, NEIGHBOUR_PROBE_RESOLUTION).is_some()
            || layer.mask.as_ref().is_some_and(|mask| neighbourhood(&mask.effects, NEIGHBOUR_PROBE_RESOLUTION).is_some())
    })
}

/// Conservative output halo in tiles for an edit to painted pixels. Without anchors each
/// filtered field reads its own effect stack. Linked filtered fields use whole-set
/// invalidation until their combined dependency range is known.
pub fn painted_tile_margin(project: &Project, ts: usize, target: PaintTarget) -> Option<u32> {
    let set = project.texture_sets.get(ts)?;
    let (width, height) = set.size();
    let resolution = set.block_resolution();
    let (PaintTarget::Layer(id) | PaintTarget::Mask(id)) = target;
    let owner = project.effect(id).map_or(id, |(owner, _)| owner);
    let mut max_reach = 0;
    for layer in project.render_tree(&set.stack).iter().filter_map(|id| project.layers.get(id)) {
        if layer.reads_anchor() {
            return (!stack_reads_neighbours(project, ts)).then_some(0);
        }
        if owner == layer.id {
            let effects = match target {
                PaintTarget::Layer(_) => layer.content_effects.as_slice(),
                PaintTarget::Mask(_) => layer.mask.as_ref().map_or(&[][..], |mask| mask.effects.as_slice()),
            };
            max_reach = neighbourhood_filters(effects, resolution, None)
                .map(|(_, radius, op)| filter_extent(radius, op, width.max(height)).1)
                .fold(0u32, u32::saturating_add);
        }
    }
    Some(max_reach.div_ceil(TILE))
}

/// Unit texel-space vector of `angle` degrees. Texel rows run down: positive angles turn
/// counterclockwise on screen.
fn direction(angle: f32) -> [f32; 2] {
    let (sn, cs) = angle.to_radians().sin_cos();
    [cs, -sn]
}

/// Gain on the curvature map's own deviation from flat (±0.5 reaches black and white).
const MAP_GAIN: f32 = 2.0;

/// Gain per texel of a detail size: blurring spreads an edge over about twice its radius, so wider sizes need proportionally more to reach full strength.
const GAIN_PER_DETAIL_TEXEL: f32 = 2.0 * std::f32::consts::SQRT_2;

/// Samples along a directional or slope blur at most (beyond, samples spread out).
const LINE_SAMPLES: i32 = 24;

/// Fractal noise over texel position `(x, y)` (`res` texels per UV unit, `scale` features per
/// UV unit), shifted by `d` in noise space.
fn texel_noise(x: f32, y: f32, d: [f32; 2], res: u32, scale: f32, seed: u32) -> f32 {
    let k = if scale.is_finite() { scale.clamp(0.01, 1000.0) } else { 1.0 } / res.max(1) as f32;
    noise::fbm([x * k + d[0], y * k + d[1], 0.0], 3, seed, noise::perlin)
}

/// The neighbourhood filters that gather samples (directional and slope blur, warp) over a
/// field region `(x0, y0, w, h)` of premultiplied `orig` texels (`comps` each) and their
/// `cover`: bilinear samples weighted by coverage, so uncovered texels don't bleed in.
/// Positions are absolute texel coordinates, so a texel's result does not depend on the region.
#[allow(clippy::too_many_arguments)]
fn gather(op: Op, orig: &[f32], cover: &[f32], comps: usize, (x0, y0, w, h): (u32, u32, u32, u32), r: f32, res: u32, opacity: f32) -> Vec<f32> {
    let mut data = orig.to_vec();
    let (wu, hu) = (w as usize, h as usize);
    // Bilinear sample at absolute texel position (fx, fy) into `acc` (components, then weight).
    let sample = |fx: f32, fy: f32, acc: &mut [f32; 5]| {
        if !(fx.is_finite() && fy.is_finite()) {
            return;
        }
        let (bx, by) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - bx, fy - by);
        for (dx, dy, k) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
            let (cx, cy) = (bx as i64 + dx - i64::from(x0), by as i64 + dy - i64::from(y0));
            if k <= 0.0 || cx < 0 || cy < 0 || cx >= wu as i64 || cy >= hu as i64 {
                continue;
            }
            let i = cy as usize * wu + cx as usize;
            let (Some(c), Some(px)) = (cover.get(i), orig.get(i * comps..(i + 1) * comps)) else { continue };
            for (a, v) in acc.iter_mut().zip(px) {
                *a += v * k;
            }
            acc[4] += c * k;
        }
    };
    data.par_chunks_mut((comps * wu).max(1)).enumerate().for_each(|(row, line)| {
        for col in 0..wu {
            if cover.get(row * wu + col).is_none_or(|c| *c <= 0.0) {
                continue;
            }
            let (x, y) = ((x0 as usize + col) as f32, (y0 as usize + row) as f32);
            let mut acc = [0.0f32; 5];
            let n = (r.ceil() as i32).clamp(1, LINE_SAMPLES);
            let step = r / n as f32;
            match op {
                Op::Line(d) => {
                    for s in -n..=n {
                        let t = s as f32 * step;
                        sample(x + d[0] * t, y + d[1] * t, &mut acc);
                    }
                }
                Op::Warp { scale, seed, dir } => {
                    let off = |salt: u32| ((texel_noise(x, y, [0.0; 2], res, scale, seed ^ salt) - 0.5) * 3.0).clamp(-1.0, 1.0) * r;
                    match dir {
                        Some(d) => {
                            let t = off(0);
                            sample(x + d[0] * t, y + d[1] * t, &mut acc);
                        }
                        None => sample(x + off(0), y + off(0x5BD1_E995), &mut acc),
                    }
                }
                Op::Edge => {
                    let center = orig.get((row * wu + col) * comps..(row * wu + col + 1) * comps).unwrap_or(&[]);
                    let at = |dx: f32, dy: f32| {
                        let mut a = [0.0f32; 5];
                        sample(x + dx, y + dy, &mut a);
                        a
                    };
                    let (l, rt, u, d) = (at(-r, 0.0), at(r, 0.0), at(0.0, -r), at(0.0, r));
                    // A side outside the UV island reads the texel itself: seams are not edges.
                    let val = |a: &[f32; 5], k: usize| if a[4] > 1e-6 { a[k] / a[4] } else { center.get(k).copied().unwrap_or(0.0) };
                    let mag = |k: usize| (val(&rt, k) - val(&l, k)).hypot(val(&d, k) - val(&u, k)).min(1.0);
                    // Content stays premultiplied by its own coverage; masks are one value.
                    let cov = if comps == 4 { center.get(3).copied().unwrap_or(0.0) } else { 1.0 };
                    for (k, a) in acc.iter_mut().enumerate().take(comps.min(3)) {
                        *a = mag(k) * cov;
                    }
                    if comps == 4 {
                        acc[3] = cov;
                    }
                    acc[4] = 1.0;
                }
                Op::Slope { scale, seed } => {
                    let e = 0.01;
                    let hgt = |dx: f32, dy: f32| texel_noise(x, y, [dx, dy], res, scale, seed);
                    // Downhill; a flat spot keeps its value.
                    let d = Vec2::new(hgt(-e, 0.0) - hgt(e, 0.0), hgt(0.0, -e) - hgt(0.0, e)).try_normalize().unwrap_or(Vec2::ZERO);
                    for s in 0..=n {
                        let t = s as f32 * step;
                        sample(x + d.x * t, y + d.y * t, &mut acc);
                    }
                }
                Op::Mosaic => {
                    let center = |v: f32| ((v / r).floor() + 0.5) * r - 0.5;
                    sample(center(x), center(y), &mut acc);
                }
                Op::Blur(_) | Op::HighPass | Op::Bevel | Op::Glow(_) | Op::Graph => {}
            }
            if acc[4] > 1e-6
                && let Some(px) = line.get_mut(col * comps..(col + 1) * comps)
            {
                for (slot, a) in px.iter_mut().zip(acc) {
                    *slot += (a / acc[4] - *slot) * opacity;
                }
            }
        }
    });
    data
}

/// Fixed-point scale of the blur. Integer running sums make a texel's result independent of
/// the region it is computed in, so tiles composited in any grouping agree exactly.
const FIX: f64 = (1u64 << 24) as f64;

/// Texels one parallel task of a neighbourhood filter handles at least. Compositing in frame slices filters regions of a few tiles: split per texel,
/// handing that little work to every thread costs far more than the work itself. Each texel is computed on its own, so the split does not change results.
const MIN_PARALLEL_TEXELS: usize = 1 << 14;

/// How many lines of `length` texels make a parallel task of at least [`MIN_PARALLEL_TEXELS`].
fn lines_per_task(length: usize) -> usize {
    (MIN_PARALLEL_TEXELS / length.max(1)).max(1)
}

/// Three running-sum box blurs (≈ Gaussian) of a `width × height` image along its rows, then its columns, each in parallel.
pub(crate) fn blur(data: &mut [f32], width: usize, height: usize, radius: usize) {
    // Bound samples before fixed-point accumulation
    const MAX_INPUT_MAGNITUDE: f32 = 1e6;

    let to_fixed = |value: &f32| if value.is_finite() { (f64::from(value.clamp(-MAX_INPUT_MAGNITUDE, MAX_INPUT_MAGNITUDE)) * FIX).round() as i64 } else { 0 };
    let mut fixed: Vec<i64> = data.par_iter().with_min_len(MIN_PARALLEL_TEXELS).map(to_fixed).collect();
    blur_rows(&mut fixed, width, radius);

    let mut columns = transpose(&fixed, width, height);
    blur_rows(&mut columns, height, radius);
    transpose_into(&columns, height, width, &mut fixed);

    data.par_iter_mut().with_min_len(MIN_PARALLEL_TEXELS).zip(fixed).for_each(|(target, blurred)| *target = (blurred as f64 / FIX) as f32);
}

/// The box blurs `blur` runs along each axis
pub(crate) const BLUR_BOX_PASSES: usize = 3;

/// Three box blurs of radius `r` along every row (`w` long) of `data`.
fn blur_rows(data: &mut [i64], w: usize, r: usize) {
    if w == 0 {
        return;
    }

    data.par_chunks_mut(w).with_min_len(lines_per_task(w)).for_each(|row| {
        let mut scratch = vec![0i64; row.len()];
        for _ in 0..BLUR_BOX_PASSES {
            box_line(row, &mut scratch, r);
            row.copy_from_slice(&scratch);
        }
    });
}

/// One box blur of radius `r` of a line (texels past the ends count as 0).
fn box_line(src: &[i64], dst: &mut [i64], r: usize) {
    let n = (2 * r + 1) as i64;

    // Rounded division of a window sum.
    let avg = |acc: i64| (2 * acc + n).div_euclid(2 * n);
    let mut acc: i64 = src.iter().take(r + 1).sum();
    for (i, d) in dst.iter_mut().enumerate() {
        *d = avg(acc);
        acc += src.get(i + r + 1).copied().unwrap_or(0);
        if i >= r {
            acc -= src.get(i - r).copied().unwrap_or(0);
        }
    }
}

/// `src` (`w × h`, row-major) transposed to `h × w`.
fn transpose(src: &[i64], w: usize, h: usize) -> Vec<i64> {
    let mut out = vec![0i64; src.len()];
    transpose_into(src, w, h, &mut out);
    out
}

fn transpose_into(src: &[i64], w: usize, h: usize, out: &mut [i64]) {
    if h > 0 {
        out.par_chunks_mut(h).with_min_len(lines_per_task(h)).enumerate().for_each(|(x, col)| {
            for (y, d) in col.iter_mut().enumerate() {
                *d = src.get(y * w + x).copied().unwrap_or(0);
            }
        });
    }
}

/// Covered texels of tile (tx, ty): (x, y, offset inside the tile).
fn tile_texels(gbuf: &UvGBuffer, tx: u32, ty: u32) -> impl Iterator<Item = (u32, u32, usize)> + '_ {
    let (w, h) = (gbuf.width, gbuf.height);
    (0..TILE).flat_map(|ly| (0..TILE).map(move |lx| (lx, ly))).filter_map(move |(lx, ly)| {
        let (x, y) = (tx * TILE + lx, ty * TILE + ly);
        (x < w && y < h && gbuf.tri.get((y * w + x) as usize).is_some_and(|t| *t != NO_TRI)).then_some((x, y, (ly * TILE + lx) as usize))
    })
}

/// Live stroke alpha at offset `o` of a preview tile.
fn stroke_at(preview: Option<&[u16]>, o: usize) -> f32 {
    preview.and_then(|t| t.get(o)).map(|v| from_u16(*v)).unwrap_or(0.0)
}

/// The full stack of tile (tx, ty) when `layer` reads anchor points (empty otherwise): evaluated
/// up to the layer, it records the anchors below it.
fn anchor_plan<'a>(inp: &Inputs<'a>, id: LayerId, channels: &[Channel], tx: u32, ty: u32, fields: Option<&'a Fields>) -> Vec<Node<'a>> {
    match inp.project.texture_set(inp.ts) {
        Ok(set) if reads_anchor(inp.project, id) => plan(inp, &set.stack, channels, tx, ty, 0, fields),
        _ => Vec::new(),
    }
}

/// The whole stack of the set planned for tile (tx, ty), for evaluating what lies below a layer.
fn stack_plan<'a>(inp: &Inputs<'a>, channels: &[Channel], tx: u32, ty: u32, fields: Option<&'a Fields>) -> Vec<Node<'a>> {
    match inp.project.texture_set(inp.ts) {
        Ok(set) => plan(inp, &set.stack, channels, tx, ty, 0, fields),
        Err(_) => Vec::new(),
    }
}

/// Whether layer `id` (its own mask, or the content it draws) reads an anchor point.
pub(crate) fn reads_anchor(p: &Project, id: LayerId) -> bool {
    p.layers.get(&id).is_some_and(Layer::reads_anchor) || p.content_layer(id).is_some_and(Layer::reads_anchor)
}

/// Whether `l` is an effect layer on `channel`: a paint or fill layer (not a folder) with visible content effects that
/// blends the channel with `Passthrough`. Its content starts from the stack below it instead of from nothing (its own
/// paint or fill laid over that), its effects work on that, and the result takes the stack's place by the layer's
/// strength: a filter on such a layer filters what is below it. Without content effects the result is the same as a
/// normal blend.
pub(crate) fn starts_from_below(l: &Layer, channel: Channel) -> bool {
    !l.is_group() && l.content_effects.iter().any(|e| e.visible) && l.blend(channel).is_some_and(|b| b.mode == BlendMode::Passthrough)
}

/// The mask layer `id` draws with: its own, or an instance source's.
fn layer_mask(p: &Project, id: LayerId) -> Option<&Mask> {
    p.layers.get(&p.mask_owner(id))?.mask.as_ref()
}

/// Whether layer `id` (or, for a folder, a layer inside it) holds an anchor point: a visible one among its content effects,
/// or one inside its enabled mask.
fn holds_anchor(p: &Project, id: LayerId, depth: usize) -> bool {
    if depth > texturecraft_core::project::MAX_DEPTH {
        return false;
    }
    layer_mask(p, id).is_some_and(|m| m.enabled && mask_anchors(m).next().is_some())
        || p.content_layer(id).is_some_and(|l| {
            (!l.is_group() && l.content_effects.iter().any(|e| e.visible && matches!(e.kind, EffectKind::Anchor)))
                || l.children().iter().any(|c| holds_anchor(p, *c, depth + 1))
        })
}

/// Layers of `list` in drawing order (bottom → top, folder and instanced folder contents after
/// their folder).
fn draw_order(p: &Project, list: &[LayerId], depth: usize, out: &mut Vec<LayerId>) {
    if depth > texturecraft_core::project::MAX_DEPTH {
        return;
    }
    for id in list {
        out.push(*id);
        if let Some(l) = p.content_layer(*id) {
            draw_order(p, l.children(), depth + 1, out);
        }
    }
}

/// What a field holds.
#[derive(Clone, Copy)]
enum Job<'a> {
    Mask(&'a Mask),
    Content(Channel),
}

/// A layer's mask or content channel after every neighbourhood filter of its stack, over `tiles`. With `cache`, the
/// filters of a chain holding a graph filter that reads the stack (their regions cover the whole set) are taken from
/// it or kept in it when they are the same whatever the tiles (see [`FieldCache`]).
fn field(
    inp: &Inputs,
    channels: &[Channel],
    below: &Fields,
    id: LayerId,
    job: Job,
    tiles: &[(u32, u32)],
    mut cache: Option<&mut FieldCache>,
) -> Option<Arc<Field>> {
    let effects = match job {
        Job::Mask(mask) => &mask.effects,
        Job::Content(_) => &inp.project.content_layer(id)?.content_effects,
    };
    let channel = match job {
        Job::Mask(_) => None,
        Job::Content(channel) => Some(channel),
    };
    let filters: Vec<(usize, f32, Op)> = neighbourhood_filters(effects, inp.tile_res(), channel).collect();
    let longest_side = inp.gbuf.width.max(inp.gbuf.height);

    // Each filter's region also holds what every filter above it reads around the tiles
    let mut margins: Vec<u32> = filters.iter().map(|&(_, radius, op)| filter_extent(radius, op, longest_side).1.div_ceil(TILE)).collect();
    for index in (0..margins.len().saturating_sub(1)).rev() {
        margins[index] += margins[index + 1];
    }

    // Only chains holding a graph filter that reads the stack (the others reach a few tiles around the block)
    let whole_set = filters.iter().any(|f| matches!(f.2, Op::Graph));
    let cache_slot = match job {
        Job::Mask(_) => FieldKey::Mask(id),
        Job::Content(c) => FieldKey::Content(id, c),
    };
    // What the fields read: the layer, and every layer drawn before it when it reads the stack below
    let reads_below = reads_stack_below(inp, id, job);
    let deps_shared = below.iter().filter(|(k, _)| reads_below || field_layer(k) == id).all(|(_, f)| f.shared);
    let read = match &cache {
        Some(_) if whole_set && deps_shared => fields_read(inp, id, reads_below),
        _ => None,
    };

    let mut previous: Option<(usize, Arc<Field>)> = None;
    for (k, (&filter, &margin)) in filters.iter().zip(&margins).enumerate() {
        let region = region_around(inp, tiles, margin)?;
        let start_shared = previous.as_ref().is_none_or(|(_, f)| f.shared);
        let shareable = region == (0, 0, inp.gbuf.width, inp.gbuf.height) && start_shared;
        let kept = match (&read, cache.as_deref_mut()) {
            (Some((roots, tree)), Some(c)) if shareable => c.get(inp, (cache_slot, k), roots, tree),
            _ => None,
        };
        if let Some(kept) = kept {
            previous = Some((filter.0, kept));
            continue;
        }
        let start = previous.as_ref().map(|(index, field)| (*index, field.as_ref()));
        let mut filtered = filter_region(inp, channels, below, id, job, effects, filter, region, start)?;
        filtered.shared &= shareable && deps_shared;
        // a chain with a graph reading the stack covers the whole set whatever the block: its fields are held as 16-bit
        // steps where they can be (the same in every block, cached or not)
        if whole_set {
            filtered.compact();
        }
        let filtered = Arc::new(filtered);
        if let (Some((roots, tree)), Some(c)) = (&read, cache.as_deref_mut())
            && filtered.shared
        {
            c.put(inp, (cache_slot, k), roots, tree, &filtered);
        }
        previous = Some((filter.0, filtered));
    }

    previous.map(|(_, field)| field)
}

/// The layer a field belongs to.
fn field_layer(k: &FieldKey) -> LayerId {
    match k {
        FieldKey::Mask(id) | FieldKey::Content(id, _) => *id,
    }
}

/// Whether a field of layer `id` evaluates the stack below the layer (see [`stack_region`]): an effect layer starts
/// from it, and anchor readers record anchor points in it. Otherwise it reads only the layer's own fields.
fn reads_stack_below(inp: &Inputs, id: LayerId, job: Job) -> bool {
    let effect_layer = match job {
        Job::Mask(_) => false,
        Job::Content(_) => inp.project.content_layer(id).is_some_and(|l| Channel::ALL.iter().any(|c| starts_from_below(l, *c))),
    };
    effect_layer || reads_anchor(inp.project, id)
}

/// The layers a field of layer `id` is computed from, drawn as the returned roots arrange them: the layer itself, or
/// with `below` every layer drawn before it in the set (with their folders) as well. None when `id` is not drawn.
fn fields_read(inp: &Inputs, id: LayerId, below: bool) -> Option<(Vec<LayerId>, std::collections::BTreeSet<LayerId>)> {
    let roots = if below {
        let stack = inp.project.texture_set(inp.ts).ok()?;
        let mut order = Vec::new();
        draw_order(inp.project, &stack.stack, 0, &mut order);
        let at = order.iter().position(|l| *l == id)?;
        order.truncate(at + 1);
        order
    } else {
        vec![id]
    };
    let tree = inp.project.render_tree(&roots);
    Some((roots, tree))
}

/// The tile-aligned texel region `(x0, y0, w, h)` around `tiles` plus `margin` tiles, clipped to the set.
fn region_around(inp: &Inputs, tiles: &[(u32, u32)], margin: u32) -> Option<(u32, u32, u32, u32)> {
    let (first_x, first_y) = tiles.iter().fold((u32::MAX, u32::MAX), |(min_x, min_y), tile| (min_x.min(tile.0), min_y.min(tile.1)));
    let (last_x, last_y) = tiles.iter().fold((0, 0), |(max_x, max_y), tile| (max_x.max(tile.0), max_y.max(tile.1)));
    let (x0, y0) = (first_x.saturating_sub(margin) * TILE, first_y.saturating_sub(margin) * TILE);
    let (x1, y1) = (((last_x + 1 + margin) * TILE).min(inp.gbuf.width), ((last_y + 1 + margin) * TILE).min(inp.gbuf.height));
    let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    (w > 0 && h > 0).then_some((x0, y0, w, h))
}

/// A neighbourhood filter's radius in whole texels and how many texels around a texel it reads.
fn filter_extent(radius: f32, op: Op, longest_side: u32) -> (u32, u32) {
    // Clamped so a hostile radius can't overflow the region arithmetic
    let radius_texels = radius.round().clamp(1.0, longest_side as f32) as u32;

    // Three box passes reach three radii; the gathering filters one radius plus a bilinear texel
    let reach = if is_box_blur(op) { 3 * radius_texels } else { radius_texels + 1 };
    (radius_texels, reach)
}

/// Whether neighbourhood filter `op` is computed from box blurs (the others gather samples).
fn is_box_blur(op: Op) -> bool {
    matches!(op, Op::Blur(_) | Op::HighPass | Op::Bevel | Op::Glow(_))
}

/// The stack up to and including neighbourhood filter `filter` over texel `region`, continued from `start` (the filter below's field) when given.
#[allow(clippy::too_many_arguments)]
fn filter_region(
    inp: &Inputs,
    channels: &[Channel],
    below: &Fields,
    id: LayerId,
    job: Job,
    effects: &[Effect],
    (index, radius, op): (usize, f32, Op),
    (rx0, ry0, w, h): (u32, u32, u32, u32),
    start: Option<(usize, &Field)>,
) -> Option<Field> {
    let filter_effect = effects.iter().filter(|effect| effect.visible).nth(index);
    let opacity = filter_effect.map(|effect| effect.opacity.clamp(0.0, 1.0)).unwrap_or(1.0);
    let (radius_texels, _) = filter_extent(radius, op, inp.gbuf.width.max(inp.gbuf.height));

    // Generator stages keep their ramp beside the mask stack below
    let generator = match (job, filter_effect.map(|effect| (&effect.kind, effect.blend))) {
        (Job::Mask(_), Some((EffectKind::Generator(generator), blend))) => Some((generator, blend)),
        _ => None,
    };
    let comps = match job {
        Job::Mask(_) if generator.is_some() => 2,
        Job::Mask(_) => 1,
        Job::Content(_) => 4,
    };
    if let Op::Graph = op {
        let effect = filter_effect?;
        let (mut data, marks, cooked) = graph_region(inp, channels, below, id, job, index, effect, (rx0, ry0, w, h), start)?;
        unpremultiply(&mut data, comps);
        return Some(Field { x0: rx0, y0: ry0, w, h, comps, data: FieldData::F32(data), marks, shared: cooked });
    }
    let (orig, cover, _, marks) = stack_region(inp, channels, below, id, job, index, (rx0, ry0, w, h), start, &[], generator.map(|(g, _)| g));
    let mut field = filtered_region(inp, effects, (index, radius_texels, op), (rx0, ry0, w, h), opacity, comps, job, orig, cover, generator)?;
    field.marks = marks;
    Some(field)
}

/// A region's stack values, whether each texel was evaluated, the anchor values asked for and the mask's own anchor
/// points (see [`stack_region`]).
type StackRegion = (Vec<f32>, Vec<f32>, Vec<Vec<[f32; 3]>>, Marks);

/// One texel of a region: its position, its stack value and the anchor values recorded below it.
type RegionTexel = (u32, u32, [f32; 4], Vec<[f32; 3]>);

/// The stack of layer `id` up to effect `index` over texel `region` (content premultiplied by coverage), continued from
/// `start` when given: one value per texel (`comps` per texel: 1 for a mask, 4 for content), whether each texel was
/// evaluated, and the values of `anchors` (an anchor point and channel each) recorded below the layer.
#[allow(clippy::too_many_arguments)]
fn stack_region(
    inp: &Inputs,
    channels: &[Channel],
    below: &Fields,
    id: LayerId,
    job: Job,
    index: usize,
    (rx0, ry0, w, h): (u32, u32, u32, u32),
    start: Option<(usize, &Field)>,
    asked: &[(LayerId, Channel)],
    generator: Option<&Generator>,
) -> StackRegion {
    let comps = match job {
        Job::Mask(_) if generator.is_some() => 2,
        Job::Mask(_) => 1,
        Job::Content(_) => 4,
    };
    // A mask's own anchor points are recorded too, for the field to carry them past the filter
    let own: Vec<LayerId> = match job {
        Job::Mask(m) => m.effects.iter().filter(|e| e.visible).take(index).filter(|e| matches!(e.kind, EffectKind::Anchor)).map(|e| e.id).collect(),
        Job::Content(_) => Vec::new(),
    };
    let anchors: Vec<(LayerId, Channel)> = asked.iter().copied().chain(own.iter().map(|a| (*a, Channel::Height))).collect();
    let anchors = anchors.as_slice();
    // Content reading an anchor point inside the layer's own mask: the mask is evaluated first, recording it
    let own_mask = match job {
        Job::Content(_) => layer_mask(inp.project, id).filter(|m| mask_anchors(m).any(|a| asked.iter().any(|(at, _)| *at == a))),
        Job::Mask(_) => None,
    };

    // Evaluate the stack up to the filter, tile by tile (content premultiplied by coverage)
    let (rx1, ry1) = (rx0 + w, ry0 + h);
    let region_tiles: Vec<(u32, u32)> = (ry0 / TILE..ry1.div_ceil(TILE)).flat_map(|y| (rx0 / TILE..rx1.div_ceil(TILE)).map(move |x| (x, y))).collect();
    let parts: Vec<Vec<RegionTexel>> = region_tiles
        .par_iter()
        .map(|&(tx, ty)| {
            let from_below = matches!(job, Job::Content(c) if inp.project.content_layer(id).is_some_and(|l| starts_from_below(l, c)));
            let full = if from_below { stack_plan(inp, channels, tx, ty, Some(below)) } else { anchor_plan(inp, id, channels, tx, ty, Some(below)) };
            let preview_tile = inp.preview.and_then(|preview| preview.alpha.tile(tx, ty));
            let own_mask = own_mask.and_then(|m| mask_plan(inp, id, m, tx, ty, Some(below)));
            let mut mask = None;
            let mut nodes = Vec::new();
            let mut generator_sizes = None;
            match job {
                Job::Mask(layer_mask) => {
                    let Some(mut mask_stack) = mask_plan(inp, id, layer_mask, tx, ty, None) else { return Vec::new() };

                    generator_sizes = mask_stack.effects.get(index).and_then(|effect| effect.curvature_sizes.clone());
                    mask_stack.effects.truncate(index);
                    mask_stack.start = start;
                    mask = Some(mask_stack);
                }
                Job::Content(channel) => {
                    nodes = plan(inp, &[id], &[channel], tx, ty, 0, Some(below));
                    if let Some(chan) = nodes.first_mut().and_then(|node| node.chans.first_mut()) {
                        chan.effects.truncate(index);
                        chan.start = start;
                    }
                }
            }
            tile_texels(inp.gbuf, tx, ty)
                .map(|(x, y, offset)| {
                    let mut texel = Texel::new(inp, x, y, offset, stroke_at(preview_tile, offset));
                    let acc = if full.is_empty() { None } else { Some(texel.anchors_below(&full, id)) };
                    if let Some(own_mask) = &own_mask {
                        texel.mask(own_mask);
                    }
                    let value = match (&mask, nodes.first()) {
                        (Some(mask_stack), _) => {
                            let ramp = generator.map_or(0.0, |generator| texel.generator_ramp(generator, generator_sizes.as_deref()));
                            [texel.mask(mask_stack), ramp, 0.0, 0.0]
                        }
                        (None, Some(node)) => match node.chans.first().and_then(|chan| {
                            let below = acc.and_then(|a| a.below(chan));
                            texel.content_over(&node.content, chan, usize::MAX, below)
                        }) {
                            Some((color, cov)) => [color[0] * cov, color[1] * cov, color[2] * cov, cov],
                            None => [0.0; 4],
                        },
                        (None, None) => [0.0; 4],
                    };
                    let recorded = anchors.iter().map(|(a, c)| texel.anchored(*a, *c)).collect::<Vec<_>>();
                    (x, y, value, recorded)
                })
                .collect()
        })
        .collect();

    // Scatter into the region, a row of tiles at a time (the region is tile aligned)
    let texel_count = (w * h) as usize;
    let mut orig = vec![0.0f32; texel_count * comps];
    let mut cover = vec![0.0f32; texel_count];
    let row_tiles = (rx1.div_ceil(TILE) - rx0 / TILE).max(1) as usize;
    let band = (TILE * w) as usize;
    orig.par_chunks_mut(band * comps).zip(cover.par_chunks_mut(band)).zip(parts.par_chunks(row_tiles)).for_each(|((orig, cover), row)| {
        for (x, y, value, _) in row.iter().flatten() {
            let texel = (((y - ry0) % TILE) * w + (x - rx0)) as usize;
            if let (Some(covered), Some(pixel)) = (cover.get_mut(texel), orig.get_mut(texel * comps..(texel + 1) * comps)) {
                *covered = 1.0;
                pixel.copy_from_slice(&value[..comps]);
            }
        }
    });
    let mut recorded: Vec<Vec<[f32; 3]>> = anchors.iter().map(|(_, c)| vec![c.default_value(); texel_count]).collect();
    if !anchors.is_empty() {
        for (x, y, _, values) in parts.iter().flatten() {
            let texel = ((y - ry0) * w + (x - rx0)) as usize;
            for (image, v) in recorded.iter_mut().zip(values) {
                if let Some(slot) = image.get_mut(texel) {
                    *slot = *v;
                }
            }
        }
    }
    let marks =
        recorded.split_off(asked.len().min(recorded.len())).into_iter().zip(own).map(|(values, a)| (a, values.into_iter().map(|v| v[0]).collect())).collect();
    (orig, cover, recorded, marks)
}

/// Whether anchor point `anchor` sits inside a layer's mask (imported projects only).
fn in_a_mask(p: &Project, anchor: LayerId) -> bool {
    p.layers.values().any(|l| l.mask.as_ref().is_some_and(|m| m.effects.iter().any(|e| e.id == anchor)))
}

/// The visible anchor points inside a mask.
fn mask_anchors(m: &Mask) -> impl Iterator<Item = LayerId> + '_ {
    m.effects.iter().filter(|e| e.visible && matches!(e.kind, EffectKind::Anchor)).map(|e| e.id)
}

/// Mask `mask` of layer `id` up to effect `index` at the texels of `region` outside the UV islands (those `padding`
/// sends elsewhere), written into `given`: evaluated at the texel itself, so painted effects add nothing there, with the
/// surface (position, normal, UV) of the nearest island texel for what fills and generators read.
#[allow(clippy::too_many_arguments)]
fn mask_outside(
    inp: &Inputs,
    channels: &[Channel],
    below: &Fields,
    id: LayerId,
    mask: &Mask,
    index: usize,
    (rx0, ry0, w, h): (u32, u32, u32, u32),
    start: Option<(usize, &Field)>,
    padding: &[u32],
    given: &mut [f32],
) {
    let (rx1, ry1) = (rx0 + w, ry0 + h);
    let region_tiles: Vec<(u32, u32)> = (ry0 / TILE..ry1.div_ceil(TILE)).flat_map(|y| (rx0 / TILE..rx1.div_ceil(TILE)).map(move |x| (x, y))).collect();
    let (gw, gh) = (inp.gbuf.width, inp.gbuf.height);
    let values: Vec<(usize, f32)> = region_tiles
        .par_iter()
        .flat_map_iter(|&(tx, ty)| {
            let mut out = Vec::new();
            let Some(mut plan) = mask_plan(inp, id, mask, tx, ty, None) else { return out };
            plan.effects.truncate(index);
            plan.start = start;
            let full = anchor_plan(inp, id, channels, tx, ty, Some(below));
            for ly in 0..TILE {
                for lx in 0..TILE {
                    let (x, y) = (tx * TILE + lx, ty * TILE + ly);
                    if x < rx0 || y < ry0 || x >= rx1 || y >= ry1 || x >= gw || y >= gh {
                        continue;
                    }
                    let i = ((y - ry0) * w + (x - rx0)) as usize;
                    let Some(&src) = padding.get(i) else { continue };
                    if src == texturecraft_texel::dilate::NONE || src as usize == i {
                        continue;
                    }
                    let (sx, sy) = (rx0 + src % w, ry0 + src / w);
                    let surface = Texel::new(inp, sx, sy, 0, 0.0).coords();
                    let mut texel = Texel::new(inp, x, y, (ly * TILE + lx) as usize, 0.0);
                    texel.coords = Some(surface);
                    if !full.is_empty() {
                        texel.anchors_below(&full, id);
                    }
                    out.push((i, texel.mask(&plan)));
                }
            }
            out
        })
        .collect();
    for (i, v) in values {
        if let Some(slot) = given.get_mut(i) {
            *slot = v;
        }
    }
}

/// For each texel of `region`, the evaluated texel (`evaluated` > 0: on the UV islands) whose value it takes: the nearest
/// one, never across the borders of a UV-tiled set's blocks. Empty when every texel was evaluated or none was.
fn outside_padding(inp: &Inputs, evaluated: &[f32], (rx0, ry0, w, h): (u32, u32, u32, u32)) -> Vec<u32> {
    let covered: Vec<bool> = evaluated.iter().map(|c| *c > 0.0).collect();
    if covered.iter().all(|c| *c) || !covered.contains(&true) {
        return Vec::new();
    }
    let whole = (rx0, ry0, w, h) == (0, 0, inp.gbuf.width, inp.gbuf.height);
    let block = match inp.tile_grid() {
        Some(g) if whole => (w / g.cols.max(1), h / g.rows.max(1)),
        _ => (0, 0),
    };
    DilationMap::in_rectangular_blocks(&covered, w, h, None, block).src
}

/// `data` (`comps` values per texel) with each texel taking the value of its `padding` source; unchanged without padding.
fn padded<T: Copy>(data: &[T], comps: usize, padding: &[u32]) -> Vec<T> {
    if padding.is_empty() {
        return data.to_vec();
    }
    let mut out = data.to_vec();
    for (i, &src) in padding.iter().enumerate() {
        if src == texturecraft_texel::dilate::NONE || src as usize == i {
            continue;
        }
        let src = src as usize;
        if let (Some(from), Some(to)) = (data.get(src * comps..(src + 1) * comps), out.get_mut(i * comps..(i + 1) * comps)) {
            to.copy_from_slice(from);
        }
    }
    out
}

/// Where a graph filter's result on a channel covers the layer (its alpha), measured on filters in a fill layer without
/// height or normal (the reference app's results).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Cover {
    /// The filter's coverage output for the channel, on any channel (a threshold whose alpha output passes its input's
    /// alpha through leaves height flat over an empty height).
    Alpha,
    /// A colour output's own alpha, on height and normal (a gradient filter passes its input's alpha, the stack's
    /// coverage, on).
    OutputAlpha,
    /// The whole layer, on height and normal, for a grey output of a graph that does not read the stack it runs on
    /// (a shut-line filter's height leaves −1 over an empty height, whatever is below).
    Whole,
    /// What the content below covers: in masks, on the other channels, and on height and normal for a grey output of a
    /// graph that reads its own stack (a bevel over an empty height leaves height flat).
    Below,
}

impl Cover {
    fn of(g: &GraphRef, channel: Channel, output: Option<&str>) -> Cover {
        if g.output_alphas.contains_key(&channel) {
            return Cover::Alpha;
        }
        if !matches!(channel.kind(), ChannelKind::Signed | ChannelKind::NormalMap) {
            return Cover::Below;
        }
        if output.is_some_and(|o| g.colour_outputs.contains(o)) {
            return Cover::OutputAlpha;
        }
        // only the filter's own input (no channel named) counts: an input named after the channel it runs on reads that
        // channel like any other named input (a weathering filter's `height` input over an empty height lays its height
        // on whole)
        let own = g.inputs.values().any(|i| matches!(i, GraphInput::Stack { channel: None }));
        if own { Cover::Below } else { Cover::Whole }
    }
}

/// A graph filter (effect `index` of layer `id`, reading the stack) over texel `region`: the stack below it and what
/// else it reads (other channels, coverage, anchors) given to the graph as images, the graph rendered (or found among
/// the cooks), and its output laid over the stack with the effect's blending. Content comes back premultiplied (see
/// [`stack_region`]). A graph that could not be rendered leaves the stack as it is.
#[allow(clippy::too_many_arguments)]
fn graph_region(
    inp: &Inputs,
    channels: &[Channel],
    below: &Fields,
    id: LayerId,
    job: Job,
    index: usize,
    effect: &Effect,
    (rx0, ry0, w, h): (u32, u32, u32, u32),
    start: Option<(usize, &Field)>,
) -> Option<(Vec<f32>, Marks, bool)> {
    let EffectKind::Graph { graph, .. } = &effect.kind else { return None };
    let own = match job {
        Job::Mask(_) => None,
        Job::Content(c) => Some(c),
    };
    let comps = if own.is_some() { 4 } else { 1 };
    let anchors: Vec<(LayerId, Channel)> = graph
        .inputs
        .values()
        .filter_map(|i| match i {
            GraphInput::Anchor { anchor, channel, .. } => Some((*anchor, channel.or(own).unwrap_or(Channel::Height))),
            _ => None,
        })
        .collect();
    let (orig, cover, mut recorded, marks) = stack_region(inp, channels, below, id, job, index, (rx0, ry0, w, h), start, &anchors, None);
    // the graph reads the stack outside the UV islands too, where the reference app's stack holds values: there it
    // reads the nearest island texel's, not empty
    let padding = outside_padding(inp, &cover, (rx0, ry0, w, h));
    let given_orig = match job {
        // under the measured rules a mask outside the islands holds what its fills and graphs give there (read at the
        // nearest island texel's surface), but no paint: strokes and polygon fills cover the islands only, so a filter
        // finds the edge of a painted mask at the islands' borders
        Job::Mask(mask) if !padding.is_empty() && inp.project.rules.blend.is_measured() => {
            let mut given = padded(&orig, comps, &padding);
            mask_outside(inp, channels, below, id, mask, index, (rx0, ry0, w, h), start, &padding, &mut given);
            given
        }
        _ => padded(&orig, comps, &padding),
    };
    for image in &mut recorded {
        *image = padded(image, 1, &padding);
    }
    let env = inp.render_env();
    let side = env.side;
    let working = inp.graphs.working_inputs(&graph.key);
    let n = (w * h) as usize;
    let at = |data: &[f32], i: usize, k: usize| data.get(i * comps + k).copied().unwrap_or(0.0);
    // another channel's stack up to the filter, when the graph reads one
    let mut others: BTreeMap<Channel, Vec<f32>> = BTreeMap::new();
    for i in graph.inputs.values() {
        if let GraphInput::Stack { channel: Some(c) } | GraphInput::StackAlpha { channel: Some(c) } = i
            && Some(*c) != own
            && own.is_some()
            && !others.contains_key(c)
        {
            let (values, _, _, _) = stack_region(inp, channels, below, id, Job::Content(*c), index, (rx0, ry0, w, h), None, &[], None);
            others.insert(*c, padded(&values, 4, &padding));
        }
    }
    let to_side = |t: Tex| if (t.w, t.h) == (side, side) { t } else { Tex::from_fn(side, side, |u, v| t.sample(u, v)) };
    let mut given: Vec<(u32, Tex)> = Vec::new();
    let mut next_anchor = 0;
    for (k, input) in &graph.inputs {
        let linear = working.contains(k);
        let image = match input {
            GraphInput::Stack { channel } => {
                let c = channel.or(own);
                // the filter's own input (no channel named); an input named after the channel the filter runs on reads the
                // same stack but as a named input: 0 where nothing is below, opaque
                let is_own = channel.is_none();
                let data = if c == own { Some(&given_orig) } else { c.and_then(|c| others.get(&c)) };
                let Some(data) = data else { continue };
                let px = (0..n)
                    .map(|i| match c {
                        // a mask reads its value
                        None => {
                            let m = at(data, i, 0);
                            [m, m, m, 1.0]
                        }
                        Some(c) => {
                            let cov = at(data, i, 3);
                            // the filter's own input carries the stack's coverage as its alpha
                            let alpha = if is_own { cov.clamp(0.0, 1.0) } else { 1.0 };
                            if cov <= 0.0 {
                                // nothing below: an input named after another channel reads 0; the filter's own reads the flat
                                // surface on height and normal
                                let empty = match c.kind() {
                                    ChannelKind::Signed if is_own => [0.5; 3],
                                    ChannelKind::NormalMap if is_own => [0.5, 0.5, 1.0],
                                    _ => [0.0; 3],
                                };
                                [empty[0], empty[1], empty[2], alpha]
                            } else {
                                let straight = [at(data, i, 0) / cov, at(data, i, 1) / cov, at(data, i, 2) / cov];
                                let g = graphs::channel_to_graph(c.kind(), straight, linear);
                                [g[0], g[1], g[2], alpha]
                            }
                        }
                    })
                    .collect();
                Tex { w: w as usize, h: h as usize, px }
            }
            GraphInput::StackAlpha { channel } => {
                let c = channel.or(own);
                let data = if c == own { Some(&given_orig) } else { c.and_then(|c| others.get(&c)) };
                let Some(data) = data else { continue };
                let px = (0..n).map(|i| if c.is_some() { at(data, i, 3) } else { 1.0 }).map(|a| [a, a, a, 1.0]).collect();
                Tex { w: w as usize, h: h as usize, px }
            }
            GraphInput::Anchor { anchor, channel, levels } => {
                // an anchor point inside a mask holds the mask's grey, whatever the channel
                let read = if in_a_mask(inp.project, *anchor) { ChannelKind::Scalar } else { channel.or(own).unwrap_or(Channel::Height).kind() };
                let Some(values) = recorded.get(next_anchor) else { continue };
                next_anchor += 1;
                let px = values
                    .iter()
                    .map(|v| {
                        let v = levels.map_or(*v, |l| per_unit(read, *v, |x| l.apply(x)));
                        let g = graphs::channel_to_graph(read, v, linear);
                        [g[0], g[1], g[2], 1.0]
                    })
                    .collect();
                Tex { w: w as usize, h: h as usize, px }
            }
            _ => continue,
        };
        given.push((*k, to_side(image)));
    }
    let use_ = GraphUse { set: inp.ts, id: effect.id, channel: own };
    let cooked = inp.graphs.cook_for(use_.clone(), graph, &env, &given);
    // a cook that failed now (none, or the one the use read last) may be found later: not kept for other blocks
    let exact = !inp.graphs.has_problem(&use_);
    let (Some(cooked), size) = (cooked, (inp.gbuf.width, inp.gbuf.height)) else { return Some((orig, marks, exact)) };
    let opacity = effect.opacity.clamp(0.0, 1.0);
    let mut data = orig.clone();
    match own {
        None => {
            let Some(plane) = cooked.output(graph.output.as_deref()) else { return Some((orig, marks, exact)) };
            for i in 0..n {
                if cover.get(i).copied().unwrap_or(0.0) <= 0.0 {
                    continue;
                }
                let (x, y) = (rx0 + i as u32 % w, ry0 + i as u32 / w);
                let m = at(&orig, i, 0);
                let g = graphs::at_texel(plane, x, y, size)[0];
                if let Some(slot) = data.get_mut(i) {
                    *slot = m + (inp.project.rules.blend.blended1(effect.blend, m, g, true) - m) * opacity;
                }
            }
        }
        Some(c) => {
            // a filter naming outputs for other channels leaves this one as it is
            let Some(output) = graph.output_for(c) else { return Some((orig, marks, exact)) };
            let Some(plane) = cooked.output(output) else { return Some((orig, marks, exact)) };
            let linear = cooked.is_linear(output);
            let alpha_plane = graph.output_alphas.get(&c).and_then(|a| cooked.output(Some(a)));
            let cover_rule = Cover::of(graph, c, output.or_else(|| cooked.outputs.keys().next().map(String::as_str)));
            for i in 0..n {
                if cover.get(i).copied().unwrap_or(0.0) <= 0.0 {
                    continue;
                }
                let (x, y) = (rx0 + i as u32 % w, ry0 + i as u32 / w);
                let cov = at(&orig, i, 3);
                let v_in = if cov > 0.0 { [at(&orig, i, 0) / cov, at(&orig, i, 1) / cov, at(&orig, i, 2) / cov] } else { [0.0; 3] };
                let g = graphs::at_texel(plane, x, y, size);
                let a = match cover_rule {
                    Cover::Alpha => alpha_plane.map_or(1.0, |p| graphs::at_texel(p, x, y, size)[0].clamp(0.0, 1.0)),
                    Cover::OutputAlpha => g[3].clamp(0.0, 1.0),
                    Cover::Whole => 1.0,
                    Cover::Below => cov,
                };
                let p = graphs::graph_to_channel(c.kind(), g, linear);
                let (v, nc) = over_comps(c.components(), inp.project.rules.blend, stores_unit(inp, c), effect.blend, v_in, cov, p, a * opacity);
                if let Some(px) = data.get_mut(i * 4..i * 4 + 4) {
                    px.copy_from_slice(&[v[0] * nc, v[1] * nc, v[2] * nc, nc]);
                }
            }
        }
    }
    Some((data, marks, exact))
}

/// Neighbourhood filter `op` (with its radius in texels) over a region's stack values (see [`stack_region`]).
#[allow(clippy::too_many_arguments)]
fn filtered_region(
    inp: &Inputs,
    effects: &[Effect],
    (index, radius_texels, op): (usize, u32, Op),
    (rx0, ry0, w, h): (u32, u32, u32, u32),
    opacity: f32,
    comps: usize,
    job: Job,
    orig: Vec<f32>,
    cover: Vec<f32>,
    generator: Option<(&Generator, BlendMode)>,
) -> Option<Field> {
    // Tiny coverage sums make normalization unstable
    const MIN_FILTER_WEIGHT: f32 = 1e-6;

    let (rx1, ry1) = (rx0 + w, ry0 + h);

    // High pass centers on the channel's mid value (0 for signed channels like height)
    let mid = match job {
        Job::Content(channel) if channel.kind() == ChannelKind::Signed => 0.0,
        _ => 0.5,
    };

    // A smaller UV tile has fewer texels per UV unit, so its filters reach fewer texels
    let filtered = |scale: f32| -> Vec<f32> {
        let res = ((inp.tile_res() as f32 * scale).round() as u32).max(1);
        if let Some((generator, blend)) = generator {
            let texture_set = inp.project.texture_set(inp.ts).map_or(inp.tile_res(), |set| set.resolution);
            return generator_region(generator, (blend, inp.project.rules.blend), opacity, &orig, &cover, (rx0, ry0, w, h), (res, texture_set));
        }

        let scaled_radius = neighbourhood_filters(effects, res, None)
            .find(|filter| filter.0 == index)
            .map_or(radius_texels, |filter| filter.1.round().clamp(1.0, radius_texels as f32) as u32);
        if !is_box_blur(op) {
            return gather(op, &orig, &cover, comps, (rx0, ry0, w, h), scaled_radius as f32, res, opacity);
        }

        // Weighted so uncovered texels don't bleed in
        let (width, height, blur_radius) = (w as usize, h as usize, scaled_radius as usize);
        let mut weights = cover.clone();
        blur(&mut weights, width, height, blur_radius);
        let mut data = orig.clone();
        for component in 0..comps {
            let mut sums: Vec<f32> =
                orig.par_chunks(comps).with_min_len(MIN_PARALLEL_TEXELS).map(|pixel| pixel.get(component).copied().unwrap_or(0.0)).collect();
            blur(&mut sums, width, height, blur_radius);
            data.par_chunks_mut(comps).with_min_len(MIN_PARALLEL_TEXELS).zip(sums.par_iter()).zip(weights.par_iter()).for_each(|((pixel, sum), weight)| {
                // Content is premultiplied, so the mid value is too (coverage itself is kept)
                let cov = if comps == 4 { pixel.get(3).copied().unwrap_or(0.0) } else { 1.0 };
                if *weight > MIN_FILTER_WEIGHT
                    && let Some(slot) = pixel.get_mut(component)
                {
                    let (original, blurred) = (*slot, sum / weight);
                    let result = match op {
                        Op::Blur(Some(amount)) => original + amount * (original - blurred),
                        Op::HighPass if component < 3 => original - blurred + mid * cov,
                        // Away from 0 by at most the value itself: 0 at the edge, full inside
                        Op::Bevel if component < 3 => original.signum() * original.abs().min((2.0 * blurred * original.signum() - original.abs()).max(0.0)),
                        Op::Glow(amount) if component < 3 => original + amount * blurred,
                        Op::HighPass | Op::Bevel | Op::Glow(_) => original,
                        _ => blurred,
                    };
                    *slot = original + (result - original) * opacity;
                }
            });
        }
        data
    };

    // Each block keeps the result at its tile's scale (blocks are TILE aligned)
    let scales = &inp.block_scales;
    let mut levels: Vec<f32> =
        (ry0..ry1).step_by(TILE as usize).flat_map(|y| (rx0..rx1).step_by(TILE as usize).map(move |x| (x, y))).map(|(x, y)| scales.at(x, y)).collect();
    levels.sort_by(f32::total_cmp);
    levels.dedup();
    let (first, rest) = levels.split_first().map_or((1.0, &[][..]), |(first, rest)| (*first, rest));

    // A generator stage's field is the mask alone
    let field_components = if generator.is_some() { 1 } else { comps };
    let mut data = filtered(first);
    for &scale in rest {
        let other = filtered(scale);
        let row = w as usize * field_components;
        data.par_chunks_mut(row).zip(other.par_chunks(row)).enumerate().for_each(|(y, (dst, src))| {
            for (x, (target, source)) in dst.chunks_mut(field_components).zip(src.chunks(field_components)).enumerate() {
                if scales.at(rx0 + x as u32, ry0 + y as u32) == scale {
                    target.copy_from_slice(source);
                }
            }
        });
    }

    unpremultiply(&mut data, field_components);
    Some(Field { x0: rx0, y0: ry0, w, h, comps: field_components, data: FieldData::F32(data), marks: Vec::new(), shared: true })
}

/// The mask over a region after a generator at its own resolution and blur: `stack` is (stack below, generator ramp) per texel, sizes in texels per side.
fn generator_region(
    generator: &Generator,
    (blend, rules): (BlendMode, BlendRules),
    opacity: f32,
    stack: &[f32],
    cover: &[f32],
    (region_x, region_y, width, height): (u32, u32, u32, u32),
    (uv_tile, texture_set): (u32, u32),
) -> Vec<f32> {
    let (columns, rows) = (width as usize, height as usize);
    let mut ramps: Vec<f32> = stack.par_chunks(2).map(|texel| texel.get(1).copied().unwrap_or(0.0)).collect();

    // Coarser than the UV tile: averaged over cells, then the blur around them
    let cell = generator.resolution.cell(uv_tile, texture_set) as usize;
    if cell > 1 {
        ramps = resampled_by_cells(&ramps, cover, (region_x as usize, region_y as usize, columns, rows), cell);
    }

    let blur_radius = generator.global.blur_radius_at(uv_tile).round() as usize;
    if blur_radius > 0 {
        let mut weights = cover.to_vec();
        blur(&mut weights, columns, rows, blur_radius);
        let mut sums: Vec<f32> = ramps.iter().zip(cover).map(|(ramp, covered)| ramp * covered).collect();
        blur(&mut sums, columns, rows, blur_radius);
        ramps.par_iter_mut().zip(sums.par_iter().zip(weights.par_iter())).for_each(|(ramp, (sum, weight))| {
            if *weight > MIN_COVERAGE_WEIGHT {
                *ramp = sum / weight;
            }
        });
    }

    // Finished and blended over the stack below, like the generator effect itself
    stack
        .par_chunks(2)
        .zip(ramps.par_iter())
        .map(|(texel, ramp)| {
            let below = texel.first().copied().unwrap_or(0.0);
            let generated = finish_generator(generator, *ramp);
            below + (rules.blended1(blend, below, generated, true) - below) * opacity
        })
        .collect()
}

/// `values` averaged over `cell` × `cell` texel cells on the mesh (`cover`), read back bilinearly between cell centers.
fn resampled_by_cells(values: &[f32], cover: &[f32], (region_x, region_y, width, height): (usize, usize, usize, usize), cell: usize) -> Vec<f32> {
    // Cells are aligned to absolute texel positions, so a texel's cell is the same in every region
    let (first_column, first_row) = (region_x / cell, region_y / cell);
    let columns = (region_x + width).div_ceil(cell) - first_column;
    let rows = (region_y + height).div_ceil(cell) - first_row;

    // Each cell's coverage-weighted average, a row of cells at a time
    let averages: Vec<Option<f32>> = (0..rows)
        .into_par_iter()
        .flat_map_iter(|row| {
            let mut sums = vec![(0.0f32, 0.0f32); columns];
            let first_y = ((first_row + row) * cell).saturating_sub(region_y);
            let last_y = ((first_row + row + 1) * cell).saturating_sub(region_y).min(height);
            for y in first_y..last_y {
                for x in 0..width {
                    let texel = y * width + x;
                    let covered = cover.get(texel).copied().unwrap_or(0.0);
                    if let Some((sum, weight)) = sums.get_mut((region_x + x) / cell - first_column) {
                        *sum += values.get(texel).copied().unwrap_or(0.0) * covered;
                        *weight += covered;
                    }
                }
            }
            sums.into_iter().map(|(sum, weight)| (weight > MIN_COVERAGE_WEIGHT).then(|| sum / weight))
        })
        .collect();
    let average = |column: isize, row: isize| -> Option<f32> {
        let column = column.clamp(0, columns as isize - 1) as usize;
        let row = row.clamp(0, rows as isize - 1) as usize;
        averages.get(row * columns + column).copied().flatten()
    };

    let cell_size = cell as f32;
    (0..height)
        .into_par_iter()
        .flat_map_iter(|y| (0..width).map(move |x| (x, y)))
        .map(|(x, y)| {
            // Position in cells, relative to the first cell's center
            let cell_x = ((region_x + x) as f32 + 0.5) / cell_size - 0.5 - first_column as f32;
            let cell_y = ((region_y + y) as f32 + 0.5) / cell_size - 0.5 - first_row as f32;
            let (left, top) = (cell_x.floor(), cell_y.floor());
            let (offset_x, offset_y) = (cell_x - left, cell_y - top);
            let (left, top) = (left as isize, top as isize);
            let corners = [
                (left, top, (1.0 - offset_x) * (1.0 - offset_y)),
                (left + 1, top, offset_x * (1.0 - offset_y)),
                (left, top + 1, (1.0 - offset_x) * offset_y),
                (left + 1, top + 1, offset_x * offset_y),
            ];
            let (sum, weight) = corners.iter().fold((0.0, 0.0), |(sum, weight), &(column, row, corner_weight)| match average(column, row) {
                Some(value) => (sum + value * corner_weight, weight + corner_weight),
                None => (sum, weight),
            });
            if weight > MIN_COVERAGE_WEIGHT { sum / weight } else { values.get(y * width + x).copied().unwrap_or(0.0) }
        })
        .collect()
}

/// A generator's (possibly blurred) ramp after balance, contrast, invert and offset.
fn finish_generator(generator: &Generator, ramp: f32) -> f32 {
    let value = generator.global.apply(ramp);
    let value = if generator.invert { 1.0 - value } else { value };
    let offset = if generator.offset.is_finite() { generator.offset } else { 0.0 };
    (value + offset).clamp(0.0, 1.0)
}

/// Clamp filtered masks to 0..1, and turn filtered content back into straight values.
fn unpremultiply(data: &mut [f32], comps: usize) {
    // Avoid amplifying rounding error near transparent texels
    const MIN_COVERAGE: f32 = 1e-5;

    data.par_chunks_mut(comps).with_min_len(MIN_PARALLEL_TEXELS).for_each(|px| match px {
        [v] => *v = v.clamp(0.0, 1.0),
        [r, g, b, cov] => {
            *cov = cov.clamp(0.0, 1.0);
            let inv = if *cov > MIN_COVERAGE { 1.0 / *cov } else { 0.0 };
            *r *= inv;
            *g *= inv;
            *b *= inv;
        }
        _ => {}
    });
}

/// Precompute the neighbourhood-filtered masks and content channels of every layer that needs
/// one, over `tiles` plus the filter's reach. Walks bottom → top so each field sees the fields
/// (and anchors) below it. Cheap when no layer has a blur or sharpen.
pub fn neighbourhood_fields(inp: &Inputs, channels: &[Channel], tiles: &[(u32, u32)]) -> Fields {
    let stack = inp.project.texture_set(inp.ts).map(|s| s.stack.as_slice()).unwrap_or_default();
    fields_of(inp, channels, stack, tiles, None)
}

/// [`neighbourhood_fields`] of the layers drawn by `list` only. `cache` (only for the whole stack: a field of a list
/// holding part of it reads fewer fields below) keeps the fields that cover the whole set (see [`FieldCache`]).
fn fields_of(inp: &Inputs, channels: &[Channel], list: &[LayerId], tiles: &[(u32, u32)], mut cache: Option<&mut FieldCache>) -> Fields {
    let mut out = Fields::new();
    if tiles.is_empty() || inp.gbuf.width == 0 {
        return out;
    }

    let working_width = inp.gbuf.width;
    let mut order = Vec::new();
    draw_order(inp.project, list, 0, &mut order);
    prepare_curvature_sizes(inp, &order);
    prepare_graphs(inp, channels, &order);

    for id in order {
        let Some(layer) = inp.project.content_layer(id) else { continue };

        if let Some(mask) = layer_mask(inp.project, id).filter(|mask| mask.enabled && neighbourhood(&mask.effects, working_width).is_some())
            && let Some(filtered) = field(inp, channels, &out, id, Job::Mask(mask), tiles, cache.as_deref_mut())
        {
            out.insert(FieldKey::Mask(id), filtered);
        }

        if layer.is_group() || neighbourhood(&layer.content_effects, working_width).is_none() {
            continue;
        }

        for channel in channels.iter().filter(|channel| layer.channels.contains_key(channel)) {
            if let Some(filtered) = field(inp, channels, &out, id, Job::Content(*channel), tiles, cache.as_deref_mut()) {
                out.insert(FieldKey::Content(id, *channel), filtered);
            }
        }
    }

    out
}

/// Cook the graphs of `layers` that read nothing of the stack, once, before tiles composite in parallel (the tiles then
/// find them; renders run on the graph runtime's own pool).
fn prepare_graphs(inp: &Inputs, channels: &[Channel], layers: &[LayerId]) {
    for id in layers {
        let (Some(own), Some(layer)) = (inp.project.layers.get(id), inp.project.content_layer(*id)) else { continue };
        if !own.visible {
            continue;
        }
        let used: Vec<Channel> = channels.iter().copied().filter(|c| layer.channels.contains_key(c)).collect();
        if let LayerKind::Fill(f) = &layer.kind {
            for c in &used {
                if let Some(g) = f.sources.get(c).and_then(Source::graph) {
                    inp.static_cook(layer.id, Some(*c), g);
                }
            }
        }
        if let Some(mask) = layer_mask(inp.project, *id).filter(|m| m.enabled) {
            for e in mask.effects.iter().filter(|e| e.visible) {
                for g in e.kind.graphs() {
                    inp.static_cook(e.id, None, g);
                }
            }
        }
        for e in layer.content_effects.iter().filter(|e| e.visible) {
            for c in &used {
                let g = match &e.kind {
                    EffectKind::Graph { graph, .. } => Some(graph),
                    EffectKind::Material(f) => f.sources.get(c).and_then(Source::graph),
                    _ => None,
                };
                if let Some(g) = g {
                    inp.static_cook(e.id, Some(*c), g);
                }
            }
        }
    }
}

/// Prepare the independent graph sources for a texture set before planning its viewport composite.
pub fn prepare_texture_set_graphs(inp: &Inputs) {
    let Ok(set) = inp.project.texture_set(inp.ts) else { return };
    let channels = set.channel_list();
    let mut order = Vec::new();
    draw_order(inp.project, &set.stack, 0, &mut order);
    let reads_stack = order.iter().filter_map(|&id| inp.project.content_layer(id)).flat_map(Layer::graphs).any(GraphRef::reads_stack);
    if reads_stack {
        let tiles = all_tiles(inp.gbuf.width, inp.gbuf.height);
        neighbourhood_fields(inp, &channels, &tiles);
    } else {
        prepare_graphs(inp, &channels, &order);
    }
}

/// Build the curvature detail sizes every visible generator of `layers` reads, once, before tiles composite in parallel.
fn prepare_curvature_sizes(inp: &Inputs, layers: &[LayerId]) {
    let visible = layers.iter().filter(|id| inp.project.layers.get(id).is_some_and(|layer| layer.visible));
    let masks = visible.filter_map(|id| layer_mask(inp.project, *id)).filter(|mask| mask.enabled);
    prepare_mask_curvature_sizes(inp, masks);
}

/// Build the curvature detail sizes the visible generators of `masks` read, once, before tiles composite in parallel.
fn prepare_mask_curvature_sizes<'a>(inp: &Inputs, masks: impl Iterator<Item = &'a Mask>) {
    let mut needed = [false; CurvatureDetail::BLURRED_SIZES];
    for effect in masks.flat_map(|mask| &mask.effects).filter(|effect| effect.visible) {
        if let EffectKind::Generator(Generator { detail: Some(detail), .. }) = &effect.kind {
            for (slot, in_use) in needed.iter_mut().zip(detail.blurred_sizes_in_use()) {
                *slot |= in_use;
            }
        }
    }
    CurvatureSizes::of(inp, &needed);
}

struct Node<'a> {
    id: LayerId,
    content: Content<'a>,
    chans: Vec<ChanPlan<'a>>,
    mask: Option<MaskPlan<'a>>,
    /// Visible anchor points of the layer: recorded after it is applied, or (`Some(k)`, an
    /// anchor with visible effects above it) with its content cut after `k` visible effects.
    anchors: Vec<(LayerId, Option<usize>)>,
    /// The node or one of its children records an anchor.
    has_anchor: bool,
    height_mask: HeightMask,
    /// Geometry mask (None: everywhere).
    geometry: Option<Geometry>,
}

/// A layer's geometry mask planned for the texture set.
enum Geometry {
    /// Whether the layer shows on each mesh submesh.
    Parts(Vec<bool>),
    /// Whether the layer shows on each atlas block (`cols` across, `block` texels each), row by row. A set without
    /// UV tiles is one block, tile 1001.
    Tiles { cols: u32, rows: u32, block: (u32, u32), allowed: Vec<bool> },
}

impl Geometry {
    fn of(inp: &Inputs, mask: &GeometryMask) -> Geometry {
        match mask {
            GeometryMask::Parts(names) => Geometry::Parts(inp.mesh.submeshes.iter().map(|s| names.contains(&s.name)).collect()),
            GeometryMask::UvTiles(tiles) => {
                let grid = inp.tile_grid();
                let (cols, rows) = grid.map_or((1, 1), |g| (g.cols.max(1), g.rows.max(1)));
                let allowed =
                    (0..rows).flat_map(|by| (0..cols).map(move |bx| (bx, by))).map(|(bx, by)| tiles.contains(&grid.map_or(1001, |g| g.udim(bx, by)))).collect();
                Geometry::Tiles { cols, rows, block: ((inp.gbuf.width / cols).max(1), (inp.gbuf.height / rows).max(1)), allowed }
            }
        }
    }
}

/// Plans of the visible effects of a stack: mask effects (`channel` None) or the content
/// effects of one channel (paint effects then read that channel's pixels).
fn effect_plans<'a>(inp: &Inputs<'a>, effects: &'a [Effect], tile_x: u32, tile_y: u32, channel: Option<Channel>) -> Vec<EffectPlan<'a>> {
    effects
        .iter()
        .filter(|effect| effect.visible)
        .map(|effect| {
            let (image, target) = match channel {
                Some(channel) => (inp.pixels.layer(effect.id, channel), PaintTarget::Layer(effect.id)),
                None => (inp.pixels.masks.get(&effect.id), PaintTarget::Mask(effect.id)),
            };
            let curvature_sizes = match &effect.kind {
                EffectKind::Generator(Generator { detail: Some(detail), .. }) => CurvatureSizes::of(inp, &detail.blurred_sizes_in_use()),
                _ => None,
            };
            let graph = match &effect.kind {
                EffectKind::Graph { graph, .. } => inp.static_cook(effect.id, channel, graph),
                EffectKind::Fill { source: Source::Graph(g), .. } => inp.static_cook(effect.id, None, g),
                EffectKind::Material(f) => channel.and_then(|c| f.sources.get(&c)).and_then(Source::graph).and_then(|g| inp.static_cook(effect.id, channel, g)),
                _ => None,
            };
            EffectPlan {
                id: effect.id,
                kind: &effect.kind,
                blend: effect.blend,
                opacity: effect.opacity,
                data: if effect.kind.painted() { tile_of(inp, image, tile_x, tile_y, channel) } else { None },
                preview: previews(inp, target),
                curvature_sizes,
                graph,
                acts: channel.is_none_or(|c| effect.acts_on(c)),
            }
        })
        .collect()
}

/// Whether the live stroke paints `target` in this composite (a stroke on a layer instanced in a
/// set of another size does not show there).
fn previews(inp: &Inputs, target: PaintTarget) -> bool {
    inp.preview.is_some_and(|p| p.target == target && (p.alpha.width(), p.alpha.height()) == (inp.gbuf.width, inp.gbuf.height))
}

/// Tile (tx, ty) of painted pixels `img` (of `channel`, or a mask when None) at the size of the
/// set being composited: as stored, or resampled (bilinear, premultiplied) when the pixels
/// belong to a layer instanced from a set of another resolution.
fn tile_of<'a>(inp: &Inputs, img: Option<&'a TileImage>, tx: u32, ty: u32, channel: Option<Channel>) -> Option<Cow<'a, [u16]>> {
    let img = img?;
    let (w, h) = (inp.gbuf.width, inp.gbuf.height);
    if (img.width(), img.height()) == (w, h) {
        return img.tile(tx, ty).map(Cow::Borrowed);
    }
    // Empty texels of signed channels hold a signed zero.
    let empty: Vec<u16> = match channel {
        Some(c) if c.kind() == ChannelKind::Signed => [vec![to_u16_signed(0.0); c.components()], vec![0]].concat(),
        _ => Vec::new(),
    };
    img.resampled_tile(w, h, tx, ty, &empty).map(Cow::Owned)
}

fn mask_plan<'a>(inp: &Inputs<'a>, id: LayerId, mask: &'a Mask, tx: u32, ty: u32, fields: Option<&'a Fields>) -> Option<MaskPlan<'a>> {
    if !mask.enabled {
        return None;
    }

    let background = if mask.background == MaskBackground::White { 1.0 } else { 0.0 };
    let effects = effect_plans(inp, &mask.effects, tx, ty, None);
    let start =
        fields.and_then(|f| f.get(&FieldKey::Mask(id))).and_then(|field| neighbourhood(&mask.effects, inp.gbuf.width).map(|(k, _, _)| (k, field.as_ref())));
    Some(MaskPlan { background, effects, start })
}

fn plan<'a>(inp: &Inputs<'a>, list: &[LayerId], channels: &[Channel], tx: u32, ty: u32, depth: usize, fields: Option<&'a Fields>) -> Vec<Node<'a>> {
    if depth > texturecraft_core::project::MAX_DEPTH {
        return Vec::new();
    }
    list.iter().filter_map(|id| plan_node(inp, *id, channels, tx, ty, depth, fields, true)).collect()
}

/// The plan of layer `id` at `depth` (None where it draws nothing); a folder's children are
/// planned when `kids`.
#[allow(clippy::too_many_arguments)]
fn plan_node<'a>(
    inp: &Inputs<'a>,
    id: LayerId,
    channels: &[Channel],
    tx: u32,
    ty: u32,
    depth: usize,
    fields: Option<&'a Fields>,
    kids: bool,
) -> Option<Node<'a>> {
    let own: &'a Layer = inp.project.layers.get(&id)?;

    // Instances draw their source's content (a missing source draws nothing)
    let layer: &'a Layer = inp.project.content_layer(id)?;
    // A layer holding anchor points (in its content or its mask) is always planned, to record them for the layers
    // reading them; it draws only when visible and blending something. The original evaluates them whatever the
    // layer's visibility (Trimmsheet: a height fill reads the masks of three hidden layers), and an imported anchor
    // layer often blends no channel at all (an empty fill holding only the anchor point: MORA, Roads3, 12324).
    let anchors_only = !own.visible;
    let holds = holds_anchor(inp.project, id, depth);
    if anchors_only && !(holds && inp.project.rules.blend.is_measured()) {
        return None;
    }

    let paint = matches!(layer.kind, LayerKind::Paint);
    let fx = !layer.is_group() && layer.content_effects.iter().any(|e| e.visible);
    let chans = channels
        .iter()
        .filter(|_| !anchors_only)
        .filter_map(|c| {
            let b = layer.blend(*c)?;
            if b.mode == BlendMode::Disable || b.opacity <= 0.0 {
                return None;
            }

            let data = if paint { tile_of(inp, inp.pixels.layer(layer.id, *c), tx, ty, Some(*c)) } else { None };
            let effects = if fx { effect_plans(inp, &layer.content_effects, tx, ty, Some(*c)) } else { Vec::new() };
            let start = fields
                .and_then(|fields| fields.get(&FieldKey::Content(id, *c)))
                .and_then(|field| neighbourhood_filters(&layer.content_effects, inp.gbuf.width, Some(*c)).last().map(|(index, ..)| (index, field.as_ref())));
            let graph = match &layer.kind {
                LayerKind::Fill(f) => f.sources.get(c).and_then(Source::graph).and_then(|g| inp.static_cook(layer.id, Some(*c), g)),
                _ => None,
            };
            Some(ChanPlan {
                ch: c.index(),
                kind: c.kind(),
                comps: c.components(),
                blend: b.mode,
                opacity: b.opacity.clamp(0.0, 1.0),
                data,
                effects,
                start,
                graph,
                from_below: starts_from_below(layer, *c),
                rules: inp.project.rules.blend,
                unit: stores_unit(inp, *c),
            })
        })
        .collect::<Vec<_>>();
    let content = match &layer.kind {
        LayerKind::Paint => Content::Paint { preview: previews(inp, PaintTarget::Layer(layer.id)) },
        LayerKind::Fill(f) => Content::Fill(f),
        LayerKind::Group { children, .. } => Content::Group(if kids { plan(inp, children, channels, tx, ty, depth + 1, fields) } else { Vec::new() }),

        // `content_layer` already followed instance links
        LayerKind::Instance { .. } => return None,
    };
    let anchors = if fx { anchor_cuts(&layer.content_effects) } else { Vec::new() };
    // A paint layer with nothing painted (and not being painted) or added by its content
    // effects contributes nothing; a layer blending none of the channels neither (unless it holds anchors, above).
    if !holds
        && let Content::Paint { preview: false } = content
        && chans.iter().all(|c| c.data.is_none() && c.effects.is_empty())
    {
        return None;
    }
    if chans.is_empty() && !holds {
        return None;
    }
    let mask = layer_mask(inp.project, id).and_then(|m| mask_plan(inp, id, m, tx, ty, fields));
    let mask_anchor = layer_mask(inp.project, id).is_some_and(|m| m.enabled && mask_anchors(m).next().is_some());
    let has_anchor = !anchors.is_empty() || mask_anchor || matches!(&content, Content::Group(nodes) if nodes.iter().any(|n| n.has_anchor));
    // The geometry mask is the layer's own (an instance in another set has other parts).
    let geometry = own.geometry_mask.as_ref().map(|g| Geometry::of(inp, g));
    Some(Node { id, content, chans, mask, anchors, has_anchor, height_mask: layer.height_mask, geometry })
}

/// The visible anchor points of a content stack with where they cut it: after their index among
/// the visible effects, or None for anchors with only anchors above them (they record the
/// layer's full result).
fn anchor_cuts(effects: &[Effect]) -> Vec<(LayerId, Option<usize>)> {
    let visible: Vec<&Effect> = effects.iter().filter(|e| e.visible).collect();
    let last = visible.iter().rposition(|e| !matches!(e.kind, EffectKind::Anchor));
    visible.iter().enumerate().filter(|(_, e)| matches!(e.kind, EffectKind::Anchor)).map(|(i, e)| (e.id, last.filter(|l| *l > i).map(|_| i))).collect()
}

/// One level of the path to the hot layer (see [`Level`]) planned for a tile.
struct LevelPlan<'a> {
    below: Vec<Node<'a>>,
    /// A folder planned without its children, or (last level) the hot layer.
    at: Option<Node<'a>>,
    above: Vec<Node<'a>>,
}

/// Plan the path to the hot layer for tile (tx, ty); the layers before the path are left out
/// on tiles the cache holds (`warm`).
fn plan_levels<'a>(inp: &Inputs<'a>, levels: &[Level], channels: &[Channel], tx: u32, ty: u32, fields: &'a Fields, warm: bool) -> Vec<LevelPlan<'a>> {
    let last = levels.len().saturating_sub(1);
    levels
        .iter()
        .enumerate()
        .map(|(d, l)| LevelPlan {
            below: if warm { Vec::new() } else { plan(inp, &l.below, channels, tx, ty, d, Some(fields)) },
            at: plan_node(inp, l.at, channels, tx, ty, d, Some(fields), d == last),
            above: plan(inp, &l.above, channels, tx, ty, d, Some(fields)),
        })
        .collect()
}

#[inline]
fn blend_chan(c: &ChanPlan, mode: BlendMode, a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    if c.comps == 3 { blend3(mode, a, b) } else { [blend1(mode, a[0], b[0]); 3] }
}

/// True if set channel `c` stores 0..1: not a float format, and not the signed height (stored as −1..1, which the
/// measured clamp to 0..1 would cut in half).
fn stores_unit(inp: &Inputs, c: Channel) -> bool {
    use texturecraft_core::ChannelFormat as F;
    if c.kind() == ChannelKind::Signed {
        return false;
    }
    let format = inp.project.texture_set(inp.ts).ok().and_then(|s| s.channels.iter().find(|(ch, _)| *ch == c).map(|(_, f)| *f));
    !matches!(format.unwrap_or_else(|| F::default_for(c)), F::L16F | F::Rgb16F | F::L32F | F::Rgb32F)
}

/// Lay value `p` with weight `a` over a straight value and coverage (premultiplied "over").
fn over(c: &ChanPlan, mode: BlendMode, v: [f32; 3], cov: f32, p: [f32; 3], a: f32) -> ([f32; 3], f32) {
    over_comps(c.comps, c.rules, c.unit, mode, v, cov, p, a)
}

/// [`over`] for a channel of `comps` components under `rules` (`unit`: the channel stores 0..1).
#[allow(clippy::too_many_arguments)]
fn over_comps(comps: usize, rules: BlendRules, unit: bool, mode: BlendMode, v: [f32; 3], cov: f32, p: [f32; 3], a: f32) -> ([f32; 3], f32) {
    if a <= 0.0 {
        return (v, cov);
    }
    let b = if cov > 0.0 { if comps == 3 { rules.blended3(mode, v, p, unit) } else { [rules.blended1(mode, v[0], p[0], unit); 3] } } else { p };
    let nc = a + cov * (1.0 - a);
    (mix3(v, b, a / nc.max(1e-5)), nc)
}

/// The plain UV projection a decal's frame coordinates are sampled through.
const DECAL_UV: Projection = Projection::Uv { scale: [1.0, 1.0], rotation: 0.0, offset: [0.0, 0.0] };

/// Per-texel context.
struct Texel<'a, 'b> {
    inp: &'b Inputs<'a>,
    x: u32,
    y: u32,
    /// Offset of this texel inside a tile (in texels).
    o: usize,
    res: u32,
    coords: Option<Coords>,
    stroke: f32,
    /// `eval` stops when it reaches this layer.
    stop: Option<LayerId>,
    /// Stack values recorded by the anchor points evaluated so far; true marks a mask anchor (gray, not channel values).
    anchors: Vec<(LayerId, Acc, bool)>,
    /// Where `eval` takes layer masks and content from.
    tape: Tape<'b>,
    /// The taps of the placed projection last asked for at this texel, by its address.
    placed: Option<(usize, Taps)>,
}

/// What a layer contributes (its mask, its content value and coverage per channel): evaluated
/// (`Off`), evaluated and recorded, or replayed from a recording.
enum Tape<'t> {
    Off,
    Record(Vec<f32>),
    Replay(&'t [f32], usize),
}

impl<'t> Tape<'t> {
    fn next(&mut self) -> f32 {
        self.take(1).first().copied().unwrap_or(0.0)
    }

    /// The next `n` replayed values (empty when not replaying).
    fn take(&mut self, n: usize) -> &'t [f32] {
        match self {
            Tape::Replay(t, i) => {
                let t: &'t [f32] = t;
                let end = i.saturating_add(n);
                let s = t.get(*i..end).unwrap_or_default();
                *i = end;
                s
            }
            _ => &[],
        }
    }
}

impl<'a, 'b> Texel<'a, 'b> {
    fn new(inp: &'b Inputs<'a>, x: u32, y: u32, o: usize, stroke: f32) -> Self {
        Texel { inp, x, y, o, res: inp.gbuf.width, coords: None, stroke, stop: None, anchors: Vec::new(), tape: Tape::Off, placed: None }
    }

    /// Run `f` with `tape` as this texel's tape.
    fn taped<R>(&mut self, tape: &mut Tape<'b>, f: impl FnOnce(&mut Self) -> R) -> R {
        std::mem::swap(&mut self.tape, tape);
        let r = f(self);
        std::mem::swap(&mut self.tape, tape);
        r
    }

    /// [`Texel::node_mask`] through the tape.
    fn tape_mask(&mut self, n: &Node) -> f32 {
        if let Tape::Replay(..) = self.tape {
            return self.tape.next();
        }
        let m = self.node_mask(n);
        if let Tape::Record(t) = &mut self.tape {
            t.push(m);
        }
        m
    }

    /// [`Texel::content`] through the tape: the coverage (0 where empty), then the value.
    fn tape_content(&mut self, content: &Content, c: &ChanPlan, below: Option<([f32; 3], f32)>) -> Option<([f32; 3], f32)> {
        if let Tape::Replay(..) = self.tape {
            let cov = self.tape.next();
            // Recorded as 0 where empty.
            if cov <= 0.0 {
                return None;
            }
            let v = if c.comps == 3 { [self.tape.next(), self.tape.next(), self.tape.next()] } else { [self.tape.next(); 3] };
            return Some((v, cov));
        }
        let r = self.content_over(content, c, usize::MAX, below);
        if let Tape::Record(t) = &mut self.tape {
            match r {
                Some((v, cov)) => {
                    t.push(cov);
                    let n = if c.comps == 3 { 3 } else { 1 };
                    t.extend_from_slice(&v[..n]);
                }
                None => t.push(0.0),
            }
        }
        r
    }

    fn coords(&mut self) -> Coords {
        if let Some(c) = self.coords {
            return c;
        }
        let g = self.inp.gbuf;
        let i = (self.y * g.width + self.x) as usize;
        let (lo, hi) = self.inp.bounds;
        let center = (lo + hi) * 0.5;
        let r = ((hi - lo).length() * 0.5).max(1e-4);
        let mut c = match g.surface(self.inp.mesh, i) {
            Some(s) => Coords { uv: s.uv, pos: (s.pos - center) / r, nrm: s.nrm },
            None => Coords { uv: Vec2::new((self.x as f32 + 0.5) / g.width as f32, (self.y as f32 + 0.5) / g.height as f32), pos: Vec3::ZERO, nrm: Vec3::Y },
        };
        // UV-tiled sets: projections see the mesh's own UVs (patterns repeat per tile).
        if let Ok(set) = self.inp.project.texture_set(self.inp.ts)
            && let Some(grid) = set.tile_grid()
        {
            c.uv = Vec2::from(set.from_atlas(&grid, c.uv.to_array()));
        }
        self.coords = Some(c);
        c
    }

    /// World position of this texel (None when uncovered).
    fn world(&self) -> Option<Vec3> {
        let i = (self.y * self.inp.gbuf.width + self.x) as usize;
        self.inp.gbuf.surface(self.inp.mesh, i).map(|s| s.pos)
    }

    fn baked(&self, m: BakedMap) -> Option<f32> {
        self.baked_color(m).map(|value| value[0])
    }

    fn baked_color(&self, map: BakedMap) -> Option<[f32; 3]> {
        self.inp.baked.sample_working(map, self.x, self.y, (self.res, self.inp.gbuf.height), &self.inp.block_scales)
    }

    /// Where projection `proj` samples this texel: the texel's coordinates, or for a decal its
    /// position in the decal's frame (then sampled as UVs) and the decal's coverage.
    fn projected<'p>(&mut self, proj: &'p Projection) -> (Coords, &'p Projection, f32) {
        let c = self.coords();
        let (lo, hi) = self.inp.bounds;
        let center = (lo + hi) * 0.5;
        match proj {
            Projection::Decal(d) => match self.world().and_then(|p| decal_uv_symmetric(d, p, c.nrm, center)) {
                Some((uv, w)) => (Coords { uv, ..c }, &DECAL_UV, w),
                None => (c, &DECAL_UV, 0.0),
            },
            _ => (c, proj, 1.0),
        }
    }

    /// How much a fill projected by `proj` covers this texel: 1, or for a decal its frame
    /// coverage times the alpha of its first image source.
    fn cover<'s>(&mut self, mut sources: impl Iterator<Item = &'s Source>, proj: &Projection) -> f32 {
        match proj {
            Projection::Decal(_) => {}
            Projection::Placed(p) => return self.placed_cover(p),
            _ => return 1.0,
        }
        let (c, _, w) = self.projected(proj);
        let image = sources.find_map(|s| match s {
            Source::Image { name, .. } => self.inp.images.get(name),
            _ => None,
        });
        w * image.map_or(1.0, |img| img.sample(c.uv.x, c.uv.y)[3].clamp(0.0, 1.0))
    }

    /// The stack value anchor point `anchor` recorded for `channel` (the channel default for an
    /// anchor not evaluated yet)
    fn anchored(&self, anchor: LayerId, channel: Channel) -> [f32; 3] {
        let values = self.anchors.iter().rev().find(|(id, ..)| *id == anchor).and_then(|(_, values, _)| values.get(channel.index()).copied());
        values.unwrap_or_else(|| channel.default_value())
    }

    /// Whether anchor point `anchor` sits in a mask, where every channel holds the mask's gray.
    fn is_mask_anchor(&self, anchor: LayerId) -> bool {
        self.anchors.iter().rev().find(|(id, ..)| *id == anchor).is_some_and(|(_, _, mask)| *mask)
    }

    /// Output `output` of a cook sampled through `projection`, as the graph holds it (RGBA); None without it
    fn graph_sample(&mut self, cooked: Option<&Cooked>, output: Option<&str>, projection: &Projection, unfiltered: bool) -> Option<[f32; 4]> {
        if let Projection::Placed(p) = projection {
            return self.placed_graph(cooked?, output, p, unfiltered);
        }
        let image = cooked?.output(output)?;
        // Laid on 1:1: the very texel (the render covers the texture set; interpolated mesh UVs would blur it at island
        // edges and seams)
        if matches!(projection, Projection::Uv { scale, rotation, offset } if *scale == [1.0, 1.0] && *rotation == 0.0 && *offset == [0.0, 0.0]) {
            return Some(graphs::at_texel(image, self.x, self.y, (self.inp.gbuf.width, self.inp.gbuf.height)));
        }
        let (coords, projection, _) = self.projected(projection);
        let footprint = if unfiltered { 0.0 } else { self.footprint(projection) };
        let cooked = cooked?;
        let mut sum = [0.0f32; 4];
        for (uv, weight) in
            image_samples_with_rules(projection, &coords, self.inp.project.rules.blend.is_measured()).into_iter().filter(|(_, weight)| *weight > 0.0)
        {
            let value = cooked.sample_filtered(output, uv.x, uv.y, footprint).unwrap_or_else(|| graphs::sample(image, uv.x, uv.y));
            for (total, value) in sum.iter_mut().zip(value) {
                *total += value * weight;
            }
        }
        Some(sum)
    }

    /// The size of this texel in the image coordinates `projection` reads ([`image_samples_with_rules`]): the UV tiling over the
    /// texels across a UV tile, or a volume projection's scale times the texel's size in projection space (from its
    /// triangle's area on the surface and in UV); 0 where it is not known (a texel off the mesh, the other projections).
    fn footprint(&mut self, projection: &Projection) -> f32 {
        let volume = match projection {
            Projection::Uv { scale, .. } => return scale[0].abs().max(scale[1].abs()) / self.inp.tile_res().max(1) as f32,
            Projection::Triplanar { scale, .. } => *scale,
            Projection::Planar { scale, .. } => 0.5 * *scale,
            // placed projections filter per tap (`placed_graph`)
            _ => return 0.0,
        };
        let g = self.inp.gbuf;
        let Some(&t) = g.tri.get((self.y * g.width + self.x) as usize) else { return 0.0 };
        let Some([a, b, c]) = self.inp.mesh.tri(t as usize) else { return 0.0 };
        let m = self.inp.mesh;
        let (Some(pa), Some(pb), Some(pc), Some(ta), Some(tb), Some(tc)) =
            (m.positions.get(a), m.positions.get(b), m.positions.get(c), m.uvs.get(a), m.uvs.get(b), m.uvs.get(c))
        else {
            return 0.0;
        };
        let (lo, hi) = self.inp.bounds;
        let r = ((hi - lo).length() * 0.5).max(1e-4);
        let surface = (*pb - *pa).cross(*pc - *pa).length() / (r * r);
        let uv = (*tb - *ta).perp_dot(*tc - *ta).abs();
        let len = (surface / uv).sqrt() / g.width.max(1) as f32;
        let f = volume.abs() * len;
        if f.is_finite() { f } else { 0.0 }
    }

    /// Where placed projection `p` samples this texel (computed once per texel and projection).
    fn placed_taps(&mut self, p: &PlacedProjection) -> Taps {
        let key = std::ptr::from_ref(p) as usize;
        if let Some((k, taps)) = self.placed
            && k == key
        {
            return taps;
        }
        let taps = placed::taps(p, self.inp.bounds, &self.placed_point());
        self.placed = Some((key, taps));
        taps
    }

    /// What a placed projection needs of this texel: its UVs, surface point and normal, its two axes on the surface.
    fn placed_point(&mut self) -> placed::Point {
        let uv = self.coords().uv;
        let texel_len = 1.0 / self.inp.tile_res().max(1) as f32;
        let i = (self.y * self.inp.gbuf.width + self.x) as usize;
        let Some(s) = self.inp.gbuf.surface(self.inp.mesh, i) else {
            return placed::Point { uv, nrm: Vec3::Y, texel_len, ..placed::Point::default() };
        };
        let (axes, world_len) = self.texel_axes().unwrap_or(([Vec3::ZERO; 2], 0.0));
        placed::Point { uv, pos: s.pos, nrm: s.nrm, axes, world_len, texel_len }
    }

    /// The world displacement of one texel step along x and along y on this texel's triangle (the surface's
    /// derivatives over its UVs), and the texel's isotropic size there; None off the mesh or on a degenerate triangle.
    fn texel_axes(&self) -> Option<([Vec3; 2], f32)> {
        let g = self.inp.gbuf;
        let m = self.inp.mesh;
        let &t = g.tri.get((self.y * g.width + self.x) as usize)?;
        let [a, b, c] = m.tri(t as usize)?;
        let (pa, pb, pc) = (*m.positions.get(a)?, *m.positions.get(b)?, *m.positions.get(c)?);
        let (ta, tb, tc) = (*m.uvs.get(a)?, *m.uvs.get(b)?, *m.uvs.get(c)?);
        let (e1, e2, d1, d2) = (pb - pa, pc - pa, tb - ta, tc - ta);
        let det = d1.x * d2.y - d2.x * d1.y;
        if !det.is_finite() || det.abs() < 1e-14 {
            return None;
        }
        let (w, h) = (g.width.max(1) as f32, g.height.max(1) as f32);
        let du = (e1 * d2.y - e2 * d1.y) / det;
        let dv = (e2 * d1.x - e1 * d2.x) / det;
        let len = (e1.cross(e2).length() / det.abs()).sqrt() / w;
        let axes = [du / w, dv / h];
        (axes.iter().all(|a| a.is_finite()) && len.is_finite()).then_some((axes, len))
    }

    /// How much placed projection `p` covers this texel: its coverage (clip, falloffs) times the share of its taps on
    /// the image (a tap outside a non-repeating image covers nothing).
    fn placed_cover(&mut self, p: &PlacedProjection) -> f32 {
        let taps = self.placed_taps(p);
        let axes = p.sampling_repeat().axes();
        let on: f32 = taps.used().iter().filter(|t| placed::covered(t.uv, axes)).map(|t| t.weight).sum();
        let c = taps.coverage * on;
        if c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 }
    }

    /// [`Self::placed_cover`] for a placed projection, 1 for the others (a graph laid on through `projection`).
    fn projection_cover(&mut self, projection: &Projection) -> f32 {
        match projection {
            Projection::Placed(p) => self.placed_cover(p),
            _ => 1.0,
        }
    }

    /// The taps of `p` read through `read` (a value at an image coordinate with a footprint, `None` outside), each
    /// weighted; the taps without a value are left out (their coverage is in [`Self::placed_cover`]).
    fn placed_read(&mut self, p: &PlacedProjection, mut read: impl FnMut(&placed::Tap) -> Option<[f32; 4]>) -> [f32; 4] {
        let taps = self.placed_taps(p);
        let (mut sum, mut total) = ([0.0f32; 4], 0.0f32);
        for tap in taps.used().iter().filter(|t| t.weight > 0.0) {
            if let Some(v) = read(tap) {
                for (s, v) in sum.iter_mut().zip(v) {
                    *s += v * tap.weight;
                }
                total += tap.weight;
            }
        }
        if total > 0.0 { sum.map(|s| s / total) } else { [0.0; 4] }
    }

    /// Output `output` of a cook through placed projection `p`: each tap prefiltered by its own footprint and probes
    /// (`unfiltered`: one bilinear read).
    fn placed_graph(&mut self, cooked: &Cooked, output: Option<&str>, p: &PlacedProjection, unfiltered: bool) -> Option<[f32; 4]> {
        let image = cooked.output(output)?;
        let side = image.width().max(image.height());
        let wrap = p.sampling_repeat().axes();
        Some(self.placed_read(p, |tap| placed::filtered(tap, side, unfiltered, |uv, footprint| cooked.sample_wrapped(output, uv.x, uv.y, footprint, wrap))))
    }

    /// An image through placed projection `p` (bilinear per tap).
    fn placed_image(&mut self, p: &PlacedProjection, image: &crate::store::ImageRes) -> [f32; 4] {
        let wrap = p.sampling_repeat().axes();
        self.placed_read(p, |tap| image.sample_wrapped(tap.uv.x, tap.uv.y, wrap))
    }

    /// The coverage of a fill source: 1, or a graph source's alpha output. A source reading its alpha output itself (a
    /// material's opacity output on the opacity channel) is laid whole: its value is the coverage, not gated by itself
    /// (the reference app's cut-out leaves show the material's opacity, not opacity² over the channel's default).
    fn source_alpha(&mut self, source: Option<&Source>, cooked: Option<&Cooked>, projection: &Projection) -> f32 {
        match source {
            Some(Source::Graph(graph)) if graph.alpha_output.is_some() && graph.output != graph.alpha_output => self
                .graph_sample(cooked, graph.alpha_output.as_deref(), projection, graph.unfiltered)
                .map_or(1.0, |alpha| if alpha[0].is_finite() { alpha[0].clamp(0.0, 1.0) } else { 0.0 }),
            _ => 1.0,
        }
    }

    /// A layer content source's coverage: [`Self::source_alpha`], and an image's own alpha (a fill laid with a
    /// partly transparent image covers only where the image is opaque; a decal's comes through [`Self::cover`]). Masks
    /// read images by their value alone.
    fn content_alpha(&mut self, source: Option<&Source>, cooked: Option<&Cooked>, projection: &Projection) -> f32 {
        match source {
            Some(Source::Image { name, .. })
                if matches!(projection, Projection::Placed(_)) || self.inp.project.rules.blend.is_measured() && !matches!(projection, Projection::Decal(_)) =>
            {
                let Some(image) = self.inp.images.get(name) else { return 1.0 };
                if let Projection::Placed(p) = projection {
                    let alpha = self.placed_image(p, image)[3];
                    return if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 0.0 };
                }
                let (coords, projection, _) = self.projected(projection);
                let alpha: f32 = image_samples_with_rules(projection, &coords, self.inp.project.rules.blend.is_measured())
                    .into_iter()
                    .filter(|(_, weight)| *weight > 0.0)
                    .map(|(uv, weight)| image.sample(uv.x, uv.y)[3] * weight)
                    .sum();
                if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 0.0 }
            }
            _ => self.source_alpha(source, cooked, projection),
        }
    }

    /// The value `source` gives this texel through `projection`, read as a channel of `kind` (`cooked` holds a graph source's outputs)
    fn source(&mut self, source: &Source, projection: &Projection, kind: ChannelKind, cooked: Option<&Cooked>) -> [f32; 3] {
        match source {
            Source::Graph(graph) => match self.graph_sample(cooked, graph.output.as_deref(), projection, graph.unfiltered) {
                Some(value) => graphs::graph_to_channel(kind, value, cooked.is_some_and(|c| c.is_linear(graph.output.as_deref()))),
                // Not rendered (the reason is in the graph store's problems)
                None => [0.5; 3],
            },
            Source::Uniform(value) => *value,

            Source::Anchor { anchor, channel, ramp } if kind == ChannelKind::Color && !ramp.is_empty() => {
                let value = self.anchored(*anchor, *channel);

                // Signed values (height) span -1..1; a mask anchor's gray already spans 0..1 in every channel
                let signed = channel.kind() == ChannelKind::Signed && !self.is_mask_anchor(*anchor);
                let lightness = if signed { lum(value) * 0.5 + 0.5 } else { lum(value) };
                texturecraft_core::gradient(ramp, lightness).unwrap_or_default()
            }
            Source::Procedural { pattern, low, high, ramp } => {
                let (coords, projection, _) = self.projected(projection);
                let pixels = if pattern.resolution.can_reduce() {
                    let uv_tile = self.inp.tile_res_at(self.x, self.y);
                    let texture_set = self.inp.project.texture_set(self.inp.ts).map_or(uv_tile, |set| set.resolution);
                    pattern_pixels(pattern, uv_tile, texture_set)
                } else {
                    None
                };
                let context = PatternContext { pixels, images: Some(self.inp.images) };
                let amount = if let Projection::Placed(p) = projection {
                    self.placed_read(p, |tap| Some([procedural_in(pattern, projection, &Coords { uv: tap.uv, ..coords }, &context); 4]))[0]
                } else {
                    procedural_in(pattern, projection, &coords, &context)
                };

                if kind == ChannelKind::Color && !ramp.is_empty() {
                    texturecraft_core::gradient(ramp, amount).unwrap_or_default()
                } else {
                    mix3(*low, *high, amount)
                }
            }
            Source::Image { name, normal_format } => {
                let Some(image) = self.inp.images.get(name) else { return [0.5; 3] };

                let sum = match projection {
                    Projection::Placed(p) => self.placed_image(p, image),
                    _ => {
                        let (coords, projection, _) = self.projected(projection);
                        let mut sum = [0.0f32; 4];
                        for (uv, weight) in image_samples_with_rules(projection, &coords, self.inp.project.rules.blend.is_measured())
                            .into_iter()
                            .filter(|(_, weight)| *weight > 0.0)
                        {
                            for (total, value) in sum.iter_mut().zip(image.sample(uv.x, uv.y)) {
                                *total += value * weight;
                            }
                        }
                        sum
                    }
                };

                let rgb = [sum[0], sum[1], sum[2]];
                match kind {
                    ChannelKind::Color if image.srgb => rgb.map(texturecraft_texel::srgb_to_linear),

                    // Normal channels are OpenGL (+Y up the image): a DirectX image's green flips
                    ChannelKind::NormalMap if *normal_format == NormalFormat::DirectX => [rgb[0], 1.0 - rgb[1], rgb[2]],
                    ChannelKind::Color | ChannelKind::NormalMap | ChannelKind::Data => rgb,
                    // A one-value channel or mask reads the image's gray, so a color image works as a mask
                    ChannelKind::Scalar => [lum(rgb); 3],
                    ChannelKind::Signed => [lum(rgb) * 2.0 - 1.0; 3],
                }
            }
            Source::Anchor { anchor, channel, .. } => self.anchor_value(*anchor, *channel, kind),
        }
    }

    /// The value anchor point `anchor` recorded for `channel`, read as `kind` (colors read as gray
    /// where a scalar is needed). An anchor not evaluated yet (above this layer, hidden) reads the
    /// channel default.
    fn anchor_value(&self, anchor: LayerId, channel: Channel, kind: ChannelKind) -> [f32; 3] {
        let value = self.anchored(anchor, channel);
        let needs_gray = matches!(channel.kind(), ChannelKind::Color | ChannelKind::NormalMap | ChannelKind::Data)
            && matches!(kind, ChannelKind::Scalar | ChannelKind::Signed);
        if needs_gray { [lum(value); 3] } else { value }
    }

    fn generator_ramp(&mut self, generator: &Generator, curvature_sizes: Option<&CurvatureSizes>) -> f32 {
        let coords = self.coords();
        let micro = generator.micro.map_or(0.0, |micro| self.micro_detail(generator.kind, micro));
        let signal = match (generator.kind, &generator.detail) {
            (GeneratorKind::Curvature | GeneratorKind::EdgeWear, Some(detail)) => {
                // Detail sizes are blurred copies of the mesh map, so a replaced curvature input reads at its own size only
                let sizes = if generator.inputs.contains_key(&BakedMap::Curvature) { None } else { curvature_sizes };
                let curvature = self.generator_input(generator, BakedMap::Curvature);
                self.curvature_detail(detail, sizes, curvature, micro)
            }
            (GeneratorKind::EdgeWear, None) => (self.generator_input(generator, BakedMap::Curvature).unwrap_or(0.5) - 0.5) * 2.0,
            (GeneratorKind::Curvature, None) => self.generator_input(generator, BakedMap::Curvature).unwrap_or(0.5),
            (GeneratorKind::Dirt, _) => 1.0 - self.generator_input(generator, BakedMap::AmbientOcclusion).unwrap_or(1.0),
            (GeneratorKind::AmbientOcclusion, _) => self.generator_input(generator, BakedMap::AmbientOcclusion).unwrap_or(1.0),
            (GeneratorKind::ThinAreas, _) => 1.0 - self.generator_input(generator, BakedMap::Thickness).unwrap_or(1.0),
            (GeneratorKind::Position, _) => coords.pos.dot(Vec3::from(generator.direction).try_normalize().unwrap_or(Vec3::Y)) * 0.5 + 0.5,
            (GeneratorKind::Direction, _) => coords.nrm.dot(Vec3::from(generator.direction).try_normalize().unwrap_or(Vec3::Y)) * 0.5 + 0.5,
        };
        let signal =
            if matches!(generator.kind, GeneratorKind::Curvature | GeneratorKind::EdgeWear) && generator.detail.is_some() { signal } else { signal + micro };
        let signal = if generator.noise_amount > 0.0 {
            let noise = world_noise(coords.pos, generator.noise_scale, generator.seed);
            if let (GeneratorKind::Curvature | GeneratorKind::EdgeWear, Some(detail)) = (generator.kind, &generator.detail) {
                // Break up curvature without inventing edges or cavities on a flat surface
                let neutral = if detail.mode == CurvatureMode::Raw { 0.5 } else { 0.0 };
                neutral + (signal - neutral) * (1.0 - noise * generator.noise_amount).max(0.0)
            } else {
                signal + (noise - 0.5) * generator.noise_amount
            }
        } else {
            signal
        };

        // A smooth step from black to white over 2 × softness around 1 − level
        let threshold = 1.0 - generator.level.clamp(0.0, 1.0);
        let softness = generator.softness.max(Generator::MIN_SOFTNESS);
        let ramp = ((signal - (threshold - softness)) / (2.0 * softness)).clamp(0.0, 1.0);
        ramp * ramp * (3.0 - 2.0 * ramp)
    }

    /// How a generator's micro detail moves its signal at this texel: the anchor's height (0 where
    /// the anchor is not evaluated yet) as an edge where raised and a cavity where recessed.
    fn micro_detail(&self, kind: GeneratorKind, micro: MicroDetail) -> f32 {
        let height = self.anchor_value(micro.anchor, Channel::Height, ChannelKind::Signed)[0];
        let amount = if micro.amount.is_finite() { micro.amount.clamp(0.0, 1.0) } else { 0.0 };
        let response = height.clamp(-1.0, 1.0) * amount;
        match kind {
            GeneratorKind::Curvature | GeneratorKind::EdgeWear => response,

            // Dirt gathers in dents; dents are occluded
            GeneratorKind::Dirt => -response,
            GeneratorKind::AmbientOcclusion => response,
            GeneratorKind::ThinAreas | GeneratorKind::Position | GeneratorKind::Direction => 0.0,
        }
    }

    /// The generator's value at this texel: its ramp finished by balance, contrast, invert and offset (its blur and resolution need neighbours: see [`generator_region`]).
    fn generator(&mut self, generator: &Generator, curvature_sizes: Option<&CurvatureSizes>) -> f32 {
        let ramp = self.generator_ramp(generator, curvature_sizes);
        finish_generator(generator, ramp)
    }

    /// Generator input `map` at this texel: what replaces the mesh map in the generator's inputs, else the mesh map (None when not baked).
    fn generator_input(&mut self, generator: &Generator, map: BakedMap) -> Option<f32> {
        match generator.inputs.get(&map) {
            Some(source) => Some(self.source(source, &Projection::default(), ChannelKind::Scalar, None)[0]),
            None => self.baked(map),
        }
    }

    /// The curvature of `detail`'s mode at this texel from `curvature` (None: nothing to read), with its blurred detail sizes when given.
    fn curvature_detail(&self, detail: &CurvatureDetail, curvature_sizes: Option<&CurvatureSizes>, curvature: Option<f32>, micro: f32) -> f32 {
        let combine = |edges: f32, cavities: f32| {
            let edges = 1.0 - (1.0 - edges) * (1.0 - micro.max(0.0));
            let cavities = 1.0 - (1.0 - cavities) * (1.0 + micro.min(0.0));
            match detail.mode {
                CurvatureMode::Edges => edges,
                CurvatureMode::Cavities => cavities,
                CurvatureMode::Both => 1.0 - (1.0 - edges) * (1.0 - cavities),
                CurvatureMode::Raw => 0.5 + 0.5 * (edges - cavities),
            }
        };

        // Without baked curvature, only painted micro height supplies edges or cavities
        let Some(curvature) = curvature else { return combine(0.0, 0.0) };

        let working_size = (self.res, self.inp.gbuf.height);
        let texel_uv = [(self.x as f32 + 0.5) / working_size.0.max(1) as f32, (self.y as f32 + 0.5) / working_size.1.max(1) as f32];
        let Some(baked_uv) = self.inp.block_scales.bake_uv(texel_uv, working_size) else { return combine(0.0, 0.0) };

        let baked_size = (self.inp.baked.resolution, self.inp.baked.height(BakedMap::Curvature));
        let (baked_x, baked_y) = ((baked_uv[0] * baked_size.0 as f32) as u32, (baked_uv[1] * baked_size.1 as f32) as u32);
        let blurred = |index: usize| {
            curvature_sizes.and_then(|sizes| sizes.levels.get(index)).and_then(Option::as_deref).map_or(0.0, |level| level.sample(baked_x, baked_y))
        };
        let (mut edges, mut cavities) = (0.0f32, 0.0f32);
        for (index, (size, weight)) in CurvatureDetail::SIZES.iter().zip(detail.weights).enumerate() {
            let deviation = if index == 0 { curvature - 0.5 } else { blurred(index - 1) };

            let response = deviation * if *size == 0.0 { MAP_GAIN } else { GAIN_PER_DETAIL_TEXEL * size };
            let weight = weight.clamp(0.0, 1.0);
            edges = 1.0 - (1.0 - edges) * (1.0 - weight * response.clamp(0.0, 1.0));
            cavities = 1.0 - (1.0 - cavities) * (1.0 - weight * (-response).clamp(0.0, 1.0));
        }

        combine(edges, cavities)
    }

    fn mask(&mut self, plan: &MaskPlan) -> f32 {
        let mut value = plan.background;
        let mut skip = 0;
        if let Some((filtered_index, field)) = plan.start
            && let Some([filtered, ..]) = field.get(self.x, self.y)
        {
            value = filtered;
            skip = filtered_index + 1;
            for (anchor, recorded) in field.marks_at(self.x, self.y) {
                self.anchors.push((anchor, Acc::uniform(recorded), true));
            }
        }

        let rules = self.inp.project.rules.blend;
        // Avoid amplifying rounding error when unpremultiplying faint paint
        const MIN_PAINT_COVERAGE: f32 = 1e-5;

        for effect in plan.effects.iter().skip(skip) {
            let opacity = effect.opacity.clamp(0.0, 1.0);
            match effect.kind {
                EffectKind::Paint | EffectKind::Path(_) => {
                    let (mut painted, mut coverage) = match effect.data.as_deref() {
                        Some(data) => (from_u16(data.get(self.o * 2).copied().unwrap_or(0)), from_u16(data.get(self.o * 2 + 1).copied().unwrap_or(0))),
                        None => (0.0, 0.0),
                    };
                    if effect.preview
                        && let Some(preview) = self.inp.preview
                    {
                        let stroke = (self.stroke * preview.opacity).clamp(0.0, 1.0);
                        if preview.erase {
                            painted *= 1.0 - stroke;
                            coverage *= 1.0 - stroke;
                        } else {
                            // Clone and smudge show the values they carry instead of the mask gray
                            let deposited = preview.carried_mask.as_ref().map_or(preview.mask_value, |carried| from_u16(carried.get(self.x, self.y)[0]));
                            painted = deposited * stroke + painted * (1.0 - stroke);
                            coverage = stroke + coverage * (1.0 - stroke);
                        }
                    }
                    if coverage > 0.0 {
                        let layer_value = painted / coverage.max(MIN_PAINT_COVERAGE);
                        value += (rules.blended1(effect.blend, value, layer_value, true) - value) * coverage * opacity;
                    }
                }
                EffectKind::Fill { source, projection } => {
                    let cooked = effect.graph.as_deref();
                    let fill = self.source(source, projection, ChannelKind::Scalar, cooked)[0];
                    let cover = self.cover(std::iter::once(source), projection) * self.source_alpha(Some(source), cooked, projection);
                    value += (rules.blended1(effect.blend, value, fill, true) - value) * opacity * cover;
                }
                EffectKind::Generator(generator) => {
                    let generated = self.generator(generator, effect.curvature_sizes.as_deref());
                    value += (rules.blended1(effect.blend, value, generated, true) - value) * opacity;
                }
                EffectKind::Levels(levels) => value += (levels.apply(value) - value) * opacity,
                EffectKind::Filter(filter) => {
                    // A mask is gray (HSL only changes its lightness); neighbourhood filters come precomputed through `MaskPlan::start`
                    let filtered = filter.point(ChannelKind::Scalar, [value; 3]).map_or(value, |gray| gray[0]);
                    value += (filtered - value) * opacity;
                }
                EffectKind::ColorSelection { colors, tolerance, map } => {
                    let id = self.baked_color(*map);
                    let within = |picked: &[f32; 3], color: [f32; 3]| (0..3).all(|component| (picked[component] - color[component]).abs() <= *tolerance);
                    let selected = if id.is_some_and(|color| colors.iter().any(|picked| within(picked, color))) { 1.0 } else { 0.0 };
                    value += (rules.blended1(effect.blend, value, selected, true) - value) * opacity;
                }
                EffectKind::Compare { op, value: compared, tolerance } => {
                    let pass = match op {
                        CompareOp::LesserThan => value < *compared,
                        CompareOp::GreaterThan => value > *compared,
                        CompareOp::WithinTolerance => (value - compared).abs() <= *tolerance,
                    };
                    let result = if pass { 1.0 } else { 0.0 };
                    value += (result - value) * opacity;
                }
                // A graph reading the stack comes precomputed through `MaskPlan::start`; one that could not be rendered
                // changes nothing
                EffectKind::Graph { graph, projection } => {
                    if let Some(v) = self.graph_sample(effect.graph.as_deref(), graph.output.as_deref(), projection, graph.unfiltered) {
                        let cover = self.projection_cover(projection);
                        value += (rules.blended1(effect.blend, value, v[0], true) - value) * opacity * cover;
                    }
                }
                // An anchor point inside a mask (imported projects) records the mask so far for fills reading it
                EffectKind::Anchor => self.anchors.push((effect.id, Acc::uniform(value.clamp(0.0, 1.0)), true)),
                // Content-only effects; a placeholder passes the mask through unchanged
                EffectKind::Material(_) | EffectKind::Placeholder { .. } => {}
            }
        }
        value.clamp(0.0, 1.0)
    }

    /// Composite `nodes` over `acc`, recording anchors. Returns true when it reached the `stop`
    /// layer (`acc` then holds the stack below it).
    fn eval(&mut self, nodes: &[Node], acc: &mut Acc) -> bool {
        for n in nodes {
            if self.stop == Some(n.id) {
                return true;
            }

            let m = self.tape_mask(n);
            match &n.content {
                // Masked-out groups still run when they record anchors or hold the stop layer
                Content::Group(children) if m > 0.0 || n.has_anchor || self.stop.is_some() => {
                    let mut inner = group_start(acc, &n.chans);
                    if self.eval(children, &mut inner) {
                        // stopped inside: the stack below the stop layer is the folder's inner stack
                        *acc = inner;
                        return true;
                    }

                    if m > 0.0 {
                        mix_group(acc, &inner, &n.chans, m);
                    }
                }
                Content::Fill(_) | Content::Paint { .. } if m > 0.0 => {
                    let before = if n.anchors.iter().any(|a| a.1.is_some()) { Some(*acc) } else { None };
                    for c in &n.chans {
                        let below = acc.below(c);
                        if let Some((v, cov)) = self.tape_content(&n.content, c, below) {
                            apply(acc, c, v, cov, m);
                        }
                    }

                    // Anchors inside the content stack read the stack cut at that point
                    for (a, cut) in &n.anchors {
                        let rec = match (cut, before) {
                            (Some(k), Some(mut b)) => {
                                for c in &n.chans {
                                    let below = b.below(c);
                                    if let Some((v, cov)) = self.content_over(&n.content, c, *k, below) {
                                        apply(&mut b, c, v, cov, m);
                                    }
                                }
                                b
                            }
                            _ => *acc,
                        };
                        self.anchors.push((*a, rec, false));
                    }
                    continue;
                }
                _ => {}
            }
            for (a, _) in &n.anchors {
                self.anchors.push((*a, *acc, false));
            }
        }
        false
    }

    /// [`Texel::eval`] of the whole stack over `acc`, split along the path to the hot layer
    /// (`levels`, top level first). Cold (`warm` false) it evaluates everything, leaving in
    /// `accs` the stack before the path layer of each level and recording on `tape` the masks
    /// of the folders on the path and what the layers after the path contribute. Warm (`accs`
    /// loaded from the cache, `tape` replaying) only the hot layer is evaluated and the blends
    /// after it are replayed. The order of evaluation differs from `eval` only in masks, which
    /// read no stack values (given no anchor readers, see `BelowCache::prepare`), so the result
    /// is identical.
    fn eval_split(&mut self, mut acc: Acc, levels: &[LevelPlan], accs: &mut [Acc], masks: &mut [f32], warm: bool, tape: &mut Tape<'b>) -> Acc {
        let last = levels.len().saturating_sub(1);
        // Down the path: the deepest level the stack reaches (a folder masked out or not drawn
        // here stops it).
        let mut reach = last;
        for (d, lv) in levels.iter().enumerate() {
            if !warm {
                self.eval(&lv.below, &mut acc);
                if let Some(slot) = accs.get_mut(d) {
                    *slot = acc;
                }
            }
            if d == last {
                break;
            }
            let m = match &lv.at {
                Some(g) => self.taped(tape, |t| t.tape_mask(g)),
                None => 0.0,
            };
            if let Some(slot) = masks.get_mut(d) {
                *slot = m;
            }
            if m > 0.0 {
                continue;
            }
            reach = d;
            break;
        }
        let mut acc = accs.get(reach).copied().unwrap_or(acc);
        if reach == last
            && let Some(hot) = levels.get(last).and_then(|l| l.at.as_ref())
        {
            self.eval(std::slice::from_ref(hot), &mut acc);
        }
        // Back up: the layers after the path, then each folder's mix over the stack outside it.
        for d in (0..=reach).rev() {
            if let Some(lv) = levels.get(d) {
                self.taped(tape, |t| t.eval(&lv.above, &mut acc));
            }
            if let Some(o) = d.checked_sub(1)
                && let (Some(outer), Some(g), Some(m)) = (accs.get(o), levels.get(o).and_then(|l| l.at.as_ref()), masks.get(o))
            {
                let inner = acc;
                acc = *outer;
                mix_group(&mut acc, &inner, &g.chans, *m);
            }
        }
        acc
    }

    /// The mask of node `n` at this texel, combined with the layer's height when it masks with
    /// height.
    fn node_mask(&mut self, n: &Node) -> f32 {
        if n.geometry.as_ref().is_some_and(|g| !self.in_geometry(g)) {
            return 0.0;
        }
        let m = match &n.mask {
            Some(mp) => self.mask(mp),
            None => 1.0,
        };
        if !n.height_mask.enabled || m <= 0.0 || m >= 1.0 {
            return m;
        }
        let height = Channel::Height.index();
        let h = n.chans.iter().find(|c| c.ch == height).and_then(|c| self.content(&n.content, c)).map_or(0.0, |(v, _)| v[0]);
        n.height_mask.apply(m, h)
    }

    /// Whether this texel lies in what geometry mask `g` shows.
    fn in_geometry(&self, g: &Geometry) -> bool {
        match g {
            Geometry::Parts(allowed) => self.in_parts(allowed),
            Geometry::Tiles { cols, rows, block, allowed } => {
                let (bx, by) = ((self.x / block.0).min(cols.saturating_sub(1)), (self.y / block.1).min(rows.saturating_sub(1)));
                allowed.get((by * cols + bx) as usize).copied().unwrap_or(true)
            }
        }
    }

    /// Whether this texel's triangle lies in a submesh `allowed` marks (uncovered texels and
    /// meshes without submeshes count as inside).
    fn in_parts(&self, allowed: &[bool]) -> bool {
        let g = self.inp.gbuf;
        let tri = g.tri.get((self.y * g.width + self.x) as usize).copied().unwrap_or(NO_TRI);
        if tri == NO_TRI {
            return true;
        }
        let first = tri.saturating_mul(3);
        let subs = &self.inp.mesh.submeshes;
        let holds = |k: usize| subs.get(k).is_some_and(|s| s.start <= first && first < s.start.saturating_add(s.count));
        // Submeshes are in index order; anything else is searched.
        let k = subs.partition_point(|s| s.start <= first).checked_sub(1).filter(|k| holds(*k)).or_else(|| (0..subs.len()).find(|k| holds(*k)));
        k.and_then(|k| allowed.get(k).copied()).unwrap_or(true)
    }

    /// Record the anchors of the stack below layer `id` (`full` is the planned stack), to
    /// evaluate that layer on its own.
    fn anchors_below(&mut self, full: &[Node], id: LayerId) -> Acc {
        self.anchors.clear();
        self.stop = Some(id);
        let mut acc = Acc::defaults();
        self.eval(full, &mut acc);
        self.stop = None;
        acc
    }

    /// Straight value and coverage of one channel of a paint or fill layer at this texel
    /// (content effects and live stroke preview included). None where the layer is empty.
    fn content(&mut self, content: &Content, c: &ChanPlan) -> Option<([f32; 3], f32)> {
        self.content_to(content, c, usize::MAX)
    }

    /// [`Texel::content`] with only the first `end` visible content effects.
    fn content_to(&mut self, content: &Content, c: &ChanPlan, end: usize) -> Option<([f32; 3], f32)> {
        self.content_over(content, c, end, None)
    }

    /// [`Texel::content_to`] starting from `below` when given (an effect layer's channel: the stack below it, shown
    /// value and coverage, with the layer's own paint or fill laid over it before its effects). TextureCraft's own
    /// rules take the stack as fully covered; the measured rules keep its coverage, so over nothing an effect layer
    /// filters nothing and adds only its own paint or fill. A precomputed field already holds it.
    fn content_over(&mut self, content: &Content, c: &ChanPlan, end: usize, below: Option<([f32; 3], f32)>) -> Option<([f32; 3], f32)> {
        let start = c.start.filter(|(k, _)| *k < end);
        let (mut v, mut cov, skip) = match start.and_then(|(k, f)| f.get(self.x, self.y).map(|d| (k, d))) {
            Some((k, d)) => ([d[0], d[1], d[2]], d[3], k + 1),
            None => {
                let (v, cov) = match content {
                    Content::Fill(f) => {
                        let ch = Channel::ALL.get(c.ch).copied()?;
                        // A channel without a source fills with its default (no clone per texel).
                        let cooked = c.graph.as_deref();
                        let v = match f.sources.get(&ch) {
                            Some(s) => self.source(s, &f.projection, ch.kind(), cooked),
                            None => ch.default_value(),
                        };
                        (v, self.cover(f.sources.values(), &f.projection) * self.content_alpha(f.sources.get(&ch), cooked, &f.projection))
                    }
                    Content::Paint { preview } => self.painted(c.data.as_deref(), c, *preview),
                    Content::Group(_) => return None,
                };
                match below {
                    Some((b, a)) if c.rules.is_measured() => {
                        let empty = Channel::ALL.get(c.ch).map_or([0.0; 3], |d| d.default_value());
                        let a = if a.is_finite() { a.clamp(0.0, 1.0) } else { 0.0 };
                        let b = if a > 0.0 { [0, 1, 2].map(|i| (b[i] - empty[i] * (1.0 - a)) / a) } else { empty };
                        let (v, cov) = over(c, BlendMode::Normal, b, a, v, cov.clamp(0.0, 1.0));
                        (v, cov, 0)
                    }
                    Some((b, _)) => (mix3(b, v, cov.clamp(0.0, 1.0)), 1.0, 0),
                    None => (v, cov, 0),
                }
            }
        };
        for e in c.effects.iter().take(end).skip(skip) {
            (v, cov) = self.content_effect(e, c, v, cov);
        }
        (cov > 0.0).then_some((v, cov.min(1.0)))
    }

    /// Straight value and coverage of painted data of one channel (premultiplied values plus
    /// coverage per texel), with the live stroke when `preview`.
    fn painted(&mut self, data: Option<&[u16]>, c: &ChanPlan, preview: bool) -> ([f32; 3], f32) {
        let pv = if preview { self.inp.preview } else { None };
        let mut sa = pv.map(|p| (self.stroke * p.opacity).clamp(0.0, 1.0)).unwrap_or(0.0);
        // Projection: the stencil's alpha shapes the stroke and its color is the base color.
        let stencil = match pv.and_then(|p| p.stencil.as_ref()) {
            Some(st) if sa > 0.0 => {
                let s = self.world().and_then(|w| st.sample_world(w));
                sa *= s.map(|(_, a)| a).unwrap_or(0.0);
                s.map(|(c, _)| c)
            }
            _ => None,
        };
        let mut val = [0.0f32; 3];
        let mut cov = 0.0f32;
        if let Some(d) = data {
            let base = self.o * (c.comps + 1);
            for (k, slot) in val.iter_mut().take(c.comps).enumerate() {
                *slot = decode(c.kind, d.get(base + k).copied().unwrap_or(0));
            }
            cov = from_u16(d.get(base + c.comps).copied().unwrap_or(0));
        }
        if let Some(p) = pv
            && sa > 0.0
        {
            if p.erase {
                val = val.map(|v| v * (1.0 - sa));
                cov *= 1.0 - sa;
            } else if let Some(ch) = Channel::ALL.get(c.ch)
                && let Some(pval) = p.values.get(ch)
            {
                let pval = match stencil {
                    Some(col) if *ch == Channel::BaseColor => col,
                    _ => match p.carried.get(ch) {
                        Some(img) => {
                            let raw = img.get(self.x, self.y).map(|v| decode(c.kind, v));
                            [raw[0], raw[1], raw[2]]
                        }
                        None => *pval,
                    },
                };
                for k in 0..c.comps {
                    val[k] = pval[k] * sa + val[k] * (1.0 - sa);
                }
                cov = sa + cov * (1.0 - sa);
            }
        }
        if cov <= 0.0 {
            return ([0.0; 3], 0.0);
        }
        let inv = 1.0 / cov.max(1e-5);
        let v = if c.comps == 3 { val.map(|x| x * inv) } else { [val[0] * inv; 3] };
        (v, cov.min(1.0))
    }

    /// Apply one content effect to a channel's straight value and coverage.
    fn content_effect(&mut self, e: &EffectPlan, c: &ChanPlan, v: [f32; 3], cov: f32) -> ([f32; 3], f32) {
        if !e.acts {
            return (v, cov);
        }
        let o = e.opacity.clamp(0.0, 1.0);
        match e.kind {
            EffectKind::Paint | EffectKind::Path(_) => {
                let (p, pc) = self.painted(e.data.as_deref(), c, e.preview);
                over(c, e.blend, v, cov, p, pc * o)
            }
            EffectKind::Material(f) => match Channel::ALL.get(c.ch).and_then(|ch| f.sources.get(ch).map(|s| (*ch, s))) {
                Some((ch, s)) => {
                    let cooked = e.graph.as_deref();
                    let p = self.source(s, &f.projection, ch.kind(), cooked);
                    let w = o * self.cover(f.sources.values(), &f.projection) * self.content_alpha(Some(s), cooked, &f.projection);
                    over(c, e.blend, v, cov, p, w)
                }
                None => (v, cov),
            },
            EffectKind::Levels(l) => (mix3(v, per_unit(c.kind, v, |x| l.apply(x)), o), cov),
            // Baked lighting reads the texel's normal and occlusion, and lights the base color only.
            EffectKind::Filter(f @ Filter::BakedLighting { .. }) => match Channel::ALL.get(c.ch) {
                Some(Channel::BaseColor) => {
                    let nrm = self.coords().nrm.to_array();
                    let k = f.baked_light(nrm, self.baked(BakedMap::AmbientOcclusion).unwrap_or(1.0)).unwrap_or(1.0);
                    (mix3(v, v.map(|x| (x * k).clamp(0.0, 1.0)), o), cov)
                }
                _ => (v, cov),
            },
            // Neighbourhood filters come precomputed through `ChanPlan::start`.
            EffectKind::Filter(f) => (f.point(c.kind, v).map_or(v, |out| mix3(v, out, o)), cov),
            // A graph reading nothing of the stack laid on like a fill: its output for the channel (a graph reading the
            // stack comes precomputed through `ChanPlan::start`).
            EffectKind::Graph { graph, projection } => match Channel::ALL.get(c.ch).and_then(|ch| Some((*ch, graph.output_for(*ch)?))) {
                Some((ch, output)) => {
                    let cooked = e.graph.as_deref();
                    match self.graph_sample(cooked, output, projection, graph.unfiltered) {
                        Some(g) => {
                            let p = graphs::graph_to_channel(ch.kind(), g, cooked.is_some_and(|k| k.is_linear(output)));
                            let alpha = match graph.output_alphas.get(&ch).map(String::as_str).or(graph.alpha_output.as_deref()) {
                                Some(a) => self.graph_sample(cooked, Some(a), projection, graph.unfiltered).map_or(1.0, |a| a[0].clamp(0.0, 1.0)),
                                None => 1.0,
                            };
                            let cover = self.projection_cover(projection);
                            over(c, e.blend, v, cov, p, o * alpha * cover)
                        }
                        None => (v, cov),
                    }
                }
                None => (v, cov),
            },
            // Anchors are recorded by `eval`; mask-only effects and placeholders leave content as it is.
            EffectKind::Anchor
            | EffectKind::Fill { .. }
            | EffectKind::Generator(_)
            | EffectKind::ColorSelection { .. }
            | EffectKind::Compare { .. }
            | EffectKind::Placeholder { .. } => (v, cov),
        }
    }
}

/// True if folder channel `c` composites its children on their own: under the measured rules, a folder whose mode is
/// not passthrough.
fn isolates(c: &ChanPlan) -> bool {
    c.rules.is_measured() && c.blend != BlendMode::Passthrough
}

/// The stack a folder's children start from: the stack below it, except on the channels the folder isolates
/// ([`isolates`]), which start from nothing.
fn group_start(acc: &Acc, chans: &[ChanPlan]) -> Acc {
    let mut inner = *acc;
    for c in chans.iter().filter(|c| isolates(c)) {
        if let (Some(slot), Some(cov), Some(d)) = (inner.v.get_mut(c.ch), inner.cov.get_mut(c.ch), Channel::ALL.get(c.ch)) {
            *slot = d.default_value();
            *cov = 0.0;
        }
    }
    inner
}

/// Mix a folder's inner stack into the stack outside it by its opacity and mask `m`: a straight mix, or on the
/// channels it isolates ([`isolates`]) its own composite laid over with the folder's mode.
fn mix_group(acc: &mut Acc, inner: &Acc, chans: &[ChanPlan], m: f32) {
    for c in chans {
        let (Some(a), Some(b), Some(&ac), Some(&bc)) = (acc.get(c.ch).copied(), inner.get(c.ch).copied(), acc.cov.get(c.ch), inner.cov.get(c.ch)) else {
            continue;
        };
        let (v, cov) = if isolates(c) {
            // The folder's composite over nothing at its coverage: straight for the blend, as it shows (over the
            // channel default) for replace, which lays the whole composite.
            let empty = Channel::ALL.get(c.ch).map_or([0.0; 3], |d| d.default_value());
            let top = if c.blend == BlendMode::Replace || bc <= 0.0 { b } else { [0, 1, 2].map(|i| (b[i] - empty[i] * (1.0 - bc)) / bc) };
            mix_over(c.blend, a, ac, top, bc, c.opacity * m, empty, c.unit, c.comps)
        } else {
            let w = c.opacity * m;
            (mix3(a, b, w), ac + (bc - ac) * w)
        };
        if let (Some(slot), Some(slot_cov)) = (acc.v.get_mut(c.ch), acc.cov.get_mut(c.ch)) {
            *slot = v;
            *slot_cov = cov;
        }
    }
}

#[inline]
fn apply(acc: &mut Acc, c: &ChanPlan, v: [f32; 3], cov: f32, mask: f32) {
    let Some(a) = acc.get(c.ch).copied() else { return };
    if c.rules.is_measured() {
        let (Some(&below), Some(empty)) = (acc.cov.get(c.ch), Channel::ALL.get(c.ch).map(|d| d.default_value())) else { return };
        let (out, out_cov) = if c.from_below {
            // An effect layer's content is the stack below carried on: it takes the stack's place, value and
            // coverage, by the layer's strength.
            let shown = [0, 1, 2].map(|i| v[i] * cov + empty[i] * (1.0 - cov));
            mix_over(BlendMode::Replace, a, below, shown, cov, c.opacity * mask, empty, c.unit, c.comps)
        } else {
            mix_over(c.blend, a, below, v, cov, c.opacity * mask, empty, c.unit, c.comps)
        };
        if let (Some(slot), Some(slot_cov)) = (acc.v.get_mut(c.ch), acc.cov.get_mut(c.ch)) {
            *slot = out;
            *slot_cov = out_cov;
        }
        return;
    }
    let w = if c.blend == BlendMode::Replace { c.opacity * mask } else { cov * c.opacity * mask };
    let blended = blend_chan(c, c.blend, a, v);
    if let Some(slot) = acc.get_mut(c.ch) {
        *slot = mix3(a, blended, w);
    }
}

/// Composite `tiles` of texture set `inp.ts` into `out` (other tiles are left as they are).
pub fn composite_tiles(inp: &Inputs, out: &mut Composite, tiles: &[(u32, u32)]) {
    composite_tiles_cached(inp, out, tiles, None);
}

/// Tiles per side of the blocks a composite works through (bounds neighbourhood-filter memory).
pub const BLOCK: u32 = 16;

/// [`composite_tiles`], serving everything but the hot layer from `below` where it can (and
/// filling it). Results are identical to [`composite_tiles`].
pub fn composite_tiles_cached(inp: &Inputs, out: &mut Composite, tiles: &[(u32, u32)], mut below: Option<&mut BelowCache>) {
    let Ok(set) = inp.project.texture_set(inp.ts) else { return };
    if (inp.gbuf.width, inp.gbuf.height) != (out.resolution, out.height) {
        return;
    }

    let split = below.as_deref_mut().and_then(|b| b.prepare(inp));

    // Avoid caching tiles that would immediately evict each other
    let levels = split.as_ref().map_or(1, Vec::len);
    let estimate = TILE_AREA * levels * set.channel_list().iter().map(|c| c.components()).sum::<usize>() * std::mem::size_of::<f32>();
    let tile_bytes = below.as_deref().and_then(BelowCache::tile_bytes).unwrap_or(estimate).max(1);
    let fits = below.as_deref().is_some_and(|b| tiles.len().max(b.pass) <= b.limit() / tile_bytes);
    let mut blocks: BTreeMap<(u32, u32), Vec<(u32, u32)>> = BTreeMap::new();
    for t in tiles {
        blocks.entry((t.1 / BLOCK, t.0 / BLOCK)).or_default().push(*t);
    }

    let channels = set.channel_list();
    let hot = split.as_deref().and_then(<[Level]>::last);

    // A stroke preview is absent from StackKey, so it disables field retention
    let retain_fields = below.is_some() && inp.preview.is_none() && has_neighbourhood(inp, &set.stack);
    let key = if retain_fields { StackKey::new(inp.project, inp.pixels, inp.baked, inp.images, inp.ts, &set.stack).map(Arc::new) } else { None };
    let size = (inp.gbuf.width, inp.gbuf.height);

    // Whole-set fields are computed once for every block: kept by the set's cache across composites, or for this
    // composite only. The stack's key, when built, also tells the cache whether its entries are current.
    let mut whole_set = match below.as_deref_mut() {
        Some(b) => std::mem::take(&mut b.fields),
        None => FieldCache::single(),
    };
    whole_set.begin(key.clone());

    // Release fields when a stroke or stack change disables retention
    if key.is_none()
        && let Some(cache) = below.as_deref_mut()
    {
        cache.held = None;
    }

    for (block_index, block) in &blocks {
        let uncached = |tile: &(u32, u32)| below.as_deref().zip(split.as_deref()).is_none_or(|(cache, _)| cache.tile(*tile).is_none());
        let fresh: Vec<(u32, u32)> = block.iter().copied().filter(uncached).collect();
        let held = below.as_deref().and_then(|cache| cache.held.as_ref()).filter(|held| {
            key.as_ref().is_some_and(|key| **key == *held.key)
                && held.size == size
                && fresh.iter().all(|tile| held.stack_tiles.contains(tile))
                && block.iter().all(|tile| held.hot_tiles.contains(tile))
        });
        let ahead: Vec<(u32, u32)> = match (&key, below.as_deref()) {
            (Some(_), Some(cache)) if held.is_none() => {
                cache.ahead.iter().copied().filter(|tile| (tile.1 / BLOCK, tile.0 / BLOCK) == *block_index && !block.contains(tile)).collect()
            }
            _ => Vec::new(),
        };
        let fields: Arc<Fields> = match (held, &key) {
            (Some(held), _) => held.fields.clone(),
            (None, Some(key)) if !ahead.is_empty() => {
                // Filter the current and following tiles as one run
                let planned_tiles: Vec<(u32, u32)> = block.iter().chain(&ahead).copied().collect();
                let uncached_planned_tiles: Vec<(u32, u32)> = planned_tiles.iter().copied().filter(uncached).collect();
                let fields = Arc::new(block_fields(inp, &channels, &set.stack, hot, &uncached_planned_tiles, &planned_tiles, &mut whole_set));
                let hot_tiles: std::collections::BTreeSet<(u32, u32)> = planned_tiles.iter().copied().collect();
                let stack_tiles = if hot.is_some() && uncached_planned_tiles.len() < planned_tiles.len() {
                    uncached_planned_tiles.into_iter().collect()
                } else {
                    hot_tiles.clone()
                };
                if let Some(cache) = below.as_deref_mut() {
                    cache.held = Some(HeldFields { key: key.clone(), size, stack_tiles, hot_tiles, fields: fields.clone() });
                }
                fields
            }
            _ => Arc::new(block_fields(inp, &channels, &set.stack, hot, &fresh, block, &mut whole_set)),
        };

        let quota = match below.as_deref() {
            Some(_) if fits => usize::MAX,
            Some(b) => b.limit().saturating_sub(b.bytes()) / tile_bytes,
            None => 0,
        };

        let cached = below.as_deref().zip(split.as_deref());
        let records = composite_block(inp, &set.stack, out, block, cached, quota, &fields);
        if let Some(b) = below.as_deref_mut() {
            for t in block {
                b.touch(*t);
            }
            for (t, bands) in records {
                b.insert(t, bands);
            }
        }
    }

    whole_set.end(inp);
    if let Some(cache) = below {
        cache.fields = whole_set;
        // Release fields at the end of the pass
        if cache.ahead.is_empty() {
            cache.held = None;
        }
    }
}

/// Whether a layer drawn by `stack` has a neighbourhood filter in its mask or content effects.
fn has_neighbourhood(inp: &Inputs, stack: &[LayerId]) -> bool {
    let working_width = inp.gbuf.width;
    let mut order = Vec::new();
    draw_order(inp.project, stack, 0, &mut order);
    order.into_iter().any(|id| {
        let masked = layer_mask(inp.project, id).is_some_and(|mask| mask.enabled && neighbourhood(&mask.effects, working_width).is_some());
        masked || inp.project.content_layer(id).is_some_and(|layer| !layer.is_group() && neighbourhood(&layer.content_effects, working_width).is_some())
    })
}

/// The neighbourhood fields compositing `tiles` reads: those of every layer of `stack` over the tiles the cache lacks (`fresh`),
/// and off the path to the `hot` layer only there; the hot layer's over all of `tiles`. Whole-set fields of the stack come
/// from `whole_set` when it holds them, and are kept in it.
fn block_fields(
    inp: &Inputs,
    channels: &[Channel],
    stack: &[LayerId],
    hot: Option<&Level>,
    fresh: &[(u32, u32)],
    tiles: &[(u32, u32)],
    whole_set: &mut FieldCache,
) -> Fields {
    match hot {
        Some(hot) if fresh.len() < tiles.len() => {
            let mut fields = fields_of(inp, channels, stack, fresh, Some(whole_set));
            fields.extend(fields_of(inp, channels, &[hot.at], tiles, None));
            fields
        }
        _ => fields_of(inp, channels, stack, tiles, Some(whole_set)),
    }
}

/// Rows of a tile composited as one parallel task.
const BAND: u32 = 16;

/// One band of a tile: per-channel buffers, what to cache of it (when recorded) and whether
/// any texel is covered.
type BandResult = (Vec<Vec<u16>>, Option<Band>, bool);

/// Per-channel tiles of one composited tile and whether any texel is covered.
type Joined = (Vec<Option<std::sync::Arc<Vec<u16>>>>, bool);

/// Composite one block of tiles, split along the path to the hot layer when `below` holds it;
/// returns what to cache (for at most `quota` tiles).
fn composite_block(
    inp: &Inputs,
    stack: &[LayerId],
    out: &mut Composite,
    tiles: &[(u32, u32)],
    below: Option<(&BelowCache, &[Level])>,
    quota: usize,
    fields: &Fields,
) -> Vec<((u32, u32), Vec<Band>)> {
    let channels: Vec<Channel> = inp.project.texture_set(inp.ts).map(|s| s.channel_list()).unwrap_or_default();
    let (w, h) = (out.resolution, out.height);
    let split = below.map(|(_, l)| l);
    let cached = |t: (u32, u32)| below.and_then(|(b, _)| b.tile(t));
    let fresh: Vec<(u32, u32)> = tiles.iter().copied().filter(|t| cached(*t).is_none()).collect();
    let recorded: std::collections::BTreeSet<(u32, u32)> = if split.is_some() { fresh.iter().copied().take(quota).collect() } else { Default::default() };
    let lanes: Vec<Lane> = channels.iter().map(|c| Lane { ch: c.index(), n: c.components(), signed: c.kind() == ChannelKind::Signed }).collect();
    let stride: usize = lanes.iter().map(|l| l.n).sum();
    let level_count = split.map_or(0, <[Level]>::len);
    let defaults = Acc::defaults();

    // Tiles split into bands of rows, so a handful of tiles still keeps every thread busy.
    let bands = TILE / BAND;
    let tasks: Vec<((u32, u32), u32)> = tiles.iter().flat_map(|t| (0..bands).map(move |b| (*t, b))).collect();
    let mut results: Vec<BandResult> = tasks
        .par_iter()
        .map(|&((tx, ty), band)| {
            let warm = cached((tx, ty)).and_then(|b| b.get(band as usize));
            let record = warm.is_none() && recorded.contains(&(tx, ty));
            let levels = split.map(|l| plan_levels(inp, l, &channels, tx, ty, fields, warm.is_some()));
            let nodes = if levels.is_none() { plan(inp, stack, &channels, tx, ty, 0, Some(fields)) } else { Vec::new() };
            let rows = (BAND * TILE) as usize;
            let mut level_values = vec![defaults; level_count];
            let mut masks = vec![0.0f32; level_count];
            let mut tape = match warm {
                Some(b) => Tape::Replay(b, 0),
                None if record => Tape::Record(Vec::new()),
                None => Tape::Off,
            };

            // Empty lower stacks reuse the enclosing level instead of storing a duplicate
            let stored: Vec<bool> = match (&levels, warm) {
                (Some(_), Some(_)) => tape.take(level_count).iter().map(|v| *v > 0.0).collect(),
                (Some(levels), None) => levels.iter().map(|l| !l.below.is_empty()).collect(),
                (None, _) => Vec::new(),
            };
            if let Tape::Record(t) = &mut tape {
                t.extend(stored.iter().map(|s| if *s { 1.0 } else { 0.0 }));
            }
            let slots = stored.iter().filter(|s| **s).count() * stride;
            let preview_tile = inp.preview.and_then(|p| p.alpha.tile(tx, ty));
            let mut channel_buffers: Vec<Vec<u16>> = channels.iter().map(|c| vec![0u16; rows * c.components()]).collect();
            let mut anchors = Vec::new();
            let mut any = false;
            for ly in band * BAND..(band + 1) * BAND {
                let y = ty * TILE + ly;
                if y >= h {
                    break;
                }
                for lx in 0..TILE {
                    let x = tx * TILE + lx;
                    if x >= w {
                        break;
                    }
                    let gbuffer_index = (y * w + x) as usize;
                    let tile_offset = (ly * TILE + lx) as usize;
                    let band_offset = ((ly - band * BAND) * TILE + lx) as usize;
                    let covered = inp.gbuf.tri.get(gbuffer_index).is_some_and(|t| *t != NO_TRI);
                    let mut acc = defaults;
                    if covered {
                        any = true;
                        let mut texel = Texel::new(inp, x, y, tile_offset, stroke_at(preview_tile, tile_offset));

                        // Reuse one anchor buffer per band.
                        texel.anchors = std::mem::take(&mut anchors);
                        match &levels {
                            Some(levels) => {
                                // The texel's stream starts with the stack at each level.
                                let start = match &mut tape {
                                    Tape::Record(t) => {
                                        t.resize(t.len() + slots, 0.0);
                                        t.len() - slots
                                    }
                                    _ => 0,
                                };
                                if warm.is_some() {
                                    load_levels(tape.take(slots), &lanes, stride, &stored, &defaults, &mut level_values);
                                }
                                acc = texel.eval_split(defaults, levels, &mut level_values, &mut masks, warm.is_some(), &mut tape);
                                if let Tape::Record(t) = &mut tape
                                    && let Some(below) = t.get_mut(start..start + slots)
                                {
                                    store_levels(below, &lanes, stride, &stored, &level_values);
                                }
                            }
                            None => {
                                texel.eval(&nodes, &mut acc);
                            }
                        }
                        anchors = texel.anchors;
                        anchors.clear();
                    }
                    for (l, buf) in lanes.iter().zip(&mut channel_buffers) {
                        let v = acc.get(l.ch).copied().unwrap_or([0.0; 3]);
                        for (k, x) in v.iter().take(l.n).enumerate() {
                            let encoded = if l.signed { to_u16_signed(*x) } else { to_u16(*x) };
                            if let Some(slot) = buf.get_mut(band_offset * l.n + k) {
                                *slot = encoded;
                            }
                        }
                    }
                }
            }
            let cache = if let Tape::Record(t) = tape { Some(t) } else { None };
            (channel_buffers, cache, any)
        })
        .collect();

    // Join the bands of each tile (in parallel; uncovered tiles are freed).
    let joined: Vec<Joined> = results
        .par_chunks(bands as usize)
        .map(|parts| {
            let any = parts.iter().any(|p| p.2);
            let data = (0..channels.len())
                .map(|k| {
                    any.then(|| {
                        let mut v = Vec::with_capacity(parts.iter().map(|p| p.0.get(k).map_or(0, Vec::len)).sum());
                        for p in parts {
                            v.extend_from_slice(p.0.get(k).map_or(&[][..], Vec::as_slice));
                        }
                        std::sync::Arc::new(v)
                    })
                })
                .collect();
            (data, any)
        })
        .collect();
    let mut records = Vec::new();
    for ((&(tx, ty), (data, any)), parts) in tiles.iter().zip(joined).zip(results.chunks_mut(bands as usize)) {
        for (c, d) in channels.iter().zip(data) {
            if let Some(img) = out.channels.get_mut(c) {
                img.set_tile(tx, ty, d);
            }
        }

        // Tiles without covered texels cost nothing to composite: not worth caching.
        if any && let Some(bands) = parts.iter_mut().map(|p| p.1.take()).collect::<Option<Vec<Band>>>() {
            records.push(((tx, ty), bands));
        }
    }
    records
}

/// Restore the stack at each nesting level of a texel: the `stored` levels from `src`, the
/// others repeat the level above them (the defaults at the top).
fn load_levels(src: &[f32], lanes: &[Lane], stride: usize, stored: &[bool], defaults: &Acc, accs: &mut [Acc]) {
    let (mut k, mut above) = (0, *defaults);
    for (a, s) in accs.iter_mut().zip(stored) {
        if *s {
            load_below(src, lanes, stride, k, a);
            k += 1;
        } else {
            *a = above;
        }
        above = *a;
    }
}

/// Store the `stored` levels of a texel's stack (see [`load_levels`]).
fn store_levels(dst: &mut [f32], lanes: &[Lane], stride: usize, stored: &[bool], accs: &[Acc]) {
    for (k, a) in accs.iter().zip(stored).filter(|(_, s)| **s).map(|(a, _)| a).enumerate() {
        store_below(dst, lanes, stride, k, a);
    }
}

/// Where a texture set channel lives in a stack value, its components and whether it is signed.
#[derive(Clone, Copy)]
struct Lane {
    ch: usize,
    n: usize,
    signed: bool,
}

/// Restore the cached stack value of slot `o` (a texel at one nesting level). Scalars are
/// cached once: only their first component is ever read.
fn load_below(cache: &[f32], lanes: &[Lane], stride: usize, o: usize, acc: &mut Acc) {
    let mut i = o * stride;
    for l in lanes {
        if let (Some(src), Some(slot)) = (cache.get(i..i + l.n), acc.get_mut(l.ch)) {
            *slot = match src {
                [a, b, c] => [*a, *b, *c],
                _ => [src.first().copied().unwrap_or(0.0); 3],
            };
        }
        i += l.n;
    }
}

fn store_below(cache: &mut [f32], lanes: &[Lane], stride: usize, o: usize, acc: &Acc) {
    let mut i = o * stride;
    for l in lanes {
        if let (Some(dst), Some(v)) = (cache.get_mut(i..i + l.n), acc.get(l.ch)) {
            for (d, x) in dst.iter_mut().zip(v) {
                *d = *x;
            }
        }
        i += l.n;
    }
}

/// UV padding for display: fill uncovered texels in `tiles` and their neighbours from their
/// nearest covered texel. Returns the tiles it wrote.
pub fn pad_tiles(out: &mut Composite, dmap: &DilationMap, tiles: &[(u32, u32)]) -> std::collections::BTreeSet<(u32, u32)> {
    let mut written = std::collections::BTreeSet::new();
    let (w, h) = (out.resolution, out.height);
    if (dmap.width, dmap.height) != (w, h) {
        return written;
    }

    let (nx, ny) = (w.div_ceil(TILE) as i64, h.div_ceil(TILE) as i64);
    let mut set = std::collections::BTreeSet::new();
    for &(tx, ty) in tiles {
        for dy in -1..=1i64 {
            for dx in -1..=1i64 {
                let (x, y) = (tx as i64 + dx, ty as i64 + dy);
                if x >= 0 && y >= 0 && x < nx && y < ny {
                    set.insert((x as u32, y as u32));
                }
            }
        }
    }
    let set: Vec<(u32, u32)> = set.into_iter().collect();
    for img in out.channels.values_mut() {
        let comps = img.comps();
        let src: &TileImage = img;

        // Each tile pads from the original image, so parallel writes stay independent
        let padded: Vec<((u32, u32), Vec<u16>)> = set
            .par_iter()
            .filter_map(|&(tx, ty)| {
                let mut data: Option<Vec<u16>> = None;
                for ly in 0..TILE {
                    for lx in 0..TILE {
                        let (x, y) = (tx * TILE + lx, ty * TILE + ly);
                        if x >= w || y >= h {
                            continue;
                        }

                        let Some((sx, sy)) = dmap.source(x, y) else { continue };
                        if (sx, sy) == (x, y) {
                            continue;
                        }

                        let v = src.get(sx, sy);
                        let d = data.get_or_insert_with(|| src.tile(tx, ty).map_or_else(|| vec![0; TILE_AREA * comps], <[u16]>::to_vec));
                        let o = (ly * TILE + lx) as usize * comps;
                        if let Some(px) = d.get_mut(o..o + comps) {
                            px.copy_from_slice(&v[..comps]);
                        }
                    }
                }
                Some(((tx, ty), data?))
            })
            .collect();
        for ((tx, ty), d) in padded {
            img.set_tile(tx, ty, Some(std::sync::Arc::new(d)));
            written.insert((tx, ty));
        }
    }
    written
}

/// The layer mask over the whole texture set as `0..=1` values, row by row from the top (the
/// orientation of the exported maps), `gbuf.width × gbuf.height` of them; black outside the mesh
/// Returns None without an enabled mask
pub fn mask_texels(inputs: &Inputs, id: LayerId) -> Option<Vec<f32>> {
    let mask = layer_mask(inputs.project, id).filter(|mask| mask.enabled)?;
    let channels = inputs.project.texture_set(inputs.ts).map(|set| set.channel_list()).unwrap_or_default();
    let (width, height) = (inputs.gbuf.width, inputs.gbuf.height);
    let coordinates = all_tiles(width, height);

    // A hidden layer's mask is left out of the stack's preparation
    prepare_mask_curvature_sizes(inputs, std::iter::once(mask));

    let fields = neighbourhood_fields(inputs, &channels, &coordinates);
    let width_usize = width as usize;
    let tile_side = TILE as usize;
    let mut output = vec![0.0f32; width_usize * height as usize];

    // Write each tile into disjoint atlas rows without temporary tile buffers
    output.par_chunks_mut(width_usize.max(1) * tile_side).enumerate().for_each(|(tile_y, rows)| {
        for tile_x in 0..width.div_ceil(TILE) {
            for (offset, value) in mask_tile(inputs, id, mask, &channels, &fields, (tile_x, tile_y as u32), 0..TILE_AREA) {
                let x = tile_x as usize * tile_side + offset % tile_side;
                if x < width_usize
                    && let Some(destination) = rows.get_mut((offset / tile_side) * width_usize + x)
                {
                    *destination = value;
                }
            }
        }
    });
    Some(output)
}

/// The layer mask as a square grayscale image, using nearest texels and black outside the mesh
/// Keep preview tiles at 8 bits instead of retaining the export's full float atlas
/// Returns None without an enabled mask
pub fn mask_image(inputs: &Inputs, id: LayerId, size: u32) -> Option<Vec<u8>> {
    let mask = layer_mask(inputs.project, id).filter(|mask| mask.enabled)?;
    let channels = inputs.project.texture_set(inputs.ts).map(|set| set.channel_list()).unwrap_or_default();
    let (width, height) = (inputs.gbuf.width, inputs.gbuf.height);
    let coordinates = all_tiles(width, height);

    // A hidden layer's mask is left out of the stack's preparation
    prepare_mask_curvature_sizes(inputs, std::iter::once(mask));

    let fields = neighbourhood_fields(inputs, &channels, &coordinates);
    let width_usize = width as usize;
    let tile_side = TILE as usize;
    let mut atlas = vec![0u8; width_usize * height as usize];

    // Write each tile row into disjoint atlas rows
    atlas.par_chunks_mut(width_usize.max(1) * tile_side).enumerate().for_each(|(tile_y, rows)| {
        for tile_x in 0..width.div_ceil(TILE) {
            for (offset, value) in mask_tile(inputs, id, mask, &channels, &fields, (tile_x, tile_y as u32), 0..TILE_AREA) {
                let x = tile_x as usize * tile_side + offset % tile_side;
                if x < width_usize
                    && let Some(destination) = rows.get_mut((offset / tile_side) * width_usize + x)
                {
                    *destination = (value * 255.0 + 0.5) as u8;
                }
            }
        }
    });

    let size = size.max(1);
    if width == size && height == size {
        return Some(atlas);
    }

    let mut output = vec![0u8; (size * size) as usize];
    for y in 0..size {
        for x in 0..size {
            let sample_x = (u64::from(x) * u64::from(width) / u64::from(size)) as u32;
            let sample_y = (u64::from(y) * u64::from(height) / u64::from(size)) as u32;
            if let Some(value) = atlas.get(sample_y as usize * width_usize + sample_x as usize)
                && let Some(destination) = output.get_mut((y * size + x) as usize)
            {
                *destination = *value;
            }
        }
    }
    Some(output)
}

/// The layer mask as one-component values at the texture set's full resolution over `tiles`
/// (uncovered texels read zero). Clone and smudge read it. Unlike [`mask_image`] it also reads a
/// disabled mask; None when the layer has no mask.
pub fn mask_values(inputs: &Inputs, id: LayerId, tiles: &[(u32, u32)]) -> Option<TileImage> {
    let mut image = TileImage::new(inputs.gbuf.width, inputs.gbuf.height, 1);
    mask_values_into(inputs, id, tiles, &mut image)?;
    Some(image)
}

/// Evaluate requested tiles directly into the stroke's mask image.
pub fn mask_values_into(inputs: &Inputs, id: LayerId, tiles: &[(u32, u32)], image: &mut TileImage) -> Option<()> {
    let mask = layer_mask(inputs.project, id)?;
    let channels = inputs.project.texture_set(inputs.ts).ok()?.channel_list();
    prepare_mask_curvature_sizes(inputs, std::iter::once(mask));

    let fields = neighbourhood_fields(inputs, &channels, tiles);
    let computed: Vec<((u32, u32), Vec<u16>)> = tiles
        .par_iter()
        .map(|&tile| {
            let mut values = vec![0u16; TILE_AREA];
            for (offset, value) in mask_tile(inputs, id, mask, &channels, &fields, tile, 0..TILE_AREA) {
                if let Some(destination) = values.get_mut(offset) {
                    *destination = to_u16(value.clamp(0.0, 1.0));
                }
            }
            (tile, values)
        })
        .collect();

    for ((tx, ty), values) in computed {
        image.set_tile(tx, ty, Some(std::sync::Arc::new(values)));
    }
    Some(())
}

/// Mask values at texture-set coordinates, in request order; None without an enabled mask
/// Uncovered and out-of-range texels read zero; neighbourhood filters also evaluate surrounding texels
pub fn mask_at(inputs: &Inputs, id: LayerId, texels: &[(u32, u32)]) -> Option<Vec<f32>> {
    let mask = layer_mask(inputs.project, id).filter(|mask| mask.enabled)?;
    let mut output = vec![0.0; texels.len()];

    // Group valid requests before planning effects, which may resample painted tiles
    let mut requested: HashMap<(u32, u32), Vec<(usize, usize)>> = HashMap::new();
    for (index, &(x, y)) in texels.iter().enumerate() {
        if x >= inputs.gbuf.width || y >= inputs.gbuf.height {
            continue;
        }

        let offset = ((y % TILE) * TILE + x % TILE) as usize;
        requested.entry((x / TILE, y / TILE)).or_default().push((offset, index));
    }

    if requested.is_empty() {
        return Some(output);
    }

    let channels = inputs.project.texture_set(inputs.ts).map(|set| set.channel_list()).unwrap_or_default();

    // A hidden layer's mask is left out of the stack's preparation
    prepare_mask_curvature_sizes(inputs, std::iter::once(mask));

    // Filters and anchors can depend on texels outside the requested tiles
    let fields = neighbourhood_fields(inputs, &channels, &all_tiles(inputs.gbuf.width, inputs.gbuf.height));
    let values: Vec<(usize, f32)> = requested
        .par_iter_mut()
        .flat_map_iter(|(&tile, requests)| {
            requests.sort_unstable_by_key(|&(offset, _)| offset);
            let groups = requests.chunk_by(|left, right| left.0 == right.0);
            let offsets = groups.clone().filter_map(|group| group.first().map(|&(offset, _)| offset));
            mask_tile(inputs, id, mask, &channels, &fields, tile, offsets)
                .zip(groups)
                .flat_map(|((_, value), group)| group.iter().map(move |&(_, index)| (index, value)))
        })
        .collect();

    for (index, value) in values {
        if let Some(destination) = output.get_mut(index) {
            *destination = value;
        }
    }
    Some(output)
}

/// Mask values in offset order, including zeros for uncovered texels so request groups stay aligned
fn mask_tile<'a>(
    inputs: &'a Inputs<'a>,
    id: LayerId,
    mask: &'a Mask,
    channels: &[Channel],
    fields: &'a Fields,
    (tile_x, tile_y): (u32, u32),
    offsets: impl Iterator<Item = usize> + 'a,
) -> impl Iterator<Item = (usize, f32)> + 'a {
    let plan = mask_plan(inputs, id, mask, tile_x, tile_y, Some(fields));
    let anchor_stack = anchor_plan(inputs, id, channels, tile_x, tile_y, Some(fields));
    offsets.map(move |offset| {
        let Some(plan) = plan.as_ref() else { return (offset, 0.0) };

        let x = tile_x * TILE + (offset % TILE as usize) as u32;
        let y = tile_y * TILE + (offset / TILE as usize) as u32;
        if x >= inputs.gbuf.width || y >= inputs.gbuf.height || inputs.gbuf.tri.get((y * inputs.gbuf.width + x) as usize).is_none_or(|tri| *tri == NO_TRI) {
            return (offset, 0.0);
        }

        let mut texel = Texel::new(inputs, x, y, offset, 0.0);
        if !anchor_stack.is_empty() {
            texel.anchors_below(&anchor_stack, id);
        }
        (offset, texel.mask(plan))
    })
}

/// Tiles [`effect_histogram`] samples at most (a million texels).
const HISTOGRAM_TILES: usize = 256;

/// The tiles [`effect_histogram`] reads: an even sample of at most [`HISTOGRAM_TILES`], which
/// keeps the cost flat up to 8K. The stride is odd: a power of two would divide the row length
/// and sample the same few columns.
fn histogram_tiles(width: u32, height: u32) -> Vec<(u32, u32)> {
    let all = all_tiles(width, height);
    let stride = all.len().div_ceil(HISTOGRAM_TILES).max(1) | 1;
    all.into_iter().step_by(stride).collect()
}

/// Histogram (`bins` buckets over 0..1) of the values entering effect `effect`: its layer's mask
/// below it, or (a content effect) the layer's `channel` below it where the layer has content,
/// color components pooled and signed values mapped to 0..1. Over the texels the mesh covers
/// (in an even sample of tiles on large sets); None for an unknown effect.
pub fn effect_histogram(inp: &Inputs, effect: LayerId, channel: Channel, bins: usize) -> Option<Vec<u32>> {
    let (owner, _) = inp.project.effect(effect)?;
    let bins = bins.max(1);
    // Evaluate the layer forced visible, its mask enabled, cut just below the effect.
    let mut cut = inp.project.clone();
    let l = cut.layers.get_mut(&owner)?;
    l.visible = true;
    let in_mask = match l.mask.as_mut().and_then(|m| m.effects.iter().position(|e| e.id == effect).map(|k| (m, k))) {
        Some((m, k)) => {
            m.enabled = true;
            m.effects.truncate(k);
            true
        }
        None => {
            let k = l.content_effects.iter().position(|e| e.id == effect)?;
            l.content_effects.truncate(k);
            false
        }
    };
    let cut_inp = Inputs { project: &cut, preview: None, block_scales: inp.block_scales.clone(), ..*inp };
    let channels = cut.texture_set(inp.ts).map(|t| t.channel_list()).unwrap_or_default();
    let tiles = histogram_tiles(inp.gbuf.width, inp.gbuf.height);
    // A field covers the bounding box of the tiles it is built for, which for a scattered sample
    // is the whole set: with a neighbourhood filter below, each tile gets its own.
    let probe = neighbourhood_fields(&cut_inp, &channels, tiles.get(..1).unwrap_or_default());
    let shared = probe.is_empty().then_some(probe);
    let bin = |v: f32| ((v.clamp(0.0, 1.0) * bins as f32) as usize).min(bins - 1);
    let hist = tiles
        .par_iter()
        .map(|&(tx, ty)| {
            let own;
            let fields = match &shared {
                Some(f) => f,
                None => {
                    own = neighbourhood_fields(&cut_inp, &channels, &[(tx, ty)]);
                    &own
                }
            };
            let mut h = vec![0u32; bins];
            // The mask is planned on its own: a layer with nothing drawn plans no node.
            let mask = if in_mask { layer_mask(&cut, owner).and_then(|m| mask_plan(&cut_inp, owner, m, tx, ty, Some(fields))) } else { None };
            let nodes = if in_mask { Vec::new() } else { plan(&cut_inp, &[owner], &channels, tx, ty, 0, Some(fields)) };
            let content = nodes.first().and_then(|n| n.chans.iter().find(|c| Channel::ALL.get(c.ch) == Some(&channel)).map(|c| (n, c)));
            if mask.is_none() && content.is_none() {
                return h;
            }
            let full = anchor_plan(&cut_inp, owner, &channels, tx, ty, Some(fields));
            for (x, y, o) in tile_texels(inp.gbuf, tx, ty) {
                let mut tex = Texel::new(&cut_inp, x, y, o, 0.0);
                if !full.is_empty() {
                    tex.anchors_below(&full, owner);
                }
                if let Some(m) = &mask {
                    if let Some(b) = h.get_mut(bin(tex.mask(m))) {
                        *b += 1;
                    }
                } else if let Some((n, c)) = content
                    && let Some((v, _)) = tex.content(&n.content, c)
                {
                    // As levels reads them (`per_unit`); normal maps pass through levels unchanged.
                    let (vals, signed) = match c.kind {
                        ChannelKind::Color | ChannelKind::Data => (&v[..], false),
                        ChannelKind::Scalar => (&v[..1], false),
                        ChannelKind::Signed => (&v[..1], true),
                        ChannelKind::NormalMap => (&v[..0], false),
                    };
                    for x in vals {
                        if let Some(b) = h.get_mut(bin(if signed { x * 0.5 + 0.5 } else { *x })) {
                            *b += 1;
                        }
                    }
                }
            }
            h
        })
        .reduce(|| vec![0u32; bins], |a, b| a.iter().zip(&b).map(|(x, y)| x.saturating_add(*y)).collect());
    Some(hist)
}

/// One flattened tile of one channel: (channel, tile, premultiplied value + coverage data).
pub type FlatTile = (Channel, (u32, u32), Vec<u16>);

/// Flatten one paint or fill layer: its content (with content effects) times its mask, as
/// premultiplied (value, coverage) tiles per enabled channel, ready to become a plain paint
/// layer. Groups are not supported (None).
pub fn flatten_layer(inputs: &Inputs, id: LayerId, tiles: &[(u32, u32)]) -> Option<Vec<FlatTile>> {
    let set = inputs.project.texture_set(inputs.ts).ok()?;
    if inputs.project.content_layer(id)?.is_group() {
        return None;
    }

    let channels: Vec<Channel> = set.channel_list();

    // Plan the layer forced visible so hidden layers flatten too
    let mut solo = inputs.project.clone();
    if let Some(l) = solo.layers.get_mut(&id) {
        l.visible = true;
    }
    let visible_inputs = Inputs { project: &solo, preview: None, block_scales: inputs.block_scales.clone(), ..*inputs };
    let fields = neighbourhood_fields(&visible_inputs, &channels, tiles);

    let out: Vec<Vec<FlatTile>> = tiles
        .par_iter()
        .map(|&(tx, ty)| {
            let nodes = plan(&visible_inputs, &[id], &channels, tx, ty, 0, Some(&fields));
            let Some(n) = nodes.first() else { return Vec::new() };
            let full = anchor_plan(&visible_inputs, id, &channels, tx, ty, Some(&fields));
            let mut buffers: Vec<(Channel, Vec<u16>, bool)> =
                n.chans.iter().filter_map(|c| Channel::ALL.get(c.ch).map(|ch| (*ch, vec![0u16; TILE_AREA * (c.comps + 1)], false))).collect();
            for (ch, buffer, _) in &mut buffers {
                if ch.kind() == ChannelKind::Signed {
                    for pixel in buffer.as_chunks_mut::<2>().0 {
                        pixel[0] = to_u16_signed(0.0);
                    }
                }
            }

            for (x, y, o) in tile_texels(inputs.gbuf, tx, ty) {
                let mut tex = Texel::new(&visible_inputs, x, y, o, 0.0);
                if !full.is_empty() {
                    tex.anchors_below(&full, id);
                }
                let m = tex.node_mask(n);
                for (c, (ch, buffer, any)) in n.chans.iter().zip(buffers.iter_mut()) {
                    let Some((v, cov)) = tex.content(&n.content, c) else { continue };
                    let a = (cov * m).clamp(0.0, 1.0);
                    if a <= 0.0 {
                        continue;
                    }

                    *any = true;
                    let comps = c.comps;
                    let Some(pixel) = buffer.get_mut(o * (comps + 1)..(o + 1) * (comps + 1)) else { continue };
                    for (slot, pv) in pixel.iter_mut().zip(v.iter().take(comps)) {
                        *slot = if ch.kind() == ChannelKind::Signed { to_u16_signed(pv * a) } else { to_u16(pv * a) };
                    }
                    if let Some(slot) = pixel.get_mut(comps) {
                        *slot = to_u16(a);
                    }
                }
            }

            buffers.into_iter().filter(|(_, _, any)| *any).map(|(ch, buffer, _)| (ch, (tx, ty), buffer)).collect()
        })
        .collect();

    Some(out.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_core::{
        ChannelBlend, CompositeRules, Effect, HeightNormal, Layer, Levels, OutputResolution, Procedural, ProceduralKind, ResolutionMode, TextureSet,
    };
    use texturecraft_geom::primitives;

    struct Fixture {
        project: Project,
        mesh: Mesh,
        gbuf: UvGBuffer,
        pixels: PixelStore,
        baked: BakedMaps,
    }

    fn fixture() -> Fixture {
        let mesh = primitives::plane(2.0, 2);
        let gbuf = UvGBuffer::new(&mesh, 0, 64, 64);
        let mut project = Project::new("t", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, 64));
        Fixture { project, mesh, gbuf, pixels: PixelStore::default(), baked: BakedMaps::default() }
    }

    fn run(f: &Fixture, preview: Option<&StrokePreview>) -> Composite {
        let images = Default::default();
        let inp = Inputs {
            project: &f.project,
            ts: 0,
            mesh: &f.mesh,
            gbuf: &f.gbuf,
            pixels: &f.pixels,
            baked: &f.baked,
            block_scales: f.project.texture_sets[0].block_scales(),
            preview,
            images: &images,
            graphs: crate::GraphStore::empty(),
            bounds: f.mesh.bounds(),
        };
        let mut out = Composite::new(&f.project.texture_sets[0].channel_list(), 64);
        composite_tiles(&inp, &mut out, &all_tiles(64, 64));
        out
    }

    #[test]
    fn procedural_resolution_uses_each_uv_tiles_scale() {
        let mut fixture = fixture();
        let set = &mut fixture.project.texture_sets[0];
        set.uv_tiles = vec![1001, 1002];
        set.resolution = 64;
        set.tile_resolutions.insert(1002, 32);
        fixture.gbuf = UvGBuffer::new(&fixture.mesh, 0, 128, 64);
        let block_scales = set.block_scales();
        let images = ImageStore::default();
        let inputs = Inputs {
            project: &fixture.project,
            ts: 0,
            mesh: &fixture.mesh,
            gbuf: &fixture.gbuf,
            pixels: &fixture.pixels,
            baked: &fixture.baked,
            block_scales,
            preview: None,
            images: &images,
            graphs: GraphStore::empty(),
            bounds: fixture.mesh.bounds(),
        };

        assert_eq!(inputs.tile_res_at(0, 0), 64);
        assert_eq!(inputs.tile_res_at(64, 0), 32);

        let mut pattern = Procedural::new(ProceduralKind::Checker);
        pattern.resolution.factor = -1;

        assert_eq!(pattern_pixels(&pattern, inputs.tile_res_at(0, 0), 64), Some(32));
        assert_eq!(pattern_pixels(&pattern, inputs.tile_res_at(64, 0), 64), Some(16));
    }

    fn fill_layer(p: &mut Project, color: [f32; 3], rough: f32) -> LayerId {
        let id = p.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::BaseColor, Source::Uniform(color));
        fill.sources.insert(Channel::Roughness, Source::Uniform([rough; 3]));
        let l = Layer::new(id, "Fill", LayerKind::Fill(fill), &[Channel::BaseColor, Channel::Roughness]);
        p.insert_layer(0, l, None, false).unwrap()
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 2e-4)
    }

    fn run_with(f: &Fixture, graphs: &GraphStore) -> Composite {
        let images = Default::default();
        let inp = Inputs {
            project: &f.project,
            ts: 0,
            mesh: &f.mesh,
            gbuf: &f.gbuf,
            pixels: &f.pixels,
            baked: &f.baked,
            block_scales: f.project.texture_sets[0].block_scales(),
            preview: None,
            images: &images,
            graphs,
            bounds: f.mesh.bounds(),
        };
        let mut out = Composite::new(&f.project.texture_sets[0].channel_list(), 64);
        composite_tiles(&inp, &mut out, &all_tiles(64, 64));
        out
    }

    #[test]
    fn graph_sources_generators_and_filters_composite() {
        use crate::graphs::tests::{COLOUR, INVERT};
        let mut graphs = GraphStore::default();
        let (colour, invert) = (graphs.add_text(COLOUR), graphs.add_text(INVERT));
        let reading = |output: &str| {
            let mut g = GraphRef::new(colour.clone(), "colour");
            g.output = Some(output.into());
            g.keep = ["colour".to_string(), "grey".to_string()].into();
            Source::Graph(Box::new(g))
        };
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [0.0; 3], 0.0);
        if let Some(LayerKind::Fill(fill)) = f.project.layers.get_mut(&id).map(|l| &mut l.kind) {
            fill.sources.insert(Channel::BaseColor, reading("colour"));
            fill.sources.insert(Channel::Roughness, reading("grey"));
        }
        let out = run_with(&f, &graphs);
        let lin = |v: [f32; 3]| v.map(texturecraft_texel::srgb_to_linear);
        assert!(close(out.value(Channel::BaseColor, 10, 10), lin([0.25, 0.5, 0.75])), "{:?}", out.value(Channel::BaseColor, 10, 10));
        assert!(close(out.value(Channel::Roughness, 10, 10), [0.5; 3]));
        assert_eq!(graphs.renders(), 1, "the channels of one fill share a render");

        // a filter reading the stack below it: the base color inverted in display values
        let mut filter = GraphRef::new(invert.clone(), "invert");
        filter.inputs.insert(0, GraphInput::Stack { channel: None });
        let fx = f.project.alloc_id();
        if let Some(l) = f.project.layers.get_mut(&id) {
            l.content_effects.push(Effect::new(fx, EffectKind::Graph { graph: filter, projection: Projection::default() }));
        }
        let out = run_with(&f, &graphs);
        assert!(close(out.value(Channel::BaseColor, 10, 10), lin([0.75, 0.5, 0.25])), "{:?}", out.value(Channel::BaseColor, 10, 10));
        let renders = graphs.renders();
        run_with(&f, &graphs);
        assert_eq!(graphs.renders(), renders, "the same stack below finds the filter's cook");

        // a mask generator from the grey output: half the fill over the defaults
        let mask_effect = f.project.alloc_id();
        if let Some(l) = f.project.layers.get_mut(&id) {
            l.content_effects.clear();
            let mut g = GraphRef::new(colour.clone(), "colour");
            g.output = Some("grey".into());
            let e = Effect::new(mask_effect, EffectKind::Graph { graph: g, projection: Projection::default() });
            l.mask = Some(Mask { effects: vec![e], ..Mask::new(MaskBackground::Black) });
        }
        let out = run_with(&f, &graphs);
        let r = out.value(Channel::Roughness, 10, 10)[0];
        assert!((r - (0.5 * 0.5 + 0.5 * Channel::Roughness.default_value()[0])).abs() < 2e-3, "{r}");

        // a graph whose text is missing shows nothing and composites without panicking
        let mut missing = GraphRef::new(texturecraft_core::text_key("absent"), "absent");
        missing.inputs.insert(0, GraphInput::Stack { channel: Some(Channel::Height) });
        if let Some(l) = f.project.layers.get_mut(&id) {
            l.content_effects.push(Effect::new(LayerId(900), EffectKind::Graph { graph: missing, projection: Projection::default() }));
        }
        run_with(&f, &graphs);
        assert!(graphs.problems().iter().any(|(u, p)| u.id == LayerId(900) && !p.stale));
    }

    /// A paint layer with nothing painted, blending base colour with `mode`, holding one content effect.
    fn filter_layer(f: &mut Fixture, mode: BlendMode, kind: EffectKind) -> LayerId {
        let id = f.project.alloc_id();
        let mut l = Layer::new(id, "Filter", LayerKind::Paint, &[Channel::BaseColor]);
        l.channels.insert(Channel::BaseColor, ChannelBlend { mode, opacity: 1.0 });
        let id = f.project.insert_layer(0, l, None, false).unwrap();
        content_fx(f, id, kind);
        id
    }

    #[test]
    fn effect_layers_filter_the_stack_below_them() {
        use crate::graphs::tests::INVERT;
        let lin = |v: [f32; 3]| v.map(texturecraft_texel::srgb_to_linear);
        let mut graphs = GraphStore::default();
        let invert = graphs.add_text(INVERT);
        let mut filter = GraphRef::new(invert, "invert");
        filter.inputs.insert(0, GraphInput::Stack { channel: None });
        let graph = EffectKind::Graph { graph: filter, projection: Projection::default() };

        // a normal paint layer filters its own (empty) content: nothing shows
        let mut f = fixture();
        fill_layer(&mut f.project, lin([0.25, 0.5, 0.75]), 0.5);
        let id = filter_layer(&mut f, BlendMode::Normal, graph.clone());
        let out = run_with(&f, &graphs);
        assert!(close(out.value(Channel::BaseColor, 10, 10), lin([0.25, 0.5, 0.75])), "{:?}", out.value(Channel::BaseColor, 10, 10));

        // passthrough: the graph filters the stack below
        f.project.layer_mut(id).unwrap().channels.insert(Channel::BaseColor, ChannelBlend { mode: BlendMode::Passthrough, opacity: 1.0 });
        let out = run_with(&f, &graphs);
        assert!(close(out.value(Channel::BaseColor, 10, 10), lin([0.75, 0.5, 0.25])), "{:?}", out.value(Channel::BaseColor, 10, 10));
        assert!(close(out.value(Channel::Roughness, 10, 10), [0.5; 3]), "channels the layer does not blend stay");

        // its strength fades between the stack and the filtered stack; a mask limits it
        f.project.layer_mut(id).unwrap().channels.insert(Channel::BaseColor, ChannelBlend { mode: BlendMode::Passthrough, opacity: 0.5 });
        let out = run_with(&f, &graphs);
        assert!(close(out.value(Channel::BaseColor, 10, 10), mix3(lin([0.25, 0.5, 0.75]), lin([0.75, 0.5, 0.25]), 0.5)));
        f.project.layer_mut(id).unwrap().channels.insert(Channel::BaseColor, ChannelBlend { mode: BlendMode::Passthrough, opacity: 1.0 });
        let me = f.project.alloc_id();
        f.project.layer_mut(id).unwrap().mask = Some(Mask { effects: vec![Effect::new(me, EffectKind::Paint)], ..Mask::new(MaskBackground::Black) });
        let img = f.pixels.masks.entry(me).or_insert_with(|| TileImage::new(64, 64, 2));
        for y in 0..64 {
            for x in 0..32 {
                img.set(x, y, &[to_u16(1.0), to_u16(1.0)]);
            }
        }
        let out = run_with(&f, &graphs);
        assert!(close(out.value(Channel::BaseColor, 10, 10), lin([0.75, 0.5, 0.25])), "{:?}", out.value(Channel::BaseColor, 10, 10));
        assert!(close(out.value(Channel::BaseColor, 50, 10), lin([0.25, 0.5, 0.75])), "{:?}", out.value(Channel::BaseColor, 50, 10));

        // a per-texel filter on an effect layer works on the stack below too
        let mut f = fixture();
        fill_layer(&mut f.project, [1.0, 0.0, 0.0], 0.2);
        filter_layer(&mut f, BlendMode::Passthrough, EffectKind::Filter(Filter::Invert));
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 5, 5), [0.0, 1.0, 1.0]), "{:?}", out.value(Channel::BaseColor, 5, 5));
        assert!(close(out.value(Channel::Roughness, 5, 5), [0.2; 3]));
    }

    fn run_cached(f: &Fixture, graphs: &GraphStore, below: &mut BelowCache) -> Composite {
        let images = Default::default();
        let inp = Inputs {
            project: &f.project,
            ts: 0,
            mesh: &f.mesh,
            gbuf: &f.gbuf,
            pixels: &f.pixels,
            baked: &f.baked,
            block_scales: f.project.texture_sets[0].block_scales(),
            preview: None,
            images: &images,
            graphs,
            bounds: f.mesh.bounds(),
        };
        let mut out = Composite::new(&f.project.texture_sets[0].channel_list(), 64);
        composite_tiles_cached(&inp, &mut out, &all_tiles(64, 64), Some(below));
        out
    }

    #[test]
    fn whole_set_fields_are_kept_until_an_edit_below_them() {
        let mut graphs = GraphStore::default();
        let blur = graphs.add_text("format 3\nsize 64\nnode 1 input idx=0\nnode 2 box_blur in=@1 intensity=8\noutput out 2\n");
        let mut filter = GraphRef::new(blur, "blur");
        filter.inputs.insert(0, GraphInput::Stack { channel: None });
        let mut f = fixture();
        let below = fill_layer(&mut f.project, [0.25, 0.5, 0.75], 0.5);
        // a paint layer below the fill's colour: half of it painted, so the blur has an edge to work on
        let paint = f.project.alloc_id();
        let l = Layer::new(paint, "Paint", LayerKind::Paint, &[Channel::BaseColor]);
        let paint = f.project.insert_layer(0, l, None, false).unwrap();
        f.project.texture_sets[0].stack.retain(|id| *id != paint);
        f.project.texture_sets[0].stack.insert(0, paint);
        let img = f.pixels.layers.entry((paint, Channel::BaseColor)).or_insert_with(|| TileImage::new(64, 64, 4));
        for y in 0..64 {
            for x in 0..32 {
                img.set(x, y, &[to_u16(1.0), to_u16(0.0), to_u16(0.0), to_u16(1.0)]);
            }
        }
        f.project.layer_mut(below).unwrap().channels.insert(Channel::BaseColor, ChannelBlend { mode: BlendMode::Normal, opacity: 0.5 });
        let id = filter_layer(&mut f, BlendMode::Passthrough, EffectKind::Graph { graph: filter, projection: Projection::default() });

        let mut cache = BelowCache::default();
        let first = run_cached(&f, &graphs, &mut cache);
        assert_eq!(first, run_with(&f, &graphs), "the same as without the cache");
        assert_eq!(cache.fields.len(), 1, "the whole-set field is kept");
        assert_eq!(cache.fields.reused(), 0);

        // composited again (as the next frame would): taken from the cache
        assert_eq!(run_cached(&f, &graphs, &mut cache), first);
        assert_eq!(cache.fields.reused(), 1);

        // an edit above the filter does not touch it
        let top = fill_layer(&mut f.project, [0.0; 3], 0.0);
        f.project.layer_mut(top).unwrap().channels.insert(Channel::BaseColor, ChannelBlend { mode: BlendMode::Normal, opacity: 0.25 });
        assert_eq!(run_cached(&f, &graphs, &mut cache), run_with(&f, &graphs));
        assert_eq!(cache.fields.reused(), 2);

        // edits below it: content, parameters, order, visibility; each one computes the field again
        let edits: [&dyn Fn(&mut Fixture); 4] = [
            &|f| f.pixels.layers.get_mut(&(paint, Channel::BaseColor)).unwrap().set(40, 20, &[to_u16(0.0), to_u16(1.0), to_u16(0.0), to_u16(1.0)]),
            &|f| {
                f.project
                    .layer_mut(below)
                    .unwrap()
                    .channels
                    .insert(Channel::BaseColor, ChannelBlend { mode: BlendMode::Normal, opacity: 0.8 })
                    .map(|_| ())
                    .unwrap()
            },
            &|f| {
                let stack = &mut f.project.texture_sets[0].stack;
                let (a, b) = (stack.iter().position(|l| *l == paint).unwrap(), stack.iter().position(|l| *l == below).unwrap());
                stack.swap(a, b);
            },
            &|f| f.project.layer_mut(paint).unwrap().visible = false,
        ];
        let mut last = run_cached(&f, &graphs, &mut cache);
        for (n, edit) in edits.iter().enumerate() {
            edit(&mut f);
            let reused = cache.fields.reused();
            let now = run_cached(&f, &graphs, &mut cache);
            assert_eq!(now, run_with(&f, &graphs), "edit {n}: the same as without the cache");
            assert_ne!(now, last, "edit {n} changes the result");
            assert_eq!(cache.fields.reused(), reused, "edit {n}: the field is computed again");
            assert_eq!(cache.fields.len(), 1, "edit {n}: the stale field is replaced");
            last = now;
        }

        // the filter's own parameters too; and deleting it drops the kept field
        f.project.layer_mut(id).unwrap().content_effects[0].opacity = 0.5;
        let reused = cache.fields.reused();
        assert_eq!(run_cached(&f, &graphs, &mut cache), run_with(&f, &graphs));
        assert_eq!(cache.fields.reused(), reused);
        f.project.layer_mut(id).unwrap().content_effects.clear();
        assert_eq!(run_cached(&f, &graphs, &mut cache), run_with(&f, &graphs));
        assert!(cache.fields.is_empty(), "nothing kept for a filter that is gone");
    }

    #[test]
    fn whole_set_fields_over_pixels_read_from_the_file_are_kept_across_frames() {
        // A project opened from its file holds layer pixels that read their tiles when first needed, each read a new
        // copy. The key a composite keeps may be taken before the tiles are read: it is the same pixels after.
        let mut graphs = GraphStore::default();
        let blur = graphs.add_text("format 3\nsize 64\nnode 1 input idx=0\nnode 2 box_blur in=@1 intensity=8\noutput out 2\n");
        let mut filter = GraphRef::new(blur, "blur");
        filter.inputs.insert(0, GraphInput::Stack { channel: None });
        let mut f = fixture();
        let paint = f.project.alloc_id();
        let l = Layer::new(paint, "Paint", LayerKind::Paint, &[Channel::BaseColor]);
        let paint = f.project.insert_layer(0, l, None, false).unwrap();
        let mut painted = TileImage::new(64, 64, 4);
        for y in 0..64 {
            for x in 0..32 {
                painted.set(x, y, &[to_u16(1.0), to_u16(0.0), to_u16(0.0), to_u16(1.0)]);
            }
        }
        let texels: Vec<u16> = painted.tile(0, 0).unwrap().to_vec();
        let from_file = |texels: Vec<u16>| TileImage::lazy(64, 64, 4, 1, Arc::new(move || Some(vec![Some(Arc::new(texels.clone()))])));
        f.pixels.layers.insert((paint, Channel::BaseColor), from_file(texels.clone()));
        // pixels no composite reads (a channel the layer does not draw) are never read at all
        f.pixels.layers.insert((paint, Channel::Roughness), from_file(texels));
        filter_layer(&mut f, BlendMode::Passthrough, EffectKind::Graph { graph: filter, projection: Projection::default() });

        let mut cache = BelowCache::default();
        let first = run_cached(&f, &graphs, &mut cache);
        assert_eq!(first, run_with(&f, &graphs), "the same as without the cache");
        assert_eq!(cache.fields.len(), 1);
        // the frames after: each takes the field from the cache instead of computing it again
        for frame in 1..4 {
            assert_eq!(run_cached(&f, &graphs, &mut cache), first);
            assert_eq!(cache.fields.reused(), frame, "frame {frame}: the field is computed once");
        }
        assert!(!f.pixels.layers[&(paint, Channel::Roughness)].is_loaded(), "comparing keys reads no pixels");
    }

    #[test]
    fn graph_filters_read_the_stack_padded_outside_the_islands() {
        let mut graphs = GraphStore::default();
        let blur = graphs.add_text("format 3\nsize 64\nnode 1 input idx=0\nnode 2 box_blur in=@1 intensity=32\noutput out 2\n");
        // the plane's UVs squeezed into the left half: the right half of the texture is outside every island
        let mut f = fixture();
        f.mesh.uvs.iter_mut().for_each(|uv| uv.x *= 0.5);
        f.gbuf = UvGBuffer::new(&f.mesh, 0, 64, 64);
        let id = fill_layer(&mut f.project, [0.6; 3], 0.6);
        let mut filter = GraphRef::new(blur, "blur");
        filter.inputs.insert(0, GraphInput::Stack { channel: None });
        content_fx(&mut f, id, EffectKind::Graph { graph: filter, projection: Projection::default() });
        let out = run_with(&f, &graphs);
        // next to the island's border the blur reads the island's value beyond it, not empty texels
        let r = out.value(Channel::Roughness, 30, 20)[0];
        assert!((r - 0.6).abs() < 2e-3, "{r}");
    }

    #[test]
    fn minified_graph_fills_average_unless_unfiltered() {
        // white on the left half of the image, black on the right
        let halves = "format 3\nsize 64\nsubgraph c\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_swizzle a=@2 mask=\"x\"\nnode 4 f_float v=0.5\nnode 5 f_lt a=@3 b=@4\nnode 1 f_out in=@5\nend\nnode 1 pixel_processor fn=\"c\"\noutput out 1\n";
        let mut graphs = GraphStore::default();
        let key = graphs.add_text(halves);
        let roughness = |unfiltered: bool| {
            let mut f = fixture();
            let id = fill_layer(&mut f.project, [0.0; 3], 0.0);
            let mut g = GraphRef::new(key.clone(), "halves");
            g.unfiltered = unfiltered;
            if let Some(LayerKind::Fill(fill)) = f.project.layers.get_mut(&id).map(|l| &mut l.kind) {
                fill.sources.insert(Channel::Roughness, Source::Graph(Box::new(g)));
                // the image once per texel, each texel's centre on the black half
                fill.projection = Projection::Uv { scale: [64.0, 64.0], rotation: 0.0, offset: [0.25, 0.0] };
            }
            run_with(&f, &graphs).value(Channel::Roughness, 10, 10)[0]
        };
        let filtered = roughness(false);
        assert!((filtered - 0.5).abs() < 0.02, "a texel spanning the whole image reads its mean: {filtered}");
        let unfiltered = roughness(true);
        assert!(unfiltered < 0.02, "bilinear at the texel's centre: {unfiltered}");
    }

    #[test]
    fn a_named_stack_input_on_its_own_channel_is_not_the_filters_own_input() {
        use crate::graphs::tests::INVERT;
        let mut graphs = GraphStore::default();
        let invert = graphs.add_text(INVERT);
        // a layer with nothing on height whose filter writes height, reading the stack's height by name (input 0) or as
        // its own input (no channel named)
        let height = |input: GraphInput| {
            let mut f = fixture();
            let id = f.project.alloc_id();
            let mut l = Layer::new(id, "Filter", LayerKind::Paint, &[Channel::Height]);
            l.channels.insert(Channel::Height, ChannelBlend { mode: BlendMode::LinearDodge, opacity: 1.0 });
            let id = f.project.insert_layer(0, l, None, false).unwrap();
            let mut filter = GraphRef::new(invert.clone(), "invert");
            filter.inputs.insert(0, input);
            filter.outputs.insert(Channel::Height, "out".into());
            content_fx(&mut f, id, EffectKind::Graph { graph: filter, projection: Projection::default() });
            run_with(&f, &graphs).value(Channel::Height, 10, 10)[0]
        };
        // named: reads 0 over the empty height (inverted: 1, the top of the signed range) and is laid on whole
        let named = height(GraphInput::Stack { channel: Some(Channel::Height) });
        assert!((named - 1.0).abs() < 1e-3, "{named}");
        // own: reads the flat surface (inverted: flat again)
        let own = height(GraphInput::Stack { channel: None });
        assert!(own.abs() < 1e-3, "{own}");
    }

    #[test]
    fn effect_layers_in_a_folder_start_from_the_folder_stack() {
        let mut f = fixture();
        fill_layer(&mut f.project, [1.0, 0.0, 0.0], 0.2);
        let gid = f.project.alloc_id();
        let g = Layer::new(gid, "Folder", LayerKind::Group { children: Vec::new(), collapsed: false }, &[Channel::BaseColor]);
        let gid = f.project.insert_layer(0, g, None, false).unwrap();
        let inner = f.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::BaseColor, Source::Uniform([0.0, 0.0, 1.0]));
        let l = Layer::new(inner, "Blue", LayerKind::Fill(fill), &[Channel::BaseColor]);
        let inner = f.project.insert_layer(0, l, Some(gid), true).unwrap();
        let fx = f.project.alloc_id();
        let mut l = Layer::new(fx, "Filter", LayerKind::Paint, &[Channel::BaseColor]);
        l.channels.insert(Channel::BaseColor, ChannelBlend { mode: BlendMode::Passthrough, opacity: 1.0 });
        let fx = f.project.insert_layer(0, l, Some(inner), false).unwrap();
        content_fx(&mut f, fx, EffectKind::Filter(Filter::Invert));
        let out = run(&f, None);
        assert!(
            close(out.value(Channel::BaseColor, 5, 5), [1.0, 1.0, 0.0]),
            "the blue fill inside the folder inverted: {:?}",
            out.value(Channel::BaseColor, 5, 5)
        );
    }

    #[test]
    fn retained_filter_buffers_release_after_the_last_tile() {
        for resolution in [128, 512] {
            for record_anchors in [false, true] {
                let mut fixture = fixture();
                fixture.gbuf = UvGBuffer::new(&fixture.mesh, 0, resolution, resolution);
                fixture.project.texture_sets[0] = TextureSet::new("Plane", 0, resolution);
                for color in [0.4, 0.2] {
                    let layer = fill_layer(&mut fixture.project, [color; 3], 0.5);
                    let mut mask = Mask::new(MaskBackground::White);
                    for radius in [8.0, 4.0, 2.0] {
                        mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Filter(Filter::Blur { radius })));
                        if record_anchors && radius != 2.0 {
                            mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Anchor));
                        }
                    }
                    fixture.project.layer_mut(layer).unwrap().mask = Some(mask);
                }

                let images = ImageStore::default();
                let input = Inputs {
                    project: &fixture.project,
                    ts: 0,
                    mesh: &fixture.mesh,
                    gbuf: &fixture.gbuf,
                    pixels: &fixture.pixels,
                    baked: &fixture.baked,
                    block_scales: fixture.project.texture_sets[0].block_scales(),
                    preview: None,
                    images: &images,
                    graphs: GraphStore::empty(),
                    bounds: fixture.mesh.bounds(),
                };
                let mut output = Composite::new(&fixture.project.texture_sets[0].channel_list(), resolution);
                let mut cache = BelowCache::default();
                cache.ahead = all_tiles(resolution, resolution).into_iter().skip(1).collect();

                composite_tiles_cached(&input, &mut output, &[(0, 0)], Some(&mut cache));
                let retained = cache.held.as_ref().expect("filter fields should be retained while tiles follow");
                let mark_count: usize = retained.fields.values().map(|field| field.marks.len()).sum();
                let weak_fields = Arc::downgrade(&retained.fields);
                assert_eq!(retained.fields.len(), 2);
                assert_eq!(mark_count, if record_anchors { 4 } else { 0 }, "only values read by mask anchors are retained");

                cache.ahead.clear();
                composite_tiles_cached(&input, &mut output, &[(1, 0)], Some(&mut cache));
                assert!(weak_fields.upgrade().is_none(), "the last tile must release retained field buffers");

                cache.ahead = all_tiles(resolution, resolution).into_iter().skip(1).collect();
                composite_tiles_cached(&input, &mut output, &[(0, 0)], Some(&mut cache));
                let weak_fields = Arc::downgrade(&cache.held.as_ref().unwrap().fields);
                cache.clear();
                assert!(weak_fields.upgrade().is_none(), "invalidating the cache releases intermediate anchor stages too");

                composite_tiles_cached(&input, &mut output, &[(0, 0)], Some(&mut cache));
                let weak_fields = Arc::downgrade(&cache.held.as_ref().unwrap().fields);
                drop(cache);
                assert!(weak_fields.upgrade().is_none(), "dropping the cache releases a cancelled run's fields");
            }
        }
    }

    #[test]
    fn histogram_samples_spread_over_every_column() {
        assert_eq!(histogram_tiles(1024, 1024), all_tiles(1024, 1024), "small sets are read whole");
        for size in [2048, 4096, 8192] {
            let tiles = histogram_tiles(size, size);
            let columns: std::collections::BTreeSet<u32> = tiles.iter().map(|t| t.0).collect();
            assert!(tiles.len() <= HISTOGRAM_TILES, "{size}: {} tiles", tiles.len());
            assert!(columns.len() as u32 >= size / TILE / 2, "{size}: only {} columns", columns.len());
        }
    }

    #[test]
    fn empty_stack_gives_channel_defaults() {
        let f = fixture();
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 10, 10), [0.5; 3]));
        assert!(close(out.value(Channel::Roughness, 10, 10), [0.5; 3]));
        assert!(close(out.value(Channel::Normal, 10, 10), [0.5, 0.5, 1.0]));
    }

    #[test]
    fn fill_layers_blend_with_opacity_and_modes() {
        let mut f = fixture();
        fill_layer(&mut f.project, [1.0, 0.0, 0.0], 0.2);
        let top = fill_layer(&mut f.project, [0.0, 0.0, 1.0], 0.8);
        f.project.layer_mut(top).unwrap().channels.insert(Channel::BaseColor, ChannelBlend { mode: BlendMode::Normal, opacity: 0.25 });
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 5, 5), [0.75, 0.0, 0.25]));
        assert!(close(out.value(Channel::Roughness, 5, 5), [0.8; 3]));
        f.project.layer_mut(top).unwrap().channels.insert(Channel::BaseColor, ChannelBlend { mode: BlendMode::Multiply, opacity: 1.0 });
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 5, 5), [0.0, 0.0, 0.0]));
    }

    #[test]
    fn paint_layers_unpremultiply_and_preview_and_masks() {
        let mut f = fixture();
        let id = f.project.alloc_id();
        let l = Layer::new(id, "Paint", LayerKind::Paint, &[Channel::BaseColor]);
        f.project.insert_layer(0, l, None, false).unwrap();

        // Half-covered red texel has premultiplied color and 0.5 coverage
        let img = f.pixels.layer_mut(id, Channel::BaseColor, (64, 64));
        img.set(3, 3, &[to_u16(0.5), 0, 0, to_u16(0.5)]);
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 3, 3), [0.75, 0.25, 0.25]));
        assert!(close(out.value(Channel::BaseColor, 4, 3), [0.5; 3]));

        // A live stroke paints blue at full alpha on texel (4, 3)
        let mut alpha = TileImage::new(64, 64, 1);
        alpha.set(4, 3, &[to_u16(1.0)]);
        let pv = StrokePreview {
            target: PaintTarget::Layer(id),
            alpha,
            opacity: 1.0,
            erase: false,
            values: [(Channel::BaseColor, [0.0, 0.0, 1.0])].into_iter().collect(),
            mask_value: 1.0,
            stencil: None,
            carried: BTreeMap::new(),
            carried_mask: None,
        };
        let out = run(&f, Some(&pv));
        assert!(close(out.value(Channel::BaseColor, 4, 3), [0.0, 0.0, 1.0]));

        // A black mask hides the layer
        f.project.layer_mut(id).unwrap().mask = Some(Mask::new(MaskBackground::Black));
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 3, 3), [0.5; 3]));
    }

    #[test]
    fn mask_effects_and_procedural_fills_stay_in_range() {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [1.0, 1.0, 1.0], 1.0);
        let eid = f.project.alloc_id();
        let mut mask = Mask::new(MaskBackground::Black);
        let pattern = Procedural::new(ProceduralKind::Checker);
        mask.effects.push(Effect::new(
            eid,
            EffectKind::Fill { source: Source::Procedural { pattern, low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() }, projection: Projection::default() },
        ));
        f.project.layer_mut(id).unwrap().mask = Some(mask);
        let out = run(&f, None);
        let vals: Vec<f32> = (0..64).map(|x| out.value(Channel::BaseColor, x, 20)[0]).collect();
        assert!(vals.iter().any(|v| *v > 0.99) && vals.iter().any(|v| *v < 0.51), "{vals:?}");
    }

    fn assert_sparse_mask_matches_image(fixture: &Fixture, layer: LayerId, texels: &[(u32, u32)]) -> Option<Vec<f32>> {
        let images = ImageStore::default();
        let inputs = Inputs {
            project: &fixture.project,
            ts: 0,
            mesh: &fixture.mesh,
            gbuf: &fixture.gbuf,
            pixels: &fixture.pixels,
            baked: &fixture.baked,
            block_scales: fixture.project.texture_sets[0].block_scales(),
            preview: None,
            images: &images,
            graphs: crate::GraphStore::empty(),
            bounds: fixture.mesh.bounds(),
        };
        let sparse = mask_at(&inputs, layer, texels);
        let image = mask_image(&inputs, layer, fixture.gbuf.width);
        assert_eq!(sparse.is_some(), image.is_some());
        let values = sparse?;
        let image = image?;
        assert_eq!(values.len(), texels.len());

        for (&(x, y), &value) in texels.iter().zip(&values) {
            let expected =
                if x < fixture.gbuf.width && y < fixture.gbuf.height { f32::from(image[(y * fixture.gbuf.width + x) as usize]) / 255.0 } else { 0.0 };
            assert!((value - expected).abs() <= 0.5 / 255.0 + 1e-6, "({x}, {y}): {value} vs {expected}");
        }
        Some(values)
    }

    #[test]
    fn mask_at_reads_the_same_values_as_the_mask_image() {
        let mut fixture = fixture();
        fixture.gbuf = UvGBuffer::new(&fixture.mesh, 0, 128, 128);
        fixture.project.texture_sets[0].resolution = 128;
        fixture.gbuf.tri[30 * 128 + 90] = NO_TRI;
        let layer = fill_layer(&mut fixture.project, [1.0; 3], 1.0);
        let maskless = fill_layer(&mut fixture.project, [1.0; 3], 1.0);
        let effect = fixture.project.alloc_id();
        let mut mask = Mask::new(MaskBackground::Black);
        let pattern = Procedural::new(ProceduralKind::Checker);
        mask.effects.push(Effect::new(
            effect,
            EffectKind::Fill { source: Source::Procedural { pattern, low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() }, projection: Projection::default() },
        ));
        fixture.project.layer_mut(layer).unwrap().mask = Some(mask);

        // Cross tile boundaries and include duplicates, a UV hole and invalid coordinates
        let texels = [(0, 0), (0, 40), (63, 63), (64, 64), (127, 127), (0, 40), (90, 30), (128, 0), (0, 128), (u32::MAX, u32::MAX)];
        let values = assert_sparse_mask_matches_image(&fixture, layer, &texels).unwrap();
        assert!(values.iter().any(|value| *value > 0.99) && values.iter().take(6).any(|value| *value < 0.01));
        assert_eq!(&values[6..], &[0.0; 4]);
        assert_eq!(assert_sparse_mask_matches_image(&fixture, layer, &[]), Some(Vec::new()));
        assert!(assert_sparse_mask_matches_image(&fixture, maskless, &texels).is_none());
        let missing = fixture.project.alloc_id();
        assert!(assert_sparse_mask_matches_image(&fixture, missing, &texels).is_none());
        fixture.project.layer_mut(layer).unwrap().mask.as_mut().unwrap().enabled = false;
        assert!(assert_sparse_mask_matches_image(&fixture, layer, &texels).is_none());
    }

    #[test]
    fn mask_image_resamples_a_non_square_atlas() {
        let mut fixture = fixture();
        fixture.gbuf = UvGBuffer::new(&fixture.mesh, 0, 128, 64);
        fixture.project.texture_sets[0].resolution = 128;
        let layer = fill_layer(&mut fixture.project, [1.0; 3], 1.0);
        let effect = fixture.project.alloc_id();
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(
            effect,
            EffectKind::Fill {
                source: Source::Procedural { pattern: Procedural::new(ProceduralKind::Checker), low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() },
                projection: Projection::default(),
            },
        ));
        fixture.project.layer_mut(layer).unwrap().mask = Some(mask);
        let images = ImageStore::default();
        let inputs = Inputs {
            project: &fixture.project,
            ts: 0,
            mesh: &fixture.mesh,
            gbuf: &fixture.gbuf,
            pixels: &fixture.pixels,
            baked: &fixture.baked,
            block_scales: fixture.project.texture_sets[0].block_scales(),
            preview: None,
            images: &images,
            graphs: GraphStore::empty(),
            bounds: fixture.mesh.bounds(),
        };
        let sampled_texels: Vec<_> = (0..64).flat_map(|y| (0..64).map(move |x| (x * 2, y))).collect();

        let preview = mask_image(&inputs, layer, 64).unwrap();
        let sparse = mask_at(&inputs, layer, &sampled_texels).unwrap();
        let atlas = mask_texels(&inputs, layer).unwrap();

        assert_eq!(preview.len(), 64 * 64);
        assert_eq!(atlas.len(), 128 * 64);
        assert!(preview.contains(&0));
        assert!(preview.contains(&255));
        for ((&preview_value, &sparse_value), &(x, y)) in preview.iter().zip(&sparse).zip(&sampled_texels) {
            assert!((f32::from(preview_value) / 255.0 - sparse_value).abs() <= 0.5 / 255.0 + 1e-6);
            assert!((atlas[(y * 128 + x) as usize] - sparse_value).abs() <= 1e-6);
        }
    }

    #[test]
    fn mask_at_ignores_out_of_range_texels_before_resampling_paint() {
        let mut fixture = fixture();
        let layer = fill_layer(&mut fixture.project, [1.0; 3], 1.0);
        let paint = fixture.project.alloc_id();
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(paint, EffectKind::Paint));
        fixture.project.layer_mut(layer).unwrap().mask = Some(mask);
        fixture.pixels.mask_mut(paint, (32, 32)).set(4, 5, &[u16::MAX; 2]);

        let texels = [(8, 10), (u32::MAX, 0), (0, u32::MAX), (u32::MAX, u32::MAX), (64, 0), (0, 64)];
        let values = assert_sparse_mask_matches_image(&fixture, layer, &texels).unwrap();
        assert!(values[0] > 0.5, "valid texels still read resampled paint");
        assert_eq!(&values[1..], &[0.0; 5]);
        assert_eq!(assert_sparse_mask_matches_image(&fixture, layer, &texels[1..]), Some(vec![0.0; 5]));
    }

    #[test]
    fn mask_at_preserves_anchors_and_neighbourhood_filters_across_tiles() {
        let mut fixture = fixture();
        fixture.gbuf = UvGBuffer::new(&fixture.mesh, 0, 128, 128);
        fixture.project.texture_sets[0].resolution = 128;
        let lower = fill_layer(&mut fixture.project, [0.0; 3], 1.0);
        let LayerKind::Fill(fill) = &mut fixture.project.layer_mut(lower).unwrap().kind else { panic!("expected a fill layer") };

        fill.sources.insert(
            Channel::BaseColor,
            Source::Procedural { pattern: Procedural::new(ProceduralKind::Checker), low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() },
        );
        let anchor = content_fx(&mut fixture, lower, EffectKind::Anchor);
        let upper = fill_layer(&mut fixture.project, [1.0; 3], 1.0);
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(
            fixture.project.alloc_id(),
            EffectKind::Fill { source: Source::Anchor { anchor, channel: Channel::BaseColor, ramp: Vec::new() }, projection: Projection::default() },
        ));
        fixture.project.layer_mut(upper).unwrap().mask = Some(mask.clone());
        let texels: Vec<_> = (0..128).map(|x| (x, 63)).collect();
        let original = assert_sparse_mask_matches_image(&fixture, upper, &texels).unwrap();
        assert!(original.iter().any(|value| *value > 0.99) && original.iter().any(|value| *value < 0.01), "the anchor supplies the checker");

        for filter in [Filter::Blur { radius: 64.0 }, Filter::DirectionalWarp { intensity: 256.0, angle: 90.0, scale: 4.0, seed: 3 }] {
            let mut filtered = mask.clone();
            filtered.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Filter(filter)));
            fixture.project.layer_mut(upper).unwrap().mask = Some(filtered);
            let values = assert_sparse_mask_matches_image(&fixture, upper, &texels).unwrap();
            assert!(values.iter().zip(&original).any(|(value, original)| (value - original).abs() > 0.1), "the filter changes the anchor values");
        }
    }

    #[test]
    fn a_mask_runs_every_neighbourhood_filter_in_order() {
        let mut fixture = fixture();
        fixture.gbuf = UvGBuffer::new(&fixture.mesh, 0, 128, 128);
        fixture.project.texture_sets[0].resolution = 128;
        let layer = fill_layer(&mut fixture.project, [1.0; 3], 1.0);
        let checker = Source::Procedural { pattern: Procedural::new(ProceduralKind::Checker), low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() };
        let warp = Filter::DirectionalWarp { intensity: 256.0, angle: 90.0, scale: 4.0, seed: 3 };
        let texels: Vec<_> = (0..128).map(|x| (x, 63)).collect();
        let mut masked = |filters: &[Filter]| {
            let mut mask = Mask::new(MaskBackground::Black);
            mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Fill { source: checker.clone(), projection: Projection::default() }));
            for filter in filters {
                mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Filter(filter.clone())));
            }
            fixture.project.layer_mut(layer).unwrap().mask = Some(mask);
            assert_sparse_mask_matches_image(&fixture, layer, &texels).unwrap()
        };

        let warped = masked(std::slice::from_ref(&warp));
        let blurred_then_warped = masked(&[Filter::Blur { radius: 64.0 }, warp]);
        assert!(blurred_then_warped.iter().zip(&warped).any(|(chained, single)| (chained - single).abs() > 0.1), "the blur under the warp still runs");
        assert!(blurred_then_warped.iter().any(|value| *value > 0.05 && *value < 0.95), "the blur softens the checker before the warp moves it");
    }

    #[test]
    fn a_content_stack_runs_every_neighbourhood_filter_in_order() {
        let mut fixture = fixture();
        let layer = fill_layer(&mut fixture.project, [0.0; 3], 1.0);
        let LayerKind::Fill(fill) = &mut fixture.project.layer_mut(layer).unwrap().kind else { panic!("expected a fill layer") };

        fill.sources.insert(
            Channel::BaseColor,
            Source::Procedural { pattern: Procedural::new(ProceduralKind::Checker), low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() },
        );
        let warp = Filter::DirectionalWarp { intensity: 256.0, angle: 90.0, scale: 4.0, seed: 3 };
        let warp_effect = content_fx(&mut fixture, layer, EffectKind::Filter(warp));
        let row = |fixture: &Fixture| -> Vec<f32> {
            let composite = run(fixture, None);
            (0..64).map(|x| composite.value(Channel::BaseColor, x, 20)[0]).collect()
        };
        let warped = row(&fixture);

        content_fx(&mut fixture, layer, EffectKind::Filter(Filter::Blur { radius: 64.0 }));
        let effects = &mut fixture.project.layer_mut(layer).unwrap().content_effects;
        let warp_index = effects.iter().position(|effect| effect.id == warp_effect).unwrap();
        let warp_on_top = effects.remove(warp_index);
        effects.push(warp_on_top);
        let blurred_then_warped = row(&fixture);
        assert!(blurred_then_warped.iter().zip(&warped).any(|(chained, single)| (chained - single).abs() > 0.1), "the blur under the warp still runs");
    }

    /// A 256² set whose curvature map has a one-texel ridge in column `ridge_x` (flat elsewhere), and a white fill layer to mask.
    fn ridge_fixture(ridge_x: usize) -> (Fixture, LayerId) {
        let mut fixture = fixture();
        fixture.gbuf = UvGBuffer::new(&fixture.mesh, 0, 256, 256);
        fixture.project.texture_sets[0].resolution = 256;
        let ridge: Vec<f32> = (0..256 * 256).map(|texel| if texel % 256 == ridge_x { 1.0 } else { 0.5 }).collect();
        fixture.baked = BakedMaps { resolution: 256, maps: [(BakedMap::Curvature, Arc::new(ridge.into()))].into() };
        let layer = fill_layer(&mut fixture.project, [1.0; 3], 1.0);
        (fixture, layer)
    }

    /// `layer`'s mask made of `effects` (bottom first) on black, read along row 128 over `xs`.
    fn mask_row(fixture: &mut Fixture, layer: LayerId, effects: Vec<Effect>, xs: std::ops::Range<u32>) -> Vec<f32> {
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects = effects;
        fixture.project.layer_mut(layer).unwrap().mask = Some(mask);
        let images = Default::default();
        let inputs = Inputs {
            project: &fixture.project,
            ts: 0,
            mesh: &fixture.mesh,
            gbuf: &fixture.gbuf,
            pixels: &fixture.pixels,
            baked: &fixture.baked,
            block_scales: BlockScales::default(),
            preview: None,
            images: &images,
            graphs: GraphStore::empty(),
            bounds: fixture.mesh.bounds(),
        };
        mask_at(&inputs, layer, &xs.map(|x| (x, 128)).collect::<Vec<_>>()).unwrap()
    }

    /// A generator effect of `fixture`'s project.
    fn generator_effect(fixture: &mut Fixture, generator: Generator) -> Effect {
        Effect::new(fixture.project.alloc_id(), EffectKind::Generator(generator))
    }

    /// An edge wear generator that marks exactly the curvature ridge (the map as baked, a hard step).
    fn ridge_generator() -> Generator {
        Generator { level: 0.5, softness: 0.01, noise_amount: 0.0, detail: None, ..Generator::new(GeneratorKind::EdgeWear) }
    }

    #[test]
    fn larger_curvature_detail_sizes_reach_further_from_an_edge() {
        let (mut fixture, layer) = ridge_fixture(128);
        let mut row_with = |weights: [f32; 7]| {
            let detail = CurvatureDetail { mode: CurvatureMode::Edges, weights };
            let generator = Generator { level: 0.5, softness: 0.5, noise_amount: 0.0, detail: Some(detail), ..Generator::new(GeneratorKind::Curvature) };
            let effect = generator_effect(&mut fixture, generator);
            mask_row(&mut fixture, layer, vec![effect], 120..137)
        };

        // The sampled ridge is index 8; five texels to its left is index 3
        let (ridge, five_texels_away) = (8, 3);
        let finest = row_with([1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        let largest = row_with([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
        assert!(finest[ridge] > 0.99 && finest[five_texels_away] < 0.01, "the map itself marks only the ridge: {finest:?}");
        assert!(largest[ridge] > 0.99 && largest[five_texels_away] > 0.5, "the largest size reaches texels away from it: {largest:?}");
    }

    #[test]
    fn a_replaced_curvature_input_is_read_at_its_own_size_only() {
        let (mut fixture, layer) = ridge_fixture(128);
        let detail = CurvatureDetail { mode: CurvatureMode::Edges, weights: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0] };
        let mut generator = Generator { level: 0.5, softness: 0.5, noise_amount: 0.0, detail: Some(detail), ..Generator::new(GeneratorKind::Curvature) };
        generator.inputs.insert(BakedMap::Curvature, Source::Uniform([0.5; 3]));
        let effect = generator_effect(&mut fixture, generator);
        let row = mask_row(&mut fixture, layer, vec![effect], 120..137);

        // The largest size alone would mark the ridge and texels around it; the flat input marks nothing
        assert!(row.iter().all(|value| *value < 0.01), "the detail sizes of the mesh map are not read: {row:?}");
    }

    #[test]
    fn generator_blur_softens_only_the_generator_not_the_stack_below() {
        let (mut fixture, layer) = ridge_fixture(100);
        let mut row_with = |blur: f32| {
            let checker = Source::Procedural { pattern: Procedural::new(ProceduralKind::Checker), low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() };
            let below = Effect::new(fixture.project.alloc_id(), EffectKind::Fill { source: checker, projection: Projection::default() });
            let mut generator = ridge_generator();
            generator.global.blur = blur;
            let mut lighten = generator_effect(&mut fixture, generator);
            lighten.blend = BlendMode::Lighten;
            mask_row(&mut fixture, layer, vec![below, lighten], 40..111)
        };

        // The sampled row crosses a checker edge; indices below 30 are outside the blur
        let (ridge, five_texels_away, far) = (60, 55, 0..30);
        let sharp = row_with(0.0);
        let blurred = row_with(1.0);
        assert!(sharp[ridge] > 0.99 && sharp[five_texels_away] < 0.01, "unblurred, the ridge is one texel: {sharp:?}");
        assert!(blurred[ridge] < 0.99 && blurred[ridge] > blurred[five_texels_away], "the blur spreads the ridge: {blurred:?}");
        assert!(blurred[five_texels_away] > 0.01, "and reaches texels away from it: {blurred:?}");
        for texel in far {
            assert_eq!(blurred[texel], sharp[texel], "the checker below keeps its hard edges at texel {texel}");
        }
    }

    #[test]
    fn a_lower_generator_resolution_spreads_a_ridge_over_its_cell() {
        let (mut fixture, layer) = ridge_fixture(100);
        let mut row_with = |factor: i8| {
            let mut generator = ridge_generator();
            generator.resolution.factor = factor;
            let effect = generator_effect(&mut fixture, generator);
            mask_row(&mut fixture, layer, vec![effect], 80..121)
        };

        // At one eighth resolution, each cell spans eight texels
        let (ridge, beside, far) = (20, 18, 0);
        let full = row_with(0);
        let eighth = row_with(-3);
        assert!(full[ridge] > 0.99 && full[beside] < 0.01, "at full resolution the ridge is one texel: {full:?}");
        assert!(eighth[ridge] < 0.5 && eighth[beside] > 0.01, "an eighth of the resolution averages it over its cell: {eighth:?}");
        assert!(eighth[far] < 0.01, "texels a few cells away stay black: {eighth:?}");
    }

    #[test]
    fn every_caller_lists_the_same_generator_stages_whatever_resolution_it_passes() {
        let custom = OutputResolution { mode: ResolutionMode::Custom, custom: 1024, factor: 0 };
        let effects = [Effect::new(LayerId(1), EffectKind::Generator(Generator { resolution: custom, ..Generator::new(GeneratorKind::Dirt) }))];

        // Tile and whole-set resolutions must select the same generator stage
        let at_tile: Vec<usize> = neighbourhood_filters(&effects, 1024, None).map(|stage| stage.0).collect();
        let at_set_width: Vec<usize> = neighbourhood_filters(&effects, 2048, None).map(|stage| stage.0).collect();
        assert_eq!(at_tile, at_set_width);
        assert_eq!(at_tile, [0], "a custom resolution can be coarser than some UV tile, so it is a stage");
    }

    #[test]
    fn a_generator_input_replaced_by_a_value_is_read_instead_of_the_mesh_map() {
        let mut fixture = fixture();
        fixture.baked = BakedMaps { resolution: 64, maps: [(BakedMap::AmbientOcclusion, Arc::new(vec![1.0; 64 * 64].into()))].into() };
        let layer = fill_layer(&mut fixture.project, [1.0; 3], 1.0);
        let value_with = |fixture: &mut Fixture, input: Option<f32>| -> f32 {
            let mut generator = Generator { level: 0.5, softness: 0.1, noise_amount: 0.0, ..Generator::new(GeneratorKind::Dirt) };
            if let Some(value) = input {
                generator.inputs.insert(BakedMap::AmbientOcclusion, Source::Uniform([value; 3]));
            }
            let mut mask = Mask::new(MaskBackground::Black);
            mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Generator(generator)));
            fixture.project.layer_mut(layer).unwrap().mask = Some(mask);
            run(fixture, None).value(Channel::BaseColor, 20, 20)[0]
        };

        // Dark occlusion reveals dirt while the baked white map stays clear
        let (baked, black) = (value_with(&mut fixture, None), value_with(&mut fixture, Some(0.0)));
        assert!(black > 0.99 && baked < black - 0.2, "baked {baked}, black input {black}");
    }

    #[test]
    fn curvature_detail_noise_keeps_flat_areas_neutral() {
        let mut fixture = fixture();
        fixture.baked = BakedMaps { resolution: 64, maps: [(BakedMap::Curvature, Arc::new(vec![0.5; 64 * 64].into()))].into() };
        let layer = fill_layer(&mut fixture.project, [1.0; 3], 1.0);
        let texels: Vec<_> = (0..64).map(|x| (x, 32)).collect();

        for kind in [GeneratorKind::Curvature, GeneratorKind::EdgeWear] {
            for mode in CurvatureMode::ALL {
                let detail = CurvatureDetail { mode, weights: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] };
                let generator = Generator { level: 0.5, softness: 0.5, noise_amount: 2.0, detail: Some(detail), ..Generator::new(kind) };
                let mut mask = Mask::new(MaskBackground::Black);
                mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Generator(generator)));
                fixture.project.layer_mut(layer).unwrap().mask = Some(mask);

                let values = assert_sparse_mask_matches_image(&fixture, layer, &texels).unwrap();
                let expected = if mode == CurvatureMode::Raw { 0.5 } else { 0.0 };
                assert!(values.iter().all(|value| (value - expected).abs() < 0.001), "{kind:?} {mode:?} paints noise on flat curvature: {values:?}");
            }
        }
    }

    #[test]
    fn color_selection_picks_from_the_baked_map_it_names() {
        let mut fixture = fixture();
        // curvature flat (0.5) on the left half, a ridge on the right; the ID map black everywhere
        let curvature: Vec<f32> = (0..64 * 64).map(|i| if i % 64 < 32 { 0.5 } else { 0.8 }).collect();
        fixture.baked = BakedMaps {
            resolution: 64,
            maps: [(BakedMap::Curvature, Arc::new(curvature.into())), (BakedMap::Id, Arc::new(vec![0.0; 64 * 64 * 3].into()))].into(),
        };
        let layer = fill_layer(&mut fixture.project, [1.0; 3], 1.0);
        let texels = [(8, 32), (48, 32)];
        for (map, expected) in [(BakedMap::Curvature, [1.0, 0.0]), (BakedMap::Id, [0.0, 0.0])] {
            let mut mask = Mask::new(MaskBackground::Black);
            mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::ColorSelection { colors: vec![[0.5; 3]], tolerance: 0.0, map }));
            fixture.project.layer_mut(layer).unwrap().mask = Some(mask);
            let values = assert_sparse_mask_matches_image(&fixture, layer, &texels).unwrap();
            assert!(values.iter().zip(expected).all(|(v, e)| (v - e).abs() < 0.001), "{map:?}: {values:?}");
        }
    }

    #[test]
    fn raw_curvature_reads_mid_gray_without_a_curvature_map() {
        let mut fixture = fixture();
        let layer = fill_layer(&mut fixture.project, [1.0; 3], 1.0);
        let detail = CurvatureDetail { mode: CurvatureMode::Raw, ..CurvatureDetail::default() };
        let generator = Generator { level: 0.5, softness: 0.5, noise_amount: 0.0, detail: Some(detail), ..Generator::new(GeneratorKind::Curvature) };
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Generator(generator)));
        fixture.project.layer_mut(layer).unwrap().mask = Some(mask);

        let values = assert_sparse_mask_matches_image(&fixture, layer, &[(32, 32)]).unwrap();
        assert!(values.iter().all(|value| (value - 0.5).abs() < 0.01), "flat curvature is mid gray in raw mode, not black: {values:?}");
    }

    #[test]
    fn stack_reads_neighbours_sees_blurs_in_masks_and_content() {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [1.0; 3], 1.0);
        assert!(!stack_reads_neighbours(&f.project, 0));
        let mut mask = Mask::new(MaskBackground::White);
        let eid = f.project.alloc_id();
        mask.effects.push(Effect::new(eid, EffectKind::Filter(Filter::Blur { radius: 8.0 })));
        f.project.layer_mut(id).unwrap().mask = Some(mask);
        assert!(stack_reads_neighbours(&f.project, 0));
        f.project.layer_mut(id).unwrap().mask = None;
        assert!(!stack_reads_neighbours(&f.project, 0));
        content_fx(&mut f, id, EffectKind::Filter(Filter::Mosaic { size: 8.0 }));
        assert!(stack_reads_neighbours(&f.project, 0));
    }

    #[test]
    fn blur_filter_softens_mask_edges() {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [1.0, 1.0, 1.0], 1.0);
        let mut mask = Mask::new(MaskBackground::Black);
        let pattern = Procedural::new(ProceduralKind::Checker);
        let (e1, e2) = (f.project.alloc_id(), f.project.alloc_id());
        mask.effects.push(Effect::new(
            e1,
            EffectKind::Fill { source: Source::Procedural { pattern, low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() }, projection: Projection::default() },
        ));
        f.project.layer_mut(id).unwrap().mask = Some(mask.clone());
        let hard: Vec<f32> = (0..64).map(|x| run(&f, None).value(Channel::BaseColor, x, 20)[0]).collect();
        mask.effects.push(Effect::new(e2, EffectKind::Filter(Filter::Blur { radius: 64.0 })));
        f.project.layer_mut(id).unwrap().mask = Some(mask);
        let soft_img = run(&f, None);
        let soft: Vec<f32> = (0..64).map(|x| soft_img.value(Channel::BaseColor, x, 20)[0]).collect();
        let mids = |v: &[f32]| v.iter().filter(|x| **x > 0.55 && **x < 0.95).count();
        assert!(
            mids(&soft) > mids(&hard) + 4,
            "hard {hard:?}
soft {soft:?}"
        );
    }

    /// A fill of `values` (channel, value, mode, opacity) at the top of the stack, or of folder `parent`.
    fn rule_fill(f: &mut Fixture, values: &[(Channel, f32, BlendMode, f32)], parent: Option<LayerId>) -> LayerId {
        let id = f.project.alloc_id();
        let mut fill = Fill::default();
        for (c, v, ..) in values {
            fill.sources.insert(*c, Source::Uniform([*v; 3]));
        }
        let mut l = Layer::new(id, "F", LayerKind::Fill(fill), &[]);
        for (c, _, mode, opacity) in values {
            l.channels.insert(*c, ChannelBlend { mode: *mode, opacity: *opacity });
        }
        f.project.insert_layer(0, l, parent, parent.is_some()).unwrap()
    }

    /// Roughness and height at one texel under TextureCraft's own and the measured rules.
    fn both_rules(f: &mut Fixture) -> [(f32, f32); 2] {
        let mut out = [(0.0, 0.0); 2];
        for (slot, rules) in out.iter_mut().zip([CompositeRules::default(), CompositeRules::measured(HeightNormal::Backward)]) {
            f.project.rules = rules;
            let c = run(f, None);
            *slot = (c.value(Channel::Roughness, 10, 10)[0], c.value(Channel::Height, 10, 10)[0]);
        }
        f.project.rules = CompositeRules::default();
        out
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 2e-3
    }

    #[test]
    fn measured_rules_lay_a_mode_only_where_something_is_below() {
        let mut f = fixture();
        rule_fill(&mut f, &[(Channel::Roughness, 0.4, BlendMode::Multiply, 1.0)], None);
        let [native, measured] = both_rules(&mut f);
        let d = Channel::Roughness.default_value()[0];
        assert!(near(native.0, d * 0.4), "TextureCraft's own: multiplies the channel default: {native:?}");
        assert!(near(measured.0, 0.4), "measured: over nothing the fill shows: {measured:?}");
        // over a full fill both multiply
        let mut f = fixture();
        rule_fill(&mut f, &[(Channel::Roughness, 0.6, BlendMode::Normal, 1.0)], None);
        rule_fill(&mut f, &[(Channel::Roughness, 0.4, BlendMode::Multiply, 1.0)], None);
        let [native, measured] = both_rules(&mut f);
        assert!(near(native.0, 0.24) && near(measured.0, 0.24), "{native:?} {measured:?}");
    }

    #[test]
    fn measured_rules_clamp_before_opacity_on_unit_channels_only() {
        let mut f = fixture();
        rule_fill(&mut f, &[(Channel::Roughness, 0.8, BlendMode::Normal, 1.0), (Channel::Height, 0.6, BlendMode::Normal, 1.0)], None);
        rule_fill(&mut f, &[(Channel::Roughness, 0.7, BlendMode::LinearDodge, 0.5), (Channel::Height, 0.7, BlendMode::LinearDodge, 0.5)], None);
        let [native, measured] = both_rules(&mut f);
        assert!(near(native.0, 1.0), "0.8 + 0.35 stored as 1: {native:?}");
        assert!(near(measured.0, 0.9), "measured: 1.5 clamped to 1, then half way from 0.8: {measured:?}");
        assert!(near(native.1, 0.95) && near(measured.1, 0.95), "height is a float channel: no clamp {native:?} {measured:?}");
    }

    #[test]
    fn measured_rules_composite_folders_on_their_own() {
        let mut f = fixture();
        rule_fill(&mut f, &[(Channel::Roughness, 0.6, BlendMode::Normal, 1.0), (Channel::Height, 0.25, BlendMode::Normal, 1.0)], None);
        let gid = f.project.alloc_id();
        let mut g = Layer::new(gid, "G", LayerKind::Group { children: vec![], collapsed: false }, &[]);
        g.channels.insert(Channel::Roughness, ChannelBlend { mode: BlendMode::Normal, opacity: 1.0 });
        g.channels.insert(Channel::Height, ChannelBlend { mode: BlendMode::LinearDodge, opacity: 1.0 });
        let gid = f.project.insert_layer(0, g, None, false).unwrap();
        rule_fill(&mut f, &[(Channel::Roughness, 0.5, BlendMode::Multiply, 1.0), (Channel::Height, 0.1, BlendMode::Normal, 1.0)], Some(gid));
        let [native, measured] = both_rules(&mut f);
        // TextureCraft's own: the children blend straight into the stack below
        assert!(near(native.0, 0.3) && near(native.1, 0.1), "{native:?}");
        // measured: the folder's composite (0.5 over nothing; height 0.1) laid normal and added
        assert!(near(measured.0, 0.5) && near(measured.1, 0.35), "{measured:?}");
        // a folder passing through behaves alike under both
        for c in [Channel::Roughness, Channel::Height] {
            f.project.layer_mut(gid).unwrap().channels.insert(c, ChannelBlend { mode: BlendMode::Passthrough, opacity: 1.0 });
        }
        let [native, measured] = both_rules(&mut f);
        assert_eq!(native, measured);
        // at half opacity a folder's composite over nothing mixes by its coverage: 0.5 at half strength over 0.6
        for (c, mode) in [(Channel::Roughness, BlendMode::Normal), (Channel::Height, BlendMode::LinearDodge)] {
            f.project.layer_mut(gid).unwrap().channels.insert(c, ChannelBlend { mode, opacity: 0.5 });
        }
        let [_, measured] = both_rules(&mut f);
        assert!(near(measured.0, 0.55) && near(measured.1, 0.3), "{measured:?}");
    }

    #[test]
    fn measured_rules_effect_layers_over_nothing_add_nothing() {
        // the stack: roughness 0.6; on top an isolating folder holding only an effect layer (a point filter, then a
        // neighbourhood one) that passes roughness through
        for filter in [Filter::Invert, Filter::Sharpen { amount: 1.0 }] {
            let mut f = fixture();
            rule_fill(&mut f, &[(Channel::Roughness, 0.6, BlendMode::Normal, 1.0)], None);
            let gid = f.project.alloc_id();
            let mut g = Layer::new(gid, "G", LayerKind::Group { children: vec![], collapsed: false }, &[]);
            g.channels.insert(Channel::Roughness, ChannelBlend { mode: BlendMode::Normal, opacity: 1.0 });
            let gid = f.project.insert_layer(0, g, None, false).unwrap();
            let fx = f.project.alloc_id();
            let mut l = Layer::new(fx, "Filter", LayerKind::Paint, &[]);
            l.channels.insert(Channel::Roughness, ChannelBlend { mode: BlendMode::Passthrough, opacity: 1.0 });
            let fx = f.project.insert_layer(0, l, Some(gid), true).unwrap();
            content_fx(&mut f, fx, EffectKind::Filter(filter.clone()));
            let [native, measured] = both_rules(&mut f);
            let want = if matches!(filter, Filter::Invert) { 0.4 } else { 0.6 };
            assert!(near(native.0, want), "TextureCraft's own: the folder passes the filtered stack: {native:?}");
            assert!(near(measured.0, 0.6), "measured: the folder starts from nothing, the filter adds nothing: {measured:?}");

            // with a fill below it inside the folder the filter works on that fill
            let inner = rule_fill(&mut f, &[(Channel::Roughness, 0.3, BlendMode::Normal, 1.0)], Some(gid));
            let LayerKind::Group { children, .. } = &mut f.project.layer_mut(gid).unwrap().kind else { panic!("expected a folder") };
            children.retain(|c| *c != inner);
            children.insert(0, inner);
            let [_, measured] = both_rules(&mut f);
            let want = if matches!(filter, Filter::Invert) { 0.7 } else { 0.3 };
            assert!(near(measured.0, want), "{filter:?}: {measured:?}");
        }
    }

    #[test]
    fn measured_formulas_reach_the_composite() {
        let mut f = fixture();
        rule_fill(&mut f, &[(Channel::Roughness, 0.8, BlendMode::Normal, 1.0)], None);
        rule_fill(&mut f, &[(Channel::Roughness, 0.4, BlendMode::Divide, 1.0)], None);
        let [native, measured] = both_rules(&mut f);
        assert!(near(native.0, 1.0) && near(measured.0, 1.0), "both end at 1 (stored): {native:?} {measured:?}");
        let mut f = fixture();
        rule_fill(&mut f, &[(Channel::Roughness, 0.25, BlendMode::Normal, 1.0)], None);
        rule_fill(&mut f, &[(Channel::Roughness, 0.7, BlendMode::SoftLight, 1.0)], None);
        let [native, measured] = both_rules(&mut f);
        assert!(near(native.0, blend1(BlendMode::SoftLight, 0.25, 0.7)), "{native:?}");
        assert!(near(measured.0, 2.0 * 0.25 * 0.3 + 0.5 * 0.4), "{measured:?}");
    }

    #[test]
    fn groups_pass_through_with_opacity() {
        let mut f = fixture();
        let gid = f.project.alloc_id();
        let g = Layer::new(gid, "G", LayerKind::Group { children: vec![], collapsed: false }, &[Channel::BaseColor]);
        f.project.insert_layer(0, g, None, false).unwrap();
        f.project.layer_mut(gid).unwrap().channels.insert(Channel::BaseColor, ChannelBlend { mode: BlendMode::Passthrough, opacity: 0.5 });
        let cid = f.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::BaseColor, Source::Uniform([1.0, 1.0, 1.0]));
        let child = Layer::new(cid, "C", LayerKind::Fill(fill), &[Channel::BaseColor]);
        f.project.insert_layer(0, child, Some(gid), true).unwrap();
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 1, 1), [0.75; 3]));
    }

    #[test]
    fn padding_copies_nearest_covered_value() {
        let mut out = Composite::new(&[Channel::Roughness], 64);
        out.channels.get_mut(&Channel::Roughness).unwrap().set(10, 10, &[to_u16(0.9)]);
        let mut covered = vec![false; 64 * 64];
        covered[10 * 64 + 10] = true;
        let dmap = DilationMap::new(&covered, 64, 64, Some(4));
        pad_tiles(&mut out, &dmap, &[(0, 0)]);
        assert!((out.value(Channel::Roughness, 12, 12)[0] - 0.9).abs() < 1e-3);
        assert!(out.value(Channel::Roughness, 30, 30)[0] < 0.1 || !out.channels[&Channel::Roughness].is_allocated(0, 0));
    }

    fn content_fx(f: &mut Fixture, id: LayerId, kind: EffectKind) -> LayerId {
        let eid = f.project.alloc_id();
        f.project.layer_mut(id).unwrap().content_effects.push(Effect::new(eid, kind));
        eid
    }

    #[test]
    fn content_effects_limited_to_channels_leave_the_others() {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [0.2, 0.4, 0.6], 0.2);
        let fx = content_fx(&mut f, id, EffectKind::Levels(Levels { out_low: 1.0, out_high: 0.0, ..Levels::default() }));
        let out = run(&f, None);
        assert!(close(out.value(Channel::Roughness, 5, 5), [0.8; 3]), "on every channel by default");
        assert!(close(out.value(Channel::BaseColor, 5, 5), [0.8, 0.6, 0.4]));
        let e = f.project.layer_mut(id).unwrap().content_effects.iter_mut().find(|e| e.id == fx).unwrap();
        e.channels = vec![Channel::BaseColor];
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 5, 5), [0.8, 0.6, 0.4]));
        assert!(close(out.value(Channel::Roughness, 5, 5), [0.2; 3]), "a channel the effect does not list is left as it is");
    }

    #[test]
    fn content_filters_hsl_and_opacity_change_layer_values() {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [1.0, 0.0, 0.0], 0.2);
        content_fx(&mut f, id, EffectKind::Filter(Filter::Hsl { hue: 1.0 / 3.0, saturation: 0.0, lightness: 0.0 }));
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 5, 5), [0.0, 1.0, 0.0]), "{:?}", out.value(Channel::BaseColor, 5, 5));
        assert!(close(out.value(Channel::Roughness, 5, 5), [0.2; 3]), "a hue shift leaves scalars alone");
        content_fx(&mut f, id, EffectKind::Filter(Filter::Invert));
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 5, 5), [1.0, 0.0, 1.0]));
        assert!(close(out.value(Channel::Roughness, 5, 5), [0.8; 3]));
        // Hidden effects are skipped and opacity fades an effect.
        let fx = &mut f.project.layer_mut(id).unwrap().content_effects;
        fx[0].visible = false;
        fx[1].opacity = 0.5;
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 5, 5), [0.5; 3]));
        assert!(close(out.value(Channel::Roughness, 5, 5), [0.5; 3]));
    }

    #[test]
    fn hsl_on_a_mask_changes_its_lightness() {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [1.0; 3], 1.0);
        let mut mask = Mask::new(MaskBackground::White);
        mask.effects.push(Effect::new(f.project.alloc_id(), EffectKind::Filter(Filter::Hsl { hue: 0.3, saturation: 1.0, lightness: -0.5 })));
        f.project.layer_mut(id).unwrap().mask = Some(mask);
        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [0.75; 3]));
    }

    #[test]
    fn content_blur_softens_fill_edges() {
        let mut f = fixture();
        let id = f.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(
            Channel::Roughness,
            Source::Procedural { pattern: Procedural::new(ProceduralKind::Checker), low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() },
        );
        f.project.insert_layer(0, Layer::new(id, "F", LayerKind::Fill(fill), &[Channel::Roughness]), None, false).unwrap();
        let row = |f: &Fixture| {
            let out = run(f, None);
            (0..64).map(|x| out.value(Channel::Roughness, x, 20)[0]).collect::<Vec<f32>>()
        };
        let hard = row(&f);
        content_fx(&mut f, id, EffectKind::Filter(Filter::Blur { radius: 64.0 }));
        let soft = row(&f);
        let mids = |v: &[f32]| v.iter().filter(|x| **x > 0.05 && **x < 0.95).count();
        assert!(mids(&soft) > mids(&hard) + 4, "hard {hard:?}\nsoft {soft:?}");
        assert!(soft.iter().all(|v| (0.0..=1.0).contains(v)));
    }

    #[test]
    fn high_pass_edge_detect_directional_warp_grayscale_and_normal_invert() {
        // Uniform content gives mid gray high pass and black edge detection
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [1.0, 0.0, 0.0], 0.2);
        let hp = content_fx(&mut f, id, EffectKind::Filter(Filter::HighPass { radius: 64.0 }));
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 30, 30), [0.5; 3]), "{:?}", out.value(Channel::BaseColor, 30, 30));
        assert!(close(out.value(Channel::Roughness, 30, 30), [0.5; 3]));
        f.project.layer_mut(id).unwrap().content_effects.retain(|e| e.id != hp);
        content_fx(&mut f, id, EffectKind::Filter(Filter::EdgeDetect { width: 64.0 }));
        assert!(close(run(&f, None).value(Channel::Roughness, 30, 30), [0.0; 3]));

        // A checker's borders light up while cell interiors stay black
        let mut f = fixture();
        let id = f.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(
            Channel::Roughness,
            Source::Procedural { pattern: Procedural::new(ProceduralKind::Checker), low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() },
        );
        f.project.insert_layer(0, Layer::new(id, "F", LayerKind::Fill(fill), &[Channel::Roughness]), None, false).unwrap();
        let plain = run(&f, None);
        let edges = content_fx(&mut f, id, EffectKind::Filter(Filter::EdgeDetect { width: 32.0 }));
        let out = run(&f, None);
        let vals: Vec<f32> = (0..64).flat_map(|y| (0..64).map(move |x| (x, y))).map(|(x, y)| out.value(Channel::Roughness, x, y)[0]).collect();
        assert!(vals.iter().all(|v| (0.0..=1.0).contains(v)));
        assert!(vals.iter().filter(|v| **v > 0.5).count() > 64 && vals.iter().filter(|v| **v < 0.01).count() > 64, "{vals:?}");
        f.project.layer_mut(id).unwrap().content_effects.retain(|e| e.id != edges);
        content_fx(&mut f, id, EffectKind::Filter(Filter::DirectionalWarp { intensity: 256.0, angle: 90.0, scale: 4.0, seed: 3 }));
        let warped = run(&f, None);
        let differs = (0..64).any(|y| (0..64).any(|x| (warped.value(Channel::Roughness, x, y)[0] - plain.value(Channel::Roughness, x, y)[0]).abs() > 0.2));
        assert!(differs, "the warp moves the checker");

        // Point filters
        assert!(close(Filter::Grayscale.point(ChannelKind::Color, [1.0, 0.0, 0.0]).unwrap(), [lum([1.0, 0.0, 0.0]); 3]));
        assert_eq!(Filter::Grayscale.point(ChannelKind::Scalar, [0.3; 3]), Some([0.3; 3]));
        assert_eq!(Filter::NormalInvert.point(ChannelKind::NormalMap, [0.5, 0.25, 1.0]), Some([0.5, 0.75, 1.0]));
        assert_eq!(Filter::NormalInvert.point(ChannelKind::Color, [0.5, 0.25, 1.0]), Some([0.5, 0.25, 1.0]));

        // Replace color shifts near colors while far colors and scalars stay
        let rc = Filter::ReplaceColor { from: [0.8, 0.1, 0.1], to: [0.1, 0.1, 0.8], tolerance: 0.1, softness: 0.1 };
        assert!(close(rc.point(ChannelKind::Color, [0.85, 0.1, 0.1]).unwrap(), [0.15, 0.1, 0.8]));
        assert!(close(rc.point(ChannelKind::Color, [0.1, 0.8, 0.1]).unwrap(), [0.1, 0.8, 0.1]));
        let half = rc.point(ChannelKind::Color, [0.8, 0.1 + 0.15 * 3f32.sqrt(), 0.1]).unwrap();
        assert!((half[2] - 0.45).abs() < 0.02, "halfway through the softness: {half:?}");
        assert_eq!(rc.point(ChannelKind::Scalar, [0.8; 3]), Some([0.8; 3]));
    }

    #[test]
    fn bevel_glow_and_mosaic() {
        let checker = |f: &mut Fixture| {
            let id = f.project.alloc_id();
            let mut fill = Fill::default();
            fill.sources.insert(
                Channel::Roughness,
                Source::Procedural { pattern: Procedural::new(ProceduralKind::Checker), low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() },
            );
            f.project.insert_layer(0, Layer::new(id, "F", LayerKind::Fill(fill), &[Channel::Roughness]), None, false).unwrap();
            id
        };
        let values = |f: &Fixture| {
            let out = run(f, None);
            (0..64 * 64).map(|i| out.value(Channel::Roughness, i % 64, i / 64)[0]).collect::<Vec<f32>>()
        };
        let mut f = fixture();
        let id = checker(&mut f);
        let plain = values(&f);

        // Bevel stays below the shape and turns edges into ramps
        let fx = content_fx(&mut f, id, EffectKind::Filter(Filter::Bevel { width: 128.0 }));
        let bevel = values(&f);
        let mids = |v: &[f32]| v.iter().filter(|x| **x > 0.05 && **x < 0.95).count();
        assert!(bevel.iter().zip(&plain).all(|(b, p)| *b <= p + 1e-4 && *b >= 0.0), "a bevel only lowers values");
        assert!(mids(&bevel) > mids(&plain) + 64, "{} vs {}", mids(&bevel), mids(&plain));

        // Mosaic reduces changes between adjacent texels
        f.project.layer_mut(id).unwrap().content_effects.retain(|e| e.id != fx);
        let fx = content_fx(&mut f, id, EffectKind::Filter(Filter::Mosaic { size: 768.0 }));
        let mosaic = values(&f);
        let steps = |v: &[f32]| v.windows(2).filter(|w| (w[0] - w[1]).abs() > 0.01).count();
        assert!(steps(&mosaic) < steps(&plain), "{} vs {}", steps(&mosaic), steps(&plain));
        assert!(mosaic.iter().all(|v| (0.0..=1.0).contains(v)));

        // Glow on uniform content adds its own blur
        f.project.layer_mut(id).unwrap().content_effects.retain(|e| e.id != fx);
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [0.1; 3], 0.2);
        content_fx(&mut f, id, EffectKind::Filter(Filter::Glow { radius: 32.0, amount: 1.0 }));
        assert!(close(run(&f, None).value(Channel::Roughness, 30, 30), [0.4; 3]));
    }

    #[test]
    fn baked_lighting_lights_the_base_color_by_the_normal() {
        let lit = |elevation: f32| {
            let mut f = fixture();
            let id = fill_layer(&mut f.project, [0.5; 3], 0.3);
            let fx = Filter::BakedLighting { azimuth: 0.0, elevation, light: 0.6, ambient: 0.4, occlusion: 1.0 };
            content_fx(&mut f, id, EffectKind::Filter(fx));
            let out = run(&f, None);
            (out.value(Channel::BaseColor, 30, 30), out.value(Channel::Roughness, 30, 30))
        };
        let n = primitives::plane(2.0, 2).normals[0];

        // Facing the light adds illumination; facing away keeps ambient only
        let (toward, rough) = lit(if n.y > 0.0 { 90.0 } else { -90.0 });
        assert!(close(toward, [0.5; 3]) && close(rough, [0.3; 3]), "{toward:?} {rough:?}");
        let (away, _) = lit(if n.y > 0.0 { -90.0 } else { 90.0 });
        assert!(close(away, [0.2; 3]), "{away:?}");
        let f = Filter::BakedLighting { azimuth: 0.0, elevation: 0.0, light: 0.0, ambient: 1.0, occlusion: 0.5 };
        assert_eq!(f.baked_light([0.0, 1.0, 0.0], 0.2), Some(0.6), "half the occlusion");
        assert_eq!(Filter::Invert.baked_light([0.0; 3], 1.0), None);
    }

    #[test]
    fn geometry_masks_show_layers_on_chosen_parts_only() {
        let mut f = fixture();

        // The plane's material spans both triangle groups
        let n = f.mesh.indices.len() as u32 / 6 * 3;
        f.mesh.submeshes = vec![
            texturecraft_geom::SubMesh { name: "Lid".into(), material: 0, start: 0, count: n },
            texturecraft_geom::SubMesh { name: "Body".into(), material: 0, start: n, count: f.mesh.indices.len() as u32 - n },
        ];
        let id = fill_layer(&mut f.project, [1.0, 0.0, 0.0], 0.2);
        let red = |f: &Fixture| {
            let out = run(f, None);
            (0..64 * 64).filter(|i| close(out.value(Channel::BaseColor, i % 64, i / 64), [1.0, 0.0, 0.0])).count()
        };
        let all = red(&f);
        f.project.layer_mut(id).unwrap().geometry_mask = Some(GeometryMask::Parts(vec!["Lid".into()]));
        let lid = red(&f);
        f.project.layer_mut(id).unwrap().geometry_mask = Some(GeometryMask::Parts(vec!["Body".into()]));
        let body = red(&f);
        assert!(lid > 0 && body > 0 && lid + body == all, "{lid} + {body} of {all}");
        f.project.layer_mut(id).unwrap().geometry_mask = Some(GeometryMask::Parts(Vec::new()));
        assert_eq!(red(&f), 0, "no parts: hidden everywhere");
    }

    #[test]
    fn geometry_masks_by_uv_tile_show_layers_on_chosen_atlas_blocks_only() {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [1.0, 0.0, 0.0], 0.2);
        let red = |f: &Fixture| {
            let out = run(f, None);
            (0..64 * 64).filter(|i| close(out.value(Channel::BaseColor, i % 64, i / 64), [1.0, 0.0, 0.0])).map(|i| i % 64).collect::<Vec<_>>()
        };
        let all = red(&f).len();
        assert!(all > 0);
        // A set without UV tiles is tile 1001.
        f.project.layer_mut(id).unwrap().geometry_mask = Some(GeometryMask::UvTiles(vec![1001]));
        assert_eq!(red(&f).len(), all);
        f.project.layer_mut(id).unwrap().geometry_mask = Some(GeometryMask::UvTiles(vec![1002]));
        assert!(red(&f).is_empty(), "tile 1002 is not in a classic set");
        // Two tiles side by side: the right half of the atlas is tile 1002.
        f.project.texture_sets[0].uv_tiles = vec![1001, 1002];
        let right = red(&f);
        assert!(!right.is_empty() && right.iter().all(|x| *x >= 32), "only the right block");
        f.project.layer_mut(id).unwrap().geometry_mask = Some(GeometryMask::UvTiles(vec![1001]));
        assert!(red(&f).iter().all(|x| *x < 32));
    }

    #[test]
    fn placeholder_effects_pass_content_and_masks_through() {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [0.25, 0.5, 0.75], 0.4);
        let before = run(&f, None);
        content_fx(&mut f, id, EffectKind::Placeholder { label: "Filter \"x\"".into(), reason: "not computed".into() });
        f.project.layer_mut(id).unwrap().mask =
            Some(Mask { effects: vec![Effect::placeholder(LayerId(900), "Generator \"y\"", "not computed")], ..Mask::new(MaskBackground::White) });
        let after = run(&f, None);
        for (x, y) in [(5, 5), (30, 40), (60, 10)] {
            assert_eq!(before.value(Channel::BaseColor, x, y), after.value(Channel::BaseColor, x, y));
            assert_eq!(before.value(Channel::Roughness, x, y), after.value(Channel::Roughness, x, y));
        }
    }

    #[test]
    fn content_paint_and_fill_effects_add_to_layers() {
        let mut f = fixture();
        let id = f.project.alloc_id();
        f.project.insert_layer(0, Layer::new(id, "P", LayerKind::Paint, &[Channel::BaseColor, Channel::Roughness]), None, false).unwrap();
        let mut material = Fill::default();
        material.sources.insert(Channel::Roughness, Source::Uniform([0.2; 3]));
        content_fx(&mut f, id, EffectKind::Material(material));
        let paint = content_fx(&mut f, id, EffectKind::Paint);

        // Roughness 0.9 is painted at half coverage
        f.pixels.layer_mut(paint, Channel::Roughness, (64, 64)).set(3, 3, &[to_u16(0.45), to_u16(0.5)]);
        let out = run(&f, None);
        assert!(close(out.value(Channel::Roughness, 10, 10), [0.2; 3]), "the fill effect covers the empty paint layer");
        assert!(close(out.value(Channel::Roughness, 3, 3), [0.55; 3]), "{:?}", out.value(Channel::Roughness, 3, 3));
        assert!(close(out.value(Channel::BaseColor, 3, 3), [0.5; 3]), "channels without a source stay untouched");

        // Flattening bakes in the content effects
        let images = Default::default();
        let inp = Inputs {
            project: &f.project,
            ts: 0,
            mesh: &f.mesh,
            gbuf: &f.gbuf,
            pixels: &f.pixels,
            baked: &f.baked,
            block_scales: f.project.texture_sets[0].block_scales(),
            preview: None,
            images: &images,
            graphs: crate::GraphStore::empty(),
            bounds: f.mesh.bounds(),
        };
        let flat = flatten_layer(&inp, id, &all_tiles(64, 64)).unwrap();
        assert!(flat.iter().all(|(c, _, _)| *c == Channel::Roughness));
        let (_, _, data) = flat.iter().find(|(_, t, _)| *t == (0, 0)).unwrap();
        let o = 3 * TILE as usize + 3;
        assert!((from_u16(data[o * 2]) - 0.55).abs() < 1e-3 && (from_u16(data[o * 2 + 1]) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn anchors_feed_masks_above_in_stack_order() {
        let mut f = fixture();
        let low = f.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::Height, Source::Uniform([0.4; 3]));
        f.project.insert_layer(0, Layer::new(low, "Low", LayerKind::Fill(fill), &[Channel::Height]), None, false).unwrap();
        let anchor = content_fx(&mut f, low, EffectKind::Anchor);
        let top = fill_layer(&mut f.project, [1.0; 3], 0.5);
        let mut mask = Mask::new(MaskBackground::Black);
        let source = Source::Anchor { anchor, channel: Channel::Height, ramp: Vec::new() };
        mask.effects.push(Effect::new(f.project.alloc_id(), EffectKind::Fill { source, projection: Projection::default() }));
        f.project.layer_mut(top).unwrap().mask = Some(mask.clone());
        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [0.7; 3]));

        // The field pre-pass reads anchors under blur, preserving flat height
        mask.effects.push(Effect::new(f.project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 32.0 })));
        f.project.layer_mut(top).unwrap().mask = Some(mask);
        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [0.7; 3]));

        // An anchor above its reader is not evaluated yet, so it reads the channel default
        f.project.move_layer(low, 1).unwrap();
        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [0.5; 3]));
    }

    /// A white fill layer on top whose mask reads `anchor` (any channel: a mask anchor is gray).
    fn anchor_reader(fixture: &mut Fixture, anchor: LayerId) -> LayerId {
        let top = fill_layer(&mut fixture.project, [1.0; 3], 0.5);
        let mut mask = Mask::new(MaskBackground::Black);
        let source = Source::Anchor { anchor, channel: Channel::Roughness, ramp: Vec::new() };
        mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Fill { source, projection: Projection::default() }));
        fixture.project.layer_mut(top).unwrap().mask = Some(mask);
        top
    }

    #[test]
    fn mask_anchors_record_the_mask_value_at_their_place() {
        let mut fixture = fixture();
        let low = fill_layer(&mut fixture.project, [0.0; 3], 0.5);
        let (gray, anchor, full) = (fixture.project.alloc_id(), fixture.project.alloc_id(), fixture.project.alloc_id());
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(gray, EffectKind::Fill { source: Source::Uniform([0.4; 3]), projection: Projection::default() }));
        mask.effects.push(Effect::new(anchor, EffectKind::Anchor));
        mask.effects.push(Effect::new(full, EffectKind::Fill { source: Source::Uniform([1.0; 3]), projection: Projection::default() }));
        fixture.project.layer_mut(low).unwrap().mask = Some(mask.clone());
        anchor_reader(&mut fixture, anchor);

        // The low layer shows fully (black), the top one at the 0.4 the anchor recorded
        assert!(close(run(&fixture, None).value(Channel::BaseColor, 5, 5), [0.4; 3]));

        // Under a blur precomputed over the region the anchor still records its own place
        mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 32.0 })));
        fixture.project.layer_mut(low).unwrap().mask = Some(mask.clone());
        let color = run(&fixture, None).value(Channel::BaseColor, 5, 5);
        assert!(close(color, [0.4; 3]), "{color:?}");

        // An empty paint layer still records its mask anchor for the layer above
        fixture.project.layer_mut(low).unwrap().kind = LayerKind::Paint;
        let color = run(&fixture, None).value(Channel::BaseColor, 5, 5);
        assert!(close(color, [0.7; 3]), "{color:?}");
    }

    #[test]
    fn mask_anchor_between_filters_keeps_the_filtered_value() {
        let mut fixture = fixture();
        fixture.gbuf = UvGBuffer::new(&fixture.mesh, 0, 128, 128);
        fixture.project.texture_sets[0].resolution = 128;
        let lower = fill_layer(&mut fixture.project, [0.0; 3], 1.0);
        let anchor = fixture.project.alloc_id();
        let mut mask = Mask::new(MaskBackground::Black);
        let checker = Source::Procedural { pattern: Procedural::new(ProceduralKind::Checker), low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() };
        mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Fill { source: checker, projection: Projection::default() }));
        mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 64.0 })));
        mask.effects.push(Effect::new(anchor, EffectKind::Anchor));
        fixture.project.layer_mut(lower).unwrap().mask = Some(mask.clone());
        let reader = anchor_reader(&mut fixture, anchor);
        let texels: Vec<_> = (0..128).map(|x| (x, 63)).collect();
        let expected = assert_sparse_mask_matches_image(&fixture, reader, &texels).unwrap();
        assert!(expected.iter().any(|value| *value > 0.1 && *value < 0.9), "the anchor records a softened checker");

        mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 128.0 })));
        fixture.project.layer_mut(lower).unwrap().mask = Some(mask);
        let actual = assert_sparse_mask_matches_image(&fixture, reader, &texels).unwrap();

        assert_eq!(actual, expected, "a filter above the anchor must not change its recorded value");
    }

    #[test]
    fn generator_micro_detail_follows_the_anchor_height() {
        let mut fixture = fixture();
        let low = fixture.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::Height, Source::Uniform([0.5; 3]));
        fixture.project.insert_layer(0, Layer::new(low, "Bumps", LayerKind::Fill(fill), &[Channel::Height]), None, false).unwrap();
        let anchor = content_fx(&mut fixture, low, EffectKind::Anchor);
        let top = fill_layer(&mut fixture.project, [1.0; 3], 0.5);
        let mut generator = Generator { level: 0.5, softness: 0.5, noise_amount: 0.0, detail: None, ..Generator::new(GeneratorKind::Curvature) };
        let with = |fixture: &mut Fixture, generator: &Generator| {
            let mut mask = Mask::new(MaskBackground::Black);
            mask.effects.push(Effect::new(LayerId(900), EffectKind::Generator(generator.clone())));
            fixture.project.layer_mut(top).unwrap().mask = Some(mask);
            run(fixture, None).value(Channel::BaseColor, 5, 5)[0]
        };

        // No curvature map: flat mid gray, so the mask is 0.5 and the base color 0.75
        let flat = with(&mut fixture, &generator);
        assert!((flat - 0.75).abs() < 0.02, "{flat}");

        // Raised height reads as an edge
        generator.micro = Some(MicroDetail { anchor, amount: 1.0 });
        assert!(with(&mut fixture, &generator) > flat + 0.2);

        // Recessed height reads as a cavity; dirt gathers there
        if let LayerKind::Fill(fill) = &mut fixture.project.layer_mut(low).unwrap().kind {
            fill.sources.insert(Channel::Height, Source::Uniform([-0.5; 3]));
        }
        assert!(with(&mut fixture, &generator) < flat - 0.2);
        let dirt =
            Generator { level: 0.5, softness: 0.5, noise_amount: 0.0, micro: Some(MicroDetail { anchor, amount: 1.0 }), ..Generator::new(GeneratorKind::Dirt) };
        let plain = Generator { micro: None, ..dirt.clone() };
        assert!(with(&mut fixture, &dirt) > with(&mut fixture, &plain) + 0.2);

        // A hostile amount does nothing
        generator.micro = Some(MicroDetail { anchor, amount: f32::NAN });
        assert!((with(&mut fixture, &generator) - flat).abs() < 0.02);
    }

    #[test]
    fn curvature_modes_classify_raised_and_recessed_micro_height() {
        let mut fixture = fixture();
        fixture.baked = BakedMaps { resolution: 64, maps: [(BakedMap::Curvature, Arc::new(vec![0.5; 64 * 64].into()))].into() };
        let lower = fixture.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::Height, Source::Uniform([0.5; 3]));
        fixture.project.insert_layer(0, Layer::new(lower, "Bumps", LayerKind::Fill(fill), &[Channel::Height]), None, false).unwrap();
        let anchor = content_fx(&mut fixture, lower, EffectKind::Anchor);
        let top = fill_layer(&mut fixture.project, [1.0; 3], 0.5);

        for kind in [GeneratorKind::Curvature, GeneratorKind::EdgeWear] {
            for (height, mode, responds) in [
                (0.5, CurvatureMode::Edges, true),
                (-0.5, CurvatureMode::Edges, false),
                (0.5, CurvatureMode::Cavities, false),
                (-0.5, CurvatureMode::Cavities, true),
                (0.5, CurvatureMode::Both, true),
                (-0.5, CurvatureMode::Both, true),
            ] {
                if let LayerKind::Fill(fill) = &mut fixture.project.layer_mut(lower).unwrap().kind {
                    fill.sources.insert(Channel::Height, Source::Uniform([height; 3]));
                }
                let detail = CurvatureDetail { mode, weights: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] };
                let generator = Generator {
                    level: 0.5,
                    softness: 0.5,
                    noise_amount: 0.0,
                    detail: Some(detail),
                    micro: Some(MicroDetail { anchor, amount: 1.0 }),
                    ..Generator::new(kind)
                };
                let mut mask = Mask::new(MaskBackground::Black);
                mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Generator(generator)));
                fixture.project.layer_mut(top).unwrap().mask = Some(mask);

                let value = run(&fixture, None).value(Channel::BaseColor, 5, 5)[0];
                let expected = if responds { 0.75 } else { 0.5 };
                assert!((value - expected).abs() < 0.02, "{kind:?} {mode:?} height {height}: {value}");
            }
        }
    }

    #[test]
    fn an_anchor_on_a_layer_blending_no_channel_still_records_the_stack() {
        let mut f = fixture();
        let low = f.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::Height, Source::Uniform([0.4; 3]));
        f.project.insert_layer(0, Layer::new(low, "Low", LayerKind::Fill(fill), &[Channel::Height]), None, false).unwrap();
        // an imported anchor layer: an empty paint layer with no channel, holding only the anchor point
        let holder = f.project.alloc_id();
        f.project.insert_layer(0, Layer::new(holder, "Anchor", LayerKind::Paint, &[]), None, false).unwrap();
        let anchor = content_fx(&mut f, holder, EffectKind::Anchor);
        let top = fill_layer(&mut f.project, [1.0; 3], 0.5);
        let mut mask = Mask::new(MaskBackground::Black);
        let source = Source::Anchor { anchor, channel: Channel::Height, ramp: Vec::new() };
        mask.effects.push(Effect::new(f.project.alloc_id(), EffectKind::Fill { source, projection: Projection::default() }));
        f.project.layer_mut(top).unwrap().mask = Some(mask);
        // 1.0 over 0.5 by the recorded 0.4 (not the channel default)
        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [0.7; 3]));
    }

    #[test]
    fn anchors_inside_masks_record_the_mask_below_them() {
        let mut f = fixture();
        fill_layer(&mut f.project, [0.0; 3], 0.0);
        let low = fill_layer(&mut f.project, [1.0; 3], 0.0);
        let anchor = f.project.alloc_id();
        let uniform = |f: &mut Fixture, v: f32| {
            Effect::new(f.project.alloc_id(), EffectKind::Fill { source: Source::Uniform([v; 3]), projection: Projection::default() })
        };
        let read = |f: &mut Fixture| {
            let source = Source::Anchor { anchor, channel: Channel::Height, ramp: Vec::new() };
            Effect::new(f.project.alloc_id(), EffectKind::Fill { source, projection: Projection::default() })
        };
        // 0.4, recorded, covered by 0, then read back: 0.4
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects = vec![uniform(&mut f, 0.4), Effect::new(anchor, EffectKind::Anchor), uniform(&mut f, 0.0), read(&mut f)];
        f.project.layer_mut(low).unwrap().mask = Some(mask.clone());
        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [0.4; 3]));
        // A neighbourhood filter between the anchor and its reader: the field carries what the anchor recorded
        let blur = Effect::new(f.project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 32.0 }));
        mask.effects.insert(3, blur);
        f.project.layer_mut(low).unwrap().mask = Some(mask);
        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [0.4; 3]));
        // A layer above reads it too: 0.5 over 0.4 at 0.4
        let top = fill_layer(&mut f.project, [0.5; 3], 0.0);
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects = vec![read(&mut f)];
        f.project.layer_mut(top).unwrap().mask = Some(mask);
        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [0.44; 3]));
    }

    #[test]
    fn anchors_inside_hidden_layers_masks_still_record() {
        let mut f = fixture();
        f.project.rules = CompositeRules::measured(HeightNormal::Native);
        fill_layer(&mut f.project, [0.0; 3], 0.0);
        // a hidden layer whose mask records 0.4: it draws nothing, but its anchor is evaluated
        let hidden = fill_layer(&mut f.project, [1.0; 3], 0.0);
        let anchor = f.project.alloc_id();
        let mut mask = Mask::new(MaskBackground::Black);
        let uniform = Effect::new(f.project.alloc_id(), EffectKind::Fill { source: Source::Uniform([0.4; 3]), projection: Projection::default() });
        mask.effects = vec![uniform, Effect::new(anchor, EffectKind::Anchor)];
        let l = f.project.layer_mut(hidden).unwrap();
        l.mask = Some(mask);
        l.visible = false;
        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [0.0; 3]), "the hidden layer draws nothing");
        // a layer above reads it: 0.5 at 0.4
        let top = fill_layer(&mut f.project, [0.5; 3], 0.0);
        let mut mask = Mask::new(MaskBackground::Black);
        let source = Source::Anchor { anchor, channel: Channel::Height, ramp: Vec::new() };
        mask.effects = vec![Effect::new(f.project.alloc_id(), EffectKind::Fill { source, projection: Projection::default() })];
        f.project.layer_mut(top).unwrap().mask = Some(mask);
        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [0.2; 3]));
    }

    #[test]
    fn graphs_read_anchors_inside_masks_as_the_masks_grey() {
        use crate::graphs::tests::INVERT;
        let mut graphs = GraphStore::default();
        let invert = graphs.add_text(INVERT);
        let reader = |anchor: LayerId, channel: Channel| {
            let mut g = GraphRef::new(invert.clone(), "invert");
            g.inputs.insert(0, GraphInput::Anchor { anchor, channel: Some(channel), levels: None });
            EffectKind::Graph { graph: g, projection: Projection::default() }
        };
        let uniform = |f: &mut Fixture, v: f32| {
            Effect::new(f.project.alloc_id(), EffectKind::Fill { source: Source::Uniform([v; 3]), projection: Projection::default() })
        };
        // a mask records 0.3 at its anchor; the mask of a layer above runs a graph on it: its grey, not a height
        let mut f = fixture();
        fill_layer(&mut f.project, [0.0; 3], 0.0);
        let low = fill_layer(&mut f.project, [1.0; 3], 0.0);
        let anchor = f.project.alloc_id();
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects = vec![uniform(&mut f, 0.3), Effect::new(anchor, EffectKind::Anchor)];
        f.project.layer_mut(low).unwrap().mask = Some(mask);
        let top = fill_layer(&mut f.project, [1.0; 3], 0.0);
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects = vec![Effect::new(f.project.alloc_id(), reader(anchor, Channel::Height))];
        f.project.layer_mut(top).unwrap().mask = Some(mask);
        let v = run_with(&f, &graphs).value(Channel::BaseColor, 5, 5)[0];
        assert!((v - (0.3 * 0.3 + 0.7)).abs() < 2e-3, "1 over 0.3 through the inverted mask 0.7: {v}");

        // content reading an anchor in its own layer's mask: the mask is evaluated first
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [0.0; 3], 0.0);
        let anchor = f.project.alloc_id();
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects = vec![uniform(&mut f, 0.3), Effect::new(anchor, EffectKind::Anchor), uniform(&mut f, 1.0)];
        let fx = f.project.alloc_id();
        let l = f.project.layer_mut(id).unwrap();
        l.mask = Some(mask);
        l.content_effects.push(Effect::new(fx, reader(anchor, Channel::BaseColor)));
        let v = run_with(&f, &graphs).value(Channel::BaseColor, 5, 5)[0];
        let want = texturecraft_texel::srgb_to_linear(0.7);
        assert!((v - want).abs() < 2e-3, "the content is the inverted grey 0.3: {v} (want {want})");
    }

    #[test]
    fn decal_fills_cover_only_their_frame_and_image_alpha() {
        let mut f = fixture();
        let s = f.gbuf.surface(&f.mesh, 32 * 64 + 32).unwrap();
        let d = texturecraft_core::Decal {
            center: s.pos.to_array(),
            normal: s.nrm.to_array(),
            up: s.nrm.any_orthonormal_vector().to_array(),
            size: [1.0, 1.0],
            rotation: 0.0,
            offset: [0.0; 2],
            depth: 0.5,
            symmetry: None,
            flip: [false; 2],
        };
        let id = f.project.alloc_id();
        let mut fill = Fill { projection: Projection::Decal(d), ..Fill::default() };
        fill.sources.insert(Channel::BaseColor, Source::image("decal"));
        f.project.insert_layer(0, Layer::new(id, "Decal", LayerKind::Fill(fill), &[Channel::BaseColor]), None, false).unwrap();
        let composite = |alpha: f32| {
            let img = crate::store::ImageRes { width: 2, height: 2, rgba: vec![[1.0, 0.0, 0.0, alpha]; 4], srgb: false };
            let images: ImageStore = [("decal".to_string(), std::sync::Arc::new(img))].into_iter().collect();
            let inp = Inputs {
                project: &f.project,
                ts: 0,
                mesh: &f.mesh,
                gbuf: &f.gbuf,
                pixels: &f.pixels,
                baked: &f.baked,
                block_scales: f.project.texture_sets[0].block_scales(),
                preview: None,
                images: &images,
                graphs: crate::GraphStore::empty(),
                bounds: f.mesh.bounds(),
            };
            let mut out = Composite::new(&f.project.texture_sets[0].channel_list(), 64);
            composite_tiles(&inp, &mut out, &all_tiles(64, 64));
            out
        };
        let out = composite(1.0);
        assert!(close(out.value(Channel::BaseColor, 32, 32), [1.0, 0.0, 0.0]), "{:?}", out.value(Channel::BaseColor, 32, 32));
        assert!(close(out.value(Channel::BaseColor, 2, 2), [0.5; 3]), "outside the frame");
        let half = composite(0.5);
        assert!(close(half.value(Channel::BaseColor, 32, 32), [0.75, 0.25, 0.25]), "the image alpha shapes the decal");
    }

    /// Composites a cube with a decal of a four-colored image, and checks each texel of the
    /// faces picked by `expect` against the image at the coordinates it gives (None: the texel
    /// is not part of the check). Returns how many texels were checked.
    fn cube_decal_texels(d: texturecraft_core::Decal, expect: impl Fn(Vec3, Vec3) -> Option<Option<(f32, f32)>>) -> usize {
        let mesh = primitives::cube(2.0);
        let gbuf = UvGBuffer::new(&mesh, 0, 64, 64);
        let mut project = Project::new("t", "builtin:cube");
        project.texture_sets.push(TextureSet::new("Cube", 0, 64));
        let id = project.alloc_id();
        let mut fill = Fill { projection: Projection::Decal(d), ..Fill::default() };
        fill.sources.insert(Channel::BaseColor, Source::image("decal"));
        project.insert_layer(0, Layer::new(id, "Decal", LayerKind::Fill(fill), &[Channel::BaseColor]), None, false).unwrap();

        // Top left red, top right blue, bottom left green, bottom right yellow
        let img = crate::store::ImageRes {
            width: 2,
            height: 2,
            rgba: vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [0.0, 1.0, 0.0, 1.0], [1.0, 1.0, 0.0, 1.0]],
            srgb: false,
        };
        let images: ImageStore = [("decal".to_string(), std::sync::Arc::new(img.clone()))].into_iter().collect();
        let pixels = PixelStore::default();
        let baked = BakedMaps::default();
        let inp = Inputs {
            project: &project,
            ts: 0,
            mesh: &mesh,
            gbuf: &gbuf,
            pixels: &pixels,
            baked: &baked,
            block_scales: project.texture_sets[0].block_scales(),
            preview: None,
            images: &images,
            graphs: GraphStore::empty(),
            bounds: mesh.bounds(),
        };
        let mut out = Composite::new(&project.texture_sets[0].channel_list(), 64);
        composite_tiles(&inp, &mut out, &all_tiles(64, 64));

        let mut checked = 0;
        for i in 0..64 * 64 {
            let Some(surface) = gbuf.surface(&mesh, i) else { continue };
            let Some(want) = expect(surface.pos, surface.nrm) else { continue };
            let got = out.value(Channel::BaseColor, (i % 64) as u32, (i / 64) as u32);
            let want = want.map_or([0.5; 3], |(u, v)| {
                let c = img.sample(u, v);
                [c[0], c[1], c[2]]
            });
            assert!(close(got, want), "texel at {:?} on a face turned to {:?}: {got:?}, expected {want:?}", surface.pos, surface.nrm);
            checked += 1;
        }
        checked
    }

    /// A 1 by 1 decal on the cube's +X face (the image's right runs toward -Z, its up toward +Y)
    fn cube_decal(center: [f32; 3], size: f32, symmetry: Option<texturecraft_core::DecalSymmetry>) -> texturecraft_core::Decal {
        texturecraft_core::Decal {
            center,
            normal: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            size: [size; 2],
            rotation: 0.0,
            offset: [0.0; 2],
            depth: 0.5,
            symmetry,
            flip: [false; 2],
        }
    }

    /// What the decal of `size` centered at (cy, cz) on the +X face gives the point (y, z) of
    /// that face: the image coordinates well inside the frame, None for a bare texel clearly
    /// outside it, and None for texels near its edge (not part of the checks).
    fn face_expect(y: f32, z: f32, (cy, cz): (f32, f32), size: f32) -> Option<Option<(f32, f32)>> {
        let (u, v) = (0.5 - (z - cz) / size, 0.5 - (y - cy) / size);
        let off = (u - 0.5).abs().max((v - 0.5).abs());
        if off < 0.4 {
            Some(Some((u, v)))
        } else if off > 0.6 {
            Some(None)
        } else {
            None
        }
    }

    #[test]
    fn a_mirrored_decal_lands_on_the_opposite_cube_face_with_its_flip() {
        use texturecraft_core::{DecalFlip, DecalSymmetry};
        let at = (0.1, -0.2);
        let on_x = |flip| {
            cube_decal_texels(cube_decal([1.0, at.0, at.1], 1.0, Some(DecalSymmetry { axis: 0, flip, offset: 0.0 })), |p, n| {
                if n.x > 0.99 {
                    face_expect(p.y, p.z, at, 1.0)
                } else if n.x < -0.99 {
                    // The mirror image: the point's reflection, then the flip of the copy
                    face_expect(p.y, p.z, at, 1.0).map(|texel| {
                        texel.map(|(u, v)| match flip {
                            DecalFlip::None => (u, v),
                            DecalFlip::U => (1.0 - u, v),
                            DecalFlip::V => (u, 1.0 - v),
                        })
                    })
                } else {
                    Some(None)
                }
            })
        };
        for flip in [DecalFlip::None, DecalFlip::U, DecalFlip::V] {
            assert!(on_x(flip) > 800, "{flip:?}: most of the cube was checked");
        }

        // No symmetry: the -X face stays bare
        let bare = cube_decal_texels(cube_decal([1.0, at.0, at.1], 1.0, None), |p, n| if n.x > 0.99 { face_expect(p.y, p.z, at, 1.0) } else { Some(None) });
        assert!(bare > 800);

        // Mirrored across Z, the copy sits on the same face on the other side of the Z = 0 plane
        let at = (0.1, -0.5);
        let across_z = Some(DecalSymmetry { axis: 2, flip: DecalFlip::None, offset: 0.0 });
        let on_z = cube_decal_texels(cube_decal([1.0, at.0, at.1], 0.4, across_z), |p, n| {
            if n.x > 0.99 {
                match (face_expect(p.y, p.z, at, 0.4), face_expect(p.y, -p.z, at, 0.4)) {
                    (Some(Some(texel)), _) | (_, Some(Some(texel))) => Some(Some(texel)),
                    (Some(None), Some(None)) => Some(None),
                    _ => None,
                }
            } else {
                Some(None)
            }
        });
        assert!(on_z > 800);
    }

    #[test]
    fn the_mirror_plane_can_be_moved_along_its_axis() {
        use texturecraft_core::{DecalFlip, DecalSymmetry};

        // Mirrored across Z with the plane at z = 0.3: a decal at z = -0.2 gets its copy at z = 0.8
        let at = (0.1, -0.2);
        let moved = Some(DecalSymmetry { axis: 2, flip: DecalFlip::None, offset: 0.3 });
        let checked = cube_decal_texels(cube_decal([1.0, at.0, at.1], 0.4, moved), |p, n| {
            if n.x > 0.99 {
                match (face_expect(p.y, p.z, at, 0.4), face_expect(p.y, 0.6 - p.z, at, 0.4)) {
                    (Some(Some(texel)), _) | (_, Some(Some(texel))) => Some(Some(texel)),
                    (Some(None), Some(None)) => Some(None),
                    _ => None,
                }
            } else {
                Some(None)
            }
        });
        assert!(checked > 800);
    }

    #[test]
    fn moving_the_decal_moves_its_mirrored_copy() {
        use texturecraft_core::{DecalFlip, DecalSymmetry};
        let symmetry = Some(DecalSymmetry { axis: 0, flip: DecalFlip::None, offset: 0.0 });
        for at in [(0.5, 0.3), (-0.4, -0.5)] {
            let checked =
                cube_decal_texels(
                    cube_decal([1.0, at.0, at.1], 0.4, symmetry),
                    |p, n| {
                        if n.x.abs() > 0.99 { face_expect(p.y, p.z, at, 0.4) } else { Some(None) }
                    },
                );
            assert!(checked > 800, "{at:?}");
        }
    }

    #[test]
    fn flipping_a_decal_mirrors_its_image_inside_the_frame() {
        let mut f = fixture();
        let s = f.gbuf.surface(&f.mesh, 32 * 64 + 32).unwrap();

        // The frame's center sits a quarter of its width to the right of the texel, so the texel reads the image's left half
        let d = texturecraft_core::Decal {
            center: s.pos.to_array(),
            normal: s.nrm.to_array(),
            up: s.nrm.any_orthonormal_vector().to_array(),
            size: [1.0, 1.0],
            rotation: 0.0,
            offset: [0.25, 0.0],
            depth: 0.5,
            symmetry: None,
            flip: [false; 2],
        };
        let id = f.project.alloc_id();
        let mut fill = Fill { projection: Projection::Decal(d), ..Fill::default() };
        fill.sources.insert(Channel::BaseColor, Source::image("decal"));
        f.project.insert_layer(0, Layer::new(id, "Decal", LayerKind::Fill(fill), &[Channel::BaseColor]), None, false).unwrap();

        // Left half red, right half blue
        let img = crate::store::ImageRes { width: 2, height: 1, rgba: vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]], srgb: false };
        let images: ImageStore = [("decal".to_string(), std::sync::Arc::new(img))].into_iter().collect();
        let texel = |f: &Fixture, flip: [bool; 2]| {
            let mut project = f.project.clone();
            if let Ok(layer) = project.layer_mut(id)
                && let LayerKind::Fill(fill) = &mut layer.kind
            {
                fill.projection = Projection::Decal(texturecraft_core::Decal { flip, ..d });
            }
            let inp = Inputs {
                project: &project,
                ts: 0,
                mesh: &f.mesh,
                gbuf: &f.gbuf,
                pixels: &f.pixels,
                baked: &f.baked,
                block_scales: project.texture_sets[0].block_scales(),
                preview: None,
                images: &images,
                graphs: GraphStore::empty(),
                bounds: f.mesh.bounds(),
            };
            let mut out = Composite::new(&project.texture_sets[0].channel_list(), 64);
            composite_tiles(&inp, &mut out, &all_tiles(64, 64));
            out.value(Channel::BaseColor, 32, 32)
        };
        assert!(close(texel(&f, [false, false]), [1.0, 0.0, 0.0]), "the left half shows red");
        assert!(close(texel(&f, [true, false]), [0.0, 0.0, 1.0]), "Flip U shows the right half there");
        assert!(close(texel(&f, [false, true]), [1.0, 0.0, 0.0]), "Flip V leaves the left half");
    }

    #[test]
    fn a_fill_image_covers_by_its_alpha() {
        let mut f = fixture();
        f.project.rules = CompositeRules::measured(HeightNormal::Native);
        fill_layer(&mut f.project, [0.2; 3], 1.0);
        let id = f.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::BaseColor, Source::image("img"));
        f.project.insert_layer(0, Layer::new(id, "Image", LayerKind::Fill(fill), &[Channel::BaseColor]), None, false).unwrap();
        let composite = |alpha: f32| {
            let img = crate::store::ImageRes { width: 2, height: 2, rgba: vec![[1.0, 0.0, 0.0, alpha]; 4], srgb: false };
            let images: ImageStore = [("img".to_string(), std::sync::Arc::new(img))].into_iter().collect();
            let inp = Inputs {
                project: &f.project,
                ts: 0,
                mesh: &f.mesh,
                gbuf: &f.gbuf,
                pixels: &f.pixels,
                baked: &f.baked,
                block_scales: f.project.texture_sets[0].block_scales(),
                preview: None,
                images: &images,
                graphs: crate::GraphStore::empty(),
                bounds: f.mesh.bounds(),
            };
            let mut out = Composite::new(&f.project.texture_sets[0].channel_list(), 64);
            composite_tiles(&inp, &mut out, &all_tiles(64, 64));
            out.value(Channel::BaseColor, 9, 9)
        };
        assert!(close(composite(1.0), [1.0, 0.0, 0.0]), "{:?}", composite(1.0));
        assert!(close(composite(0.0), [0.2; 3]), "a transparent image leaves what is below: {:?}", composite(0.0));
        assert!(close(composite(0.5), [0.6, 0.1, 0.1]), "{:?}", composite(0.5));
    }

    #[test]
    fn anchors_read_colors_as_gray_in_fill_layers() {
        let mut f = fixture();
        let low = fill_layer(&mut f.project, [0.2, 0.4, 0.6], 0.5);
        let anchor = content_fx(&mut f, low, EffectKind::Anchor);
        let id = f.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::Roughness, Source::Anchor { anchor, channel: Channel::BaseColor, ramp: Vec::new() });
        f.project.insert_layer(0, Layer::new(id, "Top", LayerKind::Fill(fill), &[Channel::Roughness]), None, false).unwrap();
        let expect = texturecraft_core::lum([0.2, 0.4, 0.6]);
        assert!(close(run(&f, None).value(Channel::Roughness, 9, 9), [expect; 3]));
    }

    const RAMP: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    #[test]
    fn an_anchor_gradient_map_recolors_the_stack_below_by_its_lightness() {
        // The stops at 0, 0.5 and 1, and halfway between them
        for (gray, want) in [(0.0, [1.0, 0.0, 0.0]), (0.25, [0.5, 0.5, 0.0]), (0.5, [0.0, 1.0, 0.0]), (0.75, [0.0, 0.5, 0.5]), (1.0, [0.0, 0.0, 1.0])] {
            let mut f = fixture();
            let low = fill_layer(&mut f.project, [gray; 3], 0.5);
            let anchor = content_fx(&mut f, low, EffectKind::Anchor);
            let id = f.project.alloc_id();
            let mut fill = Fill::default();
            fill.sources.insert(Channel::BaseColor, Source::Anchor { anchor, channel: Channel::BaseColor, ramp: RAMP.to_vec() });

            // A scalar channel reading the same anchor with stops keeps reading its gray
            fill.sources.insert(Channel::Roughness, Source::Anchor { anchor, channel: Channel::BaseColor, ramp: RAMP.to_vec() });
            fill.sources.insert(Channel::Metallic, Source::Uniform([0.3; 3]));
            let channels = [Channel::BaseColor, Channel::Roughness, Channel::Metallic];
            f.project.insert_layer(0, Layer::new(id, "Top", LayerKind::Fill(fill), &channels), None, false).unwrap();
            let out = run(&f, None);
            assert!(close(out.value(Channel::BaseColor, 9, 9), want), "gray {gray}: {:?}", out.value(Channel::BaseColor, 9, 9));
            assert!(close(out.value(Channel::Roughness, 9, 9), [gray; 3]), "gray {gray}: {:?}", out.value(Channel::Roughness, 9, 9));
            assert!(close(out.value(Channel::Metallic, 9, 9), [0.3; 3]));
        }
    }

    #[test]
    fn an_anchor_gradient_map_reads_signed_height_from_minus_one_to_one() {
        for (height, want) in [(-1.0, [1.0, 0.0, 0.0]), (0.0, [0.0, 1.0, 0.0]), (0.5, [0.0, 0.5, 0.5])] {
            let mut f = fixture();
            let low = f.project.alloc_id();
            let mut below = Fill::default();
            below.sources.insert(Channel::Height, Source::Uniform([height; 3]));
            f.project.insert_layer(0, Layer::new(low, "Below", LayerKind::Fill(below), &[Channel::Height]), None, false).unwrap();
            let anchor = content_fx(&mut f, low, EffectKind::Anchor);
            let id = f.project.alloc_id();
            let mut fill = Fill::default();
            fill.sources.insert(Channel::BaseColor, Source::Anchor { anchor, channel: Channel::Height, ramp: RAMP.to_vec() });
            f.project.insert_layer(0, Layer::new(id, "Top", LayerKind::Fill(fill), &[Channel::BaseColor]), None, false).unwrap();
            let out = run(&f, None);
            assert!(close(out.value(Channel::BaseColor, 9, 9), want), "height {height}: {:?}", out.value(Channel::BaseColor, 9, 9));
        }
    }

    #[test]
    fn a_gradient_map_reads_a_mask_anchors_gray_from_zero_to_one_in_every_channel() {
        for channel in [Channel::BaseColor, Channel::Height] {
            for (gray, want) in [(0.0, [1.0, 0.0, 0.0]), (0.25, [0.5, 0.5, 0.0]), (0.5, [0.0, 1.0, 0.0]), (1.0, [0.0, 0.0, 1.0])] {
                let mut f = fixture();
                let low = fill_layer(&mut f.project, [0.0; 3], 0.5);
                let anchor = f.project.alloc_id();
                let mut mask = Mask::new(MaskBackground::Black);
                mask.effects
                    .push(Effect::new(f.project.alloc_id(), EffectKind::Fill { source: Source::Uniform([gray; 3]), projection: Projection::default() }));
                mask.effects.push(Effect::new(anchor, EffectKind::Anchor));
                f.project.layer_mut(low).unwrap().mask = Some(mask);
                let id = f.project.alloc_id();
                let mut fill = Fill::default();
                fill.sources.insert(Channel::BaseColor, Source::Anchor { anchor, channel, ramp: RAMP.to_vec() });
                f.project.insert_layer(0, Layer::new(id, "Top", LayerKind::Fill(fill), &[Channel::BaseColor]), None, false).unwrap();
                let out = run(&f, None);
                assert!(close(out.value(Channel::BaseColor, 9, 9), want), "{channel:?}, gray {gray}: {:?}", out.value(Channel::BaseColor, 9, 9));
            }
        }
    }

    #[test]
    fn a_procedural_gradient_map_replaces_low_and_high_and_no_stops_keep_them() {
        let (low, high) = ([0.2, 0.3, 0.4], [0.8, 0.7, 0.6]);
        let composite = |ramp: Vec<[f32; 3]>, lo: [f32; 3], hi: [f32; 3]| {
            let mut f = fixture();
            let id = f.project.alloc_id();
            let mut fill = Fill::default();
            let pattern = Procedural::new(ProceduralKind::Gradient);
            fill.sources.insert(Channel::BaseColor, Source::Procedural { pattern: pattern.clone(), low: lo, high: hi, ramp: ramp.clone() });
            fill.sources.insert(Channel::Roughness, Source::Procedural { pattern, low: lo, high: hi, ramp });
            f.project.insert_layer(0, Layer::new(id, "Fill", LayerKind::Fill(fill), &[Channel::BaseColor, Channel::Roughness]), None, false).unwrap();
            let out = run(&f, None);
            (0..64).map(|x| (out.value(Channel::BaseColor, x, 20), out.value(Channel::Roughness, x, 20))).collect::<Vec<_>>()
        };

        // The pattern's own 0..1 values, then the same pattern through low and high and through the stops
        let gray = composite(Vec::new(), [0.0; 3], [1.0; 3]);
        let plain = composite(Vec::new(), low, high);
        let mapped = composite(RAMP.to_vec(), low, high);
        let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3);
        assert!(gray.iter().any(|(v, _)| v[0] < 0.25) && gray.iter().any(|(v, _)| v[0] > 0.75), "the gradient spans dark to light");
        for (((t, _), (p, _)), (m, rough)) in gray.iter().zip(&plain).zip(&mapped) {
            assert!(near(*p, mix3(low, high, t[0])), "no stops: low to high");
            assert!(near(*m, texturecraft_core::gradient(&RAMP, t[0]).unwrap()), "{t:?} -> {m:?}");
            assert!((rough[0] - mix3(low, high, t[0])[0]).abs() < 1e-3, "roughness ignores the stops: {rough:?}");
        }
    }

    /// A placed projection on the 2 × 2 plane (the scene unit is 2): `projection` on a fill of roughness `rough`.
    fn placed_roughness(projection: texturecraft_core::PlacedProjection, source: Source, graphs: &GraphStore, at: (u32, u32)) -> f32 {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [0.0; 3], 0.0);
        if let Some(LayerKind::Fill(fill)) = f.project.layers.get_mut(&id).map(|l| &mut l.kind) {
            fill.sources.insert(Channel::Roughness, source);
            fill.projection = Projection::Placed(Box::new(projection));
        }
        run_with(&f, graphs).value(Channel::Roughness, at.0, at.1)[0]
    }

    #[test]
    fn placed_tri_planar_prefilters_each_texel_unless_unfiltered() {
        use texturecraft_core::{ImagePlacement, PlacedMode, PlacedProjection, Repeat, Volume};
        // white on the left half of the image, black on the right
        let halves = "format 3\nsize 64\nsubgraph c\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_swizzle a=@2 mask=\"x\"\nnode 4 f_float v=0.5\nnode 5 f_lt a=@3 b=@4\nnode 1 f_out in=@5\nend\nnode 1 pixel_processor fn=\"c\"\noutput out 1\n";
        let mut graphs = GraphStore::default();
        let key = graphs.add_text(halves);
        let read = |unfiltered: bool| {
            let mut g = GraphRef::new(key.clone(), "halves");
            g.unfiltered = unfiltered;
            // the image once per texel (64 repetitions over the plane), each texel's centre a quarter into the white half
            let image = ImagePlacement { tiling: [64.0, 64.0], rotation: 0.0, offset: [-0.25 / 64.0, 0.0] };
            let p = PlacedProjection { mode: PlacedMode::TriPlanar { hardness: 0.5 }, volume: Volume::default(), image, repeat: Repeat::Both };
            placed_roughness(p, Source::Graph(Box::new(g)), &graphs, (10, 10))
        };
        let filtered = read(false);
        assert!((filtered - 0.5).abs() < 0.03, "a texel spanning the whole image reads its mean: {filtered}");
        let unfiltered = read(true);
        assert!(unfiltered > 0.98, "bilinear at the texel's centre: {unfiltered}");
    }

    #[test]
    fn placed_projections_cover_only_their_volume_and_image() {
        use texturecraft_core::{FacingFalloff, Falloff, ImagePlacement, PlacedMode, PlacedProjection, Repeat, Volume};
        let graphs = GraphStore::default();
        // a planar volume a quarter of the scene wide (half a unit of the plane's 2), clipped: texel 32 is inside
        // (x ≈ 0), texel 60 outside (x ≈ 0.9); outside the empty stack's roughness shows
        let planar = PlacedProjection {
            mode: PlacedMode::Planar { depth: Falloff::default(), facing: FacingFalloff::default() },
            volume: Volume { size: [0.25; 3], clip: true, ..Volume::default() },
            image: ImagePlacement::default(),
            repeat: Repeat::Both,
        };
        let empty = run(&fixture(), None).value(Channel::Roughness, 60, 32)[0];
        assert!((placed_roughness(planar.clone(), Source::Uniform([0.9; 3]), &graphs, (32, 32)) - 0.9).abs() < 1e-3);
        assert!((placed_roughness(planar, Source::Uniform([0.9; 3]), &graphs, (60, 32)) - empty).abs() < 1e-3);
        // a UV image placed twice as large without repeat covers the UV square's first half only
        let once = PlacedProjection {
            mode: PlacedMode::Uv { scale: [2.0, 2.0], rotation: 0.0, offset: [0.0, 0.0] },
            volume: Volume::default(),
            image: ImagePlacement::default(),
            repeat: Repeat::None,
        };
        assert!((placed_roughness(once.clone(), Source::Uniform([0.9; 3]), &graphs, (10, 10)) - 0.9).abs() < 1e-3);
        assert!((placed_roughness(once.clone(), Source::Uniform([0.9; 3]), &graphs, (50, 10)) - empty).abs() < 1e-3);
        let along_u = PlacedProjection { repeat: Repeat::U, ..once };
        assert!((placed_roughness(along_u, Source::Uniform([0.9; 3]), &graphs, (50, 10)) - 0.9).abs() < 1e-3);
    }
}
