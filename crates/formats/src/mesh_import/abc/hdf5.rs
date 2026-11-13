//! A clean-room, read-only reader of the HDF5 subset Alembic writers use, written from the
//! public HDF5 File Format Specification (version 3.0, which also describes the older
//! structures):
//!
//! - superblock versions 0 to 3 (found at offset 0, 512, 1024, ...; addresses are relative to
//!   the base address);
//! - object headers version 1 and version 2 (`OHDR`), with continuation blocks (`OCHK`);
//! - groups stored as symbol tables (version 1 B-tree of type 0, symbol table nodes, local
//!   heap), as link messages, or densely (link info message: fractal heap and version 2 B-tree
//!   of type 5);
//! - attribute messages versions 1 to 3, stored in the header or densely (attribute info
//!   message: fractal heap and version 2 B-tree of type 8);
//! - dataspaces versions 1 and 2; fixed-point, floating-point and fixed-length string
//!   datatypes;
//! - compact, contiguous and chunked dataset layouts (versions 1 to 3 with a version 1 B-tree of
//!   type 1; version 4 with a single chunk), with the deflate, shuffle and Fletcher-32 filters.
//!
//! Refused with a clear error: shared (committed) messages, filtered or huge fractal heap
//! objects, external and virtual storage, other chunk indexes and filters, variable-length and
//! compound datatypes. Checksums are not verified. Soft and external links are skipped.
//!
//! Every offset, size and count read from the file is checked before use; tree depth, the
//! number of structures visited and the bytes decoded are capped, and continuation blocks are
//! followed once each, so a damaged or hostile file gives an error, never a panic or a hang.

use std::borrow::Cow;
use std::collections::{HashSet, VecDeque};
use std::io::Read as _;

use super::MAX_DECODED_BYTES;
use crate::{FormatError, Result, malformed};

const KIND: &str = "Alembic (HDF5)";
const SIGNATURE: &[u8] = b"\x89HDF\r\n\x1a\n";
/// Header messages, tree and heap nodes, chunks and continuation blocks visited: bounds the
/// work a file whose structures point at each other many times over can cause.
const MAX_STEPS: usize = 4_000_000;
/// Deepest B-tree or fractal heap nesting followed.
const MAX_TREE_DEPTH: usize = 32;
/// Most dimensions of a dataspace.
const MAX_RANK: usize = 32;
/// Most continuation blocks of one object header.
const MAX_CONTINUATIONS: usize = 4096;
/// Most records of one version 2 B-tree.
const MAX_RECORDS: u64 = 1 << 22;

fn bad(msg: impl std::fmt::Display) -> FormatError {
    malformed(KIND, msg)
}

fn unsupported(what: impl std::fmt::Display) -> FormatError {
    FormatError::Unsupported(format!("HDF5 Alembic file with {what}"))
}

/// Where the superblock signature is: offset 0, 512, 1024, 2048, ...
pub(super) fn superblock_at(b: &[u8]) -> Option<usize> {
    let mut at = 0usize;
    loop {
        if b.get(at..at.checked_add(SIGNATURE.len())?)? == SIGNATURE {
            return Some(at);
        }
        at = if at == 0 { 512 } else { at.checked_mul(2)? };
    }
}

/// Little-endian reads from a slice of the file.
struct Rd<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Rd<'a> {
    fn new(b: &'a [u8]) -> Rd<'a> {
        Rd { b, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self.pos.checked_add(n).and_then(|end| self.b.get(self.pos..end)).ok_or_else(|| bad("a structure ends early"))?;
        self.pos += n;
        Ok(s)
    }

    fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?.first().copied().unwrap_or(0))
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(self.uint(2)? as u16)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(self.uint(4)? as u32)
    }

    /// An unsigned little-endian number of `n` bytes (at most 8).
    fn uint(&mut self, n: usize) -> Result<u64> {
        if n > 8 {
            return Err(bad(format!("a {n}-byte number")));
        }
        Ok(self.take(n)?.iter().rev().fold(0u64, |v, b| (v << 8) | *b as u64))
    }

    fn usize(&mut self, n: usize) -> Result<usize> {
        usize::try_from(self.uint(n)?).map_err(|_| bad("a size out of range"))
    }

    fn left(&self) -> usize {
        self.b.len().saturating_sub(self.pos)
    }
}

/// Fewest bytes that encode `n` (at least one).
fn enc_size(n: u64) -> usize {
    if n == 0 { 1 } else { (63 - n.leading_zeros() as usize) / 8 + 1 }
}

/// A header message: type, flags, attribute creation order (version 2 headers) and data.
#[derive(Clone, Copy)]
pub(super) struct Msg<'a> {
    kind: u16,
    flags: u8,
    order: Option<u16>,
    data: &'a [u8],
}

impl Msg<'_> {
    /// Stored elsewhere (a shared or committed message): not read.
    fn shared(&self) -> bool {
        self.flags & 0x02 != 0
    }
}

/// An element type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Dtype {
    Int { size: usize, signed: bool, big_endian: bool },
    Float { size: usize, big_endian: bool },
    Str { size: usize },
    Other { class: u8, size: usize },
}

impl Dtype {
    fn size(&self) -> usize {
        match *self {
            Dtype::Int { size, .. } | Dtype::Float { size, .. } | Dtype::Str { size } | Dtype::Other { size, .. } => size,
        }
    }

    fn parse(data: &[u8]) -> Result<Dtype> {
        let mut r = Rd::new(data);
        let class_version = r.u8()?;
        let bits = r.take(3)?;
        let size = r.usize(4)?;
        if size == 0 {
            return Err(bad("a datatype of size 0"));
        }
        let b0 = bits.first().copied().unwrap_or(0);
        Ok(match class_version & 0x0f {
            0 => Dtype::Int { size, signed: b0 & 0x08 != 0, big_endian: b0 & 0x01 != 0 },
            // Byte order is bits 0 and 6: 00 little-endian, 01 big-endian, 11 VAX.
            1 if b0 & 0x40 == 0 => Dtype::Float { size, big_endian: b0 & 0x01 != 0 },
            3 => Dtype::Str { size },
            class => Dtype::Other { class, size },
        })
    }
}

/// A dataspace.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Space {
    Scalar,
    Simple(Vec<u64>),
    Null,
}

