use texturecraft_geom::glam::{Mat3, Vec2, Vec3};
use texturecraft_geom::{Bvh, Mesh, NO_TRI, UvGBuffer, surface_at};
use texturecraft_paint::{StampSource, Symmetry, UvStamp};

use super::frame::Frame;

/// The mirror and radial maps of one stroke, applied to dabs through the mesh surface
pub(crate) struct UvMirror<'a> {
    mesh: &'a Mesh,
    bvh: &'a Bvh,
    gbuf: &'a UvGBuffer,
    /// The texture set's material: copies land on its triangles only, the ones the texture covers
    material: usize,
    origin: Vec3,
    /// Linear maps of the copies as `origin + map * (p - origin)`, without the identity
    maps: Vec<Mat3>,
}

impl<'a> UvMirror<'a> {
    /// The copies' maps for `symmetry` about a mesh centered at `center`, `None` when it paints no copies
    pub(crate) fn new(symmetry: &Symmetry, center: Vec3, mesh: &'a Mesh, bvh: &'a Bvh, gbuf: &'a UvGBuffer, material: usize) -> Option<Self> {
        if !symmetry.any() {
            return None;
        }

        // The identity comes first: it is the dab itself
        let maps: Vec<Mat3> = symmetry.transforms().into_iter().skip(1).collect();
        (!maps.is_empty()).then(|| UvMirror { mesh, bvh, gbuf, material, origin: symmetry.origin(center), maps })
    }

    /// Add the copies of `stamp` to `out`
    pub(crate) fn add_copies(&self, stamp: &UvStamp, out: &mut Vec<UvStamp>) {
        let Some(frame) = self.frame_at(stamp.center) else { return };

        // The stamp's x axis (turned by its angle, y up on screen) on the surface; v grows down, so an angle turns towards -V
        let (sin, cos) = stamp.angle.to_radians().sin_cos();
        let x_axis = frame.along_u * cos - frame.along_v * sin;
        let reach = stamp.radius * frame.scale;
        for map in &self.maps {
            let target = self.origin + *map * (frame.pos - self.origin);
            let Some(hit) = self.bvh.nearest(target, reach, &|triangle| self.mesh.material_of(triangle as usize) == Some(self.material)) else { continue };
            let Some(surface) = surface_at(self.mesh, hit.tri, hit.u, hit.v) else { continue };
            let Some(landing) = self.frame_of(hit.tri, hit.u, hit.v) else { continue };

            // Keep the dab's orientation as the 3D view does: its direction is mapped, its shape is not mirrored
            let angle = turn_towards(&landing, *map * x_axis).map_or(stamp.angle, f32::to_degrees);
            let source = Some(StampSource { origin: self.origin, to_source: map.transpose() });
            out.push(UvStamp { center: surface.uv, angle, source, ..*stamp });
        }
    }

    /// The surface under texture coordinate `uv`, `None` off the texture or where no triangle covers it
    fn frame_at(&self, uv: Vec2) -> Option<Frame> {
        let (width, height) = (self.gbuf.width as f32, self.gbuf.height as f32);
        if !(0.0..1.0).contains(&uv.x) || !(0.0..1.0).contains(&uv.y) {
            return None;
        }

        let texel = (uv.y * height) as usize * self.gbuf.width as usize + (uv.x * width) as usize;
        let triangle = *self.gbuf.tri.get(texel).filter(|triangle| **triangle != NO_TRI)?;
        let [a, b, c] = self.mesh.tri(triangle as usize)?;
        let (ua, ub, uc) = (*self.mesh.uvs.get(a)?, *self.mesh.uvs.get(b)?, *self.mesh.uvs.get(c)?);

        // Exact barycentrics of the dab's center, not the texel center's the G-buffer holds
        let (to_b, to_c, to_point) = (ub - ua, uc - ua, uv - ua);
        let double_area = to_b.x * to_c.y - to_c.x * to_b.y;
        if double_area.abs() < f32::EPSILON {
            return None;
        }

        let b1 = (to_point.x * to_c.y - to_c.x * to_point.y) / double_area;
        let b2 = (to_b.x * to_point.y - to_point.x * to_b.y) / double_area;
        self.frame_of(triangle, b1, b2)
    }

