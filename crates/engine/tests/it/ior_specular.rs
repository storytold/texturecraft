//! The "Use IOR for specular reflection" shader setting persists with projects and templates
//! and stays off for projects and templates without it (texturecraft #749).

use std::io::{Cursor, Write};

use serde_json::json;
use texturecraft_engine::{Session, execute, geom::primitives};
use zip::{ZipWriter, write::SimpleFileOptions};

const IOR: f32 = 1.8;

fn session() -> Session {
    Session::new_project("Ior", primitives::sphere(1.0, 16, 8), "builtin:sphere", None, 128).unwrap()
}

fn opt_in(session: &mut Session) {
    execute(session, "display.set", &json!({"iorSpecular": true, "ior": IOR})).unwrap();
}

fn new_project_uses_ior_specular(session: &mut Session, template: &str) -> bool {
    execute(session, "project.new", &json!({"template": template})).unwrap();
    session.doc.project.display.ior_specular
}

#[test]
fn the_opt_in_survives_save_and_reopen() {
    let mut session = session();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ior.tcraft");
    opt_in(&mut session);

    execute(&mut session, "file.saveAs", &json!({"path": path})).unwrap();
    execute(&mut session, "project.new", &json!({})).unwrap();
    let new_project_uses_it = session.doc.project.display.ior_specular;
    execute(&mut session, "file.open", &json!({"path": path})).unwrap();

    assert!(!new_project_uses_it, "a new project starts with the setting off");
    let display = &session.doc.project.display;
    assert!(display.ior_specular);
    assert_eq!(display.ior, IOR);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn an_older_save_cannot_clear_ior_specular_changes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ior.tcraft");

    for params in [json!({"iorSpecular": false}), json!({"ior": 2.0}), json!({"specGloss": true})] {
        let mut session = session();
        opt_in(&mut session);
        let save = session.save_task(path.to_str().unwrap(), true);

        execute(&mut session, "display.set", &params).unwrap();
        save.run(None).unwrap().apply(&mut session);

        assert!(session.modified, "the archive predates the edit {params}");
        let archived = texturecraft_engine::file_cmds::open_path(path.to_str().unwrap()).unwrap();
        assert_ne!(archived.doc.project.display, session.doc.project.display);
    }
}

#[test]
fn a_project_saved_before_the_setting_opens_with_it_off() {
    let mut session = session();
    opt_in(&mut session);
    let mut project = serde_json::to_value(&session.doc.project).unwrap();
    let removed = project["display"].as_object_mut().unwrap().remove("ior_specular");
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    archive.start_file("project.json", SimpleFileOptions::default()).unwrap();
    archive.write_all(&serde_json::to_vec(&project).unwrap()).unwrap();
    let bytes = archive.finish().unwrap().into_inner();

    let older = Session::from_project_bytes(&bytes).unwrap();

    assert_eq!(removed, Some(json!(true)), "the stripped field is the saved setting");
    let display = &older.doc.project.display;
    assert!(!display.ior_specular);
    assert_eq!(display.ior, IOR, "the other shader settings still load");
}

#[test]
fn the_opt_in_survives_a_template_and_other_templates_leave_it_off() {
    let library = tempfile::tempdir().unwrap();
    let mut author = session();
    author.load_templates(library.path().to_path_buf());
    opt_in(&mut author);
    execute(&mut author, "project.template.save", &json!({"name": "Glass"})).unwrap();

    // A template written before the setting existed
    let older = json!({"name": "Older", "resolution": 128, "ior": IOR});
    std::fs::write(library.path().join("Older.json"), serde_json::to_vec(&older).unwrap()).unwrap();

    let mut user = session();
    let warnings = user.load_templates(library.path().to_path_buf());
    let listed = execute(&mut user, "project.templates", &json!({})).unwrap();

    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(new_project_uses_ior_specular(&mut user, "Glass"));
    assert_eq!(user.doc.project.display.ior, IOR);
    assert!(!new_project_uses_ior_specular(&mut user, "Older"));
    for template in listed.as_array().unwrap() {
        let name = template["name"].as_str().unwrap();
        assert_eq!(template["iorSpecular"], json!(name == "Glass"), "{name}");
        assert_eq!(new_project_uses_ior_specular(&mut user, name), name == "Glass", "{name}");
    }
}
