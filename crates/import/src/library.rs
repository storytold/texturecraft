//! What an imported project brings for the user's own asset library ([`LibraryOffer`]): the brushes of its paint
//! effects (one per distinct brush), the bitmaps it uses as brush alphas, stencils, fills, paint colours or graph
//! inputs, and the viewer's environment map, each with where it came from ([`Origin`]) and whether a copy may be
//! shared. The project is only read; the imported document keeps its own copies whatever is added.
//!
//! The rule for sharing needs nothing but the resource URL: an item stored in the project file itself
//! (`/<name>?version=<id>`: imported into the project by the user, or made for it) is the user's own and shareable.
//! An item from a shelf (`resource://<shelf>/…`) is not: whose shelf it is cannot be told from the file without knowing
//! the reference app's own shelves, so such items are kept "from a shelf, not shareable". Brushes are made from the
//! project's own settings and are shareable. Anything else names no stored resource and is never offered.
//!
//! Baked mesh maps belong to the mesh, not the library: never offered. Bitmaps not stored in the file cannot be copied
//! and are only counted ([`LibraryOffer::left_out`]).

use std::collections::{BTreeMap, HashSet};

use texturecraft_import_reader::model::viewer::{VIEWER_INI, VIEWER_INI_2, ViewerSettings};
use texturecraft_import_reader::model::{PaintEffect, Procedural, ProjectDocument, ProjectEffect, ProjectLayer, ProjectSource};
use texturecraft_import_reader::project::ProjectFile;

use crate::error::ImportError;
use crate::replay::{ReplayBrush, ReplayTip, tip};
use crate::resources::{SHELF_SCHEME, shelf_version, split_resource_url, url_name};

/// Where an item came from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Origin {
    /// Stored in the project file, or made from the project's own settings (brushes).
    Project,
    /// From a shelf of the reference app, by the shelf's name as the URL gives it.
    Shelf(String),
}

impl Origin {
    /// The origin of a resource URL; `None` for URLs that name no stored resource.
    pub fn of_url(url: &str) -> Option<Origin> {
        if let Some(rest) = url.strip_prefix(SHELF_SCHEME) {
            let shelf = rest.split('/').next().unwrap_or_default();
            let name = shelf.split('.').next().unwrap_or_default();
            return (!name.is_empty()).then(|| Origin::Shelf(name.to_string()));
        }
        url.starts_with('/').then_some(Origin::Project)
    }

    /// True when a copy may leave the user's own library: only what the project file itself holds.
    pub fn shareable(&self) -> bool {
        matches!(self, Origin::Project)
    }

    /// The origin in words.
    pub fn describe(&self) -> String {
        match self {
            Origin::Project => "stored in the project".to_string(),
            Origin::Shelf(s) => format!("the reference app's shelf \"{s}\" (from a shelf, not shareable)"),
        }
    }

    /// A short id for replies: `project` or `shelf`.
    pub fn id(&self) -> &'static str {
        match self {
            Origin::Project => "project",
            Origin::Shelf(_) => "shelf",
        }
    }
}

/// What an item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ItemKind {
    /// A brush preset made from the brush settings of the project's strokes.
    Brush,
    /// A bitmap brush alpha.
    Alpha,
    /// A bitmap stencil.
    Stencil,
    /// A bitmap used as a fill, as paint colour or as a graph's input.
    Texture,
    /// The viewer's environment map.
    Environment,
}

impl ItemKind {
    pub fn id(self) -> &'static str {
        match self {
            ItemKind::Brush => "brush",
            ItemKind::Alpha => "alpha",
            ItemKind::Stencil => "stencil",
            ItemKind::Texture => "texture",
            ItemKind::Environment => "environment",
        }
    }
}

/// The tip of an offered brush, as TextureCraft's brush draws it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BrushTip {
    /// The round tip with this hardness; `exact` when it is the same curve as the original's.
    Round { hardness: f32, exact: bool },
}

/// An offered brush: its settings in neutral quantities ([`ReplayBrush`], the replay's mapping) and its tip.
#[derive(Debug, Clone, PartialEq)]
pub struct OfferedBrush {
    pub brush: ReplayBrush,
    pub tip: BrushTip,
    /// The paint effects' opacity it was used with most.
    pub opacity: f32,
}

