//! P4, the box family and distance neighbourhood filters (plan section 2.3). Owned by the P4
//! agent; no other package edits this file or `shaders/neighbourhood_box.wgsl`.
//!
//! What P4 implements: Blur and Bevel `wrap`, Highpass `space` (sRGB from linear or ACEScg) and
//! `alpha`, Pixelate `alpha: false`, the Gradient's `input.blur` pre-blur (a box stage, then P2's
//! gradient), Glow (`HaloGlow`), Drop Shadow, Mask Outline, and the distance filters Bevel
//! (`DistanceBevel`), Bevel Smooth and Directional Distance (separable distance transform with
//! exact integer envelope arithmetic: the CPU computes the crossings in f64). The Distance Map
//! input of Bevel Smooth and Directional Distance waits for P3's inputs: an effect with an image
//! input still composites on the CPU (`effect_needs_cpu`).
//!
//! How:
//! - [`supports`] and [`new`] take every filter the built-in stages reject (the built-in ones keep
//!   their exact settings, so `Blur { wrap: true, .. }` reaches them). Kinds: `BOX_KINDS`
//!   16..24 and `DISTANCE_KINDS` 24..32 (gpu_neighbourhood.rs), as `Kind(n)` constants in this file. `new` returns None
//!   only when the size reaches nothing (the effect then changes nothing, as on the CPU).
//!   `StageInputs::filter` builds the stage; `words` and `extra` carry further parameters.
//! - [`uses_box`], [`reach`] and [`work_per_texel`] answer for P4 kinds only; `uses_box` must
//!   agree with `box_family_uses_box` in the WGSL file. Large halos (Drop Shadow offset) belong
//!   in [`reach`].
//! - [`encode`] runs passes of P4's own (return true), else the box passes and the apply pass run;
//!   the WGSL functions `box_family_filtered`/`box_family_gathered` then compute the texel.
//!   New entry points go in [`Pipelines`].
//! - Wrapping stages ([`wraps`]) set `FilterStage::wrap`: the prepare pass then fills their field
//!   periodically past the set's edges, so the box passes stay local.
//!
//! - The Gradient's `input.blur` (with P2): P4 provides the stage (a kind whose output keeps the
//!   original colour for the opacity mix and carries the blurred gray where P2's gradient reads
//!   it); P2's `lower` queues it with `ProceduralInputs::precede_with_stage` before the gradient.
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] and `still_on_cpu` in the tests below. Every port gets a CPU
//! oracle test (`crate::gpu_composite::oracle`, `Tolerance::NEIGHBOURHOOD`) on paint and in a
//! mask, chained, and with a partial region (the halo); distance filters also compare the raw
//! distance field exactly.

use texturecraft_core::{ChannelKind, Filter, HighpassSpace};

use super::{APPLY_WORKGROUP, BOX_KINDS, BOX_PASSES, Kind, LINE_WORKGROUP, NeighbourhoodFilter, NeighbourhoodGpu, NeighbourhoodRegion, Params, StageInputs};

/// Library filter ids P4 still leaves on the CPU with their defaults (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &["bevel", "bevelSmooth", "directionalDistance", "glow", "dropShadow", "maskOutline"];

pub(super) const WGSL: &str = include_str!("shaders/neighbourhood_box.wgsl");

/// Pixelate with Alpha off: the cell's colour laid on each texel's own material alpha.
const PIXELATE_KEEPING_ALPHA: Kind = Kind(BOX_KINDS.start);
/// Blur with Wrap: the built-in box passes over a field the prepare pass fills periodically.
const WRAPPED_BLUR: Kind = Kind(BOX_KINDS.start + 1);
/// Bevel with Wrap, as [`WRAPPED_BLUR`].
const WRAPPED_BEVEL: Kind = Kind(BOX_KINDS.start + 2);
/// Highpass of colour in an sRGB space, or of the alpha too.
const HIGHPASS: Kind = Kind(BOX_KINDS.start + 3);

/// Filter sizes are texels at this many texels per side, as the CPU scales them.
const FILTER_SIZE_RESOLUTION: f32 = 2048.0;
/// The smallest radius a filter runs at, in texels.
const MIN_FILTER_RADIUS: f32 = 0.5;

