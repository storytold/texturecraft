//! The depth map of a planar or warp projection: the surface nearest its projector across its view, for depth culling

use std::sync::Arc;

use texturecraft_core::Projection;
use texturecraft_geom::Mesh;
use texturecraft_geom::glam::{Vec2, Vec3};

use crate::fill_copy::FillCopy;
use crate::organic::smoothstep;
use crate::projector_view::ProjectorView;

/// The nearest surface in each cell of a grid across a projector's view
pub struct ProjectorDepth {
    view: ProjectorView,
    /// The view point at the grid's lower corner
    origin: Vec2,
    grid_side: usize,
    /// One cell's size across and up in the view
    cell: Vec2,
    /// The mesh radius measured along the view's depth axis
    nearness_unit: f32,
    /// Per cell: the nearness of the nearest surface at the cell centre and its slope across and up (nearness -inf where no surface is)
    front: Vec<[f32; 3]>,
}

/// The smallest grid cell across and up, so a flat view still divides
const MIN_CELL_SIZE: f32 = 1e-6;

/// A cell centre this close outside a triangle's edge (as a share of its area) still counts as inside, so shared edges leave no gaps
const EDGE_TOLERANCE: f32 = 1e-6;

impl ProjectorDepth {
    /// Build the depth map of a borrowed mesh, if the projection uses depth culling.
    pub fn of(mesh: &Mesh, bounds: (Vec3, Vec3), projection: &Projection) -> Option<Arc<ProjectorDepth>> {
        let view = ProjectorView::of(projection, bounds)?;
        Self::build(view, mesh).map(Arc::new)
    }

    /// Rasterize the mesh in this view, using a coarser grid when the cell budget requires one.
    pub fn build(view: ProjectorView, mesh: &Mesh) -> Option<ProjectorDepth> {
        Self::build_for_copy(view, mesh, None, GRID_SIDE, CELL_BUDGET)
    }

