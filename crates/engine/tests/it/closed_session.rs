//! The empty state (no project open): project-dependent commands are refused, and
//! File > Close returns to it.

use serde_json::json;
use texturecraft_engine::{EngineError, Session, Tool, check_enabled, command_specs, execute, find};

fn open_project() -> Session {
    Session::new_project("Open", texturecraft_engine::geom::primitives::cube(1.0), "builtin:cube", None, 128).unwrap()
}

#[test]
fn a_closed_session_refuses_commands_that_need_a_project() {
    let mut session = Session::closed().unwrap();

    assert!(!session.project_open);
    for id in ["layer.add.paint", "file.save", "file.close", "edit.undo", "tool.paint", "bake.run", "file.exportTextures", "viewport.view", "camera.add"] {
        let error = execute(&mut session, id, &json!({})).unwrap_err();

        assert!(matches!(error, EngineError::Disabled { .. }), "{id}: {error}");
    }
    assert!(!session.can_undo(), "nothing was changed");
    assert!(!session.tool_usable(Tool::Paint), "no tool can act without a project");
}

#[test]
fn a_closed_session_lists_project_independent_commands_as_enabled() {
    let session = Session::closed().unwrap();
    for id in ["file.open", "file.new", "project.new", "project.sample"] {
        assert!(check_enabled(find(id).unwrap(), &session).is_ok(), "{id} runs with no project");
    }
    let enabled: Vec<&str> = command_specs().iter().filter(|spec| check_enabled(spec, &session).is_ok()).map(|spec| spec.id).collect();

    assert!(!enabled.iter().any(|id| id.starts_with("layer.") || id.starts_with("tool.") || id.starts_with("bake.")), "{enabled:?}");
}

#[test]
fn creating_a_project_from_the_closed_state_opens_it() {
    let mut session = Session::closed().unwrap();

    execute(&mut session, "project.new", &json!({"mesh": "cube", "resolution": 128})).unwrap();

    assert!(session.project_open);

    execute(&mut session, "layer.add.paint", &json!({})).unwrap();
}

#[test]
fn close_returns_to_the_empty_state_and_allows_reopening() {
    let mut session = open_project();
    session.modified = true;

    execute(&mut session, "file.close", &json!({})).unwrap();

    assert!(!session.project_open && !session.modified && session.path.is_none());
    assert!(execute(&mut session, "file.close", &json!({})).is_err(), "closing again has nothing to close");

    execute(&mut session, "project.new", &json!({"mesh": "cube", "resolution": 128})).unwrap();

    assert!(session.project_open, "a project can be created again");
}

#[test]
fn the_libraries_and_presets_still_answer_without_a_project() {
    let mut session = Session::closed().unwrap();
    for id in [
        "export.presets",
        "brush.preset.list",
        "alpha.list",
        "filter.list",
        "material.list",
        "mask.smart.list",
        "text.fonts",
        "project.templates",
        "project.inspect",
    ] {
        execute(&mut session, id, &json!({})).unwrap_or_else(|error| panic!("{id}: {error}"));
    }
    assert_eq!(texturecraft_engine::inspect::session(&session)["projectOpen"], json!(false));
}
