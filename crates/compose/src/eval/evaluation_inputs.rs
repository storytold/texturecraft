use std::ops::Deref;
use std::sync::{Arc, Mutex};

use texturecraft_core::{FillSymmetry, Projection, Sampling};

use super::Inputs;
use crate::ProjectorDepthCache;
use crate::fill_depth::FillDepth;
use crate::pattern::mesh_frame;
use crate::projector_depth_cache::ViewDepth;
use crate::projector_view::ProjectorView;

/// Hold up to 30 MiB of depth maps through an evaluation; later views use the shared cache
const MAX_EVALUATION_VIEWS: usize = 10;

pub(super) struct EvaluationInputs<'a> {
    inputs: &'a Inputs<'a>,
    projectors: &'a ProjectorDepthCache,
    depth_maps: Mutex<Vec<ViewDepth>>,
}

impl<'a> EvaluationInputs<'a> {
    pub(super) fn new(inputs: &'a Inputs<'a>, projectors: &'a ProjectorDepthCache) -> Self {
        Self { inputs, projectors, depth_maps: Mutex::new(Vec::new()) }
    }

    pub(super) fn projector_depth(&self, projection: &Projection, sampling: Sampling) -> Option<Arc<FillDepth>> {
        let view = ProjectorView::of(projection, self.bounds)?;
        let copies = sampling.symmetry.on.then(|| {
            let (center, radius) = mesh_frame(self.bounds);

            // Flipping the image leaves the projector's depth view unchanged
            (FillSymmetry { flip: [false; 2], ..sampling.symmetry }, center, radius)
        });
        let key = (view, copies);

        let mut depth_maps = self.depth_maps.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((_, map)) = depth_maps.iter().find(|(cached_key, _)| *cached_key == key) {
            return map.clone();
        }

        if depth_maps.len() >= MAX_EVALUATION_VIEWS {
            // Keep overflow work out of the evaluation lock
            drop(depth_maps);
            return self.projectors.get(self.mesh, key);
        }

        let depth = self.projectors.get(self.mesh, key);
        depth_maps.push((key, depth.clone()));
        depth
    }
}

impl<'a> Deref for EvaluationInputs<'a> {
    type Target = Inputs<'a>;

    fn deref(&self) -> &Self::Target {
        self.inputs
    }
}
