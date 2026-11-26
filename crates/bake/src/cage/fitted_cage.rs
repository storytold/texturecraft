use super::name_keys::NameKeys;
use crate::map::interpolate;
use crate::{BakeRequest, HighPoly, MeshExtent};
use rayon::prelude::*;
use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use texturecraft_geom::Mesh;
use texturecraft_geom::glam::Vec3;

/// How much farther than the high poly the cage reaches, as a fraction of the fitted distance: the fit reads both meshes at their vertices only. Tuned with [`MARGIN_PER_DIAGONAL`] on two production meshes: the tightest pair whose rays miss no more texels than the default distances do
const MARGIN_PER_DISTANCE: f32 = 0.5;

/// The least the cage reaches each way, as a fraction of the mesh diagonal, so a high poly lying on the low poly is still inside
const MARGIN_PER_DIAGONAL: f32 = 4e-3;

/// How many times farther than the nearest high-poly point a low-poly vertex's ray may find the high poly: beyond it (a surface more than about 75 degrees off the ray) the ray has left the vertex's own part
const MAX_RAY_PER_NEAREST_DISTANCE: f32 = 4.0;

/// Bound temporary high-vertex projections while giving Rayon thousands of independent queries per batch
const PROJECTION_BATCH_VERTICES: usize = 8192;

/// The automatic cage: how far the transfer rays start in front of each low-poly vertex and how far they run behind it, fitted to the high poly
pub(crate) struct FittedCage {
    /// (front, rear) in world units per low-poly vertex; `None` on parts no high poly is near
    distances: Vec<Option<(f32, f32)>>,
}

impl FittedCage {
    /// Fit the cage of `request`'s low poly around `high` along `ray_normals` (where parts welded back to back cancelled a vertex's normal out, along its own): each low-poly vertex reaches the high poly straight in front of or behind it, and each high-poly vertex pushes the corners of its nearest low-poly triangle out (or in) to its height. Vertices neither finds take their neighbours' distances; a part the high poly reaches nowhere gets none
    pub(crate) fn new(request: &BakeRequest, high: HighPoly, welded_indices: &[u32], ray_normals: &[Vec3], extent: MeshExtent) -> FittedCage {
        let low = request.mesh;
        let keys = NameKeys::of(request, high);
        let welded_indices = welded_within_groups(welded_indices, &keys.low_vertices);
        let welded = |vertex: usize| welded_indices.get(vertex).copied().unwrap_or(0) as usize;
        let welded_count = welded_indices.iter().max().map_or(0, |last| *last as usize + 1);

        let fitting_normals = fitting_normals(low, ray_normals);
        let mut reach: Vec<Option<(f32, f32)>> = vec![None; welded_count];
        let mut widen = |vertex: usize, height: f32| {
            if let Some(distances) = reach.get_mut(welded(vertex)) {
                let (front, rear) = distances.unwrap_or((0.0, 0.0));
                *distances = Some((front.max(height), rear.max(-height)));
            }
        };

        for (vertex, height) in heights_at_low_vertices(low, high, &fitting_normals, &keys, extent.diagonal).into_iter().enumerate() {
            if let Some(height) = height {
                widen(vertex, height);
            }
        }
        let mut projected = Vec::new();
        for (batch, positions) in high.mesh.positions.chunks(PROJECTION_BATCH_VERTICES).enumerate() {
            let first = batch * PROJECTION_BATCH_VERTICES;
            positions
                .par_iter()
                .enumerate()
                .map(|(index, &position)| project_high_vertex(request, &fitting_normals, &keys, first + index, position))
                .collect_into_vec(&mut projected);

            for (triangle, height) in projected.drain(..).flatten() {
                for corner in low.tri(triangle).into_iter().flatten() {
                    widen(corner, height);
                }
            }
        }
        drop(projected);

        spread_to_neighbours(&mut reach, low.indices.as_chunks::<3>().0.iter().map(|corners| corners.map(|corner| welded(corner as usize))));

        let floor = MARGIN_PER_DIAGONAL * extent.diagonal;
        let padded = |distance: f32| distance * (1.0 + MARGIN_PER_DISTANCE) + floor;
        let distances =
            (0..low.positions.len()).map(|vertex| reach.get(welded(vertex)).copied().flatten().map(|(front, rear)| (padded(front), padded(rear)))).collect();

        FittedCage { distances }
    }

