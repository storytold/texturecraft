//! [`ContentMapper`]: a layer's content (its effect stack, bottom → top) as TextureCraft's base content plus content
//! effects, effect for effect, with the status of every channel the content writes.
//!
//! - The base: a fill layer's first fill becomes a fill layer when it is switched on and writes every channel it
//!   touches from a value, a bitmap or an anchor at full strength (normal or replace, 100%). Otherwise the base is an
//!   empty paint layer and the first fill stays an effect.
//! - A paint layer's brush work stays only in the original's stored result: strokes are not replayed in this
//!   version, so each paint effect with strokes is a placeholder and the layer is locked (`strokes:<tool>`).
//! - Every other effect becomes one of TextureCraft's: a fill becomes one material fill effect per distinct blend
//!   mode and opacity among its channels (TextureCraft's effects have one blend each) plus one placeholder per compiled
//!   graph it uses ([`crate::graph_uses`]); levels and anchors keep their kind; filters are placeholders (or a close
//!   built-in filter when asked for); colour selection in content, polygon fills and unknown classes are placeholders.
//!
//! TextureCraft's levels and filters act on every channel of the layer, the file's only on the channels they list: an
//! effect listing fewer channels than the layer writes is approximated.

use std::collections::BTreeMap;

use texturecraft_core::{BlendMode, Channel, Effect, EffectKind, Fill, LayerKind, Levels, Projection, Source};
use texturecraft_import_reader::blockers::{Blocker, BlockerKey, BlockerKind};
use texturecraft_import_reader::model::{
    EffectHeader, FillEffect, Levels as ProjectLevels, LevelsEffect, ProjectEffect, ProjectLayer, ProjectSource, ReferenceSource,
};

use super::{Anchors, Ids, SetInput, effect, projection};
use crate::blend::{Holder, ResolvedBlend};
use crate::channels::bit_of;
use crate::graph_uses::{self, GraphUse};
use crate::resources::{MeshMapAlias, mesh_map_alias};
use crate::status::Status;

/// A layer's mapped content.
pub struct Mapped {
    pub kind: LayerKind,
    pub effects: Vec<Effect>,
    /// Channels the content writes, with the status of what it writes there.
    pub written: BTreeMap<Channel, Status>,
    pub graphs: Vec<String>,
    /// The status of what each anchor of the content holds (the content below it).
    pub anchor_status: Vec<(u64, Status)>,
}

pub struct ContentMapper<'a, 'i> {
    input: &'a SetInput<'a>,
    anchors: &'a Anchors,
    ids: &'i mut Ids,
    effects: Vec<Effect>,
    written: BTreeMap<Channel, Status>,
    graphs: Vec<String>,
    anchor_status: Vec<(u64, Status)>,
}

/// The channels and sources of one fill, grouped the way TextureCraft lays them on.
struct FillParts {
    /// Sources by blend: one group per distinct mode and opacity.
    groups: Vec<(ResolvedBlend, BTreeMap<Channel, Source>)>,
    /// Compiled graphs with the channels they write.
    graphs: Vec<(GraphUse, Vec<Channel>, ResolvedBlend)>,
    status: BTreeMap<Channel, Status>,
    projection: Projection,
}

impl<'a, 'i> ContentMapper<'a, 'i> {
    pub fn new(input: &'a SetInput<'a>, anchors: &'a Anchors, ids: &'i mut Ids) -> ContentMapper<'a, 'i> {
        ContentMapper { input, anchors, ids, effects: Vec::new(), written: BTreeMap::new(), graphs: Vec::new(), anchor_status: Vec::new() }
    }

    /// The base and effects of a layer that is not a folder.
    pub fn map(mut self, l: &ProjectLayer) -> Mapped {
        let mut kind = LayerKind::Paint;
        let mut rest: &[ProjectEffect] = &l.content;
        match (&l.kind, l.content.split_first()) {
            (texturecraft_import_reader::model::LayerKind::Fill, Some((ProjectEffect::Fill(first), after))) if first.header.enabled => {
                let parts = self.fill_parts(first);
                if let Some(fill) = as_base(&parts) {
                    kind = LayerKind::Fill(fill);
                    self.note_all(&parts.status);
                    rest = after;
                }
            }
            (texturecraft_import_reader::model::LayerKind::Paint, Some((ProjectEffect::Group(g), after))) if g.header.enabled => {
                // the brush work: what the paint layer holds itself
                for e in &g.effects {
                    self.effect(e, true);
                }
                rest = after;
            }
            _ => {}
        }
        for e in rest {
            self.effect(e, true);
        }
        Mapped { kind, effects: self.effects, written: self.written, graphs: self.graphs, anchor_status: self.anchor_status }
    }

