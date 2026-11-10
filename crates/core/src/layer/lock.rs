//! [`Lock`]: a layer whose content stands for something TextureCraft cannot compute or edit yet (an imported
//! layer built on a procedural or a filter it does not have, brush work it cannot replay), so the content is
//! kept as it came and protected from edits that would silently turn it into something else.
//!
//! A locked layer still takes part in the stack like any other. It can be shown or hidden, renamed, moved,
//! deleted, recolored, and its channel blending, opacity and mask switch can change. Everything that edits its
//! content is refused: the fill or painted pixels, the content and mask effects, the mask itself, the height
//! mask and the geometry mask ([`Layer::content_view`]). Removing the lock is an undoable edit of its own
//! (`layer.unlock`), for a user who wants to edit the layer anyway.

use serde::{Deserialize, Serialize};

use super::{Layer, LayerKind};

/// The lock source of layers a project import could not reproduce editably.
pub const IMPORT_LOCK_SOURCE: &str = "Project import";

/// One reason a layer is locked: a key naming what blocks it, stable across projects (opaque here; the source that
/// locked the layer defines it, such as `graph:<name>` for an imported compiled graph), and the text for people.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockReason {
    /// May be empty for a reason without a key.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
    pub text: String,
}

impl LockReason {
    pub fn new(key: impl Into<String>, text: impl Into<String>) -> LockReason {
        LockReason { key: key.into(), text: text.into() }
    }
}

/// Why a layer is locked and what locked it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lock {
    /// What locked it, such as [`IMPORT_LOCK_SOURCE`].
    pub source: String,
    /// What blocks it; may be empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<LockReason>,
}

impl Lock {
    pub fn new(source: impl Into<String>, reasons: Vec<LockReason>) -> Lock {
        Lock { source: source.into(), reasons }
    }

    /// A lock set by a project import.
    pub fn import(reasons: Vec<LockReason>) -> Lock {
        Lock::new(IMPORT_LOCK_SOURCE, reasons)
    }

    /// One line for a tooltip, a log or an error: the source and the reasons.
    pub fn summary(&self) -> String {
        let texts: Vec<&str> = self.reasons.iter().map(|r| r.text.as_str()).filter(|t| !t.is_empty()).collect();
        if texts.is_empty() { format!("Locked ({})", self.source) } else { format!("Locked ({}): {}", self.source, texts.join("; ")) }
    }

    /// The distinct keys of the reasons, in order (reasons without a key left out).
    pub fn keys(&self) -> Vec<&str> {
        let mut keys: Vec<&str> = Vec::new();
        for r in &self.reasons {
            if !r.key.is_empty() && !keys.contains(&r.key.as_str()) {
                keys.push(&r.key);
            }
        }
        keys
    }
}

impl Layer {
    pub fn is_locked(&self) -> bool {
        self.lock.is_some()
    }

