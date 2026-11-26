//! A mask effect that selects the parts of an ID map whose colors are in its list

use serde::{Deserialize, Serialize};

use crate::SelectionBackground;

/// Selects ID-map colors within a tolerance: the selected parts get the output value, the rest the background
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorSelection {
    /// The selected ID colors, as the ID map stores them
    pub colors: Vec<[f32; 3]>,
    /// How far each component may be from a selected color and still count as it
    pub tolerance: f32,
    /// The value selected parts get
    #[serde(default = "full", skip_serializing_if = "is_full")]
    pub output: f32,
    /// 1 is a hard edge (what selections saved before hardness did); lower ramps over 1 − hardness past the tolerance
    #[serde(default = "full", skip_serializing_if = "is_full")]
    pub hardness: f32,
    #[serde(default, skip_serializing_if = "SelectionBackground::is_default")]
    pub background: SelectionBackground,
    /// The image resource read as the ID map; none reads the texture set's ID mesh map
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_map: Option<String>,
}

fn full() -> f32 {
    1.0
}

/// Whether an output or hardness is what selections saved before it existed had (not saved then, so they save as they were)
fn is_full(value: &f32) -> bool {
    *value == full()
}

impl ColorSelection {
    /// A selection of `colors` with the defaults of a new one
    pub fn new(colors: Vec<[f32; 3]>, tolerance: f32) -> ColorSelection {
        ColorSelection { colors, tolerance, output: 1.0, hardness: 1.0, background: SelectionBackground::Black, id_map: None }
    }

    /// How much ID color `id` is selected, 0..1 (none: no ID map there)
    pub fn amount(&self, id: Option<[f32; 3]>) -> f32 {
        let Some(id) = id else { return 0.0 };
        if self.tolerance.is_nan() || self.hardness.is_nan() {
            return 0.0;
        }

        let distance = self
            .colors
            .iter()
            // Carry NaN through the component maximum so a malformed color cannot match
            .map(|picked| {
                picked
                    .iter()
                    .zip(id)
                    .map(|(picked, component)| (picked - component).abs())
                    .fold(0.0f32, |far, gap| if gap.is_nan() || gap > far { gap } else { far })
            })
            .fold(f32::INFINITY, f32::min);
        if !distance.is_finite() {
            return 0.0;
        }

        let past = distance - self.tolerance;
        if past <= 0.0 {
            return 1.0;
        }

        let width = 1.0 - self.hardness.clamp(0.0, 1.0);
        if width <= 0.0 { 0.0 } else { (1.0 - past / width).clamp(0.0, 1.0) }
    }

    /// The gray this selection gives ID color `id` and how much of it covers the mask below (a transparent background covers only the selection)
    pub fn value(&self, id: Option<[f32; 3]>) -> (f32, f32) {
        let amount = self.amount(id);
        match self.background.value() {
            Some(background) => (background + (self.output - background) * amount, 1.0),
            None => (self.output, amount),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EffectKind;

    #[test]
    fn a_selection_saved_before_loads_hard_on_black_from_the_mesh_map() {
        let saved = r#"{"ColorSelection":{"colors":[[1.0,0.0,0.0]],"tolerance":0.1}}"#;
        let EffectKind::ColorSelection(selection) = serde_json::from_str::<EffectKind>(saved).unwrap() else { panic!("not a color selection") };
        assert_eq!(selection, ColorSelection::new(vec![[1.0, 0.0, 0.0]], 0.1));
        assert_eq!(serde_json::to_string(&EffectKind::ColorSelection(selection)).unwrap(), saved, "it saves as it was saved");
    }

    #[test]
    fn the_hard_edge_selects_as_before_and_softness_ramps_past_the_tolerance() {
        let mut selection = ColorSelection::new(vec![[1.0, 0.0, 0.0]], 0.1);
        assert_eq!(selection.value(Some([0.95, 0.05, 0.0])), (1.0, 1.0));
        assert_eq!(selection.value(Some([0.8, 0.0, 0.0])), (0.0, 1.0));
        assert_eq!(selection.value(None), (0.0, 1.0));
        assert_eq!(selection.value(Some([f32::NAN, 0.0, 0.0])), (0.0, 1.0));
        selection.hardness = 0.8;
        let (ramped, _) = selection.value(Some([0.8, 0.0, 0.0]));
        assert!((ramped - 0.5).abs() < 1e-5, "{ramped}");
        selection.colors.clear();
        assert_eq!(selection.value(Some([1.0, 0.0, 0.0])), (0.0, 1.0));
    }

    #[test]
    fn a_nan_tolerance_or_hardness_selects_nothing() {
        let mut nan_tolerance = ColorSelection::new(vec![[1.0, 0.0, 0.0]], f32::NAN);
        nan_tolerance.hardness = 0.5;
        let mut nan_hardness = ColorSelection::new(vec![[1.0, 0.0, 0.0]], 0.1);
        nan_hardness.hardness = f32::NAN;

        for selection in [nan_tolerance, nan_hardness] {
            assert_eq!(selection.amount(Some([1.0, 0.0, 0.0])), 0.0, "an exact match still needs valid settings");
            assert_eq!(selection.amount(Some([0.8, 0.0, 0.0])), 0.0);
        }
    }

    #[test]
    fn the_output_and_background_set_both_sides_and_transparent_covers_only_the_selection() {
        let mut selection = ColorSelection::new(vec![[0.0, 0.0, 1.0]], 0.1);
        selection.output = 0.25;
        selection.background = SelectionBackground::White;
        assert_eq!(selection.value(Some([0.0, 0.0, 1.0])), (0.25, 1.0));
        assert_eq!(selection.value(Some([1.0, 1.0, 1.0])), (1.0, 1.0));
        selection.background = SelectionBackground::Transparent;
        assert_eq!(selection.value(Some([0.0, 0.0, 1.0])), (0.25, 1.0));
        assert_eq!(selection.value(Some([1.0, 1.0, 1.0])), (0.25, 0.0));
    }
}
