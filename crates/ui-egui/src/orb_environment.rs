//! The studio that lights every material orb: a bundled photographed environment (CC0, see ATTRIBUTION.md), prefiltered once for
//! glossy reflections and reduced to spherical harmonics for diffuse light

use std::sync::OnceLock;

use texturecraft_engine::geom::glam::Vec3;
use texturecraft_render::ibl::{self, EnvImage};

/// The bundled environment: a small photo studio with soft boxes, 512 x 256 equirect Radiance HDR
const STUDIO_HDR: &[u8] = include_bytes!("../../../assets/hdr/studio_small_09_512.hdr");

/// Prefiltered levels from mirror (0) to fully rough (last)
const LEVELS: u32 = 6;

/// How far the studio is turned about the vertical axis (radians) so its key soft box lights the orb from the upper left
const TURN: f32 = 4.4;

/// The prefiltered studio
pub(crate) struct OrbEnvironment {
    /// Level `l` holds reflections for roughness `l / (LEVELS - 1)`
    levels: Vec<EnvImage>,
    /// Diffuse light as 9 spherical-harmonics coefficients
    irradiance: [[f32; 3]; 9],
}

impl OrbEnvironment {
    /// The studio, decoded and prefiltered by the first caller (about a tenth of a second in release)
    pub(crate) fn get() -> &'static OrbEnvironment {
        static STUDIO: OnceLock<OrbEnvironment> = OnceLock::new();
        STUDIO.get_or_init(|| OrbEnvironment::from_hdr(STUDIO_HDR).unwrap_or_else(OrbEnvironment::fallback))
    }

    fn from_hdr(bytes: &[u8]) -> Option<OrbEnvironment> {
        let decoded =
            image::load_from_memory_with_format(bytes, image::ImageFormat::Hdr).inspect_err(|e| log::warn!("the orb studio did not load: {e}")).ok()?;

        let rgb = decoded.into_rgb32f();
        let (pixels, _) = rgb.as_raw().as_chunks::<3>();
        let env = EnvImage::from_linear_rgb(rgb.width(), rgb.height(), pixels)?;
        drop(rgb);

        Some(OrbEnvironment::of(&env))
    }

    /// The built-in procedural studio, should the bundled one ever fail to decode
    fn fallback() -> OrbEnvironment {
        OrbEnvironment::of(&ibl::environment("Studio Soft", 256))
    }

    fn of(env: &EnvImage) -> OrbEnvironment {
        OrbEnvironment { levels: ibl::prefilter(env, LEVELS), irradiance: ibl::sh9(env) }
    }

    /// Light reflected towards unit direction `direction` (view space: +X right, +Y up, +Z towards the viewer) by a surface of `roughness`
    pub(crate) fn reflected(&self, direction: Vec3, roughness: f32) -> Vec3 {
        let turned = turn(direction);
        let level = roughness.clamp(0.0, 1.0) * (self.levels.len().saturating_sub(1)) as f32;
        let below = level.floor() as usize;

        let sample = |index: usize| self.levels.get(index).or(self.levels.last()).map_or(Vec3::ZERO, |env| env.sample(turned));
        let fraction = level - level.floor();

        if fraction <= 0.0 { sample(below) } else { sample(below).lerp(sample(below + 1), fraction) }
    }

    /// Diffuse light reaching a surface with unit normal `normal` (view space)
    pub(crate) fn diffuse(&self, normal: Vec3) -> Vec3 {
        let basis = ibl::sh_basis(turn(normal));
        let mut light = Vec3::ZERO;

        for (coefficient, weight) in self.irradiance.iter().zip(basis) {
            light += Vec3::from(*coefficient) * weight;
        }

        light.max(Vec3::ZERO)
    }
}

/// `direction` in the studio's frame
fn turn(direction: Vec3) -> Vec3 {
    let (sin, cos) = TURN.sin_cos();
    Vec3::new(direction.x * cos + direction.z * sin, direction.y, direction.z * cos - direction.x * sin)
}
