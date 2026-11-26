//! MaterialCraft materials as fill layers. A `.mcmat` portable material becomes a fill whose
//! channel sources are its cooked maps (image resources) and whose [`MaterialLink`] keeps the
//! graph's exposed parameters. Changing one re-cooks the maps through `materialcraft-cli` at the
//! layer's picked resolution (default: the texture set's), with the set's baked mesh maps feeding graph inputs of the same name;
//! both apps use the shared craft-mcmat container and the public CLI

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::{Map, Value, json};
use texturecraft_core::{Channel, CookResolution, Fill, Layer, LayerId, LayerKind, LinkParam, MaterialLink, Source};
use texturecraft_formats::RamReservation as CookReservation;
use texturecraft_formats::mcmat::{MCRAFT_EXT, is_mcmat, is_mcraft, validate_mcmat_version};

use crate::commands::{CommandSpec, bad, boolp, channel_param, has_layer, layer_param, strp};
use crate::materials::Place;
use crate::{EngineError, Result, Session, cmd};

#[cfg(not(target_arch = "wasm32"))]
mod cook_report;
mod cooked_map;
mod cooked_material;
mod image_input;
mod link_job;
mod link_memory_budget;
#[cfg(not(target_arch = "wasm32"))]
mod map_file;
mod material_interface;

pub use cooked_map::CookedMap;
pub use cooked_material::CookedMaterial;
pub use image_input::ImageInput;
pub use link_job::LinkJob;
pub use link_memory_budget::LinkMemoryBudget;
pub use material_interface::MaterialInterface;

/// The undo label of parameter edits and re-cooks (a re-cook merges into the edit before it).
const EDIT: &str = "Edit MaterialCraft material";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "material.link.set",
            "Set material parameter",
            [],
            None,
            "{layer?, id, value (number, [..], bool, \"#rrggbb\" or {t, v}), cook?: true} — sets an exposed input of a MaterialCraft material layer and re-cooks its maps (cook: false only stores the value)",
            has_layer,
            set
        ),
        cmd!(
            "material.link.preset",
            "Apply material preset",
            [],
            None,
            "{layer?, name, cook?: true} — applies a preset of a MaterialCraft material layer",
            has_layer,
            preset
        ),
        cmd!(
            "material.link.cook",
            "Re-cook MaterialCraft material",
            [],
            None,
            "{layer?} — re-cooks a MaterialCraft material layer at its picked resolution (default: the texture set's) with the set's baked mesh maps",
            has_layer,
            |s, p| {
                let id = layer_param(s, "material.link.cook", p)?;
                cook(s, id, Map::new())
            }
        ),
        cmd!(
            "material.link.map",
            "Map material channel",
            [],
            None,
            "{layer?, channel, output} — makes a channel of a MaterialCraft material layer read another output of the material (an output id such as \"roughness\")",
            has_layer,
            map
        ),
        cmd!(
            "material.link.resolution",
            "Set material resolution",
            [],
            None,
            "{layer?, relative: -3..3 | fixed: 128..8192 (power of two), cook?: true} — the size a MaterialCraft material layer cooks at: the texture set's resolution times 2^relative (0 = the texture set's), or fixed pixels across",
            has_layer,
            resolution
        ),
        cmd!(
            "material.link.saveSource",
            "Save MaterialCraft package",
            [],
            None,
            "{layer?, path} — writes the MaterialCraft source (.mcmat, .mcraft or .sbs) behind a material layer",
            has_layer,
            save_source
        ),
    ]
}

/// The link of fill layer `id`.
fn link_of<'a>(session: &'a Session, cmd: &str, id: LayerId) -> Result<&'a MaterialLink> {
    match &session.doc.project.layer(id)?.kind {
        LayerKind::Fill(Fill { link: Some(link), .. }) => Ok(link.as_ref()),
        _ => Err(bad(cmd, format!("layer {} is not a MaterialCraft material", id.0))),
    }
}

