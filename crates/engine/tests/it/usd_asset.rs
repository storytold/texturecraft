//! `file.exportTextures` with `usdAsset`: `<mesh>.usd` next to the textures, the whole mesh with
//! a material per texture set that reads the textures the export wrote.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

/// A scratch folder that is removed when it goes out of scope, so a failing test leaves nothing behind.
struct Scratch(std::path::PathBuf);

impl std::ops::Deref for Scratch {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp(tag: &str) -> Scratch {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("tc_usd_asset_{tag}_{}_{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Scratch(dir)
}

fn ball() -> Session {
    Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap()
}

fn files(r: &Value) -> Vec<String> {
    r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().to_string()).collect()
}

fn export(s: &mut Session, dir: &std::path::Path, params: Value) -> Value {
    let mut params = params;
    params["dir"] = json!(dir.display().to_string());
    execute(s, "file.exportTextures", &params).unwrap()
}

/// What a UsdPreviewSurface input reads
#[derive(Debug, PartialEq, Clone)]
struct Read {
    file: String,
    output: String,
    colour_space: String,
    /// The texture shader flips green (a DirectX normal map)
    flips_green: bool,
}

/// The prim name the exporter gives a mesh material
fn identifier(name: &str) -> String {
    let mut id: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if !id.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        id.insert(0, '_');
    }
    id
}

/// The binary USD file read back: per mesh material, the inputs of its UsdPreviewSurface that read a texture, and the number of texture shaders
fn materials(bytes: &[u8], mesh_materials: &[String]) -> Vec<(BTreeMap<String, Read>, usize)> {
    use craft_usd::scene::Val;
    let layer = craft_usd::scene::read_layer(bytes).unwrap();
    let prim = |path: &str| layer.prims.iter().find(|p| p.path == path).unwrap_or_else(|| panic!("no prim {path}"));
    let first = |prop: Option<&craft_usd::scene::Prop>| match prop.and_then(|p| p.value.as_ref()) {
        Some(Val::Tok(values)) => values.first().cloned().unwrap_or_default(),
        other => panic!("not a token or an asset: {other:?}"),
    };
    mesh_materials
        .iter()
        .map(|name| {
            let id = identifier(name);
            let material = layer.prims.iter().find(|p| p.type_name == "Material" && p.name == id).unwrap_or_else(|| panic!("no material {id}"));
            let pbr = prim(&format!("{}/PBR", material.path));
            let mut inputs = BTreeMap::new();
            for (key, prop) in &pbr.props {
                let (Some(input), Some(source)) = (key.strip_prefix("inputs:"), &prop.connect) else { continue };
                let (shader, output) = source.rsplit_once('/').unwrap().1.split_once(".outputs:").unwrap();
                let texture = prim(&format!("{}/{shader}", material.path));
                let flips_green = match texture.props.get("inputs:scale").and_then(|p| p.value.as_ref()) {
                    Some(Val::Num(scale)) => scale.get(1).is_some_and(|green| *green < 0.0),
                    _ => false,
                };
                let read = Read {
                    file: first(texture.props.get("inputs:file")).trim_start_matches("./").to_string(),
                    output: output.into(),
                    colour_space: first(texture.props.get("inputs:sourceColorSpace")),
                    flips_green,
                };
                inputs.insert(input.to_string(), read);
            }
            let prefix = format!("{}/", material.path);
            (inputs, layer.prims.iter().filter(|p| p.path.starts_with(&prefix) && p.props.contains_key("inputs:file")).count())
        })
        .collect()
}

/// The materials of `<dir>/material_ball.usd`, indexed by mesh material
fn read_back(s: &Session, dir: &std::path::Path) -> Vec<(BTreeMap<String, Read>, usize)> {
    let materials = materials(&std::fs::read(dir.join("material_ball.usd")).unwrap(), &s.mesh.materials);
    assert_eq!(materials.len(), s.set_count());
    materials
}

fn material_of(s: &Session, set: usize) -> usize {
    s.doc.project.texture_sets[set].material
}

