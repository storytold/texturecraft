//! [`MaskBuilder`]: a layer's mask (`maskActions` with `maskEnabled` and `maskInitial`) as TextureCraft's [`Mask`],
//! effect for effect, and how exactly it reproduces the original.
//!
//! Uniform fills become grey fills, bitmap fills image fills (a mesh map slot left empty reads black), reference fills
//! anchor fills; levels and colour selection keep their kind. Painted strokes become paint effects replayed with
//! TextureCraft's brush when the import has a replayer (approximated; [`crate::replay`]). Polygon fills become paint
//! effects whose pixels are their picks resolved on the set's mesh ([`crate::polygon_fill`]). Compiled-graph generators and
//! filters, strokes that cannot be replayed, polygon fills that cannot be resolved, anchors inside the mask and unknown
//! classes are placeholders, and the first of them makes the mask not computable. An effect group of brush work with its own blending (or
//! holding an eraser) becomes one paint effect with the group's blending, its strokes replayed together; other groups are laid
//! flat (approximated unless they pass through or blend normally at full strength). A group of nothing but polygon fills with its
//! own blending is one paint effect of their picks.

use texturecraft_core::{BakedMap, BlendMode, Effect, EffectKind, Mask, MaskBackground, Source};
use texturecraft_import_reader::blockers::{Blocker, BlockerKey, BlockerKind};
use texturecraft_import_reader::model::{
    BitmapSource, ColorSelectionEffect, EffectGroup, EffectHeader, FillEffect, MaskInitial, MeshMapRef, PaintTool, PolygonFillEffect, ProjectEffect,
    ProjectMask, ProjectSource,
};

use super::content::{is_dangling, levels, reference, replay_status};
use super::{Anchors, Ids, SetInput, effect, projection};
use crate::blend::ResolvedBlend;
use crate::graph_uses::{self, GraphUse};
use crate::polygon_fill::{self, FillJob, FillRun};
use crate::replay::brush::StepTarget;
use crate::replay::{PaintMode, ReplayJob, ReplayTarget};
use crate::resources::{MeshMapAlias, mesh_map_alias, split_resource_url};
use crate::status::Status;

/// The mask for a layer (`None` = no mask) and how exact it is.
pub struct MaskOutcome {
    pub mask: Option<Mask>,
    pub status: Status,
    pub graphs: Vec<String>,
    /// Painted strokes to replay into the mask's paint effects.
    pub jobs: Vec<ReplayJob>,
    /// Polygon fills to lay into the mask's paint effects.
    pub fills: Vec<FillJob>,
    /// What each anchor point in the mask holds: the mask below it, as computable as that is.
    pub anchor_status: Vec<(u64, Status)>,
}

pub struct MaskBuilder<'a, 'i> {
    input: &'a SetInput<'a>,
    anchors: &'a Anchors,
    ids: &'i mut Ids,
    effects: Vec<Effect>,
    status: Status,
    graphs: Vec<String>,
    /// False for a switched-off mask: its effects are kept but do not count.
    counts: bool,
    /// The last strokes placeholder: its index, strokes and paint effects so far.
    strokes_run: Option<(usize, usize, usize)>,
    jobs: Vec<ReplayJob>,
    fills: Vec<FillJob>,
    /// The layer's path, for the replay jobs' labels.
    path: String,
    anchor_status: Vec<(u64, Status)>,
}

