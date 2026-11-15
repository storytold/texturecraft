//! MaterialCraft materials as fill layers. A `.mcmat` portable material becomes a fill whose
//! channel sources are its cooked maps (image resources) and whose [`MaterialLink`] keeps the
//! graph's exposed parameters. Changing one re-cooks the maps through `materialcraft-cli` at the
//! texture set's resolution, with the set's baked mesh maps feeding graph inputs of the same name.
//! The apps share the file format and the command line, not code.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use texturecraft_core::{BakedMap, Channel, Fill, Layer, LayerId, LayerKind, LinkParam, MaterialLink, Source};
#[cfg(not(target_arch = "wasm32"))]
use texturecraft_formats::mcmat::MCMAT_EXT;
use texturecraft_formats::mcmat::{MCRAFT_EXT, PortableMaterial, is_mcraft, read_mcmat};

use crate::commands::{CommandSpec, bad, boolp, has_layer, layer_param, strp};
use crate::materials::Place;
use crate::{EngineError, Result, Session, cmd};

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
            "{layer?} — re-cooks a MaterialCraft material layer at its texture set's resolution with the set's baked mesh maps",
            has_layer,
            |s, p| {
                let id = layer_param(s, "material.link.cook", p)?;
                cook(s, id, Map::new())
            }
        ),
        cmd!(
            "material.link.saveSource",
            "Save MaterialCraft package",
            [],
            None,
            "{layer?, path} — writes the MaterialCraft source (.mcraft or .sbs) behind a material layer",
            has_layer,
            save_source
        ),
    ]
}

/// The link of fill layer `id`.
fn link_of(s: &Session, cmd: &str, id: LayerId) -> Result<MaterialLink> {
    match &s.doc.project.layer(id)?.kind {
        LayerKind::Fill(Fill { link: Some(link), .. }) => Ok(link.as_ref().clone()),
        _ => Err(bad(cmd, format!("layer {} is not a MaterialCraft material", id.0))),
    }
}

/// Value types other apps may set (MaterialCraft's portable inputs), with their component count.
fn components(t: &str) -> Option<usize> {
    Some(match t {
        "float" | "int" | "enum" | "bool" => 1,
        "float2" | "int2" => 2,
        "float3" => 3,
        "float4" | "color" => 4,
        _ => return None,
    })
}

fn type_of(v: &Value) -> &str {
    v.get("t").and_then(Value::as_str).unwrap_or("")
}

/// `v` (plain or `{t, v}`) as a typed value for `param`, with finite numbers.
pub fn typed(param: &LinkParam, v: &Value) -> Option<Value> {
    let t = type_of(&param.default);
    let n = components(t)?;
    let payload = if v.get("t").is_some() { v.get("v")? } else { v };
    let num = |x: &Value| x.as_f64().filter(|f| f.is_finite());
    let payload = match (t, payload) {
        ("bool", Value::Bool(b)) => json!(b),
        ("bool", x) => json!(num(x)? != 0.0),
        ("color", Value::String(hex)) => {
            let h = hex.trim_start_matches('#');
            let c = |i: usize| Some(f64::from(u8::from_str_radix(h.get(i..i + 2)?, 16).ok()?) / 255.0);
            // Hex colors are sRGB; values are linear.
            let lin = |x: f64| if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) };
            json!([lin(c(0)?), lin(c(2)?), lin(c(4)?), 1.0])
        }
        (_, x) if n == 1 => {
            let f = num(x)?;
            if matches!(t, "int" | "enum") { json!(f.round() as i64) } else { json!(f) }
        }
        (_, Value::Array(a)) if a.len() >= n.min(3) => {
            let mut out: Vec<f64> = a.iter().take(n).map(num).collect::<Option<_>>()?;
            out.resize(n, 1.0);
            if t == "int2" { json!(out.iter().map(|f| f.round() as i64).collect::<Vec<_>>()) } else { json!(out) }
        }
        _ => return None,
    };
    Some(json!({"t": t, "v": payload}))
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
    let values = link.presets.iter().find(|pr| pr.name == name).ok_or_else(|| bad(CMD, format!("`{}` has no preset `{name}`", link.name)))?.values.clone();
    // Presets may name inputs the painter can't set; those stay as cooked.
    let values = values.into_iter().filter(|(k, _)| link.params.iter().any(|p| p.id == *k));
    update(s, CMD, id, values, boolp(p, "cook").unwrap_or(true))
}