/// Work per texel of one bilinear sample, as the built-in Pixelate counts it.
const BILINEAR_SAMPLE_WORK: u64 = 5;
/// Work per texel the built-in stages count for a box filter and its apply pass.
const BOX_FILTER_WORK: u64 = 8;
/// Work per texel of a pass of P4's own (prepare, apply, a distance line or the edges).
const OWN_PASS_WORK: u64 = 2;

/// P4's own compute pipelines, created from the neighbourhood module.
pub(super) struct Pipelines {
    prepare: wgpu::ComputePipeline,
    apply: wgpu::ComputePipeline,
}

impl Pipelines {
    pub(super) fn new(pipeline: &dyn Fn(&'static str) -> wgpu::ComputePipeline) -> Self {
        Self { prepare: pipeline("box_family_prepare"), apply: pipeline("box_family_apply") }
    }
}

/// Whether a P4 stage evaluates `filter` as the CPU does.
pub(super) fn supports(filter: &Filter) -> bool {
    matches!(filter, Filter::Mosaic { alpha: false, .. } | Filter::Blur { wrap: true, .. } | Filter::Bevel { wrap: true, .. } | Filter::HighPass { .. })
}

/// The P4 stage for `filter`, None when its size reaches nothing.
pub(super) fn new(filter: &Filter, stage: StageInputs) -> Option<NeighbourhoodFilter> {
    // Sizes at 2K, scaled as neighbourhood_filters in eval.rs scales them
    let texels = |size: f32| (size * stage.resolution as f32 / FILTER_SIZE_RESOLUTION).max(MIN_FILTER_RADIUS);

    match filter {
        Filter::Mosaic { size, alpha: false } => Some(stage.filter(PIXELATE_KEEPING_ALPHA, texels(*size))),
        Filter::Blur { radius, wrap: true } => (*radius > 0.0).then(|| stage.filter(WRAPPED_BLUR, texels(*radius))),
        // Three box passes ramp an edge over about twice their radius
        Filter::Bevel { width, wrap: true } => Some(stage.filter(WRAPPED_BEVEL, texels(*width * 0.5))),
        Filter::HighPass { radius, space, alpha } => Some(highpass(stage, texels(*radius), *space, *alpha)),
        _ => None,
    }
}

/// A Highpass stage: masks filter their value in the working space, content other than colour
/// keeps the working space (filter_region in eval.rs). The built-in stage takes what is left.
fn highpass(stage: StageInputs, texels: f32, space: HighpassSpace, alpha: bool) -> NeighbourhoodFilter {
    let space = if stage.channel == Some(ChannelKind::Color) { space } else { HighpassSpace::Working };
    let alpha = alpha && stage.channel.is_some();
    if space == HighpassSpace::Working && !alpha {
        return stage.filter(Kind::HIGH_PASS, texels);
    }

    let mut highpass = stage.filter(HIGHPASS, texels);
    let space_word = match space {
        HighpassSpace::Working => 0,
        HighpassSpace::SrgbFromLinear => 1,
        HighpassSpace::SrgbFromAcescg => 2,
    };
    highpass.words = [space_word, u32::from(alpha)];
    highpass
}

/// Whether the stage reads past the set's edges from the far side (`FilterStage::wrap`).
pub(crate) fn wraps(filter: NeighbourhoodFilter) -> bool {
    matches!(filter.kind, WRAPPED_BLUR | WRAPPED_BEVEL)
}

pub(super) fn uses_box(filter: NeighbourhoodFilter) -> bool {
    matches!(filter.kind, WRAPPED_BLUR | WRAPPED_BEVEL | HIGHPASS)
}

pub(super) fn reach(filter: NeighbourhoodFilter) -> u32 {
    match filter.kind {
        PIXELATE_KEEPING_ALPHA => filter.radius.saturating_add(1),
        _ => filter.radius.saturating_mul(BOX_PASSES as u32),
    }
}

pub(super) fn work_per_texel(filter: NeighbourhoodFilter) -> u64 {
    match filter.kind {
        PIXELATE_KEEPING_ALPHA => BILINEAR_SAMPLE_WORK,
        HIGHPASS => BOX_FILTER_WORK + OWN_PASS_WORK,
        _ => BOX_FILTER_WORK,
    }
}

/// The three horizontal and three vertical box passes over the field at `first` (read on the
/// first pass as the original is), radius `radius`; the offset of the blurred field.
fn box_passes(
    run: &mut impl FnMut(&wgpu::ComputePipeline, [u32; 4], u32, bool, [u32; 2]),
    box_blur: &wgpu::ComputePipeline,
    region: NeighbourhoodRegion,
    first: u32,
    radius: u32,
) -> u32 {
    let [_, _, width, height] = region.rect;
    let mut source = first;
    for step in 0..2 * BOX_PASSES {
        let vertical = step >= BOX_PASSES;
        let line_count = if vertical { width } else { height };
        let target = region.blur[step % 2];
        run(box_blur, [first, source, target, u32::from(vertical)], radius, step == 0, [line_count.div_ceil(LINE_WORKGROUP), 1]);
        source = target;
    }
    source
}

/// Encodes a P4 stage with passes of its own; false lets the box and apply passes run.
pub(super) fn encode(
    (gpu, pipelines): (&NeighbourhoodGpu, &Pipelines),
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    scratch: &wgpu::Buffer,
    region: NeighbourhoodRegion,
    filter: NeighbourhoodFilter,
) -> bool {
    let [_, _, width, height] = region.rect;
    let texel_groups = [width.div_ceil(APPLY_WORKGROUP), height.div_ceil(APPLY_WORKGROUP)];
    let mut run = |pipeline: &wgpu::ComputePipeline, buffers: [u32; 4], radius: u32, first_box: bool, groups: [u32; 2]| {
        let params = Params {
            rect: region.rect,
            buffers,
            mode: [radius, filter.kind.0, filter.seed, u32::from(first_box)],
            weights: [filter.opacity, filter.amount, filter.frequency, filter.middle],
            direction: [filter.direction[0], filter.direction[1], 0.0, 0.0],
            field_layout: region.field_layout(filter.words),
            extra: filter.extra,
            stage: [filter.context, u32::from(region.wrap), 0, 0],
            bounds: region.bounds,
        };
        NeighbourhoodGpu::run(pipeline, device, encoder, scratch, &params, groups);
    };
    let (original, output) = (region.original, region.output);
    match filter.kind {
        // What the box passes blur goes to the output field first, which the apply pass overwrites
        HIGHPASS => {
            run(&pipelines.prepare, [original, 0, output, 0], filter.radius, false, texel_groups);
            let blurred = box_passes(&mut run, &gpu.box_blur, region, output, filter.radius);
            run(&pipelines.apply, [original, blurred, output, 0], filter.radius, false, texel_groups);
        }
        // Pixelate gathers and the wrapping filters run the built-in passes
        _ => return false,
    }
    true
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::sync::Arc;

    use crate::gpu_composite::CompositeRegion;
    use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu, compile_effects, effect, headless};
    use crate::headless::Headless;
    use texturecraft_compose::{BakedMaps, ImageRes, ImageStore, PixelStore};
    use texturecraft_core::{
        BakedMap, BlendMode, Channel, ContentsOffset, Effect, EffectKind, Filter, Generator, GeneratorKind, GradientInput, HighpassSpace, Layer, LayerId,
        LayerKind, Mask, MaskBackground, Project, Source, TextureSet,
    };
    use texturecraft_geom::glam::Vec2;
    use texturecraft_geom::primitives;
    use texturecraft_texel::{TileImage, to_u16, to_u16_signed};

