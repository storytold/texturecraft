//! The `.tcraft` project file: a deflate zip holding
//!
//! | entry | content |
//! |---|---|
//! | `project.json` | serde JSON of [`Project`] |
//! | `mesh.<ext>` | the original imported mesh file (absent for built-in meshes, `Project.mesh = "builtin:<id>"`) |
//! | `highpoly/<name>` | a high-definition mesh file for baking, in list order |
//! | `cage/<name>` | the baking cage mesh file |
//! | `bake_paint/<ts>_<target>.bin` | cage or skew paint, stored as a one-component baked map |
//! | `pixels/layer_<id>_<channelId>.bin` | painted pixels of a paint layer (or content paint effect) channel |
//! | `pixels/mask_<id>.bin` | painted pixels of a mask paint effect |
//! | `baked/<ts>_<mapId>.bin` | a baked mesh map of texture set `<ts>` |
//! | `thumbnail.png` | optional preview supplied by the caller |
//! | `graphs/<key>.txt` | the text of a graph the layers run, once per distinct graph, named by its text key |
//! | `cooked/<key>_<output>.bin` | one output of a graph's cooked render (pixel tiles), named by the cook's key |
//! | `cooked/index.json` | the cooks' outputs (and which hold linear colour) and the cook each graph use read last |
//! | `paint_strokes.json` | the native brush operations retained for resolution changes |
//! | `strokes/<id>.bin` | the imported stroke record behind the pixels keyed by `<id>` (see [`strokes`]) |
//! | `stroke_images/<key>.bin` | a tip or stencil image stroke records lay, kept once (see [`strokes`]) |
//!
//! Pixel `.bin` files are sparse and lossless: a header (magic `TCTX`, version, width, height,
//! components, tile count; u32 LE each) followed, for every allocated 64×64 tile, by `tx`, `ty`
//! (u32 LE) and the tile's raw `u16` LE components. Baked maps use the same header with a tile
//! count of 0, followed by `width × height × components` f32 LE. Pixels of a UV-tiled texture
//! set cover its whole atlas (`resolution` per tile, see `TileGrid`).
//!
//! Graph texts are saved once per distinct text, only those the layers reference; a text whose bytes do not hash to its
//! entry's key is refused. Cooked renders are saved for the uses the last composite read (see
//! `texturecraft_compose::graphs`): a project opened again finds them by their keys and renders nothing. Cooks whose
//! entries are missing or malformed are left out with a warning: they are only renders, made again on demand.
//!
//! Stroke records are saved for the paint layers and paint effects that still exist; a record that cannot be read is
//! kept as its bytes and saved again unchanged. They add entries only, so a project without them is written as before
//! and older builds open a project with them (they skip the entries).

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::sync::Arc;

use texturecraft_compose::graphs::{Cooked, GraphUse};
use texturecraft_compose::{BakedMaps, GraphStore, PixelStore, Texels};
use texturecraft_core::{BakePaintTarget, BakedMap, Channel, FORMAT_VERSION, LayerId, Project, StoredStrokes, TANGENT_BENT_NORMAL_VERSION};
use texturecraft_texel::{TILE, TILE_AREA, TileImage};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use self::entries::Entry;
use self::load_budget::LoadBudget;
use crate::{FormatError, MAX_IMAGE_SIZE, Result, malformed};

mod entries;
mod load_budget;
mod project_open_limit;
mod ram_operation;
mod ram_reservation;
pub mod strokes;
#[cfg(not(target_arch = "wasm32"))]
pub use entries::{DEFAULT_RAM_PERCENT, DEFAULT_RESERVED_CORES, available_save_resources, configured_ram_budget, save_ram_budget, set_save_resources};
pub use project_open_limit::{DEFAULT_PROJECT_OPEN_LIMIT_GIB, project_open_limit_gib, set_project_open_limit_gib};
pub use ram_reservation::RamReservation;

const KIND: &str = "project file";
const MAGIC: &[u8; 4] = b"TCTX";
const BIN_VERSION: u32 = craft_tcraft::VERSION;
const BIN_HEADER_BYTES: usize = craft_tcraft::HEADER_BYTES as usize;
const MAX_JSON: u64 = 256 << 20;
const MAX_STROKE_REPLAY: u64 = 32 << 20;
const STROKE_REPLAY_ENTRY: &str = "paint_strokes.json";
const MAX_THUMBNAIL: u64 = 64 << 20;
const MAX_RESOURCE: u64 = 512 << 20;
const MAX_ENTRIES: usize = 100_000;
/// Bound ZIP index parsing before the archive exposes its entry count.
const MAX_ZIP_METADATA_BYTES: u64 = 32 << 20;
/// Largest graph text (they carry the images their graph embeds).
const MAX_GRAPH_TEXT: u64 = 512 << 20;
/// Largest index of cooked graph outputs.
const MAX_COOK_INDEX: u64 = 16 << 20;
/// Largest side of a cooked graph output (the graph runtime renders at most this).
const MAX_COOK_SIDE: u32 = 8192;
/// Version of `cooked/index.json`.
const COOK_INDEX_VERSION: u32 = 1;
/// Largest image (texels) of a texture set or baked map: UV-tiled sets hold several tiles.
const MAX_TEXELS: u64 = 1 << 28;

mod bake_paint_data;
mod recovery;
mod stroke_replay_data;
pub use bake_paint_data::BakePaintData;
pub use stroke_replay_data::StrokeReplayData;

/// Everything a project file holds.
#[derive(Clone, Debug)]
pub struct ProjectFile<Resource = Vec<u8>> {
    pub project: Project,
    pub pixels: PixelStore,
    /// Optional paint operations. Raster pixels are still saved for compatible readers.
    pub stroke_replay: Option<StrokeReplayData>,
    /// Baked mesh maps per texture set index.
    pub baked: BTreeMap<usize, BakedMaps>,
    /// Original mesh file: (extension, bytes).
    pub mesh: Option<(String, Vec<u8>)>,
    /// High-definition mesh files for baking: (file name, bytes). Names are
    /// [`sanitize_resource_name`]d on save.
    pub high_poly: Vec<(String, Vec<u8>)>,
    /// Baking cage mesh file: (file name, bytes).
    pub cage: Option<(String, Vec<u8>)>,
    pub bake_paint: BTreeMap<(usize, BakePaintTarget), BakePaintData>,
    pub thumbnail: Option<Vec<u8>>,
    /// Imported resources (images used by fills, alphas, stencils), by name: original bytes.
    pub resources: BTreeMap<String, Resource>,
    /// Texts of the graphs the layers run, and their cooked outputs.
    pub graphs: GraphStore,
    /// Stroke records by the id of the pixels they belong to (a paint layer, a content or mask paint effect).
    pub strokes: BTreeMap<LayerId, StoredStrokes>,
    /// Import warnings for the caller's log; never saved in the archive
    pub warnings: Vec<String>,
}

