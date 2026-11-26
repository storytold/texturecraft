//! [`MeshRequest`]: the high-poly and cage mesh files the original baked a set with. The file records only their paths
//! on the machine it was made on, and the import reads nothing but the file, so the meshes are not brought in: the
//! report names them, with the move that puts a mesh modelled next to the original into the imported mesh's space
//! (the original stores its mesh normalised, see [`Normalization`]).

use texturecraft_import_reader::decode::assbin::Normalization;
use texturecraft_import_reader::model::baking::MaterialBaking;

use super::flag;

/// Most file paths read from one setting.
const MAX_FILES: usize = 64;

/// The mesh files a set was baked from, as the file records them (paths on the machine the original was made on).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeshRequest {
    pub high_poly: Vec<String>,
    /// The cage mesh, when the original baked with one.
    pub cage: Option<String>,
}

impl MeshRequest {
    pub fn of(baking: &MaterialBaking) -> MeshRequest {
        let paths = |key| baking.detail.text(key).map(file_paths).unwrap_or_default();
        let use_cage = baking.detail.get("Detail.UseCage").and_then(flag) == Some(true);
        MeshRequest { high_poly: paths("Detail.HipolyMesh"), cage: use_cage.then(|| paths("Detail.CageMesh").into_iter().next()).flatten() }
    }

    pub fn is_empty(&self) -> bool {
        self.high_poly.is_empty() && self.cage.is_none()
    }

    /// The report note: which files, and how a mesh modelled next to the original lines up with the imported one.
    pub fn note(&self, normalization: Option<Normalization>) -> String {
        let named = |paths: &[String]| paths.iter().map(|p| format!("\"{}\" (recorded as {p})", file_name(p))).collect::<Vec<_>>().join(", ");
        let mut parts = Vec::new();
        if !self.high_poly.is_empty() {
            parts.push(format!("baked from the high-poly mesh {}", named(&self.high_poly)));
        }
        if let Some(cage) = &self.cage {
            parts.push(format!("with the cage {}", named(std::slice::from_ref(cage))));
        }
        let mut note = format!("{}; mesh files are not part of the project file: add them to the bake settings (high-poly meshes, cage file)", parts.join(" "));
        if let Some(Normalization { center: [x, y, z], radius }) = normalization {
            note += &format!(
                "; the imported mesh is the original's normalised copy, so a mesh modelled next to the original lines up after moving each point p to (p - ({x}, {y}, {z})) / {radius}"
            );
        }
        note
    }
}

/// The last part of a path with `/` or `\` separators.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The paths in a list of file URLs (`file:///a/b.obj`, several separated by `|` or `;`), without the scheme and the
/// slash before a drive letter, `%20` as a space; entries without a file name are dropped.
fn file_paths(urls: &str) -> Vec<String> {
    urls.split(['|', ';'])
        .map(str::trim)
        .map(|u| u.strip_prefix("file://").unwrap_or(u))
        .map(|u| u.trim_end_matches(['/', '\\']).replace("%20", " "))
        .map(|u| match u.strip_prefix('/') {
            Some(rest) if is_drive_path(rest) => rest.to_string(),
            _ => u,
        })
        .filter(|u| !file_name(u).is_empty())
        .take(MAX_FILES)
        .collect()
}

/// `D:/…` or `D:\…`.
fn is_drive_path(p: &str) -> bool {
    matches!(p.as_bytes(), [d, b':', b'/' | b'\\', ..] if d.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_import_reader::model::baking::{TweakList, TweakValue};

    #[test]
    fn file_urls_become_paths() {
        assert_eq!(file_paths("file:///D:/Users/x/Vase/Vase HP.OBJ"), ["D:/Users/x/Vase/Vase HP.OBJ"]);
        assert_eq!(file_paths("file:///C:/a.fbx|file:///C:/b%20c.obj"), ["C:/a.fbx", "C:/b c.obj"]);
        assert_eq!(file_paths("file:///home/u/h.obj; file:///home/u/dir/"), ["/home/u/h.obj", "/home/u/dir"]);
        assert!(file_paths("").is_empty() && file_paths("file:///").is_empty());
        assert_eq!(file_paths(&"file:///a.obj|".repeat(1000)).len(), MAX_FILES);
    }

    #[test]
    fn the_request_names_the_files_and_the_move() {
        let detail = |tweaks: Vec<(&str, TweakValue)>| MaterialBaking {
            detail: TweakList { baker: String::new(), tweaks: tweaks.into_iter().map(|(k, v)| (k.to_string(), v)).collect() },
            ..MaterialBaking::default()
        };
        let text = |s: &str| TweakValue::Text(s.into());
        let none = MeshRequest::of(&detail(vec![
            ("Detail.HipolyMesh", text("")),
            ("Detail.UseCage", TweakValue::Bool(false)),
            ("Detail.CageMesh", text("file:///c.obj")),
        ]));
        assert!(none.is_empty(), "{none:?}");
        let request = MeshRequest::of(&detail(vec![
            ("Detail.HipolyMesh", text("file:///D:/m/Ring HP.obj")),
            ("Detail.UseCage", TweakValue::Int(1)),
            ("Detail.CageMesh", text("file:///D:/m/Ring cage.obj")),
        ]));
        assert_eq!(request, MeshRequest { high_poly: vec!["D:/m/Ring HP.obj".into()], cage: Some("D:/m/Ring cage.obj".into()) });
        let note = request.note(Some(Normalization { center: [1.0, 2.0, 3.0], radius: 4.0 }));
        assert!(note.starts_with("baked from the high-poly mesh \"Ring HP.obj\" (recorded as D:/m/Ring HP.obj) with the cage \"Ring cage.obj\""), "{note}");
        assert!(note.ends_with("(p - (1, 2, 3)) / 4"), "{note}");
        assert!(!request.note(None).contains("normalised"));
    }
}
