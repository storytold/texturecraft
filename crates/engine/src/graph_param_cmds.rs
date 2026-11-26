use serde_json::{Value, json};
use texturecraft_compose::graphs::GraphParameter;
use texturecraft_core::{Channel, EffectKind, Fill, GraphRef, GraphValue, LayerId, LayerKind, Project, Source};

use crate::commands::{CommandSpec, always, bad, channel_param, content_param, strp, u64p};
use crate::{Result, Session, cmd};

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("graph.parameters", "List graph parameters", [], None, "{layer? | effect, channel? (fill source)}", always, list),
        cmd!(
            "graph.parameter.set",
            "Set graph parameter",
            [],
            None,
            "{layer? | effect, channel? (fill source), name, value (number | vector | boolean | text | null to reset), merge?}",
            always,
            set
        ),
    ]
}

#[derive(Clone, Copy)]
enum Target {
    Fill(LayerId, Channel),
    Material(LayerId, Channel),
    Effect(LayerId),
}

impl Target {
    fn from_params(session: &Session, cmd: &str, p: &Value) -> Result<Self> {
        let channel = || channel_param(cmd, p, "channel")?.ok_or_else(|| bad(cmd, "a fill source needs a `channel`"));
        if let Some(id) = u64p(p, "effect").map(LayerId) {
            return match session.doc.project.effect(id).map(|(_, effect)| &effect.kind) {
                Some(EffectKind::Material(_)) => Ok(Self::Material(id, channel()?)),
                Some(EffectKind::Graph { .. } | EffectKind::Fill { .. }) => Ok(Self::Effect(id)),
                _ => Err(bad(cmd, "the effect has no graph parameters")),
            };
        }
        Ok(Self::Fill(content_param(session, cmd, p)?, channel()?))
    }

    fn graph(self, project: &Project) -> Option<&GraphRef> {
        match self {
            Self::Fill(id, channel) => match &project.layers.get(&id)?.kind {
                LayerKind::Fill(fill) => fill.sources.get(&channel)?.graph(),
                _ => None,
            },
            Self::Material(id, channel) => match &project.effect(id)?.1.kind {
                EffectKind::Material(fill) => fill.sources.get(&channel)?.graph(),
                _ => None,
            },
            Self::Effect(id) => match &project.effect(id)?.1.kind {
                EffectKind::Graph { graph, .. } => Some(graph),
                EffectKind::Fill { source, .. } => source.graph(),
                _ => None,
            },
        }
    }

    fn change(self, project: &mut Project, reference: &GraphRef, name: &str, new: Option<GraphValue>) -> Option<()> {
        match self {
            Self::Fill(id, _) => {
                let LayerKind::Fill(fill) = &mut project.layers.get_mut(&id)?.kind else { return None };
                change_fill(fill, reference, name, new);
            }
            Self::Material(id, _) => {
                for linked in material_effects(project, id, reference)? {
                    let EffectKind::Material(fill) = &mut project.effect_mut(linked)?.kind else { return None };
                    change_fill(fill, reference, name, new.clone());
                }
            }
            Self::Effect(id) => {
                let graph = match &mut project.effect_mut(id)?.kind {
                    EffectKind::Graph { graph, .. } => graph,
                    EffectKind::Fill { source: Source::Graph(graph), .. } => graph.as_mut(),
                    _ => return None,
                };
                change_value(graph, name, new);
            }
        }
        Some(())
    }
}

fn change_fill(fill: &mut Fill, reference: &GraphRef, name: &str, new: Option<GraphValue>) {
    for source in fill.sources.values_mut() {
        if let Source::Graph(graph) = source
            && reference.shares_parameters_with(graph)
        {
            change_value(graph, name, new.clone());
        }
    }
}

fn material_effects(project: &Project, id: LayerId, reference: &GraphRef) -> Option<Vec<LayerId>> {
    let (owner, effect) = project.effect(id)?;
    let stack = &project.layers.get(&owner)?.content_effects;
    let EffectKind::Material(fill) = &effect.kind else { return None };
    let reads = |other: &Fill| other.sources.values().filter_map(Source::graph).any(|g| reference.shares_parameters_with(g));
    if reference.parameter_group.is_some() {
        return Some(stack.iter().filter(|e| matches!(&e.kind, EffectKind::Material(fill) if reads(fill))).map(|e| e.id).collect());
    }

    let at = stack.iter().position(|e| e.id == id)?;
    // Older imports identify adjacent parts of a split fill by their name, projection and shared outputs.
    let other_part = |other: &&texturecraft_core::Effect| {
        let EffectKind::Material(other_fill) = &other.kind else { return false };
        project.import.is_some()
            && other.name == effect.name
            && other_fill.projection == fill.projection
            && (other.blend != effect.blend || other.opacity != effect.opacity)
            && !other_fill.sources.keys().any(|channel| fill.sources.contains_key(channel))
            && reads(other_fill)
    };
    let before = stack.iter().take(at).rev().take_while(other_part);
    let after = stack.iter().skip(at + 1).take_while(other_part);
    Some(std::iter::once(id).chain(before.chain(after).map(|e| e.id)).collect())
}

