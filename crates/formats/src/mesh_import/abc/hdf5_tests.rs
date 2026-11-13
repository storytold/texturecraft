//! Alembic files in the HDF5 container, built by a small test-only HDF5 writer, then imported.
//!
//! The writer emits only the structures these tests need, in three styles:
//! - [`Style::Classic`]: superblock version 0, version 1 object headers (with a continuation
//!   block), symbol-table groups (a version 1 B-tree of type 0, one symbol table node, a local
//!   heap), version 1 attribute messages, version 1 dataspaces, contiguous datasets.
//! - [`Style::Compact`]: superblock version 2, version 2 object headers with attribute creation
//!   order and an OCHK continuation block, link info and link messages, version 3 attribute
//!   messages, version 2 dataspaces, compact datasets.
//! - [`Style::Dense`]: superblock version 2, version 2 object headers, dense links and dense
//!   attributes (fractal heaps whose root is a direct block or an indirect block of direct
//!   blocks, version 2 B-trees whose root is a leaf or an internal node over leaves), chunked
//!   datasets (layout version 3, a version 1 B-tree of type 1) compressed with deflate.
//!
//! Every style also writes a fill value message the reader must skip. Checksums and name hashes
//! are written as zero: the reader does not verify them. Sizes of offsets and lengths are 8.

use std::io::Write as _;

use texturecraft_geom::glam::Vec3;

use super::*;
use crate::mesh_import::finish;

const UNDEF: u64 = u64::MAX;
/// Stand-in address a test patches afterwards (to make cycles).
const PLACEHOLDER: u64 = 0x5eed_5eed_5eed_5eed;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Style {
    Classic,
    Compact,
    Dense,
}

const STYLES: [Style; 3] = [Style::Classic, Style::Compact, Style::Dense];

/// An element type of a dataset or attribute.
#[derive(Clone, Copy)]
enum Ty {
    U8,
    I32,
    U32,
    F32,
    F64,
    Str(usize),
}

impl Ty {
    fn size(self) -> usize {
        match self {
            Ty::U8 => 1,
            Ty::I32 | Ty::U32 | Ty::F32 => 4,
            Ty::F64 => 8,
            Ty::Str(n) => n,
        }
    }
}

fn u16le(v: usize) -> [u8; 2] {
    (v as u16).to_le_bytes()
}

fn pad8(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(8) {
        v.push(0);
    }
}

/// Fewest bytes that hold `n`.
fn width(n: u64) -> usize {
    ((64 - n.leading_zeros() as usize).div_ceil(8)).max(1)
}

/// A datatype message (version 1).
fn datatype(t: Ty) -> Vec<u8> {
    let mut v = Vec::new();
    match t {
        Ty::U8 | Ty::I32 | Ty::U32 => {
            let signed = matches!(t, Ty::I32);
            v.push(0x10);
            v.extend([if signed { 0x08 } else { 0 }, 0, 0]);
            v.extend((t.size() as u32).to_le_bytes());
            v.extend(0u16.to_le_bytes());
            v.extend(((t.size() * 8) as u16).to_le_bytes());
        }
        Ty::F32 => {
            v.push(0x11);
            v.extend([0x20, 31, 0]);
            v.extend(4u32.to_le_bytes());
            v.extend(0u16.to_le_bytes());
            v.extend(32u16.to_le_bytes());
            v.extend([23, 8, 0, 23]);
            v.extend(127u32.to_le_bytes());
        }
        Ty::F64 => {
            v.push(0x11);
            v.extend([0x20, 63, 0]);
            v.extend(8u32.to_le_bytes());
            v.extend(0u16.to_le_bytes());
            v.extend(64u16.to_le_bytes());
            v.extend([52, 11, 0, 52]);
            v.extend(1023u32.to_le_bytes());
        }
        Ty::Str(n) => {
            v.push(0x13);
            v.extend([0, 0, 0]);
            v.extend((n as u32).to_le_bytes());
        }
    }
    v
}

/// The test HDF5 writer: structures appended at 8-byte aligned addresses, the superblock last.
struct W {
    b: Vec<u8>,
    style: Style,
    /// Element count every dataset's dataspace claims instead of its real one (hostile tests).
    fake_count: Option<u64>,
}

impl W {
    fn new(style: Style) -> W {
        W { b: vec![0; 128], style, fake_count: None }
    }

    fn put(&mut self, bytes: &[u8]) -> u64 {
        pad8(&mut self.b);
        let at = self.b.len() as u64;
        self.b.extend_from_slice(bytes);
        at
    }

    fn patch(&mut self, at: u64, bytes: &[u8]) {
        self.b[at as usize..at as usize + bytes.len()].copy_from_slice(bytes);
    }

    /// A dataspace message: `None` is scalar, an empty slice is the null space (version 2 only).
    fn dataspace(&self, dims: Option<&[u64]>) -> Vec<u8> {
        let dims_list = dims.unwrap_or(&[]);
        let mut v = if self.style == Style::Classic {
            vec![1, dims_list.len() as u8, 0, 0, 0, 0, 0, 0]
        } else {
            let kind = match dims {
                None => 0,
                Some([]) => 2,
                Some(_) => 1,
            };
            vec![2, dims_list.len() as u8, 0, kind]
        };
        for d in dims_list {
            v.extend(d.to_le_bytes());
        }
        v
    }

