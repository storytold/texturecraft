//! The material orbs of the Properties panel: a sphere showing the whole material (every channel, height and normal relief
//! included) lit by a photographed studio, or one channel unlit as the viewport's channel view shows it, rendered at its
//! on-screen size in physical pixels, off the UI thread, cached by look

use egui::{Color32, ColorImage, Painter, Rect, pos2};
use rayon::prelude::*;
use texturecraft_engine::core::{Channel, DIELECTRIC_REFLECTANCE};
use texturecraft_engine::geom::glam::Vec3;
use texturecraft_engine::texel::linear_to_srgb;

use crate::orb_environment::OrbEnvironment;
use crate::orb_material::{OrbMaterial, OrbSample, orb_coords};
use crate::orb_previews::{OrbJob, OrbPreviews};
use crate::preview_image::aces;
use crate::theme::Tokens;

/// Largest orb rendered, in physical pixels
const MAX_SIDE_PX: usize = 512;

/// The sphere's radius in halves of the square
const SPHERE_RADIUS: f32 = 0.84;

/// The lowest roughness shaded (a perfect mirror aliases at orb sizes)
const MIN_ROUGHNESS: f32 = 0.03;

/// How deep the height channel's full range carves the surface, in orb radii
const HEIGHT_DEPTH: f32 = 0.05;

/// Exposure before tone mapping
const EXPOSURE: f32 = 0.55;

/// The least facing a normal turned past the outline keeps, so the reflection and the BRDF fit never see a grazing zero
const MIN_NORMAL_DOT_VIEW: f32 = 1e-4;

/// Draws `material`'s orb into `rect` on the panel's darkest backdrop; each `place` keeps its last picture while a new one renders
pub(crate) fn material_orb(painter: &Painter, rect: Rect, material: &OrbMaterial, tokens: &Tokens, place: impl std::hash::Hash + std::fmt::Debug) {
    let ctx = painter.ctx();
    let side = ((rect.width() * ctx.pixels_per_point()).round() as usize).clamp(1, MAX_SIDE_PX);
    let key = OrbJob::key_of(material, side, tokens.chrome);
    let job = || OrbJob::new(material.clone(), side, tokens.chrome);
    if let Some(texture) = OrbPreviews::texture(ctx, egui::Id::unique(("material_orb", place)), key, job) {
        painter.image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
}

/// The orb `side` pixels square, on `backdrop`
pub(crate) fn render(material: &OrbMaterial, side: usize, backdrop: Color32) -> ColorImage {
    let side = side.clamp(1, MAX_SIDE_PX);
    // One pixel in orb radii: the step for the height slope and the reach of the anti-aliased outline
    let pixel = 2.0 / (side as f32 * SPHERE_RADIUS);

    match material.alone() {
        Some(channel) => {
            let footprint = pixel * std::f32::consts::FRAC_1_PI;
            render_pixels(side, backdrop, pixel, |normal, coverage| (channel_view(channel, material.value(channel, &orb_coords(normal), footprint)), coverage))
        }
        None => {
            let studio = OrbEnvironment::get();
            let uniform = (!material.varies()).then(|| material.sample(&orb_coords(Vec3::Y), 1.0));
            render_pixels(side, backdrop, pixel, |normal, coverage| {
                let (linear, opacity) = shade(material, studio, normal, pixel, uniform);
                (linear.map(|component| linear_to_srgb(aces(component * EXPOSURE))), coverage * opacity)
            })
        }
    }
}

fn render_pixels(side: usize, backdrop: Color32, pixel: f32, shade_pixel: impl Fn(Vec3, f32) -> ([f32; 3], f32) + Sync) -> ColorImage {
    let back = [backdrop.r(), backdrop.g(), backdrop.b()].map(|component| f32::from(component) / 255.0);
    let mut pixels = vec![Color32::TRANSPARENT; side * side];
    pixels.par_chunks_mut(side).enumerate().for_each(|(y, row)| {
        for (x, out) in row.iter_mut().enumerate() {
            let u = ((x as f32 + 0.5) / side as f32 * 2.0 - 1.0) / SPHERE_RADIUS;
            let v = (1.0 - (y as f32 + 0.5) / side as f32 * 2.0) / SPHERE_RADIUS;
            let distance = (u * u + v * v).sqrt();
            let coverage = ((1.0 - distance) / pixel + 0.5).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                *out = backdrop;
                continue;
            }

            // Pixels on the outline shade the sphere's rim
            let (u, v) = if distance > 1.0 { (u / distance, v / distance) } else { (u, v) };
            let normal = Vec3::new(u, v, (1.0 - u * u - v * v).max(0.0).sqrt());
            let (shown, alpha) = shade_pixel(normal, coverage);
            let [r, g, b] = [0, 1, 2].map(|channel| ((back[channel] + (shown[channel] - back[channel]) * alpha) * 255.0).round().clamp(0.0, 255.0) as u8);
            *out = Color32::from_rgb(r, g, b);
        }
    });
    ColorImage::new([side, side], pixels)
}

