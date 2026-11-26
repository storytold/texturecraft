//! The section and light controls of the Curvature, Ambient Occlusion, Position, World Space Normals and Light generators on a sphere, beside their earlier model

use std::sync::Arc;

use texturecraft_compose::BakedMaps;
use texturecraft_core::{AxisBlend, BakedMap, Generator, GeneratorKind, LightParameters, SectionParameters, Source};
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::primitives;

use super::generator_fixture::GeneratorFixture;

const RES: u32 = 128;

/// How far a mask value may sit from the expected one
const TOLERANCE: f32 = 1e-3;

/// A unit sphere with a white fill layer
fn fixture() -> GeneratorFixture {
    let mesh = primitives::sphere(1.0, 32, 16);
    GeneratorFixture::new(mesh, "builtin:sphere", RES)
}

/// A baked map of `map` holding `value` everywhere
fn uniform_map(map: BakedMap, value: f32) -> BakedMaps {
    BakedMaps { resolution: RES, maps: [(map, Arc::new(vec![value; (RES * RES) as usize * map.components()]))].into() }
}

/// Every covered texel with its world position and normal
fn surface(fixture: &GeneratorFixture) -> Vec<((u32, u32), Vec3, Vec3)> {
    (0..RES * RES)
        .filter_map(|index| fixture.gbuf.surface(&fixture.mesh, index as usize).map(|surface| ((index % RES, index / RES), surface.pos, surface.nrm)))
        .collect()
}

/// The earlier model's smooth step of `signal` at `generator`'s level and softness
fn earlier_ramp(generator: &Generator, signal: f32) -> f32 {
    let threshold = 1.0 - generator.level;
    let ramp = ((signal - (threshold - generator.softness)) / (2.0 * generator.softness)).clamp(0.0, 1.0);
    ramp * ramp * (3.0 - 2.0 * ramp)
}

fn earlier(kind: GeneratorKind) -> Generator {
    Generator { section: None, light: None, ..Generator::new(kind) }
}

fn with_section(kind: GeneratorKind, change: impl FnOnce(&mut SectionParameters)) -> Generator {
    let mut generator = Generator::new(kind);
    let mut section = generator.section.unwrap();
    change(&mut section);
    generator.section = Some(section);
    generator
}

/// `generator` with a uniform `value` plugged into its Position input
fn plugged(mut generator: Generator, value: f32) -> Generator {
    generator.inputs.insert(BakedMap::Position, Source::Uniform([value; 3]));
    generator
}

fn with_light(change: impl FnOnce(&mut LightParameters)) -> Generator {
    let mut generator = Generator::new(GeneratorKind::Direction);
    let mut light = generator.light.unwrap();
    change(&mut light);
    generator.light = Some(light);
    generator
}

fn assert_close(actual: &[f32], expected: &[f32], what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}: texels read");
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert!((actual - expected).abs() < TOLERANCE, "{what}: texel {index} is {actual}, expected {expected}");
    }
}

#[test]
fn generators_saved_before_their_new_controls_render_as_before() {
    let mut fixture = fixture();
    let points = surface(&fixture);
    let texels: Vec<_> = points.iter().map(|point| point.0).collect();
    let (low, high) = fixture.mesh.bounds();
    let (center, radius) = ((low + high) * 0.5, (high - low).length() * 0.5);

    let position = earlier(GeneratorKind::Position);
    let expected: Vec<f32> = points.iter().map(|(_, pos, _)| earlier_ramp(&position, ((*pos - center) / radius).y * 0.5 + 0.5)).collect();
    assert_close(&fixture.sample(position, &texels), &expected, "position");

    let direction = earlier(GeneratorKind::Direction);
    let expected: Vec<f32> = points.iter().map(|(_, _, nrm)| earlier_ramp(&direction, nrm.y * 0.5 + 0.5)).collect();
    assert_close(&fixture.sample(direction, &texels), &expected, "direction");

    fixture.baked = uniform_map(BakedMap::AmbientOcclusion, 0.3);
    let occlusion = earlier(GeneratorKind::AmbientOcclusion);
    let expected = vec![earlier_ramp(&occlusion, 0.3); texels.len()];
    assert_close(&fixture.sample(occlusion, &texels), &expected, "ambient occlusion");
    assert_close(&fixture.sample(Generator::new(GeneratorKind::AmbientOcclusion), &texels), &vec![0.3; texels.len()], "new ambient occlusion");

    fixture.baked = uniform_map(BakedMap::Curvature, 0.7);
    let curvature = Generator { detail: None, ..earlier(GeneratorKind::Curvature) };
    let expected = vec![earlier_ramp(&curvature, 0.7); texels.len()];
    assert_close(&fixture.sample(curvature, &texels), &expected, "curvature");
}

