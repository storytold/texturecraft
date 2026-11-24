//! How long one repeat of a fill's tile is on the mesh (mesh units): Physical size reads it from the tiling and writes tiling back

use std::f32::consts::TAU;

use texturecraft_core::{FULL_TURN_DEGREES, Projection};

use crate::pattern::{MIN_CYLINDER_ANGLE, cylinder_axis_index, projector_stretch};

mod mesh_size;

pub use mesh_size::MeshSize;

/// Mesh units one tile covers across and down at a tiling of 1, the projector's stretch included; None without a physical size (per UV tile, spherical, or a stretch uneven across the planes a tile wraps round)
fn unit_tile(projection: &Projection, size: MeshSize) -> Option<[f32; 2]> {
    let radius = size.radius;
    match projection {
        Projection::Uv { .. } => Some([size.uv_density; 2]),
        Projection::Triplanar { scale, projector, .. } => {
            // Every plane repeats in the one length only while the stretch is even
            let stretch = projector_stretch(projector).abs();
            let even = stretch.x == stretch.y && stretch.y == stretch.z;
            even.then_some([radius * stretch.x / scale; 2])
        }
        Projection::Planar { placed: Some(frame), projector, .. } | Projection::Warp { frame, projector, .. } => {
            let stretch = projector_stretch(projector).abs();
            Some([frame.size[0] * stretch.x, frame.size[1] * stretch.y])
        }
        Projection::Planar { scale, axis, projector, .. } => {
            let stretch = projector_stretch(projector).abs();
            let [across, down] = match axis {
                0 => [stretch.z, stretch.y],
                1 => [stretch.x, stretch.z],
                _ => [stretch.x, stretch.y],
            };
            Some([2.0 * radius * across / scale, 2.0 * radius * down / scale])
        }
        Projection::Cylindrical { scale, axis, angle, projector, .. } => {
            let stretch = projector_stretch(projector).abs();
            let ([across, depth], height) = match cylinder_axis_index(*axis) {
                0 => ([stretch.y, stretch.z], stretch.x),
                2 => ([stretch.x, stretch.y], stretch.z),
                _ => ([stretch.x, stretch.z], stretch.y),
            };

            // Round an uneven cross-section the turn is no fixed length of surface
            if across != depth {
                return None;
            }

            let around = TAU * radius * across * (angle.max(MIN_CYLINDER_ANGLE) / FULL_TURN_DEGREES);
            Some([around / scale, 2.0 * radius * height / scale])
        }
        Projection::UvTile | Projection::Spherical { .. } => None,
    }
}

/// The tiling a physical size reads and writes: UV's own tiling, a 3D projection's UV tiling
fn tiling(projection: &Projection) -> Option<[f32; 2]> {
    match projection {
        Projection::Uv { scale, .. } => Some(*scale),
        Projection::Triplanar { uv, .. } | Projection::Planar { uv, .. } | Projection::Cylindrical { uv, .. } | Projection::Warp { uv, .. } => Some(uv.tiling),
        Projection::UvTile | Projection::Spherical { .. } => None,
    }
}

/// Mesh units one repeat of the tile covers across and down
pub fn tile_length(projection: &Projection, size: MeshSize) -> Option<[f32; 2]> {
    let unit = unit_tile(projection, size)?;
    let tiling = tiling(projection)?;
    Some([unit[0] / tiling[0].max(f32::EPSILON), unit[1] / tiling[1].max(f32::EPSILON)])
}

