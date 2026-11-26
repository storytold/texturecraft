use super::{FittedCage, RayStart};
use crate::MeshExtent;
use crate::map::{interpolate, welded};
use crate::skew::PreparedSkew;
use crate::{BakeError, BakePaintMap, BakeRequest};
use texturecraft_core::CageType;
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::{Mesh, Surface};

/// What decides where transfer rays start: the cage mesh, the automatic cage's fitted distances, or the averaged normals, skew and painted cage amount
pub(crate) struct RayFrame<'a> {
    mesh: &'a Mesh,
    ray_normals: Vec<Vec3>,
    cage: Option<&'a Mesh>,
    fitted: Option<FittedCage>,
    skew: Option<PreparedSkew<'a>>,
    cage_paint: Option<&'a BakePaintMap>,
    /// Cage-file offsets this short count as none: the ray starts from the distance-based cage instead
    ray_epsilon: f32,
    /// World units per unit of the cage distance settings
    pub(crate) cage_units: f32,
}

impl<'a> RayFrame<'a> {
    pub(crate) fn new(request: &BakeRequest<'a>, extent: MeshExtent) -> Result<RayFrame<'a>, BakeError> {
        let settings = request.settings;

        // The automatic cage is one closed shell, so its rays always follow the averaged normals
        let automatic_high = request.high.filter(|_| settings.cage.kind == CageType::Automatic);
        let (ray_normals, fitted) = match (settings.cage.average_normals || automatic_high.is_some()) && request.high.is_some() {
            true => {
                let (welded_indices, _, normals) = welded(request.mesh, extent.diagonal);
                let ray_normals: Vec<Vec3> = welded_indices.iter().map(|index| normals.get(*index as usize).copied().unwrap_or(Vec3::Y)).collect();
                let fitted = automatic_high.map(|high| FittedCage::new(request, high, &welded_indices, &ray_normals, extent));
                (ray_normals, fitted)
            }
            false => (Vec::new(), None),
        };

        Ok(RayFrame {
            mesh: request.mesh,
            ray_normals,
            cage: if settings.cage.kind == CageType::CustomFile { request.cage } else { None },
            fitted,
            skew: request.skew.map(|skew_map| PreparedSkew::new(skew_map, request.mesh, extent.scale, settings.skew)).transpose()?,
            cage_paint: request.cage_paint,
            ray_epsilon: extent.ray_epsilon(),
            cage_units: if settings.cage.relative_to_bounding_box { extent.scale } else { 1.0 },
        })
    }

    /// Where the transfer ray of the low-poly point `surface` at `barycentric` starts
    pub(crate) fn start(&self, surface: &Surface, barycentric: [f32; 2]) -> RayStart {
        let cage_offset = self
            .cage
            .and_then(|cage_mesh| interpolate(cage_mesh, &cage_mesh.positions, surface.tri, barycentric))
            .map(|cage_position| surface.pos - cage_position)
            .filter(|offset| offset.length() > self.ray_epsilon);
        if let Some(offset) = cage_offset {
            return RayStart::CageMesh(offset);
        }

        let averaged_normal = interpolate(self.mesh, &self.ray_normals, surface.tri, barycentric).and_then(Vec3::try_normalize).unwrap_or(surface.nrm);
        let fitted_distances = self.fitted.as_ref().and_then(|fitted| fitted.distances(self.mesh, surface.tri, barycentric));
        if let Some((front, rear)) = fitted_distances {
            return RayStart::Fitted { up: averaged_normal, front, rear };
        }

        let up = match &self.skew {
            Some(skew) => skew.correct(self.mesh, surface, averaged_normal),
            None => averaged_normal,
        };
        RayStart::Distance { up, scale: self.cage_paint.map_or(1.0, |map| map.sample(surface.uv)) }
    }
}
