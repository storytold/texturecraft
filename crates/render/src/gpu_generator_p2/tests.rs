use std::collections::BTreeMap;
use std::sync::Arc;

use texturecraft_compose::{BakedMaps, ImageRes, ImageStore, PixelStore};
use texturecraft_core::{
    BakedMap, BlendMode, Channel, ContentsOffset, Effect, EffectKind, Fill, Generator, GeneratorKind, Layer, LayerId, LayerKind, Mask, MaskBackground,
    MicroCurvatureType, MicroDetails, NormalFormat, Procedural, ProceduralKind, Project, Projection, Sampling, Source, TextureLayer, TextureSet,
};
use texturecraft_geom::primitives;
use texturecraft_texel::TILE;

use super::super::{LOOKUP_WORDS, ProceduralInputs};
use super::{bind_sources, block_groups, grunge_record, texture_record};
use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu, earlier_generator, headless};

/// Blocks of one size come out together, the clipped last column, row and corner apart
#[test]
fn blocks_group_by_their_clipped_size() {
    let groups = block_groups([10, 7], [3, 2]);

    let sizes: Vec<_> = groups.iter().map(|group| (group.origin, group.count, group.size)).collect();
    assert_eq!(sizes, vec![([0, 0], [3, 2], [3, 3]), ([9, 0], [1, 2], [1, 3]), ([0, 6], [3, 1], [3, 1]), ([9, 6], [1, 1], [1, 1])]);
    assert_eq!(block_groups([8, 8], [1, 1]).len(), 1, "a classic set is one block");
}

/// A texture slot holding a procedural and a wear grunge's own pattern read source operations
/// whose indices become absolute once the program lays out its generator sources
#[test]
fn slot_and_grunge_sources_become_operation_indices() {
    let mut inputs = ProceduralInputs::new(64);
    let pattern = Source::Procedural { pattern: Procedural::new(ProceduralKind::Clouds), low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() };
    let texture = TextureLayer { enabled: true, opacity: 1.0, source: Some(pattern), ..TextureLayer::default() };
    let textured = Generator { texture: Some(Box::new(texture)), ..Generator::new(GeneratorKind::Curvature) };
    const OPERATIONS: u32 = 100;

    let texture = texture_record(&textured, &mut inputs).unwrap();
    let grunge = grunge_record(&Generator::new(GeneratorKind::EdgeWear), &mut inputs).unwrap();
    bind_sources(&mut inputs, OPERATIONS);

    let word = |record: u32, index: u32| inputs.ramp_words[(record - LOOKUP_WORDS + index) as usize];
    assert_eq!(word(texture, 9), OPERATIONS + 1, "the texture's procedural, the first source");
    assert_eq!((word(grunge, 1), word(grunge, 2)), (OPERATIONS + 2, 0), "the edge wear's scratches, without a breakup");
    assert_eq!(inputs.generator_sources.len(), 2);
}

/// An image of `size` texels of hashed noise, each channel its own
fn noise(size: (u32, u32), seed: u32) -> Arc<ImageRes> {
    let hash = |texel: u32, channel: u32| {
        let mixed = (texel.wrapping_mul(0x9e37_79b9) ^ (channel + seed).wrapping_mul(0x85eb_ca6b)).wrapping_mul(0xc2b2_ae35);
        (mixed >> 8) as f32 / (1 << 24) as f32
    };
    let rgba = (0..size.0 * size.1).map(|texel| [hash(texel, 0), hash(texel, 1), hash(texel, 2), 1.0]).collect();
    Arc::new(ImageRes::new(size.0, size.1, rgba, false))
}

