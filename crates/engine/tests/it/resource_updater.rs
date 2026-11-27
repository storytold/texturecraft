//! The Resources Updater: image resources remember their files, a changed file shows as outdated,
//! Update reads it again (one undo step, recompositing its layers) and Relink follows another file

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};
use texturecraft_engine::core::{BakedMap, Channel};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{ResourceStatus, Session, execute};

const RESOLUTION: u32 = 128;
const SAMPLE_STRIDE: usize = 8;
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];

fn session() -> Session {
    Session::new_project("t", primitives::sphere(1.0, 16, 8), "builtin:sphere", None, RESOLUTION).unwrap()
}

/// Write a `size` × `size` PNG of one colour (sizes differ between versions, so the file size changes too)
fn write_png(path: &Path, size: u32, color: [u8; 4]) {
    image::RgbaImage::from_pixel(size, size, image::Rgba(color)).save(path).unwrap();
}

fn text(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// Give the file at `path` the modification time `time` without changing its contents
fn set_modified(path: &Path, time: SystemTime) {
    std::fs::File::options().write(true).open(path).unwrap().set_modified(time).unwrap();
}

fn modified(path: &Path) -> SystemTime {
    std::fs::metadata(path).unwrap().modified().unwrap()
}

fn import(s: &mut Session, path: &Path) -> String {
    execute(s, "resource.import", &json!({"path": text(path)})).unwrap()["name"].as_str().unwrap().to_string()
}

/// Import `path` and show it as a base colour fill; returns the resource name
fn import_as_fill(s: &mut Session, path: &Path) -> String {
    let name = import(s, path);
    execute(s, "layer.add.image", &json!({"name": name})).unwrap();
    s.update();
    name
}

fn base_colors(s: &Session) -> Vec<[f32; 3]> {
    let composite = &s.caches[s.ts()].composite;
    let samples = || (0..RESOLUTION).step_by(SAMPLE_STRIDE);
    samples().flat_map(|y| samples().map(move |x| (x, y))).map(|(x, y)| composite.value(Channel::BaseColor, x, y)).collect()
}

fn status_of(s: &mut Session, name: &str) -> ResourceStatus {
    s.resource_states().into_iter().find(|state| state.name == name).expect("the resource is listed").status
}

#[test]
fn a_changed_file_is_outdated_and_update_replaces_its_pixels_as_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bricks.png");
    write_png(&file, 4, RED);
    let mut s = session();
    let name = import_as_fill(&mut s, &file);
    let before = base_colors(&s);
    assert_eq!(status_of(&mut s, &name), ResourceStatus::UpToDate);

    write_png(&file, 8, GREEN);
    let status = execute(&mut s, "resource.status", &Value::Null).unwrap();

    assert_eq!(status["outdated"], json!(1));
    assert_eq!(status["resources"][0]["name"], json!(name));
    assert_eq!(status["resources"][0]["status"], json!("outdated"));

    let steps = s.history_labels().len();
    let updated = execute(&mut s, "resource.update", &json!({})).unwrap();
    s.update();
    let after = base_colors(&s);

    assert_eq!(updated["updated"], json!([name]));
    assert_eq!(s.history_labels().len(), steps + 1, "one undo step");
    assert_eq!(status_of(&mut s, &name), ResourceStatus::UpToDate);
    assert_eq!(s.images[&name].width, 8, "the new pixels are decoded");
    assert_ne!(after, before, "the fill using the resource recomposited");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    s.update();

    assert_eq!(s.images[&name].width, 4, "undo brings back the old pixels under the same name");
    assert_eq!(base_colors(&s), before);
    assert_eq!(status_of(&mut s, &name), ResourceStatus::Outdated, "the file on disk is still the new one");

    execute(&mut s, "edit.redo", &Value::Null).unwrap();
    s.update();

    assert_eq!(s.images[&name].width, 8);
    assert_eq!(base_colors(&s), after);
}

