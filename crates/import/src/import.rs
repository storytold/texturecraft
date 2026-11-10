//! [`import`]: one project file → an [`Imported`] TextureCraft project. Reads the mesh, binds texture sets, and for
//! each set reads the stored result (texture cache), the baked mesh maps and the layer stack, measures the stack
//! against the stored result (the safety net) and writes the import report.
//!
//! Nothing of the original file is kept in the project: no layer document, no mesh file, no graph packages. Bitmaps
//! the layers use become image resources of the project; the stored result becomes locked "Original result" layers.

use std::collections::{BTreeMap, BTreeSet};

use texturecraft_compose::{BakedMaps, ImageStore, PixelStore};
use texturecraft_core::{
    BlendMode, Channel, ChannelBlend, Fidelity, Fill, ImportReport, Layer, LayerKind, LayerReport, LayerRole, MAX_RESOLUTION, MIN_RESOLUTION, NormalFormat,
    Project, Source, TextureSet, import_report::original_result_layer,
};
use texturecraft_geom::Mesh;
use texturecraft_import_reader::decode::assbin::{AssbinGeometry, AssbinScene};
use texturecraft_import_reader::decode::document::{MaterialRecord, NormalMapSyscoord};
use texturecraft_import_reader::model::baking::{BAKING_INI, ProjectBakeSettings};
use texturecraft_import_reader::model::{ProjectDocument, ProjectTextureSet};
use texturecraft_import_reader::project::{ProjectFile, paths};

use crate::channels::{self, ChannelConfidence, SetChannels, project_channel_default};
use crate::error::ImportError;
use crate::image::Rgba;
use crate::layers::{self, ImportedStack, SetInput};
use crate::options::ImportOptions;
use crate::resources::{CacheImage, ProjectImages, read_cache};
use crate::safety_net::{self, AGREEMENT_STEPS, Score, SetScene, Strategy};
use crate::sets::{SetBinding, SetSource};
use crate::{bake_settings, baked, mesh};

/// Resolution for sets with neither a stored result nor a recorded resolution (the reference app's default).
const FALLBACK_RESOLUTION: u32 = 2048;
/// Name of the uniform fill holding the channel values under every layer.
pub const DEFAULTS_LAYER_NAME: &str = "Channel defaults";
/// What imported the project, for the report.
pub const IMPORTER: &str = concat!("texturecraft-import ", env!("CARGO_PKG_VERSION"));

/// An imported project: everything a `.tcraft` holds besides the mesh file, plus the mesh itself.
pub struct Imported {
    pub project: Project,
    pub pixels: PixelStore,
    /// Baked mesh maps by texture set index.
    pub baked: BTreeMap<usize, BakedMaps>,
    pub mesh: Mesh,
    /// Image resources by name: file bytes.
    pub resources: BTreeMap<String, Vec<u8>>,
    /// Per texture set: what the safety net found.
    pub sets: Vec<SetSummary>,
}

/// What the safety net found for one texture set.
#[derive(Debug, Clone, PartialEq)]
pub struct SetSummary {
    pub name: String,
    pub strategy: Strategy,
    /// Per channel the composite against the stored result (none when there is no stored result or no measuring).
    pub scores: Vec<(Channel, Score)>,
    /// Per channel the status by the layers' parts: exact, approximated or not computable.
    pub channels: Vec<(Channel, String)>,
    pub layers: usize,
    pub locked: usize,
    pub placeholders: usize,
}

/// Progress callback (fraction 0..1, step) and the cancel check the import asks between steps.
pub struct Hooks<'h> {
    pub progress: &'h mut dyn FnMut(f32, &str),
    pub cancel: &'h dyn Fn() -> bool,
}

impl Hooks<'_> {
    fn check(&self) -> Result<(), ImportError> {
        if (self.cancel)() { Err(ImportError::Cancelled) } else { Ok(()) }
    }
}

/// Imports the project file `bytes`; `file_name` names it in the report and the project.
pub fn import_bytes(bytes: Vec<u8>, file_name: &str, options: &ImportOptions, hooks: &mut Hooks) -> Result<Imported, ImportError> {
    let project = ProjectFile::from_bytes(bytes)?;
    import(&project, file_name, options, hooks)
}