    fn attribute(&self, name: &str, t: Ty, dims: Option<&[u64]>, data: &[u8]) -> Vec<u8> {
        let mut name_z = name.as_bytes().to_vec();
        name_z.push(0);
        let (dt, ds) = (datatype(t), self.dataspace(dims));
        let mut v = Vec::new();
        if self.style == Style::Classic {
            v.extend([1, 0]);
            v.extend(u16le(name_z.len()));
            v.extend(u16le(dt.len()));
            v.extend(u16le(ds.len()));
            for mut part in [name_z, dt, ds] {
                pad8(&mut part);
                v.extend(part);
            }
        } else {
            v.extend([3, 0]);
            v.extend(u16le(name_z.len()));
            v.extend(u16le(dt.len()));
            v.extend(u16le(ds.len()));
            v.push(0);
            v.extend(name_z);
            v.extend(dt);
            v.extend(ds);
        }
        v.extend_from_slice(data);
        v
    }

    fn string_attribute(&self, name: &str, s: &str) -> Vec<u8> {
        let bytes = if s.is_empty() { vec![0] } else { s.as_bytes().to_vec() };
        self.attribute(name, Ty::Str(bytes.len()), None, &bytes)
    }

    /// Message list bytes of one header chunk.
    fn messages(&self, msgs: &[(u16, Vec<u8>)], order: &mut u16) -> Vec<u8> {
        let mut body = Vec::new();
        for (t, d) in msgs {
            if self.style == Style::Classic {
                let mut d = d.clone();
                pad8(&mut d);
                body.extend(t.to_le_bytes());
                body.extend(u16le(d.len()));
                body.extend([0, 0, 0, 0]);
                body.extend(d);
            } else {
                body.push(*t as u8);
                body.extend(u16le(d.len()));
                body.push(0);
                if self.style == Style::Compact {
                    body.extend(order.to_le_bytes());
                    *order += 1;
                }
                body.extend(d);
            }
        }
        body
    }

    /// An object header holding `msgs`; past three messages the rest go to a continuation block.
    fn header(&mut self, msgs: &[(u16, Vec<u8>)]) -> u64 {
        let split = if msgs.len() > 3 { 2 } else { msgs.len() };
        let (first, rest) = msgs.split_at(split);
        let mut order = 0u16;
        let mut first = first.to_vec();
        if !rest.is_empty() {
            let mut chunk = self.messages(rest, &mut order);
            if self.style != Style::Classic {
                let mut c = b"OCHK".to_vec();
                c.extend(chunk);
                c.extend([0; 4]);
                chunk = c;
            }
            let at = self.put(&chunk);
            let mut cont = at.to_le_bytes().to_vec();
            cont.extend((chunk.len() as u64).to_le_bytes());
            first.push((0x10, cont));
        }
        // Creation order restarts: the continued messages were numbered first on purpose.
        let body = self.messages(&first, &mut order);
        let mut h = Vec::new();
        if self.style == Style::Classic {
            h.extend([1, 0]);
            h.extend(u16le(msgs.len() + usize::from(!rest.is_empty())));
            h.extend(1u32.to_le_bytes());
            h.extend((body.len() as u32).to_le_bytes());
            h.extend([0; 4]);
        } else {
            h.extend(b"OHDR");
            h.push(2);
            h.push(if self.style == Style::Compact { 0x06 } else { 0x02 });
            h.extend((body.len() as u32).to_le_bytes());
        }
        h.extend(body);
        if self.style != Style::Classic {
            h.extend([0; 4]);
        }
        self.put(&h)
    }

    fn link(name: &str, addr: u64, order: Option<u64>) -> Vec<u8> {
        let mut v = vec![1, if order.is_some() { 0x04 } else { 0 }];
        if let Some(o) = order {
            v.extend(o.to_le_bytes());
        }
        v.push(name.len() as u8);
        v.extend(name.as_bytes());
        v.extend(addr.to_le_bytes());
        v
    }

