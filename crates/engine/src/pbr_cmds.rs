//! `file.importPbrFolder` and `file.scanPbrFolder`: a folder (or selected files, or a `.zip`) of
//! PBR texture maps becomes one fill layer, like `file.importMaterialX` does for a `.mtlx`.
//!
//! The maps are found by file name ([`texturecraft_formats::pbr_maps`]: albedo, roughness,
//! metallic, normal with its OpenGL / DirectX convention, height, occlusion, emissive, opacity,
//! packed ORM). A packed ORM image is split into three single-channel resources, a glossiness
//! map is inverted into roughness, and every other map keeps its original bytes as the resource.
//! The folder is untrusted: symbolic links and other non-regular files are not followed, every
//! file and the whole import are size-capped, and an image that cannot be decoded is skipped with a
//! warning while the rest is imported. The whole import is one undo step. A programmatic call never
//! asks: when several files name one map the best is used and the others are listed in
//! `candidates`, so a caller can scan first (`file.scanPbrFolder`) and pass `choose`.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use texturecraft_core::{Channel, Fill, Layer, LayerKind, NormalFormat, Projection, Source};
use texturecraft_formats::pbr_maps::{self, MapKind, PbrSet};

use crate::commands::{CommandSpec, always, bad, strp};
use crate::{EngineError, Result, Session, cmd};

/// Most files looked at in one folder or archive.
pub const MAX_FILES: usize = 4096;
/// Largest single image file read.
pub const MAX_FILE_BYTES: u64 = 256 << 20;
/// Largest decoded image (width × height).
pub const MAX_IMAGE_TEXELS: u64 = 64 << 20;
/// Largest sum of decoded texels in one import.
pub const MAX_TOTAL_TEXELS: u64 = 320 << 20;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.importPbrFolder",
            "Import PBR texture folder...",
            ["File"],
            None,
            "{path: a folder or .zip of PBR maps, or files: [paths], name?: layer name, normalFormat?: auto|openGl|directX (auto: from _gl/_dx in the name, else openGl; a value forces it), projection?: uv|triplanar|planar..., scale?, choose?: {map id: file} for maps with several candidates} → {layer, name, maps, candidates, normalFormat, channels, resources, ignored, warnings}: one fill layer; maps are found by file name (baseColor, roughness, glossiness, metallic, normal, height, ambientOcclusion, emissive, opacity, orm); packed ORM is split, glossiness inverted",
            always,
            import
        ),
        cmd!(
            "file.scanPbrFolder",
            "Scan PBR texture folder",
            [],
            None,
            "{path | files} → {name, maps, candidates, normalFormat, ignored}: what file.importPbrFolder would use, changing nothing",
            always,
            scan
        ),
    ]
}

/// One file offered to the importer: its name (folders joined by `/`) and size.
pub type FileList = Vec<(String, u64)>;
/// Reads a file of the list by name.
pub type Loader<'a> = &'a dyn Fn(&str) -> std::result::Result<Vec<u8>, String>;

/// How to import.
#[derive(Clone, Debug, Default)]
pub struct PbrOptions {
    pub name: Option<String>,
    /// Forces every normal map's convention; `None` reads it from the file name (default OpenGL).
    pub normal_format: Option<NormalFormat>,
    pub projection: Projection,
    /// Files chosen by the caller where several name one map.
    pub choose: BTreeMap<MapKind, String>,
}

/// The detection for a file list with the caller's choices applied.
fn detect_with(files: &FileList, choose: &BTreeMap<MapKind, String>) -> Result<PbrSet> {
    let mut set = pbr_maps::detect(files.iter().map(|(n, s)| (n.as_str(), *s)));
    for (kind, file) in choose {
        let list = set.candidates.get(kind).ok_or_else(|| EngineError::Other(format!("choose: no file names the map {}", kind.id())))?;
        let pick =
            list.iter().find(|c| c.name == *file).ok_or_else(|| EngineError::Other(format!("choose: {file} is not a candidate for {}", kind.id())))?.clone();
        // The previously chosen file becomes an ignored one, the picked one stops being ignored.
        if let Some(old) = set.chosen.insert(*kind, pick.clone())
            && old.name != pick.name
        {
            set.ignored.retain(|(n, _)| *n != pick.name);
            set.ignored.push((old.name, format!("another file is used for {}", kind.id())));
        }
    }
    Ok(set)
}