/// Maximum length of a resource name after sanitization.
pub const MAX_RESOURCE_NAME_LEN: usize = 120;

/// A resource name safe as a zip entry and a file name: `[A-Za-z0-9_.-]`, at most 120 chars.
pub fn sanitize_resource_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') { c } else { '_' }).take(MAX_RESOURCE_NAME_LEN).collect();
    let s = s.trim_start_matches('.').to_string();
    if s.is_empty() { "resource".into() } else { s }
}

// ---------------------------------------------------------------------------------------------
// Save.

pub fn save_project_bytes(pf: &ProjectFile) -> Result<Vec<u8>> {
    RamReservation::for_save()?.save_bytes(pf)
}

/// Save atomically through a newly created temporary file beside `path`.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_project(path: &std::path::Path, project_file: &ProjectFile) -> Result<()> {
    RamReservation::for_save()?.save(path, project_file)
}

#[cfg(not(target_arch = "wasm32"))]
fn write_project_atomic(path: &std::path::Path, project_file: &ProjectFile<impl Borrow<Vec<u8>>>) -> Result<()> {
    let (temporary_path, file) = crate::create_temp(path)?;
    let result = (|| -> Result<()> {
        let mut file = std::io::BufWriter::new(file);
        write_project(&mut file, project_file)?;
        file.into_inner().map_err(|error| error.into_error())?.sync_all()?;
        std::fs::rename(&temporary_path, path)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }

    result
}

fn zip_err(e: zip::result::ZipError) -> FormatError {
    match e {
        zip::result::ZipError::Io(e) => FormatError::Io(e),
        e => malformed(KIND, e),
    }
}

fn header(w: u32, h: u32, comps: usize, count: usize) -> Vec<u8> {
    let mut b = MAGIC.to_vec();
    for v in [BIN_VERSION, w, h, comps as u32, count as u32] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

fn codec_write_error(error: craft_tcraft::Error) -> std::io::Error {
    match error {
        craft_tcraft::Error::Io(error) => error,
        other => std::io::Error::other(other),
    }
}

fn write_tiles(w: &mut impl Write, img: &TileImage) -> std::io::Result<()> {
    let expected = img.allocated_count();
    let img = img.temporarily_loaded();
    if img.allocated_count() != expected {
        return Err(std::io::Error::other("could not read the saved paint tiles"));
    }
    let layout = craft_tcraft::Layout { width: img.width(), height: img.height(), components: img.comps() };
    let count = u32::try_from(img.allocated_count()).map_err(std::io::Error::other)?;
    let tiles = img.allocated().map(|(x, y)| (x, y, img.tile(x, y).unwrap_or_default()));
    craft_tcraft::write_tiles(w, layout, count, tiles).map_err(codec_write_error)
}

/// Keep `[a-z0-9]`, at most 8 characters (the mesh entry's extension).
fn sanitize_ext(ext: &str) -> String {
    let e: String = ext.trim_start_matches('.').chars().filter(char::is_ascii_alphanumeric).take(8).collect::<String>().to_ascii_lowercase();
    if e.is_empty() { "bin".into() } else { e }
}

/// The sanitized `name`, with a numeric suffix before the extension when `used` already holds it.
fn unique_entry_name(dir: &str, name: &str, used: &mut HashSet<String>) -> String {
    let base = sanitize_resource_name(name);
    let mut entry = format!("{dir}/{base}");
    let mut n = 2u32;
    while !used.insert(entry.clone()) {
        let (stem, ext) = match base.rfind('.') {
            Some(i) if i > 0 => base.split_at(i),
            _ => (base.as_str(), ""),
        };
        entry = format!("{dir}/{stem}_{n}{ext}");
        n += 1;
    }
    entry
}

const BAKED_CHUNK: usize = 1 << 16;

fn write_baked(w: &mut dyn Write, res: u32, h: u32, comps: usize, data: &[f32]) -> std::io::Result<()> {
    let layout = craft_tcraft::Layout { width: res, height: h, components: comps };
    craft_tcraft::write_baked(w, layout, data).map_err(codec_write_error)
}

/// A baked map's entry: its header, then its values as little-endian floats (also those held as 16-bit steps).
fn write_texels(w: &mut dyn Write, res: u32, h: u32, comps: usize, data: &Texels) -> std::io::Result<()> {
    w.write_all(&header(res, h, comps, 0))?;
    let mut buf = Vec::with_capacity(BAKED_CHUNK * 4);
    data.try_for_chunks(BAKED_CHUNK, |chunk| {
        buf.clear();
        buf.extend(chunk.iter().flat_map(|v| v.to_le_bytes()));
        w.write_all(&buf)
    })
}

/// The zip entries of a project file, in file order. Bodies run later, possibly on other threads.
fn entries(pf: &ProjectFile<impl Borrow<Vec<u8>>>) -> Result<Vec<Entry<'_>>> {
    let deflate = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).large_file(true);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let json = serde_json::to_vec_pretty(&pf.project).map_err(|e| FormatError::Invalid(e.to_string()))?;
    let mut out = vec![Entry { name: "project.json".into(), options: deflate, weight: json.len() as u64, body: Box::new(move |w| w.write_all(&json)) }];
    if let Some(replay) = &pf.stroke_replay {
        let bytes = replay.bytes()?;
        let weight = bytes.len() as u64;
        if weight > MAX_STROKE_REPLAY {
            return Err(FormatError::Invalid("paint stroke archive data exceeds the supported size".into()));
        }
        out.push(Entry { name: STROKE_REPLAY_ENTRY.into(), options: deflate, weight, body: Box::new(move |w| w.write_all(&bytes)) });
    }
    if let Some((ext, bytes)) = &pf.mesh {
        out.push(Entry {
            name: format!("mesh.{}", sanitize_ext(ext)),
            options: deflate,
            weight: bytes.len() as u64,
            body: Box::new(move |w| w.write_all(bytes)),
        });
    }
    let mut used = HashSet::new();
    for (dir, (name, bytes)) in pf.high_poly.iter().map(|m| ("highpoly", m)).chain(pf.cage.iter().map(|m| ("cage", m))) {
        out.push(Entry {
            name: unique_entry_name(dir, name, &mut used),
            options: deflate,
            weight: bytes.len() as u64,
            body: Box::new(move |w| w.write_all(bytes)),
        });
    }
    for ((texture_set, target), map) in &pf.bake_paint {
        if map.width == 0 || map.height == 0 || map.values.len() as u64 != u64::from(map.width) * u64::from(map.height) {
            return Err(FormatError::Invalid("bake paint map dimensions do not match its values".into()));
        }

        let target_name = match target {
            BakePaintTarget::Cage => "cage",
            BakePaintTarget::Skew => "skew",
        };
        out.push(Entry {
            name: format!("bake_paint/{texture_set}_{target_name}.bin"),
            options: deflate,
            weight: map.values.len() as u64 * 4,
            body: Box::new(move |w| write_baked(w, map.width, map.height, 1, &map.values)),
        });
    }
    let mut layers: Vec<_> = pf.pixels.layers.iter().collect();
    layers.sort_by_key(|(k, _)| **k);
    for ((id, c), img) in layers {
        let weight = (img.allocated_count() * (TILE_AREA * img.comps() * 2 + 8)) as u64;
        out.push(Entry {
            name: format!("pixels/layer_{}_{}.bin", id.0, c.id()),
            options: deflate,
            weight,
            body: Box::new(move |mut w| write_tiles(&mut w, img)),
        });
    }
    let mut masks: Vec<_> = pf.pixels.masks.iter().collect();
    masks.sort_by_key(|(k, _)| **k);
    for (id, img) in masks {
        let weight = (img.allocated_count() * (TILE_AREA * img.comps() * 2 + 8)) as u64;
        out.push(Entry { name: format!("pixels/mask_{}.bin", id.0), options: deflate, weight, body: Box::new(move |mut w| write_tiles(&mut w, img)) });
    }
    for (ts, maps) in &pf.baked {
        let r = maps.resolution;
        for (m, data) in &maps.maps {
            // Skip inconsistent maps so the saved project can reopen and bake them again
            let h = maps.height(*m);
            if h == 0 || data.len() != (r as usize) * (h as usize) * m.components() {
                continue;
            }

            let comps = m.components();
            out.push(Entry {
                name: format!("baked/{ts}_{}.bin", m.id()),
                options: deflate,
                weight: data.len() as u64 * 4,
                body: Box::new(move |w| write_texels(w, r, h, comps, data)),
            });
        }
    }
    if let Some(png) = &pf.thumbnail {
        out.push(Entry { name: "thumbnail.png".into(), options: stored, weight: png.len() as u64, body: Box::new(move |w| w.write_all(png)) });
    }
    used.clear();
    for (name, resource) in &pf.resources {
        let bytes = resource.borrow();
        out.push(Entry {
            name: unique_entry_name("resources", name, &mut used),
            options: stored,
            weight: bytes.len() as u64,
            body: Box::new(move |w| w.write_all(bytes)),
        });
    }
    graph_entries(pf, deflate, &mut out)?;
    // the images the records lay, each once however many records and stamps lay it
    let mut images = strokes::ImageTable::default();
    for (id, stored) in &pf.strokes {
        if texture_set_of(&pf.project, *id).is_none() {
            continue;
        }
        let bytes = strokes::entry_bytes(stored, &mut images).map_err(FormatError::Invalid)?;
        if bytes.is_empty() {
            continue;
        }
        out.push(Entry { name: strokes::entry_name(*id), options: deflate, weight: bytes.len() as u64, body: Box::new(move |w| w.write_all(&bytes)) });
    }
    for (key, values) in images.by_key {
        out.push(Entry {
            name: strokes::image_entry_name(&key),
            options: deflate,
            weight: values.len() as u64 * 4,
            body: Box::new(move |w| w.write_all(&strokes::encode_image(&values))),
        });
    }
    Ok(out)
}