#[test]
fn world_space_normals_combine_the_axes_of_the_normal() {
    let mut fixture = fixture();
    let points = surface(&fixture);
    let texels: Vec<_> = points.iter().map(|point| point.0).collect();
    let channel = |axis: usize| -> Vec<f32> { points.iter().map(|(_, _, nrm)| nrm[axis] * 0.5 + 0.5).collect() };

    assert_close(&fixture.sample(Generator::new(GeneratorKind::WorldSpaceNormals), &texels), &channel(1), "top to bottom by default");
    for (index, axis) in [(SectionParameters::RIGHT_TO_LEFT, 0), (SectionParameters::FRONT_TO_BACK, 2)] {
        let generator = with_section(GeneratorKind::WorldSpaceNormals, |section| {
            section.axes[SectionParameters::TOP_TO_BOTTOM].weight = 0.0;
            section.axes[index].weight = 1.0;
        });
        assert_close(&fixture.sample(generator, &texels), &channel(axis), "one axis alone");
    }

    let multiplied = with_section(GeneratorKind::WorldSpaceNormals, |section| {
        section.axes[SectionParameters::RIGHT_TO_LEFT] = texturecraft_core::AxisLayer { invert: true, blend: AxisBlend::Multiply, weight: 1.0 };
    });
    let expected: Vec<f32> = channel(1).iter().zip(channel(0)).map(|(top, right)| top * (1.0 - right)).collect();
    assert_close(&fixture.sample(multiplied, &texels), &expected, "inverted right to left multiplies top to bottom");

    // Without a baked map, the generator's own World Space Normal input reads instead of the mesh
    let mut uniform = Generator::new(GeneratorKind::WorldSpaceNormals);
    uniform.inputs.insert(BakedMap::WorldNormal, Source::Uniform([0.2, 0.9, 0.4]));
    assert_close(&fixture.sample(uniform, &texels), &vec![0.9; texels.len()], "a replaced input");
}

#[test]
fn position_reads_the_mesh_bounds_unless_an_image_is_plugged_in() {
    let mut fixture = fixture();
    let points = surface(&fixture);
    let texels: Vec<_> = points.iter().map(|point| point.0).collect();
    let (low, high) = fixture.mesh.bounds();
    let expected: Vec<f32> = points.iter().map(|(_, pos, _)| (pos.y - low.y) / (high.y - low.y)).collect();
    assert_close(&fixture.sample(Generator::new(GeneratorKind::Position), &texels), &expected, "bounds");

    fixture.baked = uniform_map(BakedMap::Position, 0.25);
    assert_close(&fixture.sample(Generator::new(GeneratorKind::Position), &texels), &expected, "a baked Position map is not read");

    let balanced = plugged(with_section(GeneratorKind::Position, |section| section.balance = 0.25), 0.25);
    assert_close(&fixture.sample(balanced, &texels), &vec![0.25f32.powf(0.2); texels.len()], "a power curve balance");
}

#[test]
fn occlusion_balance_is_linear_and_brightness_lerps() {
    let mut fixture = fixture();
    let texels: Vec<_> = surface(&fixture).iter().map(|point| point.0).collect();
    fixture.baked = uniform_map(BakedMap::AmbientOcclusion, 0.6);
    let balanced = with_section(GeneratorKind::AmbientOcclusion, |section| section.balance = 0.25);
    assert_close(&fixture.sample(balanced, &texels), &vec![0.2; texels.len()], "balance 0.25 stretches the upper half");

    let inverted = with_section(GeneratorKind::AmbientOcclusion, |section| section.invert = true);
    assert_close(&fixture.sample(inverted, &texels), &vec![0.4; texels.len()], "section invert");

    let brighter = plugged(with_section(GeneratorKind::Position, |section| section.brightness = 0.5), 0.4);
    assert_close(&fixture.sample(brighter, &texels), &vec![0.7; texels.len()], "brightness 0.5 halves the way to white");
    let darker = plugged(with_section(GeneratorKind::Position, |section| section.brightness = -0.5), 0.4);
    assert_close(&fixture.sample(darker, &texels), &vec![0.2; texels.len()], "brightness -0.5 halves the value");
}

