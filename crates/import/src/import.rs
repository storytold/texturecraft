//! [`import`]: one project file → an [`Imported`] TextureCraft project. Reads the mesh, binds texture sets, and for
//! each set reads the stored result (texture cache), the baked mesh maps and the layer stack, measures the stack
//! against the stored result (the safety net) and writes the import report.
//!
//! Nothing of the original file is kept in the project: no layer document, no mesh file, no graph packages. Bitmaps
//! the layers use become image resources of the project; the stored result becomes locked "Original result" layers.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use texturecraft_compose::graphs::Cancel;
use texturecraft_compose::{BakedMaps, Composite, GraphStore, ImageStore, PixelStore};
use texturecraft_core::{
    BlendMode, Channel, ChannelBlend, CompositeRules, Fidelity, Fill, HeightNormal, ImportReport, Layer, LayerId, LayerKind, LayerReport, LayerRole, Lock,
    LockReason, MAX_RESOLUTION, MIN_RESOLUTION, NormalFormat, Project, Source, StoredStrokes, TextureSet, import_report::original_result_layer,
};
use texturecraft_geom::{Mesh, UvGBuffer};
use texturecraft_import_reader::decode::assbin::{AssbinGeometry, AssbinScene, Normalization};
use texturecraft_import_reader::decode::document::{MaterialRecord, NormalMapSyscoord};
use texturecraft_import_reader::model::baking::{BAKING_INI, ProjectBakeSettings};
use texturecraft_import_reader::model::cameras::{CAMERA_INI, SavedCameras};
use texturecraft_import_reader::model::{ProjectDocument, ProjectTextureSet};
use texturecraft_import_reader::project::{CacheEntryRef, ProjectFile, paths};

use crate::channels::{self, ChannelConfidence, SetChannels, uncovered_default};
use crate::correction;
use crate::error::ImportError;
use crate::graph_library::GraphLibrary;
use crate::image::Rgba;
use crate::layers::{self, ImportedStack, SetInput};
use crate::options::ImportOptions;
use crate::polygon_fill::FillJob;
use crate::polygon_fill::geometry::SetGeometry;
use crate::progress::{WATCH_PERIOD, share, watching};
use crate::replay::{ReplayJob, ReplayScene, ReplayTarget, ReplayedPixels, StrokeReplayer};
use crate::resources::{CacheImage, ProjectImages, read_cache};
use crate::safety_net::{self, AGREEMENT_STEPS, Score, SetScene, Strategy};
use crate::sets::{SetBinding, SetSource};
use crate::status::Status;
use crate::{bake_settings, baked, mesh, uv_tiles};

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
    /// The texts of the graphs the layers run, and their cooked outputs.
    pub graphs: GraphStore,
    /// Per texture set: what the safety net found.
    pub sets: Vec<SetSummary>,
    /// The stroke records of the replay jobs, by the id of the pixels they paint.
    pub strokes: BTreeMap<LayerId, StoredStrokes>,
}

/// A reduced, view-only material computed from the file's saved texture channels.
pub struct StoredPreview {
    pub mesh: Mesh,
    /// Material slots for all sets, in the same order as the import.
    pub materials: Vec<usize>,
    /// The set whose saved channels are ready. The remaining sets use neutral textures.
    pub texture_set: usize,
    pub camera: Option<texturecraft_core::Camera>,
    pub normal_format: NormalFormat,
    pub composite: Composite,
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

/// What an import is doing, for a progress display. Texture sets count from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Reading the mesh, the layer document, the settings and the compiled graphs.
    ReadingFile,
    /// Building texture set `set` of `of`: its stored result, baked maps and layer stack.
    ConvertingSet { set: usize, of: usize },
    /// Laying set `set`'s brush strokes with the replayer.
    ReplayingStrokes { set: usize, of: usize },
    /// Rendering set `set`'s graphs (and measuring its stack against the stored result).
    CookingGraphs { set: usize, of: usize },
    /// Putting the converted project together (and, in the engine, storing it as a project).
    Saving,
    /// Compositing the opened project's view once (a front end that shows the project when the import finishes).
    ComputingView,
}

impl Step {
    /// A stable id: `reading`, `converting`, `replaying`, `cooking`, `saving` or `view`.
    pub fn id(&self) -> &'static str {
        match self {
            Step::ReadingFile => "reading",
            Step::ConvertingSet { .. } => "converting",
            Step::ReplayingStrokes { .. } => "replaying",
            Step::CookingGraphs { .. } => "cooking",
            Step::Saving => "saving",
            Step::ComputingView => "view",
        }
    }

    /// The texture set (from 1) and the set count, for the per-set steps.
    pub fn set(&self) -> Option<(usize, usize)> {
        match *self {
            Step::ConvertingSet { set, of } | Step::ReplayingStrokes { set, of } | Step::CookingGraphs { set, of } => Some((set, of)),
            Step::ReadingFile | Step::Saving | Step::ComputingView => None,
        }
    }
}

impl std::fmt::Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Step::ReadingFile => f.write_str("reading the file"),
            Step::ConvertingSet { set, of } => write!(f, "converting texture set {set} of {of}"),
            Step::ReplayingStrokes { set, of } => write!(f, "replaying strokes (texture set {set} of {of})"),
            Step::CookingGraphs { set, of } => write!(f, "cooking graphs (texture set {set} of {of})"),
            Step::Saving => f.write_str("saving"),
            Step::ComputingView => f.write_str("computing the view"),
        }
    }
}

/// Progress callback (fraction 0..1, step), the cancel flag and the stroke replayer (the brush engine the import lays
/// strokes with; `None`: strokes stay placeholders).
///
/// The import checks the flag between steps, texture sets and stroke and graph passes; the imported project's graph
/// store renders under the same flag, so a graph render in progress stops at its next node. A replayer that should stop
/// early reads the flag itself.
pub struct Hooks<'h> {
    pub progress: &'h (dyn Fn(f32, Step) + Sync),
    pub cancel: &'h Cancel,
    pub replay: Option<&'h dyn StrokeReplayer>,
    /// Where graph cooks are also kept between imports (a folder on the desktop; `None`: render every one).
    pub kept_cooks: Option<std::sync::Arc<dyn texturecraft_compose::graphs::KeptCooks>>,
    /// Live diagnostics from graph evaluation, scoped to this import.
    pub trace: Option<texturecraft_compose::graphs::CookTrace>,
    pub accelerator: Option<std::sync::Arc<dyn texturecraft_compose::graphs::GraphAccelerator>>,
    /// Publishes the saved material before translating the editable stack.
    pub preview: Option<&'h (dyn Fn(StoredPreview) + Sync)>,
}

