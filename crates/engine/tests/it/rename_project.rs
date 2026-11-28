//! `project.rename`: the name `$project` stands for in export file names.

use serde_json::json;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

#[test]
fn a_renamed_project_exports_under_its_new_name_until_undone() {
    let mut session = Session::new_project("hero", primitives::cube(2.0), "builtin:cube", None, 128).unwrap();
    let mut preset = execute(&mut session, "export.preset.get", &json!({"name": "Document channels + Normal + AO (No Alpha)"})).unwrap();
    preset["maps"][0]["file"] = json!("$project_$textureSet");

    execute(&mut session, "project.rename", &json!({"name": "  Knight final "})).unwrap();

    let plan = execute(&mut session, "export.plan", &json!({"preset": preset})).unwrap();
    let renamed_file = plan[0]["file"].as_str().unwrap();
    assert!(renamed_file.starts_with("Knight_final_"), "{renamed_file}");

    let reopened = Session::from_project_bytes(&session.to_project_bytes().unwrap()).unwrap();
    assert_eq!(reopened.doc.project.name, "Knight final");

    execute(&mut session, "edit.undo", &json!({})).unwrap();
    let plan = execute(&mut session, "export.plan", &json!({"preset": preset})).unwrap();

    assert_eq!(session.doc.project.name, "hero");
    let restored_file = plan[0]["file"].as_str().unwrap();
    assert!(restored_file.starts_with("hero_"), "{restored_file}");
}

#[test]
fn an_unusable_or_unchanged_project_name_edits_nothing() {
    use texturecraft_engine::project_cmds::MAX_PROJECT_NAME_CHARS;

    let mut session = Session::new_project("hero", primitives::cube(2.0), "builtin:cube", None, 128).unwrap();

    for name in [json!(null), json!(7), json!("   "), json!("a\nb"), json!("界".repeat(MAX_PROJECT_NAME_CHARS + 1))] {
        assert!(execute(&mut session, "project.rename", &json!({"name": name})).is_err(), "{name}");
    }

    execute(&mut session, "project.rename", &json!({"name": "hero"})).unwrap();

    assert_eq!(session.doc.project.name, "hero");
    assert!(!session.modified, "nothing changed");
    assert!(!session.can_undo());

    let at_limit = "界".repeat(MAX_PROJECT_NAME_CHARS);
    let result = execute(&mut session, "project.rename", &json!({"name": format!(" {at_limit} ")})).unwrap();

    assert_eq!(result["name"], at_limit);
}

#[test]
fn a_renamed_project_is_inserted_into_the_template_as_literal_text() {
    let mut session = Session::new_project("hero", primitives::cube(2.0), "builtin:cube", None, 128).unwrap();
    let mut preset = execute(&mut session, "export.preset.get", &json!({"name": "Document channels + Normal + AO (No Alpha)"})).unwrap();
    preset["maps"][0]["file"] = json!("$project/$mesh");

    execute(&mut session, "project.rename", &json!({"name": "Knight/$mesh"})).unwrap();
    let plan = execute(&mut session, "export.plan", &json!({"preset": preset})).unwrap();

    let file = plan[0]["file"].as_str().unwrap();
    assert!(file.starts_with("Knight__mesh/cube."), "{file}");
}