fn write_project<W: Write + Seek>(out: W, pf: &ProjectFile<impl Borrow<Vec<u8>>>) -> Result<()> {
    let mut zip = ZipWriter::new(out);
    entries::write_entries(&mut zip, entries(pf)?)?;
    zip.finish().map_err(zip_err)?;
    Ok(())
}

/// What `cooked/index.json` holds.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct CookIndex {
    version: u32,
    /// By cook key: its outputs (output name → entry) and which of them hold linear colour.
    cooks: BTreeMap<String, CookEntry>,
    /// The cook each graph use read last.
    #[serde(default)]
    uses: Vec<(GraphUse, String)>,
}

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct CookEntry {
    outputs: BTreeMap<String, String>,
    #[serde(default)]
    linear: Vec<String>,
}

/// An output name as part of an entry name: as it is when it is plain, else hex.
fn output_entry(key: &str, output: &str) -> String {
    let plain = !output.is_empty() && output.len() <= 64 && output.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if plain { format!("cooked/{key}_{output}.bin") } else { format!("cooked/{key}_x{}.bin", output.bytes().map(|b| format!("{b:02x}")).collect::<String>()) }
}

/// The graph texts the project references and the cooks its uses read.
fn graph_entries<'a>(pf: &'a ProjectFile<impl Borrow<Vec<u8>>>, deflate: SimpleFileOptions, out: &mut Vec<Entry<'a>>) -> Result<()> {
    let graphs = &pf.graphs;
    let mut keys = pf.project.graph_keys();
    // the graphs stroke records paint, so that the strokes can be laid again
    let painted = pf.strokes.values().filter_map(StoredStrokes::record).flat_map(|r| r.graphs()).flat_map(|g| g.text_keys());
    keys.extend(painted.map(str::to_string));
    for key in &keys {
        if !texturecraft_core::is_key(key) {
            continue;
        }
        let Some(text) = graphs.texts.get(key) else { continue };
        out.push(Entry {
            name: format!("graphs/{key}.txt"),
            options: deflate,
            weight: text.len() as u64,
            body: Box::new(move |w| w.write_all(text.as_bytes())),
        });
    }
    let uses = graphs.archive_uses(&pf.project);
    if uses.is_empty() {
        return Ok(());
    }
    let mut index = CookIndex { version: COOK_INDEX_VERSION, ..CookIndex::default() };
    // cooks one at a time: a spilled one is read back only while its outputs are written
    let keys: BTreeSet<_> = uses.values().cloned().collect();
    for key in keys {
        if !texturecraft_core::is_key(&key) {
            continue;
        }
        let Some(outline) = graphs.cook_outline(&key) else { continue };
        let mut entry = CookEntry { linear: outline.linear.iter().cloned().collect(), ..CookEntry::default() };
        // the cook read once for all its outputs, and let go when the last is written
        let shared: Arc<std::sync::Mutex<(Option<Arc<Cooked>>, usize)>> = Arc::new(std::sync::Mutex::new((None, outline.outputs.len())));
        for (output, (comps, allocated)) in &outline.outputs {
            let name = output_entry(&key, output);
            let weight = (allocated * (TILE_AREA * comps * 2 + 8)) as u64;
            let (key_of_cook, name_of_output, shared) = (key.clone(), output.clone(), Arc::clone(&shared));
            out.push(Entry {
                name: name.clone(),
                options: deflate,
                weight,
                body: Box::new(move |mut w| {
                    let cooked = {
                        let mut s = shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        if s.0.is_none() {
                            s.0 = graphs.cooked(&key_of_cook);
                        }
                        let c = s.0.clone();
                        s.1 = s.1.saturating_sub(1);
                        if s.1 == 0 {
                            s.0 = None;
                        }
                        c
                    };
                    match cooked.as_ref().and_then(|c| c.outputs.get(&name_of_output)) {
                        Some(img) => write_tiles(&mut w, img),
                        None => Ok(()),
                    }
                }),
            });
            entry.outputs.insert(output.clone(), name);
        }
        index.cooks.insert(key, entry);
    }
    index.uses = uses.into_iter().filter(|(_, k)| index.cooks.contains_key(k)).collect();
    let json = serde_json::to_vec_pretty(&index).map_err(|e| FormatError::Invalid(e.to_string()))?;
    out.push(Entry { name: "cooked/index.json".into(), options: deflate, weight: json.len() as u64, body: Box::new(move |w| w.write_all(&json)) });
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Load.

