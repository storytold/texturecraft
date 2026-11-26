//! The Grayscale Conversion generator on a plane filling its UV tile: its image turned gray by the chosen type, tiled, turned and balanced

use std::sync::Arc;

use texturecraft_compose::ImageRes;
use texturecraft_core::{Generator, GeneratorGrayscale, GeneratorKind, GrayscaleConversionParameters, KindControls, Source};
use texturecraft_geom::primitives;

use super::generator_fixture::GeneratorFixture;

const RES: u32 = 64;

/// Width of the image: its left half red, its right half blue
const IMAGE_WIDTH: u32 = 16;

fn fixture() -> GeneratorFixture {
    let mesh = primitives::plane(2.0, 1);
    let mut fixture = GeneratorFixture::new(mesh, "builtin:plane", RES);

    let texel = |index: u32| if index % IMAGE_WIDTH < IMAGE_WIDTH / 2 { [1.0, 0.0, 0.0, 1.0] } else { [0.0, 0.0, 1.0, 1.0] };
    let image = ImageRes::new(IMAGE_WIDTH, IMAGE_WIDTH, (0..IMAGE_WIDTH * IMAGE_WIDTH).map(texel).collect(), false);
    fixture.images.insert("split".to_owned(), Arc::new(image));
    fixture
}

/// The values along a row a quarter of the way down, left to right
fn row(fixture: &mut GeneratorFixture, grayscale: GrayscaleConversionParameters) -> Vec<f32> {
    let generator = Generator {
        controls: Some(Box::new(KindControls::Grayscale(GrayscaleConversionParameters { source: Some(Source::image("split")), ..grayscale }))),
        ..Generator::new(GeneratorKind::GrayscaleConversion)
    };
    let texels: Vec<(u32, u32)> = (0..RES).map(|x| (x, RES / 4)).collect();
    fixture.sample(generator, &texels)
}

fn red_channel() -> GrayscaleConversionParameters {
    GrayscaleConversionParameters { grayscale: GeneratorGrayscale::Red, ..GrayscaleConversionParameters::default() }
}

/// How many times the row changes between dark and light
fn edges(values: &[f32]) -> usize {
    values.windows(2).filter(|pair| (pair[0] > 0.5) != (pair[1] > 0.5)).count()
}

#[test]
fn the_type_picks_what_turns_white() {
    let mut fixture = fixture();
    let red = row(&mut fixture, red_channel());
    let blue = row(&mut fixture, GrayscaleConversionParameters { grayscale: GeneratorGrayscale::Blue, ..GrayscaleConversionParameters::default() });

    assert!(red[2] > 0.9 && red[RES as usize - 3] < 0.1, "red on the left: {red:?}");
    assert!(blue[2] < 0.1 && blue[RES as usize - 3] > 0.9, "blue on the right");
}

#[test]
fn tile_repeats_the_image_and_a_half_turn_mirrors_it() {
    let mut fixture = fixture();
    let once = row(&mut fixture, red_channel());
    let four = row(&mut fixture, GrayscaleConversionParameters { tile: 4.0, ..red_channel() });
    let turned = row(&mut fixture, GrayscaleConversionParameters { rotation: 180.0, ..red_channel() });

    assert!(edges(&four) > edges(&once) + 4, "{} vs {}", edges(&four), edges(&once));
    assert!(turned[2] < 0.1 && turned[RES as usize - 3] > 0.9, "the red half swaps sides");
}

#[test]
fn safe_rotation_turns_by_quarter_turns_only() {
    let mut fixture = fixture();
    let snapped = row(&mut fixture, GrayscaleConversionParameters { rotation: 170.0, ..red_channel() });
    let half = row(&mut fixture, GrayscaleConversionParameters { rotation: 180.0, ..red_channel() });

    assert_eq!(snapped, half);
}
