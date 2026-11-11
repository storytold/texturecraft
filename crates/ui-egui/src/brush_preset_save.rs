//! Saving the current brush as a preset (Properties › Brush › Preset); presets show on the shelf's Brushes tab

use egui::Ui;
use serde_json::json;
use texturecraft_engine::brush_preset_cmds::BRUSH_EXT;

use crate::TextureCraftApp;
use crate::i18n::tr;

/// The preset name field's width in points
const NAME_WIDTH: f32 = 110.0;

/// A name field and a button that saves the current brush under that name
pub(crate) fn row(app: &mut TextureCraftApp, ui: &mut Ui) {
    let dir = crate::user_dir("brushes");
    ui.horizontal(|ui| {
        let name_id = ui.make_persistent_id("brush_preset_name");
        let mut new_name: String = ui.data_mut(|data| data.get_temp(name_id)).unwrap_or_default();
        ui.add(egui::TextEdit::singleline(&mut new_name).hint_text(tr("Brush name")).desired_width(NAME_WIDTH));

        let file = texturecraft_engine::formats::export::sanitize_file_stem(new_name.trim());
        if ui.add_enabled(!new_name.trim().is_empty(), egui::Button::new(tr("Save current brush"))).clicked() {
            let path = dir.join(format!("{file}.{BRUSH_EXT}"));
            // A folder that can't be made fails the save, which reports it
            let _ = std::fs::create_dir_all(&dir);
            if app.run("brush.preset.save", json!({"path": path.display().to_string()})).is_some() {
                new_name.clear();
                // Saving over a preset keeps its file name, so its tile must redraw
                app.brushes = None;
                app.thumbs.forget_saved_files();
            }
        }

        ui.data_mut(|data| data.insert_temp(name_id, new_name));
    });
}
