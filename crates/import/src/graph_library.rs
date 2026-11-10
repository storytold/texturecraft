//! [`GraphLibrary`]: every compiled graph of a project's embedded packages, decompiled once per import into graph text
//! (the reader's decompiler) and looked up by the graph name a procedural uses, of its own package version where the
//! project holds several.
//!
//! The import decompiles by default ([`crate::ImportOptions::graphs`]): a placeholder stands for nothing and locks its
//! layer, a decompiled graph computes. Keeping graphs as placeholders remains an option.

use std::collections::BTreeMap;
use std::sync::Arc;

use texturecraft_core::{BakedMap, GraphValue};
use texturecraft_import_reader::decode::graph_package::GraphPackage;
use texturecraft_import_reader::decompile::{Decompiled, decompile};
use texturecraft_import_reader::format_names::PACKAGE_SUFFIX;
use texturecraft_import_reader::model::Procedural;
use texturecraft_import_reader::project::ProjectFile;

/// Nominal size of decompiled graphs (they render at the texture set's resolution).
const NOMINAL: usize = 512;

/// A decompiled graph with its text key.
#[derive(Debug, Clone)]
pub struct LibraryGraph {
    pub graph: Arc<Decompiled>,
    /// [`texturecraft_core::text_key`] of its text.
    pub key: String,
}

/// See the module docs.
#[derive(Debug, Default)]
pub struct GraphLibrary {
    /// One graph is found under several names (its name, its package path): they share it, as a graph's text carries
    /// its embedded images.
    by_name: BTreeMap<String, LibraryGraph>,
    /// By package version and graph name.
    by_version: BTreeMap<(String, String), LibraryGraph>,
    by_key: BTreeMap<String, LibraryGraph>,
}

impl GraphLibrary {
    /// Decompiles every graph of the project's embedded packages, one package's bytes at a time. Packages or graphs
    /// that cannot be read are left out (their uses stay placeholders).
    pub fn from_project(project: &ProjectFile) -> GraphLibrary {
        let mut lib = GraphLibrary::default();
        for r in project.resources().iter().filter(|r| r.file_name.ends_with(PACKAGE_SUFFIX)) {
            if let Ok(data) = project.read(&r.path) {
                lib.add_package(r.file_name.trim_end_matches(PACKAGE_SUFFIX), &data);
            }
        }
        lib
    }

    /// Decompiles every graph of one package (its bytes) with its version (empty when unknown).
    pub fn add_package(&mut self, version: &str, data: &[u8]) {
        let Ok(archive) = GraphPackage::read(data) else { return };
        for g in archive.graphs() {
            let (Ok(desc), Ok(asm)) = (g.description(), g.assembly()) else { continue };
            let d = decompile(&desc, &asm, NOMINAL);
            let package_path = desc.pkg_url.strip_prefix("pkg://").map(|p| p.trim_start_matches('/').to_string());
            let mut names = vec![g.name().to_string()];
            names.extend(package_path);
            self.insert_as(&names, version, d);
        }
    }

    /// Adds a graph under `names` (keeping graphs already there), and under `version` when it is not empty.
    pub fn insert_as(&mut self, names: &[String], version: &str, d: Decompiled) {
        let key = texturecraft_core::text_key(&d.text);
        let entry = self.by_key.entry(key.clone()).or_insert_with(|| LibraryGraph { graph: Arc::new(d), key }).clone();
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
        let version = p.url.split_once("?version=").map(|(_, v)| v.trim_end_matches(PACKAGE_SUFFIX).to_string());
        version.and_then(|v| self.by_version.get(&(v, name.to_string()))).or_else(|| self.by_name.get(name))
    }

    /// The text under `key`, if a graph of the library has it.
    pub fn text(&self, key: &str) -> Option<Arc<str>> {
        self.by_key.get(key).map(|g| g.graph.text.clone())
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

/// A tweak's value as a graph parameter value; `None` for text and values of unknown classes.
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
        TweakValue::Text(_) | TweakValue::Other(_) => return None,
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
        assert_eq!(tweak_value(&TweakValue::Text("x".into())), None);
        assert_eq!(tweak_value(&TweakValue::Floats(Vec::new())), None);
    }
}
