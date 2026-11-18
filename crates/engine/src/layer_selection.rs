//! Several selected layers, as in the reference app: Shift+click selects the rows between the
//! last clicked layer and the clicked one, Ctrl/Cmd+click adds or removes one layer, and the
//! batch layer commands (remove, duplicate, group, visibility, move) act on every selected layer.
//!
//! The selected layer (`Session::selected`) stays the active one the properties panel shows. The
//! layers picked with it live beside it and count only while it is still the layer they were
//! picked with, so any command that selects another layer leaves a single selection without
//! having to know about this one.

use serde_json::{Value, json};
use texturecraft_core::project::MAX_DEPTH;
use texturecraft_core::{LayerId, LayerKind, Project};

use crate::commands::{bad, u64p};
use crate::session::StackTarget;
use crate::{Result, Session};

mod multi_selection;

pub use multi_selection::MultiSelection;

impl Session {
    /// Every selected layer of the active texture set, top row first: the selected layer and the
    /// layers picked with it (Shift/Ctrl+click), or the selected layer alone.
    pub fn selected_layers(&self) -> Vec<LayerId> {
        let ts = self.ts();
        let picked = self.selection_in(ts);
        if picked.len() < 2 {
            return picked;
        }

        stack_rows(&self.doc.project, ts, false).into_iter().filter(|id| picked.contains(id)).collect()
    }

    /// Whether `id` is one of the [`Session::selected_layers`].
    pub fn is_layer_selected(&self, id: LayerId) -> bool {
        let ts = self.ts();
        let Some(active) = self.active_in(ts) else { return false };

        match self.multi.get(&ts).filter(|multi| multi.active == active) {
            Some(multi) => multi.layers.contains(&id) && self.doc.project.locate(id).is_some_and(|(set, _, _)| set == ts),
            None => active == id,
        }
    }

    /// The selected layer of texture set `ts`, if it is still in that set's stack.
    fn active_in(&self, ts: usize) -> Option<LayerId> {
        self.selected.get(&ts).copied().filter(|id| self.doc.project.locate(*id).is_some_and(|(t, _, _)| t == ts))
    }

    /// The layers picked with the selected layer of `ts`, while it is still the one they were
    /// picked with.
    fn multi_in(&self, ts: usize) -> Option<&MultiSelection> {
        let active = self.active_in(ts)?;
        self.multi.get(&ts).filter(|multi| multi.active == active)
    }

    /// The selected layers of texture set `ts` in the order they were picked.
    fn selection_in(&self, ts: usize) -> Vec<LayerId> {
        match self.multi_in(ts) {
            Some(multi) => multi.layers.iter().copied().filter(|id| self.doc.project.locate(*id).is_some_and(|(t, _, _)| t == ts)).collect(),
            None => self.active_in(ts).into_iter().collect(),
        }
    }

    /// Select layer `id` alone in texture set `ts`.
    pub(crate) fn select_one(&mut self, ts: usize, id: LayerId) {
        self.selected.insert(ts, id);
        self.multi.remove(&ts);
    }

    /// Select no layer in texture set `ts`.
    pub(crate) fn select_none(&mut self, ts: usize) {
        self.selected.remove(&ts);
        self.multi.remove(&ts);
    }

    /// Select `layers` of texture set `ts` with `active` as the active layer (one of them) and
    /// `anchor` where the next Shift+click range starts.
    pub(crate) fn select_several(&mut self, ts: usize, active: LayerId, anchor: LayerId, layers: Vec<LayerId>) {
        self.selected.insert(ts, active);
        if layers.len() > 1 {
            self.multi.insert(ts, MultiSelection { active, anchor, layers });
        } else {
            self.multi.remove(&ts);
        }
    }
}

