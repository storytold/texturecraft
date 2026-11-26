//! The frame an axis planar projection projects from: moving one on the model places it as that frame, so its images land where they did

use texturecraft_core::{Decal, Projection, Projector, Source};
use texturecraft_geom::glam::{EulerRot, Mat3, Vec3};

use crate::frame_placement::culling_folds;
use crate::pattern::{DEPTH_FADE_START, frame_uv_from_corner, mesh_frame, nonzero_stretch, projector_stretch};

/// Place an axis planar on a mesh within `bounds` while keeping its sampling and culling.
/// See [`culling_folds`] for when culling prevents placement.
pub fn placed_from_axis(proj: &Projection, bounds: (Vec3, Vec3)) -> Option<Projection> {
    let Projection::Planar { scale, axis, offset, uv, projector, placed: None } = proj else { return None };

    if !culling_folds(projector) {
        return None;
    }

    let frame = axis_frame(*scale, *axis, Vec3::from(*offset), projector, bounds)?;

    // The frame carries rotation and scale while the projected rectangle keeps axis coverage
    let projector = Projector { rotation: [0.0; 3], scale: [1.0; 3], ignore_image_alpha: true, ..*projector };

    // An axis planar turns its image round the top left, a frame round its centre
    let uv = frame_uv_from_corner(uv);
    Some(Projection::Planar { scale: *scale, axis: *axis, offset: *offset, uv, projector, placed: Some(frame) })
}

/// Whether placing an axis planar that reads `sources` would move a pattern: a procedural reads an axis planar's plane but a placed frame's uv, so such a planar keeps its axis
pub fn placing_moves_pattern<'a>(mut sources: impl Iterator<Item = &'a Source>) -> bool {
    sources.any(|source| matches!(source, Source::Procedural { .. }))
}

