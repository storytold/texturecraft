//! MaterialCraft materials (`.mcmat`) as linked fill layers: import, parameters, re-cooks,
//! undo, mesh-map inputs and saving. Cooking through a real `materialcraft-cli` runs only when
//! `MATERIALCRAFT_CLI` points at one.

use std::io::Write;
use std::sync::Arc;

use serde_json::{Value, json};
use texturecraft_engine::compose::BakedMaps;
use texturecraft_engine::core::{BakedMap, Channel, CookResolution, LayerId, LayerKind, NormalFormat, Source};
use texturecraft_engine::formats::mcmat::mesh_map_png;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::link_cmds::{CookedMap, CookedMaterial, LinkMemoryBudget};
use texturecraft_engine::materials::Place;
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 32, 16), "builtin:sphere", None, 128).unwrap();
    s.update();
    s
}

/// A CLI bake with known pixels for testing the linked layer independently of the subprocess.
fn cooked(roughness: f32, wear: f32) -> CookedMaterial {
    let gray = |v: f32| mesh_map_png(4, 1, &[v; 16]).unwrap();
    let interface = json!({
        "name": "Brick wall", "graph": "Brick_wall",
        "parameters": [
            {"id": "wear", "label": "Wear", "default": {"t": "float", "v": 0.5}, "min": 0.0, "max": 1.0, "value": {"t": "float", "v": wear}},
            {"id": "tint", "label": "Tint", "default": {"t": "color", "v": [1.0, 1.0, 1.0, 1.0]}, "value": {"t": "color", "v": [1.0, 1.0, 1.0, 1.0]}},
            {"id": "note", "label": "Note", "default": {"t": "text", "v": ""}, "value": {"t": "text", "v": ""}}
        ],
        "inputs": [
            {"identifier": "curvature", "kind": "gray"},
            {"identifier": "normal", "kind": "color"},
            {"identifier": "worldNormal", "kind": "color"},
            {"identifier": "ambientOcclusion", "kind": "gray"},
            {"identifier": "bentNormal", "kind": "color"}
        ],
        "presets": [{"name": "Worn", "values": {"wear": {"t": "float", "v": 1.0}, "note": {"t": "text", "v": "x"}}}],
    });
    let mut budget = LinkMemoryBudget::new(1 << 20);
    let maps = [(Channel::BaseColor, mesh_map_png(4, 3, &[0.5; 48]).unwrap()), (Channel::Roughness, gray(roughness)), (Channel::AmbientOcclusion, gray(1.0))]
        .into_iter()
        .map(|(channel, bytes)| CookedMap::decode(channel, bytes, [4, 4], &mut budget).unwrap())
        .collect();
    CookedMaterial::new(serde_json::from_value(interface).unwrap(), maps, 4)
}

fn install(session: &mut Session) -> LayerId {
    session.add_cooked_material(&cooked(0.25, 0.5), Arc::new(b"package bytes".to_vec()), "mcmat", Place::Selection).unwrap()
}

fn roughness(s: &mut Session) -> f32 {
    s.update();
    s.caches[0].composite.value(Channel::Roughness, 64, 64)[0]
}

fn link(s: &Session, id: LayerId) -> texturecraft_engine::core::MaterialLink {
    match &s.doc.project.layer(id).unwrap().kind {
        LayerKind::Fill(f) => f.link.as_deref().unwrap().clone(),
        _ => panic!("not a fill"),
    }
}

fn map_name(s: &Session, id: LayerId, c: Channel) -> String {
    match &s.doc.project.layer(id).unwrap().kind {
        LayerKind::Fill(f) => match f.sources.get(&c) {
            Some(Source::Image { name, .. }) => name.clone(),
            other => panic!("{other:?}"),
        },
        _ => panic!("not a fill"),
    }
}

