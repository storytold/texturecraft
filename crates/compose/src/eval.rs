//! The layer-stack evaluator: composites a texture set's stack into one image per channel,
//! tile by tile in parallel. This is the reference implementation (the oracle) for the viewport
//! and for export.

mod draw_placement;
mod evaluation_inputs;
pub(crate) mod thumbnail;

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use rayon::prelude::*;
use texturecraft_core::{
    AnisotropicKuwahara, BakedMap, Bevel, BevelSmooth, BlendMode, BlockScales, Channel, ChannelKind, ColorMatch, CompareSource, ContentsOffset,
    CurvatureDetail, CurvatureMode, Effect, EffectKind, FLAT_CURVATURE, Fill, Filter, Filtering, Generator, GeneratorGlobal, GeneratorKind,
    GrayscaleConversionParameters, HeightMask, HeightToNormal, HighpassSpace, ImageInput, KindControls, Layer, LayerId, LayerKind, Mask, MaskBackground,
    MaskOutline, Mirror, NormalFormat, PreparedColorMatch, PreparedGradient, Project, Projection, RepeatMode, Sampling, SectionParameters, ShapeCrop, Source,
    SourceColorMode, SourceSlopeBlur, SourceWarp, StylizedShading, TargetColorMode, TextureSet, TileGrid, TileTransform, WarpSourceMode, WearParameters,
    blend1, blend3, lum, mix3, per_unit, posterize, udim_at,
};
use texturecraft_geom::glam::{Vec2, Vec3};
use texturecraft_geom::{Mesh, NO_TRI, Surface, UvGBuffer};
use texturecraft_texel::{DilationMap, TILE, TILE_AREA, TileImage, from_u16, noise, to_u16, to_u16_signed};

use crate::ProjectorDepthCache;
use crate::below::{Band, BelowCache, Level, StackKey};
use crate::curvature_sizes::{CurvatureSizes, LEVEL_RADII};
use crate::dripping_rust;
use crate::effect_histogram::EffectHistogram;
use crate::environment_sh::EnvironmentSh;
use crate::fill_copies::FillCopies;
use crate::fill_copy::FillCopy;
use crate::fill_depth::FillDepth;
use crate::fill_place::FillPlace;
use crate::generator_maps::GeneratorMaps;
use crate::held_fields::HeldFields;
use crate::mask_gray::MaskGray;
use crate::micro_detail::AnchorHeights;
use crate::pattern::{Coords, image_samples, mesh_frame, pattern_pixels, placement, procedural_in, world_noise};
use crate::pattern_context::PatternContext;
use crate::projector_cover::{framed_cover, projector_cover, projector_covers_all};
use crate::sections;
use crate::shape_distance::distance_filter;
use crate::slope_texel::slope_texel;
use crate::store::{BakedMaps, ImageRes, ImageStore, PixelStore, decode};
use crate::texture_layer;
use crate::three_d_distance;
use crate::uv_border_distance;
use crate::warp_texel::{Input, MIN_READ_WEIGHT, Sample, warp_texel};
use crate::wear;
use crate::wear_inputs::WearInputs;
use draw_placement::DrawPlacement;
use evaluation_inputs::EvaluationInputs;

const NCH: usize = Channel::ALL.len();

/// Coverage past which a symmetric fill's remaining copies would not show under the ones above
const COVERED: f32 = 0.9999;

/// Largest projected step as a share of the image's longer side for non-UV projections
const SEAM_JUMP_SHARE: f32 = 0.25;

/// Narrowest extent of the mesh's bounds a position is normalised across
const MIN_BOUNDS_EXTENT: f32 = 1e-6;

/// Stack values being composited, per channel.
type Acc = [[f32; 3]; NCH];

/// How tall a baked height of 1 stands, as a fraction of a UV tile, when Baked Lighting Stylized tilts normals by its slope
const HEIGHT_RELIEF: f32 = 1.0 / 32.0;

/// The Height Intensity of Baked Lighting Environment that tilts the normals as Baked Lighting Stylized's Height to Normal Intensity 1 does (its default)
const ENVIRONMENT_HEIGHT_UNIT: f32 = 4.0;

/// What a live stroke paints into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// Gray painted into a mask
    pub mask_gray: MaskGray,
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

    /// Mesh UV units per atlas texel along each axis, within the tile containing `(x, y)`.
    fn mesh_uv_step(&self, x: u32, y: u32) -> Vec2 {
        let atlas_step = Vec2::new(self.gbuf.width.max(1) as f32, self.gbuf.height.max(1) as f32).recip();
        let Ok(set) = self.project.texture_set(self.ts) else { return atlas_step };
        let Some(grid) = set.tile_grid() else { return atlas_step };

        let grid_step = atlas_step * Vec2::new(grid.cols as f32, grid.rows as f32);
        if set.tile_resolutions.is_empty() {
            return grid_step;
        }

        // Locate the atlas cell before undoing the tile scale
        let atlas_uv = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) * atlas_step;
        let tile_scale = udim_at(grid.from_atlas(atlas_uv.to_array())).map_or(1.0, |tile| set.tile_scale(tile));
        grid_step / tile_scale
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
}

enum Content<'a> {
    Paint {
        preview: bool,
    },
    /// A fill (with the Scale and Rotation of the folders around it laid on)
    Fill {
        fill: &'a Fill,
        projection: Option<Box<Projection>>,
        /// Its projector's depth map when it culls by depth
        depth: Option<Arc<FillDepth>>,
        /// Whether it covers every texel (see [`covers_every_texel`]), so no texel asks
        covers_all: bool,
    },
    Group(Vec<Node<'a>>),
}

struct EffectPlan<'a> {
    /// The effect id (an anchor point records under it).
    id: LayerId,
    kind: &'a EffectKind,
    /// The source projection after applying an enclosing folder's transform
    projection: Option<Projection>,
    blend: BlendMode,
    opacity: f32,
    data: Option<Cow<'a, [u16]>>,
    preview: bool,
    /// The detail sizes and micro details a generator reads
    maps: GeneratorMaps,
    /// A filtered mask stage retained because an anchor above it records that stage.
    filtered_mask: Option<&'a Field>,
    /// The projector's depth map of a fill or material effect that culls by depth
    depth: Option<Arc<FillDepth>>,
    /// Whether a fill or material effect covers every texel (see [`covers_every_texel`]), so no texel asks
    covers_all: bool,
    /// The viewport environment's light, for Baked Lighting Environment
    environment: Option<EnvironmentSh>,
    /// Baked Lighting Stylized's settings worked out once for every texel it shades
    stylized: Option<StylizedShading>,
    /// The source a point filter's image input reads (Grayscale Conversion's Custom Input)
    input: Option<Source>,
    /// Custom stops shared across tile plans and channels
    gradient: Option<Arc<PreparedGradient>>,
}

impl EffectPlan<'_> {
    fn point(&self, filter: &Filter, kind: ChannelKind, value: [f32; 3]) -> Option<[f32; 3]> {
        match &self.gradient {
            Some(gradient) => gradient.point(kind, value),
            None => filter.point(kind, value),
        }
    }

    fn ramp_at(&self, filter: &Filter, position: f32) -> Option<[f32; 3]> {
        match &self.gradient {
            Some(gradient) => gradient.at(position),
            None => filter.ramp_at(position),
        }
    }
}

struct MaskPlan<'a> {
    background: f32,
    effects: Vec<EffectPlan<'a>>,
    /// The layer's own content in the channels its compare effects read (planned only when one reads the stack)
    layer: Option<(Content<'a>, Vec<ChanPlan<'a>>)>,
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
    data: Vec<f32>,
    /// Intermediate mask stages needed by anchors, owned by this drawn layer's final field.
    anchor_stages: Vec<(usize, Field)>,
}

impl Field {
    fn get(&self, x: u32, y: u32) -> Option<&[f32]> {
        if x < self.x0 || y < self.y0 || x >= self.x0 + self.w || y >= self.y0 + self.h {
            return None;
        }

        let i = ((y - self.y0) * self.w + (x - self.x0)) as usize * self.comps;
        self.data.get(i..i + self.comps)
    }
}

/// A layer's mask or content channel field, filtered under the folders' Scale and Rotation (as bits) its traversal lays on its fills
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldKey {
    Mask(LayerId, [u32; 2]),
    Content(LayerId, Channel, [u32; 2]),
    InstanceMask(LayerId, [u32; 2], usize),
    InstanceContent(LayerId, Channel, [u32; 2], usize),
}

impl FieldKey {
    fn in_instance(self, instance: usize) -> Self {
        match self {
            Self::Mask(id, offset) if instance != 0 => Self::InstanceMask(id, offset, instance),
            Self::Content(id, channel, offset) if instance != 0 => Self::InstanceContent(id, channel, offset, instance),
            key => key,
        }
    }

    fn field(self, fields: &Fields, instance: usize) -> Option<&Field> {
        if instance != 0
            && let Some(field) = fields.get(&self.in_instance(instance))
        {
            return Some(field);
        }

        fields.get(&self)
    }

    /// The key of layer `id`'s mask field under the folders' offset `around`
    pub fn mask(id: LayerId, around: ContentsOffset) -> FieldKey {
        FieldKey::Mask(id, [around.scale.to_bits(), around.rotation.to_bits()])
    }

    /// The key of layer `id`'s `channel` content field under the folders' offset `around`
    pub fn content(id: LayerId, channel: Channel, around: ContentsOffset) -> FieldKey {
        FieldKey::Content(id, channel, [around.scale.to_bits(), around.rotation.to_bits()])
    }
}

pub type Fields = HashMap<FieldKey, Field>;

/// How a neighbourhood filter reads around a texel
#[derive(Clone, Copy)]
enum Op {
    /// Box blurs (≈ Gaussian), sharpening by the amount when given
    Blur(Option<f32>),
    /// The value minus its box blur, around the channel's mid value: colors in `space`, the coverage too when `alpha`
    HighPass { space: HighpassSpace, alpha: bool },
    /// Values ramped down towards their shape's edge, from the box blur
    Bevel,
    /// The value plus its box blur times the amount
    Glow(f32),
    /// The value at the center of its cell (`r` texels across); the texel's own coverage kept unless `alpha`
    Mosaic { alpha: bool },
    /// Stair-stepped edges blended across (FXAA); the texel's own coverage kept unless `alpha`
    Fxaa { alpha: bool },
    /// The mean of the calmest sector of an ellipse along the local direction (see [`kuwahara_texel`])
    Kuwahara(AnisotropicKuwahara),
    /// Average along a direction (unit vector in texels)
    Line([f32; 2]),
    /// Read at an offset given by a noise vector field, or by noise along a direction
    Warp { scale: f32, seed: u32, dir: Option<[f32; 2]> },
    /// Gradient magnitude from central differences
    Edge,
    /// Average downhill along a noise height field
    Slope { scale: f32, seed: u32 },
    /// Box blurs, then the effect's own point filter on the blurred value (a gradient's Blur Intensity)
    PreBlur,
    /// A halo from the box blur of the shapes, laid around (or inside) them
    Halo,
    /// The shapes moved by `offset` (whole texels), box blurred and laid under them
    Shadow { offset: [i32; 2] },
    /// Each UV tile's texels read from where `mirror` sends them in their tile (`block` texels across the set's atlas blocks)
    Mirror { mirror: Mirror, block: u32 },
    /// Each UV tile's texels read from where `transform` sends them (its `affine`, worked out once), repeating round the tile or empty past it
    Transform { transform: TileTransform, affine: ([[f32; 2]; 2], [f32; 2]), block: u32 },
    /// A band along the edge of the shapes past a threshold, from the box blur of the shapes
    Outline(MaskOutline),
    /// Drawn from the distance to the shapes' edge (see [`distance_filter`])
    Distance,
    /// Read from where a warp's source sends each texel (see [`warp_texel`])
    SourceWarp(SourceWarp),
    /// The values downhill of each texel along a source's slope, blended (see [`slope_texel`])
    SourceSlopeBlur(SourceSlopeBlur),
    /// The normal of the slopes of the layer's Height (see [`height_to_normal_texel`])
    HeightToNormal(HeightToNormal),
    /// Colors moved from a source color to a target color; the source the whole set's mean in Source Color Mode Average (see [`color_match`])
    ColorMatch(ColorMatch),
}

/// The last neighbourhood filter of an effect stack (see [`neighbourhood_filters`])
fn neighbourhood(effects: &[Effect], res: u32) -> Option<(usize, f32, Op)> {
    neighbourhood_filters(effects, res, None).last()
}

/// Below this squared length an unpacked baked normal is no direction (an unbaked texel)
const MIN_NORMAL_LENGTH_SQUARED: f32 = 1e-6;

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
    effects
        .iter()
        .filter(|effect| effect.visible)
        .enumerate()
        .filter(move |(_, effect)| channel.is_none_or(|channel| effect.blend_of(Some(channel)).mode != BlendMode::Disable))
        .filter_map(move |(index, effect)| match &effect.kind {
            EffectKind::Filter(Filter::Blur { radius, .. }) if *radius > 0.0 => Some((index, texels(*radius), Op::Blur(None))),
            EffectKind::Filter(Filter::Sharpen { amount }) => {
                Some((index, (SHARPEN_RADIUS * res as f32 / FILTER_SIZE_RESOLUTION).max(MIN_SHARPEN_RADIUS), Op::Blur(Some(*amount))))
            }
            EffectKind::Filter(Filter::DirectionalBlur { radius, angle }) if *radius > 0.0 => Some((index, texels(*radius), Op::Line(direction(*angle)))),
            EffectKind::Filter(Filter::Warp { intensity, scale, seed, divider }) if *intensity > 0.0 => {
                Some((index, texels(divided(*intensity, *divider)), Op::Warp { scale: *scale, seed: *seed, dir: None }))
            }
            EffectKind::Filter(Filter::DirectionalWarp { intensity, angle, scale, seed, divider }) if *intensity > 0.0 => {
                Some((index, texels(divided(*intensity, *divider)), Op::Warp { scale: *scale, seed: *seed, dir: Some(direction(*angle)) }))
            }
            EffectKind::Filter(Filter::SourceWarp(warp)) if warp.move_texels(res) > 0.0 => Some((index, warp.reach(res).max(0.5), Op::SourceWarp(*warp))),
            EffectKind::Filter(Filter::SourceSlopeBlur(slope)) if slope.move_texels(res) > 0.0 => {
                Some((index, slope.reach(res).max(0.5), Op::SourceSlopeBlur(*slope)))
            }
            EffectKind::Filter(Filter::HighPass { radius, space, alpha }) => Some((index, texels(*radius), Op::HighPass { space: *space, alpha: *alpha })),
            EffectKind::Filter(Filter::EdgeDetect { width }) => Some((index, texels(*width), Op::Edge)),
            // Three box passes ramp an edge over about twice their radius
            EffectKind::Filter(Filter::Bevel { width, .. }) => Some((index, texels(*width * 0.5), Op::Bevel)),
            EffectKind::Filter(Filter::Glow { radius, amount }) => Some((index, texels(*radius), Op::Glow(*amount))),
            EffectKind::Filter(Filter::Mosaic { size, alpha }) => Some((index, texels(*size), Op::Mosaic { alpha: *alpha })),
            EffectKind::Filter(Filter::Fxaa { alpha }) => Some((index, FXAA_SEARCH as f32 + 1.0, Op::Fxaa { alpha: *alpha })),
            // Both the direction search and the ellipse can reach twice the radius
            EffectKind::Filter(Filter::AnisotropicKuwahara(kuwahara)) if kuwahara.radius > 0.0 => {
                Some((index, texels(kuwahara.radius.min(AnisotropicKuwahara::MAX_RADIUS)) * (2.0 + kuwahara.tensor_reach()) + 1.0, Op::Kuwahara(*kuwahara)))
            }

            // The slopes come from the heights right around a texel
            EffectKind::Filter(Filter::HeightToNormal(filter)) if channel.is_none_or(|channel| channel == Channel::Normal) => {
                Some((index, 1.0, Op::HeightToNormal(*filter)))
            }

            // Average reads the mean color of the whole set; the other modes read the texel alone
            EffectKind::Filter(Filter::ColorMatch(matcher)) if channel.is_none_or(|channel| channel.kind() == ChannelKind::Color) => {
                Some((index, if matcher.source_mode == SourceColorMode::Average { WHOLE_SET } else { 1.0 }, Op::ColorMatch(*matcher)))
            }

            // Use reference-resolution reach so all callers classify the same filters as stages
            EffectKind::Generator(generator)
                if generator.applied_global().blur > 0.0 || generator.resolution.can_reduce() || sections::generator_blur_sigma(generator) > 0.0 =>
            {
                let section_blur = sections::generator_blur_sigma(generator);
                let section_reach = if section_blur > 0.0 { texels(section_blur) } else { 0.0 };
                let reach = texels(generator.applied_global().blur_radius()) + section_reach + generator.resolution.largest_cell(res) as f32;
                Some((index, reach, Op::Blur(None)))
            }

            // A mirror reads anywhere in its UV tile
            EffectKind::Filter(Filter::Mirror(mirror)) => Some((index, res as f32, Op::Mirror { mirror: *mirror, block: res })),
            // A transform reads anywhere in its UV tile; one that changes nothing is skipped
            EffectKind::Filter(Filter::Transform(transform)) if !transform.is_identity() => {
                Some((index, res as f32, Op::Transform { transform: *transform, affine: transform.affine(), block: res }))
            }

            // Three box passes reach the band's width
            EffectKind::Filter(Filter::MaskOutline(outline)) => {
                Some((index, (outline.width * res as f32 / BLUR_BOX_PASSES as f32).max(MIN_FILTER_RADIUS), Op::Outline(*outline)))
            }

            // Three box passes' reach covers the distance and its smoothing
            EffectKind::Filter(filter @ (Filter::DistanceBevel(_) | Filter::BevelSmooth(_) | Filter::DirectionalDistance(_))) => {
                Some((index, distance_reach(filter, res) / BLUR_BOX_PASSES as f32 + 1.0, Op::Distance))
            }
            EffectKind::Filter(Filter::HaloGlow { width, .. }) => Some((index, texels(width.abs() * Filter::HALO_REACH), Op::Halo)),
            EffectKind::Filter(Filter::DropShadow { angle, distance, size, .. }) => {
                // Cast away from the light: `distance` of a UV tile, against the light's direction
                let [dx, dy] = direction(*angle);
                let reach = distance.clamp(-Filter::SHADOW_DISTANCE_MAX, Filter::SHADOW_DISTANCE_MAX) * res as f32;
                let offset = [(-dx * reach).round() as i32, (-dy * reach).round() as i32];
                Some((index, texels(size * Filter::HALO_REACH), Op::Shadow { offset }))
            }
            EffectKind::Filter(Filter::GradientMap { input, debug: false, .. }) if input.blur > 0.0 => Some((index, texels(input.blur), Op::PreBlur)),
            EffectKind::Filter(Filter::SlopeBlur { radius, scale, seed, divider }) if *radius > 0.0 => {
                Some((index, texels(divided(*radius, *divider)), Op::Slope { scale: *scale, seed: *seed }))
            }
            _ => None,
        })
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

/// A neighbourhood radius that reaches the whole set ([`filter_extent`] caps it at the set's longest side)
const WHOLE_SET: f32 = 65_536.0;

/// How many texels around a texel distance filter `filter` reads at `res` texels across a UV tile: its reach and its smoothing's three box passes
fn distance_reach(filter: &Filter, res: u32) -> f32 {
    match filter {
        Filter::DistanceBevel(bevel) => bevel.reach(res) + BLUR_BOX_PASSES as f32 * bevel.smoothing_texels(res),
        Filter::BevelSmooth(smooth) => smooth.reach(res) + BLUR_BOX_PASSES as f32 * smooth.smoothing_texels(res),
        Filter::DirectionalDistance(trail) => trail.reach(res),
        _ => 0.0,
    }
}

/// `size` over its intensity divider (a divider of 0 from a hostile file counts as 1)
fn divided(size: f32, divider: u16) -> f32 {
    size / f32::from(divider.max(1))
}

/// Unit texel-space vector of `angle` degrees. Texel rows run down: positive angles turn
/// counterclockwise on screen.
pub(crate) fn direction(angle: f32) -> [f32; 2] {
    let (sn, cs) = angle.to_radians().sin_cos();
    [cs, -sn]
}

/// Gain on the curvature map's own deviation from flat (±0.5 reaches black and white).
const MAP_GAIN: f32 = 2.0;

/// Gain per texel of a detail size: blurring spreads an edge over about twice its radius, so wider sizes need proportionally more to reach full strength.
const GAIN_PER_DETAIL_TEXEL: f32 = 2.0 * std::f32::consts::SQRT_2;

/// Samples along a directional or slope blur at most (beyond, samples spread out).
pub(crate) const LINE_SAMPLES: i32 = 24;

/// Fractal noise over texel position `(x, y)` (`res` texels per UV unit, `scale` features per
/// UV unit), shifted by `d` in noise space.
pub(crate) fn texel_noise(x: f32, y: f32, d: [f32; 2], res: u32, scale: f32, seed: u32) -> f32 {
    let per_texel = if scale.is_finite() { scale.clamp(Filter::NOISE_SCALE_MIN, Filter::NOISE_SCALE_MAX) } else { 1.0 } / res.max(1) as f32;
    noise::fbm([x * per_texel + d[0], y * per_texel + d[1], 0.0], 3, seed, noise::perlin)
}

/// Gain on a noise warp's value around mid gray: values a third away from it already move the whole radius, as the noise seldom strays further
const WARP_NOISE_GAIN: f32 = 3.0;

/// Mixed into a noise warp's seed for its vertical move, so it differs from the horizontal one (any odd constant does)
const WARP_Y_SALT: u32 = 0x5BD1_E995;

/// How far apart, in noise space, a slope blur measures its noise's slope
const SLOPE_NOISE_PROBE: f32 = 0.01;