impl Hooks<'_> {
    fn check(&self) -> Result<(), ImportError> {
        if self.cancel.is_cancelled() { Err(ImportError::Cancelled) } else { Ok(()) }
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
    (hooks.progress)(0.0, Step::ReadingFile);
    let scene = file.read(paths::SCENE)?;
    let decode = |part| move |source| ImportError::Decode { part, source };
    let geometry = AssbinGeometry::parse(&scene).map_err(decode("mesh"))?;
    let materials = AssbinScene::parse(&scene).map_err(decode("mesh"))?.materials;
    let mut mesh = mesh::mesh_of(&geometry).map_err(ImportError::Invalid)?;
    // per submesh (one per part, in order): the source polygons, for polygon-fill picks
    let polygon_starts: Vec<Vec<bool>> = geometry.parts.iter().map(|p| p.polygon_starts.clone()).collect();
    let mut report = ImportReport::new(source_label(file, file_name), IMPORTER);
    report.add("mesh", Fidelity::Exact, format!("{} parts, {} triangles, at the file's normalised scale", mesh.submeshes.len(), mesh.triangle_count()));
    hooks.check()?;
    (hooks.progress)(0.05, Step::ReadingFile);
    let records = file.document_materials().unwrap_or_else(|e| {
        report.add("texture sets", Fidelity::Approximated, format!("the document's texture set list could not be read ({e}); sets named after the mesh"));
        Vec::new()
    });
    let settings = file.document_settings().ok();
    let syscoord = settings.as_ref().and_then(|s| s.normal_map_syscoord);
    let mut project_cameras = Vec::new();
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
    // the rules measured from the original's renders, with the height filter of the file's document format
    let height_normal = settings.as_ref().map_or(HeightNormal::Backward, |s| HeightNormal::for_document_format(s.format_version));
    let rules = CompositeRules::measured(height_normal);
    report.add("blending", Fidelity::Exact, "the project follows the blend rules measured from the original (set in the project's compositing rules)");
    match &settings {
        Some(s) => {
            let filter = if height_normal == HeightNormal::Sobel { "a Sobel filter" } else { "backward differences" };
            report.add(
                "height to normal",
                Fidelity::Exact,
                format!("exported normals derive height with {filter} at the original's slope (document format {})", s.format_version),
            );
        }
        None => {
            report.add("height to normal", Fidelity::Approximated, "the document format is unknown: exported normals derive height with backward differences")
        }
    }
    let saved = file.read(CAMERA_INI).ok().map(|d| SavedCameras::parse(&d));
    let (lo, hi) = mesh.bounds();
    match saved {
        Some(Ok(saved)) => {
            let imported = crate::cameras::cameras(&saved, (lo + hi) * 0.5);
            let note = format!(
                "{} imported (the saved view as \"{}\"); depth of field and display settings not taken over",
                imported.len(),
                crate::cameras::SAVED_VIEW
            );
            report.add("cameras", Fidelity::Approximated, note);
            project_cameras = imported;
        }
        Some(Err(e)) => report.add("cameras", Fidelity::Approximated, format!("the saved cameras could not be read ({e})")),
        None => report.add("cameras", Fidelity::Exact, "none saved"),
    }
    let bake_ini = file.read(BAKING_INI).ok().and_then(|d| ProjectBakeSettings::parse(&d).ok());
    let normalization = file.mesh_normalization(&scene).ok().flatten();
    let cache_entries = file.cache_entries();
    let cache_sets: BTreeSet<String> = cache_entries.iter().map(|e| e.set.clone()).collect();
    let used_slots: BTreeSet<usize> = mesh.submeshes.iter().filter(|part| part.count >= 3).map(|part| part.material).collect();
    let binding = SetBinding::bind_slots(&materials, &records, &cache_sets, |slot| used_slots.contains(&slot));
    if binding.sets.is_empty() {
        return Err(ImportError::Invalid("the project has no texture set".into()));
    }
    for (slot, material) in materials.iter().enumerate().filter(|(slot, _)| !used_slots.contains(slot)) {
        report.add(
            format!("mesh material slot {} ({:?})", slot + 1, material.name),
            Fidelity::Approximated,
            "no triangles reference this slot, so it was omitted",
        );
    }
    let stem = std::path::Path::new(file_name).file_stem().map_or_else(|| "Imported".to_string(), |s| s.to_string_lossy().into_owned());
    let mut project = Project::new(stem, "mesh.glb");
    project.rules = rules;
    project.cameras = project_cameras;
    project.normal_format = if syscoord == Some(NormalMapSyscoord::DirectX) { NormalFormat::DirectX } else { NormalFormat::OpenGl };
    let images = ProjectImages::new(file);
    let mut imported = Imported {
        project: Project::new("", ""),
        pixels: PixelStore::default(),
        baked: BTreeMap::new(),
        mesh: Mesh::default(),
        resources: BTreeMap::new(),
        graphs: GraphStore::with_cancel(hooks.cancel.clone()),
        sets: Vec::new(),
        strokes: BTreeMap::new(),
    };
    imported.graphs.keep_cooks_in(hooks.kept_cooks.clone());
    imported.graphs.trace_cooks(hooks.trace.clone());
    imported.graphs.set_accelerator(hooks.accelerator.clone());
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
    let sets = || {
        binding.sets.iter().enumerate().map(|(i, source)| {
            let span = (0.1 + 0.8 * i as f32 / count as f32, 0.8 / count as f32);
            let project_set = source.record.scene_uid.filter(|_| source.in_document).and_then(|uid| model.as_ref()?.texture_set(uid));
            SetContext {
                file,
                source,
                project_set,
                mesh: &mesh,
                polygon_starts: &polygon_starts,
                images: &images,
                options,
                syscoord,
                bake: bake_ini.as_ref(),
                normalization,
                cache: &cache_entries,
                hooks: &*hooks,
                graphs: None,
                index: (i + 1, count),
                span,
            }
        })
    };
    let mut preview = hooks.preview.filter(|_| !options.translate_only).map(|publish| {
        let mesh = &mesh;
        let materials = binding.sets.iter().map(|source| source.slot).collect();
        let camera = project.cameras.iter().find(|camera| camera.name == crate::cameras::SAVED_VIEW).cloned();
        let normal_format = project.normal_format;
        move |texture_set, composite| publish(StoredPreview { mesh: mesh.clone(), materials, texture_set, camera, normal_format, composite })
    });
    let mut early = None;
    if preview.is_some()
        && let Some(ctx) = sets().find(|ctx| ctx.source.cache_set.is_some() && ctx.project_set.is_none_or(|set| !set.uses_uv_tiles))
    {
        let i = ctx.index.0 - 1;
        let mut stored = read_stored_channels(&ctx, &mut report, true)?;
        if let Some(composite) = stored.composite.take()
            && let Some(publish) = preview.take()
        {
            publish(i, composite);
        }
        early = Some((i, stored));
    }
    if options.graphs == crate::GraphMode::Decompile {
        (hooks.progress)(0.08, Step::ReadingFile);
    }
    let library = graph_library(file, options, hooks, &mut report)?;
    // the finished sets' layer pixels, out of memory until the import returns
    let mut finished = crate::pixel_spill::PixelSpill::default();
    let mut finished_baked = crate::pixel_spill::BakedSpill::default();
    for (i, mut ctx) in sets().enumerate() {
        hooks.check()?;
        (hooks.progress)(ctx.span.0, Step::ConvertingSet { set: i + 1, of: count });
        let StoredChannels { cache, resolution, composite } = match early.take_if(|(index, _)| *index == i) {
            Some((_, stored)) => stored,
            None => read_stored_channels(&ctx, &mut report, preview.is_some())?,
        };
        if let Some(composite) = composite
            && let Some(publish) = preview.take()
        {
            publish(i, composite);
        }
        ctx.graphs = library.as_ref();
        let summary = build_set(&ctx, &mut project, &mut imported, &mut report, cache, resolution)?;
        imported.sets.push(summary);
        // the set's cooks out of memory (read back when a composite or the save needs them), so that the sets do not
        // all hold theirs at once; the last set's stay, the project opens with them
        if i + 1 < count {
            imported.graphs.spill_cooks();
            finished.spill(&mut imported.pixels);
            finished_baked.spill(&mut imported.baked);
        }
        // and its decoded images: a later set that uses one reads it again
        images.release();
    }
    // read from the spill file when needed (a save of the import writes them one by one)
    finished.hand_over(&mut imported.pixels);
    for r in &binding.unused {
        report.add(format!("texture set \"{}\"", r.name), Fidelity::Approximated, "no triangles are assigned to this texture set, so it was omitted");
    }
    hooks.check()?;
    (hooks.progress)(0.95, Step::Saving);
    if let Some(document) = &model {
        report_unhandled_document(document, &mut report);
    }
    imported.resources = images.files();
    mesh::prepare_stored_materials(&mut mesh, project.texture_sets.iter_mut().map(|s| &mut s.material)).map_err(ImportError::Invalid)?;
    project.import = Some(report);
    imported.project = project;
    imported.mesh = mesh;
    (hooks.progress)(1.0, Step::Saving);
    Ok(imported)
}

