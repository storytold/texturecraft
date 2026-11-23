//! Properties of an instance: INSTANCE OF, the card of the layer it draws (a click goes to it), or a missing layer where the source is gone

use egui::epaint::RectShape;
use egui::{Color32, CornerRadius, Painter, Rect, TextureId, Ui, pos2};
use serde_json::json;
use texturecraft_engine::compose::ThumbTarget;
use texturecraft_engine::core::{Channel, Layer, LayerId, Project};

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::property_sheet::PropertySheet;
use crate::slot_thumbnail::{THUMB_RADIUS, paint_empty_slot};
use crate::theme::Tokens;

/// The properties of instance `layer`: where it draws from
pub(crate) fn instance_properties(app: &mut TextureCraftApp, ui: &mut Ui, layer: &Layer) {
    let Some(source) = layer.instance_source() else { return };
    let project = &app.session.doc.project;
    let missing = !project.layers.contains_key(&source);
    let name = project.layers.get(&source).map_or_else(|| tr("Missing layer").to_string(), |source_layer| source_layer.name.clone());

    // A missing source has no texture set left; the instance's own names where it was
    let set = set_name(project, if missing { layer.id } else { source });
    let picture = if missing {
        None
    } else {
        app.layer_panel.texture_outside(ui.ctx(), &app.session, ThumbTarget::Content { layer: source, channel: Channel::BaseColor })
    };
    let tokens = Tokens::get(ui.ctx());
    let thumbnail = |painter: &Painter, rect: Rect| paint_thumbnail(painter, rect, picture, missing, &tokens);

    let clicked = PropertySheet::show(ui, ("instance_sheet", layer.id.0), |sheet| {
        sheet.section(tr("Instance of"), false);
        sheet.source_card(&thumbnail, &name, &set, missing)
    });
    if clicked {
        app.run("layer.select", json!({"layer": source.0, "target": "content"}));
    }
}

/// The name of the texture set holding `layer`, empty when none does
fn set_name(project: &Project, layer: LayerId) -> String {
    project.locate(layer).and_then(|(set_index, _, _)| project.texture_sets.get(set_index)).map(|texture_set| texture_set.name.clone()).unwrap_or_default()
}

/// The source's thumbnail with rounded corners (the field colour until it is rendered), or an empty outlined square for a missing source
fn paint_thumbnail(painter: &Painter, rect: Rect, picture: Option<TextureId>, missing: bool, tokens: &Tokens) {
    if missing {
        paint_empty_slot(painter, rect, tokens);
        return;
    }

    let radius = CornerRadius::same(THUMB_RADIUS);
    match picture {
        Some(texture) => {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.add(RectShape::filled(rect, radius, Color32::WHITE).with_texture(texture, uv));
        }
        None => {
            painter.rect_filled(rect, radius, tokens.field);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use texturecraft_engine::compose::ThumbTarget;
    use texturecraft_engine::core::{Channel, LayerId};

    use crate::docking::Panel;

    #[test]
    fn the_source_card_renders_its_picture_while_layers_is_closed() {
        // Long enough for the render worker on a busy machine
        const ATTEMPTS: usize = 500;
        const WAIT: Duration = Duration::from_millis(10);
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut harness = crate::frame_gizmo::tests::ball_app(false);
        let source = harness.state_mut().run("layer.add.fill", json!({})).unwrap()["layer"].as_u64().unwrap();
        let instance = harness.state_mut().run("layer.instance", json!({"layer": source})).unwrap()["layer"].as_u64().unwrap();
        harness.state_mut().run("layer.select", json!({"layer": instance, "target": "content"})).unwrap();
        harness.state_mut().ui.dock.close(Panel::Layers);
        let target = ThumbTarget::Content { layer: LayerId(source), channel: Channel::BaseColor };

        let rendered = (0..ATTEMPTS).any(|_| {
            harness.step();
            let fresh = harness.state().layer_panel.service().is_fresh(target);
            if !fresh {
                std::thread::sleep(WAIT);
            }
            fresh
        });

        assert!(!harness.state().ui.dock.is_open(Panel::Layers), "Layers stays closed");
        assert!(rendered, "the Instance card's source picture renders without the Layers panel");
    }
}
