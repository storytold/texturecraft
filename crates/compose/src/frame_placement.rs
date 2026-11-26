//! A frame on the surface (a placed planar or a warp) as its 3D projection settings show it: where its centre is, how it is turned and how big it is

use texturecraft_core::{Decal, Projection, Projector};
use texturecraft_geom::glam::{EulerRot, Mat3, Quat, Vec3};

use crate::pattern::{decal_axes, mesh_frame, projector_stretch};

/// The shortest frame side a depth is divided by, so a collapsed frame keeps a finite depth
pub const MIN_FRAME_SIDE: f32 = 1e-6;

/// Below this the frame faces nearly straight up or down, so world up gives no upright direction
const MIN_UP_CROSS_SQUARED: f32 = 1e-6;

/// The longer side of a frame of `size`, at least [`MIN_FRAME_SIDE`]: a warp's Projection Depth counts in it
pub fn longer_side(size: [f32; 2]) -> f32 {
    size[0].max(size[1]).max(MIN_FRAME_SIDE)
}

/// How far a warp whose frame has `size` reaches in front of and behind it at Projection Depth `depth`, in world units
pub fn warp_reach(size: [f32; 2], depth: f32) -> f32 {
    depth * longer_side(size)
}

/// A frame's Offset (its centre in mesh radii from the mesh's centre), Rotation (degrees round X, Y and Z, as a projector turns) and Scale (its width and height, then how far it reaches in front of and behind it, in mesh diagonals)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FramePlacement {
    pub offset: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

/// Whether moving the projector's scale into the frame preserves culling
pub fn culling_folds(projector: &Projector) -> bool {
    let stretch = projector_stretch(projector);
    let size = stretch.abs();
    let even = size.x == size.y && size.y == size.z;
    let facing_preserved = stretch.x.signum() * stretch.y.signum() * stretch.z.signum() > 0.0;
    (!projector.depth.on || facing_preserved) && (!projector.backface.on || (even && facing_preserved))
}

/// Fold a placed planar or warp's turn and scale into its frame while preserving crop and culling.
/// See [`culling_folds`] for when a fold would change culling.
pub fn folded(proj: &Projection) -> Option<Projection> {
    let (frame, projector) = match proj {
        Projection::Planar { placed: Some(frame), projector, .. } | Projection::Warp { frame, projector, .. } => (frame, projector),
        _ => return None,
    };

    if !projector.turns() {
        return Some(proj.clone());
    }

    if !culling_folds(projector) {
        return None;
    }

    let (origin, across, up, _) = decal_axes(frame)?;
    let [x, y, z] = projector.rotation.map(f32::to_radians);
    let turn = Mat3::from_cols(across, up, across.cross(up)) * Mat3::from_euler(EulerRot::ZYX, z, y, x);
    let stretch = projector_stretch(projector);

    // A negative scale turns that side over
    let (across, up) = (turn.x_axis * stretch.x.signum(), turn.y_axis * stretch.y.signum());
    let size = [frame.size[0] * stretch.x.abs(), frame.size[1] * stretch.y.abs()];
    let reach = stretch.z.abs();
    let new_frame = Decal {
        center: origin.to_array(),
        normal: across.cross(up).to_array(),
        up: up.to_array(),
        size,
        rotation: 0.0,
        offset: [0.0; 2],
        depth: frame.depth * reach,
        symmetry: frame.symmetry,
        flip: frame.flip,
    };
    let unturned = Projector { rotation: [0.0; 3], scale: [1.0; 3], ..*projector };
    let mut out = proj.clone();
    match &mut out {
        Projection::Planar { placed: Some(placed), projector, .. } => {
            *placed = new_frame;
            *projector = unturned;
        }
        Projection::Warp { frame: warp, projector, depth, .. } => {
            // Its depth counts in lengths of the frame's longer side
            *depth *= reach * longer_side(frame.size) / longer_side(size);
            *warp = new_frame;
            *projector = unturned;
        }
        _ => return None,
    }
    let finite = new_frame.center.iter().chain(&new_frame.normal).chain(&new_frame.up).chain(&new_frame.size).all(|v| v.is_finite());
    finite.then_some(out)
}

/// How far frame projection `proj` reaches in front of and behind its frame, in world units (a warp counts its depth in lengths of the longer side)
fn reach(proj: &Projection) -> Option<f32> {
    match proj {
        Projection::Planar { placed: Some(frame), .. } => Some(frame.depth),
        Projection::Warp { frame, depth, .. } => Some(warp_reach(frame.size, *depth)),
        _ => None,
    }
}

