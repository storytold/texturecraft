//! What Baked Lighting Stylized adds to Baked Lighting's sun and ambient: sky colors, two more lights, cavity, edges and highlights

use serde::{Deserialize, Serialize};

use crate::directional_light::DirectionalLight;
use crate::lighting_output::LightingOutput;
use crate::stylized_shading::StylizedShading;
use crate::sun_and_sky::SunAndSky;

/// Baked Lighting Stylized's settings past the sun's direction and strength, the sky's strength and Diffuse AO (kept on the filter, as Baked Lighting saved them)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StylizedLighting {
    pub sun_color: [f32; 3],
    /// The ambient light from straight above, at the horizon and from below; normals between blend them
    pub sky_color: [f32; 3],
    pub horizon_color: [f32; 3],
    pub ground_color: [f32; 3],
    /// Light 1 and Light 2
    pub lights: [DirectionalLight; 2],
    /// How much the cavities (concave curvature) darken the lit surface and its highlights
    pub diffuse_cavity: f32,
    pub specular_cavity: f32,
    /// How much the baked ambient occlusion darkens the highlights
    pub specular_ao: f32,
    /// The concavity (and convexity, for edges) over which cavity and edges fade in: 0 is a hard step
    pub cavity_smoothness: f32,
    /// How much the edges (convex curvature) lighten toward white
    pub edges_intensity: f32,
    pub edges_smoothness: f32,
    /// Reads the Specular and Glossiness channels instead of Metallic and Roughness (Input)
    pub specular_glossiness: bool,
    pub output: LightingOutput,
    /// The reflectance of non-metals at normal incidence
    pub dielectric_reflectance: f32,
    /// Lights the mesh's own normals, without the baked normal and height detail (Normal Details Type Mesh only)
    pub mesh_only: bool,
    pub height_to_normal: f32,
}

/// The reflectance of non-metals at normal incidence
pub const DIELECTRIC_REFLECTANCE: f32 = 0.04;

impl StylizedLighting {
    /// The settings that light exactly as Baked Lighting did: white light and sky, no extra lights, cavity, edges or highlights, the mesh's own normals
    pub const PLAIN: StylizedLighting = StylizedLighting {
        sun_color: [1.0; 3],
        sky_color: [1.0; 3],
        horizon_color: [1.0; 3],
        ground_color: [1.0; 3],
        lights: [DirectionalLight::off(90.0), DirectionalLight::off(180.0)],
        diffuse_cavity: 0.0,
        specular_cavity: 0.0,
        specular_ao: 0.0,
        cavity_smoothness: 0.25,
        edges_intensity: 0.0,
        edges_smoothness: 0.0,
        specular_glossiness: false,
        output: LightingOutput::Diffuse,
        dielectric_reflectance: DIELECTRIC_REFLECTANCE,
        mesh_only: true,
        height_to_normal: 1.0,
    };

    /// A new Baked Lighting Stylized: a warm sun, a blue sky over a brown ground, highlights, and the baked normal and height detail
    pub fn library() -> StylizedLighting {
        StylizedLighting {
            sun_color: [1.0, 0.905, 0.716],
            sky_color: [0.305, 0.456, 0.791],
            horizon_color: [0.644, 0.644, 0.61],
            ground_color: [0.102, 0.08, 0.061],
            output: LightingOutput::DiffuseSpecular,
            mesh_only: false,
            ..StylizedLighting::PLAIN
        }
    }

    /// These settings lit by `light`, worked out once for the shading of every texel
    pub fn shading(&self, light: SunAndSky) -> StylizedShading {
        StylizedShading::new(self, light)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Channel, Filter};

    const UP: [f32; 3] = [0.0, 1.0, 0.0];

    fn material(roughness: f32) -> Vec<[f32; 3]> {
        let mut channels: Vec<[f32; 3]> = Channel::ALL.iter().map(|c| c.default_value()).collect();
        channels[Channel::Roughness.index()] = [roughness; 3];
        channels
    }

    fn light(elevation: f32, intensity: f32, sky_intensity: f32) -> SunAndSky {
        SunAndSky { sun: DirectionalLight { azimuth: 0.0, elevation, intensity, color: [1.0; 3] }, sky_intensity, diffuse_ao: 0.5 }
    }