fn graph_library(file: &ProjectFile, options: &ImportOptions, hooks: &Hooks<'_>, report: &mut ImportReport) -> Result<Option<GraphLibrary>, ImportError> {
    if options.graphs != crate::GraphMode::Decompile {
        return Ok(None);
    }
    hooks.check()?;
    let library = GraphLibrary::from_project(file);
    report.add("compiled graphs", Fidelity::Approximated, format!("{} decompiled and run by TextureCraft's graph runtime", library.len()));
    Ok(Some(library))
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
    /// Per submesh, the per-triangle flags of its source polygons.
    polygon_starts: &'a [Vec<bool>],
    images: &'a ProjectImages<'a>,
    options: &'a ImportOptions,
    syscoord: Option<NormalMapSyscoord>,
    bake: Option<&'a ProjectBakeSettings>,
    /// How the original normalised its mesh (where high-poly files modelled next to it belong).
    normalization: Option<Normalization>,
    cache: &'a [texturecraft_import_reader::project::CacheEntryRef],
    hooks: &'a Hooks<'h>,
    graphs: Option<&'a GraphLibrary>,
    /// This set's number (from 1) and the set count, and its share of the progress bar (start, length).
    index: (usize, usize),
    span: (f32, f32),
}

/// One channel of a set's stored result: where it is in the file, its size and how many bytes it is short.
struct StoredChannel<'e> {
    channel: Channel,
    entry: &'e CacheEntryRef,
    size: (u32, u32),
    missing_bytes: usize,
    prepared: Option<texturecraft_texel::TileImage>,
}

impl StoredChannel<'_> {
    /// The channel read from the file, at `resolution` square.
    fn read(&self, file: &ProjectFile, resolution: u32) -> Option<CacheImage> {
        read_cache(file, self.entry).ok().map(|img| resized_cache_image(img, resolution))
    }
}

fn resized_cache_image(mut image: CacheImage, resolution: u32) -> CacheImage {
    if (image.image.width, image.image.height) != (resolution, resolution) {
        image.image = resample(&image.image, resolution);
    }
    image
}

/// Shares of a set's span (start, length): translating its layers, replaying strokes, cooking graphs (the composite
/// that cooks them and measures the stack). Reading the stored result and baked maps comes before the first.
const TRANSLATING: (f32, f32) = (0.02, 0.08);
const REPLAYING: (f32, f32) = (0.1, 0.25);
const COOKING: (f32, f32) = (0.35, 0.65);
/// Share of the replay step that renders the brushes' graphs (the rest lays the strokes).
const BRUSH_GRAPHS: f32 = 0.3;

