//! [`ContentMapper`]: a layer's content (its effect stack, bottom → top) as TextureCraft's base content plus content
//! effects, effect for effect, with the status of every channel the content writes.
//!
//! - The base: a fill layer's first fill becomes a fill layer when it is switched on and writes every channel it
//!   touches from a value, a bitmap or an anchor at full strength (normal or replace, 100%). Otherwise the base is an
//!   empty paint layer and the first fill stays an effect.
//! - A paint layer's brush work (its first effect group, paint effects only) becomes the layer's own pixels, replayed
//!   with TextureCraft's brush ([`crate::replay`]) when the import has a replayer: approximated, not locked. Paint
//!   effects further up become content paint effects the same way. A paint effect that cannot be replayed (or any
//!   paint effect when there is no replayer) is a placeholder and locks the layer: `brush:<what>` /
//!   `brush-alpha:<graph>` for why it cannot be replayed, `strokes:<tool>` when replay is off; one that cannot be
//!   replayed keeps the rest of its layer's brush work a placeholder too (they paint one image in order).
//! - Every other effect becomes one of TextureCraft's: a fill becomes one material fill effect per distinct blend
//!   mode and opacity among its channels (TextureCraft's effects have one blend each) plus one placeholder per compiled
//!   graph it uses ([`crate::graph_uses`]); levels and anchors keep their kind; filters are placeholders (or a close
//!   built-in filter when asked for); colour selection in content and unknown classes are placeholders.
//! - A polygon fill becomes a content paint effect whose pixels are its picks' colour in base colour, at full coverage
//!   on the picked triangles (laid by [`crate::polygon_fill::FillJob`]); measured on one file format: the other
//!   channels show what is below.
//!
//! TextureCraft's levels and filters act on every channel of the layer, the file's only on the channels they list: an
//! effect listing fewer channels than the layer writes is approximated.

use std::collections::BTreeMap;

use texturecraft_core::{BlendMode, Channel, ChannelKind, Effect, EffectKind, Fill, LayerId, LayerKind, Levels, Projection, Source};
use texturecraft_import_reader::blockers::{Blocker, BlockerKey, BlockerKind};
use texturecraft_import_reader::model::{
    EffectGroup, EffectHeader, FillEffect, Levels as ProjectLevels, LevelsEffect, PaintEffect, PaintTool, PolygonFillEffect, ProjectEffect, ProjectLayer,
    ProjectSource, ReferenceSource,
};

use super::{Anchors, Ids, SetInput, effect, projection};
use crate::blend::{Holder, ResolvedBlend};
use crate::channels::bit_of;
use crate::graph_uses::{self, GraphUse};
use crate::polygon_fill::{self, FillJob};
use crate::replay::brush::StepTarget;
use crate::replay::{PaintMode, REPLAYED, ReplayJob, ReplayStep, ReplayTarget};
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
    /// Brush work to replay into the layer's pixels or its content paint effects.
    pub jobs: Vec<ReplayJob>,
    /// Polygon fills to lay into content paint effects.
    pub fills: Vec<FillJob>,
}

pub struct ContentMapper<'a, 'i> {
    input: &'a SetInput<'a>,
    anchors: &'a Anchors,
    ids: &'i mut Ids,
    effects: Vec<Effect>,
    written: BTreeMap<Channel, Status>,
    graphs: Vec<String>,
    anchor_status: Vec<(u64, Status)>,
    /// The last strokes placeholder: its index, strokes and paint effects so far.
    strokes_run: Option<(usize, usize, usize)>,
    jobs: Vec<ReplayJob>,
    fills: Vec<FillJob>,
    /// Mapping a layer's brush work that could not be replayed as a whole.
    in_brush_work: bool,
    /// The layer's path, for the replay jobs' labels.
    path: String,
}

/// The channels and sources of one fill, grouped the way TextureCraft lays them on.
struct FillParts {
    /// Sources by blend: one group per distinct mode and opacity.
    groups: Vec<(ResolvedBlend, BTreeMap<Channel, Source>)>,
    /// Compiled graphs that cannot run, with the channels they write.
    graphs: Vec<(GraphUse, Vec<Channel>, ResolvedBlend)>,
    /// Compiled graphs running as sources of the groups.
    graph_names: Vec<String>,
    status: BTreeMap<Channel, Status>,
    projection: Projection,
}

