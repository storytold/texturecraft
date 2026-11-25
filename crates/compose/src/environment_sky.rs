//! The built-in environments (drawn in code, no image assets) and their diffuse light as spherical harmonics, for the viewport and Baked Lighting Environment

use std::f32::consts::{PI, TAU};

use rayon::prelude::*;
use texturecraft_geom::glam::{Vec2, Vec3};

use crate::soft_box::SoftBox;

/// The width the viewport draws a built-in environment at (half as tall)
pub const VIEWPORT_WIDTH: u32 = 512;

/// The width an environment is box-averaged down to before its diffuse light is worked out: averaging keeps a sun smaller than a texel
const SH_WIDTH: u32 = 128;

/// The brightest radiance the viewport keeps of an imported environment (it stores half floats)
const HALF_FLOAT_MAX: f32 = 65_000.0;

/// The irradiance convolution factor of each SH band (the cosine lobe's π, 2π/3 and π/4), one per coefficient
const COSINE_LOBE_BANDS: [f32; 9] = [PI, 2.0 * PI / 3.0, 2.0 * PI / 3.0, 2.0 * PI / 3.0, PI / 4.0, PI / 4.0, PI / 4.0, PI / 4.0, PI / 4.0];

/// The linear radiance of a component `v` of an imported environment's texel (sRGB-encoded when `srgb`): not a number is 0, capped at the half-float range, sRGB at 1
pub fn environment_linear(v: f32, srgb: bool) -> f32 {
    let v = if v.is_finite() { v.clamp(0.0, HALF_FLOAT_MAX) } else { 0.0 };
    if srgb { texturecraft_texel::srgb_to_linear(v.min(1.0)) } else { v }
}

/// The direction at equirect place `u = φ / 2π`, `v = θ / π`: `(sin θ cos φ, cos θ, sin θ sin φ)`, row 0 straight up
pub fn dir_of(u: f32, v: f32) -> Vec3 {
    let (theta, phi) = (v * PI, u * TAU);
    Vec3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin())
}

/// The equirect place of direction `d`
pub fn uv_of(d: Vec3) -> Vec2 {
    let phi = d.z.atan2(d.x);
    Vec2::new((phi / TAU).rem_euclid(1.0), d.y.clamp(-1.0, 1.0).acos() / PI)
}

/// Smoothstep of `t` over 0..1
pub(crate) fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How fast a sky turns from its horizon to its zenith and to its ground (powers of the height; under 1 turns early)
const ZENITH_FALLOFF: f32 = 0.6;
const GROUND_FALLOFF: f32 = 0.25;

/// A sky blending `horizon` up to `zenith` and down to `ground`
fn sky(d: Vec3, zenith: Vec3, horizon: Vec3, ground: Vec3) -> Vec3 {
    if d.y >= 0.0 { horizon.lerp(zenith, d.y.powf(ZENITH_FALLOFF)) } else { horizon.lerp(ground, (-d.y).powf(GROUND_FALLOFF)) }
}

/// The Sunset sun's disc: the cosine of its edge from the centre, the width of its soft rim, and its brightness
const SUN_EDGE_COS: f32 = 0.9985;
const SUN_EDGE_WIDTH: f32 = 0.0012;
const SUN_RADIANCE: f32 = 120.0;

/// The light a built-in environment `name` sends from direction `d` (unknown names give the default studio)
pub fn radiance(name: &str, d: Vec3) -> Vec3 {
    if name == "Sunset" {
        let base = sky(d, Vec3::new(0.15, 0.25, 0.55), Vec3::new(1.4, 0.65, 0.3), Vec3::new(0.08, 0.06, 0.05));
        let sun_dir = dir_of(0.12, 0.47);
        let sun = smooth((d.dot(sun_dir) - SUN_EDGE_COS) / SUN_EDGE_WIDTH) * SUN_RADIANCE;
        return base + Vec3::new(1.0, 0.6, 0.3) * sun;
    }

    let (azimuth, elevation) = SoftBox::angles(d);
    match name {
        "Studio Contrast" => {
            let base = sky(d, Vec3::splat(0.04), Vec3::splat(0.03), Vec3::splat(0.01));
            base + SoftBox { azimuth: 35.0, elevation: 30.0, half_width: 22.0, half_height: 18.0, color: Vec3::new(14.0, 13.2, 12.0) }.eval(azimuth, elevation)
                + SoftBox { azimuth: -150.0, elevation: 40.0, half_width: 6.0, half_height: 30.0, color: Vec3::new(4.0, 4.4, 5.0) }.eval(azimuth, elevation)
        }
        "Overcast" => {
            let base = sky(d, Vec3::new(1.2, 1.25, 1.35), Vec3::new(0.95, 0.95, 0.95), Vec3::new(0.18, 0.2, 0.15));
            base + SoftBox { azimuth: 60.0, elevation: 55.0, half_width: 40.0, half_height: 25.0, color: Vec3::splat(0.6) }.eval(azimuth, elevation)
        }
        "Warehouse" => {
            let mut c = sky(d, Vec3::splat(0.25), Vec3::splat(0.18), Vec3::splat(0.06));
            for k in 0..6 {
                c += SoftBox { azimuth: -150.0 + k as f32 * 60.0, elevation: 50.0, half_width: 6.0, half_height: 3.0, color: Vec3::new(9.0, 8.5, 7.5) }
                    .eval(azimuth, elevation);
            }
            c + SoftBox { azimuth: 90.0, elevation: 5.0, half_width: 30.0, half_height: 12.0, color: Vec3::new(1.6, 1.8, 2.2) }.eval(azimuth, elevation)
        }
        "Night" => {
            let base = sky(d, Vec3::new(0.01, 0.015, 0.04), Vec3::new(0.03, 0.03, 0.06), Vec3::splat(0.005));
            base + SoftBox { azimuth: 20.0, elevation: 15.0, half_width: 10.0, half_height: 10.0, color: Vec3::new(0.4, 1.6, 4.0) }.eval(azimuth, elevation)
                + SoftBox { azimuth: -110.0, elevation: 10.0, half_width: 12.0, half_height: 8.0, color: Vec3::new(4.0, 0.8, 1.6) }.eval(azimuth, elevation)
        }

        // "Studio Soft" (default)
        _ => {
            let base = sky(d, Vec3::new(0.5, 0.52, 0.56), Vec3::new(0.32, 0.32, 0.33), Vec3::new(0.08, 0.075, 0.07));
            base + SoftBox { azimuth: 40.0, elevation: 35.0, half_width: 28.0, half_height: 20.0, color: Vec3::new(5.5, 5.2, 4.8) }.eval(azimuth, elevation)
                + SoftBox { azimuth: -125.0, elevation: 20.0, half_width: 35.0, half_height: 22.0, color: Vec3::new(1.1, 1.25, 1.5) }.eval(azimuth, elevation)
                + SoftBox { azimuth: 175.0, elevation: 55.0, half_width: 8.0, half_height: 25.0, color: Vec3::new(3.0, 3.0, 3.0) }.eval(azimuth, elevation)
        }
    }
}

