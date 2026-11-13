//! Stroke record entries (`strokes/<id>.bin`): one per [`StrokeRecord`], keyed by the id that keys the pixels it
//! belongs to; and the grey images the records lay (`stroke_images/<key>.bin`), each kept once per project however many
//! records and stamps lay it.
//!
//! ```text
//! strokes/<id>.bin
//!   "TCSR"  u32 LE record version (texturecraft_core::strokes::STROKE_RECORD_VERSION)
//!   version 2: the image keys (postcard: a list of strings, one per image of StrokeRecord::images, in its order),
//!              then the record in postcard's layout with every image's values left empty
//!   version 1: the record in postcard's layout, its images inside it
//!   (postcard: serde field order; floats keep their bits; a graph source as its JSON text)
//! stroke_images/<key>.bin
//!   "TCSI"  u32 LE image version (1)  the values, f32 LE; <key> is the StableHash of those bytes
//! ```
//!
//! Reading never stops a load: an entry that is damaged, of another version, or naming an image the project does not
//! hold becomes [`StoredStrokes::Unreadable`] with its bytes, written back unchanged on save. Older builds ignore these
//! entries (they skip entries they do not know), so a project with stroke records still opens there, without them.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use texturecraft_core::strokes::{STROKE_RECORD_VERSION, STROKE_RECORD_VERSION_INLINE, StrokeRecord, UnreadableStrokes};
use texturecraft_core::{LayerId, StableHash, StoredStrokes};

const MAGIC: &[u8; 4] = b"TCSR";
const IMAGE_MAGIC: &[u8; 4] = b"TCSI";
const IMAGE_VERSION: u32 = 1;
const HEADER: usize = 8;
/// Directory of the entries in the archive.
pub const DIR: &str = "strokes/";
/// Directory of the images records lay.
pub const IMAGE_DIR: &str = "stroke_images/";
/// Largest stroke record entry.
pub const MAX_ENTRY: u64 = 256 << 20;
/// Largest stroke image entry (a 2048² stencil is 16 MiB).
pub const MAX_IMAGE_ENTRY: u64 = 64 << 20;

/// The entry name of the record keyed by `id`.
pub fn entry_name(id: LayerId) -> String {
    format!("{DIR}{}.bin", id.0)
}

/// The id an entry name keys, if it is a stroke record entry's.
pub fn entry_id(name: &str) -> Option<Option<LayerId>> {
    let rest = name.strip_prefix(DIR)?;
    Some(rest.strip_suffix(".bin").and_then(|n| n.parse().ok()).map(LayerId))
}

/// The entry name of the image under `key`.
pub fn image_entry_name(key: &str) -> String {
    format!("{IMAGE_DIR}{key}.bin")
}

/// The key an entry name holds, if it is a stroke image entry's (`Some(None)`: in the directory, but not a key).
pub fn image_entry_key(name: &str) -> Option<Option<&str>> {
    let rest = name.strip_prefix(IMAGE_DIR)?;
    Some(rest.strip_suffix(".bin").filter(|k| texturecraft_core::is_key(k)))
}

/// The images a project's records lay, by key: each written once.
#[derive(Debug, Default)]
pub struct ImageTable {
    pub by_key: BTreeMap<String, Arc<[f32]>>,
    /// Keys of images already met, by where their values sit (records laying one tip share its values).
    seen: HashMap<(usize, usize), String>,
}

impl ImageTable {
    /// The key of `values`, keeping them under it.
    pub fn add(&mut self, values: &Arc<[f32]>) -> String {
        let at = (Arc::as_ptr(values).addr(), values.len());
        if let Some(key) = self.seen.get(&at) {
            return key.clone();
        }
        let key = image_key(values);
        self.by_key.entry(key.clone()).or_insert_with(|| Arc::clone(values));
        self.seen.insert(at, key.clone());
        key
    }
}

/// The key of an image's values: the [`StableHash`] of their bytes.
pub fn image_key(values: &[f32]) -> String {
    let mut h = StableHash::new();
    for v in values {
        h.bytes(&v.to_le_bytes());
    }
    h.key()
}