impl<'a, 'i> ContentMapper<'a, 'i> {
    pub fn new(input: &'a SetInput<'a>, anchors: &'a Anchors, ids: &'i mut Ids) -> ContentMapper<'a, 'i> {
        ContentMapper {
            input,
            anchors,
            ids,
            effects: Vec::new(),
            written: BTreeMap::new(),
            graphs: Vec::new(),
            anchor_status: Vec::new(),
            strokes_run: None,
            jobs: Vec::new(),
            fills: Vec::new(),
            in_brush_work: false,
            path: String::new(),
        }
    }

    /// The base and effects of a layer that is not a folder; `id` is the layer's, `path` names it.
    pub fn map(mut self, l: &ProjectLayer, id: LayerId, path: &str) -> Mapped {
        self.path = path.to_string();
        let mut kind = LayerKind::Paint;
        let mut rest: &[ProjectEffect] = &l.content;
        let mut base = None;
        match (&l.kind, l.content.split_first()) {
            (texturecraft_import_reader::model::LayerKind::Fill, Some((ProjectEffect::Fill(first), after))) if first.header.enabled => {
                let parts = self.fill_parts(first);
                if let Some(fill) = as_base(&parts) {
                    kind = LayerKind::Fill(fill);
                    self.note_all(&parts.status);
                    self.name_graphs(parts.graph_names);
                    rest = after;
                    base = Some(first);
                }
            }
            (texturecraft_import_reader::model::LayerKind::Paint, Some((ProjectEffect::Group(g), after))) if g.header.enabled => {
                // the brush work: what the paint layer holds itself
                if !self.replay_brush_work(g, id) {
                    self.in_brush_work = true;
                    for e in &g.effects {
                        self.effect(e, true);
                    }
                    self.in_brush_work = false;
                }
                rest = after;
            }
            _ => {}
        }
        for e in rest {
            self.effect(e, true);
        }
        // A fill layer whose effects write channels its fill does not hold (a filter on every channel over a fill of base
        // colour only): the original's fill covers nothing on those channels, so a filter there works on nothing and
        // leaves the stack below as it is. TextureCraft's fill layer lays a channel's default where it has no source;
        // the fill becomes the first content effect of a paint layer instead, which covers only the channels it holds.
        if let (LayerKind::Fill(fill), Some(first)) = (&kind, base)
            && self.written.keys().any(|c| !fill.sources.contains_key(c))
        {
            let after = std::mem::take(&mut self.effects);
            self.fill(first, true);
            self.effects.extend(after);
            kind = LayerKind::Paint;
        }
        Mapped {
            kind,
            effects: self.effects,
            written: self.written,
            graphs: self.graphs,
            anchor_status: self.anchor_status,
            jobs: self.jobs,
            fills: self.fills,
        }
    }

    /// A paint layer's brush work replayed into its own pixels (`id`): every switched-on paint effect with strokes, in
    /// order. False when there is no replayer, the group holds other effects, or one of them cannot be replayed (the
    /// caller then lays them as placeholders, each with its reason).
    fn replay_brush_work(&mut self, g: &EffectGroup, id: LayerId) -> bool {
        if !self.input.replay {
            return false;
        }
        let mut paints = Vec::new();
        for e in &g.effects {
            match e {
                ProjectEffect::Paint(p) if p.header.enabled && p.stroke_count() > 0 => paints.push(p),
                ProjectEffect::Paint(_) => {}
                _ => return false,
            }
        }
        if paints.is_empty() {
            return false;
        }
        let Ok(steps) = paints.iter().map(|p| self.step(p)).collect::<Result<Vec<_>, _>>() else { return false };
        self.add_job(ReplayTarget::Content(id), "brush work", steps);
        true
    }

