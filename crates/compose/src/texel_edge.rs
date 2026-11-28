//! A UV seam side in texel space

use texturecraft_geom::UvSeamSide;
use texturecraft_geom::glam::Vec2;

/// A seam side in texel space, with its unit normal into the island
pub(crate) struct TexelEdge {
    pub(crate) start: Vec2,
    pub(crate) end: Vec2,
    pub(crate) inward: Vec2,
}

impl TexelEdge {
    pub(crate) fn new(side: UvSeamSide, size: Vec2) -> Option<TexelEdge> {
        let (start, end, interior) = (side.start * size, side.end * size, side.interior * size);
        let normal = (end - start).perp().try_normalize()?;
        let inward = if normal.dot(interior - start) < 0.0 { -normal } else { normal };
        Some(TexelEdge { start, end, inward })
    }

    /// How far along the edge (0..1) and how far outside the island `point` is, when it is beside the edge on the outside
    pub(crate) fn outside(&self, point: Vec2) -> Option<(f32, f32)> {
        let direction = self.end - self.start;
        let along = (point - self.start).dot(direction) / direction.length_squared();
        let outside = -(point - self.start).dot(self.inward);
        ((0.0..=1.0).contains(&along) && outside > 0.0).then_some((along, outside))
    }

    /// The x range at height `y` of the band `reach` deep outside the edge: every point there that `outside` takes within `reach`
    pub(crate) fn band_span(&self, reach: f32, y: f32) -> Option<(f32, f32)> {
        let depth = self.inward * -reach;
        let corners = [self.start, self.end, self.end + depth, self.start + depth];
        let mut span: Option<(f32, f32)> = None;
        for (index, &from) in corners.iter().enumerate() {
            let to = corners[(index + 1) % corners.len()];
            if y < from.y.min(to.y) || y > from.y.max(to.y) {
                continue;
            }

            // A level side spans its whole length at its height
            let xs = if from.y == to.y { [from.x, to.x] } else { [from.x + (to.x - from.x) * (y - from.y) / (to.y - from.y); 2] };
            for x in xs {
                span = Some(span.map_or((x, x), |(low, high)| (low.min(x), high.max(x))));
            }
        }
        span
    }
}

#[cfg(test)]
mod tests {
    use texturecraft_geom::UvSeamSide;
    use texturecraft_geom::glam::Vec2;

    use super::TexelEdge;

    #[test]
    fn a_diagonal_edges_band_spans_every_point_beside_it_and_little_else() {
        let size = Vec2::splat(512.0);
        let side = UvSeamSide { triangle: 0, start: Vec2::new(0.05, 0.05), end: Vec2::new(0.95, 0.9), interior: Vec2::new(0.9, 0.1) };
        let edge = TexelEdge::new(side, size).unwrap();
        let reach = 32.0;

        let (mut beside, mut spanned) = (0, 0);
        for y in 0..512 {
            let centre_y = y as f32 + 0.5;
            let span = edge.band_span(reach, centre_y);
            spanned += span.map_or(0, |(low, high)| (high - low).ceil() as usize + 1);
            for x in 0..512 {
                let point = Vec2::new(x as f32 + 0.5, centre_y);
                if edge.outside(point).is_some_and(|(_, outside)| outside <= reach) {
                    beside += 1;
                    let (low, high) = span.unwrap_or((f32::MAX, f32::MIN));
                    assert!((low..=high).contains(&point.x), "{point} is beside the edge but outside its row's span {span:?}");
                }
            }
        }

        // The diagonal's bounding box holds far more texels than its band
        assert!(beside > 0 && spanned < 512 * 512 / 4, "{spanned} texels spanned for {beside} beside the edge");
    }
}
