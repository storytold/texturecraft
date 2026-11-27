//! Layers, masks, effects and fill sources.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::curve_preset::CurvePreset;
use crate::gradient_input::{GradientInput, ramp, shape_gray};
use crate::{
    AnisotropicKuwahara, Backface, BakedMap, Bevel, BevelSmooth, BlendMode, Channel, ChannelKind, ColorMatch, CompareSource, ContentsOffset, DepthCulling,
    DirectionalDistance, DirectionalLight, GrayscaleConversion, HeightToNormal, HighpassSpace, MaskOutline, Mirror, PbrValidate, Projector, RandomizedSeed,
    ShapeCrop, SourceSlopeBlur, SourceWarp, StylizedLighting, SunAndSky, ThresholdMode, TileTransform, finite_unit, lum,
};

mod anchored_micro_detail;
mod axis_blend;
mod axis_layer;
mod border_distance_parameters;
mod curvature_detail;
mod curvature_mode;
mod decal_flip;
mod decal_symmetry;
mod distance_parameters;
mod dripping_rust_parameters;
mod filtering;
mod generator_global;
mod generator_grayscale;
mod grayscale_conversion_parameters;
mod kind_controls;
mod light_parameters;
mod linear_gradient_parameters;
mod micro_curvature_type;
mod micro_details;
mod output_resolution;
mod resolution_mode;
mod sampling;
mod scale_mode;
mod section_parameters;
mod texture_blend;
mod texture_layer;
mod tile_blending;
mod tile_generator;
mod tile_input_distribution;
mod tile_shape;
mod tile_size_mode;
mod tile_symmetry;
mod uv_checker_parameters;
mod uv_random_color_parameters;
mod uv_transform;
mod uv_wrap;
mod wear_parameters;

use anchored_micro_detail::AnchoredMicroDetail;
pub use axis_blend::AxisBlend;
pub use axis_layer::AxisLayer;
pub use border_distance_parameters::BorderDistanceParameters;
pub use curvature_detail::CurvatureDetail;
pub use curvature_mode::CurvatureMode;
pub use decal_flip::DecalFlip;
pub use decal_symmetry::DecalSymmetry;
pub use distance_parameters::DistanceParameters;
pub use dripping_rust_parameters::DrippingRustParameters;
pub use filtering::Filtering;
pub use generator_global::GeneratorGlobal;
pub use generator_grayscale::GeneratorGrayscale;
pub use grayscale_conversion_parameters::GrayscaleConversionParameters;
pub use kind_controls::KindControls;
pub use light_parameters::LightParameters;
pub use linear_gradient_parameters::LinearGradientParameters;
pub use micro_curvature_type::MicroCurvatureType;
pub use micro_details::MicroDetails;
pub use output_resolution::OutputResolution;
pub use resolution_mode::ResolutionMode;
pub use sampling::Sampling;
pub use scale_mode::ScaleMode;
pub use section_parameters::SectionParameters;
pub use texture_blend::TextureBlend;
pub use texture_layer::TextureLayer;
pub use tile_blending::TileBlending;
pub use tile_generator::TileGenerator;
pub use tile_input_distribution::TileInputDistribution;
pub use tile_shape::TileShape;
pub use tile_size_mode::TileSizeMode;
pub use tile_symmetry::TileSymmetry;
pub use uv_checker_parameters::UvCheckerParameters;
pub use uv_random_color_parameters::UvRandomColorParameters;
pub use uv_transform::UvTransform;
pub use uv_wrap::UvWrap;
pub use wear_parameters::WearParameters;

/// Id of a layer or effect (one id space per project).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LayerId(pub u64);

/// Blend mode and opacity of one channel of a layer. A channel missing from
/// [`Layer::channels`] is disabled for that layer.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelBlend {
    pub mode: BlendMode,
    pub opacity: f32,
}

impl Default for ChannelBlend {
    fn default() -> Self {
        ChannelBlend { mode: BlendMode::Normal, opacity: 1.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub kind: LayerKind,
    pub channels: BTreeMap<Channel, ChannelBlend>,
    #[serde(default)]
    pub mask: Option<Mask>,
    /// Effects applied to the layer's content, bottom → top.
    #[serde(default)]
    pub content_effects: Vec<Effect>,
    /// Color tag (0..8), shown as a stripe on the layer row.
    #[serde(default)]
    pub color_tag: Option<u8>,
    /// Combine the mask with the layer's height ("mask with height").
    #[serde(default)]
    pub height_mask: HeightMask,
    /// Geometry mask: the mesh parts (submesh names) the layer shows on; None shows it on all.
    #[serde(default)]
    pub geometry_mask: Option<Vec<String>>,
}

/// Mask with height: partial mask values follow the layer's own height, so a transition
/// reveals the high parts first. With mask `m` and the layer's height `h` (−1..1):
/// `m' = saturate(m + contrast · 4m(1 − m) · (h + offset))`. Fully hidden (0) and fully shown
/// (1) texels are unchanged, as is everything where the layer has no height.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HeightMask {
    pub enabled: bool,
    /// How strongly the height pushes the mask (0 = plain mask), 0..64.
    pub contrast: f32,
    /// Added to the height before it pushes the mask, −1..1.
    pub offset: f32,
}

impl Default for HeightMask {
    fn default() -> Self {
        HeightMask { enabled: false, contrast: 1.0, offset: 0.0 }
    }
}

impl HeightMask {
    /// Combines a mask value with height (see [`HeightMask`]).
    pub fn apply(&self, mask: f32, height: f32) -> f32 {
        if !self.enabled || !height.is_finite() {
            return mask;
        }

        // The height response peaks at the mask midpoint and vanishes at both ends
        (mask + self.contrast * 4.0 * mask * (1.0 - mask) * (height + self.offset)).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[expect(
    clippy::large_enum_variant,
    reason = "a fill holds its projection inline; boxing it would cost every fill read an indirection to save a few hundred bytes per paint layer or folder"
)]
pub enum LayerKind {
    /// Painted pixels, stored per channel by the engine under the layer id.
    Paint,
    /// A procedural or uniform material applied everywhere (shaped by the mask).
    Fill(Fill),
    /// A folder of layers, bottom → top.
    Group {
        children: Vec<LayerId>,
        collapsed: bool,
        /// Scale and Rotation laid on every fill inside (none in folders saved before folders had one)
        #[serde(default, skip_serializing_if = "ContentsOffset::is_none")]
        contents: ContentsOffset,
        /// The seeds inside that its Randomize seeds changed, until its Contents reset puts them back (none in folders saved before)
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        randomized: Vec<RandomizedSeed>,
    },
    /// A linked instance of `source` (possibly in another texture set): renders the source's
    /// content, content effects and channel blending, and its mask unless the instance has its
    /// own. Edits to the source show in every instance.
    Instance { source: LayerId },
}

impl LayerKind {
    /// A new folder: empty, open, with no Scale or Rotation laid on its contents
    pub fn folder() -> LayerKind {
        LayerKind::Group { children: Vec::new(), collapsed: false, contents: ContentsOffset::NONE, randomized: Vec::new() }
    }

    pub fn name(&self) -> &'static str {
        match self {
            LayerKind::Paint => "paint",
            LayerKind::Fill(_) => "fill",
            LayerKind::Group { .. } => "group",
            LayerKind::Instance { .. } => "instance",
        }
    }
}

impl Layer {
    /// A layer with every channel of `channels` enabled at Normal/100%.
    pub fn new(id: LayerId, name: impl Into<String>, kind: LayerKind, channels: &[Channel]) -> Layer {
        Layer {
            id,
            name: name.into(),
            visible: true,
            kind,
            channels: channels.iter().map(|c| (*c, ChannelBlend::default())).collect(),
            mask: None,
            content_effects: Vec::new(),
            color_tag: None,
            height_mask: HeightMask::default(),
            geometry_mask: None,
        }
    }

    pub fn is_group(&self) -> bool {
        matches!(self.kind, LayerKind::Group { .. })
    }

    pub fn children(&self) -> &[LayerId] {
        match &self.kind {
            LayerKind::Group { children, .. } => children,
            _ => &[],
        }
    }

    pub fn blend(&self, c: Channel) -> Option<ChannelBlend> {
        self.channels.get(&c).copied()
    }

    /// The layer this one instances, if it is an instance.
    pub fn instance_source(&self) -> Option<LayerId> {
        match self.kind {
            LayerKind::Instance { source } => Some(source),
            _ => None,
        }
    }
}

/// Where a texture or procedural is sampled from on the surface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Projection {
    /// Texture coordinates of the mesh.
    Uv { scale: [f32; 2], rotation: f32, offset: [f32; 2] },
    /// The source stretched once over each UV tile (0..1 in every tile)
    UvTile,
    /// World-space box projection blended by the normal.
    Triplanar {
        scale: f32,
        sharpness: f32,
        offset: [f32; 3],
        #[serde(default, skip_serializing_if = "UvTransform::is_default")]
        uv: UvTransform,
        #[serde(default, skip_serializing_if = "Projector::is_default")]
        projector: Projector,
    },
    /// World-space planar projection along an axis (0 = X, 1 = Y, 2 = Z), or from a frame placed on the surface
    Planar {
        scale: f32,
        axis: u8,
        offset: [f32; 3],
        #[serde(default, skip_serializing_if = "UvTransform::is_default")]
        uv: UvTransform,
        #[serde(default, skip_serializing_if = "Projector::is_default")]
        projector: Projector,
        /// The frame it projects from when placed on the surface (scale, axis and offset then unused)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placed: Option<Decal>,
    },
    /// Spherical projection around the mesh center
    Spherical {
        scale: f32,
        #[serde(default, skip_serializing_if = "UvTransform::is_default")]
        uv: UvTransform,
        #[serde(default, skip_serializing_if = "is_zero_offset")]
        offset: [f32; 3],
        #[serde(default, skip_serializing_if = "Projector::is_default")]
        projector: Projector,
    },
    /// Cylindrical projection around an axis
    Cylindrical {
        scale: f32,
        axis: u8,
        #[serde(default, skip_serializing_if = "UvTransform::is_default")]
        uv: UvTransform,
        #[serde(default, skip_serializing_if = "is_zero_offset")]
        offset: [f32; 3],
        #[serde(default, skip_serializing_if = "Projector::is_default")]
        projector: Projector,
        /// The arc one repeat of the source spans around the axis (degrees), centred on the front
        #[serde(default = "full_turn", skip_serializing_if = "is_full_turn")]
        angle: f32,
    },
    /// A planar projection from a frame whose four corners are pulled freely, bending the image to the quad they make
    Warp {
        /// The frame it projects from (its depth is unused: `depth` sets the reach)
        frame: Decal,
        /// Where the frame's corners sit (top left, top right, bottom right, bottom left), across and up in frame sizes from its centre
        corners: [[f32; 2]; 4],
        /// Projection Depth: how far in front of and behind the frame it reaches when cropped to its shape, in lengths of the frame's longer side
        depth: f32,
        uv: UvTransform,
        projector: Projector,
    },
}

/// The frame a planar projection placed on the surface (a decal) projects from, in world space
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decal {
    /// The surface point the decal was placed on.
    pub center: [f32; 3],
    /// The surface normal there: the decal projects along it.
    pub normal: [f32; 3],
    /// Up direction of the unrotated frame.
    pub up: [f32; 3],
    /// Width and height (world units).
    pub size: [f32; 2],
    /// Rotation around the normal (degrees).
    pub rotation: f32,
    /// Shift of the frame from `center` along the unrotated right and up directions (world units).
    pub offset: [f32; 2],
    /// How far in front of and behind the frame the decal reaches (world units).
    pub depth: f32,
    /// A mirrored copy of the decal on the other side of the mesh (None: no copy). The copy
    /// follows this frame, so moving, scaling or turning the decal moves it too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symmetry: Option<DecalSymmetry>,
    /// Mirror the image inside the frame: left to right (`[0]`), top to bottom (`[1]`). Old projects have neither.
    #[serde(default, skip_serializing_if = "not_flipped")]
    pub flip: [bool; 2],
}

/// Serde helper: a decal that mirrors nothing saves without the field
fn not_flipped(flip: &[bool; 2]) -> bool {
    !flip[0] && !flip[1]
}

/// Serde default of a cylindrical projection's angle: the whole way round
fn full_turn() -> f32 {
    crate::FULL_TURN_DEGREES
}

/// Whether a cylindrical projection spans the whole way round, as ones saved before the angle did (not saved then)
fn is_full_turn(angle: &f32) -> bool {
    *angle == full_turn()
}

/// Whether a spherical or cylindrical projection sits unshifted, as ones saved before the offset did (not saved then)
fn is_zero_offset(offset: &[f32; 3]) -> bool {
    *offset == [0.0; 3]
}

impl Default for Projection {
    fn default() -> Self {
        Projection::Uv { scale: [1.0, 1.0], rotation: 0.0, offset: [0.0, 0.0] }
    }
}

impl Projection {
    /// A warp's corners before any is pulled: the frame's own
    pub const FRAME_CORNERS: [[f32; 2]; 4] = [[-0.5, 0.5], [0.5, 0.5], [0.5, -0.5], [-0.5, -0.5]];

    /// A new warp's Projection Depth
    pub const WARP_DEPTH: f32 = 0.1;

    /// A new tri-planar projection's blend sharpness
    pub const TRIPLANAR_SHARPNESS: f32 = 8.0;

    /// The blend sharpness a tri-planar projection takes: an exponent from soft (1) to near hard (64)
    pub const TRIPLANAR_SHARPNESS_RANGE: std::ops::RangeInclusive<f32> = 1.0..=64.0;

    /// The arc in degrees a cylindrical projection wraps its image round; a zero arc would divide by zero
    pub const CYLINDER_ANGLE_RANGE: std::ops::RangeInclusive<f32> = 1.0..=crate::FULL_TURN_DEGREES;

    /// A tri-planar projection with a new fill's settings
    pub fn triplanar() -> Projection {
        Projection::Triplanar {
            scale: 1.0,
            sharpness: Self::TRIPLANAR_SHARPNESS,
            offset: [0.0; 3],
            uv: UvTransform::default(),
            projector: Projector::default(),
        }
    }

    /// A warp projection from `frame` with its corners where the frame's are
    pub fn warp(frame: Decal) -> Projection {
        Projection::Warp {
            frame,
            corners: Projection::FRAME_CORNERS,
            depth: Projection::WARP_DEPTH,
            uv: UvTransform::default(),
            projector: Projector::depth_culled(),
        }
    }

    /// A planar projection from `frame` with a decal's look: cropped to its frame, depth and image alpha, off faces turned away
    pub fn decal(frame: Decal) -> Projection {
        let projector = Projector { crop: ShapeCrop::Cropped, backface: Backface { on: true, ..Backface::default() }, ..Projector::default() };
        Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3], uv: UvTransform::default(), projector, placed: Some(frame) }
    }

    /// The depth culling of a planar or warp projection that has it on
    pub fn depth_culling(&self) -> Option<DepthCulling> {
        match self {
            Projection::Planar { projector, .. } | Projection::Warp { projector, .. } => Some(projector.depth).filter(|depth| depth.on),
            _ => None,
        }
    }

    /// Whether the projection works in 3D, so a fill's symmetry can copy it (UV projections cannot)
    pub fn takes_symmetry(&self) -> bool {
        !matches!(self, Projection::Uv { .. } | Projection::UvTile)
    }

    /// The frame on the surface a placed planar projection projects from
    pub fn placed(&self) -> Option<&Decal> {
        match self {
            Projection::Planar { placed, .. } => placed.as_ref(),
            _ => None,
        }
    }

    /// The frame on the surface a placed planar or a warp projection projects from
    pub fn frame(&self) -> Option<&Decal> {
        match self {
            Projection::Planar { placed, .. } => placed.as_ref(),
            Projection::Warp { frame, .. } => Some(frame),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Projection::UvTile => "Fill (Match per UV Tile)",
            Projection::Uv { .. } => "UV projection",
            Projection::Triplanar { .. } => "Tri-planar projection",
            Projection::Planar { .. } => "Planar projection",
            Projection::Spherical { .. } => "Spherical projection",
            Projection::Cylindrical { .. } => "Cylindrical projection",
            Projection::Warp { .. } => "Warp projection",
        }
    }
}