/// Micro Details on the earlier model match the CPU: the Sobel, Standard and Smooth curvature
/// of a height and a normal image (Sobel's normal DirectX, with the detail sizes blurring the
/// micro curvature; Smooth's kernel and box blurs), a uniform height and an empty normal slot,
/// the occlusion of a height image over at most and over more than 16 steps, a radius that
/// occludes nothing and no baked occlusion; the bake's own detail sizes beside two micro
/// curvatures' in one mask; a texture and a custom grunge leave the earlier model as it is
#[test]
fn micro_details_match_the_cpu() {
    let Some(mut gpu) = headless() else { return };

    // A 512 bake: Smooth's sigmas reach the box blur, AO Radius 1 more than 16 steps
    const BAKE: u32 = 512;
    let mesh = Arc::new(primitives::plane(1.0, 1));
    let map = |phase: f32| -> Arc<Vec<f32>> {
        Arc::new((0..BAKE * BAKE).map(|texel| 0.5 + 0.2 * ((texel % BAKE) as f32 * 0.07 + phase).sin() * ((texel / BAKE) as f32 * 0.05).cos()).collect())
    };
    let baked = BakedMaps { resolution: BAKE, maps: BTreeMap::from([(BakedMap::Curvature, map(0.0)), (BakedMap::AmbientOcclusion, map(1.3))]) };
    let images = ImageStore::from([("height".to_string(), noise((101, 97), 1)), ("normal".to_string(), noise((89, 83), 2))]);
    let height = Some(Source::Image { name: "height".into(), normal_format: NormalFormat::OpenGl });
    let normal = Some(Source::Image { name: "normal".into(), normal_format: NormalFormat::DirectX });
    let micro = |kind| MicroDetails { enabled: true, ..MicroDetails::defaults(kind).unwrap() };
    let curvature = |micro: MicroDetails| Generator { detail: None, micro: Some(Box::new(micro)), ..earlier_generator(GeneratorKind::Curvature) };
    let images_of = |curvature_type| MicroDetails {
        curvature_type,
        height_source: height.clone(),
        normal_source: Some(Source::image("normal")),
        ..micro(GeneratorKind::Curvature)
    };
    let unbaked = BakedMaps::default();
    let cases = [
        (
            "Sobel with detail sizes",
            vec![Generator {
                micro: Some(Box::new(MicroDetails { normal_source: normal, curvature_intensity: 0.6, ..images_of(MicroCurvatureType::Sobel) })),
                ..earlier_generator(GeneratorKind::Curvature)
            }],
            &baked,
        ),
        ("Standard", vec![curvature(images_of(MicroCurvatureType::Standard))], &baked),
        ("Smooth", vec![curvature(MicroDetails { height_intensity: 4.0, ..images_of(MicroCurvatureType::Smooth) })], &baked),
        (
            "a uniform height and an empty normal",
            vec![curvature(MicroDetails {
                curvature_type: MicroCurvatureType::Standard,
                height_source: Some(Source::Uniform([0.3, 0.5, 0.7])),
                ..micro(GeneratorKind::Curvature)
            })],
            &baked,
        ),
        (
            "occlusion over more than 16 steps",
            vec![Generator {
                micro: Some(Box::new(MicroDetails { height_source: height.clone(), ao_radius: 1.0, ao_depth: 0.3, ..micro(GeneratorKind::AmbientOcclusion) })),
                ..earlier_generator(GeneratorKind::AmbientOcclusion)
            }],
            &baked,
        ),
        (
            "occlusion over 16 steps at most",
            vec![Generator {
                micro: Some(Box::new(MicroDetails {
                    height: true,
                    height_source: height.clone(),
                    ao_radius: 0.5,
                    ao_depth: 0.5,
                    ..micro(GeneratorKind::Dirt)
                })),
                ..earlier_generator(GeneratorKind::Dirt)
            }],
            &baked,
        ),
        (
            "an occlusion radius too short to occlude",
            vec![Generator {
                micro: Some(Box::new(MicroDetails { height_source: height.clone(), ao_radius: 0.1, ..micro(GeneratorKind::AmbientOcclusion) })),
                ..earlier_generator(GeneratorKind::AmbientOcclusion)
            }],
            &baked,
        ),
        (
            "occlusion without a baked map",
            vec![Generator {
                micro: Some(Box::new(MicroDetails { height_source: height.clone(), ao_radius: 1.0, ..micro(GeneratorKind::AmbientOcclusion) })),
                ..earlier_generator(GeneratorKind::AmbientOcclusion)
            }],
            &unbaked,
        ),
        (
            "a texture and a custom grunge, which the earlier model does not read",
            vec![Generator {
                detail: None,
                texture: Some(Box::new(TextureLayer { enabled: true, opacity: 0.8, source: height.clone(), ..TextureLayer::default() })),
                grunge: height.clone(),
                ..earlier_generator(GeneratorKind::Curvature)
            }],
            &baked,
        ),
        (
            "the bake's own detail sizes and two micro curvatures'",
            vec![
                earlier_generator(GeneratorKind::Curvature),
                Generator { micro: Some(Box::new(images_of(MicroCurvatureType::Sobel))), ..earlier_generator(GeneratorKind::Curvature) },
                Generator { micro: Some(Box::new(images_of(MicroCurvatureType::Standard))), ..earlier_generator(GeneratorKind::EdgeWear) },
            ],
            &baked,
        ),
    ];
    for (label, generators, baked) in cases {
        let mut project = Project::new("Micro Details", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, TILE));
        let owner = project.alloc_id();
        let fill = Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.9, 0.6, 0.3]))]), ..Default::default() };
        let mut layer = Layer::new(owner, "Micro", LayerKind::Fill(fill), &[Channel::BaseColor]);
        let mut mask = Mask::new(MaskBackground::Black);
        for (index, generator) in generators.into_iter().enumerate() {
            let mut generated = Effect::new(project.alloc_id(), EffectKind::Generator(generator));
            if index > 0 {
                generated.blend = BlendMode::Multiply;
            }
            mask.effects.push(generated);
        }
        layer.mask = Some(mask);
        project.insert_layer(0, layer, None, false).unwrap();

        let pixels = PixelStore::default();
        let mut case = OracleCase::new(&project, &pixels, baked, &images, &mesh);
        case.mask_owner = Some(owner);
        assert_matches_cpu(&mut gpu, &case, Tolerance::NEIGHBOURHOOD, label);
    }
}

