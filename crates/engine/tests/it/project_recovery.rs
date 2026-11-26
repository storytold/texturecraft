use std::io::{Cursor, Write};

use serde_json::json;
use texturecraft_engine::{LogLevel, Session, execute, geom::primitives};
use zip::{ZipWriter, write::SimpleFileOptions};

#[test]
fn recovered_project_opens_with_a_warning_in_the_session_log() {
    let mut original = Session::new_project("Recovery", primitives::sphere(1.0, 16, 8), "builtin:sphere", None, 128).unwrap();
    execute(&mut original, "layer.add.fill", &json!({"color": "#336699"})).unwrap();
    let mut json = serde_json::to_value(&original.doc.project).unwrap();
    json["texture_sets"][0]["bake"]["maps"] = json!(["Normal", "AmbientOcclusion"]);
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    archive.start_file("project.json", SimpleFileOptions::default()).unwrap();
    archive.write_all(&serde_json::to_vec(&json).unwrap()).unwrap();
    let bytes = archive.finish().unwrap().into_inner();

    let recovered = Session::from_project_bytes(&bytes).unwrap();
    assert_eq!(recovered.doc.project, original.doc.project);
    let warnings: Vec<_> = recovered.log.iter().filter(|message| message.starts_with("Warning:")).collect();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("could not be imported"));
    assert!(warnings[0].contains("recovered what it could"));

    let reopened = Session::from_project_bytes(&recovered.to_project_bytes().unwrap()).unwrap();
    assert_eq!(reopened.doc.project, original.doc.project);
    assert!(!reopened.log.iter().any(|message| message.starts_with("Warning:")));

    let path = std::env::temp_dir().join(format!("texturecraft-recovery-{}.tcraft", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    execute(&mut original, "file.open", &json!({"path": path})).unwrap();

    // Both entry points report the same recovery messages. The archive timing has
    // a different label for path opens, and elapsed times vary between runs.
    let entries = |log: &texturecraft_engine::SessionLog| {
        log.iter().filter(|entry| entry.source != "Performance").map(|entry| (entry.level, entry.line())).collect::<Vec<_>>()
    };
    assert_eq!(entries(&original.log), entries(&recovered.log));
    for (log, archive_message) in [(&original.log, "CPU project archive read and decoded in "), (&recovered.log, "CPU project archive decoded in ")] {
        let timings: Vec<_> = log.iter().filter(|entry| entry.source == "Performance").collect();
        assert_eq!(timings.len(), 2);
        assert!(timings[0].message.starts_with(archive_message));
        assert!(timings[1].message.starts_with("CPU project setup:"));
        assert_eq!(log.last().map(|entry| entry.level), Some(LogLevel::Warning), "the recovery warning stays visible in the status bar");
    }
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    std::fs::remove_file(path).unwrap();
}