    /// Side of the oracle sets, in texels.
    const SIZE: u32 = 128;

    /// Cell centres are a hard decision: at most 0.1 % of the texels may flip.
    const CELL_CENTRES: Tolerance = Tolerance::NEIGHBOURHOOD.with_flips((SIZE * SIZE / 1000) as usize);

    /// A stack of (filter, effect opacity, effect blend).
    type Stack = Vec<(Filter, f32, BlendMode)>;

    /// The oracle's layer: paint in the middle or across the set's edges, the set's side, and
    /// whether the mesh leaves a border of the set uncovered.
    type Fixture = (bool, u32, bool);

    /// Paint in the middle of a 128 texel set on a mesh covering it.
    const MIDDLE_PAINT: Fixture = (false, SIZE, false);
    /// Paint across the set's edges.
    const EDGE_PAINT: Fixture = (true, SIZE, false);

    /// The middle of the paint, where the tone and alpha steps meet.
    const MIDDLE: CompositeRegion = CompositeRegion { min: [61, 57], max: [75, 73] };
    /// A corner of the set, whose halo reaches across its edges.
    const CORNER: CompositeRegion = CompositeRegion { min: [120, 2], max: [128, 18] };

    /// P4's filters and settings that still composite on the CPU, in content and in masks. Port
    /// one: delete its row (this test fails while a ported row stays) and add its oracle test.
    fn still_on_cpu() -> Vec<Filter> {
        vec![
            Filter::DropShadow { angle: 45.0, distance: 0.02, size: 8.0, spread: 0.0, opacity: 1.0, color: [0.0; 3] },
            Filter::GradientMap {
                colors: vec![[0.0; 3], [1.0; 3]],
                positions: Vec::new(),
                quantity: None,
                input: GradientInput { blur: 2.0, ..Default::default() },
                linear: true,
                debug: false,
            },
        ]
    }

