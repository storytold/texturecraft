use texturecraft_core::{GeneratorKind, ProceduralKind};

/// The pattern that breaks a wear kind up, its repeats across the UV square (or the mesh, triplanar) per unit of grunge scale, its rotation in degrees, and its mean and spread over the UV square (measured by `grunge_is_normalised`)
pub(crate) struct GrungePattern {
    pub(crate) kind: ProceduralKind,
    pub(crate) repeats_per_scale: f32,
    pub(crate) rotation: f32,
    /// A seeded pattern and its repeats per unit of grunge scale that varies the pattern's strength, for patterns the seed doesn't change
    pub(crate) breakup: Option<(ProceduralKind, f32)>,
    pub(crate) mean: f32,
    pub(crate) spread: f32,
    /// How strongly a custom grunge's gray away from mid gray counts, in units of the normalised grunge (Dirt reads a brighter custom grunge as cleaner)
    pub(crate) custom_gain: f32,
}

/// Fine scratches on metal edges, a woven fibre on fibreglass, blotchy grime for dirt
pub(crate) const EDGE_WEAR_GRUNGE: GrungePattern =
    GrungePattern { kind: ProceduralKind::Scratches, repeats_per_scale: 4.0, rotation: 0.0, breakup: None, mean: 0.037, spread: 0.149, custom_gain: 6.22 };
pub(crate) const FIBER_GLASS_GRUNGE: GrungePattern = GrungePattern {
    kind: ProceduralKind::Weave,
    repeats_per_scale: 16.0,
    rotation: 45.0,
    breakup: Some((ProceduralKind::Grunge, 4.0)),
    mean: 0.539,
    spread: 0.198,
    custom_gain: 1.9,
};
pub(crate) const DIRT_GRUNGE: GrungePattern =
    GrungePattern { kind: ProceduralKind::Grunge, repeats_per_scale: 4.0, rotation: 0.0, breakup: None, mean: 0.682, spread: 0.242, custom_gain: -5.56 };

pub(crate) fn grunge_pattern(kind: GeneratorKind) -> &'static GrungePattern {
    match kind {
        GeneratorKind::Dirt => &DIRT_GRUNGE,
        GeneratorKind::FiberGlassEdgeWear => &FIBER_GLASS_GRUNGE,
        _ => &EDGE_WEAR_GRUNGE,
    }
}