    /// Effects on a folder (rare): kept as placeholders, not computed; with the status of every channel they write.
    pub fn folder_effects(mut self, content: &[ProjectEffect]) -> (Vec<Effect>, Vec<(Channel, Status)>) {
        let mut statuses = Vec::new();
        for e in content {
            let channels = self.channels_of(e.header());
            let label = format!("folder effect ({})", e.kind());
            let status = Status::not_computable(Blocker::feature("folder-effects", "effects on a folder"));
            self.placeholder(&label, "effects on folders are not computed yet", e.is_enabled());
            if e.is_enabled() {
                statuses.extend(channels.into_iter().map(|c| (c, status.clone())));
            }
        }
        (self.effects, statuses)
    }

    fn note(&mut self, channel: Channel, status: &Status) {
        self.written.entry(channel).or_default().and(status);
    }

    fn note_all(&mut self, statuses: &BTreeMap<Channel, Status>) {
        for (c, s) in statuses {
            self.note(*c, s);
        }
    }

    /// The declared channels an effect writes: those its `channelTypes` lists (all for an empty list).
    fn channels_of(&self, header: Option<&EffectHeader>) -> Vec<Channel> {
        let ch = self.input.channels;
        ch.all().filter(|c| header.is_none_or(|h| h.channels.is_empty() || ch.mask_has(h.channels, *c))).collect()
    }

    /// The declared channels an effect writes and whose blending (as `holder`) does not switch off.
    fn writes(&self, header: &EffectHeader, holder: Holder) -> Vec<Channel> {
        let ch = self.input.channels;
        self.channels_of(Some(header)).into_iter().filter(|c| ResolvedBlend::of(&header.blending, ch.project_channel(*c), holder).contributes()).collect()
    }

    fn placeholder(&mut self, label: &str, reason: &str, visible: bool) {
        let id = self.ids.alloc();
        let mut e = Effect::placeholder(id, label, reason);
        e.visible = visible;
        self.effects.push(e);
    }

    /// Places a compiled graph's use writing `channels`.
    fn graph_use(&mut self, g: GraphUse, channels: &[Channel], blend: Option<&ResolvedBlend>, visible: bool) {
        let mut e = effect(self.ids.alloc(), "", g.effect, visible);
        if let Some(b) = blend {
            e.blend = b.mode();
            e.opacity = b.opacity;
        }
        self.effects.push(e);
        if visible {
            for c in channels {
                self.note(*c, &g.status);
            }
        }
        if !self.graphs.contains(&g.graph) {
            self.graphs.push(g.graph);
        }
    }