    /// A group with hard links and attribute messages, stored as the style prescribes.
    fn group(&mut self, links: &[(String, u64)], attrs: &[Vec<u8>]) -> u64 {
        let mut msgs = vec![(5u16, vec![2, 2, 2, 0])];
        match self.style {
            Style::Classic => {
                let st = self.symbol_table(links);
                msgs.push((0x11, st));
                msgs.extend(attrs.iter().map(|a| (0x0c, a.clone())));
            }
            Style::Compact => {
                let mut info = vec![0, 3];
                info.extend((links.len() as u64).to_le_bytes());
                for _ in 0..3 {
                    info.extend(UNDEF.to_le_bytes());
                }
                msgs.push((2, info));
                // Link messages in reverse: creation order, not message order, decides.
                for (i, (name, addr)) in links.iter().enumerate().rev() {
                    msgs.push((6, W::link(name, *addr, Some(i as u64))));
                }
                msgs.extend(attrs.iter().map(|a| (0x0c, a.clone())));
            }
            Style::Dense => {
                let objs: Vec<Vec<u8>> = links.iter().enumerate().map(|(i, (n, a))| W::link(n, *a, Some(i as u64))).collect();
                let (heap, ids) = self.heap(&objs, 32, 512);
                // Records in reverse, as a name-hash order would scramble them.
                let recs: Vec<Vec<u8>> = ids.iter().rev().map(|id| [&[0u8; 4][..], id].concat()).collect();
                let tree = self.btree(5, &recs, 512);
                let mut info = vec![0, 0];
                info.extend(heap.to_le_bytes());
                info.extend(tree.to_le_bytes());
                msgs.push((2, info));
                if !attrs.is_empty() {
                    let (heap, ids) = self.heap(attrs, 40, 128);
                    let recs: Vec<Vec<u8>> = ids
                        .iter()
                        .enumerate()
                        .rev()
                        .map(|(i, id)| {
                            let mut r = id.clone();
                            r.push(0);
                            r.extend((i as u32).to_le_bytes());
                            r.extend([0; 4]);
                            r
                        })
                        .collect();
                    let tree = self.btree(8, &recs, 128);
                    let mut ainfo = vec![0, 0];
                    ainfo.extend(heap.to_le_bytes());
                    ainfo.extend(tree.to_le_bytes());
                    msgs.push((0x15, ainfo));
                }
            }
        }
        self.header(&msgs)
    }

    /// Local heap, symbol table node and group B-tree; returns the symbol table message.
    fn symbol_table(&mut self, links: &[(String, u64)]) -> Vec<u8> {
        let mut sorted = links.to_vec();
        sorted.sort();
        let mut data = vec![0u8; 8];
        let mut offsets = Vec::new();
        for (name, _) in &sorted {
            offsets.push(data.len() as u64);
            data.extend(name.as_bytes());
            data.push(0);
            pad8(&mut data);
        }
        let data_at = self.put(&data);
        let mut heap = b"HEAP".to_vec();
        heap.extend([0; 4]);
        heap.extend((data.len() as u64).to_le_bytes());
        heap.extend(UNDEF.to_le_bytes());
        heap.extend(data_at.to_le_bytes());
        let heap_at = self.put(&heap);
        let mut snod = b"SNOD".to_vec();
        snod.extend([1, 0]);
        snod.extend(u16le(sorted.len()));
        for (i, (_, addr)) in sorted.iter().enumerate() {
            snod.extend(offsets[i].to_le_bytes());
            snod.extend(addr.to_le_bytes());
            snod.extend([0; 24]);
        }
        // 2K entries, K = 16 (the superblock's group leaf node K).
        snod.resize(8 + 32 * 40, 0);
        let snod_at = self.put(&snod);
        let mut tree = b"TREE".to_vec();
        tree.extend([0, 0]);
        tree.extend(1u16.to_le_bytes());
        tree.extend(UNDEF.to_le_bytes());
        tree.extend(UNDEF.to_le_bytes());
        tree.extend(0u64.to_le_bytes());
        tree.extend(snod_at.to_le_bytes());
        tree.extend(offsets.last().copied().unwrap_or(0).to_le_bytes());
        let tree_at = self.put(&tree);
        let mut st = tree_at.to_le_bytes().to_vec();
        st.extend(heap_at.to_le_bytes());
        st
    }

    /// A fractal heap holding `objs` (table width 4, largest direct block 64 KiB, objects up to
    /// 4 KiB); returns the header address and each object's managed heap ID.
    fn heap(&mut self, objs: &[Vec<u8>], bits: u16, start: u64) -> (u64, Vec<Vec<u8>>) {
        let off_bytes = (bits as usize).div_ceil(8);
        let row_size = |r: u64| if r < 2 { start } else { start << (r - 1) };
        let header_len = 4 + 1 + 2 + 2 + 1 + 4 + 12 * 8 + 2 + 8 + 8 + 2 + 2 + 8 + 2 + 4;
        let hdr = self.put(&vec![0; header_len]);
        let dblock_header = 5 + 8 + off_bytes;
        // Objects laid into consecutive direct blocks (entry e is in row e / 4).
        let mut blocks: Vec<(u64, Vec<u8>)> = Vec::new();
        let (mut pos, mut entry) = (0u64, 0u64);
        let mut cur = Vec::new();
        let mut ids = Vec::new();
        for o in objs {
            if (dblock_header + cur.len() + o.len()) as u64 > row_size(entry / 4) {
                blocks.push((pos, std::mem::take(&mut cur)));
                pos += row_size(entry / 4);
                entry += 1;
            }
            let mut id = vec![0u8];
            id.extend(&(pos + (dblock_header + cur.len()) as u64).to_le_bytes()[..off_bytes]);
            id.extend(&(o.len() as u64).to_le_bytes()[..2]);
            ids.push(id);
            cur.extend(o);
        }
        blocks.push((pos, cur));
        let mut addrs = Vec::new();
        for (i, (off, data)) in blocks.iter().enumerate() {
            let mut d = b"FHDB".to_vec();
            d.push(0);
            d.extend(hdr.to_le_bytes());
            d.extend(&off.to_le_bytes()[..off_bytes]);
            d.extend(data);
            d.resize(row_size(i as u64 / 4) as usize, 0);
            addrs.push(self.put(&d));
        }
        let (root, rows) = if blocks.len() == 1 {
            (addrs[0], 0u16)
        } else {
            let rows = (blocks.len() as u64 - 1) / 4 + 1;
            let mut ib = b"FHIB".to_vec();
            ib.push(0);
            ib.extend(hdr.to_le_bytes());
            ib.extend(vec![0; off_bytes]);
            for e in 0..rows * 4 {
                ib.extend(addrs.get(e as usize).copied().unwrap_or(UNDEF).to_le_bytes());
            }
            ib.extend([0; 4]);
            (self.put(&ib), rows as u16)
        };
        let mut h = b"FRHP".to_vec();
        h.push(0);
        h.extend(u16le(1 + off_bytes + 2));
        h.extend([0, 0, 0]);
        h.extend(4096u32.to_le_bytes());
        let total: u64 = (0..blocks.len() as u64).map(|e| row_size(e / 4)).sum();
        for v in [0, UNDEF, 0, UNDEF, total, total, total, objs.len() as u64, 0, 0, 0, 0] {
            h.extend(v.to_le_bytes());
        }
        h.extend(4u16.to_le_bytes());
        h.extend(start.to_le_bytes());
        h.extend(65536u64.to_le_bytes());
        h.extend(bits.to_le_bytes());
        h.extend(0u16.to_le_bytes());
        h.extend(root.to_le_bytes());
        h.extend(rows.to_le_bytes());
        h.extend([0; 4]);
        assert_eq!(h.len(), header_len);
        self.patch(hdr, &h);
        (hdr, ids)
    }