/// The linear light leaving the orb at view-space unit `direction` from its centre, and the material's opacity there
fn shade(material: &OrbMaterial, env: &OrbEnvironment, direction: Vec3, pixel: f32, uniform: Option<OrbSample>) -> ([f32; 3], f32) {
    // Facing the viewer, the orb's poles sit on its outline, where their UV pinch does not show
    let coords = orb_coords(direction);
    let footprint = pixel * std::f32::consts::FRAC_1_PI;
    let sample = uniform.unwrap_or_else(|| material.sample(&coords, footprint));

    // Tangent frame on the material: east along the UVs, then north
    let east = Vec3::new(-direction.z, 0.0, direction.x).try_normalize().unwrap_or(Vec3::X);
    let north = east.cross(direction).try_normalize().unwrap_or(Vec3::Y);
    let mapped = Vec3::from(sample.normal.map(|component| component * 2.0 - 1.0));
    let mut surface = (east * mapped.x + north * mapped.y + direction * mapped.z).try_normalize().unwrap_or(direction);
    if material.has_relief() {
        let height = material.height(&coords, footprint);
        let slope = |toward: Vec3| (material.height(&orb_coords((direction + toward * pixel).normalize()), footprint) - height) / pixel;
        surface = (surface - (east * slope(east) + north * slope(north)) * HEIGHT_DEPTH).try_normalize().unwrap_or(surface);
    }

    let normal = surface;
    let roughness = sample.roughness.clamp(MIN_ROUGHNESS, 1.0);
    let metallic = sample.metallic.clamp(0.0, 1.0);
    let normal_dot_view = normal.z.max(MIN_NORMAL_DOT_VIEW);

    // Mirror the direction towards the viewer (+Z) about the normal
    let reflected = (2.0 * normal_dot_view * normal - Vec3::Z).normalize_or_zero();
    let specular_light = env.reflected(reflected, roughness);
    let diffuse_light = env.diffuse(normal);
    let (scale, bias) = environment_brdf(roughness, normal_dot_view);
    let mut out = [0.0; 3];
    for (channel, value) in out.iter_mut().enumerate() {
        let base = sample.base[channel].clamp(0.0, 1.0);
        let f0 = DIELECTRIC_REFLECTANCE + (base - DIELECTRIC_REFLECTANCE) * metallic;
        let specular = f0 * scale + bias;
        *value = diffuse_light[channel] * base * (1.0 - metallic) * (1.0 - specular) + specular_light[channel] * specular + sample.emissive[channel].max(0.0);
    }
    (out, sample.opacity.clamp(0.0, 1.0))
}

/// `channel`'s `value` on screen (0..1) as the viewport's channel view shows it: colours sRGB-encoded, height around mid gray
fn channel_view(channel: Channel, value: [f32; 3]) -> [f32; 3] {
    let value = value.map(|c| if c.is_finite() { c } else { 0.0 });
    let shown = match channel {
        Channel::BaseColor | Channel::Emissive | Channel::Specular | Channel::SheenColor => value.map(|c| linear_to_srgb(c.clamp(0.0, 1.0))),
        // The viewport encodes these as it does colours (they share its colour output)
        Channel::Metallic | Channel::Roughness | Channel::Glossiness => [linear_to_srgb(value[0].clamp(0.0, 1.0)); 3],
        Channel::Normal | Channel::CoatNormal => value,
        Channel::Height => [value[0] * 0.5 + 0.5; 3],
        _ => [value[0]; 3],
    };
    shown.map(|c| c.clamp(0.0, 1.0))
}

