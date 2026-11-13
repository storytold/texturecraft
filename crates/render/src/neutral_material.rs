//! The plain material bake mode can show the low poly with, so the bake reads without the painted colours

/// The highest Quality: reflection samples per pixel
pub const MAX_SAMPLES_PER_PIXEL: u32 = 256;

/// A plain material for the low poly in bake mode: its color comes from the low poly's overlay; its normal and occlusion are the texture set's as shown (the bake under any painted normal, height or AO)
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct NeutralMaterial {
    pub roughness: f32,
    pub metallic: f32,
    /// How much the texture set's ambient occlusion (baked × painted) darkens the material (0 none, 1 fully)
    pub ao_intensity: f32,
    /// Environment reflection samples per pixel, taken by every frame so a moving view looks like a still one
    pub samples_per_pixel: u32,
    /// Light the material along the baked bent normal: the open direction above each point
    pub bent_normal: bool,
    /// How far the diffuse light turns to the bent normal, and how much reflections facing away from it dim (0..1)
    pub bent_normal_diffuse: f32,
    pub bent_normal_specular: f32,
}

impl Default for NeutralMaterial {
    /// Low quality (16 samples per pixel), a slightly rough half metal, most of the baked occlusion, no bent normal
    fn default() -> Self {
        Self {
            roughness: 0.3,
            metallic: 0.6,
            ao_intensity: 0.75,
            samples_per_pixel: 16,
            bent_normal: false,
            bent_normal_diffuse: 1.0,
            bent_normal_specular: 1.0,
        }
    }
}

impl NeutralMaterial {
    /// The material's shader globals: x roughness, y metallic, z AO intensity, w 1 (on)
    pub(crate) fn globals(&self) -> [f32; 4] {
        [self.roughness, self.metallic, self.ao_intensity, 1.0]
    }

    /// Reflection samples each frame takes: the whole quality, from 1 to [`MAX_SAMPLES_PER_PIXEL`] (a damaged settings file can't stall the GPU)
    pub(crate) fn samples_per_frame(&self) -> u32 {
        self.samples_per_pixel.clamp(1, MAX_SAMPLES_PER_PIXEL)
    }

    /// The bent normal's shader globals: x diffuse amount, y specular amount (both 0 while off), zw -
    pub(crate) fn bent_normal_globals(&self) -> [f32; 4] {
        if self.bent_normal { [self.bent_normal_diffuse, self.bent_normal_specular, 0.0, 0.0] } else { [0.0; 4] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_frame_takes_the_whole_quality() {
        let quality = |samples_per_pixel| NeutralMaterial { samples_per_pixel, ..NeutralMaterial::default() };
        assert_eq!(quality(16).samples_per_frame(), 16);
        assert_eq!(quality(256).samples_per_frame(), 256);
        assert_eq!(quality(u32::MAX).samples_per_frame(), MAX_SAMPLES_PER_PIXEL, "a damaged file is held to the highest quality");
        assert_eq!(quality(0).samples_per_frame(), 1);
    }

    #[test]
    fn the_bent_normal_reaches_the_shader_only_when_on() {
        let off = NeutralMaterial::default();
        assert_eq!(off.bent_normal_globals(), [0.0; 4]);
        let on = NeutralMaterial { bent_normal: true, bent_normal_diffuse: 0.5, bent_normal_specular: 0.25, ..off };
        assert_eq!(on.bent_normal_globals(), [0.5, 0.25, 0.0, 0.0]);
    }
}