fn format_name(f: NormalFormat) -> &'static str {
    match f {
        NormalFormat::OpenGl => "openGl",
        NormalFormat::DirectX => "directX",
    }
}

/// The convention used for the chosen normal map.
fn normal_format_for(set: &PbrSet, forced: Option<NormalFormat>) -> NormalFormat {
    forced.or_else(|| set.chosen.get(&MapKind::Normal).and_then(|c| c.detected.normal)).unwrap_or(NormalFormat::OpenGl)
}

fn report(set: &PbrSet, forced: Option<NormalFormat>) -> Value {
    let maps: BTreeMap<&str, &str> = set.chosen.iter().map(|(k, c)| (k.id(), c.name.as_str())).collect();
    let candidates: BTreeMap<&str, Vec<&str>> =
        set.candidates.iter().filter(|(_, v)| v.len() > 1).map(|(k, v)| (k.id(), v.iter().map(|c| c.name.as_str()).collect())).collect();
    json!({
        "maps": maps,
        "candidates": candidates,
        "normalFormat": format_name(normal_format_for(set, forced)),
        "ignored": set.ignored.iter().map(|(n, why)| json!([n, why])).collect::<Vec<_>>(),
    })
}

/// A file's bytes and its decoded image.
type Loaded = (Arc<Vec<u8>>, texturecraft_formats::DecodedImage);

/// How a channel's image is made from the file chosen for a map.
#[derive(Clone, Copy, PartialEq)]
enum Take {
    /// The file as it is.
    Whole,
    /// One colour channel (0 = red) of a packed image.
    Channel(usize),
    /// 1 − red: glossiness as roughness.
    Inverted,
}

