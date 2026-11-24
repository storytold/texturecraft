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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use texturecraft_compose::{BakedMaps, Composite, DisplayKind, ImageRes, ImageStore, Inputs, MaskGray, PaintTarget, PixelStore, StrokePreview, composite_tiles, tile_bytes};
    use texturecraft_core::{
        BakedMap, Bevel, BlendMode, Channel, Effect, EffectKind, Fill, Filter, Generator, GeneratorKind, GradientInput, Layer, LayerId, LayerKind, Mask,
        MaskBackground, Mirror, Procedural, ProceduralKind, Project, Projection, Sampling, Source, SourceWarp, StylizedLighting, TextureSet, TileTransform,
        WarpSource, WarpSourceMode,
    };
    use texturecraft_geom::glam::Vec2;
    use texturecraft_geom::{Mesh, UvGBuffer, primitives};
    use texturecraft_texel::{TILE, TileImage, to_u16, to_u16_signed};

    use super::super::oracle::{OracleCase, Tolerance, assert_matches_cpu, headless};
    use super::super::{BakedInputs, CompositeProgram, CompositeRegion, LiveBrush, LiveBrushTarget};
    use super::UdimLayout;
    use crate::headless::Headless;
    use crate::readback::Readback;

    /// Texels per side of an atlas block: tiles 1001 and 1011 fill theirs.
    const BLOCK: u32 = 64;
    /// Texels per side of the atlas, two blocks across and down.
    const SIDE: u32 = 2 * BLOCK;
    /// The atlas's UV tiles: UDIM, own resolution when scaled, and the mesh UV of its corner.
    const TILES: [(u32, u32, [f32; 2]); 4] = [(1001, BLOCK, [0.0, 0.0]), (1002, BLOCK / 2, [1.0, 0.0]), (1011, BLOCK, [0.0, -1.0]), (1012, BLOCK / 4, [1.0, -1.0])];
    /// A region on the corner of all four blocks, so a halo crosses tiles of every scale.
    const BLOCK_CORNER: CompositeRegion = CompositeRegion { min: [56, 54], max: [74, 70] };
    /// Hard decisions (edge searches, cell centres): at most 0.1 % of the texels may flip.
    const DECISIONS: Tolerance = Tolerance::TRANSCENDENTAL.with_flips((SIDE * SIDE / 1000) as usize);

    /// A set of the four tiles in a 2 × 2 atlas, each with a resolution of its own when `scaled`
    /// (1002 half its block, 1012 a quarter), and a mesh of four planes, one per tile, its UVs
    /// laid in the atlas as an imported UV tile mesh's are.
    fn atlas(scaled: bool, channels: &[Channel]) -> (Project, Arc<Mesh>) {
        let mut set = TextureSet::new("Tiles", 0, BLOCK);
        set.uv_tiles = TILES.map(|(udim, ..)| udim).to_vec();
        set.channels.retain(|(channel, _)| channels.contains(channel));
        if scaled {
            set.tile_resolutions.extend(TILES.iter().filter(|(_, resolution, _)| *resolution != BLOCK).map(|&(udim, resolution, _)| (udim, resolution)));
        }
        let grid = set.tile_grid().unwrap();

        let mut mesh = Mesh::default();
        for (index, &(_, _, corner)) in TILES.iter().enumerate() {
            let mut part = primitives::plane(1.0, 4);
            for position in &mut part.positions {
                position.x += 1.5 * index as f32;
            }
            let corner = Vec2::from(corner);
            part.map_material_uvs_with(0, |centroid, uv| Vec2::from(set.to_atlas(&grid, (centroid + corner).to_array(), (uv + corner).to_array())));
            mesh.append(&part);
        }
        for submesh in &mut mesh.submeshes {
            submesh.material = 0;
        }

        let mut project = Project::new("Tiles", "builtin:plane");
        project.texture_sets.push(set);
        (project, Arc::new(mesh))
    }

    /// A paint layer with Base Color, Roughness and Height painted in stripes of four tones and
    /// two alphas over the whole atlas, a painted mask over a black background, and `stack` after
    /// the content and after the mask's paint, on [`atlas`]'s set.
    fn painted(scaled: bool, stack: &[(Filter, f32, BlendMode)]) -> (Project, Arc<Mesh>, PixelStore, LayerId, LayerId) {
        let channels = [Channel::BaseColor, Channel::Roughness, Channel::Height];
        let (mut project, mesh) = atlas(scaled, &channels);
        let owner = project.alloc_id();
        let mut layer = Layer::new(owner, "Paint", LayerKind::Paint, &channels);
        let mut mask = Mask::new(MaskBackground::Black);
        let mask_paint = project.alloc_id();
        mask.effects.push(Effect::new(mask_paint, EffectKind::Paint));
        for effects in [&mut layer.content_effects, &mut mask.effects] {
            for (filter, opacity, blend) in stack {
                let mut added = Effect::new(project.alloc_id(), EffectKind::Filter(filter.clone()));
                added.opacity = *opacity;
                added.blend = *blend;
                effects.push(added);
            }
        }
        layer.mask = Some(mask);
        project.insert_layer(0, layer, None, false).unwrap();

        // Slanted stripes cross every block border, tones away from gray
        let size = (SIDE, SIDE);
        let mut pixels = PixelStore::default();
        let mut mask_pixels = TileImage::new(SIDE, SIDE, 2);
        for y in 0..SIDE {
            for x in 0..SIDE {
                let tone = 0.15 + 0.2 * ((x / 6 + y / 9) % 4) as f32;
                let alpha = if (x + 2 * y) % 23 < 5 { 0.6 } else { 1.0 };
                let premultiplied = |value: f32| to_u16(value * alpha);
                pixels.layer_mut(owner, Channel::BaseColor, size).set(x, y, &[premultiplied(tone), premultiplied(0.9 - tone), premultiplied(0.5 * tone + 0.2), to_u16(alpha)]);
                pixels.layer_mut(owner, Channel::Roughness, size).set(x, y, &[premultiplied(tone), to_u16(alpha)]);
                pixels.layer_mut(owner, Channel::Height, size).set(x, y, &[to_u16_signed((tone - 0.45) * alpha), to_u16(alpha)]);
                mask_pixels.set(x, y, &[to_u16(1.0 - tone), u16::MAX]);
            }
        }
        pixels.masks.insert(mask_paint, mask_pixels);
        (project, mesh, pixels, owner, mask_paint)
    }

    /// Composites each labelled stack of [`painted`] on the CPU and the GPU, on the uniform and the
    /// scaled atlas: every map, the mask preview and a partial update across the block corner.
    fn check_stacks(gpu: &mut Headless, cases: &[(&str, Vec<(Filter, f32, BlendMode)>)], tolerance: Tolerance) {
        let (baked, images) = (BakedMaps::default(), ImageStore::default());
        for scaled in [false, true] {
            for (label, stack) in cases {
                let (project, mesh, pixels, owner, _) = painted(scaled, stack);
                let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
                case.mask_owner = Some(owner);
                case.region = Some(BLOCK_CORNER);

                assert_matches_cpu(gpu, &case, tolerance, &format!("{label}, scaled {scaled}"));
            }
        }
    }

    /// UV tile sets compile on the GPU, as the material and as the mask preview; a stage runs once
    /// per distinct tile scale, the base stage first; baked maps of different heights are taken.
    #[test]
    fn uv_tile_sets_and_baked_layouts_compile_on_the_gpu() {
        let stack = [(Filter::Blur { radius: 24.0, wrap: false }, 1.0, BlendMode::Normal)];
        let (uniform, _, pixels, owner, _) = painted(false, &stack);
        let (scaled, ..) = painted(true, &stack);
        let mut baked = BakedMaps { resolution: 4, ..Default::default() };
        baked.maps.insert(BakedMap::AmbientOcclusion, Arc::new(vec![0.7; 16]));
        baked.maps.insert(BakedMap::Curvature, Arc::new(vec![0.4; 8]));

        let uniform_program = CompositeProgram::try_compile_region(&uniform, &pixels, 0, None).unwrap();
        let scaled_program = CompositeProgram::try_compile_region(&scaled, &pixels, 0, None).unwrap();
        let mask = CompositeProgram::try_compile_mask_preview(&scaled, &pixels, 0, owner, None, None);
        let separate = BakedInputs::new(&baked, false).unwrap().unwrap();

        assert!(mask.is_ok(), "{:?}", mask.err());
        let levels = |program: &CompositeProgram| program.filters.stages.iter().map(|stage| stage.level).collect::<Vec<_>>();
        let (uniform_levels, scaled_levels) = (levels(&uniform_program), levels(&scaled_program));
        assert!(!uniform_levels.is_empty() && uniform_levels.iter().all(|&level| level == 0), "one stage per stack at one scale: {uniform_levels:?}");
        assert_eq!(scaled_levels.len(), 3 * uniform_levels.len());
        assert!(scaled_levels.chunks(3).all(|stack| stack == [0, 1, 2]), "a stage per scale after its base stage: {scaled_levels:?}");
        assert_eq!(UdimLayout::of(&scaled.texture_sets[0]).map(|layout| layout.levels), Some(vec![1.0, 0.5, 0.25]));
        assert!(uniform_program.udim != 0 && scaled_program.udim != 0);
        assert!(separate.separate() && separate.uniform_words()[0] == 2, "the Curvature's rows: {:?}", separate.uniform_words());
    }

    /// A fill seen through the mesh's own UVs per tile: a reduced Checker repeating once per UV
    /// tile at each tile's resolution, Clouds through the UV projection, a Bilinear HQ image (the
    /// footprint of each tile's UV step), and the gradient's debug ramp across each tile, in the
    /// material and in the mask.
    #[test]
    fn fills_and_the_gradient_debug_view_follow_each_uv_tile() {
        let Some(mut gpu) = headless() else { return };
        let channels = [Channel::BaseColor, Channel::Roughness, Channel::Height];
        let stripes: Vec<[f32; 4]> = (0..96 * 4).map(|texel| [0.2 + 0.6 * f32::from((texel % 96) % 6 >= 3), 0.4, 0.3, 1.0]).collect();
        let images = ImageStore::from([("stripes".to_owned(), Arc::new(ImageRes::new(96, 4, stripes, false)))]);
        let baked = BakedMaps::default();
        let mut checker = Procedural::new(ProceduralKind::Checker);
        checker.resolution.factor = -1;
        let clouds = Procedural { scale: 3.0, seed: 7, ..Procedural::new(ProceduralKind::Clouds) };
        let debug = Filter::GradientMap {
            colors: vec![[0.05, 0.1, 0.6], [0.9, 0.4, 0.1], [0.3, 0.8, 0.2]],
            positions: vec![0.0, 0.3, 0.9],
            quantity: None,
            input: GradientInput::default(),
            linear: true,
            debug: true,
        };

        for scaled in [false, true] {
            let (mut project, mesh) = atlas(scaled, &channels);
            let owner = project.alloc_id();
            let sources = BTreeMap::from([
                (Channel::BaseColor, Source::Procedural { pattern: checker.clone(), low: [0.1, 0.2, 0.3], high: [0.8, 0.7, 0.2], ramp: Vec::new() }),
                (Channel::Roughness, Source::Procedural { pattern: clouds.clone(), low: [0.2; 3], high: [0.9; 3], ramp: Vec::new() }),
                (Channel::Height, Source::image("stripes")),
            ]);
            let fill = Fill { sources, projection: Projection::UvTile, ..Fill::default() };
            let mut layer = Layer::new(owner, "Tiles", LayerKind::Fill(fill), &channels);
            let mut shown = Effect::new(project.alloc_id(), EffectKind::Filter(debug.clone()));
            shown.opacity = 0.5;
            layer.content_effects.push(shown);
            let mut mask = Mask::new(MaskBackground::Black);
            let checker_mask = Source::Procedural { pattern: checker.clone(), low: [0.2; 3], high: [0.9; 3], ramp: Vec::new() };
            mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Fill { source: checker_mask, projection: Projection::default(), sampling: Sampling::default() }));
            let mut masked = Effect::new(project.alloc_id(), EffectKind::Filter(debug.clone()));
            masked.opacity = 0.6;
            mask.effects.push(masked);
            layer.mask = Some(mask);
            project.insert_layer(0, layer, None, false).unwrap();
            let pixels = PixelStore::default();
            let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
            case.mask_owner = Some(owner);

            assert_matches_cpu(&mut gpu, &case, Tolerance::BILINEAR, &format!("UV tile fills, scaled {scaled}"));
        }
    }

    /// Box and distance stages run at each tile's scale and each texel takes its own tile's: a
    /// Blur, the library's Bevel and a Drop Shadow, alone, below full opacity under Multiply, set
    /// to Disable (the mask applies it anyway) and chained.
    #[test]
    fn box_and_distance_stages_run_per_tile_scale() {
        let Some(mut gpu) = headless() else { return };
        let normal = BlendMode::Normal;
        let blur = Filter::Blur { radius: 40.0, wrap: false };
        let bevel = Filter::DistanceBevel(Bevel::LIBRARY);
        let shadow = Filter::DropShadow { angle: 45.0, distance: 0.05, size: 0.1, spread: 0.0, opacity: 0.6, color: [0.1, 0.05, 0.0] };
        let cases = [
            ("Blur", vec![(blur.clone(), 1.0, normal)]),
            ("Bevel", vec![(bevel.clone(), 1.0, normal)]),
            ("Drop Shadow", vec![(shadow, 1.0, normal)]),
            ("Bevel under Multiply", vec![(bevel.clone(), 0.7, BlendMode::Multiply)]),
            ("Bevel disabled", vec![(bevel.clone(), 1.0, BlendMode::Disable)]),
            ("Bevel after a blur", vec![(blur.clone(), 1.0, normal), (bevel, 0.85, normal), (Filter::Invert, 1.0, normal)]),
        ];

        check_stacks(&mut gpu, &cases, Tolerance::NEIGHBOURHOOD);
    }

    /// Whole-tile gathers stay in their UV tile, a smaller tile in the part of its block it fills:
    /// a Mirror, a Transform, and a Warp whose source repeats within the tile.
    #[test]
    fn whole_tile_gathers_stay_in_their_uv_tile() {
        let Some(mut gpu) = headless() else { return };
        let normal = BlendMode::Normal;
        let mirror = Filter::Mirror(Mirror { offset: [0.4, 0.5], ..Mirror::LIBRARY });
        let transform = Filter::Transform(TileTransform { rotation: 30.0, offset: [0.1, -0.2], ..TileTransform::LIBRARY });
        let repeating = WarpSource { mode: WarpSourceMode::PreviousInput, tiling: 2.0, ..WarpSource::LIBRARY };
        let warp = Filter::SourceWarp(SourceWarp { intensity: 0.5, source: repeating, ..SourceWarp::LIBRARY });
        let cases = [
            ("Mirror", vec![(mirror.clone(), 1.0, normal)]),
            ("Transform", vec![(transform, 1.0, normal)]),
            ("Warp from a repeating source", vec![(warp, 1.0, normal)]),
            ("Mirror under Multiply", vec![(mirror.clone(), 0.7, BlendMode::Multiply)]),
            ("Mirror after a blur", vec![(Filter::Blur { radius: 24.0, wrap: false }, 1.0, normal), (mirror, 0.85, normal)]),
        ];

        check_stacks(&mut gpu, &cases, DECISIONS);
    }

    /// A mask generator at a coarser resolution and blurred takes each tile's cells and blur.
    #[test]
    fn a_reduced_generator_takes_each_tiles_cells() {
        let Some(mut gpu) = headless() else { return };
        let mut generator = Generator::new(GeneratorKind::Dirt);
        generator.global.blur = 0.4;
        generator.resolution.factor = -2;
        generator.inputs.insert(BakedMap::AmbientOcclusion, Source::image("Occlusion"));
        let occlusion: Vec<[f32; 4]> = (0..64).map(|texel| [0.1 + 0.8 * ((texel % 8 + texel / 8 * 3) % 8) as f32 / 7.0; 4]).collect();
        let images = ImageStore::from([("Occlusion".into(), Arc::new(ImageRes::new(8, 8, occlusion, false)))]);
        let baked = BakedMaps::default();

        for scaled in [false, true] {
            let (mut project, mesh, pixels, owner, _) = painted(scaled, &[]);
            let mut generated = Effect::new(project.alloc_id(), EffectKind::Generator(generator.clone()));
            generated.blend = BlendMode::Multiply;
            project.layers.get_mut(&owner).and_then(|layer| layer.mask.as_mut()).unwrap().effects.push(generated);
            let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
            case.mask_owner = Some(owner);
            case.region = Some(BLOCK_CORNER);

            assert_matches_cpu(&mut gpu, &case, Tolerance::NEIGHBOURHOOD, &format!("reduced generator, scaled {scaled}"));
        }
    }

    /// Bakes of the uniform bake atlas, half the working atlas's side: a smooth occlusion, a
    /// curvature ramp, a wavy Height and a tilted World Normal.
    fn atlas_bakes() -> BakedMaps {
        const BAKE_SIDE: usize = SIDE as usize / 2;
        let at = |index: usize| ((index % BAKE_SIDE) as f32 / BAKE_SIDE as f32, (index / BAKE_SIDE) as f32 / BAKE_SIDE as f32);
        let texels = 0..BAKE_SIDE * BAKE_SIDE;
        let wave = |(u, v): (f32, f32)| 0.5 + 0.15 * (u * std::f32::consts::TAU * 3.0).sin() * (v * std::f32::consts::TAU * 2.0).cos();
        let tilted = |(u, v): (f32, f32)| [0.5 + 0.2 * (v * std::f32::consts::TAU).sin(), 0.85, 0.5 + 0.2 * (u * std::f32::consts::TAU).cos()];
        let maps = BTreeMap::from([
            (BakedMap::AmbientOcclusion, Arc::new(texels.clone().map(|index| 0.35 + 0.6 * at(index).0 * at(index).1).collect::<Vec<_>>())),
            (BakedMap::Curvature, Arc::new(texels.clone().map(|index| 0.21 + 0.6 * at(index).0).collect::<Vec<_>>())),
            (BakedMap::Height, Arc::new(texels.clone().map(|index| wave(at(index))).collect::<Vec<_>>())),
            (BakedMap::WorldNormal, Arc::new(texels.flat_map(|index| tilted(at(index))).collect::<Vec<_>>())),
        ]);
        BakedMaps { resolution: BAKE_SIDE as u32, maps }
    }

    /// Baked Lighting Stylized lights the detail normal of each tile's own height slopes, read
    /// from the bake atlas's share of each block, and the displays read the bakes so too.
    #[test]
    fn lighting_reads_each_tiles_height_slopes_and_bakes() {
        let Some(mut gpu) = headless() else { return };
        let channels = [Channel::BaseColor, Channel::Metallic, Channel::Roughness];
        let lighting =
            Filter::BakedLighting { azimuth: 30.0, elevation: 50.0, light: 1.0, ambient: 1.0, occlusion: 0.4, stylized: Some(Box::new(StylizedLighting::library())) };
        let (baked, images) = (atlas_bakes(), ImageStore::default());

        for scaled in [false, true] {
            let (mut project, mesh) = atlas(scaled, &channels);
            let owner = project.alloc_id();
            let sources = BTreeMap::from([
                (Channel::BaseColor, Source::Uniform([0.62, 0.45, 0.3])),
                (Channel::Metallic, Source::Uniform([0.2; 3])),
                (Channel::Roughness, Source::Uniform([0.55; 3])),
            ]);
            let mut layer = Layer::new(owner, "Lit", LayerKind::Fill(Fill { sources, ..Fill::default() }), &channels);
            let mut lit = Effect::new(project.alloc_id(), EffectKind::Filter(lighting.clone()));
            lit.off_channels = channels.iter().copied().filter(|&channel| channel != Channel::BaseColor).collect();
            layer.content_effects.push(lit);
            project.insert_layer(0, layer, None, false).unwrap();
            let pixels = PixelStore::default();
            let case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);

            assert_matches_cpu(&mut gpu, &case, Tolerance::TRANSCENDENTAL, &format!("UV tile lighting, scaled {scaled}"));
        }
    }

    /// Baked maps of different heights read each map at its own height: the occlusion and the
    /// normal in the displays, the curvature and the occlusion in mask generators, on a classic
    /// set and on the scaled atlas.
    #[test]
    fn baked_maps_of_different_heights_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let ramp = |side: usize, height: usize, value: fn(f32, f32) -> f32| -> Arc<Vec<f32>> {
            Arc::new((0..side * height).map(|index| value((index % side) as f32 / side as f32, (index / side) as f32 / height as f32)).collect())
        };
        let mut baked = BakedMaps { resolution: 64, ..Default::default() };
        baked.maps.insert(BakedMap::AmbientOcclusion, ramp(64, 64, |u, v| 0.3 + 0.6 * u * v));
        baked.maps.insert(BakedMap::Curvature, ramp(64, 32, |u, _| 0.2 + 0.6 * u));
        baked.maps.insert(BakedMap::Thickness, ramp(64, 16, |_, v| 0.25 + 0.5 * v));
        let normals: Vec<f32> = (0..64 * 8).flat_map(|index| [0.5 + 0.3 * (index % 64) as f32 / 64.0, 0.55, 0.85]).collect();
        baked.maps.insert(BakedMap::Normal, Arc::new(normals));
        let images = ImageStore::default();

        for udim in [false, true] {
            let (mut project, mesh, pixels, owner, _) = painted(udim, &[]);
            if !udim {
                let set = &mut project.texture_sets[0];
                set.uv_tiles.clear();
                set.tile_resolutions.clear();
                set.resolution = SIDE;
            }
            let generators: Vec<Effect> = [GeneratorKind::Curvature, GeneratorKind::AmbientOcclusion, GeneratorKind::ThinAreas]
                .into_iter()
                .map(|kind| {
                    let mut generated = Effect::new(project.alloc_id(), EffectKind::Generator(Generator::new(kind)));
                    generated.blend = BlendMode::Multiply;
                    generated
                })
                .collect();
            project.layers.get_mut(&owner).and_then(|layer| layer.mask.as_mut()).unwrap().effects.extend(generators);
            let mesh = if udim { mesh } else { Arc::new(primitives::plane(1.0, 1)) };
            let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
            case.mask_owner = Some(owner);

            assert_matches_cpu(&mut gpu, &case, Tolerance::POINT, &format!("bakes of different heights, UV tiles {udim}"));
        }
    }

    /// A live mask stroke on the scaled atlas, under a Blur that runs at each tile's scale after
    /// it, matches the CPU's preview of the same stroke.
    #[test]
    fn a_live_mask_stroke_on_uv_tiles_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let stack = [(Filter::Blur { radius: 24.0, wrap: false }, 1.0, BlendMode::Normal)];
        let (project, mesh, pixels, _, mask_paint) = painted(true, &stack);
        let size = (SIDE, SIDE);
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIDE, SIDE));
        let (baked, images) = (BakedMaps::default(), ImageStore::default());
        let mut stroke_alpha = TileImage::new(SIDE, SIDE, 1);
        let mut dense_alpha = vec![0u16; (SIDE * SIDE) as usize];
        for y in 40..100 {
            for x in 30..110 {
                let value = to_u16(0.2 + 0.006 * (x - 30) as f32);
                stroke_alpha.set(x, y, &[value]);
                dense_alpha[(y * SIDE + x) as usize] = value;
            }
        }
        let alpha = super::super::input_texture(&gpu.device, "test UV tile stroke", size, 1, wgpu::TextureFormat::R16Uint);
        super::super::write_input(&gpu.queue, &alpha, size, (0, 0), &dense_alpha);
        let preview = StrokePreview {
            target: PaintTarget::Mask(mask_paint),
            alpha: stroke_alpha,
            opacity: 0.85,
            erase: false,
            values: BTreeMap::new(),
            mask_gray: MaskGray::Value(0.8),
            stencil: None,
            carried: BTreeMap::new(),
            carried_mask: None,
        };
        let set = &project.texture_sets[0];
        let inputs = Inputs {
            project: &project,
            ts: 0,
            mesh: &mesh,
            gbuf: &gbuf,
            pixels: &pixels,
            baked: &baked,
            block_scales: set.block_scales(),
            preview: Some(&preview),
            images: &images,
            bounds: mesh.bounds(),
        };
        let channels: Vec<Channel> = set.channels.iter().map(|&(channel, _)| channel).collect();
        let tiles: Vec<(u32, u32)> = (0..SIDE / TILE).flat_map(|ty| (0..SIDE / TILE).map(move |tx| (tx, ty))).collect();
        let mut cpu = Composite { block_scales: set.block_scales(), ..Composite::new(&channels, SIDE) };
        composite_tiles(&inputs, &mut cpu, &tiles);
        let live = LiveBrush {
            target: LiveBrushTarget::Mask { effect: mask_paint, value: 0.8, image: None },
            opacity: 0.85,
            erase: false,
            alpha: alpha.create_view(&Default::default()),
            stencil: None,
        };

        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[(size, 0)], 1);
        let mut program = CompositeProgram::try_compile_with_brush(&project, &pixels, 0, None, Some(live)).unwrap();
        program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
        program.set_tile_images(&images);
        gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
        let texture = gpu.renderer.display_textures()[0][DisplayKind::BaseColor as usize];
        let (_, _, actual) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap();

        let mut expected = vec![0; actual.len()];
        for &(tx, ty) in &tiles {
            let tile = tile_bytes(&cpu, &baked, DisplayKind::BaseColor, tx, ty);
            for (y, row) in tile.chunks_exact(TILE as usize * 4).enumerate() {
                let start = ((ty * TILE + y as u32) * SIDE + tx * TILE) as usize * 4;
                expected[start..start + row.len()].copy_from_slice(row);
            }
        }
        let off = actual.iter().zip(&expected).filter(|(on_gpu, on_cpu)| on_gpu.abs_diff(**on_cpu) > Tolerance::NEIGHBOURHOOD.lsb).count();
        assert_eq!(off, 0, "the GPU's live stroke differs from the CPU's preview in {off} components");
    }
}