impl Space {
    fn parse(data: &[u8], sl: usize) -> Result<Space> {
        let mut r = Rd::new(data);
        let version = r.u8()?;
        let rank = r.u8()? as usize;
        let _flags = r.u8()?;
        let kind = match version {
            1 => {
                r.skip(5)?;
                if rank == 0 { 0 } else { 1 }
            }
            2 => r.u8()?,
            v => return Err(unsupported(format!("dataspace version {v}"))),
        };
        if rank > MAX_RANK {
            return Err(bad(format!("a dataspace of {rank} dimensions")));
        }
        let dims = (0..rank).map(|_| r.uint(sl)).collect::<Result<Vec<u64>>>()?;
        Ok(match kind {
            0 => Space::Scalar,
            1 => Space::Simple(dims),
            2 => Space::Null,
            k => return Err(bad(format!("dataspace type {k}"))),
        })
    }

    /// Number of elements, `None` if it overflows.
    fn count(&self) -> Option<u64> {
        match self {
            Space::Scalar => Some(1),
            Space::Null => Some(0),
            Space::Simple(d) => d.iter().try_fold(1u64, |n, d| n.checked_mul(*d)),
        }
    }
}

/// An attribute: name, type, shape, raw data and creation order (when the file records it).
pub(super) struct Attr<'a> {
    pub name: String,
    pub dtype: Dtype,
    pub data: &'a [u8],
    order: Option<u64>,
}

/// A hard link of a group.
pub(super) struct Link {
    pub name: String,
    pub addr: u64,
    order: Option<u64>,
}

/// The raw values of a dataset.
pub(super) struct Dataset<'a> {
    pub dtype: Dtype,
    pub data: Cow<'a, [u8]>,
}

/// A parsed fractal heap header (the parts needed to find managed and tiny objects).
struct Heap {
    id_len: usize,
    width: u64,
    start: u64,
    max_direct: u64,
    off_bytes: usize,
    len_bytes: usize,
    root: Option<u64>,
    rows: u64,
    max_direct_rows: u64,
}

/// Data layout of a dataset.
enum Layout<'a> {
    Compact(&'a [u8]),
    Contiguous(Option<u64>),
    /// Chunk dimensions (rank + 1 entries, the last one the element size, may be absent) and
    /// the chunk index.
    Chunked {
        dims: Vec<u64>,
        index: ChunkIndex,
    },
}

enum ChunkIndex {
    /// Version 1 B-tree of type 1.
    BTree(Option<u64>),
    /// One chunk: address, stored size (filtered) and filter mask.
    Single { addr: Option<u64>, size: Option<u64>, mask: u32 },
}

/// The HDF5 file.
pub(super) struct H5<'a> {
    b: &'a [u8],
    base: u64,
    /// Size of offsets and of lengths.
    so: usize,
    sl: usize,
    pub root: u64,
    steps: usize,
    budget: usize,
}