impl Session {
    /// Import the maps found among `files` as one fill layer on the active texture set; `load`
    /// reads a file by name. Returns the reply of `file.importPbrFolder`.
    pub fn import_pbr(&mut self, files: &FileList, load: Loader, opts: &PbrOptions) -> Result<Value> {
        let set = detect_with(files, &opts.choose)?;
        if set.is_empty() {
            return Err(EngineError::Other(format!(
                "file.importPbrFolder: no PBR texture maps found among {} files (names such as albedo, roughness, metallic, normal, height, ao)",
                files.len()
            )));
        }
        let ts = self.ts();
        let available = self.doc.project.texture_set(ts).map(|t| t.channel_list()).unwrap_or_default();
        let nf = normal_format_for(&set, opts.normal_format);
        let mut warnings: Vec<String> = Vec::new();

        // Which file feeds which channel, and how.
        let mut plan: Vec<(Channel, MapKind, Take)> = Vec::new();
        let has = |k: MapKind| set.chosen.contains_key(&k);
        let direct = [
            (MapKind::BaseColor, Channel::BaseColor),
            (MapKind::Normal, Channel::Normal),
            (MapKind::Height, Channel::Height),
            (MapKind::Emissive, Channel::Emissive),
            (MapKind::Opacity, Channel::Opacity),
        ];
        for (kind, channel) in direct {
            if has(kind) {
                plan.push((channel, kind, Take::Whole));
            }
        }
        // Separate files win; a packed image fills what they leave open (R = occlusion, G = roughness, B = metallic).
        let orm = has(MapKind::PackedOrm);
        for (channel, kind, index) in
            [(Channel::AmbientOcclusion, MapKind::AmbientOcclusion, 0), (Channel::Roughness, MapKind::Roughness, 1), (Channel::Metallic, MapKind::Metallic, 2)]
        {
            if has(kind) {
                plan.push((channel, kind, Take::Whole));
            } else if channel == Channel::Roughness && has(MapKind::Glossiness) {
                plan.push((channel, MapKind::Glossiness, Take::Inverted));
            } else if orm {
                plan.push((channel, MapKind::PackedOrm, Take::Channel(index)));
            }
        }
        if orm && plan.iter().all(|(_, k, _)| *k != MapKind::PackedOrm) {
            warnings.push("the packed ORM image was not used: separate occlusion, roughness and metallic maps exist".into());
        }
        if has(MapKind::Glossiness) && has(MapKind::Roughness) {
            warnings.push("the glossiness map was not used: a roughness map exists".into());
        }

        let mut fill = Fill { projection: opts.projection.clone(), ..Fill::default() };
        let mut added: Vec<(String, Arc<Vec<u8>>)> = Vec::new();
        let mut decoded = Vec::new();
        let mut channels: Vec<Channel> = Vec::new();
        let mut total_texels = 0u64;
        // Decoded source files by name (a packed image feeds three channels).
        let mut sources: BTreeMap<String, Option<Loaded>> = BTreeMap::new();
        // Resource per (file, how) so a file used twice is stored once.
        let mut made: BTreeMap<(String, usize), String> = BTreeMap::new();

        let mut uses: BTreeMap<String, usize> = BTreeMap::new();
        for (_, kind, _) in &plan {
            if let Some(c) = set.chosen.get(kind) {
                *uses.entry(c.name.clone()).or_default() += 1;
            }
        }
        for (channel, kind, take) in plan {
            if !available.contains(&channel) {
                warnings.push(format!("{} was skipped: this texture set has no such channel", channel.id()));
                continue;
            }
            let Some(chosen) = set.chosen.get(&kind) else { continue };
            let file = chosen.name.clone();
            let slot = match take {
                Take::Whole => 9,
                Take::Inverted => 8,
                Take::Channel(c) => c,
            };
            let resource = if let Some(name) = made.get(&(file.clone(), slot)) {
                name.clone()
            } else {
                if !sources.contains_key(&file) {
                    let loaded = load(&file).and_then(|bytes| {
                        if bytes.len() as u64 > MAX_FILE_BYTES {
                            return Err(format!("the file is over {} MB", MAX_FILE_BYTES >> 20));
                        }
                        let image = texturecraft_formats::decode_image_within(&bytes, MAX_IMAGE_TEXELS).map_err(|e| e.to_string())?;
                        total_texels = total_texels.saturating_add(u64::from(image.width) * u64::from(image.height));
                        if total_texels > MAX_TOTAL_TEXELS {
                            return Err("the textures together are too large to import".into());
                        }
                        Ok((Arc::new(bytes), image))
                    });
                    match loaded {
                        Ok(v) => sources.insert(file.clone(), Some(v)),
                        Err(e) => {
                            warnings.push(format!("{} was skipped: {file}: {e}", channel.id()));
                            sources.insert(file.clone(), None)
                        }
                    };
                }
                let remaining = uses.entry(file.clone()).or_default();
                *remaining = remaining.saturating_sub(1);
                let last_use = *remaining == 0;
                let leaf = file.rsplit(['/', '\\']).next().unwrap_or(&file).to_string();
                let stem = leaf.rsplit_once('.').map_or(leaf.as_str(), |(s, _)| s).to_string();
                let (res_name, res_bytes, res_image) = match take {
                    Take::Whole => {
                        // The file itself; its decoded image moves into the session on its last use.
                        let entry = if last_use { sources.remove(&file).flatten() } else { sources.get(&file).cloned().flatten() };
                        let Some((bytes, image)) = entry else { continue };
                        (leaf, bytes, image)
                    }
                    Take::Channel(_) | Take::Inverted => {
                        let Some(Some((_, image))) = sources.get(&file) else { continue };
                        let (c, suffix, invert) = match take {
                            Take::Channel(c) => (c, ["ao", "roughness", "metallic"].get(c).copied().unwrap_or("channel"), false),
                            _ => (0, "roughness", true),
                        };
                        let values = image.rgba.iter().map(|p| {
                            let v = p.get(c).copied().unwrap_or(0.0);
                            if invert { 1.0 - v } else { v }
                        });
                        let png = texturecraft_formats::gray16_png(image.width, image.height, values).map_err(|e| EngineError::Other(e.to_string()))?;
                        let again = texturecraft_formats::decode_image(&png).map_err(|e| EngineError::Other(e.to_string()))?;
                        (format!("{stem}_{suffix}.png"), Arc::new(png), again)
                    }
                };
                let base = texturecraft_formats::sanitize_resource_name(&res_name);
                let (mut unique, mut n) = (base.clone(), 2);
                while self.doc.resources.contains_key(&unique) || added.iter().any(|(a, _)| *a == unique) {
                    unique = format!("{n}_{base}");
                    n += 1;
                }
                added.push((unique.clone(), res_bytes));
                decoded.push((unique.clone(), crate::resource_cmds::decoded(res_image)));
                made.insert((file.clone(), slot), unique.clone());
                unique
            };
            let source = if channel == Channel::Normal { Source::Image { name: resource, normal_format: nf } } else { Source::image(resource) };
            fill.sources.insert(channel, source);
            channels.push(channel);
        }
        if channels.is_empty() {
            let why = if warnings.is_empty() { String::new() } else { format!(" ({})", warnings.join("; ")) };
            return Err(EngineError::Other(format!("file.importPbrFolder: nothing could be imported{why}")));
        }
        let base_name = opts.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "PBR material".to_string());
        let layer_name = self.doc.project.next_layer_name(ts, &base_name);
        let resource_names: Vec<String> = added.iter().map(|(n, _)| n.clone()).collect();
        let id = self.edit("Import PBR textures", |s| {
            for (n, bytes) in &added {
                s.doc.resources.insert(n.clone(), bytes.clone());
            }
            let id = s.doc.project.alloc_id();
            crate::layer_cmds::insert_selected(s, Layer::new(id, layer_name, LayerKind::Fill(fill), &channels))
        })?;
        self.images.extend(decoded);
        let mut reply = report(&set, opts.normal_format);
        if let Some(o) = reply.as_object_mut() {
            o.insert("layer".into(), json!(id.0));
            o.insert("name".into(), json!(base_name));
            o.insert("channels".into(), json!(channels.iter().map(|c| c.id()).collect::<Vec<_>>()));
            o.insert("resources".into(), json!(resource_names));
            o.insert("warnings".into(), json!(warnings));
        }
        Ok(reply)
    }

    /// [`Session::import_pbr`] for files already in memory (the browser, tests).
    pub fn import_pbr_bytes(&mut self, files: Vec<(String, Vec<u8>)>, opts: &PbrOptions) -> Result<Value> {
        let list: FileList = files.iter().map(|(n, b)| (n.clone(), b.len() as u64)).collect();
        let by_name: BTreeMap<&str, &Vec<u8>> = files.iter().map(|(n, b)| (n.as_str(), b)).collect();
        let load = |name: &str| by_name.get(name).map(|b| (*b).clone()).ok_or_else(|| "no such file".to_string());
        self.import_pbr(&list, &load, opts)
    }
}