/// Value types other apps may set (MaterialCraft's portable inputs), with their component count.
fn components(parameter_type: &str) -> Option<usize> {
    Some(match parameter_type {
        "float" | "int" | "enum" | "bool" => 1,
        "float2" | "int2" => 2,
        "float3" => 3,
        "float4" | "color" => 4,
        _ => return None,
    })
}

fn type_of(value: &Value) -> &str {
    value.get("t").and_then(Value::as_str).unwrap_or("")
}

/// Convert a plain or tagged value to the parameter type, rejecting non-finite numbers.
pub fn typed(parameter: &LinkParam, value: &Value) -> Option<Value> {
    let parameter_type = type_of(&parameter.default);
    let component_count = components(parameter_type)?;
    let payload = if value.get("t").is_some() { value.get("v")? } else { value };
    let finite = |value: &Value| value.as_f64().filter(|number| number.is_finite());

    let payload = match (parameter_type, payload) {
        ("bool", Value::Bool(enabled)) => json!(enabled),
        ("bool", value) => json!(finite(value)? != 0.0),
        ("color", Value::String(hex)) => {
            let digits = hex.trim_start_matches('#');
            if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return None;
            }

            let component = |index: usize| Some(f64::from(u8::from_str_radix(digits.get(index..index + 2)?, 16).ok()?) / f64::from(u8::MAX));

            // Convert the sRGB hex input to linear parameter values
            let linear = |value: f64| if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) };
            json!([linear(component(0)?), linear(component(2)?), linear(component(4)?), 1.0])
        }
        (_, value) if component_count == 1 => {
            let number = finite(value)?;
            if matches!(parameter_type, "int" | "enum") { json!(number.round() as i64) } else { json!(number) }
        }
        (_, Value::Array(values)) if values.len() >= component_count.min(3) => {
            let mut numbers: Vec<f64> = values.iter().take(component_count).map(finite).collect::<Option<_>>()?;
            numbers.resize(component_count, 1.0);
            if parameter_type == "int2" { json!(numbers.iter().map(|number| number.round() as i64).collect::<Vec<_>>()) } else { json!(numbers) }
        }
        _ => return None,
    };

    Some(json!({"t": parameter_type, "v": payload}))
}

/// `overrides` checked against the link's parameters.
fn checked(cmd: &str, link: &MaterialLink, overrides: &Map<String, Value>) -> Result<Map<String, Value>> {
    overrides
        .iter()
        .map(|(id, v)| {
            let p = link.params.iter().find(|p| p.id == *id).ok_or_else(|| bad(cmd, format!("`{}` has no parameter `{id}`", link.name)))?;
            Ok((id.clone(), typed(p, v).ok_or_else(|| bad(cmd, format!("`{id}` takes a {}", type_of(&p.default))))?))
        })
        .collect()
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "material.link.set";
    let id = layer_param(s, CMD, p)?;
    let pid = strp(p, "id").ok_or_else(|| bad(CMD, "`id` is required"))?;
    let value = p.get("value").ok_or_else(|| bad(CMD, "`value` is required"))?;
    update(s, CMD, id, Map::from_iter([(pid.to_string(), value.clone())]), boolp(p, "cook").unwrap_or(true))
}

fn preset(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "material.link.preset";
    let id = layer_param(s, CMD, p)?;
    let name = strp(p, "name").ok_or_else(|| bad(CMD, "`name` is required"))?;
    let link = link_of(s, CMD, id)?;
    let preset = link.presets.iter().find(|preset| preset.name == name).ok_or_else(|| bad(CMD, format!("`{}` has no preset `{name}`", link.name)))?;

    // Leave values the painter cannot set unchanged
    let values =
        preset.values.iter().filter(|(key, _)| link.params.iter().any(|param| &param.id == *key)).map(|(key, value)| (key.clone(), value.clone())).collect();
    update(s, CMD, id, values, boolp(p, "cook").unwrap_or(true))
}

