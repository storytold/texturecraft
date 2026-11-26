//! The 2D view's UV tile grid: lines over the whole view, the red U and green V axis through the UV origin, and each
//! tile's UDIM number in a small box at its lower-left corner (dimmer for tiles the set does not paint)

use std::ops::RangeInclusive;

use egui::{Color32, Painter, Rect, Stroke, pos2, vec2};
use texturecraft_engine::core::TextureSet;
use texturecraft_engine::core::project::TileGrid;

use crate::uv_view_map::UvViewMap;

/// Opacity of the grid lines
const GRID_ALPHA: u8 = 40;

/// Width of the grid lines and of the U and V axis lines, in points
const GRID_WIDTH: f32 = 1.0;
const AXIS_WIDTH: f32 = 1.5;

/// The axis lines' colors: U red, V green, as the axis gizmo's
const U_AXIS_COLOR: Color32 = Color32::from_rgb(0xd8, 0x4a, 0x3a);
const V_AXIS_COLOR: Color32 = Color32::from_rgb(0x5c, 0xc0, 0x4a);

/// Smallest tile on screen, in points, that still gets grid lines (a finer grid is only a haze)
const MIN_GRID_CELL: f32 = 4.0;

/// Smallest tile on screen, in points, that gets its number
const MIN_NUMBER_CELL: f32 = 40.0;

/// A number box: the number's inset from the tile's lower-left corner, the box's padding and the number's text size, in points
const NUMBER_INSET: f32 = 4.0;
const NUMBER_PADDING: egui::Vec2 = vec2(3.0, 1.0);
const NUMBER_TEXT_SIZE: f32 = 11.0;

/// Fill alpha of a number box of a tile the set paints, and of an empty tile
const NUMBER_BOX_ALPHA: u8 = 120;
const EMPTY_NUMBER_BOX_ALPHA: u8 = 60;

/// How much of the lettering's strength an empty tile's number keeps
const EMPTY_NUMBER_STRENGTH: f32 = 0.45;

/// UDIM number at the block offset from the atlas’s top left, including valid tiles outside the atlas.
pub(crate) fn udim_of(grid: TileGrid, block_x: i32, block_y: i32) -> Option<u32> {
    let column = (grid.col as i32).checked_add(block_x)?;
    let row = (grid.row as i32).checked_sub(block_y)?;

    ((0..=9).contains(&column) && (0..=99).contains(&row)).then(|| (1001 + column + 10 * row) as u32)
}

/// The atlas blocks the view shows, as ranges of block columns and rows (outside the grid's own blocks too)
pub(crate) fn visible_blocks(map: UvViewMap, grid: TileGrid) -> (RangeInclusive<i32>, RangeInclusive<i32>) {
    let corners = [map.rect.left_top(), map.rect.right_top(), map.rect.right_bottom(), map.rect.left_bottom()].map(|corner| map.to_uv(corner));
    let (low, high) = corners.iter().fold((corners[0], corners[0]), |(low, high), corner| (low.min(*corner), high.max(*corner)));

    // Leave room to step one block past either end when drawing grid lines
    const MAX_BLOCK_OFFSET: f32 = 100_000.0;
    let block = |value: f32, count: u32| (value * count as f32).floor().clamp(-MAX_BLOCK_OFFSET, MAX_BLOCK_OFFSET) as i32;
    (block(low.x, grid.cols)..=block(high.x, grid.cols), block(low.y, grid.rows)..=block(high.y, grid.rows))
}

/// The UDIM tile the set paints under screen point `pointer`
pub(crate) fn tile_under(map: UvViewMap, set: &TextureSet, grid: TileGrid, pointer: egui::Pos2) -> Option<u32> {
    let uv = map.to_uv(pointer);
    let (column, row) = (uv.x * grid.cols as f32, uv.y * grid.rows as f32);
    let (block_x, block_y) = (column.floor() as i32, row.floor() as i32);
    let udim = udim_of(grid, block_x, block_y).filter(|udim| set.uv_tiles.contains(udim))?;

    // A tile smaller than its block fills the block's top-left corner
    let scale = set.tile_scale(udim);
    (column - block_x as f32 <= scale && row - block_y as f32 <= scale).then_some(udim)
}

