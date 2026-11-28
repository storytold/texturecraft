//! Mesh parts that share UV coordinates (mirrored/symmetric models): a 3D brush stroke or
//! path on any of them paints the shared texels, not only the part that owns them in the
//! G-buffer. Driven through engine commands the way agents paint.

use serde_json::json;
use texturecraft_engine::core::Channel;
use texturecraft_engine::geom::glam::{Vec2, Vec3, Vec4};
use texturecraft_engine::geom::{Mesh, primitives};
use texturecraft_engine::{Session, execute};

const TEXTURE_SIZE: f32 = 256.0;
const VIEWPORT: f32 = 400.0;

fn session(mesh: Mesh) -> Session {
    let mut session = Session::new_project("t", mesh, "builtin:test", None, TEXTURE_SIZE as u32).unwrap();
    session.viewport = Vec2::splat(VIEWPORT);
    session.camera.yaw = 0.0;
    session.camera.pitch = 0.0;
    session.update();
    execute(&mut session, "tool.material", &json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]})).unwrap();
    execute(&mut session, "tool.brush", &json!({"size": 6, "hardness": 90})).unwrap();
    session
}

/// Two 2×2 walls side by side along X, both facing the camera (+Z), with identical UVs:
/// two mesh parts sharing one UV island. Wall UVs run (x + 1) / 2, so the first wall's point
/// (-0.5, 0) is uv (0.25, 0.5) and the second wall's point (4.5, 0) — its unshifted twin
/// (0.5, 0) — is uv (0.75, 0.5).
fn two_walls_sharing_uvs() -> Mesh {
    let mut wall = primitives::plane(2.0, 8);
    for point in &mut wall.positions {
        *point = Vec3::new(point.x, -point.z, 0.0);
    }
    wall.normals.iter_mut().for_each(|normal| *normal = Vec3::Z);
    wall.tangents.iter_mut().for_each(|tangent| *tangent = Vec4::new(1.0, 0.0, 0.0, 1.0));

    let n = wall.positions.len() as u32;
    let positions: Vec<Vec3> = wall.positions.iter().copied().chain(wall.positions.iter().map(|p| *p + Vec3::new(4.0, 0.0, 0.0))).collect();
    let normals: Vec<Vec3> = wall.normals.iter().chain(wall.normals.iter()).copied().collect();
    let uvs: Vec<Vec2> = wall.uvs.iter().chain(wall.uvs.iter()).copied().collect();
    let indices: Vec<u32> = wall.indices.iter().copied().chain(wall.indices.iter().map(|i| i + n)).collect();
    Mesh::from_parts(positions, Some(normals), uvs, indices, "two walls").unwrap()
}

/// Viewport pixel of world `point`
fn screen(session: &Session, point: Vec3) -> Vec2 {
    let ndc = session.camera.project(point, 1.0).unwrap();
    Vec2::new((ndc.x * 0.5 + 0.5) * VIEWPORT, (0.5 - ndc.y * 0.5) * VIEWPORT)
}

/// Is the composite base color at the texel of UV `uv` the painted red?
fn red_at_uv(session: &Session, uv: Vec2) -> bool {
    let color = session.caches[0].composite.value(Channel::BaseColor, (uv.x * TEXTURE_SIZE) as u32, (uv.y * TEXTURE_SIZE) as u32);
    color[0] > 0.85 && color[1] < 0.15
}

#[test]
fn a_stroke_on_either_part_sharing_uvs_paints_the_shared_texels() {
    let mut session = session(two_walls_sharing_uvs());
    assert!(session.caches[0].gbuf.overlaps > 0, "both walls cover the same texels");

    // Stroke each wall where it is on screen; each stroke must paint the texel its own
    // surface point maps to, whichever wall the G-buffer picked as the texel's owner.
    for point in [Vec3::new(-0.5, 0.0, 0.0), Vec3::new(4.5, 0.0, 0.0)] {
        let pixel = screen(&session, point);
        let hit = session.pick(pixel, session.viewport).unwrap();
        execute(&mut session, "tool.stroke", &json!({"points": [[pixel.x, pixel.y]]})).unwrap();
        session.update();
        assert!(red_at_uv(&session, hit.uv), "a stroke at {point} paints its texel");
    }
}
