//! Acceptance: an agent completes realistic texturing tasks over MCP, checked only through MCP
//! results (plus the exported files on disk), and the server holds its security line.

use base64::Engine as _;
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
use rmcp::service::RunningService;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{Value, json};
use texturecraft_automation::{AuthorizedWorkspace, BridgeClient, Filesystem, Headless, TexturecraftMcp, rpc};

#[derive(Clone, Default)]
struct Client;
impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}

type Conn = RunningService<RoleClient, Client>;
type Server = tokio::task::JoinHandle<()>;

async fn connect(fs: Filesystem) -> (Conn, Server) {
    let (s, c) = tokio::io::duplex(1 << 22);
    let session = texturecraft_engine::Session::new_project("t", texturecraft_engine::geom::primitives::cube(2.0), "builtin:cube", None, 128).unwrap();
    let server = tokio::spawn(async move {
        if let Ok(running) = TexturecraftMcp::headless(Headless::new(session, fs)).serve(s).await {
            let _ = running.waiting().await;
        }
    });
    (Client.serve(c).await.expect("client init"), server)
}

/// Close the client and wait for the server, so its directory handles are released before the
/// test removes the folder (Windows refuses otherwise).
async fn shutdown(c: Conn, server: Server) {
    c.cancel().await.unwrap();
    server.await.unwrap();
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("tc-agent-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn workspace(root: &std::path::Path) -> Filesystem {
    Filesystem::Workspace(AuthorizedWorkspace::new(Some(root), Some(root)).unwrap())
}

fn text(r: &CallToolResult) -> String {
    r.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n")
}

async fn call(c: &Conn, name: &str, args: Value) -> Result<CallToolResult, rmcp::ServiceError> {
    let mut p = CallToolRequestParams::new(name.to_owned());
    if let Value::Object(m) = args {
        p = p.with_arguments(m);
    }
    c.call_tool(p).await
}

/// Call a tool and parse its JSON result, failing the test on a tool error.
async fn tool(c: &Conn, name: &str, args: Value) -> Value {
    let r = call(c, name, args).await.expect("transport");
    assert_ne!(r.is_error, Some(true), "{name} failed: {}", text(&r));
    serde_json::from_str(&text(&r)).unwrap_or(Value::String(text(&r)))
}

/// The tool's error text (the test fails if the tool succeeded).
async fn tool_err(c: &Conn, name: &str, args: Value) -> String {
    match call(c, name, args).await {
        Ok(r) => {
            assert_eq!(r.is_error, Some(true), "{name} should fail: {}", text(&r));
            text(&r)
        }
        Err(e) => e.to_string(),
    }
}

async fn run(c: &Conn, id: &str, params: Value) -> Value {
    tool(c, "command_run", json!({"id": id, "params": params})).await
}

async fn inspect(c: &Conn) -> Value {
    tool(c, "session_inspect", json!({})).await
}

#[tokio::test(flavor = "multi_thread")]
async fn textures_export_accepts_per_map_settings() {
    let dir = tmp("map-options");
    let (client, server) = connect(workspace(&dir)).await;

    let result = tool(
        &client,
        "textures_export",
        json!({"dir": "textures", "preset": "glTF PBR Metal Roughness", "maps": [
            {"map": 0, "format": "pgm", "bitDepth": 16},
            {"map": 1, "enabled": false}
        ]}),
    )
    .await;

    let files = result["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "{result}");
    let graymap = files.iter().filter_map(Value::as_str).find(|path| path.ends_with(".pgm")).unwrap();
    assert!(std::fs::read(dir.join(graymap)).unwrap().starts_with(b"P5\n128 128\n65535\n"));
    shutdown(client, server).await;
    std::fs::remove_dir_all(dir).unwrap();
}

fn top(s: &Value) -> &Value {
    &s["layers"][0]
}

/// Painted tiles of a paint layer's base color.
fn painted(layer: &Value) -> u64 {
    layer["painted"].as_array().into_iter().flatten().filter(|p| p["channel"] == "baseColor").map(|p| p["tiles"].as_u64().unwrap()).sum()
}

/// Sort `painted` lists by channel: their order follows a hash map in the engine.
fn canon(v: &mut Value) {
    match v {
        Value::Object(m) => {
            if let Some(Value::Array(p)) = m.get_mut("painted") {
                p.sort_by(|a, b| a["channel"].as_str().cmp(&b["channel"].as_str()));
            }
            m.values_mut().for_each(canon);
        }
        Value::Array(a) => a.iter_mut().for_each(canon),
        _ => {}
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_completes_texturing_tasks() {
    let dir = tmp("tasks");
    let (c, server) = connect(workspace(&dir)).await;

    // 1. New project on a built-in mesh.
    let s = tool(&c, "project_new", json!({"mesh": "sphere", "resolution": 256, "name": "Agent"})).await;
    assert_eq!(s["project"], "Agent");
    assert!(s["mesh"]["triangles"].as_u64().unwrap() > 1000, "task 1: {}", s["mesh"]);
    assert_eq!(s["textureSets"][0]["resolution"], 256);
    assert_eq!(top(&s)["kind"], "paint");

    // 2. Fill layer with a color and roughness.
    run(&c, "layer.add.fill", json!({"color": [0.2, 0.4, 0.6], "roughness": 0.2})).await;
    let s = inspect(&c).await;
    let fill = &top(&s)["fill"]["sources"];
    assert_eq!(top(&s)["kind"], "fill", "task 2: {}", top(&s));
    assert!((fill["roughness"]["uniform"][0].as_f64().unwrap() - 0.2).abs() < 1e-6, "task 2: {fill}");
    assert!((fill["baseColor"]["uniform"][2].as_f64().unwrap() - 0.6).abs() < 1e-6, "task 2: {fill}");

    // 3. Bake, then a black mask with an edge-wear generator.
    tool(&c, "bake", json!({"resolution": 128})).await;
    run(&c, "mask.add.black", json!({})).await;
    run(&c, "effect.add.generator", json!({"generator": "edgeWear", "level": 0.4})).await;
    let s = inspect(&c).await;
    assert!(!s["textureSets"][0]["baked"]["maps"].as_array().unwrap().is_empty(), "task 3: {}", s["textureSets"]);
    let mask = &top(&s)["mask"];
    assert_eq!(mask["background"], "black", "task 3: {mask}");
    assert_eq!(mask["effects"][0]["kind"], "Generator", "task 3: {mask}");
    assert_eq!(mask["effects"][0]["generator"]["type"], "EdgeWear", "task 3: {mask}");
    assert!((mask["effects"][0]["generator"]["level"].as_f64().unwrap() - 0.4).abs() < 1e-6);

    // 4. Paint a red stroke across the middle of the view on a new paint layer.
    run(&c, "layer.add.paint", json!({"name": "Ink"})).await;
    let r = tool(&c, "paint_stroke", json!({"points": [[600, 400], [680, 400]], "color": [1, 0, 0], "roughness": 0.9, "size": 10})).await;
    assert!(r["stamps"].as_u64().unwrap() > 1, "task 4: {r}");
    let s = inspect(&c).await;
    assert_eq!(top(&s)["name"], "Ink");
    let tiles = painted(top(&s));
    assert!(tiles > 0, "task 4: {}", top(&s));

    // 5. Undo and redo the stroke.
    run(&c, "edit.undo", json!({})).await;
    assert_eq!(painted(top(&inspect(&c).await)), 0, "task 5: undo");
    run(&c, "edit.redo", json!({})).await;
    assert_eq!(painted(top(&inspect(&c).await)), tiles, "task 5: redo");

    // 6. A smart material lands as a folder of layers.
    run(&c, "material.apply", json!({"material": "iron_rusted"})).await;
    let s = inspect(&c).await;
    assert_eq!(top(&s)["kind"], "group", "task 6: {}", top(&s));
    assert_eq!(top(&s)["name"], "Iron Rusted");
    assert!(!top(&s)["children"].as_array().unwrap().is_empty());

    // 7. Export with the glTF preset.
    let r = tool(&c, "textures_export", json!({"dir": "tex", "preset": "glTF PBR Metal Roughness"})).await;
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    assert!(files.iter().any(|f| f.contains("BaseColor")), "task 7: {r}");
    assert!(files.iter().any(|f| f.contains("OcclusionRoughnessMetallic")), "task 7: {r}");
    for f in &files {
        assert!(std::fs::metadata(dir.join(f)).unwrap().len() > 100, "task 7: {f}");
    }

    std::fs::create_dir_all(dir.join("lib")).unwrap();
    // 11. Import an image resource and use it as a fill layer (kept by save/reopen below).
    image::RgbaImage::from_fn(32, 32, |x, y| image::Rgba([(x * 8) as u8, (y * 8) as u8, 90, 255])).save(dir.join("grunge.png")).unwrap();
    let r = tool(&c, "resource_import", json!({"path": "grunge.png"})).await;
    let name = r["name"].as_str().unwrap().to_string();
    run(&c, "layer.add.image", json!({"name": name})).await;
    let s = inspect(&c).await;
    // The selected layer is the material's folder, so the new layer goes inside it.
    let layer = &top(&s)["children"][0];
    assert_eq!(layer["fill"]["sources"]["baseColor"]["image"], name.as_str(), "task 11: {layer}");

    // 12. Save the rust folder as a smart material file and load it back as a new folder.
    let rust = top(&s)["id"].clone();
    let saved = tool(&c, "material_save", json!({"path": "lib/rust", "layer": rust, "name": "My Rust"})).await;
    assert_eq!(saved["path"], "lib/rust.tcmat", "task 12: parent folders must exist: {saved}");
    let loaded = tool(&c, "material_load", json!({"path": "lib/rust.tcmat"})).await["layer"].clone();
    // Added above the selection: inside the selected folder.
    let s = inspect(&c).await;
    let copy = top(&s)["children"].as_array().unwrap().iter().find(|l| l["id"] == loaded).cloned().unwrap_or_default();
    assert_eq!(copy["name"], "My Rust", "task 12: {}", top(&s));
    assert_eq!(copy["kind"], "group", "task 12: {copy}");

    // 8. Save, reopen and compare.
    let before = inspect(&c).await;
    let saved = tool(&c, "project_save", json!({"path": "agent"})).await;
    assert_eq!(saved["path"], "agent.tcraft");
    tool(&c, "project_new", json!({"mesh": "cube", "resolution": 128})).await;
    let after = tool(&c, "project_open", json!({"path": "agent.tcraft"})).await;
    let (mut before, mut after) = (before, after);
    canon(&mut before);
    canon(&mut after);
    for k in ["project", "mesh", "textureSets", "layers"] {
        assert_eq!(before[k], after[k], "task 8: `{k}` differs after reopening");
    }
    assert_eq!(after["path"], "agent.tcraft");
    assert_eq!(after["modified"], false);

    // 9. Render a PNG (skipped without a GPU adapter).
    match call(&c, "viewport_render", json!({"width": 320, "height": 200, "yaw": 30})).await.unwrap() {
        r if r.is_error == Some(true) => {
            assert!(text(&r).contains("cannot render"), "task 9: {}", text(&r));
            eprintln!("skipping render: {}", text(&r));
        }
        r => {
            let img = r.content.iter().find_map(|c| c.as_image()).expect("image content");
            assert_eq!(img.mime_type, "image/png");
            let png = base64::engine::general_purpose::STANDARD.decode(&img.data).unwrap();
            let px = image::load_from_memory(&png).unwrap().into_rgba8();
            assert_eq!(px.dimensions(), (320, 200));
            assert_ne!(px.get_pixel(160, 100), px.get_pixel(2, 2), "task 9: the mesh differs from the background");
        }
    }

    // 10. A batch stops at the failing step and says which.
    let err = tool_err(&c, "command_batch", json!({"steps": [{"id": "layer.add.paint"}, {"id": "no.such.command"}, {"id": "layer.add.paint"}]})).await;
    let v: Value = serde_json::from_str(&err).unwrap();
    assert_eq!(v["completed"], 1, "task 10: {v}");
    assert_eq!(v["failed"]["index"], 1, "task 10: {v}");

    shutdown(c, server).await;
    std::fs::remove_dir_all(&dir).unwrap();
}

const TOOLS: [&str; 22] = [
    "mesh_reimport",
    "mesh_export",
    "bake_mesh_add",
    "resource_import",
    "material_save",
    "material_load",
    "session_inspect",
    "command_list",
    "command_run",
    "command_batch",
    "project_new",
    "project_open",
    "project_save",
    "mesh_import",
    "textures_export",
    "bake",
    "paint_stroke",
    "viewport_render",
    "viewport_screenshot",
    "ui_screenshot",
    "ui_inspect",
    "ui_menu_invoke",
];

#[tokio::test(flavor = "multi_thread")]
async fn paths_outside_the_workspace_are_rejected() {
    let base = tmp("security");
    let (root, outside) = (base.join("root"), base.join("outside"));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let (c, server) = connect(workspace(&root)).await;
    tool(&c, "project_save", json!({"path": "ok.tcraft"})).await;
    std::fs::copy(root.join("ok.tcraft"), outside.join("secret.tcraft")).unwrap();
    let abs = outside.join("secret.tcraft").display().to_string();
    for path in ["../outside/secret.tcraft", abs.as_str(), "C:/secret.tcraft", "/etc/passwd", "a/../../outside/secret.tcraft", "a\\..\\x.tcraft"] {
        tool_err(&c, "project_open", json!({"path": path})).await;
        tool_err(&c, "mesh_import", json!({"path": path})).await;
        tool_err(&c, "resource_import", json!({"path": path})).await;
        tool_err(&c, "material_load", json!({"path": path})).await;
        tool_err(&c, "material_save", json!({"path": path})).await;
        tool_err(&c, "project_save", json!({"path": path})).await;
        tool_err(&c, "mesh_export", json!({"path": path})).await;
        tool_err(&c, "mesh_reimport", json!({"path": path})).await;
        tool_err(&c, "bake_mesh_add", json!({"path": path})).await;
    }
    let e = tool_err(&c, "textures_export", json!({"dir": "../outside/tex"})).await;
    assert!(e.contains("rejected"), "{e}");
    assert!(!outside.join("tex").exists());
    for (id, p) in [
        ("file.open", json!({"path": abs})),
        ("file.saveAs", json!({"path": abs})),
        ("file.save", json!({})),
        ("file.exportTextures", json!({"dir": "x"})),
        ("resource.import", json!({"path": abs})),
        ("material.save", json!({"path": abs})),
    ] {
        let e = tool_err(&c, "command_run", json!({"id": id, "params": p})).await;
        assert!(e.contains("disabled for automation"), "{id}: {e}");
    }
    shutdown(c, server).await;

    // Without roots there is no file access at all.
    let (c, server) = connect(Filesystem::Denied).await;
    assert!(tool_err(&c, "project_save", json!({"path": "x.tcraft"})).await.contains("not granted"));
    assert!(tool_err(&c, "project_open", json!({"path": "x.tcraft"})).await.contains("not granted"));
    shutdown(c, server).await;
    std::fs::remove_dir_all(&base).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn every_tool_rejects_junk_without_panicking() {
    let (c, server) = connect(Filesystem::Denied).await;
    let listed: Vec<String> = c.list_all_tools().await.unwrap().into_iter().map(|t| t.name.to_string()).collect();
    for t in TOOLS {
        assert!(listed.iter().any(|l| l == t), "missing tool {t}");
    }
    let wrong_types = json!({
        "id": 7, "params": "x", "filter": 3, "steps": "x", "mesh": 1, "resolution": "big", "name": [], "path": 5, "dir": false,
        "preset": 1, "ts": "nope", "format": 1, "padding": 2, "size": -1, "maps": "x", "points": "x", "viewport": 1, "color": "nope",
        "roughness": "x", "metallic": "x", "height": "x", "hardness": "x", "view": 5, "width": -5, "yaw": "x", "pitch": "x"
    });
    let hostile = json!({
        "id": "no.such", "params": {"layer": 1e300}, "filter": "\u{0}", "steps": [{"id": "x"}], "mesh": "../../etc", "resolution": 3, "name": "",
        "path": "../../x", "dir": "../x", "preset": "nope", "ts": 99, "format": "zzz", "padding": "?", "size": 3, "maps": ["nope"], "points": [],
        "viewport": [0, 0], "color": "#zzzzzz", "roughness": 1e30, "metallic": -1e30, "height": 1e30, "hardness": 1e30, "view": "4d",
        "width": 99999, "yaw": 1e30, "pitch": -1e30
    });
    for (n, junk) in [Value::Null, wrong_types, hostile].into_iter().enumerate() {
        for t in &listed {
            // No arguments means defaults for these (a real project, a real bake): not junk.
            if n == 0 && ["project_new", "bake", "viewport_render"].contains(&t.as_str()) {
                continue;
            }
            let may_succeed = t == "session_inspect" || (t == "command_list" && n != 1);
            let r = call(&c, t, junk.clone()).await;
            let failed = r.as_ref().map_or(true, |r| r.is_error == Some(true));
            assert!(failed || may_succeed, "{t} accepted junk #{n}: {:?}", r.map(|r| text(&r)));
        }
    }
    // Still serving.
    assert!(inspect(&c).await["layers"].is_array());
    shutdown(c, server).await;
}

/// Bridge mode against a control-protocol server (the headless one speaks the app's protocol).
#[tokio::test(flavor = "multi_thread")]
async fn bridge_mode_forwards_to_a_control_server() {
    let session = texturecraft_engine::Session::new_project("t", texturecraft_engine::geom::primitives::cube(2.0), "builtin:cube", None, 128).unwrap();
    let h = std::sync::Arc::new(std::sync::Mutex::new(Headless::new(session, Filesystem::Denied)));
    let token = "bridge-test-token-0123456789".to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    let t = token.clone();
    std::thread::spawn(move || rpc::serve_tcp(0, h, t, |a| tx.send(a.port()).unwrap()));
    let port = rx.recv().unwrap();

    let (s, c) = tokio::io::duplex(1 << 20);
    let server = tokio::spawn(async move {
        if let Ok(running) = TexturecraftMcp::bridge(BridgeClient::new(port, token)).serve(s).await {
            let _ = running.waiting().await;
        }
    });
    let c = Client.serve(c).await.unwrap();
    run(&c, "layer.add.fill", json!({"roughness": 0.7})).await;
    run(&c, "layer.add.paint", json!({})).await;
    tool(&c, "paint_stroke", json!({"points": [[640, 400]], "color": "#ff0000"})).await;
    let s = inspect(&c).await;
    assert_eq!(s["layers"][1]["kind"], "fill");
    assert!(painted(&s["layers"][0]) > 0, "{}", s["layers"][0]);
    let b = tool(&c, "command_batch", json!({"steps": [{"id": "layer.add.paint"}, {"id": "edit.undo"}]})).await;
    assert_eq!(b["completed"], 2);
    assert!(tool_err(&c, "command_run", json!({"id": "file.open", "params": {"path": "/etc/passwd"}})).await.contains("disabled"));
    assert!(tool_err(&c, "project_open", json!({"path": "x.tcraft"})).await.contains("headless mode only"));
    assert!(tool_err(&c, "ui_inspect", json!({})).await.contains("running app"));
    shutdown(c, server).await;

    let (s, c) = tokio::io::duplex(1 << 20);
    let server = tokio::spawn(async move {
        if let Ok(running) = TexturecraftMcp::bridge(BridgeClient::new(port, "wrong-token-0123456789".into())).serve(s).await {
            let _ = running.waiting().await;
        }
    });
    let c = Client.serve(c).await.unwrap();
    assert!(tool_err(&c, "session_inspect", json!({})).await.contains("authentication failed"));
    shutdown(c, server).await;
}

/// A one-triangle .gltf whose buffer is the file `bin_uri`, and that buffer's bytes.
fn split_gltf(bin_uri: &str) -> (Vec<u8>, Vec<u8>) {
    let bin: Vec<u8> = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect();
    let doc = json!({
        "asset": { "version": "2.0" },
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0 }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0 } }] }],
        "buffers": [{ "uri": bin_uri, "byteLength": bin.len() }],
        "bufferViews": [{ "buffer": 0, "byteLength": bin.len() }],
        "accessors": [{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0] }]
    });
    (serde_json::to_vec(&doc).unwrap(), bin)
}

