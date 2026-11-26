use texturecraft_core::ProceduralKind;

/// A wear kind's grunge pattern, its repeats across the UV square per unit of grunge scale, its
/// rotation, its breakup (pattern and repeats), its mean and spread, and a custom grunge's gain
/// (compose's `grunge_pattern.rs`)
pub(super) struct GrungePattern {
    pub(super) kind: ProceduralKind,
    pub(super) repeats_per_scale: f32,
    pub(super) rotation: f32,
    pub(super) breakup: Option<(ProceduralKind, f32)>,
    pub(super) mean: f32,
    pub(super) spread: f32,
    pub(super) custom_gain: f32,
}

pub(super) const EDGE_WEAR_GRUNGE: GrungePattern =
    GrungePattern { kind: ProceduralKind::Scratches, repeats_per_scale: 4.0, rotation: 0.0, breakup: None, mean: 0.037, spread: 0.149, custom_gain: 6.22 };
pub(super) const FIBER_GLASS_GRUNGE: GrungePattern = GrungePattern {
    kind: ProceduralKind::Weave,
    repeats_per_scale: 16.0,
    rotation: 45.0,
    breakup: Some((ProceduralKind::Grunge, 4.0)),
    mean: 0.539,
    spread: 0.198,
    custom_gain: 1.9,
};
pub(super) const DIRT_GRUNGE: GrungePattern =
    GrungePattern { kind: ProceduralKind::Grunge, repeats_per_scale: 4.0, rotation: 0.0, breakup: None, mean: 0.682, spread: 0.242, custom_gain: -5.56 };

/// The triplanar seams' sharpness at triplanar contrast 0 and 1 (compose's wear.rs)
pub(super) const TRIPLANAR_SHARPNESS: std::ops::RangeInclusive<f32> = 1.0..=16.0;
