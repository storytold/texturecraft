//! The projector a 3D projection's preview square draws round its object, as the 3D view's gizmo shows it round the model: a planar's frame with the dot its image travels to, a sphere's rim, equator and meridian, a tri-planar box or a cylinder

use texturecraft_engine::compose::axis_frame::axis_frame;
use texturecraft_engine::compose::pattern::{cylinder_axis_index, decal_axes};
use texturecraft_engine::core::Projection;
use texturecraft_engine::geom::glam::{Vec2, Vec3};

use crate::round_gizmo::{BOX_REACH, CIRCLE_POINTS, CYLINDER_HEIGHT, CYLINDER_REACH, SPHERE_REACH, box_corner, box_edges};

mod place;

use place::Place;

/// How far in front of the object a planar's frame stands (preview units)
const FRAME_GAP: f32 = 0.15;

/// How far a planar's direction line reaches back from its frame (preview units)
const DIRECTION_LENGTH: f32 = 0.35;

/// The projector's outline in the preview's coordinates (the compositor's: the mesh's centre at the origin, its half diagonal 1)
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ProjectorOutline {
    /// Polylines; a closed one repeats its first point at its end
    pub lines: Vec<Vec<Vec3>>,
    /// Where a planar's image travels to from its frame: the direction dot
    pub tip: Option<Vec3>,
    /// The projector's centre: a frame's middle, a round projector's centre
    pub center: Vec3,
}

impl ProjectorOutline {
    /// Build an outline for an axis-aligned planar, sphere, cylinder or tri-planar projector around an object with the given half sizes.
    pub(crate) fn of(projection: &Projection, object: Vec3) -> Option<ProjectorOutline> {
        let outline = match projection {
            Projection::Planar { scale, axis, offset, projector, placed: None, .. } => {
                // The compositor's coordinates as a mesh: centred, half diagonal 1, so each corner sits 1/√3 out along every axis
                let corner = Vec3::splat(1.0 / 3f32.sqrt());
                let frame = axis_frame(*scale, *axis, Vec3::from(*offset), projector, (-corner, corner))?;
                planar(frame.size, decal_axes(&frame)?, object)
            }
            Projection::Triplanar { scale, offset, projector, .. } => {
                let place = Place::new(projector.rotation, projector.scale, Vec3::from(*offset), *scale);
                let size = object * BOX_REACH;
                let corner = |index: usize| place.world(place.local + size * box_corner(index));
                let lines = box_edges().map(|(first, second)| vec![corner(first), corner(second)]).collect();

                ProjectorOutline { lines, tip: None, center: place.world(place.local) }
            }
            Projection::Spherical { offset, projector, .. } => {
                let place = Place::new(projector.rotation, projector.scale, Vec3::from(*offset), 1.0);
                let radius = object.max_element() * SPHERE_REACH;
                let (u, v) = place.facing().any_orthonormal_pair();
                let ring = |a: Vec3, b: Vec3| circle(|t| place.world(place.local + (a * t.x + b * t.y) * radius));
                ProjectorOutline { lines: vec![ring(u, v), ring(Vec3::X, Vec3::Z), ring(Vec3::Y, Vec3::Z)], tip: None, center: place.world(place.local) }
            }
            Projection::Cylindrical { axis, offset, projector, .. } => {
                let place = Place::new(projector.rotation, projector.scale, Vec3::from(*offset), 1.0);
                let axis = cylinder_axis_index(*axis);
                let along = Vec3::AXES[axis];
                let (u, v) = (Vec3::AXES[(axis + 1) % Vec3::AXES.len()], Vec3::AXES[(axis + 2) % Vec3::AXES.len()]);
                let radius = (object * (Vec3::ONE - along)).max_element() * CYLINDER_REACH;
                let half_height = object[axis] * CYLINDER_HEIGHT;
                let cap = |height: f32| circle(|t| place.world(place.local + along * height + (u * t.x + v * t.y) * radius));
                let mut lines = vec![cap(half_height), cap(-half_height)];

                // The two side lines, where the round side turns away from the camera
                let facing = place.facing();
                if let Some(side) = along.cross(facing - along * facing.dot(along)).try_normalize() {
                    for edge in [side, -side] {
                        lines.push(vec![
                            place.world(place.local + edge * radius + along * half_height),
                            place.world(place.local + edge * radius - along * half_height),
                        ]);
                    }
                }
                ProjectorOutline { lines, tip: None, center: place.world(place.local) }
            }
            Projection::Planar { placed: Some(_), .. } | Projection::Warp { .. } | Projection::Uv { .. } | Projection::UvTile => return None,
        };
        let finite = outline.points().all(|point| point.is_finite()) && outline.center.is_finite();
        finite.then_some(outline)
    }