pub fn load_project_bytes(bytes: &[u8]) -> Result<ProjectFile> {
    crate::guard(KIND, || load(Cursor::new(bytes)))
}

/// Open a project with its layer pixels, baked maps and cooked graph results loaded on demand.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_project(path: &std::path::Path) -> Result<ProjectFile> {
    let origin = Origin::new(path)?;
    let limit_bytes = u64::from(project_open_limit_gib()) << 30;
    crate::guard(KIND, || load_with_budget(origin.reader(), LoadBudget::new(limit_bytes), Some(&origin)))
}

/// Retain the opened archive so lazy layers in undo history survive an atomic save replacing its path.
#[derive(Clone)]
struct Origin {
    file: Arc<std::sync::Mutex<std::fs::File>>,
    len: u64,
}

impl Origin {
    #[cfg(not(target_arch = "wasm32"))]
    fn new(path: &std::path::Path) -> io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self { file: Arc::new(std::sync::Mutex::new(file)), len })
    }

    fn reader(&self) -> io::BufReader<OriginReader> {
        io::BufReader::new(OriginReader { origin: self.clone(), position: 0 })
    }

    fn open(&self) -> Result<ZipArchive<craft_zip::MetadataReader<io::BufReader<OriginReader>>>> {
        open_archive(self.reader(), MAX_ZIP_METADATA_BYTES)
    }
}

/// A separate logical cursor for each archive reader. Only the seek and read share a lock.
struct OriginReader {
    origin: Origin,
    position: u64,
}

impl Read for OriginReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut file = self.origin.file.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        file.seek(SeekFrom::Start(self.position))?;
        let len = file.read(buf)?;
        self.position += len as u64;
        Ok(len)
    }
}

impl Seek for OriginReader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.position = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
            SeekFrom::End(offset) => self.origin.len.checked_add_signed(offset),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid archive seek"))?;
        Ok(self.position)
    }
}

/// The tiles of the entry `name` (a `size` image of `comps` components) read from `origin`, slot by slot.
fn lazy_tiles(origin: &Origin, name: &str, size: (u32, u32), comps: usize) -> Option<Vec<Option<texturecraft_texel::TileData>>> {
    let mut zip = origin.open().ok()?;
    let mut f = zip.by_name(name).ok()?;
    let limit_bytes = u64::from(project_open_limit_gib()) << 30;
    let img = read_tiles(&mut f, size, comps, &mut LoadBudget::new(limit_bytes), name).ok()?;
    Some((0..img.tiles_y()).flat_map(|ty| (0..img.tiles_x()).map(move |tx| (tx, ty))).map(|(tx, ty)| img.tile_handle(tx, ty).cloned()).collect())
}

/// The image of entry `name`: its header read and checked now, its tiles from `origin` when needed.
fn lazy_image(
    f: &mut impl Read,
    origin: &Origin,
    name: &str,
    size: (u32, u32),
    comps: usize,
    declared_bytes: u64,
    budget: &mut LoadBudget,
) -> Result<TileImage> {
    budget.take(BIN_HEADER_BYTES as u64).map_err(FormatError::TooLarge)?;
    let (width, height, stored, count) = read_header(f, name)?;
    if (width, height) != size || stored != comps {
        return Err(malformed(KIND, format!("{name}: {width}×{height}×{stored} does not match the texture set ({}×{}×{comps})", size.0, size.1)));
    }
    check_tile_header(width, height, comps, count, declared_bytes, name)?;
    let (origin, entry) = (origin.clone(), name.to_string());
    Ok(TileImage::lazy(width, height, comps, count, Arc::new(move || lazy_tiles(&origin, &entry, size, comps))))
}

/// The baked map of entry `name`: its header read and checked now, its values from `origin` when needed.
fn lazy_baked(f: &mut impl Read, origin: &Origin, name: &str, declared_bytes: u64, components: usize, budget: &mut LoadBudget) -> Result<(u32, u32, Texels)> {
    budget.take(BIN_HEADER_BYTES as u64).map_err(FormatError::TooLarge)?;
    let (width, height, stored, _) = read_header(f, name)?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_TEXELS || stored != components {
        return Err(malformed(KIND, format!("{name}: bad size {width}×{height}×{stored}")));
    }
    let count = u64::from(width) * u64::from(height) * components as u64;
    if declared_bytes != BIN_HEADER_BYTES as u64 + count * 4 {
        return Err(malformed(KIND, format!("{name}: {declared_bytes} bytes for a {width}×{height}×{components} map")));
    }
    let (origin, entry) = (origin.clone(), name.to_string());
    let source = Arc::new(move || {
        let mut zip = origin.open().ok()?;
        let mut f = zip.by_name(&entry).ok()?;
        let limit_bytes = u64::from(project_open_limit_gib()) << 30;
        read_baked(&mut f, declared_bytes, components, &mut LoadBudget::new(limit_bytes), &entry).ok().map(|(_, _, values)| values)
    });
    Ok((width, height, Texels::lazy(count as usize, source)))
}