/// What an item is made from.
#[derive(Debug, Clone, PartialEq)]
pub enum ItemSource {
    /// An image resource by its URL (stored in the project).
    Image {
        url: String,
    },
    Brush(OfferedBrush),
}

/// One item the project brings.
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryItem {
    /// Unique within the offer: what an add names.
    pub key: String,
    pub kind: ItemKind,
    /// The name it gets in the library.
    pub name: String,
    pub origin: Origin,
    /// How many effects use it.
    pub uses: usize,
    pub source: ItemSource,
}

impl LibraryItem {
    /// False for content that is not the user's own: "from this project, not shareable".
    pub fn shareable(&self) -> bool {
        self.origin.shareable()
    }
}

/// A project's items for the user's library.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LibraryOffer {
    /// The project's file name.
    pub project: String,
    /// Brushes first, then alphas, stencils, textures and the environment; most used first within a kind.
    pub items: Vec<LibraryItem>,
    /// What was seen but not offered, and why (`<count> × <why>`).
    pub left_out: Vec<String>,
}

impl LibraryOffer {
    pub fn get(&self, key: &str) -> Option<&LibraryItem> {
        self.items.iter().find(|i| i.key == key)
    }
}

/// Most brushes and images offered per project (a guard against hostile files).
const MAX_ITEMS: usize = 4096;
/// Deepest layer and effect nesting walked (a guard against hostile files).
const MAX_DEPTH: usize = 64;

/// The items of the opened project `file` (`file_name` names it): its layer document, the baked maps its texture sets
/// name (left out) and the viewer's environment map. Fails only when the layer document cannot be read.
pub fn offer(file: &ProjectFile, file_name: &str) -> Result<LibraryOffer, ImportError> {
    let (document, _) = file.model()?;
    let baked: HashSet<String> = file.document_materials().unwrap_or_default().into_iter().flat_map(|m| m.baked).map(|b| b.url).collect();
    let environment =
        file.read(VIEWER_INI).or_else(|_| file.read(VIEWER_INI_2)).ok().and_then(|d| ViewerSettings::parse(&d).ok()).and_then(|v| v.environment_url);
    let stored = |url: &str| {
        let id = split_resource_url(url).map(|(_, id)| id).or_else(|| shelf_version(url));
        id.is_some_and(|id| crate::resources::resource_dataset(file, id).is_ok())
    };
    let facts = Facts { project: file_name.to_string(), baked, environment, stored: &stored };
    Ok(Collector::new(&facts).collect(&document))
}

/// The items of the project file at `path`.
#[cfg(not(target_arch = "wasm32"))]
pub fn offer_path(path: &std::path::Path) -> Result<LibraryOffer, ImportError> {
    let file = ProjectFile::open(path)?;
    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    offer(&file, &name)
}

/// The items of the project file `bytes` (`file_name` names it).
pub fn offer_bytes(bytes: Vec<u8>, file_name: &str) -> Result<LibraryOffer, ImportError> {
    offer(&ProjectFile::from_bytes(bytes)?, file_name)
}

/// What the walk needs to know besides the layer document.
pub struct Facts<'a> {
    pub project: String,
    /// URLs of the baked mesh maps.
    pub baked: HashSet<String>,
    pub environment: Option<String>,
    /// True when the image a URL names is stored in the project (so it can be copied).
    pub stored: &'a dyn Fn(&str) -> bool,
}

struct ImageSeen {
    origin: Origin,
    uses: usize,
}

/// One brush seen (by its character), with the settings it was used with and how often.
struct BrushSeen {
    variants: Vec<(String, OfferedBrush, usize)>,
    uses: usize,
    order: usize,
}

/// Walks a document for items.
pub struct Collector<'a> {
    facts: &'a Facts<'a>,
    images: BTreeMap<(ItemKind, String), ImageSeen>,
    brushes: BTreeMap<String, BrushSeen>,
    left_out: BTreeMap<String, usize>,
}

/// A brush without what an artist changes from stroke to stroke (size, flow, opacity): effects with the same character
/// are one brush.
fn character(b: &OfferedBrush) -> String {
    let brush = ReplayBrush { radius: 0.0, flow: 0.0, ..b.brush.clone() };
    format!("{:?}|{:?}", brush, b.tip)
}

