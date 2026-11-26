//! The wear, section and light controls of the generators with an earlier model on the GPU, as
//! compose's `generator_ramp` evaluates them (wear.rs, wear_scan.rs, sections.rs).
//!
//! The operation keeps its kind's opcode (40..46) and `Operation::ext[0]` holds the offset of its
//! side-table words (`P1_*` in generator_p1.wgsl): a nonzero offset means the controls. Its
//! sources (replaced inputs) follow each other in the generator sources, the first in
//! `effects[2]`. A section adjusts its signal before the generator's stage, or after it when the
//! stage averages cells or runs the section blur; a stage that would need both the section blur
//! and the global blur, or cells and the global blur around the adjustment, stays on the CPU.

use bytemuck::Zeroable;
use texturecraft_compose::generator_terms;
use texturecraft_core::{
    AxisBlend, BakedMap, ChannelKind, Filter, Generator, GeneratorKind, LightParameters, Projection, Sampling, SectionParameters, Source, WearParameters,
};
use texturecraft_geom::Mesh;

use super::{FIRST_GENERATOR, NeighbourhoodFilter, Operation, ProceduralInputs, TilePatterns, bind_tile_source, source_operation};

/// The opcode of the last kind with an earlier model; opcodes from FIRST_GENERATOR to it carry the controls
const LAST_CONTROLS_OPCODE: u32 = FIRST_GENERATOR + 6;

const WEAR: u32 = 1;
const SECTION: u32 = 2;
const LIGHT: u32 = 3;

// Words of every controls operation (generator_p1.wgsl)
const MODEL_WORD: usize = 0;
const FLAGS_WORD: usize = 1;
const BOUNDS_WORD: usize = 2;
const SOURCE_COUNT_WORD: usize = 3;
const ROLES_WORD: usize = 4;
const ROLE_COUNT: usize = 3;
const PARAMETERS_WORD: usize = ROLES_WORD + ROLE_COUNT;

const LATE_ADJUST: u32 = 1;
const LEVELS: u32 = 2;
const NO_LEVEL: u32 = u32::MAX;

/// The mesh's bounds the stand-in for P3a's mesh extent reads: low, high, centre and radius
const BOUNDS_WORDS: usize = 10;
/// The shortest mesh radius the UV surface normalises positions by (`GpuCompositor::uv_surface`)
const MIN_MESH_RADIUS: f32 = 1e-4;

// Section flags (the first section parameter)
const HAS_BALANCE: u32 = 1;
const LINEAR_BALANCE: u32 = 2;
const HAS_BRIGHTNESS: u32 = 4;

