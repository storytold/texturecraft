//! Baked Lighting Stylized's settings worked out once per filter, and the shading of each texel with them

use std::f32::consts::PI;

use crate::lighting_output::LightingOutput;
use crate::stylized_lighting::StylizedLighting;
use crate::sun_and_sky::SunAndSky;
use crate::{Channel, finite_unit, mix3};

/// The curvature map's value on a flat surface (below it concave, above convex)
pub const FLAT_CURVATURE: f32 = 0.5;

/// The scale that takes either side of [`FLAT_CURVATURE`] to 0..1
const CURVATURE_SIDE_SCALE: f32 = 2.0;

/// The Cook-Torrance denominator 4·(n·l)·(n·v) with the viewer straight on (n·v = 1), n·l cancelled by the light's own facing
const COOK_TORRANCE_DENOMINATOR: f32 = 4.0;

/// The roughness highlights never go below (a mirror would be a single texel of infinite light)
const MIN_ROUGHNESS: f32 = 0.08;

/// The smoothness below which cavity and edges switch on as a step
const MIN_SMOOTHNESS: f32 = 1e-3;

/// A [`StylizedLighting`] under its [`SunAndSky`], with every value that depends only on the settings worked out once
#[derive(Clone, Debug, PartialEq)]
pub struct StylizedShading {
    /// The sun, Light 1 and Light 2: the unit direction toward each, its intensity and its color
    lights: [([f32; 3], f32, [f32; 3]); 3],
    sky_intensity: f32,
    sky_color: [f32; 3],
    horizon_color: [f32; 3],
    ground_color: [f32; 3],
    /// Diffuse AO, Specular AO, Diffuse Cavity, Specular Cavity and Edges Intensity, each 0..1
    diffuse_ao: f32,
    specular_ao: f32,
    diffuse_cavity: f32,
    specular_cavity: f32,
    edges_intensity: f32,
    cavity_smoothness: f32,
    edges_smoothness: f32,
    specular_glossiness: bool,
    output: LightingOutput,
    /// The reflectance of non-metals at normal incidence, 0..1
    dielectric_reflectance: f32,
    /// The relief the baked height gives the lit normals; None lights the mesh's own normals
    relief: Option<f32>,
}

impl StylizedShading {
    /// `settings` lit by `light`
    pub fn new(settings: &StylizedLighting, light: SunAndSky) -> StylizedShading {
        let sun = (light.sun.direction(), light.sun.intensity, settings.sun_color);
        let [first, second] = settings.lights.map(|extra| (extra.direction(), extra.intensity, extra.color));
        StylizedShading {
            lights: [sun, first, second],
            sky_intensity: light.sky_intensity,
            sky_color: settings.sky_color,
            horizon_color: settings.horizon_color,
            ground_color: settings.ground_color,
            diffuse_ao: finite_unit(light.diffuse_ao),
            specular_ao: finite_unit(settings.specular_ao),
            diffuse_cavity: finite_unit(settings.diffuse_cavity),
            specular_cavity: finite_unit(settings.specular_cavity),
            edges_intensity: finite_unit(settings.edges_intensity),
            cavity_smoothness: settings.cavity_smoothness,
            edges_smoothness: settings.edges_smoothness,
            specular_glossiness: settings.specular_glossiness,
            output: settings.output,
            dielectric_reflectance: finite_unit(settings.dielectric_reflectance),
            relief: (!settings.mesh_only).then_some(settings.height_to_normal),
        }
    }

    /// The relief the baked height gives the normals it lights (Height to Normal Intensity); None lights the mesh's own normals
    pub fn relief(&self) -> Option<f32> {
        self.relief
    }

    /// The ambient light reaching a surface facing `normal` from the sky, horizon and ground
    fn ambient(&self, normal: [f32; 3]) -> [f32; 3] {
        let up = normal[1].clamp(-1.0, 1.0);
        if up >= 0.0 { mix3(self.horizon_color, self.sky_color, up) } else { mix3(self.horizon_color, self.ground_color, -up) }
    }

