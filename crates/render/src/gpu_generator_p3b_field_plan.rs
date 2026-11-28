//! The whole-map generator fields a compiled GPU program reads and what they are built from (P3b,
//! `gpu_generator_p3b.rs`).

use std::sync::Arc;

use texturecraft_compose::BakedMaps;
use texturecraft_core::BakedMap;
use texturecraft_geom::{Mesh, UvGBuffer};

use super::field_request::FieldRequest;

/// The fields, one array layer each in request order, the material whose UV islands they read,
/// and the mesh, UV coverage and occlusion bake `CompositeProgram::set_surface_inputs` gives
#[derive(Clone, Debug, Default)]
pub(crate) struct FieldPlan {
    pub(crate) requests: Vec<FieldRequest>,
    pub(crate) material: usize,
    pub(crate) surface: Option<(Arc<Mesh>, Arc<UvGBuffer>)>,
    /// The occlusion bake's values, its width and height, and its components per texel
    pub(crate) occlusion: Option<(Arc<Vec<f32>>, u32, u32, usize)>,
}

impl FieldPlan {
    pub(crate) fn new(requests: Vec<FieldRequest>, material: usize) -> Self {
        Self { requests, material, surface: None, occlusion: None }
    }

    /// Keep what the fields read, when the program has any
    pub(crate) fn set_inputs(&mut self, mesh: &Arc<Mesh>, gbuf: &Arc<UvGBuffer>, baked: Option<&BakedMaps>) {
        if self.requests.is_empty() {
            return;
        }

        self.surface = Some((mesh.clone(), gbuf.clone()));
        self.occlusion = baked.and_then(|baked| {
            let map = baked.maps.get(&BakedMap::AmbientOcclusion)?;
            let height = baked.height(BakedMap::AmbientOcclusion);
            (baked.resolution > 0 && height > 0).then(|| (map.clone(), baked.resolution, height, BakedMap::AmbientOcclusion.components()))
        });
    }
}