/// Procedural patterns available to fills, generators and masks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProceduralKind {
    /// Smooth fractal value noise.
    Clouds,
    /// Gradient (Perlin-style) noise.
    Perlin,
    /// Fine high-frequency noise.
    Noise,
    /// Worley / cellular noise.
    Cells,
    /// Blotchy grunge: thresholded multi-scale noise.
    Grunge,
    /// Thin random scratches.
    Scratches,
    /// Checkerboard.
    Checker,
    /// Stripes.
    Stripes,
    /// Linear gradient along U (or the projection axis).
    Gradient,
    /// Wood grain: growth rings with fibers along X.
    Wood,
    /// Running-bond bricks with mortar joints (0 = mortar).
    Bricks,
    /// Square tiles with grout joints (0 = grout).
    Tiles,
    /// Plain fabric weave.
    Weave,
    /// 2/2 twill: diagonal ribs, the warp brighter than the weft (denim, carbon fiber).
    Twill,
    /// Leather grain: pebbled cells with creases.
    Leather,
    /// Hammered metal dimples.
    Hammered,
    /// Fine brushed-metal lines along X.
    Brushed,
    /// Marble veins.
    Marble,
    /// Flat random tone per Voronoi cell (crystals, spangle, stones).
    Voronoi,
    /// Vertical streaks and leaks (along the projection's up axis).
    Streaks,
    /// Fingerprints and smudges.
    Smudges,
    /// Small round pores and pits (0 = pore).
    Pores,
    /// Sharp-edged paint chips.
    Chips,
    /// Hexagon tiles with joints (0 = joint).
    HexagonTiles,
    /// Rows of overlapping fan-shaped waves, each a set of concentric arcs.
    WaveScales,
    /// Herringbone planks with gaps and grain (0 = gap).
    HerringbonePlanks,
    /// A halftone screen: dots on a diagonal grid, growing and shrinking across the pattern.
    HalftoneDots,
    /// Raised diamonds of concentric rings on a twill ground.
    DiamondBrocade,
    /// Truchet tiles: quarter-circle arcs that join into winding paths.
    TruchetArcs,
    /// Long smooth locks of fur, shaded like a height map.
    SmoothFur,
    /// Crowded soft cloudlets over a dark sky.
    MackerelSky,
    /// Warped clouds crossed by fine strands.
    Billow,
    /// Overlapping reptile scales, domed and lit from above.
    ReptileScales,
    /// Young bark: tall plates parted by wandering furrows (0 = furrow).
    Bark,
    /// Rotting flesh: soft lobes, veins and patches of decay.
    Flesh,
    /// Four flat tones of interlocking blotches, for camouflage.
    Camouflage,
    /// Stockinette knit: V-shaped stitches of twisted yarn.
    Knit,
    /// A basket weave: threads in pairs, alternately across and down.
    BasketWeave,
    /// A goose-eye twill: diagonal ribs that turn back into small diamonds.
    GooseEye,
    /// Raw silk with slubs: threads that swell here and there.
    SlubSilk,
    /// Strip flooring: long boards with staggered butt joints (0 = gap).
    StripFlooring,
    /// Wood grain that bends around knots.
    KnottyWood,
    /// Shapes laid out in a grid of tiles, with the settings in [`Procedural::tile`]
    TileGenerator,
}

impl ProceduralKind {
    pub const ALL: [ProceduralKind; 43] = [
        ProceduralKind::Clouds,
        ProceduralKind::Perlin,
        ProceduralKind::Noise,
        ProceduralKind::Cells,
        ProceduralKind::Grunge,
        ProceduralKind::Scratches,
        ProceduralKind::Checker,
        ProceduralKind::Stripes,
        ProceduralKind::Gradient,
        ProceduralKind::Wood,
        ProceduralKind::Bricks,
        ProceduralKind::Tiles,
        ProceduralKind::Weave,
        ProceduralKind::Twill,
        ProceduralKind::Leather,
        ProceduralKind::Hammered,
        ProceduralKind::Brushed,
        ProceduralKind::Marble,
        ProceduralKind::Voronoi,
        ProceduralKind::Streaks,
        ProceduralKind::Smudges,
        ProceduralKind::Pores,
        ProceduralKind::Chips,
        ProceduralKind::HexagonTiles,
        ProceduralKind::WaveScales,
        ProceduralKind::HerringbonePlanks,
        ProceduralKind::HalftoneDots,
        ProceduralKind::DiamondBrocade,
        ProceduralKind::TruchetArcs,
        ProceduralKind::SmoothFur,
        ProceduralKind::MackerelSky,
        ProceduralKind::Billow,
        ProceduralKind::ReptileScales,
        ProceduralKind::Bark,
        ProceduralKind::Flesh,
        ProceduralKind::Camouflage,
        ProceduralKind::Knit,
        ProceduralKind::BasketWeave,
        ProceduralKind::GooseEye,
        ProceduralKind::SlubSilk,
        ProceduralKind::StripFlooring,
        ProceduralKind::KnottyWood,
        ProceduralKind::TileGenerator,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ProceduralKind::Clouds => "Clouds",
            ProceduralKind::Perlin => "Perlin noise",
            ProceduralKind::Noise => "Fine noise",
            ProceduralKind::Cells => "Cells",
            ProceduralKind::Grunge => "Grunge",
            ProceduralKind::Scratches => "Scratches",
            ProceduralKind::Checker => "Checker",
            ProceduralKind::Stripes => "Stripes",
            ProceduralKind::Gradient => "Gradient",
            ProceduralKind::Wood => "Wood grain",
            ProceduralKind::Bricks => "Bricks",
            ProceduralKind::Tiles => "Tiles",
            ProceduralKind::Weave => "Fabric weave",
            ProceduralKind::Twill => "Twill weave",
            ProceduralKind::Leather => "Leather grain",
            ProceduralKind::Hammered => "Hammered",
            ProceduralKind::Brushed => "Brushed lines",
            ProceduralKind::Marble => "Marble veins",
            ProceduralKind::Voronoi => "Voronoi",
            ProceduralKind::Streaks => "Streaks",
            ProceduralKind::Smudges => "Fingerprints",
            ProceduralKind::Pores => "Pores",
            ProceduralKind::Chips => "Paint chips",
            ProceduralKind::HexagonTiles => "Hexagon tiles",
            ProceduralKind::WaveScales => "Wave scales",
            ProceduralKind::HerringbonePlanks => "Herringbone planks",
            ProceduralKind::HalftoneDots => "Halftone dots",
            ProceduralKind::DiamondBrocade => "Diamond brocade",
            ProceduralKind::TruchetArcs => "Truchet arcs",
            ProceduralKind::SmoothFur => "Smooth fur",
            ProceduralKind::MackerelSky => "Mackerel sky",
            ProceduralKind::Billow => "Soft billow",
            ProceduralKind::ReptileScales => "Reptile scales",
            ProceduralKind::Bark => "Young bark",
            ProceduralKind::Flesh => "Rotting flesh",
            ProceduralKind::Camouflage => "Camouflage",
            ProceduralKind::Knit => "Knit stitches",
            ProceduralKind::BasketWeave => "Basket weave",
            ProceduralKind::GooseEye => "Goose-eye twill",
            ProceduralKind::SlubSilk => "Slub silk",
            ProceduralKind::StripFlooring => "Strip flooring",
            ProceduralKind::KnottyWood => "Knotty wood",
            ProceduralKind::TileGenerator => "Tile Generator",
        }
    }

    pub fn parse(name: &str) -> Option<ProceduralKind> {
        let normalize = |text: &str| text.to_ascii_lowercase().replace([' ', '_', '-'], "");
        let name = normalize(name);

        ProceduralKind::ALL.into_iter().find(|kind| normalize(kind.label()) == name || normalize(&format!("{kind:?}")) == name)
    }
}

/// A procedural grayscale pattern. Its output maps 0..1 between `low` and `high` values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Procedural {
    pub kind: ProceduralKind,
    /// Pattern frequency (repeats per projection unit).
    pub scale: f32,
    pub seed: u32,
    /// Contrast around 0.5 (1 = unchanged).
    pub contrast: f32,
    /// Shifts the pattern brighter (+) or darker (−).
    pub balance: f32,
    pub invert: bool,
    /// The Tile Generator's settings; `None` for other kinds (and reads as the defaults); loaded in range, as drawing trusts them
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "tile_in_range")]
    pub tile: Option<Box<TileGenerator>>,
    /// The resolution the pattern is computed at before it is sampled onto the texture set
    #[serde(default, skip_serializing_if = "OutputResolution::is_default")]
    pub resolution: OutputResolution,
}

/// Saved Tile Generator settings brought into range (a saved project is hostile input)
fn tile_in_range<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Box<TileGenerator>>, D::Error> {
    let saved: Option<Box<TileGenerator>> = Deserialize::deserialize(deserializer)?;
    Ok(saved.map(|tile| Box::new(tile.clamped())))
}

impl Procedural {
    /// The scales a command accepts (repeats per projection unit)
    pub const MIN_SCALE: f32 = 0.001;
    pub const MAX_SCALE: f32 = 1000.0;
    /// The strongest contrast a command accepts, and the furthest balance either way
    pub const MAX_CONTRAST: f32 = 20.0;
    pub const MAX_BALANCE: f32 = 1.0;

    pub fn new(kind: ProceduralKind) -> Procedural {
        let tile = (kind == ProceduralKind::TileGenerator).then(Box::default);

        // The Tile Generator lays its whole grid over one projection unit, as its tile counts set its frequency
        let scale = if tile.is_some() { 1.0 } else { 4.0 };
        Procedural { kind, scale, seed: 1, contrast: 1.0, balance: 0.0, invert: false, tile, resolution: OutputResolution::default() }
    }
}

/// What a fill writes into one channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Source {
    /// A constant value (linear RGB for colors; `[v, v, v]` for scalars).
    Uniform([f32; 3]),
    /// A procedural pattern mixing two values, or on a color channel colored by `ramp` when it
    /// has stops (a gradient map: dark to light through evenly spaced linear RGB colors).
    Procedural {
        pattern: Procedural,
        low: [f32; 3],
        high: [f32; 3],
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ramp: Vec<[f32; 3]>,
    },
    /// An imported image resource, by name. `normal_format` is the convention of a normal map
    /// image (DirectX images have their green flipped where a normal channel samples them).
    Image {
        name: String,
        #[serde(default)]
        normal_format: crate::NormalFormat,
    },
    /// The stack value recorded by an anchor point effect (`anchor` is the effect id) for
    /// `channel`; colors read as gray where a scalar is needed. On a color channel a `ramp` with
    /// stops recolors the value by its lightness (a gradient map).
    Anchor {
        anchor: LayerId,
        channel: Channel,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ramp: Vec<[f32; 3]>,
    },
}

impl Source {
    pub fn uniform_of(c: Channel) -> Source {
        Source::Uniform(c.default_value())
    }

    /// Image resource `name` (an OpenGL normal map where a normal channel samples it).
    pub fn image(name: impl Into<String>) -> Source {
        Source::Image { name: name.into(), normal_format: crate::NormalFormat::OpenGl }
    }

    /// The image resources this source reads: its image, or the images of a Tile Generator pattern
    pub fn images(&self) -> Vec<&str> {
        match self {
            Source::Image { name, .. } => vec![name.as_str()],
            Source::Procedural { pattern, .. } => pattern.tile.iter().flat_map(|tile| tile.images()).collect(),
            Source::Uniform(_) | Source::Anchor { .. } => Vec::new(),
        }
    }

    /// The gradient map stops of a procedural or anchor source (empty: none).
    pub fn ramp(&self) -> &[[f32; 3]] {
        match self {
            Source::Procedural { ramp, .. } | Source::Anchor { ramp, .. } => ramp,
            _ => &[],
        }
    }

    /// The anchor point this source reads, if any.
    pub fn anchor(&self) -> Option<LayerId> {
        match self {
            Source::Anchor { anchor, .. } => Some(*anchor),
            _ => None,
        }
    }

    /// Whether this source reads anchor point `anchor` (any anchor point when None)
    pub fn reads_anchor(&self, anchor: Option<LayerId>) -> bool {
        self.anchor().is_some_and(|found| anchor.is_none_or(|wanted| wanted == found))
    }
}

/// A fill layer's material: one source per channel, plus how it is projected.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Fill {
    pub sources: BTreeMap<Channel, Source>,
    #[serde(serialize_with = "crate::saved_projection::save", deserialize_with = "crate::saved_projection::load")]
    pub projection: Projection,
    #[serde(default = "Sampling::saved_before", skip_serializing_if = "Sampling::is_saved_before")]
    pub sampling: Sampling,
    /// Set when the sources are maps cooked from a MaterialCraft material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Box<MaterialLink>>,
    /// The built-in material (shelf id) the sources were set from as one whole material; cleared when a channel's source changes on its own
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
}

/// A MaterialCraft material behind a fill (from a `.mcmat` portable material): its channel
/// sources are image resources cooked from the graph, and changing a parameter re-cooks them
/// through `materialcraft-cli`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaterialLink {
    pub name: String,
    /// The image resource holding the material's package (`.mcraft`).
    pub package: String,
    /// The package's graph the maps come from.
    pub graph: String,
    pub params: Vec<LinkParam>,
    #[serde(default)]
    pub presets: Vec<LinkPreset>,
    /// Image inputs of the graph (identifiers such as `curvature`), fed from baked mesh maps.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Size the maps were cooked at (pixels across).
    pub size: u32,
    /// Size to cook at; `None` cooks at the texture set's resolution
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<crate::CookResolution>,
    /// The output a channel reads, where it is not the output named like the channel
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mapping: BTreeMap<Channel, String>,
    /// The image resource of each output of the last cook, by output id. Empty in projects saved before channels could be mapped: there each channel's source is its own output
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub outputs: BTreeMap<String, String>,
}

impl MaterialLink {
    /// The output channel `c` reads
    pub fn output_of(&self, c: Channel) -> &str {
        self.mapping.get(&c).map_or(c.id(), String::as_str)
    }
}

/// An exposed input of a linked material. Values keep MaterialCraft's typed form,
/// `{"t": "float" | "float2" | "float3" | "float4" | "int" | "int2" | "bool" | "color" | "enum", "v": …}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkParam {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub group: String,
    pub value: serde_json::Value,
    pub default: serde_json::Value,
    #[serde(default)]
    pub min: f32,
    #[serde(default = "one")]
    pub max: f32,
    /// Labels of an `enum` value.
    #[serde(default)]
    pub options: Vec<String>,
}

fn one() -> f32 {
    1.0
}

/// A named set of linked-material values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkPreset {
    pub name: String,
    pub values: BTreeMap<String, serde_json::Value>,
}

impl Fill {
    /// Value used for `c` when the fill has no source for it: the channel default.
    pub fn source(&self, c: Channel) -> Source {
        self.sources.get(&c).cloned().unwrap_or_else(|| Source::uniform_of(c))
    }

