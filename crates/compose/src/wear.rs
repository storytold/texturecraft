//! The wear generators' value at a texel (Metal Edge Wear, Fiber Glass Edge Wear, Dirt), fitted to the reference app's observed results

use texturecraft_core::{GeneratorKind, Procedural, Projection, WearParameters};

use crate::curvature_sizes::LEVEL_RADII;
use crate::grunge_pattern::grunge_pattern;
use crate::pattern::{Coords, procedural};
use crate::slot_placement::SlotPlacement;
use crate::wear_inputs::WearInputs;
use crate::wear_scan::{DIRT_SCAN, EDGE_WEAR_SCAN, FIBER_GLASS_SCAN};

/// The Gaussian sigma each blurred curvature level spreads like: three box blurs of radius r spread √(r(r+1)), radii from [`LEVEL_RADII`]
const LEVEL_SIGMAS: [f32; LEVEL_RADII.len()] = [1.414, 2.449, 4.472, 8.485, 16.492, 32.496, 64.498, 128.499];

/// Gaussian sigma per unit of edges smoothness, in texels at 2K
const EDGE_WEAR_SMOOTHNESS_SIGMA: f32 = 4.6;
const FIBER_GLASS_SMOOTHNESS_SIGMA: f32 = 4.5;
/// The share of the blurred curvature in the smoothed edges
const SMOOTHED_SHARE: f32 = 0.5;
/// Dirt reads its edges and cavities slightly softened, sigma in texels at 2K
const DIRT_CURVATURE_SIGMA: f32 = 1.49;

/// How much a breakup pattern varies the grunge's strength (0 none, 1 down to nothing)
const BREAKUP_DEPTH: f32 = 0.6;

/// The triplanar seams' sharpness at triplanar contrast 0 and 1
const TRIPLANAR_SHARPNESS: std::ops::RangeInclusive<f32> = 1.0..=16.0;

/// The gray at which a custom grunge neither adds nor removes wear
const CUSTOM_GRUNGE_MID_GRAY: f32 = 0.5;

/// Where the reference app reads a custom grunge at the UV square's origin: half way across the image (triplanar: at its origin)
const CUSTOM_GRUNGE_UV_OFFSET: f32 = 0.5;

/// The Gaussian sigma (texels at 2K) the kind's blurred curvature is read at
pub(crate) fn blur_sigma(kind: GeneratorKind, wear: &WearParameters) -> f32 {
    match kind {
        GeneratorKind::Dirt => DIRT_CURVATURE_SIGMA,
        GeneratorKind::FiberGlassEdgeWear => FIBER_GLASS_SMOOTHNESS_SIGMA * wear.edges_smoothness,
        _ => EDGE_WEAR_SMOOTHNESS_SIGMA * wear.edges_smoothness,
    }
}

fn triplanar_sharpness(wear: &WearParameters) -> f32 {
    TRIPLANAR_SHARPNESS.start() + (TRIPLANAR_SHARPNESS.end() - TRIPLANAR_SHARPNESS.start()) * wear.triplanar_contrast
}

/// The wear's own grunge at `coords`, normalised to mean 0 and spread 1
pub(crate) fn grunge(kind: GeneratorKind, wear: &WearParameters, seed: u32, coords: &Coords) -> f32 {
    let grunge = grunge_pattern(kind);
    let pattern = Procedural { scale: grunge.repeats_per_scale * wear.grunge_scale, seed, ..Procedural::new(grunge.kind) };
    let projection = if wear.triplanar {
        Projection::Triplanar { scale: 1.0, sharpness: triplanar_sharpness(wear), offset: [0.0; 3], uv: Default::default(), projector: Default::default() }
    } else {
        Projection::Uv { scale: [1.0, 1.0], rotation: grunge.rotation, offset: [0.0, 0.0] }
    };
    let mut value = procedural(&pattern, &projection, coords);
    if let Some((breakup, repeats_per_scale)) = grunge.breakup {
        let breakup = Procedural { scale: repeats_per_scale * wear.grunge_scale, seed, ..Procedural::new(breakup) };
        value *= 1.0 - BREAKUP_DEPTH + BREAKUP_DEPTH * procedural(&breakup, &projection, coords);
    }
    (value - grunge.mean) / grunge.spread
}

