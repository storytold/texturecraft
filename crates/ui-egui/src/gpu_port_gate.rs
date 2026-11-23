//! The GPU port's library gate (texturecraft #713: nothing the viewport composites stays on the
//! CPU). Every filter of the library (`filter_lib::FILTERS`, which the Filters shelf and the
//! Properties filter picker list) is added the way `effect.add.filter` adds it, with its library
//! defaults and the channels it starts with, as a content effect and (for mask filters) as a mask
//! effect. Each must compile to a GPU program, or be listed in exactly one package's
//! `LIBRARY_STILL_ON_CPU` (crates/render/src/gpu_composite_*.rs and gpu_neighbourhood_*.rs).
//! Compiling needs no adapter, so this gate never skips. At the end of the port the lists are
//! empty. Package 0 owns this file; packages edit only their own list.

use std::collections::BTreeMap;

use serde_json::json;
use texturecraft_engine::core::LayerId;
use texturecraft_engine::filter_lib::FILTERS;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};
use texturecraft_render::gpu_composite::{CompositeProgram, library_filters_still_on_cpu};

/// Why library filter `id` added to a fill layer (its content, or its mask) is not composited on
/// the GPU, or None when it is.
fn cpu_reason(id: &str, mask: bool) -> Option<String> {
    let mut session = Session::new_project("Library gate", primitives::plane(1.0, 1), "builtin:plane", None, 64).unwrap();
    let added = execute(&mut session, "layer.add.fill", &json!({"color": [0.6, 0.3, 0.2]})).unwrap();
    let layer = LayerId(added["layer"].as_u64().unwrap());
    let target = if mask { "mask" } else { "content" };
    execute(&mut session, "effect.add.filter", &json!({"layer": layer.0, "target": target, "filter": id})).unwrap();
    let (project, pixels, ts) = (&session.doc.project, &session.doc.pixels, session.ts());
    if let Err(reason) = CompositeProgram::try_compile_region(project, pixels, ts, None) {
        return Some(format!("material: {reason}"));
    }
    if mask && let Err(reason) = CompositeProgram::try_compile_mask_preview(project, pixels, ts, layer, None, None) {
        return Some(format!("mask preview: {reason}"));
    }
    None
}

#[test]
fn every_library_filter_composites_on_the_gpu_or_is_listed_by_one_package() {
    let mut listed = BTreeMap::<&str, Vec<&str>>::new();
    for (package, id) in library_filters_still_on_cpu() {
        listed.entry(id).or_default().push(package);
    }
    let mut problems = Vec::new();
    for (id, packages) in &listed {
        if packages.len() > 1 {
            problems.push(format!("{id}: listed by {packages:?}, list it in one package only"));
        }
        if !FILTERS.iter().any(|def| def.id == *id) {
            problems.push(format!("{id}: listed by {packages:?} but not in the filter library"));
        }
    }
    for def in FILTERS {
        let reasons: Vec<String> =
            [false, true].into_iter().filter(|&mask| !mask || def.mask).filter_map(|mask| cpu_reason(def.id, mask).map(|reason| format!("{} {reason}", if mask { "mask" } else { "content" }))).collect();
        match (listed.get(def.id), reasons.is_empty()) {
            (None, false) => problems.push(format!("{}: composites on the CPU ({reasons:?}) but no package lists it", def.id)),
            (Some(packages), true) => problems.push(format!("{}: composites on the GPU now, remove it from {packages:?} LIBRARY_STILL_ON_CPU", def.id)),
            _ => {}
        }
    }
    assert!(problems.is_empty(), "the GPU port's library lists disagree with the compiler:\n{}", problems.join("\n"));
}