/// A micro height read from an anchor point matches the CPU: the anchor's height under the
/// reader, gathered over the whole set, feeds the micro curvature and its detail sizes
#[test]
fn a_micro_height_read_from_an_anchor_matches_the_cpu() {
    let Some(mut gpu) = headless() else { return };
    const BAKE: u32 = 256;
    let curvature = (0..BAKE * BAKE).map(|texel| 0.5 + 0.2 * ((texel % BAKE) as f32 * 0.07).sin() * ((texel / BAKE) as f32 * 0.05).cos()).collect();
    let baked = BakedMaps { resolution: BAKE, maps: BTreeMap::from([(BakedMap::Curvature, Arc::new(curvature))]) };
    let images = ImageStore::from([("height".to_string(), noise((53, 47), 3))]);
    let mut project = Project::new("Micro anchor", "builtin:plane");
    project.texture_sets.push(TextureSet::new("Plane", 0, TILE));

    // A height fill with an anchor point after it, under a layer whose mask reads it as its micro height
    let height = project.alloc_id();
    let fill = Fill { sources: BTreeMap::from([(Channel::Height, Source::image("height"))]), ..Default::default() };
    let mut layer = Layer::new(height, "Height", LayerKind::Fill(fill), &[Channel::BaseColor, Channel::Height]);
    let anchor = project.alloc_id();
    layer.content_effects.push(Effect::new(anchor, EffectKind::Anchor));
    project.insert_layer(0, layer, None, false).unwrap();
    let owner = project.alloc_id();
    let fill = Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.9, 0.6, 0.3]))]), ..Default::default() };
    let mut layer = Layer::new(owner, "Micro", LayerKind::Fill(fill), &[Channel::BaseColor]);
    let micro = MicroDetails {
        enabled: true,
        normal: false,
        height_source: Some(Source::Anchor { anchor, channel: Channel::Height, ramp: Vec::new() }),
        ..MicroDetails::defaults(GeneratorKind::Curvature).unwrap()
    };
    let mut mask = Mask::new(MaskBackground::Black);
    let generator = Generator { micro: Some(Box::new(micro)), ..earlier_generator(GeneratorKind::Curvature) };
    mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Generator(generator)));
    layer.mask = Some(mask);
    project.insert_layer(0, layer, None, false).unwrap();
    let pixels = PixelStore::default();

    // A cube's UV islands leave texels uncovered, which the anchor's heights border
    for (label, mesh) in [("plane", primitives::plane(1.0, 1)), ("cube", primitives::cube(1.0))] {
        let mesh = Arc::new(mesh);
        let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
        case.mask_owner = Some(owner);

        assert_matches_cpu(&mut gpu, &case, Tolerance::NEIGHBOURHOOD, &format!("a micro height read from an anchor point on a {label}"));
    }
}

/// A layer whose mask reads anchor point `anchor`'s height as a Curvature's micro height
fn anchor_reader(project: &mut Project, anchor: LayerId) -> Layer {
    let fill = Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.9, 0.6, 0.3]))]), ..Default::default() };
    let mut layer = Layer::new(project.alloc_id(), "Micro", LayerKind::Fill(fill), &[Channel::BaseColor]);
    let micro = MicroDetails {
        enabled: true,
        normal: false,
        height_source: Some(Source::Anchor { anchor, channel: Channel::Height, ramp: Vec::new() }),
        ..MicroDetails::defaults(GeneratorKind::Curvature).unwrap()
    };
    let mut mask = Mask::new(MaskBackground::Black);
    let generator = Generator { micro: Some(Box::new(micro)), ..earlier_generator(GeneratorKind::Curvature) };
    mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Generator(generator)));
    layer.mask = Some(mask);
    layer
}

/// A height fill of the "height" image with an anchor point after it: the layer and the anchor
fn anchored_height(project: &mut Project) -> (Layer, LayerId) {
    let fill = Fill { sources: BTreeMap::from([(Channel::Height, Source::image("height"))]), ..Default::default() };
    let mut layer = Layer::new(project.alloc_id(), "Height", LayerKind::Fill(fill), &[Channel::BaseColor, Channel::Height]);
    let anchor = project.alloc_id();
    layer.content_effects.push(Effect::new(anchor, EffectKind::Anchor));
    (layer, anchor)
}