/// Where a custom grunge image is read: repeated grunge-scale times across the UV square (or the mesh's bounds, triplanar)
pub(crate) fn custom_grunge_placement(wear: &WearParameters) -> SlotPlacement {
    if wear.triplanar {
        SlotPlacement { scale: wear.grunge_scale, offset: 0.0, triplanar_contrast: Some(wear.triplanar_contrast), mirrored_about_centre: false }
    } else {
        SlotPlacement { scale: wear.grunge_scale, offset: CUSTOM_GRUNGE_UV_OFFSET, triplanar_contrast: None, mirrored_about_centre: false }
    }
}

/// A custom grunge's `gray` as the normalised grunge the fitted signals read; the reference app's seed leaves it where it is
pub(crate) fn custom_grunge(kind: GeneratorKind, gray: f32) -> f32 {
    grunge_pattern(kind).custom_gain * (gray - CUSTOM_GRUNGE_MID_GRAY)
}

/// The two blurred curvature levels around `sigma` and the weight of the wider one; level None is the map itself
pub(crate) fn levels_around(sigma: f32) -> (Option<usize>, usize, f32) {
    let sigmas = LEVEL_SIGMAS;
    let wider = sigmas.iter().position(|level| *level >= sigma).unwrap_or(sigmas.len() - 1);
    let (narrower, narrower_sigma) = match wider.checked_sub(1) {
        Some(level) => (Some(level), sigmas[level]),
        None => (None, 0.0),
    };
    let span = sigmas[wider] - narrower_sigma;
    let weight = if span > 0.0 { ((sigma - narrower_sigma) / span).clamp(0.0, 1.0) } else { 1.0 };
    (narrower, wider, weight)
}

/// Which blurred curvature levels a wear generator reads
pub(crate) fn levels_in_use(kind: GeneratorKind, wear: &WearParameters) -> [bool; LEVEL_RADII.len()] {
    let sigma = blur_sigma(kind, wear);
    if sigma <= 0.0 {
        return [false; LEVEL_RADII.len()];
    }

    let (narrower, wider, _) = levels_around(sigma);
    std::array::from_fn(|level| level == wider || Some(level) == narrower)
}

/// The wear value (0..1) of `kind` at a texel
pub(crate) fn wear_value(kind: GeneratorKind, wear: &WearParameters, inputs: &WearInputs) -> f32 {
    match kind {
        GeneratorKind::Dirt => DIRT_SCAN.apply(dirt_signal(wear, inputs), wear),
        GeneratorKind::FiberGlassEdgeWear => FIBER_GLASS_SCAN.apply(fiber_glass_signal(wear, inputs), wear),
        _ => EDGE_WEAR_SCAN.apply(edge_wear_signal(wear, inputs), wear),
    }
}

/// Edges sharp and smoothed in equal parts
fn smoothed_edges(inputs: &WearInputs) -> f32 {
    (1.0 - SMOOTHED_SHARE) * inputs.curvature + SMOOTHED_SHARE * inputs.blurred_curvature
}

/// The masking at which the fitted cut-off is pinned, the controls' default
const DEFAULT_MASKING: f32 = 0.5;

/// 1 where the area is open enough to wear, 0 where occluded: the cut-off sits at `offset + gain × masking`, softened over `width`
fn open_area(occlusion: f32, masking: f32, (offset, gain, width): (f32, f32, f32)) -> f32 {
    // A cut-off below this leaves every occlusion fully open, so the low end eases from it to the default's cut-off instead of masking nothing
    let fully_open = -width * 0.5;
    let cut_off = if masking < DEFAULT_MASKING && offset < fully_open {
        fully_open + (offset + gain * DEFAULT_MASKING - fully_open) * masking / DEFAULT_MASKING
    } else {
        offset + gain * masking
    };
    ((occlusion - cut_off) / width + 0.5).clamp(0.0, 1.0)
}

