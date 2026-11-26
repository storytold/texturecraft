//! Named bake presets: the settings of one texture set saved under a name, loaded into any
//! set later, or copied to all of them. They persist with `UiState`.

use serde_json::json;
use texturecraft_engine::core::BakeSettings;

use crate::state::UiState;

pub use crate::bake_preset::BakePreset;

impl UiState {
    pub fn bake_preset(&self, name: &str) -> Option<&BakePreset> {
        self.bake_presets.iter().find(|preset| preset.name == name.trim())
    }

    /// Save `settings` under `name` (surrounding spaces ignored), replacing a preset of that name; false for an empty name
    pub fn save_bake_preset(&mut self, name: &str, settings: &BakeSettings) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }

        let preset = BakePreset { name: name.to_string(), settings: settings.clone() };
        match self.bake_presets.iter().position(|saved| saved.name == name) {
            Some(index) => self.bake_presets[index] = preset,
            None => self.bake_presets.push(preset),
        }
        true
    }

    pub fn delete_bake_preset(&mut self, name: &str) {
        self.bake_presets.retain(|preset| preset.name != name);
    }
}

/// The `bake.settings` parameters that put `settings` on texture sets `ts` (an index, a list or "all"), keeping their Include in Bake flags
pub fn apply_params(ts: serde_json::Value, settings: &BakeSettings) -> serde_json::Value {
    json!({ "ts": ts, "model": settings, "preserveEnabled": true, "preset": true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_twice_replaces_and_empty_names_are_refused() {
        let mut ui = UiState::default();
        let mut settings = BakeSettings::default();
        assert!(!ui.save_bake_preset("  ", &settings));
        assert!(ui.save_bake_preset(" Fast ", &settings));
        settings.common.antialiasing = texturecraft_engine::core::BakeAntialiasing::Subsampling8x8;
        assert!(ui.save_bake_preset("Fast", &settings));
        assert_eq!(ui.bake_presets.len(), 1);
        assert_eq!(ui.bake_preset("Fast").unwrap().settings, settings);
        ui.delete_bake_preset("Fast");
        assert!(ui.bake_presets.is_empty());
    }
}
