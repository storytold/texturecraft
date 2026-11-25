//! The copies a symmetric fill makes of its projection, as mirrors and turns of the surface point each copy samples

use texturecraft_core::{FULL_TURN_DEGREES, FillSymmetry, SymmetryKind};
use texturecraft_geom::glam::{Mat3, Vec3};

use crate::fill_copy::FillCopy;

/// The copies of a fill's projection one texel shows, the original among them: the one whose half (mirror) or wedge (radial) holds the texel comes first, so it lies on top where copies overlap
pub struct FillCopies {
    symmetry: FillSymmetry,
    pivot: Vec3,
    axis: usize,
    step: f32,
    first: u32,
    count: u32,
    next: u32,
}

impl FillCopies {
    /// The copies `symmetry` makes for a texel at world point `point`, on a mesh whose bounds have centre `center` and radius `radius`
    pub fn new(symmetry: &FillSymmetry, point: Vec3, center: Vec3, radius: f32) -> FillCopies {
        let reach = FillSymmetry::REACH;
        let position = Vec3::from(symmetry.position.map(|coordinate| if coordinate.is_finite() { coordinate.clamp(-reach, reach) } else { 0.0 }));
        let pivot = center + position * radius;
        let axis = symmetry.axis_index();
        let count = symmetry.copies();
        let step = symmetry.step();

        let from_pivot = point - pivot;
        let first = match symmetry.kind {
            // The original keeps the half on the axis's positive side
            SymmetryKind::Mirror => u32::from(from_pivot[axis] < 0.0),
            SymmetryKind::Radial => {
                let turned = from_pivot[(axis + 2) % 3].atan2(from_pivot[(axis + 1) % 3]).to_degrees().rem_euclid(FULL_TURN_DEGREES);
                nearest_copy(turned, step, count, symmetry.span)
            }
        };

        FillCopies { symmetry: *symmetry, pivot, axis, step, first, count, next: 0 }
    }

    /// Copy `index`: the original at 0, then the mirror or the radial copies in turn
    pub(crate) fn copy(&self, index: u32) -> FillCopy {
        let flip = if index == 0 { [false; 2] } else { self.symmetry.flip };
        let turn = match self.symmetry.kind {
            _ if index == 0 => Mat3::IDENTITY,
            SymmetryKind::Mirror => {
                let mut scale = Vec3::ONE;
                scale[self.axis] = -1.0;
                Mat3::from_diagonal(scale)
            }

            // Copy `index` is the original turned by `index` steps, so a texel samples the original turned back
            SymmetryKind::Radial => Mat3::from_axis_angle(Vec3::AXES[self.axis], -(index as f32 * self.step).to_radians()),
        };
        FillCopy::new(turn, self.pivot, flip)
    }

    pub(crate) fn next_indexed(&mut self) -> Option<(usize, FillCopy)> {
        if self.next >= self.count {
            return None;
        }

        let index = (self.first + self.next) % self.count;
        self.next += 1;
        Some((index as usize, self.copy(index)))
    }
}

/// The radial copy nearest a texel `turned` degrees round the axis, of `count` copies `step` degrees apart over `span` degrees: copy 0 across the gap a span under a full turn leaves, else the nearest by its step
fn nearest_copy(turned: f32, step: f32, count: u32, span: f32) -> u32 {
    // A span of nothing (or no step at all) stacks every copy on the original
    if step.is_nan() || step <= 0.0 {
        return 0;
    }

    let by_step = (turned / step).round() as u32;
    if span >= FULL_TURN_DEGREES || span.is_nan() {
        return by_step % count;
    }

    let last = by_step.min(count - 1);
    let past_last = (turned - last as f32 * step).abs();
    let before_first = FULL_TURN_DEGREES - turned;
    if before_first <= past_last { 0 } else { last }
}

impl Iterator for FillCopies {
    type Item = FillCopy;

    fn next(&mut self) -> Option<FillCopy> {
        self.next_indexed().map(|(_, copy)| copy)
    }
}