#[test]
fn outdated_resources_come_first_then_missing_up_to_date_unlinked_and_materials() {
    let dir = tempfile::tempdir().unwrap();
    let [fine, gone, changed] = ["b_fine.png", "c_gone.png", "d_changed.png"].map(|name| dir.path().join(name));
    for file in [&fine, &gone, &changed] {
        write_png(file, 4, RED);
    }
    let mut s = session();
    let png = std::fs::read(&fine).unwrap();
    s.import_resource_bytes("a_unlinked.png", png).unwrap();
    for file in [&fine, &gone, &changed] {
        import(&mut s, file);
    }

    // The environment map and the brush alpha are image resources like any other
    let environment = format!("{}b_fine.png", texturecraft_engine::core::project::ENV_RESOURCE_PREFIX);
    execute(&mut s, "display.set", &json!({"environment": environment})).unwrap();
    execute(&mut s, "tool.brush", &json!({"alpha": {"image": "d_changed.png"}})).unwrap();

    // A linked MaterialCraft material keeps its package as a resource without a file
    s.doc.resources.insert("a_brick.mcmat".to_string(), Arc::new(b"package".to_vec()));
    let layer = execute(&mut s, "layer.add.fill", &Value::Null).unwrap()["layer"].as_u64().unwrap();
    let link = serde_json::from_value(json!({"name": "Brick", "package": "a_brick.mcmat", "graph": "Brick", "params": [], "size": 4})).unwrap();
    if let texturecraft_engine::core::LayerKind::Fill(fill) = &mut s.doc.project.layers.get_mut(&texturecraft_engine::core::LayerId(layer)).unwrap().kind {
        fill.link = Some(link);
    }

    std::fs::remove_file(&gone).unwrap();
    write_png(&changed, 8, GREEN);

    let states = s.resource_states();
    let order: Vec<&str> = states.iter().map(|state| state.name.as_str()).collect();
    let statuses: Vec<&str> = states.iter().map(|state| state.status.id()).collect();

    assert_eq!(order, ["d_changed.png", "c_gone.png", "b_fine.png", "a_unlinked.png", "a_brick.mcmat"], "by status first, against the name order");
    assert_eq!(statuses, ["outdated", "missing", "upToDate", "noSource", "material"], "the alpha's and the environment's files are checked too");
    let error = execute(&mut s, "resource.update", &json!({"names": ["a_brick.mcmat"]})).unwrap_err();
    assert!(error.to_string().contains("only image resources can be relinked"), "{error}");
}

#[test]
fn a_same_size_file_is_compared_by_contents_once_per_time() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("same.png");
    write_png(&file, 4, RED);
    let mut s = session();
    let name = import(&mut s, &file);
    let original = std::fs::read(&file).unwrap();
    let later = modified(&file) + Duration::from_secs(10);

    // Saved again unchanged: a new time alone is no change, and updating adds no step
    set_modified(&file, later);
    let steps = s.history_labels().len();
    assert_eq!(status_of(&mut s, &name), ResourceStatus::UpToDate);
    assert_eq!(execute(&mut s, "resource.update", &json!({})).unwrap()["updated"], json!([]));
    assert_eq!(s.history_labels().len(), steps);

    // Different bytes of the same size and time are not read again: the verdict for that size and time stands
    let mut edited = original.clone();
    let last = edited.len() - 1;
    edited[last] ^= 0xff;
    std::fs::write(&file, &edited).unwrap();
    set_modified(&file, later);
    assert_eq!(status_of(&mut s, &name), ResourceStatus::UpToDate, "the remembered verdict is used");

    // Another time reads the file, and its bytes differ
    set_modified(&file, later + Duration::from_secs(10));
    assert_eq!(status_of(&mut s, &name), ResourceStatus::Outdated);
}

#[cfg(unix)]
#[test]
fn a_file_without_a_usable_timestamp_is_compared_on_each_check() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("archived.png");
    write_png(&path, 4, RED);
    set_modified(&path, SystemTime::UNIX_EPOCH - Duration::from_secs(20));
    let mut session = session();
    let name = import(&mut session, &path);
    assert_eq!(status_of(&mut session, &name), ResourceStatus::UpToDate);

    let mut changed = std::fs::read(&path).unwrap();
    changed[0] ^= 1;
    std::fs::write(&path, changed).unwrap();
    set_modified(&path, SystemTime::UNIX_EPOCH - Duration::from_secs(10));

    assert_eq!(status_of(&mut session, &name), ResourceStatus::Outdated);
}