/// Set parameter values of linked layer `id`; with `cook`, re-cook now (one undo step).
fn update(s: &mut Session, cmd: &str, id: LayerId, overrides: Map<String, Value>, cook_now: bool) -> Result<Value> {
    let link = link_of(s, cmd, id)?;
    let values = checked(cmd, link, &overrides)?;
    if cook_now {
        return cook(s, id, values);
    }

    s.edit(EDIT, |s| {
        if let LayerKind::Fill(Fill { link: Some(link), .. }) = &mut s.doc.project.layer_mut(id)?.kind {
            for p in &mut link.params {
                if let Some(v) = values.get(&p.id) {
                    p.value = v.clone();
                }
            }
        }
        Ok(())
    })?;
    Ok(json!({"layer": id.0, "cooked": false}))
}

fn map(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "material.link.map";
    let id = layer_param(s, CMD, p)?;
    let channel = channel_param(CMD, p, "channel")?.ok_or_else(|| bad(CMD, "`channel` is required"))?;
    let output = strp(p, "output").ok_or_else(|| bad(CMD, "`output` is required"))?;
    link_of(s, CMD, id)?;
    s.edit("Map material channel", |s| {
        let LayerKind::Fill(fill) = &mut s.doc.project.layer_mut(id)?.kind else { return Ok(()) };
        let outputs = fill.link_outputs();
        let image = outputs.get(output).ok_or_else(|| bad(CMD, format!("the material has no output `{output}`")))?.clone();
        let normal_format = match fill.sources.get(&channel) {
            Some(Source::Image { normal_format, .. }) => *normal_format,
            _ => Default::default(),
        };
        fill.sources.insert(channel, Source::Image { name: image, normal_format });
        if let Some(link) = &mut fill.link {
            if output == channel.id() {
                link.mapping.remove(&channel);
            } else {
                link.mapping.insert(channel, output.to_string());
            }

            // Sources no longer name every output once one is mapped elsewhere
            link.outputs = outputs;
        }
        Ok(())
    })?;
    Ok(json!({"layer": id.0, "channel": channel.id(), "output": output}))
}

/// The `relative` or `fixed` param of `material.link.resolution`; `Some(None)` is the texture set's own size
pub(crate) fn picked_resolution(p: &Value) -> Option<Option<CookResolution>> {
    match (p.get("relative"), p.get("fixed")) {
        (Some(factor), None) => {
            let factor = i8::try_from(factor.as_i64()?).ok().filter(|factor| CookResolution::FACTORS.contains(factor))?;
            Some((factor != 0).then_some(CookResolution::Relative(factor)))
        }
        (None, Some(size)) => {
            let size = u32::try_from(size.as_i64()?).ok().filter(|size| CookResolution::cookable(*size))?;
            Some(Some(CookResolution::Fixed(size)))
        }
        _ => None,
    }
}

fn resolution(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "material.link.resolution";
    let id = layer_param(s, CMD, p)?;
    let size = picked_resolution(p).ok_or_else(|| bad(CMD, "give `relative` (-3..3) or `fixed` (a power of two from 128 to 8192)"))?;
    let link = link_of(s, CMD, id)?;
    let store = |s: &mut Session| {
        s.edit(EDIT, |s| {
            if let LayerKind::Fill(Fill { link: Some(link), .. }) = &mut s.doc.project.layer_mut(id)?.kind {
                link.resolution = size;
            }
            Ok(())
        })
    };
    if !boolp(p, "cook").unwrap_or(true) {
        // Only the stored size changes, so the size it already has adds no undo step
        if link.resolution != size {
            store(s)?;
        }

        return Ok(json!({"layer": id.0, "cooked": false}));
    }

    // Install the resolution and maps in one edit so a failed install also leaves the old resolution
    let mut job = s.link_job(id, &Map::new())?;
    let (texture_set, _, _) = s.doc.project.locate(id).ok_or_else(|| bad(CMD, format!("layer {} is not in a stack", id.0)))?;
    job.size = size.unwrap_or(CookResolution::TEXTURE_SET).size(s.doc.project.texture_set(texture_set)?.resolution);
    let material = job.run()?;
    s.install_link_cook(id, &material, s.merge_edits, Some(size))?;
    Ok(json!({"layer": id.0, "cooked": true, "size": job.size}))
}

