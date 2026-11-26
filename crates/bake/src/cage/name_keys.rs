use super::high_poly_reach::vertex_keys;
use crate::map::transfer_keys;
use crate::tracing::ANY;
use crate::{BakeRequest, HighPoly};
use texturecraft_core::MeshMatchMode;

/// The mesh-name keys of a bake's low and high poly, per triangle and per vertex; all empty when every ray may hit every part
#[derive(Default)]
pub(super) struct NameKeys {
    pub(super) low_triangles: Vec<u32>,
    pub(super) low_vertices: Vec<u32>,
    pub(super) high_triangles: Vec<u32>,
    pub(super) high_vertices: Vec<u32>,
}

impl NameKeys {
    /// The keys `request` bakes with against `high`: filled only when it matches by mesh name
    pub(super) fn of(request: &BakeRequest, high: HighPoly) -> NameKeys {
        match request.settings.high_poly.match_mode {
            MeshMatchMode::Always => NameKeys::default(),
            MeshMatchMode::ByMeshName => {
                let (low_triangles, high_triangles) = transfer_keys(request);
                NameKeys {
                    low_vertices: vertex_keys(request.mesh, &low_triangles),
                    high_vertices: vertex_keys(high.mesh, &high_triangles),
                    low_triangles,
                    high_triangles,
                }
            }
        }
    }

    /// Whether a ray from the low-poly part keyed `low_key` may hit the high-poly part keyed `high_key`; a missing key is a part any ray may hit
    pub(super) fn matches(low_key: Option<&u32>, high_key: Option<&u32>) -> bool {
        let low_key = low_key.copied().unwrap_or(ANY);
        low_key == ANY || low_key == high_key.copied().unwrap_or(ANY)
    }
}
