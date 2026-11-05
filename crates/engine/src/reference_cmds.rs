//! Reference images: pictures to paint from. They live in the project (an embedded copy in the
//! project file, or a link to a file next to it), show in the Reference panel and, pinned, over
//! the 3D view. Nothing here reaches the compositor, the renderer or an export.
//!
//! A reference whose file is gone or whose bytes do not decode stays in the project with an error
//! status; opening a project never fails because of one.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use texturecraft_core::{MAX_REFERENCES, Reference, ReferenceSource};
use texturecraft_texel::{linear_to_srgb, srgb_to_linear};

use crate::commands::{CommandSpec, always, bad, boolp, f32p, strp, u64p};
use crate::{EngineError, Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "reference.add",
            "Add reference image",
            [],
            None,
            "{path | resource, name?, storage?: embedded|linked} → {id, name, width, height}; `path` (a file) is not available in the browser or over the control channel's `command.run`; `resource` names an imported image resource; `linked` keeps a path (relative to the project file once it is saved) instead of a copy in the project",
            always,
            add
        ),
        cmd!("reference.remove", "Remove reference image", [], None, "{id}", always, remove),
        cmd!(
            "reference.list",
            "List reference images",
            [],
            None,
            "{} → {references: [{id, name, storage, path?, status: ok|missing|invalid, error?, width, height, flipH, flipV, grayscale}], overlay}",
            always,
            |s, _| Ok(list(s))
        ),
        cmd!("reference.set", "Edit reference image", [], None, "{id, name?, flipH?, flipV?, grayscale?}", always, set),
        cmd!(
            "reference.overlay",
            "Pin reference over the 3D view",
            [],
            None,
            "{reference?: id | null (off), opacity? (0..1), x?, y? (image centre, fraction of the 3D view), scale? (image height, fraction of the view height)} → the overlay; the overlay is a screen overlay: exports and renders never include it",
            always,
            overlay
        ),
        cmd!(
            "reference.pick",
            "Pick colour from reference",
            [],
            None,
            "{id, u, v (0..1 across and down the image as shown: flips and grayscale applied), radius? (source pixels averaged, 0), apply?: true (sets the brush base colour)} → {linear: [r,g,b], srgb: \"#rrggbb\", applied}",
            always,
            pick
        ),
        cmd!("reference.reload", "Reload reference images", [], None, "{id?} — reads linked files again (after restoring a missing file)", always, reload),
    ]
}

/// Most texels (width × height) of a reference image.
pub const MAX_TEXELS: u64 = 1 << 25;
/// Largest reference file.
pub const MAX_BYTES: usize = 256 << 20;
/// Resource name prefix of embedded copies (kept out of the Assets list and the unused-resource sweep).
pub const RESOURCE_PREFIX: &str = "reference.";
/// Is resource `name` the embedded copy of a reference image (not an asset to list or use)?
pub fn is_embedded_copy(name: &str) -> bool {
    name.starts_with(RESOURCE_PREFIX)
}

/// Largest side of a display copy (what a GPU texture holds everywhere).
pub const DISPLAY_SIDE: u32 = 2048;
/// Largest averaging radius of a pick, in source pixels.
const MAX_PICK_RADIUS: f32 = 64.0;

/// A decoded reference: 8-bit sRGB RGBA, row-major, top row first.
#[derive(Clone, Debug, PartialEq)]
pub struct RefImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// How a reference is shown and picked from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RefView {
    pub flip_h: bool,
    pub flip_v: bool,
    pub grayscale: bool,
}

impl RefView {
    pub fn of(r: &Reference) -> RefView {
        RefView { flip_h: r.flip_h, flip_v: r.flip_v, grayscale: r.grayscale }
    }
}

fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Rec. 709 luminance of a linear colour.
pub fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