/// The neighbourhood filters that gather samples (directional and slope blur, warp) over a field region `(x0, y0, w, h)` of premultiplied `orig` texels (`comps` each) and their `cover`: bilinear samples weighted by coverage, so uncovered texels don't bleed in; positions are absolute texel coordinates, so a texel's result does not depend on the region
#[allow(clippy::too_many_arguments)]
fn gather(
    op: Op,
    (orig, cover, grays): (&[f32], &[f32], &[f32]),
    comps: usize,
    (x0, y0, w, h): (u32, u32, u32, u32),
    r: f32,
    (res, block_size, tile_size): (u32, [u32; 2], [u32; 2]),
    opacity: f32,
) -> Vec<f32> {
    let mut data = orig.to_vec();
    let (width, height) = (w as usize, h as usize);
    let warp_directions = match op {
        Op::SourceWarp(warp) => warp.angles().map(direction).collect::<Vec<_>>(),
        _ => Vec::new(),
    };

    // The filter's image input at absolute texel position (fx, fy), bilinear over the covered texels; None off them
    let input = |fx: f32, fy: f32| -> Option<f32> {
        if !(fx.is_finite() && fy.is_finite()) {
            return None;
        }

        let (bx, by) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - bx, fy - by);
        let (mut sum, mut weight) = (0.0f32, 0.0f32);
        for (dx, dy, k) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
            let (cx, cy) = (bx as i64 + dx - i64::from(x0), by as i64 + dy - i64::from(y0));
            if k <= 0.0 || cx < 0 || cy < 0 || cx >= width as i64 || cy >= height as i64 {
                continue;
            }

            let i = cy as usize * width + cx as usize;
            if let (Some(covered), Some(gray)) = (cover.get(i), grays.get(i))
                && *covered > 0.0
            {
                sum += gray * k;
                weight += k;
            }
        }
        (weight > MIN_READ_WEIGHT).then(|| sum / weight)
    };

    // Bilinear sample at absolute texel position (fx, fy) into `acc` (components, then weight)
    let sample = |fx: f32, fy: f32, acc: &mut [f32; 5]| {
        if !(fx.is_finite() && fy.is_finite()) {
            return;
        }

        let (bx, by) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - bx, fy - by);
        for (dx, dy, k) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
            let (cx, cy) = (bx as i64 + dx - i64::from(x0), by as i64 + dy - i64::from(y0));
            if k <= 0.0 || cx < 0 || cy < 0 || cx >= width as i64 || cy >= height as i64 {
                continue;
            }

            let i = cy as usize * width + cx as usize;
            let (Some(c), Some(px)) = (cover.get(i), orig.get(i * comps..(i + 1) * comps)) else { continue };
            for (a, v) in acc.iter_mut().zip(px) {
                *a += v * k;
            }
            acc[4] += c * k;
        }
    };

    data.par_chunks_mut((comps * width).max(1)).enumerate().for_each(|(row, line)| {
        for col in 0..width {
            if cover.get(row * width + col).is_none_or(|c| *c <= 0.0) {
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
                    let noise_offset = |salt: u32| ((texel_noise(x, y, [0.0; 2], res, scale, seed ^ salt) - 0.5) * WARP_NOISE_GAIN).clamp(-1.0, 1.0) * r;
                    match dir {
                        Some(d) => {
                            let t = noise_offset(0);
                            sample(x + d[0] * t, y + d[1] * t, &mut acc);
                        }
                        None => sample(x + noise_offset(0), y + noise_offset(WARP_Y_SALT), &mut acc),
                    }
                }
                Op::Edge => {
                    let center = orig.get((row * width + col) * comps..(row * width + col + 1) * comps).unwrap_or(&[]);
                    let at = |dx: f32, dy: f32| {
                        let mut read = [0.0f32; 5];
                        sample(x + dx, y + dy, &mut read);
                        read
                    };
                    let (left, right, up, down) = (at(-r, 0.0), at(r, 0.0), at(0.0, -r), at(0.0, r));

                    // A side outside the UV island reads the texel itself: seams are not edges
                    let component = |read: &[f32; 5], k: usize| {
                        if read[4] > MIN_READ_WEIGHT { read[k] / read[4] } else { center.get(k).copied().unwrap_or(0.0) }
                    };
                    let magnitude = |k: usize| (component(&right, k) - component(&left, k)).hypot(component(&down, k) - component(&up, k)).min(1.0);

                    // Content stays premultiplied by its own coverage; masks are one value
                    let coverage = if comps == 4 { center.get(3).copied().unwrap_or(0.0) } else { 1.0 };
                    for (k, slot) in acc.iter_mut().enumerate().take(comps.min(3)) {
                        *slot = magnitude(k) * coverage;
                    }
                    if comps == 4 {
                        acc[3] = coverage;
                    }
                    acc[4] = 1.0;
                }
                Op::Slope { scale, seed } => {
                    let noise_height = |dx: f32, dy: f32| texel_noise(x, y, [dx, dy], res, scale, seed);
                    let probe = SLOPE_NOISE_PROBE;

                    // Downhill; a flat spot keeps its value
                    let downhill = Vec2::new(noise_height(-probe, 0.0) - noise_height(probe, 0.0), noise_height(0.0, -probe) - noise_height(0.0, probe));
                    let d = downhill.try_normalize().unwrap_or(Vec2::ZERO);
                    for s in 0..=n {
                        let t = s as f32 * step;
                        sample(x + d.x * t, y + d.y * t, &mut acc);
                    }
                }
                Op::Mosaic { alpha } => {
                    let center = |v: f32| ((v / r).floor() + 0.5) * r - 0.5;
                    sample(center(x), center(y), &mut acc);
                    if !alpha {
                        keep_coverage(&mut acc, comps, orig.get((row * width + col) * comps + 3));
                    }
                }
                Op::Fxaa { alpha } => {
                    let center = orig.get((row * width + col) * comps..(row * width + col + 1) * comps).unwrap_or(&[]);
                    fxaa_texel((x, y), comps, center, &mut acc, &sample);
                    if !alpha {
                        keep_coverage(&mut acc, comps, center.get(3));
                    }
                }
                Op::Mirror { mirror, block } => {
                    mirror_texel(&mirror, (x, y), (block, res), comps, orig.get((row * width + col) * comps + comps - 1), &mut acc, &sample)
                }
                Op::Transform { transform, affine, block } => transform_texel(&transform, affine, (x, y), (block, res), &mut acc, &sample),
                Op::SourceWarp(warp) => warp_texel((&warp, &warp_directions), (x, y), (res, comps), (block_size, tile_size), &mut acc, (&sample, &input)),
                Op::SourceSlopeBlur(slope) => slope_texel(&slope, (x, y), (res, comps), (block_size, tile_size), &mut acc, (&sample, &input)),
                Op::HeightToNormal(filter) => {
                    let center = orig.get((row * width + col) * comps..(row * width + col + 1) * comps).unwrap_or(&[]);
                    height_to_normal_texel(&filter, (x, y), (res, center), &mut acc, &input);
                }
                Op::Kuwahara(kuwahara) => {
                    let radius = (kuwahara.radius.min(AnisotropicKuwahara::MAX_RADIUS) * res as f32 / FILTER_SIZE_RESOLUTION).max(MIN_FILTER_RADIUS);
                    kuwahara_texel(&kuwahara, (x, y), (radius, comps), &mut acc, (&sample, &input));
                }
                Op::ColorMatch(_)
                | Op::Blur(_)
                | Op::HighPass { .. }
                | Op::Bevel
                | Op::Glow(_)
                | Op::PreBlur
                | Op::Halo
                | Op::Shadow { .. }
                | Op::Outline(_)
                | Op::Distance => {}
            }
            if acc[4] > MIN_READ_WEIGHT
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

/// The linear colors a filter input gives over the tile-aligned texel region, row by row
fn input_colors(
    inputs: &EvaluationInputs,
    source: &Source,
    (draw, channels, below): ((LayerId, usize), &[Channel], &Fields),
    (left, top, width, height): (u32, u32, u32, u32),
) -> Vec<[f32; 3]> {
    if width == 0 || height == 0 {
        return Vec::new();
    }

    let mut colors = vec![[0.0; 3]; (width * height) as usize];
    let horizontal_tiles = left / TILE..(left + width).div_ceil(TILE);
    colors.par_chunks_mut((TILE * width) as usize).enumerate().for_each(|(band, colors)| {
        let tile_y = top / TILE + band as u32;
        let band_top = tile_y * TILE;
        for tile_x in horizontal_tiles.clone() {
            let anchors = if matches!(source, Source::Anchor { .. }) { anchor_plan(inputs, draw, channels, tile_x, tile_y, Some(below)) } else { Vec::new() };
            let preview_tile = inputs.preview.and_then(|preview| preview.alpha.tile(tile_x, tile_y));

            for (x, y, offset) in tile_texels(inputs.gbuf, tile_x, tile_y) {
                if x < left || x - left >= width {
                    continue;
                }

                let index = ((y - band_top) * width + (x - left)) as usize;
                let Some(pixel) = colors.get_mut(index) else { continue };
                let mut texel = Texel::new(inputs, x, y, offset, stroke_at(preview_tile, offset));
                if !anchors.is_empty() {
                    texel.anchors_below(&anchors, draw);
                }

                *pixel = texel.source(source, &Projection::default(), Sampling::default(), ChannelKind::Color);
            }
        }
    });

    colors
}

/// Color Match's per-texel source or target colors (None: one color for the whole region)
type MatchColors = Option<Vec<[f32; 3]>>;

/// Color Match over premultiplied `data` (`comps` values a texel): each covered color moved from its source (the region's mean or its Input color) to its target (Target Color or its Image Input color), mixed in by `opacity`
fn color_match(data: &mut [f32], comps: usize, matcher: &ColorMatch, (sources, targets): (&MatchColors, &MatchColors), opacity: f32) {
    if comps != 4 || opacity <= 0.0 {
        return;
    }

    let source_color = if matcher.source_mode == SourceColorMode::Average { mean_region_color(data, matcher.target_color) } else { matcher.source_color };
    let source_pixels = if matcher.source_mode == SourceColorMode::Input { sources.as_deref() } else { None };
    let target_pixels = if matcher.target_mode == TargetColorMode::ImageInput { targets.as_deref() } else { None };

    if source_pixels.is_none() && target_pixels.is_none() {
        let prepared = PreparedColorMatch::new(*matcher, source_color, matcher.target_color);
        match_pixels(data, opacity, |_, color| prepared.apply(color));
    } else {
        match_pixels(data, opacity, |index, color| {
            let source = source_pixels.and_then(|pixels| pixels.get(index)).copied().unwrap_or(source_color);
            let target = target_pixels.and_then(|pixels| pixels.get(index)).copied().unwrap_or(matcher.target_color);
            matcher.apply(color, source, target)
        });
    }
}

fn match_pixels(data: &mut [f32], opacity: f32, remap: impl Fn(usize, [f32; 3]) -> [f32; 3] + Sync) {
    data.par_chunks_mut(4).enumerate().for_each(|(index, pixel)| {
        let [red, green, blue, alpha] = *pixel else { return };
        if alpha <= 0.0 || !alpha.is_finite() {
            return;
        }

        let moved = remap(index, [red, green, blue].map(|component| component / alpha));
        for (slot, value) in pixel.iter_mut().zip(moved) {
            *slot += (value * alpha - *slot) * opacity;
        }
    });
}

/// The mean of a premultiplied color region, or the fallback when no texel is covered
fn mean_region_color(data: &[f32], fallback: [f32; 3]) -> [f32; 3] {
    // Premultiplication weights the color by coverage
    let (color_sum, coverage_sum) = data
        .par_chunks(4)
        .fold(
            || ([0.0f64; 3], 0.0f64),
            |(mut colors, mut coverage), pixel| {
                if let [red, green, blue, alpha] = *pixel
                    && alpha > 0.0
                    && alpha.is_finite()
                {
                    for (sum, component) in colors.iter_mut().zip([red, green, blue]) {
                        *sum += f64::from(component);
                    }
                    coverage += f64::from(alpha);
                }

                (colors, coverage)
            },
        )
        .reduce(
            || ([0.0; 3], 0.0),
            |(left, left_coverage), (right, right_coverage)| ([left[0] + right[0], left[1] + right[1], left[2] + right[2]], left_coverage + right_coverage),
        );

    if coverage_sum > 0.0 { color_sum.map(|component| (component / coverage_sum) as f32) } else { fallback }
}

/// Height To Normal at texel `(x, y)` into `acc`: the 3×3 Sobel normal of the heights `height` reads, laid over normal `center` unless it overwrites it
fn height_to_normal_texel(
    filter: &HeightToNormal,
    (x, y): (f32, f32),
    (res, center): (u32, &[f32]),
    acc: &mut [f32; 5],
    height: &dyn Fn(f32, f32) -> Option<f32>,
) {
    let (Some(own), [red, green, blue, cov]) = (height(x, y), center) else { return };
    if *cov <= 0.0 {
        return;
    }

    let at = |dx: f32, dy: f32| height(x + dx, y + dy).unwrap_or(own);

    // Sobel gradients over the kernel's weight sum
    let gx = (at(1.0, -1.0) + 2.0 * at(1.0, 0.0) + at(1.0, 1.0) - at(-1.0, -1.0) - 2.0 * at(-1.0, 0.0) - at(-1.0, 1.0)) / 8.0;
    let gy = (at(-1.0, 1.0) + 2.0 * at(0.0, 1.0) + at(1.0, 1.0) - at(-1.0, -1.0) - 2.0 * at(0.0, -1.0) - at(1.0, -1.0)) / 8.0;
    let made = filter.normal([gx, gy], res).map(|v| v * 0.5 + 0.5);
    let packed = if filter.overwrite { made } else { texturecraft_core::blend::rnm([red, green, blue].map(|v| v / cov), made) };
    for (slot, value) in acc.iter_mut().zip(packed) {
        *slot = value * cov;
    }
    acc[3] = *cov;
    acc[4] = 1.0;
}

/// Gathered sample `acc` (premultiplied, then weight) recoloured over the texel's own coverage `own` (content only; a sample with no coverage leaves it)
fn keep_coverage(acc: &mut [f32; 5], comps: usize, own: Option<&f32>) {
    let Some(&own) = own.filter(|_| comps == 4) else { return };
    if acc[3] <= MIN_READ_WEIGHT {
        acc[4] = 0.0;
        return;
    }

    let scale = own * acc[4] / acc[3];
    for component in acc.iter_mut().take(3) {
        *component *= scale;
    }
    acc[3] = own * acc[4];
}

/// Sample rows (and columns) on each side of the middle Anisotropic Kuwahara reads its ellipse with at most
const KUWAHARA_STEPS: i32 = 6;

/// Below this, a structure tensor has no direction
const MIN_TENSOR: f32 = 1e-12;

/// How fast a Kuwahara sample's gaussian weight falls with its squared distance across the ellipse (e^-2 at the rim)
const KUWAHARA_FALLOFF: f32 = 2.0;

/// A spread of one in a sector, in 8-bit steps: what Anisotropic Kuwahara's sector weights are measured in
const KUWAHARA_SPREAD_SCALE: f32 = 255.0;

/// Anisotropic Kuwahara at texel `(x, y)` (`radius` texels, `comps` components) into `acc`: an ellipse along the gray's direction, its sectors' means weighed by their calm
fn kuwahara_texel(
    kuwahara: &AnisotropicKuwahara,
    (x, y): (f32, f32),
    (radius, comps): (f32, usize),
    acc: &mut [f32; 5],
    (sample, input): (Sample<'_>, Input<'_>),
) {
    // A straight value at an offset; None off the UV island
    let read = |fx: f32, fy: f32| -> Option<[f32; 4]> {
        let mut sum = [0.0f32; 5];
        sample(fx, fy, &mut sum);
        (sum[4] > MIN_READ_WEIGHT).then(|| std::array::from_fn(|k| if k < comps.min(4) { sum[k] / sum[4] } else { 0.0 }))
    };
    let gray = |fx: f32, fy: f32| if kuwahara.custom_input { input(fx, fy) } else { read(fx, fy).map(|px| fxaa_luma(&px[..comps.min(4)])) };

    // Structure tensor: the gray's gradients over a 3×3 grid spread by Tensor Smoothness, the middle weighing most
    let spread = kuwahara.tensor_reach() * radius;
    let (mut xx, mut xy, mut yy) = (0.0f32, 0.0f32, 0.0f32);
    for gy in -1i32..=1 {
        for gx in -1i32..=1 {
            let (px, py) = (x + gx as f32 * spread, y + gy as f32 * spread);
            let own = gray(px, py);
            let at = |dx: f32, dy: f32| gray(px + dx, py + dy).or(own);
            let (Some(left), Some(right), Some(up), Some(down)) = (at(-1.0, 0.0), at(1.0, 0.0), at(0.0, -1.0), at(0.0, 1.0)) else { continue };

            // Central differences over two texels
            let (dx, dy) = ((right - left) * 0.5, (down - up) * 0.5);

            // A 1-2-1 binomial kernel in each direction: the middle 4, the sides 2, the corners 1
            let weight = (4 >> (gx.abs() + gy.abs())) as f32;
            xx += weight * dx * dx;
            xy += weight * dx * dy;
            yy += weight * dy * dy;
        }
    }
    let root = ((xx - yy).powi(2) + 4.0 * xy * xy).sqrt();
    let clear = if xx + yy > MIN_TENSOR { root / (xx + yy) } else { 0.0 };

    // Strokes run across the steepest change; Anisotropy Angle turns the ellipse counterclockwise on screen
    let along = 0.5 * (2.0 * xy).atan2(xx - yy) + std::f32::consts::FRAC_PI_2 - kuwahara.angle.to_radians();
    let (sin, cos) = if along.is_finite() { along.sin_cos() } else { (0.0, 1.0) };
    let (major, minor) = kuwahara.axes(radius, if clear.is_finite() { clear } else { 0.0 });

    // Each sector's weighted sum, squared sum and weight
    let mut sums = [[0.0f32; 5]; AnisotropicKuwahara::SECTORS];
    let mut squares = [0.0f32; AnisotropicKuwahara::SECTORS];
    let steps = (major.ceil() as i32).clamp(1, KUWAHARA_STEPS);
    let exponent = kuwahara.sector_exponent();
    let colors = comps.min(3);
    for j in -steps..=steps {
        for i in -steps..=steps {
            let (u, v) = (i as f32 / steps as f32, j as f32 / steps as f32);
            let distance = u * u + v * v;
            if distance > 1.0 {
                continue;
            }

            let (ox, oy) = (u * major, v * minor);
            let Some(px) = read(x + ox * cos - oy * sin, y + ox * sin + oy * cos) else { continue };
            let falloff = (-KUWAHARA_FALLOFF * distance).exp();
            let heading = v.atan2(u);
            for (k, (sum, square)) in sums.iter_mut().zip(&mut squares).enumerate() {
                // The center contributes to every sector while other weights taper at sector boundaries
                let facing = (heading - k as f32 * std::f32::consts::TAU / AnisotropicKuwahara::SECTORS as f32).cos();
                let weight = if distance <= 0.0 {
                    falloff
                } else if facing > 0.0 {
                    falloff * facing.powf(exponent)
                } else {
                    continue;
                };

                for (slot, value) in sum.iter_mut().zip(&px) {
                    *slot += weight * value;
                }
                sum[4] += weight;
                *square += weight * px.iter().take(colors).map(|value| value * value).sum::<f32>();
            }
        }
    }

    // Weight low-variance sectors more and use logarithms to avoid overflow
    let sharpness = kuwahara.sharpness_exponent();
    let logs: [Option<f32>; AnisotropicKuwahara::SECTORS] = std::array::from_fn(|sector| {
        let (sum, square) = (sums.get(sector)?, squares.get(sector)?);
        (sum[4] > MIN_READ_WEIGHT).then(|| {
            let mean = sum.iter().take(colors).map(|value| (value / sum[4]).powi(2)).sum::<f32>();
            let spread = (square / sum[4] - mean).max(0.0).sqrt();
            -sharpness * (1.0 + KUWAHARA_SPREAD_SCALE * spread).ln()
        })
    });
    let Some(top) = logs.iter().flatten().copied().reduce(f32::max) else { return };
    let mut total = [0.0f32; 5];
    for (sum, log) in sums.iter().zip(&logs) {
        let Some(log) = log else { continue };
        let weight = (log - top).exp();
        for (slot, value) in total.iter_mut().zip(sum.iter().take(4)) {
            *slot += weight * value / sum[4];
        }
        total[4] += weight;
    }
    *acc = total;
}

/// Texels an FXAA edge search walks each way along an edge at most
const FXAA_SEARCH: i32 = 8;

/// The least luma contrast around a texel FXAA smooths
const FXAA_EDGE_MIN: f32 = 0.0312;

/// The least luma contrast around a texel FXAA smooths, as a share of its brightest neighbour
const FXAA_EDGE_RELATIVE: f32 = 0.125;

/// How much a lone texel (subpixel aliasing) is blended into its neighbours at most
const FXAA_SUBPIXEL: f32 = 0.75;

/// Luma of premultiplied texel components (a mask's value)
fn fxaa_luma(px: &[f32]) -> f32 {
    match px {
        [value] => *value,
        [r, g, b, ..] => lum([*r, *g, *b]),
        _ => 0.0,
    }
}

/// FXAA (the public luma-edge method) at texel `(x, y)` with components `center` into `acc`: read across its edge by how near the edge's end is
fn fxaa_texel((x, y): (f32, f32), comps: usize, center: &[f32], acc: &mut [f32; 5], sample: &dyn Fn(f32, f32, &mut [f32; 5])) {
    let own = fxaa_luma(center);

    // Luma at an offset from the texel; off the UV island the texel's own, so seams are not edges
    let luma = |dx: f32, dy: f32| {
        let mut read = [0.0f32; 5];
        sample(x + dx, y + dy, &mut read);
        if read[4] <= MIN_READ_WEIGHT {
            return own;
        }

        let mut px = [0.0f32; 4];
        for (slot, v) in px.iter_mut().zip(&read).take(comps.min(4)) {
            *slot = v / read[4];
        }
        fxaa_luma(&px[..comps.min(4)])
    };
    let (n, s, w, e) = (luma(0.0, -1.0), luma(0.0, 1.0), luma(-1.0, 0.0), luma(1.0, 0.0));
    let (hi, lo) = (own.max(n).max(s).max(w).max(e), own.min(n).min(s).min(w).min(e));
    let range = hi - lo;
    if !range.is_finite() || range < FXAA_EDGE_MIN.max(hi * FXAA_EDGE_RELATIVE) {
        return;
    }

    let (nw, ne, sw, se) = (luma(-1.0, -1.0), luma(1.0, -1.0), luma(-1.0, 1.0), luma(1.0, 1.0));

    // An edge running across (values changing down the rows) or down (values changing along them)
    let across = 2.0 * (n - 2.0 * own + s).abs() + (nw - 2.0 * w + sw).abs() + (ne - 2.0 * e + se).abs();
    let down = 2.0 * (w - 2.0 * own + e).abs() + (nw - 2.0 * n + ne).abs() + (sw - 2.0 * s + se).abs();
    let horizontal = across >= down;
    let (before, after) = if horizontal { (n, s) } else { (w, e) };
    let (sign, side) = if (before - own).abs() >= (after - own).abs() { (-1.0, before) } else { (1.0, after) };
    let (perp, along) = if horizontal { ([0.0, sign], [1.0, 0.0]) } else { ([sign, 0.0], [0.0, 1.0]) };

    // Walk the edge's midline both ways until its luma leaves the edge's
    let edge = 0.5 * (own + side);
    let enough = 0.25 * (side - own).abs();
    let walk = |way: f32| {
        let mut end = (FXAA_SEARCH as f32, 0.0);
        for step in 1..=FXAA_SEARCH {
            let t = step as f32 * way;
            let delta = luma(perp[0] * 0.5 + along[0] * t, perp[1] * 0.5 + along[1] * t) - edge;
            end = (step as f32, delta);
            if delta.abs() >= enough {
                break;
            }
        }
        end
    };
    let ((back, back_delta), (ahead, ahead_delta)) = (walk(-1.0), walk(1.0));
    let (nearest, delta) = if back < ahead { (back, back_delta) } else { (ahead, ahead_delta) };
    let edge_blend = if (delta < 0.0) != (own < edge) { 0.5 - nearest / (back + ahead) } else { 0.0 };

    // A texel unlike all its neighbours blends in by its own contrast (sides weigh twice the corners)
    let average = (2.0 * (n + s + w + e) + nw + ne + sw + se) / 12.0;
    let contrast = ((average - own).abs() / range).clamp(0.0, 1.0);
    let eased = crate::organic::smoothstep(0.0, 1.0, contrast);
    let blend = edge_blend.max(eased * eased * FXAA_SUBPIXEL);
    if blend > 0.0 {
        sample(x + perp[0] * blend, y + perp[1] * blend, acc);
    }
}

/// Mirror `mirror` at texel `(x, y)` into `acc`: the texel it reads in its UV tile (`res` texels from its `block`'s top left), or white on a debug axis
fn mirror_texel(
    mirror: &Mirror,
    (x, y): (f32, f32),
    (block, res): (u32, u32),
    comps: usize,
    alpha: Option<&f32>,
    acc: &mut [f32; 5],
    sample: &dyn Fn(f32, f32, &mut [f32; 5]),
) {
    let (block, size) = (block.max(1) as f32, res.max(1) as f32);
    let (origin_x, origin_y) = ((x / block).floor() * block, (y / block).floor() * block);
    let (local_x, local_y) = (x - origin_x, y - origin_y);
    if local_x >= size || local_y >= size {
        return;
    }

    let uv = [(local_x + 0.5) / size, (local_y + 0.5) / size];
    let line = (Mirror::DEBUG_HALF_WIDTH * size / FILTER_SIZE_RESOLUTION).max(MIN_FILTER_RADIUS) / size;
    if mirror.debug && mirror.on_axis(uv, line) {
        // Content stays premultiplied by its own coverage; masks are one value
        let cover = if comps == 4 { alpha.copied().unwrap_or(0.0) } else { 1.0 };
        for slot in acc.iter_mut().take(comps) {
            *slot = cover;
        }
        acc[4] = 1.0;
        return;
    }

    let [u, v] = mirror.source(uv);

    // Whole texels, so an even mirror copies texels exactly
    let at = |origin: f32, along: f32| origin + (along * size - 0.5).round().clamp(0.0, size - 1.0);
    sample(at(origin_x, u), at(origin_y, v), acc);
}

/// Transform `transform` (whose read is `affine`) at texel `(x, y)` into `acc`: bilinear between the four texels round where it reads in its UV tile (`res` texels from its `block`'s top left), each wrapped round the tile; where it does not repeat, a read past the tile counts as empty
fn transform_texel(
    transform: &TileTransform,
    (columns, shift): ([[f32; 2]; 2], [f32; 2]),
    (x, y): (f32, f32),
    (block, res): (u32, u32),
    acc: &mut [f32; 5],
    sample: &dyn Fn(f32, f32, &mut [f32; 5]),
) {
    let (block, size) = (block.max(1) as f32, res.max(1) as f32);
    let (origin_x, origin_y) = ((x / block).floor() * block, (y / block).floor() * block);
    let (local_x, local_y) = (x - origin_x, y - origin_y);
    if local_x >= size || local_y >= size {
        return;
    }

    let uv = [(local_x + 0.5) / size, (local_y + 0.5) / size];

    // Texel space of the tile, centres at whole numbers
    let at = [0, 1].map(|axis| (columns[0][axis] * uv[0] + columns[1][axis] * uv[1] + shift[axis]) * size - 0.5);
    if !at.iter().all(|a| a.is_finite()) {
        return;
    }

    let (base_x, base_y) = (at[0].floor(), at[1].floor());
    let (tx, ty) = (at[0] - base_x, at[1] - base_y);
    let repeat = transform.repeat == RepeatMode::HorizontallyAndVertically;
    let mut total = [0.0f32; 5];
    for (dx, dy, k) in [(0.0, 0.0, (1.0 - tx) * (1.0 - ty)), (1.0, 0.0, tx * (1.0 - ty)), (0.0, 1.0, (1.0 - tx) * ty), (1.0, 1.0, tx * ty)] {
        if k <= 0.0 {
            continue;
        }

        let (cx, cy) = (base_x + dx, base_y + dy);
        let inside = (0.0..size).contains(&cx) && (0.0..size).contains(&cy);
        if !repeat && !inside {
            // Empty: no value, full weight
            total[4] += k;
            continue;
        }

        // One whole texel: the sample's own weights, so this matches its bilinear read
        let mut one = [0.0f32; 5];
        sample(origin_x + cx.rem_euclid(size), origin_y + cy.rem_euclid(size), &mut one);
        for (slot, value) in total.iter_mut().zip(one) {
            *slot += value * k;
        }
    }
    *acc = total;
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
    blur_with_wrap(data, width, height, radius, (false, false));
}

fn blur_with_wrap(data: &mut [f32], width: usize, height: usize, radius: usize, wrap: (bool, bool)) {
    // Bound samples before fixed-point accumulation
    const MAX_INPUT_MAGNITUDE: f32 = 1e6;

    let to_fixed = |value: &f32| if value.is_finite() { (f64::from(value.clamp(-MAX_INPUT_MAGNITUDE, MAX_INPUT_MAGNITUDE)) * FIX).round() as i64 } else { 0 };
    let mut fixed: Vec<i64> = data.par_iter().with_min_len(MIN_PARALLEL_TEXELS).map(to_fixed).collect();
    blur_rows(&mut fixed, width, radius, wrap.0);

    let mut columns = transpose(&fixed, width, height);
    blur_rows(&mut columns, height, radius, wrap.1);
    transpose_into(&columns, height, width, &mut fixed);

    data.par_iter_mut().with_min_len(MIN_PARALLEL_TEXELS).zip(fixed).for_each(|(target, blurred)| *target = (blurred as f64 / FIX) as f32);
}

/// The box blurs `blur` runs along each axis
pub(crate) const BLUR_BOX_PASSES: usize = 3;

/// Three box blurs of radius `r` along every row (`w` long) of `data`.
fn blur_rows(data: &mut [i64], w: usize, r: usize, wrap: bool) {
    if w == 0 {
        return;
    }

    data.par_chunks_mut(w).with_min_len(lines_per_task(w)).for_each(|row| {
        let mut scratch = vec![0i64; row.len()];
        for _ in 0..BLUR_BOX_PASSES {
            if wrap {
                box_line_wrapped(row, &mut scratch, r);
            } else {
                box_line(row, &mut scratch, r);
            }
            row.copy_from_slice(&scratch);
        }
    });
}

/// One box blur of radius `r` of a line (texels past the ends count as 0).
fn box_line(src: &[i64], dst: &mut [i64], r: usize) {
    let n = (2 * r + 1) as i64;

    // Round the signed window mean consistently across axes
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

/// One periodic box pass, including full periods when the kernel is wider than the line.
fn box_line_wrapped(source: &[i64], output: &mut [i64], radius: usize) {
    let period = source.len();
    let taps = 2 * radius + 1;
    let mut outgoing = (period - radius % period) % period;
    let mut incoming = (outgoing + taps % period) % period;
    let mut total = if taps >= period { source.iter().sum::<i64>() * (taps / period) as i64 } else { 0 };
    total += source[outgoing..].iter().chain(source).take(taps % period).sum::<i64>();

    for value in output {
        *value = (2 * total + taps as i64).div_euclid(2 * taps as i64);
        total += source[incoming] - source[outgoing];
        outgoing += 1;
        incoming += 1;
        if outgoing == period {
            outgoing = 0;
        }
        if incoming == period {
            incoming = 0;
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

/// Plan the stack below a draw when it reads anchors or the material under it
fn below_plan<'a>(
    inp: &EvaluationInputs<'a>,
    draw: (LayerId, usize),
    channels: &[Channel],
    (tile_x, tile_y): (u32, u32),
    fields: Option<&'a Fields>,
    lit: bool,
) -> Vec<Node<'a>> {
    if !lit && !reads_stack(inp.project, draw.0) {
        return Vec::new();
    }

    let Ok(set) = inp.project.texture_set(inp.ts) else { return Vec::new() };
    let compare_effects = layer_mask(inp.project, draw.0).into_iter().flat_map(|mask| mask.effects.iter());
    let planned = compared_channels(compare_effects, channels.to_vec());
    plan_in(inp, &set.stack, &planned, tile_x, tile_y, 0, fields, DrawPlacement::default(), Some(draw))
}

/// Plan the stack below a draw for its anchor and Compare inputs
fn anchor_plan<'a>(
    inp: &EvaluationInputs<'a>,
    draw: (LayerId, usize),
    channels: &[Channel],
    tile_x: u32,
    tile_y: u32,
    fields: Option<&'a Fields>,
) -> Vec<Node<'a>> {
    below_plan(inp, draw, channels, (tile_x, tile_y), fields, false)
}

/// `channels` with every channel the Compare effects among `effects` compare added once
fn compared_channels<'e>(effects: impl IntoIterator<Item = &'e Effect>, mut channels: Vec<Channel>) -> Vec<Channel> {
    for effect in effects {
        if let EffectKind::Compare { channel, .. } = effect.kind
            && !channels.contains(&channel)
        {
            channels.push(channel);
        }
    }
    channels
}

/// Include material channels that masks read even when the set no longer exports them.
fn evaluation_channels(inp: &Inputs, mut channels: Vec<Channel>) -> Vec<Channel> {
    fn include_masks(project: &Project, layers: &[LayerId], depth: usize, channels: &mut Vec<Channel>) {
        if depth > texturecraft_core::project::MAX_DEPTH {
            return;
        }

        for &id in layers {
            if let Some(mask) = layer_mask(project, id).filter(|mask| mask.enabled) {
                for effect in mask.effects.iter().filter(|effect| effect.visible && effect.kind.reads_stack()) {
                    if let EffectKind::Compare { channel, .. } = effect.kind
                        && !channels.contains(&channel)
                    {
                        channels.push(channel);
                    }
                }
            }

            if let Some(layer) = project.content_layer(id).filter(|layer| !layer.children().is_empty()) {
                include_masks(project, layer.children(), depth + 1, channels);
            }
        }
    }

    if let Ok(set) = inp.project.texture_set(inp.ts) {
        include_masks(inp.project, &set.stack, 0, &mut channels);
    }

    channels
}

/// Whether layer `id` (its own mask, or the content it draws) reads the stack below it: an anchor point, or a compare mask's layer sides
pub(crate) fn reads_stack(p: &Project, id: LayerId) -> bool {
    p.layers.get(&id).is_some_and(Layer::reads_stack) || p.content_layer(id).is_some_and(Layer::reads_stack)
}

/// The mask layer `id` draws with: its own, or an instance source's.
fn layer_mask(p: &Project, id: LayerId) -> Option<&Mask> {
    p.layers.get(&p.mask_owner(id))?.mask.as_ref()
}

/// Layers of `list` in drawing order (bottom → top, folder and instanced folder contents after
/// their folder).
fn draw_order(inp: &EvaluationInputs, list: &[LayerId], depth: usize, place: DrawPlacement, visible_only: bool, out: &mut Vec<(LayerId, DrawPlacement)>) {
    if depth > texturecraft_core::project::MAX_DEPTH {
        return;
    }

    for id in list {
        out.push((*id, place));
        if visible_only && !inp.project.layers.get(id).is_some_and(|layer| layer.visible) {
            continue;
        }

        if let Some(layer) = inp.project.content_layer(*id).filter(|layer| !layer.children().is_empty()) {
            draw_order(inp, layer.children(), depth + 1, place.children(inp, *id, layer), visible_only, out);
        }
    }
}

/// What a field holds.
#[derive(Clone, Copy)]
enum Job<'a> {
    Mask(&'a Mask),
    Content(Channel),
}

/// A layer's mask or content channel after every neighbourhood filter of its stack, over `tiles`.
fn field(
    inp: &EvaluationInputs,
    channels: &[Channel],
    below: &Fields,
    (id, place): (LayerId, DrawPlacement),
    job: Job,
    tiles: &[(u32, u32)],
    (reader_margin, reader_wrap): (u32, bool),
) -> Option<Field> {
    let (effects, channel) = match job {
        Job::Mask(mask) => (&mask.effects, None),
        Job::Content(channel) => (&inp.project.content_layer(id)?.content_effects, Some(channel)),
    };
    let filters: Vec<(usize, f32, Op)> = neighbourhood_filters(effects, inp.tile_res(), channel).collect();

    // Each filter's region also holds what every filter above it reads around the tiles
    let mut margins: Vec<u32> = filters.iter().map(|&(_, radius, op)| filter_extent(radius, op, inp).1.div_ceil(TILE)).collect();
    for index in (0..margins.len().saturating_sub(1)).rev() {
        margins[index] = margins[index].saturating_add(margins[index + 1]);
    }

    // A wrapped dependency expands all lower stages across any crossed axis
    let own_wraps: Vec<bool> = filters.iter().map(|filter| wraps(effects, filter.0)).collect();
    let mut wrapping = own_wraps.clone();
    for index in (0..wrapping.len().saturating_sub(1)).rev() {
        wrapping[index] |= wrapping[index + 1];
    }

    let mut previous: Option<(usize, Field)> = None;
    let mut anchor_stages = Vec::new();
    for (((&filter, &margin), &wrap), &own_wrap) in filters.iter().zip(&margins).zip(&wrapping).zip(&own_wraps) {
        let region = region_around(inp, tiles, reader_margin.saturating_add(margin), reader_wrap || wrap)?;
        let start = previous.as_ref().map(|(index, field)| (*index, field));
        let filtered = filter_region(inp, channels, below, (id, place), job, effects, filter, own_wrap, region, start)?;

        // Preserve only stages with anchors before the next filter overwrites their value
        if let Some((previous_index, previous_field)) = previous.take() {
            let records_anchor = effects
                .iter()
                .filter(|effect| effect.visible)
                .skip(previous_index + 1)
                .take(filter.0 - previous_index - 1)
                .any(|effect| matches!(effect.kind, EffectKind::Anchor));
            if records_anchor {
                anchor_stages.push((previous_index, previous_field));
            }
        }

        previous = Some((filter.0, filtered));
    }

    previous.map(|(_, mut field)| {
        field.anchor_stages = anchor_stages;
        field
    })
}

/// The tile-aligned texel region `(x0, y0, w, h)` around `tiles` plus `margin` tiles, clipped to the set; with `wrap`, an axis the margin crosses an edge of spans the whole set (the filter reads the far side there)
fn region_around(inp: &Inputs, tiles: &[(u32, u32)], margin: u32, wrap: bool) -> Option<(u32, u32, u32, u32)> {
    let (first_x, first_y) = tiles.iter().fold((u32::MAX, u32::MAX), |(min_x, min_y), tile| (min_x.min(tile.0), min_y.min(tile.1)));
    let (last_x, last_y) = tiles.iter().fold((0, 0), |(max_x, max_y), tile| (max_x.max(tile.0), max_y.max(tile.1)));
    let axis = |first: u32, last: u32, size: u32| {
        let end = last.saturating_add(1).saturating_add(margin).saturating_mul(TILE);
        if wrap && (first < margin || end > size) {
            return (0, size);
        }

        (first.saturating_sub(margin).saturating_mul(TILE), end.min(size))
    };
    let ((x0, x1), (y0, y1)) = (axis(first_x, last_x, inp.gbuf.width), axis(first_y, last_y, inp.gbuf.height));
    let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    (w > 0 && h > 0).then_some((x0, y0, w, h))
}

/// Whether the neighbourhood filter at `index` among the visible effects reads past the set's edges from the far side
fn wraps(effects: &[Effect], index: usize) -> bool {
    matches!(
        effects.iter().filter(|effect| effect.visible).nth(index).map(|effect| &effect.kind),
        Some(EffectKind::Filter(
            Filter::Blur { wrap: true, .. }
                | Filter::Bevel { wrap: true, .. }
                | Filter::DistanceBevel(Bevel { wrap: true, .. })
                | Filter::BevelSmooth(BevelSmooth { wrap: true, .. })
        ))
    )
}

/// `data` (`w` × `h` texels of `comps` values) with `pad_x` and `pad_y` texels added around it, read from the opposite side
fn wrap_pad(data: &[f32], comps: usize, (w, h): (usize, usize), (pad_x, pad_y): (usize, usize)) -> Vec<f32> {
    let (padded_w, padded_h) = (w + 2 * pad_x, h + 2 * pad_y);
    let mut out = vec![0.0f32; padded_w * padded_h * comps];
    out.par_chunks_mut(padded_w * comps).enumerate().for_each(|(y, row)| {
        let source_y = (y + h * pad_y.div_ceil(h.max(1)) - pad_y) % h.max(1);
        for (x, pixel) in row.chunks_mut(comps).enumerate() {
            let source_x = (x + w * pad_x.div_ceil(w.max(1)) - pad_x) % w.max(1);
            let at = (source_y * w + source_x) * comps;
            if let Some(source) = data.get(at..at + comps) {
                pixel.copy_from_slice(source);
            }
        }
    });
    out
}

/// The `w` × `h` texels inside [`wrap_pad`]'s border
fn unpad(data: &[f32], comps: usize, (w, h): (usize, usize), (pad_x, pad_y): (usize, usize)) -> Vec<f32> {
    let padded_w = w + 2 * pad_x;
    (pad_y..pad_y + h).flat_map(|y| data.get((y * padded_w + pad_x) * comps..(y * padded_w + pad_x + w) * comps).unwrap_or_default()).copied().collect()
}

/// A neighbourhood filter's radius in whole texels and how many texels around a texel it reads
fn filter_extent(radius: f32, op: Op, inputs: &Inputs) -> (u32, u32) {
    let longest_side = inputs.gbuf.width.max(inputs.gbuf.height);
    // Clamped so a hostile radius can't overflow the region arithmetic
    let radius_texels = radius.round().clamp(1.0, longest_side as f32) as u32;

    // Three box passes reach three radii; the gathering filters one radius plus a bilinear texel
    let reach = if is_box_blur(op) { 3 * radius_texels } else { radius_texels + 1 };

    // A shadow also reads the shapes it was moved from
    let moved = match op {
        Op::Shadow { offset: [dx, dy] } => dx.unsigned_abs().max(dy.unsigned_abs()).min(longest_side),
        _ => 0,
    };

    // Repeating a sampled source can read the opposite end of the output's UV tile
    let tiled_source = match op {
        Op::SourceWarp(warp) => Some(warp.source),
        Op::SourceSlopeBlur(slope) => Some(slope.source),
        _ => None,
    };
    let source_reach = if tiled_source.is_some_and(|source| source.mode != WarpSourceMode::DefaultNoise && source.tiling_factor() != 1.0) {
        inputs.tile_grid().map_or(longest_side, |_| inputs.tile_res())
    } else {
        0
    };

    (radius_texels, (reach + moved).max(source_reach))
}

/// Whether neighbourhood filter `op` is computed from box blurs (the others gather samples)
fn is_box_blur(op: Op) -> bool {
    matches!(op, Op::Blur(_) | Op::HighPass { .. } | Op::Bevel | Op::Glow(_) | Op::PreBlur | Op::Halo | Op::Shadow { .. } | Op::Outline(_) | Op::Distance)
}

/// The stack up to and including neighbourhood filter `filter` over texel `region`, continued from `start` (the filter below's field) when given; `wrap` when the filter reads past the set's edges from the far side
#[allow(clippy::too_many_arguments)]
fn filter_region(
    inputs: &EvaluationInputs,
    channels: &[Channel],
    below: &Fields,
    (id, place): (LayerId, DrawPlacement),
    job: Job,
    effects: &[Effect],
    (index, radius, op): (usize, f32, Op),
    wrap: bool,
    (region_x, region_y, width, height): (u32, u32, u32, u32),
    start: Option<(usize, &Field)>,
) -> Option<Field> {
    let channel = match job {
        Job::Mask(_) => None,
        Job::Content(channel) => Some(channel),
    };

    let filter_effect = effects.iter().filter(|effect| effect.visible).nth(index);
    let filter = filter_effect.and_then(|effect| match &effect.kind {
        EffectKind::Filter(filter) => Some(filter),
        _ => None,
    });
    let opacity = filter_effect.map(|effect| effect.applied_blend(channel).opacity.clamp(0.0, 1.0)).unwrap_or(1.0);

    // Height To Normal makes the Normal only (and reads the layer's Height as its gray); other channels pass
    let heights = matches!(op, Op::HeightToNormal(_)) && channel == Some(Channel::Normal);
    let opacity = if matches!(op, Op::HeightToNormal(_)) && !heights { 0.0 } else { opacity };

    // Color Match moves colors only; other channels pass
    let opacity = if matches!(op, Op::ColorMatch(_)) && channel.map(Channel::kind) != Some(ChannelKind::Color) { 0.0 } else { opacity };
    let (radius_texels, reach) = filter_extent(radius, op, inputs);

    // Generator stages keep their ramp beside the mask stack below
    let generator = match (job, filter_effect.map(|effect| (&effect.kind, effect.blend))) {
        (Job::Mask(_), Some((EffectKind::Generator(generator), blend))) => Some((generator, blend)),
        _ => None,
    };
    let components = match job {
        Job::Mask(_) if generator.is_some() => GENERATOR_STAGE_COMPONENTS,
        Job::Mask(_) => 1,
        Job::Content(_) => 4,
    };

    // The gray the filter's image input reads (its first; the filters that read one have one)
    let input = filter_effect.zip(filter).and_then(|(effect, filter)| {
        // Color Match reads each input as linear color separately
        if matches!(filter, Filter::ColorMatch(_)) {
            return None;
        }

        ImageInput::of(filter).first().map(|input| effect.input(input))
    });

    let lit_prefix =
        channel == Some(Channel::BaseColor) && effects.iter().filter(|effect| effect.visible).take(index).any(|effect| lights_material(&effect.kind));

    // Evaluate the stack up to the filter, tile by tile (content premultiplied by coverage), and its image input
    let (region_right, region_bottom) = (region_x + width, region_y + height);
    let texel_count = width as usize * height as usize;
    let mut stack_values = vec![0.0f32; texel_count * components];
    let mut cover = vec![0.0f32; texel_count];
    let band = TILE as usize * width as usize;

    let sample_band = |row: usize, (values, coverage): (&mut [f32], &mut [f32]), grays: &mut [f32]| {
        let tile_y = region_y / TILE + row as u32;
        for tile_x in region_x / TILE..region_right.div_ceil(TILE) {
            let draw = (id, place.instance);
            let lit_layer =
                if lit_prefix { plan_node(inputs, id, channels, tile_x, tile_y, 0, Some(below), false, place, None).filter(|node| node.lights) } else { None };
            let anchor_nodes = below_plan(inputs, draw, channels, (tile_x, tile_y), Some(below), lit_layer.is_some());
            let preview_tile = inputs.preview.and_then(|preview| preview.alpha.tile(tile_x, tile_y));
            let height_nodes = if heights { plan_in(inputs, &[id], &[Channel::Height], tile_x, tile_y, 0, Some(below), place, None) } else { Vec::new() };
            let mut mask = None;
            let mut nodes = Vec::new();
            let mut generator_maps = GeneratorMaps::default();
            match job {
                Job::Mask(layer_mask) => {
                    // The layer's own filtered content, for a compare of it (this mask's field is not built yet)
                    let mut mask_stack = mask_plan(inputs, id, layer_mask, tile_x, tile_y, Some(below), place);

                    generator_maps = mask_stack.effects.get(index).map(|effect| effect.maps.clone()).unwrap_or_default();
                    mask_stack.effects.truncate(index);
                    mask_stack.start = start;
                    mask = Some(mask_stack);
                }
                Job::Content(channel) => {
                    nodes = plan_in(inputs, &[id], &[channel], tile_x, tile_y, 0, Some(below), place, None);
                    if let Some(channel_plan) = nodes.first_mut().and_then(|node| node.chans.first_mut()) {
                        channel_plan.effects.truncate(index);
                        channel_plan.start = start;
                    }
                }
            }
            for (x, y, offset) in tile_texels(inputs.gbuf, tile_x, tile_y) {
                let mut texel = Texel::new(inputs, x, y, offset, stroke_at(preview_tile, offset));
                if !anchor_nodes.is_empty() {
                    texel.anchors_below(&anchor_nodes, (id, place.instance));
                }

                if let Some(layer) = &lit_layer {
                    texel.light(layer);
                }

                let value = match (&mask, nodes.first()) {
                    (Some(mask_stack), _) => {
                        let ramp = generator.map_or(0.0, |(generator, _)| texel.generator_ramp(generator, &generator_maps));
                        let texture = generator.and_then(|(generator, _)| texel.generator_texture(generator)).unwrap_or(0.0);
                        [texel.mask(mask_stack), ramp, texture, 0.0]
                    }
                    (None, Some(node)) => match node.chans.first().and_then(|channel_plan| texel.content(&node.content, channel_plan)) {
                        Some((color, coverage)) => [color[0] * coverage, color[1] * coverage, color[2] * coverage, coverage],
                        None => [0.0; 4],
                    },
                    (None, None) => [0.0; 4],
                };
                let offset_in_band = ((y % TILE) * width + (x - region_x)) as usize;
                if let Some(gray) = grays.get_mut(offset_in_band) {
                    *gray = match height_nodes.first() {
                        // Coverage brings signed height down to zero at the layer's edge
                        Some(node) => {
                            let mut height = Texel::new(inputs, x, y, offset, stroke_at(preview_tile, offset));
                            if !anchor_nodes.is_empty() {
                                height.anchors_below(&anchor_nodes, draw);
                            }

                            node.chans.first().and_then(|channel| height.content(&node.content, channel)).map_or(0.0, |(value, coverage)| value[0] * coverage)
                        }
                        None => input
                            .as_ref()
                            .map_or(1.0, |source| texel.source(source, &Projection::default(), Sampling::default(), ChannelKind::Scalar)[0])
                            .clamp(0.0, 1.0),
                    };
                }

                coverage[offset_in_band] = 1.0;
                values[offset_in_band * components..(offset_in_band + 1) * components].copy_from_slice(&value[..components]);
            }
        }
    };
    let bands = stack_values.par_chunks_mut(band * components).zip(cover.par_chunks_mut(band)).enumerate();
    let mut grays = Vec::new();
    if heights || input.is_some() {
        grays.resize(texel_count, 0.0);
        bands.zip(grays.par_chunks_mut(band)).for_each(|((row, values), gray)| sample_band(row, values, gray));
    } else {
        bands.for_each(|(row, values)| sample_band(row, values, &mut []));
    }

    // Color Match is independent of UV tile scale and can transform its input in place
    if let Op::ColorMatch(mut matcher) = op {
        if let Some(effect) = filter_effect.filter(|_| opacity > 0.0) {
            let read = |enabled: bool, input: ImageInput, uniform: &mut [f32; 3]| {
                if !enabled {
                    return None;
                }

                match effect.inputs.get(input.id) {
                    None => *uniform = [input.default; 3],
                    Some(Source::Uniform(color)) => *uniform = *color,
                    Some(source) => return Some(input_colors(inputs, source, ((id, place.instance), channels, below), (region_x, region_y, width, height))),
                }
                None
            };
            let sources = read(matcher.source_mode == SourceColorMode::Input, ImageInput::SOURCE_INPUT, &mut matcher.source_color);
            let targets = read(matcher.target_mode == TargetColorMode::ImageInput, ImageInput::TARGET_INPUT, &mut matcher.target_color);
            color_match(&mut stack_values, components, &matcher, (&sources, &targets), opacity);
        }

        unpremultiply(&mut stack_values, components);
        return Some(Field { x0: region_x, y0: region_y, w: width, h: height, comps: components, data: stack_values, anchor_stages: Vec::new() });
    }

    // High pass centers on the channel's middle value (0 for signed channels like height)
    let middle = match job {
        Job::Content(channel) if channel.kind() == ChannelKind::Signed => 0.0,
        _ => 0.5,
    };

    // Filter High Pass in its working space and preserve color when only coverage changes
    let (space, highpass_alpha) = match (op, job) {
        (Op::HighPass { space, alpha }, Job::Content(channel)) if components == 4 => {
            (if channel.kind() == ChannelKind::Color { space } else { HighpassSpace::Working }, alpha)
        }
        _ => (HighpassSpace::Working, false),
    };
    let covered = highpass_alpha.then(|| stack_values.clone());
    recode(&mut stack_values, components, |rgb| space.encode(rgb));

    // A periodic box reads the original line directly on each pass
    let wrap_axes = (wrap && region_x == 0 && width == inputs.gbuf.width, wrap && region_y == 0 && height == inputs.gbuf.height);
    let pad_x = if matches!(op, Op::Distance) && wrap_axes.0 { reach.min(width) } else { 0 };
    let pad_y = if matches!(op, Op::Distance) && wrap_axes.1 { reach.min(height) } else { 0 };

    // A smaller UV tile has fewer texels per UV unit, so its filters reach fewer texels
    let (block_size, tiled) = match inputs.tile_grid() {
        Some(_) => ([inputs.tile_res(); 2], true),
        None => ([inputs.gbuf.width, inputs.gbuf.height], false),
    };
    let filtered = |scale: f32| -> Vec<f32> {
        let tile_resolution = ((inputs.tile_res() as f32 * scale).round() as u32).max(1);
        if let Some((generator, blend)) = generator {
            let texture_set = inputs.project.texture_set(inputs.ts).map_or(inputs.tile_res(), |set| set.resolution);
            return generator_region(generator, blend, opacity, &stack_values, &cover, (region_x, region_y, width, height), (tile_resolution, texture_set));
        }

        let scaled = neighbourhood_filters(effects, tile_resolution, channel).find(|filter| filter.0 == index);
        let scaled_radius = scaled.map_or(radius_texels, |filter| filter.1.round().clamp(1.0, radius_texels as f32) as u32);
        if !is_box_blur(op) {
            let tile_size = if tiled { [tile_resolution; 2] } else { block_size };
            return gather(
                op,
                (&stack_values, &cover, &grays),
                components,
                (region_x, region_y, width, height),
                scaled_radius as f32,
                (tile_resolution, block_size, tile_size),
                opacity,
            );
        }

        // A distance transform needs samples from the opposite edge
        let pad = (pad_x as usize, pad_y as usize);
        let unpadded = (width as usize, height as usize);
        let (stack_values, cover, grays) = if pad == (0, 0) {
            (Cow::Borrowed(&stack_values), Cow::Borrowed(&cover), Cow::Borrowed(&grays))
        } else {
            let padded = |values: &[f32], components: usize| Cow::Owned(wrap_pad(values, components, unpadded, pad));
            (padded(&stack_values, components), padded(&cover, 1), if grays.is_empty() { Cow::Borrowed(&grays) } else { padded(&grays, 1) })
        };
        let (columns, rows, blur_radius) = (unpadded.0 + 2 * pad.0, unpadded.1 + 2 * pad.1, scaled_radius as usize);
        if let (Some((_, _, scaled_op @ (Op::Halo | Op::Shadow { .. }))), Some(filter)) = (scaled, filter) {
            let kind = channel.map_or(ChannelKind::Scalar, Channel::kind);
            let mut lit_values = stack_values.to_vec();
            halo(&mut lit_values, &cover, components, (columns, rows, blur_radius), (scaled_op, filter, kind), opacity);
            return if pad == (0, 0) { lit_values } else { unpad(&lit_values, components, unpadded, pad) };
        }

        if let (Some((_, _, Op::Distance)), Some(filter)) = (scaled, filter) {
            let mut distance_values = stack_values.to_vec();
            distance_filter(&mut distance_values, (&cover, &grays), components, (columns, rows), filter, tile_resolution, opacity);
            return if pad == (0, 0) { distance_values } else { unpad(&distance_values, components, unpadded, pad) };
        }

        if let Some((_, _, Op::Outline(outline))) = scaled {
            let mut outline_values = stack_values.to_vec();
            mask_outline(&mut outline_values, &cover, components, (columns, rows, blur_radius), &outline, opacity);
            return if pad == (0, 0) { outline_values } else { unpad(&outline_values, components, unpadded, pad) };
        }

        // A pre-blur takes the blur whole; the effect's opacity mixes its mapped result below
        let blurred_opacity = if matches!(op, Op::PreBlur) { 1.0 } else { opacity };
        let mut weights = cover.to_vec();
        blur_with_wrap(&mut weights, columns, rows, blur_radius, wrap_axes);
        let mut filtered_values = stack_values.to_vec();
        for component in 0..components {
            let mut sums: Vec<f32> = stack_values.par_chunks(components).map(|pixel| pixel.get(component).copied().unwrap_or(0.0)).collect();
            blur_with_wrap(&mut sums, columns, rows, blur_radius, wrap_axes);
            filtered_values.par_chunks_mut(components).zip(sums.par_iter()).zip(weights.par_iter()).for_each(|((pixel, sum), weight)| {
                // Content is premultiplied, so the middle value is too (coverage itself is kept)
                let coverage = if components == 4 { pixel.get(3).copied().unwrap_or(0.0) } else { 1.0 };
                if *weight > MIN_READ_WEIGHT
                    && let Some(slot) = pixel.get_mut(component)
                {
                    let (original, blurred) = (*slot, sum / weight);
                    let result = match op {
                        Op::Blur(Some(amount)) => original + amount * (original - blurred),
                        Op::HighPass { .. } if component < 3 => original - blurred + middle * coverage,
                        Op::HighPass { alpha: true, .. } => (original - blurred + 0.5).clamp(0.0, 1.0),

                        // Away from 0 by at most the value itself: 0 at the edge, full inside
                        Op::Bevel if component < 3 => original.signum() * original.abs().min((2.0 * blurred * original.signum() - original.abs()).max(0.0)),
                        Op::Glow(amount) if component < 3 => original + amount * blurred,
                        Op::HighPass { .. } | Op::Bevel | Op::Glow(_) => original,
                        _ => blurred,
                    };
                    *slot = original + (result - original) * blurred_opacity;
                }
            });
        }
        if matches!(op, Op::PreBlur)
            && let Some(filter) = filter
        {
            let kind = channel.map_or(ChannelKind::Scalar, Channel::kind);
            map_blurred(&mut filtered_values, &stack_values, components, filter, inputs.gradient(filter).as_deref(), kind, opacity);
        }
        if pad == (0, 0) { filtered_values } else { unpad(&filtered_values, components, unpadded, pad) }
    };

    // Each block keeps the result at its tile's scale (blocks are TILE aligned)
    let scales = &inputs.block_scales;
    let mut levels: Vec<f32> = (region_y..region_bottom)
        .step_by(TILE as usize)
        .flat_map(|y| (region_x..region_right).step_by(TILE as usize).map(move |x| (x, y)))
        .map(|(x, y)| scales.at(x, y))
        .collect();
    levels.sort_by(f32::total_cmp);
    levels.dedup();
    let (first, rest) = levels.split_first().map_or((1.0, &[][..]), |(first, rest)| (*first, rest));

    // A generator stage's field is the mask alone
    let field_components = if generator.is_some() { 1 } else { components };
    let mut result = filtered(first);
    for &scale in rest {
        let rescaled = filtered(scale);
        let row = width as usize * field_components;
        result.par_chunks_mut(row).zip(rescaled.par_chunks(row)).enumerate().for_each(|(y, (target_row, source_row))| {
            for (x, (target, source)) in target_row.chunks_mut(field_components).zip(source_row.chunks(field_components)).enumerate() {
                if scales.at(region_x + x as u32, region_y + y as u32) == scale {
                    target.copy_from_slice(source);
                }
            }
        });
    }

    // Laid on the new coverage first, so the decode divides by the alpha the colours now carry
    if let Some(before) = covered {
        recover(&mut result, &before, middle);
    }
    recode(&mut result, field_components, |rgb| space.decode(rgb));
    unpremultiply(&mut result, field_components);
    Some(Field { x0: region_x, y0: region_y, w: width, h: height, comps: field_components, data: result, anchor_stages: Vec::new() })
}

/// Where working texel (`x`, `y`) of `inp` is, for pattern evaluation (as [`Texel::coords`] sees it outside a fill copy)
pub(crate) fn texel_coords(inp: &Inputs, x: u32, y: u32) -> Coords {
    let gbuf = inp.gbuf;
    let mut coords = match gbuf.surface(inp.mesh, (y * gbuf.width + x) as usize) {
        Some(surface) => {
            let (center, radius) = mesh_frame(inp.bounds);
            Coords { uv: surface.uv, pos: (surface.pos - center) / radius, nrm: surface.nrm }
        }
        None => Coords { uv: Vec2::new((x as f32 + 0.5) / gbuf.width as f32, (y as f32 + 0.5) / gbuf.height as f32), pos: Vec3::ZERO, nrm: Vec3::Y },
    };

    // UV-tiled sets: projections see the mesh's own UVs (patterns repeat per tile)
    if let Ok(set) = inp.project.texture_set(inp.ts)
        && let Some(grid) = set.tile_grid()
    {
        coords.uv = Vec2::from(set.from_atlas(&grid, coords.uv.to_array()));
    }
    coords
}

/// What a generator's texture or custom grunge reads with nothing in its slot
const EMPTY_IMAGE_SLOT: Source = Source::Uniform([0.0; 3]);

/// The step Safe Rotation turns a Grayscale Conversion image by, so it keeps tiling seamlessly
const QUARTER_TURN_DEGREES: f32 = 90.0;

/// Values per texel of a generator stage's region: the stack below, the generator's ramp and its texture's gray
const GENERATOR_STAGE_COMPONENTS: usize = 3;

/// The mask over a region after a generator at its own resolution and blur: `stack` is (stack below, generator ramp, texture gray) per texel, sizes in texels per side
fn generator_region(
    generator: &Generator,
    blend: BlendMode,
    opacity: f32,
    stack: &[f32],
    cover: &[f32],
    (region_x, region_y, width, height): (u32, u32, u32, u32),
    (uv_tile, texture_set): (u32, u32),
) -> Vec<f32> {
    let (columns, rows) = (width as usize, height as usize);
    let mut ramps: Vec<f32> = stack.par_chunks(GENERATOR_STAGE_COMPONENTS).map(|texel| texel.get(1).copied().unwrap_or(0.0)).collect();

    // Coarser than the UV tile: averaged over cells, then the blur around them
    let cell = generator.resolution.cell(uv_tile, texture_set) as usize;
    if cell > 1 {
        ramps = resampled_by_cells(&ramps, cover, (region_x as usize, region_y as usize, columns, rows), cell);
    }

    // The section controls act on the signal before the global blur, their own blur first
    if let Some(section) = generator.section_controls() {
        blur_over_cover(&mut ramps, cover, (columns, rows), sections::blur_radius_at(generator.kind, &section, uv_tile));
        ramps.par_iter_mut().for_each(|signal| *signal = sections::adjusted(generator.kind, &section, *signal));
    }

    // The texture blends over the adjusted signal, before the global blur
    if let Some(texture) = generator.texture_in_use() {
        ramps.par_iter_mut().zip(stack.par_chunks(GENERATOR_STAGE_COMPONENTS)).for_each(|(signal, texel)| {
            *signal = texture_layer::blended_over(texture, *signal, texel.get(2).copied().unwrap_or(0.0));
        });
    }

    blur_over_cover(&mut ramps, cover, (columns, rows), generator.applied_global().blur_radius_at(uv_tile).round() as usize);

    // Finished and blended over the stack below, like the generator effect itself
    stack
        .par_chunks(GENERATOR_STAGE_COMPONENTS)
        .zip(ramps.par_iter())
        .map(|(texel, ramp)| {
            let below = texel.first().copied().unwrap_or(0.0);
            let generated = finish_generator(generator, *ramp);
            below + (blend1(blend, below, generated) - below) * opacity
        })
        .collect()
}

/// `values` blurred by three box passes of `radius`, weighted by `cover` so uncovered texels don't bleed in
fn blur_over_cover(values: &mut [f32], cover: &[f32], (columns, rows): (usize, usize), radius: usize) {
    if radius == 0 {
        return;
    }

    let mut weights = cover.to_vec();
    blur(&mut weights, columns, rows, radius);
    let mut sums: Vec<f32> = values.iter().zip(cover).map(|(value, covered)| value * covered).collect();
    blur(&mut sums, columns, rows, radius);
    values.par_iter_mut().zip(sums.par_iter().zip(weights.par_iter())).for_each(|(value, (sum, weight))| {
        if *weight > MIN_READ_WEIGHT {
            *value = sum / weight;
        }
    });
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
            sums.into_iter().map(|(sum, weight)| (weight > MIN_READ_WEIGHT).then(|| sum / weight))
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
            if weight > MIN_READ_WEIGHT { sum / weight } else { values.get(y * width + x).copied().unwrap_or(0.0) }
        })
        .collect()
}