    /// The layer as far as a lock protects it: everything that makes up its content, without what stays editable on a
    /// locked layer (name, visibility, color tag, channel blending and opacity, the mask switch, folder children and
    /// folding, the lock itself). Two views are equal when an edit left the content alone (pixels are compared apart).
    pub fn content_view(&self) -> Layer {
        let mut l = self.clone();
        l.name.clear();
        l.visible = true;
        l.color_tag = None;
        l.channels.clear();
        l.lock = None;
        if let Some(m) = &mut l.mask {
            m.enabled = true;
        }
        if let LayerKind::Group { children, collapsed } = &mut l.kind {
            children.clear();
            *collapsed = false;
        }
        l
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{GeometryMask, LayerId, Mask, MaskBackground};
    use crate::{BlendMode, Channel};

    #[test]
    fn summary_names_source_and_reasons_and_keys_are_distinct() {
        assert_eq!(Lock::import(Vec::new()).summary(), "Locked (Project import)");
        let dirt = LockReason::new("graph:dirt", "mask generator \"dirt\"");
        let again = LockReason::new("graph:dirt", "mask filter \"dirt\"");
        let l = Lock::new("import", vec![dirt, again, LockReason::new("", "brush strokes")]);
        assert_eq!(l.summary(), "Locked (import): mask generator \"dirt\"; mask filter \"dirt\"; brush strokes");
        assert_eq!(l.keys(), ["graph:dirt"]);
    }

    #[test]
    fn locks_round_trip_through_json_and_leave_empty_parts_out() {
        let l = Lock::import(vec![LockReason::new("", "strokes"), LockReason::new("graph:rust", "fill \"rust\"")]);
        let json = serde_json::to_value(&l).unwrap();
        assert_eq!(json, serde_json::json!({"source": "Project import", "reasons": [{"text": "strokes"}, {"key": "graph:rust", "text": "fill \"rust\""}]}));
        assert_eq!(serde_json::from_value::<Lock>(json).unwrap(), l);
        assert_eq!(serde_json::from_value::<Lock>(serde_json::json!({"source": "x"})).unwrap(), Lock::new("x", Vec::new()));
    }

    #[test]
    fn the_content_view_ignores_what_stays_editable() {
        let mut a = Layer::new(LayerId(1), "a", LayerKind::Paint, &[Channel::BaseColor]);
        a.mask = Some(Mask::new(MaskBackground::White));
        let mut b = a.clone();
        b.name = "renamed".into();
        b.visible = false;
        b.color_tag = Some(3);
        b.channels.insert(Channel::Roughness, crate::ChannelBlend { mode: BlendMode::Multiply, opacity: 0.5 });
        b.lock = Some(Lock::import(Vec::new()));
        if let Some(m) = &mut b.mask {
            m.enabled = false;
        }
        assert_eq!(a.content_view(), b.content_view());
        b.geometry_mask = Some(GeometryMask::UvTiles(vec![1001]));
        assert_ne!(a.content_view(), b.content_view(), "the geometry mask is content");
        let mut c = a.clone();
        c.mask = Some(Mask::new(MaskBackground::Black));
        assert_ne!(a.content_view(), c.content_view(), "the mask is content");
        let g = |children| Layer::new(LayerId(2), "g", LayerKind::Group { children, collapsed: false }, &[]);
        assert_eq!(g(vec![LayerId(3)]).content_view(), g(Vec::new()).content_view(), "a folder's children are arranged, not edited");
    }

    #[test]
    fn graph_sources_and_effects_are_content() {
        use crate::layer::{Effect, EffectKind, Fill, GraphInput, GraphRef, GraphValue, Projection, Source, text_key};
        let graph = GraphRef::new(text_key("g"), "g");
        let mut fill = Fill::default();
        fill.sources.insert(Channel::Roughness, Source::Graph(graph.clone()));
        let mut a = Layer::new(LayerId(1), "a", LayerKind::Fill(fill), &[Channel::Roughness]);
        a.mask = Some(Mask {
            effects: vec![Effect::new(LayerId(2), EffectKind::Graph { graph, projection: Projection::default() })],
            ..Mask::new(MaskBackground::White)
        });
        let edit = |f: &dyn Fn(&mut GraphRef)| {
            let mut b = a.clone();
            if let LayerKind::Fill(fill) = &mut b.kind
                && let Some(Source::Graph(g)) = fill.sources.get_mut(&Channel::Roughness)
            {
                f(g);
            }
            b
        };
        assert_ne!(
            a.content_view(),
            edit(&|g| {
                g.params.insert("scale".into(), GraphValue::Float(2.0));
            })
            .content_view(),
            "a parameter value"
        );
        assert_ne!(a.content_view(), edit(&|g| g.seed = 4).content_view(), "the seed");
        assert_ne!(
            a.content_view(),
            edit(&|g| {
                g.inputs.insert(0, GraphInput::Constant([1.0; 3]));
            })
            .content_view(),
            "an input"
        );
        assert_ne!(a.content_view(), edit(&|g| g.output = Some("other".into())).content_view(), "the output read");
        let mut masked = a.clone();
        if let Some(EffectKind::Graph { graph, .. }) = masked.mask.as_mut().and_then(|m| m.effects.first_mut()).map(|e| &mut e.kind) {
            graph.key = text_key("other");
        }
        assert_ne!(a.content_view(), masked.content_view(), "the graph a mask effect runs");
    }
}