impl SetContext<'_, '_> {
    /// The overall fraction at `share` (0..1) of this set's span.
    fn at(&self) -> impl Fn(f32) -> f32 + Sync + use<> {
        let span = self.span;
        move |share: f32| span.0 + share.clamp(0.0, 1.0) * span.1
    }
}

/// Layers of the tree, folders and their children included.
fn layer_count(layers: &[texturecraft_import_reader::model::ProjectLayer]) -> usize {
    layers.iter().map(|l| 1 + layer_count(l.children())).sum()
}

/// Graphs the stack runs that differ in what they render (a graph, its values and inputs, nested graphs included).
fn distinct_graphs(layers: &[Layer]) -> usize {
    let mut seen = BTreeSet::new();
    for g in layers.iter().flat_map(|l| l.graphs()).flat_map(|g| g.graphs()) {
        seen.insert(format!("{}{:?}{:?}{}", g.key, g.params, g.inputs, g.seed));
    }
    seen.len()
}

struct StoredChannels<'a> {
    cache: Vec<StoredChannel<'a>>,
    resolution: u32,
    composite: Option<Composite>,
}

/// Shares the saved channel decoding between a disposable view and the reference layer.
fn read_stored_channels<'a>(ctx: &SetContext<'a, '_>, report: &mut ImportReport, preview: bool) -> Result<StoredChannels<'a>, ImportError> {
    let record = &ctx.source.record;
    let name = &record.name;
    let item = |what: &str| format!("texture set \"{name}\" / {what}");
    // Prepare the stored reference directly for interactive opens. An explicit comparison still reads each channel
    // in its original precision after compositing the stack.
    let mut cache: Vec<StoredChannel> = Vec::new();
    let only = ctx.options.translate_only;
    let prepare_reference = !ctx.options.measure;
    let mut cached_resolution = None;
    let mut preview = (preview && ctx.project_set.is_none_or(|set| !set.uses_uv_tiles)).then(|| Composite::new(&[], 256));
    match ctx.source.cache_set.as_ref().filter(|_| !only) {
        None if only => {}
        None => report.add(item("stored result"), Fidelity::Approximated, "no stored result (the original never rendered this set)"),
        Some(folder) => {
            let (mut entries, other_tiles) = uv_tiles::one_tile(ctx.cache.iter().filter(|e| &e.set == folder).collect());
            if other_tiles > 0 {
                report.add(item("stored result"), Fidelity::Approximated, uv_tiles::other_tiles_note(other_tiles));
                preview = None;
            }
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
                let img = match read_cache(ctx.file, e) {
                    Ok(img) => img,
                    Err(err) => {
                        report.add(item(&format!("channel {label}")), Fidelity::Approximated, format!("unreadable stored result: {err}"));
                        continue;
                    }
                };
                let (size, missing_bytes) = ((img.image.width, img.image.height), img.missing_bytes);
                if let Some(preview) = &mut preview
                    && missing_bytes == 0
                    && channel.user_index().is_none()
                {
                    let default = uncovered_default(channel, ctx.project_set.and_then(|set| set.shader_model));
                    preview.channels.insert(channel, preview_tiles(&img.image, channel, default, preview.resolution));
                }
                let prepared = if prepare_reference {
                    ctx.hooks.check()?;
                    let resolution = *cached_resolution.get_or_insert_with(|| resolve_resolution(record, Some(size), ctx.options, name, report));
                    Some(resized_cache_image(img, resolution).image.layer_tiles(channel))
                } else {
                    None
                };
                cache.push(StoredChannel { channel, entry: e, size, missing_bytes, prepared });
            }
        }
    }
    let resolution = cached_resolution.unwrap_or_else(|| resolve_resolution(record, cache.first().map(|s| s.size), ctx.options, name, report));
    Ok(StoredChannels { cache, resolution, composite: preview.filter(|image| !image.channels.is_empty()) })
}

fn preview_tiles(image: &Rgba, channel: Channel, default: [f32; 3], side: u32) -> texturecraft_texel::TileImage {
    use texturecraft_texel::{TileImage, to_u16, to_u16_signed};

    let image = resample(image, side);
    let mut tiles = TileImage::new(side, side, channel.components());
    let quantize: fn(f32) -> u16 = if channel.kind() == texturecraft_core::ChannelKind::Signed { to_u16_signed } else { to_u16 };
    for y in 0..side {
        for x in 0..side {
            let [r, g, b, coverage] = image.get(x, y);
            let coverage = if coverage.is_finite() { coverage.clamp(0.0, 1.0) } else { 0.0 };
            let pixel = [r, g, b];
            let values = std::array::from_fn::<_, 3, _>(|index| {
                let value = if pixel[index].is_finite() { pixel[index] } else { default[index] };
                quantize(value * coverage + default[index] * (1.0 - coverage))
            });
            tiles.set(x, y, &values);
        }
    }
    tiles
}

