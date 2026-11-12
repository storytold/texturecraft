//! Stroke records: the brush work behind the pixels of a paint layer, a content paint effect or a mask paint effect,
//! kept so that the strokes can be laid again (replayed, re-projected, painted at another resolution) instead of
//! existing only as pixels.
//!
//! A [`StrokeRecord`] holds what TextureCraft's brush needs to lay the strokes again: per paint effect laid into the
//! target ([`StrokeStep`]) the pen's strokes as recorded (points, pressure, camera, seed, symmetry), the brush in
//! neutral quantities with its tip, the values painted and how. The pixels stay what is shown; a record never changes
//! them by itself.
//!
//! Records are saved next to the pixels they belong to (one `strokes/<id>.bin` entry per record in the project file),
//! keyed by the id that keys the pixels: a paint layer's id, a content paint effect's id or a mask paint effect's id.
//! A record that cannot be read (damaged, or written by a newer version) is kept as its bytes ([`StoredStrokes`]) and
//! written back unchanged, so opening and saving a project never loses one.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{Channel, GraphRef};

/// Version of the record layout written by this build. Records of another version stay [`StoredStrokes::Unreadable`].
pub const STROKE_RECORD_VERSION: u32 = 1;

/// A stroke record as it sits in a document: read, or kept as the bytes that could not be read.
#[derive(Debug, Clone, PartialEq)]
pub enum StoredStrokes {
    Record(Arc<StrokeRecord>),
    Unreadable(UnreadableStrokes),
}

impl StoredStrokes {
    /// The record, if it could be read.
    pub fn record(&self) -> Option<&StrokeRecord> {
        match self {
            StoredStrokes::Record(r) => Some(r),
            StoredStrokes::Unreadable(_) => None,
        }
    }
}

impl From<StrokeRecord> for StoredStrokes {
    fn from(r: StrokeRecord) -> StoredStrokes {
        StoredStrokes::Record(Arc::new(r))
    }
}

/// A record that could not be read, with why. Its bytes are written back as they were.
#[derive(Debug, Clone, PartialEq)]
pub struct UnreadableStrokes {
    pub bytes: Arc<Vec<u8>>,
    pub reason: String,
}

/// The strokes laid into one target, in the order they were painted.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StrokeRecord {
    pub target: StrokeTarget,
    /// What the strokes stand for, for labels (the layer's path and the effect).
    pub label: String,
    /// The paint effects laid into the target, bottom first.
    pub steps: Vec<StrokeStep>,
}

impl StrokeRecord {
    /// Strokes over every step, path strokes included.
    pub fn stroke_count(&self) -> usize {
        self.steps.iter().map(|s| s.strokes.len() + s.paths.len()).sum()
    }

    /// The graphs the record's steps paint, for keeping their texts with the record.
    pub fn graphs(&self) -> impl Iterator<Item = &GraphRef> {
        self.steps.iter().flat_map(|s| s.sources.iter().map(|(_, src, _)| src).chain(&s.mask_source)).filter_map(|s| match s {
            StrokeSource::Graph(g) => Some(&**g),
            StrokeSource::NotKept(_) => None,
        })
    }
}

/// Whose pixels a record's strokes paint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StrokeTarget {
    /// A paint layer's or content paint effect's channels.
    #[default]
    Content,
    /// A mask paint effect's grey value and coverage.
    Mask,
}

/// How a step lays its strokes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrokeMode {
    /// Lays the step's values "over" what is there.
    Paint,
    /// Lowers the coverage of what is there.
    Erase,
    /// Moves what the record's earlier steps laid along the strokes.
    Smudge,
}

