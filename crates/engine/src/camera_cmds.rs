//! The camera list (the viewport camera menu: the default camera plus saved and imported
//! cameras, saved with the project) and the depth of field focus pick.

use std::f32::consts::TAU;

use serde_json::{Value, json};
use texturecraft_core::Camera;
use texturecraft_geom::OrbitCamera;
use texturecraft_geom::glam::{Quat, Vec2, Vec3};

use crate::commands::{CommandSpec, always, bad, f32p, strp, u64p};
use crate::tool_cmds::viewport_of;
use crate::{Result, Session, cmd};

/// Name of the viewport's own camera in the camera menu (it is not in the list).
pub const DEFAULT_CAMERA: &str = "Default camera";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("camera.list", "Cameras", [], None, "{}", always, |s, _| Ok(list(s))),
        cmd!("camera.add", "Save view as camera", [], None, "{name?}", always, add),
        cmd!(
            "camera.select",
            "Select camera",
            [],
            None,
            "{index? | name?} (neither, or name \"Default camera\": the default camera framing the mesh)",
            always,
            select
        ),
        cmd!("camera.remove", "Remove camera", [], None, "{index? | name?}", has_cameras, remove),
        cmd!("camera.rename", "Rename camera", [], None, "{index? | name?, to}", has_cameras, rename),
        cmd!("camera.bookmark1", "Go to camera 1", ["Viewport"], Some("Ctrl+1"), "{}", always, |s, _| bookmark(s, 0)),
        cmd!("camera.bookmark2", "Go to camera 2", ["Viewport"], Some("Ctrl+2"), "{}", always, |s, _| bookmark(s, 1)),
        cmd!("camera.bookmark3", "Go to camera 3", ["Viewport"], Some("Ctrl+3"), "{}", always, |s, _| bookmark(s, 2)),
        cmd!("camera.bookmark4", "Go to camera 4", ["Viewport"], Some("Ctrl+4"), "{}", always, |s, _| bookmark(s, 3)),
        cmd!("camera.bookmark5", "Go to camera 5", ["Viewport"], Some("Ctrl+5"), "{}", always, |s, _| bookmark(s, 4)),
        cmd!("camera.bookmark6", "Go to camera 6", ["Viewport"], Some("Ctrl+6"), "{}", always, |s, _| bookmark(s, 5)),
        cmd!("camera.bookmark7", "Go to camera 7", ["Viewport"], Some("Ctrl+7"), "{}", always, |s, _| bookmark(s, 6)),
        cmd!("camera.bookmark8", "Go to camera 8", ["Viewport"], Some("Ctrl+8"), "{}", always, |s, _| bookmark(s, 7)),
        cmd!("camera.bookmark9", "Go to camera 9", ["Viewport"], Some("Ctrl+9"), "{}", always, |s, _| bookmark(s, 8)),
        cmd!(
            "camera.view",
            "Fixed view",
            [],
            None,
            "{view: front|back|left|right|top|bottom} → the camera (looks at the mesh from that side and frames it; the projection stays)",
            always,
            |s, p| fixed_view(s, strp(p, "view").ok_or_else(|| bad("camera.view", "`view` is required"))?)
        ),
        cmd!("camera.view.front", "Front view", ["Viewport"], None, "{}", always, |s, _| fixed_view(s, "front")),
        cmd!("camera.view.back", "Back view", ["Viewport"], None, "{}", always, |s, _| fixed_view(s, "back")),
        cmd!("camera.view.left", "Left view", ["Viewport"], None, "{}", always, |s, _| fixed_view(s, "left")),
        cmd!("camera.view.right", "Right view", ["Viewport"], None, "{}", always, |s, _| fixed_view(s, "right")),
        cmd!("camera.view.top", "Top view", ["Viewport"], None, "{}", always, |s, _| fixed_view(s, "top")),
        cmd!("camera.view.bottom", "Bottom view", ["Viewport"], None, "{}", always, |s, _| fixed_view(s, "bottom")),
        cmd!("camera.perspective", "Perspective", ["Viewport"], Some("F5"), "{}", always, |s, _| projection(s, false)),
        cmd!("camera.orthographic", "Orthographic", ["Viewport"], Some("F6"), "{}", always, |s, _| projection(s, true)),
        cmd!(
            "camera.rotation",
            "Camera rotation",
            [],
            None,
            "{mode: constrained|free} → the camera (constrained keeps the horizon level; free turns the camera about its own axes, so the model can be turned over)",
            always,
            |s, p| match strp(p, "mode") {
                Some("constrained") => rotation(s, false),
                Some("free") => rotation(s, true),
                _ => Err(bad("camera.rotation", "`mode` is constrained or free")),
            }
        ),
        cmd!("camera.rotation.free", "Free rotation", ["Viewport"], None, "{}", always, |s, _| rotation(s, true)),
        cmd!("camera.rotation.constrained", "Constrained rotation", ["Viewport"], None, "{}", always, |s, _| rotation(s, false)),
        cmd!(
            "camera.snapRotate",
            "Camera snap rotate",
            [],
            None,
            "{yaw?, pitch? (whole 90° steps, default 0)} → the camera, turned in steps around the model: the angles first go to the nearest step. The pitch stops at the orbit's ±89° limit. The viewport's gesture for it is Alt+Shift+left drag",
            always,
            |s, p| snap_rotate(s, f32p(p, "yaw").unwrap_or(0.0), f32p(p, "pitch").unwrap_or(0.0))
        ),
        cmd!("camera.snapRotate.left", "Camera snap rotate left", ["Viewport"], None, "{}", always, |s, _| snap_rotate(s, -1.0, 0.0)),
        cmd!("camera.snapRotate.right", "Camera snap rotate right", ["Viewport"], None, "{}", always, |s, _| snap_rotate(s, 1.0, 0.0)),
        cmd!("camera.snapRotate.up", "Camera snap rotate up", ["Viewport"], None, "{}", always, |s, _| snap_rotate(s, 0.0, 1.0)),
        cmd!("camera.snapRotate.down", "Camera snap rotate down", ["Viewport"], None, "{}", always, |s, _| snap_rotate(s, 0.0, -1.0)),
        cmd!(
            "camera.next",
            "Next camera",
            ["Viewport"],
            None,
            "{} → the camera: the next saved camera (after the last, the first; from a view that is no saved camera, the first)",
            has_cameras,
            |s, _| step_camera(s, 1)
        ),
        cmd!(
            "camera.previous",
            "Previous camera",
            ["Viewport"],
            None,
            "{} → the camera: the previous saved camera (before the first, the last; from a view that is no saved camera, the last)",
            has_cameras,
            |s, _| step_camera(s, -1)
        ),
        cmd!("viewport.focusAt", "Focus depth of field here", [], None, "{x, y (viewport pixels), viewport?: [w, h]}", always, focus_at),
    ]
}