/// Builds one texture set into `project` and `out`.
fn build_set(
    ctx: &SetContext,
    project: &mut Project,
    out: &mut Imported,
    report: &mut ImportReport,
    cache: Vec<StoredChannel>,
    resolution: u32,
) -> Result<SetSummary, ImportError> {
    let record = &ctx.source.record;
    let name = record.name.clone();
    let item = |what: &str| format!("texture set \"{name}\" / {what}");
    let only = ctx.options.translate_only;
    let mut set = TextureSet::new(name.clone(), ctx.source.slot, resolution);
    let shader_model = ctx.project_set.and_then(|s| s.shader_model);
    let mut declared = SetChannels::default();
    for c in &record.channels {
        match channels::channel_for_bit(c.type_bit) {
            Some(channel) => {
                declared.declare(channel, channels::channel_format(channel, Some(c.format)));
                if !c.user_name.is_empty() && channel.user_index().is_some() {
                    set.channel_names.insert(channel, c.user_name.clone());
                }
            }
            None => {
                let label = if c.user_name.is_empty() {
                    channels::project_channel_name(c.type_bit)
                } else {
                    format!("{} (\"{}\")", channels::project_channel_name(c.type_bit), c.user_name)
                };
                let note = match c.type_bit == 0 || !c.user_name.is_empty() {
                    true => "a channel the file numbers past the eight user channels TextureCraft keeps; not imported",
                    false => "no matching channel in TextureCraft; not imported",
                };
                report.add(item(&format!("channel {label}")), Fidelity::Approximated, note);
            }
        }
    }
    for s in &cache {
        declared.declare(s.channel, texturecraft_core::ChannelFormat::default_for(s.channel));
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
    for b in baked::sources(ctx.file, &name, &record.baked).into_iter().filter(|_| !only) {
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
                // held as 16-bit steps where it is exact, before the next map is read
                maps.insert(kind, texturecraft_compose::Texels::new(values));
                set.baked.insert(kind);
            }
            Err(e) => report.add(item(&format!("baked map \"{}\"", b.name)), Fidelity::Approximated, format!("unreadable: {e}")),
        }
    }
    let baked = baked::maps(resolution, maps);
    // bake settings
    let uid = ctx.project_set.and_then(|s| s.uid);
    let bake = ctx.bake.and_then(|b| b.for_material(uid));
    let mapped = bake_settings::map(bake, resolution);
    set.bake = mapped.settings;
    if !mapped.unmapped.is_empty() {
        report.add(item("bake settings"), Fidelity::Approximated, format!("not taken over: {}", mapped.unmapped.join("; ")));
    }
    if !mapped.meshes.is_empty() {
        report.add(item("high-poly meshes"), Fidelity::Approximated, mapped.meshes.note(ctx.normalization));
    }
    project.texture_sets.push(set);
    // the layer stack
    let parts: Vec<String> = ctx.mesh.submeshes.iter().filter(|s| s.material == ctx.source.slot).map(|s| s.name.clone()).collect();
    let geometry = SetGeometry::new(ctx.mesh, ctx.source.slot, ctx.polygon_starts);
    let mut stack = match ctx.project_set {
        Some(project_set) => {
            let replay = ctx.options.replay_strokes && (ctx.hooks.replay.is_some() || only);
            let done = AtomicUsize::new(0);
            let input = SetInput {
                channels: &declared,
                images: ctx.images,
                mesh_maps: &project_set.mesh_maps,
                parts: &parts,
                options: ctx.options,
                replay,
                graphs: ctx.graphs,
                geometry: Some(&geometry),
                layers_done: Some(&done),
            };
            // the layers as they are translated (brush tips render here)
            let expected = layer_count(&project_set.layers) as f32;
            let (progress, at, step) = (ctx.hooks.progress, ctx.at(), Step::ConvertingSet { set: ctx.index.0, of: ctx.index.1 });
            let poll = || progress(at(TRANSLATING.0 + TRANSLATING.1 * share(done.load(Ordering::Relaxed) as f32, expected)), step);
            let mut stack = watching(WATCH_PERIOD, &poll, || layers::import(project_set, &input, project.next_id));
            if let Some(note) = uv_tiles::per_tile_note(project_set, &uv_tiles::mesh_tiles(ctx.mesh, ctx.source.slot)) {
                report.add(item("UV tiles"), Fidelity::Approximated, note.clone());
                stack.verdict.set_settings.and(&Status::approximated(note));
            }
            Some(stack)
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
    if let Some(stack) = stack.as_mut().filter(|_| !only)
        && !stack.fills.is_empty()
    {
        ctx.hooks.check()?;
        lay_polygon_fills(stack, ctx.mesh, ctx.source.slot, resolution, &mut out.pixels);
    }
    if let Some(stack) = stack.as_mut() {
        // particle speeds in the stored mesh's units first: the records keep what the replay reads
        crate::replay::particles::in_file_units(&mut stack.jobs, ctx.normalization.map(|n| n.radius));
        record_strokes(stack, ctx.graphs, out);
    }
    let mut replayed = ReplayCount::default();
    if let Some(stack) = stack.as_mut()
        && let Some(replayer) = ctx.hooks.replay.filter(|_| !stack.jobs.is_empty() && !only)
    {
        ctx.hooks.check()?;
        let (set, of) = ctx.index;
        let (progress, at, step) = (ctx.hooks.progress, ctx.at(), Step::ReplayingStrokes { set, of });
        progress(at(REPLAYING.0), step);
        // the brush graphs (a share of the step by the graphs rendered), then the strokes (by the strokes laid)
        let (base, expected) = (out.graphs.cook_progress(), crate::replay::source::text_keys(&stack.jobs).len() as f32);
        let poll = || progress(at(REPLAYING.0 + BRUSH_GRAPHS * REPLAYING.1 * share(out.graphs.cook_progress() - base, expected)), step);
        watching(WATCH_PERIOD, &poll, || render_brush_sources(stack, ctx.graphs, &out.graphs, &baked, ctx.images, resolution));
        ctx.hooks.check()?;
        let scene = ReplayScene { mesh: ctx.mesh, material: ctx.source.slot, resolution };
        let (base, strokes) = (replayer.strokes_done(), stack.jobs.iter().map(ReplayJob::strokes).sum::<usize>() as f32);
        let from = REPLAYING.0 + BRUSH_GRAPHS * REPLAYING.1;
        let poll = || {
            if let (Some(now), Some(base)) = (replayer.strokes_done(), base) {
                progress(at(from + (1.0 - BRUSH_GRAPHS) * REPLAYING.1 * share(now.saturating_sub(base) as f32, strokes)), step);
            }
        };
        let pixels = &mut out.pixels;
        replayed = watching(WATCH_PERIOD, &poll, || replay_stack(stack, replayer, &scene, pixels, &|what: &str| item(what), report));
        // a replayer stopped by the flag fails its remaining jobs: they are not reported as failures
        ctx.hooks.check()?;
    }
    if let Some(stack) = stack.as_mut().filter(|_| !only)
        && !stack.fills.is_empty()
    {
        lay_fills_over_strokes(stack, ctx.mesh, ctx.source.slot, resolution, &mut out.pixels);
    }
    if let Some(stack) = &stack {
        insert_stack(project, ts, stack, report);
    }
    // channel defaults under every layer, unless a full uniform fill covers each channel
    let defaults_id = (!provides_base(project, ts, &declared)).then(|| {
        let id = project.alloc_id();
        let mut fill = Fill::default();
        for c in declared.all() {
            fill.sources.insert(c, Source::Uniform(uncovered_default(c, shader_model)));
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
    // the texts of the graphs the stack runs
    if let (Some(lib), Some(s)) = (ctx.graphs, &stack) {
        for key in s.layers.iter().flat_map(|l| l.graphs()).flat_map(|g| g.text_keys()) {
            if let Some(text) = lib.text(key) {
                out.graphs.texts.entry(key.to_string()).or_insert(text);
            }
        }
    }
    // the safety net; the composite also cooks every graph of the set once, for every channel, so that the project
    // opens without rendering them
    let cache_channels: Vec<Channel> = cache.iter().map(|s| s.channel).collect();
    // the stored result's channels as layer pixels, made where a channel is read for the comparison
    let mut kept: BTreeMap<Channel, texturecraft_texel::TileImage> = BTreeMap::new();
    let mut scores = Vec::new();
    let mut corrections: Vec<correction::ChannelCorrection> = Vec::new();
    let measuring = ctx.options.measure && !cache.is_empty() && stack.is_some();
    let cooking = !only && stack.as_ref().is_some_and(|s| s.layers.iter().any(|l| !l.graphs().is_empty()));
    if measuring || cooking {
        ctx.hooks.check()?;
        let (set, of) = ctx.index;
        let (progress, at, step) = (ctx.hooks.progress, ctx.at(), Step::CookingGraphs { set, of });
        progress(at(COOKING.0), step);
        let store: ImageStore = ctx.images.store();
        let scene = SetScene { project, ts, mesh: ctx.mesh, pixels: &out.pixels, baked: &baked, images: &store, graphs: &out.graphs };
        let mut channels = cache_channels.clone();
        if cooking {
            channels.extend(project.texture_set(ts).map(|s| s.channel_list()).unwrap_or_default().into_iter().filter(|c| !cache_channels.contains(c)));
        }
        // by the graphs cooked (each running render by its nodes), against the distinct graphs the stack runs
        let expected = stack.as_ref().map_or(0, |s| distinct_graphs(&s.layers)) as f32;
        let base = out.graphs.cook_progress();
        let poll = || progress(at(COOKING.0 + COOKING.1 * share(out.graphs.cook_progress() - base, expected)), step);
        let composite = watching(WATCH_PERIOD, &poll, || {
            if measuring {
                safety_net::composite(&scene, &channels)
            } else {
                safety_net::prepare_graphs(&scene);
                None
            }
        });
        if let Some((composite, gbuf)) = composite {
            let formats = project.texture_set(ts).map(|s| s.channels.clone()).unwrap_or_default();
            for stored in &cache {
                let c = &stored.channel;
                let Some(img) = stored.read(ctx.file, resolution) else { continue };
                scores.push((*c, safety_net::score(&composite, &gbuf, *c, &img, uncovered_default(*c, shader_model))));
                if ctx.options.original_correction {
                    let format = formats.iter().find(|(f, _)| f == c).map_or_else(|| texturecraft_core::ChannelFormat::default_for(*c), |(_, f)| *f);
                    let tolerance = correction::TEXEL_STEPS * correction::storage_step(format);
                    corrections.extend(correction::correct(&composite, &gbuf, *c, &img, uncovered_default(*c, shader_model), tolerance));
                }
                kept.insert(*c, img.image.layer_tiles(*c));
            }
        }
        // renders stopped by the flag show nothing: the set is not finished
        ctx.hooks.check()?;
    }
    let agrees = scores.iter().all(|(_, s)| s.mean <= AGREEMENT_STEPS);
    let strategy = Strategy::choose(exact, !cache.is_empty(), agrees);
    let note = scores
        .iter()
        .filter(|(_, s)| s.texels > 0)
        .map(|(c, s)| format!("{} mean {:.2} max {:.1}", declared.label(*c), s.mean, s.max))
        .collect::<Vec<_>>()
        .join(", ");
    if stack.is_some() {
        let (fidelity, description) = match strategy {
            Strategy::RealStack if note.is_empty() => (Fidelity::Approximated, "the known layer parts translated exactly and are used for the result"),
            Strategy::RealStack => (Fidelity::Exact, strategy.describe()),
            _ => (Fidelity::Approximated, strategy.describe()),
        };
        let measured = if note.is_empty() { "not measured".to_string() } else { format!("against the stored result: {note}") };
        report.add(item("layer stack"), fidelity, format!("{layer_count} layers; {description}; {measured}"));
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
        report_strokes(project_set, &item("strokes"), &replayed, report);
    }
    // the correction on top of the layers, under the stored result
    if matches!(strategy, Strategy::NotReproduced | Strategy::Disagrees) && corrections.iter().any(|c| c.share > 0.0) {
        add_correction(project, ts, corrections, &item, out, report);
    } else if ctx.options.original_correction && !measuring && !cache.is_empty() {
        report.add(item(correction::CORRECTION_NAME), Fidelity::Approximated, "not added: the stack was not compared with the stored result");
    }
    // the stored result as a locked layer: the reference, or the result on top
    if !cache.is_empty() {
        let shown = ctx.options.original_on_top && matches!(strategy, Strategy::NotReproduced | Strategy::Disagrees) || stack.is_none();
        let id = project.alloc_id();
        let layer = original_result_layer(id, &cache_channels, shown);
        for mut stored in cache {
            let c = &stored.channel;
            let tiles = stored.prepared.take().or_else(|| kept.remove(c)).or_else(|| stored.read(ctx.file, resolution).map(|img| img.image.layer_tiles(*c)));
            let Some(tiles) = tiles else { continue };
            out.pixels.layers.insert((id, *c), tiles);
            let missing = stored.missing_bytes;
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

/// The correction layer of set `ts` from its corrected channels, pushed on top of its stack.
fn add_correction(
    project: &mut Project,
    ts: usize,
    corrections: Vec<correction::ChannelCorrection>,
    item: &dyn Fn(&str) -> String,
    out: &mut Imported,
    report: &mut ImportReport,
) {
    let shares: Vec<(String, f32)> = corrections.iter().map(|c| (c.channel.label().to_lowercase(), c.share)).collect();
    let text = correction::describe(&shares);
    let channels: Vec<Channel> = corrections.iter().map(|c| c.channel).collect();
    let id = project.alloc_id();
    let mut layer = Layer::new(id, correction::CORRECTION_NAME, LayerKind::Paint, &channels);
    layer.lock = Some(Lock::import(vec![LockReason::new(correction::CORRECTION_KEY, text.clone())]));
    for c in corrections {
        out.pixels.layers.insert((id, c.channel), c.tiles);
    }
    project.layers.insert(id, layer);
    if let Ok(set) = project.texture_set_mut(ts) {
        set.stack.push(id);
    }
    let r = report.layer_mut(id);
    r.role = LayerRole::Correction;
    r.fidelity = Fidelity::Baked;
    report.add(item(correction::CORRECTION_NAME), Fidelity::Baked, text);
}

/// Lays the stack's polygon fills into their paint effects' pixels, over the set's UV G-buffer at `resolution`.
/// Fills laid over replayed strokes wait for them (see [`lay_fills_over_strokes`]).
fn lay_polygon_fills(stack: &mut ImportedStack, mesh: &Mesh, slot: usize, resolution: u32, pixels: &mut PixelStore) {
    let gbuf = UvGBuffer::new(mesh, slot, resolution, resolution);
    let (over, now): (Vec<FillJob>, Vec<FillJob>) = std::mem::take(&mut stack.fills).into_iter().partition(|j| j.over_strokes);
    stack.fills = over;
    for job in now {
        if let Some(image) = job.paint(&gbuf) {
            match job.channel {
                None => pixels.masks.insert(job.effect, image),
                Some(c) => pixels.layers.insert((job.effect, c), image),
            };
        }
    }
}

/// Lays the polygon fills that sit above replayed strokes in a mask group over the strokes' pixels: a picked texel takes
/// the fill's value at full coverage, as a fill above strokes in one group paints over them.
fn lay_fills_over_strokes(stack: &mut ImportedStack, mesh: &Mesh, slot: usize, resolution: u32, pixels: &mut PixelStore) {
    let gbuf = UvGBuffer::new(mesh, slot, resolution, resolution);
    for job in std::mem::take(&mut stack.fills) {
        let Some(top) = job.paint(&gbuf) else { continue };
        let below = match job.channel {
            None => pixels.masks.get_mut(&job.effect),
            Some(c) => pixels.layers.get_mut(&(job.effect, c)),
        };
        let Some(below) = below.filter(|b| (b.width(), b.height(), b.comps()) == (top.width(), top.height(), top.comps())) else {
            match job.channel {
                None => pixels.masks.insert(job.effect, top),
                Some(c) => pixels.layers.insert((job.effect, c), top),
            };
            continue;
        };
        let alpha = top.comps().saturating_sub(1);
        for (tx, ty) in top.allocated().collect::<Vec<_>>() {
            let side = texturecraft_texel::TILE;
            for y in ty * side..((ty + 1) * side).min(top.height()) {
                for x in tx * side..((tx + 1) * side).min(top.width()) {
                    let px = top.get(x, y);
                    if px.get(alpha).copied().unwrap_or(0) > 0 {
                        below.set(x, y, px.get(..top.comps()).unwrap_or_default());
                    }
                }
            }
        }
    }
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
            Some("strokes") => "stroke replay is off; the pixels are in the original's stored result",
            Some("brush" | "brush-alpha") => "strokes TextureCraft's brush does not replay yet; the pixels are in the original's stored result",
            _ => "not supported yet",
        };
        report.add(item(&format!("blocker {key}")), Fidelity::Baked, format!("{text}: locks {n} layers ({alone} by it alone); {unlock}"));
    }
}

/// Renders the graphs brushes of `stack`'s replay jobs paint, as a fill of them renders (the set's baked maps and
/// images, the texture's side), sharing the import's cooks (`graphs`: renders already held, or kept between imports, are
/// not made again; the project does not keep them, as no layer reads them).
fn render_brush_sources(
    stack: &mut ImportedStack,
    lib: Option<&GraphLibrary>,
    graphs: &texturecraft_compose::GraphStore,
    baked: &texturecraft_compose::BakedMaps,
    images: &ProjectImages,
    resolution: u32,
) {
    let keys = crate::replay::source::text_keys(&stack.jobs);
    if keys.is_empty() {
        return;
    }
    let mut store = texturecraft_compose::GraphStore { texts: Default::default(), cooks: graphs.cooks.clone() };
    for k in keys {
        if let Some(text) = lib.and_then(|l| l.text(&k)) {
            store.texts.insert(k, text);
        }
    }
    let images = images.store();
    let side = texturecraft_compose::graphs::RenderEnv::side_for((resolution, resolution));
    let env = texturecraft_compose::graphs::RenderEnv { baked, images: &images, side };
    crate::replay::source::render(&mut stack.jobs, &store, &env);
}

/// Keeps the record of every replay job of `stack` (taken before brush graphs render, so they keep their references), and
/// the texts of the graphs they paint.
fn record_strokes(stack: &ImportedStack, lib: Option<&GraphLibrary>, out: &mut Imported) {
    for job in &stack.jobs {
        out.strokes.insert(job.target.id(), crate::replay::record::record(job).into());
    }
    for key in crate::replay::source::text_keys(&stack.jobs) {
        if let Some(text) = lib.and_then(|l| l.text(&key)) {
            out.graphs.texts.entry(key).or_insert(text);
        }
    }
}

/// What a set's stroke replay painted: paint effects and strokes, and jobs that failed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReplayCount {
    pub effects: usize,
    pub strokes: usize,
    pub failed: usize,
}

/// Lays `stack`'s replay jobs with `replayer`: their pixels go into `pixels`; a job that fails becomes a placeholder
/// that locks its layer (`brush:replay-failed`), reported.
fn replay_stack(
    stack: &mut ImportedStack,
    replayer: &dyn StrokeReplayer,
    scene: &ReplayScene,
    pixels: &mut PixelStore,
    item: &dyn Fn(&str) -> String,
    report: &mut ImportReport,
) -> ReplayCount {
    let jobs = std::mem::take(&mut stack.jobs);
    let mut results = replayer.replay(scene, &jobs).into_iter();
    let mut count = ReplayCount::default();
    for job in &jobs {
        let result = results.next().unwrap_or_else(|| Err("the replayer gave no result".into()));
        match (result, job.target) {
            (Ok(ReplayedPixels::Content(images)), ReplayTarget::Content(id)) => {
                for (c, img) in images {
                    pixels.layers.insert((id, c), img);
                }
            }
            (Ok(ReplayedPixels::Mask(img)), ReplayTarget::Mask(id)) => {
                pixels.masks.insert(id, img);
            }
            (Ok(_), _) => {
                fail_job(stack, job, "the replayer painted the wrong kind of pixels");
                count.failed += 1;
                report.add(item(&job.label), Fidelity::Baked, "stroke replay failed: the replayer painted the wrong kind of pixels");
                continue;
            }
            (Err(why), _) => {
                fail_job(stack, job, &why);
                count.failed += 1;
                report.add(item(&job.label), Fidelity::Baked, format!("stroke replay failed ({why}); kept as a placeholder"));
                continue;
            }
        }
        count.effects += job.steps.len();
        count.strokes += job.strokes();
    }
    stack.jobs = jobs;
    count
}

/// A job that could not be painted: its effect becomes a placeholder (a layer's own brush work gets one in front of its
/// content effects), the layer is locked and its channels count as not computable.
fn fail_job(stack: &mut ImportedStack, job: &ReplayJob, why: &str) {
    let blocker = crate::replay::brush::refused("replay-failed", format!("stroke replay failed: {why}"));
    let id = job.target.id();
    let label = format!("brush strokes ({} strokes)", job.strokes());
    let reason = format!("brush strokes are kept in the original result: {}", blocker.text);
    let holder = stack
        .layers
        .iter_mut()
        .find(|l| l.id == id || l.content_effects.iter().any(|e| e.id == id) || l.mask.as_ref().is_some_and(|m| m.effects.iter().any(|e| e.id == id)));
    let Some(layer) = holder else { return };
    if layer.id == id {
        let mut e = texturecraft_core::Effect::placeholder(LayerId(stack.next_id), &label, &reason);
        e.visible = true;
        stack.next_id = stack.next_id.saturating_add(1);
        layer.content_effects.insert(0, e);
    } else {
        let effects = layer.content_effects.iter_mut().chain(layer.mask.iter_mut().flat_map(|m| m.effects.iter_mut()));
        for e in effects.filter(|e| e.id == id) {
            e.name = label.clone();
            e.kind = texturecraft_core::EffectKind::Placeholder { label: label.clone(), reason: reason.clone() };
        }
    }
    let reason = texturecraft_core::LockReason::new(blocker.key.to_string(), blocker.text.clone());
    match layer.lock.as_mut() {
        Some(lock) => lock.reasons.push(reason),
        None => layer.lock = Some(texturecraft_core::Lock::import(vec![reason])),
    }
    let layer_id = layer.id;
    if let Some(r) = stack.reports.get_mut(&layer_id) {
        r.fidelity = Fidelity::Baked;
    }
    let status = crate::status::Status::not_computable(blocker);
    for c in job.steps.iter().flat_map(|s| s.values.iter().map(|v| v.0)) {
        stack.verdict.contribute(c, &status);
    }
}

/// The brush strokes of a set: how many were replayed; the others' pixels are only in the stored result.
fn report_strokes(set: &ProjectTextureSet, item: &str, replayed: &ReplayCount, report: &mut ImportReport) {
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
        let rest = strokes.saturating_sub(replayed.strokes);
        let note = match replayed.strokes {
            0 => format!("{strokes} strokes in {paints} paint effects not replayed; their pixels are only in the original's stored result"),
            n => format!(
                "{strokes} strokes in {paints} paint effects; {n} strokes of {} paint effects replayed with TextureCraft's brush (approximated); {rest} not replayed (placeholders, their pixels only in the original's stored result)",
                replayed.effects
            ),
        };
        report.add(item, if rest == 0 { Fidelity::Approximated } else { Fidelity::Baked }, note);
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
fn resolve_resolution(record: &MaterialRecord, cached: Option<(u32, u32)>, options: &ImportOptions, name: &str, report: &mut ImportReport) -> u32 {
    let recorded = record.resolution_log2.filter(|r| r.iter().all(|v| (0..=14).contains(v))).map(|[w, h]| (1u32 << w).max(1u32 << h));
    let cached = cached.map(|(w, h)| w.max(h));
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
pub(crate) fn resample(image: &Rgba, size: u32) -> Rgba {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::polygon_fill::FillRun;
    use texturecraft_core::LayerId;
    use texturecraft_texel::TileImage;

    #[test]
    fn polygon_fills_above_strokes_paint_over_the_strokes_pixels() {
        use glam::{Vec2, Vec3};
        let mesh = Mesh {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y],
            normals: vec![Vec3::Z; 4],
            uvs: vec![Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 0.0)],
            indices: vec![0, 1, 2, 0, 2, 3],
            submeshes: vec![texturecraft_geom::SubMesh { name: "Body".into(), material: 0, start: 0, count: 6 }],
            materials: vec!["m".into()],
            ..Mesh::default()
        };
        // the strokes' pixels: grey 0.5 everywhere; the fill picks triangle 0 at 0
        let effect = LayerId(7);
        let mut strokes = TileImage::new(16, 16, 2);
        for y in 0..16 {
            for x in 0..16 {
                strokes.set(x, y, &[u16::MAX / 2, u16::MAX]);
            }
        }
        let mut pixels = PixelStore::default();
        pixels.masks.insert(effect, strokes);
        let run = FillRun { color: [0.0; 3], triangles: [0u32].into_iter().collect() };
        let job = FillJob { effect, channel: None, runs: vec![run], label: String::new(), over_strokes: true };
        let mut stack = ImportedStack { fills: vec![job], ..ImportedStack::default() };
        lay_fills_over_strokes(&mut stack, &mesh, 0, 16, &mut pixels);
        let img = pixels.masks.get(&effect).unwrap();
        // picked: the fill's value; elsewhere the strokes stay
        assert_eq!(img.get(14, 14)[..2], [0, u16::MAX]);
        assert_eq!(img.get(1, 1)[..2], [u16::MAX / 2, u16::MAX]);
        assert!(stack.fills.is_empty());
    }
}