    pub(crate) fn build_for_copy(view: ProjectorView, mesh: &Mesh, copy: Option<FillCopy>, mut grid_side: usize, cell_budget: usize) -> Option<Self> {
        let projected: Vec<_> = mesh.positions.iter().map(|&point| view.local(copy.map_or(point, |copy| copy.point(point)))).collect();
        let (min, max) = projected
            .iter()
            .flatten()
            .fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), view_point| (lo.min(view_point.truncate()), hi.max(view_point.truncate())));
        if min.x > max.x || min.y > max.y {
            return None;
        }

        // The one-cell fallback can always afford one visit per triangle
        let cell_budget = cell_budget.max(mesh.triangle_count());
        let nearness_unit = view.unit();
        while grid_side > 0 {
            let cell = ((max - min) / grid_side as f32).max(Vec2::splat(MIN_CELL_SIZE));
            let front = vec![[f32::NEG_INFINITY, 0.0, 0.0]; grid_side * grid_side];
            let mut depth = ProjectorDepth { view, origin: min, grid_side, cell, nearness_unit, front };
            let mut remaining = cell_budget;
            let mut triangles = mesh.indices.as_chunks::<3>().0.iter().filter_map(|indices| {
                let [a, b, c] = indices.map(|index| projected.get(index as usize).copied().flatten());
                Some([a?, b?, c?])
            });

            if triangles.all(|triangle| depth.raster(triangle, &mut remaining)) {
                return Some(depth);
            }

            // Discard any partial grid before retrying the whole mesh
            grid_side /= 2;
        }

        None
    }

    /// Keep `triangle` where it is nearer than what a cell holds; false once the cell budget runs out
    fn raster(&mut self, triangle: [Vec3; 3], budget: &mut usize) -> bool {
        let [a, b, c] = triangle;
        let (edge_ab, edge_ac) = (b - a, c - a);
        let determinant = edge_ab.x * edge_ac.y - edge_ab.y * edge_ac.x;
        if !determinant.is_finite() || determinant.abs() < MIN_SCREEN_AREA {
            // An edge-on triangle covers no cells
            return true;
        }

        let slope = Vec2::new((edge_ab.z * edge_ac.y - edge_ac.z * edge_ab.y) / determinant, (edge_ab.x * edge_ac.z - edge_ac.x * edge_ab.z) / determinant)
            .clamp(Vec2::splat(-MAX_SLOPE), Vec2::splat(MAX_SLOPE));

        // Evaluate coverage at cell centres
        let [pa, pb, pc] = triangle.map(|corner| (corner.truncate() - self.origin) / self.cell);
        let (lo, hi) = (pa.min(pb).min(pc), pa.max(pb).max(pc));
        let first_cell = |position: f32| (position - 0.5).ceil().clamp(0.0, self.grid_side as f32) as usize;
        let end_cell = |position: f32| ((position - 0.5).floor() + 1.0).clamp(0.0, self.grid_side as f32) as usize;
        let (x0, x1, y0, y1) = (first_cell(lo.x), end_cell(hi.x), first_cell(lo.y), end_cell(hi.y));
        let cells = x1.saturating_sub(x0).saturating_mul(y1.saturating_sub(y0));
        let Some(left) = budget.checked_sub(cells) else { return false };
        *budget = left;

        let area = edge(pa, pb, pc);
        for y in y0..y1 {
            for x in x0..x1 {
                let cell_centre = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let inside = [edge(pb, pc, cell_centre), edge(pc, pa, cell_centre), edge(pa, pb, cell_centre)]
                    .iter()
                    .all(|weight| weight * area.signum() >= -EDGE_TOLERANCE * area.abs());
                if !inside {
                    continue;
                }

                let near = a.z + slope.dot(self.origin + cell_centre * self.cell - a.truncate());
                if let Some(cell) = self.front.get_mut(y * self.grid_side + x)
                    && near > cell[0]
                {
                    *cell = [near, slope.x, slope.y];
                }
            }
        }

        true
    }

    /// How much depth culling with `hardness` leaves of a projection at world point `p`: 0 behind the surface nearest the projector, 1 on it or where nothing is in front
    pub fn cover(&self, position: Vec3, hardness: f32) -> f32 {
        // Keep the layer visible when a hand-edited hardness is invalid
        if !hardness.is_finite() {
            return 1.0;
        }

        let Some(view_point) = self.view.local(position) else { return 1.0 };
        let at = (view_point.truncate() - self.origin) / self.cell;
        if !(at.x >= 0.0 && at.y >= 0.0 && at.x <= self.grid_side as f32 && at.y <= self.grid_side as f32) {
            return 1.0;
        }

        let (x, y) = ((at.x as usize).min(self.grid_side - 1), (at.y as usize).min(self.grid_side - 1));
        let Some(&[near, across, up]) = self.front.get(y * self.grid_side + x).filter(|cell| cell[0].is_finite()) else { return 1.0 };

        // Extend the cell's plane to avoid culling a surface against itself
        let centre = self.origin + (Vec2::new(x as f32, y as f32) + 0.5) * self.cell;
        let slope = Vec2::new(across, up);
        let behind = near + slope.dot(view_point.truncate() - centre) - view_point.z;
        let unit = self.nearness_unit;

        // Add slope bias for curvature across adjacent triangles
        let bias = BIAS * unit + (slope.abs() * self.cell).element_sum().min(MAX_SLOPE_BIAS * unit);
        let width = (1.0 - hardness.clamp(0.0, 1.0)) * FADE * unit;
        if width <= f32::EPSILON {
            return f32::from(u8::from(behind <= bias));
        }

        1.0 - smoothstep(bias, bias + width, behind)
    }
}

/// Twice the signed area of triangle `a`, `b`, `q`
fn edge(a: Vec2, b: Vec2, q: Vec2) -> f32 {
    (b - a).perp_dot(q - a)
}

