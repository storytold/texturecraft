//! Reuse an unchanged material output across different parameter values.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Weak};

use texturecraft_core::{GraphRef, GraphValue, StableHash};
use texturecraft_graph::graph::PreparedGraph;
use texturecraft_texel::TileImage;

use super::{Cooked, GraphAccelerator, GraphStore, RenderEnv};

pub(super) type OutputKeys = Vec<(String, String)>;

#[derive(Default)]
pub(super) struct OutputCache(HashMap<String, (String, Weak<Cooked>)>);

impl OutputCache {
    pub(super) fn find(&self, keys: &OutputKeys) -> BTreeMap<String, TileImage> {
        keys.iter()
            .filter_map(|(name, key)| {
                let (output, cook) = self.0.get(key)?;
                let cook = cook.upgrade()?;
                Some((name.clone(), cook.outputs.get(output)?.clone()))
            })
            .collect()
    }

    pub(super) fn remember(&mut self, cook: &Arc<Cooked>, keys: OutputKeys) {
        const MAX_OUTPUTS: usize = 512;
        if self.0.len() + keys.len() > MAX_OUTPUTS {
            self.0.retain(|_, (_, cook)| cook.strong_count() > 0);
            if self.0.len() + keys.len() > MAX_OUTPUTS {
                self.0.clear();
            }
        }
        for (name, key) in keys {
            self.0.insert(key, (name, Arc::downgrade(cook)));
        }
    }
}

impl GraphStore {
    pub(super) fn output_keys(
        &self,
        graph: &GraphRef,
        env: &RenderEnv<'_>,
        plan: &PreparedGraph,
        names: &[&str],
        accelerator: Option<&dyn GraphAccelerator>,
    ) -> OutputKeys {
        if names.len() < 2 {
            return Vec::new();
        }
        let environment = GraphRef { inputs: graph.inputs.clone(), seed: graph.seed, ..GraphRef::new(&graph.key, &graph.name) };
        let Some(environment) = self.reference_key(&environment, env, &[]) else { return Vec::new() };
        names
            .iter()
            .filter_map(|&name| {
                let dependencies = plan.output_parameters(name)?;
                let mut key = StableHash::new();
                key.str(&environment).str(name).str(accelerator.map_or("", GraphAccelerator::cache_tag));
                for (param, value) in graph.params.iter().filter(|(name, _)| dependencies.contains(*name)) {
                    key.str(param);
                    let lanes = value.lanes();
                    key.u64(lanes.len() as u64);
                    for value in lanes {
                        key.f32(value);
                    }
                    if let GraphValue::Text(text) = value {
                        key.str(text);
                    }
                }
                Some((name.to_owned(), key.key()))
            })
            .collect()
    }
}
