//! Image-based lighting, computed on the CPU: procedural HDR environments (no image assets),
//! a GGX-prefiltered mip chain for specular, and 9 spherical-harmonics coefficients for
//! diffuse.
//!
//! Equirect convention: `u = φ / 2π`, `v = θ / π`, direction
//! `(sin θ cos φ, cos θ, sin θ sin φ)`; row 0 is straight up.

use std::f32::consts::{PI, TAU};

use rayon::prelude::*;
use texturecraft_compose::environment_sky;
use texturecraft_geom::glam::{Vec2, Vec3};

/// An equirect HDR image (linear RGB).
#[derive(Clone, Debug)]
pub struct EnvImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<[f32; 3]>,
}

pub use texturecraft_compose::environment_sky::{dir_of, radiance, sh_basis, uv_of};
pub use texturecraft_core::project::ENVIRONMENTS;

/// Render a built-in environment (unknown names give the default studio).
pub fn environment(name: &str, width: u32) -> EnvImage {
    let w = width.clamp(16, 4096);
    let h = w / 2;
    let mut data = vec![[0.0f32; 3]; (w * h) as usize];
    data.par_chunks_mut(w as usize).enumerate().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let d = dir_of((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
            *px = radiance(name, d).to_array();
        }
    });
    EnvImage { width: w, height: h, data }
}

impl EnvImage {
    fn get(&self, x: i64, y: i64) -> Vec3 {
        let xx = x.rem_euclid(self.width as i64) as u32;
        let yy = y.clamp(0, self.height as i64 - 1) as u32;
        self.data.get((yy * self.width + xx) as usize).map(|v| Vec3::from(*v)).unwrap_or(Vec3::ZERO)
    }

    /// Bilinear lookup by direction.
    pub fn sample(&self, d: Vec3) -> Vec3 {
        let uv = uv_of(d);
        let (fx, fy) = (uv.x * self.width as f32 - 0.5, uv.y * self.height as f32 - 0.5);
        let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let a = self.get(x0, y0).lerp(self.get(x0 + 1, y0), tx);
        let b = self.get(x0, y0 + 1).lerp(self.get(x0 + 1, y0 + 1), tx);
        a.lerp(b, ty)
    }

    /// Box-downsample by 2.
    pub fn half(&self) -> EnvImage {
        Self::half_from(self.width, self.height, |x, y| self.get(i64::from(x), i64::from(y)))
    }

    fn half_from(width: u32, height: u32, texel: impl Fn(u32, u32) -> Vec3 + Sync) -> Self {
        let (width, height, data) = environment_sky::half(width, height, texel);
        Self { width, height, data: data.into_iter().map(|rgb| rgb.to_array()).collect() }
    }

    /// An imported equirect image (`rgba` row-major, top row first, sRGB-encoded when `srgb`) as
    /// linear radiance: non-finite and negative values become 0, values are capped to the f16
    /// range, and images larger than [`MAX_IMPORT_WIDTH`] are box-downsampled. None when the
    /// pixel count does not match the size.
    pub fn from_rgba(width: u32, height: u32, rgba: &[[f32; 4]], srgb: bool) -> Option<EnvImage> {
        Self::from_pixels(width, height, rgba, srgb)
    }

    /// An equirectangular linear RGB image with the same radiance and size limits as an RGBA import.
    pub fn from_linear_rgb(width: u32, height: u32, rgb: &[[f32; 3]]) -> Option<EnvImage> {
        Self::from_pixels(width, height, rgb, false)
    }

