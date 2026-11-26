//! `uv_layout.bin` stores the generated UVs independently of the running unwrapper.
//! After the magic `TCUV`, the header stores a version and a corner count as little-endian u32s.
//! The body stores a little-endian f32 pair for each triangle corner in index order.

use std::io::{Read, Write};

use texturecraft_geom::MAX_TRIANGLES;
use texturecraft_geom::glam::Vec2;

use super::KIND;
use crate::{Result, malformed};

/// Zip entry name of the saved layout.
pub const UV_LAYOUT_ENTRY: &str = "uv_layout.bin";

const UV_LAYOUT_MAGIC: &[u8; 4] = b"TCUV";
const UV_LAYOUT_VERSION: u32 = 1;
const HEADER_BYTES: usize = 12;
const COORDINATE_BYTES: usize = 4;
const CORNER_BYTES: usize = 2 * COORDINATE_BYTES;
const BATCH_CORNERS: usize = 1024;
const MAX_LAYOUT_CORNERS: usize = MAX_TRIANGLES * 3;

/// Largest entry a valid mesh can need: every corner of the most triangles accepted.
pub const MAX_UV_LAYOUT_BYTES: u64 = uv_layout_bytes(MAX_LAYOUT_CORNERS);

/// Encoded size of a layout of `corners` UVs.
pub const fn uv_layout_bytes(corners: usize) -> u64 {
    HEADER_BYTES as u64 + corners as u64 * CORNER_BYTES as u64
}

pub fn write_uv_layout(writer: &mut dyn Write, layout: &[Vec2]) -> std::io::Result<()> {
    if layout.len() > MAX_LAYOUT_CORNERS {
        return Err(std::io::Error::other("the UV layout has too many corners"));
    }

    writer.write_all(UV_LAYOUT_MAGIC)?;
    writer.write_all(&UV_LAYOUT_VERSION.to_le_bytes())?;
    writer.write_all(&(layout.len() as u32).to_le_bytes())?;

    let mut buffer = [[0; CORNER_BYTES]; BATCH_CORNERS];
    for batch in layout.chunks(BATCH_CORNERS) {
        let encoded = &mut buffer[..batch.len()];
        for (corner, uv) in encoded.iter_mut().zip(batch) {
            if !uv.is_finite() {
                return Err(std::io::Error::other("the UV layout has non-finite UVs"));
            }

            corner.copy_from_slice([uv.x.to_le_bytes(), uv.y.to_le_bytes()].as_flattened());
        }

        writer.write_all(encoded.as_flattened())?;
    }

    Ok(())
}

/// Decode one entry while holding only the decoded layout and a fixed-size buffer.
pub fn read_uv_layout(reader: &mut impl Read, entry_bytes: u64) -> Result<Vec<Vec2>> {
    let bad = |why: &str| malformed(KIND, format!("{UV_LAYOUT_ENTRY}: {why}"));
    let mut header = [[0; COORDINATE_BYTES]; 3];
    reader.read_exact(header.as_flattened_mut()).map_err(|error| malformed(KIND, format!("{UV_LAYOUT_ENTRY}: {error}")))?;
    let [magic, version, corners] = header;
    if magic != *UV_LAYOUT_MAGIC {
        return Err(bad("bad magic"));
    }

    if u32::from_le_bytes(version) != UV_LAYOUT_VERSION {
        return Err(bad("unsupported version"));
    }

    let corners = u32::from_le_bytes(corners) as usize;
    if corners > MAX_LAYOUT_CORNERS || uv_layout_bytes(corners) != entry_bytes {
        return Err(bad("the corner count does not match the entry size"));
    }

    let mut layout = Vec::with_capacity(corners);
    let mut buffer = [[[0; COORDINATE_BYTES]; 2]; BATCH_CORNERS];
    while layout.len() < corners {
        let count = (corners - layout.len()).min(BATCH_CORNERS);
        let encoded = &mut buffer[..count];
        reader.read_exact(encoded.as_flattened_mut().as_flattened_mut()).map_err(|error| malformed(KIND, format!("{UV_LAYOUT_ENTRY}: {error}")))?;
        for &[u, v] in encoded.iter() {
            let uv = Vec2::new(f32::from_le_bytes(u), f32::from_le_bytes(v));
            if !uv.is_finite() {
                return Err(bad("non-finite UVs"));
            }

            layout.push(uv);
        }
    }

    // Read through the end of the ZIP entry so its checksum is checked
    match reader.read_exact(&mut [0]) {
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(layout),
        Err(error) => Err(malformed(KIND, format!("{UV_LAYOUT_ENTRY}: {error}"))),
        Ok(()) => Err(bad("the corner count does not match the entry size")),
    }
}