#[test]
fn section_blur_spreads_occlusion_across_a_step() {
    let mut fixture = fixture();
    let step: Vec<f32> = (0..RES * RES).map(|index| if index % RES < RES / 2 { 0.0 } else { 1.0 }).collect();
    fixture.baked = BakedMaps { resolution: RES, maps: [(BakedMap::AmbientOcclusion, Arc::new(step))].into() };
    let row: Vec<(u32, u32)> = surface(&fixture).iter().map(|point| point.0).filter(|texel| texel.0.abs_diff(RES / 2) <= 2).collect();
    assert!(!row.is_empty());

    let sharp = fixture.sample(Generator::new(GeneratorKind::AmbientOcclusion), &row);
    assert!(sharp.iter().all(|value| *value == 0.0 || *value == 1.0), "unblurred: {sharp:?}");
    let blurred = fixture.sample(with_section(GeneratorKind::AmbientOcclusion, |section| section.blur = 16.0), &row);
    assert!(blurred.iter().all(|value| *value > 0.05 && *value < 0.95), "blurred across the step: {blurred:?}");

    let sharp_position = fixture.sample(Generator::new(GeneratorKind::Position), &row);
    let blurred_position = fixture.sample(with_section(GeneratorKind::Position, |section| section.blur = 16.0), &row);
    assert_close(&blurred_position, &sharp_position, "position has no section blur");
}

#[test]
fn light_angles_light_the_faces_towards_them() {
    let mut fixture = fixture();
    let points = surface(&fixture);
    let facing = |axis: Vec3| -> Vec<(u32, u32)> { points.iter().filter(|(_, _, nrm)| nrm.dot(axis) > 0.97).map(|point| point.0).collect() };

    for (horizontal, vertical, towards) in [(0.0, 0.0, Vec3::NEG_X), (90.0, 0.0, Vec3::Z), (180.0, 0.0, Vec3::X), (0.0, 90.0, Vec3::Y)] {
        let light = with_light(|light| (light.horizontal, light.vertical, light.attenuation) = (horizontal, vertical, 0.0));
        let lit = fixture.sample(light.clone(), &facing(towards));
        let dark = fixture.sample(light, &facing(-towards));
        assert!(lit.iter().all(|value| *value > 0.95), "{horizontal}° {vertical}° lights {towards}: {lit:?}");
        assert!(dark.iter().all(|value| *value == 0.0), "{horizontal}° {vertical}° leaves {} dark: {dark:?}", -towards);
    }

    let inverted = Generator { invert: true, ..with_light(|light| (light.horizontal, light.vertical, light.attenuation) = (0.0, 0.0, 0.0)) };
    assert!(fixture.sample(inverted, &facing(Vec3::X)).iter().all(|value| *value > 0.95), "invert lights the far side");
}

#[test]
fn light_attenuation_darkens_the_side_away_from_the_light() {
    let mut fixture = fixture();
    let sides: Vec<(u32, u32)> = surface(&fixture).iter().filter(|(_, _, nrm)| nrm.y > 0.97).map(|point| point.0).collect();
    let half_lit =
        |attenuation: f32| with_light(|light| (light.horizontal, light.vertical, light.glossiness, light.attenuation) = (0.0, 0.0, 0.0, attenuation));

    let kept = fixture.sample(half_lit(0.0), &sides);
    let attenuated = fixture.sample(half_lit(0.5), &sides);
    for (kept, attenuated) in kept.iter().zip(&attenuated) {
        assert!((kept - 0.5).abs() < 0.15, "the top is about half lit from the side: {kept}");
        assert!(*attenuated < kept * 0.75, "halfway across the bounds, attenuation darkens it: {attenuated} vs {kept}");
    }
}
