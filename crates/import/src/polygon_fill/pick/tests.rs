use super::*;
use glam::Vec2;
use texturecraft_geom::{Mesh, SubMesh};

/// Quads (two triangles each, one polygon) as parts of material 0: (name, corner positions, UV x offset).
fn quads(list: &[(&str, [[f32; 3]; 4], f32)]) -> Mesh {
    let mut m = Mesh { materials: vec!["m".into()], ..Mesh::default() };
    for (name, corners, u) in list {
        let base = m.positions.len() as u32;
        m.positions.extend(corners.iter().map(|c| Vec3::from_array(*c)));
        m.normals.extend([Vec3::Z; 4]);
        // glTF convention (V down): the original's V = 1 - v
        m.uvs.extend([Vec2::new(*u, 1.0), Vec2::new(u + 0.5, 1.0), Vec2::new(u + 0.5, 0.0), Vec2::new(*u, 0.0)]);
        let start = m.indices.len() as u32;
        m.indices.extend([0, 1, 2, 0, 2, 3].map(|i| base + i));
        m.submeshes.push(SubMesh { name: name.to_string(), material: 0, start, count: 6 });
    }
    m
}

fn quad(x: f32, z: f32) -> [[f32; 3]; 4] {
    [[x, 0.0, z], [x + 1.0, 0.0, z], [x + 1.0, 1.0, z], [x, 1.0, z]]
}

/// Two separate quads side by side in UV and in the XY plane.
fn mesh() -> Mesh {
    quads(&[("q0", quad(0.0, 0.0), 0.0), ("q1", quad(1.0, 0.0), 0.5)])
}

/// Two unit quads, the second half a unit behind the first, both over x 0..1, y 0..1.
fn stacked() -> Mesh {
    quads(&[("front", quad(0.0, 0.0), 0.0), ("back", quad(0.0, -0.5), 0.5)])
}

const POLYGONS: [bool; 2] = [true, false];

fn hit(shape: HitShape, granularity: HitGranularity, view: StrokeView, proj_view_inv: [f32; 16]) -> DecalHit {
    DecalHit {
        uid: None,
        shape,
        color: [1.0; 3],
        granularity,
        view,
        proj_view_inv,
        duplicate_count: 0,
        duplicate_matrix: [0.0; 16],
        paint_through: false,
        collision: None,
        occlusion_culling: None,
    }
}

fn list(p: &Picked) -> Vec<(usize, Vec<u32>)> {
    p.iter().map(|(k, v)| (*k, v.iter().copied().collect())).collect()
}

/// An orthographic camera looking down -Z at x 0..2, y 0..1 (screen = (x / 2, y), y up): its inverse, as stored.
fn camera() -> [f32; 16] {
    let proj_view = Mat4::from_cols_array(&[1.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, -0.1, 0.0, -1.0, -1.0, 0.0, 1.0]);
    proj_view.inverse().to_cols_array()
}

const MIRROR_X1: [f32; 16] = [-1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 2.0, 0.0, 0.0, 1.0];

#[test]
fn uv_clicks_and_rectangles_grow_by_granularity() {
    let m = mesh();
    let g = SetGeometry::new(&m, 0, &[POLYGONS.to_vec(), POLYGONS.to_vec()]);
    let click = |q, gr| pick(&g, &hit(HitShape::Point(q), gr, StrokeView::Uv, [0.0; 16])).unwrap();
    // triangle 0 of the first quad is its lower-right half (the file's UV)
    assert_eq!(list(&click([0.4, 0.1], HitGranularity::Triangle)), [(0, vec![0])]);
    assert_eq!(list(&click([0.4, 0.1], HitGranularity::Polygon)), [(0, vec![0, 1])]);
    assert_eq!(list(&click([0.6, 0.9], HitGranularity::UvIsland)), [(1, vec![0, 1])]);
    assert_eq!(list(&click([0.6, 0.9], HitGranularity::Mesh)), [(1, vec![0, 1])]);
    let rect = pick(&g, &hit(HitShape::Rect([1.0, 0.0], [0.3, 0.5]), HitGranularity::Triangle, StrokeView::Uv, [0.0; 16])).unwrap();
    assert_eq!(list(&rect), [(0, vec![0]), (1, vec![0, 1])]);
    let other = pick(&g, &hit(HitShape::Point([0.1, 0.1]), HitGranularity::Other(9), StrokeView::Uv, [0.0; 16]));
    assert!(matches!(other, Err(PickProblem::Unknown(_))));
}

