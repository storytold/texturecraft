//! User smart materials: a layer (or folder) saved with its pixels and images as a `.tcmat` file
//! (the `.tcraft` container holding a one-texture-set project), and added back above the
//! selection in any project.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;
use texturecraft_compose::PixelStore;
use texturecraft_core::{LayerId, LayerKind, Project, Source, TextureSet};

use crate::commands::{CommandSpec, always, has_layer};
use crate::materials::Place;
use crate::session::{Document, StackTarget};
use crate::{EngineError, Result, Session, cmd};

/// Smart material file extension.
pub const SMART_MATERIAL_EXT: &str = "tcmat";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("material.save", "Create smart material", ["Layers"], None, "{layer?, path: .tcmat file, name?}", has_layer, save),
        cmd!(
            "material.load",
            "Add smart material from file",
            [],
            None,
            "{path: .tcmat smart material, .mcmat MaterialCraft material or .mcraft package, above?: layer id, inside?: bool, below?: layer id, top?: true, index?: n} — above the selection by default, or placed like material.apply",
            always,
            load
        ),
    ]
}

fn fe(e: texturecraft_formats::FormatError) -> EngineError {
    EngineError::Other(e.to_string())
}

/// Image resources a layer subtree uses (fill sources and their patterns, mask or content effects and the images its paths paint with)
pub(crate) fn images_used(project: &Project, id: LayerId, out: &mut Vec<String>, depth: usize) {
    if depth > texturecraft_core::project::MAX_DEPTH {
        return;
    }

    let Some(layer) = project.layers.get(&id) else { return };

    if let LayerKind::Fill(fill) = &layer.kind {
        out.extend(fill.sources.values().flat_map(Source::images).map(str::to_string));
        if let Some(link) = &fill.link {
            out.push(link.package.clone());
            out.extend(link.outputs.values().cloned());
        }
    }

    for effect in layer.mask.iter().flat_map(|mask| mask.effects.iter()).chain(&layer.content_effects) {
        out.extend(effect.images_read().map(str::to_string));
        out.extend(crate::path_tool::path_images(&effect.kind));
    }

    for child in layer.children() {
        images_used(project, *child, out, depth + 1);
    }
}

/// An empty document with one texture set shaped like `src_set` (size, tiles, channels): what a saved library file holds.
pub(crate) fn one_set_document(src_set: &TextureSet, name: &str) -> Document {
    let mut project = Project::new(name, "builtin:none");
    let mut set = TextureSet::new(name, 0, src_set.resolution);
    set.uv_tiles = src_set.uv_tiles.clone();
    set.tile_resolutions = src_set.tile_resolutions.clone();
    set.channels = src_set.channels.clone();
    project.texture_sets.push(set);
    Document {
        bake_meshes: Default::default(),
        bake_paint: BTreeMap::new(),
        project,
        pixels: PixelStore::default(),
        baked: BTreeMap::new(),
        resources: BTreeMap::new(),
    }
}

impl Session {
    /// Serialize layer `id` (with children, pixels and images) as a smart material file.
    pub fn smart_material_bytes(&self, id: LayerId, name: &str) -> Result<Vec<u8>> {
        let (ts, _, _) = self.doc.project.locate(id).ok_or_else(|| EngineError::Other("layer not in a stack".into()))?;
        let mut mini = one_set_document(self.doc.project.texture_set(ts)?, name);
        let new = crate::layer_ops::copy_tree(&self.doc, id, &mut mini, 0, None, false, 0)?;
        if let Ok(l) = mini.project.layer_mut(new) {
            l.name = name.to_string();
        }
        self.file_bytes(mini, new)
    }

    /// The smart material file holding `mini` (a document from [`one_set_document`]) and the images layer `root` of it uses.
    pub(crate) fn file_bytes(&self, mini: Document, root: LayerId) -> Result<Vec<u8>> {
        let mut reservation = texturecraft_formats::RamReservation::for_save().map_err(fe)?;
        let mut used = Vec::new();
        images_used(&mini.project, root, &mut used, 0);
        let resources = used.into_iter().filter_map(|name| self.doc.resources.get(&name).map(|bytes| (name, Arc::clone(bytes)))).collect();

        let snapshot = texturecraft_formats::ProjectFile::<Arc<Vec<u8>>> {
            project: mini.project,
            pixels: mini.pixels,
            baked: BTreeMap::new(),
            mesh: None,
            mesh_uvs: None,
            high_poly: Vec::new(),
            cage: None,
            bake_paint: BTreeMap::new(),
            thumbnail: None,
            resources,
            warnings: Vec::new(),
        };
        reservation.save_bytes(&snapshot).map_err(fe)
    }

