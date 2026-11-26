use serde::{Deserialize, Serialize};

use super::tile_blending::TileBlending;
use super::tile_input_distribution::TileInputDistribution;
use super::tile_shape::TileShape;
use super::tile_size_mode::TileSizeMode;
use super::tile_symmetry::TileSymmetry;

/// The settings of a Tile Generator pattern: shapes laid out in a grid of tiles, in the reference app's sections (pattern, size, position, rotation, color); pairs are `[horizontal, vertical]`; angles are in degrees; the seeds of the random settings are separate from the pattern's seed, as in the reference app
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TileGenerator {
    /// The level the output is cut at (0.5 leaves it unchanged)
    pub position: f32,
    /// How sharply the output is cut at [`TileGenerator::position`] (0 unchanged, 1 a hard step)
    pub contrast: f32,
    /// Tiles across and down
    pub amount: [u32; 2],
    /// The reference app's Non Square Expansion, kept with the settings: its outputs match with it on and off, so drawing ignores it
    pub non_square_expansion: bool,

    pub shape: TileShape,
    /// Texels per side the shape is drawn at before it is laid out
    pub shape_resolution: [u32; 2],
    /// Changes the shape in its own way (roundness, falloff, ripple count…)
    pub specific: f32,
    pub shape_angle: f32,
    pub shape_scale: f32,
    pub shape_size: [f32; 2],
    /// Repeats of the shape inside each tile
    pub shape_tiling: u32,
    /// Turns the shape 45 degrees
    pub shape_diagonal: bool,
    /// As [`TileGenerator::non_square_expansion`], for the shape itself
    pub shape_non_square_expansion: bool,
    /// Input images, read when [`TileGenerator::shape`] is [`TileShape::Image`]
    pub inputs: Vec<String>,
    pub input_distribution: TileInputDistribution,
    /// Quarter turns of every shape (0 to 3)
    pub quarter_turns: u8,
    /// The share of shapes turned by a random number of quarter turns
    pub quarter_turn_random: f32,
    /// Turns every other shape a quarter turn
    pub quincunx_flip: bool,
    /// The share of shapes mirrored
    pub symmetry_random: f32,
    pub symmetry: TileSymmetry,

    pub size_mode: TileSizeMode,
    /// Where the boundary between alternating columns (rows) falls; 0.5 makes them equal
    pub middle_size: [f32; 2],
    /// The gap between shapes, in [`TileSizeMode::NormalInterstice`]
    pub interstice: [f32; 2],
    pub interstice_random: [f32; 2],
    /// The shape's size as a share of its cell, in [`TileSizeMode::NormalSize`]
    pub size: [f32; 2],
    /// The shape's size as a share of the pattern, in [`TileSizeMode::Absolute`]
    pub size_absolute: [f32; 2],
    /// The shape's size in pixels, in [`TileSizeMode::Pixel`]
    pub size_pixel: [f32; 2],
    /// How much each shape's size shrinks at random, in [`TileSizeMode::Absolute`] and [`TileSizeMode::Pixel`]
    pub size_random: [f32; 2],
    pub scale: f32,
    pub scale_random: f32,
    pub scale_random_seed: u32,

    /// Shift of each row (column) from the one before, as a share of a tile
    pub offset: f32,
    pub offset_random: f32,
    pub offset_random_seed: u32,
    /// Shifts columns instead of rows
    pub vertical_offset: bool,
    pub position_random: [f32; 2],
    pub global_offset: [f32; 2],

    pub rotation: f32,
    pub rotation_random: f32,

    /// The shapes' brightness
    pub color: f32,
    pub luminance_random: f32,
    /// Brightness rises tile after tile
    pub luminance_by_number: bool,
    /// Bigger shapes are brighter
    pub luminance_by_scale: bool,
    /// Hides every other tile
    pub checker_mask: bool,
    /// Hides the odd rows
    pub horizontal_mask: bool,
    /// Hides the even columns
    pub vertical_mask: bool,
    /// The share of tiles hidden at random
    pub random_mask: f32,
    /// Shows the hidden tiles instead
    pub invert_mask: bool,
    pub blending: TileBlending,
    pub background: f32,
    /// An image under the shapes instead of [`TileGenerator::background`]
    pub background_image: Option<String>,
    pub opacity: f32,
    /// Gives tile n the randoms of the last tile but n (the reference app's Reverse Rendering Order: its blendings don't depend on order)
    pub reverse_order: bool,
}