/// A generator's (possibly blurred) ramp after balance, contrast, invert and offset (the light controls invert their shading themselves)
fn finish_generator(generator: &Generator, ramp: f32) -> f32 {
    let value = generator.applied_global().apply(ramp);
    let value = if generator.invert && generator.light.is_none() { 1.0 - value } else { value };
    let offset = if generator.offset.is_finite() { generator.offset } else { 0.0 };
    (value + offset).clamp(0.0, 1.0)
}

/// Straight colours a colour space map must leave alone to be the identity
const IDENTITY_PROBE: [f32; 3] = [0.25, 0.5, 0.75];

/// Premultiplied content `data` (four components a texel) with each straight color mapped by `map` (nothing for masks or an identity space)
fn recode(data: &mut [f32], comps: usize, map: impl Fn([f32; 3]) -> [f32; 3] + Sync) {
    if comps != 4 || map(IDENTITY_PROBE) == IDENTITY_PROBE {
        return;
    }

    data.par_chunks_mut(4).for_each(|pixel| {
        if let [r, g, b, a] = pixel
            && *a > MIN_READ_WEIGHT
        {
            let mapped = map([*r / *a, *g / *a, *b / *a]);
            [*r, *g, *b] = mapped.map(|component| component * *a);
        }
    });
}

/// Premultiplied content `data` whose coverage a Highpass changed: its colors laid on the new coverage (mid gray where there was none, as `before`)
fn recover(data: &mut [f32], before: &[f32], mid: f32) {
    data.par_chunks_mut(4).zip(before.par_chunks(4)).for_each(|(pixel, old)| {
        let (Some(&own), Some(&new)) = (old.get(3), pixel.get(3)) else { return };
        let scale = if own > MIN_READ_WEIGHT { new / own } else { 0.0 };
        for component in pixel.iter_mut().take(3) {
            *component = if own > MIN_READ_WEIGHT { *component * scale } else { mid * new };
        }
    });
}

/// Below this cover a texel is empty and has no straight colour
pub(crate) const MIN_ALPHA: f32 = 1e-5;

/// The blurred value a precise halo is solid from
const PRECISE_EDGE: f32 = 0.5;

/// How soft a full spread leaves a halo: its least hardness
const HALO_SOFTEST: f32 = 0.01;

/// The shapes' blurred share at their edge
const OUTLINE_EDGE_SHARE: f32 = 0.5;

/// Lay halo glow or drop shadow `filter` (`op` its move) over premultiplied `data` in the tint of a channel of `kind`, mixed in by `opacity`
fn halo(
    data: &mut [f32],
    cover: &[f32],
    comps: usize,
    (width, height, radius): (usize, usize, usize),
    (op, filter, kind): (Op, &Filter, ChannelKind),
    opacity: f32,
) {
    let (inner, precise, spread, strength, color, mask_white) = match *filter {
        Filter::HaloGlow { on_mask, precise, width, spread, opacity, color } => (width < 0.0, precise, spread, opacity, color, on_mask),
        Filter::DropShadow { spread, opacity, color, .. } => (false, false, spread, opacity, color, true),
        _ => return,
    };

    // Rec. 709 luminance, unlike `lum`'s weights on purpose
    let luminance = 0.2126 * color[0] + 0.7152 * color[1] + 0.0722 * color[2];
    let tint = match (comps, kind) {
        (4, ChannelKind::Color) => color,
        (4, _) => [luminance; 3],
        _ if mask_white => [1.0; 3],
        _ => [luminance; 3],
    };

    // The shapes: the content's coverage or the mask's value (the outside of them for an inner glow)
    let shape = |texel: usize| {
        let value = data.get(texel * comps + comps - 1).copied().unwrap_or(0.0);
        let value = if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 };
        if inner { 1.0 - value } else { value }
    };
    let offset = match op {
        Op::Shadow { offset } => offset,
        _ => [0, 0],
    };
    let moved = |texel: usize| -> Option<usize> {
        let (x, y) = ((texel % width.max(1)) as i64 - i64::from(offset[0]), (texel / width.max(1)) as i64 - i64::from(offset[1]));
        (x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height).then(|| y as usize * width + x as usize)
    };
    let texels = width * height;
    let mut weights: Vec<f32> = (0..texels).map(|texel| moved(texel).and_then(|from| cover.get(from).copied()).unwrap_or(0.0)).collect();
    let mut sums: Vec<f32> = (0..texels).map(|texel| moved(texel).map_or(0.0, |from| shape(from) * cover.get(from).copied().unwrap_or(0.0))).collect();
    blur(&mut weights, width, height, radius);
    blur(&mut sums, width, height, radius);

    let hardness = (1.0 - spread.clamp(0.0, 1.0)).max(HALO_SOFTEST);
    let strength = strength.clamp(0.0, 1.0);
    data.par_chunks_mut(comps).zip(sums.par_iter().zip(weights.par_iter())).for_each(|(pixel, (sum, weight))| {
        let blurred = if *weight > MIN_READ_WEIGHT { (sum / weight).clamp(0.0, 1.0) } else { 0.0 };
        let shaped = if precise { crate::organic::smoothstep(0.0, PRECISE_EDGE, blurred) } else { blurred };
        let glow = (shaped / hardness).min(1.0) * strength;
        let cov = pixel.last().copied().unwrap_or(0.0).clamp(0.0, 1.0);
        let mut before = [0.0f32; 4];
        for (was, value) in before.iter_mut().zip(pixel.iter()) {
            *was = *value;
        }
        if inner {
            // Inside the shapes, towards the tint (content premultiplied by its coverage)
            let weight = if comps == 4 { glow } else { glow * cov };
            for (slot, tint) in pixel.iter_mut().zip(tint).take(comps.min(3)) {
                let target = if comps == 4 { tint * cov } else { tint };
                *slot += (target - *slot) * weight;
            }
        } else {
            // Under the shapes: it shows where they leave room
            let under = glow * (1.0 - cov);
            for (slot, tint) in pixel.iter_mut().zip(tint).take(comps.min(3)) {
                *slot += tint * under;
            }
            if comps == 4
                && let Some(alpha) = pixel.get_mut(3)
            {
                *alpha += under;
            }
        }
        for (slot, was) in pixel.iter_mut().zip(before) {
            *slot = was + (*slot - was) * opacity;
        }
    });
}

/// Lay Mask Outline `outline` over premultiplied `data` (shapes of content from its luminance), mixed in by `opacity`
fn mask_outline(data: &mut [f32], cover: &[f32], comps: usize, (width, height, radius): (usize, usize, usize), outline: &MaskOutline, opacity: f32) {
    let value = |pixel: &[f32]| match pixel {
        [r, g, b, alpha] if *alpha > MIN_ALPHA => lum([r / alpha, g / alpha, b / alpha]),
        [value] => *value,
        _ => 0.0,
    };
    let inside: Vec<bool> = data.par_chunks(comps).zip(cover.par_iter()).map(|(pixel, covered)| *covered > 0.0 && outline.inside(value(pixel))).collect();
    let fade_curve = outline.fade_curve();
    let mut weights = cover.to_vec();
    let mut sums: Vec<f32> = inside.par_iter().zip(cover.par_iter()).map(|(inside, covered)| if *inside { *covered } else { 0.0 }).collect();
    blur(&mut weights, width, height, radius);
    blur(&mut sums, width, height, radius);
    data.par_chunks_mut(comps).zip(inside.par_iter()).zip(sums.par_iter().zip(weights.par_iter())).for_each(|((pixel, inside), (sum, weight))| {
        // The shapes' share around a texel is a half at their edge and none (outside) or all (inside) past the band
        let share = if *weight > MIN_READ_WEIGHT { (sum / weight).clamp(0.0, 1.0) } else { 0.0 };
        let closeness = if *inside { (1.0 - share) / OUTLINE_EDGE_SHARE } else { share / OUTLINE_EDGE_SHARE };
        let band = outline.band(fade_curve, *inside, closeness);

        // Content is premultiplied by its coverage, which stays
        let cov = if comps == 4 { pixel.get(3).copied().unwrap_or(0.0) } else { 1.0 };
        for slot in pixel.iter_mut().take(comps.min(3)) {
            let before = if comps == 4 { if cov > MIN_ALPHA { *slot / cov } else { 0.0 } } else { *slot };
            let result = outline.apply(before, band);
            *slot = (before + (result - before) * opacity) * cov;
        }
    });
}

/// Run point filter `filter` on blurred texels `data` (premultiplied content or masks) and mix the result over the unblurred `orig` by `opacity`
fn map_blurred(data: &mut [f32], orig: &[f32], comps: usize, filter: &Filter, gradient: Option<&PreparedGradient>, kind: ChannelKind, opacity: f32) {
    let point = |value| match gradient {
        Some(gradient) => gradient.point(kind, value),
        None => filter.point(kind, value),
    };
    let straight = |value: f32, cover: f32| if cover > MIN_ALPHA { value / cover } else { 0.0 };

    data.par_chunks_mut(comps).zip(orig.par_chunks(comps)).for_each(|(pixel, original)| match (pixel, original) {
        ([value], [before]) => {
            let mapped = point([*value; 3]).map_or(*value, |gray| gray[0]);
            *value = before + (mapped - before) * opacity;
        }
        ([r, g, b, cover], [r0, g0, b0, cover0]) => {
            let before = [straight(*r0, *cover0), straight(*g0, *cover0), straight(*b0, *cover0)];
            let blurred = [straight(*r, *cover), straight(*g, *cover), straight(*b, *cover)];
            let mixed = mix3(before, point(blurred).unwrap_or(before), opacity);
            [*r, *g, *b, *cover] = [mixed[0] * cover0, mixed[1] * cover0, mixed[2] * cover0, *cover0];
        }
        _ => {}
    });
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
pub fn neighbourhood_fields(inputs: &Inputs, channels: &[Channel], tiles: &[(u32, u32)]) -> Fields {
    let projectors = ProjectorDepthCache::default();
    let mut inputs = EvaluationInputs::new(inputs, &projectors);
    prepare_generators(&mut inputs, None);
    evaluation_fields(&inputs, &evaluation_channels(&inputs, channels.to_vec()), tiles)
}

fn evaluation_fields(inp: &EvaluationInputs, channels: &[Channel], tiles: &[(u32, u32)]) -> Fields {
    let stack = inp.project.texture_set(inp.ts).map(|s| s.stack.as_slice()).unwrap_or_default();
    fields_of(inp, channels, (stack, ContentsOffset::NONE), tiles)
}

/// [`neighbourhood_fields`] of the layers drawn by `list` only, held by folders whose Scale and Rotation make `around`
fn fields_of(inp: &EvaluationInputs, channels: &[Channel], (list, around): (&[LayerId], ContentsOffset), tiles: &[(u32, u32)]) -> Fields {
    let mut out = Fields::new();
    if tiles.is_empty() || inp.gbuf.width == 0 {
        return out;
    }

    let working_width = inp.gbuf.width;
    let mut order = Vec::new();
    draw_order(inp, list, 0, around.into(), false, &mut order);
    let ids: Vec<LayerId> = order.iter().map(|(id, _)| *id).collect();
    prepare_curvature_sizes(inp, &ids);

    let field_mask =
        |id, place: DrawPlacement| layer_mask(inp.project, id).filter(|mask| mask.enabled || (place.instance == 0 && inp.sampled_disabled_mask == Some(id)));

    // Walk backwards to find the region each Compare needs below its position in the stack
    let mut below_margin = (0u32, false);
    let margins: Vec<_> = order
        .iter()
        .rev()
        .map(|&(id, place)| {
            let mask_margin = below_margin;
            let effects = field_mask(id, place).map(|mask| mask.effects.as_slice()).unwrap_or_default();
            let first_reader = effects.iter().filter(|effect| effect.visible).position(|effect| effect.kind.reads_stack());
            if let Some(first_reader) = first_reader {
                for (index, radius, op) in neighbourhood_filters(effects, inp.tile_res(), None).filter(|(index, _, _)| *index > first_reader) {
                    let reach = filter_extent(radius, op, inp).1;
                    below_margin.0 = below_margin.0.saturating_add(reach.div_ceil(TILE));
                    below_margin.1 |= wraps(effects, index);
                }
            }

            (mask_margin, below_margin)
        })
        .collect();

    for ((id, place), (mask_margin, content_margin)) in order.into_iter().zip(margins.into_iter().rev()) {
        let Some(layer) = inp.project.content_layer(id) else { continue };
        let mask = field_mask(id, place).filter(|mask| neighbourhood(&mask.effects, working_width).is_some());

        // A mask comparing its own layer needs that layer's filtered content first
        let compares_content = mask.is_some_and(|mask| mask.effects.iter().any(|effect| effect.visible && compares_this_layer(&effect.kind)));
        if !compares_content {
            insert_mask_field(inp, channels, &mut out, (id, place), mask, tiles, mask_margin);
        }

        if !layer.is_group() && neighbourhood(&layer.content_effects, working_width).is_some() {
            for channel in channels.iter().filter(|channel| layer.channels.contains_key(channel)) {
                let mut key = FieldKey::content(id, *channel, place.offset);
                if place.instance != 0 {
                    let reads_anchor = matches!(&layer.kind, LayerKind::Fill(fill) if fill.sources.get(channel).is_some_and(|source| source.reads_anchor(None)))
                        || layer
                            .content_effects
                            .iter()
                            .any(|effect| effect.visible && (effect.reads(None) || (*channel == Channel::BaseColor && lights_material(&effect.kind))));
                    if reads_anchor {
                        key = key.in_instance(place.instance);
                    }
                }

                if !out.contains_key(&key)
                    && let Some(filtered) = field(inp, channels, &out, (id, place), Job::Content(*channel), tiles, content_margin)
                {
                    out.insert(key, filtered);
                }
            }
        }

        if compares_content {
            insert_mask_field(inp, channels, &mut out, (id, place), mask, tiles, mask_margin);
        }
    }

    out
}

/// Store a filtered mask once per transform, or per draw when it reads the stack.
fn insert_mask_field(
    inp: &EvaluationInputs,
    channels: &[Channel],
    fields: &mut Fields,
    (id, place): (LayerId, DrawPlacement),
    mask: Option<&Mask>,
    tiles: &[(u32, u32)],
    margin: (u32, bool),
) {
    let Some(mask) = mask else { return };
    let mut key = FieldKey::mask(id, place.offset);
    if place.instance != 0 && mask.effects.iter().any(|effect| effect.visible && (effect.kind.reads_stack() || effect.reads(None))) {
        key = key.in_instance(place.instance);
    }

    if !fields.contains_key(&key)
        && let Some(filtered) = field(inp, channels, fields, (id, place), Job::Mask(mask), tiles, margin)
    {
        fields.insert(key, filtered);
    }
}

/// Whether a mask effect compares the layer's own content
fn compares_this_layer(kind: &EffectKind) -> bool {
    matches!(kind, EffectKind::Compare { left, right, .. } if *left == CompareSource::ThisLayer || *right == CompareSource::ThisLayer)
}

/// Build the curvature detail sizes every visible generator of `layers` reads, once, before tiles composite in parallel
fn prepare_curvature_sizes(inp: &EvaluationInputs, layers: &[LayerId]) {
    let visible = layers.iter().filter(|id| inp.project.layers.get(id).is_some_and(|layer| layer.visible));
    let masks = visible.filter_map(|id| layer_mask(inp.project, *id)).filter(|mask| mask.enabled);
    prepare_mask_curvature_sizes(inp, masks);
}

/// Retain up to this many bytes of whole-map fields while planning tiles
const EVALUATION_FIELD_BYTES: usize = 256 << 20;

/// Generators whose fitted signal depends on a precomputed whole-atlas field
fn whole_map_generator(kind: GeneratorKind) -> bool {
    matches!(kind, GeneratorKind::UvBorderDistance | GeneratorKind::DrippingRust | GeneratorKind::UvRandomColor | GeneratorKind::UvChecker)
}

/// A requested mask beneath a hidden folder probes the visible stack before that folder
fn requested_mask_stop(inp: &EvaluationInputs, id: LayerId) -> LayerId {
    let mut stop = id;
    let mut current = id;
    for _ in 0..texturecraft_core::project::MAX_DEPTH {
        let Some((_, Some(parent), _)) = inp.project.locate(current) else { break };
        if inp.project.layers.get(&parent).is_some_and(|layer| !layer.visible) {
            stop = parent;
        }
        current = parent;
    }
    stop
}

/// Prepare whole-map fields and build Micro Height maps from bottom to top
fn prepare_generators(inp: &mut EvaluationInputs, requested_mask: Option<LayerId>) {
    let Ok(set) = inp.project.texture_set(inp.ts) else { return };
    let requested_stop = requested_mask.map(|id| requested_mask_stop(inp, id));
    let mut order = Vec::new();
    draw_order(inp, &set.stack, 0, ContentsOffset::NONE.into(), true, &mut order);

    let mut anchors_below = HashMap::new();
    let mut first_readers = HashMap::new();
    let mut generators_by_anchor = HashMap::<_, BTreeMap<_, _>>::new();
    let mut considered_fields = HashSet::new();
    let mut retained_fields = HashSet::new();
    let field_bytes = (inp.gbuf.width as usize).saturating_mul(inp.gbuf.height as usize).saturating_mul(size_of::<f32>());
    let mut retained_bytes = 0usize;

    for (index, &(id, place)) in order.iter().enumerate() {
        let visible = inp.project.layers.get(&id).is_some_and(|layer| layer.visible);
        let draw = (id, place.instance);
        let masks = visible
            .then_some((draw, id))
            .into_iter()
            .chain(requested_mask.filter(|_| requested_stop == Some(id) && place.instance == 0 && !visible).map(|reader| ((reader, 0), reader)));
        for (reader_draw, reader) in masks {
            // Clone and smudge explicitly sample disabled masks
            let requested = reader_draw.1 == 0 && requested_mask == Some(reader);
            let effects = layer_mask(inp.project, reader).filter(|mask| mask.enabled || requested).into_iter().flat_map(|mask| &mask.effects);
            for effect in effects.filter(|effect| effect.visible) {
                let EffectKind::Generator(generator) = &effect.kind else { continue };

                if let Some(anchor) = micro_height_anchor(generator)
                    && let Some(anchor_index) = anchors_below.get(&anchor)
                {
                    inp.anchor_readers.insert((reader_draw, anchor), *anchor_index);
                    first_readers.entry((anchor, *anchor_index)).or_insert(draw);
                    generators_by_anchor.entry((anchor, *anchor_index)).or_default().insert(effect.id, generator);
                }

                if whole_map_generator(generator.kind)
                    && retained_bytes.saturating_add(field_bytes) <= EVALUATION_FIELD_BYTES
                    && considered_fields.insert(effect.id)
                {
                    let maps = GeneratorMaps::of(inp, generator, &curvature_sizes_in_use(generator), None, || inp.field_identity());
                    if let Some(field) = &maps.field
                        && retained_fields.insert(Arc::as_ptr(field))
                    {
                        retained_bytes += field_bytes;
                    }
                    inp.whole_maps.insert(effect.id, maps);
                }
            }
        }

        if visible {
            let mask_effects = layer_mask(inp.project, id).filter(|mask| mask.enabled).into_iter().flat_map(|mask| &mask.effects);
            let content_effects = inp.project.content_layer(id).filter(|layer| !layer.is_group()).into_iter().flat_map(|layer| &layer.content_effects);
            for effect in content_effects.chain(mask_effects).filter(|effect| effect.visible && matches!(effect.kind, EffectKind::Anchor)) {
                anchors_below.insert(effect.id, index);
            }
        }
    }

    if inp.is_session_stack() {
        inp.projectors.micro_heights.retain_readers(inp.ts, &first_readers.keys().copied().collect::<Vec<_>>());
    }

    if first_readers.is_empty() {
        return;
    }

    let mut readers: Vec<_> = first_readers.into_iter().collect();
    readers.sort_unstable_by_key(|((anchor, index), _)| (*index, *anchor));
    let stack = (inp.is_session_stack() && inp.preview.is_none())
        .then(|| StackKey::new(inp.project, inp.pixels, inp.baked, inp.images, inp.ts, &set.stack).map(Arc::new))
        .flatten();

    for ((anchor, index), draw) in readers {
        let build = || anchor_heights(inp, draw, anchor);
        let heights = match &stack {
            Some(stack) => inp.projectors.micro_heights.get_or_build(inp, (anchor, index), stack, build),
            None => Arc::new(build()),
        };
        if let Some(generators) = generators_by_anchor.remove(&(anchor, index)) {
            for (id, generator) in generators {
                let maps = GeneratorMaps::of(inp, generator, &curvature_sizes_in_use(generator), Some(heights.clone()), || inp.field_identity());
                inp.anchor_maps.insert((id, index), maps);
            }
        }
    }
}

fn micro_height_anchor(generator: &Generator) -> Option<LayerId> {
    let micro = generator.micro_in_use().filter(|micro| micro.height)?;
    match &micro.height_source {
        Some(Source::Anchor { anchor, .. }) => Some(*anchor),
        _ => None,
    }
}

/// Anchor point `anchor`'s height over the whole texture set as layer `reader` reads it, in the working atlas's layout
fn anchor_heights(inp: &EvaluationInputs, reader: (LayerId, usize), anchor: LayerId) -> AnchorHeights {
    let (width, height) = (inp.gbuf.width, inp.gbuf.height);
    let channels = evaluation_channels(inp, vec![Channel::Height]);
    let fields = evaluation_fields(inp, &channels, &all_tiles(width, height));
    let width_usize = width as usize;
    let tile_side = TILE as usize;
    let mut data = vec![0.0f32; width_usize * height as usize];

    // Each tile row writes its own atlas rows
    data.par_chunks_mut(width_usize.max(1) * tile_side).enumerate().for_each(|(tile_y, rows)| {
        for tile_x in 0..width.div_ceil(TILE) {
            let nodes = below_plan(inp, reader, &channels, (tile_x, tile_y as u32), Some(&fields), true);
            let preview = inp.preview.and_then(|preview| preview.alpha.tile(tile_x, tile_y as u32));
            for (x, y, offset) in tile_texels(inp.gbuf, tile_x, tile_y as u32) {
                let mut texel = Texel::new(inp, x, y, offset, stroke_at(preview, offset));
                texel.anchors_below(&nodes, reader);
                if let Some(destination) = rows.get_mut((y as usize % tile_side) * width_usize + x as usize) {
                    *destination = texel.anchored(anchor, Channel::Height)[0];
                }
            }
        }
    });
    AnchorHeights::new(data, (width, height), inp.block_scales.clone())
}

/// The blurred curvature levels `generator` reads: its wear's smoothing, else its detail sizes (a replaced curvature input reads none)
fn curvature_sizes_in_use(generator: &Generator) -> [bool; LEVEL_RADII.len()] {
    if generator.inputs.contains_key(&BakedMap::Curvature) {
        return [false; LEVEL_RADII.len()];
    }

    if let Some(wear) = wear_parameters(generator) {
        return wear::levels_in_use(generator.kind, &wear);
    }

    let detail = generator.detail_in_use().as_ref().map(CurvatureDetail::blurred_sizes_in_use).unwrap_or_default();
    std::array::from_fn(|level| detail.get(level).copied().unwrap_or(false))
}

/// The wear controls `generator` computes with: its own, Fiber Glass Edge Wear's defaults when it has none (it has no earlier model), else None
fn wear_parameters(generator: &Generator) -> Option<WearParameters> {
    generator.wear.or_else(|| WearParameters::defaults(generator.kind).filter(|_| generator.kind == GeneratorKind::FiberGlassEdgeWear))
}

/// Build the curvature detail sizes the visible generators of `masks` read, once, before tiles composite in parallel
fn prepare_mask_curvature_sizes<'a>(inp: &EvaluationInputs, masks: impl Iterator<Item = &'a Mask>) {
    let mut needed = [false; LEVEL_RADII.len()];
    for effect in masks.flat_map(|mask| &mask.effects).filter(|effect| effect.visible) {
        if let EffectKind::Generator(generator) = &effect.kind {
            if micro_height_anchor(generator).is_some() || inp.whole_maps.contains_key(&effect.id) {
                continue;
            }

            if generator.micro_in_use().is_some() || whole_map_generator(generator.kind) {
                GeneratorMaps::of(inp, generator, &curvature_sizes_in_use(generator), None, || inp.field_identity());
                continue;
            }

            for (slot, in_use) in needed.iter_mut().zip(curvature_sizes_in_use(generator)) {
                *slot |= in_use;
            }
        }
    }
    CurvatureSizes::of(inp, &needed);
}

struct Node<'a> {
    draw: (LayerId, usize),
    content: Content<'a>,
    chans: Vec<ChanPlan<'a>>,
    mask: Option<MaskPlan<'a>>,
    /// Visible anchor points of the layer: recorded after it is applied, or (`Some(k)`, an anchor with visible effects above it) with its content cut after `k` visible effects
    anchors: Vec<(LayerId, Option<usize>)>,
    /// The node or one of its children records an anchor
    has_anchor: bool,
    /// Its base colour has an effect lit by the layer's material (see [`lights_material`])
    lights: bool,
    height_mask: HeightMask,
    /// Geometry mask: whether the layer shows on each mesh submesh (None: everywhere).
    parts: Option<Vec<bool>>,
}

/// Plans of the visible effects of a stack: mask effects (`channel` None) or the content
/// effects of one channel (paint effects then read that channel's pixels).
fn effect_plans<'a>(
    inp: &EvaluationInputs<'a>,
    effects: &'a [Effect],
    tile_x: u32,
    tile_y: u32,
    channel: Option<Channel>,
    place: DrawPlacement,
    id: LayerId,
) -> Vec<EffectPlan<'a>> {
    effects
        .iter()
        .filter(|effect| effect.visible)
        .map(|effect| {
            let (image, target) = match channel {
                Some(channel) => (inp.pixels.layer(effect.id, channel), PaintTarget::Layer(effect.id)),
                None => (inp.pixels.masks.get(&effect.id), PaintTarget::Mask(effect.id)),
            };
            let maps = match &effect.kind {
                EffectKind::Generator(generator) => {
                    let maps = micro_height_anchor(generator)
                        .and_then(|anchor| inp.anchor_readers.get(&((id, place.instance), anchor)))
                        .and_then(|index| inp.anchor_maps.get(&(effect.id, *index)))
                        .or_else(|| inp.whole_maps.get(&effect.id));
                    maps.cloned().unwrap_or_else(|| GeneratorMaps::of(inp, generator, &curvature_sizes_in_use(generator), None, || inp.field_identity()))
                }
                _ => GeneratorMaps::default(),
            };
            let (depth, covers_all, projection) = match &effect.kind {
                EffectKind::Fill { projection, sampling, .. } | EffectKind::Material(Fill { projection, sampling, .. }) => {
                    let (depth, covers_all) = fill_coverage(inp, projection, *sampling);
                    (depth, covers_all, (!place.offset.is_none()).then(|| place.offset.apply(projection)))
                }
                _ => (None, false, None),
            };
            let environment = match &effect.kind {
                EffectKind::Filter(Filter::EnvironmentLighting { .. })
                    if channel == Some(Channel::BaseColor) && effect.applied_blend(channel).opacity > 0.0 =>
                {
                    Some(inp.environment())
                }
                _ => None,
            };
            let stylized = match &effect.kind {
                EffectKind::Filter(filter @ Filter::BakedLighting { .. }) => filter.stylized_lighting().map(|(settings, light)| settings.shading(light)),
                _ => None,
            };
            let blend = effect.applied_blend(channel);
            EffectPlan {
                id: effect.id,
                kind: &effect.kind,
                projection,
                blend: blend.mode,
                opacity: blend.opacity,
                data: if effect.kind.painted() { tile_of(inp, image, tile_x, tile_y, channel) } else { None },
                preview: previews(inp, target),
                maps,
                filtered_mask: None,
                depth,
                covers_all,
                environment,
                stylized,
                input: match &effect.kind {
                    EffectKind::Filter(filter) if !filter.is_neighbourhood() => ImageInput::of(filter).first().map(|input| effect.input(input)),
                    _ => None,
                },
                gradient: match &effect.kind {
                    EffectKind::Filter(filter) => inp.gradient(filter),
                    _ => None,
                },
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

    // Empty texels of signed channels hold a signed zero
    let empty: Vec<u16> = match channel {
        Some(c) if c.kind() == ChannelKind::Signed => [vec![to_u16_signed(0.0); c.components()], vec![0]].concat(),
        _ => Vec::new(),
    };
    img.resampled_tile(w, h, tx, ty, &empty).map(Cow::Owned)
}

/// The plan of layer `id`'s `mask` for tile (tx, ty), its fills under the Scale and Rotation `around` of the folders holding the layer
fn mask_plan<'a>(inp: &EvaluationInputs<'a>, id: LayerId, mask: &'a Mask, tx: u32, ty: u32, fields: Option<&'a Fields>, place: DrawPlacement) -> MaskPlan<'a> {
    let background = if mask.background == MaskBackground::White { 1.0 } else { 0.0 };
    let mut effects = effect_plans(inp, &mask.effects, tx, ty, None, place, id);
    let filtered = fields.and_then(|fields| FieldKey::mask(id, place.offset).field(fields, place.instance));
    for (index, effect) in effects.iter_mut().enumerate() {
        effect.filtered_mask = filtered.and_then(|field| field.anchor_stages.iter().find(|(stage, _)| *stage == index).map(|(_, field)| field));
    }

    let start = filtered.and_then(|field| neighbourhood(&mask.effects, inp.gbuf.width).map(|(index, _, _)| (index, field)));
    let compared = compared_channels(mask.effects.iter().filter(|effect| effect.visible && effect.kind.reads_stack()), Vec::new());

    // A folder's own content is its children: its compare sides read the stack below it
    let layer = if compared.is_empty() {
        None
    } else {
        inp.project.content_layer(id).and_then(|l| plan_content(inp, id, l, &compared, tx, ty, 0, fields, false, place, None))
    };
    MaskPlan { background, effects, layer, start }
}

/// The plans of sibling layers `list` (under the Scale and Rotation of the folders holding them)
fn plan<'a>(inp: &EvaluationInputs<'a>, list: &[LayerId], channels: &[Channel], tx: u32, ty: u32, depth: usize, fields: Option<&'a Fields>) -> Vec<Node<'a>> {
    let around = list.first().map_or(ContentsOffset::NONE, |id| inp.project.contents_around(*id));
    plan_in(inp, list, channels, tx, ty, depth, fields, around.into(), None)
}