/// The split-sum environment BRDF as scale and bias on F0, from Karis' analytic fit for mobile (no lookup table)
fn environment_brdf(roughness: f32, normal_dot_view: f32) -> (f32, f32) {
    let [gloss, gloss_offset, scale_base, bias_base] = [1.0 - roughness, roughness * -0.0275 + 0.0425, roughness * -0.572 + 1.04, roughness * 0.022 - 0.04];
    let shared = (gloss * gloss).min((-9.28 * normal_dot_view).exp2()) * gloss + gloss_offset;
    (shared * -1.04 + scale_base, shared * 1.04 + bias_base)
}

#[cfg(test)]
mod tests {
    use texturecraft_engine::core::{Channel, Fill, Procedural, ProceduralKind, Source};

    use super::*;

    const BACKDROP: Color32 = Color32::from_rgb(27, 31, 39);

    fn copper() -> OrbMaterial {
        OrbMaterial::uniform([0.8, 0.35, 0.18], 1.0, 0.35)
    }

    fn at(image: &ColorImage, x: usize, y: usize) -> Color32 {
        image.pixels.get(y * image.size[0] + x).copied().unwrap_or_default()
    }

    /// Mean of how far each pixel's channels spread apart (0 = gray)
    fn saturation(image: &ColorImage) -> f32 {
        let spread = |pixel: &Color32| f32::from(pixel.r().max(pixel.g()).max(pixel.b()) - pixel.r().min(pixel.g()).min(pixel.b()));
        let shown: Vec<f32> = image.pixels.iter().filter(|pixel| **pixel != BACKDROP).map(spread).collect();
        shown.iter().sum::<f32>() / shown.len().max(1) as f32
    }

    #[test]
    fn the_orb_sits_on_its_backdrop_and_shows_its_colour() {
        let image = render(&copper(), 64, BACKDROP);
        assert_eq!(image.pixels[0], BACKDROP, "the corner is off the sphere");

        let middle = at(&image, 32, 32);
        assert!(middle.r() > middle.b(), "copper is warm: {middle:?}");
    }

    #[test]
    fn rougher_materials_spread_their_highlight() {
        let peak = |roughness: f32| {
            let image = render(&OrbMaterial::uniform([0.8, 0.35, 0.18], 1.0, roughness), 64, Color32::BLACK);
            image.pixels.iter().map(|pixel| pixel.r()).max().unwrap_or(0)
        };
        assert!(peak(0.1) > peak(0.9), "a polished orb has a brighter hot spot");
    }

    #[test]
    fn a_metal_reflects_the_studio_in_its_own_colour() {
        // The studio's soft boxes and dark walls show as bright and dark bands across a polished metal, tinted by its colour
        let image = render(&OrbMaterial::uniform([0.95, 0.64, 0.54], 1.0, 0.15), 96, BACKDROP);
        let lightness: Vec<u8> = image.pixels.iter().filter(|pixel| **pixel != BACKDROP).map(|pixel| pixel.r()).collect();
        let (darkest, brightest) = (lightness.iter().min().copied().unwrap_or(0), lightness.iter().max().copied().unwrap_or(0));
        assert!(brightest - darkest > 120, "reflections show light and dark: {darkest}..{brightest}");
        assert!(saturation(&image) > 25.0, "copper stays coloured, not gray: {}", saturation(&image));
    }

    fn fill_with(channel: Channel, source: Source) -> Fill {
        let mut fill = Fill::default();
        fill.sources.insert(channel, source);
        fill
    }

    #[test]
    fn a_colour_procedurals_gradient_map_colours_the_orb() {
        let source = |ramp: Vec<[f32; 3]>| Source::Procedural { pattern: Procedural::new(ProceduralKind::Cells), low: [0.0; 3], high: [1.0; 3], ramp };
        let on = [Channel::BaseColor];
        let gray = render(&OrbMaterial::of_fill(&fill_with(Channel::BaseColor, source(Vec::new())), &on, &Default::default()), 64, BACKDROP);
        let red = render(&OrbMaterial::of_fill(&fill_with(Channel::BaseColor, source(vec![[1.0, 0.0, 0.0]; 2])), &on, &Default::default()), 64, BACKDROP);
        let reddened =
            red.pixels.iter().zip(&gray.pixels).filter(|(r, g)| i32::from(r.r()) - i32::from(r.g()) > i32::from(g.r()) - i32::from(g.g()) + 16).count();
        assert!(reddened > 64 * 64 / 4, "the ramp's red shows on the orb: {reddened} pixels");
    }