/// The operation of `generator` on its wear, section or light controls, None for anything the GPU
/// does not evaluate as the CPU does (the caller keeps the texture, grunge and Micro Details declines)
pub(super) fn operation(generator: &Generator, inputs: &mut ProceduralInputs, patterns: &mut TilePatterns) -> Option<Operation> {
    let opcode = match generator.kind {
        GeneratorKind::EdgeWear | GeneratorKind::FiberGlassEdgeWear => 40,
        GeneratorKind::Dirt => 41,
        GeneratorKind::Curvature => 43,
        GeneratorKind::AmbientOcclusion => 44,
        GeneratorKind::Position => 45,
        GeneratorKind::Direction => 46,
        _ => return None,
    };
    let mut words = vec![0u32; PARAMETERS_WORD];
    let mut sources = Vec::new();
    let mut late_adjust = false;

    // The CPU picks the wear controls first, then the light's, then the section's
    let wear = generator.wear.or_else(|| WearParameters::defaults(generator.kind).filter(|_| generator.kind == GeneratorKind::FiberGlassEdgeWear));
    if let Some(wear) = wear {
        wear_words(generator, &wear, &mut words, &mut sources, inputs)?;
    } else if let Some(light) = &generator.light {
        light_words(generator, light, &mut words, &mut sources, inputs)?;
    } else {
        let section = generator.section_controls()?;
        late_adjust = section_words(generator, &section, &mut words, &mut sources, inputs)?;
    }

    // The bounds-normalised position the section's Position and the light read
    if matches!(generator.kind, GeneratorKind::Position | GeneratorKind::Direction) {
        words[BOUNDS_WORD] = words.len() as u32;
        words.extend([0; BOUNDS_WORDS]);
    }

    let (first_source, count) = push_sources(&sources, inputs, patterns)?;
    words[SOURCE_COUNT_WORD] = count;
    let global = generator.applied_global();
    let offset = if generator.offset.is_finite() { generator.offset } else { 0.0 };
    let mut op = Operation {
        ids: [0, opcode, generator.seed, u32::from(generator.invert && generator.light.is_none())],
        projection: [0.0, offset, global.balance, global.contrast],
        effects: [0, 0, first_source, 0],
        ..Operation::zeroed()
    };

    // The CPU's region path: blurs and reduced resolutions need neighbours
    let section_blur = generator.section_controls().is_some_and(|section| {
        SectionParameters::has_blur(generator.kind) && section.blur.clamp(*SectionParameters::BLUR_RANGE.start(), *SectionParameters::BLUR_RANGE.end()) > 0.0
    });
    op.spatial[2] = u32::from(global.blur > 0.0 || generator.resolution.can_reduce() || section_blur);
    let stages: Vec<_> =
        std::iter::once(inputs.resolution).chain(inputs.levels.iter().copied()).map(|resolution| stage(generator, resolution, inputs)).collect();
    let stages = stages.into_iter().collect::<Option<Vec<_>>>()?;
    let late = stages.iter().any(|(late, _, _)| *late);
    if let [(_, blur, cell), levels @ ..] = stages.as_slice()
        && (blur.is_some() || *cell >= 2)
    {
        inputs.generator_stages.push((*blur, *cell));
        op.spatial[1] = inputs.generator_stages.len() as u32;
        inputs.generator_levels.push(levels.iter().map(|&(_, blur, cell)| (blur, cell)).collect());
    }

    words[FLAGS_WORD] |= if late_adjust && late { LATE_ADJUST } else { 0 };
    op.ext[0] = inputs.pack_words(&words)?;
    op.projection.iter().all(|value| value.is_finite()).then_some(op)
}

/// The generator's stage at a UV tile of `resolution` texels: whether its section adjusts after the
/// stage, the blur and the cell side. None where the adjustment would fall between two passes.
fn stage(generator: &Generator, resolution: u32, inputs: &ProceduralInputs) -> Option<(bool, Option<NeighbourhoodFilter>, u32)> {
    let cell = generator.resolution.cell(resolution, inputs.texture_set);
    let global = generator.applied_global();
    let global_blur = global.blur_radius_at(resolution).round() > 0.0;
    let section_radius = generator.section_controls().map_or(0, |section| generator_terms::section_blur_radius(generator.kind, &section, resolution));
    let late = generator.section_controls().is_some() && (cell > 1 || section_radius > 0);
    if late && global_blur {
        return None;
    }

    let blur = if section_radius > 0 {
        // The section blur's three box passes at the radius compose takes
        let blur = Filter::Blur { radius: 1.0, wrap: false };
        NeighbourhoodFilter::new(&blur, resolution, inputs.longest_side, 1.0, None)
            .map(|filter| NeighbourhoodFilter { radius: section_radius as u32, ..filter })
    } else if global_blur {
        let blur = Filter::Blur { radius: global.blur_radius(), wrap: false };
        NeighbourhoodFilter::new(&blur, resolution, inputs.longest_side, 1.0, None)
    } else {
        None
    };
    Some((late, blur, cell))
}

/// A replaced input the generator reads: the source, how the CPU reads it (sampling and channel kind), and its role
type Input<'a> = (&'a Source, Sampling, ChannelKind, usize);