    #[test]
    fn box_and_distance_filters_still_on_the_cpu_fall_back() {
        let none = ContentsOffset::NONE;
        for filter in still_on_cpu() {
            let (material, _) = compile_effects(Some(effect(EffectKind::Filter(filter.clone()))), None, none);
            assert!(material.is_err(), "content {filter:?}");
            let (material, mask) = compile_effects(None, Some(effect(EffectKind::Filter(filter.clone()))), none);
            assert!(material.is_err() && mask.is_err(), "mask {filter:?}");
        }
    }

    /// Every filter and setting P4 ported compiles for the GPU, in content and in the mask.
    #[test]
    fn box_and_distance_filters_compile_on_the_gpu() {
        let none = ContentsOffset::NONE;
        let ported = [
            Filter::Mosaic { size: 96.0, alpha: false },
            Filter::Blur { radius: 8.0, wrap: true },
            Filter::Bevel { width: 16.0, wrap: true },
            Filter::HighPass { radius: 8.0, space: HighpassSpace::SrgbFromLinear, alpha: false },
            Filter::HighPass { radius: 8.0, space: HighpassSpace::SrgbFromAcescg, alpha: true },
            Filter::HighPass { radius: 8.0, space: HighpassSpace::Working, alpha: true },
        ];

        for filter in ported {
            let (material, _) = compile_effects(Some(effect(EffectKind::Filter(filter.clone()))), None, none);
            let (mask_material, mask) = compile_effects(None, Some(effect(EffectKind::Filter(filter.clone()))), none);

            assert!(material.is_ok() && mask_material.is_ok() && mask.is_ok(), "{filter:?}: {material:?} {mask_material:?} {mask:?}");
        }
    }

    /// A paint layer with Base Color, Roughness and Height painted in two tones (inside and outside
    /// the shapes at 0.5) and two alphas, a mask painted with the same tones, and `stack` after the
    /// content and after the mask's paint, on a set `side` texels across. The paint lies in the
    /// middle, or in the corners across the set's edges. Base Color blends by `channel_blend`.
    fn painted_project(
        stack: &[(Filter, f32, BlendMode)],
        channel_blend: BlendMode,
        (across_edges, side): (bool, u32),
    ) -> Option<(Project, PixelStore, LayerId)> {
        let channels = [Channel::BaseColor, Channel::Roughness, Channel::Height];
        let mut project = Project::new("Box family", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, side));
        let owner = project.alloc_id();
        let mut layer = Layer::new(owner, "Paint", LayerKind::Paint, &channels);
        layer.channels.get_mut(&Channel::BaseColor)?.mode = channel_blend;
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
        project.insert_layer(0, layer, None, false).ok()?;

