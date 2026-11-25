//! Polygon fill with symmetry: the fill also reaches the faces the brush's mirror planes and radial copies map it onto.

use std::collections::{BTreeSet, HashSet};

use texturecraft_geom::glam::Mat3;

use crate::session::Session;
use crate::triangle_partition::TrianglePartition;

/// How far a mirrored face centre may be from the surface and still count as on it, as a fraction of the mesh's bounding-box diagonal
const MIRROR_TOLERANCE: f32 = 1e-3;

impl Session {
    /// `selected` plus the faces the brush symmetry copies it onto: for each group of `groups` (the fill mode's grouping) in `selected`, the group holding the face its first triangle's centre maps to under each mirror plane and radial turn.
    /// Copies that land on no face of the texture set (an asymmetric mesh, or the other side of a thin shell) are skipped
    pub(crate) fn with_symmetry_copies(&self, groups: &TrianglePartition, selected: BTreeSet<u32>) -> BTreeSet<u32> {
        let symmetry = self.tool.brush.sanitized().symmetry;
        let copies: Vec<Mat3> = symmetry.transforms().into_iter().filter(|copy| *copy != Mat3::IDENTITY).collect();
        if copies.is_empty() || selected.is_empty() {
            return selected;
        }

        let mesh = &self.mesh;
        let (low, high) = mesh.bounds();
        let tolerance = (high - low).length() * MIRROR_TOLERANCE;
        let origin = symmetry.origin((low + high) * 0.5);
        let corners = |triangle: u32| {
            let [first, second, third] = mesh.tri(triangle as usize)?;
            Some([*mesh.positions.get(first)?, *mesh.positions.get(second)?, *mesh.positions.get(third)?])
        };

        let mut seen_groups = HashSet::new();
        let mut result = selected.clone();
        for seed in selected.iter().copied().filter(|triangle| groups.index_of(*triangle).is_some_and(|group| seen_groups.insert(group))) {
            let Some([first, second, third]) = corners(seed) else { continue };
            let centre = (first + second + third) / 3.0;
            let normal = (second - first).cross(third - first);

            for copy in &copies {
                let target = origin + *copy * (centre - origin);
                let target_normal = *copy * normal;

                // A face on the same side: a thin shell's other face lies at the same point but looks the other way
                let faces_alike = |triangle: u32| {
                    groups.index_of(triangle).is_some() && corners(triangle).is_some_and(|[a, b, c]| (b - a).cross(c - a).dot(target_normal) > 0.0)
                };
                if let Some(hit) = self.bvh.nearest(target, tolerance, &faces_alike) {
                    result.extend(groups.group(hit.tri).unwrap_or_default().iter().copied());
                }
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use texturecraft_core::Channel;
    use texturecraft_geom::glam::{Vec2, Vec3};
    use texturecraft_geom::mesh::Mesh;

    use crate::fill_hit_test::FillHitTest;
    use crate::fill_mode::FillMode;
    use crate::session::{Session, Tool};

    /// The roughness the test fills paint, to tell filled texels apart
    const FILL_ROUGHNESS: f32 = 0.9;

    /// The x ranges of the two quads on the right of the YZ plane; the left side mirrors them
    const QUAD_RANGES: [(f32, f32); 2] = [(1.0, 2.0), (3.0, 4.0)];

    /// Texture coordinate of the point `(x, y)` on the flat test mesh, each x its own place
    fn uv_of(x: f32, y: f32) -> Vec2 {
        Vec2::new((x + 4.0) / 8.0, 0.2 + 0.6 * y)
    }

    /// Four quads facing +Z (two triangles each): [`QUAD_RANGES`] on the right and their mirror images on the left, each with its own vertices and UV place
    fn mirrored_quads() -> Mesh {
        let (mut positions, mut uvs, mut indices) = (Vec::new(), Vec::new(), Vec::new());
        for (low, high) in QUAD_RANGES {
            for mirrored in [false, true] {
                let sign = if mirrored { -1.0 } else { 1.0 };
                let first = positions.len() as u32;
                for (x, y) in [(low, 0.0), (high, 0.0), (high, 1.0), (low, 1.0)] {
                    positions.push(Vec3::new(sign * x, y, 0.0));
                    uvs.push(uv_of(sign * x, y));
                }

                // A mirror image has the opposite winding, so it keeps facing +Z
                let corners = if mirrored { [0, 2, 1, 0, 3, 2] } else { [0, 1, 2, 0, 2, 3] };
                indices.extend(corners.map(|corner| first + corner));
            }
        }
        Mesh::from_parts(positions, None, uvs, indices, "m").unwrap()
    }

    /// A fill session on [`mirrored_quads`] mirroring across X
    fn mirrored_session(fill_mode: FillMode) -> Session {
        let mut session = Session::new_project("t", mirrored_quads(), "test", None, 128).unwrap();
        session.tool.tool = Tool::PolygonFill;
        session.tool.fill_mode = fill_mode;
        session.tool.brush.symmetry.x = true;
        session.tool.values.insert(Channel::Roughness, [FILL_ROUGHNESS; 3]);
        session.update();
        session
    }

    /// Whether the texel at the point `(x, y)` of the flat test mesh shows the fill
    fn filled_at(session: &Session, x: f32, y: f32) -> bool {
        let uv = uv_of(x, y);
        let (width, height) = session.caches[0].size;
        let roughness = session.caches[0].composite.value(Channel::Roughness, (uv.x * width as f32) as u32, (uv.y * height as f32) as u32);
        (roughness[0] - FILL_ROUGHNESS).abs() < 0.01
    }

    /// Whether the quad over the x range starting at `low` (negative: its mirror image on the left) shows the fill
    fn quad_filled(session: &Session, low: f32) -> bool {
        filled_at(session, if low < 0.0 { low - 0.5 } else { low + 0.5 }, 0.5)
    }

    /// Which of the four quads show the fill: right near, left near, right far, left far
    fn filled_quads(session: &Session) -> [bool; 4] {
        [quad_filled(session, 1.0), quad_filled(session, -1.0), quad_filled(session, 3.0), quad_filled(session, -3.0)]
    }

    #[test]
    fn a_triangle_click_also_fills_the_mirrored_triangle() {
        let mut session = mirrored_session(FillMode::Triangle);

        // The lower right triangle of the near quad
        assert_eq!(session.polygon_fill_uv(uv_of(1.8, 0.2)).unwrap(), 2, "the triangle and its mirror image");
        session.update();

        assert!(filled_at(&session, 1.8, 0.2) && filled_at(&session, -1.8, 0.2), "the clicked triangle and the one across the plane");
        assert!(!filled_at(&session, 1.2, 0.8) && !filled_at(&session, -1.2, 0.8), "the other triangle of each quad stays");
        assert!(!quad_filled(&session, 3.0) && !quad_filled(&session, -3.0), "the far quads stay");
    }

    #[test]
    fn a_uv_island_click_fills_the_mirrored_island() {
        let mut session = mirrored_session(FillMode::UvIsland);

        assert_eq!(session.polygon_fill_uv(uv_of(1.8, 0.2)).unwrap(), 4, "both triangles of the quad and of its mirror image");
        session.update();

        assert_eq!(filled_quads(&session), [true, true, false, false]);
    }

    #[test]
    fn a_mesh_click_fills_the_mirrored_connected_part_only() {
        let mut session = mirrored_session(FillMode::Mesh);

        // Each quad is its own piece, as its vertices touch no other quad
        assert_eq!(session.polygon_fill_uv(uv_of(3.5, 0.5)).unwrap(), 4);
        session.update();

        assert_eq!(filled_quads(&session), [false, false, true, true]);
    }

    #[test]
    fn a_rectangle_fills_the_mirrored_side_too() {
        let mut session = mirrored_session(FillMode::UvIsland);
        session.tool.fill_hit_test = FillHitTest::ContainedCenter;

        // Around the far right quad only
        let filled = session.polygon_fill_box_uv(uv_of(2.9, -0.1), uv_of(4.1, 1.1)).unwrap();
        session.update();

        assert_eq!(filled, 4);
        assert_eq!(filled_quads(&session), [false, false, true, true]);
    }

    #[test]
    fn one_undo_takes_back_the_fill_and_its_mirror() {
        let mut session = mirrored_session(FillMode::UvIsland);
        let history = session.history_labels().len();

        session.polygon_fill_uv(uv_of(1.8, 0.2)).unwrap();
        session.update();
        assert_eq!(filled_quads(&session), [true, true, false, false], "the fill and its copy show");
        assert_eq!(session.history_labels().len(), history + 1, "one step for the fill and its copy");
        session.undo().unwrap();
        session.update();

        assert_eq!(filled_quads(&session), [false; 4]);
    }

    #[test]
    fn symmetry_off_fills_only_what_was_clicked() {
        let mut session = mirrored_session(FillMode::UvIsland);
        session.tool.brush.symmetry.enabled = false;

        assert_eq!(session.polygon_fill_uv(uv_of(1.8, 0.2)).unwrap(), 2);
        session.update();

        assert_eq!(filled_quads(&session), [true, false, false, false]);
    }

    #[test]
    fn faces_without_a_mirror_match_are_skipped() {
        let mut session = mirrored_session(FillMode::UvIsland);

        // Moving the plane to x = 10 sends every copy to x ~ 20, where there is no surface
        session.tool.brush.symmetry.offset = [10.0, 0.0, 0.0];
        assert_eq!(session.polygon_fill_uv(uv_of(1.8, 0.2)).unwrap(), 2, "only the clicked quad");
        session.update();

        assert_eq!(filled_quads(&session), [true, false, false, false]);
    }

    #[test]
    fn a_face_is_not_its_own_mirror_across_its_plane() {
        let mut session = mirrored_session(FillMode::Triangle);
        session.tool.brush.symmetry.x = false;
        session.tool.brush.symmetry.z = true;

        // The flat mesh lies in the mirror plane z = 0: each triangle maps onto itself looking the other way
        assert_eq!(session.polygon_fill_uv(uv_of(1.8, 0.2)).unwrap(), 1);
    }

    #[test]
    fn a_radial_copy_turns_the_fill_around_the_axis() {
        let mut session = mirrored_session(FillMode::UvIsland);

        // Half a turn around Z through the mesh centre (0, 0.5) sends the quad onto the quad on the left
        session.tool.brush.symmetry = texturecraft_paint::Symmetry { x: false, radial: 2, axis: 2, ..Default::default() };
        assert_eq!(session.polygon_fill_uv(uv_of(1.8, 0.2)).unwrap(), 4, "the turned copy lands on the left quad");
        session.update();
        assert_eq!(filled_quads(&session), [true, true, false, false]);
    }

    #[test]
    fn a_radial_copy_with_the_axis_moved_away_lands_on_no_face() {
        let mut session = mirrored_session(FillMode::UvIsland);

        session.tool.brush.symmetry = texturecraft_paint::Symmetry { x: false, radial: 2, axis: 2, offset: [0.0, 5.0, 0.0], ..Default::default() };
        assert_eq!(session.polygon_fill_uv(uv_of(1.8, 0.2)).unwrap(), 2, "the turned copy is far above the mesh");
    }
}
