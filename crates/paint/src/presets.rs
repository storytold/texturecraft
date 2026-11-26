//! Built-in brush presets: how a brush strokes (shape, spacing, flow, hardness, jitters, path
//! following, particles). Applying one keeps the brush's size, alignment, culling, symmetry and
//! lazy mouse, so a preset changes the stroke, not the setup.

use crate::brush::{BrushAlpha, BrushSettings};

/// A named stroke recipe on top of the default stroke.
pub struct BrushPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub category: &'static str,
    recipe: fn(&mut BrushSettings),
}

impl BrushPreset {
    /// `brush` with this preset's stroke.
    pub fn applied(&self, brush: &BrushSettings) -> BrushSettings {
        let mut b = with_stroke_of(brush, &BrushSettings::default());
        (self.recipe)(&mut b);
        b.sanitized()
    }
}

/// `brush` with the stroke of `from` (shape, flow, opacity, spacing, hardness, angle, path
/// following, jitters, particles and the alpha's parameters): what a preset sets.
pub fn with_stroke_of(brush: &BrushSettings, from: &BrushSettings) -> BrushSettings {
    BrushSettings {
        flow: from.flow,
        opacity: from.opacity,
        spacing: from.spacing,
        hardness: from.hardness,
        angle: from.angle,
        follow_path: from.follow_path,
        jitter_size: from.jitter_size,
        jitter_angle: from.jitter_angle,
        jitter_flow: from.jitter_flow,
        jitter_position: from.jitter_position,
        alpha: from.alpha,
        alpha_image: from.alpha_image.clone(),
        alpha_library: from.alpha_library.clone(),
        alpha_params: from.alpha_params,
        particles: from.particles,
        ..brush.clone()
    }
    .sanitized()
}

/// A preset by id or label (case, spaces, `_` and `-` ignored).
pub fn find_preset(name: &str) -> Option<&'static BrushPreset> {
    let n = |x: &str| x.to_ascii_lowercase().replace([' ', '_', '-'], "");
    PRESETS.iter().find(|p| n(p.id) == n(name) || n(p.label) == n(name))
}

const BASIC: &str = "Basic";
const GRUNGE: &str = "Wear and grunge";
const DETAIL: &str = "Details";

fn library(b: &mut BrushSettings, id: &str) {
    b.alpha_library = Some(id.to_string());
}