impl RefImage {
    /// Decode any supported image file; sizes are checked from the header first.
    pub fn from_bytes(bytes: &[u8]) -> std::result::Result<RefImage, String> {
        if bytes.len() > MAX_BYTES {
            return Err(format!("the file is larger than {} MB", MAX_BYTES >> 20));
        }
        let d = texturecraft_formats::decode_image_within(bytes, MAX_TEXELS).map_err(|e| e.to_string())?;
        let encode = |v: f32| if d.srgb { to_u8(v) } else { to_u8(linear_to_srgb(v.clamp(0.0, 1.0))) };
        let mut rgba = Vec::with_capacity(d.rgba.len() * 4);
        for p in &d.rgba {
            rgba.extend_from_slice(&[encode(p[0]), encode(p[1]), encode(p[2]), to_u8(p[3])]);
        }
        if rgba.len() as u64 != u64::from(d.width) * u64::from(d.height) * 4 {
            return Err("the image data does not match its size".into());
        }
        Ok(RefImage { width: d.width, height: d.height, rgba })
    }

    /// The pixel at (`x`, `y`), clamped to the image.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let x = x.min(self.width.saturating_sub(1)) as usize;
        let y = y.min(self.height.saturating_sub(1)) as usize;
        let i = (y * self.width as usize + x) * 4;
        match self.rgba.get(i..i + 4) {
            Some(p) => [p[0], p[1], p[2], p[3]],
            None => [0; 4],
        }
    }

    /// The colour at (`u`, `v`) of the image as shown (0..1 across and down; flips applied),
    /// as a linear RGB value: the sRGB pixels are decoded first and, with `radius` > 0, the
    /// pixels within that many source pixels are averaged *in linear light* (alpha-weighted), so
    /// a black and white checker picks 0.5 linear, not 0.5 sRGB. Grayscale gives the Rec. 709
    /// luminance. `None` for a non-finite position or fully transparent pixels.
    pub fn pick(&self, u: f32, v: f32, radius: f32, view: RefView) -> Option<[f32; 3]> {
        if !u.is_finite() || !v.is_finite() || self.width == 0 || self.height == 0 {
            return None;
        }
        let u = if view.flip_h { 1.0 - u } else { u }.clamp(0.0, 1.0);
        let v = if view.flip_v { 1.0 - v } else { v }.clamp(0.0, 1.0);
        let cx = ((u * self.width as f32) as u32).min(self.width - 1);
        let cy = ((v * self.height as f32) as u32).min(self.height - 1);
        let r = if radius.is_finite() { radius.clamp(0.0, MAX_PICK_RADIUS).round() as u32 } else { 0 };
        let (x0, x1) = (cx.saturating_sub(r), cx.saturating_add(r).min(self.width - 1));
        let (y0, y1) = (cy.saturating_sub(r), cy.saturating_add(r).min(self.height - 1));
        let (mut sum, mut weight) = ([0.0f64; 3], 0.0f64);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let p = self.pixel(x, y);
                let a = f64::from(p[3]) / 255.0;
                for (acc, c) in sum.iter_mut().zip(&p[..3]) {
                    *acc += f64::from(srgb_to_linear(f32::from(*c) / 255.0)) * a;
                }
                weight += a;
            }
        }
        if weight <= 0.0 {
            return None;
        }
        let c = [(sum[0] / weight) as f32, (sum[1] / weight) as f32, (sum[2] / weight) as f32];
        Some(if view.grayscale { [luminance(c); 3] } else { c })
    }

    /// An RGBA copy for display, at most `max_side` across (sampled from the source), with the
    /// grayscale option applied (luminance in linear light). Flips are the display's job.
    pub fn display_copy(&self, max_side: u32, grayscale: bool) -> (u32, u32, Vec<u8>) {
        let max_side = max_side.max(1);
        let scale = (self.width.max(self.height) as f32 / max_side as f32).max(1.0);
        let w = ((self.width as f32 / scale).round() as u32).clamp(1, self.width.max(1));
        let h = ((self.height as f32 / scale).round() as u32).clamp(1, self.height.max(1));
        let mut out = Vec::with_capacity(w as usize * h as usize * 4);
        for y in 0..h {
            for x in 0..w {
                let p = self.pixel(((x as f32 + 0.5) * scale) as u32, ((y as f32 + 0.5) * scale) as u32);
                if grayscale {
                    let lin = [srgb_to_linear(f32::from(p[0]) / 255.0), srgb_to_linear(f32::from(p[1]) / 255.0), srgb_to_linear(f32::from(p[2]) / 255.0)];
                    let g = to_u8(linear_to_srgb(luminance(lin)));
                    out.extend_from_slice(&[g, g, g, p[3]]);
                } else {
                    out.extend_from_slice(&p);
                }
            }
        }
        (w, h, out)
    }
}