    /// Add a smart material file above the selection (one undo step). Returns the new layer.
    pub fn add_smart_material_bytes(&mut self, bytes: &[u8]) -> Result<LayerId> {
        self.add_smart_material_bytes_at(bytes, Place::Selection)
    }

    /// [`Session::add_smart_material_bytes`] at `place` (shelf drag and drop). MaterialCraft
    /// portable materials and packages become linked fill layers.
    pub fn add_smart_material_bytes_at(&mut self, bytes: &[u8], place: Place) -> Result<LayerId> {
        if texturecraft_formats::mcmat::is_mcmat(bytes) || texturecraft_formats::mcmat::is_mcraft(bytes) {
            return self.add_linked_material_bytes_at(bytes, place);
        }

        self.add_smart_material_project(texturecraft_formats::load_project_bytes(bytes).map_err(fe)?, place)
    }

    fn add_smart_material_project(&mut self, project_file: texturecraft_formats::ProjectFile, place: Place) -> Result<LayerId> {
        let root = project_file
            .project
            .texture_sets
            .first()
            .and_then(|t| t.stack.last().copied())
            .ok_or_else(|| EngineError::Other("the smart material file has no layer".into()))?;

        let resources: BTreeMap<String, Arc<Vec<u8>>> = project_file.resources.into_iter().map(|(k, v)| (k, Arc::new(v))).collect();
        let source = Document {
            bake_meshes: Default::default(),
            bake_paint: BTreeMap::new(),
            project: project_file.project,
            pixels: project_file.pixels,
            baked: BTreeMap::new(),
            resources,
        };

        let id = self.edit("Add smart material", |s| {
            for (name, bytes) in &source.resources {
                s.doc.resources.entry(name.clone()).or_insert_with(|| Arc::clone(bytes));
            }

            let ts = s.ts();
            let sel = s.selected_layer();
            let inside = sel.and_then(|l| s.doc.project.layer(l).ok()).is_some_and(|l| l.is_group());
            let id = crate::layer_ops::copy_tree(&source, root, &mut s.doc, ts, sel, inside, 0)?;
            place.apply(s, id)?;
            s.select_one(ts, id);
            s.target = StackTarget::Content;
            Ok(id)
        })?;

        self.sync_images();
        Ok(id)
    }
}

#[cfg(not(target_arch = "wasm32"))]
use crate::commands::{bad, layer_param, strp};
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;

#[cfg(not(target_arch = "wasm32"))]
fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, "material.save", p)?;
    let path = strp(p, "path").ok_or_else(|| bad("material.save", "`path` is required"))?;
    let name = strp(p, "name").map(str::to_string).unwrap_or_else(|| s.doc.project.layer(id).map(|l| l.name.clone()).unwrap_or_default());
    let bytes = s.smart_material_bytes(id, &name)?;
    let path = if path.ends_with(&format!(".{SMART_MATERIAL_EXT}")) { path.to_string() } else { format!("{path}.{SMART_MATERIAL_EXT}") };
    if let Some(dir) = std::path::Path::new(&path).parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| bad("material.save", format!("cannot create {}: {e}", dir.display())))?;
    }
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| bad("material.save", format!("cannot write {path}: {e}")))?;
    Ok(json!({ "path": path }))
}

#[cfg(not(target_arch = "wasm32"))]
fn load(s: &mut Session, p: &Value) -> Result<Value> {
    let path = strp(p, "path").ok_or_else(|| bad("material.load", "`path` is required"))?;
    let source = std::path::Path::new(path);
    let place = Place::from_params(p);
    let project = if crate::link_cmds::is_link_file(source) {
        None
    } else {
        let file = crate::resource_cmds::open_resource_file("material.load", source)?;
        texturecraft_formats::project_file::load_smart_material_project(file).map_err(fe)?
    };

    let id = if let Some(project) = project {
        s.add_smart_material_project(project, place)?
    } else {
        let set = s.ts();
        let resolution = s.doc.project.texture_set(set)?.resolution;
        let (source, cooked) = crate::link_cmds::cook_link_file(source, resolution, s.baked(set).cloned(), s.link_memory_budget(), Arc::default())?;
        s.add_cooked_material(&cooked, source.package, source.source_ext, place)?
    };
    Ok(json!({ "layer": id.0 }))
}

#[cfg(target_arch = "wasm32")]
fn save(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Other("file paths are not available in the browser".into()))
}

#[cfg(target_arch = "wasm32")]
fn load(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Other("file paths are not available in the browser".into()))
}