#[test]
fn it_writes_the_mesh_with_materials_that_read_the_exported_textures() {
    let mut s = ball();
    assert!(s.set_count() > 1, "the test needs several texture sets");
    let dir = temp("default");
    let r = export(&mut s, &dir, json!({"usdAsset": true}));
    assert!(files(&r).iter().any(|f| f.ends_with("material_ball.usd")), "the asset is listed: {r}");

    let materials = read_back(&s, &dir);
    for set in 0..s.set_count() {
        let name = &texturecraft_formats::export::sanitize_file_stem(&s.doc.project.texture_sets[set].name);
        let inputs = &materials[material_of(&s, set)].0;
        let read = |input: &str| inputs.get(input).unwrap_or_else(|| panic!("{input} of {name}: {inputs:?}")).clone();
        for (input, output, space) in [("diffuseColor", "rgb", "sRGB"), ("roughness", "r", "raw"), ("metallic", "r", "raw"), ("normal", "rgb", "raw")] {
            let read = read(input);
            assert_eq!((read.output.as_str(), read.colour_space.as_str()), (output, space), "{input} of {name}");
            assert!(read.file.starts_with(&format!("material_ball_{name}_")), "{input} reads a file of its own texture set: {}", read.file);
            assert!(dir.join(&read.file).is_file(), "{} was written", read.file);
            assert!(!read.flips_green, "the project's normals are OpenGL");
        }
        assert!(!inputs.contains_key("opacity") && !inputs.contains_key("clearcoat"), "channels the set does not have stay unbound: {inputs:?}");
    }
}

#[test]
fn a_packed_template_reads_different_outputs_of_one_file() {
    let mut s = ball();
    let dir = temp("packed");
    export(&mut s, &dir, json!({"preset": "Unreal Engine (Packed)", "usdAsset": true}));

    for (inputs, shaders) in read_back(&s, &dir) {
        assert_eq!(shaders, 3, "four inputs read three files: {inputs:?}");
        let (roughness, metallic) = (&inputs["roughness"], &inputs["metallic"]);
        assert_eq!(roughness.file, metallic.file, "{inputs:?}");
        assert!(roughness.file.contains("OcclusionRoughnessMetallic"), "{}", roughness.file);
        assert_eq!((roughness.output.as_str(), metallic.output.as_str()), ("g", "b"));
        assert_ne!(inputs["diffuseColor"].file, roughness.file);

        // The template writes DirectX normals
        assert!(inputs["normal"].flips_green, "{inputs:?}");
        assert!(!inputs.contains_key("opacity"), "the base colour file has no alpha without an opacity channel");
    }
}

#[test]
fn the_normal_convention_of_the_files_decides_whether_green_is_flipped() {
    let mut s = ball();
    let (open_gl, direct_x) = (temp("normal-gl"), temp("normal-dx"));
    export(&mut s, &open_gl, json!({"usdAsset": true, "normalFormat": "openGl"}));
    export(&mut s, &direct_x, json!({"usdAsset": true, "normalFormat": "directX"}));
    for (inputs, _) in read_back(&s, &open_gl) {
        assert!(!inputs["normal"].flips_green);
    }
    for (inputs, _) in read_back(&s, &direct_x) {
        assert!(inputs["normal"].flips_green);
    }
}

#[test]
fn a_texture_set_outside_the_export_has_no_textures() {
    let mut s = ball();
    let dir = temp("subset");
    export(&mut s, &dir, json!({"ts": 0, "usdAsset": true}));

    let materials = read_back(&s, &dir);
    assert!(!materials[material_of(&s, 0)].0.is_empty());
    assert!(materials[material_of(&s, 1)].0.is_empty(), "{:?}", materials[material_of(&s, 1)]);
    let name = texturecraft_formats::export::sanitize_file_stem(&s.doc.project.texture_sets[1].name);
    assert!(!std::fs::read_dir(&*dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains(&format!("_{name}_"))), "set 1 was not written");
}