impl TileGenerator {
    /// Tiles per side: the reference app's slider range
    pub const AMOUNT: std::ops::RangeInclusive<u32> = 1..=64;
    /// Shape resolutions offered (the reference app's 256 to 4096)
    pub const SHAPE_RESOLUTIONS: [u32; 5] = [256, 512, 1024, 2048, 4096];
    pub const SHAPE_TILING: std::ops::RangeInclusive<u32> = 1..=16;
    pub const MAX_SCALE: f32 = 2.0;
    pub const MAX_RANDOM_SEED: u32 = 1000;
    /// The largest shape in the pixel size mode: the largest shape resolution
    pub const MAX_SIZE_PIXELS: f32 = 4096.0;
    /// Input images a pattern reads
    pub const MAX_INPUTS: usize = 6;

    pub fn is_default(&self) -> bool {
        *self == TileGenerator::default()
    }

    /// The settings inside their ranges, with non-finite numbers at their defaults (saved projects and commands are hostile input)
    pub fn clamped(&self) -> TileGenerator {
        TileGenerator {
            inputs: self.inputs.iter().take(TileGenerator::MAX_INPUTS).cloned().collect(),
            background_image: self.background_image.clone(),
            ..self.clamped_numbers()
        }
    }

    /// [`TileGenerator::clamped`] without the input images
    fn clamped_numbers(&self) -> TileGenerator {
        let default = TileGenerator::default();
        let unit = |value: f32, fallback: f32| if value.is_finite() { value.clamp(0.0, 1.0) } else { fallback };
        let unit_pair = |pair: [f32; 2], fallback: [f32; 2]| [unit(pair[0], fallback[0]), unit(pair[1], fallback[1])];
        let angle = |value: f32| if value.is_finite() { value.rem_euclid(360.0) } else { 0.0 };
        let amount = |count: u32| count.clamp(*TileGenerator::AMOUNT.start(), *TileGenerator::AMOUNT.end());
        let resolution = |size: u32| {
            let largest = TileGenerator::SHAPE_RESOLUTIONS[TileGenerator::SHAPE_RESOLUTIONS.len() - 1];
            size.clamp(TileGenerator::SHAPE_RESOLUTIONS[0], largest).next_power_of_two().min(largest)
        };
        TileGenerator {
            position: unit(self.position, default.position),
            contrast: unit(self.contrast, default.contrast),
            amount: self.amount.map(amount),
            shape_resolution: self.shape_resolution.map(resolution),
            specific: unit(self.specific, default.specific),
            shape_angle: angle(self.shape_angle),
            shape_scale: unit(self.shape_scale, default.shape_scale),
            shape_size: unit_pair(self.shape_size, default.shape_size),
            shape_tiling: self.shape_tiling.clamp(*TileGenerator::SHAPE_TILING.start(), *TileGenerator::SHAPE_TILING.end()),
            inputs: Vec::new(),
            quarter_turns: self.quarter_turns % 4,
            quarter_turn_random: unit(self.quarter_turn_random, 0.0),
            symmetry_random: unit(self.symmetry_random, 0.0),
            middle_size: unit_pair(self.middle_size, default.middle_size),
            interstice: unit_pair(self.interstice, default.interstice),
            interstice_random: unit_pair(self.interstice_random, default.interstice_random),
            size: unit_pair(self.size, default.size),
            size_absolute: unit_pair(self.size_absolute, default.size_absolute),
            size_pixel: self
                .size_pixel
                .map(|pixels| if pixels.is_finite() { pixels.clamp(0.0, TileGenerator::MAX_SIZE_PIXELS) } else { default.size_pixel[0] }),
            size_random: unit_pair(self.size_random, default.size_random),
            scale: if self.scale.is_finite() { self.scale.clamp(0.0, TileGenerator::MAX_SCALE) } else { default.scale },
            scale_random: unit(self.scale_random, 0.0),
            scale_random_seed: self.scale_random_seed.min(TileGenerator::MAX_RANDOM_SEED),
            offset: unit(self.offset, 0.0),
            offset_random: unit(self.offset_random, 0.0),
            offset_random_seed: self.offset_random_seed.min(TileGenerator::MAX_RANDOM_SEED),
            position_random: unit_pair(self.position_random, default.position_random),
            global_offset: unit_pair(self.global_offset, default.global_offset),
            rotation: angle(self.rotation),
            rotation_random: unit(self.rotation_random, 0.0),
            color: unit(self.color, default.color),
            luminance_random: unit(self.luminance_random, 0.0),
            random_mask: unit(self.random_mask, 0.0),
            background: unit(self.background, default.background),
            opacity: unit(self.opacity, default.opacity),
            background_image: None,
            non_square_expansion: self.non_square_expansion,
            shape: self.shape,
            shape_diagonal: self.shape_diagonal,
            shape_non_square_expansion: self.shape_non_square_expansion,
            input_distribution: self.input_distribution,
            quincunx_flip: self.quincunx_flip,
            symmetry: self.symmetry,
            size_mode: self.size_mode,
            vertical_offset: self.vertical_offset,
            luminance_by_number: self.luminance_by_number,
            luminance_by_scale: self.luminance_by_scale,
            checker_mask: self.checker_mask,
            horizontal_mask: self.horizontal_mask,
            vertical_mask: self.vertical_mask,
            invert_mask: self.invert_mask,
            blending: self.blending,
            reverse_order: self.reverse_order,
        }
    }