    /// `p` as a content step, with its own blending folded into the channels' opacity (approximated when it is not
    /// normal).
    fn step(&self, p: &PaintEffect) -> Result<(ReplayStep, Status), Blocker> {
        let sources = super::LayerSources { input: self.input, anchors: self.anchors };
        let mut step = crate::replay::step(p, StepTarget::Content, self.input.channels, self.input.images, self.input.graphs, Some(&sources))?;
        let mut status = Status::Exact;
        let channels = step.values.iter_mut().map(|(c, _, o)| (*c, o)).chain(step.sources.iter_mut().map(|(c, _, o)| (*c, o)));
        for (c, opacity) in channels {
            let blend = ResolvedBlend::of(&p.header.blending, self.input.channels.project_channel(c), Holder::Layer);
            *opacity *= blend.opacity.clamp(0.0, 1.0);
            if !matches!(blend.mode(), BlendMode::Normal | BlendMode::Replace) {
                status.and(&Status::approximated(format!("a paint effect blending {:?} laid as normal", blend.mode())));
            }
        }
        Ok((step, status))
    }

    /// Adds a replay job painting `steps` into `target` and notes the channels it paints as replayed.
    fn add_job(&mut self, target: ReplayTarget, what: &str, steps: Vec<(ReplayStep, Status)>) {
        let mut status = Status::Exact;
        let mut channels = Vec::new();
        for (s, st) in &steps {
            status.and(st);
            for c in s.values.iter().map(|v| v.0).chain(s.sources.iter().map(|v| v.0)) {
                if !channels.contains(&c) {
                    channels.push(c);
                }
            }
        }
        let steps: Vec<ReplayStep> = steps.into_iter().map(|(s, _)| s).collect();
        let job = ReplayJob { target, label: format!("{} / {what}", self.path), steps };
        status.and(&replay_status(&job));
        for c in channels {
            self.note(c, &status);
        }
        self.jobs.push(job);
    }

    /// Replays one paint effect above the brush work as a content paint effect; or why it is not.
    fn replay_effect(&mut self, p: &PaintEffect, on: bool) -> Result<(), Blocker> {
        let tool = p.tool.name();
        if !self.input.replay {
            return Err(Blocker::new(BlockerKey::new(BlockerKind::Strokes, tool), format!("{tool} strokes")));
        }
        // a smudge or an eraser above other content effects works on what lies below its own effect, which a replay into
        // the effect does not reach: kept as an empty paint effect, it changes nothing (approximated, as the fork did)
        if !self.in_brush_work && p.tool == PaintTool::Smudge {
            self.inert_paint(p, on, "a smudge on its own (it moves what lies below its effect) is not replayed: it changes nothing");
            return Ok(());
        }
        let step = crate::replay::brush::alone(p).and_then(|()| self.step(p));
        if self.in_brush_work {
            return Err(match step {
                Err(b) => b,
                Ok(_) => {
                    Blocker::new(BlockerKey::new(BlockerKind::Strokes, tool), format!("{tool} strokes not replayed: other brush work of the layer cannot be"))
                }
            });
        }
        let (step, status) = step?;
        if step.mode == PaintMode::Erase {
            self.inert_paint(p, on, "an eraser above other content effects (it erases what is below it) is not replayed: it changes nothing");
            return Ok(());
        }
        let id = self.ids.alloc();
        let mut e = effect(id, &p.header.label, EffectKind::Paint, on);
        if let Some(c) = step.values.first().map(|v| v.0).or_else(|| step.sources.first().map(|v| v.0)) {
            e.blend = ResolvedBlend::of(&p.header.blending, self.input.channels.project_channel(c), Holder::Layer).mode();
        }
        self.effects.push(e);
        if on {
            self.add_job(ReplayTarget::Content(id), "paint effect", vec![(step, status)]);
        } else {
            let job = ReplayJob { target: ReplayTarget::Content(id), label: format!("{} / paint effect", self.path), steps: vec![step] };
            self.jobs.push(job);
        }
        Ok(())
    }