fn cook(s: &mut Session, id: LayerId, overrides: Map<String, Value>) -> Result<Value> {
    let job = s.link_job(id, &overrides)?;
    let material = job.run()?;
    s.apply_link_cook(id, &material, false)?;
    Ok(json!({"layer": id.0, "cooked": true, "size": job.size}))
}

#[cfg(not(target_arch = "wasm32"))]
fn save_source(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "material.link.saveSource";
    let id = layer_param(s, CMD, p)?;
    let path = strp(p, "path").ok_or_else(|| bad(CMD, "`path` is required"))?;
    let link = link_of(s, CMD, id)?;
    let ext = source_extension(&link.package);
    let path = if path.to_ascii_lowercase().ends_with(&format!(".{ext}")) { path.to_string() } else { format!("{path}.{ext}") };
    let bytes = s.link_package(id)?;
    texturecraft_formats::write_atomic(std::path::Path::new(&path), &bytes).map_err(|e| bad(CMD, e.to_string()))?;
    Ok(json!({"path": path}))
}

#[cfg(target_arch = "wasm32")]
fn save_source(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Other("file paths are not available in the browser".into()))
}

/// A MaterialCraft executable: `$<env>`, else next to this program, else `name` on `PATH`.
#[cfg(not(target_arch = "wasm32"))]
pub fn materialcraft_tool(name: &str, env: &str) -> std::path::PathBuf {
    if let Some(p) = std::env::var_os(env).filter(|p| !p.is_empty()) {
        return p.into();
    }

    let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe().ok().and_then(|e| Some(e.parent()?.join(&exe))).filter(|p| p.is_file()).unwrap_or_else(|| exe.into())
}

/// A command for a MaterialCraft executable that opens no console window.
#[cfg(not(target_arch = "wasm32"))]
fn quiet(program: &std::path::Path) -> std::process::Command {
    #[allow(unused_mut)]
    let mut c = std::process::Command::new(program);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut c, 0x0800_0000); // CREATE_NO_WINDOW
    c
}

/// Open `package` (a published material or authoring package) in the MaterialCraft app (`$MATERIALCRAFT`, next to this
/// program, or on `PATH`).
#[cfg(not(target_arch = "wasm32"))]
pub fn open_in_materialcraft(package: &std::path::Path) -> Result<()> {
    let app = materialcraft_tool("materialcraft", "MATERIALCRAFT");
    quiet(&app).arg(package).spawn().map(drop).map_err(|e| {
        EngineError::Other(format!("MaterialCraft ({}) did not start: {e}. Install MaterialCraft next to TextureCraft or set MATERIALCRAFT", app.display()))
    })
}

/// Source file extensions are chosen from the linked resource name, never an untrusted path.
pub fn source_extension(name: &str) -> &'static str {
    if name.to_ascii_lowercase().ends_with(".mcmat") {
        "mcmat"
    } else if name.to_ascii_lowercase().ends_with(".sbs") {
        "sbs"
    } else {
        MCRAFT_EXT
    }
}