    /// A version 2 B-tree of `records`: one leaf, or an internal root over leaves.
    fn btree(&mut self, ty: u8, records: &[Vec<u8>], node_size: usize) -> u64 {
        let rs = if ty == 5 { 11 } else { 17 };
        let leaf_max = (node_size - 10) / rs;
        let leaf = |w: &mut W, recs: &[Vec<u8>]| {
            let mut n = b"BTLF".to_vec();
            n.extend([0, ty]);
            for r in recs {
                n.extend(r);
            }
            n.extend([0; 4]);
            n.resize(node_size, 0);
            w.put(&n)
        };
        let (root, depth, root_n) = if records.is_empty() {
            (UNDEF, 0, 0)
        } else if records.len() <= leaf_max {
            (leaf(self, records), 0u16, records.len())
        } else {
            // Leaves of `leaf_max` records split by one separator record each.
            let mut children = Vec::new();
            let mut seps = Vec::new();
            let mut rest = records;
            loop {
                let take = rest.len().min(leaf_max);
                children.push((leaf(self, &rest[..take]), take));
                rest = &rest[take..];
                let Some((sep, more)) = rest.split_first() else { break };
                seps.push(sep.clone());
                rest = more;
            }
            let nrec_w = width(leaf_max as u64);
            let ptr = 8 + nrec_w;
            assert!(seps.len() <= (node_size - 10 - ptr) / (rs + ptr), "internal node overflows");
            let mut n = b"BTIN".to_vec();
            n.extend([0, ty]);
            for s in &seps {
                n.extend(s);
            }
            for (addr, count) in &children {
                n.extend(addr.to_le_bytes());
                n.extend(&(*count as u64).to_le_bytes()[..nrec_w]);
            }
            n.extend([0; 4]);
            n.resize(node_size, 0);
            (self.put(&n), 1, seps.len())
        };
        let mut h = b"BTHD".to_vec();
        h.extend([0, ty]);
        h.extend((node_size as u32).to_le_bytes());
        h.extend(u16le(rs));
        h.extend(depth.to_le_bytes());
        h.extend([100, 40]);
        h.extend(root.to_le_bytes());
        h.extend(u16le(root_n));
        h.extend((records.len() as u64).to_le_bytes());
        h.extend([0; 4]);
        self.put(&h)
    }

