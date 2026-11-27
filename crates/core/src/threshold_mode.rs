//! How a threshold compares a value with its level

use serde::{Deserialize, Serialize};

/// Which values a threshold turns white: those above its level, at or above it, below it, or at or below it
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThresholdMode {
    Greater,
    /// What thresholds did in projects saved before the modes
    #[default]
    GreaterOrEqual,
    Lower,
    LowerOrEqual,
}

impl ThresholdMode {
    /// Every mode, in the order a panel shows them
    pub const ALL: [ThresholdMode; 4] = [ThresholdMode::Greater, ThresholdMode::GreaterOrEqual, ThresholdMode::Lower, ThresholdMode::LowerOrEqual];

    /// The comparison sign a panel shows
    pub fn sign(self) -> &'static str {
        match self {
            ThresholdMode::Greater => ">",
            ThresholdMode::GreaterOrEqual => "≥",
            ThresholdMode::Lower => "<",
            ThresholdMode::LowerOrEqual => "≤",
        }
    }

    /// The name a tooltip shows
    pub fn label(self) -> &'static str {
        match self {
            ThresholdMode::Greater => "Greater",
            ThresholdMode::GreaterOrEqual => "Greater or equal",
            ThresholdMode::Lower => "Lower",
            ThresholdMode::LowerOrEqual => "Lower or equal",
        }
    }

    /// The id `effect.set {mode}` takes
    pub fn id(self) -> &'static str {
        match self {
            ThresholdMode::Greater => "greater",
            ThresholdMode::GreaterOrEqual => "greaterOrEqual",
            ThresholdMode::Lower => "lower",
            ThresholdMode::LowerOrEqual => "lowerOrEqual",
        }
    }

    /// Whether `value` turns white against `level`
    pub fn passes(self, value: f32, level: f32) -> bool {
        match self {
            ThresholdMode::Greater => value > level,
            ThresholdMode::GreaterOrEqual => value >= level,
            ThresholdMode::Lower => value < level,
            ThresholdMode::LowerOrEqual => value <= level,
        }
    }

    /// Whether it is the mode projects saved before the modes read as (it is then left out of a saved file)
    pub fn is_default(&self) -> bool {
        *self == ThresholdMode::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChannelKind, Filter};

    #[test]
    fn a_threshold_saved_before_the_modes_reads_and_saves_as_before() {
        let saved = r#"{"Threshold":{"level":0.5}}"#;
        let filter: Filter = serde_json::from_str(saved).unwrap();

        assert_eq!(filter, Filter::Threshold { level: 0.5, mode: ThresholdMode::GreaterOrEqual });
        assert_eq!(serde_json::to_string(&filter).unwrap(), saved);
        assert_eq!(filter.point(ChannelKind::Scalar, [0.5; 3]), Some([1.0; 3]), "a value at the level turns white as before");
    }

    #[test]
    fn each_mode_compares_its_way() {
        let at = |mode: ThresholdMode, value: f32| Filter::Threshold { level: 0.5, mode }.point(ChannelKind::Scalar, [value; 3]).map(|v| v[0]);
        assert_eq!([0.4, 0.5, 0.6].map(|value| at(ThresholdMode::Greater, value)), [Some(0.0), Some(0.0), Some(1.0)]);
        assert_eq!([0.4, 0.5, 0.6].map(|value| at(ThresholdMode::Lower, value)), [Some(1.0), Some(0.0), Some(0.0)]);
        assert_eq!([0.4, 0.5, 0.6].map(|value| at(ThresholdMode::LowerOrEqual, value)), [Some(1.0), Some(1.0), Some(0.0)]);

        let saved = serde_json::to_string(&Filter::Threshold { level: 0.01, mode: ThresholdMode::LowerOrEqual }).unwrap();

        assert!(saved.contains(r#""mode":"lowerOrEqual""#), "{saved}");
    }
}