/// Prepare the first bake without borrowing the paint document. Its original archive is kept for future cooks.
fn cook_source(
    bytes: Vec<u8>,
    size: u32,
    #[cfg(not(target_arch = "wasm32"))] baked: Option<texturecraft_compose::BakedMaps>,
    mut budget: LinkMemoryBudget,
    canceled: Arc<std::sync::atomic::AtomicBool>,
    reservation: CookReservation,
) -> Result<(LinkJob, CookedMaterial)> {
    check_source_size(bytes.len())?;
    budget.reserve(bytes.capacity() as u64)?;

    let source_ext = if is_mcmat(&bytes) {
        validate_mcmat_version(&bytes, budget.remaining()).map_err(|error| EngineError::Other(error.to_string()))?;
        "mcmat"
    } else if is_mcraft(&bytes) {
        MCRAFT_EXT
    } else {
        return Err(EngineError::Other("expected a MaterialCraft material or project archive".into()));
    };

    let job = LinkJob {
        layer: LayerId(0),
        package: Arc::new(bytes),
        source_ext,
        graph: None,
        size,
        values: Map::new(),
        input_identifiers: Vec::new(),
        #[cfg(not(target_arch = "wasm32"))]
        baked,
        #[cfg(not(target_arch = "wasm32"))]
        budget,
        canceled,
    };
    let material = job.run_reserved(reservation)?;
    Ok((job, material))
}

fn check_source_size(bytes: usize) -> Result<()> {
    let limit = crate::resource_cmds::MAX_BYTES;
    if bytes > limit {
        return Err(EngineError::Other(format!("linked source exceeds the project resource limit of {} MiB", limit >> 20)));
    }

    Ok(())
}

/// File extensions for initial linked-material imports.
#[cfg(not(target_arch = "wasm32"))]
pub fn is_link_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mcmat") || extension.eq_ignore_ascii_case(MCRAFT_EXT))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn cook_link_file(
    path: &std::path::Path,
    size: u32,
    baked: Option<texturecraft_compose::BakedMaps>,
    budget: LinkMemoryBudget,
    canceled: Arc<std::sync::atomic::AtomicBool>,
) -> Result<(LinkJob, CookedMaterial)> {
    let reservation = CookReservation::for_cook().map_err(|error| EngineError::Other(error.to_string()))?;
    let metadata = std::fs::metadata(path).map_err(|error| bad("material.load", format!("cannot read {}: {error}", path.display())))?;
    budget.check(metadata.len())?;

    let bytes = crate::resource_cmds::read_resource_file("material.load", path, budget.remaining().min(crate::resource_cmds::MAX_BYTES as u64))?;
    cook_source(bytes, size, baked, budget, canceled, reservation)
}

impl Session {
    /// Add a MaterialCraft material (`.mcmat` bytes, or a `.mcraft` package cooked first) as a
    /// fill layer at `place` (one undo step). Returns the new layer.
    pub fn add_linked_material_bytes_at(&mut self, bytes: &[u8], place: Place) -> Result<LayerId> {
        check_source_size(bytes.len())?;
        let ts = self.ts();
        let size = self.doc.project.texture_set(ts)?.resolution;
        let reservation = CookReservation::for_cook().map_err(|error| EngineError::Other(error.to_string()))?;
        let budget = self.link_memory_budget();
        budget.check(bytes.len() as u64)?;

        let mut source = Vec::new();
        source.try_reserve_exact(bytes.len()).map_err(|error| EngineError::Other(format!("cannot allocate linked source: {error}")))?;
        source.extend_from_slice(bytes);
        let (job, material) = cook_source(
            source,
            size,
            #[cfg(not(target_arch = "wasm32"))]
            self.doc.baked.get(&ts).cloned(),
            budget,
            Arc::default(),
            reservation,
        )?;
        self.add_cooked_material(&material, job.package, job.source_ext, place)
    }

    /// Install validated CLI output in one undo step. Keep the original source for future variations.
    pub fn add_cooked_material(&mut self, m: &CookedMaterial, source: Arc<Vec<u8>>, source_ext: &str, place: Place) -> Result<LayerId> {
        self.add_cooked_material_to_set(m, source, source_ext, self.ts(), place)
    }

