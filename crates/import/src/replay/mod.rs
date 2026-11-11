//! Stroke replay: the hand-off between the translator and the brush engine that paints imported strokes.
//!
//! The translator (this crate, below the brush engine in the layering) does not rasterise strokes. Where a layer's brush
//! work or a mask's paint effect can be replayed, it builds a [`ReplayJob`]: the stroke records (pen path, pressure,
//! camera and projection, symmetry, seed), the brush mapped onto neutral quantities with the measured rules applied
//! ([`brush`]), the tip ([`tip`]), the values painted per channel and the layer, content effect or mask effect the
//! pixels belong to. An engine that owns a brush engine implements [`StrokeReplayer`] and hands it to the import through
//! [`crate::Hooks::replay`]; the import calls it per texture set after the layer stack is built and before the safety
//! net composites, so the measured scores include the replayed pixels. Without a replayer, strokes stay placeholders
//! locked with `strokes:<tool>`.
//!
//! Paint effects that cannot be replayed (particles, ribbons, path strokes, projection painting without an image or on a
//! paint layer, brushes painting images or graphs, tips that are compiled graphs the project does not hold) stay
//! placeholders with a precise `brush:` or `brush-alpha:` reason.

pub mod brush;
pub mod tip;

use std::sync::Arc;

use texturecraft_core::{Channel, LayerId};
use texturecraft_geom::Mesh;
use texturecraft_texel::TileImage;

pub use brush::{check, step};

/// The words every replay note starts with in the import report.
pub const REPLAYED: &str = "strokes replayed with TextureCraft's brush";

/// How a step lays its strokes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintMode {
    /// Lays the step's values "over" what is there.
    Paint,
    /// Lowers the coverage of what is there.
    Erase,
    /// Moves what the job's earlier steps laid (values and coverage, on the step's channels): each dab draws what lies
    /// at the previous dab's place over its own.
    Smudge,
}

/// How a tip that is not round lies on the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipAlignment {
    /// On the dab's tangent plane, the view's right as its x.
    Tangent,
    /// Facing the view.
    Camera,
    /// Along the texture's axes.
    Uv,
}

/// A step's tip.
#[derive(Debug, Clone, PartialEq)]
pub enum ReplayTip {
    /// The round tip: full inside `hardness` of the radius, then a smoothstep to 0 at the rim (measured on the newer
    /// round shape; TextureCraft's round alpha is the same curve).
    Round { hardness: f32 },
    /// A grey image stretched over the dab's square (rows top first, `width × height` values 0..1).
    Image { width: u32, height: u32, values: Arc<[f32]> },
}

/// The brush of a step in neutral quantities.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayBrush {
    /// Dab radius in the mesh's units (the file's brush size is a diameter there; measured).
    pub radius: f32,
    /// Distance between dabs as a share of the diameter.
    pub spacing: f32,
    pub flow: f32,
    /// Lowest share of the size under pressure 0 (`None`: pressure does not change it): linear in the pressure.
    pub size_pressure: Option<f32>,
    /// The same for flow.
    pub flow_pressure: Option<f32>,
    /// Size and flow jitter, 0..1: a dab's size and flow are `1 − jitter·u`, `u` even in 0..1.
    pub size_jitter: f32,
    pub flow_jitter: f32,
    /// Position jitter in diameters: along both of the view's axes in a direction drawn per dab (`position_both`), or
    /// across the stroke; its distance drawn from a normal distribution (`position_normal`) or evenly in −1..1.
    pub position_jitter: f32,
    pub position_both: bool,
    pub position_normal: bool,
    /// The tip's turn, radians, clockwise on the screen; and its jitter, radians either way.
    pub rotation: f32,
    pub rotation_jitter: f32,
    /// The tip also turns with the stroke (its top ahead).
    pub follow: bool,
    pub alignment: TipAlignment,
    /// Culling: texels whose normal turns further than this share of a half turn from the normal at the dab's centre
    /// are not reached; a smooth fade 0.5 wide in the cosine around it (measured). `None`: no culling.
    pub culling: Option<f32>,
    pub tip: ReplayTip,
    /// A dynamic alpha's stamps: dab `k` of a stroke lays `cycle[(k + start) mod n]` instead of `tip`, `start` 0 or, with
    /// `cycle_drawn_start`, drawn from the stroke's seed. Empty: every dab lays `tip`.
    pub cycle: Vec<ReplayTip>,
    pub cycle_drawn_start: bool,
}

/// A stencil's grey image.
#[derive(Debug, Clone, PartialEq)]
pub enum StencilImage {
    /// One value everywhere, inside and outside the image.
    Uniform(f32),
    /// Rows top first, `width × height` values 0..1; nothing outside it.
    Image { width: u32, height: u32, values: Arc<[f32]> },
}