    /// One effect of the content (`on`: it and everything holding it is switched on).
    fn effect(&mut self, e: &ProjectEffect, on: bool) {
        // An effect of a class the reader does not know has no switch to read: it counts as on.
        let on = on && (e.is_enabled() || matches!(e, ProjectEffect::Other(_)));
        match e {
            ProjectEffect::Fill(fill) => self.fill(fill, on),
            ProjectEffect::Group(g) => {
                let channels = self.channels_of(Some(&g.header));
                let passthrough = channels.iter().all(|c| {
                    ResolvedBlend::of(&g.header.blending, self.input.channels.project_channel(*c), Holder::EffectGroup).mode() == BlendMode::Passthrough
                });
                let before = self.written.keys().copied().collect::<Vec<_>>();
                for inner in &g.effects {
                    self.effect(inner, on);
                }
                if !passthrough && on {
                    let why = Status::approximated("effect group with its own blending laid flat");
                    let touched: Vec<Channel> = self.written.keys().copied().filter(|c| !before.contains(c) || channels.contains(c)).collect();
                    for c in touched {
                        self.note(c, &why);
                    }
                }
            }
            ProjectEffect::Paint(p) => {
                if p.stroke_count() == 0 {
                    return;
                }
                let channels = match p.colors.is_empty() {
                    true => self.channels_of(Some(&p.header)),
                    false => {
                        self.input.channels.all().filter(|c| p.colors.iter().any(|s| s.channels().contains(self.input.channels.project_channel(*c)))).collect()
                    }
                };
                let label = format!("{} strokes ({})", p.tool.name(), p.stroke_count());
                self.placeholder(&label, "brush strokes are kept in the original result; stroke replay comes later", on);
                if on {
                    let why = Status::not_computable(Blocker::new(BlockerKey::new(BlockerKind::Strokes, p.tool.name()), format!("{} strokes", p.tool.name())));
                    for c in channels {
                        self.note(c, &why);
                    }
                }
            }
            ProjectEffect::Filter(f) => {
                let channels = self.writes(&f.header, Holder::Filter);
                let Some(_) = f.procedural.as_ref() else {
                    // "no filter selected": changes nothing; kept switched off
                    self.placeholder("filter (none selected)", "the original's filter has no graph selected", false);
                    return;
                };
                let blend = channels.first().map(|c| ResolvedBlend::of(&f.header.blending, self.input.channels.project_channel(*c), Holder::Filter));
                if self.input.options.native_filters
                    && let Some((filter, note)) = graph_uses::native_filter(f)
                {
                    let mut e = effect(self.ids.alloc(), &f.header.label, EffectKind::Filter(filter), on);
                    if let Some(b) = &blend {
                        e.blend = b.mode();
                        e.opacity = b.opacity;
                    }
                    self.effects.push(e);
                    if on {
                        let mut s = Status::approximated(note);
                        s.and(&self.subset_status(&channels));
                        for c in &channels {
                            self.note(*c, &s);
                        }
                    }
                    return;
                }
                self.graph_use(graph_uses::filter(f, false), &channels, blend.as_ref(), on);
            }
            ProjectEffect::Levels(l) => self.levels(l, on),
            ProjectEffect::ColorSelection(c) => {
                let channels = self.writes(&c.header, Holder::Layer);
                self.placeholder("colour selection", "colour selection in layer content is not computed", on);
                if on {
                    let why = Status::not_computable(Blocker::feature("colour-selection-content", "colour selection in layer content"));
                    for c in channels {
                        self.note(c, &why);
                    }
                }
            }
            ProjectEffect::Anchor(h) => {
                let id = h.uid.and_then(|u| self.anchors.get(&u)).map_or_else(|| self.ids.alloc(), |a| a.id);
                self.effects.push(effect(id, &h.label, EffectKind::Anchor, on));
                if let Some(uid) = h.uid {
                    let mut held = Status::Exact;
                    for s in self.written.values() {
                        held.and(s);
                    }
                    self.anchor_status.push((uid, held));
                }
            }
            ProjectEffect::PolygonFill(p) => {
                let writes = self.writes(&p.header, Holder::Layer).contains(&Channel::BaseColor);
                self.placeholder("polygon fill", "polygon fills are not computed yet", on);
                if on && writes {
                    self.note(Channel::BaseColor, &Status::not_computable(Blocker::feature("polygon-fill", "polygon fill")));
                }
            }
            ProjectEffect::Other(raw) => {
                self.placeholder(&format!("unknown effect {}", raw.class), "an effect class this version does not read", on);
                if on {
                    let why = Status::not_computable(Blocker::class(&raw.class, format!("unknown effect {}", raw.class)));
                    for c in self.channels_of(None) {
                        self.note(c, &why);
                    }
                }
            }
        }
    }

    /// Approximated when the effect lists fewer channels than the layer writes so far (TextureCraft applies it to all).
    fn subset_status(&self, channels: &[Channel]) -> Status {
        match self.written.keys().any(|c| !channels.contains(c)) {
            true => Status::approximated("an effect on some channels applies to every channel of the layer"),
            false => Status::Exact,
        }
    }

    fn levels(&mut self, l: &LevelsEffect, on: bool) {
        let channels = self.writes(&l.header, Holder::Filter);
        let (levels, mut status) = levels(l.levels.as_ref());
        let blend = channels.first().map(|c| ResolvedBlend::of(&l.header.blending, self.input.channels.project_channel(*c), Holder::Filter));
        let mut e = effect(self.ids.alloc(), &l.header.label, EffectKind::Levels(levels), on);
        if let Some(b) = &blend {
            e.blend = b.mode();
            e.opacity = b.opacity;
            status.and(&b.status());
            if b.mode() != BlendMode::Normal {
                status.and(&Status::approximated("levels with a blend mode other than normal"));
            }
        }
        status.and(&self.subset_status(&channels));
        self.effects.push(e);
        if on {
            for c in channels {
                self.note(c, &status);
            }
        }
    }

