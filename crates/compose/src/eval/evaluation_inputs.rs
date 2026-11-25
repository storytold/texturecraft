use std::collections::HashMap;
use std::ops::Deref;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use texturecraft_core::{FillSymmetry, Filter, LayerId, PreparedGradient, Project, Projection, Sampling};

use super::Inputs;
use crate::ProjectorDepthCache;
use crate::environment_sh::EnvironmentSh;
use crate::fill_depth::FillDepth;
use crate::generator_maps::GeneratorMaps;
use crate::pattern::mesh_frame;
use crate::projector_depth_cache::ViewDepth;
use crate::projector_view::ProjectorView;

/// Hold up to 30 MiB of depth maps through an evaluation; later views use the shared cache
const MAX_EVALUATION_VIEWS: usize = 10;

pub(super) struct EvaluationInputs<'a> {
    inputs: &'a Inputs<'a>,
    pub(super) projectors: &'a ProjectorDepthCache,
    environment: OnceLock<EnvironmentSh>,
    instance_draws: OnceLock<HashMap<(usize, LayerId), usize>>,
    depth_maps: Mutex<Vec<ViewDepth>>,
    /// The disabled mask being sampled directly by clone or smudge
    pub(super) sampled_disabled_mask: Option<LayerId>,
    pub(super) anchor_readers: HashMap<((LayerId, usize), LayerId), usize>,
    pub(super) whole_maps: HashMap<LayerId, GeneratorMaps>,
    pub(super) anchor_maps: HashMap<(LayerId, usize), GeneratorMaps>,
    gradients: Mutex<HashMap<usize, Arc<PreparedGradient>>>,
    field_geometry: OnceLock<u64>,
    /// The project is the session's own stack as the viewport composites it, not one cut down (a histogram's, a thumbnail's, a solo layer's)
    session_stack: bool,
}

impl<'a> EvaluationInputs<'a> {
    pub(super) fn new(inputs: &'a Inputs<'a>, projectors: &'a ProjectorDepthCache) -> Self {
        Self {
            inputs,
            projectors,
            environment: OnceLock::new(),
            instance_draws: OnceLock::new(),
            depth_maps: Mutex::new(Vec::new()),
            sampled_disabled_mask: None,
            anchor_readers: HashMap::new(),
            whole_maps: HashMap::new(),
            anchor_maps: HashMap::new(),
            gradients: Mutex::default(),
            field_geometry: OnceLock::new(),
            session_stack: false,
        }
    }

    /// These inputs evaluating the session's own stack: they keep the anchor heights the stack reads and drop the rest
    pub(super) fn of_the_session_stack(self) -> Self {
        Self { session_stack: true, ..self }
    }

    /// Whether the project evaluated is the session's own stack (see [`EvaluationInputs::of_the_session_stack`])
    pub(super) fn is_session_stack(&self) -> bool {
        self.session_stack
    }

    /// The whole-map fields share one fingerprint of the sampled geometry and layout
    pub(super) fn field_identity(&self) -> u64 {
        *self.field_geometry.get_or_init(|| crate::field_cache::field_identity(self.inputs))
    }

    pub(super) fn environment(&self) -> EnvironmentSh {
        if let Some(environment) = self.environment.get() {
            return *environment;
        }

        // Rayon may enter another plan while taking samples for the first one
        let environment = EnvironmentSh::of(&self.project.display, self.images);
        let _ = self.environment.set(environment);
        environment
    }

    /// Each path through nested folder instances gets a stable identity within an evaluation.
    pub(super) fn instance_draw(&self, parent: usize, id: LayerId) -> usize {
        let draws = self.instance_draws.get_or_init(|| {
            let mut draws = HashMap::new();
            if let Ok(set) = self.project.texture_set(self.ts) {
                collect_instance_draws(self.project, &set.stack, 0, 0, &mut draws);
            }

            draws
        });

        draws.get(&(parent, id)).copied().unwrap_or_default()
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

    pub(super) fn gradient(&self, filter: &'a Filter) -> Option<Arc<PreparedGradient>> {
        if !matches!(filter, Filter::GradientMap { linear: false, .. }) {
            return None;
        }

        // The borrowed project keeps filter addresses stable for this evaluation
        let key = std::ptr::from_ref(filter) as usize;
        let mut gradients = self.gradients.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(prepared) = gradients.get(&key) {
            return Some(Arc::clone(prepared));
        }

        let prepared = Arc::new(PreparedGradient::of(filter)?);
        gradients.insert(key, Arc::clone(&prepared));
        Some(prepared)
    }
}

impl<'a> Deref for EvaluationInputs<'a> {
    type Target = Inputs<'a>;

    fn deref(&self) -> &Self::Target {
        self.inputs
    }
}

fn collect_instance_draws(project: &Project, layers: &[LayerId], parent: usize, depth: usize, draws: &mut HashMap<(usize, LayerId), usize>) {
    if depth > texturecraft_core::project::MAX_DEPTH {
        return;
    }

    for &id in layers {
        let Some(source) = project.content_layer(id).filter(|layer| !layer.children().is_empty()) else { continue };
        let instance = if id == source.id {
            parent
        } else {
            let next = draws.len() + 1;
            *draws.entry((parent, id)).or_insert(next)
        };
        collect_instance_draws(project, source.children(), instance, depth + 1, draws);
    }
}