/// One paint effect: its strokes and the brush and values they were laid with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrokeStep {
    pub mode: StrokeMode,
    /// Content targets: value per channel (linear) and the channel's own opacity.
    pub values: Vec<(Channel, [f32; 3], f32)>,
    /// Content targets painted with a graph or an image instead of a value, with the channel's own opacity.
    pub sources: Vec<(Channel, StrokeSource, f32)>,
    /// Mask targets: the grey value laid.
    pub mask_value: f32,
    /// Mask targets painted with a graph's grey output instead of `mask_value`.
    pub mask_source: Option<StrokeSource>,
    /// The effect's opacity: each stroke lays `opacity × coverage`.
    pub opacity: f32,
    pub brush: StrokeBrush,
    pub stencil: Option<StrokeStencil>,
    /// Projection painting on masks: the grey laid is this image's where the texel lies on the stroke's view.
    pub projection: Option<StrokeStencil>,
    /// Projection painting on content: where each channel's source is read on the stroke's view (column by column,
    /// from the view's normalised device coordinates to the image's).
    pub placed: Option<[f32; 9]>,
    pub strokes: Vec<RecordedStroke>,
    /// Path strokes (curves on the mesh), laid after `strokes`.
    pub paths: Vec<PathStroke>,
    /// A ribbon brush: the tip laid along each path as a band.
    pub ribbon: Option<StrokeRibbon>,
    /// A particle brush: each stroke lays the dabs of a particle simulation spawned along it (from the stroke's seed)
    /// instead of its own.
    pub particles: Option<StrokeParticles>,
}

/// A particle brush's simulation settings, as TextureCraft's particle simulation takes them. Lengths mesh units, times
/// seconds, angles radians.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StrokeParticles {
    pub per_point: f32,
    pub per_length: f32,
    pub budget: f32,
    pub speed: f32,
    pub speed_rand: f32,
    pub life: f32,
    pub life_rand: f32,
    pub gravity: f32,
    pub friction: f32,
    pub wander: f32,
    pub curl: f32,
    pub fan: f32,
    pub heading: f32,
    pub away: bool,
    pub kink_time: f32,
    pub kink_angle: f32,
    pub kink_branches: f32,
    pub kink_speed: f32,
    pub stop_speed: f32,
    pub view_bias: f32,
    pub spread: f32,
    pub scatter: f32,
    pub fade_size: f32,
    pub fade_size_rand: f32,
    pub fade_flow: f32,
    pub fade_flow_rand: f32,
    pub size: f32,
    pub flow: f32,
    pub step: f32,
    /// `speed` is still in the units of the mesh file the source project imported (the import converts it before it
    /// records, so its records hold `false`).
    pub file_units: bool,
}

/// What a brush paints per texel instead of a uniform value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StrokeSource {
    /// A graph, rendered before the strokes are laid. Its text is kept in the project like a layer's graph.
    Graph(#[serde(with = "graph_text")] Box<GraphRef>),
    /// A source the record does not keep (says which); the step cannot be laid again from the record.
    NotKept(String),
}

/// The brush of a step in neutral quantities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrokeBrush {
    /// Dab radius in the mesh's units.
    pub radius: f32,
    /// Distance between dabs as a share of the diameter.
    pub spacing: f32,
    pub flow: f32,
    /// Lowest share of the size under pressure 0 (`None`: pressure does not change it).
    pub size_pressure: Option<f32>,
    /// The same for flow.
    pub flow_pressure: Option<f32>,
    /// Size and flow jitter, 0..1.
    pub size_jitter: f32,
    pub flow_jitter: f32,
    /// Position jitter in diameters, along both of the view's axes (`position_both`) or across the stroke; drawn from
    /// a normal distribution (`position_normal`) or evenly.
    pub position_jitter: f32,
    pub position_both: bool,
    pub position_normal: bool,
    /// The tip's turn, radians, clockwise on the screen; and its jitter, radians either way.
    pub rotation: f32,
    pub rotation_jitter: f32,
    /// The tip also turns with the stroke.
    pub follow: bool,
    pub alignment: StrokeTipAlignment,
    /// Culling: share of a half turn from the dab's normal past which texels are not reached. `None`: no culling.
    pub culling: Option<f32>,
    pub tip: StrokeTip,
    /// A cycle of stamps laid dab by dab instead of `tip`; empty: every dab lays `tip`.
    pub cycle: Vec<StrokeTip>,
    /// The cycle starts at a stamp drawn from the stroke's seed (else at the first).
    pub cycle_drawn_start: bool,
    /// A path brush's up direction (mesh space). `None`: the tip lies along the path.
    pub up: Option<[f32; 3]>,
}

