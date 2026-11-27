use super::{covered_values, settings, transfer};
use crate::{BakeRequest, CageShell, HighPoly, HighPolyReach};
use texturecraft_core::{BakeSettings, BakedMap, CageType, MeshMatchMode};
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::{Bvh, Mesh, primitives};

/// A distance setting far larger than any fitted distance in these tests, to tell the two apart
const UNFITTED_DISTANCE: f32 = 0.5;

/// Settings baking `maps` with the automatic cage, and distance settings no fitted cage comes near
fn automatic(maps: &[BakedMap]) -> BakeSettings {
    let mut bake_settings = settings(32, maps);
    bake_settings.cage.kind = CageType::Automatic;
    bake_settings.cage.relative_to_bounding_box = false;
    (bake_settings.cage.max_frontal, bake_settings.cage.max_rear) = (UNFITTED_DISTANCE, UNFITTED_DISTANCE);
    bake_settings
}

/// How far each corner of the automatic cage of the flat `low` around `high` rises above it, with the corner's low-poly position
fn cage_heights(low: &Mesh, high: &Mesh, bake_settings: &BakeSettings) -> Vec<(Vec3, f32)> {
    let (low_bvh, high_bvh) = (Bvh::new(low), Bvh::new(high));
    let request = BakeRequest {
        low_base_color: &[],
        mesh: low,
        bvh: &low_bvh,
        material: 0,
        settings: bake_settings,
        high: Some(HighPoly { file_ids: &[], mesh: high, bvh: &high_bvh, base_color: &[] }),
        cage: None,
        skew: None,
        cage_paint: None,
        tiles: (1, 1),
    };
    let shell = CageShell::new(&request).unwrap();
    let corners = shell.corners(bake_settings.cage.max_frontal);
    shell.positions.iter().zip(corners).map(|(position, corner)| (*position, corner.y - position.y)).collect()
}

/// Every submesh on the first material, so the cage shell of material 0 covers all the parts
fn one_material(mesh: &mut Mesh) {
    mesh.materials.truncate(1);
    for submesh in &mut mesh.submeshes {
        submesh.material = 0;
    }
}

#[test]
fn the_automatic_cage_reaches_a_high_poly_the_default_distances_miss() {
    let low = primitives::cube(1.0);
    let high = primitives::sphere(0.8, 32, 16);
    let opacity_of = |bake_settings: &BakeSettings| -> Vec<f32> {
        let output = transfer(&low, &high, None, bake_settings).unwrap();
        covered_values(&low, &output, BakedMap::Opacity).into_iter().map(|(_, opacity)| opacity).collect()
    };

    let distance_based = opacity_of(&settings(32, &[BakedMap::Opacity]));
    assert!(distance_based.contains(&0.0), "the sphere is beyond the default distances at the middle of each face");

    let mut bake_settings = settings(32, &[BakedMap::Opacity]);
    bake_settings.cage.kind = CageType::Automatic;
    let fitted = opacity_of(&bake_settings);
    assert!(fitted.iter().all(|opacity| *opacity == 1.0), "every ray of the automatic cage finds the sphere");

    // The triangles at the sphere's poles have no area, so a pole vertex has no side to face the ray with
    bake_settings.high_poly.ignore_backface = false;
    let (low_bvh, high_bvh) = (Bvh::new(&low), Bvh::new(&high));
    let request = BakeRequest {
        low_base_color: &[],
        mesh: &low,
        bvh: &low_bvh,
        material: 0,
        settings: &bake_settings,
        high: Some(HighPoly { file_ids: &[], mesh: &high, bvh: &high_bvh, base_color: &[] }),
        cage: None,
        skew: None,
        cage_paint: None,
        tiles: (1, 1),
    };
    let reach = HighPolyReach::new(&request).unwrap();
    for vertex in 0..high.positions.len() {
        assert_eq!(reach.missed(vertex, 0.0, 0.0), Some(false), "high-poly vertex {vertex} is inside the cage whatever the distance settings");
    }
}

#[test]
fn the_automatic_cage_stays_low_where_the_high_poly_is_low() {
    const LOW_HEIGHT: f32 = 0.01;
    const RAISED_HEIGHT: f32 = 0.2;
    const RAISED_FROM_X: f32 = 0.25;

    let low = primitives::plane(1.0, 8);
    let mut high = primitives::plane(1.0, 40);
    for position in &mut high.positions {
        position.y = if position.x > RAISED_FROM_X { RAISED_HEIGHT } else { LOW_HEIGHT };
    }

    let heights = cage_heights(&low, &high, &automatic(&[BakedMap::Normal]));

    for (position, height) in heights {
        if position.x < 0.0 {
            assert!((LOW_HEIGHT..RAISED_HEIGHT / 2.0).contains(&height), "the cage at {position} hugs the low part: {height}");
        } else if position.x > RAISED_FROM_X + 0.125 {
            assert!((RAISED_HEIGHT..RAISED_HEIGHT * 2.0).contains(&height), "the cage at {position} clears the raised part: {height}");
        }
    }
}