    /// The image resource of each output of the linked material's last cook, by output id (empty without a link). Links saved before channels could be mapped list their channels' own outputs
    pub fn link_outputs(&self) -> BTreeMap<String, String> {
        let Some(link) = &self.link else { return BTreeMap::new() };
        if !link.outputs.is_empty() {
            return link.outputs.clone();
        }

        self.sources
            .iter()
            .filter_map(|(c, s)| match s {
                Source::Image { name, .. } => Some((link.output_of(*c).to_string(), name.clone())),
                _ => None,
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MaskBackground {
    #[default]
    White,
    Black,
}

/// A layer mask: a background color with a stack of effects on top.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    pub enabled: bool,
    pub background: MaskBackground,
    /// Effects bottom → top. Paint effects hold bitmap data keyed by their id.
    pub effects: Vec<Effect>,
}

impl Mask {
    pub fn new(background: MaskBackground) -> Mask {
        Mask { enabled: true, background, effects: Vec::new() }
    }
}

/// Levels remap: input range, gamma, output range.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Levels {
    pub in_low: f32,
    pub in_high: f32,
    pub gamma: f32,
    pub out_low: f32,
    pub out_high: f32,
    /// The one channel a content levels remaps; none remaps every channel (as levels saved before the choice did) and a mask's one value
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affects: Option<Channel>,
    /// Values past the input handles stop at the output ends; off, the remap's line carries on past them (older saves load clamped)
    #[serde(default = "saved_levels_clamp", skip_serializing_if = "is_true")]
    pub clamp: bool,
}

/// What levels saved before [`Levels::clamp`] existed did
fn saved_levels_clamp() -> bool {
    true
}

impl Default for Levels {
    fn default() -> Self {
        Levels { in_low: 0.0, in_high: 1.0, gamma: 1.0, out_low: 0.0, out_high: 1.0, affects: None, clamp: false }
    }
}

impl Levels {
    /// The gamma a levels effect takes, typed or dragged
    pub const GAMMA_RANGE: std::ops::RangeInclusive<f32> = 0.01..=10.0;

    /// Narrowest input range the remap divides by, so handles on one value never divide by zero
    const MIN_INPUT_RANGE: f32 = 1e-5;

    /// Smallest gamma the remap raises by, so a zero gamma read from a file never divides by zero
    const MIN_GAMMA: f32 = 1e-3;

    pub fn apply(&self, x: f32) -> f32 {
        let range = self.in_high - self.in_low;
        let between = (x - self.in_low) / if range > 0.0 { range } else { Self::MIN_INPUT_RANGE };

        // Past the input handles an unclamped remap carries on in a straight line (gamma only bends the part between them)
        let between = if self.clamp || (0.0..=1.0).contains(&between) { between.clamp(0.0, 1.0).powf(1.0 / self.gamma.max(Self::MIN_GAMMA)) } else { between };
        self.out_low + (self.out_high - self.out_low) * between
    }

    /// Whether these levels remap `channel` of a layer's content
    pub fn remaps(&self, channel: Channel) -> bool {
        self.affects.is_none_or(|affects| affects == channel)
    }

    /// The handles back at their defaults, keeping the channel and the clamp
    pub fn reset(&self) -> Levels {
        Levels { affects: self.affects, clamp: self.clamp, ..Levels::default() }
    }
}

/// Mesh-map driven mask generators (the "smart mask" building blocks).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GeneratorKind {
    /// Convex edges from the curvature map.
    EdgeWear,
    /// Crevices from ambient occlusion.
    Dirt,
    /// Thin parts from the thickness map.
    ThinAreas,
    /// Raw curvature.
    Curvature,
    /// Raw ambient occlusion.
    AmbientOcclusion,
    /// Gradient along a world axis (bottom → top by default).
    Position,
    /// Surfaces facing a direction (e.g. up for dust).
    Direction,
    /// Worn edges of a fibreglass-like coating: the edges fade into its grunge
    FiberGlassEdgeWear,
    /// The world space normal's axes combined into one gray
    WorldSpaceNormals,
    /// White around a point across the mesh's bounds, fading with the distance from it
    ThreeDDistance,
    /// A gradient from a start point to an end point across the mesh's bounds
    ThreeDLinearGradient,
    /// White at the UV islands' borders, fading towards their insides
    UvBorderDistance,
    /// An image turned gray, tiled and rotated across the UV square
    GrayscaleConversion,
    /// Rust in the occluded areas, running down the mesh in drips
    DrippingRust,
    /// One random value per UV island
    UvRandomColor,
    /// A checker and grid over each UV tile with the UV islands' borders drawn in, for checking a layout
    UvChecker,
}

impl GeneratorKind {
    pub const ALL: [GeneratorKind; 16] = [
        GeneratorKind::EdgeWear,
        GeneratorKind::Dirt,
        GeneratorKind::ThinAreas,
        GeneratorKind::Curvature,
        GeneratorKind::AmbientOcclusion,
        GeneratorKind::Position,
        GeneratorKind::Direction,
        GeneratorKind::FiberGlassEdgeWear,
        GeneratorKind::WorldSpaceNormals,
        GeneratorKind::ThreeDDistance,
        GeneratorKind::ThreeDLinearGradient,
        GeneratorKind::UvBorderDistance,
        GeneratorKind::GrayscaleConversion,
        GeneratorKind::DrippingRust,
        GeneratorKind::UvRandomColor,
        GeneratorKind::UvChecker,
    ];
    pub fn label(self) -> &'static str {
        match self {
            GeneratorKind::EdgeWear => "Worn Metal Edges",
            GeneratorKind::Dirt => "Dirt",
            GeneratorKind::ThinAreas => "Thin areas",
            GeneratorKind::Curvature => "Curvature",
            GeneratorKind::AmbientOcclusion => "Ambient Occlusion",
            GeneratorKind::Position => "Position",
            GeneratorKind::Direction => "Light",
            GeneratorKind::FiberGlassEdgeWear => "Exposed Glass Fibers",
            GeneratorKind::WorldSpaceNormals => "World Space Normals",
            GeneratorKind::ThreeDDistance => "3D Distance",
            GeneratorKind::ThreeDLinearGradient => "3D Linear Gradient",
            GeneratorKind::UvBorderDistance => "UV Border Distance",
            GeneratorKind::GrayscaleConversion => "Grayscale Conversion",
            GeneratorKind::DrippingRust => "Rust Runoff",
            GeneratorKind::UvRandomColor => "UV Random Color",
            GeneratorKind::UvChecker => "UV Checker",
        }
    }

