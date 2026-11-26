//! The Transform filter's settings and where it reads each texel from

use serde::{Deserialize, Serialize};

mod flip_mode;
mod repeat_mode;

pub use flip_mode::FlipMode;
pub use repeat_mode::RepeatMode;

/// A turn this close to a whole number of quarter turns (in quarter turns) keeps the tile's edges meeting as it is
const SQUARE_TURN_TOLERANCE: f32 = 1e-4;

/// A shift or step this small (a share of the tile) moves no texel
const IDENTITY_TOLERANCE: f32 = 1e-6;

/// Degrees in a quarter turn
const QUARTER_TURN_DEGREES: f32 = 90.0;

/// A Transform filter: each UV tile's content scaled, turned and moved about the tile's centre, then flipped
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TileTransform {
    pub repeat: RepeatMode,
    pub flip: FlipMode,
    /// How many times the content fits across and down the tile (2 = twice, half as big), as a fill's Tiling
    pub scaling: [f32; 2],
    /// How far the content moves, in tiles; positive reads further right and down, as a fill's Offset
    pub offset: [f32; 2],
    /// Turns only by angles that keep the tile's edges meeting, so a repeating result stays seamless
    pub tile_safe: bool,
    /// Degrees
    pub rotation: f32,
}

impl TileTransform {
    /// The Transform filter as the library adds it: no change, repeating, tile safe
    pub const LIBRARY: TileTransform = TileTransform {
        repeat: RepeatMode::HorizontallyAndVertically,
        flip: FlipMode::None,
        scaling: [1.0, 1.0],
        offset: [0.0, 0.0],
        tile_safe: true,
        rotation: 0.0,
    };

    /// The smallest and largest Scaling it renders (a hostile file's values come back within)
    pub const SCALING: (f32, f32) = (0.01, 64.0);

    /// The farthest Offset it renders, in tiles
    pub const MAX_OFFSET: f32 = 64.0;

    /// Where in the tile (0..1 across and down, before any repeat) the texel at `uv` reads from, as `linear · uv + shift`; the columns of `linear` are what one step across and one step down the result move in the source
    pub fn affine(&self) -> ([[f32; 2]; 2], [f32; 2]) {
        let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
        let (low, high) = Self::SCALING;
        let scaling = self.scaling.map(|scale| finite(scale, 1.0).clamp(low, high));
        let rotation = finite(self.rotation, 0.0);
        let (sin, cos) = rotation.to_radians().sin_cos();

        // Turn, then scale (a fill's Tiling convention): one step across moves (cos, sin) scaled
        let mut columns = [[cos * scaling[0], sin * scaling[1]], [-sin * scaling[0], cos * scaling[1]]];
        let quarter_turns = rotation / QUARTER_TURN_DEGREES;
        let square = (quarter_turns - quarter_turns.round()).abs() < SQUARE_TURN_TOLERANCE;

        if self.tile_safe && !square {
            let exact = columns;

            // Whole steps of the source per tile edge keep the edges meeting; a step can never vanish
            for column in &mut columns {
                let rounded = column.map(f32::round);
                *column = if rounded == [0.0, 0.0] {
                    if column[0].abs() >= column[1].abs() { [column[0].signum(), 0.0] } else { [0.0, column[1].signum()] }
                } else {
                    rounded
                };
            }

            // Preserve orientation when rounding would collapse the transform onto one line
            if determinant(columns) <= 0.0 {
                let [across, down] = columns;
                let across_further = squared_distance(across, exact[0]) >= squared_distance(down, exact[1]);
                columns = if across_further {
                    turning_step(exact[0], |step| [step, down]).map_or(columns, |step| [step, down])
                } else {
                    turning_step(exact[1], |step| [across, step]).map_or(columns, |step| [across, step])
                };
            }
        }

        let [flip_x, flip_y] = self.flip.sides();
        if flip_x {
            columns[0] = columns[0].map(|v| -v);
        }
        if flip_y {
            columns[1] = columns[1].map(|v| -v);
        }

        let offset = self.offset.map(|along| finite(along, 0.0).clamp(-Self::MAX_OFFSET, Self::MAX_OFFSET));

        // About the tile's centre: the centre reads the centre moved by the offset
        let shift = [0, 1].map(|axis| 0.5 + offset[axis] - 0.5 * (columns[0][axis] + columns[1][axis]));
        (columns, shift)
    }

    /// Whether it changes nothing (a turn a hair short of a whole one included)
    pub fn is_identity(&self) -> bool {
        let (columns, shift) = self.affine();
        let steps = [columns[0][0] - 1.0, columns[0][1], columns[1][0], columns[1][1] - 1.0];
        steps.iter().chain(&shift).all(|along| along.abs() < IDENTITY_TOLERANCE)
    }
}

/// How the second column turns from the first: positive anticlockwise, as every exact turn and scale does, 0 when they lie on one line
fn determinant([across, down]: [[f32; 2]; 2]) -> f32 {
    across[0] * down[1] - across[1] * down[0]
}

fn squared_distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

/// The whole step nearest `exact` (a corner of the cell it lies in) whose columns from `pair` turn as an exact pair does; a corner usually does; otherwise the rounded pair stays
fn turning_step(exact: [f32; 2], pair: impl Fn([f32; 2]) -> [[f32; 2]; 2]) -> Option<[f32; 2]> {
    let [x, y] = exact.map(f32::floor);
    [[x, y], [x + 1.0, y], [x, y + 1.0], [x + 1.0, y + 1.0]]
        .into_iter()
        .filter(|corner| determinant(pair(*corner)) > 0.0)
        .min_by(|a, b| squared_distance(*a, exact).total_cmp(&squared_distance(*b, exact)))
}

