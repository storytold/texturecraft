//! Validate a procedural MCMAT and encode caller-supplied mesh maps for MaterialCraft.
use crate::{FormatError, Result};
use std::io::Write;

mod version_marker;
use version_marker::VersionMarker;

pub const MCMAT_EXT: &str = "mcmat";
pub const MCRAFT_EXT: &str = "mcraft";

/// Diagnose old versions before starting the CLI. The shared reader in MaterialCraft
/// validates and extracts the full archive.
pub fn validate_mcmat_version(bytes: &[u8], manifest_budget_bytes: u64) -> Result<()> {
    let limits = craft_mcmat::Limits::default();
    let mut archive = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: limits.max_entries, max_total_bytes: manifest_budget_bytes })
        .map_err(mcmat_archive_error)?;
    let bytes = archive
        .read(craft_mcmat::MANIFEST_ENTRY, limits.max_manifest_bytes.min(manifest_budget_bytes))
        .map_err(mcmat_archive_error)?
        .ok_or_else(|| mcmat_invalid("missing material.json"))?;

    // A marker must be an object even though Serde also accepts positional struct fields
    if !bytes.trim_ascii_start().starts_with(b"{") {
        return Err(mcmat_invalid("expected a version object"));
    }

    let marker: VersionMarker = serde_json::from_slice(&bytes).map_err(mcmat_invalid)?;
    if marker.version != craft_mcmat::VERSION {
        return Err(FormatError::Unsupported(craft_mcmat::Error::Version(marker.version).to_string()));
    }

    Ok(())
}

fn mcmat_invalid(error: impl std::fmt::Display) -> FormatError {
    FormatError::Malformed { kind: "MaterialCraft material", msg: error.to_string() }
}

fn mcmat_archive_error(error: craft_zip::Error) -> FormatError {
    if error.is_limit() {
        return FormatError::TooLarge(error.to_string());
    }

    mcmat_invalid(error)
}

/// Whether `bytes` contain a portable material manifest within the ZIP entry limit.
pub fn is_mcmat(bytes: &[u8]) -> bool {
    craft_mcmat::is_material(bytes, craft_mcmat::Limits::default().max_entries)
}

pub fn is_mcraft(bytes: &[u8]) -> bool {
    let limits = craft_zip::Limits { max_entries: craft_mcmat::Limits::default().max_entries, max_total_bytes: 0 };
    let Ok(mut archive) = craft_zip::Archive::open(bytes, limits) else {
        return false;
    };

    // A portable material may contain source.sbs inside its envelope
    if archive.has_duplicate_names() || archive.contains(craft_mcmat::MANIFEST_ENTRY) {
        return false;
    }

    if archive.contains("package.json") {
        return true;
    }

    (0..archive.len()).any(|index| archive.entry_name(index).ok().flatten().as_deref().is_some_and(is_root_sbs))
}

pub(crate) fn is_root_sbs(name: &str) -> bool {
    !name.contains('/') && name.rsplit_once('.').is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("sbs"))
}

/// A baked mesh map (`components` floats per texel, 0..1, rows top first) as a 16-bit PNG
/// (grayscale for one component, RGB for three).
pub fn mesh_map_png(width: u32, components: usize, data: &[f32]) -> Result<Vec<u8>> {
    let mut encoded = Vec::new();
    write_mesh_map_png(&mut encoded, width, components, data)?;
    Ok(encoded)
}