/// A .gltf's buffer beside it in the workspace is read with it; a file outside the mesh's folder
/// is not, even inside the workspace.
#[tokio::test(flavor = "multi_thread")]
async fn mesh_import_reads_files_next_to_the_mesh() {
    let base = tmp("external");
    std::fs::create_dir_all(base.join("meshes")).unwrap();
    let (gltf, bin) = split_gltf("scene.bin");
    std::fs::write(base.join("meshes/scene.gltf"), gltf).unwrap();
    std::fs::write(base.join("meshes/scene.bin"), &bin).unwrap();
    std::fs::write(base.join("secret.bin"), &bin).unwrap();
    std::fs::write(base.join("meshes/escape.gltf"), split_gltf("../secret.bin").0).unwrap();
    let (c, server) = connect(workspace(&base)).await;
    let s = tool(&c, "mesh_import", json!({ "path": "meshes/scene.gltf", "resolution": 128 })).await;
    assert_eq!(s["mesh"]["triangles"], 1, "{s}");
    let e = tool_err(&c, "mesh_import", json!({ "path": "meshes/escape.gltf" })).await;
    assert!(e.contains("not found"), "{e}");
    shutdown(c, server).await;
    std::fs::remove_dir_all(&base).unwrap();
}

/// The mesh round trip an agent does over MCP: export the mesh (with its textures) into the
/// workspace, bake from it as a high poly and a cage, and reimport the project from it.
#[tokio::test(flavor = "multi_thread")]
async fn mesh_files_export_bake_and_reimport_in_the_workspace() {
    let base = tmp("mesh-files");
    let (c, server) = connect(workspace(&base)).await;
    run(&c, "layer.add.fill", json!({"color": [0.8, 0.2, 0.1], "roughness": 0.4})).await;
    let usda = tool(&c, "mesh_export", json!({"path": "out/cube", "format": "usda"})).await;
    assert_eq!(usda["path"], "out/cube.usda", "{usda}");
    let textures = usda["textures"].as_array().unwrap();
    assert!(!textures.is_empty() && textures.iter().all(|t| base.join(t.as_str().unwrap()).is_file()), "{usda}");
    assert_eq!(tool(&c, "mesh_export", json!({"path": "out/cube.glb"})).await["path"], "out/cube.glb");
    assert!(base.join("out/cube.usda").is_file() && base.join("out/cube.glb").is_file());

    // The loose usda arrives packaged with its textures
    let high = tool(&c, "bake_mesh_add", json!({"path": "out/cube.usda"})).await;
    assert_eq!(high["name"], "cube.usdz", "{high}");
    let cage = tool(&c, "bake_mesh_add", json!({"path": "out/cube.glb", "cage": true})).await;
    assert_eq!(cage["meshes"]["cage"]["name"], "cube.glb", "{cage}");
    assert_eq!(inspect(&c).await["bakeMeshes"]["highPoly"][0]["name"], "cube.usdz");

    // A built-in mesh has no file to re-read; a file does
    assert!(tool_err(&c, "mesh_reimport", json!({})).await.contains("built-in"));
    tool(&c, "mesh_reimport", json!({"path": "out/cube.glb", "preserveStrokes": true})).await;
    assert_eq!(inspect(&c).await["mesh"]["source"], "mesh.glb");
    tool(&c, "mesh_reimport", json!({})).await;
    shutdown(c, server).await;
    std::fs::remove_dir_all(&base).unwrap();
}
