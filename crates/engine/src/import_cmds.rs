//! `file.importProject`: a project file of the reference app becomes a TextureCraft project, which opens as the
//! session's project. The conversion is one-way: the original file is only read.
//!
//! The translator (`texturecraft-import`) builds the project, its layer pixels, baked maps and image resources; here the
//! mesh is stored as a binary glTF file in the project (the original's mesh format is not kept) and the result opens
//! through the same path as a saved project, so an imported project is exactly what its saved file will hold.
//! Brush strokes are replayed with TextureCraft's brush by [`crate::import_replay`], which the translator calls per texture
//! set. Programmatic calls never open dialogs; the two-window import (notice, progress) belongs to the UI.

use serde_json::{Value, json};
use texturecraft_formats::ProjectFile;
use texturecraft_import::{Hooks, ImportOptions, Imported};

use crate::commands::{CommandSpec, always, bad, boolp, strp};
use crate::{EngineError, Result, Session, cmd};

const ID: &str = "file.importProject";
/// Largest project file accepted as `bytes` (the web build holds it in memory).
const MAX_BYTES: usize = 2 << 30;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "file.importProject",
        "Import project...",
        [],
        None,
        "{path?: the project file (native only), bytes?: the project file as base64 (any target), name?: file name for the report (with bytes), originalOnTop?: bool (show the original's stored result on top of stacks that are not reproduced; default false), nativeFilters?: bool (replace a few filter graphs by close built-in filters; default false), maxResolution?: px (cap texture set resolutions), measure?: bool (compare each stack with the stored result; default true), replayStrokes?: bool (replay brush strokes with TextureCraft's brush, approximated; default true), graphs?: \"decompile\" | \"keep\" (run the project's compiled graphs decompiled, or keep them as locked placeholders; default decompile)} → the session, plus {import: {sets: [{name, strategy, layers, locked, placeholders, channels, scores}]}}. One-way: the original file is only read",
        always,
        import
    )]
}

/// The import options from command params.
fn options(p: &Value) -> Result<ImportOptions> {
    let mut o = ImportOptions::default();
    if let Some(v) = boolp(p, "originalOnTop") {
        o.original_on_top = v;
    }
    if let Some(v) = boolp(p, "nativeFilters") {
        o.native_filters = v;
    }
    if let Some(v) = boolp(p, "measure") {
        o.measure = v;
    }
    if let Some(v) = boolp(p, "replayStrokes") {
        o.replay_strokes = v;
    }
    match p.get("graphs").filter(|v| !v.is_null()).map(|v| v.as_str()) {
        None => {}
        Some(Some("decompile")) => o.graphs = texturecraft_import::GraphMode::Decompile,
        Some(Some("keep")) => o.graphs = texturecraft_import::GraphMode::Keep,
        Some(_) => return Err(bad(ID, "`graphs` must be \"decompile\" or \"keep\"")),
    }
    if let Some(r) = p.get("maxResolution").filter(|v| !v.is_null()) {
        let r = r.as_u64().and_then(|r| u32::try_from(r).ok()).filter(|r| r.is_power_of_two() && (128..=8192).contains(r));
        o.max_resolution = Some(r.ok_or_else(|| bad(ID, "`maxResolution` must be a power of two from 128 to 8192"))?);
    }
    Ok(o)
}

impl Session {
    /// A new session from an imported project: the mesh stored as binary glTF, then opened like a saved project.
    pub fn from_imported(imported: Imported) -> Result<Session> {
        let name = imported.project.name.clone();
        let glb =
            texturecraft_formats::glb_bytes(&imported.mesh, &name, &[]).map_err(|e| EngineError::Other(format!("cannot store the imported mesh: {e}")))?;
        let mut project = imported.project;
        project.mesh = "mesh.glb".into();
        project.mesh_file = "mesh.glb".into();
        let pf = ProjectFile {
            project,
            pixels: imported.pixels,
            baked: imported.baked,
            mesh: Some(("glb".into(), glb)),
            high_poly: Vec::new(),
            cage: None,
            bake_paint: Default::default(),
            thumbnail: None,
            resources: imported.resources,
            graphs: imported.graphs,
            warnings: Vec::new(),
        };
        let mut s = Session::from_project_file(pf)?;
        // open with the view the original was saved with
        if s.doc.project.cameras.iter().any(|c| c.name == texturecraft_import::cameras::SAVED_VIEW) {
            let _ = crate::execute(&mut s, "camera.select", &json!({"name": texturecraft_import::cameras::SAVED_VIEW}));
        }
        s.modified = true;
        Ok(s)
    }

    /// Imports the project file `bytes` (`name` names it) and opens it as the session's project.
    pub fn import_project_bytes(&mut self, bytes: Vec<u8>, name: &str, options: &ImportOptions) -> Result<Value> {
        if bytes.len() > MAX_BYTES {
            return Err(bad(ID, format!("the file is {} bytes; at most {MAX_BYTES} are read in memory", bytes.len())));
        }
        let mut progress = |_: f32, _: &str| {};
        let mut hooks = Hooks { progress: &mut progress, cancel: &|| false, replay: Some(&crate::import_replay::BrushReplayer) };
        let imported = texturecraft_import::import_bytes(bytes, name, options, &mut hooks).map_err(|e| bad(ID, e.to_string()))?;
        self.open_imported(imported)
    }