    #[test]
    fn every_channel_shows_on_the_orb_relief_included() {
        let flat = render(&OrbMaterial::of_fill(&Fill::default(), &[Channel::BaseColor, Channel::Height, Channel::Normal], &Default::default()), 64, BACKDROP);
        let pattern = Procedural::new(ProceduralKind::Cells);
        for channel in [Channel::BaseColor, Channel::Roughness, Channel::Height, Channel::Normal] {
            let (low, high) = match channel {
                Channel::Normal => ([0.2, 0.5, 0.9], [0.8, 0.5, 0.9]),
                Channel::Height => ([-1.0; 3], [1.0; 3]),
                _ => ([0.0; 3], [1.0; 3]),
            };
            let fill = fill_with(channel, Source::Procedural { pattern: pattern.clone(), low, high, ramp: Vec::new() });
            let on = [Channel::BaseColor, Channel::Roughness, Channel::Height, Channel::Normal];
            let shown = render(&OrbMaterial::of_fill(&fill, &on, &Default::default()), 64, BACKDROP);
            let changed = shown.pixels.iter().zip(&flat.pixels).filter(|(a, b)| a != b).count();
            assert!(changed > 64 * 64 / 4, "{channel:?} changes the orb: {changed} pixels");

            let off = render(&OrbMaterial::of_fill(&fill, &[], &Default::default()), 64, BACKDROP);
            let flat_off = render(&OrbMaterial::of_fill(&Fill::default(), &[], &Default::default()), 64, BACKDROP);
            assert_eq!(off.pixels, flat_off.pixels, "{channel:?} switched off shows its default");
        }
    }

    #[test]
    fn an_image_source_shows_on_the_orb() {
        use std::sync::Arc;
        use texturecraft_engine::compose::ImageRes;

        // Left half black, right half white
        let rgba = (0..64 * 64).map(|i| if i % 64 < 32 { [0.0, 0.0, 0.0, 1.0] } else { [1.0; 4] }).collect();
        let images = std::collections::HashMap::from([("stripes".to_owned(), Arc::new(ImageRes::new(64, 64, rgba, false)))]);
        let fill = fill_with(Channel::BaseColor, Source::Image { name: "stripes".into(), normal_format: Default::default() });
        let image = render(&OrbMaterial::of_fill(&fill, &[Channel::BaseColor], &images), 64, BACKDROP);
        let lightness: Vec<u8> = image.pixels.iter().filter(|pixel| **pixel != BACKDROP).map(|pixel| pixel.g()).collect();
        let (darkest, brightest) = (lightness.iter().min().copied().unwrap_or(0), lightness.iter().max().copied().unwrap_or(0));
        assert!(brightest - darkest > 100, "the black and white halves both show: {darkest}..{brightest}");

        let missing = render(&OrbMaterial::of_fill(&fill, &[Channel::BaseColor], &Default::default()), 64, BACKDROP);
        assert_ne!(missing.pixels, image.pixels, "a missing image reads the default");
    }

    #[test]
    fn hostile_sizes_and_values_render_without_panicking() {
        for side in [0, 1, 2, 100_000] {
            let image = render(&OrbMaterial::uniform([f32::NAN, f32::INFINITY, -1.0], f32::NAN, f32::NEG_INFINITY), side, BACKDROP);
            assert!(image.size[0] >= 1 && image.size[0] <= MAX_SIDE_PX);
        }
    }

    #[test]
    fn the_orb_renders_at_its_size_on_screen_times_the_screen_scale() {
        use std::time::{Duration, Instant};

        use crate::orb_previews::OrbPreviews;

        // 160 points: wider than the old 256-pixel cap once doubled
        for scale in [1.0_f32, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(scale);
            let material = copper();
            let expected = (160.0 * scale) as usize;
            let start = Instant::now();
            while OrbPreviews::shown_size(&ctx, "hidpi") != Some([expected, expected]) && start.elapsed() < Duration::from_secs(20) {
                let mut output = ctx.run_pass(Default::default(), |ctx| {
                    let painter = ctx.layer_painter(egui::LayerId::background());
                    let rect = Rect::from_min_size(pos2(10.0, 10.0), egui::vec2(160.0, 160.0));
                    material_orb(&painter, rect, &material, &Tokens::get(ctx), "hidpi");
                });

                // Nothing draws these textures here
                output.textures_delta.clear();
                std::thread::sleep(Duration::from_millis(5));
            }

            assert_eq!(OrbPreviews::shown_size(&ctx, "hidpi"), Some([expected, expected]), "at {scale}x");
        }
    }

