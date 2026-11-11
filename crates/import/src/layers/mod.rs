//! The layer stack of one texture set: the file's layer tree (bottom → top) as TextureCraft layers with the same order,
//! nesting, names, visibility, per-channel blending, content effects, masks and geometry masks, plus the [`Verdict`] on
//! how exactly the result reproduces the original.
//!
//! - [`content`]: a layer's effect stack as its base content plus content effects.
//! - [`mask`]: a layer's mask stack.
//! - [`projection`]: fill projections; [`geometry`]: geometry masks.
//!
//! A layer whose content or mask cannot be computed is locked with the blockers' keys as reasons. A visible layer whose
//! mask is not computable is hidden: without its mask it would cover the whole set (the original's result shows what it
//! contributes). Anchors are resolved in passes, one link of a chain of anchors read through anchors per pass.

pub mod content;
pub mod geometry;
pub mod mask;
pub mod projection;

use std::collections::{BTreeMap, BTreeSet};

use texturecraft_core::{Channel, Effect, EffectKind, Fidelity, Layer, LayerId, LayerKind, LayerReport, Lock, SafetyFlag, SafetyFlagKind};
use texturecraft_import_reader::blockers::Blocker;
use texturecraft_import_reader::model::{LayerKind as ProjectLayerKind, MeshMapRef, ProjectEffect, ProjectLayer, ProjectTextureSet, UnhandledSetting};

use crate::blend::{Holder, ResolvedBlend};
use crate::channels::SetChannels;
use crate::options::ImportOptions;
use crate::resources::ProjectImages;
use crate::status::Status;

/// Passes over a set's anchors at most: the longest chain of anchors read through anchors, plus one.
const MAX_ANCHOR_PASSES: usize = 8;

/// What the stack builder reads besides the layers.
pub struct SetInput<'a> {
    pub channels: &'a SetChannels,
    pub images: &'a ProjectImages<'a>,
    /// The set's mesh map slots, for bitmaps that name one.
    pub mesh_maps: &'a [MeshMapRef],
    /// Names of the set's mesh parts, for geometry masks.
    pub parts: &'a [String],
    pub options: &'a ImportOptions,
    /// Whether strokes are replayed (the import has a replayer and the options ask for it): painted masks and brush
    /// work become replay jobs instead of placeholders.
    pub replay: bool,
    /// The project's graphs, decompiled; `None` keeps graph uses placeholders.
    pub graphs: Option<&'a crate::graph_library::GraphLibrary>,
    /// The set's mesh parts, for polygon-fill picks; `None` keeps polygon fills placeholders.
    pub geometry: Option<&'a crate::polygon_fill::geometry::SetGeometry<'a>>,
    /// The original's volume unit (the scene's largest extent) in TextureCraft's projection coordinates (positions over
    /// the mesh's bounding radius): the radius over the largest extent. 1 without a mesh.
    pub volume_unit: f32,
}

/// One anchor of the set: the effect id it became, whether it sits in a mask, and how exactly what it holds is
/// computable.
#[derive(Debug, Clone, PartialEq)]
pub struct AnchorInfo {
    pub id: LayerId,
    pub in_mask: bool,
    pub status: Status,
}

/// Every anchor of a set by uid.
pub type Anchors = BTreeMap<u64, AnchorInfo>;

/// How exact each declared channel is over the visible, contributing layers, and what else the safety net needs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Verdict {
    pub channels: BTreeMap<Channel, Status>,
    /// Visible layers hidden because their mask is not computable: id, path and why.
    pub mask_failures: Vec<(LayerId, String, Vec<String>)>,
    pub layer_count: usize,
    /// The set's own settings that may change pixels.
    pub set_settings: Status,
}

impl Verdict {
    pub(crate) fn contribute(&mut self, channel: Channel, status: &Status) {
        self.channels.entry(channel).or_default().and(status);
    }

    /// True if every channel is exact.
    pub fn all_exact(&self) -> bool {
        self.channels.values().all(Status::is_exact) && self.set_settings.is_exact()
    }

    /// True if every channel is computable (exact or approximated).
    pub fn all_computable(&self) -> bool {
        self.channels.values().all(Status::is_computable)
    }
}