/// Imports the project file at `path` (read lazily, so large files open quickly).
#[cfg(not(target_arch = "wasm32"))]
pub fn import_path(path: &std::path::Path, options: &ImportOptions, hooks: &mut Hooks) -> Result<Imported, ImportError> {
    let project = ProjectFile::open(path)?;
    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    import(&project, &name, options, hooks)
}

/// Imports an opened project file.
pub fn import(file: &ProjectFile, file_name: &str, options: &ImportOptions, hooks: &mut Hooks) -> Result<Imported, ImportError> {
    hooks.check()?;
    (hooks.progress)(0.0, "reading the mesh");
    let scene = file.read(paths::SCENE)?;
    let decode = |part| move |source| ImportError::Decode { part, source };
    let geometry = AssbinGeometry::parse(&scene).map_err(decode("mesh"))?;
    let materials = AssbinScene::parse(&scene).map_err(decode("mesh"))?.materials;
    let mut mesh = mesh::mesh_of(&geometry).map_err(ImportError::Invalid)?;
    let mut report = ImportReport::new(source_label(file, file_name), IMPORTER);
    report.add("mesh", Fidelity::Exact, format!("{} parts, {} triangles, at the file's normalised scale", mesh.submeshes.len(), mesh.triangle_count()));
    hooks.check()?;
    (hooks.progress)(0.05, "reading the layer document");
    let records = file.document_materials().unwrap_or_else(|e| {
        report.add("texture sets", Fidelity::Approximated, format!("the document's texture set list could not be read ({e}); sets named after the mesh"));
        Vec::new()
    });
    let settings = file.document_settings().ok();
    let syscoord = settings.as_ref().and_then(|s| s.normal_map_syscoord);
    let model = match file.model() {
        Ok((document, _)) => Some(document),
        Err(e) => {
            report.add("layer document", Fidelity::Approximated, format!("the layer document could not be read ({e}); stacks stay flattened"));
            None
        }
    };
    if syscoord == Some(NormalMapSyscoord::DirectX) {
        report.add("baked normal maps", Fidelity::Exact, "stored DirectX; green flipped to OpenGL");
    }
    if let Some(s) = &settings {
        let filter = if s.format_version <= 61 { "a Sobel filter" } else { "backward differences" };
        report.add(
            "height to normal",
            Fidelity::Approximated,
            format!("the original derives normals from height with {filter} (document format {}); TextureCraft uses its own", s.format_version),
        );
    }
    report.add("cameras", Fidelity::Approximated, "the file's saved views are not imported yet");
    let bake_ini = file.read(BAKING_INI).ok().and_then(|d| ProjectBakeSettings::parse(&d).ok());
    let cache_entries = file.cache_entries();
    let cache_sets: BTreeSet<String> = cache_entries.iter().map(|e| e.set.clone()).collect();
    let binding = SetBinding::bind(&materials, &records, &cache_sets);
    if binding.sets.is_empty() {
        return Err(ImportError::Invalid("the project has no texture set".into()));
    }
    let stem = std::path::Path::new(file_name).file_stem().map_or_else(|| "Imported".to_string(), |s| s.to_string_lossy().into_owned());
    let mut project = Project::new(stem, "mesh.glb");
    project.normal_format = if syscoord == Some(NormalMapSyscoord::DirectX) { NormalFormat::DirectX } else { NormalFormat::OpenGl };
    let images = ProjectImages::new(file);
    let mut imported = Imported {
        project: Project::new("", ""),
        pixels: PixelStore::default(),
        baked: BTreeMap::new(),
        mesh: Mesh::default(),
        resources: BTreeMap::new(),
        sets: Vec::new(),
    };
    for source in &binding.sets {
        mesh::merge_slots(&mut mesh, source.slot, &source.merged_slots);
        for m in &source.merged_slots {
            let from = materials.get(*m).map_or("?", |m| m.name.as_str());
            report.add(
                format!("mesh material \"{from}\""),
                Fidelity::Exact,
                format!("shares texture set \"{}\" (collapsed in the original)", source.record.name),
            );
        }
    }
    let count = binding.sets.len();
    for (i, source) in binding.sets.iter().enumerate() {
        hooks.check()?;
        (hooks.progress)(0.1 + 0.8 * i as f32 / count as f32, &format!("converting texture set {} of {count}", i + 1));
        let project_set = source.record.scene_uid.filter(|_| source.in_document).and_then(|uid| model.as_ref()?.texture_set(uid));
        let ctx = SetContext {
            file,
            source,
            project_set,
            mesh: &mesh,
            images: &images,
            options,
            syscoord,
            bake: bake_ini.as_ref(),
            cache: &cache_entries,
            hooks: &*hooks,
        };
        let summary = build_set(&ctx, &mut project, &mut imported, &mut report)?;
        imported.sets.push(summary);
    }
    for r in &binding.unused {
        report.add(
            format!("texture set \"{}\"", r.name),
            Fidelity::Approximated,
            "in the document but on no mesh material (left over from a replaced mesh); not imported",
        );
    }
    (hooks.progress)(0.95, "finishing the project");
    if let Some(document) = &model {
        report_unhandled_document(document, &mut report);
    }
    imported.resources = images.files();
    project.import = Some(report);
    imported.project = project;
    imported.mesh = mesh;
    (hooks.progress)(1.0, "imported");
    Ok(imported)
}

