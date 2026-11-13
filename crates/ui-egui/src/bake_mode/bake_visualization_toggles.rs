use serde::{Deserialize, Serialize};
use texturecraft_render::{BakeOverlay, BakeVisualization, NeutralMaterial};

/// Opacity of the cage's surface and wireframe until changed
const CAGE_OPACITY: f32 = 0.4;

/// The Baking Visualization panel's settings for bake mode's 3D view (view state, not saved in the project)
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BakeVisualizationToggles {
    /// The panel's eye: hides the high poly and the cage, whatever their own switches say
    pub hide_baking_meshes: bool,
    pub selected_set_only: bool,
    pub high_poly_mesh: BakeOverlay,
    pub matching_error: BakeOverlay,
    pub cage_surface: BakeOverlay,
    pub cage_wire: BakeOverlay,
    pub missing_seams: BakeOverlay,
    pub project_mesh: BakeOverlay,
    /// Show the project mesh with [`Self::neutral`] instead of its painted material
    pub neutral_shown: bool,
    pub neutral: NeutralMaterial,
}

impl Default for BakeVisualizationToggles {
    fn default() -> Self {
        let parts = BakeVisualization::default();
        let shown = |overlay: BakeOverlay| BakeOverlay { shown: true, ..overlay };
        Self {
            hide_baking_meshes: false,
            selected_set_only: false,
            high_poly_mesh: shown(parts.high_poly),
            matching_error: shown(parts.matching_error),
            cage_surface: BakeOverlay { opacity: CAGE_OPACITY, ..shown(parts.cage_surface) },
            cage_wire: BakeOverlay { opacity: CAGE_OPACITY, ..shown(parts.cage_wire) },
            missing_seams: shown(parts.missing_seams),
            project_mesh: shown(parts.low_poly),
            neutral_shown: true,
            neutral: NeutralMaterial::default(),
        }
    }
}

impl BakeVisualizationToggles {
    /// What the renderer draws with these settings while texture set `selected_set` is selected
    pub fn visualization(&self, selected_set: usize) -> BakeVisualization {
        let unless_hidden = |overlay: BakeOverlay| BakeOverlay { shown: overlay.shown && !self.hide_baking_meshes, ..overlay };
        BakeVisualization {
            high_poly: unless_hidden(self.high_poly_mesh),
            matching_error: self.matching_error,
            cage_surface: unless_hidden(self.cage_surface),
            cage_wire: unless_hidden(self.cage_wire),
            missing_seams: self.missing_seams,
            low_poly: self.project_mesh,
            neutral: self.neutral_shown.then_some(self.neutral),
            only_set: self.selected_set_only.then_some(selected_set),
        }
    }

    /// Whether the project mesh shows the neutral material lit along the bent normal, which the 3D view then reads in place of a mesh map
    pub fn bent_normal_shown(&self) -> bool {
        self.project_mesh.shown && self.neutral_shown && self.neutral.bent_normal
    }

    /// Switch the panel's eye; hiding the baking meshes shows the project mesh, so the view never goes empty
    pub fn toggle_hiding(&mut self) {
        self.hide_baking_meshes = !self.hide_baking_meshes;
        self.project_mesh.shown |= self.hide_baking_meshes;
    }

    /// Put the High Definition Mesh section back to its defaults
    pub fn reset_high_poly(&mut self) {
        let defaults = Self::default();
        (self.high_poly_mesh, self.matching_error) = (defaults.high_poly_mesh, defaults.matching_error);
    }

    /// Put the Cage section back to its defaults
    pub fn reset_cage(&mut self) {
        let defaults = Self::default();
        (self.cage_surface, self.cage_wire) = (defaults.cage_surface, defaults.cage_wire);
    }

    /// Put the UV Seams section back to its defaults
    pub fn reset_seams(&mut self) {
        self.missing_seams = Self::default().missing_seams;
    }

    /// Put the Project Mesh section back to its defaults
    pub fn reset_project_mesh(&mut self) {
        let defaults = Self::default();
        (self.project_mesh, self.neutral_shown, self.neutral) = (defaults.project_mesh, defaults.neutral_shown, defaults.neutral);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_settings_saved_by_earlier_builds_start_over_from_the_defaults() {
        let mut saved = serde_json::to_value(crate::state::UiState::default()).unwrap();
        let fields = saved.as_object_mut().unwrap();
        fields.remove("baking_visualization");
        fields.insert(
            "bake_visualization".into(),
            serde_json::json!({"hide_baking_meshes": false, "missing_seams": {"shown": false, "color": [1, 0, 0], "opacity": 1}}),
        );
        let state: crate::state::UiState = serde_json::from_value(saved).unwrap();
        assert_eq!(state.bake_visualization, BakeVisualizationToggles::default());
    }

    #[test]
    fn the_settings_reach_the_renderer() {
        let mut toggles = BakeVisualizationToggles::default();
        toggles.cage_wire.color = [0.1, 0.2, 0.3];
        toggles.neutral_shown = false;
        let visualization = toggles.visualization(2);
        assert_eq!(visualization.cage_wire, toggles.cage_wire);
        assert_eq!(visualization.neutral, None);
        assert_eq!(visualization.only_set, None);
        toggles.selected_set_only = true;
        assert_eq!(toggles.visualization(2).only_set, Some(2));
    }

    #[test]
    fn the_eye_hides_the_baking_meshes_and_keeps_the_project_mesh() {
        let mut toggles = BakeVisualizationToggles::default();
        let shown = toggles.visualization(0);
        assert!(shown.high_poly.shown && shown.cage_surface.shown && shown.cage_wire.shown, "the eye starts off, as in the reference");

        toggles.project_mesh.shown = false;
        toggles.toggle_hiding();
        let hidden = toggles.visualization(0);
        assert!(!hidden.high_poly.shown && !hidden.cage_surface.shown && !hidden.cage_wire.shown);
        assert!(hidden.low_poly.shown, "hiding the baking meshes brings the project mesh back");

        toggles.toggle_hiding();
        assert!(toggles.visualization(0).high_poly.shown, "the eye switches back");
    }
}