    /// The point of `triangle` with barycentrics `b1` and `b2` of its second and third corners, with its UV directions
    fn frame_of(&self, triangle: u32, b1: f32, b2: f32) -> Option<Frame> {
        let [a, b, c] = self.mesh.tri(triangle as usize)?;
        let (pa, pb, pc) = (*self.mesh.positions.get(a)?, *self.mesh.positions.get(b)?, *self.mesh.positions.get(c)?);
        let (ua, ub, uc) = (*self.mesh.uvs.get(a)?, *self.mesh.uvs.get(b)?, *self.mesh.uvs.get(c)?);
        let (edge_b, edge_c, to_b, to_c) = (pb - pa, pc - pa, ub - ua, uc - ua);
        let double_area = to_b.x * to_c.y - to_c.x * to_b.y;
        if double_area.abs() < f32::EPSILON {
            return None;
        }

        let along_u = (edge_b * to_c.y - edge_c * to_b.y) / double_area;
        let along_v = (edge_c * to_b.x - edge_b * to_c.x) / double_area;
        let scale = (edge_b.cross(edge_c).length() / double_area.abs()).sqrt();
        let pos = pa + edge_b * b1 + edge_c * b2;
        (pos.is_finite() && along_u.is_finite() && along_v.is_finite() && scale.is_finite()).then_some(Frame { pos, along_u, along_v, scale })
    }
}

/// The angle (radians) a stamp's x axis needs in the UV frame of `landing` to point along `direction` on the surface
fn turn_towards(landing: &Frame, direction: Vec3) -> Option<f32> {
    // Least squares for direction = along_u * cos - along_v * sin, with (cos, -sin) the unknowns
    let (uu, uv, vv) = (landing.along_u.dot(landing.along_u), landing.along_u.dot(landing.along_v), landing.along_v.dot(landing.along_v));
    let (u_dot, v_dot) = (landing.along_u.dot(direction), landing.along_v.dot(direction));
    let det = uu * vv - uv * uv;
    if det.abs() < f32::EPSILON {
        return None;
    }

    let (cos, minus_sin) = ((u_dot * vv - v_dot * uv) / det, (v_dot * uu - u_dot * uv) / det);
    let sin = -minus_sin;
    (cos != 0.0 || sin != 0.0).then(|| sin.atan2(cos)).filter(|angle| angle.is_finite())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_quarter_turn_copy_turns_the_dab_angle_the_same_way() {
        // A square in the XY plane: U along +X, V down along -Y
        let positions = vec![Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 0.0), Vec3::new(-1.0, 1.0, 0.0)];
        let uvs = vec![Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 0.0)];
        let mesh = Mesh::from_parts(positions, None, uvs, vec![0, 1, 2, 0, 2, 3], "square").unwrap();
        let (bvh, gbuf) = (Bvh::new(&mesh), UvGBuffer::new(&mesh, 0, 64, 64));
        let symmetry = Symmetry { radial: 4, axis: 2, ..Symmetry::default() };
        let mirror = UvMirror::new(&symmetry, Vec3::ZERO, &mesh, &bvh, &gbuf, 0).unwrap();

        // The dab at world (0.5, 0) points 30 degrees up from +X
        let stamp = UvStamp { center: Vec2::new(0.75, 0.5), radius: 0.05, angle: 30.0, flow: 1.0, hardness: 1.0, seed: 0, source: None };
        let mut copies = Vec::new();
        mirror.add_copies(&stamp, &mut copies);

        assert_eq!(copies.len(), 3);

        // A quarter turn about +Z puts the copy at world (0, 0.5), pointing 120 degrees from +X
        let quarter = copies.iter().find(|copy| (copy.center - Vec2::new(0.5, 0.25)).length() < 0.01).unwrap();
        assert!((quarter.angle - 120.0).abs() < 0.1, "angle {}", quarter.angle);

        // Each copy is told apart from the stroke and the other copies (a smudge gives each its own trail)
        let sources: Vec<_> = copies.iter().map(|copy| copy.source).collect();
        assert!(sources.iter().all(Option::is_some), "copies carry their source");
        assert!(sources.iter().enumerate().all(|(i, a)| sources.iter().skip(i + 1).all(|b| a != b)), "every copy has its own source");
    }
}