/// Document-wide settings the import does not handle (they apply to every set).
fn report_unhandled_document(document: &ProjectDocument, report: &mut ImportReport) {
    for s in document.unhandled.iter().filter(|s| s.may_change_pixels()) {
        report.add(format!("setting {}", s.key()), Fidelity::Approximated, s.reason());
    }
}

/// `<file name> (saved with version <version>)`.
fn source_label(file: &ProjectFile, file_name: &str) -> String {
    let version = file.settings().ok().flatten().and_then(|s| s.last_saved_with.or(s.created_with)).map_or("(version unknown)".to_string(), |v| v.to_string());
    format!("{file_name} (saved with version {version})")
}

struct SetContext<'a, 'h> {
    file: &'a ProjectFile,
    source: &'a SetSource,
    project_set: Option<&'a ProjectTextureSet>,
    mesh: &'a Mesh,
    images: &'a ProjectImages<'a>,
    options: &'a ImportOptions,
    syscoord: Option<NormalMapSyscoord>,
    bake: Option<&'a ProjectBakeSettings>,
    cache: &'a [texturecraft_import_reader::project::CacheEntryRef],
    hooks: &'a Hooks<'h>,
}

/// Builds one texture set into `project` and `out`.
fn build_set(ctx: &SetContext, project: &mut Project, out: &mut Imported, report: &mut ImportReport) -> Result<SetSummary, ImportError> {
    let record = &ctx.source.record;
    let name = record.name.clone();
    let item = |what: &str| format!("texture set \"{name}\" / {what}");
    // the stored result
    let mut cache: Vec<(Channel, CacheImage)> = Vec::new();
    match &ctx.source.cache_set {
        None => report.add(item("stored result"), Fidelity::Approximated, "no stored result (the original never rendered this set)"),
        Some(folder) => {
            let mut entries: Vec<_> = ctx.cache.iter().filter(|e| &e.set == folder).collect();
            entries.sort_by_key(|e| e.channel_bit);
            for e in entries {
                ctx.hooks.check()?;
                let label = channels::project_channel_name(e.channel_bit);
                let Some(channel) = channels::channel_for_bit(e.channel_bit) else {
                    report.add(
                        item(&format!("channel {label}")),
                        Fidelity::Approximated,
                        "no matching channel in TextureCraft; its stored result is not imported",
                    );
                    continue;
                };
                match read_cache(ctx.file, e) {
                    Ok(img) => cache.push((channel, img)),
                    Err(err) => report.add(item(&format!("channel {label}")), Fidelity::Approximated, format!("unreadable stored result: {err}")),
                }
            }
        }
    }
    let resolution = resolve_resolution(record, &cache, ctx.options, &name, report);
    for (_, img) in cache.iter_mut() {
        if (img.image.width, img.image.height) != (resolution, resolution) {
            img.image = resample(&img.image, resolution);
        }
    }
    let mut set = TextureSet::new(name.clone(), ctx.source.slot, resolution);
    let mut declared = SetChannels::default();
    for c in &record.channels {
        match channels::channel_for_bit(c.type_bit) {
            Some(channel) => declared.declare(channel, channels::channel_format(channel, Some(c.format))),
            None => {
                let label = if c.user_name.is_empty() {
                    channels::project_channel_name(c.type_bit)
                } else {
                    format!("{} (\"{}\")", channels::project_channel_name(c.type_bit), c.user_name)
                };
                let note = match channels::is_user_channel(c.type_bit) || !c.user_name.is_empty() {
                    true => "a user channel: TextureCraft has no user channels yet; not imported",
                    false => "no matching channel in TextureCraft; not imported",
                };
                report.add(item(&format!("channel {label}")), Fidelity::Approximated, note);
            }
        }
    }
    for (channel, _) in &cache {
        declared.declare(*channel, texturecraft_core::ChannelFormat::default_for(*channel));
    }
    // The original's sets always have a normal channel (it is not always declared).
    declared.declare(Channel::Normal, channels::channel_format(Channel::Normal, None));
    if declared.declared().is_empty() {
        for c in Channel::DEFAULT_SET {
            declared.declare(c, texturecraft_core::ChannelFormat::default_for(c));
        }
    }
    for d in declared.declared() {
        if channels::confidence(d.channel) == ChannelConfidence::Named {
            report.add(
                item(&format!("channel {}", declared.label(d.channel))),
                Fidelity::Approximated,
                format!("mapped to {} by name (not measured)", d.channel.label()),
            );
        }
    }
    set.channels = declared.declared().iter().map(|d| (d.channel, d.format)).collect();
    let ts = project.texture_sets.len();
    // baked mesh maps
    let mut maps = BTreeMap::new();
    for b in baked::sources(ctx.file, &name, &record.baked) {
        ctx.hooks.check()?;
        let Some(kind) = b.kind else {
            report.add(item(&format!("baked map \"{}\"", b.name)), Fidelity::Approximated, format!("usage {:?} has no counterpart; not imported", b.usage));
            continue;
        };
        match baked::read(ctx.file, &b, kind, resolution, ctx.syscoord) {
            Ok((values, missing)) => {
                let fidelity = if missing > 0 { Fidelity::Approximated } else { Fidelity::Exact };
                let note = if missing > 0 { format!("stored data {missing} bytes short (zeros)") } else { String::new() };
                report.add(item(&format!("baked map \"{}\"", b.name)), fidelity, note);
                maps.insert(kind, values);
                set.baked.insert(kind);
            }
            Err(e) => report.add(item(&format!("baked map \"{}\"", b.name)), Fidelity::Approximated, format!("unreadable: {e}")),
        }
    }
    let baked = baked::maps(resolution, maps);
    // bake settings
    let uid = ctx.project_set.and_then(|s| s.uid);
    let bake = ctx.bake.and_then(|b| b.for_material(uid));
    let (bake_settings, unmapped) = bake_settings::map(bake, resolution);
    set.bake = bake_settings;
    if !unmapped.is_empty() {
        report.add(item("bake settings"), Fidelity::Approximated, format!("not taken over: {}", unmapped.join("; ")));
    }
    project.texture_sets.push(set);
    // the layer stack
    let parts: Vec<String> = ctx.mesh.submeshes.iter().filter(|s| s.material == ctx.source.slot).map(|s| s.name.clone()).collect();
    let stack = match ctx.project_set {
        Some(project_set) => {
            let input = SetInput { channels: &declared, images: ctx.images, mesh_maps: &project_set.mesh_maps, parts: &parts, options: ctx.options };
            Some(layers::import(project_set, &input, project.next_id))
        }
        None => {
            report.add(
                item("layer stack"),
                Fidelity::Baked,
                format!("not in the layer document; {} layers kept only as the stored result", record.layer_count),
            );
            None
        }
    };
    let (exact, layer_count) = match &stack {
        Some(s) => (s.verdict.all_exact(), s.verdict.layer_count),
        None => (false, 0),
    };
    if let Some(stack) = &stack {
        insert_stack(project, ts, stack, report);
    }
    // channel defaults under every layer, unless a full uniform fill covers each channel
    let defaults_id = (!provides_base(project, ts, &declared)).then(|| {
        let id = project.alloc_id();
        let mut fill = Fill::default();
        for c in declared.all() {
            fill.sources.insert(c, Source::Uniform(project_channel_default(c)));
        }
        let channels: Vec<Channel> = declared.all().collect();
        let layer = Layer::new(id, DEFAULTS_LAYER_NAME, LayerKind::Fill(fill), &channels);
        project.layers.insert(id, layer);
        if let Ok(set) = project.texture_set_mut(ts) {
            set.stack.insert(0, id);
        }
        report.layer_mut(id).role = LayerRole::ChannelDefaults;
        report.layer_mut(id).fidelity = Fidelity::Approximated;
        id
    });
    if defaults_id.is_some() {
        report.add(item(DEFAULTS_LAYER_NAME), Fidelity::Approximated, "uncovered texels show the original's channel defaults (uniform fill layer)");
    }
    // the safety net
    let cache_channels: Vec<Channel> = cache.iter().map(|(c, _)| *c).collect();
    let mut scores = Vec::new();
    if ctx.options.measure && !cache.is_empty() && stack.is_some() {
        ctx.hooks.check()?;
        let store: ImageStore = ctx.images.store();
        let scene = SetScene { project, ts, mesh: ctx.mesh, pixels: &out.pixels, baked: &baked, images: &store };
        if let Some((composite, gbuf)) = safety_net::composite(&scene, &cache_channels) {
            for (c, img) in &cache {
                scores.push((*c, safety_net::score(&composite, &gbuf, *c, img)));
            }
        }
    }
    let agrees = scores.iter().all(|(_, s)| s.mean <= AGREEMENT_STEPS);
    let strategy = Strategy::choose(exact, !cache.is_empty(), agrees);
    let note = scores.iter().map(|(c, s)| format!("{} mean {:.2} max {:.1}", declared.label(*c), s.mean, s.max)).collect::<Vec<_>>().join(", ");
    let fidelity = match strategy {
        Strategy::RealStack => Fidelity::Exact,
        Strategy::Disagrees | Strategy::NotReproduced => Fidelity::Approximated,
        Strategy::BestEffort => Fidelity::Approximated,
    };
    if stack.is_some() {
        let measured = if note.is_empty() { "not measured".to_string() } else { format!("against the stored result: {note}") };
        report.add(item("layer stack"), fidelity, format!("{layer_count} layers; {}; {measured}", strategy.describe()));
    }
    if let Some(s) = &stack {
        for (_, path, why) in &s.verdict.mask_failures {
            report.add(
                item(&format!("layer \"{path}\"")),
                Fidelity::Approximated,
                format!("mask not computable yet ({}); layer hidden so it doesn't cover the whole set", why.join("; ")),
            );
        }
        report_blockers(&s.layers, &|what: &str| item(what), report);
    }
    if let Some(project_set) = ctx.project_set {
        report_strokes(project_set, &item("strokes"), report);
    }
    // the stored result as a locked layer: the reference, or the result on top
    if !cache.is_empty() {
        let shown = ctx.options.original_on_top && matches!(strategy, Strategy::NotReproduced | Strategy::Disagrees) || stack.is_none();
        let id = project.alloc_id();
        let layer = original_result_layer(id, &cache_channels, shown);
        for (c, img) in &cache {
            out.pixels.layers.insert((id, *c), img.image.layer_tiles(*c));
            let missing = img.missing_bytes;
            let note =
                if missing > 0 { format!("the original's stored result, {missing} bytes short (zeros)") } else { "the original's stored result".to_string() };
            report.add(item(&format!("channel {}", declared.label(*c))), Fidelity::Baked, note);
        }
        project.layers.insert(id, layer);
        if let Ok(set) = project.texture_set_mut(ts) {
            set.stack.push(id);
        }
        let r = report.layer_mut(id);
        r.role = LayerRole::OriginalResult;
        r.fidelity = Fidelity::Baked;
    }
    out.baked.insert(ts, baked);
    let (locked, placeholders) =
        stack.as_ref().map_or((0, 0), |s| (s.layers.iter().filter(|l| l.is_locked()).count(), s.layers.iter().map(|l| l.placeholders().count()).sum()));
    Ok(SetSummary {
        name,
        strategy,
        scores,
        channels: stack.as_ref().map_or_else(Vec::new, |s| s.verdict.channels.iter().map(|(c, st)| (*c, status_word(st).to_string())).collect()),
        layers: layer_count,
        locked,
        placeholders,
    })
}

