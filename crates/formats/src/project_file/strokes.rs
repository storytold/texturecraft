//! Stroke record entries (`strokes/<id>.bin`): one per [`StrokeRecord`], keyed by the id that keys the pixels it
//! belongs to.
//!
//! ```text
//! "TCSR"  u32 LE record version (texturecraft_core::strokes::STROKE_RECORD_VERSION)
//! the record in postcard's layout (serde field order; floats keep their bits; a graph source as its JSON text)
//! ```
//!
//! Reading never stops a load: an entry that is damaged or of another version becomes [`StoredStrokes::Unreadable`]
//! with its bytes, written back unchanged on save. Older builds ignore these entries (they skip entries they do not
//! know), so a project with stroke records still opens there, without them.

use std::sync::Arc;

use texturecraft_core::strokes::{STROKE_RECORD_VERSION, StrokeRecord, UnreadableStrokes};
use texturecraft_core::{LayerId, StoredStrokes};

const MAGIC: &[u8; 4] = b"TCSR";
const HEADER: usize = 8;
/// Directory of the entries in the archive.
pub const DIR: &str = "strokes/";
/// Largest stroke record entry.
pub const MAX_ENTRY: u64 = 256 << 20;

/// The entry name of the record keyed by `id`.
pub fn entry_name(id: LayerId) -> String {
    format!("{DIR}{}.bin", id.0)
}

/// The id an entry name keys, if it is a stroke record entry's.
pub fn entry_id(name: &str) -> Option<Option<LayerId>> {
    let rest = name.strip_prefix(DIR)?;
    Some(rest.strip_suffix(".bin").and_then(|n| n.parse().ok()).map(LayerId))
}

/// The entry bytes of `record`.
pub fn encode(record: &StrokeRecord) -> Result<Vec<u8>, String> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&STROKE_RECORD_VERSION.to_le_bytes());
    postcard::to_extend(record, out).map_err(|e| format!("stroke record: {e}"))
}

/// The bytes written for a stored record: its encoding, or the kept bytes of an unreadable one.
pub fn entry_bytes(stored: &StoredStrokes) -> Result<Arc<Vec<u8>>, String> {
    match stored {
        StoredStrokes::Record(r) => encode(r).map(Arc::new),
        StoredStrokes::Unreadable(u) => Ok(Arc::clone(&u.bytes)),
    }
}

/// Reads an entry; the error says why it is not a record this build reads.
pub fn decode(bytes: &[u8]) -> Result<StrokeRecord, String> {
    let (head, body) = bytes.split_at_checked(HEADER).ok_or("not a stroke record (cut short)")?;
    let (magic, version) = head.split_at(MAGIC.len());
    if magic != MAGIC {
        return Err("not a stroke record".into());
    }
    let version = u32::from_le_bytes(version.try_into().map_err(|_| "not a stroke record (cut short)")?);
    if version != STROKE_RECORD_VERSION {
        return Err(format!("stroke record version {version}; this build reads version {STROKE_RECORD_VERSION}"));
    }
    let (record, rest) = postcard::take_from_bytes::<StrokeRecord>(body).map_err(|e| format!("damaged stroke record: {e}"))?;
    if !rest.is_empty() {
        return Err(format!("{} bytes after the stroke record", rest.len()));
    }
    Ok(record)
}

/// An entry as stored: the record, or its bytes with why they could not be read.
pub fn read(bytes: Vec<u8>) -> StoredStrokes {
    match decode(&bytes) {
        Ok(record) => record.into(),
        Err(reason) => StoredStrokes::Unreadable(UnreadableStrokes { bytes: Arc::new(bytes), reason }),
    }
}