fn has_cameras(s: &Session) -> std::result::Result<(), String> {
    if s.doc.project.cameras.is_empty() { Err("the project has no saved cameras".into()) } else { Ok(()) }
}

/// The view as a saved camera.
fn saved(c: &OrbitCamera, name: String) -> Camera {
    Camera {
        name,
        target: c.target.to_array(),
        yaw: c.yaw,
        pitch: c.pitch,
        distance: c.distance,
        fov: c.fov,
        ortho: c.ortho,
        orientation: c.free.then(|| c.orient.to_array()),
    }
}

/// Move the view to saved camera `c` (values from project files are checked and clamped).
fn apply(view: &mut OrbitCamera, c: &Camera) {
    let target = Vec3::from(c.target);
    if target.is_finite() {
        view.target = target;
    }

    let r = view.scene_radius.max(1e-3);
    let ok = |v: f32| v.is_finite().then_some(v);
    view.yaw = ok(c.yaw).unwrap_or(view.yaw);
    view.pitch = ok(c.pitch).map_or(view.pitch, |p| p.clamp(-89f32.to_radians(), 89f32.to_radians()));
    view.distance = ok(c.distance).map_or(view.distance, |d| d.clamp(r * 0.05, r * 200.0));
    view.fov = ok(c.fov).map_or(view.fov, |f| f.clamp(5.0, 150.0));
    view.ortho = c.ortho;

    // Older saves and invalid quaternions use the level orbit angles
    let orientation = c.orientation.filter(|_| view.free).map(Quat::from_array).filter(|orientation| orientation.is_normalized());
    if let Some(orientation) = orientation {
        view.orient = orientation.normalize();
    } else {
        view.set_angles(view.yaw, view.pitch);
    }
}

/// The saved camera the view is at, if any.
pub fn current(s: &Session) -> Option<usize> {
    let cameras = &s.doc.project.cameras;
    if let Some(name) = &s.selected_camera_name
        && let Some(index) = cameras.iter().position(|camera| &camera.name == name && matches_view(s.camera, camera))
    {
        return Some(index);
    }

    cameras.iter().position(|camera| matches_view(s.camera, camera))
}

