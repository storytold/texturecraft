//! P3a: the generator kinds that read per-texel inputs alone on the GPU: 3D Distance, 3D Linear
//! Gradient and Grayscale Conversion (their own controls). World Space Normals, and any kind on
//! wear, light or section controls, take P1's path (gpu_generator_p1.rs).
//!
//! They share the generator opcode [`GENERATOR_KIND_OPCODE`] with the kind in `procedural[0]`,
//! evaluated by `generator_p3a_ramp` in generator_p3a.wgsl. The operation layout per kind
//! (`low.w` is the effect's opacity and `uv[1]` the curvature detail, so neither holds a control):
//! - 3D Distance: `low.xyz` the point, `high.xyz` the radius, offset and contrast
//! - 3D Linear Gradient: `low.xyz` the start, `high.xyz` the end, `uv[2..]` the balance and contrast
//! - Grayscale Conversion: `high` the rotation's cosine and sine, the tile and the gray mode,
//!   `uv[2..]` the balance and contrast, `procedural[1]` the image's side-table word plus one
//!
//! `uv[0]` holds the mesh's half diagonal over its longest side ([`bind_bounds`]), which turns
//! the surface position into the CPU's evenly scaled bounds position. `effects[3]` is a replaced
//! input: the Position or World Normal input, or a Grayscale Conversion slot that is no image.

use bytemuck::Zeroable;
use texturecraft_core::{BakedMap, ChannelKind, Generator, GeneratorGrayscale, GeneratorKind, KindControls, Projection, Sampling, Source};
use texturecraft_geom::Mesh;
use texturecraft_geom::glam::Vec3;

use super::{ImageUse, Operation, ProceduralInputs, TilePatterns, bind_tile_source, generator_p1, generator_stage, source_operation};

/// The generator opcode of the kinds P3a and P3b port, the last free one below the extended
/// patterns; the kind is in `procedural[0]`
pub(super) const GENERATOR_KIND_OPCODE: u32 = 47;

/// The kinds in `procedural[0]` (4 was World Space Normals, now on P1's section path; P3b continues from 5)
const DISTANCE_KIND: u32 = 1;
const LINEAR_GRADIENT_KIND: u32 = 2;
const GRAYSCALE_KIND: u32 = 3;

/// The step Safe Rotation turns a Grayscale Conversion image by (QUARTER_TURN_DEGREES in eval.rs)
const QUARTER_TURN_DEGREES: f32 = 90.0;

/// The mesh's smallest half diagonal and bounds side (MIN_MESH_RADIUS in pattern.rs, MIN_BOUNDS_EXTENT in eval.rs)
const MIN_MESH_RADIUS: f32 = 1e-4;
const MIN_BOUNDS_EXTENT: f32 = 1e-6;

/// Whether P3a evaluates generators of `kind`
pub(super) fn ports(kind: GeneratorKind) -> bool {
    matches!(kind, GeneratorKind::ThreeDDistance | GeneratorKind::ThreeDLinearGradient | GeneratorKind::GrayscaleConversion)
}