    /// A fill that is not the layer's base: one material fill effect per blend group, one placeholder per graph.
    fn fill(&mut self, fill: &FillEffect, on: bool) {
        let parts = self.fill_parts(fill);
        for (blend, sources) in &parts.groups {
            let mut e = effect(
                self.ids.alloc(),
                &fill.header.label,
                EffectKind::Material(Fill { sources: sources.clone(), projection: parts.projection.clone(), link: None }),
                on,
            );
            e.blend = blend.mode();
            e.opacity = blend.opacity;
            self.effects.push(e);
        }
        for (g, channels, blend) in parts.graphs {
            self.graph_use(g, &channels, Some(&blend), on);
        }
        if on {
            self.note_all(&parts.status);
        }
    }

    /// What one fill writes per channel.
    fn fill_parts(&mut self, fill: &FillEffect) -> FillParts {
        let ch = self.input.channels;
        let mapped = projection::map(fill);
        let mut parts = FillParts { groups: Vec::new(), graphs: Vec::new(), status: BTreeMap::new(), projection: mapped.projection.clone() };
        let mut graph_channels: Vec<(usize, Vec<Channel>, ResolvedBlend)> = Vec::new();
        for c in self.writes(&fill.header, Holder::Layer) {
            let pc = ch.project_channel(c);
            let Some(index) = fill.sources.iter().position(|s| {
                let m = s.channels();
                m.contains(pc) || (m.is_all() && matches!(s, ProjectSource::Procedural(_)))
            }) else {
                continue;
            };
            let Some(source) = fill.sources.get(index) else { continue };
            let blend = ResolvedBlend::of(&fill.header.blending, pc, Holder::Layer);
            let mut status = blend.status();
            let ours = match source {
                ProjectSource::Uniform(u) => {
                    if projection::shapes_coverage(fill) {
                        status.and(&Status::approximated("uniform fill faded or clipped by its projection"));
                    }
                    let v = if c.components() == 3 { u.color } else { [u.color[0]; 3] };
                    Some(Source::Uniform(v.map(|x| if x.is_finite() { x } else { 0.0 })))
                }
                ProjectSource::Bitmap(b) => {
                    status.and(&mapped.status);
                    match self.bitmap(&b.url) {
                        Ok(s) => Some(s),
                        Err(why) => {
                            status.and(&why);
                            None
                        }
                    }
                }
                ProjectSource::Reference(r) => match self.reference(r, c) {
                    Ok(Some((source, s))) => {
                        status.and(&s);
                        Some(source)
                    }
                    Ok(None) => continue,
                    Err(why) => {
                        status.and(&why);
                        None
                    }
                },
                ProjectSource::Procedural(_) => {
                    match graph_channels.iter_mut().find(|(i, ..)| *i == index) {
                        Some((_, channels, _)) => channels.push(c),
                        None => graph_channels.push((index, vec![c], blend)),
                    }
                    continue;
                }
                ProjectSource::Other(raw) => {
                    status.and(&Status::not_computable(Blocker::class(&raw.class, format!("fill source {}", raw.class))));
                    None
                }
            };
            parts.status.entry(c).or_default().and(&status);
            if let Some(source) = ours {
                match parts.groups.iter_mut().find(|(b, _)| b.mode() == blend.mode() && b.opacity == blend.opacity) {
                    Some((_, sources)) => {
                        sources.insert(c, source);
                    }
                    None => parts.groups.push((blend, BTreeMap::from([(c, source)]))),
                }
            }
        }
        for (index, channels, blend) in graph_channels {
            let Some(ProjectSource::Procedural(p)) = fill.sources.get(index) else { continue };
            let names: Vec<String> = channels.iter().map(|c| ch.label(*c)).collect();
            let g = graph_uses::procedural_fill(p, &names);
            for c in &channels {
                parts.status.entry(*c).or_default().and(&g.status);
            }
            parts.graphs.push((g, channels, blend));
        }
        parts
    }