    /// The (front, rear) distances at `barycentric` of triangle `triangle` of the low poly `low` the cage was fitted to; `None` where the high poly reaches no corner
    pub(crate) fn distances(&self, low: &Mesh, triangle: u32, barycentric: [f32; 2]) -> Option<(f32, f32)> {
        let corners = low.tri(triangle as usize)?;
        let [first, second, third] = [0, 1, 2].map(|corner| self.distances.get(corners[corner]).copied().flatten());
        let ((first_front, first_rear), (second_front, second_rear), (third_front, third_rear)) = (first?, second?, third?);
        let [second_weight, third_weight] = barycentric;
        let first_weight = 1.0 - second_weight - third_weight;

        Some((
            first_front * first_weight + second_front * second_weight + third_front * third_weight,
            first_rear * first_weight + second_rear * second_weight + third_rear * third_weight,
        ))
    }
}

/// `welded_indices` with the vertices of different mesh-name groups (`vertex_keys`, empty when names don't matter) kept apart, so a group's distances come only from its own high poly and never spread into another group
fn welded_within_groups<'a>(welded_indices: &'a [u32], vertex_keys: &[u32]) -> Cow<'a, [u32]> {
    if vertex_keys.is_empty() {
        return Cow::Borrowed(welded_indices);
    }

    let mut indices: HashMap<(u32, Option<u32>), u32> = HashMap::new();
    welded_indices
        .iter()
        .enumerate()
        .map(|(vertex, &welded)| {
            let next = indices.len() as u32;
            *indices.entry((welded, vertex_keys.get(vertex).copied())).or_insert(next)
        })
        .collect()
}

/// `ray_normals`, except where parts welded back to back cancelled a vertex's normal out: there, the vertex's own normal, the direction its transfer rays fall back to
fn fitting_normals<'a>(low: &Mesh, ray_normals: &'a [Vec3]) -> Cow<'a, [Vec3]> {
    if !ray_normals.contains(&Vec3::ZERO) {
        return Cow::Borrowed(ray_normals);
    }

    ray_normals
        .iter()
        .enumerate()
        .map(|(vertex, &welded_normal)| {
            if welded_normal != Vec3::ZERO {
                return welded_normal;
            }

            low.normals.get(vertex).and_then(|normal| normal.try_normalize()).unwrap_or(welded_normal)
        })
        .collect()
}

/// Per low-poly vertex, how far along its ray normal the high-poly surface in front of it (positive) or behind it (negative) is. A ray that only finds a surface much farther than the nearest high-poly point (within `max_distance`) found another part, so the vertex takes that nearest point's height instead
fn heights_at_low_vertices(low: &Mesh, high: HighPoly, ray_normals: &[Vec3], keys: &NameKeys, max_distance: f32) -> Vec<Option<f32>> {
    low.positions
        .par_iter()
        .zip(ray_normals)
        .enumerate()
        .map(|(vertex, (&position, &up))| {
            let low_key = keys.low_vertices.get(vertex);
            let rays_reach = |triangle: u32| NameKeys::matches(low_key, keys.high_triangles.get(triangle as usize));
            let nearest = high.bvh.nearest(position, max_distance, &rays_reach)?;
            let nearest_position = interpolate(high.mesh, &high.mesh.positions, nearest.tri, [nearest.u, nearest.v])?;
            let ray_limit = nearest.t * MAX_RAY_PER_NEAREST_DISTANCE + MARGIN_PER_DIAGONAL * max_distance;
            let distance = |direction: Vec3| high.bvh.closest_filtered(position, direction, ray_limit, &rays_reach).map(|hit| hit.t);

            Some(match (distance(up), distance(-up)) {
                (Some(front), Some(rear)) if front <= rear => front,
                (Some(front), None) => front,
                (_, Some(rear)) => -rear,
                (None, None) => (nearest_position - position).dot(up),
            })
        })
        .collect()
}