/// A loaded reference (or why it could not load).
#[derive(Clone, Debug)]
pub struct RefEntry {
    pub state: std::result::Result<Arc<RefImage>, RefError>,
    /// The file a linked reference was read from.
    pub resolved: Option<std::path::PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RefError {
    /// The linked file (or the embedded copy) is not there.
    Missing(String),
    /// The bytes are not an image this build reads.
    Invalid(String),
}

impl RefError {
    pub fn message(&self) -> &str {
        match self {
            RefError::Missing(m) | RefError::Invalid(m) => m,
        }
    }
    fn status(&self) -> &'static str {
        match self {
            RefError::Missing(_) => "missing",
            RefError::Invalid(_) => "invalid",
        }
    }
}

pub type RefCache = BTreeMap<u64, RefEntry>;

// ---------- paths ----------

/// `path` with `.` and `..` folded out, without touching the file system.
pub fn normalize(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

/// `target` as a path relative to `from_dir` (both absolute and normalised), `/`-separated; `None`
/// when they share no folder below the root (another drive, or only `/`).
pub fn relative_path(from_dir: &std::path::Path, target: &std::path::Path) -> Option<String> {
    use std::path::Component;
    let from: Vec<Component> = from_dir.components().collect();
    let to: Vec<Component> = target.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let rooted = from.iter().take(common).any(|c| matches!(c, Component::Normal(_)));
    if !rooted {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    for c in from.iter().skip(common) {
        match c {
            Component::Normal(_) => parts.push("..".into()),
            _ => return None,
        }
    }
    for c in to.iter().skip(common) {
        match c {
            Component::Normal(n) => parts.push(n.to_string_lossy().into_owned()),
            _ => return None,
        }
    }
    if parts.is_empty() { None } else { Some(parts.join("/")) }
}

/// The absolute path a linked reference points to: `stored` as it is when absolute, else
/// relative to `base` (the project file's folder). `None` for a relative path with no base.
pub fn resolve_path(stored: &str, base: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(stored);
    if p.is_absolute() { Some(normalize(p)) } else { base.map(|b| normalize(&b.join(p))) }
}

/// The folder of a project file path, absolute.
fn dir_of(path: &str) -> Option<std::path::PathBuf> {
    let abs = std::path::absolute(path).ok()?;
    abs.parent().map(normalize)
}

/// How linked reference `stored` is written for a project file in `new_dir`: relative when the
/// folders share a parent below the root, else absolute. `old_base` is where `stored` resolves now.
pub fn rebase_path(stored: &str, old_base: Option<&std::path::Path>, new_dir: &std::path::Path) -> String {
    match resolve_path(stored, old_base) {
        Some(abs) => relative_path(new_dir, &abs).unwrap_or_else(|| abs.to_string_lossy().replace('\\', "/")),
        None => stored.to_string(),
    }
}

// ---------- session ----------

impl Session {
    /// The folder linked references resolve against: the project file's.
    pub fn reference_base(&self) -> Option<std::path::PathBuf> {
        self.path.as_deref().and_then(dir_of)
    }

    /// The decoded image of reference `id`, if it loaded.
    pub fn reference_image(&self, id: u64) -> Option<Arc<RefImage>> {
        self.references.get(&id).and_then(|e| e.state.as_ref().ok().cloned())
    }

    /// Why reference `id` has no image.
    pub fn reference_error(&self, id: u64) -> Option<&RefError> {
        self.references.get(&id).and_then(|e| e.state.as_ref().err())
    }

    fn load_reference(&self, r: &Reference) -> RefEntry {
        match &r.source {
            ReferenceSource::Embedded { resource } => {
                let state = match self.doc.resources.get(resource) {
                    Some(bytes) => RefImage::from_bytes(bytes).map(Arc::new).map_err(RefError::Invalid),
                    None => Err(RefError::Missing("the copy stored in the project is missing".into())),
                };
                RefEntry { state, resolved: None }
            }
            ReferenceSource::Linked { path } => self.load_linked(path),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn load_linked(&self, stored: &str) -> RefEntry {
        if !self.read_linked_files {
            return RefEntry { state: Err(RefError::Missing(format!("{stored}: linked files are not read in this session"))), resolved: None };
        }
        let Some(abs) = resolve_path(stored, self.reference_base().as_deref()) else {
            return RefEntry { state: Err(RefError::Missing(format!("{stored}: save the project to find files next to it"))), resolved: None };
        };
        let state = match std::fs::metadata(&abs) {
            Err(e) => Err(RefError::Missing(format!("{}: {e}", abs.display()))),
            Ok(m) if m.len() > MAX_BYTES as u64 => Err(RefError::Invalid(format!("{} is larger than {} MB", abs.display(), MAX_BYTES >> 20))),
            Ok(_) => match std::fs::read(&abs) {
                Ok(bytes) => RefImage::from_bytes(&bytes).map(Arc::new).map_err(|e| RefError::Invalid(format!("{}: {e}", abs.display()))),
                Err(e) => Err(RefError::Missing(format!("{}: {e}", abs.display()))),
            },
        };
        RefEntry { state, resolved: Some(abs) }
    }

    #[cfg(target_arch = "wasm32")]
    fn load_linked(&self, stored: &str) -> RefEntry {
        RefEntry { state: Err(RefError::Missing(format!("{stored}: linked files can't be read in the browser"))), resolved: None }
    }

    /// Load the references that are not loaded yet and forget removed ones (after load, undo,
    /// redo, add, remove).
    pub fn sync_references(&mut self) {
        let ids: Vec<u64> = self.doc.project.references.iter().map(|r| r.id).collect();
        self.references.retain(|id, _| ids.contains(id));
        let missing: Vec<Reference> = self.doc.project.references.iter().filter(|r| !self.references.contains_key(&r.id)).cloned().collect();
        for r in missing {
            let entry = self.load_reference(&r);
            if let Err(e) = &entry.state {
                self.log.push(format!("reference {}: {}", r.name, e.message()));
            }
            self.references.insert(r.id, entry);
        }
    }

    fn reference_index(&self, cmd: &str, p: &Value) -> Result<usize> {
        let id = u64p(p, "id").ok_or_else(|| bad(cmd, "`id` is required"))?;
        self.doc.project.references.iter().position(|r| r.id == id).ok_or_else(|| bad(cmd, format!("no reference {id}")))
    }

    /// Add a reference from image bytes, embedded in the project (one undo step). Works
    /// everywhere, so drag and drop, paste and the browser use it.
    pub fn add_reference_bytes(&mut self, file_name: &str, bytes: Vec<u8>) -> Result<u64> {
        self.add_reference_from("reference.add", file_name, bytes, None)
    }

    /// Add the image file at `path`, embedded or linked.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn add_reference_file(&mut self, path: &str, linked: bool) -> Result<u64> {
        let meta = std::fs::metadata(path).map_err(|e| bad("reference.add", format!("cannot read {path}: {e}")))?;
        if meta.len() > MAX_BYTES as u64 {
            return Err(bad("reference.add", format!("{path} is larger than {} MB", MAX_BYTES >> 20)));
        }
        let bytes = std::fs::read(path).map_err(|e| bad("reference.add", format!("cannot read {path}: {e}")))?;
        let link = if linked {
            self.read_linked_files = true;
            let abs = std::path::absolute(path).map(|p| normalize(&p)).map_err(|e| bad("reference.add", format!("{path}: {e}")))?;
            let stored = match self.reference_base() {
                Some(base) => relative_path(&base, &abs).unwrap_or_else(|| abs.to_string_lossy().replace('\\', "/")),
                None => abs.to_string_lossy().replace('\\', "/"),
            };
            Some(stored)
        } else {
            None
        };
        self.add_reference_from("reference.add", path, bytes, link)
    }

    fn add_reference_from(&mut self, cmd: &str, file_name: &str, bytes: Vec<u8>, link: Option<String>) -> Result<u64> {
        if self.doc.project.references.len() >= MAX_REFERENCES {
            return Err(bad(cmd, format!("a project holds at most {MAX_REFERENCES} reference images")));
        }
        let image = RefImage::from_bytes(&bytes).map_err(|e| bad(cmd, format!("{file_name}: {e}")))?;
        let leaf = std::path::Path::new(file_name).file_name().and_then(|n| n.to_str()).unwrap_or("reference");
        let id = self.edit("Add reference", |s| {
            let id = s.doc.project.alloc_id().0;
            let source = match link {
                Some(path) => ReferenceSource::Linked { path },
                None => {
                    let resource = format!("{RESOURCE_PREFIX}{id}.{}", texturecraft_formats::sanitize_resource_name(leaf));
                    s.doc.resources.insert(resource.clone(), Arc::new(bytes));
                    ReferenceSource::Embedded { resource }
                }
            };
            s.doc.project.references.push(Reference { id, name: leaf.to_string(), source, flip_h: false, flip_v: false, grayscale: false });
            Ok(id)
        })?;
        let resolved = match self.doc.project.references.iter().find(|r| r.id == id).map(|r| r.source.clone()) {
            Some(ReferenceSource::Linked { path }) => resolve_path(&path, self.reference_base().as_deref()),
            _ => None,
        };
        self.references.insert(id, RefEntry { state: Ok(Arc::new(image)), resolved });
        Ok(id)
    }

    /// The linked references written for a project file in `new_dir` (see [`rebase_path`]).
    pub fn references_for_dir(&self, new_dir: &std::path::Path) -> Vec<Reference> {
        let base = self.reference_base();
        self.doc
            .project
            .references
            .iter()
            .map(|r| match &r.source {
                ReferenceSource::Linked { path } => {
                    Reference { source: ReferenceSource::Linked { path: rebase_path(path, base.as_deref(), new_dir) }, ..r.clone() }
                }
                _ => r.clone(),
            })
            .collect()
    }

    /// Adopt `refs` (from [`Session::references_for_dir`]) in the document and its history, so
    /// an undo after "Save as" in another folder keeps pointing at the same files.
    pub fn adopt_reference_paths(&mut self, refs: &[Reference]) {
        let apply = |project: &mut texturecraft_core::Project| {
            for r in &mut project.references {
                if let (ReferenceSource::Linked { .. }, Some(new)) = (&r.source, refs.iter().find(|n| n.id == r.id))
                    && let ReferenceSource::Linked { .. } = &new.source
                {
                    r.source = new.source.clone();
                }
            }
        };
        apply(&mut self.doc.project);
        for (_, d) in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            apply(&mut d.project);
        }
    }
}

// ---------- commands ----------

fn storage_linked(p: &Value) -> Result<bool> {
    match strp(p, "storage") {
        None | Some("embedded") => Ok(false),
        Some("linked") => Ok(true),
        Some(other) => Err(bad("reference.add", format!("unknown storage `{other}` (embedded or linked)"))),
    }
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let linked = storage_linked(p)?;
    let id = if let Some(resource) = strp(p, "resource") {
        if linked {
            return Err(bad("reference.add", "a resource is already stored in the project; use `path` to link a file"));
        }
        let bytes = s.doc.resources.get(resource).map(|b| b.as_ref().clone()).ok_or_else(|| bad("reference.add", format!("no resource `{resource}`")))?;
        s.add_reference_from("reference.add", resource, bytes, None)?
    } else if let Some(path) = strp(p, "path") {
        add_path(s, path, linked)?
    } else {
        return Err(bad("reference.add", "`path` or `resource` is required"));
    };
    if let Some(name) = strp(p, "name").filter(|n| !n.trim().is_empty()) {
        let name = name.trim().chars().take(120).collect::<String>();
        s.edit("Rename reference", |s| {
            if let Some(r) = s.doc.project.references.iter_mut().find(|r| r.id == id) {
                r.name = name;
            }
            Ok(())
        })?;
    }
    Ok(entry_json(s, id))
}

#[cfg(not(target_arch = "wasm32"))]
fn add_path(s: &mut Session, path: &str, linked: bool) -> Result<u64> {
    s.add_reference_file(path, linked)
}

#[cfg(target_arch = "wasm32")]
fn add_path(_: &mut Session, _: &str, _: bool) -> Result<u64> {
    Err(EngineError::Other("file paths are not available in the browser; drop or paste the image instead".into()))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let i = s.reference_index("reference.remove", p)?;
    s.edit("Remove reference", |s| {
        let r = s.doc.project.references.remove(i);
        if let ReferenceSource::Embedded { resource } = r.source {
            s.doc.resources.remove(&resource);
        }
        if s.doc.project.reference_overlay.reference == Some(r.id) {
            s.doc.project.reference_overlay.reference = None;
        }
        Ok(())
    })?;
    s.sync_references();
    Ok(Value::Null)
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let i = s.reference_index("reference.set", p)?;
    s.edit("Edit reference", |s| {
        let r = s.doc.project.references.get_mut(i).ok_or_else(|| EngineError::Other("reference vanished".into()))?;
        if let Some(name) = strp(p, "name").filter(|n| !n.trim().is_empty()) {
            r.name = name.trim().chars().take(120).collect();
        }
        if let Some(v) = boolp(p, "flipH") {
            r.flip_h = v;
        }
        if let Some(v) = boolp(p, "flipV") {
            r.flip_v = v;
        }
        if let Some(v) = boolp(p, "grayscale") {
            r.grayscale = v;
        }
        Ok(())
    })?;
    let id = s.doc.project.references.get(i).map(|r| r.id).unwrap_or_default();
    Ok(entry_json(s, id))
}

fn overlay(s: &mut Session, p: &Value) -> Result<Value> {
    let pinned = match p.get("reference") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(v) => {
            let id = v.as_u64().ok_or_else(|| bad("reference.overlay", "`reference` is a reference id or null"))?;
            if !s.doc.project.references.iter().any(|r| r.id == id) {
                return Err(bad("reference.overlay", format!("no reference {id}")));
            }
            Some(Some(id))
        }
    };
    let (opacity, x, y, scale) = (f32p(p, "opacity"), f32p(p, "x"), f32p(p, "y"), f32p(p, "scale"));
    s.edit("Reference overlay", |s| {
        let o = &mut s.doc.project.reference_overlay;
        if let Some(r) = pinned {
            o.reference = r;
        }
        if let Some(v) = opacity {
            o.opacity = v.clamp(0.0, 1.0);
        }
        if let Some(v) = x {
            o.center[0] = v.clamp(-1.0, 2.0);
        }
        if let Some(v) = y {
            o.center[1] = v.clamp(-1.0, 2.0);
        }
        if let Some(v) = scale {
            o.scale = v.clamp(0.02, 8.0);
        }
        Ok(())
    })?;
    Ok(json!(s.doc.project.reference_overlay))
}

fn pick(s: &mut Session, p: &Value) -> Result<Value> {
    let i = s.reference_index("reference.pick", p)?;
    let r = s.doc.project.references.get(i).ok_or_else(|| bad("reference.pick", "reference vanished"))?.clone();
    let image = s.reference_image(r.id).ok_or_else(|| {
        let why = s.reference_error(r.id).map(|e| e.message().to_string()).unwrap_or_default();
        bad("reference.pick", format!("reference {} has no image: {why}", r.id))
    })?;
    let (u, v) = (f32p(p, "u"), f32p(p, "v"));
    let (Some(u), Some(v)) = (u, v) else { return Err(bad("reference.pick", "`u` and `v` (0..1) are required")) };
    let radius = f32p(p, "radius").unwrap_or(0.0);
    let linear = image.pick(u, v, radius, RefView::of(&r)).ok_or_else(|| bad("reference.pick", "that pixel is transparent"))?;
    let apply = boolp(p, "apply").unwrap_or(true);
    if apply {
        set_brush_color(s, linear);
    }
    Ok(json!({ "linear": linear, "srgb": hex(linear), "applied": apply }))
}

/// Make `linear` the base colour of the brush (the material painted into Base color).
pub fn set_brush_color(s: &mut Session, linear: [f32; 3]) {
    s.tool.values.insert(texturecraft_core::Channel::BaseColor, linear.map(|x| x.clamp(0.0, 1.0)));
    s.tool.enabled.insert(texturecraft_core::Channel::BaseColor);
    s.revision = s.revision.wrapping_add(1);
}

/// `#rrggbb` of a linear colour.
pub fn hex(linear: [f32; 3]) -> String {
    let c = |x: f32| to_u8(linear_to_srgb(x.clamp(0.0, 1.0)));
    format!("#{:02x}{:02x}{:02x}", c(linear[0]), c(linear[1]), c(linear[2]))
}

fn reload(s: &mut Session, p: &Value) -> Result<Value> {
    match u64p(p, "id") {
        Some(id) => {
            if !s.doc.project.references.iter().any(|r| r.id == id) {
                return Err(bad("reference.reload", format!("no reference {id}")));
            }
            s.references.remove(&id);
        }
        None => s.references.clear(),
    }
    s.sync_references();
    Ok(list(s))
}

fn entry_json(s: &Session, id: u64) -> Value {
    let Some(r) = s.doc.project.references.iter().find(|r| r.id == id) else { return Value::Null };
    let mut v = json!({
        "id": r.id, "name": r.name, "flipH": r.flip_h, "flipV": r.flip_v, "grayscale": r.grayscale,
    });
    match &r.source {
        ReferenceSource::Embedded { .. } => v["storage"] = json!("embedded"),
        ReferenceSource::Linked { path } => {
            v["storage"] = json!("linked");
            v["path"] = json!(path);
        }
    }
    match s.references.get(&id).map(|e| &e.state) {
        Some(Ok(img)) => {
            v["status"] = json!("ok");
            v["width"] = json!(img.width);
            v["height"] = json!(img.height);
        }
        Some(Err(e)) => {
            v["status"] = json!(e.status());
            v["error"] = json!(e.message());
            v["width"] = json!(0);
            v["height"] = json!(0);
        }
        None => v["status"] = json!("missing"),
    }
    v
}

pub fn list(s: &Session) -> Value {
    json!({
        "references": s.doc.project.references.iter().map(|r| entry_json(s, r.id)).collect::<Vec<_>>(),
        "overlay": s.doc.project.reference_overlay,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba(f(x, y)));
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap_or_default();
        out
    }

    #[test]
    fn picking_decodes_srgb_to_linear() {
        let img = RefImage::from_bytes(&png(2, 1, |x, _| if x == 0 { [128, 128, 128, 255] } else { [255, 0, 0, 255] })).unwrap();
        let grey = img.pick(0.25, 0.5, 0.0, RefView::default()).unwrap();
        // sRGB 128 is 0.2158 linear, not 0.502.
        assert!((grey[0] - 0.2158).abs() < 1e-3, "{grey:?}");
        assert_eq!(hex(grey), "#808080");
        let red = img.pick(0.75, 0.5, 0.0, RefView::default()).unwrap();
        assert_eq!(red, [1.0, 0.0, 0.0]);
        // Flipping shows the other pixel at the same spot.
        let flipped = img.pick(0.25, 0.5, 0.0, RefView { flip_h: true, ..RefView::default() }).unwrap();
        assert_eq!(flipped, red);
    }

    #[test]
    fn averaging_happens_in_linear_light() {
        let img = RefImage::from_bytes(&png(2, 2, |x, y| if (x + y) % 2 == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] })).unwrap();
        let mean = img.pick(0.5, 0.5, 2.0, RefView::default()).unwrap();
        assert!((mean[0] - 0.5).abs() < 1e-4, "{mean:?}");
        // Which encodes to sRGB 188, not 128.
        assert_eq!(hex(mean), "#bcbcbc");
    }

    #[test]
    fn grayscale_picks_luminance_and_transparent_pixels_pick_nothing() {
        let img = RefImage::from_bytes(&png(2, 1, |x, _| if x == 0 { [0, 255, 0, 255] } else { [9, 9, 9, 0] })).unwrap();
        let g = img.pick(0.25, 0.5, 0.0, RefView { grayscale: true, ..RefView::default() }).unwrap();
        assert!((g[0] - 0.7152).abs() < 1e-4 && g[0] == g[1] && g[1] == g[2], "{g:?}");
        assert!(img.pick(0.75, 0.5, 0.0, RefView::default()).is_none());
        // Alpha weights an average: the transparent half does not darken it.
        let both = img.pick(0.5, 0.5, 1.0, RefView::default()).unwrap();
        assert_eq!(both, [0.0, 1.0, 0.0]);
        for bad in [f32::NAN, f32::INFINITY] {
            assert!(img.pick(bad, 0.5, 0.0, RefView::default()).is_none());
        }
        // Out-of-range positions clamp to the edge.
        assert!(img.pick(-5.0, 9.0, f32::NAN, RefView::default()).is_some());
    }

    #[test]
    fn display_copies_are_bounded_and_gray() {
        let img = RefImage::from_bytes(&png(8, 4, |_, _| [255, 0, 0, 255])).unwrap();
        let (w, h, px) = img.display_copy(4, false);
        assert_eq!((w, h, px.len()), (4, 2, 4 * 2 * 4));
        let (_, _, gray) = img.display_copy(8, true);
        assert!(gray.chunks(4).all(|p| p[0] == p[1] && p[1] == p[2] && p[0] > 0 && p[3] == 255));
    }

    #[test]
    fn corrupt_images_are_errors_not_panics() {
        let good = png(4, 4, |_, _| [1, 2, 3, 255]);
        assert!(RefImage::from_bytes(b"").is_err());
        assert!(RefImage::from_bytes(b"not an image at all").is_err());
        for cut in [1, 8, 20, good.len() / 2] {
            assert!(RefImage::from_bytes(good.get(..cut).unwrap_or_default()).is_err(), "cut at {cut}");
        }
        // A missing last byte may still decode; it must not panic.
        let _ = RefImage::from_bytes(good.get(..good.len() - 1).unwrap_or_default());
        let mut flipped = good.clone();
        for i in (8..flipped.len()).step_by(7) {
            flipped[i] ^= 0xA5;
        }
        let _ = RefImage::from_bytes(&flipped);
        // A header that claims a huge image is refused before anything is allocated.
        let mut huge = good.clone();
        huge[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(RefImage::from_bytes(&huge).is_err());
    }

    #[test]
    fn relative_paths_walk_up_and_down() {
        let rel = |from: &str, to: &str| relative_path(Path::new(from), Path::new(to));
        assert_eq!(rel("/work/proj", "/work/proj/refs/a.png").as_deref(), Some("refs/a.png"));
        assert_eq!(rel("/work/proj", "/work/photos/a.png").as_deref(), Some("../photos/a.png"));
        assert_eq!(rel("/work/proj", "/other/a.png"), None);
        assert_eq!(rel("/work/proj", "/work/proj"), None);
        assert_eq!(normalize(Path::new("/a/b/../c/./d")), Path::new("/a/c/d"));
        assert_eq!(resolve_path("../photos/a.png", Some(Path::new("/work/proj"))), Some("/work/photos/a.png".into()));
        assert_eq!(resolve_path("a.png", None), None);
        // An absolute path needs a drive on Windows
        let absolute = if cfg!(windows) { "C:/abs/a.png" } else { "/abs/a.png" };
        assert_eq!(resolve_path(absolute, None), Some(absolute.into()));
        // Saving elsewhere rewrites the path so it still finds the file.
        assert_eq!(rebase_path("refs/a.png", Some(Path::new("/work/proj")), Path::new("/work/other")), "../proj/refs/a.png");
    }
}