impl<'a, 'i> MaskBuilder<'a, 'i> {
    /// The graphs brushes painting a compiled graph paint in this set.
    fn sources(&self) -> super::LayerSources<'a> {
        super::LayerSources { input: self.input, anchors: self.anchors }
    }

    pub fn new(input: &'a SetInput<'a>, anchors: &'a Anchors, ids: &'i mut Ids) -> MaskBuilder<'a, 'i> {
        MaskBuilder {
            input,
            anchors,
            ids,
            effects: Vec::new(),
            status: Status::Exact,
            graphs: Vec::new(),
            counts: true,
            strokes_run: None,
            jobs: Vec::new(),
            fills: Vec::new(),
            path: String::new(),
            anchor_status: Vec::new(),
        }
    }

    /// The layer's path, for the replay jobs' labels.
    pub fn at(mut self, path: &str) -> Self {
        self.path = path.to_string();
        self
    }

    pub fn build(mut self, mask: Option<&ProjectMask>) -> MaskOutcome {
        let Some(m) = mask else {
            return MaskOutcome { mask: None, status: Status::Exact, graphs: Vec::new(), jobs: Vec::new(), fills: Vec::new(), anchor_status: Vec::new() };
        };
        let background = match m.initial {
            MaskInitial::Black => MaskBackground::Black,
            MaskInitial::White => MaskBackground::White,
            // An initial value we cannot read: the layer gets no mask (a switched-off one does not matter).
            MaskInitial::Other(n) => {
                let status = match m.enabled {
                    true => Status::not_computable(Blocker::feature("mask-initial-value", format!("mask initial value {n}"))),
                    false => Status::Exact,
                };
                return MaskOutcome { mask: None, status, graphs: Vec::new(), jobs: Vec::new(), fills: Vec::new(), anchor_status: Vec::new() };
            }
        };
        self.counts = m.enabled;
        for e in &m.effects {
            self.effect(e, true);
        }
        let mask = Mask { enabled: m.enabled, background, effects: self.effects };
        MaskOutcome { mask: Some(mask), status: self.status, graphs: self.graphs, jobs: self.jobs, fills: self.fills, anchor_status: self.anchor_status }
    }

    /// Adds the status of a part that counts.
    fn note(&mut self, on: bool, status: &Status) {
        if on && self.counts {
            self.status.and(status);
        }
    }

    fn push(&mut self, header: Option<&EffectHeader>, kind: EffectKind, on: bool) {
        let blend = header.map_or_else(|| ResolvedBlend::of_mask(&[]), |h| ResolvedBlend::of_mask(&h.blending));
        let mut e = effect(self.ids.alloc(), header.map_or("", |h| h.label.as_str()), kind, on);
        e.blend = blend.mode();
        e.opacity = blend.opacity;
        self.effects.push(e);
    }

    fn graph_use(&mut self, header: &EffectHeader, g: GraphUse, on: bool) {
        self.note(on, &g.status);
        if !self.graphs.contains(&g.graph) {
            self.graphs.push(g.graph.clone());
        }
        self.push(Some(header), g.effect, on);
    }

    fn placeholder(&mut self, header: Option<&EffectHeader>, label: &str, reason: &str, on: bool, why: Status) {
        self.note(on, &why);
        self.push(header, EffectKind::Placeholder { label: label.into(), reason: reason.into() }, on);
    }

    /// One mask effect (`on`: it and everything holding it is switched on).
    fn effect(&mut self, e: &ProjectEffect, on: bool) {
        // An effect of a class the reader does not know has no switch to read: it counts as on.
        let on = on && (e.is_enabled() || matches!(e, ProjectEffect::Other(_)));
        let header = e.header();
        if let Some(h) = header {
            let blend = ResolvedBlend::of_mask(&h.blending);
            self.note(on && blend.contributes(), &blend.status());
        }
        match e {
            ProjectEffect::Group(g) => {
                let blend = ResolvedBlend::of_mask(&g.header.blending);
                let flat =
                    blend.mode() == BlendMode::Passthrough || (blend.mode() == BlendMode::Normal && blend.opacity >= 1.0) || g.header.blending.is_empty();
                // an eraser lowers only the group's own coverage, so it needs the group's layer even when it blends normally
                // a smudge moves the strokes before it in the group, so it needs the group's layer too
                let isolating =
                    g.effects.iter().any(|e| e.is_enabled() && matches!(e, ProjectEffect::Paint(p) if matches!(p.tool, PaintTool::Eraser | PaintTool::Smudge)));
                if (!flat || isolating) && self.group_layer(g, on) {
                    return;
                }
                if !flat {
                    self.note(on, &Status::approximated("mask effect group with its own blending laid flat"));
                }
                for inner in &g.effects {
                    self.effect(inner, on);
                }
            }
            ProjectEffect::Paint(p) => {
                if p.stroke_count() == 0 {
                    return;
                }
                // a smudge on its own moves what lies below its effect, which a replay into the effect does not reach: kept as
                // an empty paint effect, it changes nothing (approximated, as the fork did)
                if self.input.replay && p.tool == PaintTool::Smudge {
                    self.push(header, EffectKind::Paint, on);
                    self.note(on, &Status::approximated("a smudge on its own (it moves what lies below its effect) is not replayed: it changes nothing"));
                    return;
                }
                let blocker = match self.input.replay {
                    false => Blocker::new(BlockerKey::new(BlockerKind::Strokes, "painted-mask"), "painted mask (strokes)"),
                    true => match crate::replay::brush::alone(p).and_then(|()| {
                        crate::replay::step(p, StepTarget::Mask, self.input.channels, self.input.images, self.input.graphs, Some(&self.sources()))
                    }) {
                        Ok(step) => {
                            self.push(header, EffectKind::Paint, on);
                            let id = self.effects.last().map_or(texturecraft_core::LayerId(0), |e| e.id);
                            let job = ReplayJob { target: ReplayTarget::Mask(id), label: format!("{} / painted mask", self.path), steps: vec![step] };
                            self.note(on, &replay_status(&job));
                            self.jobs.push(job);
                            return;
                        }
                        Err(b) => b,
                    },
                };
                let reason = match blocker.key.kind {
                    BlockerKind::Strokes => "brush strokes are kept in the original result; stroke replay is off".to_string(),
                    _ => format!("brush strokes are kept in the original result: {}", blocker.text),
                };
                let why = Status::not_computable(blocker);
                // consecutive paint effects share one placeholder
                if let Some((at, strokes, effects)) = self.strokes_run.as_mut()
                    && *at + 1 == self.effects.len()
                    && let Some(EffectKind::Placeholder { label, .. }) = self.effects.get_mut(*at).map(|e| &mut e.kind)
                {
                    *strokes += p.stroke_count();
                    *effects += 1;
                    *label = format!("painted mask ({strokes} strokes in {effects} paint effects)");
                    let label = label.clone();
                    if let Some(e) = self.effects.get_mut(*at) {
                        e.name = label;
                        e.visible |= on;
                    }
                    self.note(on, &why);
                    return;
                }
                let label = format!("painted mask ({} strokes)", p.stroke_count());
                self.placeholder(header, &label, &reason, on, why);
                self.strokes_run = Some((self.effects.len() - 1, p.stroke_count(), 1));
            }
            // An anchor point records the mask below it (grey on every channel) for the fills reading it
            ProjectEffect::Anchor(h) => match h.uid.and_then(|u| self.anchors.get(&u).map(|a| (u, a.id))) {
                Some((uid, id)) => {
                    self.effects.push(effect(id, &h.label, EffectKind::Anchor, on));
                    self.anchor_status.push((uid, self.status.clone()));
                }
                None => self.placeholder(header, "anchor point", "an anchor point nothing can read", false, Status::Exact),
            },
            ProjectEffect::Fill(fill) => self.fill(fill, on),
            ProjectEffect::Filter(f) if f.procedural.is_none() => {
                self.placeholder(header, "filter (none selected)", "the original's filter has no graph selected", false, Status::Exact);
            }
            ProjectEffect::Filter(f) => {
                if self.input.options.native_filters
                    && let Some((filter, note)) = graph_uses::native_filter(f)
                {
                    self.note(on, &Status::approximated(note));
                    self.push(header, EffectKind::Filter(filter), on);
                    return;
                }
                self.graph_use(&f.header, graph_uses::filter(self.input, self.anchors, f, None), on);
            }
            ProjectEffect::Levels(l) => {
                let (ours, status) = levels(l.levels.as_ref());
                self.note(on, &status);
                self.push(header, EffectKind::Levels(ours), on);
            }
            ProjectEffect::ColorSelection(c) => self.color_selection(c, on),
            ProjectEffect::PolygonFill(p) => self.polygon_fill(p, on),
            ProjectEffect::Other(raw) => {
                let why = Status::not_computable(Blocker::class(&raw.class, format!("mask effect {}", raw.class)));
                self.placeholder(None, &format!("unknown effect {}", raw.class), "an effect class this version does not read", on, why);
            }
        }
    }

    /// A group with its own blending (or holding an eraser or a smudge) whose actions are all brush work: its strokes
    /// painted on their own, over nothing, into one paint effect (a brush lays its value over what the group holds, an
    /// eraser lowers the group's coverage, a smudge moves it) that blends with the mask below with the group's mode and opacity, as the original
    /// composites such a group. False (nothing pushed) when the group holds anything else or a stroke that cannot be
    /// replayed: the caller lays it flat.
    fn group_layer(&mut self, g: &EffectGroup, on: bool) -> bool {
        if self.polygon_fill_group(g, on) {
            return true;
        }
        if !self.input.replay {
            return false;
        }
        // strokes, then (above them all) polygon fills laid over the replayed strokes. Strokes above a polygon fill are
        // laid under it too (approximated): laying the group flat instead would lose its blending, which matters more
        // (a subtracting group laid flat adds what it should take away)
        let mut paints = Vec::new();
        let mut polygons = Vec::new();
        let mut strokes_above_fills = false;
        for e in g.effects.iter().filter(|e| e.is_enabled()) {
            match e {
                ProjectEffect::Paint(p) => {
                    strokes_above_fills |= !polygons.is_empty() && p.stroke_count() > 0;
                    paints.push(p);
                }
                ProjectEffect::PolygonFill(p) => polygons.push(p),
                _ => return false,
            }
        }
        let mut steps = Vec::new();
        let mut status = Status::Exact;
        if strokes_above_fills {
            status.and(&Status::approximated("mask group: strokes above a polygon fill laid under it (the group's polygon fills lie over all its strokes)"));
        }
        for p in paints.into_iter().filter(|p| p.stroke_count() > 0) {
            let Ok(mut step) = crate::replay::step(p, StepTarget::Mask, self.input.channels, self.input.images, self.input.graphs, Some(&self.sources()))
            else {
                return false;
            };
            if p.tool == PaintTool::Eraser {
                step.mode = PaintMode::Erase;
            }
            let own = ResolvedBlend::of_mask(&p.header.blending);
            let plain = p.header.blending.is_empty() || (matches!(own.mode(), BlendMode::Normal | BlendMode::Passthrough) && own.opacity >= 1.0);
            if !plain {
                status.and(&Status::approximated(format!(
                    "paint effect with its own blend ({:?} {}) inside a mask group laid over normally",
                    own.mode(),
                    own.opacity
                )));
            }
            steps.push(step);
        }
        let mut picks = Vec::new();
        for p in polygons {
            let runs = polygon_fill::runs(self.input.geometry, p);
            if !runs.status.is_computable() {
                return false;
            }
            status.and(&runs.status);
            let own = ResolvedBlend::of_mask(&p.header.blending);
            let plain = p.header.blending.is_empty() || (matches!(own.mode(), BlendMode::Normal | BlendMode::Passthrough) && own.opacity >= 1.0);
            if !plain {
                status.and(&Status::approximated(format!(
                    "polygon fill with its own blend ({:?} {}) inside a mask group laid over normally",
                    own.mode(),
                    own.opacity
                )));
            }
            picks.extend(runs.runs);
        }
        if steps.is_empty() {
            // a group without strokes changes nothing (one of polygon fills alone is laid by `polygon_fill_group`)
            return true;
        }
        self.push(Some(&g.header), EffectKind::Paint, on);
        let id = self.effects.last().map_or(texturecraft_core::LayerId(0), |e| e.id);
        let job = ReplayJob { target: ReplayTarget::Mask(id), label: format!("{} / painted mask group", self.path), steps };
        status.and(&replay_status(&job));
        self.note(on, &status);
        self.jobs.push(job);
        if picks.iter().any(|r| !r.triangles.is_empty()) {
            let label = format!("{} / polygon fills of a painted mask group", self.path);
            self.fills.push(FillJob { effect: id, channel: None, runs: picks, label, over_strokes: true });
        }
        true
    }

    /// A polygon fill: one paint effect whose pixels are its picks, later ones over earlier ones (as TextureCraft's polygon
    /// fill tool leaves them); a placeholder when the picks cannot be resolved on the mesh.
    fn polygon_fill(&mut self, p: &PolygonFillEffect, on: bool) {
        let runs = polygon_fill::runs(self.input.geometry, p);
        if !runs.status.is_computable() {
            self.placeholder(Some(&p.header), "polygon fill", "the polygon fill's picks could not be resolved on the mesh", on, runs.status);
            return;
        }
        let mut status = runs.status.clone();
        let blend = ResolvedBlend::of_mask(&p.header.blending);
        let normal = matches!(blend.mode(), BlendMode::Normal | BlendMode::Passthrough) && blend.opacity >= 1.0;
        if !normal && distinct_values(&runs.runs) > 1 {
            status.and(&Status::approximated("polygon fill: picks with several values blended as one effect (not measured)"));
        }
        self.note(on && blend.contributes(), &status);
        self.push(Some(&p.header), EffectKind::Paint, on);
        if !runs.picks_nothing() {
            self.fill_job(runs.runs, "polygon fill");
        }
    }

    /// A group with its own blending holding nothing but polygon fills (switched-off entries aside): one paint effect of
    /// all their picks laid over each other, blending with the mask below with the group's blend. False (nothing pushed)
    /// when the group holds anything else or a pick that cannot be resolved.
    fn polygon_fill_group(&mut self, g: &EffectGroup, on: bool) -> bool {
        let mut fills = Vec::new();
        for e in g.effects.iter().filter(|e| e.is_enabled()) {
            match e {
                ProjectEffect::PolygonFill(p) => fills.push(p),
                _ => return false,
            }
        }
        if fills.is_empty() {
            return false;
        }
        let mut status = Status::Exact;
        let mut all = Vec::new();
        for p in fills {
            let runs = polygon_fill::runs(self.input.geometry, p);
            if !runs.status.is_computable() {
                return false;
            }
            status.and(&runs.status);
            let own = ResolvedBlend::of_mask(&p.header.blending);
            let plain = p.header.blending.is_empty() || (matches!(own.mode(), BlendMode::Normal | BlendMode::Passthrough) && own.opacity >= 1.0);
            if !plain {
                status.and(&Status::approximated(format!(
                    "polygon fill with its own blend ({:?} {}) inside a mask group laid over normally",
                    own.mode(),
                    own.opacity
                )));
            }
            all.extend(runs.runs);
        }
        self.note(on, &status);
        self.push(Some(&g.header), EffectKind::Paint, on);
        if all.iter().any(|r| !r.triangles.is_empty()) {
            self.fill_job(all, "polygon fill group");
        }
        true
    }

    /// Lays `runs` into the paint effect just pushed.
    fn fill_job(&mut self, runs: Vec<FillRun>, what: &str) {
        let id = self.effects.last().map_or(texturecraft_core::LayerId(0), |e| e.id);
        self.fills.push(FillJob { effect: id, channel: None, runs, label: format!("{} / {what}", self.path), over_strokes: false });
    }

    /// TextureCraft selects within a per-component tolerance on the baked map the selection's bitmap names (the ID map
    /// when it names none, or a bitmap that is no mesh map of the set); the file's hardness is left out.
    fn color_selection(&mut self, c: &ColorSelectionEffect, on: bool) {
        let tolerance = if c.tolerance.is_finite() { c.tolerance.clamp(0.0, 1.0) } else { 0.0 };
        self.note(on, &Status::approximated("colour selection: tolerance per component, hardness left out"));
        let map = match selection_map(&c.bitmap_url, self.input.mesh_maps) {
            Some(map) => map,
            None if c.bitmap_url.is_empty() => BakedMap::Id,
            None => {
                self.note(on, &Status::approximated("colour selection from a bitmap that is not one of the set's mesh maps: picked on the ID map"));
                BakedMap::Id
            }
        };
        let colors = c.colors.iter().map(|v| v.map(|x| if x.is_finite() { x } else { 0.0 })).collect();
        self.push(Some(&c.header), EffectKind::ColorSelection { colors, tolerance, map }, on);
    }

    /// A fill, by its (first) source.
    fn fill(&mut self, fill: &FillEffect, on: bool) {
        let blend = ResolvedBlend::of_mask(&fill.header.blending);
        let counts = on && blend.contributes();
        let Some(source) = fill.sources.first() else { return };
        let mapped = projection::map(fill, self.input.geometry.map(|geometry| geometry.mesh));
        let projection = mapped.projection.clone();
        match source {
            ProjectSource::Uniform(u) => {
                let v = if u.color[0].is_finite() { u.color[0] } else { 0.0 };
                // laid on through the projection's coverage (clip, falloffs): as exact as the projection
                if projection::shapes_coverage(fill) {
                    self.note(counts, &mapped.status);
                }
                self.push(Some(&fill.header), EffectKind::Fill { source: Source::Uniform([v; 3]), projection }, on);
            }
            ProjectSource::Bitmap(b) => self.bitmap(fill, b, counts, on, mapped),
            ProjectSource::Procedural(p) => {
                let mut g = graph_uses::mask_generator(self.input, self.anchors, p, projection);
                if let EffectKind::Graph { graph, .. } = &mut g.effect {
                    graph.unfiltered = projection::bilinear(fill);
                }
                if g.status.is_computable() {
                    self.note(counts, &mapped.status);
                }
                self.graph_use(&fill.header, g, on);
            }
            ProjectSource::Reference(r) if is_dangling(r) => {
                // changes nothing, as if hidden
                self.placeholder(Some(&fill.header), "reference (no anchor)", "the reference reads no anchor", false, Status::Exact);
            }
            ProjectSource::Reference(r) => match reference(self.anchors, r, texturecraft_core::Channel::Height) {
                Ok(Some((source, status))) => {
                    self.note(counts, &status);
                    self.push(Some(&fill.header), EffectKind::Fill { source, projection }, on);
                }
                Ok(None) => {}
                Err(why) => self.placeholder(Some(&fill.header), "reference to an anchor", "what the anchor holds is not computed", on, why),
            },
            ProjectSource::Other(raw) => {
                let why = Status::not_computable(Blocker::class(&raw.class, format!("mask source {}", raw.class)));
                self.placeholder(Some(&fill.header), &format!("unknown source {}", raw.class), "a source class this version does not read", on, why);
            }
        }
    }

    /// A bitmap mask fill: the image as a resource; a mesh map slot left empty reads black (measured once).
    fn bitmap(&mut self, fill: &FillEffect, b: &BitmapSource, counts: bool, on: bool, mapped: projection::MappedProjection) {
        let url = match mesh_map_alias(&b.url, self.input.mesh_maps) {
            MeshMapAlias::Unassigned(name) => {
                self.note(counts, &Status::approximated(format!("mask bitmap of the empty mesh map slot \"{name}\" read as black")));
                self.push(Some(&fill.header), EffectKind::Fill { source: Source::Uniform([0.0; 3]), projection: mapped.projection }, on);
                return;
            }
            MeshMapAlias::Assigned(url) => {
                self.note(counts, &Status::approximated("mask bitmap of a mesh map slot read as the map assigned to it"));
                url
            }
            MeshMapAlias::Unknown => {
                let why = Status::not_computable(Blocker::feature("mesh-map-alias", "mask bitmap of an unknown mesh map slot"));
                self.placeholder(Some(&fill.header), "mesh map", "an unknown mesh map slot", on, why);
                return;
            }
            MeshMapAlias::NotAlias => b.url.as_str(),
        };
        self.note(counts, &mapped.status);
        match self.input.images.load(url) {
            Ok(loaded) => self.push(Some(&fill.header), EffectKind::Fill { source: Source::image(loaded.name), projection: mapped.projection }, on),
            Err(why) => {
                let why = Status::not_computable(Blocker::feature("unreadable-bitmap", why));
                self.placeholder(Some(&fill.header), "bitmap", "the bitmap could not be read", on, why);
            }
        }
    }
}