/// [`plan`] of sibling layers under `around`, with an optional marker for a stack probe
#[allow(clippy::too_many_arguments)]
fn plan_in<'a>(
    inp: &EvaluationInputs<'a>,
    list: &[LayerId],
    channels: &[Channel],
    tx: u32,
    ty: u32,
    depth: usize,
    fields: Option<&'a Fields>,
    place: DrawPlacement,
    stop: Option<(LayerId, usize)>,
) -> Vec<Node<'a>> {
    if depth > texturecraft_core::project::MAX_DEPTH {
        return Vec::new();
    }

    list.iter().filter_map(|id| plan_node(inp, *id, channels, tx, ty, depth, fields, true, place, stop)).collect()
}

/// The plan of layer `id` at `depth` (None where it draws nothing), its fills under the Scale and Rotation `around` of the folders holding it; a folder's children are planned when `kids`
#[allow(clippy::too_many_arguments)]
fn plan_node<'a>(
    inp: &EvaluationInputs<'a>,
    id: LayerId,
    channels: &[Channel],
    tx: u32,
    ty: u32,
    depth: usize,
    fields: Option<&'a Fields>,
    kids: bool,
    place: DrawPlacement,
    stop: Option<(LayerId, usize)>,
) -> Option<Node<'a>> {
    let own: &'a Layer = inp.project.layers.get(&id)?;

    // A stack probe stops here even when this layer draws nothing
    if stop == Some((id, place.instance)) {
        return Some(Node {
            draw: (id, place.instance),
            content: Content::Paint { preview: false },
            chans: Vec::new(),
            mask: None,
            anchors: Vec::new(),
            has_anchor: false,
            lights: false,
            height_mask: HeightMask::default(),
            parts: None,
        });
    }

    // Instances draw their source's content (a missing source draws nothing)
    let layer: &'a Layer = inp.project.content_layer(id)?;
    if !own.visible {
        return None;
    }

    let (content, chans) = plan_content(inp, id, layer, channels, tx, ty, depth, fields, kids, place, stop)?;
    let mask = layer_mask(inp.project, id).filter(|mask| mask.enabled).map(|mask| mask_plan(inp, id, mask, tx, ty, fields, place));

    // Anchors in the mask record even where the layer draws nothing
    let mask_anchor = mask.as_ref().is_some_and(|m| m.effects.iter().any(|e| matches!(*e.kind, EffectKind::Anchor)));

    // An empty paint layer still matters if its mask records an anchor
    if let Content::Paint { preview: false } = content
        && chans.iter().all(|c| c.data.is_none() && c.effects.is_empty())
        && !mask_anchor
    {
        return None;
    }

    let probe_group = stop.is_some() && matches!(&content, Content::Group(children) if !children.is_empty());
    if chans.is_empty() && !mask_anchor && !probe_group {
        return None;
    }

    let has_content_effects = !layer.is_group() && layer.content_effects.iter().any(|e| e.visible);
    let anchors = if has_content_effects { anchor_cuts(&layer.content_effects) } else { Vec::new() };
    let has_anchor = !anchors.is_empty() || mask_anchor || matches!(&content, Content::Group(nodes) if nodes.iter().any(|n| n.has_anchor));
    let base = Channel::BaseColor.index();
    let lights = chans.iter().any(|chan_plan| chan_plan.ch == base && chan_plan.effects.iter().any(|effect| lights_material(effect.kind)));

    // The geometry mask is the layer's own (an instance in another set has other parts)
    let parts = own.geometry_mask.as_ref().map(|names| inp.mesh.submeshes.iter().map(|s| names.contains(&s.name)).collect());
    Some(Node { draw: (id, place.instance), content, chans, mask, anchors, has_anchor, lights, height_mask: layer.height_mask, parts })
}

/// Whether a base colour effect reads the layer's other channels as its material: Baked Lighting Stylized's highlights, Environment Lighting, PBR Validate's metal
pub(crate) fn lights_material(kind: &EffectKind) -> bool {
    match kind {
        EffectKind::Filter(filter @ Filter::BakedLighting { .. }) => filter.stylized_lighting().is_some_and(|(stylized, _)| stylized.output.specular()),
        EffectKind::Filter(Filter::EnvironmentLighting { .. } | Filter::PbrValidate(_)) => true,
        _ => false,
    }
}