/// Micro height readers inside folders match the CPU (a sibling's anchor, the folder's own mask
/// anchor), and so do readers after and between the two draws of an instanced folder's anchor; a
/// reader inside an instanced folder, drawn twice, composes on the CPU
#[test]
fn micro_anchor_readers_in_folders_and_instances() {
    let Some(mut gpu) = headless() else { return };
    let channels = [Channel::BaseColor, Channel::Height];
    let mesh = Arc::new(primitives::plane(1.0, 1));
    let baked = BakedMaps {
        resolution: 128,
        maps: BTreeMap::from([(BakedMap::Curvature, Arc::new((0..128 * 128).map(|texel| 0.5 + 0.1 * ((texel % 128) as f32 * 0.09).sin()).collect()))]),
    };
    let images = ImageStore::from([("height".to_string(), noise((53, 47), 3))]);
    let project = || {
        let mut project = Project::new("Micro anchor", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, TILE));
        project
    };
    let folder = |project: &mut Project| {
        let id = project.alloc_id();
        project.insert_layer(0, Layer::new(id, "Folder", LayerKind::folder(), &channels), None, false).unwrap();
        id
    };

    // An instance of `source` in a folder that scales and turns it, so its draw publishes another height
    let instance_of = |project: &mut Project, source: LayerId| {
        let around = folder(project);
        if let Some(LayerKind::Group { contents, .. }) = project.layers.get_mut(&around).map(|layer| &mut layer.kind) {
            *contents = ContentsOffset { scale: 2.0, rotation: 30.0 };
        }
        let instance = project.alloc_id();
        project.insert_layer(0, Layer::new(instance, "Instance", LayerKind::Instance { source }, &channels), Some(around), true).unwrap();
    };

    // A reader beside the anchor's layer inside a folder
    let mut inside = project();
    let group = folder(&mut inside);
    let (height, anchor) = anchored_height(&mut inside);
    inside.insert_layer(0, height, Some(group), true).unwrap();
    let reader = anchor_reader(&mut inside, anchor);
    let sibling = inside.insert_layer(0, reader, Some(group), true).unwrap();

    // A reader inside a folder whose own mask holds the anchor
    let mut masked = project();
    let group = folder(&mut masked);
    let anchor = masked.alloc_id();
    let mut mask = Mask::new(MaskBackground::Black);
    let fill = EffectKind::Fill { source: Source::image("height"), projection: Projection::default(), sampling: Sampling::SAVED_BEFORE };
    mask.effects.push(Effect::new(masked.alloc_id(), fill));
    mask.effects.push(Effect::new(anchor, EffectKind::Anchor));
    masked.layers.get_mut(&group).unwrap().mask = Some(mask);
    let reader = anchor_reader(&mut masked, anchor);
    let under_mask = masked.insert_layer(0, reader, Some(group), true).unwrap();

    // An instanced folder publishes its anchor twice; the reader after both reads the last
    let mut instanced = project();
    let group = folder(&mut instanced);
    let (height, anchor) = anchored_height(&mut instanced);
    instanced.insert_layer(0, height, Some(group), true).unwrap();
    instance_of(&mut instanced, group);
    let reader = anchor_reader(&mut instanced, anchor);
    let after = instanced.insert_layer(0, reader, None, false).unwrap();

    // A reader between the two: it reads the first, the instance's draw after it hidden
    let mut between = project();
    let group = folder(&mut between);
    let (height, anchor) = anchored_height(&mut between);
    between.insert_layer(0, height, Some(group), true).unwrap();
    let reader = anchor_reader(&mut between, anchor);
    let before = between.insert_layer(0, reader, None, false).unwrap();
    instance_of(&mut between, group);

    let pixels = PixelStore::default();
    let cases = [
        ("inside a folder", &inside, sibling),
        ("under its folder's mask anchor", &masked, under_mask),
        ("after an instance", &instanced, after),
        ("between an instance's draws", &between, before),
    ];
    for (label, project, owner) in cases {
        let mut case = OracleCase::new(project, &pixels, &baked, &images, &mesh);
        case.mask_owner = Some(owner);

        assert_matches_cpu(&mut gpu, &case, Tolerance::NEIGHBOURHOOD, &format!("a micro anchor reader {label}"));
    }

    // The reader inside the instanced folder: drawn twice, it composes on the CPU
    let reader = anchor_reader(&mut instanced, anchor);
    instanced.insert_layer(0, reader, Some(group), true).unwrap();
    let compiled = super::super::CompositeProgram::try_compile_region(&instanced, &pixels, 0, None);
    assert_eq!(compiled.err(), Some("A micro height reader drawn more than once uses CPU compositing."));
}