/// The baked map a colour selection picks from: the mesh map of the set its bitmap URL names (directly, through a mesh
/// map slot, or by the resource name the file gives baked maps); `None` for any other bitmap and for an empty slot.
pub(crate) fn selection_map(url: &str, maps: &[MeshMapRef]) -> Option<BakedMap> {
    let url = match mesh_map_alias(url, maps) {
        MeshMapAlias::NotAlias => url,
        MeshMapAlias::Assigned(assigned) => assigned,
        MeshMapAlias::Unassigned(_) | MeshMapAlias::Unknown => return None,
    };
    if let Some(m) = maps.iter().find(|m| m.url == url) {
        return crate::baked::baked_kind(m.usage);
    }
    let (name, _) = split_resource_url(url)?;
    crate::baked::baked_kind_of_name(name)
}

/// How many different greys the runs write.
fn distinct_values(runs: &[FillRun]) -> usize {
    let mut seen: Vec<f32> = Vec::new();
    for r in runs.iter().filter(|r| !r.triangles.is_empty()) {
        if !seen.contains(&r.color[0]) {
            seen.push(r.color[0]);
        }
    }
    seen.len()
}

#[cfg(test)]
mod selection_map_tests {
    use super::*;

    fn maps() -> Vec<MeshMapRef> {
        vec![
            MeshMapRef { usage: Some(0), url: "/Color Map from Mesh S?version=a".to_string() },
            MeshMapRef { usage: Some(2), url: "/Curvature Map from Mesh S?version=b".to_string() },
            MeshMapRef { usage: Some(1), url: "resourcealias:AmbientOcclusion".to_string() },
            MeshMapRef { usage: Some(5), url: "/my thickness?version=c".to_string() },
        ]
    }

    #[test]
    fn a_colour_selection_picks_from_the_mesh_map_its_bitmap_names() {
        let maps = maps();
        assert_eq!(selection_map("/Curvature Map from Mesh S?version=b", &maps), Some(BakedMap::Curvature));
        assert_eq!(selection_map("/Color Map from Mesh S?version=a", &maps), Some(BakedMap::Id));
        // a slot's assigned map, under any resource name
        assert_eq!(selection_map("/my thickness?version=c", &maps), Some(BakedMap::Thickness));
        assert_eq!(selection_map("resourcealias:Thickness", &maps), Some(BakedMap::Thickness));
        // a baked map the set does not list, by the name the file gives baked maps
        assert_eq!(selection_map("/Ambient Occlusion Map from Mesh S?version=d", &maps), Some(BakedMap::AmbientOcclusion));
        // an empty slot, an unknown slot, any other bitmap, nothing
        assert_eq!(selection_map("resourcealias:AmbientOcclusion", &maps), None);
        assert_eq!(selection_map("resourcealias:Nonsense", &maps), None);
        assert_eq!(selection_map("/painted grunge?version=e", &maps), None);
        assert_eq!(selection_map("", &maps), None);
    }
}