/// An individual effect copy gets its own parameter group. Whole-layer copies retain links within the new layer.
pub(crate) fn separate_copied_parameters(effect: &mut texturecraft_core::Effect) {
    if let EffectKind::Material(fill) = &mut effect.kind {
        for source in fill.sources.values_mut() {
            if let Source::Graph(graph) = source {
                let source = graph.parameter_group.map_or(0, |[_, source]| source);
                graph.parameter_group = Some([effect.id.0, source]);
            }
        }
    }
}

fn change_value(graph: &mut GraphRef, name: &str, new: Option<GraphValue>) {
    if let Some(new) = new {
        graph.params.insert(name.into(), new);
    } else {
        graph.params.remove(name);
    }
}

fn list(session: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "graph.parameters";
    let target = Target::from_params(session, CMD, p)?;
    let graph = target.graph(&session.doc.project).ok_or_else(|| bad(CMD, "the source has no graph parameters"))?;
    let parameters = session.doc.graphs.parameters(&graph.key).map_err(|reason| bad(CMD, reason))?;
    let parameters: Vec<_> = parameters
        .iter()
        .map(|p| json!({ "name": p.name, "default": value_json(&p.default), "value": value_json(&p.value(graph)), "range": p.range, "integer": p.integer }))
        .collect();
    Ok(json!({ "graph": graph.name, "parameters": parameters }))
}

fn set(session: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "graph.parameter.set";
    let target = Target::from_params(session, CMD, p)?;
    let graph = target.graph(&session.doc.project).ok_or_else(|| bad(CMD, "the source has no graph parameters"))?;
    let name = strp(p, "name").ok_or_else(|| bad(CMD, "`name` is required"))?;
    let parameters = session.doc.graphs.parameters(&graph.key).map_err(|reason| bad(CMD, reason))?;
    let parameter = parameters.iter().find(|parameter| parameter.name == name).ok_or_else(|| bad(CMD, format!("the graph has no parameter named `{name}`")))?;
    let new = p.get("value").ok_or_else(|| bad(CMD, "`value` is required"))?;
    let new =
        if new.is_null() { None } else { Some(read_value(new, parameter).ok_or_else(|| bad(CMD, format!("invalid value for graph parameter `{name}`")))?) };
    let unchanged = match &new {
        Some(value) => *value == parameter.value(graph),
        None => !graph.params.contains_key(name),
    };
    if unchanged {
        return Ok(Value::Null);
    }
    let reference = graph.clone();
    session.edit("Change graph parameter", |session| {
        target.change(&mut session.doc.project, &reference, name, new).ok_or_else(|| bad(CMD, "the graph is missing"))?;
        Ok(Value::Null)
    })
}

fn read_value(value: &Value, parameter: &GraphParameter) -> Option<GraphValue> {
    let number = |v: &Value| v.as_f64().map(|x| x as f32).filter(|x| x.is_finite());
    let integer = |v: &Value| v.as_i64().and_then(|x| i32::try_from(x).ok());
    match &parameter.default {
        GraphValue::Bool(_) => value.as_bool().map(GraphValue::Bool),
        GraphValue::Text(_) => value.as_str().map(|v| GraphValue::Text(v.into())),
        GraphValue::Float(_) => number(value).map(GraphValue::Float),
        GraphValue::Int(_) => integer(value).map(GraphValue::Int),
        GraphValue::Int2(_) => {
            let [x, y] = value.as_array()?.as_slice() else { return None };
            Some(GraphValue::Int2([integer(x)?, integer(y)?]))
        }
        GraphValue::Vector(lanes) => {
            let input = value.as_array()?.iter();
            if input.len() != lanes.len() {
                return None;
            }
            let values: Option<Vec<_>> = if parameter.integer { input.map(|v| integer(v).map(|v| v as f32)).collect() } else { input.map(number).collect() };
            values.map(GraphValue::Vector)
        }
    }
}

fn value_json(value: &GraphValue) -> Value {
    match value {
        GraphValue::Float(value) => json!(value),
        GraphValue::Vector(value) => json!(value),
        GraphValue::Int(value) => json!(value),
        GraphValue::Int2(value) => json!(value),
        GraphValue::Bool(value) => json!(value),
        GraphValue::Text(value) => json!(value),
    }
}