/// The GPU operation of a generator of a kind P3a ports, or None where the GPU would differ from
/// the CPU, so the stack composes on the CPU
pub(super) fn generator_operation(generator: &Generator, inputs: &mut ProceduralInputs, patterns: &mut TilePatterns) -> Option<Operation> {
    // The CPU computes wear, light and section controls before a kind's own controls (P1 takes those generators first)
    if generator_p1::takes(generator) {
        return None;
    }

    let global = generator.applied_global();
    let mut op = Operation {
        ids: [0, GENERATOR_KIND_OPCODE, generator.seed, u32::from(generator.invert)],
        projection: [0.0, generator.offset, global.balance, global.contrast],
        ..Operation::zeroed()
    };
    let controls = generator.kind_controls();
    let input = match (generator.kind, controls.as_ref()) {
        (GeneratorKind::ThreeDDistance, Some(KindControls::Distance(distance))) => {
            op.procedural[0] = DISTANCE_KIND;
            op.low[..3].copy_from_slice(&distance.point);
            op.high[..3].copy_from_slice(&[distance.radius, distance.offset, distance.contrast]);
            generator.inputs.get(&BakedMap::Position)
        }
        (GeneratorKind::ThreeDLinearGradient, Some(KindControls::LinearGradient(gradient))) => {
            op.procedural[0] = LINEAR_GRADIENT_KIND;
            op.low[..3].copy_from_slice(&gradient.start);
            op.high[..3].copy_from_slice(&gradient.end);
            op.uv[2..].copy_from_slice(&[gradient.balance, gradient.contrast]);
            generator.inputs.get(&BakedMap::Position)
        }
        (GeneratorKind::GrayscaleConversion, Some(KindControls::Grayscale(grayscale))) => {
            let degrees = if grayscale.safe_rotation { (grayscale.rotation / QUARTER_TURN_DEGREES).round() * QUARTER_TURN_DEGREES } else { grayscale.rotation };
            let (sin, cos) = degrees.to_radians().sin_cos();
            let mode = GeneratorGrayscale::ALL.iter().position(|mode| *mode == grayscale.grayscale)? as f32;
            op.procedural[0] = GRAYSCALE_KIND;
            op.high = [cos, sin, grayscale.tile, mode];
            op.uv[2..].copy_from_slice(&[grayscale.balance, grayscale.contrast]);

            // An image reads its stored texels; any other slot reads like a source (an empty slot is black)
            match grayscale.source.as_ref() {
                Some(Source::Image { name, .. }) => {
                    op.procedural[1] = inputs.pack_image(name, ImageUse::Material)?.checked_add(1)?;
                    None
                }
                other => Some(other.unwrap_or(&EMPTY_IMAGE_SLOT)),
            }
        }
        _ => return None,
    };

    // Inputs read through the same source as the CPU's `Texel::source` with the default sampling
    if let Some(source) = input {
        let mut source_op = source_operation(Some(source), &Projection::default(), Sampling::default(), [0.0; 3], ChannelKind::NormalMap, inputs)?;
        bind_tile_source(&mut source_op, Some(source), patterns);
        inputs.generator_sources.push(source_op);
        op.effects[3] = inputs.generator_sources.len() as u32;
    }

    // The output resolution's cells (these kinds apply no global blur)
    let staged = Generator { global, ..generator.clone() };
    op.spatial[2] = u32::from(global.blur > 0.0 || generator.resolution.can_reduce());
    let (blur, cell) = generator_stage(&staged, inputs.resolution, inputs);
    if let (Some(_), _) | (None, 2..) = (blur, cell) {
        inputs.generator_stages.push((blur, cell));
        op.spatial[1] = inputs.generator_stages.len() as u32;
        let levels: Vec<_> = inputs.levels.iter().map(|&resolution| generator_stage(&staged, resolution, inputs)).collect();
        inputs.generator_levels.push(levels);
    }
    op.low.iter().chain(&op.high).chain(&op.projection).chain(&op.uv).all(|value| value.is_finite()).then_some(op)
}

/// What an empty Grayscale Conversion slot reads (EMPTY_IMAGE_SLOT in eval.rs)
const EMPTY_IMAGE_SLOT: Source = Source::Uniform([0.0; 3]);