// The signals' coefficients are fitted to the reference app's sweeps on a hard-surface bake
fn edge_wear_signal(wear: &WearParameters, inputs: &WearInputs) -> f32 {
    let edges = smoothed_edges(inputs) * wear.curvature_weight;
    let open = open_area(inputs.occlusion, wear.ao_masking, (-0.779, 2.244, 0.935));
    let grunge = wear.grunge_amount;
    edges * open * (1.0 + 0.031 * grunge) - 0.009 * grunge + 0.057 * grunge.powf(0.731) * inputs.grunge
}

fn fiber_glass_signal(wear: &WearParameters, inputs: &WearInputs) -> f32 {
    // The grunge takes over from the edges as it grows
    let edges = smoothed_edges(inputs) * wear.curvature_weight * 1.02;
    let open = open_area(inputs.occlusion, wear.ao_masking, (0.144, 0.726, 0.669));
    let grunge = wear.grunge_amount;
    (edges * (1.0 - 0.989 * grunge) + 0.167 * grunge + 0.094 * grunge.powf(1.64) * inputs.grunge) * open
}

fn dirt_signal(wear: &WearParameters, inputs: &WearInputs) -> f32 {
    let edges = inputs.blurred_curvature;
    let grunge = wear.grunge_amount;
    0.486 * (0.869 - inputs.occlusion) - wear.edges_masking * 0.817 * edges.max(0.0)
        + 0.203 * (-edges).max(0.0)
        + 0.241 * grunge
        + 0.0895 * grunge * inputs.grunge
}

#[cfg(test)]
mod tests {
    use texturecraft_geom::glam::{Vec2, Vec3};

    use super::*;
    use crate::grunge_pattern::{DIRT_GRUNGE, EDGE_WEAR_GRUNGE, FIBER_GLASS_GRUNGE};

    fn inputs(curvature: f32, occlusion: f32) -> WearInputs {
        WearInputs { curvature, blurred_curvature: curvature, occlusion, grunge: 0.0 }
    }

    fn defaults(kind: GeneratorKind) -> WearParameters {
        WearParameters { grunge_amount: 0.0, ..WearParameters::defaults(kind).unwrap() }
    }

    #[test]
    fn edge_wear_shows_on_open_convex_edges_only() {
        for kind in [GeneratorKind::EdgeWear, GeneratorKind::FiberGlassEdgeWear] {
            let wear = WearParameters { curvature_weight: 1.0, ..defaults(kind) };
            let value = |curvature, occlusion| wear_value(kind, &wear, &inputs(curvature, occlusion));
            assert!(value(0.4, 1.0) > 0.9, "{kind:?}: a sharp open edge wears");
            assert!(value(0.0, 1.0) < 0.1, "{kind:?}: a flat face does not");
            assert!(value(-0.4, 1.0) < 0.1, "{kind:?}: nor a cavity");
            assert!(value(0.4, 0.0) < value(0.4, 1.0), "{kind:?}: occlusion masks the wear");
        }
    }

    #[test]
    fn dirt_gathers_in_occluded_cavities_and_spares_edges() {
        let wear = defaults(GeneratorKind::Dirt);
        let value = |curvature, occlusion| wear_value(GeneratorKind::Dirt, &wear, &inputs(curvature, occlusion));
        assert!(value(-0.3, 0.2) > value(0.0, 1.0), "occluded cavities collect dirt");
        assert!(value(0.3, 0.6) < value(0.0, 0.6), "edges are masked");
    }

    #[test]
    fn every_ao_masking_above_0_masks_more_occluded_edges() {
        let wear = |ao_masking| WearParameters { ao_masking, ..defaults(GeneratorKind::EdgeWear) };
        let occluded_edge = inputs(0.1, 0.0);
        let values: Vec<f32> = [0.0, 0.05, 0.1, 0.3].iter().map(|masking| wear_value(GeneratorKind::EdgeWear, &wear(*masking), &occluded_edge)).collect();
        assert!(values.windows(2).all(|pair| pair[1] < pair[0]), "{values:?}");
        assert_eq!(wear_value(GeneratorKind::EdgeWear, &wear(1.0), &occluded_edge), 0.0, "full masking clears occluded areas");
    }