/// Where frame projection `proj` (a placed planar or a warp) sits on a mesh with bounds `bounds`, its projector folded in (None for other projections)
pub fn placement(proj: &Projection, bounds: (Vec3, Vec3)) -> Option<FramePlacement> {
    let proj = folded(proj)?;
    let frame = proj.frame()?;
    let (center, radius) = mesh_frame(bounds);
    let (origin, across, up, out) = decal_axes(frame)?;
    let (z, y, x) = Quat::from_mat3(&Mat3::from_cols(across, up, out)).to_euler(EulerRot::ZYX);
    let diagonal = 2.0 * radius;
    let placement = FramePlacement {
        offset: ((origin - center) / radius).to_array(),
        rotation: [x, y, z].map(f32::to_degrees),
        scale: [frame.size[0] / diagonal, frame.size[1] / diagonal, reach(&proj)? / diagonal],
    };
    let values = placement.offset.iter().chain(&placement.rotation).chain(&placement.scale);
    values.copied().all(f32::is_finite).then_some(placement)
}

/// Frame projection `proj` (a placed planar or a warp) moved to `placement`; its projector's turn and scale are folded in first, its other settings kept (None for other projections or a degenerate placement)
pub fn placed(proj: &Projection, placement: &FramePlacement, bounds: (Vec3, Vec3)) -> Option<Projection> {
    let values = placement.offset.iter().chain(&placement.rotation).chain(&placement.scale);
    let [width_scale, height_scale, reach_scale] = placement.scale;
    let valid_reach = reach_scale > 0.0 || (reach_scale == 0.0 && matches!(proj, Projection::Warp { .. }));
    if !values.copied().all(f32::is_finite) || width_scale <= 0.0 || height_scale <= 0.0 || !valid_reach {
        return None;
    }

    let mut proj = folded(proj)?;
    let old = *proj.frame()?;
    let (center, radius) = mesh_frame(bounds);
    let diagonal = 2.0 * radius;
    let [x, y, z] = placement.rotation.map(f32::to_radians);
    let turn = Mat3::from_euler(EulerRot::ZYX, z, y, x);
    let [width, height, deep] = placement.scale.map(|scale| scale * diagonal);
    let frame = Decal {
        center: (center + Vec3::from(placement.offset) * radius).to_array(),
        normal: turn.z_axis.to_array(),
        up: turn.y_axis.to_array(),
        size: [width, height],
        rotation: 0.0,
        offset: [0.0; 2],
        depth: deep,
        symmetry: old.symmetry,
        flip: old.flip,
    };
    match &mut proj {
        Projection::Planar { placed: Some(placed), .. } => *placed = frame,
        Projection::Warp { frame: warp, depth, .. } => {
            *depth = deep / longer_side([width, height]);

            // The warp's own depth is what reaches; the frame's is unused
            *warp = Decal { depth: warp.depth, ..frame };
        }
        _ => return None,
    }
    Some(proj)
}

/// A frame's turn round its own normal, as seen face on with the world's up (or, looking straight down, the far side) at the top: degrees
pub fn upright_turn(frame: &Decal) -> Option<f32> {
    let (_, _, up, out) = decal_axes(frame)?;
    let world_up = if out.cross(Vec3::Y).length_squared() > MIN_UP_CROSS_SQUARED { Vec3::Y } else { Vec3::NEG_Z };
    let upright = (world_up - out * out.dot(world_up)).try_normalize()?;
    let upright_across = upright.cross(out);
    let turn = (-up.dot(upright_across)).atan2(up.dot(upright)).to_degrees();
    turn.is_finite().then_some(turn)
}

#[cfg(test)]
mod tests {
    use texturecraft_core::{Backface, DecalFlip, DecalSymmetry, ShapeCrop, UvTransform};
    use texturecraft_geom::glam::Vec2;

    use super::*;
    use crate::projector_cover::framed_cover;

    const BOUNDS: (Vec3, Vec3) = (Vec3::new(-1.0, -2.0, -0.5), Vec3::new(3.0, 1.0, 1.5));

    fn frame() -> Decal {
        Decal {
            center: [0.4, -0.2, 1.3],
            normal: [0.2, 0.1, 1.0],
            up: [0.0, 1.0, 0.1],
            size: [1.2, 0.8],
            rotation: 25.0,
            offset: [0.1, -0.2],
            depth: 0.6,
            symmetry: None,
            flip: [false; 2],
        }
    }

