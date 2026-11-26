//! P3b: the generator kinds that read the whole map (UV Border Distance, Dripping Rust, UV
//! Random Color and UV Checker; compose's `whole_map_generator`). Each reads a field compose
//! builds once per layout and settings (`GeneratorMaps::field`); the GPU builds the same field in
//! passes of its own before the composite ([`GeneratorFieldsGpu`], shaders/generator_p3b.wgsl)
//! and the composite reads it at the texel (shaders/generator_p3b_sample.wgsl, binding 16).
//!
//! The UV islands are numbered on the CPU, as `TrianglePartition::uv_islands` numbers them: the
//! per-island values hash that number. Everything per texel runs on the GPU: the coverage, the
//! exact distance transform per UV tile block, the box blurs, the occlusion and grunge, the drips.
//!
//! Declined (composed on the CPU): wear, light or section controls on these kinds, which compose
//! as other kinds do, and UV tile atlases whose tiles have resolutions of their own.

#[path = "gpu_generator_p3b_field_plan.rs"]
pub(crate) mod field_plan;
#[path = "gpu_generator_p3b_field_request.rs"]
pub(crate) mod field_request;
#[path = "gpu_generator_p3b_fields_gpu.rs"]
mod fields_gpu;
#[path = "gpu_generator_p3b_params.rs"]
mod params;

use texturecraft_compose::field_recipe;
use texturecraft_core::{Generator, GeneratorKind, KindControls, ProceduralKind};

pub(crate) use field_plan::FieldPlan;
use field_request::FieldRequest;
pub(crate) use fields_gpu::GeneratorFieldsGpu;

use super::{Operation, ProceduralInputs};

/// The generator kinds' opcode beyond the earlier model, with the kind in `procedural[0]` (P3a
/// takes 1..5)
pub(super) const GENERATOR_KIND_OPCODE: u32 = 47;
/// The kinds in `procedural[0]` (the same constants in generator_p3b_sample.wgsl)
const BORDER_DISTANCE: u32 = 5;
const DRIPPING_RUST: u32 = 6;
const RANDOM_COLOR: u32 = 7;
const CHECKER: u32 = 8;
/// The kinds this module lowers, by `procedural[0]`
pub(super) const FIELD_KINDS: std::ops::Range<u32> = BORDER_DISTANCE..CHECKER + 1;

/// Whether `kind` reads a whole-map field
pub(super) fn is_field_kind(kind: GeneratorKind) -> bool {
    matches!(kind, GeneratorKind::UvBorderDistance | GeneratorKind::DrippingRust | GeneratorKind::UvRandomColor | GeneratorKind::UvChecker)
}

/// Whether `op` reads a whole-map field
pub(super) fn reads_field(op: &Operation) -> bool {
    op.ids[1] == GENERATOR_KIND_OPCODE && FIELD_KINDS.contains(&op.procedural[0])
}