impl<'a> H5<'a> {
    pub(super) fn open(b: &'a [u8]) -> Result<H5<'a>> {
        let at = superblock_at(b).ok_or_else(|| bad("no HDF5 signature"))?;
        let mut r = Rd::new(b.get(at..).unwrap_or_default());
        r.skip(SIGNATURE.len())?;
        let version = r.u8()?;
        let mut h = H5 { b, base: 0, so: 8, sl: 8, root: 0, steps: 0, budget: MAX_DECODED_BYTES };
        let check = |so: usize, sl: usize| {
            if [2, 4, 8].contains(&so) && [2, 4, 8].contains(&sl) { Ok(()) } else { Err(unsupported(format!("{so}-byte offsets and {sl}-byte lengths"))) }
        };
        match version {
            0 | 1 => {
                r.skip(4)?;
                let (so, sl) = (r.u8()? as usize, r.u8()? as usize);
                check(so, sl)?;
                (h.so, h.sl) = (so, sl);
                r.skip(1 + 2 + 2 + 4)?;
                if version == 1 {
                    r.skip(4)?;
                }
                h.base = r.uint(so)?;
                r.skip(3 * so)?;
                // Root group symbol table entry: link name offset, object header address.
                r.skip(so)?;
                h.root = h.addr(&mut r)?.ok_or_else(|| bad("the root group has no address"))?;
            }
            2 | 3 => {
                let (so, sl) = (r.u8()? as usize, r.u8()? as usize);
                check(so, sl)?;
                (h.so, h.sl) = (so, sl);
                r.skip(1)?;
                h.base = r.uint(so)?;
                r.skip(2 * so)?;
                h.root = h.addr(&mut r)?.ok_or_else(|| bad("the root group has no address"))?;
            }
            v => return Err(unsupported(format!("superblock version {v}"))),
        }
        Ok(h)
    }

    /// A file address, `None` for the undefined address (all bits set).
    fn addr(&self, r: &mut Rd) -> Result<Option<u64>> {
        let v = r.uint(self.so)?;
        let undefined = if self.so >= 8 { u64::MAX } else { (1u64 << (8 * self.so)) - 1 };
        Ok((v != undefined).then_some(v))
    }

    fn len(&self, r: &mut Rd) -> Result<u64> {
        r.uint(self.sl)
    }

    /// File bytes from address `addr` to the end.
    fn from(&self, addr: u64) -> Result<&'a [u8]> {
        self.base
            .checked_add(addr)
            .and_then(|a| usize::try_from(a).ok())
            .and_then(|a| self.b.get(a..))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| bad(format!("address {addr} is past the end of the file")))
    }

    /// `len` file bytes at address `addr`.
    fn at(&self, addr: u64, len: u64) -> Result<&'a [u8]> {
        let s = self.from(addr)?;
        usize::try_from(len).ok().and_then(|n| s.get(..n)).ok_or_else(|| bad(format!("{len} bytes at address {addr} overrun the file")))
    }

    fn step(&mut self) -> Result<()> {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return Err(FormatError::TooLarge("the HDF5 file's structures point at each other too often".into()));
        }
        Ok(())
    }

    pub(super) fn charge(&mut self, bytes: usize) -> Result<()> {
        self.budget =
            self.budget.checked_sub(bytes).ok_or_else(|| FormatError::TooLarge(format!("the Alembic file decodes to more than {MAX_DECODED_BYTES} bytes")))?;
        Ok(())
    }

    /// The messages of the object header at `addr`, continuation blocks included.
    fn header(&mut self, addr: u64) -> Result<Vec<Msg<'a>>> {
        self.step()?;
        let start = self.from(addr)?;
        let mut msgs = Vec::new();
        let mut queue = VecDeque::new();
        let v2 = start.starts_with(b"OHDR");
        let mut r = Rd::new(start);
        let mut creation_order = false;
        if v2 {
            r.skip(4)?;
            let version = r.u8()?;
            if version != 2 {
                return Err(unsupported(format!("object header version {version}")));
            }
            let flags = r.u8()?;
            creation_order = flags & 0x04 != 0;
            if flags & 0x20 != 0 {
                r.skip(16)?;
            }
            if flags & 0x10 != 0 {
                r.skip(4)?;
            }
            let size = r.usize(1 << (flags & 0x03))?;
            let body = r.take(size)?;
            self.messages_v2(body, creation_order, &mut msgs, &mut queue)?;
        } else {
            let version = r.u8()?;
            if version != 1 {
                return Err(bad(format!("no object header at address {addr}")));
            }
            r.skip(1 + 2 + 4)?;
            let size = r.usize(4)?;
            r.skip(4)?;
            let body = r.take(size)?;
            self.messages_v1(body, &mut msgs, &mut queue)?;
        }
        let mut seen = HashSet::new();
        while let Some((at, len)) = queue.pop_front() {
            if !seen.insert(at) || seen.len() > MAX_CONTINUATIONS {
                return Err(bad(format!("the object header at {addr} continues in a loop")));
            }
            let block = self.at(at, len)?;
            if v2 {
                let body = block
                    .strip_prefix(b"OCHK")
                    .and_then(|b| b.get(..b.len().checked_sub(4)?))
                    .ok_or_else(|| bad(format!("no continuation block at address {at}")))?;
                self.messages_v2(body, creation_order, &mut msgs, &mut queue)?;
            } else {
                self.messages_v1(block, &mut msgs, &mut queue)?;
            }
        }
        Ok(msgs)
    }

    fn continuation(&self, data: &[u8], queue: &mut VecDeque<(u64, u64)>) -> Result<()> {
        let mut r = Rd::new(data);
        let at = self.addr(&mut r)?.ok_or_else(|| bad("a continuation to the undefined address"))?;
        let len = self.len(&mut r)?;
        queue.push_back((at, len));
        Ok(())
    }

    /// Version 1 messages: type (2), size (2), flags (1), reserved (3), data; 8-byte aligned.
    fn messages_v1(&mut self, body: &'a [u8], msgs: &mut Vec<Msg<'a>>, queue: &mut VecDeque<(u64, u64)>) -> Result<()> {
        let mut r = Rd::new(body);
        while r.left() >= 8 {
            self.step()?;
            let kind = r.u16()?;
            let size = r.u16()? as usize;
            let flags = r.u8()?;
            r.skip(3)?;
            let data = r.take(size)?;
            r.pos = r.pos.next_multiple_of(8).min(body.len());
            match kind {
                0 => {}
                0x10 => self.continuation(data, queue)?,
                _ => msgs.push(Msg { kind, flags, order: None, data }),
            }
        }
        Ok(())
    }

    /// Version 2 messages: type (1), size (2), flags (1), creation order (2, if tracked), data.
    fn messages_v2(&mut self, body: &'a [u8], creation_order: bool, msgs: &mut Vec<Msg<'a>>, queue: &mut VecDeque<(u64, u64)>) -> Result<()> {
        let prefix = if creation_order { 6 } else { 4 };
        let mut r = Rd::new(body);
        // Fewer bytes than a message prefix left: the gap before the checksum.
        while r.left() >= prefix {
            self.step()?;
            let kind = r.u8()? as u16;
            let size = r.u16()? as usize;
            let flags = r.u8()?;
            let order = if creation_order { Some(r.u16()?) } else { None };
            let data = r.take(size)?;
            match kind {
                0 => {}
                0x10 => self.continuation(data, queue)?,
                _ => msgs.push(Msg { kind, flags, order, data }),
            }
        }
        Ok(())
    }

    /// The hard links of the group at `addr`, in creation order when the file records it (else
    /// as stored: name order for symbol tables).
    pub(super) fn links(&mut self, addr: u64) -> Result<Vec<Link>> {
        let mut out = Vec::new();
        for m in self.header(addr)? {
            match m.kind {
                0x11 => self.symbol_table(m.data, &mut out)?,
                0x06 => out.extend(self.link(m.data)?),
                0x02 => {
                    let mut r = Rd::new(m.data);
                    let _version = r.u8()?;
                    let flags = r.u8()?;
                    if flags & 0x01 != 0 {
                        r.skip(8)?;
                    }
                    let heap = self.addr(&mut r)?;
                    let names = self.addr(&mut r)?;
                    if let (Some(heap), Some(names)) = (heap, names) {
                        let heap = self.heap(heap)?;
                        for rec in self.btree2(names, 5)? {
                            let id = rec.get(4..).unwrap_or_default();
                            let obj = self.heap_object(&heap, id)?;
                            out.extend(self.link(obj)?);
                        }
                    }
                }
                _ => {}
            }
        }
        out.sort_by_key(|l| l.order.unwrap_or(u64::MAX));
        Ok(out)
    }

    /// A link message; `None` for soft, external and user-defined links.
    fn link(&self, data: &[u8]) -> Result<Option<Link>> {
        let mut r = Rd::new(data);
        let version = r.u8()?;
        if version != 1 {
            return Err(unsupported(format!("link message version {version}")));
        }
        let flags = r.u8()?;
        let kind = if flags & 0x08 != 0 { r.u8()? } else { 0 };
        let order = if flags & 0x04 != 0 { Some(r.uint(8)?) } else { None };
        if flags & 0x10 != 0 {
            r.skip(1)?;
        }
        let n = r.usize(1 << (flags & 0x03))?;
        let name = String::from_utf8_lossy(r.take(n)?).into_owned();
        if kind != 0 {
            return Ok(None);
        }
        let addr = self.addr(&mut r)?.ok_or_else(|| bad(format!("link \"{name}\" points at the undefined address")))?;
        Ok(Some(Link { name, addr, order }))
    }

    /// Links of a symbol-table group: a version 1 B-tree of symbol table nodes, names in a
    /// local heap.
    fn symbol_table(&mut self, data: &[u8], out: &mut Vec<Link>) -> Result<()> {
        let mut r = Rd::new(data);
        let tree = self.addr(&mut r)?;
        let heap = self.addr(&mut r)?.ok_or_else(|| bad("a group without a local heap"))?;
        let mut h = Rd::new(self.from(heap)?);
        if h.take(4)? != b"HEAP" {
            return Err(bad(format!("no local heap at address {heap}")));
        }
        h.skip(4)?;
        let size = self.len(&mut h)?;
        let _free = self.len(&mut h)?;
        let seg_at = self.addr(&mut h)?.ok_or_else(|| bad("a local heap without data"))?;
        let segment = self.at(seg_at, size)?;
        if let Some(tree) = tree {
            self.group_node(tree, None, 0, segment, out)?;
        }
        Ok(())
    }

    /// A version 1 B-tree node of type 0 at `addr` (`level` when its parent knows it).
    fn group_node(&mut self, addr: u64, level: Option<u8>, depth: usize, names: &[u8], out: &mut Vec<Link>) -> Result<()> {
        self.step()?;
        if depth > MAX_TREE_DEPTH {
            return Err(bad("a group B-tree is nested too deeply"));
        }
        let mut r = Rd::new(self.from(addr)?);
        if r.take(4)? != b"TREE" || r.u8()? != 0 {
            return Err(bad(format!("no group B-tree node at address {addr}")));
        }
        let node_level = r.u8()?;
        if level.is_some_and(|l| l != node_level) {
            return Err(bad("group B-tree levels do not descend"));
        }
        let entries = r.u16()? as usize;
        r.skip(2 * self.so)?;
        r.skip(self.sl)?;
        for _ in 0..entries {
            let child = self.addr(&mut r)?.ok_or_else(|| bad("a group B-tree child at the undefined address"))?;
            r.skip(self.sl)?;
            if node_level == 0 {
                self.symbol_node(child, names, out)?;
            } else {
                self.group_node(child, Some(node_level - 1), depth + 1, names, out)?;
            }
        }
        Ok(())
    }

    /// A symbol table node: entries of link name offset, object header address, cache type,
    /// reserved and scratch-pad.
    fn symbol_node(&mut self, addr: u64, names: &[u8], out: &mut Vec<Link>) -> Result<()> {
        self.step()?;
        let mut r = Rd::new(self.from(addr)?);
        if r.take(4)? != b"SNOD" {
            return Err(bad(format!("no symbol table node at address {addr}")));
        }
        r.skip(2)?;
        let count = r.u16()?;
        for _ in 0..count {
            self.step()?;
            let name_at = r.usize(self.so)?;
            let target = self.addr(&mut r)?;
            let cache = r.u32()?;
            r.skip(4 + 16)?;
            let name = names.get(name_at..).ok_or_else(|| bad("a link name past its local heap"))?;
            let name = String::from_utf8_lossy(name.split(|c| *c == 0).next().unwrap_or_default()).into_owned();
            // Cache type 2 is a soft link.
            if let (Some(addr), false) = (target, cache == 2) {
                out.push(Link { name, addr, order: None });
            }
        }
        Ok(())
    }

    /// The attributes of the object at `addr`, in creation order when the file records it.
    pub(super) fn attributes(&mut self, addr: u64) -> Result<Vec<Attr<'a>>> {
        let mut out = Vec::new();
        for m in self.header(addr)? {
            match m.kind {
                0x0c => {
                    if m.shared() {
                        return Err(unsupported("shared attributes"));
                    }
                    let mut a = self.attribute(m.data)?;
                    a.order = m.order.map(u64::from);
                    out.push(a);
                }
                0x15 => {
                    let mut r = Rd::new(m.data);
                    let _version = r.u8()?;
                    let flags = r.u8()?;
                    if flags & 0x01 != 0 {
                        r.skip(2)?;
                    }
                    let heap = self.addr(&mut r)?;
                    let names = self.addr(&mut r)?;
                    if let (Some(heap), Some(names)) = (heap, names) {
                        let heap = self.heap(heap)?;
                        for rec in self.btree2(names, 8)? {
                            let mut rr = Rd::new(rec);
                            let id = rr.take(8)?;
                            let flags = rr.u8()?;
                            let order = rr.u32()?;
                            if flags & 0x02 != 0 {
                                return Err(unsupported("shared attributes"));
                            }
                            let obj = self.heap_object(&heap, id)?;
                            let mut a = self.attribute(obj)?;
                            a.order = Some(order as u64);
                            out.push(a);
                        }
                    }
                }
                _ => {}
            }
        }
        out.sort_by_key(|a| a.order.unwrap_or(u64::MAX));
        Ok(out)
    }

    /// An attribute message (versions 1 to 3).
    fn attribute(&self, data: &'a [u8]) -> Result<Attr<'a>> {
        let mut r = Rd::new(data);
        let version = r.u8()?;
        let flags = r.u8()?;
        if !(1..=3).contains(&version) {
            return Err(unsupported(format!("attribute message version {version}")));
        }
        if version > 1 && flags & 0x03 != 0 {
            return Err(unsupported("attributes with shared types"));
        }
        let name_size = r.u16()? as usize;
        let type_size = r.u16()? as usize;
        let space_size = r.u16()? as usize;
        if version == 3 {
            r.skip(1)?;
        }
        // Version 1 pads the name, datatype and dataspace to 8 bytes each.
        let padded = |n: usize| if version == 1 { n.next_multiple_of(8) } else { n };
        let name = r.take(padded(name_size))?.get(..name_size).unwrap_or_default();
        let name = String::from_utf8_lossy(name.split(|c| *c == 0).next().unwrap_or_default()).into_owned();
        let dtype = Dtype::parse(r.take(padded(type_size))?.get(..type_size).unwrap_or_default())?;
        let space = Space::parse(r.take(padded(space_size))?.get(..space_size).unwrap_or_default(), self.sl)?;
        let bytes = space
            .count()
            .and_then(|n| n.checked_mul(dtype.size() as u64))
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n <= r.left())
            .ok_or_else(|| bad(format!("attribute \"{name}\" claims more data than it holds")))?;
        let data = r.take(bytes)?;
        Ok(Attr { name, dtype, data, order: None })
    }

    /// The fractal heap header at `addr`.
    fn heap(&mut self, addr: u64) -> Result<Heap> {
        self.step()?;
        let mut r = Rd::new(self.from(addr)?);
        if r.take(4)? != b"FRHP" {
            return Err(bad(format!("no fractal heap at address {addr}")));
        }
        r.skip(1)?;
        let id_len = r.u16()? as usize;
        let filters = r.u16()?;
        if filters != 0 {
            return Err(unsupported("filtered fractal heaps"));
        }
        r.skip(1)?;
        let max_managed = r.u32()? as u64;
        r.skip(self.sl + self.so + self.sl + self.so + 8 * self.sl)?;
        let width = r.u16()? as u64;
        let start = self.len(&mut r)?;
        let max_direct = self.len(&mut r)?;
        let bits = r.u16()? as usize;
        let _start_rows = r.u16()?;
        let root = self.addr(&mut r)?;
        let rows = r.u16()? as u64;
        let power = |v: u64| v.is_power_of_two();
        if !power(width) || !power(start) || !power(max_direct) || start > max_direct || !(1..=64).contains(&bits) || rows > 64 || max_managed == 0 {
            return Err(bad(format!("the fractal heap at {addr} has an impossible doubling table")));
        }
        let log2 = |v: u64| v.trailing_zeros() as u64;
        Ok(Heap {
            id_len,
            width,
            start,
            max_direct,
            off_bytes: bits.div_ceil(8),
            len_bytes: (log2(max_direct) as usize).div_ceil(8).min(enc_size(max_managed)),
            root,
            rows,
            max_direct_rows: log2(max_direct) - log2(start) + 2,
        })
    }

    /// The object a heap ID names (managed or tiny).
    fn heap_object(&mut self, heap: &Heap, id: &'a [u8]) -> Result<&'a [u8]> {
        let mut r = Rd::new(id);
        let first = r.u8()?;
        match (first >> 4) & 0x03 {
            0 => {
                let off = r.uint(heap.off_bytes)?;
                let len = r.uint(heap.len_bytes)?;
                self.managed(heap, off, len)
            }
            2 => {
                let len = if heap.id_len > 18 { (((first as usize & 0x0f) << 8) | r.u8()? as usize) + 1 } else { (first as usize & 0x0f) + 1 };
                r.take(len)
            }
            1 => Err(unsupported("huge fractal heap objects")),
            _ => Err(bad("a heap ID of unknown type")),
        }
    }

    /// Bytes of each block in doubling-table row `row`.
    fn row_size(heap: &Heap, row: u64) -> Option<u64> {
        if row < 2 { Some(heap.start) } else { heap.start.checked_mul(1u64.checked_shl(u32::try_from(row - 1).ok()?)?) }
    }

    /// The managed object at heap offset `off` (`len` bytes).
    fn managed(&mut self, heap: &Heap, off: u64, len: u64) -> Result<&'a [u8]> {
        let root = heap.root.ok_or_else(|| bad("an object in an empty fractal heap"))?;
        if heap.rows == 0 {
            // The root is a direct block; offsets count from its start (header included).
            if off.checked_add(len).is_none_or(|end| end > heap.max_direct) {
                return Err(bad("a heap object past its direct block"));
            }
            return self.at(root.checked_add(off).ok_or_else(|| bad("heap offset out of range"))?, len);
        }
        let (mut block, mut rows, mut block_off) = (root, heap.rows, 0u64);
        for _ in 0..MAX_TREE_DEPTH {
            self.step()?;
            let mut r = Rd::new(self.from(block)?);
            if r.take(4)? != b"FHIB" {
                return Err(bad(format!("no fractal heap indirect block at address {block}")));
            }
            r.skip(1 + self.so + heap.off_bytes)?;
            let entries = r.pos;
            let direct_rows = rows.min(heap.max_direct_rows);
            let direct = direct_rows.checked_mul(heap.width).ok_or_else(|| bad("heap table too wide"))?;
            let mut pos = block_off;
            let mut found = None;
            for row in 0..rows {
                let size = H5::row_size(heap, row).ok_or_else(|| bad("heap block size out of range"))?;
                let row_bytes = size.checked_mul(heap.width).ok_or_else(|| bad("heap row out of range"))?;
                let Some(inside) = off.checked_sub(pos).filter(|d| *d < row_bytes) else {
                    pos = pos.checked_add(row_bytes).ok_or_else(|| bad("heap offset out of range"))?;
                    continue;
                };
                let col = inside / size;
                found = Some((row, row * heap.width + col, pos + col * size, size));
                break;
            }
            let (row, entry, entry_off, size) = found.ok_or_else(|| bad("a heap offset outside its heap"))?;
            let slot = |i: u64| -> Result<Option<u64>> {
                let at = (i as usize).checked_mul(self.so).and_then(|o| o.checked_add(entries)).ok_or_else(|| bad("heap entry out of range"))?;
                let mut rr = Rd::new(r.b);
                rr.pos = at;
                self.addr(&mut rr)
            };
            if row < heap.max_direct_rows {
                let addr = slot(entry)?.ok_or_else(|| bad("a heap object in an unallocated block"))?;
                let inner = off - entry_off;
                if inner.checked_add(len).is_none_or(|end| end > size) || size > heap.max_direct {
                    return Err(bad("a heap object past its direct block"));
                }
                return self.at(addr.checked_add(inner).ok_or_else(|| bad("heap offset out of range"))?, len);
            }
            // The indirect block's child indirect blocks follow its `direct` direct blocks.
            if entry < direct {
                return Err(bad("a heap offset outside its heap"));
            }
            block = slot(entry)?.ok_or_else(|| bad("a heap object in an unallocated block"))?;
            rows = (size.trailing_zeros() as u64).saturating_sub(heap.start.trailing_zeros() as u64) + 1;
            block_off = entry_off;
        }
        Err(bad("fractal heap blocks are nested too deeply"))
    }

    /// Every record of the version 2 B-tree at `addr`, which must be of type `kind`.
    fn btree2(&mut self, addr: u64, kind: u8) -> Result<Vec<&'a [u8]>> {
        self.step()?;
        let mut r = Rd::new(self.from(addr)?);
        if r.take(4)? != b"BTHD" {
            return Err(bad(format!("no B-tree header at address {addr}")));
        }
        r.skip(1)?;
        if r.u8()? != kind {
            return Err(bad(format!("the B-tree at {addr} is not of type {kind}")));
        }
        let node_size = r.u32()? as u64;
        let record_size = r.u16()? as u64;
        let depth = r.u16()? as usize;
        r.skip(2)?;
        let root = self.addr(&mut r)?;
        let root_records = r.u16()? as u64;
        let total = self.len(&mut r)?;
        let Some(root) = root else { return Ok(Vec::new()) };
        if record_size == 0 || node_size < 10 + record_size || depth > MAX_TREE_DEPTH || total > MAX_RECORDS {
            return Err(bad(format!("the B-tree at {addr} has impossible sizes")));
        }
        // Most records per node at each depth, and below it, give the widths of the child
        // record counts in internal nodes.
        let mut max = vec![(node_size - 10) / record_size];
        let mut cum = max.clone();
        let mut widths = vec![(0usize, 0usize)];
        for d in 1..=depth {
            let below = max.get(d - 1).copied().unwrap_or(0);
            let below_cum = cum.get(d - 1).copied().unwrap_or(0);
            let w = (enc_size(below), if d > 1 { enc_size(below_cum) } else { 0 });
            let pointer = (self.so + w.0 + w.1) as u64;
            let room = node_size.checked_sub(10 + pointer).ok_or_else(|| bad("B-tree nodes too small for their pointers"))?;
            let m = room / (record_size + pointer);
            max.push(m);
            cum.push(m.saturating_add(1).saturating_mul(below_cum).saturating_add(m));
            widths.push(w);
        }
        let shape = Shape { node_size, record_size, kind, max, widths };
        let mut out = Vec::new();
        self.btree2_node(&shape, root, depth, root_records, &mut out)?;
        Ok(out)
    }

    fn btree2_node(&mut self, shape: &Shape, addr: u64, depth: usize, records: u64, out: &mut Vec<&'a [u8]>) -> Result<()> {
        self.step()?;
        if records > shape.max.get(depth).copied().unwrap_or(0) || out.len() as u64 > MAX_RECORDS {
            return Err(bad(format!("the B-tree node at {addr} claims {records} records")));
        }
        let node = self.at(addr, shape.node_size)?;
        let mut r = Rd::new(node);
        let sig = r.take(4)?;
        if sig != if depth == 0 { b"BTLF" } else { b"BTIN" } || r.u8()? != 0 || r.u8()? != shape.kind {
            return Err(bad(format!("no B-tree node at address {addr}")));
        }
        for _ in 0..records {
            out.push(r.take(shape.record_size as usize)?);
        }
        if depth > 0 {
            let (w_count, w_total) = shape.widths.get(depth).copied().unwrap_or((1, 0));
            for _ in 0..=records {
                let child = self.addr(&mut r)?.ok_or_else(|| bad("a B-tree child at the undefined address"))?;
                let count = r.uint(w_count)?;
                r.skip(w_total)?;
                self.btree2_node(shape, child, depth - 1, count, out)?;
            }
        }
        Ok(())
    }

    /// Whether the object at `addr` is a dataset (its header has a data layout).
    pub(super) fn is_dataset(&mut self, addr: u64) -> Result<bool> {
        Ok(self.header(addr)?.iter().any(|m| m.kind == 0x08))
    }

    /// The type and raw values of the dataset at `addr`.
    pub(super) fn dataset(&mut self, addr: u64) -> Result<Dataset<'a>> {
        let msgs = self.header(addr)?;
        let find = |kind: u16| msgs.iter().find(|m| m.kind == kind);
        let (Some(space), Some(dtype), Some(layout)) = (find(0x01), find(0x03), find(0x08)) else {
            return Err(bad(format!("the object at {addr} is not a dataset")));
        };
        if space.shared() || dtype.shared() || layout.shared() {
            return Err(unsupported("shared datatypes or dataspaces"));
        }
        if find(0x07).is_some() {
            return Err(unsupported("external data files"));
        }
        let space = Space::parse(space.data, self.sl)?;
        let dtype = Dtype::parse(dtype.data)?;
        let filters = match find(0x0b) {
            Some(f) => filter_ids(f.data)?,
            None => Vec::new(),
        };
        let bytes = space
            .count()
            .and_then(|n| n.checked_mul(dtype.size() as u64))
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n <= MAX_DECODED_BYTES)
            .ok_or_else(|| FormatError::TooLarge(format!("a dataset of {space:?} elements")))?;
        self.charge(bytes)?;
        if bytes == 0 {
            return Ok(Dataset { dtype, data: Cow::Borrowed(&[]) });
        }
        let data = match self.layout(layout.data)? {
            Layout::Compact(raw) => Cow::Borrowed(raw.get(..bytes).ok_or_else(|| bad("a compact dataset holds less than its size"))?),
            Layout::Contiguous(at) => {
                let at = at.ok_or_else(|| unsupported("datasets without stored data"))?;
                Cow::Borrowed(self.at(at, bytes as u64)?)
            }
            Layout::Chunked { dims, index } => {
                let Space::Simple(shape) = &space else { return Err(bad("a chunked dataset without dimensions")) };
                Cow::Owned(self.chunked(shape, &dims, index, dtype.size(), &filters, bytes)?)
            }
        };
        Ok(Dataset { dtype, data })
    }

    fn layout(&self, data: &'a [u8]) -> Result<Layout<'a>> {
        let mut r = Rd::new(data);
        let version = r.u8()?;
        match version {
            1 | 2 => {
                let rank = r.u8()? as usize;
                let class = r.u8()?;
                r.skip(5)?;
                if rank > MAX_RANK + 1 {
                    return Err(bad("a layout of too many dimensions"));
                }
                let at = if class == 0 { None } else { self.addr(&mut r)? };
                let dims = (0..rank).map(|_| r.uint(4)).collect::<Result<Vec<u64>>>()?;
                match class {
                    0 => {
                        let n = r.usize(4)?;
                        Ok(Layout::Compact(r.take(n)?))
                    }
                    1 => Ok(Layout::Contiguous(at)),
                    2 => Ok(Layout::Chunked { dims, index: ChunkIndex::BTree(at) }),
                    c => Err(unsupported(format!("layout class {c}"))),
                }
            }
            3 | 4 => match r.u8()? {
                0 => {
                    let n = r.u16()? as usize;
                    Ok(Layout::Compact(r.take(n)?))
                }
                1 => Ok(Layout::Contiguous(self.addr(&mut r)?)),
                2 if version == 3 => {
                    let rank = r.u8()? as usize;
                    if rank > MAX_RANK + 1 {
                        return Err(bad("a layout of too many dimensions"));
                    }
                    let at = self.addr(&mut r)?;
                    let dims = (0..rank).map(|_| r.uint(4)).collect::<Result<Vec<u64>>>()?;
                    Ok(Layout::Chunked { dims, index: ChunkIndex::BTree(at) })
                }
                2 => {
                    let flags = r.u8()?;
                    let rank = r.u8()? as usize;
                    let width = r.u8()? as usize;
                    if rank > MAX_RANK + 1 {
                        return Err(bad("a layout of too many dimensions"));
                    }
                    let dims = (0..rank).map(|_| r.uint(width)).collect::<Result<Vec<u64>>>()?;
                    match r.u8()? {
                        1 => {
                            let (size, mask) = if flags & 0x02 != 0 { (Some(self.len(&mut r)?), r.u32()?) } else { (None, 0) };
                            let addr = self.addr(&mut r)?;
                            Ok(Layout::Chunked { dims, index: ChunkIndex::Single { addr, size, mask } })
                        }
                        i => Err(unsupported(format!("chunk index type {i}"))),
                    }
                }
                c => Err(unsupported(format!("layout class {c}"))),
            },
            v => Err(unsupported(format!("layout message version {v}"))),
        }
    }

    /// Assembles a chunked dataset of `shape` elements (`size` bytes each, `bytes` in all).
    fn chunked(&mut self, shape: &[u64], dims: &[u64], index: ChunkIndex, size: usize, filters: &[u16], bytes: usize) -> Result<Vec<u8>> {
        let rank = shape.len();
        let chunk = dims.get(..rank).filter(|c| rank > 0 && c.iter().all(|d| *d > 0)).ok_or_else(|| bad("chunk dimensions do not match the dataspace"))?;
        let chunk_bytes = chunk
            .iter()
            .try_fold(size as u64, |n, d| n.checked_mul(*d))
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n <= MAX_DECODED_BYTES)
            .ok_or_else(|| FormatError::TooLarge("a dataset chunk is too large".into()))?;
        let mut chunks = Vec::new();
        match index {
            ChunkIndex::BTree(Some(tree)) => self.chunk_node(tree, None, 0, rank, &mut chunks)?,
            ChunkIndex::BTree(None) => {}
            ChunkIndex::Single { addr, size: stored, mask } => {
                if let Some(addr) = addr {
                    chunks.push(ChunkRef { offsets: vec![0; rank], addr, size: stored.unwrap_or(chunk_bytes as u64), mask });
                }
            }
        }
        // Alembic writes every chunk: refusing gaps also keeps a damaged size from allocating
        // much more than the chunks hold.
        if (chunks.len() as u64).checked_mul(chunk_bytes as u64).is_none_or(|n| n < bytes as u64) {
            return Err(unsupported("chunked datasets with unwritten chunks"));
        }
        // Allocated once a chunk has decoded to its full size.
        let mut out = None;
        for c in chunks {
            self.step()?;
            let raw = self.at(c.addr, c.size)?;
            self.charge(chunk_bytes)?;
            let data = unfilter(raw, c.mask, filters, size, chunk_bytes)?;
            let out = out.get_or_insert_with(|| vec![0u8; bytes]);
            copy_chunk(out, &data, shape, chunk, &c.offsets, size)?;
        }
        Ok(out.unwrap_or_default())
    }

    /// Chunks of a version 1 B-tree node of type 1: keys of stored size (4), filter mask (4) and
    /// rank + 1 element offsets (8 each), around child addresses.
    fn chunk_node(&mut self, addr: u64, level: Option<u8>, depth: usize, rank: usize, out: &mut Vec<ChunkRef>) -> Result<()> {
        self.step()?;
        if depth > MAX_TREE_DEPTH {
            return Err(bad("a chunk B-tree is nested too deeply"));
        }
        let mut r = Rd::new(self.from(addr)?);
        if r.take(4)? != b"TREE" || r.u8()? != 1 {
            return Err(bad(format!("no chunk B-tree node at address {addr}")));
        }
        let node_level = r.u8()?;
        if level.is_some_and(|l| l != node_level) {
            return Err(bad("chunk B-tree levels do not descend"));
        }
        let entries = r.u16()?;
        r.skip(2 * self.so)?;
        for _ in 0..entries {
            let size = r.u32()? as u64;
            let mask = r.u32()?;
            let offsets = (0..rank).map(|_| r.uint(8)).collect::<Result<Vec<u64>>>()?;
            r.skip(8)?;
            let child = self.addr(&mut r)?.ok_or_else(|| bad("a chunk at the undefined address"))?;
            if node_level == 0 {
                out.push(ChunkRef { offsets, addr: child, size, mask });
            } else {
                self.chunk_node(child, Some(node_level - 1), depth + 1, rank, out)?;
            }
        }
        Ok(())
    }
}