/// Cells across and up the grid
pub(crate) const GRID_SIDE: usize = 512;

/// A triangle flatter than this on screen is seen edge on
const MIN_SCREEN_AREA: f32 = 1e-12;

/// Cell visits available at each resolution before large overlapping triangles require a coarser grid.
pub(crate) const CELL_BUDGET: usize = 256 * GRID_SIDE * GRID_SIDE;

/// Steepest surface slope kept, in nearness per view unit
const MAX_SLOPE: f32 = 1e4;

/// How far behind the nearest surface a point may sit and keep everything, in mesh radii
const BIAS: f32 = 0.002;

/// Most the slope adds to the bias, in mesh radii
const MAX_SLOPE_BIAS: f32 = 0.05;

/// How far behind the nearest surface culling fades out at hardness 0, in mesh radii
const FADE: f32 = 0.1;

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_core::{Projector, UvTransform};

    /// Two squares facing +Z, one at z = `front` covering x 0..1, one at z = `back` covering x -1..1
    fn two_squares(front: f32, back: f32) -> Mesh {
        let positions = vec![
            Vec3::new(0.0, -1.0, front),
            Vec3::new(1.0, -1.0, front),
            Vec3::new(1.0, 1.0, front),
            Vec3::new(0.0, 1.0, front),
            Vec3::new(-1.0, -1.0, back),
            Vec3::new(1.0, -1.0, back),
            Vec3::new(1.0, 1.0, back),
            Vec3::new(-1.0, 1.0, back),
        ];
        Mesh { positions, indices: vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7], ..Mesh::default() }
    }

    fn planar_z(on: bool, hardness: f32) -> Projection {
        let mut projector = Projector::depth_culled();
        projector.depth.on = on;
        projector.depth.hardness = hardness;
        Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3], uv: UvTransform::default(), projector, placed: None }
    }

    #[test]
    fn surfaces_behind_others_are_culled_and_the_nearest_keeps_everything() {
        let mesh = two_squares(0.5, -0.5);
        let bounds = mesh.bounds();
        let depth = ProjectorDepth::build(ProjectorView::of(&planar_z(true, 1.0), bounds).unwrap(), &mesh).unwrap();

        assert_eq!(depth.cover(Vec3::new(0.5, 0.3, 0.5), 1.0), 1.0, "the front square");
        assert_eq!(depth.cover(Vec3::new(0.5, 0.3, -0.5), 1.0), 0.0, "the back square behind it");
        assert_eq!(depth.cover(Vec3::new(-0.5, 0.3, -0.5), 1.0), 1.0, "the back square where nothing is in front");
        let soft = depth.cover(Vec3::new(0.5, 0.3, 0.5 - 0.01), 0.0);
        assert!(soft > 0.0 && soft < 1.0, "a soft cull fades just behind the front: {soft}");
        assert!(ProjectorView::of(&planar_z(false, 1.0), bounds).is_none(), "off, there is nothing to build");
    }

    #[test]
    fn folding_positive_depth_stretch_preserves_soft_culling() {
        let mesh = two_squares(0.5, -0.5);
        let bounds = mesh.bounds();
        let mut projector = Projector::depth_culled();
        projector.scale[2] = 2.0;
        projector.depth.hardness = 0.0;
        let stretched = Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3], uv: UvTransform::default(), projector, placed: None };
        let placed = crate::axis_frame::placed_from_axis(&stretched, bounds).unwrap();
        let axis_depth = ProjectorDepth::of(&mesh, bounds, &stretched).unwrap();
        let placed_depth = ProjectorDepth::of(&mesh, bounds, &placed).unwrap();
        let just_behind = Vec3::new(0.5, 0.3, 0.44);

        let before = axis_depth.cover(just_behind, 0.0);
        let after = placed_depth.cover(just_behind, 0.0);

        assert!(before > 0.0 && before < 1.0 && (before - after).abs() < 1e-4, "axis {before}, placed {after}");
    }

    #[test]
    fn the_upper_right_grid_boundary_can_be_depth_culled() {
        let mesh = two_squares(0.5, -0.5);
        let depth = ProjectorDepth::of(&mesh, mesh.bounds(), &planar_z(true, 1.0)).unwrap();

        assert_eq!(depth.cover(Vec3::new(1.0, 1.0, -0.5), 1.0), 0.0);
    }

    #[test]
    fn raster_budget_fallback_accounts_for_the_last_triangle() {
        let mut mesh = two_squares(0.5, -0.5);
        let rear_triangles = [4, 5, 6, 4, 6, 7];
        let front_triangles = [0, 1, 2, 0, 2, 3];
        mesh.indices = rear_triangles.repeat(CELL_BUDGET / (GRID_SIDE * GRID_SIDE));
        mesh.indices.extend(front_triangles);
        let projection = planar_z(true, 1.0);
        let behind = Vec3::new(0.5, 0.3, -0.5);

        let front_last = ProjectorDepth::of(&mesh, mesh.bounds(), &projection).unwrap();
        mesh.indices.rotate_right(front_triangles.len());
        let front_first = ProjectorDepth::of(&mesh, mesh.bounds(), &projection).unwrap();

        assert_eq!(front_last.cover(behind, 1.0), 0.0);
        assert_eq!(front_last.cover(behind, 1.0), front_first.cover(behind, 1.0));
    }

    #[test]
    fn a_nan_hardness_culls_nothing() {
        let mesh = two_squares(0.5, -0.5);
        let depth = ProjectorDepth::build(ProjectorView::of(&planar_z(true, 1.0), mesh.bounds()).unwrap(), &mesh).unwrap();

        let behind = depth.cover(Vec3::new(0.5, 0.3, -0.5), f32::NAN);

        assert_eq!(behind, 1.0, "a hand-edited NaN hardness keeps the back square visible");
    }

    #[test]
    fn interleaved_meshes_keep_their_own_projector_depth() {
        let mut mesh = two_squares(0.5, -0.5);

        // Add enough unused vertices for the old sample to skip the changed edge
        mesh.positions.resize(200, Vec3::ZERO);
        let mut edited = mesh.clone();
        for right in [1, 2] {
            edited.positions[right].x = 0.5;
        }
        assert_eq!(mesh.bounds(), edited.bounds());

        let proj = planar_z(true, 1.0);
        let behind_the_cut = Vec3::new(0.75, 0.3, -0.5);
        let bounds = mesh.bounds();
        assert_eq!(ProjectorDepth::of(&mesh, bounds, &proj).unwrap().cover(behind_the_cut, 1.0), 0.0);

        let rebuilt = ProjectorDepth::of(&edited, bounds, &proj).unwrap();
        assert_eq!(rebuilt.cover(behind_the_cut, 1.0), 1.0, "nothing is in front there any more");
    }

    #[test]
    fn a_slanted_surface_is_never_behind_itself() {
        let positions = vec![Vec3::new(-1.0, -1.0, -0.9), Vec3::new(1.0, -1.0, 0.9), Vec3::new(1.0, 1.0, 0.9), Vec3::new(-1.0, 1.0, -0.9)];
        let mesh = Mesh { positions, indices: vec![0, 1, 2, 0, 2, 3], ..Mesh::default() };
        let depth = ProjectorDepth::build(ProjectorView::of(&planar_z(true, 1.0), mesh.bounds()).unwrap(), &mesh).unwrap();
        for i in 0..1000 {
            let (x, y) = ((i as f32 * 0.618).fract() * 1.98 - 0.99, (i as f32 * 0.377).fract() * 1.98 - 0.99);

            assert_eq!(depth.cover(Vec3::new(x, y, x * 0.9), 1.0), 1.0, "{x} {y}");
        }
    }
}
