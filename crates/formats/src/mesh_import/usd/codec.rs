//! Pieces of the USD Crate (`.usdc`) binary format shared by the reader and the writer: value
//! type ids, the LZ4 framing (one chunk-count byte, then LZ4 blocks) and the integer coding
//! (most common delta + 2-bit width codes + variable-width deltas).

use super::KIND;
use crate::{FormatError, Result, malformed};

pub(crate) const MAGIC: &[u8; 8] = b"PXR-USDC";
/// Size of the bootstrap header: magic, version, TOC offset, reserved.
pub(crate) const BOOTSTRAP_LEN: usize = 88;
/// Arrays shorter than this are never stored compressed.
pub(crate) const MIN_COMPRESSED_ARRAY: usize = 16;

/// ValueRep bits.
pub(crate) const IS_ARRAY: u64 = 1 << 63;
pub(crate) const IS_INLINED: u64 = 1 << 62;
pub(crate) const IS_COMPRESSED: u64 = 1 << 61;
pub(crate) const IS_ARRAY_EDIT: u64 = 1 << 60;
pub(crate) const PAYLOAD_MASK: u64 = (1 << 48) - 1;

/// Value type ids (crate data types).
pub(crate) mod ty {
    pub const BOOL: u8 = 1;
    pub const UCHAR: u8 = 2;
    pub const INT: u8 = 3;
    pub const UINT: u8 = 4;
    pub const INT64: u8 = 5;
    pub const UINT64: u8 = 6;
    pub const HALF: u8 = 7;
    pub const FLOAT: u8 = 8;
    pub const DOUBLE: u8 = 9;
    pub const STRING: u8 = 10;
    pub const TOKEN: u8 = 11;
    pub const ASSET_PATH: u8 = 12;
    pub const MATRIX2D: u8 = 13;
    pub const MATRIX4D: u8 = 15;
    pub const QUATD: u8 = 16;
    pub const QUATH: u8 = 18;
    pub const VEC2D: u8 = 19;
    pub const VEC4I: u8 = 30;
    pub const TOKEN_LIST_OP: u8 = 32;
    pub const PATH_LIST_OP: u8 = 34;
    pub const REFERENCE_LIST_OP: u8 = 35;
    pub const PATH_VECTOR: u8 = 40;
    pub const TOKEN_VECTOR: u8 = 41;
    pub const SPECIFIER: u8 = 42;
    pub const PERMISSION: u8 = 43;
    pub const VARIABILITY: u8 = 44;
    pub const TIME_SAMPLES: u8 = 46;
    pub const DOUBLE_VECTOR: u8 = 48;
}

/// Scalar kind and component count of the numeric types (vectors, quaternions, matrices).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Scalar {
    U8,
    I32,
    U32,
    I64,
    U64,
    F16,
    F32,
    F64,
}

impl Scalar {
    pub fn size(self) -> usize {
        match self {
            Scalar::U8 => 1,
            Scalar::F16 => 2,
            Scalar::I32 | Scalar::U32 | Scalar::F32 => 4,
            Scalar::I64 | Scalar::U64 | Scalar::F64 => 8,
        }
    }
}

/// (scalar, components) of numeric type `t`, or None for non-numeric types.
pub(crate) fn numeric(t: u8) -> Option<(Scalar, usize)> {
    use Scalar::*;
    Some(match t {
        ty::BOOL | ty::UCHAR => (U8, 1),
        ty::INT => (I32, 1),
        ty::UINT => (U32, 1),
        ty::INT64 => (I64, 1),
        ty::UINT64 => (U64, 1),
        ty::HALF => (F16, 1),
        ty::FLOAT => (F32, 1),
        ty::DOUBLE => (F64, 1),
        ty::MATRIX2D => (F64, 4),
        14 => (F64, 9),
        ty::MATRIX4D => (F64, 16),
        ty::QUATD => (F64, 4),
        17 => (F32, 4),
        ty::QUATH => (F16, 4),
        // Vec{2,3,4}{d,f,h,i}: ids 19..=30 in that order.
        ty::VEC2D..=ty::VEC4I => {
            let k = t - ty::VEC2D;
            let scalar = [F64, F32, F16, I32][usize::from(k % 4)];
            (scalar, usize::from(k / 4) + 2)
        }
        _ => return None,
    })
}

/// Decompress Crate's LZ4 framing into at most `max_out` bytes.
pub(crate) fn decompress(input: &[u8], max_out: usize) -> Result<Vec<u8>> {
    craft_usd::decompress(input, max_out).map_err(|e| malformed(KIND, e.to_string()))
}

/// Crate's LZ4 framing for one chunk (inputs are far below LZ4's ~2 GB chunk limit).
pub(crate) fn compress(input: &[u8]) -> Result<Vec<u8>> {
    // Keep one compressed output reservation within the app's decoded-data ceiling.
    craft_usd::compress::compress(input, super::MAX_DECODED_BYTES).map_err(|e| malformed(KIND, e.to_string()))
}

/// Upper bound of the encoded size of `n` integers of `width` bytes.
pub(crate) fn encoded_ints_len(n: usize, width: usize) -> usize {
    if n == 0 { 0 } else { width.saturating_add(n.saturating_mul(2).saturating_add(7) / 8).saturating_add(n.saturating_mul(width)) }
}

/// Decode packed integers after the caller charges its aggregate decoded-byte budget.
pub(crate) fn decode_ints(data: &[u8], n: usize, width: usize) -> Result<Vec<i64>> {
    let width = match width {
        4 => craft_usd::integers::Width::I32,
        8 => craft_usd::integers::Width::I64,
        _ => return Err(malformed(KIND, "unsupported integer width")),
    };
    craft_usd::integers::decode(data, n, width, super::MAX_DECODED_BYTES).map_err(|e| malformed(KIND, e.to_string()))
}

/// Encode packed integers with a combined scratch/output reservation cap.
pub(crate) fn encode_ints(values: &[i64], width: usize) -> Result<Vec<u8>> {
    let width = match width {
        4 => craft_usd::integers::Width::I32,
        8 => craft_usd::integers::Width::I64,
        _ => return Err(malformed(KIND, "unsupported integer width")),
    };
    craft_usd::integers::encode(values, width, super::MAX_DECODED_BYTES).map_err(|e| malformed(KIND, e.to_string()))
}

/// Error for counts that cannot be real (more items than bytes left could describe).
pub(crate) fn check_count(n: u64, cap: usize) -> Result<usize> {
    usize::try_from(n).ok().filter(|n| *n <= cap).ok_or_else(|| FormatError::TooLarge(format!("USD array of {n} items (limit {cap})")))
}