/// Node sizes of a version 2 B-tree: most records per node at each depth and the widths of an
/// internal node's child record count and total record count.
struct Shape {
    node_size: u64,
    record_size: u64,
    kind: u8,
    max: Vec<u64>,
    widths: Vec<(usize, usize)>,
}

/// A stored chunk: element offsets, address, stored size and skipped-filter mask.
struct ChunkRef {
    offsets: Vec<u64>,
    addr: u64,
    size: u64,
    mask: u32,
}

/// Filter IDs of a filter pipeline message (versions 1 and 2), in the order they were applied.
fn filter_ids(data: &[u8]) -> Result<Vec<u16>> {
    let mut r = Rd::new(data);
    let version = r.u8()?;
    let count = r.u8()?;
    if version == 1 {
        r.skip(6)?;
    } else if version != 2 {
        return Err(unsupported(format!("filter pipeline version {version}")));
    }
    let mut ids = Vec::new();
    for _ in 0..count {
        let id = r.u16()?;
        let name_len = if version == 1 || id >= 256 { r.u16()? as usize } else { 0 };
        let _flags = r.u16()?;
        let values = r.u16()? as usize;
        r.skip(name_len)?;
        r.skip(4 * values)?;
        if version == 1 && values % 2 == 1 {
            r.skip(4)?;
        }
        ids.push(id);
    }
    Ok(ids)
}

