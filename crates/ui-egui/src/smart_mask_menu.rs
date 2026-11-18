//! "Create smart mask" in a layer's context menu: the mask goes to the user library as a `.tcmask` file the shelf lists

use serde_json::json;
use texturecraft_engine::core::Layer;
use texturecraft_engine::smart_masks::SMART_MASK_EXT;

use crate::TextureCraftApp;
use crate::i18n::trf;

/// Save `layer`'s mask into `dir` under the layer's name (a number is added when the name is taken, so no earlier file is overwritten)
pub(crate) fn save(app: &mut TextureCraftApp, layer: &Layer, dir: &std::path::Path) {
    let stem = texturecraft_engine::formats::sanitize_resource_name(&layer.name);
    let mut file = dir.join(format!("{stem}.{SMART_MASK_EXT}"));
    let mut number = 2;
    while file.exists() {
        file = dir.join(format!("{stem} {number}.{SMART_MASK_EXT}"));
        number += 1;
    }

    if let Some(saved) = app.run("mask.smart.create", json!({"layer": layer.id.0, "path": file.display().to_string(), "name": layer.name})) {
        let path = saved.get("path").and_then(|path| path.as_str()).unwrap_or_default().to_string();
        app.toast(trf("Smart mask saved to {path}", &[("path", &path)]), false);
        app.library = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> (TextureCraftApp, u64) {
        let mut session = texturecraft_engine::sample_session(128, 16).unwrap();
        let layer = texturecraft_engine::execute(&mut session, "layer.add.fill", &serde_json::Value::Null).unwrap()["layer"].as_u64().unwrap();
        texturecraft_engine::execute(&mut session, "mask.add.black", &json!({"layer": layer})).unwrap();
        texturecraft_engine::execute(&mut session, "effect.add.generator", &json!({"layer": layer, "generator": "edgeWear"})).unwrap();
        (TextureCraftApp::new(&eframe::CreationContext::_new_kittest(egui::Context::default()), session), layer)
    }

    #[test]
    fn saves_the_mask_to_the_library_and_never_over_an_earlier_one() {
        let dir = std::env::temp_dir().join(format!("tc-smart-mask-menu-{}", std::process::id()));
        let (mut app, id) = app();
        let layer = app.session.doc.project.layer(texturecraft_engine::core::LayerId(id)).unwrap().clone();
        app.library = Some(Vec::new());

        save(&mut app, &layer, &dir);
        save(&mut app, &layer, &dir);

        let stem = texturecraft_engine::formats::sanitize_resource_name(&layer.name);
        let first = dir.join(format!("{stem}.{SMART_MASK_EXT}"));
        let second = dir.join(format!("{stem} 2.{SMART_MASK_EXT}"));
        let (first_saved, second_saved) = (first.exists(), second.exists());
        let bytes = std::fs::read(&first).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);

        assert!(first_saved && second_saved, "the second save gets its own file");
        assert!(texturecraft_engine::formats::load_project_bytes(&bytes).is_ok(), "the file is a project container");
        assert!(app.library.is_none(), "the shelf rescans the library");
        assert_eq!(app.toasts.len(), 2, "each save says where it went");
    }
}
