//! The layer menu's Gamma corrected alpha/mask, Export channels to files and Lock layer, and the
//! lock icon on a locked layer's row.

use std::path::PathBuf;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use texturecraft_engine::core::LayerId;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::file_dialogs::Answer;

const LAYER: &str = "Masked layer";

fn harness(mask: bool) -> (Harness<'static, TextureCraftApp>, LayerId) {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let layer = LayerId(execute(&mut session, "layer.add.fill", &json!({"name": LAYER})).unwrap()["layer"].as_u64().unwrap());
    if mask {
        execute(&mut session, "mask.add.black", &Value::Null).unwrap();
    }
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    (h, layer)
}

fn open_menu(h: &mut Harness<'_, TextureCraftApp>) {
    h.get_by_label(LAYER).click_secondary();
    h.run_steps(3);
}

fn layer<'a>(h: &'a Harness<'_, TextureCraftApp>, id: LayerId) -> &'a texturecraft_engine::core::Layer {
    h.state().session.doc.project.layer(id).unwrap()
}

#[test]
fn gamma_corrected_is_a_checkable_mask_item_stored_with_the_project() {
    let (mut h, id) = harness(true);
    open_menu(&mut h);
    h.get_by_label("Gamma corrected alpha/mask").click();
    h.run_steps(3);
    assert!(layer(&h, id).mask.as_ref().unwrap().gamma_corrected);

    // The item is checked now, and a second click switches it off again
    open_menu(&mut h);
    h.get_by_label("Gamma corrected alpha/mask").click();
    h.run_steps(3);
    assert!(!layer(&h, id).mask.as_ref().unwrap().gamma_corrected);
}

#[test]
fn gamma_corrected_needs_a_mask() {
    let (mut h, id) = harness(false);
    open_menu(&mut h);
    h.get_by_label("Gamma corrected alpha/mask").click();
    h.run_steps(3);
    assert!(layer(&h, id).mask.is_none());
    assert!(h.state().session.doc.project.layer(id).is_ok());
}

#[test]
fn lock_layer_locks_the_row_and_its_lock_icon_unlocks() {
    let (mut h, id) = harness(false);
    assert!(h.query_by_label("Unlock layer").is_none(), "no lock icon on an unlocked layer");
    open_menu(&mut h);
    h.get_by_label("Lock layer").click();
    h.run_steps(3);
    assert!(layer(&h, id).locked);

    h.get_by_label("Unlock layer").click();
    h.run_steps(3);
    assert!(!layer(&h, id).locked);
    assert!(h.query_by_label("Unlock layer").is_none(), "the icon goes with the lock");
}

fn temp_file(test: &str, name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texturecraft-layer-menu-ui-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn export_channels_writes_one_image_per_channel_where_the_dialog_answered() {
    let (mut h, _) = harness(false);
    let file = temp_file("export", "wall.png");
    let answer = file.clone();

    // The dialog never opens for real: the override answers it
    h.state_mut().file_dialogs.set_override(Some(Box::new(move |_| Answer::Paths(vec![answer.clone()]))));
    open_menu(&mut h);
    h.get_by_label("Export channels to files").click();
    h.run_steps(4);

    let dir = file.parent().unwrap();
    assert!(dir.join("wall_baseColor.png").exists(), "{:?}", std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>());
    assert!(
        h.state().toasts.iter().any(|t| !t.error && t.text.contains("channel images")),
        "{:?}",
        h.state().toasts.iter().map(|t| &t.text).collect::<Vec<_>>()
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_canceled_channel_export_writes_nothing() {
    let (mut h, _) = harness(false);
    h.state_mut().file_dialogs.set_override(Some(Box::new(|_| Answer::Cancelled)));
    open_menu(&mut h);
    h.get_by_label("Export channels to files").click();
    h.run_steps(4);
    assert!(h.state().toasts.is_empty());
}
