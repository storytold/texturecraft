use rayon::prelude::*;
use texturecraft_bake::HighPolyReach;

/// The value [`high_poly_missed`] gives a vertex that `only_set` doesn't own, which the 3D view leaves out
pub const HIDDEN_HIGH_POLY_VERTEX: f32 = -1.0;

/// Per vertex of the validated high poly: 1.0 if its owning set's bake misses it at that set's (max frontal, max rear), 0.0 if reached or unknown, [`HIDDEN_HIGH_POLY_VERTEX`] if `only_set` doesn't own it
pub fn high_poly_missed(reach: &[Option<HighPolyReach>], distances: &[(f32, f32)], only_set: Option<usize>, vertex_count: usize) -> Vec<f32> {
    // Ownership is already assigned by HighPolyReach; other sets must not color the vertex
    let sets: Vec<(&HighPolyReach, f32, f32)> = reach
        .iter()
        .zip(distances)
        .enumerate()
        .filter(|(texture_set, _)| only_set.is_none_or(|only| only == *texture_set))
        .filter_map(|(_, (reach, &(max_frontal, max_rear)))| Some((reach.as_ref()?, max_frontal, max_rear)))
        .collect();

    (0..vertex_count)
        .into_par_iter()
        .map(|vertex| {
            if only_set.is_some_and(|only| !owns(reach, only, vertex)) {
                return HIDDEN_HIGH_POLY_VERTEX;
            }

            let missed = sets.iter().find_map(|(reach, max_frontal, max_rear)| reach.missed(vertex, *max_frontal, *max_rear));
            f32::from(u8::from(missed.unwrap_or(false)))
        })
        .collect()
}

/// Whether texture set `texture_set`'s reach claims high-poly vertex `vertex`
fn owns(reach: &[Option<HighPolyReach>], texture_set: usize, vertex: usize) -> bool {
    reach.get(texture_set).and_then(Option::as_ref).and_then(|set| set.vertices.get(vertex)).is_some_and(Option::is_some)
}