/// The 9 real spherical-harmonics basis functions at unit direction `d` (the standard band 0..2 constants)
pub fn sh_basis(d: Vec3) -> [f32; 9] {
    let (x, y, z) = (d.x, d.y, d.z);
    [
        0.282_095,
        0.488_603 * y,
        0.488_603 * z,
        0.488_603 * x,
        1.092_548 * x * y,
        1.092_548 * y * z,
        0.315_392 * (3.0 * z * z - 1.0),
        1.092_548 * x * z,
        0.546_274 * (x * x - y * y),
    ]
}

/// Diffuse irradiance as 9 SH coefficients of the equirect image `width` x `height` whose texels `radiance` gives (already convolved with the cosine lobe and divided by π, so `Σ c·Y(n)` times albedo is the diffuse radiance)
pub fn sh9(width: u32, height: u32, radiance: impl Fn(u32, u32) -> Vec3) -> [[f32; 3]; 9] {
    let (w, h) = (width.max(1), height.max(1));

    // Sample each large texel across its angular footprint so a constant sky lights all directions
    const MIN_COLUMNS: u32 = 32;
    const MIN_ROWS: u32 = 16;
    let samples_x = MIN_COLUMNS.div_ceil(w);
    let samples_y = MIN_ROWS.div_ceil(h);
    let (w, h) = (w * samples_x, h * samples_y);
    let mut sums = [Vec3::ZERO; 9];
    for y in 0..h {
        let theta = PI * (y as f32 + 0.5) / h as f32;
        let solid_angle = (TAU / w as f32) * (PI / h as f32) * theta.sin();
        for x in 0..w {
            let direction = dir_of((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);

            // No firefly clamp: the projection integrates, so a bright sun lights diffuse fully
            let weighted = radiance(x / samples_x, y / samples_y) * solid_angle;
            for (sum, basis) in sums.iter_mut().zip(sh_basis(direction)) {
                *sum += weighted * basis;
            }
        }
    }
    std::array::from_fn(|k| (sums[k] * COSINE_LOBE_BANDS[k] / PI).to_array())
}

/// [`sh9`] of the equirect image `width` x `height` whose texels `radiance` gives, box-halved while wider than [`SH_WIDTH`] (an odd last row or column is dropped, as the viewport drops it)
pub fn sh9_averaged(width: u32, height: u32, radiance: impl Fn(u32, u32) -> Vec3 + Sync) -> [[f32; 3]; 9] {
    if width <= SH_WIDTH {
        return sh9(width, height, radiance);
    }

    let mut image = half(width, height, radiance);
    while image.0 > SH_WIDTH {
        let (w, h, data) = &image;
        image = half(*w, *h, |x, y| data.get((y * w + x) as usize).copied().unwrap_or(Vec3::ZERO));
    }
    let (w, h, data) = image;
    sh9(w, h, |x, y| data.get((y * w + x) as usize).copied().unwrap_or(Vec3::ZERO))
}

/// The equirect image `width` x `height` whose texels `texel` gives, box-downsampled by 2 (wrapping around, clamped at the poles): its width, height and texels
pub fn half(width: u32, height: u32, texel: impl Fn(u32, u32) -> Vec3 + Sync) -> (u32, u32, Vec<Vec3>) {
    let (w, h) = ((width / 2).max(1), (height / 2).max(1));
    let get = |x: u32, y: u32| texel(x % width.max(1), y.min(height.saturating_sub(1)));
    let mut data = vec![Vec3::ZERO; (w * h) as usize];
    data.par_chunks_mut(w as usize).enumerate().for_each(|(y, row)| {
        for (x, texel) in row.iter_mut().enumerate() {
            let (sx, sy) = (x as u32 * 2, y as u32 * 2);
            *texel = (get(sx, sy) + get(sx + 1, sy) + get(sx, sy + 1) + get(sx + 1, sy + 1)) * 0.25;
        }
    });
    (w, h, data)
}