    #[test]
    fn a_higher_level_spreads_the_wear_and_a_full_level_is_a_hard_step() {
        let low = WearParameters { level: 0.25, ..defaults(GeneratorKind::EdgeWear) };
        let high = WearParameters { level: 0.9, ..low };
        let mild = inputs(0.1, 1.0);
        assert!(wear_value(GeneratorKind::EdgeWear, &high, &mild) > wear_value(GeneratorKind::EdgeWear, &low, &mild));

        let full = WearParameters { level: 1.0, ..low };
        for curvature in [-0.5, 0.0, 0.05, 0.5] {
            let value = wear_value(GeneratorKind::EdgeWear, &full, &inputs(curvature, 1.0));
            assert!(value == 0.0 || value == 1.0, "{curvature}: {value}");
        }
    }

    #[test]
    fn grunge_is_normalised() {
        for (kind, pattern) in
            [(GeneratorKind::EdgeWear, &EDGE_WEAR_GRUNGE), (GeneratorKind::FiberGlassEdgeWear, &FIBER_GLASS_GRUNGE), (GeneratorKind::Dirt, &DIRT_GRUNGE)]
        {
            let wear = WearParameters::defaults(kind).unwrap();
            let samples: Vec<f32> = (0..256 * 256)
                .map(|texel| {
                    let uv = Vec2::new((texel % 256) as f32 / 256.0, (texel / 256) as f32 / 256.0);
                    grunge(kind, &wear, 7, &Coords { uv, pos: uv.extend(0.0), nrm: Vec3::Z })
                })
                .collect();
            let mean = samples.iter().sum::<f32>() / samples.len() as f32;
            let spread = (samples.iter().map(|value| (value - mean).powi(2)).sum::<f32>() / samples.len() as f32).sqrt();
            let measured = (pattern.mean + mean * pattern.spread, spread * pattern.spread);
            assert!(mean.abs() < 0.1 && (spread - 1.0).abs() < 0.1, "{kind:?}: normalised mean {mean}, spread {spread}; the pattern's own {measured:?}");
        }
    }

    #[test]
    fn every_kind_s_grunge_changes_with_the_seed() {
        for kind in [GeneratorKind::EdgeWear, GeneratorKind::FiberGlassEdgeWear, GeneratorKind::Dirt] {
            let wear = WearParameters::defaults(kind).unwrap();
            let sample = |seed| -> Vec<f32> {
                (0..64 * 64)
                    .map(|texel| {
                        let uv = Vec2::new((texel % 64) as f32 / 64.0, (texel / 64) as f32 / 64.0);
                        grunge(kind, &wear, seed, &Coords { uv, pos: uv.extend(0.0), nrm: Vec3::Z })
                    })
                    .collect()
            };
            let (first, second) = (sample(1), sample(2));
            let difference = first.iter().zip(&second).map(|(a, b)| (a - b).abs()).sum::<f32>() / first.len() as f32;
            assert!(difference > 0.2, "{kind:?}: another seed barely changes the grunge ({difference})");
        }
    }

    #[test]
    fn the_blurred_levels_around_a_sigma_bracket_it() {
        assert_eq!(levels_around(0.5).0, None, "below the first level the map itself is the narrower one");
        let (narrower, wider, weight) = levels_around(4.6);
        assert_eq!((narrower, wider), (Some(2), 3));
        assert!((0.0..=0.1).contains(&weight), "4.6 sits just past radius 4's sigma: {weight}");
        assert_eq!(levels_around(500.0).1, LEVEL_RADII.len() - 1, "wider than every level reads the widest");
        for (radius, sigma) in LEVEL_RADII.iter().zip(LEVEL_SIGMAS) {
            assert!(((radius * (radius + 1.0)).sqrt() - sigma).abs() < 0.001, "{radius}: {sigma}");
        }
        let widest = super::blur_sigma(
            GeneratorKind::EdgeWear,
            &WearParameters { edges_smoothness: *WearParameters::EDGES_SMOOTHNESS_RANGE.end(), ..WearParameters::defaults(GeneratorKind::EdgeWear).unwrap() },
        );
        assert!(widest < LEVEL_SIGMAS[LEVEL_RADII.len() - 1], "the whole smoothness range lands between built levels: {widest}");
    }
}