    /// The name commands and saved projects use for this kind
    pub fn id(self) -> &'static str {
        match self {
            GeneratorKind::EdgeWear => "EdgeWear",
            GeneratorKind::Dirt => "Dirt",
            GeneratorKind::ThinAreas => "ThinAreas",
            GeneratorKind::Curvature => "Curvature",
            GeneratorKind::AmbientOcclusion => "AmbientOcclusion",
            GeneratorKind::Position => "Position",
            GeneratorKind::Direction => "Direction",
            GeneratorKind::FiberGlassEdgeWear => "FiberGlassEdgeWear",
            GeneratorKind::WorldSpaceNormals => "WorldSpaceNormals",
            GeneratorKind::ThreeDDistance => "ThreeDDistance",
            GeneratorKind::ThreeDLinearGradient => "ThreeDLinearGradient",
            GeneratorKind::UvBorderDistance => "UvBorderDistance",
            GeneratorKind::GrayscaleConversion => "GrayscaleConversion",
            GeneratorKind::DrippingRust => "DrippingRust",
            GeneratorKind::UvRandomColor => "UvRandomColor",
            GeneratorKind::UvChecker => "UvChecker",
        }
    }

    /// Whether the generator picker offers this kind (Thin areas only loads from saved projects and smart masks)
    pub fn in_picker(self) -> bool {
        self != GeneratorKind::ThinAreas
    }

    /// Whether the kind's result depends on its seed
    pub fn includes_randomness(self) -> bool {
        matches!(
            self,
            GeneratorKind::EdgeWear
                | GeneratorKind::Dirt
                | GeneratorKind::FiberGlassEdgeWear
                | GeneratorKind::DrippingRust
                | GeneratorKind::UvRandomColor
                | GeneratorKind::UvChecker
        )
    }

    /// The mesh maps this kind reads, each an input that an image, a uniform value or an anchor can replace.
    pub fn inputs(self) -> &'static [BakedMap] {
        match self {
            GeneratorKind::EdgeWear | GeneratorKind::Dirt | GeneratorKind::FiberGlassEdgeWear => &[BakedMap::Curvature, BakedMap::AmbientOcclusion],
            GeneratorKind::Curvature => &[BakedMap::Curvature],
            GeneratorKind::AmbientOcclusion => &[BakedMap::AmbientOcclusion],
            GeneratorKind::ThinAreas => &[BakedMap::Thickness],
            GeneratorKind::Position | GeneratorKind::ThreeDDistance | GeneratorKind::ThreeDLinearGradient => &[BakedMap::Position],
            GeneratorKind::WorldSpaceNormals => &[BakedMap::WorldNormal],
            GeneratorKind::Direction => &[BakedMap::WorldNormal, BakedMap::Position],
            // Dripping Rust reads the baked occlusion and the mesh's bounds whole, so no input replaces them yet
            GeneratorKind::UvBorderDistance
            | GeneratorKind::GrayscaleConversion
            | GeneratorKind::DrippingRust
            | GeneratorKind::UvRandomColor
            | GeneratorKind::UvChecker => &[],
        }
    }

    /// Whether the kind was added with the reference's controls only, so it never computes with level, softness and noise
    pub fn has_no_earlier_model(self) -> bool {
        matches!(
            self,
            GeneratorKind::FiberGlassEdgeWear
                | GeneratorKind::ThreeDDistance
                | GeneratorKind::ThreeDLinearGradient
                | GeneratorKind::UvBorderDistance
                | GeneratorKind::GrayscaleConversion
                | GeneratorKind::DrippingRust
                | GeneratorKind::UvRandomColor
                | GeneratorKind::UvChecker
        )
    }

    pub fn parse(name: &str) -> Option<GeneratorKind> {
        let normalised = |text: &str| text.to_ascii_lowercase().replace([' ', '_', '-', ':'], "");
        let wanted = normalised(name);

        GeneratorKind::ALL.into_iter().find(|kind| {
            normalised(kind.id()) == wanted || normalised(kind.label()) == wanted || kind.former_labels().iter().any(|label| normalised(label) == wanted)
        })
    }

    /// Earlier names accepted by commands and picker search
    pub fn former_labels(self) -> &'static [&'static str] {
        match self {
            GeneratorKind::EdgeWear => &["Mask Editor: Edge wear", "Metal Edge Wear"],
            GeneratorKind::Position => &["Position gradient"],
            GeneratorKind::Direction => &["Light / direction"],
            GeneratorKind::FiberGlassEdgeWear => &["Fiber Glass Edge Wear"],
            GeneratorKind::DrippingRust => &["Dripping Rust"],
            _ => &[],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(remote = "Self")]
pub struct Generator {
    pub kind: GeneratorKind,
    /// Where the mask starts (0..1, generator specific threshold).
    pub level: f32,
    /// Edge softness (0 = hard).
    pub softness: f32,
    /// Breaks the result up with world-space noise (0 = none).
    pub noise_amount: f32,
    pub noise_scale: f32,
    pub seed: u32,
    /// Added to the finished mask value and clamped to 0..1 (−1..1; 0 = unchanged).
    #[serde(default)]
    pub offset: f32,
    /// Direction for `Position` / `Direction` generators (world space).
    pub direction: [f32; 3],
    pub invert: bool,
    /// Mode and detail sizes of `Curvature` / `EdgeWear` generators (None: the curvature map as baked, as before detail sizes existed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<CurvatureDetail>,
    /// Blur, balance and contrast of the finished value (neutral for generators saved before they existed).
    #[serde(default, skip_serializing_if = "GeneratorGlobal::is_neutral")]
    pub global: GeneratorGlobal,
    /// The resolution the generator is computed at (the texture set's for generators saved before it existed).
    #[serde(default, skip_serializing_if = "OutputResolution::is_default")]
    pub resolution: OutputResolution,
    /// What an input reads instead of its mesh map (an image, a uniform value or an anchor); inputs not listed read their mesh map.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<BakedMap, Source>,
    /// The wear kinds' controls (None: the level, softness and noise above, as saved before they existed)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wear: Option<WearParameters>,
    /// The section controls of Curvature, Ambient Occlusion, Position and World Space Normals (None: the level, softness and noise above, as saved before they existed)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<SectionParameters>,
    /// The Light generator's controls (None: the level, softness and noise above, as saved before they existed)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<LightParameters>,
    /// The texture blended over the signal on the section controls (None: Use Texture never touched, as saved before it existed); boxed, so a generator effect stays small
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<Box<TextureLayer>>,
    /// The image a wear generator reads as its grunge once Use Custom Grunge is on (None: an empty slot)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grunge: Option<Source>,
    /// The micro details of Curvature, Ambient Occlusion and the wear kinds (None: never touched, their defaults: off); boxed, so a generator effect stays small
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub micro: Option<Box<MicroDetails>>,
    /// The controls of 3D Distance, 3D Linear Gradient, UV Border Distance or Grayscale Conversion (None on the other kinds); boxed, so a generator effect stays small
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<Box<KindControls>>,
}

impl Serialize for Generator {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Generator::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Generator {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Generator, D::Error> {
        let mut saved = serde_json::Value::deserialize(deserializer)?;

        // Micro detail saved before Micro Details: an anchor point's height and its amount
        let anchored = saved.get_mut("micro").filter(|micro| micro.get("anchor").is_some()).map(serde_json::Value::take);
        let mut generator = Generator::deserialize(saved).map_err(serde::de::Error::custom)?;
        if let Some(anchored) = anchored {
            let AnchoredMicroDetail { anchor, amount } = serde_json::from_value(anchored).map_err(serde::de::Error::custom)?;
            generator.micro = MicroDetails::of_anchor(generator.kind, anchor, amount).map(Box::new);
        }
        Ok(generator)
    }
}

impl Generator {
    /// The softest a ramp gets, so its threshold never divides by zero.
    pub const MIN_SOFTNESS: f32 = 1e-3;
    /// The most noise a generator mixes in, and the range of its noise scale.
    pub const MAX_NOISE_AMOUNT: f32 = 2.0;
    pub const MIN_NOISE_SCALE: f32 = 0.01;
    pub const MAX_NOISE_SCALE: f32 = 1000.0;

    /// The global blur, balance and contrast the generator applies: none on the wear and light controls, which have no such controls
    pub fn applied_global(&self) -> GeneratorGlobal {
        if self.wear.is_some() || self.light.is_some() || self.kind.has_no_earlier_model() { GeneratorGlobal::default() } else { self.global }
    }

    /// The section controls the generator computes with: its own, World Space Normals' defaults when it has none (it has no earlier model), else None
    pub fn section_controls(&self) -> Option<SectionParameters> {
        self.section.or_else(|| SectionParameters::defaults(self.kind).filter(|_| self.kind == GeneratorKind::WorldSpaceNormals))
    }

    /// The 3D Distance controls the generator computes with: its own, else the defaults on a 3D Distance generator
    pub fn distance_controls(&self) -> Option<DistanceParameters> {
        match self.kind_controls() {
            Some(KindControls::Distance(distance)) => Some(distance),
            _ => None,
        }
    }

    /// The 3D Linear Gradient controls the generator computes with: its own, else the defaults on a 3D Linear Gradient generator
    pub fn linear_gradient_controls(&self) -> Option<LinearGradientParameters> {
        match self.kind_controls() {
            Some(KindControls::LinearGradient(gradient)) => Some(gradient),
            _ => None,
        }
    }

    /// The UV Border Distance controls the generator computes with: its own, else the defaults on a UV Border Distance generator
    pub fn border_distance_controls(&self) -> Option<BorderDistanceParameters> {
        match self.kind_controls() {
            Some(KindControls::BorderDistance(border)) => Some(border),
            _ => None,
        }
    }

    /// The Grayscale Conversion controls the generator computes with: its own, else the defaults on a Grayscale Conversion generator
    pub fn grayscale_controls(&self) -> Option<GrayscaleConversionParameters> {
        match self.kind_controls() {
            Some(KindControls::Grayscale(grayscale)) => Some(grayscale),
            _ => None,
        }
    }

    /// The controls of a kind with only its own controls: its saved ones (brought into range) when they are that kind's, else its defaults
    pub fn kind_controls(&self) -> Option<KindControls> {
        let saved = self.controls.as_deref().filter(|controls| controls.kind() == self.kind).cloned().map(KindControls::clamped);
        saved.or_else(|| KindControls::defaults(self.kind))
    }

    /// The Grayscale Conversion controls as saved, without a copy
    fn grayscale_controls_ref(&self) -> Option<&GrayscaleConversionParameters> {
        match self.controls.as_deref() {
            Some(KindControls::Grayscale(grayscale)) if self.kind == GeneratorKind::GrayscaleConversion => Some(grayscale),
            _ => None,
        }
    }

    /// The mode and detail sizes the generator reads its curvature with: its own, the defaults on Curvature's section controls, else None
    pub fn detail_in_use(&self) -> Option<CurvatureDetail> {
        let on_section = self.kind == GeneratorKind::Curvature && self.section_controls().is_some();
        self.detail.clone().or_else(|| on_section.then(CurvatureDetail::default))
    }

    /// The mesh maps the generator reads: none for Position and Light on the earlier model, which read the mesh itself
    pub fn inputs_in_use(&self) -> &'static [BakedMap] {
        let reads_geometry = self.on_earlier_model() && matches!(self.kind, GeneratorKind::Position | GeneratorKind::Direction);
        if reads_geometry { &[] } else { self.kind.inputs() }
    }

    /// The texture the generator blends over its signal: only on the section controls, turned on with some opacity
    pub fn texture_in_use(&self) -> Option<&TextureLayer> {
        self.texture.as_deref().filter(|texture| texture.in_use() && self.section_controls().is_some())
    }

    /// Everything the generator reads instead of its mesh maps: its inputs, then its texture, its custom grunge, its micro details and its Grayscale Conversion image
    pub fn sources(&self) -> impl Iterator<Item = &Source> {
        let micro = self.micro.iter().flat_map(|micro| micro.sources()).chain(self.grayscale_controls_ref().and_then(|grayscale| grayscale.source.as_ref()));
        self.inputs.values().chain(self.texture.as_ref().and_then(|texture| texture.source.as_ref())).chain(self.grunge.as_ref()).chain(micro)
    }

    /// The micro details as saved, when the generator computes with them: with a slot switched on, except a micro detail saved before Micro Details on a generator still on the earlier model, which [`Generator::earlier_micro`] reads (bring them into range before use: saved files are not trusted)
    pub fn micro_in_use(&self) -> Option<&MicroDetails> {
        self.micro.as_deref().filter(|micro| micro.in_use(self.kind) && !(micro.earlier && self.on_earlier_model()))
    }

    /// The anchor point and amount (0..1) of a micro detail saved before Micro Details, while the generator is still on the earlier model: its signal adds that anchor's height times the amount, as it did when saved
    pub fn earlier_micro(&self) -> Option<(LayerId, f32)> {
        let micro = self.micro.as_deref().filter(|micro| micro.earlier && micro.in_use(self.kind) && micro.height && self.on_earlier_model())?;
        let Some(Source::Anchor { anchor, .. }) = &micro.height_source else { return None };

        let amount = micro.height_intensity / MicroDetails::HEIGHT_INTENSITY_PER_AMOUNT;
        Some((*anchor, if amount.is_finite() { amount.clamp(0.0, 1.0) } else { 0.0 }))
    }

    /// Whether the generator is on the level, softness and noise it had before the reference app's controls
    pub fn on_earlier_model(&self) -> bool {
        self.wear.is_none() && self.light.is_none() && self.section_controls().is_none() && !self.kind.has_no_earlier_model()
    }

    pub fn new(kind: GeneratorKind) -> Generator {
        let (level, softness, noise_amount) = match kind {
            GeneratorKind::EdgeWear => (0.5, 0.25, 0.3),
            GeneratorKind::Curvature => (0.5, 0.5, 0.0),
            GeneratorKind::Dirt => (0.55, 0.25, 0.35),
            GeneratorKind::ThinAreas => (0.3, 0.2, 0.0),
            _ => (0.5, 0.25, 0.0),
        };
        let detail = match kind {
            GeneratorKind::Curvature => Some(CurvatureDetail::default()),
            GeneratorKind::EdgeWear => Some(CurvatureDetail { mode: CurvatureMode::Edges, weights: CurvatureDetail::EDGE_WEIGHTS }),
            _ => None,
        };
        Generator {
            kind,
            level,
            softness,
            noise_amount,
            noise_scale: 8.0,
            seed: 7,
            offset: 0.0,
            direction: [0.0, 1.0, 0.0],
            invert: false,
            detail,
            global: GeneratorGlobal::default(),
            resolution: OutputResolution::default(),
            inputs: BTreeMap::new(),
            wear: WearParameters::defaults(kind),
            section: SectionParameters::defaults(kind),
            light: LightParameters::defaults(kind),
            texture: None,
            grunge: None,
            micro: None,
            controls: KindControls::defaults(kind).map(Box::new),
        }
    }

    /// The mask value for a (noise-broken) signal `s`: the smooth threshold at `level` over
    /// `softness`, inverted if asked, then shifted by `offset` (a non-finite offset counts as 0).
    pub fn shape(&self, s: f32) -> f32 {
        let th = 1.0 - self.level.clamp(0.0, 1.0);
        let soft = self.softness.max(1e-3);
        let t = ((s - (th - soft)) / (2.0 * soft)).clamp(0.0, 1.0);
        let v = t * t * (3.0 - 2.0 * t);
        let v = if self.invert { 1.0 - v } else { v };
        let offset = if self.offset.is_finite() { self.offset } else { 0.0 };
        (v + offset).clamp(0.0, 1.0)
    }
}

/// Filters usable on masks and content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Filter {
    Blur {
        radius: f32,
        /// Reads past the texture's edges from its opposite side, so tiling textures stay seamless; off in projects saved before it
        #[serde(default, skip_serializing_if = "is_false")]
        wrap: bool,
    },
    Sharpen {
        amount: f32,
    },
    Invert,
    /// Invert switched off: changes nothing until it is switched back on ([`Filter::Invert`])
    InvertOff,
    Threshold {
        level: f32,
        /// Which side of `level` turns white; at or above it in projects saved before the modes
        #[serde(default, skip_serializing_if = "ThresholdMode::is_default")]
        mode: ThresholdMode,
    },
    /// Hue/saturation/lightness shift (color channels; lightness only on grays and masks).
    /// `hue` is in turns, `saturation` and `lightness` in −1..1; all zero = unchanged.
    Hsl {
        hue: f32,
        saturation: f32,
        lightness: f32,
        /// Shifts in a perceptual color space (OKLCh), so a hue turn keeps the colors' lightness; plain HSL in projects saved before it
        #[serde(default, skip_serializing_if = "is_false")]
        perceptive: bool,
    },
    /// Contrast around mid gray and a luminosity shift, both −1..1 (0 = unchanged).
    ContrastLuminosity {
        contrast: f32,
        luminosity: f32,
    },
    /// Contrast, luminosity (both −1..1, 0 = unchanged) and saturation (0..1, 0.5 = unchanged) for the shadows, midtones and highlights, each `[contrast, luminosity, saturation]`, weighted by the texel's lightness as [`Filter::ColorBalance`]
    ColorCorrect {
        shadows: [f32; 3],
        midtones: [f32; 3],
        highlights: [f32; 3],
    },
    /// Color offsets (−1..1 per component) weighted towards the shadows, midtones and highlights.
    ColorBalance {
        shadows: [f32; 3],
        midtones: [f32; 3],
        highlights: [f32; 3],
    },
    /// The texel's gray (after `input`) mapped through a ramp of colors (scalars read the ramp's gray)
    GradientMap {
        colors: Vec<[f32; 3]>,
        /// Where each color sits on the ramp (0..1); evenly spaced in projects saved before
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        positions: Vec<f32>,
        /// How many of `colors` the ramp uses (2..10), the rest kept for later; all of them in projects saved before
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quantity: Option<u8>,
        /// How a texel becomes the position it reads; luma, untouched, in projects saved before
        #[serde(default, skip_serializing_if = "GradientInput::is_unchanged")]
        input: GradientInput,
        /// Blends neighbouring colors in linear RGB (off: in sRGB, as a color picker shows them); on in projects saved before
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        linear: bool,
        /// Shows the ramp itself across the texture (left to right) in place of the mapped values
        #[serde(default, skip_serializing_if = "is_false")]
        debug: bool,
    },
    /// Values remapped through a preset curve, after balance, contrast and invert like a gradient's input
    GradientCurve {
        preset: CurvePreset,
        invert: bool,
        balance: f32,
        contrast: f32,
    },
    /// A color read off a gradient source at `position` across it (`vertical`: down it); the source is a uniform color
    GradientDynamic {
        vertical: bool,
        position: f32,
        color: [f32; 3],
    },
    /// Quantize to `levels` steps (2..64).
    Posterize {
        levels: f32,
        /// Steps the content's coverage too (Apply to Alpha); off in projects saved before it
        #[serde(default, skip_serializing_if = "is_false")]
        alpha: bool,
    },
    /// The values between `min` and `max` eased from black to white (smoothstep); outside them black or white
    Smoothstep {
        min: f32,
        max: f32,
    },
    /// Heights flipped (`invert`), scaled by `multiply` and raised by `offset` around mid gray
    HeightAdjust {
        invert: bool,
        offset: f32,
        multiply: f32,
    },
    /// Keep values within `min..max`.
    Clamp {
        min: f32,
        max: f32,
    },
    /// A ramp slid through the values: those above `position` go towards white, those below
    /// towards black, over a width `contrast` (0..1) narrows to a hard step.
    HistogramScan {
        position: f32,
        contrast: f32,
        /// Scans from the other end: the position read as 1 − position (Invert Position); off in projects saved before it
        #[serde(default, skip_serializing_if = "is_false")]
        invert_position: bool,
    },
    /// The values squeezed towards `position`: `range` 1 keeps them, 0 sets them all to it.
    HistogramRange {
        range: f32,
        position: f32,
    },
    /// The values offset by `offset` (−1..1), wrapping around past white and black.
    HistogramShift {
        offset: f32,
    },
    /// Shift each texel's lookup along a noise vector field: `intensity` in texels at 2K, `scale` noise features per UV unit (projects saved before the warp modes; new ones are [`Filter::SourceWarp`])
    Warp {
        intensity: f32,
        scale: f32,
        seed: u32,
        /// Divides `intensity` (1, 10, 100 or 1000) for fine displacements; 1 in projects saved before it
        #[serde(default = "unit_divider", skip_serializing_if = "is_unit_divider")]
        divider: u16,
    },
    /// Blur along one direction: `radius` in texels at 2K, `angle` in degrees.
    DirectionalBlur {
        radius: f32,
        angle: f32,
    },
    /// Values smeared downhill along a noise height field: `radius` in texels at 2K, `scale` noise features per UV unit (projects saved before its source settings)
    SlopeBlur {
        radius: f32,
        scale: f32,
        seed: u32,
        /// Divides `radius` like [`Filter::Warp`]'s divider
        #[serde(default = "unit_divider", skip_serializing_if = "is_unit_divider")]
        divider: u16,
    },
    /// The detail finer than `radius` (texels at 2K) around mid gray: the value minus its blur.
    HighPass {
        radius: f32,
        /// Color Space: colors filtered in sRGB from linear sRGB or ACEScg
        #[serde(default, skip_serializing_if = "is_working_space")]
        space: HighpassSpace,
        /// Apply To Alpha: the content's coverage takes its detail too
        #[serde(default, skip_serializing_if = "is_false")]
        alpha: bool,
    },
    /// Gradient magnitude across `width` texels at 2K: edges light, flat areas black.
    EdgeDetect {
        width: f32,
    },
    /// Like [`Filter::Warp`], but every texel moves along one direction (`angle` in degrees; projects saved before the warp modes)
    DirectionalWarp {
        intensity: f32,
        angle: f32,
        scale: f32,
        seed: u32,
        /// Divides `intensity` like [`Filter::Warp`]'s divider
        #[serde(default = "unit_divider", skip_serializing_if = "is_unit_divider")]
        divider: u16,
    },
    /// Colors to their luminance (scalars and masks unchanged): projects saved before Grayscale Conversion's settings
    Grayscale,
    /// The gray of every channel (or of its Custom Input) by its mode and weights, balanced, contrasted and maybe inverted
    GrayscaleConversion(GrayscaleConversion),
    /// Flip the green component of normal maps (OpenGL ↔ DirectX); other channels unchanged.
    NormalInvert,
    /// Shapes rounded off at their edges: values ramp up from 0 at a shape's edge to their own
    /// level over about `width` texels at 2K (a mask's or height's bevel).
    Bevel {
        width: f32,
        /// Reads past the texture's edges like [`Filter::Blur`]'s wrap
        #[serde(default, skip_serializing_if = "is_false")]
        wrap: bool,
    },
    /// Shapes sloped at their edge, inwards or outwards over a share of the UV tile (projects saved before keep [`Filter::Bevel`])
    DistanceBevel(Bevel),
    /// Texels read from where a source sends them, in one of the warp modes (projects saved before keep [`Filter::Warp`] and [`Filter::DirectionalWarp`])
    SourceWarp(SourceWarp),
    /// Values read downhill along a source's slope, blended (projects saved before keep [`Filter::SlopeBlur`])
    SourceSlopeBlur(SourceSlopeBlur),
    /// A smooth ramp from the shapes' edge, outwards, inwards or across it
    BevelSmooth(BevelSmooth),
    /// A trail fading away from the shapes in one direction
    DirectionalDistance(DirectionalDistance),
    /// One side of an axis (or one quarter) of each UV tile copied over the rest
    Mirror(Mirror),
    /// Each UV tile's content scaled, turned, moved and flipped, repeating or not
    Transform(TileTransform),
    /// A band along the edge where values cross a threshold
    MaskOutline(MaskOutline),
    /// Square cells `size` texels at 2K across, each the value at its center (Pixelate; see [`Filter::pixels_amount`])
    Mosaic {
        size: f32,
        /// Steps the content's coverage into the cells too (Apply To Alpha); on in projects saved before the switch
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        alpha: bool,
    },
    /// Painterly smoothing along the strokes' direction: each texel the mean of the calmest sector around it
    AnisotropicKuwahara(AnisotropicKuwahara),
    /// Base colors outside the physically plausible ranges shown in red (reads the layer's metal)
    PbrValidate(PbrValidate),
    /// Height To Normal: the layer's Height turned into its Normal
    HeightToNormal(HeightToNormal),
    /// Color Match: the colors moved so a source color (the texture's mean by default) lands on a target color
    ColorMatch(ColorMatch),
    /// Stair-stepped edges smoothed by blending across them (FXAA)
    Fxaa {
        /// Smooths the content's coverage too (Affect Alpha Channel)
        #[serde(default, skip_serializing_if = "is_false")]
        alpha: bool,
    },
    /// Adds a blur of the values (`radius` texels at 2K) times `amount`: light bleeds around bright areas (projects saved before the halo glow below)
    Glow {
        radius: f32,
        amount: f32,
    },
    /// A halo in `color` around the shapes (content coverage, mask values), inside them below 0 `width`; see [`Filter::HALO_REACH`]
    HaloGlow {
        /// Laid on a mask as white rather than the color's luminance (Used on a Mask)
        on_mask: bool,
        /// Ends sharper (Mode Precise) instead of fading out (Soft)
        precise: bool,
        width: f32,
        spread: f32,
        opacity: f32,
        color: [f32; 3],
    },
    /// The shapes in `color` laid under them, moved `distance` of a UV tile away from a light at `angle` and blurred over `size`
    DropShadow {
        angle: f32,
        distance: f32,
        size: f32,
        spread: f32,
        opacity: f32,
        color: [f32; 3],
    },
    /// Lighting painted into the base color: `ambient` (times the baked ambient occlusion by `occlusion`) plus a directional light of strength `light` from `azimuth` around +Y (degrees from +Z) and `elevation` above the horizon (degrees), on the mesh's normals. `stylized` adds sky colors, more lights, cavity, edges and highlights (Baked Lighting Stylized); projects saved before have none and light as they did
    BakedLighting {
        azimuth: f32,
        elevation: f32,
        light: f32,
        ambient: f32,
        occlusion: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stylized: Option<Box<StylizedLighting>>,
    },
    /// The base color lit by the viewport environment (its diffuse light), turned `horizontal` and tilted `vertical` degrees, brighter by 2^`exposure`, on the mesh's normals tilted by the baked height (`height_intensity`) and darkened by the baked ambient occlusion (`ambient_occlusion` on the diffuse light, `specular_occlusion` on the reflections)
    EnvironmentLighting {
        horizontal: f32,
        vertical: f32,
        exposure: f32,
        height_intensity: f32,
        ambient_occlusion: f32,
        specular_occlusion: f32,
    },
    /// Colors near `from` (within `tolerance`, fading out over `softness`, as RGB distance
    /// 0..1) shifted by `to − from`, so their shading and texture stay (linear RGB).
    ReplaceColor {
        from: [f32; 3],
        to: [f32; 3],
        tolerance: f32,
        softness: f32,
    },
    /// No filter picked yet: changes nothing
    Empty,
}

/// The divider of projects saved before intensities had one
fn unit_divider() -> u16 {
    1
}

/// Serde default of a switch that was on in projects saved before it existed
fn yes() -> bool {
    true
}

/// Whether a switch is on, its default where it defaults to [`yes`] (not saved then)
fn is_true(on: &bool) -> bool {
    *on
}

/// Whether a high pass runs in the working color space, its default (not saved then)
fn is_working_space(space: &HighpassSpace) -> bool {
    *space == HighpassSpace::Working
}

/// Whether a switch is off, its default (not saved then)
fn is_false(on: &bool) -> bool {
    !*on
}

/// Whether `divider` leaves its intensity as it is (not saved then)
fn is_unit_divider(divider: &u16) -> bool {
    *divider == 1
}

/// The fewest and most steps a posterize keeps
pub const POSTERIZE_LEVELS_MIN: f32 = 2.0;
pub const POSTERIZE_LEVELS_MAX: f32 = 64.0;

/// Filter sizes are given in texels at 2K, as in the reference workflow
pub const SIZE_REFERENCE_RES: f32 = 2048.0;

/// `x` (0..1) rounded to `levels` steps (2..64) from black to white
pub fn posterize(x: f32, levels: f32) -> f32 {
    let steps = (levels.clamp(POSTERIZE_LEVELS_MIN, POSTERIZE_LEVELS_MAX).round() - 1.0).max(1.0);
    let x = if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 };
    (x * steps).round() / steps
}

/// Bounds of a smoothstep closer than this are a step: easing over a narrower span would divide by almost nothing
const MIN_SMOOTHSTEP_SPAN: f32 = 1e-6;

