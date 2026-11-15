//! MaterialCraft portable materials (`.mcmat`, read only): a zip with `material.json` (the
//! manifest: maps by usage, exposed parameters, image inputs, presets), `maps/<usage>.png`
//! (8-bit sRGB for color usages, 16-bit linear otherwise; OpenGL normals) and the
//! source package. The shared reader handles the portable container and manifest.
//!
//! Also the 16-bit PNG encoder that hands baked mesh maps to MaterialCraft as graph inputs.

use std::io::Cursor;

use serde::Deserialize;
use texturecraft_core::{LinkParam, LinkPreset};

use crate::{FormatError, Result};

/// The portable material extension.
pub const MCMAT_EXT: &str = "mcmat";
/// MaterialCraft's package extension.
pub const MCRAFT_EXT: &str = "mcraft";
const VERSION: u32 = 2;
/// Largest entry read (maps are at most 8192² × 16 bits, compressed).
const MAX_ENTRY: u64 = 512 << 20;
/// Most map entries one manifest may name (a material has at most nine usages).
const MAX_MAPS: usize = 64;
/// Largest total payload read from one `.mcmat`: maps and source together (audit: TextureCraft #4).
const MAX_TOTAL: u64 = 1 << 30;
const KIND: &str = "MaterialCraft material";

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub name: String,
    pub graph: String,
    pub size: [u32; 2],
    pub maps: Vec<MapEntry>,
    #[serde(default)]
    pub parameters: Vec<LinkParam>,
    #[serde(default)]
    pub inputs: Vec<ImageInput>,
    #[serde(default)]
    pub presets: Vec<LinkPreset>,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapEntry {
    /// `baseColor`, `normal`, `roughness`, `metallic`, `height`, `ambientOcclusion`, `emissive`,
    /// `opacity` or `specular`.
    pub usage: String,
    pub file: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ImageInput {
    pub identifier: String,
}

/// A read `.mcmat`: the manifest, each map's PNG bytes by usage, the source package.
#[derive(Clone, Debug)]
pub struct PortableMaterial {
    pub manifest: Manifest,
    pub maps: Vec<(String, Vec<u8>)>,
    pub source: Vec<u8>,
}

fn malformed(msg: impl Into<String>) -> FormatError {
    FormatError::Malformed { kind: KIND, msg: msg.into() }
}

fn shared_error(error: craft_mcmat::Error) -> FormatError {
    match error {
        craft_mcmat::Error::Version(version) => FormatError::Unsupported(format!("{KIND} version {version} (this version reads up to {VERSION})")),
        craft_mcmat::Error::Limit(message) => malformed(message),
        craft_mcmat::Error::Archive(ref archive) if archive.is_limit() => FormatError::TooLarge(error.to_string()),
        other => malformed(other.to_string()),
    }
}

/// Read `.mcmat` bytes within TextureCraft's existing per-entry and total payload limits.
pub fn read_mcmat(bytes: &[u8]) -> Result<PortableMaterial> {
    let limits = craft_mcmat::Limits {
        min_version: 0,
        max_version: VERSION,
        max_entries: 100_000,
        max_maps: MAX_MAPS,
        max_manifest_bytes: MAX_ENTRY,
        max_entry_bytes: MAX_ENTRY,
        max_total_bytes: MAX_TOTAL,
    };
    let mut archive = craft_mcmat::Material::open(bytes, limits).map_err(shared_error)?;
    // Consume the JSON values rather than cloning the entire untrusted manifest into a second tree.
    let parameters = std::mem::take(&mut archive.manifest.parameters)
        .into_iter()
        .map(serde_json::from_value)
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| malformed(format!("material.json: {e}")))?;
    let inputs = std::mem::take(&mut archive.manifest.inputs)
        .into_iter()
        .map(serde_json::from_value)
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| malformed(format!("material.json: {e}")))?;
    let presets = std::mem::take(&mut archive.manifest.presets)
        .into_iter()
        .map(serde_json::from_value)
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| malformed(format!("material.json: {e}")))?;
    let shared = &archive.manifest;
    let manifest = Manifest {
        format: shared.format.clone(),
        version: shared.version,
        name: shared.name.clone(),
        graph: shared.graph.clone(),
        size: shared.size,
        maps: shared.maps.iter().map(|m| MapEntry { usage: m.usage.clone(), file: m.file.clone() }).collect(),
        parameters,
        inputs,
        presets,
        source: shared.source.clone(),
    };
    let maps = manifest.maps.iter().map(|map| Ok((map.usage.clone(), archive.read(&map.file).map_err(shared_error)?))).collect::<Result<Vec<_>>>()?;
    let source = archive.read_source().map_err(shared_error)?;
    Ok(PortableMaterial { manifest, maps, source })
}

/// Whether `bytes` contain a portable material manifest within the ZIP entry limit.
pub fn is_mcmat(bytes: &[u8]) -> bool {
    craft_mcmat::is_material(bytes, 100_000)
}

pub fn is_mcraft(bytes: &[u8]) -> bool {
    let Ok(mut archive) = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: 100_000, max_total_bytes: 0 }) else { return false };
    // A portable material may itself contain source.sbs, so it is not a source archive.
    if archive.has_duplicate_names() || archive.contains("material.json") {
        return false;
    }
    if archive.contains("package.json") {
        return true;
    }
    (0..archive.len()).any(|i| archive.entry_name(i).ok().flatten().is_some_and(|name| !name.contains('/') && name.to_ascii_lowercase().ends_with(".sbs")))
}

