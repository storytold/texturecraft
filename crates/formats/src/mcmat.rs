//! MaterialCraft portable materials (`.mcmat`, read only): a zip with `material.json` (the
//! manifest: maps by usage, exposed parameters, image inputs, presets), `maps/<usage>.png`
//! (8-bit sRGB for color usages, 16-bit linear otherwise; OpenGL normals) and the source
//! `.mcraft` package. TextureCraft keeps its own reader (the apps share the format, not code).
//!
//! Also the 16-bit PNG encoder that hands baked mesh maps to MaterialCraft as graph inputs.

use std::io::{Cursor, Read};

use serde::Deserialize;
use texturecraft_core::{LinkParam, LinkPreset};

use crate::{FormatError, Result};

/// The portable material extension.
pub const MCMAT_EXT: &str = "mcmat";
/// MaterialCraft's package extension.
pub const MCRAFT_EXT: &str = "mcraft";
const FORMAT: &str = "craft-material";
const VERSION: u32 = 1;
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

fn read_entry(name: &str, entry: impl Read, total_bytes: &mut u64) -> Result<Vec<u8>> {
    // Earlier maps remain live while the next entry is read
    let remaining_bytes = MAX_TOTAL.saturating_sub(*total_bytes);
    let entry_limit = MAX_ENTRY.min(remaining_bytes);
    let mut bytes = Vec::new();
    entry.take(entry_limit + 1).read_to_end(&mut bytes).map_err(|error| malformed(format!("{name}: {error}")))?;
    if bytes.len() as u64 > MAX_ENTRY {
        return Err(FormatError::TooLarge(name.to_string()));
    }

    let next_total = total_bytes.saturating_add(bytes.len() as u64);
    if next_total > MAX_TOTAL {
        return Err(FormatError::TooLarge(format!("{KIND} payload (over {MAX_TOTAL} bytes)")));
    }

    // Keep spare read capacity from accumulating across retained map entries
    bytes.shrink_to_fit();
    *total_bytes = next_total;

    Ok(bytes)
}

/// Read `.mcmat` bytes.
pub fn read_mcmat(bytes: &[u8]) -> Result<PortableMaterial> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| malformed(error.to_string()))?;
    let mut total_bytes = 0u64;
    let mut read = |name: &str| -> Result<Vec<u8>> {
        let entry = archive.by_name(name).map_err(|_| malformed(format!("{name} is missing")))?;
        read_entry(name, entry, &mut total_bytes)
    };

    let manifest: Manifest = serde_json::from_slice(&read("material.json")?).map_err(|error| malformed(format!("material.json: {error}")))?;
    if manifest.format != FORMAT {
        return Err(malformed(format!("format `{}`", manifest.format)));
    }

    if manifest.version > VERSION {
        return Err(FormatError::Unsupported(format!("{KIND} version {} (this version reads up to {VERSION})", manifest.version)));
    }

    if manifest.maps.len() > MAX_MAPS {
        return Err(malformed(format!("{} maps, over the {MAX_MAPS} map limit", manifest.maps.len())));
    }

    let maps = manifest.maps.iter().map(|map| Ok((map.usage.clone(), read(&map.file)?))).collect::<Result<Vec<_>>>()?;
    let source = read(&manifest.source)?;

    Ok(PortableMaterial { manifest, maps, source })
}

/// Whether `bytes` are a zip holding `entry` (`material.json`: a portable material,
/// `package.json`: a MaterialCraft package).
fn zip_has(bytes: &[u8], entry: &str) -> bool {
    zip::ZipArchive::new(Cursor::new(bytes)).is_ok_and(|mut z| z.by_name(entry).is_ok())
}

pub fn is_mcmat(bytes: &[u8]) -> bool {
    zip_has(bytes, "material.json")
}

pub fn is_mcraft(bytes: &[u8]) -> bool {
    zip_has(bytes, "package.json")
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
        let manifest = serde_json::json!({
            "format": "craft-material", "version": version, "name": "Brick wall", "graph": "Brick_wall", "size": [4, 4],
            "maps": [{"usage": "baseColor", "file": "maps/baseColor.png", "colorSpace": "srgb"}],
            "parameters": [{"id": "wear", "label": "Wear", "default": {"t": "float", "v": 0.5}, "min": 0.0, "max": 1.0, "value": {"t": "float", "v": 0.25}}],
            "inputs": [{"identifier": "curvature", "kind": "gray"}],
            "presets": [{"name": "Worn", "values": {"wear": {"t": "float", "v": 1.0}}}],
            "source": "source.mcraft", "futureField": true
        });
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("material.json", o).unwrap();
        z.write_all(manifest.to_string().as_bytes()).unwrap();
        z.start_file("maps/baseColor.png", o).unwrap();
        z.write_all(&mesh_map_png(4, 3, &[0.5; 48]).unwrap()).unwrap();
        z.start_file("source.mcraft", o).unwrap();
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
        assert!(matches!(read_mcmat(&sample(2)), Err(FormatError::Unsupported(_))));
        assert!(read_mcmat(b"junk").is_err());
    }

    #[test]
    fn an_unbounded_map_list_is_refused() {
        let maps: Vec<_> = (0..=MAX_MAPS).map(|_| serde_json::json!({"usage": "baseColor", "file": "maps/baseColor.png"})).collect();
        let manifest =
            serde_json::json!({"format": FORMAT, "version": VERSION, "name": "n", "graph": "g", "size": [4, 4], "maps": maps, "source": "source.mcraft"});
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
    fn entry_read_stops_at_the_remaining_payload_budget() {
        let payload = vec![7u8; 1024];
        let mut input = Cursor::new(&payload);
        let mut total_bytes = MAX_TOTAL - 8;

        let result = read_entry("maps/extra.png", &mut input, &mut total_bytes);

        assert!(matches!(result, Err(FormatError::TooLarge(_))));
        assert_eq!(input.position(), 9, "only one byte beyond the remaining budget is needed to reject the entry");
        assert_eq!(total_bytes, MAX_TOTAL - 8, "a rejected entry does not consume the payload budget");
    }

    #[test]
    fn payload_budget_accepts_the_exact_limit_and_empty_entries() {
        let mut total_bytes = MAX_TOTAL - 4;

        let bytes = read_entry("map.png", Cursor::new([1, 2, 3, 4]), &mut total_bytes).unwrap();

        assert_eq!(bytes, [1, 2, 3, 4]);
        assert_eq!(total_bytes, MAX_TOTAL);
        assert!(read_entry("empty.mcraft", Cursor::new([]), &mut total_bytes).unwrap().is_empty());
        assert!(matches!(read_entry("extra.png", Cursor::new([1]), &mut total_bytes), Err(FormatError::TooLarge(_))));
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
