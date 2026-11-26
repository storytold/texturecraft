//! The UV Border Distance generator on a plane filling its UV tile: white at the tile's edge, fading inwards over its distance, softened by its smoothness

use texturecraft_core::{BorderDistanceParameters, Generator, GeneratorKind, KindControls};
use texturecraft_geom::primitives;

use super::generator_fixture::GeneratorFixture;

const RES: u32 = 64;

fn fixture() -> GeneratorFixture {
    let mesh = primitives::plane(2.0, 1);
    GeneratorFixture::new(mesh, "builtin:plane", RES)
}

/// The middle row's values, left edge to centre
fn row(fixture: &mut GeneratorFixture, border: BorderDistanceParameters) -> Vec<f32> {
    let generator = Generator { controls: Some(Box::new(KindControls::BorderDistance(border))), ..Generator::new(GeneratorKind::UvBorderDistance) };
    let texels: Vec<(u32, u32)> = (0..RES / 2).map(|x| (x, RES / 2)).collect();
    fixture.sample(generator, &texels)
}

#[test]
fn the_border_is_white_and_the_inside_fades_to_black() {
    let mut fixture = fixture();
    let values = row(&mut fixture, BorderDistanceParameters { distance: 0.5, ..BorderDistanceParameters::default() });

    assert!(values[0] > 0.9, "the edge, a texel in: {}", values[0]);
    assert!(values.windows(2).all(|pair| pair[1] <= pair[0]), "falling inwards: {values:?}");
    assert!(*values.last().unwrap() < 0.05, "half a tile in is black at distance 0.5");
}

#[test]
fn a_larger_distance_reaches_further_in() {
    let mut fixture = fixture();
    let near = row(&mut fixture, BorderDistanceParameters { distance: 0.1, ..BorderDistanceParameters::default() });
    let far = row(&mut fixture, BorderDistanceParameters { distance: 0.6, ..BorderDistanceParameters::default() });

    assert!(far.iter().sum::<f32>() > near.iter().sum::<f32>() + 1.0);
}

#[test]
fn smoothness_darkens_the_edge_with_the_space_around_the_island() {
    let mut fixture = fixture();
    let sharp = row(&mut fixture, BorderDistanceParameters::default());
    let smooth = row(&mut fixture, BorderDistanceParameters { smoothness: 1.0, ..BorderDistanceParameters::default() });

    assert!(smooth[0] < sharp[0] - 0.2, "{} vs {}", smooth[0], sharp[0]);
}

#[test]
fn saved_values_out_of_range_render_as_their_nearest_valid_ones() {
    let mut fixture = fixture();
    let hostile =
        row(&mut fixture, BorderDistanceParameters { distance: f32::NAN, smoothness: f32::INFINITY, contrast: 7.0, ..BorderDistanceParameters::default() });
    let valid = row(&mut fixture, BorderDistanceParameters { contrast: 1.0, ..BorderDistanceParameters::default() });

    assert_eq!(hostile, valid, "non-finite values read their defaults, others their nearest valid value");
}
