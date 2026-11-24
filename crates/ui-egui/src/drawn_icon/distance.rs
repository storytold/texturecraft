//! Signed distances to the simple shapes the drawn icons are built from, in the icon's unit square (y down), negative inside

/// A point or a direction in the icon's unit square
pub(crate) type Point = [f32; 2];

/// The distance from `point` to a convex polygon with `corners` in either order; squared off past its corners, so outlines stay sharp
pub(crate) fn convex_polygon(point: Point, corners: &[Point]) -> f32 {
    let Some(&first) = corners.first() else { return f32::INFINITY };

    let count = corners.len() as f32;
    let centre = corners.iter().fold([0.0, 0.0], |sum, corner| [sum[0] + corner[0] / count, sum[1] + corner[1] / count]);
    let next = corners.iter().skip(1).chain(std::iter::once(&first));

    let mut edge_count = 0;
    let mut greatest_distance = f32::NEG_INFINITY;
    for (from, to) in corners.iter().zip(next) {
        let edge = sub(*to, *from);
        let length = edge[0].hypot(edge[1]);

        // Repeated corners contribute no edge
        if length > f32::EPSILON {
            let normal = [edge[1] / length, -edge[0] / length];
            let side = dot(sub(point, *from), normal);
            let signed = if dot(sub(centre, *from), normal) > 0.0 { -side } else { side };
            greatest_distance = greatest_distance.max(signed);
            edge_count += 1;
        }
    }

    if edge_count < POLYGON_MIN_EDGES { f32::INFINITY } else { greatest_distance }
}

/// Fewer edges than a triangle's enclose nothing
const POLYGON_MIN_EDGES: usize = 3;

/// The distance from `point` to the axis-aligned rectangle from `min` to `max`
pub(crate) fn rectangle(point: Point, min: Point, max: Point) -> f32 {
    convex_polygon(point, &[min, [max[0], min[1]], max, [min[0], max[1]]])
}

/// The distance from `point` to the disc around `centre`
pub(crate) fn disc(point: Point, centre: Point, radius: f32) -> f32 {
    let offset = sub(point, centre);
    offset[0].hypot(offset[1]) - radius
}

/// `from` minus `to`, per axis
pub(crate) fn sub(from: Point, to: Point) -> Point {
    [from[0] - to[0], from[1] - to[1]]
}

/// The dot product of `a` and `b`
pub(crate) fn dot(a: Point, b: Point) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rectangle_is_negative_inside_and_its_edge_distance_outside() {
        assert!((rectangle([0.5, 0.5], [0.0, 0.0], [1.0, 1.0]) + 0.5).abs() < 1e-6, "the centre is half a side in");
        assert!((rectangle([1.25, 0.5], [0.0, 0.0], [1.0, 1.0]) - 0.25).abs() < 1e-6, "beside an edge");
        let reversed = convex_polygon([1.25, 0.5], &[[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]);
        assert!((reversed - 0.25).abs() < 1e-6, "either winding order");
        assert_eq!(convex_polygon([0.0, 0.0], &[]), f32::INFINITY, "no corners, no ink");
        assert!((disc([0.0, 2.0], [0.0, 0.0], 1.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_corner_list_that_closes_on_its_first_corner_draws_the_same_polygon() {
        let open = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let closed = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
        for point in [[0.5, 0.5], [0.25, 0.75], [1.25, 0.5]] {
            assert!((convex_polygon(point, &closed) - convex_polygon(point, &open)).abs() < 1e-6, "{point:?}");
        }
        assert_eq!(convex_polygon([0.0, 0.0], &[[0.0, 0.0], [1.0, 0.0], [0.0, 0.0]]), f32::INFINITY, "a line encloses no ink");
    }
}