    fn open_imported(&mut self, imported: Imported) -> Result<Value> {
        let summary = summary(&imported);
        let next = Session::from_imported(imported)?;
        self.switch_to(next);
        let mut reply = crate::inspect::session(self);
        if let Some(o) = reply.as_object_mut() {
            o.insert("import".into(), summary);
        }
        Ok(reply)
    }
}

/// What the safety net found per texture set.
fn summary(imported: &Imported) -> Value {
    let sets: Vec<Value> = imported
        .sets
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "strategy": s.strategy.id(),
                "layers": s.layers,
                "locked": s.locked,
                "placeholders": s.placeholders,
                "channels": s.channels.iter().map(|(c, st)| json!({"channel": c.id(), "status": st})).collect::<Vec<_>>(),
                "scores": s.scores.iter().map(|(c, sc)| json!({"channel": c.id(), "mean": sc.mean, "max": sc.max, "texels": sc.texels})).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({ "sets": sets })
}

/// Base64 (standard alphabet, padding optional) or a JSON array of byte values.
fn bytes_param(v: &Value) -> Result<Vec<u8>> {
    match v {
        Value::String(s) => decode_base64(s).ok_or_else(|| bad(ID, "`bytes` is not valid base64")),
        Value::Array(a) => {
            if a.len() > MAX_BYTES {
                return Err(bad(ID, "`bytes` is too long"));
            }
            a.iter()
                .map(|x| x.as_u64().and_then(|b| u8::try_from(b).ok()))
                .collect::<Option<Vec<u8>>>()
                .ok_or_else(|| bad(ID, "`bytes` must hold values 0 to 255"))
        }
        _ => Err(bad(ID, "`bytes` must be a base64 string or an array of byte values")),
    }
}

/// Standard base64; `None` on any character outside the alphabet.
fn decode_base64(s: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let clean = clean.strip_suffix(b"==").or_else(|| clean.strip_suffix(b"=")).unwrap_or(&clean);
    if clean.len() / 4 * 3 > MAX_BYTES {
        return None;
    }
    let mut out = Vec::with_capacity(clean.len() / 4 * 3 + 3);
    for chunk in clean.chunks(4) {
        let mut acc = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            acc |= value(*c)? << (18 - 6 * i as u32);
        }
        let bytes = acc.to_be_bytes();
        match chunk.len() {
            4 => out.extend_from_slice(&bytes[1..4]),
            3 => out.extend_from_slice(&bytes[1..3]),
            2 => out.push(bytes[1]),
            _ => return None,
        }
    }
    Some(out)
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let options = options(p)?;
    if let Some(v) = p.get("bytes").filter(|v| !v.is_null()) {
        let bytes = bytes_param(v)?;
        let name = strp(p, "name").unwrap_or("Imported project");
        return s.import_project_bytes(bytes, name, &options);
    }
    import_path(s, p, &options)
}

#[cfg(not(target_arch = "wasm32"))]
fn import_path(s: &mut Session, p: &Value, options: &ImportOptions) -> Result<Value> {
    let path = strp(p, "path").filter(|x| !x.trim().is_empty()).ok_or_else(|| bad(ID, "give `path` (a project file) or `bytes`"))?;
    let mut progress = |_: f32, _: &str| {};
    let mut hooks = Hooks { progress: &mut progress, cancel: &|| false, replay: Some(&crate::import_replay::BrushReplayer) };
    let imported = texturecraft_import::import_path(std::path::Path::new(path), options, &mut hooks).map_err(|e| bad(ID, format!("{path}: {e}")))?;
    s.open_imported(imported)
}

#[cfg(target_arch = "wasm32")]
fn import_path(_: &mut Session, _: &Value, _: &ImportOptions) -> Result<Value> {
    Err(bad(ID, "`path` is not available in the browser: give `bytes`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_and_byte_arrays_decode_and_garbage_is_refused() {
        assert_eq!(decode_base64("aGVsbG8="), Some(b"hello".to_vec()));
        assert_eq!(decode_base64("aGVsbG8"), Some(b"hello".to_vec()));
        assert_eq!(decode_base64("aGk="), Some(b"hi".to_vec()));
        assert_eq!(decode_base64("a"), None);
        assert_eq!(decode_base64("a*bc"), None);
        assert_eq!(bytes_param(&json!([1, 2, 255])).unwrap(), vec![1, 2, 255]);
        assert!(bytes_param(&json!([256])).is_err());
        assert!(bytes_param(&json!(3)).is_err());
    }

    #[test]
    fn garbage_files_are_errors_not_panics() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::builtin("sphere").unwrap(), "builtin:sphere", None, 128).unwrap();
        for p in [json!({}), json!({"bytes": "AAAA"}), json!({"bytes": [0, 1, 2]}), json!({"path": "/nonexistent/file"}), json!({"maxResolution": 100})] {
            assert!(import(&mut s, &p).is_err(), "{p}");
        }
        assert_eq!(s.project().name, "t", "a failed import leaves the session as it was");
    }
}
