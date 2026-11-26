//! How much of a texel a 3D projection reaches: its shape crop and its backface culling

use texturecraft_core::{Backface, Decal, DecalFlip, Projection, Projector, ShapeCrop};
use texturecraft_geom::glam::{Vec2, Vec3};

use crate::frame_placement::warp_reach;
use crate::organic::smoothstep;
use crate::pattern::{Coords, depth_fade, frame_crop, frame_local, frame_sample, placed_uv, projector_local};
use crate::warp::warp_uv;

/// Nearer a cylinder's axis than this a texel has no outward direction, so it counts as facing out
const MIN_AXIS_DISTANCE: f32 = 1e-6;

/// The turn (in cosine) over which backface culling fades at hardness 0
const BACKFACE_FADE: f32 = 0.6;

/// The shape crop and backface culling [`projector_cover`] applies, None where it lets every texel through
fn cropping_projector(proj: &Projection) -> Option<&Projector> {
    let projector = match proj {
        Projection::Triplanar { projector, .. }
        | Projection::Planar { projector, .. }
        | Projection::Spherical { projector, .. }
        | Projection::Cylindrical { projector, .. } => projector,
        // Framed on the surface: see [`framed_cover`]
        Projection::Warp { .. } | Projection::Uv { .. } | Projection::UvTile => return None,
    };
    Some(projector).filter(|projector| projector.crop != ShapeCrop::ExtendsOutside || projector.backface.on)
}

/// Whether [`projector_cover`] is 1 on every texel, so it needs no texel's place
pub fn projector_covers_all(proj: &Projection) -> bool {
    cropping_projector(proj).is_none()
}

/// How much of the texel at `coords` the projection reaches through its shape crop and backface culling; not for placed planars or warps (see [`framed_cover`])
pub fn projector_cover(proj: &Projection, coords: &Coords) -> f32 {
    let Some(projector) = cropping_projector(proj) else { return 1.0 };

    let local = projector_local(projector, coords);
    let (inside, facing) = match proj {
        Projection::Triplanar { .. } => (local.pos.abs().max_element() <= 1.0, None),
        Projection::Planar { scale, axis, offset, .. } => {
            let scaled = local.pos * *scale + Vec3::from(*offset);

            // The frame across the axis, and the side the image reads the right way round from
            let (frame, facing) = match axis {
                0 => (scaled.z.abs().max(scaled.y.abs()), -local.nrm.x),
                1 => (scaled.x.abs().max(scaled.z.abs()), local.nrm.y),
                _ => (scaled.x.abs().max(scaled.y.abs()), local.nrm.z),
            };
            (frame <= 1.0, Some(facing))
        }
        Projection::Spherical { offset, .. } => ((local.pos + Vec3::from(*offset)).length() <= 1.0, None),
        Projection::Cylindrical { axis, offset, .. } => {
            let position = local.pos + Vec3::from(*offset);
            let normal = local.nrm;
            let ((across, height, depth), (normal_across, normal_depth)) = match axis {
                0 => ((position.y, position.x, position.z), (normal.y, normal.z)),
                2 => ((position.x, position.z, position.y), (normal.x, normal.y)),
                _ => ((position.x, position.y, position.z), (normal.x, normal.z)),
            };
            let radius = across.hypot(depth);
            let outward = if radius > MIN_AXIS_DISTANCE { (normal_across * across + normal_depth * depth) / radius } else { 1.0 };
            (radius <= 1.0 && height.abs() <= 1.0, Some(outward))
        }
        Projection::Warp { .. } | Projection::Uv { .. } | Projection::UvTile => (true, None),
    };
    if projector.crop == ShapeCrop::Cropped && !inside {
        return 0.0;
    }

    match facing {
        Some(facing) if projector.backface.on => backface_cover(&projector.backface, facing),
        _ => 1.0,
    }
}

/// How much a face turned `facing` (the cosine of its turn from the projection) keeps
pub fn backface_cover(backface: &Backface, facing: f32) -> f32 {
    // A hand-edited NaN angle or hardness keeps the layer visible rather than making its texels NaN
    if !(backface.angle.is_finite() && backface.hardness.is_finite()) {
        return 1.0;
    }

    // cos(angle) as sin(90 - angle): exactly 0 at 90 degrees
    let threshold = (90.0 - backface.angle).to_radians().sin();
    let width = (1.0 - backface.hardness.clamp(0.0, 1.0)) * BACKFACE_FADE;
    if width <= f32::EPSILON {
        return f32::from(u8::from(facing > threshold));
    }

    smoothstep(threshold, threshold + width, facing)
}

