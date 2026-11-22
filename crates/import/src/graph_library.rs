//! [`GraphLibrary`]: compiled graphs referenced by imported texture sets, decompiled into graph text once per import
//! and looked up by name and package version.
//!
//! The import decompiles by default ([`crate::ImportOptions::graphs`]): a placeholder stands for nothing and locks its
//! layer, a decompiled graph computes. Keeping graphs as placeholders remains an option.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use texturecraft_core::{BakedMap, GraphValue};
use texturecraft_import_reader::decode::graph_package::GraphPackage;
use texturecraft_import_reader::decompile::{Decompiled, decompile};
use texturecraft_import_reader::format_names::PACKAGE_SUFFIX;
use texturecraft_import_reader::model::{Procedural, ProjectEffect, ProjectSource, ProjectTextureSet};
use texturecraft_import_reader::project::ProjectFile;

use crate::native_graphs::{NativeGraph, recognize};
use crate::{Cancel, ImportError};

/// Nominal size of decompiled graphs (they render at the texture set's resolution).
const NOMINAL: usize = 512;

pub(crate) fn package_version(p: &Procedural) -> Option<&str> {
    p.url.split_once("?version=").map(|(_, version)| version.trim_end_matches(PACKAGE_SUFFIX))
}

/// A decompiled graph with its text key.
#[derive(Debug, Clone)]
pub struct LibraryGraph {
    pub graph: Arc<Decompiled>,
    /// [`texturecraft_core::text_key`] of its text.
    pub key: String,
    pub(crate) package_version: Option<String>,
    pub(crate) native: Option<NativeGraph>,
}

/// See the module docs.
#[derive(Debug, Default)]
pub struct GraphLibrary {
    /// One graph is found under several names (its name, its package path): they share it, as a graph's text carries
    /// its embedded images.
    by_name: BTreeMap<String, LibraryGraph>,
    /// By package version and graph name.
    by_version: BTreeMap<(String, String), LibraryGraph>,
    by_key: BTreeMap<String, Arc<Decompiled>>,
    /// Grey renders already made ([`crate::graph_uses::render_grey`]), by cook key, output and side: a brush tip is
    /// rendered once and shared by every brush and stamp using it (a project lays one tip thousands of times).
    grey: std::sync::Mutex<GreyRenders>,
}

/// See [`GraphLibrary::shared_grey`].
#[derive(Debug, Default)]
struct GreyRenders {
    by_key: std::collections::HashMap<String, Option<Arc<[f32]>>>,
    /// Bytes of values held.
    bytes: usize,
}

/// Most bytes of grey renders the library keeps for sharing; past it a render is made and not kept.
const MAX_GREY_BYTES: usize = 1 << 30;

fn used_graph_names<'a>(sets: impl Iterator<Item = &'a ProjectTextureSet>) -> HashSet<&'a str> {
    let mut names = HashSet::new();
    let mut effects = Vec::new();
    let mut sources = Vec::new();
    let mut procedurals = Vec::new();

    for layer in sets.flat_map(|set| set.layers.iter().flat_map(|layer| layer.walk())) {
        effects.extend(&layer.content);
        if let Some(mask) = &layer.mask {
            effects.extend(&mask.effects);
        }
        while let Some(effect) = effects.pop() {
            match effect {
                ProjectEffect::Fill(fill) => sources.extend(&fill.sources),
                ProjectEffect::Paint(paint) => {
                    sources.extend(&paint.colors);
                    sources.extend(&paint.alpha);
                    sources.extend(paint.stencil.iter());
                }
                ProjectEffect::Group(group) => effects.extend(&group.effects),
                ProjectEffect::Filter(filter) => procedurals.extend(filter.procedural.iter()),
                _ => {}
            }
        }
        while let Some(source) = sources.pop() {
            if let ProjectSource::Procedural(source) = source {
                procedurals.extend(source.procedural.iter());
            }
        }
        while let Some(procedural) = procedurals.pop() {
            names.insert(procedural.name());
            for input in &procedural.inputs {
                if let Some(ProjectSource::Procedural(source)) = input.source.as_deref() {
                    procedurals.extend(source.procedural.iter());
                }
            }
        }
    }
    names
}