/// Undoes the filters of a chunk (last applied first), skipping those `mask` marks.
fn unfilter(raw: &[u8], mask: u32, filters: &[u16], size: usize, expect: usize) -> Result<Vec<u8>> {
    let mut data = raw.to_vec();
    for (i, id) in filters.iter().enumerate().rev() {
        if i < 32 && mask & (1 << i) != 0 {
            continue;
        }
        data = match id {
            1 => {
                let mut out = Vec::new();
                flate2::read::ZlibDecoder::new(data.as_slice())
                    .take(expect as u64 + 1)
                    .read_to_end(&mut out)
                    .map_err(|e| bad(format!("a compressed chunk does not inflate: {e}")))?;
                out
            }
            2 => unshuffle(&data, size),
            3 => {
                data.truncate(data.len().saturating_sub(4));
                data
            }
            other => return Err(unsupported(format!("HDF5 filter {other}"))),
        };
    }
    if data.len() != expect {
        return Err(bad(format!("a chunk holds {} bytes instead of {expect}", data.len())));
    }
    Ok(data)
}

/// Reverses the shuffle filter: byte `b` of every element was stored together.
fn unshuffle(data: &[u8], size: usize) -> Vec<u8> {
    if size < 2 {
        return data.to_vec();
    }
    let n = data.len() / size;
    let mut out = data.to_vec();
    for e in 0..n {
        for b in 0..size {
            if let (Some(dst), Some(src)) = (out.get_mut(e * size + b), data.get(b * n + e)) {
                *dst = *src;
            }
        }
    }
    out
}