/// A planar projection placed on the surface at world point `p` with normal `nrm`: its texture coordinates and how much its backface culling and shape crop let through (None when the frame is degenerate)
pub fn placed_cover(frame: &Decal, projector: &Projector, p: Vec3, nrm: Vec3) -> Option<(Vec2, f32)> {
    let (local, mut cover) = if projector.backface.on {
        let (local, facing) = frame_sample(frame, projector, p, nrm)?;
        (local, backface_cover(&projector.backface, facing))
    } else {
        (frame_local(frame, projector, p)?, 1.0)
    };

    if projector.crop == ShapeCrop::Cropped {
        cover *= frame_crop(frame, local);
    }
    Some((flip_uv(placed_uv(local), frame.flip), cover))
}

/// [`placed_cover`] for a frame with a mirrored copy: whichever of the frame and its copy covers the point more. The copy is the frame reflected through the plane `offset` away from `center` (the mesh center) on the symmetry axis, its image flipped as asked
pub fn placed_cover_symmetric(frame: &Decal, projector: &Projector, p: Vec3, nrm: Vec3, center: Vec3) -> Option<(Vec2, f32)> {
    let direct = placed_cover(frame, projector, p, nrm);
    let Some(symmetry) = frame.symmetry else { return direct };

    let axis = usize::from(symmetry.axis.min(2));
    let (mut mirrored_position, mut mirrored_normal) = (p, nrm);
    mirrored_position[axis] = 2.0 * (center[axis] + symmetry.offset) - p[axis];
    mirrored_normal[axis] = -nrm[axis];
    let mirrored = placed_cover(frame, projector, mirrored_position, mirrored_normal).map(|(uv, cover)| {
        let uv = match symmetry.flip {
            DecalFlip::None => uv,
            DecalFlip::U => Vec2::new(1.0 - uv.x, uv.y),
            DecalFlip::V => Vec2::new(uv.x, 1.0 - uv.y),
        };
        (uv, cover)
    });
    match (direct, mirrored) {
        (Some(a), Some(b)) => Some(if b.1 > a.1 { b } else { a }),
        (a, b) => a.or(b),
    }
}

/// `uv` with the image mirrored left to right (`flip[0]`) and/or top to bottom (`flip[1]`) inside the frame
fn flip_uv(uv: Vec2, flip: [bool; 2]) -> Vec2 {
    Vec2::new(if flip[0] { 1.0 - uv.x } else { uv.x }, if flip[1] { 1.0 - uv.y } else { uv.y })
}

/// A projection from a frame on the surface (a placed planar or a warp) at world point `p` with normal `nrm`, `center` the mesh center a placed planar's mirrored copy reflects across: its texture coordinates and how much it covers there (None for other projections, a degenerate frame or where nothing lands)
pub fn framed_cover(proj: &Projection, p: Vec3, nrm: Vec3, center: Vec3) -> Option<(Vec2, f32)> {
    match proj {
        Projection::Planar { projector, placed: Some(frame), .. } => placed_cover_symmetric(frame, projector, p, nrm, center),
        Projection::Warp { frame, corners, depth, projector, .. } => warp_cover(frame, corners, *depth, projector, p),
        _ => None,
    }
}