/// The nearest matching low-poly triangle and signed height of one high-poly vertex
fn project_high_vertex(request: &BakeRequest, ray_normals: &[Vec3], keys: &NameKeys, vertex: usize, position: Vec3) -> Option<(usize, f32)> {
    let low = request.mesh;
    let high_key = keys.high_vertices.get(vertex);
    let rays_reach = |triangle: u32| NameKeys::matches(keys.low_triangles.get(triangle as usize), high_key);
    let nearest = request.bvh.nearest(position, f32::INFINITY, &rays_reach)?;
    let barycentric = [nearest.u, nearest.v];
    let low_position = interpolate(low, &low.positions, nearest.tri, barycentric)?;
    let up = interpolate(low, ray_normals, nearest.tri, barycentric)?.try_normalize()?;

    Some((nearest.tri as usize, (position - low_position).dot(up)))
}

/// Give each vertex without distances the largest of its nearest fitted neighbours', walking outward along the edges of `triangles`
fn spread_to_neighbours(reach: &mut [Option<(f32, f32)>], triangles: impl Iterator<Item = [usize; 3]>) {
    // Only unfitted vertices adjacent to a fitted region can gain distances
    if reach.iter().all(Option::is_some) || reach.iter().all(Option::is_none) {
        return;
    }

    let mut neighbours = vec![Vec::new(); reach.len()];
    for corners in triangles {
        for (corner, next) in [(0, 1), (1, 2), (2, 0)] {
            if let (true, Some(list)) = (corners[next] < reach.len(), neighbours.get_mut(corners[corner])) {
                list.push(corners[next]);
            }
            if let (true, Some(list)) = (corners[corner] < reach.len(), neighbours.get_mut(corners[next])) {
                list.push(corners[corner]);
            }
        }
    }

    // Propagate distances from the preceding breadth-first layer
    let mut steps: Vec<Option<u32>> = reach.iter().map(|distances| distances.map(|_| 0)).collect();
    let mut queue: VecDeque<usize> = (0..reach.len()).filter(|vertex| reach[*vertex].is_some()).collect();
    while let Some(vertex) = queue.pop_front() {
        let (Some((front, rear)), Some(step)) = (reach[vertex], steps[vertex]) else { continue };

        for &neighbour in &neighbours[vertex] {
            match steps[neighbour] {
                None => {
                    steps[neighbour] = Some(step + 1);
                    reach[neighbour] = Some((front, rear));
                    queue.push_back(neighbour);
                }
                Some(neighbour_step) if neighbour_step == step + 1 => {
                    let (neighbour_front, neighbour_rear) = reach[neighbour].unwrap_or((0.0, 0.0));
                    reach[neighbour] = Some((neighbour_front.max(front), neighbour_rear.max(rear)));
                }
                Some(_) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{spread_to_neighbours, welded_within_groups};

    #[test]
    fn coincident_vertices_of_different_name_groups_are_not_welded() {
        // The coincident pairs span different mesh-name groups
        assert_eq!(welded_within_groups(&[0, 0, 1, 1], &[5, 7, 7, 5]).as_ref(), [0, 1, 2, 3]);
        assert_eq!(welded_within_groups(&[0, 0, 1, 1], &[5, 5, 7, 7]).as_ref(), [0, 0, 1, 1], "same group: still welded");
        assert_eq!(welded_within_groups(&[0, 0, 1], &[]).as_ref(), [0, 0, 1], "without name matching nothing changes");
    }

    #[test]
    fn unfitted_vertices_take_the_largest_distances_of_their_nearest_fitted_neighbours() {
        // A strip of triangles 0-1-2, 1-2-3, 2-3-4: vertices 0 and 4 are fitted
        let mut reach = vec![Some((1.0, 0.5)), None, None, None, Some((3.0, 0.25))];

        spread_to_neighbours(&mut reach, [[0, 1, 2], [1, 2, 3], [2, 3, 4]].into_iter());

        assert_eq!(reach[1], Some((1.0, 0.5)), "one step from vertex 0 only");
        assert_eq!(reach[3], Some((3.0, 0.25)), "one step from vertex 4 only");
        assert_eq!(reach[2], Some((3.0, 0.5)), "one step from both: the larger of each");
    }
}