/// The rows of texture set `ts` from the top, as the Layers panel lists them: each layer, then
/// its children unless `expanded_only` and the folder is collapsed.
pub fn stack_rows(project: &Project, ts: usize, expanded_only: bool) -> Vec<LayerId> {
    let mut rows = Vec::new();
    if let Some(set) = project.texture_sets.get(ts) {
        push_rows(project, &set.stack, expanded_only, 0, &mut rows);
    }
    rows
}

fn push_rows(project: &Project, list: &[LayerId], expanded_only: bool, depth: usize, rows: &mut Vec<LayerId>) {
    if depth > MAX_DEPTH {
        return;
    }

    for id in list.iter().rev() {
        let Some(layer) = project.layers.get(id) else { continue };
        rows.push(*id);
        if let LayerKind::Group { children, collapsed } = &layer.kind
            && !(expanded_only && *collapsed)
        {
            push_rows(project, children, expanded_only, depth + 1, rows);
        }
    }
}

/// `layer.select` with `toggle` (Ctrl/Cmd+click) or `range` (Shift+click) on `id` of texture set
/// `ts`. Returns the layer that becomes the active one (none when nothing stays selected).
///
/// A range covers the rows shown between the anchor and `id` (a collapsed folder's layers are
/// not shown, so they are not selected); `id` becomes active and the anchor stays. A toggle adds
/// `id` and makes it the active layer and the anchor, or takes it out when it was selected (the
/// active layer and the anchor stay, or the last picked layer takes their place when it was them);
/// taking out the only selected layer leaves an empty selection.
pub(crate) fn extend(s: &mut Session, ts: usize, id: LayerId, range: bool) -> Option<LayerId> {
    let current = s.selection_in(ts);
    let anchor = s.multi_in(ts).map(|multi| multi.anchor).filter(|anchor| current.contains(anchor)).or_else(|| s.active_in(ts));
    if range {
        let rows = stack_rows(&s.doc.project, ts, true);
        let span = anchor.and_then(|anchor| Some((anchor, rows.iter().position(|row| *row == anchor)?, rows.iter().position(|row| *row == id)?)));
        match span {
            Some((anchor, from, to)) => {
                let layers = rows.get(from.min(to)..=from.max(to)).map(<[LayerId]>::to_vec).unwrap_or_else(|| vec![id]);
                s.select_several(ts, id, anchor, layers);
            }

            // Nothing to range from (or it is hidden in a collapsed folder): start over at `id`
            None => s.select_several(ts, id, id, vec![id]),
        }
        return Some(id);
    }

    let mut layers = current;
    if layers.contains(&id) {
        layers.retain(|layer| *layer != id);

        // Taking out the only selected layer leaves nothing selected
        let Some(last) = layers.last().copied() else {
            s.select_none(ts);
            return None;
        };

        let kept = |layer: Option<LayerId>| layer.filter(|layer| layers.contains(layer)).unwrap_or(last);
        let (active, anchor) = (kept(s.active_in(ts)), kept(anchor));
        s.select_several(ts, active, anchor, layers);
        Some(active)
    } else {
        layers.push(id);
        s.select_several(ts, id, id, layers);
        Some(id)
    }
}

/// `layer.select {layers, layer?}`: select every layer in `layers` at once, with `layer` (one of
/// them; default the last) as the active one.
pub(crate) fn select_list(s: &mut Session, p: &Value) -> Result<Value> {
    let (ts, layers) = ids_param(s, "layer.select", p)?;
    let active = match u64p(p, "layer").map(LayerId) {
        Some(active) if layers.contains(&active) => active,
        Some(_) => return Err(bad("layer.select", "`layer` must be one of `layers`")),
        None => layers.last().copied().ok_or_else(|| bad("layer.select", "`layers` is empty"))?,
    };

    // Paths paint into the active set
    if ts != s.ts() {
        s.finish_path()?;
        s.tool.clone_source = None;
    }

    s.active_set = ts;
    s.select_several(ts, active, active, layers);
    s.target = StackTarget::Content;
    s.revision = s.revision.wrapping_add(1);
    Ok(json!({ "layer": active.0, "textureSet": ts, "layers": ids_json(&s.selected_layers()) }))
}