/// The imported layers of one set.
#[derive(Debug, Clone, Default)]
pub struct ImportedStack {
    /// Every layer, children of folders included.
    pub layers: Vec<Layer>,
    /// Top-level layers, bottom → top.
    pub roots: Vec<LayerId>,
    pub reports: BTreeMap<LayerId, LayerReport>,
    pub verdict: Verdict,
    /// The next free id after the stack's.
    pub next_id: u64,
    /// Compiled graphs the stack uses (each one a placeholder for now).
    pub graphs: BTreeSet<String>,
    pub anchors: Anchors,
    /// Strokes to replay into the stack's paint layers and paint effects.
    pub jobs: Vec<crate::replay::ReplayJob>,
    /// Polygon fills to lay into the stack's mask paint effects.
    pub fills: Vec<crate::polygon_fill::FillJob>,
}

/// Hands out layer and effect ids.
#[derive(Debug, Clone, Copy)]
pub struct Ids {
    next: u64,
}

impl Ids {
    pub fn new(next: u64) -> Ids {
        Ids { next }
    }

    pub fn alloc(&mut self) -> LayerId {
        let id = LayerId(self.next);
        self.next = self.next.saturating_add(1);
        id
    }

    pub fn next(&self) -> u64 {
        self.next
    }
}

/// Imports `set`'s layers with ids from `first_id` on.
pub fn import(set: &ProjectTextureSet, input: &SetInput, first_id: u64) -> ImportedStack {
    let mut ids = Ids::new(first_id);
    let mut anchors = Anchors::new();
    collect_anchors(&set.layers, &mut ids, &mut anchors);
    let start = ids;
    let mut stack = Builder::new(input, &anchors, start).build(&set.layers);
    for _ in 1..MAX_ANCHOR_PASSES {
        if stack.anchors == anchors {
            break;
        }
        anchors = stack.anchors.clone();
        stack = Builder::new(input, &anchors, start).build(&set.layers);
    }
    for s in set.unhandled.iter().filter(|s| s.may_change_pixels()) {
        stack.verdict.set_settings.and(&Status::approximated(s.reason()));
    }
    stack
}

/// Gives every anchor of the tree (content and masks, nested effect groups too) an effect id; anchors start exact.
fn collect_anchors(layers: &[ProjectLayer], ids: &mut Ids, anchors: &mut Anchors) {
    fn effects(list: &[ProjectEffect], in_mask: bool, ids: &mut Ids, anchors: &mut Anchors) {
        for e in list {
            match e {
                ProjectEffect::Anchor(h) => {
                    if let Some(uid) = h.uid
                        && !anchors.contains_key(&uid)
                    {
                        anchors.insert(uid, AnchorInfo { id: ids.alloc(), in_mask, status: Status::Exact });
                    }
                }
                ProjectEffect::Group(g) => effects(&g.effects, in_mask, ids, anchors),
                _ => {}
            }
        }
    }
    for l in layers {
        effects(&l.content, false, ids, anchors);
        if let Some(m) = &l.mask {
            effects(&m.effects, true, ids, anchors);
        }
        collect_anchors(l.children(), ids, anchors);
    }
}

/// What a folder passes down to its children.
#[derive(Clone)]
struct Context {
    visible: bool,
    path: String,
    /// Per channel the status of the folders' blending and masks above (`None`: a folder switches the channel off).
    channels: BTreeMap<Channel, Option<Status>>,
    /// Unhandled settings of the folders above that may change pixels.
    settings: Status,
}

struct Builder<'a> {
    input: &'a SetInput<'a>,
    anchors: &'a Anchors,
    ids: Ids,
    out: ImportedStack,
}

