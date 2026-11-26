//! The per-generator terms of the wear, section and light controls, for the GPU compositor to evaluate them as compose does

use texturecraft_core::{GeneratorKind, SectionParameters, WearParameters};
use texturecraft_geom::glam::Vec3;

use crate::curvature_sizes::LEVEL_RADII;
use crate::{sections, wear};

/// The blurred curvature levels a wear generator of `kind` reads between: the narrower (None: the map itself), the wider and the wider one's weight
pub fn wear_levels_around(kind: GeneratorKind, wear: &WearParameters) -> (Option<usize>, usize, f32) {
    wear::levels_around(wear::blur_sigma(kind, wear))
}

/// Which blurred curvature levels a wear generator of `kind` reads
pub fn wear_levels_in_use(kind: GeneratorKind, wear: &WearParameters) -> [bool; LEVEL_RADII.len()] {
    wear::levels_in_use(kind, wear)
}

/// The box radius, in texels at `resolution` texels per side, of the section blur of a generator of `kind`
pub fn section_blur_radius(kind: GeneratorKind, section: &SectionParameters, resolution: u32) -> usize {
    sections::blur_radius_at(kind, section, resolution)
}

/// The power the section balance raises the signal to (the kinds whose balance is a power curve)
pub fn section_balance_power(balance: f32) -> f32 {
    sections::balance_power(balance)
}

/// The direction the Light generator's light comes from
pub fn light_direction(horizontal: f32, vertical: f32) -> Vec3 {
    sections::light_direction(horizontal, vertical)
}
