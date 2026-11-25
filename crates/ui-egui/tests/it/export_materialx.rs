//! Export textures › SETTINGS: the Export MaterialX option (texturecraft #760). It passes
//! `materialX: true` to `file.exportTextures`, which writes an OpenPBR material per texture set.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::ConsoleLine;

const OPENPBR: &str = "OpenPBR (separate maps)";

/// The Export textures window writing into a scratch folder of its own for `name`
fn harness(name: &str) -> (Harness<'static, TextureCraftApp>, std::path::PathBuf) {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut app_harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    app_harness.state_mut().ui.prefs.language = "en".into();
    app_harness.run_steps(4);

    let dir = std::env::temp_dir().join(format!("tc-export-materialx-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    app_harness.state_mut().ui.export.dir = dir.display().to_string();
    app_harness.state_mut().ui.windows.export = true;
    app_harness.run_steps(4);

    (app_harness, dir)
}

fn checkbox<'a>(app_harness: &'a Harness<'static, TextureCraftApp>) -> egui_kittest::Node<'a> {
    app_harness.get_by_role_and_label(egui::Role::CheckBox, "Export MaterialX")
}

/// Click Export and run frames until the export ends
fn export(app_harness: &mut Harness<'static, TextureCraftApp>) {
    app_harness.get_all_by_label("Export").last().unwrap().click();
    for _ in 0..64 {
        app_harness.run_steps(1);
        if app_harness.state().ui.export.job.is_none() {
            break;
        }
    }
    app_harness.run_steps(2);
}

fn mtlx_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "mtlx")).collect();
    files.sort();
    files
}

#[test]
fn export_materialx_sits_directly_below_the_usd_option() {
    let (app_harness, _dir) = harness("order");
    let usd = app_harness.get_by_label("Export USD asset").rect().min.y;
    let materialx = checkbox(&app_harness).rect().min.y;
    let shaders = app_harness.get_by_label("Export shader settings").rect().min.y;

    assert!(usd < materialx && materialx < shaders, "USD {usd}, MaterialX {materialx}, shaders {shaders}");
}

#[test]
fn the_window_offers_export_materialx_off_by_default() {
    let (app_harness, _dir) = harness("offer");

    assert!(!checkbox(&app_harness).accesskit_node().is_disabled());
    assert!(!app_harness.state().ui.export.materialx);
}