#[cfg(test)]
mod tests {
    use super::*;

    impl TileTransform {
        /// Where the texel at `uv` reads from in its tile (0..1), or None where the content does not repeat and no longer reaches (the renderer's read, without its bilinear corners)
        fn source(&self, [u, v]: [f32; 2]) -> Option<[f32; 2]> {
            let (columns, shift) = self.affine();
            let at = [0, 1].map(|axis| columns[0][axis] * u + columns[1][axis] * v + shift[axis]);
            match self.repeat {
                RepeatMode::HorizontallyAndVertically => Some(at.map(|along| along.rem_euclid(1.0))),
                RepeatMode::None => at.iter().all(|along| (0.0..=1.0).contains(along)).then_some(at),
            }
        }
    }

    fn near(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
    }

    #[test]
    fn the_library_transform_reads_each_texel_from_itself() {
        let transform = TileTransform::LIBRARY;
        assert!(transform.is_identity());
        assert!(near(transform.source([0.3, 0.7]).unwrap_or_default(), [0.3, 0.7]));
    }

    #[test]
    fn scaling_offset_and_flip_move_the_reads_about_the_centre() {
        let mut transform = TileTransform { scaling: [2.0, 2.0], ..TileTransform::LIBRARY };
        assert!(near(transform.source([0.5, 0.5]).unwrap_or_default(), [0.5, 0.5]), "the centre stays");
        assert!(near(transform.source([0.75, 0.5]).unwrap_or_default(), [0.0, 0.5]), "twice the content: a quarter tile reads half a tile on");

        transform.repeat = RepeatMode::None;

        assert_eq!(transform.source([0.9, 0.5]), None, "past the content reads nothing without repeat");

        transform = TileTransform { offset: [0.25, 0.0], ..TileTransform::LIBRARY };

        assert!(near(transform.source([0.5, 0.5]).unwrap_or_default(), [0.75, 0.5]));
        assert!(near(transform.source([0.9, 0.5]).unwrap_or_default(), [0.15, 0.5]), "the read wraps round the tile");

        transform = TileTransform { flip: FlipMode::Horizontally, ..TileTransform::LIBRARY };

        assert!(near(transform.source([0.2, 0.3]).unwrap_or_default(), [0.8, 0.3]));

        transform.flip = FlipMode::Vertically;

        assert!(near(transform.source([0.2, 0.3]).unwrap_or_default(), [0.2, 0.7]));
    }

    #[test]
    fn a_tile_safe_turn_moves_whole_tiles_per_edge() {
        let transform = TileTransform { scaling: [2.0, 2.0], rotation: 30.0, ..TileTransform::LIBRARY };
        let (columns, _) = transform.affine();
        assert_eq!(columns, [[2.0, 1.0], [-1.0, 2.0]], "the turn snaps to whole steps, about 26.6°");

        // A small turn of the plain tile snaps back to no turn, never to nothing
        let small = TileTransform { rotation: 10.0, ..TileTransform::LIBRARY };
        assert_eq!(small.affine().0, [[1.0, 0.0], [0.0, 1.0]]);

        let tiny = TileTransform { scaling: [0.1, 0.1], rotation: 45.0, ..TileTransform::LIBRARY };
        assert!(tiny.affine().0.iter().all(|column| column != &[0.0, 0.0]));

        // Unsafe, the turn is exact
        let free = TileTransform { tile_safe: false, ..transform };
        assert!((free.affine().0[0][1] - 2.0 * 30f32.to_radians().sin()).abs() < 1e-5);

        // A square turn keeps its scaling
        let square = TileTransform { scaling: [1.5, 1.5], rotation: 90.0, ..TileTransform::LIBRARY };
        assert!((square.affine().0[1][0] + 1.5).abs() < 1e-5);

        // A turn just short of a whole one is square too (a tiny negative turn is stored that way)
        let nearly_whole = TileTransform { scaling: [1.5, 1.5], rotation: 359.99995, ..TileTransform::LIBRARY };
        assert!((nearly_whole.affine().0[0][0] - 1.5).abs() < 1e-5, "{:?}", nearly_whole.affine().0);
    }

    #[test]
    fn a_tile_safe_turn_never_rounds_both_steps_onto_one_line() {
        // Narrow and tall, turned half a quarter: both steps round straight down, so one moves off that line
        let narrow = TileTransform { scaling: [0.1, 2.0], rotation: 45.0, ..TileTransform::LIBRARY };

        let (columns, _) = narrow.affine();

        assert!(determinant(columns) > 0.0, "the source is read in two directions: {columns:?}");
    }

    #[test]
    fn a_turn_a_hair_short_of_a_whole_one_changes_nothing() {
        for tile_safe in [true, false] {
            let transform = TileTransform { rotation: 359.99998, tile_safe, ..TileTransform::LIBRARY };
            assert!(transform.is_identity(), "tile safe {tile_safe}: {:?}", transform.affine());
        }

        assert!(!TileTransform { rotation: 359.9, tile_safe: false, ..TileTransform::LIBRARY }.is_identity());
    }

    #[test]
    fn hostile_values_stay_in_the_tile() {
        let transform = TileTransform { scaling: [f32::NAN, f32::INFINITY], offset: [f32::NAN, -1e30], rotation: f32::INFINITY, ..TileTransform::LIBRARY };
        let [u, v] = transform.source([0.9, 0.9]).unwrap_or_default();
        assert!((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v));
    }
}