/// `layer.select {all: true}` (Ctrl/Cmd+A in the Layers panel): every row shown in the active
/// texture set: the folder rows and the layers inside open folders, but not the layers inside
/// collapsed ones. The active layer and the Shift+click anchor stay when they are among them,
/// else the top row takes their place. An empty stack changes nothing.
pub(crate) fn select_all(s: &mut Session) -> Result<Value> {
    let ts = s.ts();
    let rows = stack_rows(&s.doc.project, ts, true);
    let Some(top) = rows.first().copied() else {
        return Ok(json!({ "layer": Value::Null, "textureSet": ts, "layers": [] }));
    };

    let active = s.active_in(ts).filter(|active| rows.contains(active)).unwrap_or(top);
    let anchor = s.multi_in(ts).map(|multi| multi.anchor).filter(|anchor| rows.contains(anchor)).unwrap_or(active);
    s.select_several(ts, active, anchor, rows);
    s.target = StackTarget::Content;
    s.revision = s.revision.wrapping_add(1);
    Ok(json!({ "layer": active.0, "textureSet": ts, "layers": ids_json(&s.selected_layers()) }))
}

/// The `layers` param: ids of layers in one texture set's stack, without repeats.
fn ids_param(s: &Session, cmd: &str, p: &Value) -> Result<(usize, Vec<LayerId>)> {
    let list = p.get("layers").and_then(Value::as_array).ok_or_else(|| bad(cmd, "`layers` must be a list of layer ids"))?;
    let mut set = None;
    let mut ids = Vec::new();
    for value in list {
        let id = value.as_u64().map(LayerId).ok_or_else(|| bad(cmd, "`layers` must be a list of layer ids"))?;
        let (ts, _, _) = s.doc.project.locate(id).ok_or_else(|| bad(cmd, format!("no layer {} in a stack", id.0)))?;
        if *set.get_or_insert(ts) != ts {
            return Err(bad(cmd, "`layers` must all be in one texture set"));
        }

        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let ts = set.ok_or_else(|| bad(cmd, "`layers` is empty"))?;
    Ok((ts, ids))
}

/// The layers a batch command acts on, top row first: `layers`, else `layer` alone, else every
/// selected layer. With `outermost`, a layer whose folder is also in the list is left out (the
/// folder takes it along).
pub(crate) fn batch_param(s: &Session, cmd: &str, p: &Value, outermost: bool) -> Result<Vec<LayerId>> {
    let ids = if p.get("layers").is_some() {
        ids_param(s, cmd, p)?.1
    } else if u64p(p, "layer").is_some() {
        vec![crate::commands::layer_param(s, cmd, p)?]
    } else {
        s.selected_layers()
    };
    if ids.len() < 2 {
        return if ids.is_empty() { Err(bad(cmd, "no layer selected and no `layer` given")) } else { Ok(ids) };
    }

    let project = &s.doc.project;
    let Some((ts, _, _)) = ids.first().and_then(|id| project.locate(*id)) else { return Ok(ids) };
    Ok(stack_rows(project, ts, false).into_iter().filter(|id| ids.contains(id) && !(outermost && in_listed_folder(project, *id, &ids))).collect())
}

/// Whether a folder holding `id` (at any depth) is in `ids`.
fn in_listed_folder(project: &Project, id: LayerId, ids: &[LayerId]) -> bool {
    let mut at = id;
    for _ in 0..=MAX_DEPTH {
        match project.locate(at) {
            Some((_, Some(parent), _)) if ids.contains(&parent) => return true,
            Some((_, Some(parent), _)) => at = parent,
            _ => return false,
        }
    }
    false
}

pub(crate) fn ids_json(ids: &[LayerId]) -> Vec<u64> {
    ids.iter().map(|id| id.0).collect()
}
