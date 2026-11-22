//! Imports of real projects (`TC_IMPORT_CORPUS`, see `common`): every file imports without an error into a valid
//! project whose locks, placeholders and report agree. Ignored by default; nothing read is kept.

use crate::common;

use std::collections::BTreeSet;

use texturecraft_core::{LayerRole, Project};
use texturecraft_import::{Cancel, Hooks, ImportOptions, import_path};

/// Every layer of every set, with its id unique across the project's layers and effects.
fn check(project: &Project) -> Result<(), String> {
    project.validate().map_err(|e| e.to_string())?;
    let mut ids = BTreeSet::new();
    for l in project.layers.values() {
        if !ids.insert(l.id) {
            return Err(format!("layer id {} twice", l.id.0));
        }
        for e in l.mask.iter().flat_map(|m| m.effects.iter()).chain(&l.content_effects) {
            if !ids.insert(e.id) {
                return Err(format!("effect id {} twice", e.id.0));
            }
        }
        if l.placeholders().next().is_some() && !l.is_locked() {
            return Err(format!("layer \"{}\" holds a placeholder but is not locked", l.name));
        }
        if let Some(lock) = &l.lock
            && (lock.reasons.is_empty() || lock.keys().iter().any(|k| !k.contains(':') && *k != "original-result"))
        {
            return Err(format!("layer \"{}\" has a lock without keyed reasons: {lock:?}", l.name));
        }
    }
    let report = project.import.as_ref().ok_or("no import report")?;
    for id in report.layers.keys() {
        if !project.layers.contains_key(id) {
            return Err(format!("the report names layer {} that is not in the project", id.0));
        }
    }
    let results = report.layers.values().filter(|r| r.role == LayerRole::OriginalResult).count();
    if results > project.texture_sets.len() {
        return Err(format!("{results} original results for {} texture sets", project.texture_sets.len()));
    }
    Ok(())
}

#[test]
#[ignore = "needs TC_IMPORT_CORPUS"]
fn corpus_projects_import_into_valid_projects() {
    let Some(files) = common::files() else { return };
    let mut failures = Vec::new();
    for file in &files {
        let cancel = Cancel::new();
        let mut hooks = Hooks { progress: &|_, _| {}, cancel: &cancel, replay: None, kept_cooks: None, trace: None, accelerator: None, preview: None };
        let result = import_path(file, &ImportOptions::default(), &mut hooks).map_err(|e| e.to_string()).and_then(|i| check(&i.project));
        if let Err(e) = result {
            failures.push(format!("{}: {e}", file.display()));
        }
    }
    assert!(failures.is_empty(), "{} of {} failed:\n{}", failures.len(), files.len(), failures.join("\n"));
}

#[test]
#[ignore = "needs TC_IMPORT_CORPUS"]
fn a_cancelled_import_stops() {
    let Some(files) = common::files() else { return };
    let Some(file) = files.first() else { return };
    let cancel = Cancel::new();
    cancel.cancel();
    let mut hooks = Hooks { progress: &|_, _| {}, cancel: &cancel, replay: None, kept_cooks: None, trace: None, accelerator: None, preview: None };
    assert!(matches!(import_path(file, &ImportOptions::default(), &mut hooks), Err(texturecraft_import::ImportError::Cancelled)));
}
