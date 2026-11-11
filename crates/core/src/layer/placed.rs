//! [`PlacedProjection`]: a projection with every placement setting spelled out: a volume placed, turned and sized in
//! the scene, the image's tiling, turn and offset inside it, repetition per image axis, and the sampling rules that go
//! with them (the tri-planar blend curve, depth and facing falloffs, a per-texel anisotropic prefilter).
//!
//! TextureCraft's own projections ([`super::Projection::Triplanar`] and the others) stay as they are; nothing made in
//! TextureCraft uses this one. A project import writes it for fills whose placement the plain projections cannot
//! hold, so they open as they were made. Every field has a default, and files written before it existed never
//! contain it.
//!
//! Units: a *scene unit* is the largest extent of the whole mesh's bounding box. A volume of size 1 is a cube one
//! scene unit wide centred on the box's centre (so it encloses the mesh), moved by `offset` scene units and turned by
//! `rotation`. Inside it, local coordinates run −½ … ½ on each axis; the mode turns them into image coordinates
//! (0 … 1 once across the image, v down) and the [`ImagePlacement`] is applied last.

use serde::{Deserialize, Serialize};

/// A projection placed in a volume of the scene (or on the UVs) with the settings listed in the module notes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlacedProjection {
    pub mode: PlacedMode,
    #[serde(default)]
    pub volume: Volume,
    #[serde(default)]
    pub image: ImagePlacement,
    #[serde(default)]
    pub repeat: Repeat,
}

/// How a [`PlacedProjection`] maps the surface into the image.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PlacedMode {
    /// The mesh's texture coordinates placed as [`super::Projection::Uv`] (`uv · R(rotation) · scale + offset`); the
    /// volume and image placement are not used. Exists for the per-axis repetition.
    Uv { scale: [f32; 2], rotation: f32, offset: [f32; 2] },
    /// Three planes along the volume's axes, each seen from the side the normal faces (never mirrored), blended by the
    /// local normal: each plane's share is `smoothstep(½ + ½·hardness, 1, |n_i| / max|n|)`, normalised. Hardness 0
    /// still blends only between about 28° and 62° from an axis, 1 keeps the plane the normal faces most. Always
    /// repeats.
    TriPlanar { hardness: f32 },
    /// Seen from the volume's +z, looking along −z: image `(x + ½, ½ − y)`; a depth falloff along local z and a facing
    /// falloff on the normal against +z.
    Planar {
        #[serde(default)]
        depth: Falloff,
        #[serde(default)]
        facing: FacingFalloff,
    },
    /// Longitude from local +z towards +x across the image, latitude down from the top at +y.
    Spherical,
    /// Around the local y axis: the angle from +z across `coverage` degrees of the image, `½ − y` down; a facing falloff
    /// on the radial direction, caps (faces along the axis) optionally hidden.
    Cylindrical {
        #[serde(default)]
        facing: FacingFalloff,
        /// Degrees of the circumference the image covers (360: all of it); outside nothing is covered.
        coverage: f32,
        #[serde(default)]
        hide_caps: bool,
    },
    /// The image laid on a smooth surface through a grid of anchors with Bézier handles inside the volume, projected
    /// along the grid's z up to its depth on either side, faded by the depth falloff over that depth.
    Warp {
        #[serde(default)]
        depth: Falloff,
        grid: WarpGrid,
    },
}

/// The grid of a warp: `rows × columns` anchors row by row (row 0 at the top), in grid units (−1 … 1 across the
/// volume). No points: the regular grid, which shows the image as the planar projection would.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarpGrid {
    pub rows: u32,
    pub columns: u32,
    /// How far the projection reaches on either side of the surface (grid units).
    pub depth: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<WarpPoint>,
}

/// One anchor of a [`WarpGrid`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarpPoint {
    /// Position in grid units.
    pub pos: [f32; 3],
    /// The image coordinate shown there (v down).
    pub uv: [f32; 2],
    /// Handles as offsets from the anchor, towards the previous and next anchor along a row, then along a column; a zero
    /// handle is automatic (a third of the way towards the neighbour, smoothed through the anchor).
    pub tangents: [[f32; 3]; 4],
}