impl<'a> Builder<'a> {
    fn new(input: &'a SetInput<'a>, anchors: &'a Anchors, ids: Ids) -> Builder<'a> {
        let mut out = ImportedStack { anchors: anchors.clone(), ..ImportedStack::default() };
        for c in input.channels.all() {
            out.verdict.channels.insert(c, Status::Exact);
        }
        Builder { input, anchors, ids, out }
    }

    fn build(mut self, layers: &[ProjectLayer]) -> ImportedStack {
        let top = Context {
            visible: true,
            path: String::new(),
            channels: self.input.channels.all().map(|c| (c, Some(Status::Exact))).collect(),
            settings: Status::Exact,
        };
        self.out.roots = layers.iter().map(|l| self.layer(l, &top)).collect();
        self.out.next_id = self.ids.next();
        self.out
    }

    fn blend(&self, l: &ProjectLayer, channel: Channel) -> ResolvedBlend {
        ResolvedBlend::of(&l.blending, self.input.channels.project_channel(channel), Holder::Layer)
    }

    fn layer(&mut self, l: &ProjectLayer, ctx: &Context) -> LayerId {
        self.out.verdict.layer_count += 1;
        let id = self.ids.alloc();
        let path = if ctx.path.is_empty() { l.name.clone() } else { format!("{} / {}", ctx.path, l.name) };
        let visible = ctx.visible && l.visible;
        let mut flags = Vec::new();
        // Imported sets are classic (one UV tile).
        let (geometry_mask, geometry_status) = geometry::map(l.geometry.as_ref(), self.input.parts, &[]);
        let mut mask = mask::MaskBuilder::new(self.input, self.anchors, &mut self.ids).at(&path).build(l.mask.as_ref());
        self.out.jobs.append(&mut mask.jobs);
        self.out.fills.append(&mut mask.fills);
        mask.status.and(&geometry_status);
        let mut note = mask.status.clone();
        let own_settings = settings_status(l);
        note.and(&own_settings);
        let mut settings = ctx.settings.clone();
        settings.and(&own_settings);
        for s in unhandled_of(l) {
            flags.push(SafetyFlag::new(SafetyFlagKind::UnhandledSetting, s.key(), s.reason()));
        }
        let mut layer = Layer::new(id, l.name.clone(), LayerKind::Paint, &[]);
        layer.visible = l.visible;
        match &l.kind {
            ProjectLayerKind::Group(children) => {
                let mut inner = Context { visible, path: path.clone(), channels: BTreeMap::new(), settings: settings.clone() };
                for c in self.input.channels.all() {
                    let blend = self.blend(l, c);
                    layer.channels.insert(c, texturecraft_core::ChannelBlend { mode: blend.mode(), opacity: blend.opacity });
                    if blend.contributes() {
                        note.and(&blend.status());
                    }
                    let mut status = ctx.channels.get(&c).cloned().flatten();
                    if let Some(s) = status.as_mut() {
                        s.and(&mask.status);
                        s.and(&self.blend_status(&blend, &mut flags));
                    }
                    inner.channels.insert(c, status.filter(|_| blend.contributes()));
                }
                // Effects on a folder itself (rare): kept as placeholders, not computed.
                let folder_fx = content::ContentMapper::new(self.input, self.anchors, &mut self.ids).folder_effects(&l.content);
                for (_, s) in &folder_fx.1 {
                    note.and(s);
                }
                layer.content_effects = folder_fx.0;
                let kids: Vec<LayerId> = children.iter().map(|c| self.layer(c, &inner)).collect();
                layer.kind = LayerKind::Group { children: kids, collapsed: false };
            }
            kind => {
                let mut mapped = content::ContentMapper::new(self.input, self.anchors, &mut self.ids).map(l, id, &path);
                self.out.jobs.append(&mut mapped.jobs);
                self.out.graphs.extend(mapped.graphs.iter().cloned());
                for (anchor, status) in &mapped.anchor_status {
                    if let Some(a) = self.out.anchors.get_mut(anchor) {
                        a.status = status.clone();
                    }
                }
                let unknown = match kind {
                    ProjectLayerKind::Other(class) => Status::not_computable(Blocker::class(class, format!("unknown layer class {class}"))),
                    _ => Status::Exact,
                };
                note.and(&unknown);
                for (c, s) in &mapped.written {
                    note.and(s);
                    let blend = self.blend(l, *c);
                    if blend.contributes() {
                        note.and(&blend.status());
                    }
                    layer.channels.insert(*c, texturecraft_core::ChannelBlend { mode: blend.mode(), opacity: blend.opacity });
                    let Some(Some(inherited)) = ctx.channels.get(c).filter(|_| visible && blend.contributes()) else { continue };
                    let mut status = s.clone();
                    status.and(inherited);
                    status.and(&mask.status);
                    status.and(&unknown);
                    status.and(&settings);
                    status.and(&self.blend_status(&blend, &mut flags));
                    self.out.verdict.contribute(*c, &status);
                }
                layer.kind = mapped.kind;
                layer.content_effects = mapped.effects;
            }
        }
        self.out.graphs.extend(mask.graphs.iter().cloned());
        layer.mask = mask.mask;
        layer.geometry_mask = geometry_mask;
        if !note.is_computable() {
            layer.lock = Some(Lock::import(note.lock_reasons()));
            for b in note.blockers() {
                let kind = if mask.status.blockers().iter().any(|m| m.key == b.key) {
                    SafetyFlagKind::MaskNotComputable
                } else {
                    SafetyFlagKind::ContentNotComputable
                };
                flags.push(SafetyFlag::new(kind, b.key.to_string(), b.text.clone()));
            }
        }
        if l.visible && !mask.status.is_computable() {
            layer.visible = false;
            self.out.verdict.mask_failures.push((id, path.clone(), mask.status.reasons()));
        }
        let fidelity = match &note {
            Status::Exact => Fidelity::Exact,
            Status::Approximated(_) => Fidelity::Approximated,
            Status::NotComputable(_) => Fidelity::Baked,
        };
        self.out.reports.insert(id, LayerReport { role: texturecraft_core::LayerRole::Imported, fidelity, path, flags });
        self.out.layers.push(layer);
        id
    }

    /// The blend's status; a guessed mode is flagged on the layer.
    fn blend_status(&self, blend: &ResolvedBlend, flags: &mut Vec<SafetyFlag>) -> Status {
        let status = blend.status();
        if blend.mapping.confidence == crate::blend::Confidence::Guessed {
            let text = status.reasons().join("; ");
            if !flags.iter().any(|f| f.text == text) {
                flags.push(SafetyFlag::new(SafetyFlagKind::Guessed, format!("blend:{}", blend.project_mode.raw()), text));
            }
        }
        status
    }
}

/// The unhandled settings of a layer that may change pixels: its own, its enabled content effects' (nested groups
/// too) and its mask effects' while the mask is on.
fn unhandled_of(l: &ProjectLayer) -> Vec<&UnhandledSetting> {
    fn effects<'e>(list: &'e [ProjectEffect], out: &mut Vec<&'e UnhandledSetting>) {
        for e in list.iter().filter(|e| e.is_enabled()) {
            if let Some(h) = e.header() {
                out.extend(h.unhandled.iter().filter(|s| s.may_change_pixels() && concerns(e, s)));
            }
            if let ProjectEffect::Group(g) = e {
                effects(&g.effects, out);
            }
        }
    }
    let mut out: Vec<&UnhandledSetting> = l.unhandled.iter().filter(|s| s.may_change_pixels()).collect();
    effects(&l.content, &mut out);
    if let Some(m) = l.mask.as_ref().filter(|m| m.enabled) {
        effects(&m.effects, &mut out);
    }
    out
}

/// Whether a setting found on `effect` can change what it computes: the stencil's post blending only with a stencil.
fn concerns(effect: &ProjectEffect, setting: &UnhandledSetting) -> bool {
    match (effect, setting.field.as_ref()) {
        (ProjectEffect::Paint(p), "stencilPostBlending") => p.stencil.is_some(),
        _ => true,
    }
}

/// Approximated with one reason per unhandled setting of the layer that may change pixels.
fn settings_status(l: &ProjectLayer) -> Status {
    let mut out = Status::Exact;
    for s in unhandled_of(l) {
        out.and(&Status::approximated(s.reason()));
    }
    out
}

/// An effect named after its label in the file, or its kind.
pub fn effect(id: LayerId, label: &str, kind: EffectKind, visible: bool) -> Effect {
    let mut e = Effect::new(id, kind);
    if !label.is_empty() {
        e.name = label.to_string();
    }
    e.visible = visible;
    e
}

#[cfg(test)]
mod tests;