#[test]
fn a_missing_file_is_reported_and_updating_it_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("gone.png");
    write_png(&file, 4, RED);
    let mut s = session();
    let name = import_as_fill(&mut s, &file);
    std::fs::remove_file(&file).unwrap();
    let steps = s.history_labels().len();

    let state = s.resource_states().into_iter().find(|state| state.name == name).unwrap();
    let error = execute(&mut s, "resource.update", &json!({"names": [name]})).unwrap_err();

    assert_eq!(state.status, ResourceStatus::Missing);
    assert!(state.error.is_some(), "the reason is shown");
    assert!(error.to_string().contains("nothing was updated"), "{error}");
    assert_eq!(s.history_labels().len(), steps);
    assert_eq!(s.images[&name].width, 4, "the resource keeps its pixels");
}

#[test]
fn an_unreadable_file_fails_alone_while_the_others_update() {
    let dir = tempfile::tempdir().unwrap();
    let (kept, gone) = (dir.path().join("kept.png"), dir.path().join("gone.png"));
    write_png(&kept, 4, RED);
    write_png(&gone, 4, RED);
    let mut s = session();
    let kept_name = import_as_fill(&mut s, &kept);
    let gone_name = import_as_fill(&mut s, &gone);
    write_png(&kept, 8, GREEN);
    std::fs::remove_file(&gone).unwrap();

    let result = execute(&mut s, "resource.update", &json!({"names": [kept_name, gone_name]})).unwrap();

    assert_eq!(result["updated"], json!([kept_name]));
    assert_eq!(result["failed"][0]["name"], json!(gone_name));
    assert_eq!(s.images[&kept_name].width, 8);
}

#[test]
fn relink_reads_another_file_and_follows_it_from_then_on() {
    let dir = tempfile::tempdir().unwrap();
    let (first, second) = (dir.path().join("first.png"), dir.path().join("second.png"));
    write_png(&first, 4, RED);
    write_png(&second, 8, GREEN);
    let mut s = session();
    let name = import_as_fill(&mut s, &first);
    let before = base_colors(&s);
    let steps = s.history_labels().len();

    execute(&mut s, "resource.relink", &json!({"name": name, "path": text(&second)})).unwrap();
    s.update();

    assert_eq!(s.history_labels().len(), steps + 1, "one undo step");
    assert_eq!(s.images[&name].width, 8, "the resource keeps its name and takes the new file's pixels");
    assert_ne!(base_colors(&s), before);
    assert!(s.doc.project.resource_sources[&name].path.ends_with("second.png"));

    // The old file no longer matters, the new one does
    write_png(&first, 16, GREEN);
    assert_eq!(status_of(&mut s, &name), ResourceStatus::UpToDate);
    write_png(&second, 16, RED);
    assert_eq!(status_of(&mut s, &name), ResourceStatus::Outdated);

    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(s.doc.project.resource_sources[&name].path.ends_with("first.png"), "undo restores the link");
}

#[test]
fn relink_refuses_resources_that_are_not_images() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("picture.png");
    write_png(&file, 4, RED);
    let mut s = session();
    s.doc.resources.insert("material.mcmat".to_string(), Arc::new(vec![1, 2, 3]));
    let steps = s.history_labels().len();

    let error = execute(&mut s, "resource.relink", &json!({"name": "material.mcmat", "path": text(&file)})).unwrap_err();

    assert!(error.to_string().contains("no image resource"), "{error}");
    assert_eq!(*s.doc.resources["material.mcmat"], vec![1, 2, 3]);
    assert_eq!(s.history_labels().len(), steps);
}

#[test]
fn resources_without_a_file_cannot_update_and_a_reopened_project_keeps_its_links() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("kept.png");
    write_png(&file, 4, RED);
    let mut s = session();
    let linked = import_as_fill(&mut s, &file);
    let png = std::fs::read(&file).unwrap();
    let unlinked = s.import_resource_bytes("made.png", png).unwrap();

    assert_eq!(status_of(&mut s, &unlinked), ResourceStatus::NoSource);
    assert!(execute(&mut s, "resource.update", &json!({"names": [unlinked]})).is_err());

    let mut reopened = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();
    write_png(&file, 8, GREEN);

    assert_eq!(status_of(&mut reopened, &linked), ResourceStatus::Outdated);
}

