//! The Material section of a brush tool's Properties: the channel strip, then one card per channel that is on with the value the brush paints there

use serde_json::json;
use texturecraft_engine::Tool;
use texturecraft_engine::core::{Channel, ChannelKind, Source};

use crate::TextureCraftApp;
use crate::fill_material_panel::{pill_name, take_color_pick};
use crate::i18n::tr;
use crate::property_sheet::PropertySheet;
use crate::slot_thumbnail::{paint_thumbnail, thumbnail};
use crate::source_slot::SourceSlot;
use crate::theme::Tokens;

/// The channel strip (the fill layer's pills), then a card per channel that is on; the eraser only picks channels
pub(crate) fn material_section(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>) {
    let channels = app.session.doc.project.texture_set(app.session.ts()).map(|texture_set| texture_set.channel_list()).unwrap_or_default();
    let names: Vec<&str> = channels.iter().map(|channel| tr(pill_name(*channel))).collect();
    let enabled: Vec<bool> = channels.iter().map(|channel| app.session.tool.enabled.contains(channel)).collect();
    if let Some((index, solo)) = sheet.channel_pills(&names, &enabled)
        && let Some(channel) = channels.get(index)
    {
        let enable = solo || !enabled.get(index).copied().unwrap_or(false);
        app.run("tool.material", json!({"channel": channel.id(), "enabled": enable, "solo": solo}));
    }
    if app.session.tool.tool == Tool::Eraser {
        return;
    }

    let shown: Vec<Channel> = channels.into_iter().filter(|channel| app.session.tool.enabled.contains(channel)).collect();
    for channel in shown {
        channel_card(app, sheet, channel);
    }
}

/// One channel's card: its swatch, name and source, the ⟲ back to the channel's default, then its colour bar or gray ramp (a normal paints flat and has none)
fn channel_card(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, channel: Channel) {
    let slot_of = SourceSlot::Channel(channel);
    let kind = channel.kind();
    let default = channel.default_value();
    let value = if kind == ChannelKind::NormalMap { default } else { app.session.tool.values.get(&channel).copied().unwrap_or(default) };

    let tokens = Tokens::get(sheet.ui.ctx());
    let picture = thumbnail(app, &sheet.ui.ctx().clone(), &Source::Uniform(value));
    let thumb = |painter: &egui::Painter, rect: egui::Rect| paint_thumbnail(painter, rect, &picture, &tokens);
    let line = if kind == ChannelKind::NormalMap { tr("Flat") } else { tr("Uniform") };
    if sheet.slot_fixed(&thumb, slot_of.name(), line, value != default).reset {
        app.run("tool.material", json!({"channel": channel.id(), "value": default}));
    }

    match kind {
        ChannelKind::Color => {
            let pick = sheet.color_bar(slot_of.name(), value, app.picking_color, &app.session.doc.project.swatches);
            if let Some(color) = take_color_pick(app, pick) {
                app.run("tool.material", json!({"channel": channel.id(), "color": color}));
            }
        }
        ChannelKind::Scalar | ChannelKind::Signed => {
            let mut gray = value[0];
            let edit = sheet.ramp(slot_of.name(), &mut gray, slot_of.range());
            if edit.invert {
                let inverted = if kind == ChannelKind::Signed { -gray } else { 1.0 - gray };
                app.run("tool.material", json!({"channel": channel.id(), "value": inverted}));
            } else if edit.changed.is_some() {
                app.run("tool.material", json!({"channel": channel.id(), "value": gray}));
            }
        }
        ChannelKind::NormalMap => {}
    }
    sheet.close_card();
}