    fn from_pixels<const COMPONENTS: usize>(width: u32, height: u32, pixels: &[[f32; COMPONENTS]], srgb: bool) -> Option<EnvImage> {
        if width == 0 || height == 0 || pixels.len() as u64 != u64::from(width) * u64::from(height) {
            return None;
        }

        let linear_rgb = |pixel: &[f32; COMPONENTS]| std::array::from_fn(|component| environment_sky::environment_linear(pixel[component], srgb));
        let mut img = if width > MAX_IMPORT_WIDTH || height > MAX_IMPORT_WIDTH {
            // Linearize only the texels that contribute to the first reduced level
            Self::half_from(width, height, |x, y| Vec3::from(linear_rgb(&pixels[y as usize * width as usize + x as usize])))
        } else {
            Self { width, height, data: pixels.par_iter().map(linear_rgb).collect() }
        };

        while img.width > MAX_IMPORT_WIDTH || img.height > MAX_IMPORT_WIDTH {
            img = img.half();
        }

        // The mip chain needs at least 4×2 texels at its last level
        if img.width < 128 || img.height < 64 {
            let (w, h) = (img.width.max(128), img.height.max(64));
            let data = (0..w * h).map(|i| img.sample(dir_of(((i % w) as f32 + 0.5) / w as f32, ((i / w) as f32 + 0.5) / h as f32)).to_array()).collect();
            img = EnvImage { width: w, height: h, data };
        }
        Some(img)
    }
}

/// Widest imported environment kept (2K equirect: about 5.6 texels per degree).
pub const MAX_IMPORT_WIDTH: u32 = 2048;

fn hammersley(i: u32, n: u32) -> Vec2 {
    Vec2::new(i as f32 / n as f32, i.reverse_bits() as f32 * 2.328_306_4e-10)
}