/// The frame of an axis planar projection: its image's 0..1 square on the plane through the mesh, turned and sized by `projector`
pub fn axis_frame(scale: f32, axis: u8, offset: Vec3, projector: &Projector, bounds: (Vec3, Vec3)) -> Option<Decal> {
    let (center, radius) = mesh_frame(bounds);
    let [x, y, z] = projector.rotation.map(f32::to_radians);
    let turn = Mat3::from_euler(EulerRot::ZYX, z, y, x);
    let stretch = projector_stretch(projector);
    let scale = nonzero_stretch(scale);

    // Where the image's centre sits, then its across, up and out directions with the stretch along each
    let origin = center + turn * (stretch * -offset / scale) * radius;
    let (across, up, out) = match axis {
        0 => (Vec3::Z, Vec3::Y, Vec3::NEG_X),
        1 => (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        _ => (Vec3::X, Vec3::Y, Vec3::Z),
    };
    let side = |direction: Vec3| 2.0 * radius * direction.abs().dot(stretch) / scale;

    // A negative stretch or scale turns that side over
    let flip = |direction: Vec3| direction * side(direction).signum();
    let (across, up) = (flip(across), flip(up));
    let size = [side(across).abs(), side(up).abs()];
    let out = turn * across.cross(up).normalize_or(out);

    // Deep enough never to crop the mesh by depth: the fade starts at three quarters of it
    let reach = (radius + (origin - center).length()) / DEPTH_FADE_START * DEPTH_SPARE;
    let frame = Decal {
        center: origin.to_array(),
        normal: out.to_array(),
        up: (turn * up).to_array(),
        size,
        rotation: 0.0,
        offset: [0.0; 2],
        depth: reach,
        symmetry: None,
        flip: [false; 2],
    };
    (origin.is_finite() && size.iter().all(|side| side.is_finite() && *side > 0.0) && reach.is_finite()).then_some(frame)
}

/// Extra reach past the farthest mesh point
const DEPTH_SPARE: f32 = 1.01;

#[cfg(test)]
mod tests {
    use texturecraft_core::{Backface, ShapeCrop, UvTransform};
    use texturecraft_geom::glam::Vec2;

    use super::*;
    use crate::pattern::{Coords, image_samples};
    use crate::projector_cover::{framed_cover, projector_cover};

    /// Points spread through the box `lo`..`hi` with varied normals
    fn points(lo: Vec3, hi: Vec3) -> Vec<(Vec3, Vec3)> {
        (0..64)
            .map(|i| {
                let spread = |salt: u32| ((i * 37 + salt * 11) % 64) as f32 / 63.0;
                let position = lo + (hi - lo) * Vec3::new(spread(1), spread(2), spread(3));
                let normal = Vec3::new(spread(4) - 0.4, spread(5) - 0.3, spread(6) - 0.2).normalize_or(Vec3::Z);
                (position, normal)
            })
            .collect()
    }

    #[test]
    fn a_placed_axis_planar_reads_images_where_it_did() {
        let (lo, hi) = (Vec3::new(-1.0, -0.5, -2.0), Vec3::new(3.0, 1.5, 0.5));
        let center = (lo + hi) * 0.5;
        let radius = (hi - lo).length() * 0.5;
        for axis in 0..3 {
            for (rotation, stretch) in [([0.0; 3], [1.0; 3]), ([20.0, -35.0, 50.0], [1.5, 0.7, 2.0])] {
                for crop in [ShapeCrop::ExtendsOutside, ShapeCrop::Cropped] {
                    let projector = Projector { rotation, scale: stretch, crop, ..Projector::default() };
                    let uv = UvTransform { tiling: [2.0, 3.0], rotation: 30.0, offset: [0.1, -0.2] };
                    let axis_planar = Projection::Planar { scale: 1.7, axis, offset: [0.3, -0.4, 0.2], uv, projector, placed: None };
                    let placed = placed_from_axis(&axis_planar, (lo, hi)).expect("an axis planar has a frame");
                    for (position, normal) in points(lo, hi) {
                        let coords = Coords { uv: Vec2::ZERO, pos: (position - center) / radius, nrm: normal };
                        let (before, _) = image_samples(&axis_planar, &coords)[0];
                        let (uv_after, cover) = framed_cover(&placed, position, normal, center).expect("a placed frame covers");
                        let (after, _) = image_samples(&placed, &Coords { uv: uv_after, ..coords })[0];

                        assert!(before.distance(after) < 1e-3, "axis {axis} {rotation:?}: {before} became {after}");
                        assert!((projector_cover(&axis_planar, &coords) - cover).abs() < 1e-3, "axis {axis} {crop:?}: cover changed at {position}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_placed_axis_planar_keeps_what_backface_culling_hides() {
        let (lo, hi) = (Vec3::new(-1.0, -0.5, -2.0), Vec3::new(3.0, 1.5, 0.5));
        let center = (lo + hi) * 0.5;
        let radius = (hi - lo).length() * 0.5;
        let backface = Backface { on: true, angle: 60.0, hardness: 1.0 };

        // An oblique normal exposes tilted and reversed culling under stretch
        let halfway = (center, Vec3::new(1.0, 0.0, 1.0).normalize());

        for stretch in [[4.0, 1.0, 1.0], [1.5, -1.5, 1.5], [1.7; 3]] {
            let projector = Projector { scale: stretch, backface, ..Projector::default() };
            let axis_planar = Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3], uv: UvTransform::default(), projector, placed: None };
            let Some(placed) = placed_from_axis(&axis_planar, (lo, hi)) else {
                assert!(!culling_folds(&projector), "{stretch:?}: only a stretch culling cannot keep stays unplaced");
                continue;
            };

            for (position, normal) in points(lo, hi).into_iter().chain([halfway]) {
                let coords = Coords { uv: Vec2::ZERO, pos: (position - center) / radius, nrm: normal };
                let (_, cover) = framed_cover(&placed, position, normal, center).expect("a placed frame covers");
                assert!((projector_cover(&axis_planar, &coords) - cover).abs() < 1e-3, "{stretch:?}: culling changed at {normal}");
            }
        }

        let even = Projector { scale: [1.7; 3], backface, ..Projector::default() };
        let axis_planar = Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3], uv: UvTransform::default(), projector: even, placed: None };
        assert!(placed_from_axis(&axis_planar, (lo, hi)).is_some(), "an even stretch is still placed");
    }

    #[test]
    fn only_axis_planars_are_placed() {
        let bounds = (Vec3::NEG_ONE, Vec3::ONE);

        assert!(placed_from_axis(&Projection::default(), bounds).is_none());

        let frame = axis_frame(1.0, 2, Vec3::ZERO, &Projector::default(), bounds).expect("a frame");
        assert!(placed_from_axis(&Projection::decal(frame), bounds).is_none());
        assert!(axis_frame(f32::NAN, 2, Vec3::ZERO, &Projector::default(), bounds).is_none());
    }
}
