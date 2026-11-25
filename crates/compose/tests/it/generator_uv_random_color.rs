//! The UV Random Color generator on a cube: one value per UV island, reshuffled by the seed, or picked along a gradient

use texturecraft_core::{Generator, GeneratorKind, KindControls, UvRandomColorParameters};
use texturecraft_geom::{TrianglePartition, primitives};

use super::generator_fixture::GeneratorFixture;

const RES: u32 = 128;

fn fixture() -> GeneratorFixture {
    let mesh = primitives::cube(1.0);
    GeneratorFixture::new(mesh, "builtin:cube", RES)
}

/// Every covered texel and its UV island
fn covered(fixture: &GeneratorFixture) -> Vec<((u32, u32), usize)> {
    let islands = TrianglePartition::uv_islands(&fixture.mesh, 0);
    (0..RES * RES)
        .filter_map(|index| {
            let triangle = *fixture.gbuf.tri.get(index as usize)?;
            islands.index_of(triangle).map(|island| ((index % RES, index / RES), island))
        })
        .collect()
}

fn mask(fixture: &mut GeneratorFixture, random: UvRandomColorParameters, seed: u32, texels: &[(u32, u32)]) -> Vec<f32> {
    let generator = Generator { controls: Some(Box::new(KindControls::UvRandomColor(random))), seed, ..Generator::new(GeneratorKind::UvRandomColor) };
    fixture.sample(generator, texels)
}

/// Each island's values, by island
fn by_island(texels: &[((u32, u32), usize)], values: &[f32]) -> std::collections::BTreeMap<usize, Vec<f32>> {
    let mut islands = std::collections::BTreeMap::<usize, Vec<f32>>::new();
    for ((_, island), value) in texels.iter().zip(values) {
        islands.entry(*island).or_default().push(*value);
    }
    islands
}

#[test]
fn each_island_is_one_value_and_the_islands_differ() {
    let mut fixture = fixture();
    let texels = covered(&fixture);
    let at: Vec<(u32, u32)> = texels.iter().map(|(texel, _)| *texel).collect();

    let islands = by_island(&texels, &mask(&mut fixture, UvRandomColorParameters::default(), 0, &at));
    assert!(islands.len() > 1, "the cube has several UV islands");
    assert!(islands.values().all(|values| values.iter().all(|value| (value - values[0]).abs() < 1e-6)));
    let firsts: Vec<f32> = islands.values().map(|values| values[0]).collect();
    assert!(firsts.iter().any(|value| (value - firsts[0]).abs() > 0.01));
}

#[test]
fn the_seed_reshuffles_the_values() {
    let mut fixture = fixture();
    let at: Vec<(u32, u32)> = covered(&fixture).iter().map(|(texel, _)| *texel).collect();

    let first = mask(&mut fixture, UvRandomColorParameters::default(), 0, &at);
    let second = mask(&mut fixture, UvRandomColorParameters::default(), 2, &at);
    assert_ne!(first, second);
}

#[test]
fn a_one_color_gradient_colors_every_island_alike() {
    let mut fixture = fixture();
    let at: Vec<(u32, u32)> = covered(&fixture).iter().map(|(texel, _)| *texel).collect();

    let values =
        mask(&mut fixture, UvRandomColorParameters { custom_gradient: true, gradient: vec![[0.1, 0.2, 0.3]], ..UvRandomColorParameters::default() }, 0, &at);
    assert!(values.iter().all(|value| (value - 0.2).abs() < 1e-5), "the mean of its channels");
}

#[test]
fn moving_a_gradient_stop_brightens_the_islands() {
    let mut fixture = fixture();
    let at: Vec<(u32, u32)> = covered(&fixture).iter().map(|(texel, _)| *texel).collect();
    let black_to_white = |positions: Vec<f32>| UvRandomColorParameters {
        custom_gradient: true,
        gradient: vec![[0.0; 3], [1.0; 3]],
        positions,
        ..UvRandomColorParameters::default()
    };

    let even = mask(&mut fixture, black_to_white(Vec::new()), 0, &at);
    let white_early = mask(&mut fixture, black_to_white(vec![0.0, 0.2]), 0, &at);
    let mean = |values: &[f32]| values.iter().sum::<f32>() / values.len().max(1) as f32;
    assert!(mean(&white_early) > mean(&even), "white reached sooner along the gradient: brighter islands");
    assert!(white_early.iter().zip(&even).all(|(early, even)| early >= even), "no island darker");
}
