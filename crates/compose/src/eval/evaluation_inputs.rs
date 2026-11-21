use std::collections::HashMap;
use std::ops::Deref;
use std::sync::{Arc, Mutex, PoisonError};

use texturecraft_core::{Filter, PreparedGradient};

use super::Inputs;
use crate::ProjectorDepthCache;

pub(super) struct EvaluationInputs<'a> {
    inputs: &'a Inputs<'a>,
    pub(super) projectors: &'a ProjectorDepthCache,
    gradients: Mutex<HashMap<usize, Arc<PreparedGradient>>>,
}

impl<'a> EvaluationInputs<'a> {
    pub(super) fn new(inputs: &'a Inputs<'a>, projectors: &'a ProjectorDepthCache) -> Self {
        Self { inputs, projectors, gradients: Mutex::default() }
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