#[test]
fn a_list_gives_each_texture_set_its_own_options() {
    let mut s = ball();
    let dir = temp("list");
    export(&mut s, &dir, json!({"ts": 0}));
    let list = json!([{"ts": 0}, {"ts": 1, "preset": "Unreal Engine (Packed)"}]);
    export(&mut s, &dir, json!({"ts": 1, "preset": "Unreal Engine (Packed)", "usdAsset": list}));

    let materials = read_back(&s, &dir);
    let first = &materials[material_of(&s, 0)].0;
    let second = &materials[material_of(&s, 1)].0;
    assert_eq!(first["roughness"].output, "r", "{first:?}");
    assert_eq!(second["roughness"].output, "g", "{second:?}");
    for read in first.values().chain(second.values()) {
        assert!(dir.join(&read.file).is_file(), "{} was written", read.file);
    }
}

#[test]
fn it_is_not_written_unless_asked_for() {
    let mut s = ball();
    let dir = temp("off");
    let off = export(&mut s, &dir, json!({}));
    let explicit = export(&mut s, &dir, json!({"usdAsset": false}));
    for r in [off, explicit] {
        assert!(!files(&r).is_empty(), "the textures are written");
        assert!(!files(&r).iter().any(|f| f.ends_with(".usd")), "{r}");
    }
    assert!(!dir.join("material_ball.usd").exists());
}

#[test]
fn a_failure_to_write_it_is_a_warning_and_the_textures_are_still_reported() {
    let mut s = ball();
    let dir = temp("blocked");

    // A folder where the file belongs: the asset cannot be written there
    std::fs::create_dir_all(dir.join("material_ball.usd")).unwrap();
    let r = export(&mut s, &dir, json!({"usdAsset": true}));

    assert!(!files(&r).iter().any(|f| f.ends_with(".usd")), "{r}");
    assert!(files(&r).iter().any(|f| f.ends_with(".png")), "the textures are reported: {r}");
    let warnings = r["warnings"].as_array().unwrap();
    assert!(warnings.iter().any(|w| w.as_str().unwrap().starts_with("The USD asset was not written")), "{r}");
    assert!(dir.join("material_ball.usd").is_dir());
}

#[test]
fn the_asset_opens_again_with_its_textures_from_the_folder() {
    let mut s = ball();
    let dir = temp("roundtrip");
    export(&mut s, &dir, json!({"usdAsset": true}));

    // The importer reads only the texture of the base colour, but every input the file binds reads a file of the folder
    for (inputs, _) in read_back(&s, &dir) {
        assert!(inputs.len() >= 4, "{inputs:?}");
        for read in inputs.values() {
            assert!(dir.join(&read.file).is_file(), "{} is in the folder", read.file);
        }
    }

    let file = texturecraft_formats::read_mesh_file(&dir.join("material_ball.usd")).unwrap();
    assert!(file.warnings.is_empty(), "every texture it names is in the folder: {:?}", file.warnings);
    assert!(file.name.ends_with(".usdz"), "the base colour texture is found and packaged: {}", file.name);
    let package = zip::ZipArchive::new(std::io::Cursor::new(&file.bytes)).unwrap();
    assert!(package.file_names().any(|name| name.is_ok_and(|name| name.ends_with(".png"))));

    let back = texturecraft_formats::import_mesh_bytes(&file.name, &file.bytes).unwrap();
    assert_eq!(back.mesh.triangle_count(), s.mesh.triangle_count());
    assert_eq!(back.mesh.materials.len(), s.set_count());
}

#[test]
fn a_bad_usd_asset_value_is_an_error_before_anything_is_written() {
    let mut s = ball();
    for value in [json!("x"), json!([]), json!([1]), json!([{"ts": 0}, "x"]), json!([{"ts": 99}]), json!({"ts": 0})] {
        let dir = temp("bad");
        let r = execute(&mut s, "file.exportTextures", &json!({"dir": dir.display().to_string(), "usdAsset": value}));
        assert!(r.is_err(), "{value}");
        assert!(!dir.exists(), "nothing was written for {value}");
    }
}

