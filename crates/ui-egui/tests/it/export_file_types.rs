//! Export textures › file types (texturecraft #672): the reference's file type list, a bit depth
//! list that follows the file type, dithering and "Dilation + diffusion".

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use texturecraft_ui_egui::TextureCraftApp;

/// The Export textures window open on SETTINGS
fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    h.state_mut().ui.windows.export = true;
    h.run_steps(4);
    h
}

fn combo<'a>(h: &'a Harness<'_, TextureCraftApp>, value: &str) -> egui_kittest::Node<'a> {
    h.get_all_by_role(egui::Role::ComboBox).find(|c| c.value().as_deref() == Some(value)).unwrap_or_else(|| panic!("no dropdown at {value:?}"))
}

fn open_combo(h: &mut Harness<'_, TextureCraftApp>, value: &str) {
    combo(h, value).click();
    h.run_steps(4);
}

fn click(h: &mut Harness<'_, TextureCraftApp>, label: &str) {
    h.get_all_by_label(label).last().unwrap_or_else(|| panic!("no {label}")).click();
    h.run_steps(4);
}

/// Pick file type `name` in the global File type list showing `from`
fn pick_type(h: &mut Harness<'_, TextureCraftApp>, from: &str, name: &str) {
    open_combo(h, from);
    click(h, name);
}

/// The entries of the open list, top to bottom
fn entries(h: &Harness<'_, TextureCraftApp>, labels: &[&str]) -> Vec<String> {
    let mut found: Vec<(f32, String)> = labels.iter().filter_map(|l| h.query_all_by_label(l).last().map(|n| (n.rect().top(), (*l).to_string()))).collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().map(|(_, l)| l).collect()
}

const TYPES: [&str; 14] = ["bmp", "jpeg", "jng", "pgm", "png", "targa", "tiff", "hdr", "exr", "j2k", "jp2", "webp", "psd", "sbsar"];

#[test]
fn the_file_type_list_is_the_reference_one_with_three_greyed() {
    let mut h = harness();
    open_combo(&mut h, "Based on output template");
    assert_eq!(entries(&h, &TYPES), TYPES.map(String::from).to_vec());

    // Bottom up, so no tooltip covers the next entry
    for name in ["sbsar", "jp2", "j2k"] {
        assert!(h.get_all_by_label(name).last().unwrap().accesskit_node().is_disabled(), "{name} is greyed");
        h.get_all_by_label(name).last().unwrap().scroll_to_me();
        h.run_steps(6);
        let at = h.get_all_by_label(name).last().unwrap().rect().center();
        h.hover_at(at);
        h.run_steps(30);
        assert!(h.query_by_label("Not available yet").is_some(), "{name} at {at:?}");
    }
    assert!(!h.get_all_by_label("psd").last().unwrap().accesskit_node().is_disabled());
}

#[test]
fn the_bit_depth_list_follows_the_file_type() {
    let mut h = harness();
    assert!(combo(&h, "").accesskit_node().is_disabled(), "empty and greyed while the template decides");

    pick_type(&mut h, "Based on output template", "psd");
    assert_eq!((h.state().ui.export.format.as_str(), h.state().ui.export.bit_depth), ("psd", 8));
    open_combo(&mut h, "8 bits");
    assert_eq!(entries(&h, &["8 bits", "8 bits + dithering", "16 bits", "32f bits"]), ["8 bits", "8 bits + dithering", "16 bits"]);
    click(&mut h, "8 bits + dithering");
    assert!(h.state().ui.export.dithering);

    pick_type(&mut h, "psd", "exr");
    assert_eq!((h.state().ui.export.bit_depth, h.state().ui.export.dithering), (32, false), "exr starts at 32f bits");
    open_combo(&mut h, "32f bits");
    assert_eq!(entries(&h, &["8 bits", "16f bits", "16 bits", "32f bits"]), ["16f bits", "32f bits"]);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);

    pick_type(&mut h, "exr", "tiff");
    open_combo(&mut h, "8 bits");
    assert_eq!(entries(&h, &["8 bits", "8 bits + dithering", "16 bits", "32f bits"]), ["8 bits", "8 bits + dithering", "16 bits", "32f bits"]);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);

    pick_type(&mut h, "tiff", "hdr");
    assert!(combo(&h, "32f bits").accesskit_node().is_disabled(), "hdr has only 32f bits");
    pick_type(&mut h, "hdr", "jpeg");
    open_combo(&mut h, "8 bits");
    assert_eq!(entries(&h, &["8 bits", "8 bits + dithering", "16 bits"]), ["8 bits", "8 bits + dithering"]);
}

#[test]
fn padding_offers_diffusion_with_a_width() {
    let mut h = harness();
    open_combo(&mut h, "Dilation infinite");
    click(&mut h, "Dilation + diffusion");
    assert_eq!(h.state().ui.export.padding, "diffusion");
    assert!(!h.get_by_label("16").accesskit_node().is_disabled(), "diffusion has a dilation width");
}

#[test]
fn export_writes_the_chosen_type_and_depth() {
    let mut h = harness();
    let dir = std::env::temp_dir().join(format!("tc-export-file-types-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let e = &mut h.state_mut().ui.export;
    e.dir = dir.display().to_string();
    e.format = "pgm".into();
    e.bit_depth = 16;
    h.run_steps(3);
    click(&mut h, "Export");

    // The export writes a texture set per frame
    for _ in 0..64 {
        if h.state().ui.export.job.is_none() {
            break;
        }

        h.run_steps(1);
    }
    h.run_steps(2);
    let files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
    assert!(!files.is_empty());
    for f in &files {
        assert_eq!(f.extension().unwrap(), "pgm");
        assert!(std::fs::read(f).unwrap().starts_with(b"P5\n256 256\n65535\n"), "{}", f.display());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_old_hidden_bit_depth_is_not_exported() {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();

    // Saved by an older build: "From template" with 16 bits
    let e = &mut h.state_mut().ui.export;
    (e.format, e.bit_depth) = (String::new(), 16);
    h.state_mut().ui.windows.export = true;
    h.run_steps(4);
    assert_eq!(h.state().ui.export.bit_depth, 0, "what the window shows (no depth) is what exports");

    // A type with a depth it does not have gets its default
    let e = &mut h.state_mut().ui.export;
    (e.format, e.bit_depth) = ("jpeg".into(), 16);
    h.run_steps(2);
    assert_eq!(h.state().ui.export.bit_depth, 8);
}