/// 0 at or below `min`, 1 at or above `max`, eased between them (swapped bounds are read in order; equal bounds are a step)
pub(crate) fn smoothstep(min: f32, max: f32, x: f32) -> f32 {
    let (low, high) = (finite_or(min.min(max), 0.0), finite_or(min.max(max), 1.0));
    if !x.is_finite() {
        return 0.0;
    }

    if high - low <= MIN_SMOOTHSTEP_SPAN {
        return if x >= high { 1.0 } else { 0.0 };
    }

    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `v`, or `fallback` where it is not a number
fn finite_or(v: f32, fallback: f32) -> f32 {
    if v.is_finite() { v } else { fallback }
}

/// Push `x` (0..1) towards 1 (`amount` > 0) or 0 (`amount` < 0); `amount` in −1..1.
fn shift_unit(x: f32, amount: f32) -> f32 {
    let a = amount.clamp(-1.0, 1.0);
    if a >= 0.0 { x + (1.0 - x) * a } else { x * (1.0 + a) }
}

fn hue_to_rgb(p: f32, q: f32, t: f32) -> f32 {
    let t = t.rem_euclid(1.0);
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

/// HSL adjustment of a 0..1 color (see [`Filter::Hsl`]). A gray stays gray and only its
/// lightness changes.
pub fn hsl_adjust(rgb: [f32; 3], hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    let [r, g, b] = rgb.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let (d, l) = (mx - mn, (mx + mn) * 0.5);
    let (h, s) = if d <= 1e-6 {
        (0.0, 0.0)
    } else {
        let s = if l > 0.5 { d / (2.0 - mx - mn) } else { d / (mx + mn) };
        let h = if mx == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if mx == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        (h / 6.0, s)
    };
    let h = h + if hue.is_finite() { hue } else { 0.0 };
    let (s, l) = (if d <= 1e-6 { 0.0 } else { shift_unit(s, saturation) }, shift_unit(l, lightness));
    if s <= 0.0 {
        return [l; 3];
    }

    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    [hue_to_rgb(p, q, h + 1.0 / 3.0), hue_to_rgb(p, q, h), hue_to_rgb(p, q, h - 1.0 / 3.0)]
}

/// Smooth weights of the shadows, midtones and highlights at lightness `l` (0..1), summing to 1: shadows fade out and highlights in with lightness
fn tone_weights(l: f32) -> (f32, f32, f32) {
    ((1.0 - l) * (1.0 - l), 2.0 * l * (1.0 - l), l * l)
}

/// [`hsl_adjust`] in OKLCh: the hue turns and the chroma scales (`saturation` −1 = gray, 1 = twice as vivid) at the color's own perceived lightness, which `lightness` then pushes towards white or black (linear 0..1 colors)
pub fn perceptive_adjust(rgb: [f32; 3], hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    let clean = |x: f32| if x.is_finite() { x } else { 0.0 };
    let [l, a, b] = crate::oklab::from_linear(rgb.map(|v| clean(v).clamp(0.0, 1.0)));
    let (sin, cos) = (clean(hue) * std::f32::consts::TAU).sin_cos();
    let chroma = 1.0 + clean(saturation).clamp(-1.0, 1.0);
    let (a, b) = ((a * cos - b * sin) * chroma, (a * sin + b * cos) * chroma);
    let l = shift_unit(l.clamp(0.0, 1.0), clean(lightness));
    crate::oklab::to_linear([l, a, b]).map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 })
}

/// Apply a 0..1 function to each component of a channel value: signed values map through
/// 0..1, normals are left alone.
pub fn per_unit(kind: ChannelKind, v: [f32; 3], f: impl Fn(f32) -> f32) -> [f32; 3] {
    match kind {
        ChannelKind::NormalMap => v,
        // One-component channels hold their value in the first slot (read as gray).
        ChannelKind::Signed => [f(v[0] * 0.5 + 0.5) * 2.0 - 1.0; 3],
        ChannelKind::Scalar => [f(v[0]); 3],
        ChannelKind::Color => v.map(f),
    }
}

/// The color at `t` (0..1) of a gradient through evenly spaced `colors` (None when empty).
pub fn gradient(colors: &[[f32; 3]], t: f32) -> Option<[f32; 3]> {
    let last = colors.len().checked_sub(1)?;
    let pos = if t.is_finite() { t.clamp(0.0, 1.0) * last as f32 } else { 0.0 };
    let i = (pos.floor() as usize).min(last);
    let (a, b) = (colors.get(i)?, colors.get((i + 1).min(last))?);
    let f = pos - i as f32;
    Some(std::array::from_fn(|k| a[k] + (b[k] - a[k]) * f))
}

impl Filter {
    /// The most Baked Lighting Environment's Exposure brightens or darkens, in stops
    pub const ENVIRONMENT_EXPOSURE_STOPS: f32 = 5.0;

    /// The choices of an intensity's divider
    pub const DIVIDERS: [u16; 4] = [1, 10, 100, 1000];

    /// The most colors a gradient's Color Quantity uses
    pub const MAX_GRADIENT_COLORS: u8 = 10;

    /// Texels at 2K a Blur Intensity (Blur, Blur Directional) of 1 reaches
    pub const BLUR_INTENSITY: f32 = 16.0;

    /// Texels at 2K a halo glow's Width or a drop shadow's Size of 1 reaches
    pub const HALO_REACH: f32 = 128.0;

    /// A drop shadow is cast at most this share of a UV tile away, either way
    pub const SHADOW_DISTANCE_MAX: f32 = 0.5;

    /// The noise features per UV unit a noise-driven warp or slope blur reads, at least and at most
    pub const NOISE_SCALE_MIN: f32 = 0.01;
    pub const NOISE_SCALE_MAX: f32 = 1000.0;

    /// A gradient as projects saved before the Input Transformation read it: `colors` evenly spaced, luma, linear blending
    pub fn even_gradient(colors: Vec<[f32; 3]>) -> Filter {
        Filter::GradientMap { colors, positions: Vec::new(), quantity: None, input: GradientInput::default(), linear: true, debug: false }
    }

    /// Histogram Shift's Position (0..1) for its `offset`: a shift back by 1 − position, so 1 shifts nothing (offsets above 0 saved before the Position read as themselves, the same shift but for white and black)
    pub fn shift_position(offset: f32) -> f32 {
        let offset = if offset.is_finite() { offset.clamp(-1.0, 1.0) } else { 0.0 };
        if offset <= 0.0 { offset + 1.0 } else { offset }
    }

    /// The Histogram Shift offset a Position (0..1) sets (see [`Filter::shift_position`])
    pub fn shift_offset(position: f32) -> f32 {
        let position = if position.is_finite() { position.clamp(0.0, 1.0) } else { 1.0 };
        position - 1.0
    }

    /// Pixelate's Pixels Amount (cells across a UV tile) for a cell `size` in texels at 2K
    pub fn pixels_amount(size: f32) -> f32 {
        SIZE_REFERENCE_RES / if size.is_finite() { size.max(1.0) } else { SIZE_REFERENCE_RES }
    }

    /// The cell size in texels at 2K that a Pixels Amount (cells across a UV tile) sets
    pub fn pixel_size(amount: f32) -> f32 {
        SIZE_REFERENCE_RES / if amount.is_finite() { amount.clamp(1.0, SIZE_REFERENCE_RES) } else { 1.0 }
    }

    /// The colors a gradient's ramp uses: the first Color Quantity of them (all of them where it has none)
    pub fn ramp_colors(&self) -> &[[f32; 3]] {
        match self {
            Filter::GradientMap { colors, quantity, .. } => {
                colors.get(..quantity.map_or(colors.len(), |count| usize::from(count.clamp(2, Self::MAX_GRADIENT_COLORS))).min(colors.len())).unwrap_or(colors)
            }
            _ => &[],
        }
    }

    /// A gradient's color at ramp position `t` (None for other filters and an empty ramp)
    pub fn ramp_at(&self, t: f32) -> Option<[f32; 3]> {
        let Filter::GradientMap { positions, linear, .. } = self else { return None };
        let colors = self.ramp_colors();

        // Evenly spaced linear ramps take the path projects saved before took, so they render bit for bit as before
        if positions.is_empty() && *linear { gradient(colors, t) } else { ramp(colors, positions, *linear, t) }
    }

    /// Whether the filter reads neighbouring texels (the compositor runs it over a region).
    pub fn is_neighbourhood(&self) -> bool {
        matches!(
            self,
            Filter::Blur { .. }
                | Filter::Sharpen { .. }
                | Filter::Warp { .. }
                | Filter::DirectionalBlur { .. }
                | Filter::SlopeBlur { .. }
                | Filter::HighPass { .. }
                | Filter::EdgeDetect { .. }
                | Filter::DirectionalWarp { .. }
                | Filter::Bevel { .. }
                | Filter::Mosaic { .. }
                | Filter::Fxaa { .. }
                | Filter::AnisotropicKuwahara(_)
                | Filter::HeightToNormal(_)
                | Filter::ColorMatch(_)
                | Filter::DistanceBevel(_)
                | Filter::SourceWarp(_)
                | Filter::SourceSlopeBlur(_)
                | Filter::BevelSmooth(_)
                | Filter::DirectionalDistance(_)
                | Filter::Mirror(_)
                | Filter::Transform(_)
                | Filter::MaskOutline(_)
                | Filter::Glow { .. }
                | Filter::HaloGlow { .. }
                | Filter::DropShadow { .. }
        )
    }