#[cfg(test)]
mod tests {
    use texturecraft_geom::glam::Vec2;

    use super::*;

    fn symmetry(kind: SymmetryKind, axis: u8) -> FillSymmetry {
        FillSymmetry { on: true, kind, axis, ..FillSymmetry::OFF }
    }

    #[test]
    fn a_mirror_puts_the_texels_own_half_first_and_reflects_across_the_axis_position() {
        let mut mirror = symmetry(SymmetryKind::Mirror, 0);
        mirror.position = [0.5, 0.0, 0.0];
        let left: Vec<FillCopy> = FillCopies::new(&mirror, Vec3::new(-1.0, 0.0, 0.0), Vec3::ZERO, 2.0).collect();

        assert_eq!(left.len(), 2);

        // The plane passes at x = 1 (half a radius of 2): a point at -1 sees the original at 3 first
        assert!(left[0].point(Vec3::new(-1.0, 0.2, 0.0)).abs_diff_eq(Vec3::new(3.0, 0.2, 0.0), 1e-5));
        assert!(left[1].point(Vec3::new(-1.0, 0.2, 0.0)).abs_diff_eq(Vec3::new(-1.0, 0.2, 0.0), 1e-5));
        assert!(left[0].normal(Vec3::X).abs_diff_eq(-Vec3::X, 1e-5));

        let right: Vec<FillCopy> = FillCopies::new(&mirror, Vec3::new(2.0, 0.0, 0.0), Vec3::ZERO, 2.0).collect();

        assert!(right[0].point(Vec3::new(2.0, 0.3, 0.0)).abs_diff_eq(Vec3::new(2.0, 0.3, 0.0), 1e-5));
    }

    #[test]
    fn radial_copies_turn_by_their_step_and_flip_only_the_copies() {
        let mut radial = symmetry(SymmetryKind::Radial, 2);
        radial.count = 4;
        radial.flip = [true, false];
        let copies: Vec<FillCopy> = FillCopies::new(&radial, Vec3::X, Vec3::ZERO, 1.0).collect();

        assert_eq!(copies.len(), 4);
        assert_eq!(copies[0].uv(Vec2::new(0.25, 0.5)), Vec2::new(0.25, 0.5));
        assert_eq!(copies[1].uv(Vec2::new(0.25, 0.5)), Vec2::new(0.75, 0.5));

        // Copy 1 sits a quarter turn on: a texel on +Y samples the original on +X
        assert!(copies[1].point(Vec3::Y).abs_diff_eq(Vec3::X, 1e-5));

        // A texel on +Y lies in copy 1's wedge, so copy 1 comes first there
        let on_y: Vec<FillCopy> = FillCopies::new(&radial, Vec3::Y, Vec3::ZERO, 1.0).collect();

        assert!(on_y[0].point(Vec3::Y).abs_diff_eq(Vec3::X, 1e-5));
    }

    #[test]
    fn the_nearest_copy_is_found_by_its_step_as_a_search_of_every_copy_finds_it() {
        for (count, span) in [(6, 360.0), (5, 360.0), (4, 180.0), (3, 90.0), (64, 300.0), (2, 1.0)] {
            let step = radial_step(count, span);
            for tenth in 0..3600 {
                let turned = tenth as f32 / 10.0 + 0.05;
                let gap = |index: u32| {
                    let past_copy = (turned - index as f32 * step).rem_euclid(FULL_TURN_DEGREES);
                    past_copy.min(FULL_TURN_DEGREES - past_copy)
                };
                let searched = (0..count).min_by(|a, b| gap(*a).total_cmp(&gap(*b))).unwrap_or(0);

                assert_eq!(nearest_copy(turned, step, count, span), searched, "{count} copies over {span} at {turned}");
            }
        }
    }

    fn radial_step(count: u32, span: f32) -> f32 {
        FillSymmetry { count, span, ..symmetry(SymmetryKind::Radial, 2) }.step()
    }
}