#[test]
fn a_removed_resource_forgets_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("old.png");
    write_png(&file, 4, RED);
    let mut s = session();
    let name = import(&mut s, &file);

    execute(&mut s, "resource.remove", &json!({"name": name})).unwrap();
    let png = std::fs::read(&file).unwrap();
    let reused = s.import_resource_bytes("old.png", png).unwrap();

    assert_eq!(reused, name, "the name is free again");
    assert_eq!(status_of(&mut s, &reused), ResourceStatus::NoSource, "a new resource under the old name does not inherit the file");
}

/// The material ball: texture sets "Ball" and "Base"
fn ball() -> Session {
    Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap()
}

fn ambient_occlusion(s: &Session) -> Vec<f32> {
    s.doc.baked[&0].maps[&BakedMap::AmbientOcclusion].as_ref().clone()
}

#[test]
fn an_imported_mesh_map_follows_its_changed_file_until_something_else_replaces_it() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Ball_ao.png");
    write_png(&file, 4, [60, 60, 60, 255]);
    let mut s = ball();
    execute(&mut s, "textureSet.meshMap.import", &json!({"files": [{"path": text(&file)}]})).unwrap();
    let name = s.doc.project.resource_sources.keys().next().expect("the file is remembered").clone();
    let imported = ambient_occlusion(&s);

    write_png(&file, 8, [200, 200, 200, 255]);
    execute(&mut s, "resource.update", &json!({})).unwrap();
    let updated = ambient_occlusion(&s);

    assert_ne!(updated, imported, "the mesh map is resampled from the new file");

    // Another image in the slot is not overwritten by a later change of the file
    let other = dir.path().join("other.png");
    write_png(&other, 4, [10, 10, 10, 255]);
    let other_name = import(&mut s, &other);
    execute(&mut s, "textureSet.meshMap.set", &json!({"ts": 0, "map": "ambientOcclusion", "image": other_name})).unwrap();
    let replaced = ambient_occlusion(&s);
    write_png(&file, 16, [120, 120, 120, 255]);
    let result = execute(&mut s, "resource.update", &json!({"names": [name]})).unwrap();

    assert_eq!(result["updated"], json!([name]), "the resource itself still updates");
    assert_eq!(ambient_occlusion(&s), replaced, "the mesh map keeps the image set over it");
}

#[test]
fn an_explicit_mesh_map_replacement_breaks_the_import_link_even_when_pixels_match() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("Ball_ao.png");
    let replacement = dir.path().join("replacement.png");
    let equal_color = [60, 60, 60, 255];
    write_png(&original, 4, equal_color);
    write_png(&replacement, 4, equal_color);
    let mut session = ball();
    let replacement_name = import(&mut session, &replacement);
    execute(&mut session, "textureSet.meshMap.import", &json!({"files": [{"path": text(&original)}]})).unwrap();
    let imported = ambient_occlusion(&session);

    execute(&mut session, "textureSet.meshMap.set", &json!({"ts": 0, "map": "ambientOcclusion", "image": replacement_name})).unwrap();
    write_png(&original, 8, [200, 200, 200, 255]);
    let result = execute(&mut session, "resource.update", &Value::Null).unwrap();

    assert_eq!(result["updated"], json!(["Ball_ao.png"]));
    assert!(ambient_occlusion(&session) == imported, "replacing a map explicitly takes ownership even when the pixels were equal");
}