/// How a tip that is not round lies on the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrokeTipAlignment {
    /// On the dab's tangent plane, the view's right as its x.
    Tangent,
    /// Facing the view.
    Camera,
    /// Along the texture's axes.
    Uv,
}

/// A brush tip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StrokeTip {
    /// The round tip: full inside `hardness` of the radius, then a smoothstep to 0 at the rim.
    Round { hardness: f32 },
    /// A grey image stretched over the dab's square (rows top first, `width × height` values 0..1).
    Image { width: u32, height: u32, values: Vec<f32> },
}

/// A grey image a step's strokes go through, placed on the stroke's view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrokeStencil {
    pub image: StencilValues,
    /// Column by column, from the view's normalised device coordinates (y up) to the image's (0..1, y up).
    pub matrix: [f32; 9],
}

/// A stencil's grey image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StencilValues {
    /// One value everywhere.
    Uniform(f32),
    /// Rows top first, `width × height` values 0..1; nothing outside it.
    Image { width: u32, height: u32, values: Vec<f32> },
}

/// Where a stroke was painted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StrokeSpace {
    /// The 3D view through the stroke's camera: view and projection matrices, column-major, in the mesh's space.
    Mesh { view: [f32; 16], projection: [f32; 16] },
    /// The 2D view: points are texture coordinates.
    Uv,
}

/// One point of a stroke's path.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StrokePoint {
    /// Normalised device coordinates (−1..1, y up) in the 3D view; texture coordinates in the 2D view.
    pub position: [f32; 2],
    /// Pen pressure 0..1 (1 for a mouse).
    pub pressure: f32,
}

/// One stroke as the pen made it: pen down, the points, pen up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordedStroke {
    pub space: StrokeSpace,
    pub points: Vec<StrokePoint>,
    /// Seed of the brush's random variation, so the same dabs are laid again.
    pub seed: u32,
    /// The lazy mouse's string as a share of the view's height (0: off).
    pub lazy: f32,
    /// Symmetry: `copies` further copies, each `copy_matrix` (column-major, mesh space) applied once more.
    pub copies: u32,
    pub copy_matrix: [f32; 16],
}

/// One cubic piece of a path stroke (mesh space), with the surface normal and the anchors' size at its ends.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PathPiece {
    pub points: [[f32; 3]; 4],
    pub normals: [[f32; 3]; 2],
    pub size: [f32; 2],
}

/// A path stroke: a curve on the mesh, no view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathStroke {
    pub pieces: Vec<PathPiece>,
    /// A path of one anchor: its position, normal and size.
    pub anchor: Option<([f32; 3], [f32; 3], f32)>,
    pub seed: u32,
    pub copies: u32,
    pub copy_matrix: [f32; 16],
}

/// How a ribbon places its tip along a path.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RibbonTiling {
    /// Whole copies, as many as fit best, stretched to fill the path.
    Repeat,
    /// One copy stretched over the whole path.
    Whole,
    /// One copy over the whole path, its ends before `guides[0]` and after `guides[1]` kept, the middle stretched.
    Caps { guides: [f32; 2] },
}

/// A ribbon brush.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StrokeRibbon {
    /// The band's full width, mesh units.
    pub width: f32,
    /// One copy's length before fitting.
    pub copy: f32,
    pub tiling: RibbonTiling,
    /// How far from the path along the surface's normal the band reaches.
    pub depth: f32,
}

/// A graph reference kept as its JSON text inside a record, so that reading one is bounded the way `project.json` is
/// (nested graphs cannot exhaust the stack).
mod graph_text {
    use super::GraphRef;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(g: &GraphRef, s: S) -> Result<S::Ok, S::Error> {
        let text = serde_json::to_string(g).map_err(serde::ser::Error::custom)?;
        s.serialize_str(&text)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Box<GraphRef>, D::Error> {
        let text = String::deserialize(d)?;
        serde_json::from_str(&text).map(Box::new).map_err(serde::de::Error::custom)
    }
}
