//! Where a planar or warp projection's projector looks from, for depth culling

use texturecraft_core::{Decal, Projection, Projector};
use texturecraft_geom::glam::{Vec2, Vec3};

use crate::pattern::{Coords, frame_local, mesh_frame, nonzero_stretch, projector_local};

/// The view of a depth-culled planar or warp projection: points in it run across, up and towards the projector
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProjectorView {
    /// An axis planar projection, reading positions around the mesh centre in mesh radii
    Axis { axis: u8, projector: Projector, center: Vec3, radius: f32 },
    /// A planar projection placed on the surface or a warp, from its frame
    Frame { frame: Decal, projector: Projector, radius: f32 },
}

impl ProjectorView {
    /// The view of `proj` over a mesh within `bounds`, None unless it is a planar or warp projection with depth culling on
    pub fn of(proj: &Projection, bounds: (Vec3, Vec3)) -> Option<ProjectorView> {
        proj.depth_culling()?;
        let (center, radius) = mesh_frame(bounds);
        match proj {
            Projection::Planar { placed: Some(frame), projector, .. } | Projection::Warp { frame, projector, .. } => {
                Some(ProjectorView::Frame { frame: *frame, projector: turn_only(projector), radius })
            }
            Projection::Planar { axis, projector, .. } => Some(ProjectorView::Axis { axis: *axis, projector: turn_only(projector), center, radius }),
            _ => None,
        }
    }

    /// World point `point` in the view: across, up and how near the projector (None where the view is degenerate)
    pub fn local(&self, point: Vec3) -> Option<Vec3> {
        let local = match self {
            ProjectorView::Axis { axis, projector, center, radius } => {
                let at = Coords { uv: Vec2::ZERO, pos: (point - *center) / *radius, nrm: Vec3::Z };
                let projected = projector_local(projector, &at).pos;

                // The side each axis projects from, as its backface culling reads it
                match axis {
                    0 => Vec3::new(projected.z, projected.y, -projected.x),
                    1 => Vec3::new(projected.x, projected.z, projected.y),
                    _ => projected,
                }
            }
            ProjectorView::Frame { frame, projector, .. } => frame_local(frame, projector, point)?,
        };
        local.is_finite().then_some(local)
    }

    /// The mesh radius in the view's nearness units
    pub fn unit(&self) -> f32 {
        match self {
            ProjectorView::Axis { axis, projector, .. } => 1.0 / nonzero_stretch(projector.scale[usize::from((*axis).min(2))]).abs(),
            ProjectorView::Frame { projector, radius, .. } => radius / nonzero_stretch(projector.scale[2]).abs(),
        }
    }
}

/// Only what moves the view: the turn and stretch, not the culling settings (changing those keeps the depth map)
fn turn_only(projector: &Projector) -> Projector {
    Projector { rotation: projector.rotation, scale: projector.scale, ..Projector::default() }
}