impl GraphLibrary {
    /// Decompiles candidate graphs for the sets being imported, one package at a time.
    pub(crate) fn for_sets<'a>(project: &ProjectFile, sets: impl Iterator<Item = &'a ProjectTextureSet>, cancel: &Cancel) -> Result<GraphLibrary, ImportError> {
        let names = used_graph_names(sets);
        let mut lib = GraphLibrary::default();
        if names.is_empty() {
            return Ok(lib);
        }
        for r in project.resources().iter().filter(|r| r.file_name.ends_with(PACKAGE_SUFFIX)) {
            if cancel.is_cancelled() {
                return Err(ImportError::Cancelled);
            }
            if let Ok(data) = project.read(&r.path) {
                // Some references omit a version or name a version that is missing. Keep matching
                // names in their original package order so both versioned and fallback lookups work.
                lib.add_matching_package(r.file_name.trim_end_matches(PACKAGE_SUFFIX), &data, |name, path| {
                    names.contains(name) || path.is_some_and(|path| names.contains(path))
                });
            }
        }
        Ok(lib)
    }

    /// Decompiles every graph of one package (its bytes) with its version (empty when unknown).
    pub fn add_package(&mut self, version: &str, data: &[u8]) {
        self.add_matching_package(version, data, |_, _| true);
    }

    fn add_matching_package(&mut self, version: &str, data: &[u8], wanted: impl Fn(&str, Option<&str>) -> bool) {
        let Ok(archive) = GraphPackage::read(data) else { return };
        for g in archive.graphs() {
            let Ok(desc) = g.description() else { continue };
            let package_path = desc.pkg_url.strip_prefix("pkg://").map(|p| p.trim_start_matches('/'));
            if !wanted(g.name(), package_path) {
                continue;
            }
            let Ok(asm) = g.assembly() else { continue };
            let d = decompile(&desc, &asm, NOMINAL);
            let key = texturecraft_core::text_key(&d.text);
            let native = recognize(version, &desc.pkg_url, &key, g.compiled_graph);
            let mut names = vec![g.name().to_string()];
            names.extend(package_path.map(str::to_owned));
            self.insert_graph(&names, version, d, key, native);
        }
    }

    /// Adds a graph under `names` (keeping graphs already there), and under `version` when it is not empty.
    pub fn insert_as(&mut self, names: &[String], version: &str, d: Decompiled) {
        let key = texturecraft_core::text_key(&d.text);
        self.insert_graph(names, version, d, key, None);
    }

    fn insert_graph(&mut self, names: &[String], version: &str, d: Decompiled, key: String, native: Option<NativeGraph>) {
        let graph = self.by_key.entry(key.clone()).or_insert_with(|| Arc::new(d)).clone();
        let entry = LibraryGraph { graph, key, package_version: (!version.is_empty()).then(|| version.to_owned()), native };
        for name in names {
            if !version.is_empty() {
                self.by_version.insert((version.to_string(), name.clone()), entry.clone());
            }
            self.by_name.entry(name.clone()).or_insert_with(|| entry.clone());
        }
    }

    /// The graph a procedural runs: of the package version its URL names, else any of its name.
    pub fn of(&self, p: &Procedural) -> Option<&LibraryGraph> {
        let name = p.name();
        package_version(p).and_then(|v| self.by_version.get(&(v.to_owned(), name.to_owned()))).or_else(|| self.by_name.get(name))
    }

    /// The text under `key`, if a graph of the library has it.
    pub fn text(&self, key: &str) -> Option<Arc<str>> {
        self.by_key.get(key).map(|g| g.text.clone())
    }

    /// The grey render kept under `key`, or `render()` kept under it (within [`MAX_GREY_BYTES`]). A render depends only
    /// on what its cook key covers, so a kept one is the same values.
    pub fn shared_grey(&self, key: String, render: impl FnOnce() -> Option<Arc<[f32]>>) -> Option<Arc<[f32]>> {
        if let Some(hit) = self.grey.lock().ok().and_then(|m| m.by_key.get(&key).cloned()) {
            return hit;
        }
        let values = render();
        if let Ok(mut m) = self.grey.lock() {
            let bytes = values.as_ref().map_or(0, |v| v.len() * size_of::<f32>());
            if m.bytes + bytes <= MAX_GREY_BYTES && !m.by_key.contains_key(&key) {
                m.bytes += bytes;
                m.by_key.insert(key, values.clone());
            }
        }
        values
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

/// The baked map a graph's image input of this name reads, if it names a mesh map (file data: the input identifiers the
/// reference app's graphs use, a misspelt one included).
pub fn baked_map_for(identifier: &str) -> Option<BakedMap> {
    Some(match identifier.to_ascii_lowercase().as_str() {
        "position" | "position_map" => BakedMap::Position,
        "world_space_normals" | "world_normal" | "worldspacenormals" | "normal_world" | "normaws" | "normalws" => BakedMap::WorldNormal,
        "ambient_occlusion" | "ambientocclusion" | "ao" => BakedMap::AmbientOcclusion,
        "curvature" => BakedMap::Curvature,
        "thickness" => BakedMap::Thickness,
        "normal" | "normal_map" => BakedMap::Normal,
        "id" | "id_map" => BakedMap::Id,
        _ => return None,
    })
}

/// A tweak's value as a graph parameter value; `None` for values of unknown classes.
pub fn tweak_value(v: &texturecraft_import_reader::model::TweakValue) -> Option<GraphValue> {
    use texturecraft_import_reader::model::TweakValue;
    let finite = |x: f32| if x.is_finite() { x } else { 0.0 };
    Some(match v {
        TweakValue::Int(i) => GraphValue::Int(*i),
        TweakValue::Int2(v) => GraphValue::Int2(*v),
        TweakValue::Floats(f) => match f.as_slice() {
            [] => return None,
            [x] => GraphValue::Float(finite(*x)),
            many => GraphValue::Vector(many.iter().copied().map(finite).collect()),
        },
        TweakValue::Text(t) => GraphValue::Text(t.clone()),
        TweakValue::Other(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(text: &str) -> Decompiled {
        Decompiled { text: text.into(), images: Vec::new(), image_defaults: Vec::new(), outputs: Vec::new(), warnings: Vec::new(), colour_outputs: Vec::new() }
    }

    fn procedural(url: &str) -> Procedural {
        Procedural { uid: None, url: url.into(), tweaks: Vec::new(), outputs: Vec::new(), inputs: Vec::new(), alpha_output: String::new(), mono_output: 0 }
    }

    #[test]
    fn the_package_version_a_procedural_names_wins_over_the_name() {
        let mut lib = GraphLibrary::default();
        lib.insert_as(&["g".into()], "", graph("first"));
        lib.insert_as(&["g".into()], "v2", graph("second"));
        let text = |p: &Procedural| lib.of(p).map(|g| g.graph.text.to_string());
        assert_eq!(text(&procedural(&format!("resource://shelf/g/g?version=v2{PACKAGE_SUFFIX}"))).as_deref(), Some("second"));
        assert_eq!(text(&procedural("resource://shelf/g/g?version=unknown")).as_deref(), Some("first"));
        assert_eq!(text(&procedural("/g")).as_deref(), Some("first"));
        assert!(lib.of(&procedural("/missing")).is_none());
        assert_eq!(lib.len(), 2);
        let key = texturecraft_core::text_key("second");
        assert_eq!(lib.text(&key).as_deref(), Some("second"));
    }

    #[test]
    fn unreadable_packages_add_nothing() {
        let mut lib = GraphLibrary::default();
        lib.add_package("v", b"not a package");
        lib.add_package("v", &[]);
        assert!(lib.is_empty());
    }

    #[test]
    fn mesh_map_inputs_and_tweaks_map_over() {
        assert_eq!(baked_map_for("normaWS"), Some(BakedMap::WorldNormal), "the misspelt channel tag some graphs use");
        assert_eq!(baked_map_for("Ambient_Occlusion"), Some(BakedMap::AmbientOcclusion));
        assert_eq!(baked_map_for("texture"), None);
        use texturecraft_import_reader::model::TweakValue;
        assert_eq!(tweak_value(&TweakValue::Floats(vec![0.5])), Some(GraphValue::Float(0.5)));
        assert_eq!(tweak_value(&TweakValue::Floats(vec![1.0, f32::NAN])), Some(GraphValue::Vector(vec![1.0, 0.0])));
        assert_eq!(tweak_value(&TweakValue::Int2([8, 8])), Some(GraphValue::Int2([8, 8])));
        assert_eq!(tweak_value(&TweakValue::Text("x".into())), Some(GraphValue::Text("x".into())));
        assert_eq!(tweak_value(&TweakValue::Floats(Vec::new())), None);
    }
}