    /// Milliseconds per orb, written by `cargo test --release -p texturecraft-ui-egui --lib -- orb_render_timing --ignored --nocapture`
    #[test]
    #[ignore = "a timing, not a check"]
    fn orb_render_timing() {
        use std::time::Instant;

        use texturecraft_engine::compose::ImageRes;

        let started = Instant::now();
        crate::orb_environment::OrbEnvironment::get();
        eprintln!("studio prefilter (once): {:.1} ms", started.elapsed().as_secs_f64() * 1000.0);
        let rgba = (0..512 * 512).map(|i| [(i % 512) as f32 / 512.0, 0.5, (i / 512) as f32 / 512.0, 1.0]).collect();
        let images = std::collections::HashMap::from([("photo".to_owned(), std::sync::Arc::new(ImageRes::new(512, 512, rgba, true)))]);
        let mut complex = Fill::default();
        complex.sources.insert(Channel::BaseColor, Source::Image { name: "photo".into(), normal_format: Default::default() });
        complex.sources.insert(
            Channel::Roughness,
            Source::Procedural { pattern: Procedural::new(ProceduralKind::Clouds), low: [0.2; 3], high: [0.8; 3], ramp: Vec::new() },
        );
        complex
            .sources
            .insert(Channel::Height, Source::Procedural { pattern: Procedural::new(ProceduralKind::Cells), low: [-1.0; 3], high: [1.0; 3], ramp: Vec::new() });
        complex.sources.insert(
            Channel::Normal,
            Source::Procedural { pattern: Procedural::new(ProceduralKind::Perlin), low: [0.3, 0.5, 0.9], high: [0.7, 0.5, 0.9], ramp: Vec::new() },
        );
        let on = [Channel::BaseColor, Channel::Roughness, Channel::Height, Channel::Normal];
        let looks = [("uniform", copper()), ("image+procedurals+height+normal", OrbMaterial::of_fill(&complex, &on, &images))];
        for (name, look) in looks {
            for side in [25, 100, 200, 400] {
                let runs = 10;
                let started = Instant::now();
                for _ in 0..runs {
                    std::hint::black_box(render(&look, side, BACKDROP));
                }
                eprintln!("{name} {side}px: {:.2} ms", started.elapsed().as_secs_f64() * 1000.0 / f64::from(runs));
            }
        }
    }

    #[test]
    fn a_channel_alone_shows_unlit_as_the_viewports_channel_view_draws_it() {
        let mut fill = fill_with(Channel::BaseColor, Source::Uniform([0.8, 0.35, 0.18]));
        fill.sources.insert(Channel::Roughness, Source::Uniform([0.3; 3]));
        fill.sources.insert(Channel::Height, Source::Uniform([0.5; 3]));
        let on = [Channel::BaseColor, Channel::Roughness, Channel::Height];
        let byte = |value: f32| (value * 255.0).round() as u8;
        let encoded = |value: f32| byte(linear_to_srgb(value));

        for (channel, expected) in [
            (Channel::BaseColor, Color32::from_rgb(encoded(0.8), encoded(0.35), encoded(0.18))),
            (Channel::Roughness, Color32::from_gray(encoded(0.3))),
            (Channel::Height, Color32::from_gray(byte(0.75))),
            (Channel::Metallic, Color32::from_gray(0)),
        ] {
            let image = render(&OrbMaterial::of_channel(&fill, channel, &on, &Default::default()), 64, BACKDROP);

            assert_eq!(at(&image, 32, 32), expected, "{channel:?} at the centre");
            assert_eq!(at(&image, 12, 32), expected, "{channel:?} unlit: the same off the centre");
            assert_eq!(image.pixels[0], BACKDROP, "{channel:?} sits on the backdrop");
        }
    }

    #[test]
    fn the_key_changes_with_the_look_and_only_with_it() {
        let a = OrbMaterial::uniform([0.8, 0.35, 0.18], 1.0, 0.35);
        assert_eq!(a.key(), copper().key());
        assert_ne!(a.key(), OrbMaterial::uniform([0.8, 0.35, 0.18], 1.0, 0.36).key());
    }
}
