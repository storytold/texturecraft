//! The UV Checker generator on a plane filling its UV tile: a checker with grid lines, its border along the tile's edge

use texturecraft_core::{Generator, GeneratorKind, KindControls, UvCheckerParameters, UvRandomColorParameters};
use texturecraft_geom::primitives;

use super::generator_fixture::GeneratorFixture;

const RES: u32 = 256;

fn fixture() -> GeneratorFixture {
    let mesh = primitives::plane(2.0, 1);
    GeneratorFixture::new(mesh, "builtin:plane", RES)
}

/// The middle row, left to right
fn row(fixture: &mut GeneratorFixture, checker: UvCheckerParameters) -> Vec<f32> {
    let generator = Generator { controls: Some(Box::new(KindControls::UvChecker(checker))), ..Generator::new(GeneratorKind::UvChecker) };
    generator_row(fixture, generator)
}

fn generator_row(fixture: &mut GeneratorFixture, generator: Generator) -> Vec<f32> {
    let texels: Vec<(u32, u32)> = (0..RES).map(|x| (x, RES / 2 + 3)).collect();
    fixture.sample(generator, &texels)
}

/// The distinct values of `values`, to 0.01
fn tones(values: &[f32]) -> std::collections::BTreeSet<u32> {
    values.iter().map(|value| (value * 100.0).round() as u32).collect()
}

#[test]
fn without_grid_lines_the_checker_has_two_tones_and_a_border() {
    let mut fixture = fixture();
    let values = row(&mut fixture, UvCheckerParameters { grid_opacity: 0.0, grid_tile: 1, ..UvCheckerParameters::default() });

    assert_eq!(tones(&values[8..RES as usize - 8]).len(), 2, "two checker tones inside: {:?}", tones(&values));
    assert!((values[0] - 1.0 / 3.0).abs() < 0.01, "the red border reads a third: {}", values[0]);
}

#[test]
fn grid_lines_only_darken() {
    let mut fixture = fixture();
    let plain = row(&mut fixture, UvCheckerParameters { grid_opacity: 0.0, ..UvCheckerParameters::default() });
    let lined = row(&mut fixture, UvCheckerParameters::default());

    assert!(lined.iter().zip(&plain).all(|(lined, plain)| lined <= plain));
    assert!(lined.iter().zip(&plain).any(|(lined, plain)| lined < plain));
}

#[test]
fn checker_keeps_its_pattern_after_random_color_on_the_same_mesh() {
    let mut fixture = fixture();

    // These controls encode to the same parameter bits when the kind is omitted
    let random = UvRandomColorParameters { custom_gradient: true, gradient: vec![[1.0, 0.0, 0.5], [0.5, 0.5, 0.0]], positions: vec![0.0, 1.0], quantity: None };
    let generator = Generator { controls: Some(Box::new(KindControls::UvRandomColor(random))), ..Generator::new(GeneratorKind::UvRandomColor) };
    let checker =
        UvCheckerParameters { random_colors: true, grid_tile: 2, grid_opacity: 1.0, border_width: 0.0, checker_color: [0.5; 3], border_color: [0.0, 0.0, 1.0] };

    let island = generator_row(&mut fixture, generator);
    let pattern = row(&mut fixture, checker);

    assert_eq!(tones(&island).len(), 1, "the plane is a single island");
    assert!(tones(&pattern).len() > 1, "the checker varies across that island");
}
