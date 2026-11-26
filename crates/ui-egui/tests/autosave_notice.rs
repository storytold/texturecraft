//! A failed autosave is announced in the interface. This is its own test binary because the data
//! folder it breaks is fixed once for the whole process.

use std::time::{Duration, Instant};

use egui_kittest::Harness;
use texturecraft_ui_egui::TextureCraftApp;

fn notices(harness: &Harness<'_, TextureCraftApp>) -> usize {
    // Other loaders name the broken data folder too: only the autosave's own notice counts
    harness.state().toasts.iter().filter(|toast| toast.text.starts_with("Autosave to ") && toast.text.contains("autosave-notice")).count()
}

/// Run frames until `done`, giving the autosave worker real time to finish.
fn run_until(harness: &mut Harness<'_, TextureCraftApp>, done: impl Fn(&Harness<'_, TextureCraftApp>) -> bool) {
    let started = Instant::now();
    while !done(harness) {
        assert!(started.elapsed() < Duration::from_secs(60), "the autosave did not report");
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_failing_autosave_is_announced_once_until_it_works_again() {
    let root = std::env::temp_dir().join(format!("tc-autosave-notice-{}", std::process::id()));
    // A file where the data folder should be, so the autosave folder cannot be created
    std::fs::write(&root, b"not a folder").unwrap();
    texturecraft_ui_egui::set_data_root(root.clone());

    let session = texturecraft_engine::sample_session(128, 32).unwrap();
    // Each frame is longer than the shortest autosave period, so every frame is due
    let mut harness = Harness::builder()
        .with_step_dt(61.0)
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    // The notice is matched by its English text, whatever the system's language
    harness.state_mut().ui.prefs.language = "en".into();
    // One frame applies the language before any autosave reports
    harness.step();
    harness.state_mut().ui.prefs.autosave_minutes = 1;
    harness.state_mut().session.modified = true;

    run_until(&mut harness, |harness| notices(harness) > 0);
    for _ in 0..20 {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(notices(&harness), 1, "the same failure is not announced again");

    std::fs::remove_file(&root).unwrap();
    run_until(&mut harness, |harness| notices(harness) == 0);
    let _ = std::fs::remove_dir_all(&root);
}