/// Copies a decoded chunk at element `offsets` into the dataset `out` of `shape` (row-major),
/// clipping chunks at the dataset's edges.
fn copy_chunk(out: &mut [u8], chunk: &[u8], shape: &[u64], dims: &[u64], offsets: &[u64], size: usize) -> Result<()> {
    let rank = shape.len();
    if rank == 0 || offsets.len() != rank || dims.len() != rank || offsets.iter().zip(shape).any(|(o, s)| o >= s) {
        return Err(bad("a chunk outside its dataset"));
    }
    let last = rank - 1;
    // Elements per run along the last dimension.
    let run = dims[last].min(shape[last] - offsets[last]) as usize;
    let mut idx = vec![0u64; rank];
    loop {
        if idx.iter().zip(offsets).zip(shape).all(|((i, o), s)| o + i < *s) {
            let (mut src, mut dst) = (0u64, 0u64);
            for d in 0..rank {
                src = src * dims[d] + idx[d];
                dst = dst * shape[d] + offsets[d] + idx[d];
            }
            let bytes = run * size;
            let (src, dst) = (src as usize * size, dst as usize * size);
            let from = chunk.get(src..src + bytes).ok_or_else(|| bad("a chunk shorter than its dimensions"))?;
            out.get_mut(dst..dst + bytes).ok_or_else(|| bad("a chunk outside its dataset"))?.copy_from_slice(from);
        }
        // Next row: odometer over all but the last dimension.
        let mut d = last;
        loop {
            if d == 0 {
                return Ok(());
            }
            d -= 1;
            idx[d] += 1;
            if idx[d] < dims[d] {
                break;
            }
            idx[d] = 0;
        }
    }
}