#[test]
fn the_tiles_of_a_uv_tiled_set_are_read_through_one_udim_name() {
    let mut s = Session::new_project("t", primitives::udim_plane(), "builtin:udim_plane", None, 128).unwrap();
    let dir = temp("udim");
    export(&mut s, &dir, json!({"usdAsset": true}));

    let materials = materials(&std::fs::read(dir.join("udim_plane.usd")).unwrap(), &s.mesh.materials);
    let colour = &materials[0].0["diffuseColor"];
    assert!(colour.file.ends_with(".<UDIM>.png"), "{}", colour.file);
    for tile in ["1001", "1002"] {
        assert!(dir.join(colour.file.replace("<UDIM>", tile)).is_file(), "tile {tile} was written");
    }
}

/// A template with one map, the base colour, whose name does not tell texture sets apart
fn shared_name_template() -> Value {
    let rgb: Vec<Value> = (0..3)
        .map(|k| {
            let dest = ["R", "G", "B"][k];
            json!([dest, {"Channel": {"channel": "BaseColor", "component": k}}])
        })
        .collect();
    json!({"name": "shared", "maps": [{"file": "colour", "format": "Png", "bit_depth": 8, "channels": rgb}]})
}

#[test]
fn a_file_name_an_earlier_export_of_the_job_has_is_not_read_by_a_later_one() {
    let mut s = ball();
    let dir = temp("same-name");
    let list = json!([{"ts": 0, "preset": shared_name_template()}, {"ts": 1, "preset": shared_name_template()}]);
    export(&mut s, &dir, json!({"ts": 1, "preset": shared_name_template(), "usdAsset": list}));

    let materials = read_back(&s, &dir);
    assert_eq!(materials[material_of(&s, 0)].0["diffuseColor"].file, "colour.png");
    assert!(materials[material_of(&s, 1)].0.is_empty(), "the later set's file would have replaced the earlier one: {:?}", materials[material_of(&s, 1)]);
}

#[test]
fn a_material_without_a_texture_set_has_no_textures() {
    let mut s = ball();

    // Both texture sets paint the first material (which the first set stands for): the second has none
    s.doc.project.texture_sets[1].material = 0;
    let dir = temp("no-set");
    export(&mut s, &dir, json!({"usdAsset": true}));

    let materials = read_back(&s, &dir);
    assert!(!materials[0].0.is_empty());
    assert!(materials[1].0.is_empty(), "{:?}", materials[1]);
}

/// Two two-tile planes, a material each: the first covers tiles 1001 and 1002, the second starts `shift_tiles` tiles to the right
fn two_udim_sets(shift_tiles: f32) -> Session {
    let mut mesh = primitives::udim_plane();
    let mut second = primitives::udim_plane();
    second.materials[0] = "Other".into();
    for uv in &mut second.uvs {
        uv.x += shift_tiles;
    }
    mesh.append(&second);
    Session::new_project("t", mesh, "builtin:udim_plane", None, 128).unwrap()
}

/// A material per half of a two-tile plane: the first material covers tiles 1001 and 1002, the second 1002 and 1003
fn overlapping_tiles() -> Session {
    let s = two_udim_sets(1.0);
    assert_eq!(s.doc.project.texture_sets[0].uv_tiles, [1001, 1002]);
    assert_eq!(s.doc.project.texture_sets[1].uv_tiles, [1002, 1003]);
    s
}

/// The shared colour map plus a roughness map whose names tell the texture sets apart
fn colour_and_own_roughness_template() -> Value {
    let gray = json!([["L", {"Channel": {"channel": "Roughness", "component": 0}}]]);
    let mut preset = shared_name_template();
    preset["maps"].as_array_mut().unwrap().push(json!({"file": "rough_$textureSet", "format": "Png", "bit_depth": 8, "channels": gray}));
    preset
}