    /// Install a background import in the texture set selected when it started.
    pub fn add_cooked_material_to_set(&mut self, m: &CookedMaterial, source: Arc<Vec<u8>>, source_ext: &str, ts: usize, place: Place) -> Result<LayerId> {
        check_source_size(source.len())?;
        self.link_memory_budget().check(material_bytes(m).saturating_add(source.capacity() as u64))?;
        if m.maps.is_empty() {
            return Err(EngineError::Other(format!("{} has no maps TextureCraft can paint with", m.interface.name)));
        }

        let label = format!("Add {}", m.interface.name);
        let (id, names) = self.edit(&label, |s| {
            let stem = texturecraft_formats::sanitize_resource_name(&m.interface.name);
            let ext = match source_ext {
                "mcmat" | "sbs" | "mcraft" => source_ext,
                _ => return Err(EngineError::Other("unknown linked source format".into())),
            };
            let package = unique(s, &stem, &format!(".{ext}"));
            s.doc.resources.insert(package.clone(), source);
            let link = MaterialLink {
                name: m.interface.name.clone(),
                package,
                graph: m.interface.graph.clone(),
                params: m.interface.parameters.iter().filter(|p| components(type_of(&p.default)).is_some()).cloned().collect(),
                presets: m.interface.presets.clone(),
                inputs: m.interface.inputs.iter().map(|i| i.identifier.clone()).collect(),
                size: m.size,
                resolution: None,
                mapping: BTreeMap::new(),
                outputs: BTreeMap::new(),
            };
            let mut fill = Fill { link: Some(Box::new(link)), ..Fill::default() };
            let names = put_maps(s, &stem, &m.maps, &mut fill);
            for c in fill.sources.keys() {
                crate::set_cmds::add_channel(s, ts, *c)?;
            }
            let channels: Vec<Channel> = fill.sources.keys().copied().collect();
            let lid = s.doc.project.alloc_id();
            let name = s.doc.project.next_layer_name(ts, &m.interface.name);
            let layer = Layer::new(lid, name, LayerKind::Fill(fill), &channels);
            let lid = if ts == s.ts() { crate::layer_cmds::insert_selected(s, layer)? } else { s.doc.project.insert_layer(ts, layer, None, false)? };
            place.apply(s, lid)?;
            if ts == s.ts() {
                s.select_one(ts, lid);
                s.target = crate::session::StackTarget::Content;
            }
            Ok((lid, names))
        })?;
        self.cache_decoded(names, &m.maps);
        Ok(id)
    }

    /// Account for existing encoded resources and decoded images when admitting a linked bake.
    pub fn link_memory_budget(&self) -> LinkMemoryBudget {
        #[cfg(target_arch = "wasm32")]
        let cap_bytes = u64::MAX;
        #[cfg(not(target_arch = "wasm32"))]
        let cap_bytes = texturecraft_formats::configured_ram_budget();
        let mut budget = LinkMemoryBudget::new(cap_bytes);
        let mut resources = std::collections::HashSet::new();
        let mut mesh_maps = std::collections::HashSet::new();
        let documents = std::iter::once(&self.doc).chain(self.undo.iter().chain(&self.redo).map(|(_, document)| document));
        let mut retained_bytes = 0u64;
        for document in documents {
            for bytes in document.resources.values().filter(|bytes| resources.insert(Arc::as_ptr(bytes))) {
                retained_bytes = retained_bytes.saturating_add(bytes.capacity() as u64);
            }

            for data in document.baked.values().flat_map(|baked| baked.maps.values()).filter(|data| mesh_maps.insert(Arc::as_ptr(data))) {
                retained_bytes = retained_bytes.saturating_add(data.capacity() as u64 * std::mem::size_of::<f32>() as u64);
            }
        }

        let mut images = std::collections::HashSet::new();
        for image in self.images.values().filter(|image| images.insert(Arc::as_ptr(image))) {
            retained_bytes = retained_bytes.saturating_add(image.retained_texel_bytes());
        }

        budget.set_retained(retained_bytes);
        budget
    }

    /// The source package behind linked layer `id`.
    pub fn link_package(&self, id: LayerId) -> Result<Arc<Vec<u8>>> {
        let link = link_of(self, "material.link", id)?;
        self.doc.resources.get(&link.package).cloned().ok_or_else(|| EngineError::Other(format!("the package of {} is missing", link.name)))
    }