/// A name for the material from the files' shared leading text (`brick_wall_albedo`, `brick_wall_rough` → `brick wall`).
pub fn common_name(files: &[String]) -> Option<String> {
    let stems: Vec<&str> = files.iter().map(|f| f.rsplit(['/', '\\']).next().unwrap_or(f)).map(|l| l.rsplit_once('.').map_or(l, |(s, _)| s)).collect();
    let first = stems.first()?;
    let mut len = first.len();
    for s in &stems[1..] {
        len = first.bytes().zip(s.bytes()).take(len).take_while(|(a, b)| a.eq_ignore_ascii_case(b)).count();
    }
    let prefix = first.get(..len)?.trim_matches(|c: char| !c.is_alphanumeric());
    (prefix.chars().filter(|c| c.is_alphanumeric()).count() >= 2).then(|| prefix.replace(['_', '-'], " "))
}

// ---------------------------------------------------------------------------------------------
// Parameters and sources (native).

fn options(cmd: &str, p: &Value) -> Result<PbrOptions> {
    let normal_format = match strp(p, "normalFormat") {
        None | Some("auto") => None,
        Some("openGl" | "opengl") => Some(NormalFormat::OpenGl),
        Some("directX" | "directx") => Some(NormalFormat::DirectX),
        Some(other) => return Err(bad(cmd, format!("unknown normal format `{other}` (auto, openGl or directX)"))),
    };
    let mut choose = BTreeMap::new();
    if let Some(obj) = p.get("choose").and_then(Value::as_object) {
        for (k, v) in obj {
            let kind = MapKind::from_id(k).ok_or_else(|| bad(cmd, format!("choose: unknown map `{k}`")))?;
            let file = v.as_str().ok_or_else(|| bad(cmd, format!("choose.{k} must be a file name")))?;
            choose.insert(kind, file.to_string());
        }
    }
    Ok(PbrOptions { name: strp(p, "name").map(str::to_string), normal_format, projection: crate::layer_cmds::projection_from(p, cmd, None)?, choose })
}