/// Estimated temporary workspace for `write_mesh_map_png` with a file destination.
pub fn mesh_map_png_working_bytes(width_px: u32, height_px: u32, components: usize) -> u64 {
    // PngRows in the pinned codec deflates bands of up to 512 KiB filtered bytes,
    // or one scanline for wider images.
    const DEFLATE_BAND_BYTES: u64 = 512 << 10;
    const DEFLATE_WORKSPACE_BYTES: u64 = 1 << 20;

    let row_bytes = u64::from(width_px).saturating_mul(components as u64).saturating_mul(std::mem::size_of::<u16>() as u64);
    let filtered_row_bytes = row_bytes.saturating_add(1);
    let band_rows = (DEFLATE_BAND_BYTES / filtered_row_bytes).max(1).min(u64::from(height_px));
    let pending = row_bytes.saturating_mul(band_rows).saturating_mul(2);
    let filtered = filtered_row_bytes.saturating_mul(band_rows);
    let deflated = filtered.saturating_add(filtered / 8).saturating_add(1_024);
    // Include spare row capacity and keep both the deflate output and the
    // assembled zlib buffer in the estimate. Input floats are already charged.
    let row_buffers = row_bytes.saturating_mul(8);
    row_buffers.saturating_add(pending).saturating_add(filtered).saturating_add(deflated.saturating_mul(2)).saturating_add(DEFLATE_WORKSPACE_BYTES)
}

