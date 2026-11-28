//! `file.exportTextures` writes jng, pgm and psd, dithers 8-bit files on request and pads with
//! diffusion (texturecraft #672).

use serde_json::json;
use texturecraft_engine::execute;

#[test]
fn the_new_file_types_dithering_and_diffusion_export() {
    let mut s = texturecraft_engine::sample_session(128, 64).unwrap();
    let dir = std::env::temp_dir().join(format!("tc_export_file_types_{}", std::process::id()));
    for (format, depth) in [("jng", 8), ("pgm", 16), ("psd", 16), ("tiff", 32)] {
        let r = execute(
            &mut s,
            "file.exportTextures",
            &json!({"dir": dir.display().to_string(), "ts": 0, "format": format, "bitDepth": depth, "padding": "diffusion", "dilation": 4}),
        )
        .unwrap();
        let files = r["files"].as_array().unwrap();
        let ext = if format == "tiff" { "tif" } else { format };
        assert!(!files.is_empty() && files.iter().all(|f| f.as_str().unwrap().ends_with(&format!(".{ext}"))), "{format}: {files:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);

    let plan = execute(&mut s, "export.plan", &json!({"ts": 0, "format": "png", "bitDepth": 8, "dithering": true})).unwrap();
    assert!(plan.as_array().unwrap().iter().all(|f| f["dithering"] == true), "{plan}");
    let plan = execute(&mut s, "export.plan", &json!({"ts": 0, "format": "exr", "dithering": true})).unwrap();
    assert!(plan.as_array().unwrap().iter().all(|f| f["dithering"] == false), "float files are never dithered");
    assert!(execute(&mut s, "export.plan", &json!({"padding": "smudge"})).is_err());
}

#[test]
fn in_memory_psd_export_only_renders_the_first_plan_for_each_name() {
    const SMALL_SIDE: u32 = 128;
    let mesh = texturecraft_engine::geom::primitives::udim_plane();
    let mut session = texturecraft_engine::Session::new_project("test", mesh, "builtin:udim_plane", None, SMALL_SIDE).unwrap();
    let preset =
        json!({"name": "Roughness", "maps": [{"file": "rough", "format": "Psd", "bit_depth": 8, "channels": [["L", {"Channel": {"channel": "Roughness"}}]]}]});
    let options = |size| texturecraft_engine::file_cmds::export_options(&session, &json!({"preset": preset, "size": size, "padding": "none"}), "test").unwrap();
    let small = options(SMALL_SIDE);
    let large = options(4096);
    let first = session.export_files(&[0], &small).unwrap();

    assert!(first.len() > 1, "the mesh supplies multiple PSD tiles");

    let jobs = std::iter::once((0, &small)).chain(std::iter::repeat_n((0, &large), 4));
    let batch = session.export_file_jobs(jobs).unwrap();

    assert_eq!(batch.len(), first.len());
    assert!(batch.iter().zip(&first).all(|(kept, original)| kept.name == original.name && kept.bytes == original.bytes));
    assert!(batch.iter().zip(&first).all(|(kept, original)| kept.warnings.len() > original.warnings.len()));
    println!("Kept {} PSDs totaling {} encoded bytes", batch.len(), batch.iter().map(|file| file.bytes.len()).sum::<usize>());
}

#[test]
fn a_psd_file_and_another_sets_folder_conflict_during_planning() {
    use texturecraft_engine::core::Channel;
    use texturecraft_engine::file_cmds::export_options;

    let mut session = texturecraft_engine::sample_session(128, 32).unwrap();
    let sets = &mut session.doc.project.texture_sets;
    sets[0].name = "Body".into();
    sets[0].channels.retain(|(channel, _)| *channel != Channel::Metallic);
    sets[1].name = "Trim".into();
    let preset = json!({"name": "Conflicting outputs", "maps": [
        {"file": "roughness", "format": "Psd", "bit_depth": 8, "channels": [["L", {"Channel": {"channel": "Roughness"}}]]},
        {"file": "$mesh_Body.psd/detail", "format": "Png", "bit_depth": 8, "channels": [["L", {"Channel": {"channel": "Metallic"}}]]}
    ]});
    let dir = tempfile::tempdir().unwrap();
    let mut params = json!({"preset": preset, "padding": "none", "dir": dir.path()});

    params["ts"] = json!(0);
    let body = execute(&mut session, "export.plan", &params).unwrap();
    params["ts"] = json!(1);
    let trim = execute(&mut session, "export.plan", &params).unwrap();
    let file = body[0]["file"].as_str().unwrap();
    let nested_file = trim.as_array().unwrap().iter().find_map(|file| file["file"].as_str().filter(|name| name.contains('/'))).unwrap();
    assert_eq!(nested_file.split('/').next(), Some(file), "separate plans collide when combined");

    for order in [[0, 1], [1, 0]] {
        params["ts"] = json!(order);
        let plan_error = execute(&mut session, "export.plan", &params).unwrap_err();
        let options = export_options(&session, &params, "export.plan").unwrap();
        let render_error = session.export_files(&order, &options).unwrap_err();
        let write_error = execute(&mut session, "file.exportTextures", &params).unwrap_err();

        for error in [plan_error, render_error, write_error] {
            assert!(error.to_string().contains("folder"), "{error}");
        }
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }
}

#[test]
fn a_map_size_wins_over_the_export_size_in_the_plan() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();

    let plan = execute(&mut session, "export.plan", &json!({"ts": 0, "size": 512, "maps": [{"map": 0, "size": 128}]})).unwrap();

    let (overridden, inherited): (Vec<_>, Vec<_>) = plan.as_array().unwrap().iter().partition(|file| file["map"] == 0);

    assert!(!overridden.is_empty() && !inherited.is_empty(), "{plan}");
    assert!(overridden.iter().all(|file| file["size"] == 128), "{plan}");
    assert!(inherited.iter().all(|file| file["size"] == 512), "{plan}");
    assert!(execute(&mut session, "export.plan", &json!({"ts": 0, "maps": [{"map": 0, "size": 100}]})).is_err());
}

#[test]
fn shader_parameters_write_the_shaders_json_next_to_the_textures() {
    let mut session = texturecraft_engine::sample_session(128, 32).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(texturecraft_engine::shader_parameters::shader_parameters_file(&session.doc.project.name));

    let result = execute(&mut session, "file.exportTextures", &json!({"dir": dir.path().display().to_string(), "ts": 1, "shaderParameters": [0, 1]})).unwrap();

    assert!(result["files"].as_array().unwrap().iter().any(|f| f.as_str() == Some(file.to_str().unwrap())), "{result}");
    let written: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    let names: Vec<&str> = session.doc.project.texture_sets.iter().take(2).map(|set| set.name.as_str()).collect();

    assert_eq!(written["shaders"][0]["textureSets"], json!(names));
    assert!(execute(&mut session, "file.exportTextures", &json!({"dir": dir.path().display().to_string(), "shaderParameters": "yes"})).is_err());
}

#[test]
fn a_shaders_json_that_cannot_be_written_is_a_warning_after_the_textures() {
    let mut session = texturecraft_engine::sample_session(128, 32).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(texturecraft_engine::shader_parameters::shader_parameters_file(&session.doc.project.name));
    std::fs::create_dir_all(&file).unwrap();

    let result = execute(&mut session, "file.exportTextures", &json!({"dir": dir.path().display().to_string(), "ts": 0, "shaderParameters": true})).unwrap();

    let files = result["files"].as_array().unwrap();

    assert!(!files.is_empty() && files.iter().all(|f| f.as_str() != Some(file.to_str().unwrap())), "{result}");
    let warnings = result["warnings"].as_array().unwrap();

    assert!(warnings.iter().any(|w| w.as_str().is_some_and(|w| w.starts_with("The shader settings file was not written"))), "{result}");
}

#[test]
fn a_shaders_json_named_like_an_earlier_commands_folder_is_a_warning() {
    let mut session = texturecraft_engine::sample_session(128, 32).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let name = texturecraft_engine::shader_parameters::shader_parameters_file(&session.doc.project.name);
    let reserved = format!("{}/Color.png", name.to_uppercase());

    let result = execute(
        &mut session,
        "file.exportTextures",
        &json!({"dir": dir.path().display().to_string(), "ts": 0, "shaderParameters": true, "reservedNames": [reserved]}),
    )
    .unwrap();

    assert!(!dir.path().join(&name).exists(), "{result}");
    let warnings = result["warnings"].as_array().unwrap();

    assert!(warnings.iter().any(|w| w.as_str().is_some_and(|w| w.contains("already used by a texture file or folder"))), "{result}");
}

#[test]
fn a_psd_warning_outlives_a_reserved_file_that_carried_it() {
    let mut session = texturecraft_engine::sample_session(128, 32).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let roughness = json!([["L", {"Channel": {"channel": "Roughness"}}]]);
    let colour = json!([["R", {"Channel": {"channel": "BaseColor"}}], ["G", {"Channel": {"channel": "BaseColor", "component": 1}}], ["B", {"Channel": {"channel": "BaseColor", "component": 2}}], ["A", {"Constant": 0.5}]]);
    let mut preset = json!({"name": "Two and a PSD", "maps": [
        {"file": "first", "format": "Png", "bit_depth": 8, "channels": roughness},
        {"file": "second", "format": "Png", "bit_depth": 8, "channels": roughness},
    ]});

    // Four 16-bit RGBA layers at 8192 pixels exceed the 2 GiB PSD limit
    for layer in 0..4 {
        preset["maps"].as_array_mut().unwrap().push(json!({"file": format!("layer{layer}"), "format": "Psd", "bit_depth": 16, "channels": colour}));
    }
    let sizes: Vec<_> = (2..6).map(|map| json!({"map": map, "size": 8192})).collect();
    let params = |reserved: Vec<String>| json!({"dir": dir.path().display().to_string(), "ts": 0, "preset": preset, "maps": sizes, "reservedNames": reserved});
    let plan = execute(&mut session, "export.plan", &params(Vec::new())).unwrap();
    let first = plan[0]["file"].as_str().unwrap().to_owned();
    let second = plan[1]["file"].as_str().unwrap().to_owned();
    let psd_warning =
        |warnings: &serde_json::Value| warnings.as_array().unwrap().iter().any(|w| w.as_str().is_some_and(|w| w.contains(".psd\" was not written")));
    assert!(plan.as_array().unwrap().len() == 2 && psd_warning(&plan[0]["warnings"]), "{plan}");

    let result = execute(&mut session, "file.exportTextures", &params(vec![first.clone()])).unwrap();

    assert!(psd_warning(&result["warnings"]), "{result}");
    assert!(dir.path().join(&second).exists() && !dir.path().join(&first).exists(), "{result}");
}
