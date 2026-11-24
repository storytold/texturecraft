//! P8, UDIM texture sets and baked maps with different layouts (plan section 2.4, last two
//! rows). Owned by the P8 agent; no other package edits this file or
//! `shaders/composite_udim.wgsl`. P8 lands last: it changes coordinates everywhere, so it starts
//! after every other package has merged and then may touch their files.
//!
//! What P8 implements, as the CPU's `eval.rs` evaluates a UV tile atlas:
//! - the tile grid and each block's tile scale in a side-table record (`bind`, `udim_record` in
//!   WGSL, its offset in `params.packages[7].x`);
//! - the mesh's own UV for every projection and pattern (`TextureSet::from_atlas`), the UV step of
//!   footprints and the gradient's debug ramp per UV tile;
//! - procedural resolution per UV tile: a reduced pattern's texels per repeat at each tile scale
//!   (`pattern_levels`, a side-table table in `Operation::ext[3]`);
//! - neighbourhood stages once per distinct tile scale: a level stage after each base stage
//!   (`level_filters`, `FilterStage::level`), whose output the texels of that scale's blocks read
//!   (`udim_level_slot`), and generator cells and blur per scale;
//! - per-tile height slopes and bake reads that leave out what a smaller tile leaves empty of its
//!   block (`BlockScales::bake_uv`), and Extra's tile scale for the viewport's height slopes;
//! - baked maps of different heights, each height's maps in rows of their own
//!   (`BakedInputs::separate`, the header row in `params.packages[7].y`).
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] (no library filter is P8's).

use texturecraft_compose::pattern::pattern_pixels;
use texturecraft_core::{ChannelKind, Filter, Procedural, TextureSet};

use super::{BakedInputs, CompositeProgram, FilterPlan, ProceduralInputs};
use crate::gpu_neighbourhood::{NeighbourhoodFilter, gather};

#[path = "gpu_composite_udim_layout.rs"]
mod layout;
pub(super) use layout::UdimLayout;

/// Library filter ids P8 still leaves on the CPU (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &[];

pub(super) const WGSL: &str = include_str!("shaders/composite_udim.wgsl");
/// A classic set's coordinates, bakes and stage slots, for programs without a UV tile atlas or
/// baked maps of different heights.
pub(super) const WGSL_STUB: &str = "
fn udim_mesh_uv(uv: vec2<f32>) -> vec2<f32> { return uv; }
fn udim_value_uv(layer: Operation, uv: vec2<f32>) -> vec2<f32> { return uv; }
fn udim_mesh_uv_step(pixel: vec2<u32>) -> vec2<f32> { return 1.0 / vec2<f32>(params.source.xy); }
fn udim_placed(layer: Operation, uv: vec2<f32>) -> Operation { return layer; }
fn udim_level_slot(slot: u32, pixel: vec2<u32>) -> u32 { return slot; }
fn udim_tile_frame(pixel: vec2<u32>) -> vec4<u32> { return vec4(0u, 0u, params.source.xy); }
fn udim_bake_uv(uv: vec2<f32>) -> vec3<f32> { return vec3(uv, 1.0); }
fn udim_bake_valid(pixel: vec2<u32>) -> bool { return true; }
fn udim_absent_bake(block: u32) -> vec4<u32> { return vec4(0u); }
fn udim_separate_bake(pixel: vec2<u32>, block: u32) -> vec4<u32> { return vec4(0u); }
fn udim_extra_scale(pixel: vec2<u32>) -> u32 { return 0u; }
";

/// UV tile atlases and baked maps of every layout composite on the GPU; the hub keeps the gate
/// for its callers.
pub(super) fn check_set(_set: &TextureSet, _mask_preview: bool) -> Result<(), &'static str> {
    Ok(())
}

/// Whether a program needs the full `composite_udim.wgsl`: a UV tile atlas, or baked maps of
/// different heights.
pub(super) fn uses_shader(program: &CompositeProgram) -> bool {
    program.udim != 0 || program.baked.as_ref().is_some_and(BakedInputs::separate)
}

/// P8's vec4 of the band uniform (`params.packages[7]`): the UV tile atlas's record, and the
/// header row plus one of baked maps of different heights (0: none).
pub(super) fn band_uniform(program: &CompositeProgram) -> [u32; 4] {
    let header = program.baked.as_ref().filter(|baked| baked.separate()).map_or(0, |baked| baked.separate_layout().1 + 1);
    [program.udim, header, 0, 0]
}

