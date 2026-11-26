use std::collections::BTreeMap;
use std::sync::Arc;

use texturecraft_compose::{BakedMaps, ImageRes, ImageStore, PixelStore};
use texturecraft_core::{
    BakedMap, Channel, Effect, EffectKind, Fill, Generator, GeneratorKind, Layer, LayerKind, Mask, MaskBackground, MicroCurvatureType, MicroDetails,
    NormalFormat, Project, Source, TextureLayer, TextureSet,
};
use texturecraft_geom::primitives;
use texturecraft_texel::TILE;

use super::block_groups;
use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu, earlier_generator, headless};

/// Blocks of one size come out together, the clipped last column, row and corner apart
#[test]
fn blocks_group_by_their_clipped_size() {
    let groups = block_groups([10, 7], [3, 2]);

    let sizes: Vec<_> = groups.iter().map(|group| (group.origin, group.count, group.size)).collect();
    assert_eq!(sizes, vec![([0, 0], [3, 2], [3, 3]), ([9, 0], [1, 2], [1, 3]), ([0, 6], [3, 1], [3, 1]), ([9, 6], [1, 1], [1, 1])]);
    assert_eq!(block_groups([8, 8], [1, 1]).len(), 1, "a classic set is one block");
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
/// occludes nothing and no baked occlusion; a texture and a custom grunge leave the earlier
/// model as it is
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
            Generator {
                micro: Some(Box::new(MicroDetails { normal_source: normal, curvature_intensity: 0.6, ..images_of(MicroCurvatureType::Sobel) })),
                ..earlier_generator(GeneratorKind::Curvature)
            },
            &baked,
        ),
        ("Standard", curvature(images_of(MicroCurvatureType::Standard)), &baked),
        ("Smooth", curvature(MicroDetails { height_intensity: 4.0, ..images_of(MicroCurvatureType::Smooth) }), &baked),
        (
            "a uniform height and an empty normal",
            curvature(MicroDetails {
                curvature_type: MicroCurvatureType::Standard,
                height_source: Some(Source::Uniform([0.3, 0.5, 0.7])),
                ..micro(GeneratorKind::Curvature)
            }),
            &baked,
        ),
        (
            "occlusion over more than 16 steps",
            Generator {
                micro: Some(Box::new(MicroDetails { height_source: height.clone(), ao_radius: 1.0, ao_depth: 0.3, ..micro(GeneratorKind::AmbientOcclusion) })),
                ..earlier_generator(GeneratorKind::AmbientOcclusion)
            },
            &baked,
        ),
        (
            "occlusion over 16 steps at most",
            Generator {
                micro: Some(Box::new(MicroDetails {
                    height: true,
                    height_source: height.clone(),
                    ao_radius: 0.5,
                    ao_depth: 0.5,
                    ..micro(GeneratorKind::Dirt)
                })),
                ..earlier_generator(GeneratorKind::Dirt)
            },
            &baked,
        ),
        (
            "an occlusion radius too short to occlude",
            Generator {
                micro: Some(Box::new(MicroDetails { height_source: height.clone(), ao_radius: 0.1, ..micro(GeneratorKind::AmbientOcclusion) })),
                ..earlier_generator(GeneratorKind::AmbientOcclusion)
            },
            &baked,
        ),
        (
            "occlusion without a baked map",
            Generator {
                micro: Some(Box::new(MicroDetails { height_source: height.clone(), ao_radius: 1.0, ..micro(GeneratorKind::AmbientOcclusion) })),
                ..earlier_generator(GeneratorKind::AmbientOcclusion)
            },
            &unbaked,
        ),
        (
            "a texture and a custom grunge, which the earlier model does not read",
            Generator {
                detail: None,
                texture: Some(Box::new(TextureLayer { enabled: true, opacity: 0.8, source: height.clone(), ..TextureLayer::default() })),
                grunge: height.clone(),
                ..earlier_generator(GeneratorKind::Curvature)
            },
            &baked,
        ),
    ];
    for (label, generator, baked) in cases {
        let mut project = Project::new("Micro Details", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, TILE));
        let owner = project.alloc_id();
        let fill = Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.9, 0.6, 0.3]))]), ..Default::default() };
        let mut layer = Layer::new(owner, "Micro", LayerKind::Fill(fill), &[Channel::BaseColor]);
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Generator(generator)));
        layer.mask = Some(mask);
        project.insert_layer(0, layer, None, false).unwrap();

        let pixels = PixelStore::default();
        let mut case = OracleCase::new(&project, &pixels, baked, &images, &mesh);
        case.mask_owner = Some(owner);
        assert_matches_cpu(&mut gpu, &case, Tolerance::NEIGHBOURHOOD, label);
    }
}