    /// A one-dimensional dataset of `raw` (elements of `t`) with a sample `key` attribute.
    fn dataset(&mut self, t: Ty, raw: &[u8]) -> u64 {
        let count = (raw.len() / t.size()) as u64;
        let space = match (self.fake_count, count) {
            (Some(n), _) => self.dataspace(Some(&[n])),
            (None, 0) if self.style != Style::Classic => self.dataspace(Some(&[])),
            (None, c) => self.dataspace(Some(&[c])),
        };
        let mut msgs = vec![(1u16, space), (3, datatype(t)), (5, vec![2, 2, 2, 0])];
        match self.style {
            Style::Classic => {
                let at = if raw.is_empty() { UNDEF } else { self.put(raw) };
                let mut l = vec![3, 1];
                l.extend(at.to_le_bytes());
                l.extend((raw.len() as u64).to_le_bytes());
                msgs.push((8, l));
            }
            Style::Dense if !raw.is_empty() => {
                let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                z.write_all(raw).unwrap();
                let packed = z.finish().unwrap();
                let chunk = self.put(&packed);
                let mut tree = b"TREE".to_vec();
                tree.extend([1, 0]);
                tree.extend(1u16.to_le_bytes());
                tree.extend(UNDEF.to_le_bytes());
                tree.extend(UNDEF.to_le_bytes());
                tree.extend((packed.len() as u32).to_le_bytes());
                tree.extend(0u32.to_le_bytes());
                tree.extend([0; 16]);
                tree.extend(chunk.to_le_bytes());
                tree.extend(0u32.to_le_bytes());
                tree.extend(0u32.to_le_bytes());
                tree.extend(count.to_le_bytes());
                tree.extend(0u64.to_le_bytes());
                let tree_at = self.put(&tree);
                let mut l = vec![3, 2, 2];
                l.extend(tree_at.to_le_bytes());
                l.extend((count as u32).to_le_bytes());
                l.extend((t.size() as u32).to_le_bytes());
                msgs.push((8, l));
                // Filter pipeline version 2: deflate (id 1, no name) at level 6.
                let mut f = vec![2, 1];
                f.extend([1, 0, 0, 0, 1, 0]);
                f.extend(6u32.to_le_bytes());
                msgs.push((0x0b, f));
            }
            _ => {
                let mut l = vec![3, 0];
                l.extend(u16le(raw.len()));
                l.extend(raw);
                msgs.push((8, l));
            }
        }
        let key = self.attribute("key", Ty::U8, Some(&[16]), &[7; 16]);
        msgs.push((0x0c, key));
        self.header(&msgs)
    }

    fn finish(mut self, root: u64) -> Vec<u8> {
        let eof = self.b.len() as u64;
        let mut s = b"\x89HDF\r\n\x1a\n".to_vec();
        if self.style == Style::Classic {
            s.extend([0, 0, 0, 0, 0, 8, 8, 0]);
            s.extend(16u16.to_le_bytes());
            s.extend(16u16.to_le_bytes());
            s.extend([0; 4]);
            for v in [0, UNDEF, eof, UNDEF] {
                s.extend(v.to_le_bytes());
            }
            // Root symbol table entry (no cached scratch-pad).
            s.extend(0u64.to_le_bytes());
            s.extend(root.to_le_bytes());
            s.extend([0; 24]);
        } else {
            s.extend([2, 8, 8, 0]);
            for v in [0, UNDEF, eof, root] {
                s.extend(v.to_le_bytes());
            }
            s.extend([0; 4]);
        }
        self.patch(0, &s);
        self.b
    }
}

/// A property's attributes on its compound group and its link there (sample or child group).
struct P {
    attrs: Vec<Vec<u8>>,
    link: Option<(String, u64)>,
}

const F32: u32 = 10;
const I32: u32 = 6;
const U32: u32 = 5;
const U8: u32 = 1;
const F64: u32 = 11;

/// Alembic's HDF5 layout on top of [`W`].
struct Abc {
    w: W,
}

impl Abc {
    fn new(style: Style) -> Abc {
        Abc { w: W::new(style) }
    }

    fn header(&self, name: &str, info: &[u32], meta: &str) -> Vec<Vec<u8>> {
        let raw: Vec<u8> = info.iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut v = vec![self.w.attribute(&format!("{name}.info"), Ty::U32, Some(&[info.len() as u64]), &raw)];
        if !meta.is_empty() {
            v.push(self.w.string_attribute(&format!("{name}.meta"), meta));
        }
        v
    }

    fn array(&mut self, name: &str, meta: &str, pod: u32, extent: u32, t: Ty, raw: &[u8]) -> P {
        let ds = self.w.dataset(t, raw);
        P { attrs: self.header(name, &[2 | (pod << 2) | (extent << 8)], meta), link: Some((format!("{name}.smp0"), ds)) }
    }

    fn scalar(&mut self, name: &str, pod: u32, extent: u32, t: Ty, raw: &[u8]) -> P {
        let mut attrs = self.header(name, &[1 | (pod << 2) | (extent << 8)], "");
        let dims = [extent as u64];
        attrs.push(self.w.attribute(&format!("{name}.smp0"), t, (extent > 1).then_some(&dims[..]), raw));
        P { attrs, link: None }
    }

    fn compound_group(&mut self, props: Vec<P>) -> u64 {
        let attrs: Vec<Vec<u8>> = props.iter().flat_map(|p| p.attrs.clone()).collect();
        let links: Vec<(String, u64)> = props.into_iter().filter_map(|p| p.link).collect();
        self.w.group(&links, &attrs)
    }

    fn compound(&mut self, name: &str, meta: &str, props: Vec<P>) -> P {
        let g = self.compound_group(props);
        P { attrs: self.header(name, &[0], meta), link: Some((name.to_string(), g)) }
    }

    /// An object group: its own metadata, properties and child objects (name, group).
    fn object(&mut self, meta: &str, props: Vec<P>, children: &[(&str, u64)]) -> u64 {
        let props = self.compound_group(props);
        let mut links = vec![(".prop".to_string(), props)];
        links.extend(children.iter().map(|(n, g)| (n.to_string(), *g)));
        let attrs = self.header(".prop", &[0], meta);
        self.w.group(&links, &attrs)
    }

