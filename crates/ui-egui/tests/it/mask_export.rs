//! A layer's context menu saves its evaluated mask as an image file or copies it to the clipboard.

use std::path::PathBuf;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::file_dialogs::Answer;

const RESOLUTION: u32 = 128;
const LAYER: &str = "Masked layer";

/// A project with a layer named [`LAYER`], whose mask is 0.5 where the mesh covers the texture
/// when `mask` is set, and a harness showing it.
fn harness(mask: bool) -> Harness<'static, TextureCraftApp> {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, RESOLUTION).unwrap();
    execute(&mut session, "layer.add.fill", &json!({"name": LAYER})).unwrap();
    if mask {
        execute(&mut session, "mask.add.black", &Value::Null).unwrap();
        execute(&mut session, "effect.add.fill", &json!({"value": 0.5})).unwrap();
    }
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    h
}

fn open_menu(h: &mut Harness<'_, TextureCraftApp>) {
    h.get_by_label(LAYER).click_secondary();
    h.run_steps(3);
}

/// A path in a new temporary folder.
fn temp_file(test: &str, name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texturecraft-mask-export-ui-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn copied_images(h: &Harness<'_, TextureCraftApp>) -> Vec<egui::ColorImage> {
    h.output()
        .platform_output
        .commands
        .iter()
        .filter_map(|command| match command {
            egui::OutputCommand::CopyImage(image) => Some(image.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn export_mask_to_file_writes_the_mask_where_the_dialog_answered() {
    let mut h = harness(true);
    let file = temp_file("file", "picked.png");
    let answer = file.clone();
    h.state_mut().file_dialogs.set_override(Some(Box::new(move |_| Answer::Paths(vec![answer.clone()]))));
    open_menu(&mut h);
    h.get_by_label("Export mask to file...").click();
    h.run_steps(4);

    let gray = image::open(&file).expect("the mask file was written").into_luma16();
    assert_eq!(gray.dimensions(), (RESOLUTION, RESOLUTION), "the texture set's size");
    assert!(gray.pixels().any(|p| p.0[0] == 32768), "the mask's 0.5 as a 16-bit level");
    assert!(h.state().toasts.iter().any(|t| !t.error && t.text.contains("picked.png")), "{:?}", h.state().toasts.iter().map(|t| &t.text).collect::<Vec<_>>());
    let _ = std::fs::remove_dir_all(file.parent().unwrap());
}

#[test]
fn a_canceled_dialog_writes_nothing() {
    let mut h = harness(true);
    h.state_mut().file_dialogs.set_override(Some(Box::new(|_| Answer::Cancelled)));
    open_menu(&mut h);
    h.get_by_label("Export mask to file...").click();
    h.run_steps(4);
    assert!(h.state().toasts.is_empty());
}

#[test]
fn copy_mask_to_clipboard_puts_the_gray_image_on_the_clipboard() {
    let mut h = harness(true);
    open_menu(&mut h);
    h.get_by_label("Copy mask to clipboard").click();

    // The copy command is in the output of the frame that handled the click only
    h.step();

    let images = copied_images(&h);
    assert_eq!(images.len(), 1, "one image was copied");
    assert_eq!(images[0].size, [RESOLUTION as usize, RESOLUTION as usize]);
    assert!(images[0].pixels.iter().any(|p| *p == egui::Color32::from_gray(128)), "the mask's 0.5 as 8-bit gray");
    assert!(h.state().toasts.iter().any(|t| !t.error && t.text.contains("clipboard")));
}

#[test]
fn a_layer_without_a_mask_cannot_export_or_copy_one() {
    let mut h = harness(false);
    open_menu(&mut h);
    h.get_by_label("Copy mask to clipboard").click();
    h.step();
    assert!(copied_images(&h).is_empty(), "nothing to copy");
    assert_eq!(h.state().file_dialogs.pending(), 0);
}