        // Cells, blurs and distances across the tone and alpha steps and the paint's edges, laid out at 128 texels across
        let size = (side, side);
        let scale = side / SIZE;
        let mut pixels = PixelStore::default();
        let mut mask_pixels = TileImage::new(side, side, 2);
        for y in 0..side {
            for x in 0..side {
                let (at_x, at_y) = (x / scale, y / scale);
                let painted =
                    if across_edges { !(26..104).contains(&at_x) && !(30..100).contains(&at_y) } else { (23..98).contains(&at_x) && (22..103).contains(&at_y) };
                if !painted {
                    continue;
                }
                let (alpha, tone) = if across_edges {
                    (if at_y < 64 { 1.0 } else { 0.65 }, if at_x < 64 { 0.8 } else { 0.4 })
                } else {
                    (if at_x >= 63 { 1.0 } else { 0.65 }, if at_y <= 64 { 0.8 } else { 0.4 })
                };
                let premultiplied = |value: f32| to_u16(value * alpha);
                let color = [premultiplied(tone), premultiplied(0.9 * tone + 0.05), premultiplied(0.6 * tone), to_u16(alpha)];
                pixels.layer_mut(owner, Channel::BaseColor, size).set(x, y, &color);
                pixels.layer_mut(owner, Channel::Roughness, size).set(x, y, &[premultiplied(tone), to_u16(alpha)]);
                pixels.layer_mut(owner, Channel::Height, size).set(x, y, &[to_u16_signed((tone - 0.6) * alpha), to_u16(alpha)]);
                mask_pixels.set(x, y, &[to_u16(tone), u16::MAX]);
            }
        }
        pixels.masks.insert(mask_paint, mask_pixels);
        Some((project, pixels, owner))
    }

    /// The plane, or the plane's UVs shrunk so a border of the set lies off the mesh.
    fn oracle_mesh(uncovered_border: bool) -> Arc<texturecraft_geom::Mesh> {
        let mut mesh = primitives::plane(1.0, 1);
        if uncovered_border {
            for uv in &mut mesh.uvs {
                *uv = *uv * 0.8 + Vec2::splat(0.1);
            }
        }
        Arc::new(mesh)
    }

    /// Composites `project` on the CPU and the GPU: every map, the mask preview of `owner` and a
    /// partial update of `region`.
    fn check_project(
        gpu: &mut Headless,
        (project, pixels, owner): (&Project, &PixelStore, LayerId),
        uncovered_border: bool,
        region: CompositeRegion,
        tolerance: Tolerance,
        label: &str,
    ) {
        let mesh = oracle_mesh(uncovered_border);
        let (baked, images) = (BakedMaps::default(), ImageStore::default());
        let mut case = OracleCase::new(project, pixels, &baked, &images, &mesh);
        case.mask_owner = Some(owner);
        case.region = Some(region);

        assert_matches_cpu(gpu, &case, tolerance, label);
    }

    /// Composites each (label, stack, Base Color blend) on the CPU and the GPU on `fixture`'s layer.
    /// None when a project could not be built.
    fn check_cases(gpu: &mut Headless, cases: &[(&str, Stack, BlendMode)], fixture: Fixture, region: CompositeRegion, tolerance: Tolerance) -> Option<()> {
        let (across_edges, side, uncovered_border) = fixture;
        for (label, stack, channel_blend) in cases {
            let (project, pixels, owner) = painted_project(stack, *channel_blend, (across_edges, side))?;
            check_project(gpu, (&project, &pixels, owner), uncovered_border, region, tolerance, label);
        }
        Some(())
    }

    /// Pixelate with Alpha off matches the CPU in Base Color, Roughness, Height and the mask: at
    /// the library's size, below full opacity under a Multiply layer, set to Disable (the mask
    /// applies it anyway), after a blur and before one, and in a partial update that reads the halo.
    #[test]
    fn pixelate_keeping_alpha_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let pixelate = |size: f32| Filter::Mosaic { size, alpha: false };
        let blur = Filter::Blur { radius: 24.0, wrap: false };
        let normal = BlendMode::Normal;
        let cases = [
            ("Pixelate, library size", vec![(pixelate(Filter::pixel_size(32.0)), 1.0, normal)], normal),
            ("Pixelate under Multiply", vec![(pixelate(160.0), 0.7, normal)], BlendMode::Multiply),
            ("Pixelate disabled", vec![(pixelate(96.0), 1.0, BlendMode::Disable)], normal),
            ("Pixelate after a blur", vec![(blur.clone(), 1.0, normal), (pixelate(96.0), 0.85, normal), (Filter::Invert, 1.0, normal)], normal),
            ("Pixelate before a blur", vec![(Filter::Invert, 1.0, normal), (pixelate(96.0), 0.85, normal), (blur, 1.0, normal)], normal),
        ];

        assert!(check_cases(&mut gpu, &cases, MIDDLE_PAINT, MIDDLE, CELL_CENTRES).is_some());
    }

    /// Blur and Bevel with Wrap match the CPU on paint across the set's edges, whole and in a
    /// corner's partial update: below full opacity under Multiply, disabled, between filters, and
    /// after another blur, whose field the wrapping stage reads past the set's edges.
    #[test]
    fn wrapping_blur_and_bevel_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let blur = Filter::Blur { radius: 160.0, wrap: true };
        let normal = BlendMode::Normal;
        let cases = [
            ("wrapped Blur", vec![(blur.clone(), 1.0, normal)], normal),
            ("wrapped Blur under Multiply", vec![(blur.clone(), 0.7, normal)], BlendMode::Multiply),
            ("wrapped Blur disabled", vec![(blur.clone(), 1.0, BlendMode::Disable)], normal),
            (
                "wrapped Blur between filters",
                vec![(Filter::Invert, 1.0, normal), (blur.clone(), 0.8, normal), (Filter::Clamp { min: 0.18, max: 0.86 }, 1.0, normal)],
                normal,
            ),
            ("wrapped Blur after a blur", vec![(Filter::Blur { radius: 24.0, wrap: false }, 1.0, normal), (blur, 1.0, normal)], normal),
            ("wrapped Bevel", vec![(Filter::Bevel { width: 320.0, wrap: true }, 0.9, normal)], normal),
        ];

        assert!(check_cases(&mut gpu, &cases, EDGE_PAINT, CORNER, Tolerance::NEIGHBOURHOOD).is_some());
    }

    /// A Blur with Wrap after a mask generator's blurred ramp at a coarser resolution (its cells)
    /// matches the CPU near the set's edges.
    #[test]
    fn a_wrapping_blur_after_a_blurred_generator_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let stack = vec![(Filter::Blur { radius: 160.0, wrap: true }, 1.0, BlendMode::Normal)];
        let (mut project, pixels, owner) = painted_project(&stack, BlendMode::Normal, (true, SIZE)).unwrap();
        let mut generator = Generator::new(GeneratorKind::Dirt);
        generator.global.blur = 0.85;
        generator.resolution.factor = -3;
        generator.inputs.insert(BakedMap::AmbientOcclusion, Source::image("Occlusion"));
        let mut generated = Effect::new(project.alloc_id(), EffectKind::Generator(generator));
        generated.blend = BlendMode::Multiply;
        project.layers.get_mut(&owner).and_then(|layer| layer.mask.as_mut()).unwrap().effects.insert(1, generated);

        // An occlusion that changes across the set, so the generator's cells differ
        let occlusion: Vec<[f32; 4]> = (0..64).map(|texel| [0.1 + 0.8 * ((texel % 8 + texel / 8 * 3) % 8) as f32 / 7.0; 4]).collect();
        let images = ImageStore::from([("Occlusion".into(), Arc::new(ImageRes::new(8, 8, occlusion, false)))]);
        let mesh = oracle_mesh(false);
        let baked = BakedMaps::default();
        let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
        case.mask_owner = Some(owner);
        case.region = Some(CORNER);

        assert_matches_cpu(&mut gpu, &case, Tolerance::NEIGHBOURHOOD, "wrapped Blur after a generator's cells");
    }

    /// Highpass in either sRGB space and of the alpha matches the CPU (colour spaces apply to Base
    /// Color only, the alpha to content only), below full opacity, disabled and between filters.
    #[test]
    fn highpass_spaces_and_alpha_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let highpass = |space, alpha| Filter::HighPass { radius: 96.0, space, alpha };
        let normal = BlendMode::Normal;
        let cases = [
            ("Highpass sRGB from linear", vec![(highpass(HighpassSpace::SrgbFromLinear, false), 1.0, normal)], normal),
            ("Highpass sRGB from ACEScg", vec![(highpass(HighpassSpace::SrgbFromAcescg, false), 0.8, normal)], BlendMode::Multiply),
            ("Highpass alpha", vec![(highpass(HighpassSpace::Working, true), 1.0, normal)], normal),
            ("Highpass sRGB and alpha", vec![(highpass(HighpassSpace::SrgbFromLinear, true), 0.7, normal)], normal),
            ("Highpass disabled", vec![(highpass(HighpassSpace::SrgbFromAcescg, true), 1.0, BlendMode::Disable)], normal),
            (
                "Highpass between filters",
                vec![
                    (Filter::Blur { radius: 24.0, wrap: false }, 1.0, normal),
                    (highpass(HighpassSpace::SrgbFromLinear, true), 0.9, normal),
                    (Filter::Invert, 1.0, normal),
                ],
                normal,
            ),
        ];

        assert!(check_cases(&mut gpu, &cases, MIDDLE_PAINT, MIDDLE, Tolerance::NEIGHBOURHOOD).is_some());
    }
}