    /// Every image the pattern reads
    pub fn images(&self) -> impl Iterator<Item = &str> {
        self.inputs.iter().chain(&self.background_image).map(String::as_str)
    }
}

impl Default for TileGenerator {
    fn default() -> Self {
        TileGenerator {
            position: 0.5,
            contrast: 0.0,
            amount: [10, 10],
            non_square_expansion: true,
            shape: TileShape::default(),
            shape_resolution: [256, 256],
            specific: 0.5,
            shape_angle: 0.0,
            shape_scale: 1.0,
            shape_size: [1.0, 1.0],
            shape_tiling: 1,
            shape_diagonal: false,
            shape_non_square_expansion: true,
            inputs: Vec::new(),
            input_distribution: TileInputDistribution::default(),
            quarter_turns: 0,
            quarter_turn_random: 0.0,
            quincunx_flip: false,
            symmetry_random: 0.0,
            symmetry: TileSymmetry::default(),
            size_mode: TileSizeMode::default(),
            middle_size: [0.5, 0.5],
            interstice: [0.0, 0.0],
            interstice_random: [0.0, 0.0],
            size: [1.0, 1.0],
            size_absolute: [0.1, 0.1],
            size_pixel: [1.0, 1.0],
            size_random: [0.0, 0.0],
            scale: 1.0,
            scale_random: 0.0,
            scale_random_seed: 0,
            offset: 0.0,
            offset_random: 0.0,
            offset_random_seed: 0,
            vertical_offset: false,
            position_random: [0.0, 0.0],
            global_offset: [0.0, 0.0],
            rotation: 0.0,
            rotation_random: 0.0,
            color: 1.0,
            luminance_random: 0.0,
            luminance_by_number: false,
            luminance_by_scale: false,
            checker_mask: false,
            horizontal_mask: false,
            vertical_mask: false,
            random_mask: 0.0,
            invert_mask: false,
            blending: TileBlending::default(),
            background: 0.0,
            background_image: None,
            opacity: 1.0,
            reverse_order: false,
        }
    }
}