/// Write the mesh's half diagonal over its longest side into the P3a operations that read the
/// evenly scaled bounds position (`uniform_position_input` in eval.rs)
pub(super) fn bind_bounds(operations: &mut [Operation], mesh: &Mesh) {
    let (low, high) = mesh.bounds();
    let radius = ((high - low).length() * 0.5).max(MIN_MESH_RADIUS);
    let longest = (high - low).max_element().max(MIN_BOUNDS_EXTENT);
    let reads_bounds = |op: &Operation| op.ids[1] == GENERATOR_KIND_OPCODE && matches!(op.procedural[0], DISTANCE_KIND | LINEAR_GRADIENT_KIND);
    for op in operations.iter_mut().filter(|op| reads_bounds(op)) {
        op.uv[0] = radius / longest;
    }

    // P1: the position across the bounds per axis (`bounds_position` in eval.rs), as `low.xyz + position × high.xyz`
    let extent = (high - low).max(Vec3::splat(MIN_BOUNDS_EXTENT));
    let (offset, scale) = (((high - low) * 0.5 / extent).to_array(), (Vec3::splat(radius) / extent).to_array());
    for op in operations.iter_mut().filter(|op| super::generator_p1::reads_bounds(op)) {
        op.low[..3].copy_from_slice(&offset);
        op.high[..3].copy_from_slice(&scale);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use texturecraft_compose::{BakedMaps, ImageRes, ImageStore, PixelStore};
    use texturecraft_core::{
        AxisBlend, AxisLayer, BakedMap, BlendMode, Channel, DistanceParameters, Effect, EffectKind, Fill, Generator, GeneratorGrayscale, GeneratorKind,
        GrayscaleConversionParameters, KindControls, Layer, LayerId, LayerKind, LightParameters, LinearGradientParameters, Mask, MaskBackground,
        OutputResolution, Procedural, ProceduralKind, Project, SectionParameters, Source, TextureSet, WearParameters,
    };
    use texturecraft_geom::glam::Vec3;
    use texturecraft_geom::{Mesh, primitives};
    use texturecraft_texel::TILE;

    use crate::gpu_composite::CompositeProgram;
    use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu, headless};
    use crate::headless::Headless;

    /// Texture set side of the oracle cases
    const SIZE: u32 = TILE * 2;

    /// Texels a hard step may flip: 0.1 % of the set
    const FLIPS: usize = (SIZE * SIZE / 1000) as usize;

    /// A sphere stretched and moved off the origin, so its bounds differ on every axis and its
    /// normals turn everywhere
    fn stretched_sphere() -> Arc<Mesh> {
        let mut mesh = primitives::sphere(1.0, 48, 24);
        for position in &mut mesh.positions {
            *position = *position * Vec3::new(1.6, 0.9, 0.6) + Vec3::new(0.3, -0.2, 0.1);
        }
        Arc::new(mesh)
    }

    /// A non-square sRGB colour image, and a float image read as a Position input
    fn images() -> ImageStore {
        const COLOR_SIZE: (u32, u32) = (48, 32);
        const POSITION_SIDE: u32 = 16;
        let color = (0..COLOR_SIZE.0 * COLOR_SIZE.1)
            .map(|index| {
                let (x, y) = (index % COLOR_SIZE.0, index / COLOR_SIZE.0);
                [x as f32 / 47.0, y as f32 / 31.0, ((x / 4 + y / 3) % 2) as f32 * 0.8 + 0.1, 1.0]
            })
            .collect();
        let position = (0..POSITION_SIDE * POSITION_SIDE)
            .map(|index| {
                let (x, y) = (index % POSITION_SIDE, index / POSITION_SIDE);
                [x as f32 / 15.0, y as f32 / 15.0, ((x + y) % 5) as f32 / 4.0, 1.0]
            })
            .collect();
        ImageStore::from([
            ("color".into(), Arc::new(ImageRes::new(COLOR_SIZE.0, COLOR_SIZE.1, color, true))),
            ("position".into(), Arc::new(ImageRes::new(POSITION_SIDE, POSITION_SIDE, position, false))),
        ])
    }

    /// The anchor point on the layer below, recording its clouds
    const BELOW_ANCHOR: LayerId = LayerId(950);

    /// Clouds between 0.1 and 0.9
    fn clouds() -> Source {
        let pattern = Procedural { scale: 3.7, seed: 19, ..Procedural::new(ProceduralKind::Clouds) };
        Source::Procedural { pattern, low: [0.1; 3], high: [0.9; 3], ramp: Vec::new() }
    }

    /// The colour [`BELOW_ANCHOR`] recorded, as an input
    fn below_anchor() -> Source {
        Source::Anchor { anchor: BELOW_ANCHOR, channel: Channel::BaseColor, ramp: Vec::new() }
    }

    /// A white layer over clouds with an anchor point, masked by `generator` blended `blend` at
    /// `opacity` over a gray background fill: the project and the masked layer
    fn generator_project(generator: Generator, blend: BlendMode, opacity: f32) -> (Project, LayerId) {
        let channels = [Channel::BaseColor];
        let mut project = Project::new("P3a generators", "builtin:sphere");
        project.texture_sets.push(TextureSet::new("Set", 0, SIZE));
        let fill = |source: Source| LayerKind::Fill(Fill { sources: BTreeMap::from([(Channel::BaseColor, source)]), ..Fill::default() });
        let below = project.alloc_id();
        let mut below = Layer::new(below, "Below", fill(clouds()), &channels);
        below.content_effects.push(Effect::new(BELOW_ANCHOR, EffectKind::Anchor));
        assert!(project.insert_layer(0, below, None, false).is_ok());

        let owner = project.alloc_id();
        let mut layer = Layer::new(owner, "Masked", fill(Source::Uniform([1.0; 3])), &channels);
        let mut mask = Mask::new(MaskBackground::Black);
        let under = EffectKind::Fill { source: Source::Uniform([0.4; 3]), projection: Default::default(), sampling: Default::default() };
        mask.effects.push(Effect::new(project.alloc_id(), under));
        let mut generated = Effect::new(project.alloc_id(), EffectKind::Generator(generator));
        generated.blend = blend;
        generated.opacity = opacity;
        mask.effects.push(generated);
        layer.mask = Some(mask);
        assert!(project.insert_layer(0, layer, None, false).is_ok());
        (project, owner)
    }

    /// A generator of `kind` with `controls`
    fn with_controls(kind: GeneratorKind, controls: KindControls) -> Generator {
        Generator { controls: Some(Box::new(controls)), ..Generator::new(kind) }
    }

    /// Section controls that leave the signal as it is, with `axes`
    fn neutral_section(axes: [AxisLayer; 3]) -> SectionParameters {
        SectionParameters { invert: false, blur: 0.0, balance: 0.5, contrast: 0.0, brightness: 0.0, axes }
    }

    /// `generator` compiles for the GPU and matches the CPU in the material and its mask preview
    fn assert_generator_matches(
        gpu: &mut Headless,
        mesh: &Arc<Mesh>,
        baked: &BakedMaps,
        (generator, blend, opacity): (Generator, BlendMode, f32),
        tolerance: Tolerance,
        label: &str,
    ) {
        let (project, owner) = generator_project(generator, blend, opacity);
        let (pixels, images) = (PixelStore::default(), images());
        let mut case = OracleCase::new(&project, &pixels, baked, &images, mesh);
        case.mask_owner = Some(owner);
        assert_matches_cpu(gpu, &case, tolerance, label);
    }

    /// 3D Distance and 3D Linear Gradient match the CPU on a mesh whose bounds differ per axis:
    /// off-centre points, balance and contrast, invert and offset, a Position image input, a
    /// reduced output resolution and a hard step; and on wear, light or section controls, which
    /// the CPU computes ahead of their own (P1's paths)
    #[test]
    fn three_d_distance_and_linear_gradient_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let (mesh, baked) = (stretched_sphere(), BakedMaps::default());
        let distance = DistanceParameters { point: [0.3, 0.6, 0.45], radius: 0.7, offset: 0.4, contrast: 0.3 };
        let gradient = LinearGradientParameters { start: [0.2, 0.1, 0.7], end: [0.8, 0.9, 0.3], balance: 0.4, contrast: 0.2 };
        let inverted = Generator { invert: true, offset: -0.1, ..with_controls(GeneratorKind::ThreeDDistance, KindControls::Distance(distance)) };
        let reduced = Generator {
            resolution: OutputResolution { factor: -1, ..OutputResolution::default() },
            ..with_controls(GeneratorKind::ThreeDLinearGradient, KindControls::LinearGradient(gradient))
        };
        let mut position_input = with_controls(GeneratorKind::ThreeDDistance, KindControls::Distance(distance));
        position_input.inputs.insert(BakedMap::Position, Source::image("position"));
        let hard = DistanceParameters { contrast: 1.0, ..distance };
        let mut anchored = with_controls(GeneratorKind::ThreeDLinearGradient, KindControls::LinearGradient(gradient));
        anchored.inputs.insert(BakedMap::Position, below_anchor());
        let mut patterned = with_controls(GeneratorKind::ThreeDDistance, KindControls::Distance(distance));
        patterned.inputs.insert(BakedMap::Position, clouds());

        // Wear, light and section controls come before a kind's own (P1's paths)
        let without_grunge = WearParameters::defaults(GeneratorKind::EdgeWear).map(|wear| WearParameters { grunge_amount: 0.0, ..wear });
        let worn = Generator { wear: without_grunge, ..with_controls(GeneratorKind::ThreeDDistance, KindControls::Distance(distance)) };
        let lit = Generator { light: LightParameters::defaults(GeneratorKind::Direction), ..Generator::new(GeneratorKind::ThreeDLinearGradient) };
        let section =
            SectionParameters { invert: true, brightness: -0.3, ..neutral_section([AxisLayer { invert: false, blend: AxisBlend::Add, weight: 1.0 }; 3]) };
        let sectioned = Generator { section: Some(section), ..Generator::new(GeneratorKind::ThreeDDistance) };
        let cases = [
            ("distance on wear controls", (worn, BlendMode::Normal, 1.0), Tolerance::POINT),
            ("gradient on light controls", (lit, BlendMode::Normal, 1.0), Tolerance::POINT),
            ("distance on section controls", (sectioned, BlendMode::Multiply, 0.7), Tolerance::POINT),
            ("distance, inverted with an offset", (inverted, BlendMode::Normal, 1.0), Tolerance::POINT),
            (
                "gradient, multiplied",
                (with_controls(GeneratorKind::ThreeDLinearGradient, KindControls::LinearGradient(gradient)), BlendMode::Multiply, 0.8),
                Tolerance::POINT,
            ),
            ("gradient at half resolution", (reduced, BlendMode::Normal, 1.0), Tolerance::NEIGHBOURHOOD),
            ("distance from a Position image", (position_input, BlendMode::Normal, 1.0), Tolerance::BILINEAR),
            ("gradient from an anchor point", (anchored, BlendMode::Normal, 1.0), Tolerance::POINT),
            ("distance from a pattern", (patterned, BlendMode::Normal, 1.0), Tolerance::POINT),
            (
                "distance as a hard step",
                (with_controls(GeneratorKind::ThreeDDistance, KindControls::Distance(hard)), BlendMode::Normal, 1.0),
                Tolerance::POINT.with_flips(FLIPS),
            ),
        ];
        for (label, generator, tolerance) in cases {
            assert_generator_matches(&mut gpu, &mesh, &baked, generator, tolerance, label);
        }
    }

    /// Grayscale Conversion matches the CPU in every gray mode, tiled and turned with and without
    /// Safe Rotation, balanced and contrasted, and with an image not loaded, an empty slot and a
    /// uniform slot
    #[test]
    fn grayscale_conversion_matches_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let (mesh, baked) = (stretched_sphere(), BakedMaps::default());
        let image = |grayscale, rotation, safe_rotation, tile| GrayscaleConversionParameters {
            grayscale,
            rotation,
            safe_rotation,
            tile,
            source: Some(Source::image("color")),
            ..GrayscaleConversionParameters::default()
        };
        let mut cases: Vec<(String, GrayscaleConversionParameters)> = GeneratorGrayscale::ALL
            .into_iter()
            .enumerate()
            .map(|(index, mode)| (format!("{mode:?}"), image(mode, 37.0 + 41.0 * index as f32, index % 2 == 1, 1.0 + (index % 3) as f32)))
            .collect();
        cases.push((
            "balanced and contrasted".into(),
            GrayscaleConversionParameters { balance: 0.3, contrast: 0.4, ..image(GeneratorGrayscale::Luma, 15.0, false, 2.0) },
        ));
        cases.push(("image not loaded".into(), GrayscaleConversionParameters { source: Some(Source::image("absent")), ..Default::default() }));
        cases.push(("empty slot".into(), GrayscaleConversionParameters { balance: 0.2, ..Default::default() }));
        cases.push((
            "anchor slot".into(),
            GrayscaleConversionParameters { source: Some(below_anchor()), grayscale: GeneratorGrayscale::Luma, ..Default::default() },
        ));
        cases.push(("pattern slot".into(), GrayscaleConversionParameters { source: Some(clouds()), contrast: 0.3, ..Default::default() }));
        cases.push((
            "uniform slot".into(),
            GrayscaleConversionParameters { source: Some(Source::Uniform([0.2, 0.7, 0.4])), grayscale: GeneratorGrayscale::Max, ..Default::default() },
        ));
        for (label, grayscale) in cases {
            let generator = with_controls(GeneratorKind::GrayscaleConversion, KindControls::Grayscale(grayscale));
            assert_generator_matches(&mut gpu, &mesh, &baked, (generator, BlendMode::Normal, 1.0), Tolerance::BILINEAR, &label);
        }
    }

    /// World Space Normals' axes match the CPU from the surface normals, a baked World Normal
    /// map and a replaced input, with every blend, an inverted axis and partial weights, and with
    /// its section adjusted between its section blur and a global blur (P1's section path)
    #[test]
    fn world_space_normals_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };
        let mesh = stretched_sphere();
        let axis = |invert, blend, weight| AxisLayer { invert, blend, weight };
        let sections = [
            [axis(true, AxisBlend::Multiply, 0.6), axis(false, AxisBlend::Normal, 1.0), axis(false, AxisBlend::Max, 0.4)],
            [axis(false, AxisBlend::Subtract, 0.7), axis(true, AxisBlend::Add, 0.9), axis(false, AxisBlend::AddSub, 0.5)],
        ];
        let generator = |axes| Generator { section: Some(neutral_section(axes)), ..Generator::new(GeneratorKind::WorldSpaceNormals) };
        let stored = (0..SIZE * SIZE).flat_map(|index| [(index % SIZE) as f32 / SIZE as f32, 0.3 + 0.4 * ((index / SIZE) % 7) as f32 / 6.0, 0.8]).collect();
        let world_normal = BakedMaps { resolution: SIZE, maps: BTreeMap::from([(BakedMap::WorldNormal, Arc::new(stored))]) };
        let mut replaced = generator(sections[1]);
        replaced.inputs.insert(BakedMap::WorldNormal, Source::Uniform([0.3, 0.8, 0.55]));
        let mut anchored = generator(sections[0]);
        anchored.inputs.insert(BakedMap::WorldNormal, below_anchor());
        let mut patterned = generator(sections[1]);
        patterned.inputs.insert(BakedMap::WorldNormal, clouds());
        let adjusted = SectionParameters { balance: 0.7, contrast: 0.25, brightness: 0.2, invert: true, blur: 6.0, axes: sections[0] };
        let mut blurred_twice = Generator { section: Some(adjusted), ..Generator::new(GeneratorKind::WorldSpaceNormals) };
        blurred_twice.global.blur = 0.2;
        let cases = [
            ("section adjusted and blurred twice", blurred_twice, BakedMaps::default()),
            ("surface normals", generator(sections[0]), BakedMaps::default()),
            ("surface normals, other blends", generator(sections[1]), BakedMaps::default()),
            ("baked World Normal", generator(sections[0]), world_normal),
            ("replaced input", replaced, BakedMaps::default()),
            ("anchor point input", anchored, BakedMaps::default()),
            ("pattern input", patterned, BakedMaps::default()),
        ];
        for (label, generator, baked) in cases {
            assert_generator_matches(&mut gpu, &mesh, &baked, (generator, BlendMode::Normal, 1.0), Tolerance::POINT, label);
        }
    }

    /// A kind's wear controls with grunge stay on the CPU until the grunge is on the GPU (P2)
    #[test]
    fn controls_the_gpu_does_not_evaluate_compose_on_the_cpu() {
        let worn = Generator { wear: texturecraft_core::WearParameters::defaults(GeneratorKind::EdgeWear), ..Generator::new(GeneratorKind::ThreeDDistance) };
        let (project, _) = generator_project(worn, BlendMode::Normal, 1.0);
        let compiled = CompositeProgram::try_compile_region(&project, &PixelStore::default(), 0, None);
        assert_eq!(compiled.err(), Some("A mask generator uses CPU compositing."));
    }
}
