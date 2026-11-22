//! The file's cameras as TextureCraft's saved cameras: the viewer camera as the project was saved (named
//! "Saved view") and the mesh's scene cameras under their own names.
//!
//! A file camera is a position and a rotation (turning the camera's −z viewing axis into the scene) in the file's
//! normalised mesh space, which is the imported mesh's space. TextureCraft's cameras orbit a target: the target is
//! put on the viewing axis where it passes closest to the mesh's centre. A perspective camera's vertical field of view
//! is `2 · atan(film height / 2 / focal length)`; an orthographic camera keeps its view height.

use glam::{Quat, Vec3};
use texturecraft_core::Camera;
use texturecraft_import_reader::model::cameras::{CameraProjection, ProjectCamera, SavedCameras};

/// Name of the imported camera that keeps the view the project was saved with.
pub const SAVED_VIEW: &str = "Saved view";
/// Nearest the orbit target may be to the camera.
const MIN_DISTANCE: f32 = 1e-3;

/// The file's viewer and scene cameras, around a mesh centred at `center`.
pub fn cameras(saved: &SavedCameras, center: Vec3) -> Vec<Camera> {
    let mut out = Vec::new();
    if let Some(v) = &saved.viewer
        && let Some(c) = camera(v, SAVED_VIEW, center)
    {
        out.push(c);
    }
    for s in &saved.scene {
        let name = if s.label.trim().is_empty() { "Camera" } else { s.label.as_str() };
        out.extend(camera(s, name, center));
    }
    out
}

/// One file camera as an orbit camera; `None` when its values are unusable.
pub fn camera(c: &ProjectCamera, name: &str, center: Vec3) -> Option<Camera> {
    let position = Vec3::from_array(c.position);
    let [x, y, z, w] = c.rotation;
    let rotation = Quat::from_xyzw(x, y, z, w);
    if !position.is_finite() || !rotation.is_finite() || rotation.length() < 1e-6 {
        return None;
    }
    let rotation = rotation.normalize();
    let forward = (rotation * Vec3::NEG_Z).try_normalize()?;
    let distance = (center - position).dot(forward).max(MIN_DISTANCE);
    let target = position + forward * distance;
    // eye − target = distance · (cos p · sin y, sin p, cos p · cos y)
    let back = -forward;
    let pitch = back.y.clamp(-1.0, 1.0).asin();
    let yaw = back.x.atan2(back.z);
    let (fov, ortho) = match c.projection {
        CameraProjection::Orthographic => (c.orthographic_height, true),
        _ => ((2.0 * (0.5 * c.film_size[1] / c.focal_length).atan()).to_degrees(), false),
    };
    (fov.is_finite() && fov > 0.0).then(|| Camera {
        name: name.to_string(),
        target: target.to_array(),
        yaw,
        pitch,
        distance,
        fov,
        ortho,
        orientation: Some(rotation.to_array()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_camera(position: [f32; 3], rotation: [f32; 4]) -> ProjectCamera {
        ProjectCamera {
            uid: None,
            label: "Cam".into(),
            projection: CameraProjection::Perspective,
            position,
            rotation,
            focal_length: 16.668,
            film_size: [20.7264, 15.5448],
            orthographic_height: 1.0,
            focus_distance: None,
        }
    }

    #[test]
    fn a_camera_looking_down_minus_z_orbits_the_mesh_centre_with_its_field_of_view() {
        let c = camera(&project_camera([0.0, 0.0, 3.0], [0.0, 0.0, 0.0, 1.0]), "View", Vec3::ZERO).unwrap();
        assert!((c.distance - 3.0).abs() < 1e-5 && c.target.iter().all(|v| v.abs() < 1e-5), "{c:?}");
        assert!(c.yaw.abs() < 1e-5 && c.pitch.abs() < 1e-5);
        assert!((c.fov - 50.0).abs() < 0.05, "the viewer camera's lens gives 50 degrees: {}", c.fov);
        // the orbit camera's eye is where the file camera stands
        let eye = Vec3::from_array(c.target) + Vec3::new(c.pitch.cos() * c.yaw.sin(), c.pitch.sin(), c.pitch.cos() * c.yaw.cos()) * c.distance;
        assert!((eye - Vec3::new(0.0, 0.0, 3.0)).length() < 1e-4);
        // turned a quarter around +y: looking down −x from +x
        let q = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let side = camera(&project_camera([2.0, 0.0, 0.0], q.to_array()), "Side", Vec3::ZERO).unwrap();
        assert!((side.yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-4, "{side:?}");
    }

    #[test]
    fn unusable_cameras_are_left_out() {
        assert!(camera(&project_camera([f32::NAN, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]), "x", Vec3::ZERO).is_none());
        assert!(camera(&project_camera([0.0; 3], [0.0; 4]), "x", Vec3::ZERO).is_none());
        let mut flat = project_camera([0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0]);
        flat.focal_length = 0.0;
        assert!(camera(&flat, "x", Vec3::ZERO).is_some_and(|c| c.fov > 179.0), "a zero focal length is the widest lens");
        flat.film_size = [0.0, f32::NAN];
        assert!(camera(&flat, "x", Vec3::ZERO).is_none());
        let saved = SavedCameras {
            viewer: Some(project_camera([0.0, 0.0, 3.0], [0.0, 0.0, 0.0, 1.0])),
            scene: vec![project_camera([0.0, 0.0, 2.0], [0.0, 0.0, 0.0, 1.0])],
            selected: None,
        };
        let all = cameras(&saved, Vec3::ZERO);
        assert_eq!(all.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), [SAVED_VIEW, "Cam"]);
    }
}
