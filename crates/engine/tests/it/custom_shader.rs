//! Shader imports and settings share undo, persistence and the automation command path.
use serde_json::json;
use texturecraft_engine::{Session, execute, geom::primitives};

const SOURCE: &str = include_str!("../../../../examples/shaders/banded.glsl");

fn session() -> Session {
    Session::new_project("Shader", primitives::sphere(1.0, 16, 8), "builtin:sphere", None, 128).unwrap()
}

#[test]
fn import_select_edit_and_reopen_keep_the_shader_and_its_parameters() {
    let mut s = session();
    execute(&mut s, "shader.import", &json!({"name":"Banded","source":SOURCE})).unwrap();
    execute(&mut s, "shader.select", &json!({"name":"Banded"})).unwrap();
    let before = execute(&mut s, "shader.list", &json!({})).unwrap();
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 255, 0, 255])).write_to(&mut png, image::ImageFormat::Png).unwrap();
    let resource = s.import_resource_bytes("green.png", png.into_inner()).unwrap();
    execute(&mut s, "shader.set", &json!({"parameters":{"bands":5,"use_normal":false},"textures":{"tint_texture":resource},"instanceName":"Paint preview"}))
        .unwrap();
    let edited = execute(&mut s, "shader.list", &json!({})).unwrap();
    assert_ne!(before, edited);
    execute(&mut s, "edit.undo", &json!({})).unwrap();
    assert_eq!(execute(&mut s, "shader.list", &json!({})).unwrap(), before);
    execute(&mut s, "edit.redo", &json!({})).unwrap();
    execute(&mut s, "shader.select", &json!({"name":"open-pbr"})).unwrap();
    execute(&mut s, "file.removeUnused", &json!({})).unwrap();
    assert!(s.images.contains_key(&resource), "an inactive shader still needs its assigned image");
    execute(&mut s, "shader.select", &json!({"name":"Banded"})).unwrap();
    let renamed = execute(&mut s, "resource.rename", &json!({"name":resource,"to":"shader-green.png"})).unwrap();
    assert_eq!(s.doc.project.display.custom_shaders[0].textures[0].resource.as_deref(), renamed["name"].as_str());
    execute(&mut s, "edit.undo", &json!({})).unwrap();
    assert_eq!(execute(&mut s, "shader.list", &json!({})).unwrap(), edited);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shader.tcraft");
    execute(&mut s, "file.saveAs", &json!({"path":path})).unwrap();
    let mut saved = texturecraft_engine::formats::project_file::load_project(&path).unwrap();
    saved.project.display.custom_shaders[0].wgsl = "invalid cached GPU program".into();
    saved.project.display.custom_shaders[0].parameters[0].kind = "vec4".into();
    texturecraft_engine::formats::project_file::save_project(&path, &saved).unwrap();
    execute(&mut s, "project.new", &json!({})).unwrap();
    execute(&mut s, "file.open", &json!({"path":path})).unwrap();
    assert_eq!(execute(&mut s, "shader.list", &json!({})).unwrap(), edited);
    assert_eq!(s.doc.project.display.custom_shaders[0].source.as_ref(), SOURCE);
    execute(&mut s, "shader.set", &json!({"reset":true})).unwrap();
    assert!(s.doc.project.display.custom_shaders[0].textures.iter().all(|input| input.resource.is_none()));
    assert!(s.doc.project.display.custom_shaders[0].parameters.iter().all(|p| p.value == p.default));
    execute(&mut s, "edit.undo", &json!({})).unwrap();
    assert_eq!(execute(&mut s, "shader.list", &json!({})).unwrap(), edited);
}

#[test]
fn invalid_import_and_parameter_updates_preserve_the_active_shader() {
    let mut s = session();
    execute(&mut s, "shader.import", &json!({"name":"Banded","source":SOURCE})).unwrap();
    execute(&mut s, "shader.select", &json!({"name":"Banded"})).unwrap();
    let before = s.doc.project.display.clone();
    for params in [
        json!({"parameters":{"bands":1}}),
        json!({"parameters":{"bands":3.5}}),
        json!({"parameters":{"use_normal":1}}),
        json!({"parameters":{"shadow_tint":[0.0,1.0]}}),
        json!({"parameters":{"bands":4,"unknown":0}}),
        json!({"textures":{"tint_texture":"missing"}}),
        json!({"reset":"yes"}),
    ] {
        assert!(execute(&mut s, "shader.set", &params).is_err());
        assert_eq!(s.doc.project.display, before);
    }
    assert!(execute(&mut s, "shader.import", &json!({"name":"Bad","source":"void shade(V2F i) { nonsense; }"})).is_err());
    assert_eq!(s.doc.project.display, before);
    assert!(execute(&mut s, "shader.select", &json!({"name":"missing"})).is_err());
    assert_eq!(s.doc.project.display, before);
}