#[test]
fn a_udim_map_that_lost_a_tile_to_a_name_clash_is_not_bound_but_the_other_maps_are() {
    let mut s = overlapping_tiles();
    let dir = temp("udim-clash");

    // The colour map's names do not tell the sets apart, so tile 1002 of the second set is not written;
    // the roughness map's names do.
    export(&mut s, &dir, json!({"ts": [0, 1], "preset": colour_and_own_roughness_template(), "usdAsset": true}));

    let materials = materials(&std::fs::read(dir.join("udim_plane.usd")).unwrap(), &s.mesh.materials);
    let (first, second) = (&materials[material_of(&s, 0)].0, &materials[material_of(&s, 1)].0);
    assert_eq!(first["diffuseColor"].file, "colour.<UDIM>.png", "{first:?}");
    assert!(first["roughness"].file.starts_with("rough_") && first["roughness"].file.contains("<UDIM>"), "{first:?}");
    assert!(!second.contains_key("diffuseColor"), "tile 1002 would read the first set's file: {second:?}");
    assert!(second["roughness"].file.starts_with("rough_") && second["roughness"].file.contains("<UDIM>"), "{second:?}");
    assert_ne!(first["roughness"].file, second["roughness"].file);
}

#[test]
fn a_udim_map_whose_tiles_the_window_reserved_for_an_earlier_command_is_not_bound() {
    let mut session = two_udim_sets(2.0);
    assert_eq!(session.doc.project.texture_sets[1].uv_tiles, [1003, 1004]);
    let dir = temp("reserved");
    let first = json!({"ts": 0, "preset": colour_and_own_roughness_template()});
    let written = export(&mut session, &dir, first.clone());

    // The window reserves the earlier files and the UDIM patterns of earlier MaterialX documents
    let mut reserved: Vec<Value> = files(&written).iter().map(|file| json!(std::path::Path::new(file).file_name().unwrap().to_str().unwrap())).collect();
    reserved.push(json!("colour.<UDIM>.png"));
    let second = json!({"ts": 1, "preset": colour_and_own_roughness_template(), "reservedNames": reserved});
    let mut last = second.clone();
    last["usdAsset"] = json!([first, second]);
    export(&mut session, &dir, last);

    let materials = materials(&std::fs::read(dir.join("udim_plane.usd")).unwrap(), &session.mesh.materials);
    let (first, second) = (&materials[material_of(&session, 0)].0, &materials[material_of(&session, 1)].0);
    assert_eq!(first["diffuseColor"].file, "colour.<UDIM>.png", "{first:?}");
    assert!(!dir.join("colour.1003.png").exists(), "the texture export skipped the reserved pattern");
    assert!(!second.contains_key("diffuseColor"), "the second set's colour tiles were not written: {second:?}");
    assert!(second["roughness"].file.starts_with("rough_") && second["roughness"].file.contains("<UDIM>"), "{second:?}");
    assert!(dir.join(second["roughness"].file.replace("<UDIM>", "1003")).is_file());
}

#[test]
fn a_mesh_without_materials_gets_a_material_bound_to_the_default_texture_set() {
    let mut mesh = primitives::plane(2.0, 4);
    mesh.materials.clear();
    mesh.submeshes.clear();
    let mut s = Session::new_project("t", mesh, "builtin:plane", None, 128).unwrap();
    assert_eq!(s.set_count(), 1);
    let dir = temp("no-materials");
    export(&mut s, &dir, json!({"usdAsset": true}));

    let materials = materials(&std::fs::read(dir.join("plane.usd")).unwrap(), &["default".to_string()]);
    let inputs = &materials[0].0;
    assert!(["diffuseColor", "roughness", "metallic", "normal"].iter().all(|input| inputs.contains_key(*input)), "{inputs:?}");
    for read in inputs.values() {
        assert!(read.file.starts_with("plane_Default_"), "reads the Default texture set's files: {}", read.file);
        assert!(dir.join(&read.file).is_file(), "{} was written", read.file);
    }
}
