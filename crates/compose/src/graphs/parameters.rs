//! Typed parameter controls from a stored graph's definition.

use std::collections::BTreeMap;
use std::sync::Arc;
use texturecraft_core::{GraphRef, GraphValue};
use texturecraft_graph::func::{Value, graph_inputs};

use super::{GraphStore, lock};

pub struct GraphParameter {
    pub name: String,
    pub default: GraphValue,
    pub range: Option<[f32; 2]>,
    pub integer: bool,
}

impl GraphParameter {
    /// Read the instance override with the type and lane count the graph declares.
    pub fn value(&self, graph: &GraphRef) -> GraphValue {
        let Some(value) = graph.params.get(&self.name) else { return self.default.clone() };
        let lanes = value.lanes();
        match &self.default {
            GraphValue::Bool(default) => GraphValue::Bool(lanes.first().map_or(*default, |v| *v != 0.0)),
            GraphValue::Float(default) => GraphValue::Float(lanes.first().copied().unwrap_or(*default)),
            GraphValue::Int(default) => GraphValue::Int(lanes.first().map_or(*default, |v| *v as i32)),
            GraphValue::Int2(default) => GraphValue::Int2(std::array::from_fn(|i| lanes.get(i).map_or(default[i], |v| *v as i32))),
            GraphValue::Vector(default) => {
                let read = |i, default| lanes.get(i).copied().unwrap_or(default);
                GraphValue::Vector(default.iter().copied().enumerate().map(|(i, default)| read(i, default)).collect())
            }
            GraphValue::Text(_) if matches!(value, GraphValue::Text(_)) => value.clone(),
            GraphValue::Text(_) => self.default.clone(),
        }
    }
}

impl GraphStore {
    /// Parameter definitions from the parsed graph. Imported projects already store input defaults and exposed ranges.
    pub fn parameters(&self, key: &str) -> Result<Arc<[GraphParameter]>, String> {
        if let Some(parameters) = lock(&self.read_cache().parameters).get(key) {
            return Ok(parameters.clone());
        }
        let graph = self.parsed(key)?;
        let exposures: BTreeMap<_, _> = graph.exposed.iter().map(|e| (e.label.as_str(), e)).collect();
        let mut parameters = BTreeMap::new();
        let range = |lo: f32, hi: f32| (lo.is_finite() && hi.is_finite() && lo < hi).then_some([lo, hi]);
        for (name, value) in graph_inputs(&graph) {
            if name.starts_with('$') {
                continue;
            }
            let bounds = exposures.get(name.as_str()).and_then(|e| range(e.min, e.max));
            let (default, integer) = match value {
                Value::Bool(value) => (GraphValue::Bool(value), false),
                Value::Str(value) => (GraphValue::Text(value.to_string()), false),
                Value::Num(value) => {
                    let v = value.v.map(|x| x as f32);
                    let [x, y, ..] = v;
                    let default = match (value.lanes, value.int) {
                        (1, true) => GraphValue::Int(x as i32),
                        (2, true) => GraphValue::Int2([x as i32, y as i32]),
                        (1, false) => GraphValue::Float(x),
                        _ => GraphValue::Vector(v.into_iter().take(usize::from(value.lanes)).collect()),
                    };
                    (default, value.int)
                }
            };
            parameters.insert(name.clone(), GraphParameter { name, default, range: bounds, integer });
        }
        for (name, exposure) in exposures {
            let Some(node) = graph.nodes.get(&exposure.node) else { continue };
            let value = node.nums.get(&exposure.param).copied().unwrap_or(exposure.min);
            parameters.entry(name.to_owned()).or_insert_with(|| GraphParameter {
                name: name.to_owned(),
                default: GraphValue::Float(value),
                range: range(exposure.min, exposure.max),
                integer: false,
            });
        }
        let parameters: Arc<[GraphParameter]> = parameters.into_values().collect();
        lock(&self.read_cache().parameters).insert(key.to_owned(), parameters.clone());
        Ok(parameters)
    }
}