#[test]
fn a_file_import_replaces_bake_provenance_and_clear_detaches_the_link() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Ball_position.png");
    write_png(&file, 4, RED);
    let mut session = ball();
    execute(&mut session, "bake.run", &json!({"ts": 0, "resolution": 32, "maps": ["position"], "dilation": 0})).unwrap();
    execute(&mut session, "bake.settings", &json!({"ts": 0, "positionMode": "boundingBox"})).unwrap();
    assert!(!session.bake_changes(0, BakedMap::Position).is_empty(), "the previous bake is stale");

    execute(&mut session, "textureSet.meshMap.import", &json!({"files": [{"path": text(&file)}]})).unwrap();
    write_png(&file, 8, [0, 0, 255, 255]);
    let imported = session.baked(0).unwrap().maps[&BakedMap::Position].clone();
    execute(&mut session, "resource.update", &Value::Null).unwrap();

    assert_ne!(*imported, *session.baked(0).unwrap().maps[&BakedMap::Position], "the replacement follows its file");
    assert!(session.bake_changes(0, BakedMap::Position).is_empty(), "the imported map has no bake provenance");

    execute(&mut session, "textureSet.meshMap.clear", &json!({"ts": 0, "map": "position"})).unwrap();
    assert!(session.doc.project.resource_sources["Ball_position.png"].mesh_map.is_none(), "clearing releases the file link");
    write_png(&file, 16, RED);
    execute(&mut session, "resource.update", &Value::Null).unwrap();
    assert!(!session.baked(0).unwrap().maps.contains_key(&BakedMap::Position), "updating the old source leaves the slot empty");
}

#[test]
fn removing_unused_resources_keeps_the_file_that_feeds_an_imported_mesh_map() {
    let dir = tempfile::tempdir().unwrap();
    let map_file = dir.path().join("Ball_ao.png");
    let unused_file = dir.path().join("unused.png");
    write_png(&map_file, 4, [60, 60, 60, 255]);
    write_png(&unused_file, 4, RED);
    let mut session = ball();
    let unused_name = import(&mut session, &unused_file);
    execute(&mut session, "textureSet.meshMap.import", &json!({"files": [{"path": text(&map_file)}]})).unwrap();
    let imported = ambient_occlusion(&session);

    let cleanup = execute(&mut session, "file.removeUnused", &Value::Null).unwrap();

    assert_eq!(cleanup["removed"], json!([unused_name]));
    write_png(&map_file, 8, [200, 200, 200, 255]);
    let update = execute(&mut session, "resource.update", &Value::Null).unwrap();

    assert_eq!(update["updated"], json!(["Ball_ao.png"]));
    assert_ne!(ambient_occlusion(&session), imported);
}

#[test]
fn updating_duplicate_names_replaces_each_image_once_and_releases_old_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let files = ["first.png", "second.png"].map(|name| dir.path().join(name));
    let mut session = session();
    let names = files.each_ref().map(|path| {
        write_png(path, 4, RED);
        import(&mut session, path)
    });
    let old_pixels = names.each_ref().map(|name| Arc::downgrade(&session.images[name]));
    for file in &files {
        write_png(file, 8, GREEN);
    }

    let result = execute(&mut session, "resource.update", &json!({"names": [&names[0], &names[1], &names[0]]})).unwrap();

    assert_eq!(result["updated"], json!(names));
    assert!(old_pixels.iter().all(|pixels| pixels.upgrade().is_none()), "history keeps the encoded bytes, so replaced decoded images can drop");
    for name in names {
        assert_eq!(session.images[&name].width, 8);
    }
}