/// The numbers in `data` (elements of `dtype`) as f64.
pub(super) fn numbers(dtype: Dtype, data: &[u8]) -> Result<Vec<f64>> {
    let size = dtype.size();
    let elements = data.chunks_exact(size);
    let word = |c: &[u8], big: bool| -> u64 {
        if big { c.iter().fold(0u64, |v, b| (v << 8) | *b as u64) } else { c.iter().rev().fold(0u64, |v, b| (v << 8) | *b as u64) }
    };
    match dtype {
        Dtype::Int { size: 1 | 2 | 4 | 8, signed, big_endian } => Ok(elements
            .map(|c| {
                let v = word(c, big_endian);
                if signed {
                    let shift = 64 - 8 * size as u32;
                    (((v << shift) as i64) >> shift) as f64
                } else {
                    v as f64
                }
            })
            .collect()),
        Dtype::Float { size: 2, big_endian } => Ok(elements.map(|c| half::f16::from_bits(word(c, big_endian) as u16).to_f64()).collect()),
        Dtype::Float { size: 4, big_endian } => Ok(elements.map(|c| f32::from_bits(word(c, big_endian) as u32) as f64).collect()),
        Dtype::Float { size: 8, big_endian } => Ok(elements.map(|c| f64::from_bits(word(c, big_endian))).collect()),
        other => Err(unsupported(format!("values of type {other:?}"))),
    }
}