/// Read a native smart material in place. MaterialCraft packages go to the bounded cook reader.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_smart_material_project(file: std::fs::File) -> Result<Option<ProjectFile>> {
    crate::guard(KIND, || {
        let archive = open_archive(std::io::BufReader::new(file), MAX_ZIP_METADATA_BYTES)?;
        if archive.index_for_name(craft_mcmat::MANIFEST_ENTRY).is_some()
            || archive.index_for_name("package.json").is_some()
            || archive.file_names().any(|name| name.is_ok_and(|name| crate::mcmat::is_root_sbs(&name)))
        {
            return Ok(None);
        }

        let budget = LoadBudget::new(u64::from(project_open_limit_gib()) << 30);
        load_archive(archive, budget, None).map(Some)
    })
}

fn read_all(reader: &mut impl Read, cap: u64, budget: &mut LoadBudget, what: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();

    // Read at most one byte beyond either limit to detect overflow without a full entry allocation
    let read_limit = cap.min(budget.remaining()).saturating_add(1);
    reader.take(read_limit).read_to_end(&mut out).map_err(|e| malformed(KIND, format!("{what}: {e}")))?;
    if out.len() as u64 > cap {
        return Err(FormatError::TooLarge(format!("{what} is larger than {cap} bytes")));
    }

    budget.take(out.len() as u64).map_err(FormatError::TooLarge)?;
    Ok(out)
}

/// Transfer the bytes charged by the shared codec to this project's one load budget.
fn codec_result<T>(result: std::result::Result<T, craft_tcraft::Error>, shared: &craft_zip::Budget, budget: &mut LoadBudget, what: &str) -> Result<T> {
    budget.take(shared.used()).map_err(FormatError::TooLarge)?;
    result.map_err(|error| match error {
        craft_tcraft::Error::Budget(_) => FormatError::TooLarge(budget.exceeded_message()),
        craft_tcraft::Error::Limit(message) => FormatError::TooLarge(format!("{what}: {message}")),
        craft_tcraft::Error::NewerVersion(found) => FormatError::NewerVersion { found, supported: craft_tcraft::VERSION },
        other => malformed(KIND, format!("{what}: {other}")),
    })
}

/// Read the layout without retaining the payload of a lazy entry.
fn read_header(reader: &mut impl Read, name: &str) -> Result<(u32, u32, usize, usize)> {
    let mut header = [0u8; BIN_HEADER_BYTES];
    reader.read_exact(&mut header).map_err(|error| malformed(KIND, format!("{name}: {error}")))?;
    let word = |index: usize| {
        let offset = index * 4;
        u32::from_le_bytes([header[offset], header[offset + 1], header[offset + 2], header[offset + 3]])
    };
    if header.get(..4) != Some(MAGIC.as_slice()) {
        return Err(malformed(KIND, format!("{name}: bad magic")));
    }
    let version = word(1);
    if version > BIN_VERSION {
        return Err(FormatError::NewerVersion { found: version, supported: BIN_VERSION });
    }
    Ok((word(2), word(3), word(4) as usize, word(5) as usize))
}

fn check_tile_header(w: u32, h: u32, comps: usize, count: usize, declared_bytes: u64, name: &str) -> Result<()> {
    let max_tiles = u64::from(w.div_ceil(TILE)) * u64::from(h.div_ceil(TILE));
    if count as u64 > max_tiles {
        return Err(malformed(KIND, format!("{name}: {count} tiles is more than the image holds")));
    }
    let expected_bytes = BIN_HEADER_BYTES as u64 + count as u64 * (TILE_AREA * comps * 2 + 8) as u64;
    if declared_bytes != expected_bytes {
        return Err(malformed(KIND, format!("{name}: {declared_bytes} bytes for {count} tiles, expected {expected_bytes}")));
    }
    Ok(())
}

fn read_tiles(reader: &mut impl Read, size: (u32, u32), components: usize, budget: &mut LoadBudget, what: &str) -> Result<TileImage> {
    let layout = craft_tcraft::Layout { width: size.0, height: size.1, components };
    let mut shared = craft_zip::Budget::new("project", budget.remaining());
    let mut image = None;
    let result = craft_tcraft::read_tiles(reader, layout, MAX_TEXELS, &mut shared, |x, y, values| {
        image.get_or_insert_with(|| TileImage::new(size.0, size.1, components)).set_tile(x, y, Some(Arc::new(values)));
        Ok(())
    });
    codec_result(result, &shared, budget, what)?;
    Ok(image.unwrap_or_else(|| TileImage::new(size.0, size.1, components)))
}

fn read_baked(reader: &mut impl Read, declared_bytes: u64, components: usize, budget: &mut LoadBudget, what: &str) -> Result<(u32, u32, Vec<f32>)> {
    let mut shared = craft_zip::Budget::new("project", budget.remaining());
    let result = craft_tcraft::read_baked(reader, declared_bytes, components, MAX_TEXELS, &mut shared);
    let (layout, values) = codec_result(result, &shared, budget, what)?;
    Ok((layout.width, layout.height, values))
}

/// One cook of the index read whole from the archive, or `None` when an output is missing or malformed.
fn read_cook<R: Read + Seek>(zip: &mut ZipArchive<R>, entry: &CookEntry, budget: &mut LoadBudget) -> Option<Cooked> {
    let mut outputs = BTreeMap::new();
    for (output, file) in &entry.outputs {
        let (w, h, comps, _) = cook_output_head(zip, file).ok()?;
        let mut f = zip.by_name(file).ok()?;
        outputs.insert(output.clone(), read_tiles(&mut f, (w, h), comps, budget, file).ok()?);
    }
    let linear = entry.linear.iter().filter(|l| outputs.contains_key(*l)).cloned().collect();
    Some(Cooked::new(outputs, linear))
}