#[test]
fn pbr_folder_and_materialx_imports_remember_their_image_files() {
    let dir = tempfile::tempdir().unwrap();
    let long_name = |suffix: &str| format!("{}{suffix}", "m".repeat(texturecraft_formats::MAX_RESOURCE_NAME_LEN - suffix.len()));
    let color = dir.path().join(long_name("_col.png"));
    write_png(&color, 4, RED);
    std::fs::create_dir(dir.path().join("tex")).unwrap();
    let base_name = long_name("_base.png");
    let base = dir.path().join("tex").join(&base_name);
    write_png(&base, 4, GREEN);
    let mtlx = dir.path().join("m.mtlx");
    let document = r#"<?xml version="1.0"?>
<materialx version="1.39" fileprefix="tex/">
  <open_pbr_surface name="S" type="surfaceshader"><input name="base_color" type="color3" nodename="a" /></open_pbr_surface>
  <tiledimage name="a" type="color3"><input name="file" type="filename" value="base.png" /></tiledimage>
  <surfacematerial name="M" type="material"><input name="surfaceshader" type="surfaceshader" nodename="S" /></surfacematerial>
</materialx>"#;
    std::fs::write(&mtlx, document.replace("base.png", &base_name)).unwrap();
    let mut s = session();
    import(&mut s, &color);
    import(&mut s, &base);

    let pbr = execute(&mut s, "file.importPbrFolder", &json!({"files": [text(&color)]})).unwrap();
    let materialx = execute(&mut s, "file.importMaterialX", &json!({"path": text(&mtlx)})).unwrap();
    let pbr_name = pbr["resources"][0].as_str().unwrap().to_string();
    let materialx_name = materialx["resources"][0].as_str().unwrap().to_string();

    let mut reopened = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();
    assert_eq!(Path::new(&reopened.doc.project.resource_sources[&pbr_name].path).canonicalize().unwrap(), color.canonicalize().unwrap());
    assert_eq!(Path::new(&reopened.doc.project.resource_sources[&materialx_name].path).canonicalize().unwrap(), base.canonicalize().unwrap());

    write_png(&color, 8, GREEN);
    write_png(&base, 8, RED);

    let rows = reopened.resource_states();
    let missing: Vec<_> = [&pbr_name, &materialx_name]
        .into_iter()
        .filter(|name| !rows.iter().any(|row| row.name == name.as_str() && row.status == ResourceStatus::Outdated))
        .collect();
    assert!(missing.is_empty(), "both material imports must keep their source images after reopening: {missing:?}");
}

#[test]
fn a_mesh_map_imported_twice_follows_the_latest_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Ball_ao.png");
    let import_file = |s: &mut Session| execute(s, "textureSet.meshMap.import", &json!({"files": [{"path": text(&file)}]})).unwrap();
    let mut s = ball();
    write_png(&file, 4, [60, 60, 60, 255]);
    import_file(&mut s);
    let first = s.doc.project.resource_sources.keys().next().unwrap().clone();
    write_png(&file, 8, [200, 200, 200, 255]);
    import_file(&mut s);
    let latest = s.doc.project.resource_sources.keys().find(|name| **name != first).expect("a second resource").clone();
    let imported = ambient_occlusion(&s);

    write_png(&file, 16, [10, 10, 10, 255]);
    execute(&mut s, "resource.update", &json!({"names": [latest]})).unwrap();

    assert_ne!(ambient_occlusion(&s), imported, "the mesh map follows the file imported last");
    assert!(s.doc.project.resource_sources[&first].mesh_map.is_none(), "the earlier import no longer feeds the slot");
}

#[test]
fn an_undecodable_saved_resource_is_reported_even_when_its_file_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("broken.png");
    write_png(&file, 4, RED);
    let mut session = session();
    let name = import(&mut session, &file);
    let invalid = b"incomplete image".to_vec();
    std::fs::write(&file, &invalid).unwrap();
    session.doc.resources.insert(name.clone(), Arc::new(invalid));
    session.images.remove(&name);

    let result = execute(&mut session, "resource.update", &json!({"names": [name]}));

    assert!(result.unwrap_err().to_string().contains("nothing was updated"));
    assert!(!session.images.contains_key(&name));
}

#[test]
fn a_rejected_update_restores_all_previously_decoded_images() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session();
    let names = ["left.png", "right.png"].map(|name| {
        let path = dir.path().join(name);
        write_png(&path, 4, RED);
        let imported = import_as_fill(&mut session, &path);
        write_png(&path, 8, GREEN);
        imported
    });

    // Make final document validation fail after both replacements have been processed
    let layer = session.doc.project.layers.values_mut().next().unwrap();
    layer.kind = texturecraft_engine::core::LayerKind::Instance { source: layer.id };
    let steps = session.history_labels().len();

    let result = execute(&mut session, "resource.update", &json!({"names": names}));

    assert!(result.unwrap_err().to_string().contains("would draw itself"));
    assert_eq!(session.history_labels().len(), steps);
    for name in names {
        assert_eq!(session.images[&name].width, 4);
        assert_eq!(status_of(&mut session, &name), ResourceStatus::Outdated);
    }
}