#[test]
fn ticking_export_materialx_writes_one_mtlx_per_texture_set() {
    let (mut app_harness, dir) = harness("on");
    app_harness.state_mut().ui.export.preset = OPENPBR.into();
    app_harness.run_steps(3);

    checkbox(&app_harness).click();
    app_harness.run_steps(3);
    assert!(app_harness.state().ui.export.materialx);

    export(&mut app_harness);

    let files = mtlx_files(&dir);
    assert_eq!(files.len(), app_harness.state().session.set_count(), "one material per texture set: {files:?}");
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        assert!(text.contains("<open_pbr_surface"), "{text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ticking_export_materialx_with_any_template_writes_the_openpbr_maps_beside_its_files() {
    let (mut app_harness, dir) = harness("any-template");
    assert_eq!(app_harness.state().ui.export.preset, texturecraft_engine::formats::DEFAULT_PRESET);

    checkbox(&app_harness).click();
    app_harness.run_steps(3);
    export(&mut app_harness);

    let files = mtlx_files(&dir);
    assert_eq!(files.len(), app_harness.state().session.set_count(), "one material per texture set: {files:?}");
    let mut references = 0;
    for file in &files {
        for reference in std::fs::read_to_string(file).unwrap().split("name=\"file\" type=\"filename\" value=\"").skip(1) {
            let name = reference.split('"').next().unwrap();
            assert!(dir.join(name).is_file(), "{name} is not next to the material");
            references += 1;
        }
    }
    assert!(references > 0);

    let written: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert!(written.iter().any(|f| f.ends_with("_baseColor.png")), "the template's own maps are still written: {written:?}");
    assert!(written.iter().any(|f| f.ends_with("_base_color.png")), "{written:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn materialx_uses_the_selected_sets_effective_file_type_and_size() {
    use texturecraft_ui_egui::state::{FileType, SetExport};

    let (mut harness, directory) = harness("set-overrides");
    harness.state_mut().ui.export.skip = vec![0];
    harness
        .state_mut()
        .ui
        .export
        .sets
        .insert(1, SetExport { file_type: Some(FileType { format: "tiff".into(), bit_depth: 16, dithering: false }), size: Some(128), ..Default::default() });
    harness.run_steps(3);
    checkbox(&harness).click();
    harness.run_steps(3);

    export(&mut harness);

    let materials = mtlx_files(&directory);
    assert_eq!(materials.len(), 1);
    let selected_set = &harness.state().session.doc.project.texture_sets[1];
    let expected_stem = texturecraft_engine::formats::export::sanitize_file_stem(&format!("material_ball_{}", selected_set.name));
    assert_eq!(materials[0].file_stem().unwrap(), expected_stem.as_str());

    let document = std::fs::read_to_string(&materials[0]).unwrap();
    let reference = document
        .split("name=\"file\" type=\"filename\" value=\"")
        .skip(1)
        .filter_map(|input| input.split('"').next())
        .find(|filename| filename.ends_with("_base_color.tif"))
        .unwrap();
    let image = image::open(directory.join(reference)).unwrap();
    assert_eq!((image.width(), image.height()), (128, 128));
    assert_eq!(image.color(), image::ColorType::Rgb16);
    let _ = std::fs::remove_dir_all(&directory);
}

/// The option is off by default, so a plain export writes no `.mtlx` even with the OpenPBR template
#[test]
fn no_mtlx_is_written_when_the_option_is_off() {
    let (mut app_harness, dir) = harness("off");
    app_harness.state_mut().ui.export.preset = OPENPBR.into();
    app_harness.run_steps(3);

    export(&mut app_harness);

    assert!(std::fs::read_dir(&dir).unwrap().next().is_some(), "the textures were written");
    assert!(mtlx_files(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn restoring_the_default_parameters_unticks_export_materialx() {
    let (mut app_harness, _dir) = harness("reset");
    checkbox(&app_harness).click();
    app_harness.run_steps(3);
    assert!(app_harness.state().ui.export.materialx);

    app_harness.get_by_label("Restore the default export parameters").click();
    app_harness.run_steps(3);
    assert!(!app_harness.state().ui.export.materialx);
}

/// Every texture set's export raises this warning: the material's normal map is read as OpenGL
const DIRECTX_WARNING: &str = "MaterialX: the normalmap node reads OpenGL (+Y) normals";

#[test]
fn a_warning_every_texture_set_raises_shows_once_in_the_console_and_the_log() {
    let (mut app_harness, dir) = harness("warning");
    assert!(app_harness.state().session.set_count() >= 2, "the warning has to come from more than one texture set");
    let e = &mut app_harness.state_mut().ui.export;
    e.preset = OPENPBR.into();
    e.normal_format = "directX".into();
    app_harness.run_steps(3);

    checkbox(&app_harness).click();
    app_harness.run_steps(3);
    export(&mut app_harness);

    let lines: Vec<&str> = app_harness
        .state()
        .ui
        .export
        .console
        .iter()
        .filter_map(|line| match line {
            ConsoleLine::Step(text) if text.starts_with("Warning: ") => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(lines.iter().filter(|line| line.contains(DIRECTX_WARNING)).count(), 1, "{lines:?}");
    assert_eq!(app_harness.state().session.log.iter().filter(|line| line.contains(DIRECTX_WARNING)).count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_export_with_no_enabled_maps_reports_that_no_files_were_written() {
    use texturecraft_ui_egui::state::{MapExport, SetExport};

    let (mut app_harness, directory) = harness("empty");
    let map_count = app_harness.state().session.export_preset(OPENPBR).unwrap().maps.len();
    let maps = (0..map_count).map(|index| (index, MapExport { enabled: false, ..Default::default() })).collect();
    let skipped = (1..app_harness.state().session.set_count()).collect();
    let settings = &mut app_harness.state_mut().ui.export;
    settings.preset = OPENPBR.into();
    settings.skip = skipped;
    settings.sets.insert(0, SetExport { maps_template: OPENPBR.into(), maps, ..Default::default() });
    settings.materialx = true;
    app_harness.run_steps(3);

    export(&mut app_harness);

    assert!(std::fs::read_dir(&directory).unwrap().next().is_none());
    let console = &app_harness.state().ui.export.console;
    assert!(console.iter().any(|line| matches!(line, ConsoleLine::Step(text) if text.contains("has no OpenPBR maps"))), "{console:?}");
    assert!(matches!(console.last(), Some(ConsoleLine::Step(text)) if text == "No files were written"), "{console:?}");
    let _ = std::fs::remove_dir(&directory);
}

/// A template with one map, the base colour, named `$mesh_$textureSet` (no input suffix)
fn base_colour_template() -> texturecraft_engine::formats::ExportPreset {
    use texturecraft_engine::formats::{Dest, ExportPreset, FileFormat, MapSpec, SourceRef};

    let source = |component| SourceRef::Channel { channel: texturecraft_engine::core::Channel::BaseColor, component };
    ExportPreset {
        name: "base colour only".into(),
        maps: vec![MapSpec {
            file: "$mesh_$textureSet".into(),
            format: FileFormat::Png,
            bit_depth: 8,
            channels: vec![(Dest::R, source(0)), (Dest::G, source(1)), (Dest::B, source(2))],
            inverted: Vec::new(),
        }],
    }
}

/// The base colour of texture set `ts` as the template writes it, from a plain export
fn expected(app_harness: &mut Harness<'static, TextureCraftApp>, ts: usize, preset: &str, file: &str, tag: &str) -> Vec<u8> {
    let dir = std::env::temp_dir().join(format!("tc-export-materialx-expected-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let params = serde_json::json!({"dir": dir.display().to_string(), "preset": preset, "ts": ts});

    texturecraft_engine::execute(&mut app_harness.state_mut().session, "file.exportTextures", &params).unwrap();
    let bytes = std::fs::read(dir.join(file)).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    bytes
}

#[test]
fn texture_sets_of_one_export_never_overwrite_each_others_files() {
    const SHARED: &str = "material_ball_a_base_color.png";

    let cases = [
        // Set 0's own file has the name set 1's OpenPBR base colour would get
        ("a_base_color", "a", "base colour only", "OpenPBR (separate maps)", None),
        // Set 0's OpenPBR base colour has the name set 1's own file would get
        ("a", "a_base_color", "OpenPBR (separate maps)", "base colour only", Some("material_ball_a.mtlx")),
    ];
    for (index, (first, second, template_of_set_0, template_of_set_1, referred_by)) in cases.into_iter().enumerate() {
        let (mut app_harness, dir) = harness(&format!("overwrite-{index}"));
        let project = &mut app_harness.state_mut().session.doc.project;
        project.texture_sets[0].name = first.into();
        project.texture_sets[1].name = second.into();
        let template = base_colour_template();
        let name = template.name.clone();
        app_harness.state_mut().session.export_presets.list.push(template);
        app_harness.state_mut().ui.export.preset = name;
        app_harness.run_steps(3);
        checkbox(&app_harness).click();
        app_harness.run_steps(3);

        export(&mut app_harness);

        let want = expected(&mut app_harness, 0, template_of_set_0, SHARED, &index.to_string());
        let rival = expected(&mut app_harness, 1, template_of_set_1, SHARED, &format!("{index}-rival"));
        assert_ne!(want, rival, "case {index}: the two files differ, so a replacement shows");
        assert_eq!(std::fs::read(dir.join(SHARED)).unwrap(), want, "case {index}: the earlier texture set's file was replaced");

        let console =
            app_harness.state().ui.export.console.iter().filter_map(|line| if let ConsoleLine::Step(text) = line { Some(text.as_str()) } else { None });
        assert!(
            console.clone().any(|text| text.starts_with("Warning: ") && text.contains("\"material_ball_a_base_color.png\" was not written")),
            "case {index}: {:?}",
            console.collect::<Vec<_>>()
        );

        // A material only points at files that exist, and only the earlier set's material at SHARED
        let materials = mtlx_files(&dir);
        assert_eq!(materials.len(), 2, "{materials:?}");
        for material in materials {
            let text = std::fs::read_to_string(&material).unwrap();
            let file_name = material.file_name().unwrap().to_string_lossy().into_owned();
            for reference in text.split("name=\"file\" type=\"filename\" value=\"").skip(1) {
                let name = reference.split('"').next().unwrap();
                assert!(dir.join(name).is_file(), "case {index}: {name} is missing");
                assert!(name != SHARED || referred_by == Some(file_name.as_str()), "case {index}: {file_name} refers to {name}");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