/// GGX-prefiltered mip chain: level `l` of `levels` is filtered for roughness `l / (levels-1)`.
pub fn prefilter(env: &EnvImage, levels: u32) -> Vec<EnvImage> {
    let levels = levels.clamp(1, 8);
    // Clamp fireflies, then build a box pyramid to sample from (filtered importance sampling).
    let clamped = EnvImage { data: env.data.iter().map(|c| c.map(|v| v.min(64.0))).collect(), ..env.clone() };
    let mut pyramid = vec![clamped];
    while pyramid.len() < 10 && pyramid.last().is_some_and(|p| p.width > 8) {
        if let Some(next) = pyramid.last().map(EnvImage::half) {
            pyramid.push(next);
        }
    }
    let mut out = vec![env.clone()];
    let mut size = (env.width, env.height);
    for l in 1..levels {
        size = ((size.0 / 2).max(4), (size.1 / 2).max(2));
        let rough = l as f32 / (levels - 1) as f32;
        let a = (rough * rough).max(1e-3);
        let samples = 128u32;
        let (w, h) = size;
        let sa_texel = 4.0 * PI / (env.width as f32 * env.height as f32);
        // With n = v, every sample's half vector, n·l and source level depend only on its index:
        // tabulate them once per level as (tangent-space half vector, n·l, pyramid level).
        let table: Vec<(Vec3, f32, &EnvImage)> = (0..samples)
            .filter_map(|i| {
                let xi = hammersley(i, samples);
                let phi = TAU * xi.x;
                let cos_t = ((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y)).max(0.0).sqrt();
                let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
                let ndl = 2.0 * cos_t * cos_t - 1.0;
                if ndl <= 0.0 {
                    return None;
                }
                let dd = a * a / (PI * ((cos_t * cos_t) * (a * a - 1.0) + 1.0).powi(2)).max(1e-8);
                let sa_sample = 1.0 / (samples as f32 * dd / 4.0 + 1e-6);
                let lod = (0.5 * (sa_sample / sa_texel).log2() + 1.0).clamp(0.0, (pyramid.len() - 1) as f32);
                let src = pyramid.get(lod.round() as usize).unwrap_or(&pyramid[0]);
                Some((Vec3::new(sin_t * phi.cos(), sin_t * phi.sin(), cos_t), ndl, src))
            })
            .collect();
        let wsum: f32 = table.iter().map(|t| t.1).sum();
        let mut data = vec![[0.0f32; 3]; (w * h) as usize];
        data.par_chunks_mut(w as usize).enumerate().for_each(|(y, row)| {
            for (x, px) in row.iter_mut().enumerate() {
                let n = dir_of((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                let up = if n.y.abs() < 0.999 { Vec3::Y } else { Vec3::X };
                let tx = up.cross(n).normalize();
                let ty = n.cross(tx);
                let mut acc = Vec3::ZERO;
                for (hv, ndl, src) in &table {
                    let hw = tx * hv.x + ty * hv.y + n * hv.z;
                    acc += src.sample(hw * (2.0 * hv.z) - n) * *ndl;
                }
                *px = (acc / wsum.max(1e-6)).to_array();
            }
        });
        out.push(EnvImage { width: w, height: h, data });
    }
    out
}

/// Diffuse irradiance as 9 SH coefficients, convolved with the cosine lobe and divided by π, so `Σ c·Y(n)` times albedo is the diffuse radiance
pub fn sh9(env: &EnvImage) -> [[f32; 3]; 9] {
    environment_sky::sh9_averaged(env.width, env.height, |x, y| env.get(x as i64, y as i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uv_and_direction_round_trip() {
        for i in 1..20 {
            for j in 1..10 {
                let (u, v) = (i as f32 / 20.0, j as f32 / 10.0);
                let back = uv_of(dir_of(u, v));
                assert!((back.x - u).abs() < 1e-4 && (back.y - v).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn sh_of_a_uniform_environment_is_constant_irradiance() {
        let env = EnvImage { width: 64, height: 32, data: vec![[1.0, 1.0, 1.0]; 64 * 32] };
        let c = sh9(&env);
        for d in [Vec3::Y, Vec3::X, Vec3::NEG_Z, Vec3::new(0.3, -0.8, 0.5).normalize()] {
            let b = sh_basis(d);
            let e: f32 = (0..9).map(|k| c[k][0] * b[k]).sum();
            assert!((e - 1.0).abs() < 0.03, "{e}");
        }
    }

    #[test]
    fn prefilter_levels_shrink_and_stay_finite() {
        let env = environment("Studio Soft", 128);
        let mips = prefilter(&env, 5);
        assert_eq!(mips.len(), 5);
        assert!(mips[4].width < mips[1].width);
        for m in &mips {
            assert!(m.data.iter().all(|c| c.iter().all(|v| v.is_finite() && *v >= 0.0)));
        }
        for name in ENVIRONMENTS {
            let e = environment(name, 64);
            assert!(e.data.iter().any(|c| c[0] > 0.5), "{name} has no light");
        }
    }

    #[test]
    fn imported_images_become_safe_linear_environments() {
        assert!(EnvImage::from_rgba(4, 2, &[[0.0; 4]; 7], false).is_none(), "pixel count must match");
        // HDR values stay above 1; NaN, infinity and negatives become 0; tiny images grow to 128×64.
        let px = [[50.0, f32::NAN, -1.0, 1.0], [f32::INFINITY, 0.5, 0.25, 1.0]];
        let e = EnvImage::from_rgba(2, 1, &px, false).unwrap();
        assert_eq!((e.width, e.height), (128, 64));
        assert!(e.data.iter().all(|c| c.iter().all(|v| v.is_finite() && *v >= 0.0)));
        assert!(e.data.iter().any(|c| c[0] > 40.0), "HDR range is kept");
        // 8-bit images are sRGB-decoded; big ones are halved down to the import limit.
        let e = EnvImage::from_rgba(4096, 2048, &vec![[0.5, 0.5, 0.5, 1.0]; 4096 * 2048], true).unwrap();
        assert_eq!((e.width, e.height), (MAX_IMPORT_WIDTH, MAX_IMPORT_WIDTH / 2));
        assert!((e.data[0][0] - 0.214).abs() < 0.01, "{}", e.data[0][0]);
        let mips = prefilter(&e, 6);
        assert!(mips.iter().all(|m| m.data.iter().all(|c| c.iter().all(|v| v.is_finite()))));
    }

    /// CPU cost of a 2K import: `cargo test -p texturecraft-render --release -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_prefilter_2k() {
        let src = environment("Sunset", 2048);
        let t = std::time::Instant::now();
        let mips = prefilter(&src, 6);
        let sh = sh9(&src);
        println!("2K environment: prefilter + SH {:.0} ms ({} levels, sh0 {:?})", t.elapsed().as_secs_f64() * 1000.0, mips.len(), sh[0]);
    }
}