    fn archive(mut self, top: u64) -> Vec<u8> {
        let attrs = vec![
            self.w.attribute("abc_version", Ty::I32, None, &1i32.to_le_bytes()),
            self.w.attribute("abc_release_version", Ty::I32, None, &10709i32.to_le_bytes()),
        ];
        let root = self.w.group(&[("ABC".into(), top)], &attrs);
        self.w.finish(root)
    }
}

fn f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}
fn i32s(v: &[i32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// The scene of the Ogawa tests' sample file: a two-quad strip (face sets "left" / "right",
/// indexed face-varying UVs, vertex normals) under an Xform translated by (1, 2, 3) and scaled
/// by 2, plus a lone triangle under the top object.
fn sample_file(style: Style, with_tri: bool) -> Vec<u8> {
    let mut a = Abc::new(style);
    let p = f32s(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 2.0, 1.0, 0.0]);
    let pp = a.array("P", "interpretation=point", F32, 3, Ty::F32, &p);
    let fi = a.array(".faceIndices", "", I32, 1, Ty::I32, &i32s(&[0, 3, 4, 1, 1, 4, 5, 2]));
    let fc = a.array(".faceCounts", "", I32, 1, Ty::I32, &i32s(&[4, 4]));
    let vals = a.array(".vals", "", F32, 2, Ty::F32, &f32s(&[0.0, 0.0, 0.5, 0.0, 1.0, 0.0, 0.0, 1.0, 0.5, 1.0, 1.0, 1.0]));
    let uvi = a.array(".indices", "", U32, 1, Ty::U32, &i32s(&[0, 3, 4, 1, 1, 4, 5, 2]));
    let uv = a.compound("uv", "geoScope=fvr;podName=float32_t;podExtent=2", vec![vals, uvi]);
    let n = a.array("N", "geoScope=vtx", F32, 3, Ty::F32, &f32s(&[0.0, 0.0, 1.0].repeat(6)));
    let geom = a.compound(".geom", "schema=AbcGeom_PolyMesh_v1", vec![pp, fi, fc, uv, n]);
    let fs_meta = "schema=AbcGeom_FaceSet_v1;schemaObjTitle=AbcGeom_FaceSet_v1:.faceset";
    let left_faces = a.array(".faces", "", I32, 1, Ty::I32, &i32s(&[0]));
    let left_fs = a.compound(".faceset", "schema=AbcGeom_FaceSet_v1", vec![left_faces]);
    let left = a.object(fs_meta, vec![left_fs], &[]);
    let right_faces = a.array(".faces", "", I32, 1, Ty::I32, &i32s(&[1]));
    let right_fs = a.compound(".faceset", "schema=AbcGeom_FaceSet_v1", vec![right_faces]);
    let right = a.object(fs_meta, vec![right_fs], &[]);
    let mesh_meta = "schema=AbcGeom_PolyMesh_v1;schemaObjTitle=AbcGeom_PolyMesh_v1:.geom";
    let mesh = a.object(mesh_meta, vec![geom], &[("left", left), ("right", right)]);
    let ops = a.scalar(".ops", U8, 2, Ty::U8, &[0x10, 0x00]);
    let xv: Vec<u8> = [1.0f64, 2.0, 3.0, 2.0, 2.0, 2.0].iter().flat_map(|x| x.to_le_bytes()).collect();
    let xvals = a.scalar(".vals", F64, 6, Ty::F64, &xv);
    let inherits = a.scalar(".inherits", 0, 1, Ty::U8, &[1]);
    let xform = a.compound(".xform", "schema=AbcGeom_Xform_v3", vec![ops, xvals, inherits]);
    let xf = a.object("schema=AbcGeom_Xform_v3", vec![xform], &[("stripShape", mesh)]);
    let tp = a.array("P", "", F32, 3, Ty::F32, &f32s(&[0.0, 0.0, 5.0, 0.0, 1.0, 5.0, 1.0, 0.0, 5.0]));
    let ti = a.array(".faceIndices", "", I32, 1, Ty::I32, &i32s(&[0, 1, 2]));
    let tc = a.array(".faceCounts", "", I32, 1, Ty::I32, &i32s(&[3]));
    let tgeom = a.compound(".geom", "schema=AbcGeom_PolyMesh_v1", vec![tp, ti, tc]);
    let tri = a.object("schema=AbcGeom_PolyMesh_v1", vec![tgeom], &[]);
    let mut children = vec![("strip", xf), ("tri", tri)];
    children.truncate(if with_tri { 2 } else { 1 });
    let top = a.object("", vec![], &children);
    a.archive(top)
}

/// Positions, triangles per (material, part), UVs and normals, for comparing imports.
type Summary = (Vec<Vec3>, Vec<(String, String, Vec<u32>)>, Option<Vec<texturecraft_geom::glam::Vec2>>, Option<Vec<Vec3>>);

fn summary(raw: RawMesh) -> Summary {
    (raw.positions, raw.groups, raw.uvs, raw.normals)
}

#[test]
fn hdf5_files_import_like_their_ogawa_twin() {
    for with_tri in [true, false] {
        let ogawa = summary(read(&super::tests::sample_file(with_tri)).unwrap());
        for style in STYLES {
            let bytes = sample_file(style, with_tri);
            let raw = read(&bytes).unwrap_or_else(|e| panic!("{style:?}: {e}"));
            assert_eq!(summary(raw), ogawa, "{style:?} with_tri={with_tri}");
        }
    }
}

#[test]
fn hdf5_strip_keeps_uvs_normals_and_face_sets() {
    for style in STYLES {
        let raw = read(&sample_file(style, false)).unwrap();
        let mut names: Vec<(&str, &str)> = raw.groups.iter().map(|(m, p, _)| (m.as_str(), p.as_str())).collect();
        names.sort();
        assert_eq!(names, vec![("left", "strip"), ("right", "strip")], "{style:?}");
        assert!(raw.normals.is_some(), "{style:?}");
        assert_eq!(raw.triangle_count(), 4);
        let imported = finish(raw).unwrap();
        assert!(!imported.report.uvs_generated, "{style:?}");
        for (p, uv) in imported.mesh.positions.iter().zip(&imported.mesh.uvs) {
            let local = (*p - Vec3::new(1.0, 2.0, 3.0)) / 2.0;
            assert!((uv.x - local.x / 2.0).abs() < 1e-6 && (uv.y - (1.0 - local.y)).abs() < 1e-6, "{style:?} {p} {uv}");
        }
    }
}

#[test]
fn hdf5_triangle_imports_through_the_public_entry_point() {
    let mut a = Abc::new(Style::Compact);
    let pp = a.array("P", "", F32, 3, Ty::F32, &f32s(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0]));
    let fi = a.array(".faceIndices", "", I32, 1, Ty::I32, &i32s(&[0, 1, 2]));
    let fc = a.array(".faceCounts", "", I32, 1, Ty::I32, &i32s(&[3]));
    let uv = a.array("uv", "", F32, 2, Ty::F32, &f32s(&[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]));
    let geom = a.compound(".geom", "schema=AbcGeom_PolyMesh_v1", vec![pp, fi, fc, uv]);
    let mesh = a.object("schema=AbcGeom_PolyMesh_v1", vec![geom], &[]);
    let top = a.object("", vec![], &[("m", mesh)]);
    let imported = crate::import_mesh_bytes("m.abc", &a.archive(top)).unwrap();
    assert!(!imported.report.uvs_generated);
    for (p, uv) in imported.mesh.positions.iter().zip(&imported.mesh.uvs) {
        assert_eq!(*uv, texturecraft_geom::glam::Vec2::new(p.x, 1.0 - p.y));
    }
}