/// The exact settings.
fn exact(b: &OfferedBrush) -> String {
    format!("{:?}|{:?}|{}", b.brush, b.tip, b.opacity)
}

impl<'a> Collector<'a> {
    pub fn new(facts: &'a Facts<'a>) -> Collector<'a> {
        Collector { facts, images: BTreeMap::new(), brushes: BTreeMap::new(), left_out: BTreeMap::new() }
    }

    /// Every item of `document` and the viewer settings.
    pub fn collect(mut self, document: &ProjectDocument) -> LibraryOffer {
        for set in &document.texture_sets {
            self.layers(&set.layers, 0);
        }
        if let Some(url) = self.facts.environment.clone() {
            self.image(ItemKind::Environment, &url);
        }
        self.finish()
    }

    fn leave_out(&mut self, why: impl Into<String>) {
        *self.left_out.entry(why.into()).or_default() += 1;
    }

    fn layers(&mut self, layers: &[ProjectLayer], depth: usize) {
        if depth > MAX_DEPTH {
            return self.leave_out("layer folders nested too deeply to walk");
        }
        for l in layers {
            self.effects(&l.content, depth);
            if let Some(mask) = &l.mask {
                self.effects(&mask.effects, depth);
            }
            self.layers(l.children(), depth + 1);
        }
    }

    fn effects(&mut self, effects: &[ProjectEffect], depth: usize) {
        if depth > MAX_DEPTH {
            return self.leave_out("effect groups nested too deeply to walk");
        }
        for e in effects {
            match e {
                ProjectEffect::Paint(p) => self.paint(p),
                ProjectEffect::Fill(f) => f.sources.iter().for_each(|s| self.source(s, ItemKind::Texture, depth)),
                ProjectEffect::Filter(f) => {
                    if let Some(p) = &f.procedural {
                        self.procedural(p, depth);
                    }
                }
                ProjectEffect::Group(g) => self.effects(&g.effects, depth + 1),
                _ => {}
            }
        }
    }

    fn paint(&mut self, p: &PaintEffect) {
        if !p.strokes.is_empty() {
            match brush(p) {
                Ok(b) => {
                    if self.brushes.len() >= MAX_ITEMS {
                        self.leave_out("brush past the limit of items");
                    } else {
                        let order = self.brushes.len();
                        let seen = self.brushes.entry(character(&b)).or_insert(BrushSeen { variants: Vec::new(), uses: 0, order });
                        seen.uses += 1;
                        let key = exact(&b);
                        match seen.variants.iter_mut().find(|(k, _, _)| *k == key) {
                            Some((_, _, n)) => *n += 1,
                            None => seen.variants.push((key, b, 1)),
                        }
                    }
                }
                Err(why) => self.leave_out(format!("brush with {why}")),
            }
        }
        if let Some(ProjectSource::Bitmap(b)) = p.alpha.first() {
            self.image(ItemKind::Alpha, &b.url);
        }
        if let Some(stencil) = &p.stencil {
            self.source(stencil, ItemKind::Stencil, 0);
        }
        p.colors.iter().for_each(|s| self.source(s, ItemKind::Texture, 0));
    }

    fn source(&mut self, s: &ProjectSource, kind: ItemKind, depth: usize) {
        match s {
            ProjectSource::Bitmap(b) => self.image(kind, &b.url),
            ProjectSource::Procedural(ps) => {
                if let Some(p) = &ps.procedural {
                    self.procedural(p, depth + 1);
                }
            }
            _ => {}
        }
    }

    /// The bitmaps plugged into a graph's image inputs (most are baked maps, which are skipped).
    fn procedural(&mut self, p: &Procedural, depth: usize) {
        if depth > MAX_DEPTH {
            return self.leave_out("graph inputs nested too deeply to walk");
        }
        for input in &p.inputs {
            if let Some(s) = input.source.as_deref() {
                self.source(s, ItemKind::Texture, depth);
            }
        }
    }

    fn image(&mut self, kind: ItemKind, url: &str) {
        let Some(origin) = Origin::of_url(url) else { return };
        if self.facts.baked.contains(url) || crate::baked::baked_kind_of_name(url_name(url)).is_some() {
            return;
        }
        if !(self.facts.stored)(url) {
            return self.leave_out(format!("{} not stored in the project", kind.id()));
        }
        if self.images.len() >= MAX_ITEMS && !self.images.contains_key(&(kind, url.to_string())) {
            return self.leave_out("image past the limit of items");
        }
        self.images.entry((kind, url.to_string())).or_insert(ImageSeen { origin, uses: 0 }).uses += 1;
    }

    fn finish(self) -> LibraryOffer {
        let stem = std::path::Path::new(&self.facts.project).file_stem().map_or_else(|| "project".to_string(), |s| s.to_string_lossy().into_owned());
        let mut items = Vec::new();
        let mut seen: Vec<BrushSeen> = self.brushes.into_values().collect();
        seen.sort_by(|a, b| b.uses.cmp(&a.uses).then(a.order.cmp(&b.order)));
        for (i, b) in seen.into_iter().enumerate() {
            let uses = b.uses;
            // the settings used most (the first of them on a tie)
            let mut best: Option<(OfferedBrush, usize)> = None;
            for (_, offered, n) in b.variants {
                if best.as_ref().is_none_or(|(_, m)| n > *m) {
                    best = Some((offered, n));
                }
            }
            let Some((offered, _)) = best else { continue };
            let BrushTip::Round { hardness, .. } = offered.tip;
            let tip = if hardness >= 0.75 {
                "hard round tip"
            } else if hardness <= 0.25 {
                "soft round tip"
            } else {
                "round tip"
            };
            items.push(LibraryItem {
                key: format!("brush:{}", i + 1),
                kind: ItemKind::Brush,
                name: format!("{stem} brush {} - {tip}", i + 1),
                origin: Origin::Project,
                uses,
                source: ItemSource::Brush(offered),
            });
        }
        let mut images: Vec<((ItemKind, String), ImageSeen)> = self.images.into_iter().collect();
        images.sort_by(|((ka, ua), a), ((kb, ub), b)| ka.cmp(kb).then(b.uses.cmp(&a.uses)).then(ua.cmp(ub)));
        for ((kind, url), seen) in images {
            items.push(LibraryItem {
                key: format!("{}:{url}", kind.id()),
                kind,
                name: url_name(&url).to_string(),
                origin: seen.origin,
                uses: seen.uses,
                source: ItemSource::Image { url },
            });
        }
        let left_out = self.left_out.into_iter().map(|(why, n)| format!("{n} × {why}")).collect();
        LibraryOffer { project: self.facts.project.clone(), items, left_out }
    }
}