/// The tiling that makes one repeat `length` mesh units long across and down
pub fn tiling_for_length(projection: &Projection, size: MeshSize, length: [f32; 2]) -> Option<[f32; 2]> {
    let unit = unit_tile(projection, size)?;
    Some([unit[0] / length[0].max(f32::EPSILON), unit[1] / length[1].max(f32::EPSILON)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::{Coords, image_samples};
    use texturecraft_core::{Projector, UvTransform};
    use texturecraft_geom::glam::{Vec2, Vec3};

    const SIZE: MeshSize = MeshSize { radius: 2.0, uv_density: 3.0 };

    #[test]
    fn uv_tile_length_is_the_uv_density_over_the_tiling() {
        let uv = Projection::Uv { scale: [2.0, 4.0], rotation: 0.0, offset: [0.0; 2] };

        assert_eq!(tile_length(&uv, SIZE), Some([1.5, 0.75]));
    }

    #[test]
    fn a_length_read_back_gives_the_same_tiling() {
        let uv = UvTransform { tiling: [3.0, 1.5], ..UvTransform::default() };
        for projection in [
            Projection::Uv { scale: [3.0, 1.5], rotation: 0.0, offset: [0.0; 2] },
            Projection::Triplanar { scale: 1.5, sharpness: Projection::TRIPLANAR_SHARPNESS, offset: [0.0; 3], uv, projector: Default::default() },
            Projection::Planar { scale: 0.5, axis: 2, offset: [0.0; 3], uv, projector: Default::default(), placed: None },
            Projection::Cylindrical { scale: 2.0, axis: 1, uv, offset: [0.0; 3], projector: Default::default(), angle: 360.0 },
        ] {
            let length = tile_length(&projection, SIZE).unwrap();
            let tiling = tiling_for_length(&projection, SIZE, length).unwrap();

            assert!((tiling[0] - 3.0).abs() < 1e-5 && (tiling[1] - 1.5).abs() < 1e-5, "{projection:?}: {tiling:?}");
        }
    }

    #[test]
    fn a_written_length_is_the_repeat_on_the_mesh_through_a_stretched_projector() {
        let projector = Projector { scale: [2.0, 1.0, 1.0], ..Projector::default() };
        let axis_planar = |uv| Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3], uv, projector, placed: None };
        let length = 2.0;

        let tiling = tiling_for_length(&axis_planar(UvTransform::default()), SIZE, [length; 2]).unwrap();
        let planar = axis_planar(UvTransform { tiling, ..UvTransform::default() });
        let at = |x: f32, y: f32| image_samples(&planar, &Coords { uv: Vec2::ZERO, pos: Vec3::new(x, y, 0.0) / SIZE.radius, nrm: Vec3::Z })[0].0;
        let across = at(0.3 + length, 0.1) - at(0.3, 0.1);
        let down = at(0.3, 0.1 + length) - at(0.3, 0.1);

        assert_eq!(tile_length(&axis_planar(UvTransform::default()), SIZE), Some([8.0, 4.0]), "the stretch doubles the repeat across");
        assert!((across.x.abs() - 1.0).abs() < 1e-4 && across.y.abs() < 1e-4, "one length across is one tile: {across}");
        assert!(down.x.abs() < 1e-4 && (down.y.abs() - 1.0).abs() < 1e-4, "one length down is one tile: {down}");
    }

    #[test]
    fn a_stretch_uneven_round_the_planes_has_no_physical_size() {
        let projector = Projector { scale: [2.0, 1.0, 1.0], ..Projector::default() };
        let uv = UvTransform::default();

        let triplanar = Projection::Triplanar { scale: 1.0, sharpness: Projection::TRIPLANAR_SHARPNESS, offset: [0.0; 3], uv, projector };
        let cylinder = Projection::Cylindrical { scale: 1.0, axis: 1, uv, offset: [0.0; 3], projector, angle: 360.0 };
        let along_axis = Projection::Cylindrical { scale: 1.0, axis: 0, uv, offset: [0.0; 3], projector, angle: 360.0 };

        assert_eq!(tile_length(&triplanar, SIZE), None);
        assert_eq!(tile_length(&cylinder, SIZE), None);
        assert_eq!(tile_length(&along_axis, SIZE).map(|[_, down]| down), Some(8.0), "a stretch along the axis lengthens the repeat down it");
    }

    #[test]
    fn a_cylinder_of_no_angle_still_has_a_length() {
        let flat = Projection::Cylindrical { scale: 1.0, axis: 1, uv: UvTransform::default(), offset: [0.0; 3], projector: Default::default(), angle: 0.0 };
        let length = tile_length(&flat, SIZE).unwrap();

        assert!(length[0] > 0.0, "{length:?}");
        assert!(tiling_for_length(&flat, SIZE, length).unwrap()[0] > 0.0, "a written-back tiling is never 0");
    }
}