#[test]
fn three_d_clicks_rectangles_and_symmetry() {
    let m = mesh();
    let g = SetGeometry::new(&m, 0, &[]);
    let click = pick(&g, &hit(HitShape::Point([0.75, 0.25]), HitGranularity::Triangle, StrokeView::Mesh, camera())).unwrap();
    assert_eq!(list(&click), [(1, vec![0])]);
    let rect = pick(&g, &hit(HitShape::Rect([0.0, 0.0], [0.45, 1.0]), HitGranularity::Triangle, StrokeView::Mesh, camera())).unwrap();
    assert_eq!(list(&rect), [(0, vec![0, 1])]);
    // mirrored at x = 1: the click on quad 1 also picks the mirror image in quad 0
    let mut h = hit(HitShape::Point([0.75, 0.25]), HitGranularity::Triangle, StrokeView::Mesh, camera());
    h.duplicate_count = 1;
    h.duplicate_matrix = MIRROR_X1;
    assert_eq!(pick(&g, &h).unwrap().keys().copied().collect::<Vec<_>>(), [0, 1]);
    // a camera that cannot be inverted
    let bad = pick(&g, &hit(HitShape::Point([0.5, 0.5]), HitGranularity::Triangle, StrokeView::Mesh, [0.0; 16]));
    assert_eq!(bad, Err(PickProblem::BadCamera));
}

#[test]
fn three_d_rectangles_follow_collision_mode_and_culling() {
    let m = stacked();
    let g = SetGeometry::new(&m, 0, &[POLYGONS.to_vec(), POLYGONS.to_vec()]);
    let rect = |a, b, gr, collision, occlusion_culling| {
        let mut h = hit(HitShape::Rect(a, b), gr, StrokeView::Mesh, camera());
        h.collision = collision;
        h.occlusion_culling = occlusion_culling;
        list(&pick(&g, &h).unwrap())
    };
    let all = vec![(0, vec![0, 1]), (1, vec![0, 1])];
    // older files (neither field): any part inside, hidden polygons too
    assert_eq!(rect([0.1, 0.1], [0.2, 0.2], HitGranularity::Polygon, None, None), all);
    // intersection with culling: only what the camera sees
    assert_eq!(rect([0.1, 0.1], [0.2, 0.2], HitGranularity::Polygon, Some(1), Some(true)), [(0, vec![0, 1])]);
    // centre test: the quads' centre (screen 0.25, 0.5) is outside a rectangle over their left fifth …
    assert_eq!(rect([0.0, 0.0], [0.1, 1.0], HitGranularity::Polygon, Some(0), None), []);
    assert_eq!(rect([0.0, 0.0], [0.3, 1.0], HitGranularity::Polygon, Some(0), None), all);
    assert_eq!(rect([0.0, 0.0], [0.3, 1.0], HitGranularity::Polygon, Some(0), Some(true)), [(0, vec![0, 1])]);
    // … while in triangle mode each triangle's own centre counts (triangle 1's is at x 1/3, screen 1/6)
    assert_eq!(rect([0.0, 0.0], [0.2, 1.0], HitGranularity::Triangle, Some(0), Some(true)), [(0, vec![1])]);
    // a rectangle of zero height picks nothing
    assert_eq!(rect([0.1, 0.5], [0.2, 0.5], HitGranularity::Polygon, None, None), []);
}