/// Encode a mesh map directly into its destination using one row of quantized samples.
pub fn write_mesh_map_png(output: impl Write, width_px: u32, components: usize, data: &[f32]) -> Result<()> {
    let invalid_size = || FormatError::Invalid(format!("mesh map of {} values, {width_px} wide, {components} components", data.len()));
    let layout = match components {
        1 => craft_image_codecs::ChannelLayout::Gray,
        3 => craft_image_codecs::ChannelLayout::Rgb,
        _ => return Err(invalid_size()),
    };
    let samples_per_row = (width_px as usize).checked_mul(components).filter(|&count| count != 0).ok_or_else(invalid_size)?;
    if data.is_empty() || !data.len().is_multiple_of(samples_per_row) {
        return Err(invalid_size());
    }

    let height_px = u32::try_from(data.len() / samples_per_row).map_err(|_| invalid_size())?;
    let mut scanline = Vec::new();
    scanline.try_reserve_exact(samples_per_row).map_err(|error| FormatError::TooLarge(error.to_string()))?;

    let mut opts = craft_image_codecs::EncodeOptions::default();
    opts.png_compression = craft_image_codecs::PngCompression::Fast;
    opts.embed_icc = false;
    opts.embed_metadata = false;

    let codec_err = |error: craft_image_codecs::CodecError| FormatError::Invalid(error.to_string());
    let mut rows = data.chunks_exact(samples_per_row);
    let first_row = rows.next().ok_or_else(invalid_size)?;
    scanline.extend(first_row.iter().map(|value| (value.clamp(0.0, 1.0) * f32::from(u16::MAX)).round() as u16));
    let first_band = craft_image_codecs::Image::from_u16(width_px, 1, layout, &scanline).map_err(codec_err)?;
    let mut writer = craft_image_codecs::PngRows::new(output, &first_band, height_px, &opts).map_err(codec_err)?;
    writer.push(&first_band).map_err(codec_err)?;
    for row in rows {
        scanline.clear();
        scanline.extend(row.iter().map(|value| (value.clamp(0.0, 1.0) * f32::from(u16::MAX)).round() as u16));
        let band = craft_image_codecs::Image::from_u16(width_px, 1, layout, &scanline).map_err(codec_err)?;
        writer.push(&band).map_err(codec_err)?;
    }
    writer.finish().map_err(codec_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn mesh_map_png_budget_covers_live_band_allocations() {
        // The pinned encoder holds 85 rows for a 1024px RGB16 map. These
        // three reserves coexist while deflating, even for compressible data.
        const PENDING_BYTES: u64 = 522_240;
        const FILTERED_BYTES: u64 = 522_325;
        const DEFLATE_OUTPUT_BYTES: u64 = 588_639;
        let unavoidable_workspace = PENDING_BYTES + FILTERED_BYTES + DEFLATE_OUTPUT_BYTES;
        let estimate = mesh_map_png_working_bytes(1_024, 1_024, 3);
        assert!(estimate >= unavoidable_workspace, "estimated {estimate} bytes for at least {unavoidable_workspace} bytes of workspace");
        assert!(mesh_map_png_working_bytes(1_024, 1, 3) < estimate);
        assert_eq!(mesh_map_png_working_bytes(u32::MAX, u32::MAX, usize::MAX), u64::MAX);
    }

    #[test]
    fn recognizes_v3_and_directs_older_authors_to_republish() {
        let source = craft_mcmat::write(b"<package/>", []).unwrap();
        assert!(is_mcmat(&source) && !is_mcraft(&source));
        validate_mcmat_version(&source, 1 << 20).unwrap();
        for version in [1, 2] {
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            zip.start_file("material.json", zip::write::SimpleFileOptions::default()).unwrap();
            write!(zip, r#"{{"version":{version}}}"#).unwrap();
            let err = validate_mcmat_version(&zip.finish().unwrap().into_inner(), 1 << 20).unwrap_err();
            assert!(err.to_string().contains("Republish"));
        }
    }

    #[test]
    fn version_marker_requires_one_named_version() {
        for marker in [r#"{"version":2,"version":3}"#, "[3]"] {
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            zip.start_file(craft_mcmat::MANIFEST_ENTRY, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(marker.as_bytes()).unwrap();
            let bytes = zip.finish().unwrap().into_inner();

            assert!(validate_mcmat_version(&bytes, 1 << 20).is_err());
        }
    }

    #[test]
    fn mesh_maps_encode_as_16_bit() {
        let png = mesh_map_png(2, 1, &[0.0, 0.25, 0.5, 1.0]).unwrap();
        let decoded = crate::decode_image(&png).unwrap();

        assert_eq!((decoded.width, decoded.height), (2, 2));
        assert!((decoded.rgba[1][0] - 0.25).abs() < 1e-4);
        assert!(mesh_map_png(2, 1, &[0.0; 3]).is_err());
        assert!(mesh_map_png(0, 1, &[]).is_err());
        assert!(mesh_map_png(2, 2, &[0.0; 8]).is_err());
    }

    #[test]
    fn mesh_maps_encode_gray16_and_rgb16_with_exact_png_headers() {
        let decode_raw_16 = |bytes: &[u8]| {
            let decoder = png::Decoder::new(Cursor::new(bytes));
            let mut reader = decoder.read_info().unwrap();
            let info = reader.info().clone();
            let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
            let frame = reader.next_frame(&mut buf).unwrap();
            let samples: Vec<u16> = buf[..frame.buffer_size()].as_chunks::<2>().0.iter().copied().map(u16::from_be_bytes).collect();
            (info, samples)
        };

        let gray = mesh_map_png(2, 1, &[-0.5, 0.25, 0.5, 1.5]).unwrap();
        let (gray_info, gray_samples) = decode_raw_16(&gray);
        assert_eq!(gray_info.bit_depth, png::BitDepth::Sixteen);
        assert_eq!(gray_info.color_type, png::ColorType::Grayscale);
        assert_eq!(gray_samples, [0, 16384, 32768, 65535]);

        let rgb = mesh_map_png(2, 3, &[0.0, 0.25, 0.5, 1.0, 0.75, 0.125, 0.5, 0.5, 0.5, 0.0, 1.0, 0.0]).unwrap();
        let (rgb_info, rgb_samples) = decode_raw_16(&rgb);
        assert_eq!(rgb_info.bit_depth, png::BitDepth::Sixteen);
        assert_eq!(rgb_info.color_type, png::ColorType::Rgb);
        assert_eq!(rgb_samples, [0, 16384, 32768, 65535, 49151, 8192, 32768, 32768, 32768, 0, 65535, 0]);
    }

    #[test]
    fn write_mesh_map_png_propagates_writer_errors() {
        struct FailingWriter;
        impl Write for FailingWriter {
            fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "disk full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let error = write_mesh_map_png(FailingWriter, 2, 1, &[0.0, 0.25, 0.5, 1.0]).unwrap_err();
        assert!(matches!(error, FormatError::Invalid(_)), "{error}");
    }
}