/// Set what a UV tile atlas changes in the program inputs: the base scale's tile resolution, the
/// set's own resolution, the atlas's longest side and block, and the further scales'
/// resolutions. A classic set keeps the inputs as they are.
pub(super) fn prepare(layout: Option<&UdimLayout>, inputs: &mut ProceduralInputs) {
    let Some(layout) = layout else { return };

    let resolutions = layout.level_resolutions();
    inputs.resolution = resolutions.first().copied().unwrap_or(layout.block);
    inputs.texture_set = layout.texture_set;
    inputs.longest_side = layout.longest_side;
    inputs.block = layout.block;
    inputs.levels = resolutions.into_iter().skip(1).collect();
}

/// `filter`'s stage at each further UV tile scale (`ProceduralInputs::levels`), as the CPU's
/// `filter_region` runs it there: built at the tile's resolution, its radius at most the base
/// stage's, whole-tile reads within the base block. None where the filter changes nothing at
/// that scale: the stage then passes its field on.
pub(super) fn level_filters(
    filter: &Filter,
    base: NeighbourhoodFilter,
    (opacity, channel): (f32, Option<ChannelKind>),
    inputs: &ProceduralInputs,
) -> Vec<Option<NeighbourhoodFilter>> {
    inputs
        .levels
        .iter()
        .map(|&resolution| {
            let mut level = NeighbourhoodFilter::new(filter, resolution, inputs.longest_side, opacity, channel)?;
            level.radius = level.radius.min(base.radius);
            gather::set_block(&mut level, inputs.block);
            Some(level)
        })
        .collect()
}

/// `pattern`'s reduced texels per repeat at every UV tile scale, base scale first, as a
/// side-table table (`Operation::ext[3]`, `udim_placed` in WGSL); 0 when no scale changes them.
/// None when the side table is full.
pub(super) fn pattern_levels(pattern: &Procedural, inputs: &mut ProceduralInputs) -> Option<u32> {
    if inputs.levels.is_empty() {
        return Some(0);
    }

    let texture_set = inputs.texture_set;
    let pixels: Vec<u32> =
        std::iter::once(inputs.resolution).chain(inputs.levels.iter().copied()).map(|tile| pattern_pixels(pattern, tile, texture_set).unwrap_or(0)).collect();
    if pixels.iter().all(|&texels| Some(&texels) == pixels.first()) {
        return Some(0);
    }
    inputs.pack_words(&pixels)
}

/// Pack a UV tile atlas's record into the side table and return its offset (`udim_record` in
/// WGSL), 0 for a classic set: the grid, the block's side, whether a tile is scaled, the stage
/// level table, then each block's scale and level. A base stage and its level stages run
/// together, all in the whole-set pre-pass or all in the bands. None when the side table is full.
pub(super) fn bind(layout: Option<&UdimLayout>, filters: &mut FilterPlan, inputs: &mut ProceduralInputs) -> Option<u32> {
    let Some(layout) = layout else { return Some(0) };

    // A base stage with its level stages, the pre-pass's or the bands' alike
    let stages = &mut filters.stages;
    let mut first = 0;
    while first < stages.len() {
        let end = first + 1 + stages[first + 1..].iter().take_while(|stage| stage.level != 0).count();
        let whole = stages[first].whole;
        if stages[first..end].iter().any(|stage| stage.whole != whole) {
            stages[first..end].iter_mut().for_each(|stage| stage.whole = false);
        }
        first = end;
    }

    // Per stage slot, 1 where level stages follow it (`udim_level_slot` in WGSL)
    let slots: Vec<u32> = (0..stages.len()).map(|slot| u32::from(stages.get(slot + 1).is_some_and(|next| next.level == 1))).collect();
    let table = if slots.contains(&1) {
        let count = u32::try_from(slots.len()).ok()?;
        inputs.pack_words(&[&[count][..], &slots].concat())?
    } else {
        0
    };
    let grid = layout.grid;
    let mut words = vec![grid.cols, grid.rows, grid.col, grid.row, layout.block, u32::from(layout.scaled), table];
    words.extend(layout.scales.iter().map(|scale| scale.to_bits()));
    words.extend(layout.block_levels());
    inputs.pack_words(&words)
}