#[test]
fn uv_rectangles_with_the_centre_test_pick_units_whose_uv_centre_is_inside() {
    let m = mesh();
    let g = SetGeometry::new(&m, 0, &[POLYGONS.to_vec(), POLYGONS.to_vec()]);
    let rect = |a, b, gr, collision| {
        let mut h = hit(HitShape::Rect(a, b), gr, StrokeView::Uv, [0.0; 16]);
        h.collision = collision;
        list(&pick(&g, &h).unwrap())
    };
    // quad 0's UV centre is (0.25, 0.5)
    assert_eq!(rect([0.0, 0.0], [0.3, 0.6], HitGranularity::Polygon, Some(0)), [(0, vec![0, 1])]);
    assert_eq!(rect([0.0, 0.0], [0.2, 0.6], HitGranularity::Polygon, Some(0)), []);
    // the touching rule (intersection, and older files) picks it
    assert_eq!(rect([0.0, 0.0], [0.2, 0.6], HitGranularity::Polygon, Some(1)), [(0, vec![0, 1])]);
    assert_eq!(rect([0.0, 0.0], [0.2, 0.6], HitGranularity::Polygon, None), [(0, vec![0, 1])]);
    // triangle mode: triangle 1 (the upper-left half) has its centre at (1/6, 2/3), triangle 0 at (1/3, 1/3)
    assert_eq!(rect([0.0, 0.5], [0.2, 0.7], HitGranularity::Triangle, Some(0)), [(0, vec![1])]);
}

#[test]
fn culled_rectangles_also_pick_triangles_whose_centre_shows_elsewhere() {
    // the front quad over x 0..1, and behind it a quad over x 0.8..1.8 whose left edge hides behind the front one
    let m = quads(&[("front", quad(0.0, 0.0), 0.0), ("back", quad(0.8, -0.5), 0.5)]);
    let g = SetGeometry::new(&m, 0, &[]);
    // a rectangle over x 0.85..0.95, y 0.4..0.6 sees only the front quad; behind it the back quad's upper-left triangle
    // (centre at x 1.13, in view beside the front quad) overlaps it and is picked, its other triangle does not overlap,
    // and the front quad's upper-left triangle (outside the rectangle) is not picked
    let mut h = hit(HitShape::Rect([0.425, 0.4], [0.475, 0.6]), HitGranularity::Triangle, StrokeView::Mesh, camera());
    h.collision = Some(1);
    h.occlusion_culling = Some(true);
    assert_eq!(list(&pick(&g, &h).unwrap()), [(0, vec![0]), (1, vec![1])]);
}

#[test]
fn symmetry_copies_of_culled_rectangles_pick_the_mirror_image() {
    let m = mesh();
    let g = SetGeometry::new(&m, 0, &[POLYGONS.to_vec(), POLYGONS.to_vec()]);
    // a rectangle over quad 1 (x 1.2..1.4), mirrored at x = 1: the copy picks quad 0, whose own centre is seen
    let mut h = hit(HitShape::Rect([0.6, 0.2], [0.7, 0.4]), HitGranularity::Polygon, StrokeView::Mesh, camera());
    h.collision = Some(1);
    h.occlusion_culling = Some(true);
    h.duplicate_count = 1;
    h.duplicate_matrix = MIRROR_X1;
    assert_eq!(list(&pick(&g, &h).unwrap()), [(0, vec![0, 1]), (1, vec![0, 1])]);
    // with the centre test neither quad's centre (x 1.5, mirrored 0.5 -> 1.5) falls in the rectangle
    h.collision = Some(0);
    assert_eq!(list(&pick(&g, &h).unwrap()), []);
}

#[test]
fn hostile_hits_pick_nothing_or_fail() {
    let m = mesh();
    let g = SetGeometry::new(&m, 0, &[]);
    let mut h = hit(HitShape::Rect([f32::NAN, 0.0], [f32::INFINITY, 1.0]), HitGranularity::Mesh, StrokeView::Uv, [0.0; 16]);
    assert_eq!(list(&pick(&g, &h).unwrap()), []);
    h.view = StrokeView::Mesh;
    h.proj_view_inv = [f32::NAN; 16];
    assert_eq!(pick(&g, &h), Err(PickProblem::BadCamera));
    let mut sym = hit(HitShape::Point([0.75, 0.25]), HitGranularity::Triangle, StrokeView::Mesh, camera());
    sym.duplicate_count = u64::MAX;
    sym.duplicate_matrix = MIRROR_X1;
    assert!(pick(&g, &sym).is_ok(), "copies are capped");
    // a set without parts
    let empty = SetGeometry::new(&m, 7, &[]);
    assert_eq!(list(&pick(&empty, &hit(HitShape::Point([0.4, 0.1]), HitGranularity::Mesh, StrokeView::Uv, [0.0; 16])).unwrap()), []);
}