    /// The plain settings light as Baked Lighting did: the base color times sun facing plus occluded ambient
    #[test]
    fn plain_settings_light_as_baked_lighting_did() {
        let legacy = Filter::BakedLighting { azimuth: 0.0, elevation: 30.0, light: 0.6, ambient: 0.4, occlusion: 0.5, stylized: None };
        for (nrm, ao) in [(UP, 1.0), ([0.6, 0.8, 0.0], 0.3), ([0.0, -1.0, 0.0], 0.7)] {
            let k = legacy.baked_light(nrm, ao).unwrap_or(f32::NAN);
            let shaded = StylizedLighting::PLAIN.shading(light(30.0, 0.6, 0.4)).shade([0.5, 0.25, 1.0], nrm, [ao, 0.5], &material(0.5));
            let expected = [0.5f32, 0.25, 1.0].map(|x| (x * k).clamp(0.0, 1.0));
            assert!(shaded.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-5), "{nrm:?}: {shaded:?} vs {expected:?}");
        }
    }

    /// The sky, horizon and ground colors light a surface by where it faces; Light 1 adds its color
    #[test]
    fn sky_colors_and_extra_lights() {
        let colors = StylizedLighting { sky_color: [0.0, 0.0, 1.0], horizon_color: [0.0, 1.0, 0.0], ground_color: [1.0, 0.0, 0.0], ..StylizedLighting::PLAIN };
        let sky = light(45.0, 0.0, 1.0);
        let white = [1.0; 3];
        assert_eq!(colors.shading(sky).shade(white, UP, [1.0, 0.5], &material(0.5)), [0.0, 0.0, 1.0]);
        assert_eq!(colors.shading(sky).shade(white, [1.0, 0.0, 0.0], [1.0, 0.5], &material(0.5)), [0.0, 1.0, 0.0]);
        assert_eq!(colors.shading(sky).shade(white, [0.0, -1.0, 0.0], [1.0, 0.5], &material(0.5)), [1.0, 0.0, 0.0]);

        let mut extra = StylizedLighting::PLAIN;
        extra.lights[0] = DirectionalLight { azimuth: 0.0, elevation: 90.0, intensity: 0.5, color: [1.0, 0.0, 0.0] };
        let dark = light(45.0, 0.0, 0.0);
        let lit = extra.shading(dark).shade(white, UP, [1.0, 0.5], &material(0.5));
        assert!((lit[0] - 0.5).abs() < 1e-5 && lit[1] == 0.0, "{lit:?}");
    }

    /// Highlights: a smooth surface shines brighter than a rough one; Specular only keeps them alone, Diffuse only drops them
    #[test]
    fn highlights_follow_roughness_and_the_output() {
        let sun = light(90.0, 1.0, 0.0);
        let shiny = StylizedLighting { output: LightingOutput::Specular, ..StylizedLighting::library() };
        let albedo = [0.0; 3];
        let smooth = shiny.shading(sun).shade(albedo, UP, [1.0, 0.5], &material(0.2));
        let rough = shiny.shading(sun).shade(albedo, UP, [1.0, 0.5], &material(0.9));
        assert!(smooth[0] > rough[0] + 0.1 && rough[0] > 0.0, "{smooth:?} {rough:?}");

        let diffuse = StylizedLighting { output: LightingOutput::Diffuse, ..StylizedLighting::library() };
        assert_eq!(diffuse.shading(sun).shade(albedo, UP, [1.0, 0.5], &material(0.2)), [0.0; 3]);
    }

    /// Cavities darken the lit surface by Diffuse Cavity; edges lighten toward white by Edges Intensity
    #[test]
    fn cavities_darken_and_edges_lighten() {
        let sky = light(45.0, 0.0, 1.0);
        let gray = [0.5; 3];
        let cavities = StylizedLighting { diffuse_cavity: 1.0, ..StylizedLighting::PLAIN };
        assert_eq!(cavities.shading(sky).shade(gray, UP, [1.0, 0.5], &material(0.5)), gray, "flat: no cavity");
        assert_eq!(cavities.shading(sky).shade(gray, UP, [1.0, 0.2], &material(0.5)), [0.0; 3], "a deep cavity");

        let sun = light(90.0, 1.0, 0.0);
        assert_eq!(cavities.shading(sun).shade(gray, UP, [1.0, 0.5], &material(0.5)), gray, "flat in the sun: no cavity");
        assert_eq!(cavities.shading(sun).shade(gray, UP, [1.0, 0.2], &material(0.5)), [0.0; 3], "a deep cavity in the sun");

        let edges = StylizedLighting { edges_intensity: 1.0, ..StylizedLighting::PLAIN };
        assert_eq!(edges.shading(sky).shade(gray, UP, [1.0, 0.9], &material(0.5)), [1.0; 3], "a sharp edge");
    }
}