#[test]
fn hdf5_superblock_after_a_user_block_is_found() {
    // A 512-byte user block before the HDF5 data: addresses are relative to the base address.
    for style in [Style::Classic, Style::Compact] {
        let bytes = sample_file(style, true);
        let mut shifted = vec![b'u'; 512];
        shifted.extend_from_slice(&bytes);
        // The base address field (after the version 0 or version 2 superblock's fixed fields).
        let base = if style == Style::Classic { 24 } else { 12 };
        shifted[512 + base..512 + base + 8].copy_from_slice(&512u64.to_le_bytes());
        assert_eq!(summary(read(&shifted).unwrap()), summary(read(&bytes).unwrap()), "{style:?}");
    }
}

#[test]
fn hdf5_empty_file_and_damaged_headers_are_clear_errors() {
    let e = read(b"\x89HDF\r\n\x1a\n rest of a damaged file").err().unwrap().to_string();
    assert!(e.contains("HDF5"), "{e}");
    // A valid HDF5 file without an Alembic archive.
    let mut w = W::new(Style::Compact);
    let root = w.group(&[], &[]);
    let e = read(&w.finish(root)).err().unwrap().to_string();
    assert!(e.contains("Alembic"), "{e}");
}

#[test]
fn hdf5_truncated_and_corrupted_files_never_panic() {
    for style in STYLES {
        let bytes = sample_file(style, true);
        for n in (0..bytes.len()).step_by(7) {
            let _ = read(&bytes[..n]);
        }
        // The root header is written last, so truncation alone stops early: punch holes too.
        for n in (0..bytes.len()).step_by(5) {
            let mut b = bytes.clone();
            let end = (n + 24).min(b.len());
            b[n..end].fill(0xff);
            let _ = read(&b);
        }
        let mut rng = 0x9e37_79b9_7f4a_7c15u64;
        for i in 0..6000 {
            let mut b = bytes.clone();
            for _ in 0..4 {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                let i = (rng % b.len() as u64) as usize;
                b[i] = (rng >> 32) as u8;
            }
            // Finishing (welding, unwrapping) is slow: run it on every 50th file that reads.
            if let Ok(raw) = read(&b)
                && i % 50 == 0
            {
                let _ = finish(raw);
            }
        }
    }
}

/// Replaces every [`PLACEHOLDER`] in `b` with `addr`.
fn patch_placeholder(b: &mut [u8], addr: u64) {
    let pat = PLACEHOLDER.to_le_bytes();
    let mut i = 0;
    while i + 8 <= b.len() {
        if b[i..i + 8] == pat {
            b[i..i + 8].copy_from_slice(&addr.to_le_bytes());
        }
        i += 1;
    }
}