    /// `p` as an empty paint effect: nothing replayed into it, so it changes nothing; its channels approximated by `why`.
    fn inert_paint(&mut self, p: &PaintEffect, on: bool, why: &str) {
        let id = self.ids.alloc();
        self.effects.push(effect(id, &p.header.label, EffectKind::Paint, on));
        if on {
            let status = Status::approximated(why);
            for c in self.channels_of(Some(&p.header)) {
                self.note(c, &status);
            }
        }
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

    fn name_graphs(&mut self, names: Vec<String>) {
        for n in names {
            if !self.graphs.contains(&n) {
                self.graphs.push(n);
            }
        }
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
        if matches!(e.kind, EffectKind::Filter(_)) {
            e.channels = channels.to_vec();
        }
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
                let blocker = match self.replay_effect(p, on) {
                    Ok(()) => return,
                    Err(b) => b,
                };
                let merged = match self.strokes_run.as_mut() {
                    Some((at, strokes, effects)) if *at + 1 == self.effects.len() => {
                        *strokes += p.stroke_count();
                        *effects += 1;
                        let label = format!("brush strokes ({strokes} strokes in {effects} paint effects)");
                        if let Some(e) = self.effects.get_mut(*at) {
                            e.name = label.clone();
                            e.visible |= on;
                            if let EffectKind::Placeholder { label: l, .. } = &mut e.kind {
                                *l = label;
                            }
                        }
                        true
                    }
                    _ => false,
                };
                if !merged {
                    let label = format!("{} strokes ({})", p.tool.name(), p.stroke_count());
                    let reason = match blocker.key.kind {
                        BlockerKind::Strokes if !self.input.replay => "brush strokes are kept in the original result; stroke replay is off".to_string(),
                        _ => format!("brush strokes are kept in the original result: {}", blocker.text),
                    };
                    self.placeholder(&label, &reason, on);
                    self.strokes_run = Some((self.effects.len() - 1, p.stroke_count(), 1));
                }
                if on {
                    let why = Status::not_computable(blocker);
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
                let g = graph_uses::filter(self.input, self.anchors, f, Some(&channels));
                self.graph_use(g, &channels, blend.as_ref(), on);
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
            ProjectEffect::PolygonFill(p) => self.polygon_fill(p, on),
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
    /// A polygon fill: a paint effect holding its picks' colour in base colour at full coverage on the picked
    /// triangles, blending with the fill's base colour blending (measured on one file format: it writes base colour
    /// only, at full strength on its picks; the other channels show what is below). A fill whose picks cover nothing
    /// writes nothing; picks that cannot be resolved leave a placeholder.
    fn polygon_fill(&mut self, p: &PolygonFillEffect, on: bool) {
        let writes = self.writes(&p.header, Holder::Layer).contains(&Channel::BaseColor);
        let runs = polygon_fill::runs(self.input.geometry, p);
        if !runs.status.is_computable() {
            self.placeholder("polygon fill", "the polygon fill's picks could not be resolved on the mesh", on);
            if on && writes {
                self.note(Channel::BaseColor, &runs.status);
            }
            return;
        }
        let blend = ResolvedBlend::of(&p.header.blending, self.input.channels.project_channel(Channel::BaseColor), Holder::Layer);
        let mut e = effect(self.ids.alloc(), &p.header.label, EffectKind::Paint, on);
        e.blend = blend.mode();
        e.opacity = blend.opacity;
        let id = e.id;
        self.effects.push(e);
        if !writes {
            return;
        }
        if on {
            let mut status = runs.status.clone();
            status.and(&blend.status());
            status.and(&Status::approximated("polygon fill in layer content: base colour from its picks' colour (measured on one file format)"));
            self.note(Channel::BaseColor, &status);
        }
        if !runs.picks_nothing() {
            self.fills.push(FillJob {
                effect: id,
                channel: Some(Channel::BaseColor),
                runs: runs.runs,
                label: format!("{} / polygon fill", self.path),
                over_strokes: false,
            });
        }
    }

    fn subset_status(&self, channels: &[Channel]) -> Status {
        match self.written.keys().any(|c| !channels.contains(c)) {
            true => Status::approximated("an effect on some channels applies to every channel of the layer"),
            false => Status::Exact,
        }
    }

    fn levels(&mut self, l: &LevelsEffect, on: bool) {
        let channels = self.writes(&l.header, Holder::Filter);
        let (levels, mut status) = levels(l.levels.as_ref());
        // on a signed channel (height) the file's levels count in its −1..1 range; TextureCraft's act on the channel
        // mapped to 0..1 (measured on Signs: the hammered relief keeps the original's range only this way)
        let signed = !channels.is_empty() && channels.iter().all(|c| c.kind() == ChannelKind::Signed);
        let levels = if signed { signed_levels(levels) } else { levels };
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
        // the reference app's levels act on the channels they list only; the others are left as they are. Levels listing
        // none of the set's channels (glossiness only) change nothing: kept, switched off
        e.channels.clone_from(&channels);
        if channels.is_empty() {
            e.visible = false;
        }
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
        self.name_graphs(parts.graph_names);
        if on {
            self.note_all(&parts.status);
        }
    }

    /// What one fill writes per channel.
    fn fill_parts(&mut self, fill: &FillEffect) -> FillParts {
        let ch = self.input.channels;
        let mapped = projection::map(fill, self.input.geometry.map(|geometry| geometry.mesh));
        let mut parts =
            FillParts { groups: Vec::new(), graphs: Vec::new(), graph_names: Vec::new(), status: BTreeMap::new(), projection: mapped.projection.clone() };
        // per fill source: its channels, each with its own blending
        let mut graph_channels: Vec<(usize, Vec<(Channel, ResolvedBlend)>)> = Vec::new();
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
                    // laid on through the projection's coverage (clip, falloffs): as exact as the projection
                    if projection::shapes_coverage(fill) {
                        status.and(&mapped.status);
                    }
                    let v = crate::channels::uniform_value(c, u.color);
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
                        Some((_, channels)) => channels.push((c, blend)),
                        None => graph_channels.push((index, vec![(c, blend)])),
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
        for (index, blends) in graph_channels {
            let Some(ProjectSource::Procedural(p)) = fill.sources.get(index) else { continue };
            let channels: Vec<Channel> = blends.iter().map(|(c, _)| *c).collect();
            match graph_uses::fill_sources(self.input, self.anchors, p, &channels) {
                Ok(fg) => {
                    // a channel the graph has no output for is not written; each is laid on with its own blending
                    for (c, mut source) in fg.sources {
                        let Some((_, blend)) = blends.iter().find(|(b, _)| *b == c) else { continue };
                        if let Source::Graph(g) = &mut source {
                            g.unfiltered = projection::bilinear(fill);
                            g.parameter_group = Some([self.ids.next(), index as u64]);
                        }
                        let mut status = blend.status();
                        status.and(&mapped.status);
                        status.and(&fg.status);
                        parts.status.entry(c).or_default().and(&status);
                        match parts.groups.iter_mut().find(|(b, _)| b.mode() == blend.mode() && b.opacity == blend.opacity) {
                            Some((_, sources)) => {
                                sources.insert(c, source);
                            }
                            None => parts.groups.push((*blend, BTreeMap::from([(c, source)]))),
                        }
                    }
                    parts.graph_names.push(fg.graph);
                }
                Err(g) => {
                    for c in &channels {
                        parts.status.entry(*c).or_default().and(&g.status);
                    }
                    let Some((_, blend)) = blends.first().cloned() else { continue };
                    parts.graphs.push((*g, channels, blend));
                }
            }
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
    Ok(Some((Source::Anchor { anchor: info.id, channel: read, ramp: Vec::new() }, status)))
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

/// Levels counted in a signed channel's −1..1 range as TextureCraft applies them, on the channel mapped to 0..1
/// (`v / 2 + 0.5`); the gamma stays.
pub fn signed_levels(l: Levels) -> Levels {
    let unit = |v: f32| v * 0.5 + 0.5;
    Levels { in_low: unit(l.in_low), in_high: unit(l.in_high), gamma: l.gamma, out_low: unit(l.out_low), out_high: unit(l.out_high) }
}

/// What a replayed job counts as: approximated, with its notes.
pub fn replay_status(job: &ReplayJob) -> Status {
    let mut text = format!("{REPLAYED} ({} strokes)", job.strokes());
    let notes = job.notes();
    if !notes.is_empty() {
        text.push_str(&format!("; {}", notes.join("; ")));
    }
    Status::approximated(text)
}
