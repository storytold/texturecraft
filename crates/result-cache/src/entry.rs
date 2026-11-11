// SPDX-License-Identifier: MIT OR Apache-2.0
//! One entry's bytes: a fixed header (format, key, lengths, checksum of the packed data), then the data packed with LZ4.
//! [`unpack`] checks everything and gives nothing rather than something doubtful.

const MAGIC: &[u8; 8] = b"TCRESLT1";
/// Longest key: keys are hexadecimal fingerprints.
pub const MAX_KEY: usize = 128;

/// FNV-1a over 128 bits: enough to tell a damaged entry from a whole one (the key says what it is for).
fn checksum(length: usize, packed: &[u8]) -> u128 {
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    let mut h = OFFSET;
    // the unpacked length too: unpacking allocates it before anything else can be checked
    for b in (length as u64).to_le_bytes().iter().chain(packed) {
        h ^= u128::from(*b);
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// True for a key an entry can be stored under: 1 to [`MAX_KEY`] ASCII letters, digits, `-` or `_` (it names a file).
pub fn valid_key(key: &str) -> bool {
    (1..=MAX_KEY).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Packs `data` for storing under `key` (a [`valid_key`]).
pub fn pack(key: &str, data: &[u8]) -> Vec<u8> {
    let packed = lz4_flex::block::compress(data);
    let mut out = Vec::with_capacity(8 + 1 + key.len() + 32 + packed.len());
    out.extend_from_slice(MAGIC);
    out.push(key.len().min(MAX_KEY) as u8);
    out.extend_from_slice(key.as_bytes().get(..key.len().min(MAX_KEY)).unwrap_or_default());
    out.extend_from_slice(&(packed.len() as u64).to_le_bytes());
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    out.extend_from_slice(&checksum(data.len(), &packed).to_le_bytes());
    out.extend_from_slice(&packed);
    out
}

/// Takes the next `n` bytes.
fn take<'a>(rest: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    let (now, after) = rest.split_at_checked(n)?;
    *rest = after;
    Some(now)
}

fn number(rest: &mut &[u8]) -> Option<usize> {
    usize::try_from(u64::from_le_bytes(take(rest, 8)?.try_into().ok()?)).ok()
}

/// The data of a stored entry, if `stored` is a whole, undamaged entry for `key`. `max_length` caps the unpacked size
/// (it is allocated before the data is checked).
pub fn unpack(key: &str, stored: &[u8], max_length: usize) -> Option<Vec<u8>> {
    let mut rest = stored;
    if take(&mut rest, 8)? != MAGIC {
        return None;
    }
    let key_length = usize::from(*take(&mut rest, 1)?.first()?);
    if take(&mut rest, key_length)? != key.as_bytes() {
        return None;
    }
    let packed_length = number(&mut rest)?;
    let length = number(&mut rest)?;
    let sum = u128::from_le_bytes(take(&mut rest, 16)?.try_into().ok()?);
    if rest.len() != packed_length || length > max_length || checksum(length, rest) != sum {
        return None;
    }
    let data = lz4_flex::block::decompress(rest, length).ok()?;
    (data.len() == length).then_some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: usize = 1 << 30;

    #[test]
    fn round_trip() {
        let data: Vec<u8> = (0..100_000u32).map(|v| (v % 7) as u8).collect();
        let stored = pack("a1", &data);
        assert!(stored.len() < data.len());
        assert_eq!(unpack("a1", &stored, MAX).as_deref(), Some(data.as_slice()));
        assert_eq!(unpack("a1", &pack("a1", &[]), MAX).as_deref(), Some(&[][..]));
        assert_eq!(unpack("a1", &stored, data.len() - 1), None, "longer than allowed");
    }

    #[test]
    fn every_cut_and_every_changed_byte_is_refused() {
        let data: Vec<u8> = (0..300u32).map(|v| (v * v) as u8).collect();
        let stored = pack("k", &data);
        for cut in 0..stored.len() {
            assert_eq!(unpack("k", &stored[..cut], MAX), None, "cut at {cut}");
        }
        for at in 0..stored.len() {
            let mut bad = stored.clone();
            bad[at] ^= 0x40;
            assert_eq!(unpack("k", &bad, MAX), None, "changed at {at}");
        }
        let mut longer = stored.clone();
        longer.push(0);
        assert_eq!(unpack("k", &longer, MAX), None);
    }

    #[test]
    fn another_key_is_refused() {
        assert_eq!(unpack("b", &pack("a", b"data"), MAX), None);
        assert_eq!(unpack("ab", &pack("a", b"data"), MAX), None);
    }

    #[test]
    fn nonsense_is_refused_without_panicking() {
        assert_eq!(unpack("a", b"", MAX), None);
        assert_eq!(unpack("a", b"TCRESLT1 and then not much", MAX), None);
        assert_eq!(unpack("a", &vec![0xff; 4096], MAX), None);
    }

    #[test]
    fn keys_name_files() {
        assert!(valid_key("0123abcdef") && valid_key("cook-1_x"));
        for bad in ["", "../x", "a/b", "a.b", "é", &"a".repeat(MAX_KEY + 1)] {
            assert!(!valid_key(bad), "{bad}");
        }
    }
}