    /// The parameter values of linked layer `id`, by id.
    pub fn link_values(&self, id: LayerId) -> Result<Map<String, Value>> {
        Ok(link_of(self, "material.link", id)?.params.iter().map(|p| (p.id.clone(), p.value.clone())).collect())
    }

    /// Prepare a re-cook with current parameter values and matching baked mesh maps
    pub fn link_job(&self, id: LayerId, overrides: &Map<String, Value>) -> Result<LinkJob> {
        let link = link_of(self, "material.link", id)?;
        #[cfg(not(target_arch = "wasm32"))]
        let (texture_set, _, _) = self.doc.project.locate(id).ok_or_else(|| EngineError::Other(format!("layer {} is not in a stack", id.0)))?;
        let mut values = self.link_values(id)?;
        values.extend(overrides.clone());

        Ok(LinkJob {
            layer: id,
            package: self.link_package(id)?,
            source_ext: source_extension(&link.package),
            graph: Some(link.graph.clone()),
            size: self.link_size(id)?,
            values,
            input_identifiers: link.inputs.clone(),
            #[cfg(not(target_arch = "wasm32"))]
            baked: self.doc.baked.get(&texture_set).cloned(),
            #[cfg(not(target_arch = "wasm32"))]
            budget: self.link_memory_budget(),
            canceled: Arc::default(),
        })
    }

    /// Pixels across linked layer `id` cooks at: its picked resolution, else its texture set's
    pub fn link_size(&self, id: LayerId) -> Result<u32> {
        let link = link_of(self, "material.link", id)?;
        let (texture_set, _, _) = self.doc.project.locate(id).ok_or_else(|| EngineError::Other(format!("layer {} is not in a stack", id.0)))?;
        // UDIM tiles share one cook at the texture set's base resolution
        Ok(link.resolution.unwrap_or(CookResolution::TEXTURE_SET).size(self.doc.project.texture_set(texture_set)?.resolution))
    }

    /// Install the maps and exposed values returned by a completed [`LinkJob`].
    /// With `merge`, this folds into the previous undo step when that was an edit of the same kind
    /// (a parameter set while the cook ran).
    pub fn apply_link_cook(&mut self, id: LayerId, material: &CookedMaterial, merge: bool) -> Result<()> {
        self.install_link_cook(id, material, merge, None)
    }

    fn install_link_cook(&mut self, id: LayerId, material: &CookedMaterial, merge: bool, resolution_change: Option<Option<CookResolution>>) -> Result<()> {
        self.link_memory_budget().check(material_bytes(material))?;
        let names = self.edit_merging(EDIT, merge, |session| {
            let LayerKind::Fill(mut fill) = session.doc.project.layer(id)?.kind.clone() else {
                return Err(EngineError::Other(format!("layer {} is not a fill", id.0)));
            };

            let link = fill.link.as_mut().ok_or_else(|| EngineError::Other(format!("layer {} is not a MaterialCraft material", id.0)))?;
            if let Some(resolution) = resolution_change {
                link.resolution = resolution;
            }

            for parameter in &mut link.params {
                if let Some(cooked) = material.interface.parameters.iter().find(|candidate| candidate.id == parameter.id) {
                    parameter.value = cooked.value.clone();
                }
            }
            link.size = material.size;
            let stem = texturecraft_formats::sanitize_resource_name(&link.name);
            let mut old_names: Vec<String> =
                fill.sources.values().filter_map(|source| if let Source::Image { name, .. } = source { Some(name.clone()) } else { None }).collect();
            old_names.extend(fill.link_outputs().into_values());
            let names = put_maps(session, &stem, &material.maps, &mut fill);
            session.doc.project.layer_mut(id)?.kind = LayerKind::Fill(fill);

            // Duplicated layers may still use the earlier maps
            let mut used_names = Vec::new();
            for texture_set in &session.doc.project.texture_sets {
                for &root in &texture_set.stack {
                    crate::material_lib::images_used(&session.doc.project, root, &mut used_names, 0);
                }
            }

            for name in old_names.iter().filter(|name| !used_names.contains(name)) {
                session.doc.resources.remove(name);
            }

            Ok(names)
        })?;

        // Reuse the validated maps before synchronization considers decoding missing images
        self.cache_decoded(names, &material.maps);
        self.sync_images();

        Ok(())
    }

