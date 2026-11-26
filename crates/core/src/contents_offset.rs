//! A folder's Scale and Rotation for the fills inside it

use serde::{Deserialize, Serialize};

use crate::layer::Projection;

/// Scale and Rotation a folder lays on every fill inside it, on top of each fill's own: the tiling is multiplied by `scale` and `rotation` (degrees) is added to the fill's angle; the fills themselves never change, so dropping the offset gives each its own back
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContentsOffset {
    pub scale: f32,
    pub rotation: f32,
}

impl Default for ContentsOffset {
    /// Nothing changed, as in every folder saved before folders had an offset
    fn default() -> Self {
        ContentsOffset::NONE
    }
}

impl ContentsOffset {
    /// The offset that changes nothing
    pub const NONE: ContentsOffset = ContentsOffset { scale: 1.0, rotation: 0.0 };

    /// The smallest and largest Scale a folder takes
    pub const SCALE_RANGE: (f32, f32) = (0.01, 100.0);

    /// The Rotation a folder takes, in degrees: a half turn each way
    pub const ROTATION_RANGE: (f32, f32) = (-180.0, 180.0);

    /// Whether the offset changes nothing
    pub fn is_none(&self) -> bool {
        *self == ContentsOffset::NONE
    }

    /// An offset with `scale` kept in [`ContentsOffset::SCALE_RANGE`] and `rotation` wrapped into [`ContentsOffset::ROTATION_RANGE`] (non-finite values change nothing)
    pub fn new(scale: f32, rotation: f32) -> ContentsOffset {
        let (low, high) = ContentsOffset::SCALE_RANGE;
        let scale = if scale.is_finite() { scale.clamp(low, high) } else { 1.0 };

        let (turn_start, turn_end) = ContentsOffset::ROTATION_RANGE;
        let rotation = if rotation.is_finite() { (rotation - turn_start).rem_euclid(turn_end - turn_start) + turn_start } else { 0.0 };

        ContentsOffset { scale, rotation }
    }

    /// The offset of a folder inside one with offset `self` (`inner` is the inner folder's own): scales multiply, rotations add, and the result stays in range however deep folders nest
    pub fn then(self, inner: ContentsOffset) -> ContentsOffset {
        ContentsOffset::new(self.scale * inner.scale, self.rotation + inner.rotation)
    }

    /// `projection` with this offset laid on its tiling and rotation (a projection stretched once over each UV tile has neither)
    pub fn apply(self, projection: &Projection) -> Projection {
        // A loaded offset never went through `new`
        let offset = ContentsOffset::new(self.scale, self.rotation);
        let mut out = projection.clone();

        match &mut out {
            Projection::Uv { scale, rotation, .. } => {
                *scale = scale.map(|s| s * offset.scale);
                *rotation += offset.rotation;
            }
            Projection::UvTile => {}
            Projection::Triplanar { uv, .. }
            | Projection::Planar { uv, .. }
            | Projection::Spherical { uv, .. }
            | Projection::Cylindrical { uv, .. }
            | Projection::Warp { uv, .. } => {
                uv.tiling = uv.tiling.map(|t| t * offset.scale);
                uv.rotation += offset.rotation;
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::UvTransform;

    #[test]
    fn an_offset_multiplies_the_tiling_and_adds_to_the_angle_of_every_projection() {
        let offset = ContentsOffset::new(1.5, 30.0);
        let uv = offset.apply(&Projection::Uv { scale: [2.0, 4.0], rotation: 10.0, offset: [0.25, 0.0] });

        assert_eq!(uv, Projection::Uv { scale: [3.0, 6.0], rotation: 40.0, offset: [0.25, 0.0] });

        let spherical = Projection::Spherical {
            scale: 1.0,
            uv: UvTransform { tiling: [2.0, 1.0], rotation: -5.0, offset: [0.0; 2] },
            offset: [0.0; 3],
            projector: Default::default(),
        };
        let Projection::Spherical { scale, uv, .. } = offset.apply(&spherical) else { panic!("expected a spherical projection") };

        assert_eq!((scale, uv.tiling, uv.rotation), (1.0, [3.0, 1.5], 25.0));
        assert_eq!(offset.apply(&Projection::UvTile), Projection::UvTile);
        assert_eq!(ContentsOffset::NONE.apply(&spherical), spherical);
    }

    #[test]
    fn nested_offsets_combine_and_hostile_values_are_kept_in_range() {
        let both = ContentsOffset::new(2.0, 170.0).then(ContentsOffset::new(1.5, 20.0));

        assert_eq!((both.scale, both.rotation), (3.0, -170.0));
        assert_eq!(ContentsOffset::new(f32::NAN, f32::INFINITY), ContentsOffset::NONE);
        assert_eq!(ContentsOffset::new(1e9, 190.0), ContentsOffset { scale: 100.0, rotation: -170.0 });
        assert_eq!(ContentsOffset::new(-3.0, -180.0), ContentsOffset { scale: 0.01, rotation: -180.0 });
        assert!(ContentsOffset::default().is_none());
    }

    #[test]
    fn deep_nesting_and_loaded_offsets_keep_the_scale_in_range() {
        let deep = (0..17).fold(ContentsOffset::NONE, |around, _| around.then(ContentsOffset::new(100.0, 0.0)));

        assert_eq!(deep.scale, ContentsOffset::SCALE_RANGE.1);

        let loaded: ContentsOffset = serde_json::from_str(r#"{"scale":0,"rotation":0}"#).unwrap();
        let tiled = loaded.apply(&Projection::Uv { scale: [2.0, 2.0], rotation: 0.0, offset: [0.0; 2] });

        assert_eq!(tiled, Projection::Uv { scale: [0.02, 0.02], rotation: 0.0, offset: [0.0; 2] }, "a loaded Scale of 0 is kept at the smallest");
    }
}