/// The generator's replacement for `map`, read as the CPU reads it in `role`
fn input(generator: &Generator, map: BakedMap, role: usize) -> Option<Input<'_>> {
    let source = generator.inputs.get(&map)?;
    let (sampling, channel) = match map {
        // `generator_input`: the earlier sampling for the scalar maps
        BakedMap::Curvature | BakedMap::AmbientOcclusion | BakedMap::Thickness => (Sampling::SAVED_BEFORE, ChannelKind::Scalar),
        _ => (Sampling::default(), ChannelKind::NormalMap),
    };
    Some((source, sampling, channel, role))
}

/// Each input's source operation in a row of generator sources: the first's index plus one and the count
fn push_sources(sources: &[Input], inputs: &mut ProceduralInputs, patterns: &mut TilePatterns) -> Option<(u32, u32)> {
    if sources.is_empty() {
        return Some((0, 0));
    }

    let first = inputs.generator_sources.len() as u32 + 1;
    for &(source, sampling, channel, _) in sources {
        let mut op = source_operation(Some(source), &Projection::default(), sampling, [0.0; 3], channel, inputs)?;
        bind_tile_source(&mut op, Some(source), patterns);
        inputs.generator_sources.push(op);
    }
    Some((first, sources.len() as u32))
}

fn role_word<'a>(words: &mut [u32], sources: &mut Vec<Input<'a>>, input: Option<Input<'a>>) {
    if let Some(input) = input {
        words[ROLES_WORD + input.3] = sources.len() as u32 + 1;
        sources.push(input);
    }
}

/// The wear words: its variant, the curvature levels around its sigma and its controls as saved
fn wear_words<'a>(
    generator: &'a Generator,
    wear: &WearParameters,
    words: &mut Vec<u32>,
    sources: &mut Vec<Input<'a>>,
    inputs: &mut ProceduralInputs,
) -> Option<()> {
    let variant = match generator.kind {
        GeneratorKind::EdgeWear => 0,
        GeneratorKind::FiberGlassEdgeWear => 1,
        GeneratorKind::Dirt => 2,
        _ => return None,
    };

    // P2: the grunge (its own pattern or a custom one) plugs in at `generator_grunge`; without it the grunge term is 0
    if wear.grunge_amount != 0.0 {
        return None;
    }

    words[MODEL_WORD] = WEAR;
    role_word(words, sources, input(generator, BakedMap::Curvature, 0));
    role_word(words, sources, input(generator, BakedMap::AmbientOcclusion, 1));

    // Blurred levels are copies of the mesh map, so a replaced curvature input reads at its own size only
    let replaced = generator.inputs.contains_key(&BakedMap::Curvature);
    if !replaced {
        words[FLAGS_WORD] |= LEVELS;
        for (size, needed) in inputs.curvature_sizes.iter_mut().zip(generator_terms::wear_levels_in_use(generator.kind, wear)) {
            *size |= needed;
        }
    }
    let (narrower, wider, weight) = generator_terms::wear_levels_around(generator.kind, wear);
    let values = [wear.level, wear.contrast, wear.grunge_amount, wear.ao_masking, wear.curvature_weight, wear.edges_masking];
    if !weight.is_finite() || values.iter().any(|value| !value.is_finite()) {
        return None;
    }
    words.extend([variant, narrower.map_or(NO_LEVEL, |level| level as u32), wider as u32, weight.to_bits()]);
    words.extend(values.map(f32::to_bits));
    Some(())
}

