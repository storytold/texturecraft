//! A generator's texture (Use Texture) on a sphere: read in UV or triplanar, turned gray, blended over the signal before the global controls

use std::sync::Arc;

use super::surface_point::SurfacePoint;

use texturecraft_compose::{BakedMaps, ImageRes};
use texturecraft_core::{BakedMap, Generator, GeneratorGrayscale, GeneratorKind, SectionParameters, Source, TextureBlend, TextureLayer};
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::primitives;

use super::generator_fixture::GeneratorFixture;

const RES: u32 = 128;

/// How far a mask value may sit from the expected one
const TOLERANCE: f32 = 1e-3;

/// The ambient occlusion every texel of the sphere reads
const OCCLUSION: f32 = 0.3;

/// Width of the split image: its left half black, its right half white
const SPLIT_WIDTH: u32 = 64;

/// How far from a split's edge (in UV) a texel must sit to read one side only
const SPLIT_MARGIN: f32 = 0.05;

/// A unit sphere with a white fill layer, uniform occlusion and the images `flat` (one color) and `split` (black left, white right)
fn fixture(flat: [f32; 3]) -> GeneratorFixture {
    let mesh = primitives::sphere(1.0, 32, 16);
    let mut fixture = GeneratorFixture::new(mesh, "builtin:sphere", RES);

    let baked = BakedMaps { resolution: RES, maps: [(BakedMap::AmbientOcclusion, Arc::new(vec![OCCLUSION; (RES * RES) as usize]))].into() };

    let [red, green, blue] = flat;
    let flat = ImageRes::new(4, 4, vec![[red, green, blue, 1.0]; 16], false);
    let split_texel = |index: u32| if index % SPLIT_WIDTH < SPLIT_WIDTH / 2 { [0.0, 0.0, 0.0, 1.0] } else { [1.0; 4] };
    let split = ImageRes::new(SPLIT_WIDTH, SPLIT_WIDTH, (0..SPLIT_WIDTH * SPLIT_WIDTH).map(split_texel).collect(), false);
    let images = [("flat".to_string(), Arc::new(flat)), ("split".to_string(), Arc::new(split))].into();
    fixture.baked = baked;
    fixture.images = images;
    fixture
}

/// Every covered texel
fn surface(fixture: &GeneratorFixture) -> Vec<SurfacePoint> {
    (0..RES * RES)
        .filter_map(|index| {
            let texel = (index % RES, index / RES);
            let uv = [(texel.0 as f32 + 0.5) / RES as f32, (texel.1 as f32 + 0.5) / RES as f32];
            fixture.gbuf.surface(&fixture.mesh, index as usize).map(|surface| SurfacePoint { texel, uv, position: surface.pos, normal: surface.nrm })
        })
        .collect()
}

/// Ambient Occlusion on its section controls with `texture` reading image `image`
fn occlusion_with(image: &str, texture: TextureLayer) -> Generator {
    Generator { texture: Some(Box::new(TextureLayer { source: Some(Source::image(image)), ..texture })), ..Generator::new(GeneratorKind::AmbientOcclusion) }
}

/// A texture turned on at full opacity, replacing the signal
fn replacing() -> TextureLayer {
    TextureLayer { enabled: true, opacity: 1.0, blend: TextureBlend::Normal, ..TextureLayer::default() }
}

fn assert_all(actual: &[f32], expected: f32, what: &str) {
    assert!(!actual.is_empty(), "{what}: no texels read");
    for (index, actual) in actual.iter().enumerate() {
        assert!((actual - expected).abs() < TOLERANCE, "{what}: texel {index} is {actual}, expected {expected}");
    }
}

#[test]
fn a_texture_turned_off_or_at_opacity_zero_leaves_the_signal() {
    let mut fixture = fixture([0.6; 3]);
    let texels: Vec<_> = surface(&fixture).iter().map(|point| point.texel).collect();

    assert_all(&fixture.sample(occlusion_with("flat", TextureLayer { enabled: false, ..replacing() }), &texels), OCCLUSION, "turned off");
    assert_all(&fixture.sample(occlusion_with("flat", TextureLayer { opacity: 0.0, ..replacing() }), &texels), OCCLUSION, "opacity 0");
    assert_all(&fixture.sample(occlusion_with("flat", TextureLayer::default()), &texels), OCCLUSION, "the reference's defaults");
}

#[test]
fn an_empty_texture_slot_blends_black() {
    let mut fixture = fixture([0.6; 3]);
    let texels: Vec<_> = surface(&fixture).iter().map(|point| point.texel).collect();
    let empty = Generator { texture: Some(Box::new(TextureLayer { opacity: 0.5, ..replacing() })), ..Generator::new(GeneratorKind::AmbientOcclusion) };
    assert_all(&fixture.sample(empty, &texels), OCCLUSION * 0.5, "half way to black");
}

#[test]
fn a_texture_needs_the_section_controls() {
    let mut fixture = fixture([0.6; 3]);
    let texels: Vec<_> = surface(&fixture).iter().map(|point| point.texel).collect();
    let earlier = Generator { section: None, global: Default::default(), ..occlusion_with("flat", replacing()) };
    let without = Generator { texture: None, ..earlier.clone() };
    assert_eq!(fixture.sample(earlier, &texels), fixture.sample(without, &texels));
}