/// Draws the grid lines, the axis lines and, with `numbers`, the tile numbers of `set` over the view
pub(crate) fn draw(painter: &Painter, map: UvViewMap, set: &TextureSet, grid: TileGrid, numbers: bool, text: Color32) {
    let (columns, rows) = (grid.cols as f32, grid.rows as f32);
    let (blocks_x, blocks_y) = visible_blocks(map, grid);
    let (low_u, high_u) = (*blocks_x.start() as f32 / columns, (*blocks_x.end() + 1) as f32 / columns);
    let (low_v, high_v) = (*blocks_y.start() as f32 / rows, (*blocks_y.end() + 1) as f32 / rows);
    let block_on_screen = (map.to_screen(1.0 / columns, 0.0) - map.to_screen(0.0, 0.0)).length();

    if block_on_screen >= MIN_GRID_CELL {
        let stroke = Stroke::new(GRID_WIDTH, Color32::from_white_alpha(GRID_ALPHA));
        for block_x in *blocks_x.start()..=*blocks_x.end() + 1 {
            let u = block_x as f32 / columns;
            painter.line_segment([map.to_screen(u, low_v), map.to_screen(u, high_v)], stroke);
        }
        for block_y in *blocks_y.start()..=*blocks_y.end() + 1 {
            let v = block_y as f32 / rows;
            painter.line_segment([map.to_screen(low_u, v), map.to_screen(high_u, v)], stroke);
        }
    }

    // The UV origin is the lower-left corner of tile 1001: the U axis runs along its bottom edge, the V axis up its left edge
    let origin = grid.to_atlas([0.0, 1.0]);
    painter.line_segment([map.to_screen(low_u, origin[1]), map.to_screen(high_u, origin[1])], Stroke::new(AXIS_WIDTH, U_AXIS_COLOR));
    painter.line_segment([map.to_screen(origin[0], low_v), map.to_screen(origin[0], high_v)], Stroke::new(AXIS_WIDTH, V_AXIS_COLOR));

    if !numbers || block_on_screen < MIN_NUMBER_CELL {
        return;
    }

    let view = map.rect.expand(NUMBER_INSET * 8.0);
    for block_y in blocks_y.clone() {
        for block_x in blocks_x.clone() {
            let Some(udim) = udim_of(grid, block_x, block_y) else { continue };
            let painted = set.uv_tiles.contains(&udim);

            // A tile smaller than its block fills the block's top-left corner
            let scale = if painted { set.tile_scale(udim) } else { 1.0 };
            let corner = map.to_screen(block_x as f32 / columns, (block_y as f32 + scale) / rows);
            if view.contains(corner) {
                number_box(painter, corner, udim, painted, text);
            }
        }
    }
}