/// The stencil a step's strokes go through: a stroke's coverage times the image at the texel's place on the stroke's
/// view.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayStencil {
    pub image: StencilImage,
    /// Column by column, from the view's normalised device coordinates (y up) to the image's (0..1, y up).
    pub matrix: [f32; 9],
}

/// Where a stroke was painted.
#[derive(Debug, Clone, PartialEq)]
pub enum StrokeSpace {
    /// The 3D view through the stroke's camera: view and projection matrices, column-major, in the mesh's space.
    Mesh { view: [f32; 16], projection: [f32; 16] },
    /// The 2D view: points are texture coordinates (rows down, as the mesh's UVs).
    Uv,
}

/// One point of a stroke's path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokePoint {
    /// Normalised device coordinates (−1..1, y up) in the 3D view; texture coordinates in the 2D view.
    pub position: [f32; 2],
    /// Pen pressure 0..1 (1 for a mouse).
    pub pressure: f32,
}

/// One stroke as recorded.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayStroke {
    pub space: StrokeSpace,
    pub points: Vec<StrokePoint>,
    pub seed: u32,
    /// The lazy mouse's string as a share of the view's height (0: off).
    pub lazy: f32,
    /// Symmetry: `copies` further copies, each `copy_matrix` (column-major, mesh space) applied once more.
    pub copies: u32,
    pub copy_matrix: [f32; 16],
}

/// One paint effect of the file, ready to lay.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayStep {
    pub mode: PaintMode,
    /// Content targets: value per channel (TextureCraft's linear values) and the channel's own opacity.
    pub values: Vec<(Channel, [f32; 3], f32)>,
    /// Mask targets: the grey value laid.
    pub mask_value: f32,
    /// The effect's opacity: each stroke lays `opacity × coverage`.
    pub opacity: f32,
    pub brush: ReplayBrush,
    pub stencil: Option<ReplayStencil>,
    /// Projection painting (masks): the grey laid at each texel is this image's where the texel lies on the stroke's
    /// view, in place of `mask_value`.
    pub projection: Option<ReplayStencil>,
    pub strokes: Vec<ReplayStroke>,
    /// What the step leaves out or approximates, for the report.
    pub notes: Vec<String>,
}

/// Whose pixels a job paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayTarget {
    /// A paint layer's own pixels, or a content paint effect's (both keyed by their id).
    Content(LayerId),
    /// A mask's paint effect: grey value and coverage.
    Mask(LayerId),
}

impl ReplayTarget {
    pub fn id(self) -> LayerId {
        match self {
            ReplayTarget::Content(id) | ReplayTarget::Mask(id) => id,
        }
    }
}

/// Paint effects of the file laid in order into one target.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayJob {
    pub target: ReplayTarget,
    /// What the job stands for, for the report (the layer's path and the effect).
    pub label: String,
    pub steps: Vec<ReplayStep>,
}

impl ReplayJob {
    /// Strokes in the job.
    pub fn strokes(&self) -> usize {
        self.steps.iter().map(|s| s.strokes.len()).sum()
    }

    /// Every note of its steps, without repeats.
    pub fn notes(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for n in self.steps.iter().flat_map(|s| &s.notes) {
            if !out.contains(n) {
                out.push(n.clone());
            }
        }
        out
    }
}

/// What a replayer paints on: the imported mesh (in the file's normalised space, where the stroke cameras look at
/// it), the texture set's material slot and resolution.
pub struct ReplayScene<'a> {
    pub mesh: &'a Mesh,
    pub material: usize,
    pub resolution: u32,
}

/// The pixels a job painted, in the stores' layouts: per channel value and coverage (a paint layer's tiles), or a
/// mask effect's premultiplied grey and coverage.
#[derive(Debug, Clone)]
pub enum ReplayedPixels {
    Content(Vec<(Channel, TileImage)>),
    Mask(TileImage),
}

/// Paints replay jobs. Implemented where a brush engine is available (the engine); the import only calls it.
pub trait StrokeReplayer: Sync {
    /// Paints `jobs` on `scene`: one result per job, in order; an error says why a job could not be painted.
    fn replay(&self, scene: &ReplayScene, jobs: &[ReplayJob]) -> Vec<Result<ReplayedPixels, String>>;
}

/// The pressure timeline at `time`: linear between keys, held before the first and after the last; 1 without keys.
pub fn pressure_at(keys: &[texturecraft_import_reader::model::TimelineKey], time: f32) -> f32 {
    let after = keys.partition_point(|k| k.time <= time);
    let v = match (after.checked_sub(1).and_then(|i| keys.get(i)), keys.get(after)) {
        (None, None) => 1.0,
        (Some(k), None) | (None, Some(k)) => k.value,
        (Some(a), Some(b)) => {
            let span = b.time - a.time;
            if span > 0.0 { a.value + (b.value - a.value) * (time - a.time) / span } else { a.value }
        }
    };
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 }
}

#[cfg(test)]
mod tests;