/// The brush of a paint effect, or why it is not offered. Only round tips are: a bitmap alpha would need the image in
/// the library with it, and TextureCraft's library keeps no alpha images (the bitmap is offered on its own); a graph
/// alpha or a constant has no tip of TextureCraft's.
fn brush(p: &PaintEffect) -> Result<OfferedBrush, String> {
    crate::replay::check(p).map_err(|b| b.text)?;
    if matches!(p.brush.as_ref().map(|b| &b.emitter), Some(texturecraft_import_reader::model::StampEmitter::Ribbon(_))) {
        return Err("a path ribbon brush (TextureCraft's brush presets lay stamps)".to_string());
    }
    let (hardness, exact) = match (tip::round_tip_hardness(&p.alpha), p.alpha.first()) {
        (Some(h), _) => h,
        (None, Some(ProjectSource::Bitmap(_))) => return Err("a bitmap alpha (TextureCraft's library keeps no alpha images)".to_string()),
        (None, Some(ProjectSource::Procedural(q))) => {
            return Err(format!("a compiled-graph alpha \"{}\"", q.procedural.as_ref().map_or("?", |q| q.name())));
        }
        (None, Some(other)) => return Err(format!("a {} alpha", other.kind())),
        (None, None) => return Err("no alpha".to_string()),
    };
    let mut notes = Vec::new();
    let brush = crate::replay::brush::map_brush(p, ReplayTip::Round { hardness }, &mut notes);
    let opacity = if p.opacity.is_finite() { p.opacity.clamp(0.0, 1.0) } else { 1.0 };
    Ok(OfferedBrush { brush, tip: BrushTip::Round { hardness, exact }, opacity })
}

#[cfg(test)]
mod tests;