fn matches_view(camera: OrbitCamera, saved_camera: &Camera) -> bool {
    const POSE_TOLERANCE: f32 = 1e-4;
    let mut saved_view = camera;
    apply(&mut saved_view, saved_camera);

    let same_orientation = if camera.free {
        // Either sign of a unit quaternion represents the same orientation
        saved_view.orient.abs_diff_eq(camera.orient, POSE_TOLERANCE) || saved_view.orient.abs_diff_eq(-camera.orient, POSE_TOLERANCE)
    } else {
        let yaw_difference = (saved_view.yaw - camera.yaw).abs() % TAU;
        yaw_difference.min(TAU - yaw_difference) <= POSE_TOLERANCE && (saved_view.pitch - camera.pitch).abs() <= POSE_TOLERANCE
    };
    let close = |a: f32, b: f32| (a - b).abs() <= POSE_TOLERANCE * (1.0 + a.abs());
    (saved_view.target - camera.target).length() <= POSE_TOLERANCE * (1.0 + saved_view.distance)
        && same_orientation
        && close(saved_view.distance, camera.distance)
        && close(saved_view.fov, camera.fov)
        && saved_view.ortho == camera.ortho
}

fn list(s: &Session) -> Value {
    let cams: Vec<Value> = s
        .doc
        .project
        .cameras
        .iter()
        .enumerate()
        .map(|(i, c)| {
            json!({
                "index": i, "name": c.name, "fov": c.fov, "ortho": c.ortho, "distance": c.distance,
                "yaw": c.yaw.to_degrees(), "pitch": c.pitch.to_degrees(), "target": c.target,
            })
        })
        .collect();
    json!({ "cameras": cams, "current": current(s) })
}

/// The camera `index` or `name` of params `p` points at.
fn which(s: &Session, cmd: &str, p: &Value) -> Result<usize> {
    let cams = &s.doc.project.cameras;
    let found = match (u64p(p, "index"), strp(p, "name")) {
        (Some(i), _) => usize::try_from(i).ok().filter(|i| *i < cams.len()),
        (None, Some(n)) => cams.iter().position(|c| c.name == n),
        (None, None) => return Err(bad(cmd, "give `index` or `name`")),
    };
    found.ok_or_else(|| bad(cmd, "no such camera (see camera.list)"))
}

fn unique_name(s: &Session, base: &str) -> String {
    let taken = |n: &str| n == DEFAULT_CAMERA || s.doc.project.cameras.iter().any(|c| c.name == n);
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or_default()
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let base = strp(p, "name").map(str::trim).filter(|n| !n.is_empty()).unwrap_or("Camera");
    let name = unique_name(s, base);
    let cam = saved(&s.camera, name.clone());
    s.edit("Save camera", |s| {
        s.doc.project.cameras.push(cam);
        Ok(json!({ "index": s.doc.project.cameras.len() - 1, "name": name }))
    })
}

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    if u64p(p, "index").is_none() && strp(p, "name").is_none_or(|n| n == DEFAULT_CAMERA) {
        // The default camera: fov and projection from the display settings, framing the mesh
        s.selected_camera_name = None;
        s.camera.fov = s.doc.project.display.fov;
        s.camera.ortho = false;
        return crate::execute(s, "viewport.frame", &Value::Null);
    }

    let i = which(s, "camera.select", p)?;
    if let Some(camera) = s.doc.project.cameras.get(i) {
        // A saved view is clamped against the whole mesh, not against a part framed with F
        s.camera.scene_radius = s.mesh.radius();
        apply(&mut s.camera, camera);
        s.selected_camera_name = Some(camera.name.clone());
    }
    s.revision = s.revision.wrapping_add(1);
    Ok(crate::view_cmds::camera_json(s))
}

/// The fixed views: name, yaw and pitch in degrees. Yaw 0 puts the eye on +Z (the front), yaw 90
/// on +X (the right side); top and bottom stop at the orbit's ±89° pitch limit.
pub const VIEWS: [(&str, f32, f32); 6] =
    [("front", 0.0, 0.0), ("back", 180.0, 0.0), ("left", -90.0, 0.0), ("right", 90.0, 0.0), ("top", 0.0, 89.0), ("bottom", 0.0, -89.0)];

/// Look at the mesh from fixed view `name` and frame it, keeping the projection. The yaw is the
/// one nearest the current yaw, so the desktop UI's glide takes the short way round.
fn fixed_view(s: &mut Session, name: &str) -> Result<Value> {
    let Some(&(_, yaw, pitch)) = VIEWS.iter().find(|(view, ..)| *view == name) else {
        return Err(bad("camera.view", format!("unknown view `{name}`: one of front, back, left, right, top, bottom")));
    };

    let current = if s.camera.yaw.is_finite() { s.camera.yaw } else { 0.0 };
    let yaw = current + (yaw.to_radians() - current + TAU * 0.5).rem_euclid(TAU) - TAU * 0.5;
    s.camera.set_angles(yaw, pitch.to_radians());
    crate::execute(s, "viewport.frame", &Value::Null)
}