/// Set parameter values of linked layer `id`; with `cook`, re-cook now (one undo step).
fn update(s: &mut Session, cmd: &str, id: LayerId, overrides: impl IntoIterator<Item = (String, Value)>, cook_now: bool) -> Result<Value> {
    let link = link_of(s, cmd, id)?;
    let values = checked(cmd, &link, &overrides.into_iter().collect::<Map<_, _>>())?;
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

fn cook(s: &mut Session, id: LayerId, overrides: Map<String, Value>) -> Result<Value> {
    let job = s.link_job(id, &overrides)?;
    let bytes = job.run()?;
    s.apply_link_cook(id, &bytes, false)?;
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

/// A re-cook of a linked layer, to run anywhere (a worker thread) and hand back to
/// [`Session::apply_link_cook`].
#[derive(Clone, Debug)]
pub struct LinkJob {
    pub layer: LayerId,
    pub package: Arc<Vec<u8>>,
    /// The source package extension (`mcraft` or `sbs`).
    pub source_ext: &'static str,
    /// The named graph in a linked material; direct package imports use its default.
    pub graph: Option<String>,
    /// Pixels across (the texture set's resolution).
    pub size: u32,
    /// Every parameter's value, by id, in MaterialCraft's typed form.
    pub values: Map<String, Value>,
    /// Graph inputs fed from baked mesh maps: identifier → 16-bit PNG.
    pub inputs: Vec<(String, Vec<u8>)>,
}

#[cfg(not(target_arch = "wasm32"))]
static NEXT_JOB: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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

/// Open `package` (a `.mcraft` or `.sbs` file) in the MaterialCraft app (`$MATERIALCRAFT`, next to this
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
    if name.to_ascii_lowercase().ends_with(".sbs") { "sbs" } else { MCRAFT_EXT }
}

impl LinkJob {
    /// Cook through `materialcraft-cli` (`$MATERIALCRAFT_CLI`, next to this program, or on
    /// `PATH`); returns `.mcmat` bytes. Blocks for the cook (about a second at 2K).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn run(&self) -> Result<Vec<u8>> {
        self.run_with_cli(&materialcraft_tool("materialcraft-cli", "MATERIALCRAFT_CLI"))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn run_with_cli(&self, cli: &std::path::Path) -> Result<Vec<u8>> {
        let other = |e: String| EngineError::Other(e);
        let n = NEXT_JOB.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("texturecraft-link-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| other(format!("{}: {e}", dir.display())))?;
        let result = (|| {
            let utf8 = |p: &std::path::Path| p.to_str().map(str::to_string).ok_or_else(|| other(format!("{} is not a UTF-8 path", p.display())));
            let write = |name: &str, bytes: &[u8]| {
                let p = dir.join(name);
                std::fs::write(&p, bytes).map_err(|e| other(format!("{name}: {e}")))?;
                utf8(&p)
            };
            let ext = if self.source_ext == "sbs" { "sbs" } else { MCRAFT_EXT };
            let src = write(&format!("source.{ext}"), &self.package)?;
            let mut inputs = Map::new();
            for (i, (id, png)) in self.inputs.iter().enumerate() {
                inputs.insert(id.clone(), json!(write(&format!("input{i}.png"), png)?));
            }
            let out = dir.join(format!("out.{MCMAT_EXT}"));
            let params = json!({"path": utf8(&out)?, "size": self.size, "values": self.values, "inputs": inputs});
            let mut command = quiet(cli);
            if let Some(graph) = &self.graph {
                // Opening the source starts on its first graph; select the graph saved in the link.
                let script = json!({"open": src, "steps": [
                    {"id": "graph.open", "params": {"graph": graph}},
                    {"id": "file.exportMaterial", "params": params},
                ]});
                let script_path = write("cook.json", script.to_string().as_bytes())?;
                command.arg("batch").arg(script_path);
            } else {
                command.arg("run").arg(&src).arg("file.exportMaterial").arg(params.to_string());
            }
            let run = command.output().map_err(|e| {
                other(format!(
                    "MaterialCraft's command line ({}) did not start: {e}. Install MaterialCraft next to TextureCraft or set MATERIALCRAFT_CLI",
                    cli.display()
                ))
            })?;
            if !run.status.success() {
                return Err(other(format!("MaterialCraft could not cook the material: {}", String::from_utf8_lossy(&run.stderr).trim())));
            }
            std::fs::read(&out).map_err(|e| other(format!("{}: {e}", out.display())))
        })();
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    #[cfg(target_arch = "wasm32")]
    pub fn run(&self) -> Result<Vec<u8>> {
        Err(EngineError::Other("re-cooking a MaterialCraft material needs its command line, which the browser can't run".into()))
    }
}

/// Decode every map of `m` (validating them) with the channel it feeds.
fn decoded_maps(m: &PortableMaterial) -> Result<Vec<(Channel, Vec<u8>, texturecraft_formats::DecodedImage)>> {
    m.maps
        .iter()
        .filter_map(|(usage, bytes)| Some((Channel::ALL.into_iter().find(|c| c.id() == usage)?, bytes)))
        .map(|(c, bytes)| {
            let img = texturecraft_formats::decode_image(bytes).map_err(|e| EngineError::Other(format!("{} map: {e}", c.id())))?;
            Ok((c, bytes.clone(), img))
        })
        .collect()
}

impl Session {
    /// Add a MaterialCraft material (`.mcmat` bytes, or a `.mcraft` package cooked first) as a
    /// fill layer at `place` (one undo step). Returns the new layer.
    pub fn add_linked_material_bytes_at(&mut self, bytes: &[u8], place: Place) -> Result<LayerId> {
        let cooked;
        let bytes = if is_mcraft(bytes) {
            let ts = self.ts();
            let size = self.doc.project.texture_set(ts)?.resolution;
            let job = LinkJob {
                layer: LayerId(0),
                package: Arc::new(bytes.to_vec()),
                source_ext: MCRAFT_EXT,
                graph: None,
                size,
                values: Map::new(),
                inputs: Vec::new(),
            };
            cooked = job.run()?;
            cooked.as_slice()
        } else {
            bytes
        };
        let m = read_mcmat(bytes).map_err(|e| EngineError::Other(e.to_string()))?;
        let maps = decoded_maps(&m)?;
        if maps.is_empty() {
            return Err(EngineError::Other(format!("{} has no maps TextureCraft can paint with", m.manifest.name)));
        }
        let label = format!("Add {}", m.manifest.name);
        let (id, names) = self.edit(&label, |s| {
            let ts = s.ts();
            let stem = texturecraft_formats::sanitize_resource_name(&m.manifest.name);
            let package = unique(s, &format!("{stem}.{}", source_extension(&m.manifest.source)));
            s.doc.resources.insert(package.clone(), Arc::new(m.source.clone()));
            let link = MaterialLink {
                name: m.manifest.name.clone(),
                package,
                graph: m.manifest.graph.clone(),
                params: m.manifest.parameters.iter().filter(|p| components(type_of(&p.default)).is_some()).cloned().collect(),
                presets: m.manifest.presets.clone(),
                inputs: m.manifest.inputs.iter().map(|i| i.identifier.clone()).collect(),
                size: m.manifest.size[0],
            };
            let mut fill = Fill { link: Some(Box::new(link)), ..Fill::default() };
            let names = put_maps(s, &stem, &maps, &mut fill);
            for c in fill.sources.keys() {
                crate::set_cmds::add_channel(s, ts, *c)?;
            }
            let channels: Vec<Channel> = fill.sources.keys().copied().collect();
            let lid = s.doc.project.alloc_id();
            let name = s.doc.project.next_layer_name(ts, &m.manifest.name);
            let lid = crate::layer_cmds::insert_selected(s, Layer::new(lid, name, LayerKind::Fill(fill), &channels))?;
            place.apply(s, lid)?;
            s.selected.insert(ts, lid);
            s.target = crate::session::StackTarget::Content;
            Ok((lid, names))
        })?;
        self.cache_decoded(names, maps);
        Ok(id)
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
        let (texture_set, _, _) = self.doc.project.locate(id).ok_or_else(|| EngineError::Other(format!("layer {} is not in a stack", id.0)))?;
        let mut values = self.link_values(id)?;
        values.extend(overrides.clone());

        let mut inputs = Vec::new();
        if let Some(baked) = self.doc.baked.get(&texture_set) {
            for identifier in &link.inputs {
                let Some((map, data)) = BakedMap::parse(identifier).and_then(|map| Some((map, baked.maps.get(&map)?))) else { continue };

                let png = texturecraft_formats::mcmat::mesh_map_png(baked.resolution, map.components(), data)
                    .map_err(|error| EngineError::Other(error.to_string()))?;
                inputs.push((identifier.clone(), png));
            }
        }

        // UDIM tiles share one cook at the texture set's base resolution
        let size = self.doc.project.texture_set(texture_set)?.resolution;
        Ok(LinkJob {
            layer: id,
            package: self.link_package(id)?,
            source_ext: source_extension(&link.package),
            graph: Some(link.graph.clone()),
            size,
            values,
            inputs,
        })
    }

    /// Swap in maps a [`LinkJob`] cooked (`.mcmat` bytes) and the values they were cooked with.
    /// With `merge`, this folds into the previous undo step when that was an edit of the same kind
    /// (a parameter set while the cook ran).
    pub fn apply_link_cook(&mut self, id: LayerId, bytes: &[u8], merge: bool) -> Result<()> {
        let material = read_mcmat(bytes).map_err(|error| EngineError::Other(error.to_string()))?;
        let maps = decoded_maps(&material)?;

        let names = self.edit_merging(EDIT, merge, |session| {
            let LayerKind::Fill(mut fill) = session.doc.project.layer(id)?.kind.clone() else {
                return Err(EngineError::Other(format!("layer {} is not a fill", id.0)));
            };

            let link = fill.link.as_mut().ok_or_else(|| EngineError::Other(format!("layer {} is not a MaterialCraft material", id.0)))?;
            for parameter in &mut link.params {
                if let Some(cooked) = material.manifest.parameters.iter().find(|candidate| candidate.id == parameter.id) {
                    parameter.value = cooked.value.clone();
                }
            }
            link.size = material.manifest.size[0];
            let stem = texturecraft_formats::sanitize_resource_name(&link.name);
            let old_names: Vec<String> =
                fill.sources.values().filter_map(|source| if let Source::Image { name, .. } = source { Some(name.clone()) } else { None }).collect();
            let names = put_maps(session, &stem, &maps, &mut fill);
            session.doc.project.layer_mut(id)?.kind = LayerKind::Fill(fill);

            // Duplicated layers may still use the earlier maps
            let mut used_names = Vec::new();
            for set in 0..session.doc.project.texture_sets.len() {
                for root in session.doc.project.texture_sets.get(set).map(|texture_set| texture_set.stack.clone()).unwrap_or_default() {
                    crate::material_lib::images_used(&session.doc.project, root, &mut used_names, 0);
                }
            }
            for name in old_names.iter().filter(|name| !used_names.contains(name)) {
                session.doc.resources.remove(name);
            }

            Ok(names)
        })?;

        // Reuse the validated maps before synchronization considers decoding missing images
        self.cache_decoded(names, maps);
        self.sync_images();

        Ok(())
    }

    /// Keep the images decoded while validating maps instead of decoding them again.
    fn cache_decoded(&mut self, names: Vec<String>, maps: Vec<(Channel, Vec<u8>, texturecraft_formats::DecodedImage)>) {
        for (name, (_, _, img)) in names.into_iter().zip(maps) {
            self.images.insert(name, crate::resource_cmds::decoded(img));
        }
    }
}

/// A resource name not in use: `name`, else `2_name`, `3_name`…
fn unique(s: &Session, name: &str) -> String {
    (1..).map(|n| if n == 1 { name.to_string() } else { format!("{n}_{name}") }).find(|n| !s.doc.resources.contains_key(n)).unwrap_or_else(|| name.to_string())
}

/// Store `maps` as new resources (names carry a fresh id, so the decoded-image cache never
/// confuses two cooks) and point `fill`'s channels at them. Returns the names, in map order.
fn put_maps(s: &mut Session, stem: &str, maps: &[(Channel, Vec<u8>, texturecraft_formats::DecodedImage)], fill: &mut Fill) -> Vec<String> {
    let cook = s.doc.project.alloc_id().0;
    let mut names = Vec::new();
    for (c, bytes, _) in maps {
        let name = unique(s, &format!("{stem}_{}_{cook}.png", c.id()));
        s.doc.resources.insert(name.clone(), Arc::new(bytes.clone()));
        // A normal map the user marked DirectX stays marked through re-cooks.
        let normal_format = match fill.sources.get(c) {
            Some(Source::Image { normal_format, .. }) => *normal_format,
            _ => Default::default(),
        };
        fill.sources.insert(*c, Source::Image { name: name.clone(), normal_format });
        names.push(name);
    }
    names
}

#[cfg(all(test, unix))]
mod subprocess_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn cook_uses_the_saved_graph_and_source_type_without_global_environment() {
        let id = NEXT_JOB.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("texturecraft-cook-test-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cli = dir.join("materialcraft-cli");
        let script = r#"#!/bin/sh
set -eu
case "$1" in
  batch)
    grep -F '"graph":"Second"' "$2" >/dev/null
    grep -F '"id":"file.exportMaterial"' "$2" >/dev/null
    test -f "$(dirname "$2")/source.sbs"
    test -f "$(dirname "$2")/input0.png"
    cp "$(dirname "$0")/result.mcmat" "$(dirname "$2")/out.mcmat"
    ;;
  run)
    test "$(basename "$2")" = "source.mcraft"
    cp "$(dirname "$0")/result.mcmat" "$(dirname "$2")/out.mcmat"
    ;;
  *) exit 1 ;;
esac
"#;
        std::fs::write(&cli, script).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(dir.join("result.mcmat"), b"cooked").unwrap();
        let mut job = LinkJob {
            layer: LayerId(1),
            package: Arc::new(b"source".to_vec()),
            source_ext: "sbs",
            graph: Some("Second".into()),
            size: 64,
            values: Map::new(),
            inputs: vec![("curvature".into(), b"png".to_vec())],
        };
        assert_eq!(job.run_with_cli(&cli).unwrap(), b"cooked");
        job.graph = None;
        job.source_ext = "mcraft";
        assert_eq!(job.run_with_cli(&cli).unwrap(), b"cooked");
        job.graph = Some("wrong".into());
        assert!(job.run_with_cli(&cli).is_err());
        assert!(job.run_with_cli(&dir.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