// Real cook reservations must be independent of saves in other parallel integration tests
fn run_in_private_process() -> bool {
    const CHILD_TEST: &str = "TC_MATERIALCRAFT_CHILD_TEST";

    let thread = std::thread::current();
    let name = thread.name().unwrap();
    if std::env::var(CHILD_TEST).as_deref() == Ok(name) {
        return false;
    }

    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--include-ignored", "--nocapture"])
        .env(CHILD_TEST, name)
        .status()
        .unwrap();
    assert!(status.success(), "{name} returned {status}");
    true
}

#[test]
fn version_two_requires_republishing_before_install() {
    if run_in_private_process() {
        return;
    }

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("material.json", zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(br#"{"version":2,"format":"craft-material"}"#).unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    let mut session = session();
    assert!(session.add_linked_material_bytes_at(&bytes, Place::Selection).unwrap_err().to_string().contains("Republish"));
}

#[test]
fn imports_as_a_linked_fill_with_its_maps() {
    let mut s = session();
    let id = install(&mut s);
    assert_eq!(s.selected_layer(), Some(id));
    assert!((roughness(&mut s) - 0.25).abs() < 2e-3);

    // The fill adds its mapped channels and omits text parameters
    assert!(s.doc.project.texture_sets[0].has_channel(Channel::AmbientOcclusion));
    let l = link(&s, id);
    assert_eq!(l.graph, "Brick_wall");
    assert_eq!(l.params.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["wear", "tint"]);
    assert_eq!(l.inputs, ["curvature", "normal", "worldNormal", "ambientOcclusion", "bentNormal"]);
    assert_eq!(s.doc.resources.get(&l.package).map(|b| b.as_slice()), Some(&b"package bytes"[..]));
    assert!(!s.images.contains_key(&l.package), "the package is not decoded as an image");

    // One undo removes the link exposed through inspect
    let inspect = execute(&mut s, "project.inspect", &Value::Null).unwrap().to_string();
    assert!(inspect.contains("Brick_wall"), "{inspect}");
    let n = s.doc.resources.len();
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(s.doc.project.layer(id).is_err() && s.doc.resources.len() < n);

    // Invalid archives leave the stack untouched
    assert!(s.add_linked_material_bytes_at(b"PK not really", Place::Selection).is_err());
}

#[test]
fn a_built_in_material_assignment_preserves_a_linked_fill() {
    let mut session = session();
    let layer = install(&mut session);
    let original_link = link(&session, layer);
    let original_map = map_name(&session, layer, Channel::Roughness);
    let original_history = session.history_labels();

    for material in [json!("copper_hammered"), Value::Null] {
        let error = execute(&mut session, "layer.fill.material", &json!({"layer": layer.0, "material": material})).unwrap_err();

        assert!(error.to_string().contains("linked material"));
        assert_eq!(link(&session, layer), original_link);
        assert_eq!(map_name(&session, layer, Channel::Roughness), original_map);
        assert_eq!(session.history_labels(), original_history);
    }
}

/// An image marked DirectX keeps the mark when a re-cook replaces it.
#[test]
fn recooks_keep_the_normal_format_of_a_map() {
    let mut session = session();
    let id = install(&mut session);
    execute(&mut session, "layer.fill.set", &json!({"layer": id.0, "channel": "roughness", "normalFormat": "directX"})).unwrap();

    session.apply_link_cook(id, &cooked(0.75, 0.5), false).unwrap();

    let LayerKind::Fill(fill) = &session.doc.project.layer(id).unwrap().kind else { panic!("a fill") };

    assert!(matches!(fill.sources.get(&Channel::Roughness), Some(Source::Image { normal_format: NormalFormat::DirectX, .. })));
}

#[test]
fn parameters_are_checked_stored_and_recooked_in_one_undo_step() {
    let mut session = session();
    let id = install(&mut session);
    let set = |session: &mut Session, params: Value| execute(session, "material.link.set", &params);

    assert!(set(&mut session, json!({"id": "nope", "value": 1, "cook": false})).is_err());
    assert!(set(&mut session, json!({"id": "wear", "value": "soft", "cook": false})).is_err());
    assert!(set(&mut session, json!({"id": "wear", "value": f64::NAN, "cook": false})).is_err());

    set(&mut session, json!({"id": "tint", "value": "#ff0000", "cook": false})).unwrap();

    assert_eq!(link(&session, id).params[1].value, json!({"t": "color", "v": [1.0, 0.0, 0.0, 1.0]}));

    set(&mut session, json!({"id": "wear", "value": 0.9, "cook": false})).unwrap();

    assert_eq!(link(&session, id).params[0].value, json!({"t": "float", "v": 0.9}));

    // Cooks follow the texture set resolution
    let job = session.link_job(id, &serde_json::Map::new()).unwrap();

    assert_eq!(job.size, 128);
    assert_eq!(job.values["wear"], json!({"t": "float", "v": 0.9}));

    // Merge the cooked maps into the parameter edit
    let old = map_name(&session, id, Channel::Roughness);
    session.apply_link_cook(id, &cooked(0.75, 0.9), true).unwrap();

    assert!((roughness(&mut session) - 0.75).abs() < 2e-3);
    assert!(!session.doc.resources.contains_key(&old), "old maps are dropped");

    execute(&mut session, "edit.undo", &Value::Null).unwrap();

    assert!((roughness(&mut session) - 0.25).abs() < 2e-3, "undo restores the earlier maps");
    assert_eq!(link(&session, id).params[0].value, json!({"t": "float", "v": 0.5}));

    execute(&mut session, "edit.redo", &Value::Null).unwrap();

    assert!((roughness(&mut session) - 0.75).abs() < 2e-3);

    // Presets skip unsupported inputs and reject unknown names
    assert!(execute(&mut session, "material.link.preset", &json!({"name": "Nope", "cook": false})).is_err());

    execute(&mut session, "material.link.preset", &json!({"name": "Worn", "cook": false})).unwrap();

    assert_eq!(link(&session, id).params[0].value, json!({"t": "float", "v": 1.0}));
}

/// A channel can read another output of the material; the choice survives re-cooks, undo and saving, and links saved before it existed map their channels too
#[test]
fn channels_read_the_output_they_are_mapped_to() {
    let mut s = session();
    let id = install(&mut s);
    let map = |s: &mut Session, output: &str| execute(s, "material.link.map", &json!({"layer": id.0, "channel": "roughness", "output": output}));

    assert!(map(&mut s, "curvature").is_err(), "outputs the material lacks are errors");
    assert!(execute(&mut s, "material.link.map", &json!({"layer": id.0, "output": "roughness"})).is_err());

    // Roughness reads the ambient occlusion output (all white)
    map(&mut s, "ambientOcclusion").unwrap();

    assert!((roughness(&mut s) - 1.0).abs() < 2e-3);
    assert_eq!(link(&s, id).output_of(Channel::Roughness), "ambientOcclusion");

    let saved = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();

    assert_eq!(link(&saved, id), link(&s, id));

    s.apply_link_cook(id, &cooked(0.75, 0.5), false).unwrap();

    assert!((roughness(&mut s) - 1.0).abs() < 2e-3, "a re-cook keeps the mapping");

    map(&mut s, "roughness").unwrap();

    assert!((roughness(&mut s) - 0.75).abs() < 2e-3);
    assert!(link(&s, id).mapping.is_empty(), "a channel's own output is no mapping");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    assert!((roughness(&mut s) - 1.0).abs() < 2e-3);

    // A link saved before mappings (no outputs list) maps from its channels' maps
    let mut old = session();
    let id = install(&mut old);
    if let LayerKind::Fill(fill) = &mut old.doc.project.layer_mut(id).unwrap().kind {
        let link = fill.link.as_mut().unwrap();
        link.outputs.clear();
        let saved = serde_json::to_value(&**link).unwrap();

        assert!(saved.get("outputs").is_none() && saved.get("mapping").is_none() && saved.get("resolution").is_none(), "{saved}");
    }
    execute(&mut old, "material.link.map", &json!({"layer": id.0, "channel": "roughness", "output": "ambientOcclusion"})).unwrap();

    assert!((roughness(&mut old) - 1.0).abs() < 2e-3);
    assert!(link(&old, id).outputs.contains_key("roughness"), "the unmapped output stays listed");

    let kept = old.doc.resources.len();
    execute(&mut old, "file.removeUnused", &Value::Null).unwrap();

    assert_eq!(old.doc.resources.len(), kept, "outputs no channel reads are kept");
}

/// The cook size: relative sizes follow the texture set, a fixed size is stored and cooked at
#[test]
fn materials_cook_at_their_picked_resolution() {
    let mut s = session();
    let id = install(&mut s);
    let size = |s: &Session| s.link_job(id, &serde_json::Map::new()).unwrap().size;
    let pick = |s: &mut Session, mut size: Value| {
        size["layer"] = json!(id.0);
        size["cook"] = json!(false);
        execute(s, "material.link.resolution", &size)
    };

    assert_eq!(size(&s), 128);

    pick(&mut s, json!({"fixed": 1024})).unwrap();

    assert_eq!((link(&s, id).resolution, size(&s)), (Some(CookResolution::Fixed(1024)), 1024));

    pick(&mut s, json!({"relative": 2})).unwrap();

    assert_eq!(size(&s), 512);

    let steps = s.history_labels().len();
    pick(&mut s, json!({"relative": 2})).unwrap();

    assert_eq!(s.history_labels().len(), steps, "the size it already has adds no step");

    for bad in [
        json!({"fixed": 300}),
        json!({"fixed": 64}),
        json!({"fixed": 16384}),
        json!({"relative": 4}),
        json!({"relative": "x"}),
        json!({}),
        json!({"relative": 1, "fixed": 256}),
        json!({"relative": "x", "fixed": 256}),
        json!({"relative": 1, "fixed": null}),
    ] {
        assert!(pick(&mut s, bad.clone()).is_err(), "{bad}");
    }

    assert_eq!(size(&s), 512, "rejected options keep the current resolution");
    assert_eq!(s.history_labels().len(), steps, "rejected options add no undo step");

    pick(&mut s, json!({"relative": 0})).unwrap();

    assert_eq!((link(&s, id).resolution, size(&s)), (None, 128));

    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    assert_eq!(size(&s), 512);

    // A hostile project's size falls back to the texture set's
    if let LayerKind::Fill(fill) = &mut s.doc.project.layer_mut(id).unwrap().kind {
        fill.link.as_mut().unwrap().resolution = Some(CookResolution::Fixed(u32::MAX));
    }
    assert_eq!(size(&s), 128);
}

#[test]
fn duplicates_keep_their_maps_and_mesh_maps_feed_inputs() {
    let mut session = session();
    let id = install(&mut session);
    let duplicate = LayerId(execute(&mut session, "layer.duplicate", &json!({"layer": id.0})).unwrap()["layer"].as_u64().unwrap());
    let shared = map_name(&session, duplicate, Channel::Roughness);
    session.apply_link_cook(id, &cooked(0.75, 0.5), false).unwrap();
    assert!(session.doc.resources.contains_key(&shared), "the duplicate still shows the first cook");

    let maps = [BakedMap::Curvature, BakedMap::Normal, BakedMap::WorldNormal, BakedMap::AmbientOcclusion, BakedMap::BentNormal];
    session.doc.baked.insert(0, BakedMaps { resolution: 4, maps: maps.into_iter().map(|map| (map, Arc::new(vec![0.5f32; 16 * map.components()]))).collect() });
    let job = session.link_job(id, &serde_json::Map::new()).unwrap();
    assert_eq!(job.input_identifiers.iter().map(String::as_str).collect::<Vec<_>>(), maps.map(BakedMap::id));
    assert_eq!(maps.map(BakedMap::id), ["curvature", "normal", "worldNormal", "ambientOcclusion", "bentNormal"]);
}

#[test]
fn projects_save_the_link_and_its_package() {
    let mut s = session();
    let id = install(&mut s);
    let snapshot = s.project_snapshot();
    assert!(snapshot.resources.iter().all(|(name, bytes)| Arc::ptr_eq(bytes, &s.doc.resources[name])));
    let owned = s.project_file();
    let saved = s.to_project_bytes().unwrap();

    assert_eq!(saved, texturecraft_engine::formats::save_project_bytes(&owned).unwrap());
    let back = Session::from_project_bytes(&saved).unwrap();
    assert_eq!(link(&back, id), link(&s, id));
    assert!(back.doc.resources.contains_key(&link(&s, id).package));

    // Layer exports also carry the source package
    let tcmat = s.smart_material_bytes(id, "Bricks").unwrap();
    let mut other = session();
    let copy = other.add_smart_material_bytes(&tcmat).unwrap();
    assert!(other.doc.resources.contains_key(&link(&other, copy).package));
}

/// The whole path through a real MaterialCraft: a package cooks on import, then re-cooks.
#[test]
fn cooks_through_materialcraft_cli_when_available() {
    let Some(cli) = std::env::var_os("MATERIALCRAFT_CLI") else {
        eprintln!("MATERIALCRAFT_CLI is not set: skipping the materialcraft-cli round trip");
        return;
    };

    if run_in_private_process() {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let pkg = dir.join("bricks.mcraft");
    let ok = std::process::Command::new(&cli).args(["new", "pbr"]).arg(&pkg).args(["--size", "64"]).status().unwrap();
    assert!(ok.success());
    let exposed = dir.join("exposed.mcraft");
    let rectangular = dir.join("rectangular.mcraft");
    let resize =
        std::process::Command::new(&cli).arg("run").arg(&pkg).args(["graph.size", r#"{"width":64,"height":32}"#, "-o"]).arg(&rectangular).status().unwrap();
    assert!(resize.success());
    let ok = std::process::Command::new(&cli)
        .arg("run")
        .arg(&rectangular)
        .arg("graph.exposed.add")
        .arg(r#"{"id":"wear","bind":{"node":5,"param":"color"}}"#)
        .arg("-o")
        .arg(&exposed)
        .status()
        .unwrap();
    assert!(ok.success());

    // Feed metallic from the texture set's baked curvature
    let with_input = dir.join("with_input.mcraft");
    let add = std::process::Command::new(&cli)
        .arg("run")
        .arg(&exposed)
        .arg("graph.node.add")
        .arg(r#"{"def":"input_grayscale","params":{"identifier":"curvature"}}"#)
        .arg("-o")
        .arg(&with_input)
        .output()
        .unwrap();
    assert!(add.status.success(), "{}", String::from_utf8_lossy(&add.stderr));
    let new_node: Value = serde_json::from_slice(&add.stdout).unwrap();
    let connect = json!({"from":new_node["id"],"to":8,"toPort":"input"});
    let ok =
        std::process::Command::new(&cli).arg("run").arg(&with_input).arg("graph.connect").arg(connect.to_string()).arg("-o").arg(&exposed).status().unwrap();
    assert!(ok.success());
    let mut s = session();
    s.doc.baked.insert(0, BakedMaps { resolution: 16, maps: [(BakedMap::Curvature, Arc::new(vec![0.35; 256]))].into() });
    let id = s.add_linked_material_bytes_at(&std::fs::read(&exposed).unwrap(), Place::Selection).unwrap();
    assert_eq!(link(&s, id).size, 128, "cooked at the texture set's resolution");
    let image = &s.images[&map_name(&s, id, Channel::Roughness)];
    assert_eq!([image.width, image.height], [128, 128]);
    assert!(link(&s, id).params.iter().any(|p| p.id == "wear"));
    assert!((roughness(&mut s) - 0.5).abs() < 0.01);
    assert!((s.caches[0].composite.value(Channel::Metallic, 64, 64)[0] - 0.35).abs() < 0.01);
    execute(&mut s, "material.link.set", &json!({"layer": id.0, "id": "wear", "value": [0.8,0.8,0.8,1.0]})).unwrap();
    let wear = link(&s, id).params.iter().find(|p| p.id == "wear").unwrap().value["v"][0].as_f64().unwrap();
    assert!((wear - 0.8).abs() < 1e-6);
    assert!((roughness(&mut s) - 0.8).abs() < 0.01);
    let out = dir.join("back.mcraft");
    execute(&mut s, "material.link.saveSource", &json!({"layer": id.0, "path": out.to_str().unwrap()})).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), std::fs::read(&exposed).unwrap());

    // Publish a graph with distinct pixels to verify graph selection
    let multi = dir.join("multi.mcraft");
    let ok = std::process::Command::new(&cli).arg("run").arg(&exposed).arg("graph.duplicate").arg("{}").arg("-o").arg(&multi).status().unwrap();
    assert!(ok.success());
    let edited = dir.join("edited.mcraft");
    let script = dir.join("edit-graph.json");
    std::fs::write(
        &script,
        json!({
            "open": multi,
            "steps": [
                {"id":"graph.open","params":{"graph":1}},
                {"id":"graph.exposed.set","params":{"id":"wear","value":[0.2,0.2,0.2,1.0]}}
            ],
            "save": edited
        })
        .to_string(),
    )
    .unwrap();
    let changed = std::process::Command::new(&cli).arg("batch").arg(&script).output().unwrap();
    assert!(changed.status.success(), "{}", String::from_utf8_lossy(&changed.stderr));
    let info = std::process::Command::new(&cli).arg("run").arg(&edited).args(["material.info", r#"{"graph":1}"#]).output().unwrap();
    assert!(info.status.success(), "{}", String::from_utf8_lossy(&info.stderr));
    let interface: Value = serde_json::from_slice(&info.stdout).unwrap();
    let expected_graph = interface["graph"].as_str().unwrap();
    let portable = dir.join("second.mcmat");
    let ok = std::process::Command::new(&cli).arg("publish").arg(&edited).arg(&portable).args(["--graph", "1"]).status().unwrap();
    assert!(ok.success());
    let renamed = dir.join("second.asset");
    std::fs::rename(portable, &renamed).unwrap();
    let loaded = execute(&mut s, "material.load", &json!({"path":renamed})).unwrap();
    let second = LayerId(loaded["layer"].as_u64().unwrap());
    assert_eq!(link(&s, second).graph, expected_graph);
    let second_map = map_name(&s, second, Channel::Roughness);
    assert!((s.images[&second_map].rgba[0][0] - 0.2).abs() < 0.01);
    s.update();
    assert!((s.caches[0].composite.value(Channel::Metallic, 64, 64)[0] - 0.35).abs() < 0.01);
    let job = s.link_job(second, &serde_json::Map::new()).unwrap();
    assert_eq!(job.source_ext, "mcmat");
    assert_eq!(job.package.as_slice(), std::fs::read(&renamed).unwrap());
    let result = job.run().unwrap();
    assert_eq!(result.interface.graph, expected_graph);
    assert_eq!(result.size, 128);
}

#[test]
fn installing_a_bake_reuses_its_encoded_map_buffers() {
    let material = cooked(0.25, 0.5);
    let prepared = &material.maps[1];
    let mut session = session();

    let id = session.add_cooked_material(&material, Arc::new(Vec::new()), "mcmat", Place::Selection).unwrap();
    let name = map_name(&session, id, Channel::Roughness);

    assert!(Arc::ptr_eq(&session.doc.resources[&name], &prepared.bytes));
    assert!(Arc::ptr_eq(&session.images[&name], &prepared.image));
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires MaterialCraft CLI and at least 8 GiB of available memory"]
fn three_8k_maps_respect_budget_and_survive_reopen() {
    use texturecraft_engine::formats::save_ram_budget;
    use texturecraft_engine::link_cmds::cook_link_file;

    const RESOLUTION: u32 = 8192;
    const FIXTURE_AVAILABLE_BYTES: u64 = 16 << 30;
    if run_in_private_process() {
        return;
    }

    let cli = std::env::var_os("MATERIALCRAFT_CLI").expect("set MATERIALCRAFT_CLI to the compatible CLI");
    let temp = tempfile::tempdir().unwrap();
    let mut authoring = temp.path().join("pbr.mcraft");
    assert!(std::process::Command::new(&cli).args(["new", "pbr"]).arg(&authoring).args(["--size", "32"]).status().unwrap().success());

    // Keep base color, roughness and metallic from the PBR fixture
    for node in [4, 10, 12] {
        let next = temp.path().join(format!("after-{node}.mcraft"));
        let args = json!({"to":node,"toPort":"input"}).to_string();
        let output = std::process::Command::new(&cli).arg("run").arg(&authoring).args(["graph.disconnect", &args, "-o"]).arg(&next).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        authoring = next;
    }

    let path = temp.path().join("three.mcmat");
    assert!(std::process::Command::new(&cli).arg("publish").arg(&authoring).arg(&path).args(["--graph", "0"]).status().unwrap().success());
    let mut session = Session::new_project("Budget", primitives::cube(1.0), "builtin:cube", None, RESOLUTION).unwrap();
    let original = session.to_project_bytes().unwrap();

    let save = texturecraft_engine::formats::RamReservation::for_save().unwrap();
    let blocked = cook_link_file(&path, RESOLUTION, None, LinkMemoryBudget::new(FIXTURE_AVAILABLE_BYTES), Arc::default()).unwrap_err();
    assert!(blocked.to_string().contains("a project save is using the RAM allowance"));
    drop(save);

    let low = LinkMemoryBudget::new(save_ram_budget(FIXTURE_AVAILABLE_BYTES, 1));
    let companion_limited = LinkMemoryBudget::new(save_ram_budget(FIXTURE_AVAILABLE_BYTES, 10));

    let error = cook_link_file(&path, RESOLUTION, None, low, Arc::default()).unwrap_err();

    assert!(error.to_string().contains("configured cap"), "{error}");
    assert_eq!(session.to_project_bytes().unwrap(), original);

    let error = cook_link_file(&path, RESOLUTION, None, companion_limited, Arc::default()).unwrap_err();

    assert!(error.to_string().contains("MaterialCraft cook failed"), "{error}");
    assert!(error.to_string().contains("configured cap"), "{error}");
    assert_eq!(session.to_project_bytes().unwrap(), original);

    let high = LinkMemoryBudget::new(save_ram_budget(FIXTURE_AVAILABLE_BYTES, 50));
    let (source, cook) = cook_link_file(&path, RESOLUTION, None, high, Arc::default()).unwrap();
    let probes: Vec<_> = cook.maps.iter().map(|map| Arc::downgrade(&map.image)).collect();

    assert_eq!(cook.maps.len(), 3);
    assert!(cook.maps.iter().all(|map| [map.image.width, map.image.height] == [RESOLUTION; 2]));

    let layer = session.add_cooked_material(&cook, source.package, source.source_ext, Place::Selection).unwrap();
    let save_blocked = session.to_project_bytes().unwrap_err();
    assert!(save_blocked.to_string().contains("a MaterialCraft cook is using the RAM allowance"));
    drop(cook);
    let encoded = session.to_project_bytes().unwrap();

    assert!(probes.iter().all(|probe| probe.upgrade().is_some()));
    execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert!(probes.iter().all(|probe| probe.upgrade().is_none()));
    drop(session);

    let reopened = Session::from_project_bytes(&encoded).unwrap();

    assert!(reopened.doc.project.layer(layer).is_ok());
    for channel in [Channel::BaseColor, Channel::Roughness, Channel::Metallic] {
        let name = map_name(&reopened, layer, channel);
        let image = &reopened.images[&name];
        assert_eq!([image.width, image.height], [RESOLUTION; 2]);
    }
}

#[test]
fn image_with_material_extension_reopens_as_an_image() {
    let mut session = session();
    let bytes = mesh_map_png(2, 1, &[0.5; 4]).unwrap();
    let name = session.import_resource_bytes("swatch.sbs", bytes).unwrap();

    let reopened = Session::from_project_bytes(&session.to_project_bytes().unwrap()).unwrap();

    assert_eq!(reopened.images[&name].rgba.len(), 4);
}
