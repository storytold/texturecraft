use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use texturecraft_core::{EffectKind, GraphRef, GraphValue, LayerId, LayerKind, Project};

use super::{GraphStore, GraphUse, lock};

/// A result and the parameter state of the graph whose evaluation produced it.
#[derive(Clone, Debug)]
pub(super) struct CookUse {
    pub key: String,
    pub source: Option<Arc<CookSource>>,
}

#[derive(Debug)]
pub(super) struct CookSource {
    key: String,
    params: BTreeMap<String, GraphValue>,
    seed: u32,
}

impl CookUse {
    pub fn new(key: String, source: Option<&GraphRef>) -> Self {
        let source = source.map(|graph| Arc::new(CookSource { key: graph.key.clone(), params: graph.params.clone(), seed: graph.seed }));
        Self { key, source }
    }

    fn matches(&self, graph: Option<&GraphRef>) -> bool {
        self.source
            .as_ref()
            .is_none_or(|source| graph.is_some_and(|graph| source.key == graph.key && source.params == graph.params && source.seed == graph.seed))
    }
}

impl GraphStore {
    /// Archive keys describe the graph parameter values saved alongside them.
    pub fn set_archive_uses(&self, project: &Project, uses: BTreeMap<GraphUse, String>) {
        let sources = GraphSources::new(project);
        let uses = uses
            .into_iter()
            .map(|(use_, key)| {
                let cook = CookUse::new(key, sources.graph(&use_));
                (use_, cook)
            })
            .collect();
        *lock(&self.cooks.uses) = uses;
    }

    /// Keep only results that can be presented as belonging to this project snapshot. A render
    /// for a previous edit may still run while the project is saved or after Undo.
    pub fn archive_uses(&self, project: &Project) -> BTreeMap<GraphUse, String> {
        let uses = lock(&self.cooks.uses).clone();
        let sources = GraphSources::new(project);
        uses.into_iter().filter(|(use_, cook)| cook.matches(sources.graph(use_))).map(|(use_, cook)| (use_, cook.key)).collect()
    }
}

struct GraphSources<'a> {
    project: &'a Project,
    effects: HashMap<LayerId, &'a EffectKind>,
}

impl<'a> GraphSources<'a> {
    fn new(project: &'a Project) -> Self {
        let effects = project
            .layers
            .values()
            .flat_map(|layer| layer.content_effects.iter().chain(layer.mask.iter().flat_map(|mask| &mask.effects)))
            .map(|effect| (effect.id, &effect.kind))
            .collect();
        Self { project, effects }
    }

    fn graph(&self, use_: &GraphUse) -> Option<&'a GraphRef> {
        if let Some(layer) = self.project.layers.get(&use_.id) {
            let LayerKind::Fill(fill) = &layer.kind else { return None };
            return fill.sources.get(&use_.channel?)?.graph();
        }
        match self.effects.get(&use_.id)? {
            EffectKind::Graph { graph, .. } => Some(graph),
            EffectKind::Fill { source, .. } => source.graph(),
            EffectKind::Material(fill) => fill.sources.get(&use_.channel?)?.graph(),
            _ => None,
        }
    }
}