/// A string attribute's text (fixed-length string or 8-bit characters), up to the first NUL.
pub(super) fn text(attr: &Attr) -> String {
    match attr.dtype {
        Dtype::Str { .. } | Dtype::Int { size: 1, .. } => String::from_utf8_lossy(attr.data.split(|c| *c == 0).next().unwrap_or_default()).into_owned(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_are_placed_and_clipped_row_major() {
        // A 3×5 dataset of u8 from 2×3 chunks: the chunks at the right and bottom edges are cut.
        let shape = [3u64, 5];
        let dims = [2u64, 3];
        let mut out = vec![0u8; 15];
        for (oy, ox) in [(0u64, 0u64), (0, 3), (2, 0), (2, 3)] {
            let chunk: Vec<u8> = (0..6).map(|i| (10 * (oy + i / 3) + ox + i % 3) as u8).collect();
            copy_chunk(&mut out, &chunk, &shape, &dims, &[oy, ox], 1).unwrap();
        }
        let want: Vec<u8> = (0..15).map(|i| (10 * (i / 5) + i % 5) as u8).collect();
        assert_eq!(out, want);
        assert!(copy_chunk(&mut out, &[0; 6], &shape, &dims, &[3, 0], 1).is_err());
        assert!(copy_chunk(&mut out, &[0; 2], &shape, &dims, &[0, 0], 1).is_err());
        assert!(copy_chunk(&mut out, &[0; 2], &[], &[], &[], 1).is_err());
    }

    #[test]
    fn shuffle_is_undone_and_byte_orders_decode() {
        // Two u16 values 0x0102 and 0x0304, shuffled: low bytes first, then high bytes.
        assert_eq!(unshuffle(&[0x02, 0x04, 0x01, 0x03], 2), vec![0x02, 0x01, 0x04, 0x03]);
        let be = Dtype::Int { size: 2, signed: true, big_endian: true };
        assert_eq!(numbers(be, &[0xff, 0xfe, 0x00, 0x05]).unwrap(), vec![-2.0, 5.0]);
        let f = Dtype::Float { size: 4, big_endian: true };
        assert_eq!(numbers(f, &1.5f32.to_be_bytes()).unwrap(), vec![1.5]);
        let h = Dtype::Float { size: 2, big_endian: false };
        assert_eq!(numbers(h, &half::f16::from_f32(0.25).to_bits().to_le_bytes()).unwrap(), vec![0.25]);
        assert!(numbers(Dtype::Int { size: 3, signed: false, big_endian: false }, &[0; 3]).is_err());
    }

    #[test]
    fn filter_pipelines_of_both_versions_parse() {
        // Version 1: deflate with a padded name and one client value (padded to even), shuffle.
        let mut v1 = vec![1, 2, 0, 0, 0, 0, 0, 0];
        v1.extend([1, 0, 8, 0, 0, 0, 1, 0]);
        v1.extend(b"deflate\0");
        v1.extend([6, 0, 0, 0, 0, 0, 0, 0]);
        v1.extend([2, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(filter_ids(&v1).unwrap(), vec![1, 2]);
        // Version 2: filters below 256 carry no name.
        assert_eq!(filter_ids(&[2, 1, 2, 0, 0, 0, 1, 0, 4, 0, 0, 0]).unwrap(), vec![2]);
        assert!(filter_ids(&[2, 3, 1, 0]).is_err());
    }

    #[test]
    fn signature_is_found_at_powers_of_two_only() {
        let mut b = vec![0u8; 2048 + 8];
        b[1024..1032].copy_from_slice(SIGNATURE);
        assert_eq!(superblock_at(&b), Some(1024));
        let mut c = vec![0u8; 1000];
        c[700..708].copy_from_slice(SIGNATURE);
        assert_eq!(superblock_at(&c), None);
    }
}
