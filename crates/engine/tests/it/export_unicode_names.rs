//! Texture sets named in any alphabet export files of their own

use serde_json::json;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

#[test]
fn texture_sets_with_non_latin_names_of_one_length_each_export_their_files() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let names = ["身体", "头发"];
    for (index, name) in names.iter().enumerate() {
        execute(&mut session, "textureSet.rename", &json!({"ts": index, "name": name})).unwrap();
    }
    let dir = std::env::temp_dir().join(format!("tc-engine-unicode-names-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let result = execute(&mut session, "file.exportTextures", &json!({"dir": dir.display().to_string()})).unwrap();

    let written: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    let of = |name: &str| written.iter().filter(|file| file.contains(name)).count();
    assert!(of(names[0]) > 0, "{written:?} {result}");
    assert_eq!(of(names[0]), of(names[1]), "{written:?} {result}");
    std::fs::remove_dir_all(&dir).unwrap();
}