/// The content of `layer` (layer `id`'s own, or its instance source's; None for an unfollowed instance) and its plan per drawn channel of `channels`, its fills under the Scale and Rotation `around` of the folders holding `id`; a folder's children are planned when `kids`
#[allow(clippy::too_many_arguments)]
fn plan_content<'a>(
    inp: &EvaluationInputs<'a>,
    id: LayerId,
    layer: &'a Layer,
    channels: &[Channel],
    tx: u32,
    ty: u32,
    depth: usize,
    fields: Option<&'a Fields>,
    kids: bool,
    place: DrawPlacement,
    stop: Option<(LayerId, usize)>,
) -> Option<(Content<'a>, Vec<ChanPlan<'a>>)> {
    let paint = matches!(layer.kind, LayerKind::Paint);
    let has_content_effects = !layer.is_group() && layer.content_effects.iter().any(|e| e.visible);
    let chans = channels
        .iter()
        .filter_map(|c| {
            let b = layer.blend(*c)?;
            if b.mode == BlendMode::Disable || b.opacity <= 0.0 {
                return None;
            }

            let data = if paint { tile_of(inp, inp.pixels.layer(layer.id, *c), tx, ty, Some(*c)) } else { None };
            let effects = if has_content_effects { effect_plans(inp, &layer.content_effects, tx, ty, Some(*c), place, id) } else { Vec::new() };
            let start = fields
                .and_then(|fields| FieldKey::content(id, *c, place.offset).field(fields, place.instance))
                .and_then(|field| neighbourhood_filters(&layer.content_effects, inp.tile_res(), Some(*c)).last().map(|(index, _, _)| (index, field)));
            Some(ChanPlan { ch: c.index(), kind: c.kind(), comps: c.components(), blend: b.mode, opacity: b.opacity.clamp(0.0, 1.0), data, effects, start })
        })
        .collect::<Vec<_>>();
    let content = match &layer.kind {
        LayerKind::Paint => Content::Paint { preview: previews(inp, PaintTarget::Layer(layer.id)) },
        LayerKind::Fill(fill) => {
            let (depth, covers_all) = fill_coverage(inp, &fill.projection, fill.sampling);
            let projection = (!place.offset.is_none()).then(|| Box::new(place.offset.apply(&fill.projection)));
            Content::Fill { fill, projection, depth, covers_all }
        }
        LayerKind::Group { children, .. } => {
            Content::Group(if kids { plan_in(inp, children, channels, tx, ty, depth + 1, fields, place.children(inp, id, layer), stop) } else { Vec::new() })
        }

        // `content_layer` already followed instance links
        LayerKind::Instance { .. } => return None,
    };
    Some((content, chans))
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
fn plan_levels<'a>(inp: &EvaluationInputs<'a>, levels: &[Level], channels: &[Channel], tx: u32, ty: u32, fields: &'a Fields, warm: bool) -> Vec<LevelPlan<'a>> {
    let last = levels.len().saturating_sub(1);
    levels
        .iter()
        .zip(level_offsets(inp.project, levels))
        .enumerate()
        .map(|(d, (l, around))| LevelPlan {
            below: if warm { Vec::new() } else { plan_in(inp, &l.below, channels, tx, ty, d, Some(fields), around.into(), None) },
            at: plan_node(inp, l.at, channels, tx, ty, d, Some(fields), d == last, around.into(), None),
            above: plan_in(inp, &l.above, channels, tx, ty, d, Some(fields), around.into(), None),
        })
        .collect()
}

/// The Scale and Rotation the folders (instanced ones included) above each level of the path to the hot layer lay on its fills
fn level_offsets(p: &Project, levels: &[Level]) -> Vec<ContentsOffset> {
    let mut around = ContentsOffset::NONE;
    levels
        .iter()
        .map(|level| {
            let held = around;
            around = around.then(p.content_layer(level.at).map_or(ContentsOffset::NONE, |layer| p.contents_offset(layer.id)));
            held
        })
        .collect()
}

#[inline]
fn blend_chan(c: &ChanPlan, mode: BlendMode, a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    if c.comps == 3 { blend3(mode, a, b) } else { [blend1(mode, a[0], b[0]); 3] }
}

/// Lay value `p` with weight `a` over a straight value and coverage (premultiplied "over")
fn over(c: &ChanPlan, mode: BlendMode, v: [f32; 3], cov: f32, p: [f32; 3], a: f32) -> ([f32; 3], f32) {
    if a <= 0.0 {
        return (v, cov);
    }

    let b = if cov > 0.0 { blend_chan(c, mode, v, p) } else { p };
    let nc = a + cov * (1.0 - a);
    (mix3(v, b, a / nc.max(1e-5)), nc)
}

/// Whether a fill placed by `proj` repeats past its tile both ways, so no texel falls outside it (tri-planar never ends)
fn repeats_everywhere(proj: &Projection, sampling: Sampling) -> bool {
    sampling.wrap.repeats() == (true, true) || matches!(proj, Projection::Triplanar { .. })
}

/// Whether a fill placed by `proj` covers every texel: it repeats, nothing crops or culls it and it is not framed on the surface
fn covers_every_texel(proj: &Projection, sampling: Sampling, depth: Option<&FillDepth>) -> bool {
    let framed = matches!(proj, Projection::Planar { placed: Some(_), .. } | Projection::Warp { .. });
    let depth_culled = depth.is_some() && proj.depth_culling().is_some();
    !framed && !depth_culled && repeats_everywhere(proj, sampling) && projector_covers_all(proj)
}

/// A fill's projector depth (None: not culled by depth) and whether it covers every texel
fn fill_coverage(inp: &EvaluationInputs, projection: &Projection, sampling: Sampling) -> (Option<Arc<FillDepth>>, bool) {
    let depth = inp.projector_depth(projection, sampling);
    let covers_all = covers_every_texel(projection, sampling, depth.as_deref());
    (depth, covers_all)
}

/// Whether a fill placed by `proj` is sampled once per copy of `sampling`'s symmetry
fn takes_symmetry(proj: &Projection, sampling: Sampling) -> bool {
    sampling.symmetry.on && proj.takes_symmetry()
}

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
    stop: Option<(LayerId, usize)>,
    /// Stack values recorded by the anchor points evaluated so far; true marks a mask anchor (gray, not channel values).
    anchors: Vec<(LayerId, Acc, bool)>,
    /// The stack below the layer whose mask compares its channels (recorded by `eval` there, or where it stops)
    below: Option<Acc>,
    /// The material Baked Lighting Stylized reads its highlights from: the stack with the layer it lights (its other channels) laid on
    lit: Option<Acc>,
    /// Where `eval` takes layer masks and content from.
    tape: Tape<'b>,
    /// The copy of a symmetric fill being sampled: the surface point and normal it sees, and its image's flip
    copy: Option<(usize, FillCopy)>,
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

/// Each channel's content (value and coverage) where the layer has one
type ChannelContents = [Option<([f32; 3], f32)>; NCH];

impl<'a, 'b> Texel<'a, 'b> {
    fn new(inp: &'b Inputs<'a>, x: u32, y: u32, o: usize, stroke: f32) -> Self {
        Texel { inp, x, y, o, res: inp.gbuf.width, coords: None, stroke, stop: None, anchors: Vec::new(), below: None, lit: None, tape: Tape::Off, copy: None }
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
    fn tape_content(&mut self, content: &Content, c: &ChanPlan) -> Option<([f32; 3], f32)> {
        if let Tape::Replay(..) = self.tape {
            let cov = self.tape.next();
            // Recorded as 0 where empty.
            if cov <= 0.0 {
                return None;
            }
            let v = if c.comps == 3 { [self.tape.next(), self.tape.next(), self.tape.next()] } else { [self.tape.next(); 3] };
            return Some((v, cov));
        }
        let r = self.content(content, c);
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

        #[cfg(test)]
        tests::COORDS_LOOKUPS.with(|count| count.set(count.get() + 1));

        let g = self.inp.gbuf;
        let c = match self.surface((self.y * g.width + self.x) as usize) {
            Some(s) => self.coords_of(s),
            None => self.tile_uv(Coords {
                uv: Vec2::new((self.x as f32 + 0.5) / g.width as f32, (self.y as f32 + 0.5) / g.height as f32),
                pos: Vec3::ZERO,
                nrm: Vec3::Y,
            }),
        };
        self.coords = Some(c);
        c
    }

    /// The coordinates projections see at surface `s`: its UV, and its position in mesh radii from the mesh's centre
    #[inline(always)]
    fn coords_of(&self, s: Surface) -> Coords {
        let (center, radius) = mesh_frame(self.inp.bounds);
        self.tile_uv(Coords { uv: s.uv, pos: (s.pos - center) / radius, nrm: s.nrm })
    }

    /// `c` with its UV moved out of the atlas on a UV-tiled set, so projections see the mesh's own UVs (patterns repeat per tile)
    #[inline(always)]
    fn tile_uv(&self, mut c: Coords) -> Coords {
        if let Ok(set) = self.inp.project.texture_set(self.inp.ts)
            && let Some(grid) = set.tile_grid()
        {
            c.uv = Vec2::from(set.from_atlas(&grid, c.uv.to_array()));
        }
        c
    }

    /// The coordinates of texel (`x`, `y`) through `proj`, uncached (None when uncovered)
    fn projected_at(&self, proj: &Projection, x: u32, y: u32) -> Option<Coords> {
        let g = self.inp.gbuf;
        if x >= g.width || y >= g.height {
            return None;
        }

        let s = self.surface((y * g.width + x) as usize)?;
        let c = self.coords_of(s);
        if proj.frame().is_none() {
            return Some(c);
        }

        let (center, _) = mesh_frame(self.inp.bounds);
        framed_cover(proj, s.pos, s.nrm, center).map(|(uv, _)| Coords { uv, ..c })
    }

    /// Image texels spanned by one surface texel in each projection plane.
    fn footprints(&self, proj: &Projection, samples: &[(Vec2, f32); 3], image: &ImageRes) -> [f32; 3] {
        let size = Vec2::new(image.width as f32, image.height as f32);
        if matches!(proj, Projection::Uv { .. } | Projection::UvTile) {
            // Use the current tile's affine mapping even where a neighbor wraps or is uncovered
            let step = self.inp.mesh_uv_step(self.x, self.y);
            let span = if let Projection::Uv { scale, rotation, .. } = proj {
                let transform = placement(Vec2::from(*scale), *rotation, Vec2::ZERO);
                let across = (transform(Vec2::new(step.x, 0.0)) * size).length();
                let down = (transform(Vec2::new(0.0, step.y)) * size).length();
                across.max(down)
            } else {
                (step * size).max_element()
            };
            return [if span.is_finite() { span.max(1.0) } else { 1.0 }, 1.0, 1.0];
        }

        let mut out = [1.0f32; 3];
        let seam_limit = size.max_element() * SEAM_JUMP_SHARE;
        for (dx, dy) in [(1, 0), (0, 1)] {
            let Some(next) = self.projected_at(proj, self.x + dx, self.y + dy) else { continue };

            for ((footprint, (uv, _)), (next_uv, _)) in out.iter_mut().zip(samples).zip(self.image_uvs(proj, &next)) {
                let span = ((next_uv - *uv) * size).length();

                // Projected neighbors can land on disconnected surfaces at a UV seam
                if span.is_finite() && span < seam_limit {
                    *footprint = footprint.max(span);
                }
            }
        }
        out
    }

    /// World position of this texel as the fill copy being sampled sees it (None when uncovered)
    fn world(&self) -> Option<Vec3> {
        self.surface((self.y * self.inp.gbuf.width + self.x) as usize).map(|s| s.pos)
    }

    /// The surface at G-buffer index `i`, as the fill copy being sampled sees it
    #[inline(always)]
    fn surface(&self, i: usize) -> Option<Surface> {
        let mut s = self.inp.gbuf.surface(self.inp.mesh, i)?;
        if let Some((_, copy)) = self.copy {
            s.pos = copy.point(s.pos);
            s.nrm = copy.normal(s.nrm);
            s.geo_nrm = copy.normal(s.geo_nrm);
        }
        Some(s)
    }

    /// Where `proj` reads images at `coords`, turned over as the fill copy being sampled says
    fn image_uvs(&self, proj: &Projection, coords: &Coords) -> [(Vec2, f32); 3] {
        let samples = image_samples(proj, coords);
        match self.copy {
            Some((_, copy)) => samples.map(|(uv, weight)| (copy.uv(uv), weight)),
            None => samples,
        }
    }

    /// A fill's value and coverage through `proj`, from `one` (which samples it once): with `sampling`'s symmetry on, every copy of the projection is sampled and the copies are laid over each other, the texel's own copy on top
    fn symmetric(&mut self, proj: &Projection, sampling: Sampling, mut one: impl FnMut(&mut Self) -> ([f32; 3], f32)) -> ([f32; 3], f32) {
        // Skip the world-space lookup when no symmetry copies are needed
        if !takes_symmetry(proj, sampling) {
            return one(self);
        }

        self.symmetric_copies(sampling, one)
    }

    /// [`Texel::symmetric`] with symmetry on: every copy sampled and laid over the others
    fn symmetric_copies(&mut self, sampling: Sampling, mut one: impl FnMut(&mut Self) -> ([f32; 3], f32)) -> ([f32; 3], f32) {
        let Some(world) = self.inp.gbuf.surface(self.inp.mesh, (self.y * self.inp.gbuf.width + self.x) as usize).map(|surface| surface.pos) else {
            return one(self);
        };

        let (center, radius) = mesh_frame(self.inp.bounds);
        let cached = self.coords.take();
        let mut value = [0.0f32; 3];
        let mut cover = 0.0f32;
        let mut copies = FillCopies::new(&sampling.symmetry, world, center, radius);
        while let Some((index, copy)) = copies.next_indexed() {
            self.copy = Some((index, copy));
            self.coords = None;
            let (copy_value, copy_cover) = one(self);
            let weight = copy_cover.clamp(0.0, 1.0) * (1.0 - cover);
            for (total, part) in value.iter_mut().zip(copy_value) {
                *total += part * weight;
            }
            cover += weight;
            if cover >= COVERED {
                break;
            }
        }
        self.copy = None;
        self.coords = cached;
        if cover <= 0.0 {
            return ([0.0; 3], 0.0);
        }

        (value.map(|v| v / cover), cover)
    }

    /// The normal Baked Lighting Stylized lights: the mesh's (no `relief`), or (Mesh + Height + Normal) its [`Texel::detail_normal`] at `relief`
    fn lit_normal(&mut self, relief: Option<f32>) -> [f32; 3] {
        match relief {
            Some(relief) => self.detail_normal(relief),
            None => self.coords().nrm.to_array(),
        }
    }

    /// The baked world normal (the mesh's without one) tilted by the baked height's slope times `relief`
    fn detail_normal(&mut self, relief: f32) -> [f32; 3] {
        let mesh = self.coords().nrm;
        let detail = self
            .baked_color(BakedMap::WorldNormal)
            .map(|packed| Vec3::from(packed) * 2.0 - Vec3::ONE)
            .filter(|n| n.length_squared() > MIN_NORMAL_LENGTH_SQUARED);
        let nrm = detail.map_or(mesh, Vec3::normalize);
        let i = (self.y * self.inp.gbuf.width + self.x) as usize;
        let (Some(surface), Some(slope)) = (self.surface(i), self.height_slope()) else { return nrm.to_array() };
        let tangent = surface.tangent.truncate();
        let bitangent = nrm.cross(tangent) * surface.tangent.w;
        let tilted = nrm - (tangent * slope[0] + bitangent * slope[1]) * HEIGHT_RELIEF * relief;
        tilted.try_normalize().unwrap_or(nrm).to_array()
    }

    /// The baked height's change per UV unit along U and V at this texel (central differences); None without a height bake
    fn height_slope(&self) -> Option<[f32; 2]> {
        let (width, height) = (self.res, self.inp.gbuf.height);
        let at = |x: u32, y: u32| self.inp.baked.sample_working(BakedMap::Height, x, y, (width, height), &self.inp.block_scales).map(|h| h[0]);
        let (origin_x, origin_y, tile_width, tile_height) = if let Some(grid) = self.inp.tile_grid() {
            let block = (width / grid.cols.max(1)).max(1);
            let tile = ((block as f32 * self.inp.block_scales.at(self.x, self.y)).round() as u32).max(1);
            (self.x / block * block, self.y / block * block, tile, tile)
        } else {
            (0, 0, width, height)
        };

        let last_x = origin_x.saturating_add(tile_width.saturating_sub(1)).min(width.saturating_sub(1));
        let last_y = origin_y.saturating_add(tile_height.saturating_sub(1)).min(height.saturating_sub(1));
        if self.x > last_x || self.y > last_y {
            return None;
        }

        let (left, right) = (self.x.saturating_sub(1).max(origin_x), (self.x + 1).min(last_x));
        let (up, down) = (self.y.saturating_sub(1).max(origin_y), (self.y + 1).min(last_y));
        let du = (at(right, self.y)? - at(left, self.y)?) / (right - left).max(1) as f32 * tile_width as f32;
        let dv = (at(self.x, down)? - at(self.x, up)?) / (down - up).max(1) as f32 * tile_height as f32;
        Some([du, dv]).filter(|slope| slope.iter().all(|s| s.is_finite()))
    }

    fn baked(&self, m: BakedMap) -> Option<f32> {
        self.baked_color(m).map(|value| value[0])
    }

    fn baked_color(&self, map: BakedMap) -> Option<[f32; 3]> {
        self.inp.baked.sample_working(map, self.x, self.y, (self.res, self.inp.gbuf.height), &self.inp.block_scales)
    }

    /// This texel's ID color: from image resource `image` (nearest, as stored), or from the ID mesh map without one
    fn id_color(&self, image: Option<&str>) -> Option<[f32; 3]> {
        let Some(name) = image else { return self.baked_color(BakedMap::Id) };
        let image = self.inp.images.get(name)?;
        let [red, green, blue, _] =
            image.sample_nearest((self.x as f32 + 0.5) / self.res.max(1) as f32, (self.y as f32 + 0.5) / self.inp.gbuf.height.max(1) as f32);
        Some([red, green, blue])
    }

    /// Where `proj` samples this texel: its coordinates, or for a projection from a frame on the surface (placed planar, warp) its place in the frame, and how much backface culling and the shape crop let through
    fn projected(&mut self, proj: &Projection) -> (Coords, f32) {
        let c = self.coords();
        if proj.frame().is_none() {
            return (c, 1.0);
        }

        let (center, _) = mesh_frame(self.inp.bounds);

        match self.world().and_then(|p| framed_cover(proj, p, c.nrm, center)) {
            Some((uv, cover)) => (Coords { uv, ..c }, cover),
            None => (c, 0.0),
        }
    }

    /// `place`'s fill at this texel in channel `ch`, every symmetric copy laid over each other when its symmetry is on
    #[inline(always)]
    fn fill_place_texel(&mut self, place: FillPlace, source: Option<&Source>, ch: Channel) -> ([f32; 3], f32) {
        // Sample directly without symmetry to keep the common path free of the copy search
        if takes_symmetry(place.projection, place.fill.sampling) {
            self.symmetric(place.projection, place.fill.sampling, |texel| texel.fill_texel(place, source, ch))
        } else {
            self.fill_texel(place, source, ch)
        }
    }

    /// `place`'s fill sampled once at this texel: its value in channel `ch` (`source`, else the channel's default) and its coverage
    #[inline(always)]
    fn fill_texel(&mut self, place: FillPlace, source: Option<&Source>, ch: Channel) -> ([f32; 3], f32) {
        let FillPlace { fill, projection, depth, covers_all } = place;
        let value = match source {
            Some(source) => self.source(source, projection, fill.sampling, ch.kind()),
            None => ch.default_value(),
        };
        let cover = if covers_all { 1.0 } else { self.cover(fill.sources.values(), projection, fill.sampling, depth) };
        (value, cover)
    }

    /// How much a fill projected by `proj` covers this texel: [`Texel::shape_cover`], then depth culling through the projector's `depth` map
    fn cover<'s>(&mut self, sources: impl Iterator<Item = &'s Source>, proj: &Projection, sampling: Sampling, depth: Option<&FillDepth>) -> f32 {
        #[cfg(test)]
        tests::COVER_CALLS.with(|count| count.set(count.get() + 1));

        let cover = self.shape_cover(sources, proj, sampling);
        match depth.zip(proj.depth_culling()) {
            Some((depth, culling)) if cover > 0.0 => {
                let copy_index = self.copy.map_or(0, |(index, _)| index);
                cover * self.world().map_or(1.0, |point| depth.cover(copy_index, point, culling.hardness))
            }
            _ => cover,
        }
    }

    /// How much a fill projected by `proj` covers this texel: wrap, shape crop and backface culling, and a cropped framed projection's image alpha
    fn shape_cover<'s>(&mut self, mut sources: impl Iterator<Item = &'s Source>, proj: &Projection, sampling: Sampling) -> f32 {
        let (Projection::Planar { projector, placed: Some(_), .. } | Projection::Warp { projector, .. }) = proj else {
            // Repeating with nothing cropped or culled, a fill covers every texel without looking up where the texel is
            if repeats_everywhere(proj, sampling) && projector_covers_all(proj) {
                return 1.0;
            }

            let coords = self.coords();
            return self.tile_cover(proj, &coords, sampling) * projector_cover(proj, &coords);
        };

        let (coords, cover) = self.projected(proj);
        if projector.crop == ShapeCrop::ExtendsOutside || projector.ignore_image_alpha {
            return cover * self.tile_cover(proj, &coords, sampling);
        }

        // Cropped to its shape, a framed projection takes the shape of its image too
        let image = sources.find_map(|source| match source {
            Source::Image { name, .. } => self.inp.images.get(name),
            _ => None,
        });
        let samples = self.image_uvs(proj, &coords);
        let [(uv, _), ..] = samples;

        // Use the image's sampling footprint so minified color and coverage agree
        let alpha = image.map_or(1.0, |image| {
            let [footprint, ..] = match sampling.filtering {
                Filtering::BilinearHq => self.footprints(proj, &samples, image),
                Filtering::BilinearSharp | Filtering::Nearest => [1.0; 3],
            };
            image.sample_alpha(uv.x, uv.y, sampling, footprint).clamp(0.0, 1.0)
        });
        cover * alpha * self.tile_cover(proj, &coords, sampling)
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

    /// 1 where a fill placed by `proj` at `coords` lands inside its 0..1 tile on every axis `sampling.wrap` does not repeat, else 0
    fn tile_cover(&self, proj: &Projection, coords: &Coords, sampling: Sampling) -> f32 {
        if repeats_everywhere(proj, sampling) {
            return 1.0;
        }

        let (across, down) = sampling.wrap.repeats();
        let [(uv, _), ..] = self.image_uvs(proj, coords);
        let inside = |t: f32, repeats: bool| repeats || (0.0..1.0).contains(&t);
        if inside(uv.x, across) && inside(uv.y, down) { 1.0 } else { 0.0 }
    }

    /// The value `source` gives this texel through `projection`, read as a channel of `kind`
    fn source(&mut self, source: &Source, projection: &Projection, sampling: Sampling, kind: ChannelKind) -> [f32; 3] {
        match source {
            Source::Uniform(value) => *value,

            Source::Anchor { anchor, channel, ramp } if kind == ChannelKind::Color && !ramp.is_empty() => {
                let value = self.anchored(*anchor, *channel);

                // Signed values (height) span -1..1; a mask anchor's gray already spans 0..1 in every channel
                let signed = channel.kind() == ChannelKind::Signed && !self.is_mask_anchor(*anchor);
                let lightness = if signed { lum(value) * 0.5 + 0.5 } else { lum(value) };
                texturecraft_core::gradient(ramp, lightness).unwrap_or_default()
            }
            Source::Procedural { pattern, low, high, ramp } => {
                let (coords, _) = self.projected(projection);
                let pixels = if pattern.resolution.can_reduce() {
                    let uv_tile = self.inp.tile_res_at(self.x, self.y);
                    let texture_set = self.inp.project.texture_set(self.inp.ts).map_or(uv_tile, |set| set.resolution);
                    pattern_pixels(pattern, uv_tile, texture_set)
                } else {
                    None
                };
                let amount = procedural_in(pattern, projection, &coords, &PatternContext { pixels, images: Some(self.inp.images) });

                if kind == ChannelKind::Color && !ramp.is_empty() {
                    texturecraft_core::gradient(ramp, amount).unwrap_or_default()
                } else {
                    mix3(*low, *high, amount)
                }
            }
            Source::Image { name, normal_format } => {
                let Some(image) = self.inp.images.get(name) else { return [0.5; 3] };

                let (coords, _) = self.projected(projection);
                let samples = self.image_uvs(projection, &coords);
                let footprints = match sampling.filtering {
                    Filtering::BilinearHq => self.footprints(projection, &samples, image),
                    Filtering::BilinearSharp | Filtering::Nearest => [1.0; 3],
                };

                // Blend projected planes in the same space when any of them reads a linear mip
                let is_srgb_color = kind == ChannelKind::Color && image.srgb;
                let planes = samples.into_iter().zip(footprints).filter(|((_, weight), _)| *weight > 0.0);
                let blend_linear = is_srgb_color && planes.clone().any(|(_, footprint)| crate::store::uses_mips(footprint));
                let mut rgb = [0.0; 3];
                for ((uv, weight), footprint) in planes {
                    let sampled = if blend_linear {
                        image.sample_srgb_color(uv.x, uv.y, sampling, footprint)
                    } else {
                        image.sample_filtered(uv.x, uv.y, sampling, footprint)
                    };

                    for (total, value) in rgb.iter_mut().zip(sampled) {
                        *total += value * weight;
                    }
                }

                match kind {
                    ChannelKind::Color if is_srgb_color && !blend_linear => rgb.map(texturecraft_texel::srgb_to_linear),

                    // Normal channels are OpenGL (+Y up the image): a DirectX image's green flips
                    ChannelKind::NormalMap if *normal_format == NormalFormat::DirectX => [rgb[0], 1.0 - rgb[1], rgb[2]],
                    ChannelKind::Color | ChannelKind::NormalMap => rgb,

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
        let needs_gray = matches!(channel.kind(), ChannelKind::Color | ChannelKind::NormalMap) && matches!(kind, ChannelKind::Scalar | ChannelKind::Signed);
        if needs_gray { [lum(value); 3] } else { value }
    }

    /// The generator's ramp at this texel, before its global blur, balance, contrast and invert (on the section controls, before those too).
    fn generator_ramp(&mut self, generator: &Generator, maps: &GeneratorMaps) -> f32 {
        if let Some(wear) = wear_parameters(generator) {
            return self.wear_ramp(generator, &wear, maps);
        }

        if let Some(light) = &generator.light {
            let normal =
                self.generator_input_color(generator, BakedMap::WorldNormal).map_or_else(|| self.coords().nrm, |encoded| Vec3::from(encoded) * 2.0 - 1.0);
            let position = Vec3::from(self.position_input(generator));
            return sections::light_value(light, normal, position, generator.invert);
        }

        if let Some(section) = generator.section_controls() {
            return self.section_signal(generator, &section, maps);
        }

        // The kinds with only their own controls: resolved once per plan, not per texel
        let resolved;
        let controls = match maps.controls.as_deref() {
            Some(controls) => Some(controls),
            None => {
                resolved = generator.kind_controls();
                resolved.as_ref()
            }
        };
        match controls {
            Some(KindControls::Distance(distance)) => {
                let position = self.uniform_position_input(generator);
                return three_d_distance::distance_value(distance, position);
            }
            Some(KindControls::LinearGradient(gradient)) => {
                let position = self.uniform_position_input(generator);
                return three_d_distance::linear_gradient_value(gradient, position);
            }
            Some(KindControls::BorderDistance(border)) => return uv_border_distance::border_value(border, self.field_value(maps)),
            Some(KindControls::Grayscale(grayscale)) => return self.grayscale_conversion(grayscale),
            Some(KindControls::DrippingRust(rust)) => return dripping_rust::rust_value(rust, self.field_value(maps)),
            Some(KindControls::UvRandomColor(_) | KindControls::UvChecker(_)) => return self.field_value(maps),
            None => {}
        }

        let coords = self.coords();
        let earlier_micro = generator.earlier_micro().map_or(0.0, |(anchor, amount)| self.earlier_micro_detail(generator.kind, anchor, amount));

        // Micro details saved on the earlier model add to its curvature and occlusion inputs too
        let signal = match (generator.kind, &generator.detail) {
            (GeneratorKind::Curvature | GeneratorKind::EdgeWear, Some(detail)) => {
                // Detail sizes are blurred copies of the mesh map, so a replaced curvature input reads at its own size only
                let sizes = if generator.inputs.contains_key(&BakedMap::Curvature) { None } else { maps.sizes.as_deref() };
                let curvature = self.curvature_input(generator, maps);
                self.curvature_detail(detail, sizes, curvature, earlier_micro)
            }
            (GeneratorKind::EdgeWear, None) | (GeneratorKind::FiberGlassEdgeWear, _) => (self.curvature_input(generator, maps).unwrap_or(0.5) - 0.5) * 2.0,
            (GeneratorKind::Curvature, None) => self.curvature_input(generator, maps).unwrap_or(0.5),
            (GeneratorKind::Dirt, _) => 1.0 - self.occlusion_input(generator, maps).unwrap_or(1.0),
            (GeneratorKind::AmbientOcclusion, _) => self.occlusion_input(generator, maps).unwrap_or(1.0),
            (GeneratorKind::ThinAreas, _) => 1.0 - self.generator_input(generator, BakedMap::Thickness).unwrap_or(1.0),
            (GeneratorKind::Position, _) => coords.pos.dot(Vec3::from(generator.direction).try_normalize().unwrap_or(Vec3::Y)) * 0.5 + 0.5,
            (GeneratorKind::Direction, _) => coords.nrm.dot(Vec3::from(generator.direction).try_normalize().unwrap_or(Vec3::Y)) * 0.5 + 0.5,
            // World Space Normals and 3D Distance always compute with their own controls: these arms only keep the match whole
            (GeneratorKind::WorldSpaceNormals, _) => coords.nrm.y * 0.5 + 0.5,
            (
                GeneratorKind::ThreeDDistance
                | GeneratorKind::ThreeDLinearGradient
                | GeneratorKind::UvBorderDistance
                | GeneratorKind::GrayscaleConversion
                | GeneratorKind::DrippingRust
                | GeneratorKind::UvRandomColor
                | GeneratorKind::UvChecker,
                _,
            ) => 0.0,
        };
        let signal = if matches!(generator.kind, GeneratorKind::Curvature | GeneratorKind::EdgeWear) && generator.detail.is_some() {
            signal
        } else {
            signal + earlier_micro
        };
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

    /// The signal a generator on its section controls adjusts at this texel: the axes of its map, its curvature or its occlusion
    fn section_signal(&mut self, generator: &Generator, section: &SectionParameters, maps: &GeneratorMaps) -> f32 {
        match generator.kind {
            GeneratorKind::Curvature => {
                let detail = generator.detail_in_use().unwrap_or_default();
                let sizes = if generator.inputs.contains_key(&BakedMap::Curvature) { None } else { maps.sizes.as_deref() };
                let curvature = self.curvature_input(generator, maps);
                self.curvature_detail(&detail, sizes, curvature, 0.0)
            }
            GeneratorKind::AmbientOcclusion => self.occlusion_input(generator, maps).unwrap_or(1.0),
            GeneratorKind::Position => {
                let position = self.position_input(generator);
                sections::axes_signal(position, &section.axes)
            }
            GeneratorKind::WorldSpaceNormals => {
                let normal = self.generator_input_color(generator, BakedMap::WorldNormal).unwrap_or_else(|| (self.coords().nrm * 0.5 + 0.5).to_array());
                sections::axes_signal(normal, &section.axes)
            }

            // Other kinds use their own evaluator
            GeneratorKind::EdgeWear
            | GeneratorKind::Dirt
            | GeneratorKind::ThinAreas
            | GeneratorKind::Direction
            | GeneratorKind::FiberGlassEdgeWear
            | GeneratorKind::ThreeDDistance
            | GeneratorKind::ThreeDLinearGradient
            | GeneratorKind::UvBorderDistance
            | GeneratorKind::GrayscaleConversion
            | GeneratorKind::DrippingRust
            | GeneratorKind::UvRandomColor
            | GeneratorKind::UvChecker => 0.0,
        }
    }

    /// The image in `generator`'s Position input, else this texel's position across the mesh's bounds: the baked Position map is not read, its normalisation is a bake setting
    fn position_input(&mut self, generator: &Generator) -> [f32; 3] {
        match generator.inputs.get(&BakedMap::Position) {
            Some(source) => self.source(source, &Projection::default(), Sampling::default(), ChannelKind::NormalMap),
            None => self.bounds_position().to_array(),
        }
    }

    /// A generator's whole-map field at this texel (0 without one)
    fn field_value(&self, maps: &GeneratorMaps) -> f32 {
        let texel = (self.y * self.inp.gbuf.width + self.x) as usize;
        maps.field.as_ref().and_then(|field| field.get(texel)).copied().unwrap_or(0.0)
    }

    /// The Grayscale Conversion generator at this texel: its image tiled and turned about the UV square's centre, made gray, balanced and contrasted
    fn grayscale_conversion(&mut self, grayscale: &GrayscaleConversionParameters) -> f32 {
        let degrees = if grayscale.safe_rotation { (grayscale.rotation / QUARTER_TURN_DEGREES).round() * QUARTER_TURN_DEGREES } else { grayscale.rotation };
        let (sin, cos) = degrees.to_radians().sin_cos();
        let centred = self.coords().uv - Vec2::splat(0.5);
        let at = Vec2::new(centred.x * cos - centred.y * sin, centred.x * sin + centred.y * cos) * grayscale.tile + Vec2::splat(0.5);
        let rgb = match grayscale.source.as_ref().unwrap_or(&EMPTY_IMAGE_SLOT) {
            Source::Image { name, .. } => self.inp.images.get(name).map_or([0.0; 3], |image| {
                let [red, green, blue, _] = image.sample(at.x, at.y);
                [red, green, blue]
            }),
            other => self.source(other, &Projection::default(), Sampling::default(), ChannelKind::NormalMap),
        };
        let gray = grayscale.grayscale.gray(rgb).clamp(0.0, 1.0);
        GeneratorGlobal { balance: grayscale.balance, contrast: grayscale.contrast, ..GeneratorGlobal::default() }.apply(gray)
    }

    /// The image in `generator`'s Position input, else this texel's position across the mesh's bounds scaled evenly: the longest side spans 0..1, the others centred on 0.5
    fn uniform_position_input(&mut self, generator: &Generator) -> Vec3 {
        if let Some(source) = generator.inputs.get(&BakedMap::Position) {
            return Vec3::from(self.source(source, &Projection::default(), Sampling::default(), ChannelKind::NormalMap));
        }

        let (low, high) = self.inp.bounds;
        let longest = (high - low).max_element().max(MIN_BOUNDS_EXTENT);
        self.world().map_or(Vec3::splat(0.5), |position| (position - (low + high) * 0.5) / longest + Vec3::splat(0.5))
    }

    /// This texel's world position across the mesh's bounds, 0..1 on each axis (0 where uncovered)
    fn bounds_position(&self) -> Vec3 {
        let (low, high) = self.inp.bounds;
        self.world().map_or(Vec3::ZERO, |position| ((position - low) / (high - low).max(Vec3::splat(MIN_BOUNDS_EXTENT))).clamp(Vec3::ZERO, Vec3::ONE))
    }

    /// A wear generator's value at this texel from its curvature, occlusion and grunge, before its global adjustments and invert
    fn wear_ramp(&mut self, generator: &Generator, wear: &WearParameters, maps: &GeneratorMaps) -> f32 {
        let curvature = self.curvature_input(generator, maps).map_or(0.0, |curvature| curvature - 0.5);

        // Blurred levels are copies of the mesh map, so a replaced curvature input is read at its own size only
        let sizes = if generator.inputs.contains_key(&BakedMap::Curvature) { None } else { maps.sizes.as_deref() };
        let blurred_curvature = self.blurred_curvature(wear::blur_sigma(generator.kind, wear), sizes, curvature);

        let occlusion = self.occlusion_input(generator, maps).unwrap_or(1.0);
        let grunge = if wear.custom_grunge {
            // An image not loaded yet reads mid gray, which leaves the wear as without grunge
            let gray = self.slot_color(generator, generator.grunge.as_ref(), &wear::custom_grunge_placement(wear)).map_or(0.5, lum);
            wear::custom_grunge(generator.kind, gray)
        } else {
            wear::grunge(generator.kind, wear, generator.seed, &self.coords())
        };
        wear::wear_value(generator.kind, wear, &WearInputs { curvature, blurred_curvature, occlusion, grunge })
    }

    /// The curvature deviation at this texel blurred to `sigma` (texels at 2K) between the two built levels around it; `curvature` without them
    fn blurred_curvature(&self, sigma: f32, curvature_sizes: Option<&CurvatureSizes>, curvature: f32) -> f32 {
        let Some((sizes, (baked_x, baked_y))) = curvature_sizes.zip(self.baked_curvature_texel()) else { return curvature };

        let level = |index: usize| sizes.levels.get(index).and_then(Option::as_deref).map(|level| level.sample(baked_x, baked_y));
        let (narrower, wider, weight) = wear::levels_around(sigma);
        let narrower = narrower.map_or(Some(curvature), level);
        match (narrower, level(wider)) {
            (Some(narrower), Some(wider)) => narrower + (wider - narrower) * weight,
            _ => curvature,
        }
    }

    /// The baked curvature map's texel under this texel (None outside the baked layout)
    fn baked_curvature_texel(&self) -> Option<(u32, u32)> {
        let working_size = (self.res, self.inp.gbuf.height);
        let texel_uv = [(self.x as f32 + 0.5) / working_size.0.max(1) as f32, (self.y as f32 + 0.5) / working_size.1.max(1) as f32];
        let baked_uv = self.inp.block_scales.bake_uv(texel_uv, working_size)?;
        let baked_size = (self.inp.baked.resolution, self.inp.baked.height(BakedMap::Curvature));
        Some(((baked_uv[0] * baked_size.0 as f32) as u32, (baked_uv[1] * baked_size.1 as f32) as u32))
    }

    /// The generator's value at this texel: its ramp finished by balance, contrast, invert and offset (its blur and resolution need neighbours: see [`generator_region`])
    fn generator(&mut self, generator: &Generator, maps: &GeneratorMaps) -> f32 {
        let ramp = self.generator_ramp(generator, maps);
        let adjusted = generator.section_controls().map_or(ramp, |section| sections::adjusted(generator.kind, &section, ramp));

        // An image still loading reads black, as it does where blur or resolution compute the generator over a region
        let textured = match generator.texture_in_use() {
            Some(texture) => texture_layer::blended_over(texture, adjusted, self.generator_texture(generator).unwrap_or(0.0)),
            None => adjusted,
        };

        finish_generator(generator, textured)
    }

    /// The gray of the generator's texture at this texel (None when it blends no texture)
    fn generator_texture(&mut self, generator: &Generator) -> Option<f32> {
        let texture = generator.texture_in_use()?;
        let rgb = self.slot_color(generator, texture.source.as_ref(), &texture_layer::texture_placement(texture))?;
        Some(texture_layer::texture_gray(texture, rgb))
    }

    /// The color of a generator's image slot holding `source` (None: empty, read as black) at this texel, read where `placement` puts it (None when its image isn't loaded)
    fn slot_color(&mut self, generator: &Generator, source: Option<&Source>, placement: &texture_layer::SlotPlacement) -> Option<[f32; 3]> {
        match source.unwrap_or(&EMPTY_IMAGE_SLOT) {
            Source::Image { name, .. } => {
                let image = self.inp.images.get(name)?.clone();
                let read = |coordinates: Vec2| {
                    let [red, green, blue, _] = image.sample(coordinates.x, coordinates.y);
                    Vec3::new(red, green, blue)
                };
                let color = match placement.triplanar_contrast {
                    Some(contrast) => {
                        let normal = self
                            .generator_input_color(generator, BakedMap::WorldNormal)
                            .map_or_else(|| self.coords().nrm, |encoded| Vec3::from(encoded) * 2.0 - 1.0);
                        let weights = texture_layer::triplanar_weights(contrast, normal);
                        let coordinates = texture_layer::triplanar_coordinates(placement, self.bounds_position());
                        weights.iter().zip(coordinates).fold(Vec3::ZERO, |sum, (weight, at)| sum + read(at) * *weight)
                    }
                    None => read(texture_layer::uv_coordinates(placement, self.coords().uv)),
                };
                Some(color.to_array())
            }

            // A value or an anchor reads the same wherever it is projected
            other => Some(self.source(other, &Projection::default(), Sampling::default(), ChannelKind::NormalMap)),
        }
    }

    /// How a micro detail saved before Micro Details moves an earlier-model generator's signal at this texel: anchor point `anchor`'s height times `amount` (0 where the anchor is not evaluated yet), an edge where raised and a cavity where recessed
    fn earlier_micro_detail(&self, kind: GeneratorKind, anchor: LayerId, amount: f32) -> f32 {
        let response = self.anchor_value(anchor, Channel::Height, ChannelKind::Signed)[0].clamp(-1.0, 1.0) * amount;
        match kind {
            GeneratorKind::Curvature | GeneratorKind::EdgeWear | GeneratorKind::AmbientOcclusion => response,

            // Dirt gathers in dents
            GeneratorKind::Dirt => -response,

            // Kinds the micro detail never moved, or without an earlier model
            GeneratorKind::FiberGlassEdgeWear
            | GeneratorKind::ThinAreas
            | GeneratorKind::Position
            | GeneratorKind::Direction
            | GeneratorKind::WorldSpaceNormals
            | GeneratorKind::ThreeDDistance
            | GeneratorKind::ThreeDLinearGradient
            | GeneratorKind::UvBorderDistance
            | GeneratorKind::GrayscaleConversion
            | GeneratorKind::DrippingRust
            | GeneratorKind::UvRandomColor
            | GeneratorKind::UvChecker => 0.0,
        }
    }

    /// The generator's curvature input at this texel with what its micro details add
    fn curvature_input(&mut self, generator: &Generator, maps: &GeneratorMaps) -> Option<f32> {
        let curvature = self.generator_input(generator, BakedMap::Curvature)?;
        let working_size = (self.res, self.inp.gbuf.height);
        let micro = maps.micro_curvature.as_ref().and_then(|micro| micro.term.at((self.x, self.y), working_size, &self.inp.block_scales));
        Some(curvature + micro.unwrap_or(0.0))
    }

    /// The generator's occlusion input at this texel, darkened by its micro details
    fn occlusion_input(&mut self, generator: &Generator, maps: &GeneratorMaps) -> Option<f32> {
        let occlusion = self.generator_input(generator, BakedMap::AmbientOcclusion)?;
        let working_size = (self.res, self.inp.gbuf.height);
        let micro = maps.micro_occlusion.as_ref().and_then(|micro| micro.at((self.x, self.y), working_size, &self.inp.block_scales));
        Some(occlusion * micro.unwrap_or(1.0))
    }

    /// Generator input `map` at this texel: what replaces the mesh map in the generator's inputs, else the mesh map (None when not baked).
    fn generator_input(&mut self, generator: &Generator, map: BakedMap) -> Option<f32> {
        match generator.inputs.get(&map) {
            // Keep legacy sampling for generator inputs without a sampling setting
            Some(source) => Some(self.source(source, &Projection::default(), Sampling::SAVED_BEFORE, ChannelKind::Scalar)[0]),
            None => self.baked(map),
        }
    }

    /// Generator input `map` at this texel as three channels (an encoded normal or position): what replaces the mesh map, else the mesh map (None when not baked)
    fn generator_input_color(&mut self, generator: &Generator, map: BakedMap) -> Option<[f32; 3]> {
        match generator.inputs.get(&map) {
            Some(source) => Some(self.source(source, &Projection::default(), Sampling::default(), ChannelKind::NormalMap)),
            None => self.baked_color(map),
        }
    }

    /// The curvature of `detail`'s mode at this texel from `curvature` (None: nothing to read), with its blurred detail sizes when given and an earlier micro detail's term (see [`Texel::earlier_micro_detail`]) as an edge where positive and a cavity where negative
    fn curvature_detail(&self, detail: &CurvatureDetail, curvature_sizes: Option<&CurvatureSizes>, curvature: Option<f32>, earlier_micro: f32) -> f32 {
        let combine = |edges: f32, cavities: f32| {
            let edges = 1.0 - (1.0 - edges) * (1.0 - earlier_micro.max(0.0));
            let cavities = 1.0 - (1.0 - cavities) * (1.0 + earlier_micro.min(0.0));
            match detail.mode {
                CurvatureMode::Edges => edges,
                CurvatureMode::Cavities => cavities,
                CurvatureMode::Both => 1.0 - (1.0 - edges) * (1.0 - cavities),
                CurvatureMode::Raw => 0.5 + 0.5 * (edges - cavities),
            }
        };

        // Without curvature to read only an earlier micro detail supplies edges or cavities
        let Some(curvature) = curvature else { return combine(0.0, 0.0) };
        let Some((baked_x, baked_y)) = self.baked_curvature_texel() else { return combine(0.0, 0.0) };

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
            && let Some(filtered) = field.get(self.x, self.y).and_then(|texel| texel.first().copied())
        {
            value = filtered;
            skip = filtered_index + 1;
        }

        // Replay the prefix to record anchors skipped by the precomputed filter
        if let Some(last) = plan.effects.iter().take(skip).rposition(|e| matches!(*e.kind, EffectKind::Anchor)) {
            self.mask_effects(plan, plan.background, 0, last + 1);
        }
        self.mask_effects(plan, value, skip, plan.effects.len())
    }

    /// Mask effects `from..to` of `plan` over `value`, recording the anchors among them.
    fn mask_effects(&mut self, plan: &MaskPlan, mut value: f32, from: usize, to: usize) -> f32 {
        // Avoid amplifying rounding error when unpremultiplying faint paint
        const MIN_PAINT_COVERAGE: f32 = 1e-5;

        for effect in plan.effects.iter().take(to).skip(from) {
            if let Some(filtered) = effect.filtered_mask.and_then(|field| field.get(self.x, self.y)).and_then(|texel| texel.first()) {
                value = *filtered;
                continue;
            }

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
                            let deposited = preview.carried_mask.as_ref().map_or_else(
                                || preview.mask_gray.at(self.x, self.y, (self.inp.gbuf.width, self.inp.gbuf.height)),
                                |carried| from_u16(carried.get(self.x, self.y)[0]),
                            );
                            painted = deposited * stroke + painted * (1.0 - stroke);
                            coverage = stroke + coverage * (1.0 - stroke);
                        }
                    }
                    if coverage > 0.0 {
                        let layer_value = painted / coverage.max(MIN_PAINT_COVERAGE);
                        value += (blend1(effect.blend, value, layer_value) - value) * coverage * opacity;
                    }
                }
                EffectKind::Fill { source, projection, sampling } => {
                    let projection = effect.projection.as_ref().unwrap_or(projection);
                    let once = |texel: &mut Self| {
                        let fill = texel.source(source, projection, *sampling, ChannelKind::Scalar);
                        let cover = if effect.covers_all { 1.0 } else { texel.cover(std::iter::once(source), projection, *sampling, effect.depth.as_deref()) };
                        (fill, cover)
                    };

                    // Sampled directly without symmetry, as `Texel::fill_place_texel` is
                    let ([fill, ..], cover) = if takes_symmetry(projection, *sampling) { self.symmetric(projection, *sampling, once) } else { once(self) };
                    value += (blend1(effect.blend, value, fill) - value) * opacity * cover;
                }
                EffectKind::Generator(generator) => {
                    let generated = self.generator(generator, &effect.maps);
                    value += (blend1(effect.blend, value, generated) - value) * opacity;
                }
                EffectKind::Levels(levels) => value += (levels.apply(value) - value) * opacity,
                EffectKind::Filter(filter @ Filter::GradientMap { debug: true, .. }) => {
                    let ramp = effect.ramp_at(filter, self.coords().uv.x.rem_euclid(1.0)).map_or(value, lum);
                    value += (ramp - value) * opacity;
                }
                EffectKind::Filter(Filter::GrayscaleConversion(conversion)) => {
                    let read = match &effect.input {
                        Some(source) => self.source(source, &Projection::default(), Sampling::default(), ChannelKind::Color),
                        None => [value; 3],
                    };
                    value += (conversion.gray(read, 1.0) - value) * opacity;
                }
                EffectKind::Filter(filter) => {
                    // A gray mask uses HSL lightness and reads neighbourhoods from the prepared mask plan
                    let filtered = effect.point(filter, ChannelKind::Scalar, [value; 3]).map_or(value, |gray| gray[0]);
                    value += (filtered - value) * opacity;
                }
                EffectKind::ColorSelection(selection) => {
                    let (selected, cover) = selection.value(self.id_color(selection.id_map.as_deref()));
                    value += (blend1(effect.blend, value, selected) - value) * opacity * cover;
                }
                EffectKind::Compare { op, value: constant, tolerance, left, right, channel, hardness } => {
                    let left = self.compare_side(*left, *channel, plan, value, *constant);
                    let right = self.compare_side(*right, *channel, plan, value, *constant);
                    let compared = op.mask(left, right, *tolerance, *hardness);
                    value += (blend1(effect.blend, value, compared) - value) * opacity;
                }

                // Every channel of a mask anchor reads the mask's gray
                EffectKind::Anchor => self.anchors.push((effect.id, [[value.clamp(0.0, 1.0); 3]; NCH], true)),

                // Content-only effects
                EffectKind::Material(_) => {}
            }
        }
        value.clamp(0.0, 1.0)
    }

    /// What one side of a compare effect in mask `plan` reads at this texel (0..1): `mask` is the mask so far
    fn compare_side(&mut self, side: CompareSource, channel: Channel, plan: &MaskPlan, mask: f32, constant: f32) -> f32 {
        let stack_below = self.below;
        let below = || stack_below.and_then(|acc| acc.get(channel.index()).copied()).unwrap_or_else(|| channel.default_value());
        let read = match side {
            CompareSource::ThisMask => return mask,
            CompareSource::Constant => return constant,
            CompareSource::LayersBelow => below(),
            CompareSource::ThisLayer => {
                let own = plan.layer.as_ref().and_then(|(content, chans)| {
                    let chan_plan = chans.iter().find(|chan_plan| chan_plan.ch == channel.index())?;
                    self.content(content, chan_plan)
                });

                // Where the layer is empty (or thin) it shows the stack below it
                let below = below();
                own.map_or(below, |(value, coverage)| mix3(below, value, coverage))
            }
        };
        match channel.kind() {
            ChannelKind::Color | ChannelKind::NormalMap => lum(read),
            ChannelKind::Scalar => read[0],
            ChannelKind::Signed => read[0] * 0.5 + 0.5,
        }
    }

    /// Composite `nodes` over `acc`, recording anchors; true when it reached the `stop` layer (`acc` then holds the stack below it)
    fn eval(&mut self, nodes: &[Node], acc: &mut Acc) -> bool {
        for node in nodes {
            if self.stop == Some(node.draw) {
                self.below = Some(*acc);
                return true;
            }

            if node.mask.as_ref().is_some_and(|mask| mask.layer.is_some()) {
                self.below = Some(*acc);
            }
            let mask = self.tape_mask(node);
            match &node.content {
                // Masked-out groups still run when they record anchors or hold the stop layer
                Content::Group(children) if mask > 0.0 || node.has_anchor || self.stop.is_some() => {
                    let mut inner = *acc;
                    if self.eval(children, &mut inner) {
                        return true;
                    }

                    if mask > 0.0 {
                        mix_group(acc, &inner, &node.chans, mask);
                    }
                }
                Content::Fill { .. } | Content::Paint { .. } if mask > 0.0 => {
                    let before = if node.anchors.iter().any(|(_, cut)| cut.is_some()) { Some(*acc) } else { None };

                    // A layer lit by its material evaluates its other channels first (see `Texel::lit_content`)
                    if node.lights {
                        self.lit_content(node, acc, mask);
                    } else {
                        self.lit = None;
                        for chan_plan in &node.chans {
                            if let Some((value, coverage)) = self.tape_content(&node.content, chan_plan) {
                                apply(acc, chan_plan, value, coverage, mask);
                            }
                        }
                    }

                    // Anchors inside the content stack read the stack cut at that point, lit by the same material
                    for (anchor, cut) in &node.anchors {
                        let recorded = match (cut, before) {
                            (Some(cut), Some(mut cut_acc)) => {
                                for chan_plan in &node.chans {
                                    if let Some((value, coverage)) = self.content_to(&node.content, chan_plan, *cut) {
                                        apply(&mut cut_acc, chan_plan, value, coverage, mask);
                                    }
                                }
                                cut_acc
                            }
                            _ => *acc,
                        };
                        self.anchors.push((*anchor, recorded, false));
                    }
                    self.lit = None;
                    continue;
                }
                _ => {}
            }
            for (anchor, _) in &node.anchors {
                self.anchors.push((*anchor, *acc, false));
            }
        }
        false
    }

    /// Lay a layer lit by its material over `acc` under `mask`: its other channels go first, each evaluated once, and `self.lit` holds them over the stack below for its base colour's lighting
    #[inline(never)]
    fn lit_content(&mut self, node: &Node, acc: &mut Acc, mask: f32) {
        let base = Channel::BaseColor.index();
        let (material, others) = self.material(node, *acc, mask);
        self.lit = Some(material);

        for chan_plan in &node.chans {
            let content = match others.get(chan_plan.ch) {
                Some(other) if chan_plan.ch != base => *other,
                _ => self.tape_content(&node.content, chan_plan),
            };
            if let Some((value, coverage)) = content {
                apply(acc, chan_plan, value, coverage, mask);
            }
        }
    }

    /// Layer `node`'s other channels than its base colour laid over `below` under `mask`, and each one's content
    fn material(&mut self, node: &Node, below: Acc, mask: f32) -> (Acc, ChannelContents) {
        let base = Channel::BaseColor.index();
        let mut others = [None; NCH];
        let mut material = below;
        for chan_plan in node.chans.iter().filter(|chan_plan| chan_plan.ch != base) {
            let content = self.tape_content(&node.content, chan_plan);
            if let Some((value, coverage)) = content {
                apply(&mut material, chan_plan, value, coverage, mask);
            }
            if let Some(slot) = others.get_mut(chan_plan.ch) {
                *slot = content;
            }
        }
        (material, others)
    }

    /// Light layer `node`'s base colour as the full render does, after [`Texel::anchors_below`] recorded the stack below it
    fn light(&mut self, node: &Node) {
        let below = self.below.unwrap_or_else(|| Channel::ALL.map(Channel::default_value));
        let mask = self.node_mask(node);
        self.lit = Some(self.material(node, below, mask).0);
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

        // A hidden or masked folder limits how far the evaluator descends
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

        // Back up: the layers after the path, then each folder's mix over the stack outside it
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
        if n.parts.as_ref().is_some_and(|p| !self.in_parts(p)) {
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

        // Submeshes are in index order; anything else is searched
        let k = subs.partition_point(|s| s.start <= first).checked_sub(1).filter(|k| holds(*k)).or_else(|| (0..subs.len()).find(|k| holds(*k)));
        k.and_then(|k| allowed.get(k).copied()).unwrap_or(true)
    }

    /// Record the anchors of the stack below layer `id` (`full` is the planned stack), to evaluate that layer on its own
    fn anchors_below(&mut self, full: &[Node], draw: (LayerId, usize)) {
        self.anchors.clear();
        self.stop = Some(draw);
        let mut acc = Channel::ALL.map(Channel::default_value);
        self.eval(full, &mut acc);
        self.stop = None;
    }

    /// Straight value and coverage of one channel of a paint or fill layer at this texel (content effects and live stroke preview included); None where the layer is empty
    fn content(&mut self, content: &Content, c: &ChanPlan) -> Option<([f32; 3], f32)> {
        self.content_to(content, c, usize::MAX)
    }

    /// [`Texel::content`] with only the first `end` visible content effects
    fn content_to(&mut self, content: &Content, chan_plan: &ChanPlan, end: usize) -> Option<([f32; 3], f32)> {
        let start = chan_plan.start.and_then(|(stage, field)| {
            if stage < end {
                Some((stage, field))
            } else {
                field.anchor_stages.iter().rev().find(|(stage, _)| *stage < end).map(|(stage, field)| (*stage, field))
            }
        });
        let (mut value, mut coverage, skip) = match start.and_then(|(stage, field)| field.get(self.x, self.y).map(|texel| (stage, texel))) {
            Some((stage, texel)) => {
                let component = |i: usize| texel.get(i).copied().unwrap_or(0.0);
                ([component(0), component(1), component(2)], component(3), stage + 1)
            }
            None => {
                let (value, coverage) = match content {
                    Content::Fill { fill, projection, depth, covers_all } => {
                        let ch = Channel::ALL.get(chan_plan.ch).copied()?;
                        let source = fill.sources.get(&ch);
                        let projection = projection.as_deref().unwrap_or(&fill.projection);
                        self.fill_place_texel(FillPlace { fill, projection, depth: depth.as_deref(), covers_all: *covers_all }, source, ch)
                    }
                    Content::Paint { preview } => self.painted(chan_plan.data.as_deref(), chan_plan, *preview),
                    Content::Group(_) => return None,
                };
                (value, coverage, 0)
            }
        };
        for effect in chan_plan.effects.iter().take(end).skip(skip) {
            (value, coverage) = self.content_effect(effect, chan_plan, value, coverage);
        }
        (coverage > 0.0).then_some((value, coverage.min(1.0)))
    }

    /// Straight value and coverage of painted data of one channel (premultiplied values plus
    /// coverage per texel), with the live stroke when `preview`.
    fn painted(&mut self, data: Option<&[u16]>, c: &ChanPlan, preview: bool) -> ([f32; 3], f32) {
        let pv = if preview { self.inp.preview } else { None };
        let mut sa = pv.map(|p| (self.stroke * p.opacity).clamp(0.0, 1.0)).unwrap_or(0.0);

        // Projection: the stencil's alpha shapes the stroke and its color is the base color
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

    /// Apply one content effect to a channel's straight value and coverage
    fn content_effect(&mut self, effect: &EffectPlan, chan_plan: &ChanPlan, value: [f32; 3], coverage: f32) -> ([f32; 3], f32) {
        if effect.blend == BlendMode::Disable && effect.kind.blends() {
            return (value, coverage);
        }

        let opacity = effect.opacity.clamp(0.0, 1.0);
        match effect.kind {
            EffectKind::Paint | EffectKind::Path(_) => {
                let (painted, painted_coverage) = self.painted(effect.data.as_deref(), chan_plan, effect.preview);
                over(chan_plan, effect.blend, value, coverage, painted, painted_coverage * opacity)
            }
            EffectKind::Material(fill) => match Channel::ALL.get(chan_plan.ch).and_then(|ch| fill.sources.get(ch).map(|source| (*ch, source))) {
                Some((ch, source)) => {
                    let projection = effect.projection.as_ref().unwrap_or(&fill.projection);
                    let place = FillPlace { fill, projection, depth: effect.depth.as_deref(), covers_all: effect.covers_all };
                    let (material_value, weight) = self.fill_place_texel(place, Some(source), ch);
                    over(chan_plan, effect.blend, value, coverage, material_value, weight * opacity)
                }
                None => (value, coverage),
            },
            EffectKind::Levels(levels) => match Channel::ALL.get(chan_plan.ch) {
                Some(&channel) if levels.remaps(channel) => (mix3(value, per_unit(chan_plan.kind, value, |x| levels.apply(x)), opacity), coverage),
                _ => (value, coverage),
            },
            // Baked lighting reads the texel's normal and occlusion, and lights the base color only
            EffectKind::Filter(filter @ Filter::BakedLighting { .. }) => match Channel::ALL.get(chan_plan.ch) {
                Some(Channel::BaseColor) => {
                    let ao = self.baked(BakedMap::AmbientOcclusion).unwrap_or(1.0);
                    let lit = match &effect.stylized {
                        Some(stylized) => {
                            let nrm = self.lit_normal(stylized.relief());
                            let curvature = self.baked(BakedMap::Curvature).unwrap_or(FLAT_CURVATURE);
                            let material = self.lit.unwrap_or_else(|| Channel::ALL.map(Channel::default_value));
                            stylized.shade(value, nrm, [ao, curvature], &material)
                        }
                        None => {
                            let factor = filter.baked_light(self.coords().nrm.to_array(), ao).unwrap_or(1.0);
                            value.map(|x| (x * factor).clamp(0.0, 1.0))
                        }
                    };
                    (mix3(value, lit, opacity), coverage)
                }
                _ => (value, coverage),
            },
            // Environment lighting reads the texel's normal, occlusion and metal, and lights the base color only
            EffectKind::Filter(Filter::EnvironmentLighting { horizontal, vertical, exposure, height_intensity, ambient_occlusion, specular_occlusion }) => {
                match (Channel::ALL.get(chan_plan.ch), &effect.environment) {
                    (Some(Channel::BaseColor), Some(environment)) => {
                        let ao = self.baked(BakedMap::AmbientOcclusion).unwrap_or(1.0);
                        let nrm = Vec3::from(self.detail_normal(height_intensity / ENVIRONMENT_HEIGHT_UNIT));
                        let light = environment.irradiance(nrm, horizontal.to_radians(), vertical.to_radians())
                            * exposure.clamp(-Filter::ENVIRONMENT_EXPOSURE_STOPS, Filter::ENVIRONMENT_EXPOSURE_STOPS).exp2();
                        let material = self.lit.unwrap_or_else(|| Channel::ALL.map(Channel::default_value));
                        let lit = EnvironmentSh::shade(value, light, ao, [*ambient_occlusion, *specular_occlusion], &material);
                        (mix3(value, lit, opacity), coverage)
                    }
                    _ => (value, coverage),
                }
            }

            // PBR Validate checks the base color against the layer's metal and leaves the other channels as they are
            EffectKind::Filter(Filter::PbrValidate(validate)) => match Channel::ALL.get(chan_plan.ch) {
                Some(Channel::BaseColor) => {
                    let metal = self.lit.and_then(|material| material.get(Channel::Metallic.index()).map(|metal| metal[0])).unwrap_or(0.0);
                    (mix3(value, validate.check(value, metal), opacity), coverage)
                }
                _ => (value, coverage),
            },
            // A gradient's debug view shows its ramp left to right across each UV tile
            EffectKind::Filter(filter @ Filter::GradientMap { debug: true, .. }) => {
                let ramp = effect
                    .ramp_at(filter, self.coords().uv.x.rem_euclid(1.0))
                    .map(|color| if chan_plan.kind == ChannelKind::Color { color } else { [lum(color); 3] });
                (ramp.map_or(value, |ramp| mix3(value, ramp, opacity)), coverage)
            }

            // Posterize with Apply to Alpha steps the content's coverage too
            EffectKind::Filter(filter @ Filter::Posterize { levels, alpha: true }) => {
                let stepped = coverage + (posterize(coverage, *levels) - coverage) * opacity;
                (filter.point(chan_plan.kind, value).map_or(value, |out| mix3(value, out, opacity)), stepped)
            }

            // Grayscale Conversion weighs the coverage as Alpha, or reads its Custom Input
            EffectKind::Filter(Filter::GrayscaleConversion(conversion)) => {
                let (custom_input, alpha) = match &effect.input {
                    Some(source) => (Some(self.source(source, &Projection::default(), Sampling::default(), ChannelKind::Color)), 1.0),
                    None => (None, coverage),
                };
                (mix3(value, conversion.channel_value(chan_plan.kind, value, custom_input, alpha), opacity), coverage)
            }

            // Neighbourhood filters come precomputed through `ChanPlan::start`
            EffectKind::Filter(filter) => (effect.point(filter, chan_plan.kind, value).map_or(value, |out| mix3(value, out, opacity)), coverage),
            // Anchors are recorded by `eval`; mask-only effects leave content as it is
            EffectKind::Anchor | EffectKind::Fill { .. } | EffectKind::Generator(_) | EffectKind::ColorSelection(_) | EffectKind::Compare { .. } => {
                (value, coverage)
            }
        }
    }
}

/// Mix a folder's inner stack into the stack outside it by its opacity and mask `m`.
fn mix_group(acc: &mut Acc, inner: &Acc, chans: &[ChanPlan], m: f32) {
    for c in chans {
        if let (Some(a), Some(b)) = (acc.get(c.ch).copied(), inner.get(c.ch).copied())
            && let Some(slot) = acc.get_mut(c.ch)
        {
            *slot = mix3(a, b, c.opacity * m);
        }
    }
}

#[inline]
fn apply(acc: &mut Acc, c: &ChanPlan, v: [f32; 3], cov: f32, mask: f32) {
    let Some(a) = acc.get(c.ch).copied() else { return };
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
/// filling it). The borrowed mesh is fingerprinted once per batch. Callers with an owned mesh
/// can use [`composite_tiles_with_projectors`] for constant-time identity checks.
pub fn composite_tiles_cached(inputs: &Inputs, out: &mut Composite, tiles: &[(u32, u32)], mut below: Option<&mut BelowCache>) {
    if let Some(cache) = below.as_deref_mut() {
        cache.bind_borrowed_mesh(inputs.mesh);
    }

    let projectors = ProjectorDepthCache::default();
    let mut inputs = EvaluationInputs::new(inputs, &projectors);
    composite_evaluation_tiles(&mut inputs, out, tiles, below);
}

/// Composite tile batches with a reusable cache for the mesh owned by the caller.
pub fn composite_tiles_with_projectors(
    inputs: &Inputs,
    mesh: &Arc<Mesh>,
    projectors: &mut ProjectorDepthCache,
    out: &mut Composite,
    tiles: &[(u32, u32)],
    mut below: Option<&mut BelowCache>,
) {
    if !std::ptr::eq(inputs.mesh, mesh.as_ref()) {
        composite_tiles_cached(inputs, out, tiles, None);
        return;
    }

    projectors.bind(mesh);
    if let Some(cache) = below.as_deref_mut() {
        cache.bind_mesh(mesh);
    }

    let mut inputs = EvaluationInputs::new(inputs, projectors).of_the_session_stack();
    composite_evaluation_tiles(&mut inputs, out, tiles, below);
}

fn composite_evaluation_tiles(inp: &mut EvaluationInputs, out: &mut Composite, tiles: &[(u32, u32)], mut below: Option<&mut BelowCache>) {
    let Ok(set) = inp.project.texture_set(inp.ts) else { return };
    if (inp.gbuf.width, inp.gbuf.height) != (out.resolution, out.height) {
        return;
    }

    if !tiles.is_empty() {
        prepare_generators(inp, None);
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

    let channels = evaluation_channels(inp, set.channel_list());
    if !blocks.is_empty() && channels.contains(&Channel::BaseColor) && uses_environment(inp.project, &set.stack, 0) {
        inp.environment();
    }

    let hot = split.as_deref().and_then(<[Level]>::last);

    // A stroke preview is absent from StackKey, so it disables field retention
    let retain_fields = below.is_some() && inp.preview.is_none() && has_neighbourhood(inp, &set.stack);
    let key = if retain_fields { StackKey::new(inp.project, inp.pixels, inp.baked, inp.images, inp.ts, &set.stack).map(Arc::new) } else { None };
    let size = (inp.gbuf.width, inp.gbuf.height);

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
                let fields = Arc::new(block_fields(inp, &channels, &set.stack, split.as_deref(), &uncached_planned_tiles, &planned_tiles));
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
            _ => Arc::new(block_fields(inp, &channels, &set.stack, split.as_deref(), &fresh, block)),
        };

        let quota = match below.as_deref() {
            Some(_) if fits => usize::MAX,
            Some(b) => b.limit().saturating_sub(b.bytes()) / tile_bytes,
            None => 0,
        };

        let cached = below.as_deref().zip(split.as_deref());
        let records = composite_block(inp, (&set.stack, &channels), out, block, cached, quota, &fields);
        if let Some(b) = below.as_deref_mut() {
            for t in block {
                b.touch(*t);
            }
            for (t, bands) in records {
                b.insert(t, bands);
            }
        }
    }

    // Release fields at the end of the pass
    if let Some(cache) = below.filter(|cache| cache.ahead.is_empty()) {
        cache.held = None;
    }
}

/// Whether a visible layer in this draw needs the diffuse environment before parallel tile planning.
fn uses_environment(project: &Project, stack: &[LayerId], depth: usize) -> bool {
    if depth > texturecraft_core::project::MAX_DEPTH {
        return false;
    }

    stack.iter().any(|id| {
        let Some(own) = project.layers.get(id).filter(|layer| layer.visible) else { return false };
        let Some(layer) = project.content_layer(own.id) else { return false };
        if !layer.children().is_empty() {
            return uses_environment(project, layer.children(), depth + 1);
        }

        layer.blend(Channel::BaseColor).is_some_and(|blend| blend.opacity > 0.0 && blend.mode != BlendMode::Disable)
            && layer.content_effects.iter().any(|effect| {
                effect.visible
                    && matches!(effect.kind, EffectKind::Filter(Filter::EnvironmentLighting { .. }))
                    && effect.applied_blend(Some(Channel::BaseColor)).opacity > 0.0
            })
    })
}

/// Whether a layer drawn by `stack` has a neighbourhood filter in its mask or content effects.
fn has_neighbourhood(inp: &EvaluationInputs, stack: &[LayerId]) -> bool {
    let working_width = inp.gbuf.width;
    let mut order = Vec::new();
    draw_order(inp, stack, 0, DrawPlacement::default(), false, &mut order);
    order.into_iter().any(|(id, _)| {
        let masked = layer_mask(inp.project, id).is_some_and(|mask| mask.enabled && neighbourhood(&mask.effects, working_width).is_some());
        masked || inp.project.content_layer(id).is_some_and(|layer| !layer.is_group() && neighbourhood(&layer.content_effects, working_width).is_some())
    })
}

/// The neighbourhood fields compositing `tiles` reads: those of every layer of `stack` over the tiles the cache lacks (`fresh`),
/// and off the path to the `hot` layer only there; the hot layer's over all of `tiles`.
fn block_fields(
    inp: &EvaluationInputs,
    channels: &[Channel],
    stack: &[LayerId],
    split: Option<&[Level]>,
    fresh: &[(u32, u32)],
    tiles: &[(u32, u32)],
) -> Fields {
    let hot = split.and_then(|levels| levels.last().zip(level_offsets(inp.project, levels).last().copied()));
    match hot {
        Some((hot, around)) if fresh.len() < tiles.len() => {
            let mut fields = fields_of(inp, channels, (stack, ContentsOffset::NONE), fresh);
            fields.extend(fields_of(inp, channels, (&[hot.at], around), tiles));
            fields
        }
        _ => fields_of(inp, channels, (stack, ContentsOffset::NONE), tiles),
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
    inp: &EvaluationInputs,
    (stack, planned_channels): (&[LayerId], &[Channel]),
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
    let defaults: Acc = Channel::ALL.map(Channel::default_value);

    // Tiles split into bands of rows, so a handful of tiles still keeps every thread busy
    let bands = TILE / BAND;
    let tasks: Vec<((u32, u32), u32)> = tiles.iter().flat_map(|t| (0..bands).map(move |b| (*t, b))).collect();
    let mut results: Vec<BandResult> = tasks
        .par_iter()
        .map(|&((tx, ty), band)| {
            let warm = cached((tx, ty)).and_then(|b| b.get(band as usize));
            let record = warm.is_none() && recorded.contains(&(tx, ty));
            let levels = split.map(|l| plan_levels(inp, l, planned_channels, tx, ty, fields, warm.is_some()));
            let nodes = if levels.is_none() { plan(inp, stack, planned_channels, tx, ty, 0, Some(fields)) } else { Vec::new() };
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

                        // Reuse one anchor buffer per band
                        texel.anchors = std::mem::take(&mut anchors);
                        match &levels {
                            Some(levels) => {
                                // The texel's stream starts with the stack at each level
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

    // Join the bands of each tile (in parallel; uncovered tiles are freed)
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

        // Tiles without covered texels cost nothing to composite: not worth caching
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
    let projectors = ProjectorDepthCache::default();
    let inputs = &mut EvaluationInputs::new(inputs, &projectors);
    let mask = layer_mask(inputs.project, id).filter(|mask| mask.enabled)?;
    let channels = evaluation_channels(inputs, inputs.project.texture_set(inputs.ts).map(|set| set.channel_list()).unwrap_or_default());
    let (width, height) = (inputs.gbuf.width, inputs.gbuf.height);
    let coordinates = all_tiles(width, height);

    // A hidden layer's mask is left out of the stack's preparation
    prepare_generators(inputs, Some(id));
    prepare_mask_curvature_sizes(inputs, std::iter::once(mask));

    let fields = evaluation_fields(inputs, &channels, &coordinates);
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
/// Returns None without an enabled mask or when the pixel count exceeds the supported range
pub fn mask_image(inputs: &Inputs, id: LayerId, size: u32) -> Option<Vec<u8>> {
    let size = size.max(1);
    let pixel_count = usize::try_from(size.checked_mul(size)?).ok()?;
    let projectors = ProjectorDepthCache::default();
    let inputs = &mut EvaluationInputs::new(inputs, &projectors);
    let mask = layer_mask(inputs.project, id).filter(|mask| mask.enabled)?;
    let channels = evaluation_channels(inputs, inputs.project.texture_set(inputs.ts).map(|set| set.channel_list()).unwrap_or_default());
    let (width, height) = (inputs.gbuf.width, inputs.gbuf.height);
    let coordinates = all_tiles(width, height);

    // A hidden layer's mask is left out of the stack's preparation
    prepare_generators(inputs, Some(id));
    prepare_mask_curvature_sizes(inputs, std::iter::once(mask));

    let fields = evaluation_fields(inputs, &channels, &coordinates);
    if size <= width.min(height) && (width != size || height != size) {
        return Some(downsample_mask(inputs, id, mask, &channels, &fields, size));
    }

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

    if width == size && height == size {
        return Some(atlas);
    }

    let mut output = Vec::new();
    output.try_reserve_exact(pixel_count).ok()?;
    output.resize(pixel_count, 0);
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

/// Sample a smaller mask image directly into disjoint output row groups
fn downsample_mask(inputs: &EvaluationInputs, id: LayerId, mask: &Mask, channels: &[Channel], fields: &Fields, size: u32) -> Vec<u8> {
    let (width, height) = (inputs.gbuf.width, inputs.gbuf.height);
    let source_at = |pixel, span| ((u64::from(pixel) * u64::from(span)) / u64::from(size)) as u32;
    let first_output = |source, span| (u64::from(source) * u64::from(size)).div_ceil(u64::from(span)).min(u64::from(size)) as u32;
    let mut output = vec![0; size as usize * size as usize];
    let mut destination = output.as_mut_slice();
    let mut bands = Vec::with_capacity(height.div_ceil(TILE) as usize);

    // Partition rows by the tile that contains each row's source texels
    for tile_y in 0..height.div_ceil(TILE) {
        let first_row = first_output(tile_y * TILE, height);
        let past_row = first_output((tile_y + 1) * TILE, height);
        if past_row > first_row {
            let (rows, rest) = destination.split_at_mut((past_row - first_row) as usize * size as usize);
            bands.push((tile_y, first_row, past_row, rows));
            destination = rest;
        }
    }

    bands.into_par_iter().for_each(|(tile_y, first_row, past_row, rows)| {
        for tile_x in 0..width.div_ceil(TILE) {
            let first_column = first_output(tile_x * TILE, width);
            let past_column = first_output((tile_x + 1) * TILE, width);
            if first_column == past_column {
                continue;
            }

            let offsets = (first_row..past_row).flat_map(|row| {
                let y = source_at(row, height) % TILE;
                (first_column..past_column).map(move |column| (y * TILE + source_at(column, width) % TILE) as usize)
            });
            let texels = mask_tile(inputs, id, mask, channels, fields, (tile_x, tile_y), offsets);
            let pixels = rows.chunks_exact_mut(size as usize).flat_map(|row| &mut row[first_column as usize..past_column as usize]);
            for (pixel, (_, value)) in pixels.zip(texels) {
                *pixel = (value * 255.0 + 0.5) as u8;
            }
        }
    });

    output
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
    let projectors = ProjectorDepthCache::default();
    let inputs = &mut EvaluationInputs::new(inputs, &projectors);
    let mask = layer_mask(inputs.project, id)?;
    inputs.sampled_disabled_mask = (!mask.enabled).then_some(id);
    let mut channels = inputs.project.texture_set(inputs.ts).ok()?.channel_list();
    if !mask.enabled {
        channels = compared_channels(mask.effects.iter().filter(|effect| effect.visible && effect.kind.reads_stack()), channels);
    }

    let channels = evaluation_channels(inputs, channels);
    prepare_generators(inputs, Some(id));
    prepare_mask_curvature_sizes(inputs, std::iter::once(mask));

    let fields = evaluation_fields(inputs, &channels, tiles);
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
    let projectors = ProjectorDepthCache::default();
    let inputs = &mut EvaluationInputs::new(inputs, &projectors);
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

    let channels = evaluation_channels(inputs, inputs.project.texture_set(inputs.ts).map(|set| set.channel_list()).unwrap_or_default());

    // A hidden layer's mask is left out of the stack's preparation
    prepare_generators(inputs, Some(id));
    prepare_mask_curvature_sizes(inputs, std::iter::once(mask));

    // Filters and anchors can depend on texels outside the requested tiles
    let fields = evaluation_fields(inputs, &channels, &all_tiles(inputs.gbuf.width, inputs.gbuf.height));
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
    inputs: &'a EvaluationInputs<'a>,
    id: LayerId,
    mask: &'a Mask,
    channels: &[Channel],
    fields: &'a Fields,
    (tile_x, tile_y): (u32, u32),
    offsets: impl Iterator<Item = usize> + 'a,
) -> impl Iterator<Item = (usize, f32)> + 'a {
    let plan = mask_plan(inputs, id, mask, tile_x, tile_y, Some(fields), inputs.project.contents_around(id).into());
    let anchor_stack = anchor_plan(inputs, (id, 0), channels, tile_x, tile_y, Some(fields));
    offsets.map(move |offset| {
        let x = tile_x * TILE + (offset % TILE as usize) as u32;
        let y = tile_y * TILE + (offset / TILE as usize) as u32;
        if x >= inputs.gbuf.width || y >= inputs.gbuf.height || inputs.gbuf.tri.get((y * inputs.gbuf.width + x) as usize).is_none_or(|tri| *tri == NO_TRI) {
            return (offset, 0.0);
        }

        let mut texel = Texel::new(inputs, x, y, offset, 0.0);
        if !anchor_stack.is_empty() {
            texel.anchors_below(&anchor_stack, (id, 0));
        }
        (offset, texel.mask(&plan))
    })
}

/// Tiles [`effect_histogram`] samples at most (a million texels)
const HISTOGRAM_TILES: usize = 256;

/// The tiles [`effect_histogram`] reads: an even sample of at most [`HISTOGRAM_TILES`], which keeps the cost flat up to 8K; the stride is odd, as a power of two would divide the row length and sample the same few columns
fn histogram_tiles(width: u32, height: u32) -> Vec<(u32, u32)> {
    let all = all_tiles(width, height);
    let stride = all.len().div_ceil(HISTOGRAM_TILES).max(1) | 1;
    all.into_iter().step_by(stride).collect()
}

/// Histogram (`bins` buckets over 0..1) of the values entering effect `effect`: its layer's mask below it, or (a content effect) the layer's `channel` below it where the layer has content, signed values mapped to 0..1, a color's components pooled, on their own and as luminance. Over the texels the mesh covers (in an even sample of tiles on large sets); None for an unknown effect
pub fn effect_histogram(inp: &Inputs, effect: LayerId, channel: Channel, bins: usize) -> Option<EffectHistogram> {
    let (owner, _) = inp.project.effect(effect)?;
    let bins = bins.max(1);
    let (cut, in_mask) = cut_below(inp.project, owner, effect)?;
    let cut_inp = Inputs { project: &cut, preview: None, block_scales: inp.block_scales.clone(), ..*inp };
    let projectors = ProjectorDepthCache::default();
    let mut cut_inp = EvaluationInputs::new(&cut_inp, &projectors);
    prepare_generators(&mut cut_inp, in_mask.then_some(owner));
    let channels = evaluation_channels(&cut_inp, cut.texture_set(inp.ts).map(|set| set.channel_list()).unwrap_or_default());
    let tiles = histogram_tiles(inp.gbuf.width, inp.gbuf.height);
    let shared = shared_histogram_fields(&cut_inp, &channels, &tiles);
    let color = !in_mask && channel.kind() == ChannelKind::Color;

    let histogram = tiles
        .par_iter()
        .map(|&(tx, ty)| {
            let mut own = None;
            let fields = histogram_tile_fields(shared.as_ref(), &mut own, &cut_inp, &channels, (tx, ty));
            let mut histogram = EffectHistogram::new(bins, color);

            // The mask is planned on its own: a layer with nothing drawn plans no node
            let mask = if in_mask {
                layer_mask(&cut, owner)
                    .filter(|mask| mask.enabled)
                    .map(|mask| mask_plan(&cut_inp, owner, mask, tx, ty, Some(fields), cut.contents_around(owner).into()))
            } else {
                None
            };
            let nodes = if in_mask { Vec::new() } else { plan(&cut_inp, &[owner], &channels, tx, ty, 0, Some(fields)) };
            let content = nodes
                .first()
                .and_then(|node| node.chans.iter().find(|chan_plan| Channel::ALL.get(chan_plan.ch) == Some(&channel)).map(|chan_plan| (node, chan_plan)));
            if mask.is_none() && content.is_none() {
                return histogram;
            }

            let lit_layer = content.filter(|(node, _)| node.lights && channel == Channel::BaseColor).map(|(node, _)| node);
            let full = below_plan(&cut_inp, (owner, 0), &channels, (tx, ty), Some(fields), lit_layer.is_some());
            for (x, y, offset) in tile_texels(inp.gbuf, tx, ty) {
                let mut texel = Texel::new(&cut_inp, x, y, offset, 0.0);
                if !full.is_empty() {
                    texel.anchors_below(&full, (owner, 0));
                }

                if let Some(node) = lit_layer {
                    texel.light(node);
                }

                if let Some(mask) = &mask {
                    histogram.add_value(texel.mask(mask));
                } else if let Some((node, chan_plan)) = content
                    && let Some((value, _)) = texel.content(&node.content, chan_plan)
                {
                    // As levels reads them (`per_unit`); normal maps pass through levels unchanged
                    match chan_plan.kind {
                        ChannelKind::Color => histogram.add_color(value),
                        ChannelKind::Scalar => histogram.add_value(value[0]),
                        ChannelKind::Signed => histogram.add_value(value[0] * 0.5 + 0.5),
                        ChannelKind::NormalMap => {}
                    }
                }
            }
            histogram
        })
        .reduce(|| EffectHistogram::new(bins, color), |left, right| left.merged(&right));
    Some(histogram)
}

/// The neighbourhood fields every sampled tile of a histogram shares; None with only local neighbourhood filters below, where each tile builds its own
fn shared_histogram_fields(inp: &EvaluationInputs, channels: &[Channel], tiles: &[(u32, u32)]) -> Option<Fields> {
    let probe = evaluation_fields(inp, channels, tiles.get(..1).unwrap_or_default());
    let covers_set = |field: &Field| field.x0 == 0 && field.y0 == 0 && field.w == inp.gbuf.width && field.h == inp.gbuf.height;

    // When every field covers the set, their inputs already span the sample
    if probe.values().all(covers_set) {
        return Some(probe);
    }

    // Mixed fields need the sample's bounds to cover every dependency
    let whole_set = probe.values().any(covers_set);
    drop(probe);

    whole_set.then(|| evaluation_fields(inp, channels, tiles))
}

/// The fields histogram tile `tile` reads: the `shared` ones, else its own built into `own`
fn histogram_tile_fields<'f>(
    shared: Option<&'f Fields>,
    own: &'f mut Option<Fields>,
    inp: &EvaluationInputs,
    channels: &[Channel],
    tile: (u32, u32),
) -> &'f Fields {
    match shared {
        Some(fields) => fields,
        None => own.insert(evaluation_fields(inp, channels, &[tile])),
    }
}

/// The project with layer `owner` forced visible, its mask enabled, cut just below its effect `effect`, and whether the effect is in the mask
fn cut_below(project: &Project, owner: LayerId, effect: LayerId) -> Option<(Project, bool)> {
    let mut cut = project.clone();
    let layer = cut.layers.get_mut(&owner)?;
    layer.visible = true;
    let in_mask = match layer.mask.as_mut().and_then(|mask| mask.effects.iter().position(|entry| entry.id == effect).map(|index| (mask, index))) {
        Some((mask, effect_index)) => {
            mask.enabled = true;
            mask.effects.truncate(effect_index);
            true
        }
        None => {
            let effect_index = layer.content_effects.iter().position(|entry| entry.id == effect)?;
            layer.content_effects.truncate(effect_index);
            false
        }
    };

    Some((cut, in_mask))
}

/// Histograms (`bins` buckets over 0..1) of what compare mask `effect` reads on its left and right, over [`effect_histogram`]'s texels (None for another effect)
pub fn compare_histograms(inp: &Inputs, effect: LayerId, bins: usize) -> Option<[Vec<u32>; 2]> {
    let (owner, found) = inp.project.effect(effect)?;
    let EffectKind::Compare { left, right, channel, value: constant, .. } = found.kind else { return None };
    let bins = bins.max(1);
    let (cut, in_mask) = cut_below(inp.project, owner, effect)?;
    let set = cut.texture_set(inp.ts).ok()?;
    if !in_mask {
        return None;
    }

    let cut_inp = Inputs { project: &cut, preview: None, block_scales: inp.block_scales.clone(), ..*inp };
    let projectors = ProjectorDepthCache::default();
    let mut cut_inp = EvaluationInputs::new(&cut_inp, &projectors);
    prepare_generators(&mut cut_inp, in_mask.then_some(owner));
    let mut channels = evaluation_channels(&cut_inp, set.channel_list());
    if !channels.contains(&channel) {
        channels.push(channel);
    }
    let reads_stack = found.kind.reads_stack();
    let tiles = histogram_tiles(inp.gbuf.width, inp.gbuf.height);
    let shared = shared_histogram_fields(&cut_inp, &channels, &tiles);
    let empty = || [EffectHistogram::new(bins, false), EffectHistogram::new(bins, false)];

    let [left_counts, right_counts] = tiles
        .par_iter()
        .map(|&(tx, ty)| {
            let mut own = None;
            let fields = histogram_tile_fields(shared.as_ref(), &mut own, &cut_inp, &channels, (tx, ty));
            let mut sides = empty();

            // The mask under the compare, and (sides reading layers) the layer's content and the stack below it
            let Some(mut mask) = layer_mask(&cut, owner)
                .filter(|mask| mask.enabled)
                .map(|mask| mask_plan(&cut_inp, owner, mask, tx, ty, Some(fields), cut.contents_around(owner).into()))
            else {
                return sides;
            };

            // Every channel, so the content also serves the compares under this one
            if reads_stack {
                mask.layer = cut
                    .content_layer(owner)
                    .and_then(|layer| plan_content(&cut_inp, owner, layer, &channels, tx, ty, 0, Some(fields), false, cut.contents_around(owner).into(), None));
            }

            // The stack below, for this compare's sides and for the anchors and compares under it
            let mut full = anchor_plan(&cut_inp, (owner, 0), &channels, tx, ty, Some(fields));
            if reads_stack && full.is_empty() {
                full = plan_in(&cut_inp, &set.stack, &channels, tx, ty, 0, Some(fields), DrawPlacement::default(), Some((owner, 0)));
            }

            for (x, y, offset) in tile_texels(inp.gbuf, tx, ty) {
                let mut texel = Texel::new(&cut_inp, x, y, offset, 0.0);
                if !full.is_empty() {
                    texel.anchors_below(&full, (owner, 0));
                }
                let so_far = texel.mask(&mask);
                for (side, histogram) in [left, right].into_iter().zip(sides.iter_mut()) {
                    histogram.add_value(texel.compare_side(side, channel, &mask, so_far, constant));
                }
            }
            sides
        })
        .reduce(empty, |[left, right], [more_left, more_right]| [left.merged(&more_left), right.merged(&more_right)]);
    Some([left_counts.all, right_counts.all])
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
    let projectors = ProjectorDepthCache::default();
    let mut visible_inputs = EvaluationInputs::new(&visible_inputs, &projectors);
    prepare_generators(&mut visible_inputs, Some(id));
    let planned_channels = evaluation_channels(&visible_inputs, channels.clone());
    let fields = evaluation_fields(&visible_inputs, &planned_channels, tiles);

    let out: Vec<Vec<FlatTile>> = tiles
        .par_iter()
        .map(|&(tx, ty)| {
            let nodes = plan(&visible_inputs, &[id], &channels, tx, ty, 0, Some(&fields));
            let Some(n) = nodes.first() else { return Vec::new() };
            let full = below_plan(&visible_inputs, (id, 0), &planned_channels, (tx, ty), Some(&fields), n.lights);
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
                    tex.anchors_below(&full, (id, 0));
                }
                if n.lights {
                    tex.light(n);
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
        ChannelBlend, CompareOp, Effect, Layer, MicroDetails, OutputResolution, PbrValidate, Procedural, ProceduralKind, ResolutionMode, StylizedLighting,
        TextureSet,
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

    fn inputs<'a>(f: &'a Fixture, images: &'a ImageStore, preview: Option<&'a StrokePreview>) -> Inputs<'a> {
        Inputs {
            project: &f.project,
            ts: 0,
            mesh: &f.mesh,
            gbuf: &f.gbuf,
            pixels: &f.pixels,
            baked: &f.baked,
            block_scales: f.project.texture_sets[0].block_scales(),
            preview,
            images,
            bounds: f.mesh.bounds(),
        }
    }

    fn run_with_images(f: &Fixture, preview: Option<&StrokePreview>, images: &ImageStore) -> Composite {
        let mut out = Composite::new(&f.project.texture_sets[0].channel_list(), 64);
        composite_tiles(&inputs(f, images, preview), &mut out, &all_tiles(64, 64));
        out
    }

    fn run(f: &Fixture, preview: Option<&StrokePreview>) -> Composite {
        run_with_images(f, preview, &ImageStore::default())
    }

    #[test]
    fn a_mirrored_fill_culls_against_the_mesh_in_each_copy_view() {
        let mut fixture = fixture();
        let mut rear = primitives::plane(2.0, 1);
        for position in &mut rear.positions {
            *position = Vec3::new(position.x, position.z, 0.0);
        }

        let mut front = rear.clone();
        for position in &mut front.positions {
            position.x = (position.x - 1.0) * 0.5;
            position.z = 1.0;
        }

        // Keep the occluder out of the UV raster so both samples read the rear surface
        front.uvs.fill(Vec2::ZERO);
        rear.append(&front);
        fixture.gbuf = UvGBuffer::new(&rear, 0, 64, 64);
        fixture.mesh = rear;

        let id = fill_layer(&mut fixture.project, [1.0, 0.0, 0.0], 0.5);
        let LayerKind::Fill(fill) = &mut fixture.project.layer_mut(id).unwrap().kind else { panic!("fill fixture") };
        fill.projection = Projection::Planar {
            scale: 1.0,
            axis: 2,
            offset: [0.0; 3],
            uv: Default::default(),
            projector: texturecraft_core::Projector { depth: texturecraft_core::DepthCulling { on: true, hardness: 1.0 }, ..Default::default() },
            placed: None,
        };
        fill.sampling.symmetry.on = true;

        let composite = run(&fixture, None);

        assert!(close(composite.value(Channel::BaseColor, 16, 32), [0.5; 3]), "the occluded side stays at the channel default");
        assert!(close(composite.value(Channel::BaseColor, 48, 32), [1.0, 0.0, 0.0]), "the exposed side takes the fill");
    }

    #[test]
    fn baked_height_slopes_stay_within_each_uv_tile() {
        let mut fixture = fixture();
        let set = &mut fixture.project.texture_sets[0];
        set.uv_tiles = vec![1001, 1002];
        set.tile_resolutions.insert(1002, 32);
        fixture.gbuf = UvGBuffer::new(&fixture.mesh, 0, 128, 64);
        fixture.baked.resolution = 128;
        let heights: Vec<_> = (0..128 * 64).map(|index| ((index % 64) + index / 128) as f32 / 64.0).collect();
        fixture.baked.maps.insert(BakedMap::Height, Arc::new(heights));
        let images = ImageStore::default();
        let inputs = inputs(&fixture, &images, None);

        for (x, y) in [(24, 10), (63, 10), (64, 0), (95, 31)] {
            let slope = Texel::new(&inputs, x, y, 0, 0.0).height_slope();

            assert_eq!(slope, Some([1.0; 2]), "slope within the tile at ({x}, {y})");
        }
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

    thread_local! {
        /// How many times a texel looked up its place on the surface on this thread
        pub(super) static COORDS_LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        /// How many times a texel worked out how much of it a fill covers on this thread
        pub(super) static COVER_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// What `counter` counts while `fixture` composites, on one thread so the count sees every texel
    fn count_while_compositing(counter: &'static std::thread::LocalKey<std::cell::Cell<usize>>, fixture: &Fixture) -> usize {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        pool.install(|| {
            counter.with(|count| count.set(0));
            run(fixture, None);
            counter.with(std::cell::Cell::get)
        })
    }

    /// How many texels looked up their place on the surface while `fixture` composites
    fn coords_lookups(fixture: &Fixture) -> usize {
        count_while_compositing(&COORDS_LOOKUPS, fixture)
    }

    #[test]
    fn a_repeating_uniform_fill_covers_every_texel_without_looking_up_where_it_is() {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [1.0, 0.0, 0.0], 0.2);

        assert_eq!(coords_lookups(&f), 0, "a uniform fill that repeats needs no texel's place, as before fills could crop");

        // Not repeating, it crops to its tile, so it has to know where each texel lands
        let Ok(LayerKind::Fill(fill)) = f.project.layer_mut(id).map(|layer| &mut layer.kind) else { panic!("fill layer") };
        fill.sampling.wrap = texturecraft_core::UvWrap::None;

        assert!(coords_lookups(&f) > 0);
    }

    /// How many times a texel worked out a fill's coverage while `fixture` composites
    fn cover_calls(fixture: &Fixture) -> usize {
        count_while_compositing(&COVER_CALLS, fixture)
    }

    #[test]
    fn a_fill_covering_every_texel_is_decided_once_per_tile_not_per_texel() {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [1.0, 0.0, 0.0], 0.2);

        assert_eq!(cover_calls(&f), 0, "a repeating fill with nothing cropped or culled covers every texel, so no texel asks");
        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [1.0, 0.0, 0.0]));

        // Not repeating, it crops to its tile, so each texel works its coverage out
        let Ok(LayerKind::Fill(fill)) = f.project.layer_mut(id).map(|layer| &mut layer.kind) else { panic!("fill layer") };
        fill.sampling.wrap = texturecraft_core::UvWrap::None;

        assert!(cover_calls(&f) > 0);
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
                        mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Filter(Filter::Blur { radius, wrap: false })));
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
                    bounds: fixture.mesh.bounds(),
                };
                let mut output = Composite::new(&fixture.project.texture_sets[0].channel_list(), resolution);
                let mut cache = BelowCache::default();
                cache.ahead = all_tiles(resolution, resolution).into_iter().skip(1).collect();

                composite_tiles_cached(&input, &mut output, &[(0, 0)], Some(&mut cache));
                let retained = cache.held.as_ref().expect("filter fields should be retained while tiles follow");
                let fields = || retained.fields.values().flat_map(|field| std::iter::once(field).chain(field.anchor_stages.iter().map(|(_, stage)| stage)));
                let field_bytes: usize = fields().map(|field| field.data.capacity() * std::mem::size_of::<f32>()).sum();
                let field_count = fields().count();
                let stage_index_bytes: usize =
                    retained.fields.values().map(|field| field.anchor_stages.capacity() * std::mem::size_of::<(usize, Field)>()).sum();
                let weak_fields = Arc::downgrade(&retained.fields);
                let expected_fields = if record_anchors { 6 } else { 2 };

                assert_eq!(field_count, expected_fields, "only filter stages needed by anchors remain beside the final mask fields");
                assert_eq!(field_bytes, expected_fields * resolution as usize * resolution as usize * std::mem::size_of::<f32>());
                println!(
                    "resolution: {resolution}, mask layers: 2, anchors: {record_anchors}, fields: {field_count}, buffer capacity: {field_bytes} bytes, stage-index capacity: {stage_index_bytes} bytes"
                );

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
        let preview = StrokePreview {
            target: PaintTarget::Layer(id),
            alpha,
            opacity: 1.0,
            erase: false,
            values: [(Channel::BaseColor, [0.0, 0.0, 1.0])].into_iter().collect(),
            mask_gray: MaskGray::Value(1.0),
            stencil: None,
            carried: BTreeMap::new(),
            carried_mask: None,
        };
        let out = run(&f, Some(&preview));
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
            EffectKind::Fill {
                source: Source::Procedural { pattern, low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() },
                projection: Projection::default(),
                sampling: Default::default(),
            },
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
            EffectKind::Fill {
                source: Source::Procedural { pattern, low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() },
                projection: Projection::default(),
                sampling: Default::default(),
            },
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
                sampling: Default::default(),
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
            EffectKind::Fill {
                source: Source::Anchor { anchor, channel: Channel::BaseColor, ramp: Vec::new() },
                projection: Projection::default(),
                sampling: Default::default(),
            },
        ));
        fixture.project.layer_mut(upper).unwrap().mask = Some(mask.clone());
        let texels: Vec<_> = (0..128).map(|x| (x, 63)).collect();
        let original = assert_sparse_mask_matches_image(&fixture, upper, &texels).unwrap();

        assert!(original.iter().any(|value| *value > 0.99) && original.iter().any(|value| *value < 0.01), "the anchor supplies the checker");

        for filter in [Filter::Blur { radius: 64.0, wrap: false }, Filter::DirectionalWarp { intensity: 256.0, angle: 90.0, scale: 4.0, seed: 3, divider: 1 }] {
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
        let warp = Filter::DirectionalWarp { intensity: 256.0, angle: 90.0, scale: 4.0, seed: 3, divider: 1 };
        let texels: Vec<_> = (0..128).map(|x| (x, 63)).collect();
        let mut masked = |filters: &[Filter]| {
            let mut mask = Mask::new(MaskBackground::Black);
            mask.effects.push(Effect::new(
                fixture.project.alloc_id(),
                EffectKind::Fill { source: checker.clone(), projection: Projection::default(), sampling: Default::default() },
            ));
            for filter in filters {
                mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Filter(filter.clone())));
            }
            fixture.project.layer_mut(layer).unwrap().mask = Some(mask);
            assert_sparse_mask_matches_image(&fixture, layer, &texels).unwrap()
        };

        let warped = masked(std::slice::from_ref(&warp));
        let blurred_then_warped = masked(&[Filter::Blur { radius: 64.0, wrap: false }, warp]);
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
        let warp = Filter::DirectionalWarp { intensity: 256.0, angle: 90.0, scale: 4.0, seed: 3, divider: 1 };
        let warp_effect = content_fx(&mut fixture, layer, EffectKind::Filter(warp));
        let row = |fixture: &Fixture| -> Vec<f32> {
            let composite = run(fixture, None);
            (0..64).map(|x| composite.value(Channel::BaseColor, x, 20)[0]).collect()
        };
        let warped = row(&fixture);

        content_fx(&mut fixture, layer, EffectKind::Filter(Filter::Blur { radius: 64.0, wrap: false }));
        let effects = &mut fixture.project.layer_mut(layer).unwrap().content_effects;
        let warp_index = effects.iter().position(|effect| effect.id == warp_effect).unwrap();
        let warp_on_top = effects.remove(warp_index);
        effects.push(warp_on_top);
        let blurred_then_warped = row(&fixture);
        assert!(blurred_then_warped.iter().zip(&warped).any(|(chained, single)| (chained - single).abs() > 0.1), "the blur under the warp still runs");
    }

    #[test]
    fn disabled_content_filters_leave_their_channel_unchanged() {
        let mut fixture = fixture();
        let layer = fill_layer(&mut fixture.project, [0.0; 3], 0.2);
        let LayerKind::Fill(fill) = &mut fixture.project.layer_mut(layer).unwrap().kind else { panic!("expected a fill layer") };
        let checker = Source::Procedural { pattern: Procedural::new(ProceduralKind::Checker), low: [0.1; 3], high: [0.8; 3], ramp: Vec::new() };
        fill.sources.insert(Channel::BaseColor, checker.clone());
        fill.sources.insert(Channel::Roughness, checker);
        let unfiltered = run(&fixture, None);

        for filter in [Filter::Blur { radius: 256.0, wrap: false }, Filter::Invert] {
            let effect_id = content_fx(&mut fixture, layer, EffectKind::Filter(filter));
            let filtered = run(&fixture, None);
            fixture.project.effect_mut(effect_id).unwrap().channel_blends.insert(Channel::Roughness, ChannelBlend { mode: BlendMode::Disable, opacity: 1.0 });
            let disabled = run(&fixture, None);

            assert!(filtered.channels[&Channel::Roughness] != unfiltered.channels[&Channel::Roughness], "the filter changes Roughness");
            assert!(disabled.channels[&Channel::Roughness] == unfiltered.channels[&Channel::Roughness], "Disable bypasses the filter");
            assert!(disabled.channels[&Channel::BaseColor] == filtered.channels[&Channel::BaseColor], "the other channel stays filtered");

            fixture.project.layer_mut(layer).unwrap().content_effects.clear();
        }
    }

    #[test]
    fn compare_mask_on_empty_paint_reads_its_own_position_in_the_stack() {
        let mut fixture = fixture();
        fill_layer(&mut fixture.project, [0.0; 3], 0.8);
        let paint_id = fixture.project.alloc_id();
        let compare_id = fixture.project.alloc_id();
        let mut paint = Layer::new(paint_id, "Empty paint", LayerKind::Paint, &Channel::DEFAULT_SET);
        let mut mask = Mask::new(MaskBackground::White);
        mask.effects.push(Effect::new(
            compare_id,
            EffectKind::Compare {
                op: CompareOp::GreaterThan,
                value: 0.7,
                tolerance: 0.0,
                left: CompareSource::LayersBelow,
                right: CompareSource::Constant,
                channel: Channel::Roughness,
                hardness: 1.0,
            },
        ));
        paint.mask = Some(mask);
        fixture.project.insert_layer(0, paint, None, false).unwrap();
        fill_layer(&mut fixture.project, [1.0; 3], 0.2);

        let images = ImageStore::default();
        let inputs = inputs(&fixture, &images, None);
        let sampled_mask = mask_at(&inputs, paint_id, &[(32, 32)]).unwrap();
        let [below, _] = compare_histograms(&inputs, compare_id, 4).unwrap();

        assert!(sampled_mask[0] > 0.99 && below[3] > 0 && below[..3] == [0; 3], "mask {sampled_mask:?}, histogram {below:?}");
    }

    /// A 256² set whose curvature map has a one-texel ridge in column `ridge_x` (flat elsewhere), and a white fill layer to mask.
    fn ridge_fixture(ridge_x: usize) -> (Fixture, LayerId) {
        let mut fixture = fixture();
        fixture.gbuf = UvGBuffer::new(&fixture.mesh, 0, 256, 256);
        fixture.project.texture_sets[0].resolution = 256;
        let ridge: Vec<f32> = (0..256 * 256).map(|texel| if texel % 256 == ridge_x { 1.0 } else { 0.5 }).collect();
        fixture.baked = BakedMaps { resolution: 256, maps: [(BakedMap::Curvature, Arc::new(ridge))].into() };
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
        Generator { level: 0.5, softness: 0.01, noise_amount: 0.0, detail: None, wear: None, ..Generator::new(GeneratorKind::EdgeWear) }
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
            let below =
                Effect::new(fixture.project.alloc_id(), EffectKind::Fill { source: checker, projection: Projection::default(), sampling: Default::default() });
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
        fixture.baked = BakedMaps { resolution: 64, maps: [(BakedMap::AmbientOcclusion, Arc::new(vec![1.0; 64 * 64]))].into() };
        let layer = fill_layer(&mut fixture.project, [1.0; 3], 1.0);
        let value_with = |fixture: &mut Fixture, input: Option<f32>| -> f32 {
            let mut generator = Generator { level: 0.5, softness: 0.1, noise_amount: 0.0, wear: None, ..Generator::new(GeneratorKind::Dirt) };
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
    fn a_generator_input_image_reads_plain_bilinear_as_saved_projects_did() {
        // Every fourth column white: a texel centre reads between two black columns, a mip level a quarter gray
        const SIZE: u32 = 256;
        let striped = (0..SIZE * SIZE).map(|i| if (i % SIZE).is_multiple_of(4) { [1.0; 4] } else { [0.0, 0.0, 0.0, 1.0] }).collect();
        let images: ImageStore = [("ao".to_string(), Arc::new(crate::store::ImageRes::new(SIZE, SIZE, striped, false)))].into_iter().collect();
        let mut generator = Generator::new(GeneratorKind::Dirt);
        generator.inputs.insert(BakedMap::AmbientOcclusion, Source::image("ao"));
        let fixture = fixture();
        let inp = inputs(&fixture, &images, None);

        let read = Texel::new(&inp, 20, 20, 0, 0.0).generator_input(&generator, BakedMap::AmbientOcclusion);

        assert_eq!(read, Some(0.0));
    }

    #[test]
    fn curvature_detail_noise_keeps_flat_areas_neutral() {
        let mut fixture = fixture();
        fixture.baked = BakedMaps { resolution: 64, maps: [(BakedMap::Curvature, Arc::new(vec![0.5; 64 * 64]))].into() };
        let layer = fill_layer(&mut fixture.project, [1.0; 3], 1.0);
        let texels: Vec<_> = (0..64).map(|x| (x, 32)).collect();

        for kind in [GeneratorKind::Curvature, GeneratorKind::EdgeWear] {
            for mode in CurvatureMode::ALL {
                let detail = CurvatureDetail { mode, weights: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] };
                let generator =
                    Generator { level: 0.5, softness: 0.5, noise_amount: 2.0, detail: Some(detail), wear: None, section: None, ..Generator::new(kind) };
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
        mask.effects.push(Effect::new(eid, EffectKind::Filter(Filter::Blur { radius: 8.0, wrap: false })));
        f.project.layer_mut(id).unwrap().mask = Some(mask);

        assert!(stack_reads_neighbours(&f.project, 0));

        f.project.layer_mut(id).unwrap().mask = None;

        assert!(!stack_reads_neighbours(&f.project, 0));

        content_fx(&mut f, id, EffectKind::Filter(Filter::Mosaic { size: 8.0, alpha: true }));

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
            EffectKind::Fill {
                source: Source::Procedural { pattern, low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() },
                projection: Projection::default(),
                sampling: Default::default(),
            },
        ));
        f.project.layer_mut(id).unwrap().mask = Some(mask.clone());
        let hard: Vec<f32> = (0..64).map(|x| run(&f, None).value(Channel::BaseColor, x, 20)[0]).collect();
        mask.effects.push(Effect::new(e2, EffectKind::Filter(Filter::Blur { radius: 64.0, wrap: false })));
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

    #[test]
    fn groups_pass_through_with_opacity() {
        let mut f = fixture();
        let gid = f.project.alloc_id();
        let g = Layer::new(
            gid,
            "G",
            LayerKind::Group { children: vec![], collapsed: false, contents: Default::default(), randomized: Vec::new() },
            &[Channel::BaseColor],
        );
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
    fn content_filters_hsl_and_opacity_change_layer_values() {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [1.0, 0.0, 0.0], 0.2);
        content_fx(&mut f, id, EffectKind::Filter(Filter::Hsl { hue: 1.0 / 3.0, saturation: 0.0, lightness: 0.0, perceptive: false }));
        let out = run(&f, None);

        assert!(close(out.value(Channel::BaseColor, 5, 5), [0.0, 1.0, 0.0]), "{:?}", out.value(Channel::BaseColor, 5, 5));
        assert!(close(out.value(Channel::Roughness, 5, 5), [0.2; 3]), "a hue shift leaves scalars alone");

        content_fx(&mut f, id, EffectKind::Filter(Filter::Invert));
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 5, 5), [1.0, 0.0, 1.0]));
        assert!(close(out.value(Channel::Roughness, 5, 5), [0.8; 3]));

        // Hidden effects are skipped and opacity fades an effect
        let fx = &mut f.project.layer_mut(id).unwrap().content_effects;
        fx[0].visible = false;
        fx[1].opacity = 0.5;
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 5, 5), [0.5; 3]));
        assert!(close(out.value(Channel::Roughness, 5, 5), [0.5; 3]));
    }

    /// Posterize steps a paint layer's coverage only with Apply to Alpha
    #[test]
    fn posterize_steps_the_coverage_when_it_applies_to_alpha() {
        let mut f = fixture();
        let id = f.project.alloc_id();
        let l = Layer::new(id, "Paint", LayerKind::Paint, &[Channel::BaseColor]);
        f.project.insert_layer(0, l, None, false).unwrap();

        // Half-covered red over the default mid gray
        f.pixels.layer_mut(id, Channel::BaseColor, (64, 64)).set(3, 3, &[to_u16(0.5), 0, 0, to_u16(0.5)]);
        let posterize = content_fx(&mut f, id, EffectKind::Filter(Filter::Posterize { levels: 2.0, alpha: false }));
        assert!(close(run(&f, None).value(Channel::BaseColor, 3, 3), [0.75, 0.25, 0.25]), "the coverage stays as it is");

        f.project.effect_mut(posterize).unwrap().kind = EffectKind::Filter(Filter::Posterize { levels: 2.0, alpha: true });

        assert!(close(run(&f, None).value(Channel::BaseColor, 3, 3), [1.0, 0.0, 0.0]), "half coverage rounds up to whole");
    }

    /// A paint layer with base color texels set by `paint` (x, y -> Some(linear color, coverage))
    fn painted(f: &mut Fixture, paint: impl Fn(u32, u32) -> Option<([f32; 3], f32)>) -> LayerId {
        let id = f.project.alloc_id();
        let l = Layer::new(id, "Paint", LayerKind::Paint, &[Channel::BaseColor]);
        f.project.insert_layer(0, l, None, false).unwrap();
        let tile = f.pixels.layer_mut(id, Channel::BaseColor, (64, 64));
        for (x, y) in (0..64).flat_map(|y| (0..64).map(move |x| (x, y))) {
            if let Some((color, cover)) = paint(x, y) {
                tile.set(x, y, &[to_u16(color[0] * cover), to_u16(color[1] * cover), to_u16(color[2] * cover), to_u16(cover)]);
            }
        }
        id
    }

    /// Pixelate steps the coverage into its cells only with Apply To Alpha; off, each texel keeps its own
    #[test]
    fn pixelate_keeps_each_texels_coverage_without_apply_to_alpha() {
        let mut f = fixture();

        // One red texel in a cell of 4 × 4 texels (128 texels at 2K at 64 across), whose center reads it a quarter
        let id = painted(&mut f, |x, y| ((x, y) == (1, 1)).then_some(([1.0, 0.0, 0.0], 1.0)));
        let fx = content_fx(&mut f, id, EffectKind::Filter(Filter::Mosaic { size: 128.0, alpha: true }));
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 1, 1), [0.625, 0.375, 0.375]), "the cell's coverage: {:?}", out.value(Channel::BaseColor, 1, 1));
        assert!(close(out.value(Channel::BaseColor, 2, 2), [0.625, 0.375, 0.375]), "spread over the cell");

        f.project.effect_mut(fx).unwrap().kind = EffectKind::Filter(Filter::Mosaic { size: 128.0, alpha: false });
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 1, 1), [1.0, 0.0, 0.0]), "the texel keeps its whole coverage");
        assert!(close(out.value(Channel::BaseColor, 2, 2), [0.5; 3]), "an uncovered texel stays uncovered");
    }

    /// FXAA blends a stair-stepped edge and leaves flat areas; the coverage only with Affect Alpha Channel
    #[test]
    fn fxaa_smooths_stair_steps() {
        let stairs = |x: u32, y: u32| x >= 20 + y / 4;
        let mids = |out: &Composite| (0..64 * 64).filter(|i| (0.05..0.95).contains(&out.value(Channel::BaseColor, i % 64, i / 64)[0])).count();

        // Black and white, both covered: the edge gets in-between values
        let mut f = fixture();
        let id = painted(&mut f, |x, y| Some((if stairs(x, y) { [1.0; 3] } else { [0.0; 3] }, 1.0)));
        assert_eq!(mids(&run(&f, None)), 0);

        content_fx(&mut f, id, EffectKind::Filter(Filter::Fxaa { alpha: false }));
        let out = run(&f, None);
        assert!(mids(&out) > 16, "{} texels blended", mids(&out));
        assert!(close(out.value(Channel::BaseColor, 2, 30), [0.0; 3]) && close(out.value(Channel::BaseColor, 60, 30), [1.0; 3]), "flat areas stay");

        // White shapes on nothing: the coverage changes only with Affect Alpha Channel
        let mut f = fixture();
        let id = painted(&mut f, |x, y| stairs(x, y).then_some(([1.0; 3], 1.0)));
        let fx = content_fx(&mut f, id, EffectKind::Filter(Filter::Fxaa { alpha: false }));
        let plain = |out: &Composite| (0..64 * 64).all(|i| [0.5, 1.0].iter().any(|v| (out.value(Channel::BaseColor, i % 64, i / 64)[0] - v).abs() < 2e-4));

        assert!(plain(&run(&f, None)), "each texel keeps its coverage");

        f.project.effect_mut(fx).unwrap().kind = EffectKind::Filter(Filter::Fxaa { alpha: true });

        assert!(!plain(&run(&f, None)), "the coverage is smoothed");
    }

    #[test]
    fn hsl_on_a_mask_changes_its_lightness() {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [1.0; 3], 1.0);
        let mut mask = Mask::new(MaskBackground::White);
        mask.effects.push(Effect::new(f.project.alloc_id(), EffectKind::Filter(Filter::Hsl { hue: 0.3, saturation: 1.0, lightness: -0.5, perceptive: false })));
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
        content_fx(&mut f, id, EffectKind::Filter(Filter::Blur { radius: 64.0, wrap: false }));
        let soft = row(&f);
        let mids = |v: &[f32]| v.iter().filter(|x| **x > 0.05 && **x < 0.95).count();
        assert!(mids(&soft) > mids(&hard) + 4, "hard {hard:?}\nsoft {soft:?}");
        assert!(soft.iter().all(|v| (0.0..=1.0).contains(v)));
    }

    /// A fill layer of a black and white checker on Roughness, under `filter` when given
    fn checker_roughness(fixture: &mut Fixture, filter: Option<Filter>) {
        let id = fixture.project.alloc_id();
        let mut fill = Fill::default();
        let checker = Source::Procedural { pattern: Procedural::new(ProceduralKind::Checker), low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() };
        fill.sources.insert(Channel::Roughness, checker);
        fixture.project.insert_layer(0, Layer::new(id, "F", LayerKind::Fill(fill), &[Channel::Roughness]), None, false).unwrap();
        if let Some(filter) = filter {
            content_fx(fixture, id, EffectKind::Filter(filter));
        }
    }

    #[test]
    fn a_wrapping_blur_reads_the_far_side_of_the_texture() {
        let row = |wrap: Option<bool>| {
            let mut f = fixture();
            checker_roughness(&mut f, wrap.map(|wrap| Filter::Blur { radius: 96.0, wrap }));
            let out = run(&f, None);
            (0..64).map(|x| out.value(Channel::Roughness, x, 20)[0]).collect::<Vec<f32>>()
        };

        // The checker tiles: its period P repeats across the whole row
        let hard = row(None);
        let period = (1..64).find(|&p| (0..64 - p).all(|x| (hard[x] - hard[x + p]).abs() < 1e-3)).unwrap();

        assert!(period <= 32, "{hard:?}");

        // Wrapped, the blur at the texture's edge sees the far side, so the result keeps tiling; clipped, the edge differs
        let wrapped = row(Some(true));
        assert!((0..64 - period).all(|x| (wrapped[x] - wrapped[x + period]).abs() < 0.02), "{wrapped:?}");

        let clipped = row(Some(false));
        assert!((clipped[0] - clipped[period]).abs() > 0.05, "{clipped:?}");
        assert!((12..52).all(|x| (clipped[x] - wrapped[x]).abs() < 0.02), "away from the edges, wrap changes nothing");
    }

    #[test]
    fn a_wrapped_blur_spanning_more_than_one_period_keeps_the_full_kernel() {
        let mut fixture = fixture();
        checker_roughness(&mut fixture, None);
        let id = fixture.project.texture_sets[0].stack[0];
        let LayerKind::Fill(fill) = &mut fixture.project.layer_mut(id).unwrap().kind else { panic!("expected fill") };
        let Some(Source::Procedural { pattern, .. }) = fill.sources.get_mut(&Channel::Roughness) else { panic!("expected checker") };
        pattern.scale = 2.0;
        let original = run(&fixture, None);
        let region: Vec<_> = (0..64 * 64).map(|index| original.value(Channel::Roughness, index % 64, index / 64)[0]).collect();
        let radius = 40;
        let reach = radius * BLUR_BOX_PASSES;
        let mut expected = wrap_pad(&region, 1, (64, 64), (reach, reach));
        blur(&mut expected, 64 + 2 * reach, 64 + 2 * reach, radius);
        let expected = unpad(&expected, 1, (64, 64), (reach, reach));

        content_fx(&mut fixture, id, EffectKind::Filter(Filter::Blur { radius: radius as f32 * 2048.0 / 64.0, wrap: true }));
        let output = run(&fixture, None);

        for (index, reference) in expected.iter().enumerate() {
            let sample = output.value(Channel::Roughness, index as u32 % 64, index as u32 / 64)[0];
            assert!((sample - reference).abs() <= 2.0 / 65535.0, "at {index}: {sample} vs {reference}");
        }
    }

    /// A set several tiles wide: a tile at its edge composited alone reads the far side as the whole set does, so the result keeps tiling across the edge
    #[test]
    fn a_wrapping_filter_reads_the_far_side_of_a_set_several_tiles_wide() {
        const WIDTH: u32 = 256;
        const HEIGHT: u32 = 64;
        let row_of = |out: &Composite| (0..WIDTH).map(|x| out.value(Channel::Roughness, x, 20)[0]).collect::<Vec<f32>>();
        let composite = |filter: Option<&Filter>, tiles: &[(u32, u32)]| {
            let mut f = fixture();
            f.gbuf = UvGBuffer::new(&f.mesh, 0, WIDTH, HEIGHT);
            f.project.texture_sets[0].resolution = WIDTH;
            checker_roughness(&mut f, filter.cloned());
            let images = ImageStore::default();
            let mut out = Composite::with_size(&f.project.texture_sets[0].channel_list(), WIDTH, HEIGHT);
            composite_tiles(&inputs(&f, &images, None), &mut out, tiles);
            row_of(&out)
        };
        let hard = composite(None, &all_tiles(WIDTH, HEIGHT));
        let period = (1..WIDTH as usize).find(|&p| (0..WIDTH as usize - p).all(|x| (hard[x] - hard[x + p]).abs() < 1e-3)).unwrap();

        assert!(period <= WIDTH as usize / 2, "the checker tiles across the set: {hard:?}");

        let wrapping = [
            Filter::Blur { radius: 96.0, wrap: true },
            Filter::DistanceBevel(Bevel { wrap: true, ..Bevel::LIBRARY }),
            Filter::BevelSmooth(BevelSmooth { wrap: true, ..BevelSmooth::LIBRARY }),
        ];
        for filter in wrapping {
            let whole = composite(Some(&filter), &all_tiles(WIDTH, HEIGHT));
            let edge_tile = composite(Some(&filter), &[(0, 0)]);

            let label = filter.label();
            assert!((0..TILE as usize).all(|x| (edge_tile[x] - whole[x]).abs() < 1e-3), "{label}: the edge tile alone\n{edge_tile:?}\nwhole\n{whole:?}");
            assert!((0..WIDTH as usize - period).all(|x| (whole[x] - whole[x + period]).abs() < 0.02), "{label} keeps tiling: {whole:?}");
        }
    }

    #[test]
    fn a_gradients_blur_intensity_blurs_what_it_looks_up() {
        let row = |filter: Option<Filter>| {
            let mut f = fixture();
            checker_roughness(&mut f, filter);
            let out = run(&f, None);
            (16..48).map(|x| out.value(Channel::Roughness, x, 20)[0]).collect::<Vec<f32>>()
        };
        let mut gradient = Filter::even_gradient(vec![[0.0; 3], [1.0; 3]]);
        let hard = row(Some(gradient.clone()));

        assert!(hard.iter().zip(row(None)).all(|(a, b)| (a - b).abs() < 1e-3), "a black to white ramp keeps a scalar as it is");

        // Blurred first, then looked up: the same as a blur of that size
        if let Filter::GradientMap { input, .. } = &mut gradient {
            input.blur = 8.0;
        }
        let blurred = row(Some(gradient));
        let reference = row(Some(Filter::Blur { radius: 8.0, wrap: false }));
        assert!(blurred.iter().zip(&reference).all(|(a, b)| (a - b).abs() < 0.02), "{blurred:?} vs {reference:?}");
        assert!(blurred.iter().zip(&hard).any(|(a, b)| (a - b).abs() > 0.1), "{blurred:?}");
    }

    #[test]
    fn highpass_works_in_its_color_space_and_applies_to_alpha() {
        // Uniform content has no detail: mid gray in the working space, sRGB mid gray (linear 0.214) in sRGB
        let mut f = fixture();
        let below = fill_layer(&mut f.project, [0.0; 3], 0.0);
        let id = fill_layer(&mut f.project, [1.0, 0.0, 0.0], 0.2);
        assert_ne!(below, id);

        let hp = content_fx(&mut f, id, EffectKind::Filter(Filter::HighPass { radius: 64.0, space: HighpassSpace::SrgbFromLinear, alpha: false }));
        let out = run(&f, None);
        assert!(close(out.value(Channel::BaseColor, 30, 30), [texturecraft_texel::srgb_to_linear(0.5); 3]), "{:?}", out.value(Channel::BaseColor, 30, 30));
        assert!(close(out.value(Channel::Roughness, 30, 30), [0.5; 3]), "scalar channels stay in the working space");

        // Apply To Alpha: full coverage minus its blur, around 0.5, lays the layer at half over the black one below
        f.project.effect_mut(hp).unwrap().kind = EffectKind::Filter(Filter::HighPass { radius: 64.0, space: HighpassSpace::Working, alpha: true });
        let out = run(&f, None);
        assert!(close(out.value(Channel::Roughness, 30, 30), [0.25; 3]), "{:?}", out.value(Channel::Roughness, 30, 30));
    }

    #[test]
    fn highpass_in_srgb_applied_to_alpha_decodes_on_the_new_coverage() {
        let mut f = fixture();
        fill_layer(&mut f.project, [0.0; 3], 0.0);
        let id = fill_layer(&mut f.project, [1.0, 0.0, 0.0], 0.2);
        content_fx(&mut f, id, EffectKind::Filter(Filter::HighPass { radius: 64.0, space: HighpassSpace::SrgbFromLinear, alpha: true }));

        let out = run(&f, None);

        // sRGB mid gray (linear 0.214) at half coverage over the black layer below
        let expected = texturecraft_texel::srgb_to_linear(0.5) * 0.5;
        assert!(close(out.value(Channel::BaseColor, 30, 30), [expected; 3]), "{:?}", out.value(Channel::BaseColor, 30, 30));
    }

    #[test]
    fn high_pass_edge_detect_directional_warp_grayscale_and_normal_invert() {
        // Uniform content gives mid gray high pass and black edge detection
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [1.0, 0.0, 0.0], 0.2);
        let high_pass = content_fx(&mut f, id, EffectKind::Filter(Filter::HighPass { radius: 64.0, space: HighpassSpace::Working, alpha: false }));
        let out = run(&f, None);

        assert!(close(out.value(Channel::BaseColor, 30, 30), [0.5; 3]), "{:?}", out.value(Channel::BaseColor, 30, 30));
        assert!(close(out.value(Channel::Roughness, 30, 30), [0.5; 3]));

        f.project.layer_mut(id).unwrap().content_effects.retain(|e| e.id != high_pass);
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
        content_fx(&mut f, id, EffectKind::Filter(Filter::DirectionalWarp { intensity: 256.0, angle: 90.0, scale: 4.0, seed: 3, divider: 1 }));
        let warped = run(&f, None);
        let differs = (0..64).any(|y| (0..64).any(|x| (warped.value(Channel::Roughness, x, y)[0] - plain.value(Channel::Roughness, x, y)[0]).abs() > 0.2));

        assert!(differs, "the warp moves the checker");

        // Point filters
        assert!(close(Filter::Grayscale.point(ChannelKind::Color, [1.0, 0.0, 0.0]).unwrap(), [lum([1.0, 0.0, 0.0]); 3]));
        assert_eq!(Filter::Grayscale.point(ChannelKind::Scalar, [0.3; 3]), Some([0.3; 3]));
        assert_eq!(Filter::NormalInvert.point(ChannelKind::NormalMap, [0.5, 0.25, 1.0]), Some([0.5, 0.75, 1.0]));
        assert_eq!(Filter::NormalInvert.point(ChannelKind::Color, [0.5, 0.25, 1.0]), Some([0.5, 0.25, 1.0]));

        // Replace color shifts near colors while far colors and scalars stay
        let replacement = Filter::ReplaceColor { from: [0.8, 0.1, 0.1], to: [0.1, 0.1, 0.8], tolerance: 0.1, softness: 0.1 };
        assert!(close(replacement.point(ChannelKind::Color, [0.85, 0.1, 0.1]).unwrap(), [0.15, 0.1, 0.8]));
        assert!(close(replacement.point(ChannelKind::Color, [0.1, 0.8, 0.1]).unwrap(), [0.1, 0.8, 0.1]));

        let half = replacement.point(ChannelKind::Color, [0.8, 0.1 + 0.15 * 3f32.sqrt(), 0.1]).unwrap();

        assert!((half[2] - 0.45).abs() < 0.02, "halfway through the softness: {half:?}");
        assert_eq!(replacement.point(ChannelKind::Scalar, [0.8; 3]), Some([0.8; 3]));
    }

    #[test]
    fn a_halo_glow_lights_around_a_masks_shapes_and_a_drop_shadow_lands_under_them_away_from_the_light() {
        let row_of = |filter: Option<Filter>| {
            let mut f = fixture();
            let id = fill_layer(&mut f.project, [1.0; 3], 1.0);
            let mut mask = Mask::new(MaskBackground::Black);
            let fill = EffectKind::Fill {
                source: Source::Procedural { pattern: Procedural::new(ProceduralKind::Checker), low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() },
                projection: Projection::default(),
                sampling: Default::default(),
            };
            mask.effects.push(Effect::new(f.project.alloc_id(), fill));
            if let Some(filter) = filter {
                mask.effects.push(Effect::new(f.project.alloc_id(), EffectKind::Filter(filter)));
            }
            f.project.layer_mut(id).unwrap().mask = Some(mask);
            let out = run(&f, None);
            (0..64 * 64).map(|i| out.value(Channel::BaseColor, i % 64, i / 64)[0]).collect::<Vec<f32>>()
        };
        let plain = row_of(None);
        let (low, high) = plain.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
        let dark = |v: f32| v < low + 0.01;

        // A glow only adds, and lights the dark texels beside the shapes
        let glow = Filter::HaloGlow { on_mask: true, precise: false, width: 1.0, spread: 0.0, opacity: 1.0, color: [1.0; 3] };
        let glowing = row_of(Some(glow));
        assert!(glowing.iter().zip(&plain).all(|(g, p)| *g >= p - 1e-4), "a glow only adds");

        let lit = (0..64 * 64).filter(|&i| dark(plain[i]) && glowing[i] > low + 0.1 * (high - low)).count();
        assert!(lit > 64, "{lit} dark texels lit");

        // A shadow from a light above lands below the shapes: dark texels under a shape take it, those above one do not
        let shadow = Filter::DropShadow { angle: 90.0, distance: 0.1, size: 0.0, spread: 0.0, opacity: 0.5, color: [0.0; 3] };
        let shadowed = row_of(Some(shadow));
        assert!(shadowed.iter().zip(&plain).all(|(s, p)| *s >= p - 1e-4), "a shadow only goes under the shapes");

        let reach = 6;
        let at = |x: usize, y: usize| plain[y * 64 + x];
        let (mut below, mut above) = (Vec::new(), Vec::new());
        for y in 8..56 {
            for x in 0..64 {
                if dark(at(x, y)) && !dark(at(x, y - reach)) && (y - 2..=y).all(|row| dark(at(x, row))) {
                    below.push(shadowed[y * 64 + x] - at(x, y));
                }
                if dark(at(x, y)) && !dark(at(x, y + reach)) && dark(at(x, y - reach)) && (y..=y + 2).all(|row| dark(at(x, row))) {
                    above.push(shadowed[y * 64 + x] - at(x, y));
                }
            }
        }

        assert!(!below.is_empty() && !above.is_empty());

        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        assert!(mean(&below) > 0.2 * (high - low), "under: {below:?}");
        assert!(mean(&above) < 0.05 * (high - low), "above: {above:?}");
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
        let fx = content_fx(&mut f, id, EffectKind::Filter(Filter::Bevel { width: 128.0, wrap: false }));
        let bevel = values(&f);
        let mids = |v: &[f32]| v.iter().filter(|x| **x > 0.05 && **x < 0.95).count();

        assert!(bevel.iter().zip(&plain).all(|(b, p)| *b <= p + 1e-4 && *b >= 0.0), "a bevel only lowers values");
        assert!(mids(&bevel) > mids(&plain) + 64, "{} vs {}", mids(&bevel), mids(&plain));

        // Mosaic reduces changes between adjacent texels
        f.project.layer_mut(id).unwrap().content_effects.retain(|e| e.id != fx);
        let fx = content_fx(&mut f, id, EffectKind::Filter(Filter::Mosaic { size: 768.0, alpha: true }));
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
            let fx = Filter::BakedLighting { azimuth: 0.0, elevation, light: 0.6, ambient: 0.4, occlusion: 1.0, stylized: None };
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

        let f = Filter::BakedLighting { azimuth: 0.0, elevation: 0.0, light: 0.0, ambient: 1.0, occlusion: 0.5, stylized: None };
        assert_eq!(f.baked_light([0.0, 1.0, 0.0], 0.2), Some(0.6), "half the occlusion");
        assert_eq!(Filter::Invert.baked_light([0.0; 3], 1.0), None);
    }

    /// Baked Lighting Stylized's highlights read the roughness of the layer it lights: a smooth layer shines brighter than a rough one under the same sun
    #[test]
    fn stylized_baked_lighting_highlights_read_the_layers_roughness() {
        let lit = |rough: f32| {
            let mut f = fixture();
            let id = fill_layer(&mut f.project, [0.2; 3], rough);
            let n = primitives::plane(2.0, 2).normals[0];
            let stylized = Some(Box::new(StylizedLighting { lights: [texturecraft_core::DirectionalLight::off(0.0); 2], ..StylizedLighting::library() }));
            let elevation = if n.y > 0.0 { 90.0 } else { -90.0 };
            let fx = Filter::BakedLighting { azimuth: 0.0, elevation, light: 1.0, ambient: 0.0, occlusion: 0.0, stylized };
            content_fx(&mut f, id, EffectKind::Filter(fx));
            run(&f, None).value(Channel::BaseColor, 30, 30)
        };
        let (smooth, rough) = (lit(0.15), lit(0.9));
        assert!(smooth[0] > rough[0] + 0.05, "smooth {smooth:?} rough {rough:?}");
    }

    #[test]
    fn a_compares_histogram_reads_the_compares_under_it_against_the_stack() {
        let mut f = fixture();
        fill_layer(&mut f.project, [0.5; 3], 0.8);
        let top = fill_layer(&mut f.project, [1.0; 3], 0.2);
        let compare = |left, right| EffectKind::Compare {
            op: CompareOp::GreaterThan,
            value: 0.5,
            tolerance: 0.0,
            left,
            right,
            channel: Channel::Roughness,
            hardness: 1.0,
        };
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(f.project.alloc_id(), compare(CompareSource::LayersBelow, CompareSource::Constant)));
        let reader = Effect::new(f.project.alloc_id(), compare(CompareSource::ThisMask, CompareSource::Constant));
        let reader_id = reader.id;
        mask.effects.push(reader);
        f.project.layer_mut(top).unwrap().mask = Some(mask);
        let images = ImageStore::default();

        let [mask_so_far, _] = compare_histograms(&inputs(&f, &images, None), reader_id, 4).unwrap();

        // The roughness below (0.8) passes the first compare everywhere: the mask so far is white
        let white = *mask_so_far.last().unwrap();

        assert!(white > 0 && mask_so_far.iter().sum::<u32>() == white, "{mask_so_far:?}");
    }

    #[test]
    fn color_match_reads_the_anchor_in_either_image_input() {
        let mut fixture = fixture();
        let original_color = [0.6, 0.19, 0.09];
        let target_color = [0.1, 0.3, 0.8];
        let lower = fill_layer(&mut fixture.project, original_color, 0.5);
        let anchor = content_fx(&mut fixture, lower, EffectKind::Anchor);
        let upper = fill_layer(&mut fixture.project, original_color, 0.5);
        let source = Source::Anchor { anchor, channel: Channel::BaseColor, ramp: Vec::new() };

        let matcher = ColorMatch { source_mode: SourceColorMode::Input, target_color, custom_variation: false, ..ColorMatch::LIBRARY };
        let effect_id = content_fx(&mut fixture, upper, EffectKind::Filter(Filter::ColorMatch(matcher)));
        let effect = fixture.project.effect_mut(effect_id).unwrap();
        effect.inputs.insert(ImageInput::SOURCE_INPUT.id.to_owned(), source.clone());

        let from_anchor = run(&fixture, None).value(Channel::BaseColor, 30, 30);

        assert!(close(from_anchor, target_color), "{from_anchor:?}");

        let effect = fixture.project.effect_mut(effect_id).unwrap();
        effect.kind = EffectKind::Filter(Filter::ColorMatch(ColorMatch {
            source_mode: SourceColorMode::Parameter,
            source_color: original_color,
            target_mode: TargetColorMode::ImageInput,
            ..matcher
        }));
        effect.inputs.clear();
        effect.inputs.insert(ImageInput::TARGET_INPUT.id.to_owned(), source);

        let to_anchor = run(&fixture, None).value(Channel::BaseColor, 30, 30);

        assert!(close(to_anchor, original_color), "{to_anchor:?}");
    }

    #[test]
    fn a_histogram_shares_one_field_of_a_stage_reading_the_whole_set() {
        let mut f = fixture();
        let id = fill_layer(&mut f.project, [0.5; 3], 0.5);
        let matcher = ColorMatch { source_mode: SourceColorMode::Average, ..ColorMatch::LIBRARY };
        content_fx(&mut f, id, EffectKind::Filter(Filter::ColorMatch(matcher)));
        let images = ImageStore::default();
        let inp = inputs(&f, &images, None);
        let channels = f.project.texture_set(0).unwrap().channel_list();

        let projectors = ProjectorDepthCache::default();
        let inputs = EvaluationInputs::new(&inp, &projectors);

        let shared = shared_histogram_fields(&inputs, &channels, &all_tiles(64, 64));

        assert!(
            shared.is_some_and(|fields| fields.contains_key(&FieldKey::content(id, Channel::BaseColor, ContentsOffset::NONE))),
            "every sampled tile reads one field"
        );
    }

    #[test]
    fn a_filtered_mask_compares_the_layers_filtered_content() {
        let mut f = fixture();
        fill_layer(&mut f.project, [0.5; 3], 0.5);
        let top = fill_layer(&mut f.project, [1.0; 3], 0.9);
        content_fx(&mut f, top, EffectKind::Filter(Filter::EdgeDetect { width: 64.0 }));
        let compare = EffectKind::Compare {
            op: CompareOp::GreaterThan,
            value: 0.7,
            tolerance: 0.0,
            left: CompareSource::ThisLayer,
            right: CompareSource::Constant,
            channel: Channel::Roughness,
            hardness: 1.0,
        };
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(f.project.alloc_id(), compare));
        mask.effects.push(Effect::new(f.project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 2.0, wrap: true })));
        f.project.layer_mut(top).unwrap().mask = Some(mask);

        let color = run(&f, None).value(Channel::BaseColor, 30, 30);

        // Uniform roughness 0.9 has no edges: black after the edge detection, it fails the compare and the layer below shows
        assert!(close(color, [0.5; 3]), "{color:?}");
    }

    #[test]
    fn a_compare_mask_blends_onto_the_mask_with_its_blend_mode() {
        let mut f = fixture();
        fill_layer(&mut f.project, [0.5; 3], 0.8);
        let top = fill_layer(&mut f.project, [1.0; 3], 0.2);

        // The roughness below (0.8) passes everywhere, but a white compare multiplied onto a black mask stays black
        let mut compare = Effect::new(
            f.project.alloc_id(),
            EffectKind::Compare {
                op: CompareOp::GreaterThan,
                value: 0.5,
                tolerance: 0.0,
                left: CompareSource::LayersBelow,
                right: CompareSource::Constant,
                channel: Channel::Roughness,
                hardness: 1.0,
            },
        );
        compare.blend = BlendMode::Multiply;
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(compare);
        f.project.layer_mut(top).unwrap().mask = Some(mask);

        let color = run(&f, None).value(Channel::BaseColor, 30, 30);

        assert!(close(color, [0.5; 3]), "the layer under shows: {color:?}");
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
        f.project.layer_mut(id).unwrap().geometry_mask = Some(vec!["Lid".into()]);
        let lid = red(&f);
        f.project.layer_mut(id).unwrap().geometry_mask = Some(vec!["Body".into()]);
        let body = red(&f);
        assert!(lid > 0 && body > 0 && lid + body == all, "{lid} + {body} of {all}");
        f.project.layer_mut(id).unwrap().geometry_mask = Some(Vec::new());
        assert_eq!(red(&f), 0, "no parts: hidden everywhere");
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
        let images = ImageStore::default();
        let inp = inputs(&f, &images, None);
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
        mask.effects.push(Effect::new(f.project.alloc_id(), EffectKind::Fill { source, projection: Projection::default(), sampling: Default::default() }));
        f.project.layer_mut(top).unwrap().mask = Some(mask.clone());

        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [0.7; 3]));

        // The field pre-pass reads anchors under blur, preserving flat height
        mask.effects.push(Effect::new(f.project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 32.0, wrap: false })));
        f.project.layer_mut(top).unwrap().mask = Some(mask);

        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [0.7; 3]));

        // An anchor above its reader is not evaluated yet, so it reads the channel default
        f.project.move_layer(low, 1).unwrap();

        assert!(close(run(&f, None).value(Channel::BaseColor, 5, 5), [0.5; 3]));
    }

    /// A white fill layer on top whose mask reads `anchor` (any channel: a mask anchor is gray)
    fn anchor_reader(fixture: &mut Fixture, anchor: LayerId) -> LayerId {
        let top = fill_layer(&mut fixture.project, [1.0; 3], 0.5);
        let mut mask = Mask::new(MaskBackground::Black);
        let source = Source::Anchor { anchor, channel: Channel::Roughness, ramp: Vec::new() };
        mask.effects
            .push(Effect::new(fixture.project.alloc_id(), EffectKind::Fill { source, projection: Projection::default(), sampling: Default::default() }));
        fixture.project.layer_mut(top).unwrap().mask = Some(mask);
        top
    }

    #[test]
    fn mask_anchors_record_the_mask_value_at_their_place() {
        let mut fixture = fixture();
        let low = fill_layer(&mut fixture.project, [0.0; 3], 0.5);
        let (gray, anchor, full) = (fixture.project.alloc_id(), fixture.project.alloc_id(), fixture.project.alloc_id());
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects
            .push(Effect::new(gray, EffectKind::Fill { source: Source::Uniform([0.4; 3]), projection: Projection::default(), sampling: Default::default() }));
        mask.effects.push(Effect::new(anchor, EffectKind::Anchor));
        mask.effects
            .push(Effect::new(full, EffectKind::Fill { source: Source::Uniform([1.0; 3]), projection: Projection::default(), sampling: Default::default() }));
        fixture.project.layer_mut(low).unwrap().mask = Some(mask.clone());
        anchor_reader(&mut fixture, anchor);

        // The low layer shows fully (black), the top one at the 0.4 the anchor recorded
        assert!(close(run(&fixture, None).value(Channel::BaseColor, 5, 5), [0.4; 3]));

        // Under a blur precomputed over the region the anchor still records its own place
        mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 32.0, wrap: false })));
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
        mask.effects.push(Effect::new(
            fixture.project.alloc_id(),
            EffectKind::Fill { source: checker, projection: Projection::default(), sampling: Default::default() },
        ));
        mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 64.0, wrap: false })));
        mask.effects.push(Effect::new(anchor, EffectKind::Anchor));
        fixture.project.layer_mut(lower).unwrap().mask = Some(mask.clone());
        let reader = anchor_reader(&mut fixture, anchor);
        let texels: Vec<_> = (0..128).map(|x| (x, 63)).collect();
        let expected = assert_sparse_mask_matches_image(&fixture, reader, &texels).unwrap();

        assert!(expected.iter().any(|value| *value > 0.1 && *value < 0.9), "the anchor records a softened checker");

        mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 128.0, wrap: false })));
        fixture.project.layer_mut(lower).unwrap().mask = Some(mask);
        let actual = assert_sparse_mask_matches_image(&fixture, reader, &texels).unwrap();

        assert_eq!(actual, expected, "a filter above the anchor must not change its recorded value");
    }

    #[test]
    fn a_content_anchor_records_the_blur_below_it() {
        let mut fixture = fixture();
        let low = fixture.project.alloc_id();
        let mut fill = Fill::default();
        let checker = Source::Procedural { pattern: Procedural::new(ProceduralKind::Checker), low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() };
        fill.sources.insert(Channel::BaseColor, checker);
        let layer = Layer::new(low, "Checker", LayerKind::Fill(fill), &[Channel::BaseColor]);
        fixture.project.insert_layer(0, layer, None, false).unwrap();
        content_fx(&mut fixture, low, EffectKind::Filter(Filter::Blur { radius: 96.0, wrap: false }));
        let anchor = content_fx(&mut fixture, low, EffectKind::Anchor);
        let reader = fixture.project.alloc_id();
        let mut read = Fill::default();
        read.sources.insert(Channel::BaseColor, Source::Anchor { anchor, channel: Channel::BaseColor, ramp: Vec::new() });
        let layer = Layer::new(reader, "Reader", LayerKind::Fill(read), &[Channel::BaseColor]);
        fixture.project.insert_layer(0, layer, None, false).unwrap();
        let before = run(&fixture, None);

        content_fx(&mut fixture, low, EffectKind::Filter(Filter::Blur { radius: 160.0, wrap: false }));
        let after = run(&fixture, None);

        for x in 0..64 {
            assert_eq!(before.value(Channel::BaseColor, x, 32), after.value(Channel::BaseColor, x, 32));
        }
    }

    #[test]
    fn an_anchor_between_content_effects_keeps_the_layers_own_lighting() {
        let mut fixture = fixture();
        let low = fixture.project.alloc_id();
        let mut fill = Fill::default();

        // A gray (sRGB 140) dim for a metal but valid for a dielectric: PBR Validate turns it red only with the layer's own metal
        let dim_metal = [0.26; 3];
        fill.sources.insert(Channel::BaseColor, Source::Uniform(dim_metal));
        fill.sources.insert(Channel::Metallic, Source::Uniform([1.0; 3]));
        let layer = Layer::new(low, "Metal", LayerKind::Fill(fill), &[Channel::BaseColor, Channel::Metallic]);
        fixture.project.insert_layer(0, layer, None, false).unwrap();
        content_fx(&mut fixture, low, EffectKind::Filter(Filter::PbrValidate(PbrValidate::LIBRARY)));
        let anchor = content_fx(&mut fixture, low, EffectKind::Anchor);
        let reader = fixture.project.alloc_id();
        let mut read = Fill::default();
        read.sources.insert(Channel::BaseColor, Source::Anchor { anchor, channel: Channel::BaseColor, ramp: Vec::new() });
        let layer = Layer::new(reader, "Reader", LayerKind::Fill(read), &[Channel::BaseColor]);
        fixture.project.insert_layer(0, layer, None, false).unwrap();
        let uncut = run(&fixture, None).value(Channel::BaseColor, 5, 5);

        // An effect above the anchor cuts the content stack there: the cut is checked against the same metal
        content_fx(&mut fixture, low, EffectKind::Levels(Default::default()));
        let cut = run(&fixture, None).value(Channel::BaseColor, 5, 5);

        assert!(close(uncut, PbrValidate::INVALID), "{uncut:?}");
        assert!(close(cut, uncut), "{cut:?}");
    }

    #[test]
    fn histogram_counts_the_result_after_pbr_validate() {
        let mut fixture = fixture();
        let metal_layer = fixture.project.alloc_id();
        let mut metal = Fill::default();
        metal.sources.insert(Channel::Metallic, Source::Uniform([1.0; 3]));
        let layer = Layer::new(metal_layer, "Metal", LayerKind::Fill(metal), &[Channel::Metallic]);
        fixture.project.insert_layer(0, layer, None, false).unwrap();

        let color_layer = fill_layer(&mut fixture.project, [0.26; 3], 0.5);
        fixture.project.layer_mut(color_layer).unwrap().channels.remove(&Channel::Metallic);
        content_fx(&mut fixture, color_layer, EffectKind::Filter(Filter::PbrValidate(PbrValidate::LIBRARY)));
        let levels = content_fx(&mut fixture, color_layer, EffectKind::Levels(Default::default()));
        let images = ImageStore::default();

        let histogram = effect_histogram(&inputs(&fixture, &images, None), levels, Channel::BaseColor, 16).unwrap();

        let [red, green, blue] = histogram.components.as_slice() else { panic!("expected RGB bins") };
        let samples = red.iter().sum::<u32>();
        assert!(samples > 0);
        assert_eq!(red.last(), Some(&samples));
        assert_eq!(green.first(), Some(&samples));
        assert_eq!(blue.first(), Some(&samples));
    }

    #[test]
    fn a_filter_and_a_flatten_above_pbr_validate_keep_the_layers_own_metal() {
        let mut fixture = fixture();
        let id = fixture.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::BaseColor, Source::Uniform([0.26; 3]));
        fill.sources.insert(Channel::Metallic, Source::Uniform([1.0; 3]));
        let layer = Layer::new(id, "Metal", LayerKind::Fill(fill), &[Channel::BaseColor, Channel::Metallic]);
        fixture.project.insert_layer(0, layer, None, false).unwrap();
        content_fx(&mut fixture, id, EffectKind::Filter(Filter::PbrValidate(PbrValidate::LIBRARY)));
        let images = ImageStore::default();
        let flat = flatten_layer(&inputs(&fixture, &images, None), id, &all_tiles(64, 64)).unwrap();
        content_fx(&mut fixture, id, EffectKind::Filter(Filter::Blur { radius: 4.0, wrap: false }));

        let blurred = run(&fixture, None).value(Channel::BaseColor, 30, 30);

        // A dim gray is valid for a dielectric only: the layer's metal makes it red in the filter's prepass and in the flattened layer
        assert!(close(blurred, PbrValidate::INVALID), "{blurred:?}");

        let (_, _, data) = flat.iter().find(|(channel, tile, _)| *channel == Channel::BaseColor && *tile == (0, 0)).unwrap();
        let texel = (30 * TILE as usize + 30) * 4;
        let flattened: [f32; 3] = std::array::from_fn(|i| from_u16(data[texel + i]));
        assert!(flattened.iter().zip(PbrValidate::INVALID).all(|(value, invalid)| (value - invalid).abs() < 1e-3), "{flattened:?}");
    }

    #[test]
    fn a_micro_height_read_from_an_anchor_follows_its_raised_region() {
        let mut fixture = fixture();
        fixture.baked = BakedMaps { resolution: 64, maps: [(BakedMap::Curvature, Arc::new(vec![0.5; 64 * 64]))].into() };
        let disc = (0..64 * 64)
            .map(|index| {
                let (x, y) = ((index % 64) as f32 - 31.5, (index / 64) as f32 - 31.5);
                if x * x + y * y < 100.0 { [1.0; 4] } else { [0.0, 0.0, 0.0, 1.0] }
            })
            .collect();
        let images: ImageStore = [("disc".to_string(), Arc::new(ImageRes::new(64, 64, disc, false)))].into();
        let low = fixture.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::Height, Source::image("disc"));
        fixture.project.insert_layer(0, Layer::new(low, "Bumps", LayerKind::Fill(fill), &[Channel::Height]), None, false).unwrap();
        let anchor = content_fx(&mut fixture, low, EffectKind::Anchor);
        let top = fill_layer(&mut fixture.project, [1.0; 3], 0.5);
        let colors = |fixture: &mut Fixture, generator: &Generator| -> Vec<f32> {
            let mut mask = Mask::new(MaskBackground::Black);
            mask.effects.push(Effect::new(LayerId(900), EffectKind::Generator(generator.clone())));
            fixture.project.layer_mut(top).unwrap().mask = Some(mask);
            let composite = run_with_images(fixture, None, &images);
            (0..64 * 64).map(|index| composite.value(Channel::BaseColor, index % 64, index / 64)[0]).collect()
        };
        let largest_change = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);

        // Curvature on its section controls, then on the earlier model a project saved before Micro Details keeps
        let section = Generator::new(GeneratorKind::Curvature);
        let earlier = Generator { section: None, level: 0.5, softness: 0.5, noise_amount: 0.0, ..section.clone() };
        for plain in [section, earlier] {
            let off = colors(&mut fixture, &plain);
            let read = Generator { micro: MicroDetails::of_anchor(GeneratorKind::Curvature, anchor, 1.0).map(Box::new), ..plain.clone() };

            let raised = colors(&mut fixture, &read);
            assert!(largest_change(&raised, &off) > 0.05, "the raised disc's rim reads as curvature");

            let mut emptied = read.clone();
            if let Some(micro) = emptied.micro.as_mut() {
                micro.height_source = None;
            }
            assert_eq!(colors(&mut fixture, &emptied), off, "without an anchor the micro details change nothing");
        }
    }

    /// A 64 × 64 disc of raised height in an image named "disc", and a fixture with a flat baked curvature for micro details to read beside
    fn micro_height_fixture() -> (Fixture, ImageStore) {
        let mut fixture = fixture();
        fixture.baked = BakedMaps { resolution: 64, maps: [(BakedMap::Curvature, Arc::new(vec![0.5; 64 * 64]))].into() };
        let disc = (0..64 * 64)
            .map(|index| {
                let (x, y) = ((index % 64) as f32 - 31.5, (index / 64) as f32 - 31.5);
                if x * x + y * y < 100.0 { [1.0; 4] } else { [0.0, 0.0, 0.0, 1.0] }
            })
            .collect();
        (fixture, [("disc".to_string(), Arc::new(ImageRes::new(64, 64, disc, false)))].into())
    }

    /// A Curvature generator whose Micro Height reads anchor point `anchor`
    fn reading_micro_height(anchor: LayerId) -> Generator {
        Generator { micro: MicroDetails::of_anchor(GeneratorKind::Curvature, anchor, 1.0).map(Box::new), ..Generator::new(GeneratorKind::Curvature) }
    }

    /// A height fill of `source`, with an anchor point after it: the layer's id and the anchor's
    fn anchored_height(fixture: &mut Fixture, id: LayerId, source: Source) -> (LayerId, LayerId) {
        let mut fill = Fill::default();
        fill.sources.insert(Channel::Height, source);
        let layer = fixture.project.insert_layer(0, Layer::new(id, "Height", LayerKind::Fill(fill), &[Channel::Height]), None, false).unwrap();
        (layer, content_fx(fixture, layer, EffectKind::Anchor))
    }

    fn base_color_reds(composite: &Composite) -> Vec<f32> {
        (0..64 * 64).map(|index| composite.value(Channel::BaseColor, index % 64, index / 64)[0]).collect()
    }

    #[test]
    fn micro_height_anchor_is_read_when_its_mask_is_requested_on_a_hidden_layer() {
        let (mut fixture, images) = micro_height_fixture();
        let height_layer = fixture.project.alloc_id();
        let (_, anchor) = anchored_height(&mut fixture, height_layer, Source::image("disc"));
        let top = fill_layer(&mut fixture.project, [1.0; 3], 0.5);
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(LayerId(900), EffectKind::Generator(reading_micro_height(anchor))));
        let layer = fixture.project.layer_mut(top).unwrap();
        layer.mask = Some(mask);
        layer.visible = false;
        let sample = (28, 23);

        let hidden = mask_at(&inputs(&fixture, &images, None), top, &[sample]).unwrap();
        fixture.project.layer_mut(top).unwrap().visible = true;
        let visible = mask_at(&inputs(&fixture, &images, None), top, &[sample]).unwrap();

        assert_eq!(hidden, visible, "requested masks read their anchors whether or not their layer is displayed");
    }

    #[test]
    fn sampling_a_disabled_mask_for_painting_keeps_its_micro_height() {
        let (mut fixture, images) = micro_height_fixture();
        let height = fixture.project.alloc_id();
        let (_, anchor) = anchored_height(&mut fixture, height, Source::image("disc"));
        let reader = fill_layer(&mut fixture.project, [1.0; 3], 0.5);
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Generator(reading_micro_height(anchor))));
        fixture.project.layer_mut(reader).unwrap().mask = Some(mask);

        let sample = |fixture: &Fixture| mask_values(&inputs(fixture, &images, None), reader, &[(0, 0)]).unwrap();
        let unfiltered = sample(&fixture);
        let blur = Effect::new(fixture.project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 8.0, wrap: false }));
        fixture.project.layer_mut(reader).unwrap().mask.as_mut().unwrap().effects.push(blur);
        let enabled = sample(&fixture);
        fixture.project.layer_mut(reader).unwrap().mask.as_mut().unwrap().enabled = false;
        let disabled = sample(&fixture);

        assert!(enabled.tile(0, 0) != unfiltered.tile(0, 0), "the blur changes the anchored mask");
        assert!(enabled.tile(0, 0).unwrap().windows(2).any(|pair| pair[0] != pair[1]));
        assert!(enabled.tile(0, 0) == disabled.tile(0, 0), "direct sampling must read the anchored mask in either state");
    }

    #[test]
    fn explicit_micro_height_mask_reads_lower_anchors_under_a_hidden_folder() {
        let (mut fixture, images) = micro_height_fixture();
        let height = fixture.project.alloc_id();
        let (_, anchor) = anchored_height(&mut fixture, height, Source::image("disc"));
        let folder = fixture.project.alloc_id();
        let reader = fixture.project.alloc_id();
        let group = LayerKind::Group { children: Vec::new(), collapsed: false, contents: Default::default(), randomized: Vec::new() };
        fixture.project.insert_layer(0, Layer::new(folder, "Folder", group, &[Channel::BaseColor]), None, false).unwrap();
        fixture.project.insert_layer(0, Layer::new(reader, "Reader", LayerKind::Fill(Fill::default()), &[Channel::BaseColor]), Some(folder), true).unwrap();
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(LayerId(900), EffectKind::Generator(reading_micro_height(anchor))));
        fixture.project.layer_mut(reader).unwrap().mask = Some(mask);
        let read = |fixture: &Fixture| mask_texels(&inputs(fixture, &images, None), reader).unwrap();

        assert_eq!(fixture.project.locate(reader).and_then(|(_, parent, _)| parent), Some(folder));
        let visible = read(&fixture);
        fixture.project.layer_mut(folder).unwrap().visible = false;
        let hidden = read(&fixture);

        assert!(hidden.iter().zip(&visible).all(|(hidden, visible)| hidden == visible), "hiding a parent preserves the requested mask's lower external anchor");
    }

    #[test]
    fn micro_height_anchor_changes_when_its_input_image_changes() {
        let (mut fixture, mut images) = micro_height_fixture();
        let height_layer = fixture.project.alloc_id();
        let (_, anchor) = anchored_height(&mut fixture, height_layer, Source::image("disc"));
        let top = fill_layer(&mut fixture.project, [1.0; 3], 0.5);
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(LayerId(900), EffectKind::Generator(reading_micro_height(anchor))));
        fixture.project.layer_mut(top).unwrap().mask = Some(mask);

        let raised = mask_texels(&inputs(&fixture, &images, None), top).unwrap();
        images.insert("disc".to_owned(), Arc::new(ImageRes::new(1, 1, vec![[0.0, 0.0, 0.0, 1.0]], false)));
        let flattened = mask_texels(&inputs(&fixture, &images, None), top).unwrap();

        assert!(raised.iter().zip(flattened).any(|(raised, flat)| (*raised - flat).abs() > 1e-4));
    }

    #[test]
    fn micro_height_uses_the_live_stroke_and_restores_the_committed_anchor_after_cancel() {
        let (mut fixture, images) = micro_height_fixture();
        let painted = fixture.project.alloc_id();
        fixture.project.insert_layer(0, Layer::new(painted, "Height paint", LayerKind::Paint, &[Channel::Height]), None, false).unwrap();
        let anchor = content_fx(&mut fixture, painted, EffectKind::Anchor);
        let reader = fill_layer(&mut fixture.project, [1.0; 3], 0.5);
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(LayerId(900), EffectKind::Generator(reading_micro_height(anchor))));
        fixture.project.layer_mut(reader).unwrap().mask = Some(mask);

        let mut alpha = TileImage::new(64, 64, 1);
        for y in 24..40 {
            for x in 24..40 {
                alpha.set(x, y, &[u16::MAX]);
            }
        }

        let preview = StrokePreview {
            target: PaintTarget::Layer(painted),
            alpha,
            opacity: 1.0,
            erase: false,
            values: [(Channel::Height, [1.0; 3])].into_iter().collect(),
            mask_gray: MaskGray::Value(1.0),
            stencil: None,
            carried: BTreeMap::new(),
            carried_mask: None,
        };
        let mut projectors = ProjectorDepthCache::default();
        let mesh = Arc::new(fixture.mesh.clone());
        let composite = |preview, projectors: &mut ProjectorDepthCache| {
            let mut out = Composite::new(&[Channel::BaseColor], 64);
            let inputs = Inputs { mesh: &mesh, ..inputs(&fixture, &images, preview) };
            composite_tiles_with_projectors(&inputs, &mesh, projectors, &mut out, &[(0, 0)], None);
            base_color_reds(&out)
        };

        let original = composite(None, &mut projectors);
        let painted = composite(Some(&preview), &mut projectors);
        let cancelled = composite(None, &mut projectors);

        assert!(painted.iter().zip(&original).any(|(now, before)| (now - before).abs() > 0.05));
        assert_eq!(cancelled, original);
    }

    #[test]
    fn anchor_cache_releases_distinct_heights_when_readers_and_documents_go_away() {
        let mut projectors = ProjectorDepthCache::default();
        let (mut fixture, images) = micro_height_fixture();
        let lower = fixture.project.alloc_id();
        let (lower, first) = anchored_height(&mut fixture, lower, Source::image("disc"));
        let second = content_fx(&mut fixture, lower, EffectKind::Anchor);
        let third = content_fx(&mut fixture, lower, EffectKind::Anchor);
        let reader = fill_layer(&mut fixture.project, [1.0; 3], 0.5);
        let mut mask = Mask::new(MaskBackground::Black);
        for anchor in [first, second, third] {
            mask.effects.push(Effect::new(fixture.project.alloc_id(), EffectKind::Generator(reading_micro_height(anchor))));
        }
        fixture.project.layer_mut(reader).unwrap().mask = Some(mask);

        let inputs = inputs(&fixture, &images, None);
        let stack = Arc::new(StackKey::new(inputs.project, inputs.pixels, inputs.baked, inputs.images, 0, &inputs.project.texture_sets[0].stack).unwrap());
        let build = || AnchorHeights::new(vec![0.0; 64 * 64], (64, 64), inputs.block_scales.clone());
        let first_mesh = Arc::new(fixture.mesh.clone());
        projectors.bind(&first_mesh);

        // Keep one active evaluation alive through deletion
        let active = projectors.micro_heights.get_or_build(&inputs, (first, 0), &stack, build);
        let removed = Arc::downgrade(&projectors.micro_heights.get_or_build(&inputs, (second, 0), &stack, build));
        let retained = Arc::downgrade(&projectors.micro_heights.get_or_build(&inputs, (third, 0), &stack, build));
        projectors.micro_heights.retain_readers(0, &[(third, 0)]);

        assert!(removed.upgrade().is_none());
        assert!(retained.upgrade().is_some());
        assert_eq!(Arc::strong_count(&active), 1);

        let replacement = Arc::new(fixture.mesh.clone());
        projectors.bind(&replacement);
        assert!(retained.upgrade().is_none());

        let closed = Arc::downgrade(&projectors.micro_heights.get_or_build(&inputs, (third, 0), &stack, build));
        drop(projectors);
        assert!(closed.upgrade().is_none());
    }

    #[test]
    fn every_anchor_height_a_render_reads_stays_prepared_however_many_it_reads() {
        const OTHER_READERS: usize = 8;
        let colors = |others_read: bool| {
            let (mut fixture, images) = micro_height_fixture();
            let id = fixture.project.alloc_id();
            let (low, first) = anchored_height(&mut fixture, id, Source::image("disc"));
            let others: Vec<LayerId> = (0..OTHER_READERS).map(|_| content_fx(&mut fixture, low, EffectKind::Anchor)).collect();
            let top = fill_layer(&mut fixture.project, [1.0; 3], 0.5);
            let mut mask = Mask::new(MaskBackground::Black);
            mask.effects.push(Effect::new(LayerId(900), EffectKind::Generator(reading_micro_height(first))));
            fixture.project.layer_mut(top).unwrap().mask = Some(mask);

            // Readers of eight more anchors on a layer of height only, drawn above
            let id = fixture.project.alloc_id();
            let readers =
                fixture.project.insert_layer(0, Layer::new(id, "Readers", LayerKind::Fill(Fill::default()), &[Channel::Height]), None, false).unwrap();
            let mut mask = Mask::new(MaskBackground::Black);
            for (index, anchor) in others.iter().enumerate().filter(|_| others_read) {
                mask.effects.push(Effect::new(LayerId(901 + index as u64), EffectKind::Generator(reading_micro_height(*anchor))));
            }
            fixture.project.layer_mut(readers).unwrap().mask = Some(mask);
            base_color_reds(&run_with_images(&fixture, None, &images))
        };

        assert_eq!(colors(true), colors(false), "a ninth anchor read keeps the first one's height");
    }

    #[test]
    fn an_anchor_height_holding_a_reader_of_a_lower_anchor_is_built_after_it_whatever_the_layer_ids() {
        let colors = |top_id_first: bool, connected: bool| {
            let (mut fixture, images) = micro_height_fixture();
            let early = fixture.project.alloc_id();
            let late = fixture.project.alloc_id();
            let (top_id, middle_id) = if top_id_first { (early, late) } else { (late, early) };
            let id = fixture.project.alloc_id();
            let (_, lower) = anchored_height(&mut fixture, id, Source::image("disc"));

            // The upper anchor records the middle layer's mask: a reader of the lower anchor
            let middle =
                fixture.project.insert_layer(0, Layer::new(middle_id, "Middle", LayerKind::Fill(Fill::default()), &[Channel::Height]), None, false).unwrap();
            let upper = fixture.project.alloc_id();
            let mut mask = Mask::new(MaskBackground::Black);
            let generator = if connected { reading_micro_height(lower) } else { Generator::new(GeneratorKind::Curvature) };
            mask.effects.push(Effect::new(LayerId(900), EffectKind::Generator(generator)));
            mask.effects.push(Effect::new(upper, EffectKind::Anchor));
            fixture.project.layer_mut(middle).unwrap().mask = Some(mask);

            let mut fill = Fill::default();
            fill.sources.insert(Channel::BaseColor, Source::Uniform([1.0; 3]));
            let top = fixture.project.insert_layer(0, Layer::new(top_id, "Top", LayerKind::Fill(fill), &[Channel::BaseColor]), None, false).unwrap();
            let mut mask = Mask::new(MaskBackground::Black);
            mask.effects.push(Effect::new(LayerId(901), EffectKind::Generator(reading_micro_height(upper))));
            fixture.project.layer_mut(top).unwrap().mask = Some(mask);
            base_color_reds(&run_with_images(&fixture, None, &images))
        };

        let (by_draw_order, against_it) = (colors(false, true), colors(true, true));
        let disconnected = colors(false, false);
        let largest_change = by_draw_order.iter().zip(&against_it).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(largest_change < 1e-4, "{largest_change}");
        assert!(by_draw_order.iter().zip(disconnected).any(|(connected, disconnected)| (*connected - disconnected).abs() > 1e-4));
    }

    #[test]
    fn a_histogram_below_a_reader_leaves_the_anchor_heights_it_reads() {
        let (mut fixture, images) = micro_height_fixture();
        let id = fixture.project.alloc_id();
        let (low, anchor) = anchored_height(&mut fixture, id, Source::image("disc"));
        let levels = content_fx(&mut fixture, low, EffectKind::Levels(Default::default()));
        let top = fill_layer(&mut fixture.project, [1.0; 3], 0.5);
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(LayerId(900), EffectKind::Generator(reading_micro_height(anchor))));
        fixture.project.layer_mut(top).unwrap().mask = Some(mask);
        let anchors = ProjectorDepthCache::default();
        let evaluation = inputs(&fixture, &images, None);
        let mut session = EvaluationInputs::new(&evaluation, &anchors).of_the_session_stack();
        prepare_generators(&mut session, None);

        // The histogram evaluates the stack cut below the levels
        effect_histogram(&inputs(&fixture, &images, None), levels, Channel::Height, 16);

        let stack = StackKey::new(&fixture.project, &fixture.pixels, &fixture.baked, &images, 0, &fixture.project.texture_sets[0].stack)
            .expect("the plane's stack has a key");
        let builds = std::cell::Cell::new(0);
        anchors.micro_heights.get_or_build(&evaluation, (anchor, 0), &Arc::new(stack), || {
            builds.set(builds.get() + 1);
            AnchorHeights::new(vec![0.0; 64 * 64], (64, 64), fixture.project.texture_sets[0].block_scales())
        });
        assert_eq!(builds.get(), 0, "the render's heights are still prepared");
    }

    #[test]
    fn anchor_heights_are_built_again_at_another_working_size() {
        let (mut fixture, images) = micro_height_fixture();
        let id = fixture.project.alloc_id();
        let (_, anchor) = anchored_height(&mut fixture, id, Source::image("disc"));
        let stack = Arc::new(
            StackKey::new(&fixture.project, &fixture.pixels, &fixture.baked, &images, 0, &fixture.project.texture_sets[0].stack)
                .expect("the plane's stack has a key"),
        );
        let cache = ProjectorDepthCache::default();
        let cache = &cache.micro_heights;
        let builds = std::cell::Cell::new(0);
        let build = |size: (u32, u32)| {
            builds.set(builds.get() + 1);
            AnchorHeights::new(vec![0.0; (size.0 * size.1) as usize], size, fixture.project.texture_sets[0].block_scales())
        };
        let half = UvGBuffer::new(&fixture.mesh, 0, 32, 32);
        let inputs_at = |gbuf| Inputs { gbuf, ..inputs(&fixture, &images, None) };

        cache.get_or_build(&inputs_at(&fixture.gbuf), (anchor, 0), &stack, || build((64, 64)));
        cache.get_or_build(&inputs_at(&fixture.gbuf), (anchor, 0), &stack, || build((64, 64)));
        assert_eq!(builds.get(), 1, "the same stack and size reuse them");

        cache.get_or_build(&inputs_at(&half), (anchor, 0), &stack, || build((32, 32)));
        assert_eq!(builds.get(), 2, "a smaller working atlas builds them again");
    }

    /// A field build that counts each call in `builds`
    fn counted_build(builds: &std::cell::Cell<usize>) -> impl FnOnce((u32, u32)) -> Vec<f32> + '_ {
        move |_| {
            builds.set(builds.get() + 1);
            vec![0.0; 4]
        }
    }

    #[test]
    fn a_whole_map_field_is_built_again_when_its_mesh_map_is_read_at_another_bake_size() {
        let cache = crate::field_cache::FieldCache::new();
        let builds = std::cell::Cell::new(0);
        let mut fixture = fixture();
        let occlusion = Arc::new(vec![0.5; 16]);
        let images = ImageStore::default();
        for resolution in [4, 4, 8] {
            fixture.baked = BakedMaps { resolution, maps: [(BakedMap::AmbientOcclusion, occlusion.clone())].into() };
            let inputs = inputs(&fixture, &images, None);
            cache.get_or_build(&inputs, crate::field_cache::field_identity(&inputs), &[BakedMap::AmbientOcclusion], Vec::new(), counted_build(&builds));
        }

        assert_eq!(builds.get(), 2, "4 × 4 reuses its field; the same map read as 8 × 2 builds another");
    }

    #[test]
    fn a_whole_map_field_is_built_again_after_any_texel_of_its_coverage_changes() {
        let cache = crate::field_cache::FieldCache::new();
        let builds = std::cell::Cell::new(0);
        let mut fixture = fixture();
        let images = ImageStore::default();
        let build_on = |fixture: &Fixture| {
            let inputs = inputs(fixture, &images, None);
            cache.get_or_build(&inputs, crate::field_cache::field_identity(&inputs), &[], Vec::new(), counted_build(&builds));
        };
        build_on(&fixture);
        build_on(&fixture);

        // The second texel, in place: the same allocation, one triangle different
        fixture.gbuf.tri[1] = if fixture.gbuf.tri[1] == NO_TRI { 0 } else { NO_TRI };
        build_on(&fixture);

        assert_eq!(builds.get(), 2);
    }

    #[test]
    fn uv_checkers_follow_in_place_mesh_edits() {
        let mut fixture = fixture();
        let images = ImageStore::default();
        let checker = texturecraft_core::UvCheckerParameters { random_colors: false, border_width: 0.0, grid_tile: 1, grid_opacity: 0.0, ..Default::default() };
        let render = |fixture: &Fixture| {
            let inputs = inputs(fixture, &images, None);
            let projectors = ProjectorDepthCache::default();
            let evaluation = EvaluationInputs::new(&inputs, &projectors);
            crate::uv_islands::checker_field(&evaluation, evaluation.field_identity(), &checker, 42)
        };
        let first = render(&fixture);

        for uv in &mut fixture.mesh.uvs {
            uv.x += 0.25;
        }
        let edited = render(&fixture);

        let fresh = crate::uv_islands::checker_field(&inputs(&fixture, &images, None), 0, &checker, 42);
        assert_eq!(edited.as_slice(), fresh.as_slice(), "the edited pixels match a cold build");

        assert!(first.iter().zip(edited.iter()).any(|(first, edited)| first != edited), "moving UVs changes the checker pattern");
    }

    #[test]
    fn a_whole_map_field_read_again_outlives_fields_built_before_it() {
        let fields_kept = crate::field_cache::CACHE_ENTRIES as u32;
        let cache = crate::field_cache::FieldCache::new();
        let builds = std::cell::Cell::new(0);
        let fixture = fixture();
        let images = ImageStore::default();
        let inputs = inputs(&fixture, &images, None);
        let coverage = crate::field_cache::field_identity(&inputs);
        let field = |settings: u32| {
            cache.get_or_build(&inputs, coverage, &[], vec![settings], counted_build(&builds));
        };
        (0..fields_kept).for_each(field);
        field(0);
        field(fields_kept);
        let built = builds.get();
        field(0);

        assert_eq!(builds.get(), built, "the field read again stays when a newer one pushes the oldest out");
    }

    #[test]
    fn a_micro_detail_saved_before_micro_details_renders_as_saved_on_the_earlier_model() {
        const SAVED_AMOUNT: f32 = 0.25;
        const FLAT_CURVATURE: f32 = 0.4;
        let colors = |height: f32, curvature: f32, micro: bool| -> Vec<f32> {
            let mut fixture = fixture();
            fixture.baked = BakedMaps { resolution: 64, maps: [(BakedMap::Curvature, Arc::new(vec![curvature; 64 * 64]))].into() };
            let low = fixture.project.alloc_id();
            let mut fill = Fill::default();
            fill.sources.insert(Channel::Height, Source::Uniform([height; 3]));
            fixture.project.insert_layer(0, Layer::new(low, "Flat", LayerKind::Fill(fill), &[Channel::Height]), None, false).unwrap();
            let anchor = content_fx(&mut fixture, low, EffectKind::Anchor);
            let top = fill_layer(&mut fixture.project, [1.0; 3], 0.5);
            let earlier = Generator { section: None, detail: None, level: 0.5, softness: 0.5, noise_amount: 0.0, ..Generator::new(GeneratorKind::Curvature) };
            let micro = MicroDetails::of_anchor(GeneratorKind::Curvature, anchor, SAVED_AMOUNT).filter(|_| micro).map(Box::new);
            let mut mask = Mask::new(MaskBackground::Black);
            mask.effects.push(Effect::new(LayerId(900), EffectKind::Generator(Generator { micro, ..earlier })));
            fixture.project.layer_mut(top).unwrap().mask = Some(mask);
            base_color_reds(&run(&fixture, None))
        };

        // The saved micro detail added the anchor's height times its amount to the curvature signal, even where the height is flat
        for height in [0.0, 0.3, 0.6, 1.0] {
            let saved = colors(0.0, FLAT_CURVATURE + height * SAVED_AMOUNT, false);
            let loaded = colors(height, FLAT_CURVATURE, true);
            let largest_change = saved.iter().zip(&loaded).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
            assert!(largest_change < 1e-3, "height {height}: {largest_change}");
        }
    }

    #[test]
    fn fills_inside_folders_take_the_folders_scale_and_rotation_on_top_of_their_own() {
        // Nested folders (x2 turned 30, then x1.5 turned -10) around a procedural fill with a procedural mask fill
        let build = |outer: ContentsOffset, inner: ContentsOffset, projection: Projection| {
            let mut f = fixture();
            let folder = |f: &mut Fixture, contents: ContentsOffset| {
                let id = f.project.alloc_id();
                Layer::new(id, "G", LayerKind::Group { children: vec![], collapsed: false, contents, randomized: Vec::new() }, &[Channel::BaseColor])
            };
            let outer = folder(&mut f, outer);
            let outer = f.project.insert_layer(0, outer, None, false).unwrap();
            let inner = folder(&mut f, inner);
            let inner = f.project.insert_layer(0, inner, Some(outer), true).unwrap();
            let pattern = |seed| Source::Procedural {
                pattern: Procedural { seed, ..Procedural::new(ProceduralKind::Perlin) },
                low: [0.0; 3],
                high: [1.0; 3],
                ramp: Vec::new(),
            };
            let mut fill = Fill { projection: projection.clone(), ..Fill::default() };
            fill.sources.insert(Channel::BaseColor, pattern(3));
            let id = f.project.alloc_id();
            let mut layer = Layer::new(id, "C", LayerKind::Fill(fill), &[Channel::BaseColor]);
            let mut mask = Mask::new(MaskBackground::Black);
            mask.effects.push(Effect::new(f.project.alloc_id(), EffectKind::Fill { source: pattern(9), projection, sampling: Default::default() }));
            layer.mask = Some(mask);
            f.project.insert_layer(0, layer, Some(inner), true).unwrap();
            run(&f, None)
        };
        let own = Projection::Uv { scale: [2.0, 1.0], rotation: 15.0, offset: [0.1, 0.0] };
        let (outer, inner) = (ContentsOffset::new(2.0, 30.0), ContentsOffset::new(1.5, -10.0));
        let none = ContentsOffset::NONE;
        let offset = build(outer, inner, own.clone());
        let written = build(none, none, Projection::Uv { scale: [6.0, 3.0], rotation: 35.0, offset: [0.1, 0.0] });
        assert_eq!(offset, written, "the folders' offset renders as if it were written into the fills");
        assert_ne!(offset, build(none, none, own), "without the offset every fill keeps its own");
    }

    #[test]
    fn folder_mask_anchor_points_record_even_where_the_folder_is_masked_out() {
        let mut fixture = fixture();
        let group_id = fixture.project.alloc_id();
        let folder = Layer::new(
            group_id,
            "G",
            LayerKind::Group { children: vec![], collapsed: false, contents: Default::default(), randomized: Vec::new() },
            &[Channel::BaseColor, Channel::Roughness],
        );
        fixture.project.insert_layer(0, folder, None, false).unwrap();
        let child = fixture.project.alloc_id();
        let mut fill = Fill::default();
        fill.sources.insert(Channel::BaseColor, Source::Uniform([0.0; 3]));
        fixture.project.insert_layer(0, Layer::new(child, "C", LayerKind::Fill(fill), &[Channel::BaseColor]), Some(group_id), true).unwrap();
        let (gray, anchor, last) = (fixture.project.alloc_id(), fixture.project.alloc_id(), fixture.project.alloc_id());
        let uniform = |value: f32| EffectKind::Fill { source: Source::Uniform([value; 3]), projection: Projection::default(), sampling: Default::default() };
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects = vec![Effect::new(gray, uniform(0.4)), Effect::new(anchor, EffectKind::Anchor), Effect::new(last, uniform(0.0))];
        fixture.project.layer_mut(group_id).unwrap().mask = Some(mask);
        anchor_reader(&mut fixture, anchor);

        // The folder's mask ends at 0 (its black child hides), the reader above shows at 0.4 over the 0.5 base
        let color = run(&fixture, None).value(Channel::BaseColor, 5, 5);

        assert!(close(color, [0.7; 3]), "{color:?}");
    }

    /// A fixture under a decal fill of image "decal" framed on the plane's middle, its projection changed by `change`
    fn decal_fixture(change: impl FnOnce(&mut Fill)) -> Fixture {
        let mut fixture = fixture();
        let surface = fixture.gbuf.surface(&fixture.mesh, 32 * 64 + 32).unwrap();
        let frame = texturecraft_core::Decal {
            center: surface.pos.to_array(),
            normal: surface.nrm.to_array(),
            up: surface.nrm.any_orthonormal_vector().to_array(),
            size: [1.0, 1.0],
            rotation: 0.0,
            offset: [0.0; 2],
            depth: 0.5,
            symmetry: None,
            flip: [false; 2],
        };
        let id = fixture.project.alloc_id();
        let mut fill = Fill { projection: Projection::decal(frame), ..Fill::default() };
        fill.sources.insert(Channel::BaseColor, Source::image("decal"));
        change(&mut fill);
        fixture.project.insert_layer(0, Layer::new(id, "Decal", LayerKind::Fill(fill), &[Channel::BaseColor]), None, false).unwrap();
        fixture
    }

    /// `f` composited with `image` as its "decal" image
    fn composite_with_decal(f: &Fixture, image: crate::store::ImageRes) -> Composite {
        let images: ImageStore = [("decal".to_string(), std::sync::Arc::new(image))].into_iter().collect();
        run_with_images(f, None, &images)
    }

    /// Texels of a 64 x 64 composite whose base colour is not the fixture's grey
    fn decal_texels(out: &Composite) -> Vec<[f32; 3]> {
        (0..64 * 64).map(|i| out.value(Channel::BaseColor, i % 64, i / 64)).filter(|v| !close(*v, [0.5; 3])).collect()
    }

    #[test]
    fn decal_fills_cover_only_their_frame_and_image_alpha() {
        let fixture = decal_fixture(|_| {});
        let composite = |alpha: f32| composite_with_decal(&fixture, crate::store::ImageRes::new(2, 2, vec![[1.0, 0.0, 0.0, alpha]; 4], false));

        let opaque = composite(1.0);
        let half = composite(0.5);

        assert!(close(opaque.value(Channel::BaseColor, 32, 32), [1.0, 0.0, 0.0]), "{:?}", opaque.value(Channel::BaseColor, 32, 32));
        assert!(close(opaque.value(Channel::BaseColor, 2, 2), [0.5; 3]), "outside the frame");
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
        let mut fill = Fill { projection: Projection::decal(d), ..Fill::default() };
        fill.sources.insert(Channel::BaseColor, Source::image("decal"));
        project.insert_layer(0, Layer::new(id, "Decal", LayerKind::Fill(fill), &[Channel::BaseColor]), None, false).unwrap();

        // Top left red, top right blue, bottom left green, bottom right yellow
        let img = crate::store::ImageRes::new(2, 2, vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [0.0, 1.0, 0.0, 1.0], [1.0, 1.0, 0.0, 1.0]], false);
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

        // Offset the frame right so the texel samples the left half
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
        let mut fill = Fill { projection: Projection::decal(d), ..Fill::default() };
        fill.sources.insert(Channel::BaseColor, Source::image("decal"));
        f.project.insert_layer(0, Layer::new(id, "Decal", LayerKind::Fill(fill), &[Channel::BaseColor]), None, false).unwrap();

        // Left half red, right half blue
        let img = crate::store::ImageRes::new(2, 1, vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]], false);
        let images: ImageStore = [("decal".to_string(), std::sync::Arc::new(img))].into_iter().collect();
        let texel = |f: &Fixture, flip: [bool; 2]| {
            let mut project = f.project.clone();
            if let Ok(layer) = project.layer_mut(id)
                && let LayerKind::Fill(fill) = &mut layer.kind
            {
                fill.projection = Projection::decal(texturecraft_core::Decal { flip, ..d });
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
    fn a_cropped_frames_image_alpha_turns_with_its_image() {
        let f = decal_fixture(|fill| {
            if let Projection::Planar { uv, .. } = &mut fill.projection {
                uv.rotation = 180.0;
            }
        });

        // Opaque red on the left half, see-through green on the right
        let half = (0..8 * 8).map(|i| if i % 8 < 4 { [1.0, 0.0, 0.0, 1.0] } else { [0.0, 1.0, 0.0, 0.0] }).collect();
        let shown = decal_texels(&composite_with_decal(&f, crate::store::ImageRes::new(8, 8, half, false)));

        assert!(!shown.is_empty(), "the red half shows");
        assert!(shown.iter().all(|v| v[1] < 0.75), "the see-through green half stays hidden once turned (a seam texel blends some in): {shown:?}");
    }

    #[test]
    fn a_shrunk_cropped_frames_coverage_averages_its_image_alpha_as_its_colour_does() {
        let f = decal_fixture(|fill| fill.sampling.filtering = Filtering::BilinearHq);

        // Alternating alpha stripes exercise minification between source texels
        const SIZE: u32 = 1000;
        let stripes = (0..SIZE * SIZE).map(|i| [1.0, 0.0, 0.0, if i % 2 == 0 { 1.0 } else { 0.0 }]).collect();
        let shown = decal_texels(&composite_with_decal(&f, crate::store::ImageRes::new(SIZE, SIZE, stripes, false)));

        assert!(!shown.is_empty(), "the frame shows");

        let half = [0.75, 0.25, 0.25];
        assert!(
            shown.iter().all(|v| v.iter().zip(half).all(|(got, want)| (got - want).abs() < 0.05)),
            "every texel is half covered, as its averaged stripes are: {shown:?}"
        );
    }

    #[test]
    fn a_minified_srgb_image_averages_color_in_linear_light_and_data_as_stored() {
        let mut fixture = fixture();
        let mut fill = Fill::default();
        fill.sampling.filtering = Filtering::BilinearHq;
        fill.sources.insert(Channel::BaseColor, Source::image("stripes"));
        fill.sources.insert(Channel::Roughness, Source::image("stripes"));
        let id = fixture.project.alloc_id();
        let layer = Layer::new(id, "Stripes", LayerKind::Fill(fill), &[Channel::BaseColor, Channel::Roughness]);
        fixture.project.insert_layer(0, layer, None, false).unwrap();

        let bright = 0.8;
        let pixels = (0..256 * 256).map(|index| [if index % 2 == 0 { 0.0 } else { bright }; 4]).collect();
        let image = std::sync::Arc::new(ImageRes::new(256, 256, pixels, true));
        let images = ImageStore::from([("stripes".to_owned(), image)]);

        let composite = run_with_images(&fixture, None, &images);

        let color = composite.value(Channel::BaseColor, 32, 32);
        let roughness = composite.value(Channel::Roughness, 32, 32);

        assert!(close(roughness, [bright * 0.5; 3]), "roughness reads stored values: {roughness:?}");
        assert!(close(color, [texturecraft_texel::srgb_to_linear(bright) * 0.5; 3]), "color reads linear light: {color:?}");
    }

    #[test]
    fn high_tiling_still_minifies_a_continuous_uv_surface() {
        let mut fixture = fixture();
        const IMAGE_SIDE: u32 = 256;
        let projection = Projection::Uv { scale: [32.0; 2], rotation: 90.0, offset: [0.5 / IMAGE_SIDE as f32, 0.0] };
        let mut fill = Fill { projection, ..Fill::default() };
        fill.sampling.filtering = Filtering::BilinearHq;
        fill.sources.insert(Channel::BaseColor, Source::image("stripes"));
        let id = fixture.project.alloc_id();
        fixture.project.insert_layer(0, Layer::new(id, "Stripes", LayerKind::Fill(fill), &[Channel::BaseColor]), None, false).unwrap();
        let pixels = (0..IMAGE_SIDE * IMAGE_SIDE).map(|index| [(index % 2) as f32; 4]).collect();
        let image = Arc::new(ImageRes::new(IMAGE_SIDE, IMAGE_SIDE, pixels, false));
        let images = ImageStore::from([("stripes".to_owned(), image)]);

        let composite = run_with_images(&fixture, None, &images);

        for (x, y) in [(32, 32), (63, 63)] {
            let color = composite.value(Channel::BaseColor, x, y);

            assert!(close(color, [0.5; 3]), "stripes average at ({x}, {y}): {color:?}");
        }
    }

    #[test]
    fn hq_per_tile_filtering_uses_the_current_tiles_resolution_at_its_edges() {
        let mut fixture = fixture();
        let set = &mut fixture.project.texture_sets[0];
        set.uv_tiles = vec![1001, 1002];
        set.tile_resolutions.insert(1002, 32);
        let grid = set.tile_grid().unwrap();
        let atlas_size = set.size();
        let second = |uv: Vec2| uv + Vec2::X;
        let mut left = primitives::plane(2.0, 1);
        let mut right = left.clone();
        for position in &mut right.positions {
            position.x += 3.0;
        }

        left.map_material_uvs_with(0, |centroid, uv| Vec2::from(set.to_atlas(&grid, centroid.to_array(), uv.to_array())));
        right.map_material_uvs_with(0, |centroid, uv| Vec2::from(set.to_atlas(&grid, second(centroid).to_array(), second(uv).to_array())));
        left.append(&right);
        for submesh in &mut left.submeshes {
            submesh.material = 0;
        }

        fixture.mesh = left;
        fixture.gbuf = UvGBuffer::new(&fixture.mesh, 0, atlas_size.0, atlas_size.1);

        assert!(fixture.gbuf.covered((31 * atlas_size.0 + 95) as usize));

        let mut fill = Fill { projection: Projection::UvTile, ..Fill::default() };
        fill.sampling.filtering = Filtering::BilinearHq;
        fill.sources.insert(Channel::BaseColor, Source::image("stripes"));
        let id = fixture.project.alloc_id();
        fixture.project.insert_layer(0, Layer::new(id, "Stripes", LayerKind::Fill(fill), &[Channel::BaseColor]), None, false).unwrap();
        let pixels = (0..96).map(|index| [f32::from(index % 4 >= 2); 4]).collect();
        let image = Arc::new(ImageRes::new(96, 1, pixels, false));
        let images = ImageStore::from([("stripes".to_owned(), image)]);
        let mut composite = Composite::with_size(&[Channel::BaseColor], atlas_size.0, atlas_size.1);

        composite_tiles(&inputs(&fixture, &images, None), &mut composite, &all_tiles(atlas_size.0, atlas_size.1));

        let large_tile_edge = composite.value(Channel::BaseColor, 63, 31)[0];
        let small_tile_interior = composite.value(Channel::BaseColor, 92, 30)[0];
        let small_tile_edge = composite.value(Channel::BaseColor, 95, 31)[0];
        assert!((0.85..0.99).contains(&large_tile_edge), "the wrap at the next tile does not dominate the footprint: {large_tile_edge}");
        assert!((0.30..0.49).contains(&small_tile_interior), "the smaller tile minifies more strongly: {small_tile_interior}");
        assert!((0.51..0.70).contains(&small_tile_edge), "the last covered texel still uses its own tile: {small_tile_edge}");
    }

    #[test]
    fn a_cropped_frame_with_wrap_none_shows_one_image_however_it_tiles() {
        let opaque = || crate::store::ImageRes::new(2, 2, vec![[1.0, 0.0, 0.0, 1.0]; 4], false);
        let once = decal_texels(&composite_with_decal(&decal_fixture(|_| {}), opaque())).len();
        let tiled = decal_fixture(|fill| {
            fill.sampling.wrap = texturecraft_core::UvWrap::None;
            if let Projection::Planar { uv, .. } = &mut fill.projection {
                uv.tiling = [2.0, 2.0];
            }
        });

        let quarter = decal_texels(&composite_with_decal(&tiled, opaque())).len();

        assert!(once > 0, "the frame shows");
        assert!(quarter.abs_diff(once / 4) <= once / 16, "tiled twice with no wrap, one image fills a quarter of the frame: {quarter} of {once}");
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
                mask.effects.push(Effect::new(
                    f.project.alloc_id(),
                    EffectKind::Fill { source: Source::Uniform([gray; 3]), projection: Projection::default(), sampling: Default::default() },
                ));
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
}