/// Constrained or free rotation; the direction of view stays.
fn rotation(s: &mut Session, free: bool) -> Result<Value> {
    s.camera.set_free(free);
    s.revision = s.revision.wrapping_add(1);
    Ok(crate::view_cmds::camera_json(s))
}

/// Turn the camera `yaw` and `pitch` steps of 90° around the model. Angles between steps go to
/// the nearest one first, so repeated calls walk the steps.
fn snap_rotate(s: &mut Session, yaw: f32, pitch: f32) -> Result<Value> {
    let step = std::f32::consts::FRAC_PI_2;
    const STEPS_PER_TURN: f32 = 4.0;
    let limit = 89f32.to_radians();
    let c = &mut s.camera;
    let next_yaw = ((c.yaw / step).round() + yaw.round() % STEPS_PER_TURN) * step;
    let next_pitch = (((c.pitch / step).round() + pitch.round()) * step).clamp(-limit, limit);
    c.set_angles(next_yaw, next_pitch);
    s.revision = s.revision.wrapping_add(1);
    Ok(crate::view_cmds::camera_json(s))
}

/// The next (`direction` = 1) or previous (`direction` = -1) saved camera, wrapping round. From a view that is
/// no saved camera, next goes to the first and previous to the last.
fn step_camera(session: &mut Session, direction: i64) -> Result<Value> {
    let camera_count = session.doc.project.cameras.len() as i64;
    if camera_count == 0 {
        return Err(bad("camera.next", "the project has no saved cameras"));
    }

    let destination = match current(session) {
        Some(index) => (index as i64 + direction).rem_euclid(camera_count),
        None if direction > 0 => 0,
        None => camera_count - 1,
    };
    select(session, &json!({ "index": destination }))
}

/// Switch to a perspective or orthographic projection; position and rotation stay.
fn projection(s: &mut Session, ortho: bool) -> Result<Value> {
    s.camera.ortho = ortho;
    s.revision = s.revision.wrapping_add(1);
    Ok(crate::view_cmds::camera_json(s))
}

/// Shortcut slot `slot` (0-based): the n-th saved camera, in list order.
fn bookmark(s: &mut Session, slot: usize) -> Result<Value> {
    if slot >= s.doc.project.cameras.len() {
        return Err(bad("camera.bookmark", "no saved camera in that slot (see camera.list)"));
    }
    select(s, &json!({ "index": slot }))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let i = which(s, "camera.remove", p)?;
    s.edit("Remove camera", |s| {
        let c = s.doc.project.cameras.remove(i);
        if s.selected_camera_name.as_deref() == Some(c.name.as_str()) {
            s.selected_camera_name = None;
        }

        Ok(json!({ "removed": c.name }))
    })
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let i = which(s, "camera.rename", p)?;
    let to = strp(p, "to").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad("camera.rename", "`to` (the new name) is required"))?;
    if s.doc.project.cameras.get(i).is_some_and(|c| c.name == to) {
        return Ok(json!({ "index": i, "name": to }));
    }

    let name = unique_name(s, to);
    s.edit("Rename camera", |s| {
        if let Some(c) = s.doc.project.cameras.get_mut(i) {
            if s.selected_camera_name.as_deref() == Some(c.name.as_str()) {
                s.selected_camera_name = Some(name.clone());
            }

            c.name = name.clone();
        }
        Ok(json!({ "index": i, "name": name }))
    })
}