    /// The lit base color of linear `albedo` facing `normal` (world space, unit) with baked ambient occlusion `ao` and curvature `curvature` ([`FLAT_CURVATURE`] flat), its material read from `channels` (straight values by [`Channel::index`]); the viewer looks straight at the surface
    pub fn shade(&self, albedo: [f32; 3], normal: [f32; 3], [ao, curvature]: [f32; 2], channels: &[[f32; 3]]) -> [f32; 3] {
        let channel = |channel: Channel| channels.get(channel.index()).copied().unwrap_or_else(|| channel.default_value());
        let ao = if ao.is_finite() { ao.clamp(0.0, 1.0) } else { 1.0 };
        let curvature = if curvature.is_finite() { curvature } else { FLAT_CURVATURE };
        let cavity = fade_in((FLAT_CURVATURE - curvature) * CURVATURE_SIDE_SCALE, self.cavity_smoothness);
        let edge = if self.output.diffuse() {
            fade_in((curvature - FLAT_CURVATURE) * CURVATURE_SIDE_SCALE, self.edges_smoothness) * self.edges_intensity
        } else {
            0.0
        };

        let (roughness, f0, diffuse_albedo) = if self.specular_glossiness {
            (1.0 - finite_unit(channel(Channel::Glossiness)[0]), channel(Channel::Specular).map(finite_unit), albedo)
        } else {
            let metal = finite_unit(channel(Channel::Metallic)[0]);
            let f0 = mix3([self.dielectric_reflectance; 3], albedo, metal);

            // Without highlights the whole base color is lit, as Baked Lighting did
            let kept = if self.output.specular() { 1.0 - metal } else { 1.0 };
            (finite_unit(channel(Channel::Roughness)[0]), f0, albedo.map(|x| x * kept))
        };
        let compute_diffuse = self.output.diffuse();
        let compute_specular = self.output.specular();
        let roughness = roughness.max(MIN_ROUGHNESS);
        let sky = self.ambient(normal);

        let diffuse_ao = 1.0 + (ao - 1.0) * self.diffuse_ao;
        let specular_ao = 1.0 + (ao - 1.0) * self.specular_ao;
        let diffuse_cavity = 1.0 - cavity * self.diffuse_cavity;
        let specular_cavity = 1.0 - cavity * self.specular_cavity;

        let mut diffuse = if compute_diffuse { sky.map(|sky_light| sky_light * self.sky_intensity * diffuse_ao * diffuse_cavity) } else { [0.0; 3] };
        let mut specular = [0.0f32; 3];
        for &(direction, intensity, color) in &self.lights {
            let facing = dot(normal, direction);
            if facing <= 0.0 || intensity <= 0.0 {
                continue;
            }

            if compute_diffuse {
                for (diffuse, color) in diffuse.iter_mut().zip(color) {
                    *diffuse += color * intensity * facing * diffuse_cavity;
                }
            }
            if compute_specular {
                let half = normalize([normal[0] + direction[0], normal[1] + direction[1], normal[2] + direction[2]]);
                let normal_dot_half = dot(normal, half).max(0.0);
                let highlight = ggx(normal_dot_half, roughness) / COOK_TORRANCE_DENOMINATOR * facing * specular_cavity;
                for ((specular, color), f0) in specular.iter_mut().zip(color).zip(f0) {
                    *specular += color * intensity * highlight * schlick(f0, normal_dot_half);
                }
            }
        }
        if compute_specular {
            for ((specular, f0), sky_light) in specular.iter_mut().zip(f0).zip(sky) {
                *specular += f0 * sky_light * self.sky_intensity * specular_ao * specular_cavity;
            }
        }

        std::array::from_fn(|channel| {
            let lit = if compute_diffuse { diffuse_albedo[channel] * diffuse[channel] } else { 0.0 };
            let shine = specular[channel];
            let shaded = finite_unit(lit + shine);
            finite_unit(shaded + (1.0 - shaded) * edge)
        })
    }
}

/// `amount` (clamped 0..1) reaching full over `smoothness`: a step when it is about 0 or not a number
fn fade_in(amount: f32, smoothness: f32) -> f32 {
    let amount = amount.clamp(0.0, 1.0);
    if smoothness.is_nan() || smoothness <= MIN_SMOOTHNESS {
        return if amount > 0.0 { 1.0 } else { 0.0 };
    }

    (amount / smoothness).min(1.0)
}

/// The GGX distribution of microfacets facing `normal_dot_half` (the cosine between the normal and the half vector) at `roughness`
fn ggx(normal_dot_half: f32, roughness: f32) -> f32 {
    let alpha_squared = (roughness * roughness).powi(2);
    let denominator = normal_dot_half * normal_dot_half * (alpha_squared - 1.0) + 1.0;
    alpha_squared / (PI * denominator * denominator)
}

/// Schlick's Fresnel of reflectance `f0` at cosine `cos`
fn schlick(f0: f32, cos: f32) -> f32 {
    f0 + (1.0 - f0) * (1.0 - cos.clamp(0.0, 1.0)).powi(5)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `v` at unit length; a vector too short to have a direction points up
fn normalize(v: [f32; 3]) -> [f32; 3] {
    const MIN_LENGTH: f32 = 1e-6;

    let length = dot(v, v).sqrt();
    if length > MIN_LENGTH { v.map(|x| x / length) } else { [0.0, 1.0, 0.0] }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A smoothness that is not a number switches cavity and edges on as a step: a flat surface stays untouched
    #[test]
    fn a_nan_smoothness_fades_in_as_a_step() {
        assert_eq!((fade_in(0.0, f32::NAN), fade_in(0.3, f32::NAN)), (0.0, 1.0));
    }

    /// The relief is the Height to Normal Intensity, unless the filter lights the mesh's own normals
    #[test]
    fn the_relief_follows_mesh_only() {
        let light = SunAndSky { sun: StylizedLighting::PLAIN.lights[0], sky_intensity: 1.0, diffuse_ao: 0.0 };
        assert_eq!(StylizedShading::new(&StylizedLighting::PLAIN, light).relief(), None);
        let detailed = StylizedLighting { mesh_only: false, height_to_normal: 0.7, ..StylizedLighting::PLAIN };
        assert_eq!(StylizedShading::new(&detailed, light).relief(), Some(0.7));
    }
}