/// One line per blocker of the set: what it is, how many layers it locks and how many by it alone.
fn report_blockers(layers: &[Layer], item: &dyn Fn(&str) -> String, report: &mut ImportReport) {
    let mut by_key: BTreeMap<String, (String, usize, usize)> = BTreeMap::new();
    for l in layers {
        let Some(lock) = &l.lock else { continue };
        let keys = lock.keys();
        for k in &keys {
            let text = lock.reasons.iter().find(|r| r.key == *k).map_or(String::new(), |r| r.text.clone());
            let e = by_key.entry(k.to_string()).or_insert((text, 0, 0));
            e.1 += 1;
            if keys.len() == 1 {
                e.2 += 1;
            }
        }
    }
    for (key, (text, n, alone)) in by_key {
        let unlock = match key.split_once(':').map(|(k, _)| k) {
            Some("graph") => "stays a placeholder until compiled graphs are computed",
            Some("strokes") => "the pixels are in the original's stored result until stroke replay",
            _ => "not supported yet",
        };
        report.add(item(&format!("blocker {key}")), Fidelity::Baked, format!("{text}: locks {n} layers ({alone} by it alone); {unlock}"));
    }
}

/// The brush strokes of a set: none is replayed in this version, so their pixels are only in the stored result.
fn report_strokes(set: &ProjectTextureSet, item: &str, report: &mut ImportReport) {
    fn count(effects: &[texturecraft_import_reader::model::ProjectEffect], strokes: &mut usize, paints: &mut usize) {
        for e in effects {
            match e {
                texturecraft_import_reader::model::ProjectEffect::Paint(p) if p.stroke_count() > 0 => {
                    *strokes += p.stroke_count();
                    *paints += 1;
                }
                texturecraft_import_reader::model::ProjectEffect::Group(g) => count(&g.effects, strokes, paints),
                _ => {}
            }
        }
    }
    let (mut strokes, mut paints) = (0, 0);
    for l in set.all_layers() {
        count(&l.content, &mut strokes, &mut paints);
        if let Some(m) = &l.mask {
            count(&m.effects, &mut strokes, &mut paints);
        }
    }
    if strokes > 0 {
        report.add(
            item,
            Fidelity::Baked,
            format!("{strokes} strokes in {paints} paint effects not replayed; their pixels are only in the original's stored result"),
        );
    }
}