    /// Keep the images decoded while validating maps instead of decoding them again.
    fn cache_decoded(&mut self, names: Vec<String>, maps: &[CookedMap]) {
        for (name, map) in names.into_iter().zip(maps) {
            self.images.insert(name, Arc::clone(&map.image));
        }
    }
}

fn material_bytes(material: &CookedMaterial) -> u64 {
    material.maps.iter().fold(0u64, |total, map| total.saturating_add(map.bytes.capacity() as u64).saturating_add(map.image.retained_texel_bytes()))
}

/// A resource name that preserves its suffix through a project round trip.
fn unique(session: &Session, stem: &str, suffix: &str) -> String {
    for number in 1.. {
        let prefix = if number == 1 { String::new() } else { format!("{number}_") };
        let stem_limit = texturecraft_formats::MAX_RESOURCE_NAME_LEN.saturating_sub(prefix.len() + suffix.len());
        let stem = stem.get(..stem.len().min(stem_limit)).unwrap_or(stem);
        let name = format!("{prefix}{stem}{suffix}");
        if !session.doc.resources.contains_key(&name) {
            return name;
        }
    }

    format!("{stem}{suffix}")
}

/// Store `maps` as new resources (names carry a fresh id, so the decoded-image cache never confuses two cooks), list them as the link's outputs and point `fill`'s channels at them: each channel at its mapped output, else at the output named like it. Returns the names, in map order
fn put_maps(s: &mut Session, stem: &str, maps: &[CookedMap], fill: &mut Fill) -> Vec<String> {
    let cook = s.doc.project.alloc_id().0;
    let mut names = Vec::new();
    let mut outputs = BTreeMap::new();
    for map in maps {
        let channel = map.channel;
        let extension = if map.bytes.starts_with(crate::resource_cmds::EXR_SIGNATURE) { "exr" } else { "png" };
        let name = unique(s, stem, &format!("_{}_{cook}.{extension}", channel.id()));
        s.doc.resources.insert(name.clone(), Arc::clone(&map.bytes));
        outputs.insert(channel.id().to_string(), name.clone());
        names.push(name);
    }

    // Both callers cook a linked fill; one without a link only keeps the stored maps
    let Some(link) = fill.link.as_mut() else { return names };
    let channels: BTreeSet<Channel> = maps.iter().map(|map| map.channel).chain(link.mapping.keys().copied()).collect();
    for channel in channels {
        let Some(name) = outputs.get(link.output_of(channel)) else { continue };

        // A normal map the user marked DirectX stays marked through re-cooks
        let normal_format = match fill.sources.get(&channel) {
            Some(Source::Image { normal_format, .. }) => *normal_format,
            _ => Default::default(),
        };
        fill.sources.insert(channel, Source::Image { name: name.clone(), normal_format });
    }
    link.outputs = outputs;
    names
}

#[cfg(test)]
mod tests {
    use super::typed;
    use serde_json::json;
    use texturecraft_core::layer::LinkParam;

    fn color_param() -> LinkParam {
        let c = json!({"t": "color", "v": [0.0, 0.0, 0.0, 1.0]});
        serde_json::from_value(json!({"id": "c", "label": "C", "value": c, "default": c})).unwrap()
    }

    #[test]
    fn hex_color_rejects_signs_and_extra_digits() {
        let p = color_param();
        assert_eq!(typed(&p, &json!("#aabbccdd")), None);
        assert_eq!(typed(&p, &json!("#+f+f+f")), None);
        assert!(typed(&p, &json!("#ff8000")).is_some());
    }
}