    /// A bitmap fill source: the image as a resource of the imported project.
    fn bitmap(&self, url: &str) -> Result<Source, Status> {
        match mesh_map_alias(url, self.input.mesh_maps) {
            MeshMapAlias::NotAlias => {}
            _ => return Err(Status::not_computable(Blocker::feature("mesh-map-fill", "fill from a mesh map slot"))),
        }
        match self.input.images.load(url) {
            Ok(loaded) => Ok(Source::image(loaded.name)),
            Err(why) => Err(Status::not_computable(Blocker::feature("unreadable-bitmap", why))),
        }
    }

    /// A fill reading an anchor: the anchor's stack value for the channel its first binding names (else `channel`).
    /// `Ok(None)` for a dangling reference, which changes nothing.
    fn reference(&self, r: &ReferenceSource, channel: Channel) -> Result<Option<(Source, Status)>, Status> {
        reference(self.anchors, r, channel)
    }
}

/// A fill's only blend group as a fill layer's base: switched on, everything computed, at full strength.
fn as_base(parts: &FillParts) -> Option<Fill> {
    let [(blend, sources)] = parts.groups.as_slice() else { return None };
    let full = matches!(blend.mode(), BlendMode::Normal | BlendMode::Replace) && blend.opacity >= 1.0;
    let computed = parts.graphs.is_empty() && parts.status.values().all(Status::is_computable);
    (full && computed).then(|| Fill { sources: sources.clone(), projection: parts.projection.clone(), link: None })
}

/// True if the reference points at no anchor: the file stores 0 once the anchor it read is gone; the original then
/// renders as if nothing were connected.
pub fn is_dangling(r: &ReferenceSource) -> bool {
    r.anchor_uid.is_none_or(|u| u == 0)
}

/// A reference as an anchor source (shared by content and masks).
pub fn reference(anchors: &Anchors, r: &ReferenceSource, channel: Channel) -> Result<Option<(Source, Status)>, Status> {
    if is_dangling(r) {
        return Ok(None);
    }
    let Some(info) = r.anchor_uid.and_then(|u| anchors.get(&u)) else {
        return Err(Status::not_computable(Blocker::feature("missing-anchor", "reference to an anchor that is not in the set")));
    };
    if info.in_mask {
        return Err(Status::not_computable(Blocker::feature("mask-anchor", "reference to an anchor in a mask")));
    }
    if !info.status.is_computable() {
        let mut s = Status::Exact;
        for b in info.status.blockers() {
            s.and(&Status::not_computable(b.through("reads an anchor")));
        }
        return Err(s);
    }
    let read = r.bindings.first().and_then(|(source, _)| crate::channels::channel_for_bit(*source)).unwrap_or(channel);
    let mut status = info.status.clone();
    if r.levels.as_ref().is_some_and(|l| !levels(Some(l)).0.eq(&Levels::default())) {
        status.and(&Status::approximated("levels on what a reference reads are left out"));
    }
    if bit_of(read) == 0 {
        status.and(&Status::approximated("reference reads a channel TextureCraft has no counterpart for"));
    }
    Ok(Some((Source::Anchor { anchor: info.id, channel: read }, status)))
}

/// The file's levels as TextureCraft's: the file's gamma is the exponent itself (`t^gamma`), TextureCraft's its
/// reciprocal (measured on a real project). TextureCraft's levels act the same on every component: levels that differ
/// per component take the red one and are approximated.
pub fn levels(l: Option<&ProjectLevels>) -> (Levels, Status) {
    let Some(l) = l else { return (Levels::default(), Status::Exact) };
    let reciprocal = |g: f32| if g.is_finite() && g > 1e-6 { 1.0 / g } else { 1.0 };
    let finite = |v: f32, d: f32| if v.is_finite() { v } else { d };
    let ours = Levels {
        in_low: finite(l.input_minimum[0], 0.0),
        in_high: finite(l.input_maximum[0], 1.0),
        gamma: reciprocal(l.gamma[0]),
        out_low: finite(l.output_minimum[0], 0.0),
        out_high: finite(l.output_maximum[0], 1.0),
    };
    let same = |a: [f32; 3]| a[1] == a[0] && a[2] == a[0];
    let status = match [l.input_minimum, l.input_maximum, l.gamma, l.output_minimum, l.output_maximum].into_iter().all(same) {
        true => Status::Exact,
        false => Status::approximated("levels that differ per colour component (the red one taken)"),
    };
    (ours, status)
}