fn status_word(s: &crate::status::Status) -> &'static str {
    match s {
        crate::status::Status::Exact => "exact",
        crate::status::Status::Approximated(_) => "approximated",
        crate::status::Status::NotComputable(_) => "not computable",
    }
}

/// Puts the imported layers into the project and their reports into the import report.
fn insert_stack(project: &mut Project, ts: usize, stack: &ImportedStack, report: &mut ImportReport) {
    for l in &stack.layers {
        project.layers.insert(l.id, l.clone());
    }
    project.next_id = project.next_id.max(stack.next_id);
    if let Ok(set) = project.texture_set_mut(ts) {
        set.stack = stack.roots.clone();
    }
    for (id, r) in &stack.reports {
        report.layers.insert(*id, LayerReport { role: r.role, fidelity: r.fidelity, path: r.path.clone(), flags: r.flags.clone() });
    }
}

/// True if every declared channel is covered by a visible, unmasked top-level uniform fill at normal or replace and
/// full opacity.
fn provides_base(project: &Project, ts: usize, channels: &SetChannels) -> bool {
    let Ok(set) = project.texture_set(ts) else { return false };
    channels.all().all(|c| {
        set.stack.iter().filter_map(|id| project.layers.get(id)).any(|l| {
            let full = l.blend(c).is_some_and(|b: ChannelBlend| matches!(b.mode, BlendMode::Normal | BlendMode::Replace) && b.opacity >= 1.0);
            let value = matches!(&l.kind, LayerKind::Fill(f) if matches!(f.sources.get(&c), Some(Source::Uniform(_))));
            let unmasked = l.mask.as_ref().is_none_or(|m| !m.enabled || (m.effects.is_empty() && m.background == texturecraft_core::MaskBackground::White));
            l.visible && unmasked && full && value && l.content_effects.is_empty() && l.geometry_mask.is_none()
        })
    })
}