    /// Every point the outline draws, the direction dot included
    pub(crate) fn points(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.lines.iter().flatten().copied().chain(self.tip)
    }
}

/// A planar's frame, `size` across and up on the frame `axes` (origin, across, up, out), stood just in front of the object on the side its image reads from, and the line its image travels along into the object
fn planar(size: [f32; 2], (origin, across, up, out): (Vec3, Vec3, Vec3, Vec3), object: Vec3) -> ProjectorOutline {
    let center = origin + out * (object.max_element() + FRAME_GAP - origin.dot(out));
    let (across, up) = (across * size[0] / 2.0, up * size[1] / 2.0);
    let corners = [center - across - up, center + across - up, center + across + up, center - across + up, center - across - up];
    let tip = center - out * DIRECTION_LENGTH;
    ProjectorOutline { lines: vec![corners.to_vec(), vec![center, tip]], tip: Some(tip), center }
}

/// A closed circle through `at` of the unit circle's points
fn circle(at: impl Fn(Vec2) -> Vec3) -> Vec<Vec3> {
    (0..=CIRCLE_POINTS)
        .map(|index| {
            let angle = index as f32 / CIRCLE_POINTS as f32 * std::f32::consts::TAU;
            at(Vec2::new(angle.cos(), angle.sin()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_engine::core::{Projector, UvTransform};

    const BALL: Vec3 = Vec3::splat(0.85);

    fn spherical(projector: Projector, offset: [f32; 3]) -> Projection {
        Projection::Spherical { scale: 1.0, uv: UvTransform::default(), offset, projector }
    }

    #[test]
    fn every_3d_projection_has_its_projector_outline() {
        let planar = Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3], uv: UvTransform::default(), projector: Projector::default(), placed: None };
        let outline = ProjectorOutline::of(&planar, BALL).unwrap();

        assert_eq!(outline.lines.len(), 2, "a frame and its direction line");
        let tip = outline.tip.unwrap();
        assert!(tip.z < BALL.z + FRAME_GAP && outline.lines[0][0].z > BALL.z, "the frame stands in front of the ball, the dot points into it");

        assert_eq!(ProjectorOutline::of(&spherical(Projector::default(), [0.0; 3]), BALL).unwrap().lines.len(), 3, "rim, equator, meridian");
        let cylinder =
            Projection::Cylindrical { scale: 1.0, axis: 1, uv: UvTransform::default(), offset: [0.0; 3], projector: Projector::default(), angle: 360.0 };
        assert_eq!(ProjectorOutline::of(&cylinder, Vec3::new(0.55, 0.7, 0.55)).unwrap().lines.len(), 4, "two caps, two side lines");
        let triplanar = Projection::triplanar();
        assert_eq!(ProjectorOutline::of(&triplanar, Vec3::splat(0.55)).unwrap().lines.len(), 12, "a box's edges");
        assert!(ProjectorOutline::of(&Projection::UvTile, BALL).is_none());
    }

    #[test]
    fn a_cylinder_axis_past_z_wraps_round_y_as_the_compositor_reads_it() {
        let cylinder = |axis: u8| Projection::Cylindrical {
            scale: 1.0,
            axis,
            uv: UvTransform::default(),
            offset: [0.0; 3],
            projector: Projector::default(),
            angle: 360.0,
        };
        let object = Vec3::new(0.55, 0.7, 0.4);

        assert_eq!(ProjectorOutline::of(&cylinder(3), object), ProjectorOutline::of(&cylinder(1), object));
    }

    #[test]
    fn the_outline_follows_the_projectors_size_and_centre() {
        let reach = |projection: &Projection| ProjectorOutline::of(projection, BALL).unwrap().points().map(Vec3::length).fold(0.0, f32::max);
        let plain = reach(&spherical(Projector::default(), [0.0; 3]));

        assert!((plain - 0.85 * SPHERE_REACH).abs() < 1e-4, "{plain}");
        assert!((reach(&spherical(Projector { scale: [3.0; 3], ..Projector::default() }, [0.0; 3])) - 3.0 * plain).abs() < 1e-3, "x3 is three times as far");
        assert!(reach(&spherical(Projector::default(), [0.5, 0.0, 0.0])) > plain + 0.4, "an offset moves it");
    }
}