#[test]
fn the_texture_turns_gray_by_its_conversion() {
    let mut fixture = fixture([1.0, 0.25, 0.0]);
    let texels: Vec<_> = surface(&fixture).iter().map(|point| point.texel).collect();
    for (grayscale, expected) in [
        (GeneratorGrayscale::Average, 1.25 / 3.0),
        (GeneratorGrayscale::Red, 1.0),
        (GeneratorGrayscale::Green, 0.25),
        (GeneratorGrayscale::Desaturation, 0.5),
        (GeneratorGrayscale::Luma, 0.299 + 0.587 * 0.25),
    ] {
        let generator = occlusion_with("flat", TextureLayer { grayscale, ..replacing() });
        assert_all(&fixture.sample(generator, &texels), expected, &format!("{grayscale:?}"));
    }
}

#[test]
fn overlay_with_a_mid_gray_texture_doubles_a_dark_signal_times_the_gray() {
    let mut fixture = fixture([0.4; 3]);
    let texels: Vec<_> = surface(&fixture).iter().map(|point| point.texel).collect();
    let generator = occlusion_with("flat", TextureLayer { blend: TextureBlend::Overlay, opacity: 0.5, ..replacing() });
    let overlay = 2.0 * OCCLUSION * 0.4;
    assert_all(&fixture.sample(generator, &texels), OCCLUSION + (overlay - OCCLUSION) * 0.5, "overlay at half opacity");
}

#[test]
fn an_image_not_loaded_blends_black_with_or_without_a_blur() {
    let mut fixture = fixture([0.6; 3]);
    let texels: Vec<_> = surface(&fixture).iter().map(|point| point.texel).collect();
    let loading = occlusion_with("not loaded yet", TextureLayer { opacity: 0.5, ..replacing() });
    assert_all(&fixture.sample(loading.clone(), &texels), OCCLUSION * 0.5, "per texel");

    let mut blurred = loading;
    blurred.global.blur = 0.5;
    assert_all(&fixture.sample(blurred, &texels), OCCLUSION * 0.5, "over a region, as the blur computes it");
}

#[test]
fn the_texture_blends_after_the_section_controls_and_before_the_global_ones() {
    let mut fixture = fixture([0.6; 3]);
    let texels: Vec<_> = surface(&fixture).iter().map(|point| point.texel).collect();

    let mut section_inverted = occlusion_with("flat", replacing());
    section_inverted.section = Some(SectionParameters { invert: true, ..section_inverted.section.unwrap() });
    assert_all(&fixture.sample(section_inverted, &texels), 0.6, "the texture replaces the inverted section");

    let globally_inverted = Generator { invert: true, ..occlusion_with("flat", replacing()) };
    assert_all(&fixture.sample(globally_inverted, &texels), 0.4, "the global invert flips the texture");

    let mut blurred = occlusion_with("flat", replacing());
    blurred.section = Some(SectionParameters { blur: 2.0, ..blurred.section.unwrap() });
    assert_all(&fixture.sample(blurred, &texels), 0.6, "with the section blur (computed over a region)");
}

#[test]
fn uv_scale_repeats_the_texture_about_the_centre() {
    let mut fixture = fixture([0.0; 3]);
    let points = surface(&fixture);
    let side = |u: f32| (u.rem_euclid(1.0) >= 0.5) as u8 as f32;
    for scale in [1.0, 2.0, 3.0] {
        let read_at = |u: f32| (u - 0.5) * scale + 0.5;
        let clear: Vec<_> = points
            .iter()
            .filter(|point| {
                let u = read_at(point.uv[0]).rem_euclid(1.0);
                [0.0, 0.5, 1.0].iter().all(|edge| (u - edge).abs() > SPLIT_MARGIN * scale)
            })
            .collect();
        let texels: Vec<_> = clear.iter().map(|point| point.texel).collect();
        let values = fixture.sample(occlusion_with("split", TextureLayer { scale, ..replacing() }), &texels);
        for (point, value) in clear.iter().zip(values) {
            let expected = side(read_at(point.uv[0]));
            assert!((value - expected).abs() < TOLERANCE, "scale {scale} at u {}: {value}, expected {expected}", point.uv[0]);
        }
    }
}

#[test]
fn triplanar_reads_the_top_from_the_plane_facing_up() {
    let mut fixture = fixture([0.0; 3]);
    let (low, high) = fixture.mesh.bounds();
    let across = |position: Vec3| (position.x - low.x) / (high.x - low.x);
    let top: Vec<_> = surface(&fixture).into_iter().filter(|point| point.normal.y > 0.95 && (across(point.position) - 0.5).abs() > SPLIT_MARGIN).collect();
    assert!(!top.is_empty());

    let texels: Vec<_> = top.iter().map(|point| point.texel).collect();
    let sharp = TextureLayer { triplanar: true, triplanar_contrast: 1.0, ..replacing() };
    let values = fixture.sample(occlusion_with("split", sharp), &texels);
    for (point, value) in top.iter().zip(values) {
        let expected = if across(point.position) >= 0.5 { 1.0 } else { 0.0 };
        assert!((value - expected).abs() < TOLERANCE, "at x {}: {value}, expected {expected}", across(point.position));
    }
}