/// The box a volume projection works in (see the module notes for the units).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Volume {
    /// Centre offset from the bounding box's centre, in scene units.
    pub offset: [f32; 3],
    /// Degrees about the fixed x, y and z axes, applied in that order (`R = Rz·Ry·Rx`).
    pub rotation: [f32; 3],
    /// Width along each local axis in scene units (negative mirrors the axis).
    pub size: [f32; 3],
    /// Cover nothing outside the volume instead of continuing beyond it.
    #[serde(default)]
    pub clip: bool,
}

impl Default for Volume {
    fn default() -> Volume {
        Volume { offset: [0.0; 3], rotation: [0.0; 3], size: [1.0; 3], clip: false }
    }
}

/// Where the image sits on the projection's coordinates: tiled per axis (negative mirrors), turned counter-clockwise
/// by `rotation` degrees about the centre, moved by `offset` (image units, applied before tiling).
/// `image = tiling · R(−rotation) · (p − ½ − offset) + ½`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImagePlacement {
    pub tiling: [f32; 2],
    pub rotation: f32,
    pub offset: [f32; 2],
}

impl Default for ImagePlacement {
    fn default() -> ImagePlacement {
        ImagePlacement { tiling: [1.0, 1.0], rotation: 0.0, offset: [0.0, 0.0] }
    }
}

/// Along which image axes the image repeats. Outside a non-repeating axis's 0 … 1 the fill covers nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Repeat {
    None,
    U,
    V,
    #[default]
    Both,
}

impl Repeat {
    /// Whether u and v repeat.
    pub fn axes(self) -> [bool; 2] {
        [matches!(self, Repeat::U | Repeat::Both), matches!(self, Repeat::V | Repeat::Both)]
    }
}

/// A fade along the projection depth: full within `hardness` × half the depth, a smooth step to nothing at the
/// volume's face (a hard cut at hardness 1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Falloff {
    pub on: bool,
    pub hardness: f32,
}

impl Default for Falloff {
    fn default() -> Falloff {
        Falloff { on: false, hardness: 0.5 }
    }
}

/// A cut-off on faces turned away from the projection direction: full up to `angle` degrees less a band, nothing
/// beyond it plus the band; the band is `(1 − hardness)` × the distance from the angle to 0° or 180°, whichever is
/// nearer.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FacingFalloff {
    pub on: bool,
    pub angle: f32,
    pub hardness: f32,
}

impl Default for FacingFalloff {
    fn default() -> FacingFalloff {
        FacingFalloff { on: false, angle: 90.0, hardness: 0.5 }
    }
}

impl PlacedProjection {
    /// The repetition the image is sampled with: tri-planar always repeats.
    pub fn sampling_repeat(&self) -> Repeat {
        match self.mode {
            PlacedMode::TriPlanar { .. } => Repeat::Both,
            _ => self.repeat,
        }
    }

    /// True when the mode works in the volume (not on the UVs).
    pub fn is_volume(&self) -> bool {
        !matches!(self.mode, PlacedMode::Uv { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_fill_left_out_fields() {
        let p: PlacedProjection = serde_json::from_str(r#"{"mode":{"TriPlanar":{"hardness":0.5}}}"#).unwrap();
        assert_eq!(p.volume, Volume::default());
        assert_eq!(p.image, ImagePlacement::default());
        assert_eq!(p.repeat, Repeat::Both);
        let planar: PlacedMode = serde_json::from_str(r#"{"Planar":{}}"#).unwrap();
        assert_eq!(planar, PlacedMode::Planar { depth: Falloff::default(), facing: FacingFalloff::default() });
    }

    #[test]
    fn tri_planar_always_repeats() {
        let p = PlacedProjection {
            mode: PlacedMode::TriPlanar { hardness: 0.5 },
            volume: Volume::default(),
            image: ImagePlacement::default(),
            repeat: Repeat::None,
        };
        assert_eq!(p.sampling_repeat(), Repeat::Both);
        assert_eq!(Repeat::U.axes(), [true, false]);
        assert!(p.is_volume());
    }
}
