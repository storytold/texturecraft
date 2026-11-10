//! [`MaskBuilder`]: a layer's mask (`maskActions` with `maskEnabled` and `maskInitial`) as TextureCraft's [`Mask`],
//! effect for effect, and how exactly it reproduces the original.
//!
//! Uniform fills become grey fills, bitmap fills image fills (a mesh map slot left empty reads black), reference fills
//! anchor fills; levels and colour selection keep their kind. Compiled-graph generators and filters, painted strokes,
//! polygon fills, anchors inside the mask and unknown classes are placeholders, and the first of them makes the mask not
//! computable. Effect groups are laid flat (approximated unless they pass through or blend normally at full strength).

use texturecraft_core::{BlendMode, Effect, EffectKind, Mask, MaskBackground, Source};
use texturecraft_import_reader::blockers::{Blocker, BlockerKey, BlockerKind};
use texturecraft_import_reader::model::{BitmapSource, ColorSelectionEffect, EffectHeader, FillEffect, MaskInitial, ProjectEffect, ProjectMask, ProjectSource};

use super::content::{is_dangling, levels, reference};
use super::{Anchors, Ids, SetInput, effect, projection};
use crate::blend::ResolvedBlend;
use crate::graph_uses::{self, GraphUse};
use crate::resources::{MeshMapAlias, mesh_map_alias};
use crate::status::Status;

/// The mask for a layer (`None` = no mask) and how exact it is.
pub struct MaskOutcome {
    pub mask: Option<Mask>,
    pub status: Status,
    pub graphs: Vec<String>,
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
}

impl<'a, 'i> MaskBuilder<'a, 'i> {
    pub fn new(input: &'a SetInput<'a>, anchors: &'a Anchors, ids: &'i mut Ids) -> MaskBuilder<'a, 'i> {
        MaskBuilder { input, anchors, ids, effects: Vec::new(), status: Status::Exact, graphs: Vec::new(), counts: true, strokes_run: None }
    }

    pub fn build(mut self, mask: Option<&ProjectMask>) -> MaskOutcome {
        let Some(m) = mask else { return MaskOutcome { mask: None, status: Status::Exact, graphs: Vec::new() } };
        let background = match m.initial {
            MaskInitial::Black => MaskBackground::Black,
            MaskInitial::White => MaskBackground::White,
            // An initial value we cannot read: the layer gets no mask (a switched-off one does not matter).
            MaskInitial::Other(n) => {
                let status = match m.enabled {
                    true => Status::not_computable(Blocker::feature("mask-initial-value", format!("mask initial value {n}"))),
                    false => Status::Exact,
                };
                return MaskOutcome { mask: None, status, graphs: Vec::new() };
            }
        };
        self.counts = m.enabled;
        for e in &m.effects {
            self.effect(e, true);
        }
        let mask = Mask { enabled: m.enabled, background, effects: self.effects };
        MaskOutcome { mask: Some(mask), status: self.status, graphs: self.graphs }
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
                let why = Status::not_computable(Blocker::new(BlockerKey::new(BlockerKind::Strokes, "painted-mask"), "painted mask (strokes)"));
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
                self.placeholder(header, &label, "brush strokes are kept in the original result; stroke replay comes later", on, why);
                self.strokes_run = Some((self.effects.len() - 1, p.stroke_count(), 1));
            }
            ProjectEffect::Anchor(h) => {
                let why = match self.anchors.get(&h.uid.unwrap_or(0)) {
                    Some(_) => Status::approximated("anchor point inside a mask (TextureCraft's anchors record layer content)"),
                    None => Status::Exact,
                };
                self.placeholder(header, "anchor point", "anchors inside masks are not recorded", on, why);
            }
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
                self.graph_use(&f.header, graph_uses::filter(f, true), on);
            }
            ProjectEffect::Levels(l) => {
                let (ours, status) = levels(l.levels.as_ref());
                self.note(on, &status);
                self.push(header, EffectKind::Levels(ours), on);
            }
            ProjectEffect::ColorSelection(c) => self.color_selection(c, on),
            ProjectEffect::PolygonFill(_) => {
                let why = Status::not_computable(Blocker::feature("polygon-fill", "polygon fill in a mask"));
                self.placeholder(header, "polygon fill", "polygon fills are not computed yet", on, why);
            }
            ProjectEffect::Other(raw) => {
                let why = Status::not_computable(Blocker::class(&raw.class, format!("mask effect {}", raw.class)));
                self.placeholder(None, &format!("unknown effect {}", raw.class), "an effect class this version does not read", on, why);
            }
        }
    }

    /// TextureCraft selects within a per-component tolerance on the set's ID map; the file's hardness is left out.
    fn color_selection(&mut self, c: &ColorSelectionEffect, on: bool) {
        let tolerance = if c.tolerance.is_finite() { c.tolerance.clamp(0.0, 1.0) } else { 0.0 };
        self.note(on, &Status::approximated("colour selection: tolerance per component, hardness left out"));
        let colors = c.colors.iter().map(|v| v.map(|x| if x.is_finite() { x } else { 0.0 })).collect();
        self.push(Some(&c.header), EffectKind::ColorSelection { colors, tolerance }, on);
    }

    /// A fill, by its (first) source.
    fn fill(&mut self, fill: &FillEffect, on: bool) {
        let blend = ResolvedBlend::of_mask(&fill.header.blending);
        let counts = on && blend.contributes();
        let Some(source) = fill.sources.first() else { return };
        let mapped = projection::map(fill);
        let projection = mapped.projection.clone();
        match source {
            ProjectSource::Uniform(u) => {
                let v = if u.color[0].is_finite() { u.color[0] } else { 0.0 };
                if projection::shapes_coverage(fill) {
                    self.note(counts, &Status::approximated("uniform mask fill faded or clipped by its projection"));
                }
                self.push(Some(&fill.header), EffectKind::Fill { source: Source::Uniform([v; 3]), projection }, on);
            }
            ProjectSource::Bitmap(b) => self.bitmap(fill, b, counts, on, mapped),
            ProjectSource::Procedural(p) => self.graph_use(&fill.header, graph_uses::mask_generator(p), on),
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