    fn planar(projector: Projector) -> Projection {
        Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3], uv: UvTransform::default(), projector, placed: Some(frame()) }
    }

    fn warp(projector: Projector) -> Projection {
        Projection::Warp { frame: frame(), corners: [[-0.6, 0.5], [0.5, 0.4], [0.5, -0.5], [-0.4, -0.5]], depth: 0.7, uv: UvTransform::default(), projector }
    }

    /// The projection's texture coordinates and cover at points through the frame's neighbourhood with varied normals
    fn samples(proj: &Projection) -> Vec<Option<(Vec2, f32)>> {
        let mut out = Vec::new();
        for across in 0..7 {
            for up in 0..7 {
                for out_of in 0..3 {
                    let position = Vec3::new(-0.5 + across as f32 * 0.25, -0.8 + up as f32 * 0.2, 0.7 + out_of as f32 * 0.3);
                    let normal = Vec3::new(0.1 * across as f32 - 0.3, 0.2, 1.0).normalize();
                    out.push(framed_cover(proj, position, normal, (BOUNDS.0 + BOUNDS.1) * 0.5));
                }
            }
        }
        out
    }

    fn projector_of(proj: &Projection) -> Projector {
        match proj {
            Projection::Planar { projector, .. } | Projection::Warp { projector, .. } => *projector,
            _ => panic!("a frame projection"),
        }
    }

    fn close(a: &[Option<(Vec2, f32)>], b: &[Option<(Vec2, f32)>]) {
        assert_eq!(a.len(), b.len());
        for (index, (a, b)) in a.iter().zip(b).enumerate() {
            match (a, b) {
                (Some((uv_a, cover_a)), Some((uv_b, cover_b))) => {
                    assert!((cover_a - cover_b).abs() < 1e-3, "{index}: cover {cover_a} vs {cover_b}");
                    assert!(uv_a.distance(*uv_b) < 1e-3, "{index}: {uv_a} vs {uv_b}");
                }
                (None, None) => {}
                _ => panic!("{index}: sampled on one side only: {a:?} vs {b:?}"),
            }
        }
    }

    #[test]
    fn folding_a_turned_and_evenly_scaled_projector_into_its_frame_lands_the_same() {
        let projector = Projector { rotation: [20.0, -35.0, 50.0], scale: [1.7; 3], crop: ShapeCrop::Cropped, ..Projector::default() };
        for proj in [planar(projector), warp(projector)] {
            let flat = folded(&proj).unwrap();
            let Projector { rotation, scale, crop, .. } = projector_of(&flat);

            assert_eq!((rotation, scale, crop), ([0.0; 3], [1.0; 3], ShapeCrop::Cropped), "the turn and scale moved into the frame, the crop stayed");

            close(&samples(&proj), &samples(&flat));
        }
        let unturned = planar(Projector::default());
        assert_eq!(folded(&unturned), Some(unturned), "nothing to fold: unchanged");
        assert_eq!(folded(&Projection::default()), None);
    }

    #[test]
    fn depth_culling_prevents_folding_with_the_opposite_facing() {
        let mut projector = Projector::depth_culled();
        projector.scale[2] = -1.0;
        let bounds = (Vec3::NEG_ONE, Vec3::ONE);
        let axis_planar = Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3], uv: UvTransform::default(), projector, placed: None };

        assert!(folded(&planar(projector)).is_none(), "an existing frame would face the opposite direction");
        assert!(crate::axis_frame::placed_from_axis(&axis_planar, bounds).is_none(), "an axis projector would face the opposite direction");
        assert!(folded(&planar(Projector::depth_culled())).is_some(), "unit stretch preserves facing");
    }

    #[test]
    fn folding_keeps_what_backface_culling_hides() {
        let backface = Backface { on: true, angle: 60.0, hardness: 1.0 };
        let square = Decal { center: [0.0; 3], normal: [0.0, 0.0, 1.0], up: [0.0, 1.0, 0.0], size: [1.0, 1.0], rotation: 0.0, offset: [0.0; 2], ..frame() };

        // An oblique normal distinguishes tilt from reflection
        let normal = Vec3::new(1.0, 0.0, 1.0).normalize();
        let center = (BOUNDS.0 + BOUNDS.1) * 0.5;
        let cover = |proj: &Projection| framed_cover(proj, Vec3::ZERO, normal, center).map(|(_, cover)| cover);

        for scale in [[4.0, 1.0, 1.0], [1.5, -1.5, 1.5], [1.7; 3]] {
            let projector = Projector { scale, backface, ..Projector::default() };
            let proj = Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3], uv: UvTransform::default(), projector, placed: Some(square) };
            match folded(&proj) {
                Some(flat) => {
                    assert_eq!(cover(&proj), cover(&flat), "{scale:?}: culling changed");

                    close(&samples(&proj), &samples(&flat));
                }
                None => assert!(!culling_folds(&projector), "{scale:?}: only a scale culling cannot keep stays unfolded"),
            }
        }

        assert!(folded(&planar(Projector { scale: [1.7; 3], backface, ..Projector::default() })).is_some(), "an even scale still folds");
    }

    #[test]
    fn a_placement_read_and_written_back_lands_the_same_and_its_fields_move_the_frame() {
        let projector = Projector { rotation: [10.0, 0.0, -30.0], scale: [1.2, 0.9, 1.3], ..Projector::default() };
        for proj in [planar(Projector::default()), warp(Projector::default()), planar(projector), warp(projector)] {
            let place = placement(&proj, BOUNDS).unwrap();
            let back = placed(&proj, &place, BOUNDS).unwrap();
            close(&samples(&folded(&proj).unwrap()), &samples(&back));
            let again = placement(&back, BOUNDS).unwrap();
            for (a, b) in place.offset.iter().chain(&place.rotation).chain(&place.scale).zip(again.offset.iter().chain(&again.rotation).chain(&again.scale)) {
                assert!((a - b).abs() < 1e-3, "{place:?} vs {again:?}");
            }

            let width = folded(&proj).unwrap().frame().unwrap().size[0];
            let bigger = placed(&proj, &FramePlacement { scale: [place.scale[0] * 2.0, place.scale[1], place.scale[2]], ..place }, BOUNDS).unwrap();

            assert!((bigger.frame().unwrap().size[0] - width * 2.0).abs() < 1e-4, "Scale X doubles the width");

            let moved = placed(&proj, &FramePlacement { offset: [0.0; 3], ..place }, BOUNDS).unwrap();

            assert!((Vec3::from(moved.frame().unwrap().center) - Vec3::new(1.0, -0.5, 0.5)).length() < 1e-4, "offset 0 = the mesh's centre");
        }
        let place = placement(&planar(Projector::default()), BOUNDS).unwrap();

        assert_eq!(placed(&planar(Projector::default()), &FramePlacement { scale: [0.0, 1.0, 1.0], ..place }, BOUNDS), None, "no zero size");
        assert_eq!(placed(&planar(Projector::default()), &FramePlacement { rotation: [f32::NAN; 3], ..place }, BOUNDS), None);
        assert_eq!(placement(&Projection::default(), BOUNDS), None);
    }

    #[test]
    fn a_frames_upright_turn_is_its_turn_round_its_normal() {
        let mut flat = Decal { normal: [0.0, 0.0, 1.0], up: [0.0, 1.0, 0.0], rotation: 30.0, offset: [0.0; 2], ..frame() };

        assert!((upright_turn(&flat).unwrap() - 30.0).abs() < 1e-3);

        flat.rotation = -100.0;

        assert!((upright_turn(&flat).unwrap() + 100.0).abs() < 1e-3);

        let top = Decal { normal: [0.0, 1.0, 0.0], up: [0.0, 0.0, -1.0], rotation: 0.0, ..frame() };
        assert!(upright_turn(&top).unwrap().abs() < 1e-3, "seen from above the far side is up");
    }

    #[test]
    fn folding_and_placing_keep_a_frames_mirror_copy_and_image_flip() {
        let symmetry = Some(DecalSymmetry { axis: 0, flip: DecalFlip::V, offset: 0.2 });
        let mirrored = Decal { symmetry, flip: [true, false], ..frame() };
        let turned = Projector { rotation: [10.0, 0.0, -30.0], scale: [1.2, 0.9, 1.3], ..Projector::default() };
        for proj in [
            Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3], uv: UvTransform::default(), projector: turned, placed: Some(mirrored) },
            Projection::Warp {
                frame: mirrored,
                corners: [[-0.6, 0.5], [0.5, 0.4], [0.5, -0.5], [-0.4, -0.5]],
                depth: 0.7,
                uv: UvTransform::default(),
                projector: turned,
            },
        ] {
            let folded = folded(&proj).unwrap();
            let moved = placed(&proj, &placement(&proj, BOUNDS).unwrap(), BOUNDS).unwrap();
            for frame in [folded.frame().unwrap(), moved.frame().unwrap()] {
                assert_eq!((frame.symmetry, frame.flip), (symmetry, [true, false]));
            }
        }
    }
}