/// The entry bytes of an image.
pub fn encode_image(values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + values.len() * 4);
    out.extend_from_slice(IMAGE_MAGIC);
    out.extend_from_slice(&IMAGE_VERSION.to_le_bytes());
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Reads an image entry stored under `key`; the error says why it is not one.
pub fn decode_image(key: &str, bytes: &[u8]) -> Result<Arc<[f32]>, String> {
    let (head, body) = bytes.split_at_checked(HEADER).ok_or("not a stroke image (cut short)")?;
    let (magic, version) = head.split_at(IMAGE_MAGIC.len());
    if magic != IMAGE_MAGIC {
        return Err("not a stroke image".into());
    }
    let version = u32::from_le_bytes(version.try_into().map_err(|_| "not a stroke image (cut short)")?);
    if version != IMAGE_VERSION {
        return Err(format!("stroke image version {version}; this build reads version {IMAGE_VERSION}"));
    }
    if body.len() % 4 != 0 {
        return Err("stroke image cut short".into());
    }
    let values: Arc<[f32]> = body.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    if image_key(&values) != key {
        return Err("stroke image does not match its key".into());
    }
    Ok(values)
}

/// The entry bytes of `record`, its images added to `table`.
pub fn encode(record: &StrokeRecord, table: &mut ImageTable) -> Result<Vec<u8>, String> {
    let keys: Vec<String> = record.images().into_iter().map(|v| table.add(v)).collect();
    let mut bare = record.clone();
    let empty: Arc<[f32]> = Arc::from(Vec::new());
    for v in bare.images_mut() {
        *v = Arc::clone(&empty);
    }
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&STROKE_RECORD_VERSION.to_le_bytes());
    let out = postcard::to_extend(&keys, out).map_err(|e| format!("stroke record: {e}"))?;
    postcard::to_extend(&bare, out).map_err(|e| format!("stroke record: {e}"))
}

/// The entry bytes of `record` in the older layout (version 1), its images inside it.
pub fn encode_inline(record: &StrokeRecord) -> Result<Vec<u8>, String> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&STROKE_RECORD_VERSION_INLINE.to_le_bytes());
    postcard::to_extend(record, out).map_err(|e| format!("stroke record: {e}"))
}

/// The bytes written for a stored record: its encoding (images added to `table`), or the kept bytes of an unreadable
/// one.
pub fn entry_bytes(stored: &StoredStrokes, table: &mut ImageTable) -> Result<Arc<Vec<u8>>, String> {
    match stored {
        StoredStrokes::Record(r) => encode(r, table).map(Arc::new),
        StoredStrokes::Unreadable(u) => Ok(Arc::clone(&u.bytes)),
    }
}

/// Reads an entry, its images taken from `images` (version 2); the error says why it is not a record this build reads.
/// Images of a version 1 record that are already in `images` are shared with it; the others are added.
pub fn decode(bytes: &[u8], images: &mut HashMap<String, Arc<[f32]>>) -> Result<StrokeRecord, String> {
    let (head, body) = bytes.split_at_checked(HEADER).ok_or("not a stroke record (cut short)")?;
    let (magic, version) = head.split_at(MAGIC.len());
    if magic != MAGIC {
        return Err("not a stroke record".into());
    }
    let version = u32::from_le_bytes(version.try_into().map_err(|_| "not a stroke record (cut short)")?);
    match version {
        STROKE_RECORD_VERSION => {
            let (keys, body) = postcard::take_from_bytes::<Vec<String>>(body).map_err(|e| format!("damaged stroke record: {e}"))?;
            let mut record = take_record(body)?;
            let mut slots = record.images_mut();
            if slots.len() != keys.len() {
                return Err(format!("stroke record names {} images for {}", keys.len(), slots.len()));
            }
            for (slot, key) in slots.iter_mut().zip(&keys) {
                **slot = images.get(key).cloned().ok_or_else(|| format!("stroke image {key} is missing"))?;
            }
            drop(slots);
            Ok(record)
        }
        STROKE_RECORD_VERSION_INLINE => {
            let mut record = take_record(body)?;
            for slot in record.images_mut() {
                let key = image_key(slot);
                *slot = Arc::clone(images.entry(key).or_insert_with(|| Arc::clone(slot)));
            }
            Ok(record)
        }
        _ => Err(format!("stroke record version {version}; this build reads versions {STROKE_RECORD_VERSION_INLINE} and {STROKE_RECORD_VERSION}")),
    }
}

fn take_record(body: &[u8]) -> Result<StrokeRecord, String> {
    let (record, rest) = postcard::take_from_bytes::<StrokeRecord>(body).map_err(|e| format!("damaged stroke record: {e}"))?;
    if !rest.is_empty() {
        return Err(format!("{} bytes after the stroke record", rest.len()));
    }
    Ok(record)
}

/// An entry as stored: the record, or its bytes with why they could not be read.
pub fn read(bytes: Vec<u8>, images: &mut HashMap<String, Arc<[f32]>>) -> StoredStrokes {
    match decode(&bytes, images) {
        Ok(record) => record.into(),
        Err(reason) => StoredStrokes::Unreadable(UnreadableStrokes { bytes: Arc::new(bytes), reason }),
    }
}
