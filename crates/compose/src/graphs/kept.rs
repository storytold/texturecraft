//! Cooks kept outside the document, between imports and runs: [`KeptCooks`] is where (a folder on the desktop, given
//! by the host; nothing in the web build), [`encode`] and [`decode`] the bytes a cook is kept as.
//!
//! A cook is found under its cook key ([`super::GraphStore::cook_key`]), which covers everything that decides its
//! pixels, so a kept cook is used as it is. The bytes hold the tiles exactly as a render made them: a cook read back is
//! the same to the bit as one rendered again. Anything that is not exactly one cook decodes to nothing and is rendered.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use texturecraft_texel::{TILE_AREA, TileImage};

use super::Cooked;

/// Where cooks are kept by key (the host's folder, say). Getting nothing only costs a render, and a failed put
/// nothing: neither is an error.
pub trait KeptCooks: Send + Sync {
    fn get(&self, key: &str) -> Option<Vec<u8>>;
    fn put(&self, key: &str, bytes: &[u8]);
}

/// Format of the bytes: changing [`encode`] changes it, so that old bytes are not read as new ones.
pub const FORMAT: u32 = 1;
/// Bounds on what is decoded (hostile bytes): outputs per cook, an output name's length, an output's side.
const MAX_OUTPUTS: usize = 256;
const MAX_NAME: usize = 1024;
const MAX_SIDE: u32 = 16384;

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// A cook as bytes.
pub fn encode(cooked: &Cooked) -> Vec<u8> {
    let mut out = Vec::with_capacity(cooked.bytes() + 64);
    put_u32(&mut out, FORMAT);
    put_u32(&mut out, cooked.outputs.len() as u32);
    for (name, img) in &cooked.outputs {
        put_u32(&mut out, name.len() as u32);
        out.extend_from_slice(name.as_bytes());
        out.push(u8::from(cooked.linear.contains(name)));
        put_u32(&mut out, img.width());
        put_u32(&mut out, img.height());
        out.push(img.comps() as u8);
        for ty in 0..img.tiles_y() {
            for tx in 0..img.tiles_x() {
                match img.tile(tx, ty) {
                    Some(t) => {
                        out.push(1);
                        t.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
                    }
                    None => out.push(0),
                }
            }
        }
    }
    out
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (now, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(now)
    }

    fn byte(&mut self) -> Option<u8> {
        self.take(1)?.first().copied()
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
}

/// The cook `bytes` hold, `None` unless they are exactly one (of this [`FORMAT`]).
pub fn decode(bytes: &[u8]) -> Option<Cooked> {
    let mut r = Reader(bytes);
    if r.u32()? != FORMAT {
        return None;
    }
    let count = r.u32()? as usize;
    if count > MAX_OUTPUTS {
        return None;
    }
    let (mut outputs, mut linear) = (BTreeMap::new(), BTreeSet::new());
    for _ in 0..count {
        let length = r.u32()? as usize;
        if length > MAX_NAME {
            return None;
        }
        let name = std::str::from_utf8(r.take(length)?).ok()?.to_string();
        let is_linear = match r.byte()? {
            0 => false,
            1 => true,
            _ => return None,
        };
        let (w, h) = (r.u32()?, r.u32()?);
        let comps = usize::from(r.byte()?);
        if !(1..=MAX_SIDE).contains(&w) || !(1..=MAX_SIDE).contains(&h) || !matches!(comps, 1 | 4) {
            return None;
        }
        let mut img = TileImage::new(w, h, comps);
        let tile_bytes = TILE_AREA * comps * 2;
        for ty in 0..img.tiles_y() {
            for tx in 0..img.tiles_x() {
                match r.byte()? {
                    0 => {}
                    1 => {
                        let data: Vec<u16> = r.take(tile_bytes)?.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
                        img.set_tile(tx, ty, Some(Arc::new(data)));
                    }
                    _ => return None,
                }
            }
        }
        if is_linear {
            linear.insert(name.clone());
        }
        if outputs.insert(name, img).is_some() {
            return None;
        }
    }
    r.0.is_empty().then_some(Cooked::new(outputs, linear))
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_graph::tex::Tex;

    fn sample() -> Cooked {
        let mut outs = BTreeMap::new();
        outs.insert("grey".to_string(), Tex::from_fn(70, 3, |u, _| [u, u, u, 1.0]));
        outs.insert("colour".to_string(), Tex::from_fn(5, 130, |u, v| [u, v, 0.25, 0.5]));
        Cooked::from_textures(outs, ["colour".to_string()].into())
    }

    #[test]
    fn a_cook_comes_back_exactly() {
        let c = sample();
        let bytes = encode(&c);
        let back = decode(&bytes).unwrap();
        assert_eq!(back, c);
        assert_eq!(encode(&back), bytes);
        assert_eq!(back.outputs["grey"].comps(), 1);
        assert!(back.is_linear(Some("colour")) && !back.is_linear(Some("grey")));
        // unallocated tiles stay unallocated
        let mut sparse = TileImage::new(200, 200, 1);
        sparse.set(130, 5, &[7]);
        let c = Cooked::new([("s".to_string(), sparse)].into(), BTreeSet::new());
        let back = decode(&encode(&c)).unwrap();
        assert_eq!(back.outputs["s"].allocated_count(), 1);
        assert_eq!(back, c);
    }

    #[test]
    fn what_is_not_exactly_one_cook_is_refused() {
        let bytes = encode(&sample());
        for cut in (0..bytes.len()).step_by(7) {
            assert!(decode(&bytes[..cut]).is_none(), "cut at {cut}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(decode(&longer).is_none());
        let mut other_format = bytes.clone();
        other_format[0] ^= 1;
        assert!(decode(&other_format).is_none());
        assert!(decode(&[0xff; 64]).is_none());
        assert!(decode(&[]).is_none());
        // a huge side is refused before anything is allocated
        let mut huge = Vec::new();
        for v in [FORMAT, 1, 1] {
            huge.extend_from_slice(&v.to_le_bytes());
        }
        huge.extend_from_slice(b"x\0");
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        huge.push(4);
        assert!(decode(&huge).is_none());
    }
}