/// The stored result's size; else the document's; else the fallback; a power of two within TextureCraft's limits and the
/// options' cap.
fn resolve_resolution(record: &MaterialRecord, cache: &[(Channel, CacheImage)], options: &ImportOptions, name: &str, report: &mut ImportReport) -> u32 {
    let recorded = record.resolution_log2.filter(|r| r.iter().all(|v| (0..=14).contains(v))).map(|[w, h]| (1u32 << w).max(1u32 << h));
    let cached = cache.first().map(|(_, i)| i.image.width.max(i.image.height));
    let mut r = cached.or(recorded).unwrap_or(FALLBACK_RESOLUTION);
    let mut notes = Vec::new();
    if cached.is_none() && recorded.is_none() {
        notes.push(format!("no resolution stored; {FALLBACK_RESOLUTION} used"));
    }
    if let [w, h] = record.resolution_log2.unwrap_or([0, 0])
        && w != h
    {
        notes.push(format!("not square ({}x{}); imported square", 1u64 << w.clamp(0, 30), 1u64 << h.clamp(0, 30)));
    }
    if !r.is_power_of_two() {
        r = r.next_power_of_two();
        notes.push("not a power of two; rounded up".into());
    }
    let cap = options.max_resolution.unwrap_or(MAX_RESOLUTION).clamp(MIN_RESOLUTION, MAX_RESOLUTION);
    if r > cap {
        notes.push(format!("{r} reduced to {cap}"));
        r = cap;
    }
    let r = r.clamp(MIN_RESOLUTION, MAX_RESOLUTION);
    let fidelity = if notes.is_empty() { Fidelity::Exact } else { Fidelity::Approximated };
    report.add(
        format!("texture set \"{name}\""),
        fidelity,
        format!("resolution {r}{}", if notes.is_empty() { String::new() } else { format!(" ({})", notes.join("; ")) }),
    );
    r
}

/// `image` at `size` square: box-averaged when smaller (coverage-weighted), nearest when larger.
fn resample(image: &Rgba, size: u32) -> Rgba {
    let (sw, sh) = (image.width.max(1) as usize, image.height.max(1) as usize);
    let n = size.max(1) as usize;
    let mut px = vec![[0.0f32; 4]; n * n];
    for (i, p) in px.iter_mut().enumerate() {
        let (x, y) = (i % n, i / n);
        let (x0, x1) = (x * sw / n, ((x + 1) * sw / n).max(x * sw / n + 1).min(sw));
        let (y0, y1) = (y * sh / n, ((y + 1) * sh / n).max(y * sh / n + 1).min(sh));
        let (mut acc, mut cov, mut count) = ([0.0f32; 3], 0.0f32, 0.0f32);
        for sy in y0..y1 {
            for sx in x0..x1 {
                let s = image.get(sx as u32, sy as u32);
                for k in 0..3 {
                    acc[k] += s[k] * s[3];
                }
                cov += s[3];
                count += 1.0;
            }
        }
        if cov > 0.0 {
            *p = [acc[0] / cov, acc[1] / cov, acc[2] / cov, cov / count.max(1.0)];
        }
    }
    Rgba { width: size, height: size, px }
}
