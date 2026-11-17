//! Saving the current brush as a preset (Properties › Brush › Preset); presets show on the shelf's Brushes tab

use egui::Ui;
use serde_json::json;
use std::path::Path;
use texturecraft_engine::brush_preset_cmds::BRUSH_EXT;

use crate::TextureCraftApp;
use crate::i18n::tr;

/// The preset name field's width in points
const NAME_WIDTH: f32 = 110.0;

/// A name field and a button that saves the current brush under that name
pub(crate) fn row(app: &mut TextureCraftApp, ui: &mut Ui) {
    let dir = crate::user_dir("brushes");
    let name_id = ui.make_persistent_id("brush_preset_name");
    let confirm_id = ui.make_persistent_id("brush_preset_overwrite");
    let mut new_name: String = ui.data_mut(|data| data.get_temp(name_id)).unwrap_or_default();
    let mut pending_path: Option<std::path::PathBuf> = ui.data_mut(|data| data.get_temp(confirm_id));

    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut new_name).hint_text(tr("Brush name")).desired_width(NAME_WIDTH));

        if ui.add_enabled(!new_name.trim().is_empty(), egui::Button::new(tr("Save current brush"))).clicked() {
            let path = dir.join(format!("{}.{BRUSH_EXT}", preset_file_stem(new_name.trim())));
            match std::fs::symlink_metadata(&path) {
                Ok(_) => pending_path = Some(path),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if save_brush(app, &dir, &path) {
                        new_name.clear();
                    }
                }
                Err(_) => pending_path = Some(path),
            }
        }
    });

    if let Some(path) = pending_path.clone() {
        let mut replace = false;
        let mut cancel = false;
        egui::Window::new(tr("Replace"))
            .id(ui.make_persistent_id("brush_preset_replace_dialog"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                ui.label(path.file_name().and_then(|name| name.to_str()).unwrap_or_default());
                ui.horizontal(|ui| {
                    replace = ui.button(tr("Replace")).clicked();
                    cancel = ui.button(tr("Cancel")).clicked();
                });
            });
        if replace {
            if save_brush(app, &dir, &path) {
                new_name.clear();
            }
            pending_path = None;
        } else if cancel {
            pending_path = None;
        }
    }

    ui.data_mut(|data| {
        data.insert_temp(name_id, new_name);
        data.insert_temp(confirm_id, pending_path);
    });
}

/// Preserve Unicode names and encode filename punctuation without collisions
fn preset_file_stem(name: &str) -> String {
    let mut stem = String::new();

    // Six digits cover every Unicode scalar and separate adjacent escapes
    for character in name.chars() {
        if character.is_alphanumeric() || character == '-' || character == '_' || character == ' ' {
            stem.push(character);
        } else {
            stem.push_str(&format!("~{:06X}", character as u32));
        }
    }
    let uppercase = stem.to_ascii_uppercase();
    if matches!(
        uppercase.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        stem.insert_str(0, "brush-");
    }
    stem
}

/// Save a preset and invalidate cached tiles only when the write succeeds
fn save_brush(app: &mut TextureCraftApp, dir: &Path, path: &Path) -> bool {
    // A folder that cannot be made is reported by the save command
    let _ = std::fs::create_dir_all(dir);
    if app.run("brush.preset.save", json!({"path": path.display().to_string()})).is_none() {
        return false;
    }

    app.brushes = None;
    app.thumbs.forget_saved_files();
    true
}

#[cfg(test)]
mod tests {
    use super::preset_file_stem;

    #[test]
    fn distinct_unicode_brush_names_have_distinct_safe_file_stems() {
        assert_ne!(preset_file_stem("水彩"), preset_file_stem("油画"));
        assert_ne!(preset_file_stem("a/b"), preset_file_stem("a~00002Fb"));
        assert_eq!(preset_file_stem("水彩"), "水彩");
        assert!(!preset_file_stem("a/b").contains('/'));
        assert_eq!(preset_file_stem("CON"), "brush-CON");
    }
}
