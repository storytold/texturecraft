use std::collections::HashMap;

use egui::{Id, ScrollArea};
use texturecraft_engine::core::LayerId;
use texturecraft_engine::{Session, StackTarget};

pub(crate) type PropertiesSelection = Option<(LayerId, StackTarget)>;

/// Docked Properties offsets belong to the active document and its live selections
#[derive(Default)]
pub(crate) struct PropertiesScroll {
    project_id: u64,
    revision: u64,
    drawn: Option<(Id, PropertiesSelection)>,
    offsets: HashMap<PropertiesSelection, f32>,
}

impl PropertiesScroll {
    pub(crate) fn sync(&mut self, session: &Session) {
        if self.project_id != session.project_id {
            *self = Self { project_id: session.project_id, revision: session.revision, ..Self::default() };
        }

        if self.revision == session.revision {
            return;
        }

        self.revision = session.revision;
        let project = session.project();
        self.offsets.retain(|selection, _| {
            let Some((layer_id, target)) = selection else { return true };
            let Some(layer) = project.layers.get(layer_id) else { return false };

            match target {
                StackTarget::Content => true,
                StackTarget::Mask => project.layers.get(&project.mask_owner(*layer_id)).is_some_and(|owner| owner.mask.is_some()),
                StackTarget::Effect(effect_id) => {
                    // Instances can edit a source effect while remaining selected
                    let source = layer.instance_source().and_then(|id| project.content_layer(id)).unwrap_or(layer);
                    let mask = layer.mask.as_ref().or(source.mask.as_ref());
                    mask.into_iter().flat_map(|mask| &mask.effects).chain(&source.content_effects).any(|effect| effect.id == *effect_id)
                        || project.effect(*effect_id).is_some()
                }
            }
        });

        if self.offsets.capacity() / 2 > self.offsets.len() {
            self.offsets.shrink_to_fit();
        }
    }

    pub(crate) fn prepare(&mut self, area: ScrollArea, ui_id: Id, selection: PropertiesSelection) -> ScrollArea {
        let current = (ui_id, selection);
        if self.drawn == Some(current) {
            return area;
        }

        self.drawn = Some(current);
        area.vertical_scroll_offset(self.offsets.get(&selection).copied().unwrap_or_default())
    }

    pub(crate) fn remember(&mut self, selection: PropertiesSelection, offset_points: f32) {
        if offset_points > 0.0 {
            self.offsets.insert(selection, offset_points);
        } else {
            self.offsets.remove(&selection);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use texturecraft_engine::{execute, sample_session};

    #[test]
    fn instance_offsets_survive_selection_changes_until_the_source_targets_are_removed() {
        let mut session = sample_session(128, 16).unwrap();
        execute(&mut session, "mask.add.white", &json!({})).unwrap();
        execute(&mut session, "layer.instance", &json!({})).unwrap();
        let instance = session.selected_layer().unwrap();
        execute(&mut session, "layer.select", &json!({"layer": instance.0, "target": "mask"})).unwrap();
        let mask = Some((instance, session.target));
        execute(&mut session, "effect.add.levels", &json!({"target": "mask"})).unwrap();
        let effect = Some((instance, session.target));
        let mut scroll = PropertiesScroll::default();
        scroll.sync(&session);
        scroll.remember(mask, 24.0);
        scroll.remember(effect, 52.0);

        execute(&mut session, "layer.select", &json!({"layer": instance.0})).unwrap();
        scroll.sync(&session);

        assert_eq!(scroll.offsets.get(&mask), Some(&24.0));
        assert_eq!(scroll.offsets.get(&effect), Some(&52.0));

        execute(&mut session, "mask.remove", &json!({})).unwrap();
        scroll.sync(&session);

        assert!(scroll.offsets.is_empty());
    }

    #[test]
    fn offsets_follow_live_selections_and_clear_on_project_replacement() {
        let mut session = sample_session(128, 16).unwrap();
        let layer_id = LayerId(execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].as_u64().unwrap());
        let effect_id =
            LayerId(execute(&mut session, "effect.add.levels", &json!({"layer": layer_id.0, "target": "content"})).unwrap()["effect"].as_u64().unwrap());
        let mut scroll = PropertiesScroll::default();
        let content = Some((layer_id, StackTarget::Content));
        let effect = Some((layer_id, StackTarget::Effect(effect_id)));
        scroll.sync(&session);
        scroll.remember(content, 42.0);
        scroll.remember(effect, 68.0);

        let context = egui::Context::default();
        for (selection, expected_offset) in [(content, 42.0), (effect, 68.0), (content, 42.0)] {
            let mut frame = context.run_ui(egui::RawInput::default(), |ui| {
                let area = scroll.prepare(ScrollArea::vertical().max_height(80.0), ui.scope_id(), selection);
                let output = area.show(ui, |ui| ui.add_space(256.0));

                assert_eq!(output.state.clamped_offset().y, expected_offset);
            });

            // This headless test has no renderer to apply texture updates
            frame.textures_delta.clear();
        }

        execute(&mut session, "effect.remove", &json!({"effect": effect_id.0})).unwrap();
        scroll.sync(&session);

        assert_eq!(scroll.offsets.get(&content), Some(&42.0));
        assert!(!scroll.offsets.contains_key(&effect));

        execute(&mut session, "file.close", &json!({})).unwrap();
        scroll.sync(&session);

        assert!(scroll.offsets.is_empty() && scroll.drawn.is_none());
    }
}
