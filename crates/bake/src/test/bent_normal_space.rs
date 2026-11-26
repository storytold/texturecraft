//! Bent normal maps are stored in the low poly's tangent frame (OpenGL green), like the normal map

use super::*;

/// How far a packed bent normal may stray from the expected value: the hemisphere's few rays leave
/// a small sideways remainder
const TOLERANCE: f32 = 0.04;

/// The packed bent normal of every covered texel, with the low poly's surface normal there
fn bent_texels(mesh: &Mesh, out: &BakeOutput) -> Vec<(Vec3, [f32; 3])> {
    let res = out.resolution;
    let g = UvGBuffer::new(mesh, 0, res, res);
    let bent = &out.maps[&BakedMap::BentNormal];
    (0..(res * res) as usize).filter_map(|i| g.surface(mesh, i).map(|s| (s.nrm, [bent[i * 3], bent[i * 3 + 1], bent[i * 3 + 2]]))).collect()
}

fn near(value: [f32; 3], expected: [f32; 3]) -> bool {
    value.iter().zip(expected).all(|(v, e)| (v - e).abs() < TOLERANCE)
}

#[test]
fn every_face_of_an_open_cube_bakes_the_straight_up_tangent_value() {
    let cube = primitives::cube(2.0);
    let out = run(&cube, &settings(64, &[BakedMap::BentNormal])).unwrap();

    let texels = bent_texels(&cube, &out);
    assert!(texels.len() > 1000, "{} texels", texels.len());
    for (normal, value) in texels {
        assert!(near(value, [0.5, 0.5, 1.0]), "nothing occludes the face facing {normal}, so it bakes (0.5, 0.5, 1.0): {value:?}");
    }
}

#[test]
fn an_open_sphere_bakes_the_straight_up_tangent_value_all_around() {
    let sphere = primitives::sphere(1.0, 64, 32);
    let out = run(&sphere, &settings(128, &[BakedMap::BentNormal])).unwrap();

    // The poles squeeze the UV rows, so their tangent frame is left out
    let texels: Vec<_> = bent_texels(&sphere, &out).into_iter().filter(|(normal, _)| normal.y.abs() < 0.9).collect();
    assert!(texels.len() > 1000, "{} texels", texels.len());
    for (normal, value) in texels {
        assert!(near(value, [0.5, 0.5, 1.0]), "the world normal {normal} turns with the surface, the tangent value does not: {value:?}");
    }
}

#[test]
fn a_floor_under_a_sphere_tilts_its_bent_normals_up_the_texture() {
    let mut mesh = primitives::sphere(1.0, 64, 32);
    // The floor occludes but takes no texels (all its UVs on one point)
    let floor = Mesh::from_parts(
        vec![Vec3::new(-6.0, -1.0, -6.0), Vec3::new(6.0, -1.0, -6.0), Vec3::new(6.0, -1.0, 6.0), Vec3::new(-6.0, -1.0, 6.0)],
        Some(vec![Vec3::Y; 4]),
        vec![Vec2::ZERO; 4],
        vec![0, 2, 1, 0, 3, 2],
        "Floor",
    )
    .unwrap();
    mesh.append(&floor);
    let mut parameters = settings(128, &[BakedMap::BentNormal]);
    parameters.maps.bent_normal.parameters.rays =
        RaySettings { rays: 128, max_distance: 4.0, relative_to_bounding_box: false, ..parameters.maps.bent_normal.parameters.rays };

    let out = run(&mesh, &parameters).unwrap();

    // Around the equator the open sky is up: the sphere's V runs down, so OpenGL green (-V) points up.
    // Sideways (red) nothing differs, and the bent normal stays close to the surface normal (blue).
    let equator: Vec<_> = bent_texels(&mesh, &out).into_iter().filter(|(normal, _)| normal.y.abs() < 0.15).collect();
    assert!(equator.len() > 500, "{} texels", equator.len());
    for (normal, [red, _, blue]) in &equator {
        assert!((red - 0.5).abs() < 2.0 * TOLERANCE, "no sideways lean where the normal is {normal}: red {red}");
        assert!(*blue > 0.85, "still mostly along the normal {normal}: blue {blue}");
    }
    let green = mean(equator.iter().map(|(_, value)| value[1]));
    assert!(green > 0.56, "the bent normals lean away from the floor, up the texture: mean green {green}");
}