/// The section words: its flags and adjustments, the curvature detail or the axes, and its stage
/// position; true when the adjustment can follow the stage
fn section_words<'a>(
    generator: &'a Generator,
    section: &SectionParameters,
    words: &mut Vec<u32>,
    sources: &mut Vec<Input<'a>>,
    inputs: &mut ProceduralInputs,
) -> Option<bool> {
    let kind = generator.kind;
    let signal_map = match kind {
        GeneratorKind::Curvature => BakedMap::Curvature,
        GeneratorKind::AmbientOcclusion => BakedMap::AmbientOcclusion,
        GeneratorKind::Position => BakedMap::Position,
        _ => return None,
    };
    words[MODEL_WORD] = SECTION;
    role_word(words, sources, input(generator, signal_map, 0));

    let mut flags = 0;
    if SectionParameters::has_balance(kind) {
        flags |= HAS_BALANCE | if kind == GeneratorKind::AmbientOcclusion { LINEAR_BALANCE } else { 0 };
    }
    if SectionParameters::has_brightness(kind) {
        flags |= HAS_BRIGHTNESS;
    }
    let finite_or = |value: f32, range: std::ops::RangeInclusive<f32>| value.is_finite().then(|| value.clamp(*range.start(), *range.end()));
    let balance = finite_or(section.balance, 0.0..=1.0)?;
    let contrast = finite_or(section.contrast, 0.0..=1.0)?;
    let brightness = finite_or(section.brightness, SectionParameters::BRIGHTNESS_RANGE)?;
    let power = generator_terms::section_balance_power(balance);

    // The curvature detail at its sizes, the earlier sizes' blurred levels unless the curvature is replaced
    let (weights, mode) = match generator.detail_in_use().filter(|_| kind == GeneratorKind::Curvature) {
        Some(detail) => {
            let [offset, mode] = inputs.pack_curvature(&detail, !generator.inputs.contains_key(&BakedMap::Curvature))?;
            (offset, mode as u32)
        }
        None => (0.0, 0),
    };
    words.extend([flags, balance.to_bits(), power.to_bits(), contrast.to_bits(), brightness.to_bits(), u32::from(section.invert), weights.to_bits(), mode]);
    for axis in &section.axes {
        let blend = match axis.blend {
            AxisBlend::Normal => 0,
            AxisBlend::Add => 1,
            AxisBlend::Subtract => 2,
            AxisBlend::Multiply => 3,
            AxisBlend::AddSub => 4,
            AxisBlend::Max => 5,
        };
        words.extend([u32::from(axis.invert), blend, axis.weight.to_bits()]);
    }
    (power.is_finite() && section.axes.iter().all(|axis| axis.weight.is_finite())).then_some(true)
}

/// The light words: its direction, glossiness threshold, highlight and attenuation; it reads the
/// baked world normal unless replaced
fn light_words<'a>(
    generator: &'a Generator,
    light: &LightParameters,
    words: &mut Vec<u32>,
    sources: &mut Vec<Input<'a>>,
    inputs: &mut ProceduralInputs,
) -> Option<()> {
    if generator.kind != GeneratorKind::Direction {
        return None;
    }

    words[MODEL_WORD] = LIGHT;
    role_word(words, sources, input(generator, BakedMap::WorldNormal, 0));
    role_word(words, sources, input(generator, BakedMap::Position, 1));
    if !generator.inputs.contains_key(&BakedMap::WorldNormal) {
        inputs.read_bake_rows();
    }

    let direction = generator_terms::light_direction(light.horizontal, light.vertical);
    let glossiness = light.glossiness.clamp(*LightParameters::GLOSSINESS_RANGE.start(), *LightParameters::GLOSSINESS_RANGE.end());
    let threshold = 2.0 * glossiness - 1.0;
    let highlight_level = light.highlight_level.clamp(0.0, 1.0);
    let attenuation = light.attenuation.clamp(0.0, 1.0);
    let values = [direction.x, direction.y, direction.z, threshold, 1.0 - threshold, highlight_level, 1.0 - highlight_level, attenuation];
    if values.iter().any(|value| !value.is_finite()) {
        return None;
    }
    words.extend(values.map(f32::to_bits));
    words.push(u32::from(generator.invert));
    Some(())
}