    /// The filter applied to one value of a channel of `kind` (masks are scalars). None for
    /// neighbourhood filters and where it changes nothing (an empty gradient).
    pub fn point(&self, kind: ChannelKind, v: [f32; 3]) -> Option<[f32; 3]> {
        // Histogram Scan's narrowest soft cut, so near full contrast stays a finite slope
        const SCAN_MIN_WIDTH: f32 = 1e-3;

        // Replace Color's least softness, so a hard edge divides by no zero
        const REPLACE_MIN_SOFTNESS: f32 = 1e-6;

        Some(match self {
            Filter::Invert => per_unit(kind, v, |x| 1.0 - x),
            Filter::Threshold { level, mode } => per_unit(kind, v, |x| if mode.passes(x, *level) { 1.0 } else { 0.0 }),
            Filter::Hsl { hue, saturation, lightness, perceptive } => match kind {
                ChannelKind::Color if *perceptive => perceptive_adjust(v, *hue, *saturation, *lightness),
                ChannelKind::Color => hsl_adjust(v, *hue, *saturation, *lightness),
                _ => per_unit(kind, v, |x| hsl_adjust([x; 3], 0.0, 0.0, *lightness)[0]),
            },
            Filter::ContrastLuminosity { contrast, luminosity } => {
                let slope = 1.0 + contrast.clamp(-1.0, 1.0);
                per_unit(kind, v, |x| finite_unit((x - 0.5) * slope + 0.5 + luminosity))
            }
            Filter::ColorCorrect { shadows, midtones, highlights } => {
                // [contrast, luminosity, saturation] at the texel's lightness, weighted as color balance
                let at = |lightness: f32| -> [f32; 3] {
                    let (in_shadows, in_midtones, in_highlights) = tone_weights(finite_unit(lightness));
                    std::array::from_fn(|k| shadows[k] * in_shadows + midtones[k] * in_midtones + highlights[k] * in_highlights)
                };
                let correct = |x: f32, [contrast, luminosity, _]: [f32; 3]| (x - 0.5) * (1.0 + contrast.clamp(-1.0, 1.0)) + 0.5 + luminosity.clamp(-1.0, 1.0);
                match kind {
                    ChannelKind::Color => {
                        let tone = at(lum(v));
                        let corrected = v.map(|x| correct(x, tone));
                        let gray = lum(corrected);
                        let gain = 2.0 * tone[2].clamp(0.0, 1.0);
                        corrected.map(|x| finite_unit(gray + (x - gray) * gain))
                    }
                    _ => per_unit(kind, v, |x| finite_unit(correct(x, at(x)))),
                }
            }
            Filter::ColorBalance { shadows, midtones, highlights } => {
                let shift = |component: usize, lightness: f32| {
                    let (in_shadows, in_midtones, in_highlights) = tone_weights(finite_unit(lightness));
                    shadows.get(component).copied().unwrap_or(0.0) * in_shadows
                        + midtones.get(component).copied().unwrap_or(0.0) * in_midtones
                        + highlights.get(component).copied().unwrap_or(0.0) * in_highlights
                };
                match kind {
                    ChannelKind::Color => {
                        let lightness = lum(v);
                        std::array::from_fn(|k| finite_unit(v[k] + shift(k, lightness)))
                    }
                    _ => per_unit(kind, v, |x| finite_unit(x + (shift(0, x) + shift(1, x) + shift(2, x)) / 3.0)),
                }
            }
            Filter::GradientMap { input, .. } => input.map(kind, v, |position| self.ramp_at(position))?,
            Filter::GradientCurve { preset, invert, balance, contrast } => per_unit(kind, v, |x| preset.at(shape_gray(x, *balance, *contrast, *invert))),
            Filter::GradientDynamic { color, .. } => match kind {
                ChannelKind::Color => color.map(finite_unit),
                _ => per_unit(kind, v, |_| finite_unit(lum(*color))),
            },
            Filter::Posterize { levels, .. } => per_unit(kind, v, |x| posterize(x, *levels)),
            Filter::Smoothstep { min, max } => per_unit(kind, v, |x| smoothstep(*min, *max, x)),
            Filter::HeightAdjust { invert, offset, multiply } => {
                let (offset, multiply) = (finite_or(*offset, 0.0), finite_or(*multiply, 1.0));
                per_unit(kind, v, |x| {
                    let x = if *invert { 1.0 - x } else { x };
                    finite_unit((x - 0.5) * multiply + 0.5 + offset)
                })
            }
            Filter::InvertOff => v,
            Filter::Clamp { min, max } => {
                let (low, high) = (min.min(*max), min.max(*max));
                per_unit(kind, v, |x| x.max(low).min(high))
            }

            // Full contrast is a threshold: the position itself goes to white
            Filter::HistogramScan { position, contrast, invert_position } => {
                let position = if *invert_position { 1.0 - position } else { *position };
                if *contrast >= 1.0 {
                    per_unit(kind, v, |x| if x >= position { 1.0 } else { 0.0 })
                } else {
                    let width = (1.0 - contrast.clamp(0.0, 1.0)).max(SCAN_MIN_WIDTH);
                    per_unit(kind, v, |x| finite_unit((x - position) / width + 0.5))
                }
            }
            Filter::HistogramRange { range, position } => per_unit(kind, v, |x| finite_unit(position + (x - position) * range)),
            Filter::HistogramShift { offset } => per_unit(kind, v, |x| {
                // Wrapped by hand: `rem_euclid` would also send white (1 + 0) to black
                let y = finite_unit(x) + offset.clamp(-1.0, 1.0);
                finite_unit(if y > 1.0 {
                    y - 1.0
                } else if y < 0.0 {
                    y + 1.0
                } else {
                    y
                })
            }),
            Filter::Grayscale => match kind {
                ChannelKind::Color => [lum(v); 3],
                _ => v,
            },
            // Read opaque here: the compositor passes the content's coverage (and a Custom Input) through `GrayscaleConversion::channel_value`
            Filter::GrayscaleConversion(conversion) => conversion.channel_value(kind, v, None, 1.0),
            Filter::NormalInvert => match kind {
                ChannelKind::NormalMap => [v[0], 1.0 - v[1], v[2]],
                _ => v,
            },
            Filter::ReplaceColor { from, to, tolerance, softness } => match kind {
                ChannelKind::Color => {
                    // How far the color is from `from`, 1 across the whole cube
                    let distance = ((v[0] - from[0]).powi(2) + (v[1] - from[1]).powi(2) + (v[2] - from[2]).powi(2)).sqrt() / 3f32.sqrt();
                    let (tolerance, softness) = (tolerance.clamp(0.0, 1.0), softness.clamp(0.0, 1.0));
                    let weight = if distance <= tolerance {
                        1.0
                    } else if distance >= tolerance + softness {
                        0.0
                    } else {
                        let closeness = 1.0 - (distance - tolerance) / softness.max(REPLACE_MIN_SOFTNESS);
                        closeness * closeness * (3.0 - 2.0 * closeness)
                    };
                    std::array::from_fn(|k| finite_unit(v[k] + (to[k] - from[k]) * weight))
                }
                _ => v,
            },
            Filter::Blur { .. }
            | Filter::Sharpen { .. }
            | Filter::Warp { .. }
            | Filter::DirectionalBlur { .. }
            | Filter::SlopeBlur { .. }
            | Filter::HighPass { .. }
            | Filter::EdgeDetect { .. }
            | Filter::DirectionalWarp { .. }
            | Filter::Bevel { .. }
            | Filter::Mosaic { .. }
            | Filter::Fxaa { .. }
            | Filter::AnisotropicKuwahara(_)
            | Filter::PbrValidate(_)
            | Filter::HeightToNormal(_)
            | Filter::ColorMatch(_)
            | Filter::DistanceBevel(_)
            | Filter::SourceWarp(_)
            | Filter::SourceSlopeBlur(_)
            | Filter::BevelSmooth(_)
            | Filter::DirectionalDistance(_)
            | Filter::Mirror(_)
            | Filter::Transform(_)
            | Filter::MaskOutline(_)
            | Filter::Glow { .. }
            | Filter::HaloGlow { .. }
            | Filter::DropShadow { .. }
            | Filter::BakedLighting { .. }
            | Filter::EnvironmentLighting { .. }
            | Filter::Empty => return None,
        })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Filter::Blur { .. } => "Blur",
            Filter::Sharpen { .. } => "Sharpen",
            Filter::Invert | Filter::InvertOff => "Invert",
            Filter::Threshold { .. } => "Threshold",
            Filter::Hsl { .. } => "HSL Perceptive",
            Filter::ContrastLuminosity { .. } => "Contrast Luminosity",
            Filter::ColorBalance { .. } => "Color Balance",
            Filter::ColorCorrect { .. } => "Color Correct",
            Filter::GradientMap { .. } => "Gradient",
            Filter::GradientCurve { .. } => "Gradient Curve",
            Filter::GradientDynamic { .. } => "Gradient Dynamic",
            Filter::Posterize { .. } => "Posterize",
            Filter::Smoothstep { .. } => "Smoothstep",
            Filter::HeightAdjust { .. } => "Height Adjust",
            Filter::Clamp { .. } => "Clamp",
            Filter::HistogramScan { .. } => "Histogram Scan",
            Filter::HistogramRange { .. } => "Histogram Range",
            Filter::HistogramShift { .. } => "Histogram Shift",
            Filter::Warp { .. } => "Warp",
            Filter::DirectionalBlur { .. } => "Blur Directional",
            Filter::SlopeBlur { .. } => "Blur Slope",
            Filter::HighPass { .. } => "Highpass",
            Filter::EdgeDetect { .. } => "Edge Detect",
            Filter::DirectionalWarp { .. } => "Warp Directional",
            Filter::Grayscale | Filter::GrayscaleConversion(_) => "Grayscale Conversion",
            Filter::NormalInvert => "Normal Invert",
            Filter::Bevel { .. } => "Bevel",
            Filter::Mosaic { .. } => "Pixelate",
            Filter::Fxaa { .. } => "FXAA (Anti-aliasing)",
            Filter::AnisotropicKuwahara(_) => "Anisotropic Kuwahara",
            Filter::PbrValidate(_) => "PBR Validate (Metallic Roughness)",
            Filter::HeightToNormal(_) => "Height To Normal",
            Filter::ColorMatch(_) => "Color Match",
            Filter::DistanceBevel(_) => "Bevel",
            Filter::SourceWarp(_) => "Warp",
            Filter::SourceSlopeBlur(_) => "Blur Slope",
            Filter::BevelSmooth(_) => "Bevel Smooth",
            Filter::DirectionalDistance(_) => "Directional Distance",
            Filter::Mirror(_) => "Mirror",
            Filter::Transform(_) => "Transform",
            Filter::MaskOutline(_) => "Mask Outline",
            Filter::Glow { .. } | Filter::HaloGlow { .. } => "Glow",
            Filter::DropShadow { .. } => "Drop Shadow",
            Filter::BakedLighting { .. } => "Baked Lighting Stylized",
            Filter::EnvironmentLighting { .. } => "Baked Lighting Environment",
            Filter::ReplaceColor { .. } => "Replace Color",
            Filter::Empty => "Filter (empty)",
        }
    }

    /// The label this filter had before it took the reference workflow's name: effects named after it still follow a swap
    pub fn former_label(&self) -> Option<&'static str> {
        match self {
            Filter::ContrastLuminosity { .. } => Some("Contrast/Luminosity"),
            Filter::SlopeBlur { .. } | Filter::SourceSlopeBlur(_) => Some("Slope Blur"),
            Filter::HighPass { .. } => Some("High Pass"),
            Filter::Mosaic { .. } => Some("Mosaic"),
            Filter::Grayscale => Some("Grayscale"),
            Filter::GradientMap { .. } => Some("Gradient Map"),
            Filter::BakedLighting { .. } => Some("Baked Lighting"),
            _ => None,
        }
    }

    /// Baked lighting's factor on the base color of a surface facing `normal` (world space) with
    /// ambient occlusion `ao` (None for other filters).
    pub fn baked_light(&self, normal: [f32; 3], ao: f32) -> Option<f32> {
        let Filter::BakedLighting { azimuth, elevation, light, ambient, occlusion, .. } = self else { return None };
        let (sin_azimuth, cos_azimuth) = azimuth.to_radians().sin_cos();
        let (sin_elevation, cos_elevation) = elevation.to_radians().sin_cos();

        // Multiplied in this order so saved projects light exactly as before
        let facing = (normal[0] * cos_elevation * sin_azimuth + normal[1] * sin_elevation + normal[2] * cos_elevation * cos_azimuth).max(0.0);
        let ao = if ao.is_finite() { ao.clamp(0.0, 1.0) } else { 1.0 };
        let factor = ambient * (1.0 + (ao - 1.0) * occlusion.clamp(0.0, 1.0)) + light * facing;
        Some(if factor.is_finite() { factor.max(0.0) } else { 1.0 })
    }

    /// The channels a new content effect of the filter starts with on (None: all of them)
    pub fn starts_on(&self) -> Option<&'static [Channel]> {
        match self {
            // PBR Validate reads the metal but changes only the base color, so its Metal pill would do nothing
            Filter::BakedLighting { .. } | Filter::EnvironmentLighting { .. } | Filter::ColorMatch(_) | Filter::PbrValidate(_) => Some(&[Channel::BaseColor]),
            Filter::HeightToNormal(_) => Some(&[Channel::Normal]),
            _ => None,
        }
    }

    /// Baked Lighting Stylized's settings with its sun and sky (None for other filters and a Baked Lighting saved before them)
    pub fn stylized_lighting(&self) -> Option<(&StylizedLighting, SunAndSky)> {
        let Filter::BakedLighting { azimuth, elevation, light, ambient, occlusion, stylized: Some(stylized) } = self else { return None };
        let sun = DirectionalLight { azimuth: *azimuth, elevation: *elevation, intensity: *light, color: stylized.sun_color };
        Some((stylized, SunAndSky { sun, sky_intensity: *ambient, diffuse_ao: *occlusion }))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EffectKind {
    /// Painted grayscale (mask) or material (content) pixels keyed by the effect id
    Paint,
    /// A path sublayer: pixels keyed by the effect id like [`EffectKind::Paint`], but painted only from its stored paths and repainted whenever one is added, edited or removed
    Path(Vec<PathShape>),
    /// A uniform or procedural grayscale fill.
    Fill {
        source: Source,
        #[serde(serialize_with = "crate::saved_projection::save", deserialize_with = "crate::saved_projection::load")]
        projection: Projection,
        #[serde(default = "Sampling::saved_before", skip_serializing_if = "Sampling::is_saved_before")]
        sampling: Sampling,
    },
    Generator(Generator),
    Levels(Levels),
    Filter(Filter),
    /// Select ID-map colors within a tolerance.
    ColorSelection(crate::ColorSelection),
    /// Compare one channel (or the mask so far, or a constant) on two sides: 1 where the comparison holds, softened by `hardness`
    Compare {
        op: CompareOp,
        /// The constant a [`CompareSource::Constant`] side reads
        value: f32,
        /// How far apart two values may be and still count as equal ([`CompareOp::WithinTolerance`])
        tolerance: f32,
        #[serde(default = "CompareSource::saved_left", skip_serializing_if = "CompareSource::is_saved_left")]
        left: CompareSource,
        #[serde(default = "CompareSource::saved_right", skip_serializing_if = "CompareSource::is_saved_right")]
        right: CompareSource,
        /// The channel a layer side reads
        #[serde(default = "saved_compare_channel", skip_serializing_if = "is_saved_compare_channel")]
        channel: Channel,
        /// 1 is a hard step (what compare masks saved before hardness did); lower ramps over 1 − hardness
        #[serde(default = "saved_compare_hardness", skip_serializing_if = "is_saved_compare_hardness")]
        hardness: f32,
    },
    /// Content only: a material fill (one source per channel; channels without a source are
    /// left as they are).
    Material(Fill),
    /// Records the stack at its place so masks, fills and generators above can read it ([`Source::Anchor`]): in a content stack the composited stack right after its layer (that layer and everything below), in a mask the mask value there (every channel reads that gray)
    Anchor,
}

/// One path of a path sublayer: its control points and how it was painted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathShape {
    /// Surface positions (3D view) or texture coordinates with z = 0 (2D view).
    pub points: Vec<[f32; 3]>,
    /// Per point: the curve turns sharply there instead of passing smoothly through. Empty (and absent from old
    /// projects) when every point is smooth; a missing entry is smooth too.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corners: Vec<bool>,
    pub uv: bool,
    pub closed: bool,
    /// A closed path fills its interior instead of stroking the curve.
    pub fill: bool,
    /// Viewport size (pixels) the path was painted in.
    pub viewport: [f32; 2],
    /// Brush, material and camera the engine paints the path with (owned by the engine).
    pub paint: serde_json::Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareOp {
    LesserThan,
    WithinTolerance,
    GreaterThan,
}

impl CompareOp {
    /// Menu order
    pub const ALL: [CompareOp; 3] = [CompareOp::LesserThan, CompareOp::WithinTolerance, CompareOp::GreaterThan];

    /// Command id
    pub fn id(self) -> &'static str {
        match self {
            CompareOp::LesserThan => "lesser",
            CompareOp::WithinTolerance => "within",
            CompareOp::GreaterThan => "greater",
        }
    }

    pub fn from_id(id: &str) -> Option<CompareOp> {
        match id {
            // mask.add.compare took "less" as well as "lesser" before the ids were the op names
            "less" => Some(CompareOp::LesserThan),
            _ => CompareOp::ALL.into_iter().find(|op| op.id() == id),
        }
    }

    /// The mask value of `left` compared with `right`: a hard 0/1 at hardness 1, else a ramp over 1 − hardness past the edge
    pub fn mask(self, left: f32, right: f32, tolerance: f32, hardness: f32) -> f32 {
        // How far past the edge of the passing range `left` is (positive passes)
        let past = match self {
            CompareOp::LesserThan => right - left,
            CompareOp::GreaterThan => left - right,
            CompareOp::WithinTolerance => tolerance - (left - right).abs(),
        };
        let width = (1.0 - hardness).clamp(0.0, 1.0);
        let pass = if self == CompareOp::WithinTolerance { past >= 0.0 } else { past > 0.0 };
        if width.is_nan() || width <= 0.0 || past.is_nan() {
            return if pass { 1.0 } else { 0.0 };
        }

        let ramp = if self == CompareOp::WithinTolerance { 1.0 + past / width } else { past / width };
        ramp.clamp(0.0, 1.0)
    }
}

/// The channel of compare masks saved before the channel choice (their sides read no channel)
fn saved_compare_channel() -> Channel {
    Channel::Height
}

/// The hardness of compare masks saved before hardness existed (a hard step)
fn saved_compare_hardness() -> f32 {
    1.0
}

/// Whether a compare mask reads the channel saved ones did (not saved then)
fn is_saved_compare_channel(channel: &Channel) -> bool {
    *channel == saved_compare_channel()
}

/// Whether a compare mask is the hard step saved ones were (not saved then)
fn is_saved_compare_hardness(hardness: &f32) -> bool {
    *hardness == saved_compare_hardness()
}

impl EffectKind {
    pub fn name(&self) -> &'static str {
        match self {
            EffectKind::Paint => "Paint",
            EffectKind::Path(_) => "Path",
            EffectKind::Fill { .. } => "Fill",
            EffectKind::Generator(_) => "Generator",
            EffectKind::Levels(_) => "Levels",
            EffectKind::Filter(f) => f.label(),
            EffectKind::ColorSelection { .. } => "Color selection",
            EffectKind::Compare { .. } => "Compare mask",
            EffectKind::Material(_) => "Fill",
            EffectKind::Anchor => "Anchor point",
        }
    }

    /// Whether the effect has a blend mode and opacity of its own (levels and anchor points do not)
    pub fn blends(&self) -> bool {
        !matches!(self, EffectKind::Levels(_) | EffectKind::Anchor)
    }

    /// Whether the effect belongs in a layer's content stack (`true`), its mask (`false`), or
    /// both (`None`).
    pub fn content_only(&self) -> Option<bool> {
        match self {
            EffectKind::Material(_) => Some(true),
            EffectKind::Fill { .. } | EffectKind::Generator(_) | EffectKind::ColorSelection { .. } | EffectKind::Compare { .. } => Some(false),
            EffectKind::Paint | EffectKind::Path(_) | EffectKind::Levels(_) | EffectKind::Filter(_) | EffectKind::Anchor => None,
        }
    }

    /// Holds painted pixels keyed by the effect id (paint and path sublayers).
    pub fn painted(&self) -> bool {
        matches!(self, EffectKind::Paint | EffectKind::Path(_))
    }

    /// Whether a source of this effect reads anchor point `anchor` (any anchor point when None)
    pub fn reads(&self, anchor: Option<LayerId>) -> bool {
        match self {
            EffectKind::Fill { source, .. } => source.reads_anchor(anchor),
            EffectKind::Material(fill) => fill.sources.values().any(|source| source.reads_anchor(anchor)),
            EffectKind::Generator(generator) => generator.sources().any(|source| source.reads_anchor(anchor)),
            _ => false,
        }
    }

    /// The anchor points this effect's sources read, with the channel each reads.
    pub fn anchors_read(&self) -> Vec<(LayerId, Channel)> {
        let sources: Vec<&Source> = match self {
            EffectKind::Fill { source, .. } => vec![source],
            EffectKind::Material(fill) => fill.sources.values().collect(),
            EffectKind::Generator(generator) => generator.sources().collect(),
            _ => Vec::new(),
        };
        sources
            .into_iter()
            .filter_map(|source| match source {
                Source::Anchor { anchor, channel, .. } => Some((*anchor, *channel)),
                _ => None,
            })
            .collect()
    }

    /// The image resources this effect's sources read (a fill, its patterns, a material or a generator input, a colour selection's ID map)
    pub fn images_read(&self) -> impl Iterator<Item = &str> {
        let sources: Vec<&Source> = match self {
            EffectKind::Fill { source, .. } => vec![source],
            EffectKind::Material(fill) => fill.sources.values().collect(),
            EffectKind::Generator(generator) => generator.sources().collect(),
            _ => Vec::new(),
        };
        let id_map = match self {
            EffectKind::ColorSelection(selection) => selection.id_map.as_deref(),
            _ => None,
        };
        sources.into_iter().flat_map(Source::images).chain(id_map)
    }

    /// Reads a channel of the layer or of the stack below it (a compare mask's layer sides)
    pub fn reads_stack(&self) -> bool {
        matches!(self, EffectKind::Compare { left, right, .. } if left.reads_stack() || right.reads_stack())
    }
}

impl Layer {
    /// Whether any source of the layer (fill, mask or content effects) reads an anchor point.
    pub fn reads_anchor(&self) -> bool {
        let fill = match &self.kind {
            LayerKind::Fill(fill) => fill.sources.values().any(|source| source.reads_anchor(None)),
            _ => false,
        };
        fill || self.mask.iter().flat_map(|mask| mask.effects.iter()).chain(self.content_effects.iter()).any(|effect| effect.reads(None))
    }

    /// Whether the layer's result depends on the stack below it: it reads an anchor point, or its mask compares channels of the stack
    pub fn reads_stack(&self) -> bool {
        self.reads_anchor() || self.mask.iter().flat_map(|mask| mask.effects.iter()).any(|effect| effect.visible && effect.kind.reads_stack())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub kind: EffectKind,
    pub blend: BlendMode,
    pub opacity: f32,
    /// A content effect's channels that blend apart from `blend` and `opacity` (none in projects saved before they existed)
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub channel_blends: BTreeMap<Channel, ChannelBlend>,
    /// A content effect's channels it leaves as they are (none in projects saved before they could be switched off)
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub off_channels: BTreeSet<Channel>,
    /// A filter effect's image inputs set to a source, by [`crate::ImageInput`] id (the others read their default gray)
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, Source>,
}

impl Effect {
    pub fn new(id: LayerId, kind: EffectKind) -> Effect {
        // Generators and fills that build a mask usually add on top of the background
        Effect {
            id,
            name: kind.name().to_string(),
            visible: true,
            kind,
            blend: BlendMode::Normal,
            opacity: 1.0,
            channel_blends: BTreeMap::new(),
            off_channels: BTreeSet::new(),
            inputs: BTreeMap::new(),
        }
    }

    /// The image resources the effect reads: its kind's ([`EffectKind::images_read`]) and its filter image inputs'
    pub fn images_read(&self) -> impl Iterator<Item = &str> {
        self.kind.images_read().chain(self.inputs.values().flat_map(Source::images))
    }