/// A whole-map kind's generator operation: its field's layer in `procedural[1]`, its finish in
/// `high` (Border Distance's balance and contrast, Dripping Rust's spreading and contrast). None
/// where its ramp would not be its field's (wear, light or section controls) or the atlas's
/// tiles have resolutions of their own.
pub(super) fn operation(generator: &Generator, inputs: &mut ProceduralInputs) -> Option<Operation> {
    let other_ramp = generator.wear.is_some() || generator.light.is_some() || generator.section_controls().is_some();
    if other_ramp || !inputs.levels.is_empty() {
        return None;
    }

    let controls = generator.kind_controls()?;
    let (kind, finish, seed) = match &controls {
        KindControls::BorderDistance(border) => (BORDER_DISTANCE, [border.balance, border.contrast], 0),
        KindControls::DrippingRust(rust) => {
            field_recipe::rust_grunge().filter(|grunge| grunge.0 == ProceduralKind::Grunge)?;
            (DRIPPING_RUST, [rust.spreading, rust.contrast], generator.seed)
        }
        KindControls::UvRandomColor(_) => (RANDOM_COLOR, [0.0; 2], generator.seed),
        KindControls::UvChecker(_) => (CHECKER, [0.0; 2], generator.seed),
        _ => return None,
    };
    let request = FieldRequest { controls, seed };
    let layer = match inputs.p3b_fields.iter().position(|existing| *existing == request) {
        Some(layer) => layer,
        None => {
            inputs.p3b_fields.push(request);
            inputs.p3b_fields.len() - 1
        }
    };

    let mut op = Operation {
        ids: [0, GENERATOR_KIND_OPCODE, generator.seed, u32::from(generator.invert)],
        high: [finish[0], finish[1], 0.0, 0.0],
        // The global balance and contrast stay neutral: these kinds have none (Generator::applied_global)
        projection: [1.0, generator.offset, 0.5, 0.0],
        procedural: [kind, layer as u32, 0, 0],
        ..bytemuck::Zeroable::zeroed()
    };

    // Coarser than the UV tile: the ramps averaged over cells (generator_region), never blurred
    op.spatial[2] = u32::from(generator.resolution.can_reduce());
    let cell = generator.resolution.cell(inputs.resolution, inputs.texture_set);
    if cell >= 2 {
        inputs.generator_stages.push((None, cell));
        op.spatial[1] = inputs.generator_stages.len() as u32;
        inputs.generator_levels.push(Vec::new());
    }
    op.high.iter().chain(&op.projection).all(|value| value.is_finite()).then_some(op)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #![allow(clippy::unwrap_used)] // Tests: a fixture that does not build fails the test

    use std::collections::BTreeMap;
    use std::sync::Arc;

    use texturecraft_compose::{BakedMaps, ImageStore, PixelStore};
    use texturecraft_core::{
        BakedMap, BorderDistanceParameters, Channel, DrippingRustParameters, Effect, EffectKind, Fill, Generator, GeneratorKind, KindControls, Layer,
        LayerId, LayerKind, Mask, MaskBackground, OutputResolution, Project, SectionParameters, Source, TextureSet, UvCheckerParameters,
        UvRandomColorParameters,
    };
    use texturecraft_geom::glam::Vec2;
    use texturecraft_geom::{Mesh, primitives};

    use super::super::oracle::{OracleCase, Tolerance, assert_matches_cpu, headless};
    use super::super::{CompositeRegion, ProceduralInputs, TilePatterns, generator_operation};

    /// A `kind` generator with `controls`
    fn generator(controls: KindControls) -> Generator {
        Generator { controls: Some(Box::new(controls.clone())), ..Generator::new(controls.kind()) }
    }

    /// One fill layer per generator, its channel at `value` under a black mask holding the generator
    fn masked_fills(project: &mut Project, generators: Vec<(Channel, Generator)>) -> Vec<LayerId> {
        let channels: Vec<Channel> = project.texture_sets[0].channels.iter().map(|&(channel, _)| channel).collect();
        generators
            .into_iter()
            .map(|(channel, generator)| {
                let owner = project.alloc_id();
                let fill = Fill { sources: BTreeMap::from([(channel, Source::Uniform([0.83; 3]))]), ..Default::default() };
                let mut layer = Layer::new(owner, "Masked", LayerKind::Fill(fill), &channels);
                let mut mask = Mask::new(MaskBackground::Black);
                mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Generator(generator)));
                layer.mask = Some(mask);
                project.insert_layer(0, layer, None, false).unwrap();
                owner
            })
            .collect()
    }

    /// The field kernels and the composite's field read validate and write GLSL, without an adapter
    #[test]
    fn field_shaders_validate_without_an_adapter() {
        let source = [include_str!("shaders/noise.wgsl"), include_str!("shaders/generator_p3b.wgsl")].join("\n");
        let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default())
            .validate(&module)
            .unwrap_or_else(|error| panic!("{error:?}"));
    }

    /// Wear, light or section controls, and a UV tile atlas whose tiles have resolutions of their
    /// own, stay on the CPU; the plain kinds lower to one field each, shared when equal
    #[test]
    fn whole_map_kinds_lower_to_shared_fields_and_decline_what_they_do_not_build() {
        let (mut inputs, mut patterns) = (ProceduralInputs::new(64), TilePatterns::default());
        let checker = Generator::new(GeneratorKind::UvChecker);
        let first = generator_operation(&checker, &mut inputs, &mut patterns).unwrap();
        let again = generator_operation(&checker, &mut inputs, &mut patterns).unwrap();
        let rust = generator_operation(&Generator::new(GeneratorKind::DrippingRust), &mut inputs, &mut patterns).unwrap();
        assert_eq!((first.procedural[1], again.procedural[1], rust.procedural[1]), (0, 0, 1), "equal fields share a layer");

        let sectioned = Generator { section: SectionParameters::defaults(GeneratorKind::Curvature), ..checker.clone() };
        assert!(generator_operation(&sectioned, &mut inputs, &mut patterns).is_none(), "section controls");
        inputs.levels = vec![32];
        assert!(generator_operation(&checker, &mut inputs, &mut patterns).is_none(), "UV tiles at resolutions of their own");
    }

    /// UV Random Color, UV Checker (Random Colors, borders, grid, coarser resolution) and UV Border
    /// Distance (balance, contrast, invert, offset) over the cube's six UV islands, in the
    /// material, the Checker's mask preview and a partial update
    #[test]
    fn island_kinds_match_the_cpu_on_the_cube() {
        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::cube(1.0));
        let mut project = Project::new("Islands", "builtin:cube");
        project.texture_sets.push(TextureSet::new("Cube", 0, 128));
        let checker = UvCheckerParameters { random_colors: true, grid_tile: 3, grid_opacity: 0.6, border_width: 0.4, ..Default::default() };
        let coarse_checker =
            Generator { resolution: OutputResolution { factor: -1, ..Default::default() }, ..generator(KindControls::UvChecker(checker)) };
        let border = BorderDistanceParameters { distance: 0.05, smoothness: 0.0, balance: 0.4, contrast: 0.3 };
        let layers = masked_fills(
            &mut project,
            vec![
                (Channel::BaseColor, generator(KindControls::UvRandomColor(UvRandomColorParameters::default()))),
                (Channel::Roughness, generator(KindControls::UvChecker(checker))),
                (Channel::Metallic, Generator { invert: true, offset: 0.05, ..generator(KindControls::BorderDistance(border)) }),
                (Channel::Height, coarse_checker),
            ],
        );
        let (pixels, baked, images) = (PixelStore::default(), BakedMaps::default(), ImageStore::default());
        let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
        case.mask_owner = Some(layers[1]);
        case.region = Some(CompositeRegion { min: [40, 50], max: [90, 80] });

        assert_matches_cpu(&mut gpu, &case, Tolerance::POINT, "UV island kinds");
    }

    /// UV Border Distance blurred by Smoothness and UV Checker's borders on a UV tile atlas of two
    /// tiles: each block's distances and blur stop at its edges, and the checker reads the mesh's
    /// own UVs
    #[test]
    fn border_blur_and_tile_blocks_match_the_cpu_on_uv_tiles() {
        let Some(mut gpu) = headless() else { return };
        let mut set = TextureSet::new("Tiles", 0, 64);
        set.uv_tiles = vec![1001, 1002];
        let grid = set.tile_grid().unwrap();
        let mut mesh = Mesh::default();
        for column in 0..2 {
            let mut part = primitives::plane(1.0, 4);
            for position in &mut part.positions {
                position.x += 1.5 * column as f32;
            }
            let corner = Vec2::new(column as f32, 0.0);
            // An island inset in each tile, so the blur meets both the empty space and the block's edge
            part.map_material_uvs_with(0, |centroid, uv| {
                let inset = uv * 0.8 + 0.05;
                Vec2::from(set.to_atlas(&grid, (centroid + corner).to_array(), (inset + corner).to_array()))
            });
            mesh.append(&part);
        }
        for submesh in &mut mesh.submeshes {
            submesh.material = 0;
        }
        let mesh = Arc::new(mesh);
        let mut project = Project::new("Tiles", "builtin:plane");
        project.texture_sets.push(set);
        let border = BorderDistanceParameters { distance: 0.4, smoothness: 1.0, balance: 0.5, contrast: 0.0 };
        let checker = UvCheckerParameters { grid_tile: 2, grid_opacity: 0.8, border_width: 0.6, ..Default::default() };
        masked_fills(
            &mut project,
            vec![(Channel::BaseColor, generator(KindControls::BorderDistance(border))), (Channel::Roughness, generator(KindControls::UvChecker(checker)))],
        );
        let (pixels, baked, images) = (PixelStore::default(), BakedMaps::default(), ImageStore::default());
        let case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);

        assert_matches_cpu(&mut gpu, &case, Tolerance::NEIGHBOURHOOD, "Border Distance and Checker on UV tiles");
    }

    /// Dripping Rust from a baked occlusion: grunge, both blurs over the cover, drips down a strip
    /// that rises gently to a ridge, falls steeply past it and lies flat along the axis at one end
    #[test]
    fn dripping_rust_matches_the_cpu_over_a_ridge() {
        let Some(mut gpu) = headless() else { return };
        const BAKE: u32 = 64;
        let mut mesh = primitives::plane(2.0, 32);
        for position in &mut mesh.positions {
            position.y = if position.z < -0.5 {
                0.5
            } else if position.z <= 0.0 {
                1.0 + position.z
            } else {
                1.0 - 8.0 * position.z
            };
        }
        let mesh = Arc::new(mesh);
        let occlusion: Vec<f32> = (0..BAKE * BAKE).map(|texel| 0.35 + 0.55 * ((texel % BAKE) as f32 / BAKE as f32)).collect();
        let baked = BakedMaps { resolution: BAKE, maps: BTreeMap::from([(BakedMap::AmbientOcclusion, Arc::new(occlusion))]) };
        let mut project = Project::new("Rust", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Strip", 0, 128));
        let rust = DrippingRustParameters {
            spreading: 0.6,
            contrast: 0.4,
            spreading_smoothness: 0.6,
            drips_intensity: 1.0,
            drips_smoothness: 0.3,
            drips_samples: 12,
            axis: 1,
        };
        masked_fills(&mut project, vec![(Channel::BaseColor, generator(KindControls::DrippingRust(rust)))]);
        let (pixels, images) = (PixelStore::default(), ImageStore::default());
        let case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);

        assert_matches_cpu(&mut gpu, &case, Tolerance::TRANSCENDENTAL, "Dripping Rust");
    }
}