/// A warp projection at world point `p`: its texture coordinates in the quad its corners make, and, cropped to its shape, how much of the quad and of its Projection Depth (in lengths of the frame's longer side) cover it
pub fn warp_cover(frame: &Decal, corners: &[[f32; 2]; 4], depth: f32, projector: &Projector, p: Vec3) -> Option<(Vec2, f32)> {
    let local = frame_local(frame, projector, p)?;
    let uv = warp_uv(corners, local.truncate())?;
    if projector.crop == ShapeCrop::ExtendsOutside {
        return Some((uv, 1.0));
    }

    let inside = (0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y);
    let reach = warp_reach(frame.size, depth);
    Some((uv, if inside { depth_fade(local.z, reach) } else { 0.0 }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The decal formula from before decals were placed planar projections, kept to check they still look the same
    fn decal_before(frame: &Decal, point: Vec3, surface_normal: Vec3) -> Option<(Vec2, f32)> {
        let (origin, right, up, normal) = crate::pattern::decal_axes(frame)?;
        let facing = surface_normal.dot(normal);
        let offset = point - origin;
        let (local_x, local_y, local_z) = (offset.dot(right) / frame.size[0].max(1e-6), offset.dot(up) / frame.size[1].max(1e-6), offset.dot(normal).abs());
        let depth = frame.depth.max(1e-6);
        if facing <= 0.0 || local_x.abs() > 0.5 || local_y.abs() > 0.5 || local_z > depth || !(local_x.is_finite() && local_y.is_finite()) {
            return None;
        }

        let cover = smoothstep(0.0, 0.3, facing) * (1.0 - smoothstep(0.75 * depth, depth, local_z));
        Some((Vec2::new(local_x + 0.5, 0.5 - local_y), cover))
    }

    #[test]
    fn a_nan_backface_angle_or_hardness_culls_nothing() {
        for backface in [Backface { on: true, angle: f32::NAN, hardness: 0.5 }, Backface { on: true, angle: 60.0, hardness: f32::NAN }] {
            assert_eq!(backface_cover(&backface, -0.5), 1.0, "{backface:?}");
        }
    }

    #[test]
    fn saved_decals_cover_exactly_as_before() {
        let frames = [
            Decal {
                center: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
                up: [0.0, 1.0, 0.0],
                size: [2.0, 1.0],
                rotation: 0.0,
                offset: [0.0; 2],
                depth: 0.5,
                symmetry: None,
                flip: [false; 2],
            },
            Decal {
                center: [0.3, -0.2, 0.1],
                normal: [0.4, 0.5, 0.7],
                up: [0.0, 1.0, 0.0],
                size: [0.7, 1.3],
                rotation: 37.0,
                offset: [0.2, -0.1],
                depth: 0.4,
                symmetry: None,
                flip: [false; 2],
            },
        ];
        for frame in frames {
            let Projection::Planar { projector, placed: Some(placed), .. } = Projection::decal(frame) else { panic!("a placed planar projection") };
            for sample in 0..4000 {
                let phase = sample as f32 * 0.137;
                let point = Vec3::new(phase.sin() * 1.3, (phase * 0.71).cos() * 1.1, (phase * 0.33).sin() * 0.8);
                let normal = Vec3::new((phase * 1.7).cos(), (phase * 0.9).sin(), (phase * 0.4).cos()).normalize();
                let now = placed_cover(&placed, &projector, point, normal).filter(|(_, cover)| *cover > 0.0);
                match (decal_before(&frame, point, normal), now) {
                    (Some(before), Some(now)) => assert_eq!(before, now, "{point} {normal}"),
                    (None, None) => {}
                    (before, now) => panic!("{point} {normal}: {before:?} became {now:?}"),
                }
            }
        }
    }

    /// The mirrored copy's formula as the decal model before placed planar projections had it (flip and symmetry on a decal)
    fn symmetric_before(d: &Decal, p: Vec3, nrm: Vec3, center: Vec3) -> Option<(Vec2, f32)> {
        let flip = |uv: Vec2| flip_uv(uv, d.flip);
        let direct = decal_before(d, p, nrm).map(|(uv, w)| (flip(uv), w));
        let Some(symmetry) = d.symmetry else { return direct };
        let axis = usize::from(symmetry.axis.min(2));
        let (mut q, mut m) = (p, nrm);
        q[axis] = 2.0 * (center[axis] + symmetry.offset) - p[axis];
        m[axis] = -nrm[axis];
        let mirrored = decal_before(d, q, m).map(|(uv, w)| {
            let uv = flip(uv);
            let uv = match symmetry.flip {
                DecalFlip::None => uv,
                DecalFlip::U => Vec2::new(1.0 - uv.x, uv.y),
                DecalFlip::V => Vec2::new(uv.x, 1.0 - uv.y),
            };
            (uv, w)
        });
        match (direct, mirrored) {
            (Some(a), Some(b)) => Some(if b.1 > a.1 { b } else { a }),
            (a, b) => a.or(b),
        }
    }

    #[test]
    fn saved_mirrored_and_flipped_decals_cover_exactly_as_before() {
        let base = Decal {
            center: [0.3, -0.2, 0.1],
            normal: [0.4, 0.5, 0.7],
            up: [0.0, 1.0, 0.0],
            size: [0.7, 1.3],
            rotation: 37.0,
            offset: [0.2, -0.1],
            depth: 0.4,
            symmetry: None,
            flip: [false; 2],
        };
        let center = Vec3::new(0.1, -0.05, 0.2);
        let frames = [
            Decal { flip: [true, false], ..base },
            Decal { symmetry: Some(texturecraft_core::DecalSymmetry { axis: 0, flip: DecalFlip::None, offset: 0.0 }), ..base },
            Decal { symmetry: Some(texturecraft_core::DecalSymmetry { axis: 2, flip: DecalFlip::V, offset: -0.15 }), flip: [false, true], ..base },
        ];
        for frame in frames {
            let Projection::Planar { projector, placed: Some(placed), .. } = Projection::decal(frame) else { panic!("a placed planar projection") };
            let mut mirrored = 0;
            for i in 0..4000 {
                let t = i as f32 * 0.137;
                let p = Vec3::new(t.sin() * 1.3, (t * 0.71).cos() * 1.1, (t * 0.33).sin() * 0.8);
                let nrm = Vec3::new((t * 1.7).cos(), (t * 0.9).sin(), (t * 0.4).cos()).normalize();
                let now = placed_cover_symmetric(&placed, &projector, p, nrm, center).filter(|(_, cover)| *cover > 0.0);
                let before = symmetric_before(&frame, p, nrm, center);
                mirrored += usize::from(before.is_some() && decal_before(&frame, p, nrm).is_none());

                assert_eq!(before, now, "{p} {nrm}");
            }

            assert!(frame.symmetry.is_none() || mirrored > 0, "the mirrored copy covers points the frame does not");
        }
    }
}