    /// Whether a source of the effect, or one of its image inputs, reads anchor point `anchor` (any anchor point when None)
    pub fn reads(&self, anchor: Option<LayerId>) -> bool {
        self.kind.reads(anchor) || self.inputs.values().any(|source| source.reads_anchor(anchor))
    }

    /// The source image input `input` reads: the one set on the effect, else its default gray
    pub fn input(&self, input: &crate::ImageInput) -> Source {
        self.inputs.get(input.id).cloned().unwrap_or_else(|| input.default_source())
    }

    /// How the effect blends on `channel` of its layer's content, or on its mask (`None`)
    pub fn blend_of(&self, channel: Option<Channel>) -> ChannelBlend {
        let shared = ChannelBlend { mode: self.blend, opacity: self.opacity };
        channel.and_then(|channel| self.channel_blends.get(&channel).copied()).unwrap_or(shared)
    }

    /// How the effect lays onto `channel` when composited: [`Self::blend_of`], at no opacity on a channel it is switched off for
    pub fn applied_blend(&self, channel: Option<Channel>) -> ChannelBlend {
        let blend = self.blend_of(channel);
        match channel {
            Some(channel) if self.off_channels.contains(&channel) => ChannelBlend { opacity: 0.0, ..blend },
            _ => blend,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decal_saved_without_symmetry_loads_unchanged_and_saves_without_it() {
        let old = r#"{"center":[0.0,0.0,1.0],"normal":[0.0,0.0,1.0],"up":[0.0,1.0,0.0],"size":[1.0,1.0],"rotation":0.0,"offset":[0.0,0.0],"depth":0.5}"#;
        let decal: Decal = serde_json::from_str(old).unwrap();
        assert_eq!(decal.symmetry, None);
        assert_eq!(serde_json::to_string(&decal).unwrap(), old.replace(' ', ""), "nothing new is written for a decal without a copy");

        let mirrored = Decal { symmetry: Some(DecalSymmetry { axis: 2, flip: DecalFlip::V, offset: 0.0 }), ..decal };
        let json = serde_json::to_string(&mirrored).unwrap();
        assert!(json.contains(r#""symmetry":{"axis":2,"flip":"v"}"#), "{json}");
        assert_eq!(serde_json::from_str::<Decal>(&json).unwrap(), mirrored);
        let without_flip = json.replace(r#","flip":"v""#, "");
        assert_eq!(serde_json::from_str::<Decal>(&without_flip).unwrap().symmetry, Some(DecalSymmetry { axis: 2, flip: DecalFlip::None, offset: 0.0 }));

        // The plane's offset is written only when it is not zero
        let moved = Decal { symmetry: Some(DecalSymmetry { axis: 1, flip: DecalFlip::None, offset: -0.25 }), ..decal };
        let json = serde_json::to_string(&moved).unwrap();
        assert!(json.contains(r#""offset":-0.25}"#), "{json}");
        assert_eq!(serde_json::from_str::<Decal>(&json).unwrap(), moved);
    }

    #[test]
    fn material_effect_images_are_reported_as_used() {
        let mut fill = Fill::default();
        fill.sources.insert(Channel::BaseColor, Source::image("material.png"));
        let effect = EffectKind::Material(fill);

        assert_eq!(effect.images_read().collect::<Vec<_>>(), vec!["material.png"]);
    }

    #[test]
    fn a_colour_selection_id_map_and_a_filter_image_input_are_reported_as_used() {
        let mut selection = crate::ColorSelection::new(Vec::new(), 0.0);
        selection.id_map = Some("id.png".into());
        let mut effect = Effect::new(LayerId(1), EffectKind::ColorSelection(selection));
        effect.inputs.insert("mask".into(), Source::image("input.png"));

        assert_eq!(effect.images_read().collect::<Vec<_>>(), vec!["id.png", "input.png"]);
    }

    #[test]
    fn generator_offset_shifts_the_mask_value() {
        let mut g = Generator::new(GeneratorKind::Position);
        let base = g.shape(0.2);
        assert!(base < 0.5);
        g.offset = 0.3;
        assert!((g.shape(0.2) - (base + 0.3)).abs() < 1e-6, "the offset is added");
        g.offset = 1.0;
        assert_eq!(g.shape(0.2), 1.0, "and the result is clamped");
        g.offset = -1.0;
        assert_eq!(g.shape(0.9), 0.0);
    }

    #[test]
    fn generator_non_finite_offset_counts_as_zero() {
        let mut g = Generator::new(GeneratorKind::Position);
        let base: Vec<f32> = [0.0, 0.2, 0.5, 0.9].iter().map(|&s| g.shape(s)).collect();
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            g.offset = bad;
            let got: Vec<f32> = [0.0, 0.2, 0.5, 0.9].iter().map(|&s| g.shape(s)).collect();
            assert_eq!(got, base, "offset {bad} behaves like 0");
        }
    }

    #[test]
    fn patterns_saved_before_tile_settings_keep_their_form() {
        let saved = serde_json::json!({"kind": "Clouds", "scale": 4.0, "seed": 1, "contrast": 1.0, "balance": 0.0, "invert": false});

        let loaded: Procedural = serde_json::from_value(saved.clone()).unwrap();

        assert_eq!(loaded, Procedural::new(ProceduralKind::Clouds));
        assert_eq!(serde_json::to_value(&loaded).unwrap(), saved, "a pattern at its defaults saves as before");

        let tile = serde_json::to_value(Procedural::new(ProceduralKind::TileGenerator)).unwrap();

        assert!(
            tile.get("tile").is_some() && tile.get("resolution").is_none(),
            "only the Tile Generator carries tile settings, and a default resolution is not saved"
        );
    }

    #[test]
    fn tile_settings_saved_with_missing_or_hostile_values_load_in_range() {
        let tile = serde_json::json!({"amount": [0, 4000], "specific": 7.0, "quarter_turns": 6, "inputs": ["a", "b", "c", "d", "e", "f", "g"]});
        let saved = serde_json::json!({"kind": "TileGenerator", "scale": 1.0, "seed": 1, "contrast": 1.0, "balance": 0.0, "invert": false, "tile": tile});

        let loaded: Procedural = serde_json::from_value(saved).unwrap();
        let tile = loaded.tile.unwrap();

        assert_eq!(tile.amount, [1, 64]);
        assert_eq!(tile.specific, 1.0);
        assert_eq!(tile.quarter_turns, 2);
        assert_eq!(tile.inputs.len(), TileGenerator::MAX_INPUTS);
        assert_eq!(tile.middle_size, TileGenerator::default().middle_size, "a missing setting reads its default");

        let not_finite = TileGenerator { scale: f32::NAN, rotation: f32::INFINITY, ..TileGenerator::default() }.clamped();

        assert_eq!((not_finite.scale, not_finite.rotation), (1.0, 0.0));
    }

    #[test]
    fn generator_ids_are_the_saved_names_and_parse_back() {
        for kind in GeneratorKind::ALL {
            assert_eq!(serde_json::to_value(kind).unwrap(), serde_json::json!(kind.id()));
            assert_eq!(GeneratorKind::parse(kind.id()), Some(kind));
            assert_eq!(GeneratorKind::parse(kind.label()), Some(kind));
        }
        assert!(!GeneratorKind::ThinAreas.in_picker(), "Thin areas only loads from saved projects and smart masks");
        assert_eq!(GeneratorKind::parse("Light / direction"), Some(GeneratorKind::Direction), "earlier labels still parse");
    }

    #[test]
    fn wear_generators_saved_before_their_wear_controls_load_on_the_earlier_model() {
        let mut saved = serde_json::to_value(Generator::new(GeneratorKind::EdgeWear)).unwrap();
        assert!(saved.get("wear").is_some(), "new wear generators have wear controls");

        saved.as_object_mut().unwrap().remove("wear");
        let loaded: Generator = serde_json::from_value(saved).unwrap();
        assert_eq!(loaded.wear, None, "level, softness and noise keep driving it");
        let curvature = serde_json::to_value(Generator::new(GeneratorKind::Curvature)).unwrap();
        assert!(curvature.get("wear").is_none(), "other kinds save no wear controls");
    }

    #[test]
    fn generators_saved_before_their_section_or_light_controls_load_on_the_earlier_model() {
        for kind in [GeneratorKind::Curvature, GeneratorKind::AmbientOcclusion, GeneratorKind::Position, GeneratorKind::Direction] {
            let mut saved = serde_json::to_value(Generator::new(kind)).unwrap();
            let key = if kind == GeneratorKind::Direction { "light" } else { "section" };
            assert!(saved.get(key).is_some(), "new {kind:?} generators have the reference's controls");

            saved.as_object_mut().unwrap().remove(key);
            let loaded: Generator = serde_json::from_value(saved).unwrap();
            assert!(loaded.on_earlier_model(), "{kind:?}: level, softness and noise keep driving it");
            assert_eq!(loaded.applied_global(), loaded.global, "{kind:?} keeps its global adjustments");
        }

        let mut saved = serde_json::to_value(Generator::new(GeneratorKind::WorldSpaceNormals)).unwrap();
        saved.as_object_mut().unwrap().remove("section");
        let loaded: Generator = serde_json::from_value(saved).unwrap();
        assert_eq!(loaded.section_controls(), SectionParameters::defaults(GeneratorKind::WorldSpaceNormals), "it has no earlier model");
    }

    #[test]
    fn curvature_generators_saved_before_detail_sizes_load_without_them() {
        let mut saved = serde_json::to_value(Generator::new(GeneratorKind::Curvature)).unwrap();
        assert!(saved.get("detail").is_some(), "new curvature generators have detail sizes");

        saved.as_object_mut().unwrap().remove("detail");
        let loaded: Generator = serde_json::from_value(saved).unwrap();
        assert_eq!(loaded.detail, None);
        assert_eq!(Generator::new(GeneratorKind::Dirt).detail, None, "only curvature and edge wear generators have detail sizes");
    }

    #[test]
    fn generators_saved_before_the_new_settings_load_and_save_unchanged() {
        let saved = serde_json::to_value(Generator::new(GeneratorKind::Dirt)).unwrap();
        for field in ["global", "resolution", "inputs"] {
            assert!(saved.get(field).is_none(), "a default `{field}` is not written, so saved files stay as they were");
        }

        let loaded: Generator = serde_json::from_value(saved).unwrap();
        assert!(loaded.global.is_neutral());

        let mut blurred = Generator::new(GeneratorKind::Dirt);
        blurred.global.blur = 0.5;
        let round_trip: Generator = serde_json::from_value(serde_json::to_value(&blurred).unwrap()).unwrap();
        assert_eq!(round_trip, blurred);
    }

    #[test]
    fn levels_map_range_and_gamma() {
        let l = Levels { in_low: 0.2, in_high: 0.6, clamp: true, ..Levels::default() };
        assert!((l.apply(0.4) - 0.5).abs() < 1e-6);
        assert_eq!(l.apply(0.0), 0.0);
        assert_eq!(l.apply(1.0), 1.0);

        let g = Levels { gamma: 2.0, ..Levels::default() };
        assert!((g.apply(0.25) - 0.5).abs() < 1e-6);

        // Degenerate ranges never divide by zero
        let d = Levels { in_low: 0.5, in_high: 0.5, ..Levels::default() };
        assert!(d.apply(0.7).is_finite());
    }

    #[test]
    fn unclamped_levels_carry_on_past_the_input_handles() {
        let clamped = Levels { in_low: 0.2, in_high: 0.6, clamp: true, ..Levels::default() };
        let open = Levels { clamp: false, ..clamped };
        assert_eq!(clamped.apply(0.0), 0.0);
        assert!((open.apply(0.0) + 0.5).abs() < 1e-6, "0.2 below the black handle is half a range below black");
        assert!((open.apply(0.8) - 1.5).abs() < 1e-6);
        assert!((open.apply(0.4) - clamped.apply(0.4)).abs() < 1e-6, "between the handles both remap alike");

        let narrow = Levels { in_high: 0.000005, gamma: 2.0, ..Levels::default() };
        assert!(narrow.apply(0.0000075) > narrow.out_high);

        let bent = Levels { gamma: 2.0, ..open };
        assert!((bent.apply(0.0) + 0.5).abs() < 1e-6, "gamma bends only the part between the handles");
    }

    #[test]
    fn fills_and_effects_in_the_shape_older_projects_saved_save_unchanged() {
        let fills = [
            r#"{"sources":{"BaseColor":{"Uniform":[0.5,0.5,0.5]}},"projection":{"Uv":{"scale":[1.0,1.0],"rotation":0.0,"offset":[0.0,0.0]}}}"#,
            r#"{"sources":{},"projection":{"Triplanar":{"scale":1.0,"sharpness":0.5,"offset":[0.0,0.0,0.0]}}}"#,
            r#"{"sources":{},"projection":{"Planar":{"scale":2.0,"axis":2,"offset":[0.0,1.0,0.0]}}}"#,
            r#"{"sources":{},"projection":{"Spherical":{"scale":1.0}}}"#,
            r#"{"sources":{},"projection":{"Cylindrical":{"scale":1.0,"axis":1}}}"#,
            r#"{"sources":{},"projection":{"Decal":{"center":[0.0,0.0,0.0],"normal":[0.0,0.0,1.0],"up":[0.0,1.0,0.0],"size":[1.0,1.0],"rotation":0.0,"offset":[0.0,0.0],"depth":1.0}}}"#,
        ];
        for saved in fills {
            let loaded: Fill = serde_json::from_str(saved).unwrap();

            assert_eq!(serde_json::to_string(&loaded).unwrap(), saved, "a fill saves as it was saved");
        }

        let effects = [
            r#"{"Fill":{"source":{"Uniform":[0.5,0.5,0.5]},"projection":{"Triplanar":{"scale":1.0,"sharpness":0.5,"offset":[0.0,0.0,0.0]}}}}"#,
            r#"{"Levels":{"in_low":0.2,"in_high":0.6,"gamma":1.0,"out_low":0.0,"out_high":1.0}}"#,
            r#"{"Compare":{"op":"GreaterThan","value":0.5,"tolerance":0.1}}"#,
            r#"{"ColorSelection":{"colors":[[1.0,0.0,0.0]],"tolerance":0.1}}"#,
            r#"{"Material":{"sources":{},"projection":{"Spherical":{"scale":2.0}}}}"#,
        ];
        for saved in effects {
            let loaded: EffectKind = serde_json::from_str(saved).unwrap();

            assert_eq!(serde_json::to_string(&loaded).unwrap(), saved, "an effect saves as it was saved");
        }
    }

    #[test]
    fn levels_saved_before_the_channel_and_clamp_load_as_they_rendered() {
        let saved = serde_json::json!({"in_low": 0.2, "in_high": 0.6, "gamma": 1.0, "out_low": 0.0, "out_high": 1.0});
        let loaded: Levels = serde_json::from_value(saved.clone()).unwrap();

        assert!(loaded.clamp, "they stopped at the output ends");
        assert_eq!(loaded.affects, None, "they remapped every channel");
        assert!(Channel::ALL.iter().all(|&channel| loaded.remaps(channel)));

        let saved_again = serde_json::to_value(loaded).unwrap();

        assert!(saved_again.get("affects").is_none(), "no channel is saved, as before: {saved_again}");
        assert!(saved_again.get("clamp").is_none(), "a clamped level is saved as before: {saved_again}");
        assert_eq!(serde_json::from_value::<Levels>(saved_again).unwrap(), loaded);

        let unclamped = serde_json::to_value(Levels { clamp: false, ..loaded }).unwrap();

        assert_eq!(serde_json::from_value::<Levels>(unclamped).unwrap(), Levels { clamp: false, ..loaded }, "an unclamped level keeps its choice");

        let base = Levels { affects: Some(Channel::BaseColor), ..loaded };
        assert!(base.remaps(Channel::BaseColor) && !base.remaps(Channel::Roughness));
    }

    #[test]
    fn hsl_adjust_shifts_hue_saturation_and_lightness() {
        let close = |a: [f32; 3], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4);
        let c = [0.8, 0.3, 0.1];
        assert!(close(hsl_adjust(c, 0.0, 0.0, 0.0), c), "identity");
        assert!(close(hsl_adjust([1.0, 0.0, 0.0], 1.0 / 3.0, 0.0, 0.0), [0.0, 1.0, 0.0]), "red → green");
        assert!(close(hsl_adjust([1.0, 0.0, 0.0], -1.0 / 3.0, 0.0, 0.0), [0.0, 0.0, 1.0]), "red → blue");

        let gray = hsl_adjust(c, 0.0, -1.0, 0.0);
        assert!((gray[0] - gray[1]).abs() < 1e-6 && (gray[1] - gray[2]).abs() < 1e-6);
        // Grays only change lightness, whatever the hue and saturation
        assert!(close(hsl_adjust([0.4; 3], 0.3, 1.0, 0.5), [0.7; 3]));
        assert!(close(hsl_adjust([0.4; 3], 0.0, 0.0, -0.5), [0.2; 3]));
        assert!(hsl_adjust([f32::NAN, 2.0, -1.0], f32::NAN, 5.0, -5.0).iter().all(|v| v.is_finite()));
    }

    #[test]
    fn an_untransformed_gradient_maps_bit_for_bit_as_projects_saved_before_did() {
        let colors = vec![[1.0, 0.2, 0.0], [0.1, 0.4, 0.9], [0.0, 0.0, 1.0]];
        let gm = Filter::even_gradient(colors.clone());
        for step in 0..=1000 {
            let x = step as f32 / 1000.0;
            let color = [x, 0.5 * x, 1.0 - x];
            assert_eq!(gm.point(ChannelKind::Color, color), gradient(&colors, lum(color)), "color {color:?}");

            let height = 2.0 * x - 1.0;
            let saved_before = per_unit(ChannelKind::Signed, [height; 3], |t| gradient(&colors, t).map_or(t, lum));
            assert_eq!(gm.point(ChannelKind::Signed, [height; 3]), Some(saved_before), "height {height}");
        }
    }

    #[test]
    fn point_filters_map_values_and_skip_neighbourhood_filters() {
        let c = ChannelKind::Color;
        let close = |a: [f32; 3], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4);
        let contrast = Filter::ContrastLuminosity { contrast: 1.0, luminosity: 0.0 };
        assert!(close(contrast.point(c, [0.25, 0.5, 0.75]).unwrap(), [0.0, 0.5, 1.0]));
        assert!(close(Filter::ContrastLuminosity { contrast: 0.0, luminosity: 0.2 }.point(ChannelKind::Scalar, [0.5; 3]).unwrap(), [0.7; 3]));

        let gradient_map = Filter::even_gradient(vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]);
        assert!(close(gradient_map.point(c, [0.0; 3]).unwrap(), [1.0, 0.0, 0.0]));
        assert!(close(gradient_map.point(c, [1.0; 3]).unwrap(), [0.0, 0.0, 1.0]));
        assert_eq!(Filter::even_gradient(vec![]).point(c, [0.3; 3]), None, "an empty gradient changes nothing");

        let gray_ramp = Filter::even_gradient(vec![[0.0; 3], [1.0; 3]]);
        assert!(close(gray_ramp.point(ChannelKind::Signed, [-0.6; 3]).unwrap(), [-0.6; 3]), "a height reads the ramp at its 0..1 place");
        assert!(close(Filter::Posterize { levels: 2.0, alpha: false }.point(c, [0.2, 0.6, 0.9]).unwrap(), [0.0, 1.0, 1.0]));
        assert!(close(Filter::Clamp { min: 0.8, max: 0.3 }.point(c, [0.1, 0.5, 0.9]).unwrap(), [0.3, 0.5, 0.8]), "swapped bounds never panic");

        let scan = |position, contrast| Filter::HistogramScan { position, contrast, invert_position: false }.point(c, [0.2, 0.5, 0.9]).unwrap();

        assert!(close(scan(0.5, 0.0), [0.2, 0.5, 0.9]), "a full-width ramp at the middle keeps the values");
        assert!(close(scan(0.4, 1.0), [0.0, 1.0, 1.0]), "full contrast is a step at the position");
        assert!(close(scan(0.5, 1.0), [0.0, 1.0, 1.0]), "the position itself goes to white");
        assert!(close(scan(0.7, 0.5), [0.0, 0.1, 0.9]));

        let range = |range, position| Filter::HistogramRange { range, position }.point(c, [0.2, 0.5, 0.9]).unwrap();

        assert!(close(range(1.0, 0.3), [0.2, 0.5, 0.9]) && close(range(0.0, 0.3), [0.3; 3]));
        assert!(close(range(0.5, 0.5), [0.35, 0.5, 0.7]));

        let shift = |offset| Filter::HistogramShift { offset }.point(c, [0.0, 0.5, 1.0]).unwrap();

        assert!(close(shift(0.0), [0.0, 0.5, 1.0]), "no offset keeps white white");
        assert!(close(shift(0.25), [0.25, 0.75, 0.25]) && close(shift(-0.25), [0.75, 0.25, 0.75]));

        let color_balance = Filter::ColorBalance { shadows: [0.2, 0.0, 0.0], midtones: [0.0; 3], highlights: [0.0, 0.0, 0.2] };
        assert!(color_balance.point(c, [0.05; 3]).unwrap()[0] > 0.2 && color_balance.point(c, [0.95; 3]).unwrap()[2] > 0.99);
        assert!(close(Filter::Invert.point(ChannelKind::Signed, [0.5; 3]).unwrap(), [-0.5; 3]));
        assert_eq!(Filter::Invert.point(ChannelKind::NormalMap, [0.5, 0.5, 1.0]), Some([0.5, 0.5, 1.0]));

        for f in [
            Filter::Blur { radius: 1.0, wrap: false },
            Filter::Warp { intensity: 1.0, scale: 1.0, seed: 0, divider: 1 },
            Filter::DirectionalBlur { radius: 1.0, angle: 0.0 },
        ] {
            assert!(f.is_neighbourhood() && f.point(c, [0.5; 3]).is_none());
        }
        let nan = Filter::ContrastLuminosity { contrast: f32::NAN, luminosity: f32::NAN };
        assert!(nan.point(c, [0.5; 3]).unwrap().iter().all(|v| v.is_finite()));
    }

    #[test]
    fn gradient_map_stops_round_trip_and_sources_without_them_save_as_before() {
        let ramp = vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let procedural = |ramp: Vec<[f32; 3]>| Source::Procedural { pattern: Procedural::new(ProceduralKind::Clouds), low: [0.25; 3], high: [0.75; 3], ramp };
        let anchor = |ramp: Vec<[f32; 3]>| Source::Anchor { anchor: LayerId(7), channel: Channel::BaseColor, ramp };
        for source in [procedural(ramp.clone()), anchor(ramp.clone())] {
            let json = serde_json::to_value(&source).unwrap();
            assert!(json.to_string().contains("\"ramp\""), "{json}");
            assert_eq!(serde_json::from_value::<Source>(json).unwrap(), source);
            assert_eq!(source.ramp(), ramp.as_slice());
        }

        // Empty ramps preserve older projects' JSON shape
        let old_procedural = serde_json::json!({"Procedural": {"pattern": serde_json::to_value(Procedural::new(ProceduralKind::Clouds)).unwrap(), "low": [0.25, 0.25, 0.25], "high": [0.75, 0.75, 0.75]}});
        let old_anchor = serde_json::json!({"Anchor": {"anchor": 7, "channel": "BaseColor"}});
        for (old, source) in [(old_procedural, procedural(Vec::new())), (old_anchor, anchor(Vec::new()))] {
            assert_eq!(serde_json::to_value(&source).unwrap(), old);
            assert_eq!(serde_json::from_value::<Source>(old).unwrap(), source);
            assert!(source.ramp().is_empty());
        }
        assert!(Source::Uniform([0.5; 3]).ramp().is_empty());
    }

    #[test]
    fn effect_targets_and_anchor_reads() {
        assert_eq!(EffectKind::Anchor.content_only(), None, "anchors go on content and masks");
        assert_eq!(EffectKind::Generator(Generator::new(GeneratorKind::Dirt)).content_only(), Some(false));
        assert_eq!(EffectKind::Filter(Filter::Invert).content_only(), None);

        let mut fill_layer = Layer::new(LayerId(1), "l", LayerKind::Paint, &[Channel::BaseColor]);
        assert!(!fill_layer.reads_anchor());

        let source = Source::Anchor { anchor: LayerId(9), channel: Channel::Height, ramp: Vec::new() };
        fill_layer.mask = Some(Mask {
            effects: vec![Effect::new(LayerId(2), EffectKind::Fill { source, projection: Projection::default(), sampling: Default::default() })],
            ..Mask::new(MaskBackground::Black)
        });

        assert!(fill_layer.reads_anchor());

        let reader = fill_layer.mask.as_ref().map(|mask| &mask.effects[0].kind);
        assert!(reader.is_some_and(|reader| reader.reads(Some(LayerId(9))) && reader.reads(None) && !reader.reads(Some(LayerId(8)))));

        // A generator whose Micro Height reads an anchor reads it too
        let mut generator_layer = Layer::new(LayerId(3), "g", LayerKind::Paint, &[Channel::BaseColor]);
        let mut generator = Generator::new(GeneratorKind::Dirt);
        generator.micro = MicroDetails::of_anchor(GeneratorKind::Dirt, LayerId(9), 0.5).map(Box::new);

        assert_eq!(EffectKind::Generator(generator.clone()).anchors_read(), [(LayerId(9), Channel::Height)]);
        assert!(EffectKind::Generator(generator.clone()).reads(Some(LayerId(9))) && !EffectKind::Generator(generator.clone()).reads(Some(LayerId(8))));

        generator_layer.mask = Some(Mask { effects: vec![Effect::new(LayerId(4), EffectKind::Generator(generator))], ..Mask::new(MaskBackground::Black) });

        assert!(generator_layer.reads_anchor());
    }

    #[test]
    fn micro_detail_saved_on_an_anchor_loads_as_micro_details_reading_its_height() {
        let mut saved = serde_json::to_value(Generator::new(GeneratorKind::Curvature)).unwrap();
        saved["micro"] = serde_json::json!({"anchor": 9, "amount": 0.25});

        let generator: Generator = serde_json::from_value(saved.clone()).unwrap();
        let micro = generator.micro.as_deref().unwrap();
        let defaults = MicroDetails::defaults(GeneratorKind::Curvature).unwrap();

        assert!(micro.enabled && micro.height, "{micro:?}");
        assert_eq!(micro.height_source, Some(Source::Anchor { anchor: LayerId(9), channel: Channel::Height, ramp: Vec::new() }));
        assert!(micro.height_intensity < defaults.height_intensity, "a weaker amount reads a gentler height");
        assert_eq!((micro.curvature_type, micro.ao_radius, micro.normal_source.clone()), (defaults.curvature_type, defaults.ao_radius, None));
        assert!(EffectKind::Generator(generator.clone()).reads(Some(LayerId(9))));

        let reloaded: Generator = serde_json::from_value(serde_json::to_value(&generator).unwrap()).unwrap();
        assert_eq!(reloaded, generator, "saved again as Micro Details");

        saved["kind"] = serde_json::to_value(GeneratorKind::ThinAreas).unwrap();
        let without: Generator = serde_json::from_value(saved).unwrap();
        assert!(without.micro.is_none(), "a kind without micro details drops it");
    }

    #[test]
    fn a_micro_detail_saved_on_an_anchor_keeps_its_earlier_reading_until_the_generator_leaves_the_earlier_model() {
        // Saved before the reference's controls too: on the earlier model
        let earlier = Generator { section: None, ..Generator::new(GeneratorKind::Curvature) };
        let mut saved = serde_json::to_value(earlier).unwrap();
        saved["micro"] = serde_json::json!({"anchor": 9, "amount": 0.25});
        let generator: Generator = serde_json::from_value(saved).unwrap();

        assert_eq!(generator.earlier_micro(), Some((LayerId(9), 0.25)));
        assert!(generator.micro_in_use().is_none(), "the earlier reading replaces the Micro Height's curvature");

        let reloaded: Generator = serde_json::from_value(serde_json::to_value(&generator).unwrap()).unwrap();
        assert_eq!(reloaded.earlier_micro(), Some((LayerId(9), 0.25)), "saving again keeps the earlier reading");

        let on_sections = Generator { section: SectionParameters::defaults(GeneratorKind::Curvature), ..generator };
        assert_eq!(on_sections.earlier_micro(), None);
        assert!(on_sections.micro_in_use().is_some(), "on the reference's controls the anchor reads as a Micro Height");
    }

    #[test]
    fn compare_masks_saved_before_sources_compare_the_mask_with_the_constant_hard() {
        let saved = r#"{"Compare":{"op":"GreaterThan","value":0.5,"tolerance":0.1}}"#;
        let kind: EffectKind = serde_json::from_str(saved).unwrap();
        let EffectKind::Compare { left, right, channel, hardness, .. } = kind else { panic!("{kind:?}") };
        assert_eq!((left, right, channel, hardness), (CompareSource::ThisMask, CompareSource::Constant, Channel::Height, 1.0));
        assert_eq!(serde_json::to_string(&kind).unwrap(), saved, "an unedited compare saves only its op, value and tolerance");

        let round_trip: EffectKind = serde_json::from_str(&serde_json::to_string(&kind).unwrap()).unwrap();

        assert_eq!(round_trip, kind);
    }

    #[test]
    fn a_hard_compare_is_the_old_step_and_a_soft_one_ramps() {
        // Hardness 1 keeps the strict comparisons compare masks always made
        for (left, right) in [(0.4, 0.5), (0.5, 0.5), (0.6, 0.5), (f32::NAN, 0.5)] {
            let step = |pass: bool| if pass { 1.0 } else { 0.0 };
            assert_eq!(CompareOp::GreaterThan.mask(left, right, 0.1, 1.0), step(left > right));
            assert_eq!(CompareOp::LesserThan.mask(left, right, 0.1, 1.0), step(left < right));
            assert_eq!(CompareOp::WithinTolerance.mask(left, right, 0.05, 1.0), step((left - right).abs() <= 0.05));
        }

        assert!((CompareOp::GreaterThan.mask(0.55, 0.5, 0.1, 0.9) - 0.5).abs() < 1e-5, "halfway up a 0.1 ramp");
        assert_eq!(CompareOp::GreaterThan.mask(0.45, 0.5, 0.1, 0.0), 0.0, "below the edge stays 0 however soft");
        assert_eq!(CompareOp::GreaterThan.mask(0.6, 0.5, 0.1, f32::NAN), 1.0, "a NaN hardness compares hard");
        assert!((CompareOp::WithinTolerance.mask(0.65, 0.5, 0.1, 0.9) - 0.5).abs() < 1e-5, "0.05 past the tolerance");
    }

    #[test]
    fn height_mask_keeps_the_ends_and_follows_height() {
        let off = HeightMask::default();
        assert_eq!(off.apply(0.3, 1.0), 0.3, "disabled");
        let hm = HeightMask { enabled: true, contrast: 1.0, offset: 0.0 };
        for h in [-1.0, 0.0, 1.0] {
            assert_eq!(hm.apply(0.0, h), 0.0);
            assert_eq!(hm.apply(1.0, h), 1.0);
        }
        assert_eq!(hm.apply(0.5, 0.0), 0.5, "no height, no change");
        assert_eq!(hm.apply(0.5, 0.5), 1.0);
        assert_eq!(hm.apply(0.5, -0.5), 0.0);
        assert!(hm.apply(0.5, 0.1) > 0.5 && hm.apply(0.5, -0.1) < 0.5);
        assert!(HeightMask { offset: 0.5, ..hm }.apply(0.5, 0.0) > 0.5);
        assert_eq!(hm.apply(0.5, f32::NAN), 0.5);
    }

    #[test]
    fn old_decals_load_unflipped_and_save_unchanged() {
        let old = r#"{"center":[0.0,0.0,0.0],"normal":[0.0,0.0,1.0],"up":[0.0,1.0,0.0],"size":[1.0,2.0],"rotation":0.0,"offset":[0.0,0.0],"depth":0.5}"#;
        let decal: Decal = serde_json::from_str(old).unwrap();
        assert_eq!(decal.flip, [false, false]);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&serde_json::to_string(&decal).unwrap()).unwrap(),
            serde_json::from_str::<serde_json::Value>(old).unwrap()
        );

        let flipped = Decal { flip: [true, false], ..decal };
        let saved = serde_json::to_string(&flipped).unwrap();
        assert!(saved.contains(r#""flip":[true,false]"#), "{saved}");
        assert_eq!(serde_json::from_str::<Decal>(&saved).unwrap(), flipped);
    }

    #[test]
    fn generator_kinds_parse() {
        for kind in GeneratorKind::ALL {
            assert_eq!(GeneratorKind::parse(&format!("{kind:?}")), Some(kind));
        }
    }

    #[test]
    fn procedural_kinds_parse_labels_and_saved_names() {
        for kind in ProceduralKind::ALL {
            let saved_name = format!("{kind:?}");

            assert_eq!(ProceduralKind::parse(kind.label()), Some(kind));
            assert_eq!(ProceduralKind::parse(&saved_name), Some(kind));
            assert_eq!(serde_json::to_value(kind).unwrap(), serde_json::Value::String(saved_name));
            assert_eq!(serde_json::from_value::<ProceduralKind>(serde_json::to_value(kind).unwrap()).unwrap(), kind);
        }
    }
}
