//! The Log panel: the session's messages under a General and an Assets tab, one per channel

use egui::{RichText, Ui};
use serde::{Deserialize, Serialize};

use crate::TextureCraftApp;
use crate::i18n::tr;

/// The Log panel's channels, one tab each
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogChannel {
    /// Everything that is not about assets: projects, exports, bakes, the UI
    #[default]
    General,
    /// Loading and importing resources, templates, presets and materials
    Assets,
}

impl LogChannel {
    pub const ALL: [LogChannel; 2] = [LogChannel::General, LogChannel::Assets];

    pub fn name(self) -> &'static str {
        match self {
            LogChannel::General => "General",
            LogChannel::Assets => "Assets",
        }
    }
}

/// How messages about assets begin: a resource that does not decode, a skipped template or
/// preset, and the warnings of a PBR folder or MaterialX import
pub const ASSET_PREFIXES: [&str; 6] = ["resource ", "Project template skipped: ", "Export template skipped: ", "Brush preset skipped: ", "PBR ", "MaterialX "];

/// The channel a log message belongs to
pub fn channel_of(message: &str) -> LogChannel {
    if ASSET_PREFIXES.iter().any(|prefix| message.starts_with(prefix)) { LogChannel::Assets } else { LogChannel::General }
}

/// The Log panel body (a dock tab or rail popup)
pub(crate) fn body(app: &mut TextureCraftApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        for channel in LogChannel::ALL {
            if ui.selectable_label(app.ui.log_channel == channel, tr(channel.name())).clicked() {
                app.ui.log_channel = channel;
            }
        }
    });
    ui.separator();

    let shown = app.ui.log_channel;
    egui::ScrollArea::vertical().id_salt(("log", shown.name())).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        for line in app.session.log.iter().filter(|line| channel_of(line) == shown) {
            ui.label(RichText::new(line).monospace());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_messages_go_to_the_assets_channel() {
        assert_eq!(channel_of("resource stone.png: unsupported format"), LogChannel::Assets);
        assert_eq!(channel_of("Brush preset skipped: broken.json"), LogChannel::Assets);
        assert_eq!(channel_of("PBR bricks: no roughness map"), LogChannel::Assets);
        assert_eq!(channel_of("Saved /tmp/a.tcraft"), LogChannel::General);
        assert_eq!(channel_of("Exported 4 textures to /tmp"), LogChannel::General);
    }
}