/// Pick the one group of files that holds a material: the files at the root if they hold maps,
/// else the one folder that does. Returns the group's folder prefix (`""` or `"sub/"`).
fn pick_group(cmd: &str, files: &FileList) -> Result<String> {
    let mut dirs: BTreeMap<String, FileList> = BTreeMap::new();
    for (name, size) in files {
        let dir = name.rsplit_once('/').map_or("", |(d, _)| d).to_string();
        dirs.entry(dir).or_default().push((name.clone(), *size));
    }
    let with_maps: Vec<&String> = dirs.iter().filter(|(_, f)| !pbr_maps::detect(f.iter().map(|(n, s)| (n.as_str(), *s))).is_empty()).map(|(d, _)| d).collect();
    if with_maps.iter().any(|d| d.is_empty()) {
        return Ok(String::new());
    }
    match with_maps.as_slice() {
        [] => Err(bad(cmd, format!("no PBR texture maps found among {} files (names such as albedo, roughness, metallic, normal, height, ao)", files.len()))),
        [one] => Ok(format!("{one}/")),
        many => {
            let shown: Vec<&str> = many.iter().take(8).map(|d| d.as_str()).collect();
            Err(bad(
                cmd,
                format!("{} folders hold PBR maps ({}{}): import one of them", many.len(), shown.join(", "), if many.len() > 8 { ", ..." } else { "" }),
            ))
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    use super::*;

    /// Where the files of an import come from.
    pub enum Origin {
        Folder { root: PathBuf },
        Files(Vec<PathBuf>),
        Zip(RefCell<texturecraft_formats::zip_read::ZipFiles>),
    }

    pub struct Found {
        pub source: Origin,
        pub files: FileList,
        /// A name for the material (folder, archive or shared file-name prefix).
        pub name: Option<String>,
    }

    /// Regular files below `dir` (not following symbolic links), at most `MAX_FILES`, 4 levels deep.
    fn walk(root: &Path, dir: &Path, depth: usize, out: &mut FileList) {
        if depth > 4 || out.len() >= MAX_FILES {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for entry in rd.flatten() {
            if out.len() >= MAX_FILES {
                return;
            }
            let path = entry.path();
            // `symlink_metadata`: a link, pipe or device is neither read nor followed.
            let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
            if meta.is_dir() {
                walk(root, &path, depth + 1, out);
            } else if meta.is_file()
                && let Ok(rel) = path.strip_prefix(root)
            {
                let name = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
                out.push((name, meta.len()));
            }
        }
    }

    pub fn find(cmd: &str, p: &Value) -> Result<Found> {
        if let Some(arr) = p.get("files").and_then(Value::as_array) {
            let mut paths = Vec::new();
            let mut list = FileList::new();
            for v in arr.iter().take(MAX_FILES) {
                let Some(s) = v.as_str() else { return Err(bad(cmd, "`files` must be a list of paths")) };
                let path = PathBuf::from(s);
                let meta = std::fs::symlink_metadata(&path).map_err(|e| bad(cmd, format!("cannot read {s}: {e}")))?;
                if !meta.is_file() {
                    continue;
                }
                let leaf = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                // Names are leaf names; the same leaf twice keeps the first.
                if list.iter().any(|(n, _)| *n == leaf) {
                    continue;
                }
                list.push((leaf, meta.len()));
                paths.push(path);
            }
            let name = common_name(&list.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>());
            return Ok(Found { source: Origin::Files(paths), files: list, name });
        }
        let path = strp(p, "path").filter(|s| !s.trim().is_empty()).ok_or_else(|| bad(cmd, "`path` (a folder or .zip) or `files` is required"))?;
        let meta = std::fs::symlink_metadata(path).map_err(|e| bad(cmd, format!("cannot read {path}: {e}")))?;
        let stem = |s: &str| Path::new(s).file_stem().map(|n| n.to_string_lossy().replace(['_', '-'], " "));
        if meta.is_dir() {
            let root = Path::new(path).canonicalize().map_err(|e| bad(cmd, format!("cannot read {path}: {e}")))?;
            let mut files = FileList::new();
            walk(&root, &root, 0, &mut files);
            let group = pick_group(cmd, &files)?;
            let files: FileList = files.into_iter().filter(|(n, _)| n.strip_prefix(group.as_str()).is_some_and(|rest| !rest.contains('/'))).collect();
            let name = if group.is_empty() { stem(path) } else { stem(group.trim_end_matches('/')) }
                .or_else(|| root.file_name().map(|n| n.to_string_lossy().into_owned()));
            let root = if group.is_empty() { root } else { root.join(group.trim_end_matches('/')) };
            let files = files.into_iter().map(|(n, s)| (n.rsplit('/').next().unwrap_or(&n).to_string(), s)).collect();
            return Ok(Found { source: Origin::Folder { root }, files, name });
        }
        if meta.is_file() && path.to_ascii_lowercase().ends_with(".zip") {
            if meta.len() > MAX_FILE_BYTES * 2 {
                return Err(bad(cmd, format!("{path} is too large")));
            }
            let bytes = std::fs::read(path).map_err(|e| bad(cmd, format!("cannot read {path}: {e}")))?;
            let mut zip = texturecraft_formats::zip_read::ZipFiles::open(bytes).map_err(|e| bad(cmd, format!("{path} is not a readable zip: {e}")))?;
            let files = zip.list(MAX_FILES);
            let group = pick_group(cmd, &files)?;
            let files: FileList = files.into_iter().filter(|(n, _)| n.strip_prefix(group.as_str()).is_some_and(|rest| !rest.contains('/'))).collect();
            let name = if group.is_empty() { stem(path) } else { stem(group.trim_end_matches('/')) };
            return Ok(Found { source: Origin::Zip(RefCell::new(zip)), files, name });
        }
        Err(bad(cmd, format!("{path} is not a folder or a .zip file")))
    }

    impl Found {
        /// Reads one file of the list. Archive entries are read through a limit, since their
        /// declared size can lie.
        pub fn load(&self, name: &str) -> std::result::Result<Vec<u8>, String> {
            match &self.source {
                Origin::Folder { root } => read_regular(&root.join(name)),
                Origin::Files(paths) => {
                    let path = paths.iter().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy() == name)).ok_or("no such file")?;
                    read_regular(path)
                }
                Origin::Zip(zip) => zip.borrow_mut().read(name, MAX_FILE_BYTES).map_err(|e| e.to_string()),
            }
        }
    }

    fn read_regular(path: &Path) -> std::result::Result<Vec<u8>, String> {
        let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        // A link, pipe or device in the folder would escape it or block the read.
        if !meta.is_file() {
            return Err("it is not a regular file".into());
        }
        if meta.len() > MAX_FILE_BYTES {
            return Err(format!("the file is over {} MB", MAX_FILE_BYTES >> 20));
        }
        std::fs::read(path).map_err(|e| e.to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.importPbrFolder";
    let mut opts = options(cmd, p)?;
    let found = native::find(cmd, p)?;
    if opts.name.is_none() {
        opts.name = found.name.clone();
    }
    s.import_pbr(&found.files, &|name| found.load(name), &opts)
}

#[cfg(not(target_arch = "wasm32"))]
fn scan(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.scanPbrFolder";
    let opts = options(cmd, p)?;
    let found = native::find(cmd, p)?;
    let set = detect_with(&found.files, &opts.choose)?;
    let mut reply = report(&set, opts.normal_format);
    if let Some(o) = reply.as_object_mut() {
        o.insert("name".into(), json!(opts.name.or(found.name)));
        o.insert(
            "channelsAvailable".into(),
            json!(s.doc.project.texture_set(s.ts()).map(|t| t.channel_list()).unwrap_or_default().iter().map(|c| c.id()).collect::<Vec<_>>()),
        );
    }
    Ok(reply)
}

#[cfg(target_arch = "wasm32")]
fn import(_: &mut Session, p: &Value) -> Result<Value> {
    let _ = options("file.importPbrFolder", p)?;
    Err(EngineError::Other("file paths are not available in the browser".into()))
}

#[cfg(target_arch = "wasm32")]
fn scan(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Other("file paths are not available in the browser".into()))
}