/// P1 stand-in for P3a's mesh extent (replace at integration): write `mesh`'s bounds into the
/// words of every controls operation that reads them
pub(super) fn set_bounds(operations: &[Operation], ramp_words: &mut [u32], mesh: &Mesh) {
    let (low, high) = mesh.bounds();
    let center = (low + high) * 0.5;
    let radius = ((high - low).length() * 0.5).max(MIN_MESH_RADIUS);
    let bounds = [low.x, low.y, low.z, high.x, high.y, high.z, center.x, center.y, center.z, radius].map(f32::to_bits);
    for op in operations.iter().filter(|op| (FIRST_GENERATOR..=LAST_CONTROLS_OPCODE).contains(&op.ids[1]) && op.ext[0] != 0) {
        let Some(table) = (op.ext[0] as usize).checked_sub(super::LOOKUP_WORDS as usize) else { continue };
        let Some(&bounds_word) = ramp_words.get(table + BOUNDS_WORD) else { continue };
        if bounds_word == 0 {
            continue;
        }
        if let Some(target) = ramp_words.get_mut(table + bounds_word as usize..table + bounds_word as usize + BOUNDS_WORDS) {
            target.copy_from_slice(&bounds);
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::collections::BTreeMap;
    use std::f32::consts::TAU;
    use std::sync::Arc;

    use texturecraft_compose::{BakedMaps, ImageRes, ImageStore, PixelStore};
    use texturecraft_core::{
        AxisBlend, AxisLayer, BakedMap, Channel, Effect, EffectKind, Fill, Generator, GeneratorKind, Layer, LayerKind, LightParameters, Mask, MaskBackground,
        Project, SectionParameters, Source, TextureSet, WearParameters,
    };
    use texturecraft_geom::{Mesh, primitives};

    use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu, headless};

    const SIZE: u32 = 64;
    const BAKE_SIZE: usize = 64;

    /// Smooth occlusion, curvature waves the blurred levels flatten differently, and a world
    /// normal that is zero (the mesh's normal) along the left edge
    fn bakes() -> BakedMaps {
        let at = |index: usize| ((index % BAKE_SIZE) as f32 / BAKE_SIZE as f32, (index / BAKE_SIZE) as f32 / BAKE_SIZE as f32);
        let texels = 0..BAKE_SIZE * BAKE_SIZE;
        let occlusion = texels.clone().map(|index| 0.15 + 0.8 * at(index).0 * (1.0 - 0.5 * at(index).1)).collect::<Vec<_>>();
        let curvature = texels.clone().map(|index| 0.5 + 0.3 * (at(index).0 * TAU * 3.0).sin() * (at(index).1 * TAU * 2.0).cos()).collect::<Vec<_>>();
        let normal = |(u, v): (f32, f32)| if u < 0.1 { [0.5; 3] } else { [0.5 + 0.3 * (v * TAU).sin(), 0.8, 0.5 + 0.3 * (u * TAU).cos()] };
        let maps = BTreeMap::from([
            (BakedMap::AmbientOcclusion, Arc::new(occlusion)),
            (BakedMap::Curvature, Arc::new(curvature)),
            (BakedMap::WorldNormal, Arc::new(texels.flat_map(|index| normal(at(index))).collect::<Vec<_>>())),
        ]);
        BakedMaps { resolution: BAKE_SIZE as u32, maps }
    }

    /// An 8 × 8 image named `name` whose channels run across it
    fn images() -> ImageStore {
        let texels = (0..64).map(|texel| [(texel % 8) as f32 / 7.0, (texel / 8) as f32 / 7.0, 0.3 + 0.05 * (texel % 5) as f32, 1.0]).collect();
        ImageStore::from([("Input".into(), Arc::new(ImageRes::new(8, 8, texels, false)))])
    }

    /// Each generator alone in a black mask on a white fill, on the CPU and the GPU: the material and the mask preview
    #[allow(clippy::unwrap_used)] // A test helper: a project that does not build fails the test
    fn assert_generators_match(mesh: &Arc<Mesh>, baked: &BakedMaps, cases: &[(&str, Generator, Tolerance)]) {
        let Some(mut gpu) = headless() else { return };
        let images = images();
        let pixels = PixelStore::default();
        for (label, generator, tolerance) in cases {
            let mut project = Project::new(*label, "builtin:sphere");
            project.texture_sets.push(TextureSet::new("Set", 0, SIZE));
            let owner = project.alloc_id();
            let fill = Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([1.0; 3]))]), ..Default::default() };
            let mut layer = Layer::new(owner, "Masked", LayerKind::Fill(fill), &[Channel::BaseColor]);
            let mut mask = Mask::new(MaskBackground::Black);
            mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Generator(generator.clone())));
            layer.mask = Some(mask);
            project.insert_layer(0, layer, None, false).unwrap();

            let mut case = OracleCase::new(&project, &pixels, baked, &images, mesh);
            case.mask_owner = Some(owner);
            assert_matches_cpu(&mut gpu, &case, *tolerance, label);
        }
    }

    #[allow(clippy::unwrap_used)] // A test helper: the wear kinds have defaults
    fn wear(kind: GeneratorKind, change: impl FnOnce(&mut WearParameters)) -> Generator {
        let mut wear = WearParameters { grunge_amount: 0.0, ..WearParameters::defaults(kind).unwrap() };
        change(&mut wear);
        Generator { wear: Some(wear), ..Generator::new(kind) }
    }

    #[allow(clippy::unwrap_used)] // A test helper: the section kinds have defaults
    fn section(kind: GeneratorKind, change: impl FnOnce(&mut SectionParameters)) -> Generator {
        let mut section = SectionParameters::defaults(kind).unwrap();
        change(&mut section);
        Generator { section: Some(section), ..Generator::new(kind) }
    }

    #[allow(clippy::unwrap_used)] // A test helper: the Light generator has defaults
    fn light(change: impl FnOnce(&mut LightParameters)) -> Generator {
        let mut light = LightParameters::defaults(GeneratorKind::Direction).unwrap();
        change(&mut light);
        Generator { light: Some(light), ..Generator::new(GeneratorKind::Direction) }
    }

    /// The three wear kinds through their levels around each sigma (the map itself, the first
    /// levels, the widest two), the masking's eased low end, a hard step, replaced inputs and cells
    #[test]
    fn wear_controls_match_the_cpu() {
        let mesh = Arc::new(primitives::sphere(1.0, 16, 8));
        let mut replaced = wear(GeneratorKind::Dirt, |wear| wear.edges_masking = 0.8);
        replaced.inputs.insert(BakedMap::Curvature, Source::image("Input"));
        replaced.inputs.insert(BakedMap::AmbientOcclusion, Source::Uniform([0.62; 3]));
        replaced.resolution.factor = -2;
        replaced.invert = true;
        replaced.offset = -0.15;
        let cases = [
            ("Metal edge wear", wear(GeneratorKind::EdgeWear, |_| {}), Tolerance::POINT),
            (
                "smooth edges, light masking",
                wear(GeneratorKind::EdgeWear, |wear| {
                    wear.edges_smoothness = 16.0;
                    wear.ao_masking = 0.2;
                    (wear.level, wear.contrast) = (0.4, 0.3);
                }),
                Tolerance::POINT,
            ),
            ("sharp edges, hard step", wear(GeneratorKind::EdgeWear, |wear| (wear.edges_smoothness, wear.contrast) = (0.2, 1.0)), Tolerance::POINT),
            (
                "Fiber glass",
                wear(GeneratorKind::FiberGlassEdgeWear, |wear| (wear.ao_masking, wear.curvature_weight, wear.level) = (0.7, 1.0, 0.9)),
                Tolerance::POINT,
            ),
            ("Dirt", wear(GeneratorKind::Dirt, |wear| wear.level = 0.6), Tolerance::POINT),
            ("Dirt with replaced inputs and cells", replaced, Tolerance::NEIGHBOURHOOD),
        ];

        assert_generators_match(&mesh, &bakes(), &cases);
    }

    /// Curvature, Ambient Occlusion and Position on their section controls: each adjustment, the
    /// adjustment before a global blur and after the section blur or cells, the axes and the inputs
    #[test]
    fn section_controls_match_the_cpu() {
        let mesh = Arc::new(primitives::sphere(1.0, 16, 8));
        let axis = |invert, blend, weight| AxisLayer { invert, blend, weight };
        let mut replaced_curvature = section(GeneratorKind::Curvature, |section| section.contrast = 0.2);
        replaced_curvature.inputs.insert(BakedMap::Curvature, Source::image("Input"));
        let mut balanced = section(GeneratorKind::AmbientOcclusion, |section| (section.balance, section.contrast) = (0.3, 0.2));
        balanced.global.balance = 0.6;
        balanced.global.contrast = 0.2;
        balanced.invert = true;
        balanced.offset = 0.1;
        let mut globally_blurred = section(GeneratorKind::AmbientOcclusion, |section| section.balance = 0.7);
        globally_blurred.global.blur = 0.2;
        let mut cells = section(GeneratorKind::AmbientOcclusion, |section| (section.balance, section.invert) = (0.4, true));
        cells.resolution.factor = -2;
        let mut replaced_position = section(GeneratorKind::Position, |section| section.axes = [axis(false, AxisBlend::Normal, 1.0); 3]);
        replaced_position.inputs.insert(BakedMap::Position, Source::image("Input"));
        let cases = [
            ("Curvature", section(GeneratorKind::Curvature, |section| (section.brightness, section.contrast) = (0.3, 0.4)), Tolerance::POINT),
            ("Curvature replaced", replaced_curvature, Tolerance::POINT),
            ("Ambient Occlusion balanced", balanced, Tolerance::POINT),
            ("Ambient Occlusion blurred after", globally_blurred, Tolerance::NEIGHBOURHOOD),
            (
                "Ambient Occlusion section blur",
                section(GeneratorKind::AmbientOcclusion, |section| (section.blur, section.balance, section.contrast) = (16.0, 0.3, 0.3)),
                Tolerance::NEIGHBOURHOOD,
            ),
            ("Ambient Occlusion cells", cells, Tolerance::NEIGHBOURHOOD),
            (
                "Position axes",
                section(GeneratorKind::Position, |section| {
                    section.axes = [axis(true, AxisBlend::Multiply, 0.7), axis(false, AxisBlend::Normal, 1.0), axis(false, AxisBlend::AddSub, 0.5)];
                    (section.balance, section.brightness, section.invert) = (0.7, -0.4, true);
                }),
                Tolerance::POINT,
            ),
            (
                "Position blends",
                section(GeneratorKind::Position, |section| {
                    section.axes = [axis(false, AxisBlend::Subtract, 0.8), axis(true, AxisBlend::Max, 0.6), axis(false, AxisBlend::Add, 1.0)];
                    section.contrast = 1.0;
                }),
                Tolerance::POINT,
            ),
            ("Position replaced", replaced_position, Tolerance::POINT),
        ];

        assert_generators_match(&mesh, &bakes(), &cases);
    }

    /// The Light generator over the baked world normal (zero along an edge), over the mesh's own
    /// normals without a bake, and over replaced inputs; with a saturated highlight, the steep
    /// attenuation and invert
    #[test]
    fn light_controls_match_the_cpu() {
        let mesh = Arc::new(primitives::sphere(1.0, 16, 8));
        let mut replaced = light(|light| light.glossiness = 0.2);
        replaced.inputs.insert(BakedMap::WorldNormal, Source::image("Input"));
        replaced.inputs.insert(BakedMap::Position, Source::Uniform([0.3, 0.6, 0.2]));
        let mut steep = light(|light| (light.highlight_level, light.attenuation, light.horizontal, light.vertical) = (1.0, 0.6, 200.0, 30.0));
        steep.invert = true;
        let baked = [("Light", light(|light| light.highlight_level = 0.4), Tolerance::POINT), ("Light replaced", replaced, Tolerance::POINT)];
        let unbaked = [("Light on the mesh's normals", steep, Tolerance::POINT)];

        assert_generators_match(&mesh, &bakes(), &baked);
        assert_generators_match(&mesh, &BakedMaps::default(), &unbaked);
    }
}
