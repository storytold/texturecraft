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

    fn change(self, project: &mut Project, name: &str, new: Option<GraphValue>) -> Option<()> {
        match self {
            Self::Fill(id, channel) => {
                let LayerKind::Fill(fill) = &mut project.layers.get_mut(&id)?.kind else { return None };
                change_fill(fill, channel, name, new);
            }
            Self::Material(id, channel) => {
                let EffectKind::Material(fill) = &mut project.effect_mut(id)?.kind else { return None };
                change_fill(fill, channel, name, new);
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

fn change_fill(fill: &mut Fill, channel: Channel, name: &str, new: Option<GraphValue>) {
    let Some(graph) = fill.sources.get(&channel).and_then(Source::graph) else { return };
    let shared: Vec<_> =
        fill.sources.iter().filter_map(|(channel, source)| source.graph().filter(|other| graph.shares_parameters_with(other)).map(|_| *channel)).collect();
    for channel in shared {
        if let Some(Source::Graph(graph)) = fill.sources.get_mut(&channel) {
            change_value(graph, name, new.clone());
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
    session.edit("Change graph parameter", |session| {
        target.change(&mut session.doc.project, name, new).ok_or_else(|| bad(CMD, "the graph is missing"))?;
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