/// `udim` in a small box whose lower-left corner is `inset` from the tile's lower-left corner at `corner`
fn number_box(painter: &Painter, corner: egui::Pos2, udim: u32, painted: bool, text: Color32) {
    let color = if painted { text } else { text.gamma_multiply(EMPTY_NUMBER_STRENGTH) };
    let galley = painter.layout_no_wrap(udim.to_string(), egui::FontId::proportional(NUMBER_TEXT_SIZE), color);
    let box_size = galley.size() + NUMBER_PADDING * 2.0;
    let rect = Rect::from_min_size(pos2(corner.x + NUMBER_INSET, corner.y - NUMBER_INSET - box_size.y), box_size);
    painter.rect_filled(rect, 2.0, Color32::from_black_alpha(if painted { NUMBER_BOX_ALPHA } else { EMPTY_NUMBER_BOX_ALPHA }));
    painter.galley(rect.min + NUMBER_PADDING, galley, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> TileGrid {
        // Tiles 1001, 1002 and 1011: two columns, two rows
        TileGrid::of(&[1001, 1002, 1011]).unwrap()
    }

    #[test]
    fn far_offscreen_tiles_do_not_overflow_the_atlas_origin() {
        let offset_grid = TileGrid::of(&[1012]).unwrap();

        assert_eq!(udim_of(offset_grid, i32::MAX, 0), None);
        assert_eq!(udim_of(offset_grid, 0, i32::MIN), None);
        assert_eq!(udim_of(offset_grid, 0, 0), Some(1012));
    }

    /// Runs `draw` over a 2 x 2 tile atlas (tiles 1001, 1002 and 1011 painted) in a 400 x 400 view and returns what it drew
    fn drawn(numbers: bool, scale: f32) -> Vec<egui::Shape> {
        let mut set = TextureSet::new("Tiles", 0, 128);
        set.uv_tiles = vec![1001, 1002, 1011];
        let grid = set.tile_grid().unwrap();
        let map = UvViewMap { rect: Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0)), scale: [scale, scale], pan: [0.0, 0.0], rotation: 0.0 };
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(map.rect), ..Default::default() }, |ui| {
            draw(ui.painter(), map, &set, grid, numbers, Color32::WHITE);
        });
        output.textures_delta.clear();
        output.shapes.into_iter().map(|clipped| clipped.shape).collect()
    }

    fn shapes_flat(shapes: &[egui::Shape]) -> Vec<&egui::Shape> {
        shapes
            .iter()
            .flat_map(|shape| match shape {
                egui::Shape::Vec(inner) => inner.iter().collect(),
                other => vec![other],
            })
            .collect()
    }

    /// The numbers drawn with their color
    fn numbers_of(shapes: &[egui::Shape]) -> Vec<(String, Color32)> {
        shapes_flat(shapes)
            .into_iter()
            .filter_map(|shape| match shape {
                egui::Shape::Text(text) => Some((text.galley.text().to_owned(), text.galley.job.sections[0].format.color)),
                _ => None,
            })
            .collect()
    }

    fn lines_in(shapes: &[egui::Shape], color: Color32) -> Vec<[egui::Pos2; 2]> {
        shapes_flat(shapes)
            .into_iter()
            .filter_map(|shape| match shape {
                egui::Shape::LineSegment { points, stroke } if stroke.color == color => Some(*points),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn each_tile_gets_its_number_and_an_empty_neighbour_a_dimmer_one() {
        let numbers = numbers_of(&drawn(true, 0.5));
        let color_of = |text: &str| numbers.iter().find(|(shown, _)| shown == text).map(|(_, color)| *color);
        for painted in ["1001", "1002", "1011"] {
            assert_eq!(color_of(painted), Some(Color32::WHITE), "{painted}");
        }

        // 1012 sits right of 1011 in the grid's bounding box and is not painted
        let empty = color_of("1012").expect("the empty tile's number");
        assert!(empty.a() < 255 && empty != Color32::WHITE, "dimmer: {empty:?}");
    }

    #[test]
    fn hiding_the_tile_info_leaves_the_grid_and_axes_but_no_numbers() {
        let shapes = drawn(false, 0.5);
        assert!(numbers_of(&shapes).is_empty());
        assert!(!lines_in(&shapes, Color32::from_white_alpha(GRID_ALPHA)).is_empty(), "grid lines");
    }

    #[test]
    fn the_u_axis_is_red_along_the_bottom_of_tile_1001_and_the_v_axis_green_up_its_left_edge() {
        let shapes = drawn(true, 0.5);
        let (red, green) = (lines_in(&shapes, U_AXIS_COLOR), lines_in(&shapes, V_AXIS_COLOR));
        assert_eq!((red.len(), green.len()), (1, 1));

        // The atlas fills 100..300 on both axes: the origin is its lower-left corner, and the lines run across the whole view
        let [u_from, u_to] = red[0];
        assert!((u_from.y - 300.0).abs() < 1e-2 && (u_to.y - 300.0).abs() < 1e-2 && (u_to.x - u_from.x) > 400.0, "{u_from:?} {u_to:?}");
        let [v_from, v_to] = green[0];
        assert!((v_from.x - 100.0).abs() < 1e-2 && (v_to.x - 100.0).abs() < 1e-2 && (v_to.y - v_from.y).abs() > 400.0, "{v_from:?} {v_to:?}");
    }

    #[test]
    fn grid_lines_cover_the_whole_view_beyond_the_atlas() {
        let shapes = drawn(true, 0.5);
        let grid = lines_in(&shapes, Color32::from_white_alpha(GRID_ALPHA));

        // 200 point tiles: a line at every tile edge across the 400 point view, in both directions
        assert!(grid.iter().any(|[from, to]| from.x == to.x && from.x < 100.0), "a vertical line left of the atlas");
        assert!(grid.iter().any(|[from, to]| from.y == to.y && from.y < 100.0), "a horizontal line above the atlas");
        assert!(grid.iter().any(|[from, to]| from.x == to.x && from.x > 300.0), "a vertical line right of the atlas");
    }

    #[test]
    fn tiles_too_small_for_a_number_show_none() {
        assert!(numbers_of(&drawn(true, 0.05)).is_empty());
    }

    #[test]
    fn blocks_are_numbered_like_udim_tiles_also_outside_the_grid() {
        let grid = grid();
        assert_eq!((udim_of(grid, 0, 1), udim_of(grid, 1, 1), udim_of(grid, 0, 0), udim_of(grid, 1, 0)), (Some(1001), Some(1002), Some(1011), Some(1012)));
        assert_eq!(udim_of(grid, 2, 1), Some(1003));
        assert_eq!(udim_of(grid, 0, 2), None, "below tile 1001 is no UDIM tile");
        assert_eq!(udim_of(grid, -1, 1), None, "left of column 0");
        assert_eq!(udim_of(grid, 10, 1), None, "past column 9");
    }

    #[test]
    fn the_tile_under_the_pointer_is_one_the_set_paints() {
        let mut set = TextureSet::new("Tiles", 0, 128);
        set.uv_tiles = vec![1001, 1002, 1011];
        let grid = set.tile_grid().unwrap();
        let map = UvViewMap { rect: Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0)), scale: [0.5, 0.5], pan: [0.0, 0.0], rotation: 0.0 };

        // The atlas spans 100..300 on both axes: 1011 top-left, 1012 top-right (empty), 1001 bottom-left, 1002 bottom-right
        assert_eq!(tile_under(map, &set, grid, pos2(150.0, 250.0)), Some(1001));
        assert_eq!(tile_under(map, &set, grid, pos2(250.0, 250.0)), Some(1002));
        assert_eq!(tile_under(map, &set, grid, pos2(150.0, 150.0)), Some(1011));
        assert_eq!(tile_under(map, &set, grid, pos2(250.0, 150.0)), None, "1012 is empty");
        assert_eq!(tile_under(map, &set, grid, pos2(20.0, 20.0)), None, "outside the atlas");
    }

    #[test]
    fn the_visible_blocks_follow_pan_and_turn() {
        let grid = grid();
        let rect = Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0));
        let centered = UvViewMap { rect, scale: [0.5, 0.5], pan: [0.0, 0.0], rotation: 0.0 };
        let (x, y) = visible_blocks(centered, grid);
        assert_eq!((*x.start(), *x.end(), *y.start(), *y.end()), (-1, 3, -1, 3));

        let zoomed = UvViewMap { scale: [4.0, 4.0], pan: [-4.0, 0.0], ..centered };
        let (x, y) = visible_blocks(zoomed, grid);
        assert_eq!((*x.start(), *x.end(), *y.start(), *y.end()), (1, 2, 0, 1));
    }
}
