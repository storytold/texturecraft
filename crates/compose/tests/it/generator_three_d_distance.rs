//! The 3D Distance and 3D Linear Gradient generators on a sphere: white at a point falling off over a radius, and a gradient between two points

use std::sync::Arc;

use texturecraft_compose::BakedMaps;
use texturecraft_core::{BakedMap, DistanceParameters, Generator, GeneratorKind, KindControls, LinearGradientParameters};
use texturecraft_geom::primitives;

use super::generator_fixture::GeneratorFixture;

const RES: u32 = 64;

/// A unit sphere with a white fill layer
fn fixture() -> GeneratorFixture {
    let mesh = primitives::sphere(1.0, 32, 16);
    let mut fixture = GeneratorFixture::new(mesh, "builtin:sphere", RES);

    let baked = BakedMaps { resolution: RES, maps: [(BakedMap::AmbientOcclusion, Arc::new(vec![1.0; (RES * RES) as usize]))].into() };
    fixture.baked = baked;
    fixture
}

/// Every covered texel and its height on the sphere
fn covered(fixture: &GeneratorFixture) -> Vec<((u32, u32), f32)> {
    (0..RES * RES).filter_map(|index| fixture.gbuf.surface(&fixture.mesh, index as usize).map(|surface| ((index % RES, index / RES), surface.pos.y))).collect()
}

/// The mask of a 3D Distance generator with `distance` alone over black at `texels`
fn mask(fixture: &mut GeneratorFixture, distance: DistanceParameters, texels: &[(u32, u32)]) -> Vec<f32> {
    fixture.sample(Generator { controls: Some(Box::new(KindControls::Distance(distance))), ..Generator::new(GeneratorKind::ThreeDDistance) }, texels)
}

#[test]
fn a_point_at_the_top_lights_the_top() {
    let mut fixture = fixture();
    let texels = covered(&fixture);
    let at: Vec<(u32, u32)> = texels.iter().map(|(texel, _)| *texel).collect();

    let values = mask(&mut fixture, DistanceParameters { point: [0.5, 1.0, 0.5], radius: 0.8, contrast: 0.0, ..DistanceParameters::default() }, &at);
    let mean_where = |above: bool| {
        let picked: Vec<f32> = texels.iter().zip(&values).filter(|((_, height), _)| (*height > 0.5) == above).map(|(_, value)| *value).collect();
        picked.iter().sum::<f32>() / picked.len() as f32
    };
    assert!(mean_where(true) > mean_where(false) + 0.3, "top {} bottom {}", mean_where(true), mean_where(false));
}

#[test]
fn the_offset_slides_the_mask_from_black_to_white() {
    let mut fixture = fixture();
    let at: Vec<(u32, u32)> = covered(&fixture).iter().map(|(texel, _)| *texel).collect();

    let black = mask(&mut fixture, DistanceParameters { offset: 0.0, ..DistanceParameters::default() }, &at);
    let white = mask(&mut fixture, DistanceParameters { offset: 1.0, ..DistanceParameters::default() }, &at);
    assert!(black.iter().all(|value| *value == 0.0));
    assert!(white.iter().all(|value| *value == 1.0));
}

#[test]
fn a_larger_radius_reaches_further() {
    let mut fixture = fixture();
    let at: Vec<(u32, u32)> = covered(&fixture).iter().map(|(texel, _)| *texel).collect();
    let total = |values: Vec<f32>| values.iter().sum::<f32>();

    let small = total(mask(&mut fixture, DistanceParameters { radius: 0.3, ..DistanceParameters::default() }, &at));
    let large = total(mask(&mut fixture, DistanceParameters { radius: 1.2, ..DistanceParameters::default() }, &at));
    assert!(large > small);
}

#[test]
fn the_linear_gradient_runs_from_its_start_to_its_end() {
    let mut fixture = fixture();
    let texels = covered(&fixture);
    let at: Vec<(u32, u32)> = texels.iter().map(|(texel, _)| *texel).collect();
    let gradient = |gradient: LinearGradientParameters| Generator {
        controls: Some(Box::new(KindControls::LinearGradient(gradient))),
        ..Generator::new(GeneratorKind::ThreeDLinearGradient)
    };

    let upwards = fixture.sample(gradient(LinearGradientParameters::default()), &at);
    let downwards = fixture.sample(gradient(LinearGradientParameters { start: [0.5, 1.0, 0.5], end: [0.5, 0.0, 0.5], ..Default::default() }), &at);
    for (((_, height), up), down) in texels.iter().zip(&upwards).zip(&downwards) {
        assert!((up - (height * 0.5 + 0.5)).abs() < 0.02, "bottom to top across the sphere's height: {up} at {height}");
        assert!((up + down - 1.0).abs() < 1e-4, "swapping the points reverses it");
    }
}
