use std::io::{Cursor, Write};

use serde_json::json;
use texturecraft_engine::Session;
use texturecraft_engine::formats::mesh_import::{corner_uvs, import_mesh_bytes_with_uvs};
use texturecraft_engine::geom::glam::Vec2;
use zip::{ZipWriter, write::SimpleFileOptions};

const QUAD_OBJ: &str = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nf 1/1 2/2 3/3\nf 1/1 3/3 4/4\n";

/// A layout no unwrapper of this build makes: the quad turned a quarter and inset, per corner
const LOCKED_LAYOUT: [[f32; 2]; 6] = [[0.9, 0.1], [0.9, 0.9], [0.1, 0.9], [0.9, 0.1], [0.1, 0.9], [0.1, 0.1]];

/// `uv_layout.bin` written by hand: magic, version 1, corner count, then u and v per corner
fn layout_entry(layout: &[[f32; 2]]) -> Vec<u8> {
    let mut bytes = b"TCUV".to_vec();
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(u32::try_from(layout.len()).unwrap().to_le_bytes());
    for [u, v] in layout {
        bytes.extend(u.to_le_bytes());
        bytes.extend(v.to_le_bytes());
    }
    bytes
}

/// A format 4 archive of the quad with rebuilt UVs and the locked layout
fn locked_archive() -> Vec<u8> {
    let session = Session::from_mesh_bytes_with_uvs("quad.obj", QUAD_OBJ.as_bytes(), 128, "quad", None, true).unwrap();
    let mut project = serde_json::to_value(&session.doc.project).unwrap();
    project["format_version"] = json!(4);

    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in
        [("project.json", serde_json::to_vec(&project).unwrap()), ("mesh.obj", QUAD_OBJ.as_bytes().to_vec()), ("uv_layout.bin", layout_entry(&LOCKED_LAYOUT))]
    {
        archive.start_file(name, SimpleFileOptions::default()).unwrap();
        archive.write_all(&bytes).unwrap();
    }
    archive.finish().unwrap().into_inner()
}

#[test]
fn a_rebuilt_project_reopens_with_its_saved_uv_layout_not_a_new_unwrap() {
    let locked: Vec<Vec2> = LOCKED_LAYOUT.iter().map(|uv| Vec2::from_array(*uv)).collect();
    let unwrapped = import_mesh_bytes_with_uvs("quad.obj", QUAD_OBJ.as_bytes(), true).unwrap().mesh;

    let mut reopened = Session::from_project_bytes(&locked_archive()).unwrap();
    let opened_layout = corner_uvs(&reopened.source_mesh());
    reopened.reimport_mesh(None, false).unwrap();
    let reimported_layout = corner_uvs(&reopened.source_mesh());
    let resaved = Session::from_project_bytes(&reopened.to_project_bytes().unwrap()).unwrap();

    assert_ne!(corner_uvs(&unwrapped), locked, "the locked layout differs from what the unwrapper makes now");
    assert_eq!(opened_layout, locked);
    assert_eq!(reimported_layout, locked, "reimporting the project's own mesh keeps the saved layout");
    assert_eq!(corner_uvs(&resaved.source_mesh()), locked, "saving again keeps the layout");
}

#[test]
fn a_pending_save_keeps_the_rebuilt_layout_and_releases_unused_bake_geometry() {
    let mut session = Session::from_project_bytes(&locked_archive()).unwrap();
    session.add_high_poly_bytes("high.obj", QUAD_OBJ.as_bytes().to_vec()).unwrap();
    let expected_layout = corner_uvs(&session.source_mesh());
    let high = &session.doc.bake_meshes.high[0];
    let encoded = std::sync::Arc::downgrade(&high.bytes);
    let parsed = std::sync::Arc::downgrade(&high.mesh);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saved.tcraft");

    let save = session.save_task(path.to_str().unwrap(), true);
    drop(session);
    assert!(parsed.upgrade().is_none(), "only the source UV layout needs parsed geometry");
    assert!(encoded.upgrade().is_some(), "the pending archive shares the original file bytes");

    save.run(None).unwrap();
    let reopened = texturecraft_engine::file_cmds::open_path(path.to_str().unwrap()).unwrap();

    assert!(encoded.upgrade().is_none(), "the worker released the original bytes");
    assert_eq!(reopened.doc.bake_meshes.high[0].bytes.as_slice(), QUAD_OBJ.as_bytes());
    assert!(reopened.doc.project.mesh_rebuild_uvs);
    assert_eq!(corner_uvs(&reopened.source_mesh()), expected_layout);
}

#[test]
fn opening_a_builtin_project_refuses_rebuilt_mesh_uvs() {
    let mut file = texturecraft_engine::formats::load_project_bytes(&locked_archive()).unwrap();
    file.project.mesh = "builtin:plane".into();
    file.mesh = None;

    let error = Session::from_project_file(file).map(|_| ()).expect_err("a built-in mesh cannot use the stored layout");

    assert!(error.to_string().contains("uv_layout.bin"), "{error}");
}