/// The size, components and tile count the cooked output entry `file` declares.
fn cook_output_head<R: Read + Seek>(zip: &mut ZipArchive<R>, file: &str) -> Result<(u32, u32, usize, usize)> {
    if !file.starts_with("cooked/") || file.contains("..") {
        return Err(malformed(KIND, format!("bad cooked entry {file}")));
    }
    let mut f = zip.by_name(file).map_err(zip_err)?;
    let declared_bytes = f.size();
    let (w, h, comps, count) = read_header(&mut f, file)?;
    if w == 0 || h == 0 || w > MAX_COOK_SIDE || h > MAX_COOK_SIDE || !matches!(comps, 1 | 4) {
        return Err(malformed(KIND, format!("{file}: bad size {w}×{h}×{comps}")));
    }
    check_tile_header(w, h, comps, count, declared_bytes, file)?;
    Ok((w, h, comps, count))
}

/// The cooks `index` lists, each whole or not at all; a cook with a missing or malformed output is left out with a
/// warning (it is rendered again when it is needed). With `lazy` (the file the archive is read from) the cooks are
/// not read now: the store is told what the file holds and reads each when it is needed.
fn load_cooks<R: Read + Seek>(zip: &mut ZipArchive<R>, index: CookIndex, pf: &mut ProjectFile, budget: &mut LoadBudget, lazy: Option<&Origin>) {
    let mut dropped = 0usize;
    let mut kept: BTreeMap<String, CookEntry> = BTreeMap::new();
    let mut outlines = std::collections::HashMap::new();
    for (key, entry) in index.cooks {
        if !texturecraft_core::is_key(&key) || entry.outputs.is_empty() {
            dropped += 1;
            continue;
        }
        if lazy.is_some() {
            // what the cook holds, from its outputs' headers
            let heads: Option<BTreeMap<String, (usize, usize)>> = entry
                .outputs
                .iter()
                .map(|(output, file)| cook_output_head(zip, file).ok().map(|(_, _, comps, count)| (output.clone(), (comps, count))))
                .collect();
            match heads {
                Some(outputs) => {
                    let linear = entry.linear.iter().filter(|l| outputs.contains_key(*l)).cloned().collect();
                    outlines.insert(key.clone(), texturecraft_compose::graphs::spill::CookOutline { outputs, linear });
                    kept.insert(key, entry);
                }
                None => dropped += 1,
            }
            continue;
        }
        match read_cook(zip, &entry, budget) {
            Some(cooked) => pf.graphs.insert_cooked(key, cooked),
            None => dropped += 1,
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(origin) = lazy
        && !kept.is_empty()
    {
        pf.graphs.read_cooks_from(Arc::new(FileCooks { origin: origin.clone(), cooks: kept }), outlines);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (kept, outlines);
    pf.graphs.set_archive_uses(&pf.project, index.uses.into_iter().collect());
    if dropped > 0 {
        pf.warnings.push(format!("Note: {dropped} cooked graph result(s) could not be read; they will be rendered again when needed."));
    }
}

/// Read cooked graph results on demand from the archive opened by this document, including after a save replaces it.
#[cfg(not(target_arch = "wasm32"))]
struct FileCooks {
    origin: Origin,
    cooks: BTreeMap<String, CookEntry>,
}

#[cfg(not(target_arch = "wasm32"))]
impl texturecraft_compose::graphs::spill::CookSource for FileCooks {
    fn get(&self, key: &str) -> Option<Cooked> {
        let entry = self.cooks.get(key)?;
        let limit_bytes = u64::from(project_open_limit_gib()) << 30;
        crate::guard(KIND, || {
            let mut zip = self.origin.open()?;
            Ok(read_cook(&mut zip, entry, &mut LoadBudget::new(limit_bytes)))
        })
        .ok()
        .flatten()
    }
}

/// Texture set holding a paint layer, a content effect or a mask effect id.
fn texture_set_of(project: &Project, id: LayerId) -> Option<usize> {
    let owner = if project.layers.contains_key(&id) { id } else { project.effect(id)?.0 };
    project.locate(owner).map(|(ts, _, _)| ts)
}

fn load<R: Read + Seek>(reader: R) -> Result<ProjectFile> {
    let limit_bytes = u64::from(project_open_limit_gib()) << 30;
    load_with_budget(reader, LoadBudget::new(limit_bytes), None)
}

/// Whether a lazy open reads only the header of entry `name` and the rest when a composite or a save needs it (layer
/// and mask pixels, baked maps, cooked graph outputs). Each later read checks that entry on its own against the limit.
fn read_lazily(name: &str) -> bool {
    let bin = name.ends_with(".bin");
    bin && (name.starts_with("pixels/layer_") || name.starts_with("pixels/mask_") || name.starts_with("baked/") || name.starts_with("cooked/"))
}

/// Checks the archive before any decompression. With `lazy` an entry read lazily is charged its header only: the open
/// does not hold the rest, so the limit counts what the open actually holds.
fn check_archive<R: Read + Seek>(zip: &mut ZipArchive<R>, limit_bytes: u64, lazy: bool) -> Result<()> {
    let declared_limit = if lazy { u64::MAX } else { limit_bytes };
    craft_zip::inspect_seekable(zip, MAX_ENTRIES, declared_limit).map_err(|error| match error {
        craft_zip::InspectError::TooManyEntries { .. } => FormatError::TooLarge(format!("project file has more than {MAX_ENTRIES} entries")),
        craft_zip::InspectError::DeclaredBytesExceeded { .. } => FormatError::TooLarge(LoadBudget::new(limit_bytes).exceeded_message()),
        craft_zip::InspectError::OverlappingEntries => malformed(KIND, "overlapping ZIP entries are not supported"),
        craft_zip::InspectError::Zip(error) => zip_err(error),
    })?;
    if !lazy {
        return Ok(());
    }
    let mut declared_budget = LoadBudget::new(limit_bytes);
    for index in 0..zip.len() {
        let entry = zip.by_index_raw(index).map_err(zip_err)?;
        let held = if entry.name().is_ok_and(|name| read_lazily(&name)) { entry.size().min(BIN_HEADER_BYTES as u64) } else { entry.size() };
        declared_budget.take(held).map_err(FormatError::TooLarge)?;
    }
    Ok(())
}

fn open_archive<R: Read + Seek>(reader: R, metadata_bytes: u64) -> Result<ZipArchive<craft_zip::MetadataReader<R>>> {
    let (reader, indexing) = craft_zip::MetadataReader::new(reader, metadata_bytes)?;
    // Project archives start at byte zero; avoid scanning payloads to guess an embedded ZIP offset
    let config = zip::read::Config { archive_offset: zip::read::ArchiveOffset::Known(0) };
    let archive = ZipArchive::with_config(config, reader).map_err(zip_err)?;
    indexing.finish();
    Ok(archive)
}

fn load_with_budget<R: Read + Seek>(reader: R, budget: LoadBudget, lazy: Option<&Origin>) -> Result<ProjectFile> {
    load_archive(open_archive(reader, MAX_ZIP_METADATA_BYTES)?, budget, lazy)
}

fn load_archive<R: Read + Seek>(mut zip: ZipArchive<R>, mut budget: LoadBudget, lazy: Option<&Origin>) -> Result<ProjectFile> {
    check_archive(&mut zip, budget.remaining(), lazy.is_some())?;

    let json = read_all(&mut zip.by_name("project.json").map_err(zip_err)?, MAX_JSON, &mut budget, "project.json")?;
    let version = craft_tcraft::project_version(&json).map_err(|e| malformed(KIND, format!("project.json: {e}")))?;
    if version > FORMAT_VERSION {
        return Err(FormatError::NewerVersion { found: version, supported: FORMAT_VERSION });
    }

    let (project, mut recovered) = recovery::read_project(&json).map_err(|e| malformed(KIND, format!("project.json: {e}")))?;
    project.validate()?;
    let too_large = |t: &texturecraft_core::TextureSet| u64::from(t.size().0) * u64::from(t.size().1) > MAX_TEXELS;
    if let Some(ts) = project.texture_sets.iter().find(|t| t.resolution == 0 || t.resolution > MAX_IMAGE_SIZE || too_large(t)) {
        return Err(malformed(KIND, format!("texture set \"{}\" has resolution {} over {} UV tiles", ts.name, ts.resolution, ts.uv_tiles.len().max(1))));
    }

    let mut pf = ProjectFile {
        pixels: PixelStore::default(),
        stroke_replay: None,
        baked: BTreeMap::new(),
        mesh: None,
        high_poly: Vec::new(),
        cage: None,
        bake_paint: BTreeMap::new(),
        thumbnail: None,
        resources: BTreeMap::new(),
        graphs: GraphStore::default(),
        strokes: BTreeMap::new(),
        warnings: Vec::new(),
        project,
    };
    let mut unreadable_strokes = 0usize;
    let mut dropped_strokes = 0usize;
    let mut stroke_entries = Vec::new();
    let mut stroke_images = std::collections::HashMap::new();
    let mut cook_index: Option<CookIndex> = None;

    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(zip_err)?;
        let declared_bytes = f.size();
        let name = f.name().map_err(zip_err)?.into_owned();
        let size_of = |id: LayerId| texture_set_of(&pf.project, id).and_then(|ts| pf.project.texture_sets.get(ts)).map(|t| t.size());

        // Unrecognized entries from newer minor revisions may be skipped
        if name == "thumbnail.png" {
            pf.thumbnail = Some(read_all(&mut f, MAX_THUMBNAIL, &mut budget, &name)?);
        } else if name == STROKE_REPLAY_ENTRY {
            if declared_bytes > MAX_STROKE_REPLAY {
                pf.warnings.push("Paint stroke data exceeds the supported size. The project's raster paint is available.".into());
            } else {
                pf.stroke_replay = Some(StrokeReplayData::loaded(read_all(&mut f, MAX_STROKE_REPLAY, &mut budget, &name)?));
            }
        } else if let Some(key) = name.strip_prefix("graphs/").and_then(|n| n.strip_suffix(".txt")) {
            if !texturecraft_core::is_key(key) {
                return Err(malformed(KIND, format!("bad graph entry {name}")));
            }
            let bytes = read_all(&mut f, MAX_GRAPH_TEXT, &mut budget, &name)?;
            let text = String::from_utf8(bytes).map_err(|_| malformed(KIND, format!("{name} is not text")))?;
            if texturecraft_core::text_key(&text) != key {
                return Err(malformed(KIND, format!("{name} does not match its key")));
            }
            pf.graphs.texts.insert(key.to_string(), Arc::from(text));
        } else if name == "cooked/index.json" {
            let json = read_all(&mut f, MAX_COOK_INDEX, &mut budget, &name)?;
            match serde_json::from_slice::<CookIndex>(&json) {
                Ok(index) if index.version <= COOK_INDEX_VERSION => cook_index = Some(index),
                _ => pf.warnings.push("Note: the project's cooked graph results could not be read; graphs will be rendered again.".into()),
            }
        } else if let Some(res_name) = name.strip_prefix("resources/") {
            if res_name.is_empty() || sanitize_resource_name(res_name) != res_name {
                return Err(malformed(KIND, format!("bad resource name {name}")));
            }

            let bytes = read_all(&mut f, MAX_RESOURCE, &mut budget, &name)?;
            pf.resources.insert(res_name.to_string(), bytes);
        } else if let Some((dir, file)) = name.split_once('/').filter(|(d, _)| *d == "highpoly" || *d == "cage") {
            if file.is_empty() || sanitize_resource_name(file) != file {
                return Err(malformed(KIND, format!("bad mesh name {name}")));
            }

            let entry = (file.to_string(), read_all(&mut f, crate::mesh_import::MAX_MESH_FILE_BYTES, &mut budget, &name)?);
            match dir {
                "cage" => pf.cage = Some(entry),
                _ => pf.high_poly.push(entry),
            }
        } else if let Some(rest) = name.strip_prefix("bake_paint/").and_then(|name| name.strip_suffix(".bin")) {
            let (texture_set, target) = rest.split_once('_').ok_or_else(|| malformed(KIND, format!("bad entry {name}")))?;
            let texture_set: usize = texture_set.parse().map_err(|_| malformed(KIND, format!("bad entry {name}")))?;
            if texture_set >= pf.project.texture_sets.len() {
                return Err(malformed(KIND, format!("unknown texture set in {name}")));
            }

            let target = match target {
                "cage" => BakePaintTarget::Cage,
                "skew" => BakePaintTarget::Skew,
                _ => return Err(malformed(KIND, format!("unknown bake paint target in {name}"))),
            };
            let (width, height, values) = read_baked(&mut f, declared_bytes, 1, &mut budget, &name)?;
            pf.bake_paint.insert((texture_set, target), BakePaintData { width, height, values });
        } else if let Some(ext) = name.strip_prefix("mesh.").filter(|e| !e.contains('/')) {
            let bytes = read_all(&mut f, crate::mesh_import::MAX_MESH_FILE_BYTES, &mut budget, &name)?;
            pf.mesh = Some((ext.to_string(), bytes));
        } else if let Some(rest) = name.strip_prefix("pixels/layer_").and_then(|r| r.strip_suffix(".bin")) {
            let (id, chan) = rest.split_once('_').ok_or_else(|| malformed(KIND, format!("bad entry name {name}")))?;
            let id = LayerId(id.parse().map_err(|_| malformed(KIND, format!("bad entry name {name}")))?);
            let c = Channel::ALL.into_iter().find(|c| c.id() == chan).ok_or_else(|| malformed(KIND, format!("unknown channel in {name}")))?;

            // Drop pixels of layers that are no longer in any stack
            let Some(size) = size_of(id) else { continue };

            let img = match lazy {
                Some(origin) => lazy_image(&mut f, origin, &name, size, c.components() + 1, declared_bytes, &mut budget)?,
                None => read_tiles(&mut f, size, c.components() + 1, &mut budget, &name)?,
            };
            pf.pixels.layers.insert((id, c), img);
        } else if let Some(rest) = name.strip_prefix("pixels/mask_").and_then(|r| r.strip_suffix(".bin")) {
            let id = LayerId(rest.parse().map_err(|_| malformed(KIND, format!("bad entry name {name}")))?);
            let Some(size) = size_of(id) else { continue };

            let img = match lazy {
                Some(origin) => lazy_image(&mut f, origin, &name, size, 2, declared_bytes, &mut budget)?,
                None => read_tiles(&mut f, size, 2, &mut budget, &name)?,
            };
            pf.pixels.masks.insert(id, img);
        } else if let Some(id) = strokes::entry_id(&name) {
            let id = id.ok_or_else(|| malformed(KIND, format!("bad entry name {name}")))?;
            // Drop records of pixels that are no longer in any stack
            if size_of(id).is_none() {
                continue;
            }
            // A record larger than any this build writes (older builds kept every stamp's tip inside it) is left
            // out: the project still opens, its pixels unchanged
            if declared_bytes > strokes::MAX_ENTRY {
                dropped_strokes += 1;
                continue;
            }
            // read once the images they name are known (entries come in any order)
            stroke_entries.push((id, read_all(&mut f, strokes::MAX_ENTRY, &mut budget, &name)?));
        } else if let Some(key) = strokes::image_entry_key(&name) {
            let key = key.ok_or_else(|| malformed(KIND, format!("bad entry name {name}")))?;
            // an image that cannot be read leaves the records naming it unreadable (kept as they are)
            if let Ok(values) = strokes::decode_image(key, &read_all(&mut f, strokes::MAX_IMAGE_ENTRY, &mut budget, &name)?) {
                stroke_images.insert(key.to_string(), values);
            }
        } else if let Some(rest) = name.strip_prefix("baked/").and_then(|r| r.strip_suffix(".bin")) {
            let (ts, map) = rest.split_once('_').ok_or_else(|| malformed(KIND, format!("bad entry name {name}")))?;
            let ts: usize = ts.parse().map_err(|_| malformed(KIND, format!("bad entry name {name}")))?;
            let Some(m) = recovery::baked_map(map) else {
                recovered = true;
                continue;
            };

            if ts >= pf.project.texture_sets.len() {
                continue;
            }

            let (res, height, data) = match lazy {
                Some(origin) => lazy_baked(&mut f, origin, &name, declared_bytes, m.components(), &mut budget)?,
                None => {
                    let (res, height, values) = read_baked(&mut f, declared_bytes, m.components(), &mut budget, &name)?;
                    (res, height, Texels::new(values))
                }
            };
            let maps = pf.baked.entry(ts).or_default();
            if maps.maps.is_empty() {
                maps.resolution = res;
            } else if maps.resolution != res || maps.maps.keys().next().is_some_and(|k| maps.height(*k) != height) {
                return Err(malformed(KIND, format!("{name}: baked maps of one texture set differ in size")));
            }

            maps.maps.insert(m, Arc::new(data));
        }
        // Ignore other entries from newer minor revisions (cooked outputs are read through their index below)
    }
    if let Some(index) = cook_index {
        load_cooks(&mut zip, index, &mut pf, &mut budget, lazy);
    }

    for (id, bytes) in stroke_entries {
        let stored = strokes::read(bytes, &mut stroke_images);
        unreadable_strokes += usize::from(matches!(stored, StoredStrokes::Unreadable(_)));
        pf.strokes.insert(id, stored);
    }
    if version < TANGENT_BENT_NORMAL_VERSION {
        drop_world_space_bent_normals(&mut pf);
    }
    pf.project.format_version = FORMAT_VERSION;
    if dropped_strokes > 0 {
        pf.warnings
            .push(format!("Note: {dropped_strokes} stroke record(s) larger than {} MiB were left out; the pixels are unchanged.", strokes::MAX_ENTRY >> 20));
    }
    if unreadable_strokes > 0 {
        pf.warnings
            .push(format!("Note: {unreadable_strokes} stroke record(s) could not be read; the pixels are unchanged and the records are kept as they are."));
    }

    if recovered {
        pf.warnings.push(
            "Warning: Some project settings or mesh maps could not be imported. TextureCraft recovered what it could, using defaults where needed.".into(),
        );
    }

    Ok(pf)
}

/// Older files hold bent normal maps in world space, which every reader now takes as tangent
/// space: drop them (the bake settings stay) and ask for a re-bake.
fn drop_world_space_bent_normals(pf: &mut ProjectFile) {
    // Pixels and declaration go for every set, either one alone is enough to name the set in the warning
    let mut names = Vec::new();
    for (i, set) in pf.project.texture_sets.iter_mut().enumerate() {
        let had_pixels = pf.baked.get_mut(&i).is_some_and(|maps| maps.maps.remove(&BakedMap::BentNormal).is_some());
        if set.baked.remove(&BakedMap::BentNormal) || had_pixels {
            names.push(format!("\"{}\"", set.name));
        }
    }
    pf.baked.retain(|_, maps| !maps.maps.is_empty());
    if !names.is_empty() {
        pf.warnings.push(format!(
            "Warning: This project was saved with bent normal maps in world space. TextureCraft now bakes them in tangent space and removed the old ones from texture set {}. Bake the Bent Normals map again, or set its image again if it came from a file.",
            names.join(", ")
        ));
    }
}

#[cfg(test)]
mod tests;