#[test]
fn a_ray_that_only_finds_a_far_surface_takes_the_nearest_surface_height_instead() {
    const PATCH_HEIGHT: f32 = 0.02;
    const CEILING_HEIGHT: f32 = 1.0;

    // A low-plane vertex beside the patch's edge: the patch is its nearest high poly, the ceiling is straight above it
    const BESIDE_PATCH: Vec3 = Vec3::new(0.125, 0.0, 0.0);

    // Far enough that the ceiling's own corners are nearest to the low plane's border, not to the vertex tested
    const CEILING_REACH: f32 = 1000.0;

    // The ceiling starts just past the patch's edge (x = 0), the patch covers the low plane's other half
    const CEILING_FROM_X: f32 = 0.05;
    const PATCH_SIZE: f32 = 0.5;
    const SAME_POSITION: f32 = 1e-4;

    let low = primitives::plane(1.0, 8);
    let mut high = primitives::plane(PATCH_SIZE, 1);
    for position in &mut high.positions {
        *position = Vec3::new(position.x - PATCH_SIZE / 2.0, PATCH_HEIGHT, position.z);
    }
    let ceiling = Mesh::from_parts(
        vec![
            Vec3::new(CEILING_FROM_X, CEILING_HEIGHT, -CEILING_REACH),
            Vec3::new(CEILING_FROM_X, CEILING_HEIGHT, CEILING_REACH),
            Vec3::new(CEILING_REACH, CEILING_HEIGHT, 0.0),
        ],
        None,
        vec![Default::default(); 3],
        vec![0, 1, 2],
        "ceiling",
    )
    .unwrap();
    high.append(&ceiling);

    let heights = cage_heights(&low, &high, &automatic(&[BakedMap::Normal]));

    let (_, height) = heights.iter().find(|(position, _)| position.distance(BESIDE_PATCH) < SAME_POSITION).unwrap();
    assert!(*height < CEILING_HEIGHT / 2.0, "the cage beside the patch stays near the patch, not the ceiling: {height}");
}

#[test]
fn a_part_no_high_poly_part_matches_keeps_the_distance_settings() {
    let mut low = primitives::plane(1.0, 2);
    let mut high = primitives::plane(1.0, 2);
    low.submeshes[0].name = "panel_low".into();
    high.submeshes[0].name = "door_high".into();
    let mut bake_settings = automatic(&[BakedMap::Normal]);
    bake_settings.high_poly.match_mode = MeshMatchMode::ByMeshName;

    let heights = cage_heights(&low, &high, &bake_settings);

    assert!(heights.iter().all(|(_, height)| *height == UNFITTED_DISTANCE));
}

#[test]
fn a_part_meeting_another_part_takes_its_distances_from_its_own_high_poly_only() {
    const LOW_HEIGHT: f32 = 0.01;
    const RAISED_HEIGHT: f32 = 0.3;

    // Two low parts sharing the edge x = 0, under a low and a raised high-poly part matched by name
    let part = |name: &str, side: f32, height: f32, subdivisions: usize| {
        let mut part = primitives::plane(1.0, subdivisions);
        for position in &mut part.positions {
            *position += Vec3::new(side * 0.5, height, 0.0);
        }
        part.submeshes[0].name = name.into();
        part
    };
    let mut low = part("left_low", -1.0, 0.0, 4);
    low.append(&part("right_low", 1.0, 0.0, 4));
    let mut high = part("left_high", -1.0, LOW_HEIGHT, 8);
    high.append(&part("right_high", 1.0, RAISED_HEIGHT, 8));
    for mesh in [&mut low, &mut high] {
        one_material(mesh);
    }
    let mut bake_settings = automatic(&[BakedMap::Normal]);
    bake_settings.high_poly.match_mode = MeshMatchMode::ByMeshName;

    let heights = cage_heights(&low, &high, &bake_settings);

    // The left part's triangles come first, three corners each
    let left_corners = low.submeshes[0].count as usize;
    for (position, height) in &heights[..left_corners] {
        assert!(*height < RAISED_HEIGHT / 2.0, "the left part's cage at {position} stays near the left high poly: {height}");
    }
}

#[test]
fn parts_back_to_back_fit_their_cages_along_their_own_normals() {
    const SHEET_DISTANCE: f32 = 0.2;

    // A plane facing up and the same plane facing down, each facing its own high-poly sheet: welded, their normals cancel out
    let named = |mut mesh: Mesh, name: &str| {
        mesh.submeshes[0].name = name.into();
        mesh
    };
    let facing_down = |mut mesh: Mesh| {
        for normal in &mut mesh.normals {
            *normal = -*normal;
        }
        for triangle in mesh.indices.as_chunks_mut::<3>().0 {
            triangle.swap(1, 2);
        }
        mesh
    };
    let sheet = |height: f32| {
        let mut sheet = primitives::plane(1.0, 8);
        for position in &mut sheet.positions {
            position.y = height;
        }
        sheet
    };
    let mut low = named(primitives::plane(1.0, 4), "front_low");
    low.append(&named(facing_down(primitives::plane(1.0, 4)), "back_low"));
    let mut high = named(sheet(SHEET_DISTANCE), "front_high");
    high.append(&named(facing_down(sheet(-SHEET_DISTANCE)), "back_high"));
    for mesh in [&mut low, &mut high] {
        one_material(mesh);
    }

    for match_mode in [MeshMatchMode::Always, MeshMatchMode::ByMeshName] {
        let mut bake_settings = automatic(&[BakedMap::Normal]);
        bake_settings.high_poly.match_mode = match_mode;

        let heights = cage_heights(&low, &high, &bake_settings);

        // The front part's triangles come first, three corners each
        let front_corners = low.submeshes[0].count as usize;
        for (position, height) in &heights[..front_corners] {
            assert!(
                (SHEET_DISTANCE..UNFITTED_DISTANCE).contains(height),
                "{match_mode:?}: the front part's cage at {position} reaches the sheet above it, fitted: {height}"
            );
        }
        for (position, height) in &heights[front_corners..] {
            assert!(
                (-UNFITTED_DISTANCE..=-SHEET_DISTANCE).contains(height),
                "{match_mode:?}: the back part's cage at {position} reaches the sheet below it, fitted: {height}"
            );
        }
    }
}