/// Every built-in preset, in shelf order.
pub const PRESETS: [BrushPreset; 18] = [
    BrushPreset { id: "basic_hard", label: "Basic hard", category: BASIC, recipe: |b| b.hardness = 0.95 },
    BrushPreset { id: "basic_soft", label: "Basic soft", category: BASIC, recipe: |b| (b.hardness, b.flow) = (0.0, 0.6) },
    BrushPreset { id: "airbrush", label: "Airbrush", category: BASIC, recipe: |b| (b.hardness, b.flow, b.spacing) = (0.0, 0.12, 0.04) },
    BrushPreset { id: "square", label: "Square", category: BASIC, recipe: |b| (b.alpha, b.hardness, b.follow_path) = (BrushAlpha::Square, 0.9, true) },
    BrushPreset { id: "dots", label: "Dots", category: BASIC, recipe: |b| (b.hardness, b.spacing) = (0.9, 1.6) },
    BrushPreset {
        id: "dry_brush",
        label: "Dry brush",
        category: GRUNGE,
        recipe: |b| (b.alpha, b.follow_path, b.spacing, b.flow) = (BrushAlpha::Streaks, true, 0.04, 0.5),
    },
    BrushPreset {
        id: "chalk",
        label: "Chalk",
        category: GRUNGE,
        recipe: |b| {
            library(b, "sponge");
            (b.jitter_angle, b.spacing, b.flow) = (1.0, 0.15, 0.7);
        },
    },
    BrushPreset {
        id: "spray",
        label: "Spray",
        category: GRUNGE,
        recipe: |b| (b.alpha, b.jitter_angle, b.jitter_size, b.spacing) = (BrushAlpha::Spray, 1.0, 0.3, 0.3),
    },
    BrushPreset {
        id: "grunge",
        label: "Grunge",
        category: GRUNGE,
        recipe: |b| (b.alpha, b.jitter_angle, b.jitter_size, b.spacing) = (BrushAlpha::Grunge, 1.0, 0.4, 0.4),
    },
    BrushPreset {
        id: "splatter",
        label: "Splatter",
        category: GRUNGE,
        recipe: |b| {
            library(b, "splatter");
            (b.jitter_angle, b.jitter_size, b.jitter_position, b.spacing) = (1.0, 0.5, 1.5, 1.2);
        },
    },
    BrushPreset {
        id: "scratches",
        label: "Scratches",
        category: GRUNGE,
        recipe: |b| {
            library(b, "scratches");
            (b.follow_path, b.jitter_angle, b.spacing) = (true, 0.1, 0.5);
        },
    },
    BrushPreset {
        id: "chips",
        label: "Chips",
        category: GRUNGE,
        recipe: |b| {
            library(b, "chips");
            (b.jitter_angle, b.jitter_size, b.jitter_position, b.spacing) = (1.0, 0.5, 1.0, 0.8);
        },
    },
    BrushPreset {
        id: "cracks",
        label: "Cracks",
        category: GRUNGE,
        recipe: |b| {
            library(b, "cracks");
            (b.jitter_angle, b.spacing) = (1.0, 0.9);
        },
    },
    BrushPreset {
        id: "drips",
        label: "Drips",
        category: GRUNGE,
        recipe: |b| {
            (b.hardness, b.flow) = (0.7, 0.6);
            b.particles.enabled = true;
        },
    },
    BrushPreset {
        id: "stars",
        label: "Stars",
        category: DETAIL,
        recipe: |b| {
            library(b, "star_5");
            (b.jitter_size, b.jitter_angle, b.jitter_position, b.spacing) = (0.6, 1.0, 2.0, 1.5);
        },
    },
    BrushPreset {
        id: "stitches",
        label: "Stitches",
        category: DETAIL,
        recipe: |b| {
            library(b, "stitches");
            (b.follow_path, b.spacing) = (true, 0.9);
        },
    },
    BrushPreset {
        id: "rivets",
        label: "Rivets",
        category: DETAIL,
        recipe: |b| {
            library(b, "rivet");
            b.spacing = 2.0;
        },
    },
    BrushPreset {
        id: "fur",
        label: "Fur",
        category: DETAIL,
        recipe: |b| {
            library(b, "fur");
            (b.follow_path, b.jitter_angle, b.spacing) = (true, 0.15, 0.3);
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alpha_params::AlphaParams;

    #[test]
    fn a_saved_brush_brings_its_alpha_parameters_and_a_built_in_preset_resets_them() {
        let params = AlphaParams { size_x: 0.5, invert: true, ..AlphaParams::default() };
        let saved = BrushSettings { alpha_params: params, ..BrushSettings::default() };
        assert_eq!(with_stroke_of(&BrushSettings::default(), &saved).alpha_params, params);
        assert_eq!(PRESETS[0].applied(&saved).alpha_params, AlphaParams::default());
    }

    #[test]
    fn presets_are_unique_valid_and_keep_the_setup() {
        let mut ids: Vec<&str> = PRESETS.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), PRESETS.len());
        let mine =
            BrushSettings { size: 0.1, lazy: 12.0, backface_culling: false, jitter_size: 1.0, alpha_image: Some("x.png".into()), ..BrushSettings::default() };
        for p in &PRESETS {
            let b = p.applied(&mine);
            assert_eq!(b, b.sanitized(), "{}", p.id);
            assert_eq!((b.size, b.lazy, b.backface_culling, b.alpha_image.as_deref()), (0.1, 12.0, false, None), "{}", p.id);
            if let Some(id) = &b.alpha_library {
                assert!(crate::alphas::find(id).is_some(), "{}: library alpha {id}", p.id);
            }
            assert_eq!(find_preset(p.label).map(|f| f.id), Some(p.id));
        }
        assert_eq!(find_preset("basic hard").map(|p| p.id), Some("basic_hard"));
        assert!(find_preset("nope").is_none());
        assert_eq!(PRESETS[0].applied(&mine).jitter_size, 0.0, "jitters reset");
    }
}