/// Focus the depth of field on the mesh under viewport pixel (x, y).
fn focus_at(s: &mut Session, p: &Value) -> Result<Value> {
    let (Some(x), Some(y)) = (f32p(p, "x"), f32p(p, "y")) else { return Err(bad("viewport.focusAt", "`x` and `y` (viewport pixels) are required")) };
    let vp = viewport_of(s, p);
    let hit = s.pick(Vec2::new(x, y), vp).ok_or_else(|| bad("viewport.focusAt", "no mesh under that point"))?;
    let depth = (hit.pos - s.camera.eye()).dot(s.camera.forward()).max(1e-4);
    s.doc.project.display.dof.focus_distance = depth;
    s.revision = s.revision.wrapping_add(1);
    Ok(json!({ "focusDistance": depth }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execute;

    fn session() -> Session {
        Session::new_project("t", texturecraft_geom::primitives::sphere(1.0, 24, 12), "builtin:sphere", None, 128).unwrap()
    }

    #[test]
    fn feedback_saved_free_camera_restores_its_roll_after_project_reload() {
        let mut session = session();
        execute(&mut session, "camera.set", &json!({"yaw": 0, "pitch": 0})).unwrap();
        execute(&mut session, "camera.rotation.free", &Value::Null).unwrap();
        execute(&mut session, "camera.orbit", &json!({"dy": 170})).unwrap();
        let saved_view = session.camera;
        assert!(saved_view.up().y < -0.9);
        execute(&mut session, "camera.add", &json!({"name": "Upside down"})).unwrap();
        let bytes = texturecraft_formats::save_project_bytes(&session.project_file()).unwrap();
        let mut loaded = Session::from_project_bytes(&bytes).unwrap();

        execute(&mut loaded, "camera.rotation.free", &Value::Null).unwrap();
        execute(&mut loaded, "camera.select", &json!({"index": 0})).unwrap();

        assert!((loaded.camera.up() - saved_view.up()).length() < 1e-5, "the saved roll is restored");
        assert!((loaded.camera.eye() - saved_view.eye()).length() < 1e-5);
        assert_eq!(current(&loaded), Some(0));
        execute(&mut loaded, "camera.set", &json!({"yaw": saved_view.yaw.to_degrees(), "pitch": saved_view.pitch.to_degrees()})).unwrap();
        assert_eq!(current(&loaded), None, "a level camera at the same eye is a different view");

        execute(&mut loaded, "camera.rotation.constrained", &Value::Null).unwrap();
        execute(&mut loaded, "camera.select", &json!({"index": 0})).unwrap();
        assert_eq!(loaded.camera.up(), Vec3::Y);
        assert!(!loaded.camera.free);

        let mut legacy_camera = serde_json::to_value(&loaded.doc.project.cameras[0]).unwrap();
        legacy_camera.as_object_mut().unwrap().remove("orientation");
        loaded.doc.project.cameras.push(serde_json::from_value(legacy_camera).unwrap());
        loaded.doc.project.cameras[0].orientation = Some([0.0; 4]);
        execute(&mut loaded, "camera.rotation.free", &Value::Null).unwrap();
        for index in [0, 1] {
            execute(&mut loaded, "camera.select", &json!({"index": index})).unwrap();
            assert!(loaded.camera.up().y > 0.9, "invalid and missing orientations use the level view");
            assert!((loaded.camera.eye() - saved_view.eye()).length() < 1e-5);
        }
    }

    #[test]
    fn feedback_camera_navigation_recognizes_a_full_turn() {
        let mut session = session();
        for (name, yaw) in [("A", 90), ("Duplicate of B", 0), ("B", 0), ("C", 180)] {
            execute(&mut session, "camera.set", &json!({"yaw": yaw, "pitch": 0})).unwrap();
            execute(&mut session, "camera.add", &json!({"name": name})).unwrap();
        }

        for direction in ["camera.snapRotate.right", "camera.snapRotate.left"] {
            execute(&mut session, "camera.select", &json!({"name": "B"})).unwrap();
            for _ in 0..4 {
                execute(&mut session, direction, &Value::Null).unwrap();
            }

            assert_eq!(current(&session), Some(2), "a full turn still matches the selected B");
            execute(&mut session, "camera.next", &Value::Null).unwrap();
            assert_eq!(current(&session), Some(3));
        }
    }

    #[test]
    fn cameras_add_select_rename_remove_and_undo() {
        let mut s = session();
        let home = s.camera;
        assert_eq!(execute(&mut s, "camera.list", &Value::Null).unwrap()["current"], Value::Null);
        execute(&mut s, "camera.set", &json!({"yaw": 30, "pitch": 10, "fov": 25, "ortho": true})).unwrap();
        let side = s.camera;
        assert_eq!(execute(&mut s, "camera.add", &json!({"name": "Side"})).unwrap()["index"], 0);
        assert_eq!(execute(&mut s, "camera.add", &json!({"name": "Side"})).unwrap()["name"], "Side 2", "names stay unique");
        assert_eq!(execute(&mut s, "camera.list", &Value::Null).unwrap()["current"], 0);
        execute(&mut s, "camera.select", &json!({})).unwrap();
        assert!(!s.camera.ortho && s.camera.fov == s.doc.project.display.fov, "the default camera");
        assert_eq!((s.camera.yaw, s.camera.target), (side.yaw, home.target), "framing the mesh, keeping the angles");
        execute(&mut s, "camera.orbit", &json!({"dx": 40})).unwrap();
        execute(&mut s, "camera.select", &json!({"name": "Side"})).unwrap();
        assert_eq!(s.camera, side, "selecting brings back the saved view, fov and projection");
        execute(&mut s, "camera.rename", &json!({"index": 1, "to": "Top"})).unwrap();
        execute(&mut s, "camera.remove", &json!({"name": "Side"})).unwrap();
        let list = execute(&mut s, "camera.list", &Value::Null).unwrap();
        assert_eq!(list["cameras"].as_array().unwrap().len(), 1);
        assert_eq!(list["cameras"][0]["name"], "Top");
        assert!(execute(&mut s, "camera.select", &json!({"index": 5})).is_err());
        assert!(execute(&mut s, "camera.rename", &json!({"index": 0})).is_err(), "`to` is required");
        execute(&mut s, "edit.undo", &Value::Null).unwrap();
        assert_eq!(s.doc.project.cameras.len(), 2, "removing is undoable");
    }

    #[test]
    fn number_shortcuts_go_to_the_first_nine_cameras() {
        let mut s = session();
        assert!(execute(&mut s, "camera.bookmark1", &Value::Null).is_err(), "no cameras yet");
        execute(&mut s, "camera.set", &json!({"yaw": 30})).unwrap();
        let first = s.camera;
        execute(&mut s, "camera.add", &json!({"name": "A"})).unwrap();
        execute(&mut s, "camera.orbit", &json!({"dx": 50})).unwrap();
        execute(&mut s, "camera.add", &json!({"name": "B"})).unwrap();
        execute(&mut s, "camera.bookmark1", &Value::Null).unwrap();
        assert_eq!(s.camera, first);
        execute(&mut s, "camera.bookmark2", &Value::Null).unwrap();
        assert_eq!(execute(&mut s, "camera.list", &Value::Null).unwrap()["current"], 1);
        assert!(execute(&mut s, "camera.bookmark3", &Value::Null).is_err(), "empty slot");
        let ids: Vec<_> = crate::command_specs().iter().filter(|c| c.id.starts_with("camera.bookmark")).collect();
        assert_eq!(ids.len(), 9);
        assert!(ids.iter().all(|c| c.shortcut.is_some()));
    }

    #[test]
    fn fixed_views_look_from_their_side_frame_the_mesh_and_keep_the_projection() {
        let mut s = session();
        execute(&mut s, "camera.set", &json!({"yaw": 37, "pitch": 21, "distance": 9, "target": [0.4, -0.3, 0.2]})).unwrap();
        let (lo, hi) = s.mesh.bounds();
        let mut framed = s.camera;
        framed.frame(lo, hi);
        let sides = [("front", Vec3::Z), ("back", Vec3::NEG_Z), ("left", Vec3::NEG_X), ("right", Vec3::X), ("top", Vec3::Y), ("bottom", Vec3::NEG_Y)];
        for ortho in [false, true] {
            execute(&mut s, "camera.set", &json!({"ortho": ortho})).unwrap();
            for (view, side) in sides {
                // Through the parameterized command and through the menu command alike
                let menu_id = format!("camera.view.{view}");
                for (id, params) in [("camera.view", json!({"view": view})), (menu_id.as_str(), Value::Null)] {
                    execute(&mut s, "camera.orbit", &json!({"dx": 123, "dy": 17})).unwrap();
                    let back = execute(&mut s, id, &params).unwrap();
                    let looking_from = (s.camera.eye() - s.camera.target).normalize();
                    assert!(looking_from.dot(side) > 0.999, "{id} {view}: the eye is on the {side} side, not at {looking_from}");
                    assert_eq!(s.camera.ortho, ortho, "{id} {view} keeps the projection");
                    assert!((s.camera.target - framed.target).length() < 1e-5, "{view} centres the mesh");
                    assert!((s.camera.distance - framed.distance).abs() < 1e-4, "{view} frames the mesh");
                    assert_eq!(back["ortho"], ortho);
                }
            }
        }

        // Orbit still works from a fixed view, and a view is not an undo step
        let steps = s.history_labels().len();
        execute(&mut s, "camera.view.top", &Value::Null).unwrap();
        assert!((s.camera.pitch - 89f32.to_radians()).abs() < 1e-6, "top stops at the orbit limit");
        execute(&mut s, "camera.orbit", &json!({"dx": 30, "dy": -40})).unwrap();
        assert!((s.camera.pitch - 49f32.to_radians()).abs() < 1e-5 && s.camera.yaw.to_degrees().rem_euclid(360.0) > 1.0);
        assert_eq!(s.history_labels().len(), steps);
    }

    #[test]
    fn fixed_views_take_the_nearest_turn_and_reject_unknown_views() {
        let mut s = session();
        execute(&mut s, "camera.set", &json!({"yaw": 715})).unwrap();
        execute(&mut s, "camera.view", &json!({"view": "front"})).unwrap();
        assert!((s.camera.yaw - 720f32.to_radians()).abs() < 1e-4, "two whole turns, not a spin back to 0: {}", s.camera.yaw.to_degrees());
        let before = s.camera;
        assert!(execute(&mut s, "camera.view", &json!({"view": "sideways"})).is_err());
        assert!(execute(&mut s, "camera.view", &json!({})).is_err(), "`view` is required");
        assert!(execute(&mut s, "camera.view", &json!({"view": 3})).is_err());
        assert_eq!(s.camera, before, "a rejected view leaves the camera");
        let menu: Vec<_> = crate::command_specs().iter().filter(|c| c.id.starts_with("camera.view.")).collect();
        assert_eq!(menu.len(), 6);
        assert!(menu.iter().all(|c| c.shortcut.is_none() && c.menu == ["Viewport"]), "no default keys: users bind their own");
    }

    #[test]
    fn perspective_and_orthographic_change_only_the_projection() {
        let mut s = session();
        execute(&mut s, "camera.set", &json!({"yaw": 30, "pitch": 10, "distance": 4})).unwrap();
        let before = s.camera;
        let r = execute(&mut s, "camera.orthographic", &Value::Null).unwrap();
        assert!(s.camera.ortho && r["ortho"] == true);
        assert_eq!(OrbitCamera { ortho: false, ..s.camera }, before, "position and rotation stay");
        execute(&mut s, "camera.orthographic", &Value::Null).unwrap();
        assert!(s.camera.ortho, "a second press keeps it orthographic: two actions, not a toggle");
        execute(&mut s, "camera.perspective", &Value::Null).unwrap();
        assert_eq!(s.camera, before);
        let key = |id: &str| crate::find(id).and_then(|c| c.shortcut);
        assert_eq!((key("camera.perspective"), key("camera.orthographic")), (Some("F5"), Some("F6")));
    }

    #[test]
    fn cameras_round_trip_through_the_project_file() {
        let mut s = session();
        execute(&mut s, "camera.set", &json!({"yaw": -20, "fov": 60})).unwrap();
        execute(&mut s, "camera.add", &json!({"name": "Hero"})).unwrap();
        execute(&mut s, "display.set", &json!({"dof": {"enabled": true, "focusDistance": 3.5, "aperture": 0.2}, "ior": 1.33})).unwrap();
        let bytes = texturecraft_formats::save_project_bytes(&s.project_file()).unwrap();
        let back = Session::from_project_bytes(&bytes).unwrap();
        assert_eq!(back.doc.project.cameras, s.doc.project.cameras);
        assert_eq!(back.doc.project.display.dof, s.doc.project.display.dof);
        assert_eq!(back.doc.project.display.ior, 1.33);
        // Projects saved before cameras and depth of field existed load with the defaults.
        let mut v = serde_json::to_value(&s.doc.project).unwrap();
        v.as_object_mut().unwrap().remove("cameras");
        v["display"].as_object_mut().unwrap().remove("dof");
        let old: texturecraft_core::Project = serde_json::from_value(v).unwrap();
        assert!(old.cameras.is_empty() && !old.display.dof.enabled);
    }

    #[test]
    fn focus_at_picks_the_depth_under_the_cursor() {
        let mut s = session();
        let vp = s.viewport;
        let r = execute(&mut s, "viewport.focusAt", &json!({"x": vp.x / 2.0, "y": vp.y / 2.0})).unwrap();
        let d = r["focusDistance"].as_f64().unwrap() as f32;
        // The front of the unit sphere, one radius before the orbit target.
        assert!((d - (s.camera.distance - 1.0)).abs() < 0.05, "{d} vs {}", s.camera.distance);
        assert_eq!(s.doc.project.display.dof.focus_distance, d);
        assert!(execute(&mut s, "viewport.focusAt", &json!({"x": 1, "y": 1})).is_err(), "nothing under the corner");
    }

    #[test]
    fn free_rotation_turns_the_model_over_and_constrained_stops_at_the_pole() {
        let mut s = session();
        let undo_before = execute(&mut s, "edit.history", &Value::Null).unwrap();
        assert_eq!(execute(&mut s, "camera.get", &Value::Null).unwrap()["rotation"], "constrained");
        execute(&mut s, "camera.orbit", &json!({"dy": 170})).unwrap();
        assert!(s.camera.up().y > 0.99 && s.camera.pitch <= 89f32.to_radians() + 1e-6, "constrained keeps the horizon");

        execute(&mut s, "camera.set", &json!({"yaw": 0, "pitch": 0})).unwrap();
        assert_eq!(execute(&mut s, "camera.rotation", &json!({"mode": "free"})).unwrap()["rotation"], "free");
        let eye = s.camera.eye();
        execute(&mut s, "camera.orbit", &json!({"dy": 170})).unwrap();
        assert!(s.camera.up().y < -0.9, "the model is turned over: {}", s.camera.up());
        assert!((s.camera.eye() - eye).length() > 1.0);

        // Back to constrained: level again, still looking from the same side
        execute(&mut s, "camera.rotation.constrained", &Value::Null).unwrap();
        assert!(!s.camera.free && s.camera.up().y == 1.0);
        assert!(execute(&mut s, "camera.rotation", &json!({"mode": "spin"})).is_err());
        assert_eq!(execute(&mut s, "edit.history", &Value::Null).unwrap(), undo_before, "the view is not an undo step");
    }

    #[test]
    fn a_free_camera_is_put_level_by_fixed_views_and_set_angles() {
        let mut s = session();
        execute(&mut s, "camera.rotation.free", &Value::Null).unwrap();
        execute(&mut s, "camera.orbit", &json!({"dx": 30, "dy": 120})).unwrap();
        execute(&mut s, "camera.view.front", &Value::Null).unwrap();
        let looking_from = (s.camera.eye() - s.camera.target).normalize();
        assert!(looking_from.z > 0.999 && s.camera.up().y > 0.999, "front view, level: {looking_from}");
    }

    #[test]
    fn snap_rotate_walks_the_90_degree_steps() {
        let mut s = session();
        execute(&mut s, "camera.set", &json!({"yaw": 10, "pitch": 5})).unwrap();
        execute(&mut s, "camera.snapRotate", &json!({"yaw": 1})).unwrap();
        assert!((s.camera.yaw.to_degrees() - 90.0).abs() < 1e-3 && s.camera.pitch.abs() < 1e-6, "nearest step, then one step");
        execute(&mut s, "camera.snapRotate.right", &Value::Null).unwrap();
        assert!((s.camera.yaw.to_degrees() - 180.0).abs() < 1e-3);
        execute(&mut s, "camera.snapRotate.left", &Value::Null).unwrap();
        assert!((s.camera.yaw.to_degrees() - 90.0).abs() < 1e-3);
        execute(&mut s, "camera.snapRotate.up", &Value::Null).unwrap();
        execute(&mut s, "camera.snapRotate.up", &Value::Null).unwrap();
        assert!((s.camera.pitch.to_degrees() - 89.0).abs() < 1e-3, "the pitch stops at the orbit limit");
        execute(&mut s, "camera.snapRotate.down", &Value::Null).unwrap();
        assert!(s.camera.pitch.abs() < 1e-6, "the limit counts as the top step");
    }

    #[test]
    fn next_and_previous_camera_walk_the_saved_cameras_and_wrap() {
        let mut s = session();
        assert!(execute(&mut s, "camera.next", &Value::Null).is_err(), "no saved cameras: nothing to go to");
        for (name, yaw) in [("A", 20), ("B", 40), ("C", 60)] {
            execute(&mut s, "camera.set", &json!({"yaw": yaw})).unwrap();
            execute(&mut s, "camera.add", &json!({"name": name})).unwrap();
        }
        execute(&mut s, "camera.orbit", &json!({"dx": 5})).unwrap();
        let current = |s: &mut Session| execute(s, "camera.list", &Value::Null).unwrap()["current"].clone();
        assert_eq!(current(&mut s), Value::Null);
        for want in [0, 1, 2] {
            execute(&mut s, "camera.next", &Value::Null).unwrap();
            assert_eq!(current(&mut s), want);
        }
        execute(&mut s, "camera.next", &Value::Null).unwrap();
        assert_eq!(current(&mut s), 0, "after the last one: the first");
        execute(&mut s, "camera.previous", &Value::Null).unwrap();
        assert_eq!(current(&mut s), 2, "before the first one: the last");
        execute(&mut s, "camera.previous", &Value::Null).unwrap();
        assert_eq!(current(&mut s), 1);
        execute(&mut s, "camera.orbit", &json!({"dx": 5})).unwrap();
        execute(&mut s, "camera.previous", &Value::Null).unwrap();
        assert_eq!(current(&mut s), 2, "from a view that is no saved camera, previous goes to the last");
    }

    #[test]
    fn saved_camera_navigation_visits_distinct_cameras_with_identical_poses() {
        let mut session = session();
        for name in ["First", "Second"] {
            execute(&mut session, "camera.add", &json!({"name": name})).unwrap();
        }
        execute(&mut session, "camera.set", &json!({"yaw": 90})).unwrap();

        for index in [0, 1, 0] {
            execute(&mut session, "camera.next", &Value::Null).unwrap();

            assert_eq!(execute(&mut session, "camera.list", &Value::Null).unwrap()["current"], index);
        }
    }

    #[test]
    fn the_combined_normal_view_is_a_view_mode() {
        let mut s = session();
        execute(&mut s, "viewport.view", &json!({"mode": "normalHeightMesh"})).unwrap();
        assert_eq!(s.view, crate::ViewMode::NormalHeightMesh);
        assert_eq!(s.view.label(), "Normal+height+mesh");
        execute(&mut s, "viewport.view", &json!({"mode": "material"})).unwrap();
        assert_eq!(s.view, crate::ViewMode::Material);
    }
}