#[test]
fn hdf5_group_linking_to_itself_is_bounded() {
    for style in STYLES {
        let mut a = Abc::new(style);
        // The top object lists itself as its child "loop" (twice: exponential without limits).
        let props = a.compound_group(vec![]);
        let attrs = a.header(".prop", &[0], "");
        let links = vec![(".prop".to_string(), props), ("loop".to_string(), PLACEHOLDER), ("again".to_string(), PLACEHOLDER)];
        let top = a.w.group(&links, &attrs);
        let mut bytes = a.archive(top);
        patch_placeholder(&mut bytes, top);
        let e = read(&bytes).err().unwrap_or_else(|| panic!("{style:?} read a looping file")).to_string();
        assert!(e.contains("nested") || e.contains("too"), "{style:?}: {e}");
    }
}

#[test]
fn hdf5_continuation_pointing_at_itself_is_bounded() {
    for style in STYLES {
        let mut w = W::new(style);
        let mut cont = PLACEHOLDER.to_le_bytes().to_vec();
        cont.extend(64u64.to_le_bytes());
        let msgs = vec![(5u16, vec![2, 2, 2, 0]), (0x10u16, cont)];
        let at = w.header(&msgs);
        let mut bytes = w.finish(at);
        patch_placeholder(&mut bytes, at);
        assert!(read(&bytes).is_err(), "{style:?}");
    }
}

#[test]
fn hdf5_huge_counts_are_refused_without_allocating() {
    for style in STYLES {
        let mut a = Abc::new(style);
        a.w.fake_count = Some(1 << 60);
        let pp = a.array("P", "", F32, 3, Ty::F32, &f32s(&[0.0; 9]));
        a.w.fake_count = None;
        let fi = a.array(".faceIndices", "", I32, 1, Ty::I32, &i32s(&[0, 1, 2]));
        let fc = a.array(".faceCounts", "", I32, 1, Ty::I32, &i32s(&[3]));
        let geom = a.compound(".geom", "schema=AbcGeom_PolyMesh_v1", vec![pp, fi, fc]);
        let mesh = a.object("schema=AbcGeom_PolyMesh_v1", vec![geom], &[]);
        let top = a.object("", vec![], &[("m", mesh)]);
        assert!(read(&a.archive(top)).is_err(), "{style:?}");
    }
}

#[test]
fn hdf5_bad_offsets_are_errors() {
    for style in STYLES {
        let bytes = sample_file(style, true);
        // Root object header address past the end of the file.
        let mut b = bytes.clone();
        let root_field = if style == Style::Classic { 64 } else { 8 + 4 + 24 };
        b[root_field..root_field + 8].copy_from_slice(&(1u64 << 40).to_le_bytes());
        assert!(read(&b).is_err(), "{style:?}");
        // Every 8-byte aligned word that holds an in-file address pointed past the end in turn.
        let len = bytes.len() as u64;
        for i in (0..bytes.len() - 8).step_by(8) {
            let v = u64::from_le_bytes(bytes[i..i + 8].try_into().unwrap());
            if (128..len).contains(&v) {
                let mut b = bytes.clone();
                b[i..i + 8].copy_from_slice(&(len + 8).to_le_bytes());
                if let Ok(raw) = read(&b) {
                    let _ = finish(raw);
                }
            }
        }
    }
}

#[test]
fn hdf5_many_links_and_attributes_keep_creation_order() {
    // Twelve links and twenty attributes: the dense style needs an indirect heap root and a
    // B-tree with an internal node for them.
    for style in STYLES {
        let mut w = W::new(style);
        let leaf = w.group(&[], &[]);
        let links: Vec<(String, u64)> = (0..12).map(|i| (format!("child{:02}", 11 - i), leaf)).collect();
        let attrs: Vec<Vec<u8>> = (0..20i32).map(|i| w.attribute(&format!("attribute number {i}"), Ty::I32, None, &i.to_le_bytes())).collect();
        let g = w.group(&links, &attrs);
        let bytes = w.finish(g);
        let mut h = super::hdf5::H5::open(&bytes).unwrap();
        let got: Vec<String> = h.links(h.root).unwrap().into_iter().map(|l| l.name).collect();
        let mut want: Vec<String> = links.iter().map(|l| l.0.clone()).collect();
        if style == Style::Classic {
            // Symbol tables keep names in name order; creation order is not recorded.
            want.sort();
        }
        assert_eq!(got, want, "{style:?}");
        let attrs = h.attributes(h.root).unwrap();
        let names: Vec<&str> = attrs.iter().map(|a| a.name.as_str()).collect();
        let want: Vec<String> = (0..20).map(|i| format!("attribute number {i}")).collect();
        assert_eq!(names, want, "{style:?}");
        for (i, a) in attrs.iter().enumerate() {
            assert_eq!(super::hdf5::numbers(a.dtype, a.data).unwrap(), vec![i as f64], "{style:?}");
        }
    }
}
