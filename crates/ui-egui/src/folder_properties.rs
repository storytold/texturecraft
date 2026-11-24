//! Properties of a folder: CONTENTS, the Scale and Rotation laid on every fill inside on top of their own (the round reset drops them and puts back the seeds Randomize seeds changed), and Randomize seeds

use egui::Ui;
use egui_phosphor::regular as ic;
use serde_json::json;
use texturecraft_engine::core::{ContentsOffset, LayerId};
use texturecraft_engine::folder_cmds::{contents_changed, has_contents};

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::property_sheet::PropertySheet;

/// Halvings and doublings the Scale track spans each way from ×1 (typed values go to the engine's range)
const SCALE_DOUBLINGS: f32 = 2.0;

/// The properties of folder `folder`; every change goes through the `folder.*` commands
pub(crate) fn folder_properties(app: &mut TextureCraftApp, ui: &mut Ui, folder: LayerId) {
    let project = &app.session.doc.project;
    if !has_contents(project, folder) {
        crate::widgets::empty_note(ui, tr("No properties found"));
        return;
    }

    let offset = project.contents_offset(folder);
    let changed = contents_changed(project, folder);

    PropertySheet::show(ui, ("folder_sheet", folder.0), |sheet| {
        if sheet.section_reset(tr("Contents"), changed) {
            app.run("folder.contents.reset", json!({"layer": folder.0}));
        }

        let (low, high) = ContentsOffset::SCALE_RANGE;
        let mut scale = offset.scale;
        if let Some(merge) = sheet.factor_slider(tr("Scale"), &mut scale, SCALE_DOUBLINGS, low..=high) {
            app.run("folder.contents.set", json!({"layer": folder.0, "scale": scale, "merge": merge}));
        }

        let (turn_start, turn_end) = ContentsOffset::ROTATION_RANGE;
        let mut rotation = offset.rotation;
        if let Some(merge) = sheet.relative_angle(tr("Rotation"), &mut rotation, turn_start..=turn_end) {
            app.run("folder.contents.set", json!({"layer": folder.0, "rotation": rotation, "merge": merge}));
        }

        if sheet.button(ic::DICE_FIVE, tr("Randomize seeds")).clicked() {
            app.run("folder.randomizeSeeds", json!({"layer": folder.0}));
        }
    });
}
