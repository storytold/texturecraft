//! The sun and sky Baked Lighting Stylized shades with

use crate::directional_light::DirectionalLight;

/// The sun and sky a [`StylizedLighting`](crate::StylizedLighting) shades with, kept on the filter as Baked Lighting saved them
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunAndSky {
    pub sun: DirectionalLight,
    pub sky_intensity: f32,
    pub diffuse_ao: f32,
}