/// A baked mesh map (`components` floats per texel, 0..1, rows top first) as a 16-bit PNG
/// (grayscale for one component, RGB for three).
pub fn mesh_map_png(width: u32, components: usize, data: &[f32]) -> Result<Vec<u8>> {
    let per_row = (width as usize).saturating_mul(components);
    let height = data.len().checked_div(per_row).unwrap_or(0) as u32;
    if height == 0 || !matches!(components, 1 | 3) {
        return Err(FormatError::Invalid(format!("mesh map of {} values, {width} wide, {components} components", data.len())));
    }
    let px: Vec<u16> = data.iter().take(per_row * height as usize).map(|v| (v.clamp(0.0, 1.0) * 65535.0).round() as u16).collect();
    let img = if components == 1 {
        image::ImageBuffer::<image::Luma<u16>, _>::from_raw(width, height, px).map(image::DynamicImage::ImageLuma16)
    } else {
        image::ImageBuffer::<image::Rgb<u16>, _>::from_raw(width, height, px).map(image::DynamicImage::ImageRgb16)
    }
    .ok_or_else(|| FormatError::Invalid("mesh map size".into()))?;
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).map_err(|e| FormatError::Invalid(e.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A `.mcmat` as MaterialCraft writes it (see its `formats/src/material.rs`).
    pub(crate) fn sample(version: u32) -> Vec<u8> {
        let source = if version >= 2 { "source.sbs" } else { "source.mcraft" };
        let manifest = serde_json::json!({
            "format": "craft-material", "version": version, "name": "Brick wall", "graph": "Brick_wall", "size": [4, 4],
            "maps": [{"usage": "baseColor", "file": "maps/baseColor.png", "colorSpace": "srgb"}],
            "parameters": [{"id": "wear", "label": "Wear", "default": {"t": "float", "v": 0.5}, "min": 0.0, "max": 1.0, "value": {"t": "float", "v": 0.25}}],
            "inputs": [{"identifier": "curvature", "kind": "gray"}],
            "presets": [{"name": "Worn", "values": {"wear": {"t": "float", "v": 1.0}}}],
            "source": source, "futureField": true
        });
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("material.json", o).unwrap();
        z.write_all(manifest.to_string().as_bytes()).unwrap();
        z.start_file("maps/baseColor.png", o).unwrap();
        z.write_all(&mesh_map_png(4, 3, &[0.5; 48]).unwrap()).unwrap();
        z.start_file(source, o).unwrap();
        z.write_all(b"pkg").unwrap();
        z.finish().unwrap().into_inner()
    }

    #[test]
    fn reads_manifest_maps_and_source() {
        let bytes = sample(1);

        assert!(is_mcmat(&bytes) && !is_mcraft(&bytes));
        let material = read_mcmat(&bytes).unwrap();

        assert_eq!(material.manifest.graph, "Brick_wall");
        assert_eq!(material.manifest.parameters[0].id, "wear");
        assert_eq!(material.manifest.parameters[0].value, serde_json::json!({"t": "float", "v": 0.25}));
        assert_eq!(material.manifest.presets[0].name, "Worn");
        assert_eq!(material.manifest.inputs[0].identifier, "curvature");
        assert_eq!(material.maps[0].0, "baseColor");
        assert_eq!(crate::decode_image(&material.maps[0].1).unwrap().width, 4);
        assert_eq!(material.source, b"pkg");
        let current = read_mcmat(&sample(2)).unwrap();
        assert_eq!(current.manifest.source, "source.sbs");
        assert_eq!(current.source, b"pkg");
        assert!(matches!(read_mcmat(&sample(3)), Err(FormatError::Unsupported(_))));
        assert!(read_mcmat(b"junk").is_err());
    }

    #[test]
    fn recognizes_both_generations_of_source_archive_without_confusing_portable_materials() {
        for entry in ["package.json", "package.sbs"] {
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            zip.start_file(entry, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(b"source").unwrap();
            let bytes = zip.finish().unwrap().into_inner();
            assert!(is_mcraft(&bytes), "{entry}");
        }
        assert!(!is_mcraft(&sample(2)), "a portable material contains source.sbs too");
    }

    #[test]
    fn an_unbounded_map_list_is_refused() {
        let maps: Vec<_> = (0..=MAX_MAPS).map(|_| serde_json::json!({"usage": "baseColor", "file": "maps/baseColor.png"})).collect();
        let manifest = serde_json::json!({"format": craft_mcmat::FORMAT, "version": VERSION, "name": "n", "graph": "g", "size": [4, 4], "maps": maps, "source": "source.mcraft"});
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        archive.start_file("material.json", options).unwrap();
        archive.write_all(manifest.to_string().as_bytes()).unwrap();
        archive.start_file("maps/baseColor.png", options).unwrap();
        archive.write_all(&mesh_map_png(4, 3, &[0.5; 48]).unwrap()).unwrap();
        archive.start_file("source.mcraft", options).unwrap();
        archive.write_all(b"pkg").unwrap();

        let result = read_mcmat(&archive.finish().unwrap().into_inner());

        assert!(matches!(result, Err(FormatError::Malformed { .. })));
    }

    #[test]
    fn mesh_maps_encode_as_16_bit() {
        let png = mesh_map_png(2, 1, &[0.0, 0.25, 0.5, 1.0]).unwrap();
        let d = crate::decode_image(&png).unwrap();
        assert_eq!((d.width, d.height), (2, 2));
        assert!((d.rgba[1][0] - 0.25).abs() < 1e-4);
        assert!(mesh_map_png(0, 1, &[]).is_err());
        assert!(mesh_map_png(2, 2, &[0.0; 8]).is_err());
    }
}
