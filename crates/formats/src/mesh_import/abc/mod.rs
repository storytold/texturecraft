//! Alembic (`.abc`) in the Ogawa or the HDF5 container: every PolyMesh object at its first
//! sample, in world space through the Xform hierarchy (matrix, translate, rotate, scale ops;
//! `.inherits`), with positions (`P`), polygons (`.faceIndices` / `.faceCounts`, Alembic's
//! clockwise winding turned counter-clockwise), normals (`N`) and UVs (`uv`), indexed or not, at
//! any geometry scope, and FaceSet children as materials. Alembic is Y up; units are kept as
//! written.
//!
//! The mesh walk reads objects and properties through [`Archive`], which each container
//! implements. The Ogawa container is a tree of groups (child offset lists) and data blocks;
//! Alembic stores an archive group, objects (property compound, children, header block) and
//! properties (bit-packed header blocks, one group per property holding its samples). Sample
//! hashes are skipped. The HDF5 container is read by [`hdf5`] and mapped onto objects and
//! properties by [`hdf5_archive`].

mod hdf5;
mod hdf5_archive;
#[cfg(test)]
mod hdf5_tests;
#[cfg(test)]
mod tests;

use texturecraft_geom::glam::{Mat3, Mat4, Vec2, Vec3};

use super::{Corner, PolygonSink, RawMesh, check_vertices};
use crate::{FormatError, Result, malformed};

const KIND: &str = "Alembic";
const OGAWA_MAGIC: &[u8] = b"Ogawa";
/// Set on a child reference that points at a data block (clear: a group).
const DATA_BIT: u64 = 1 << 63;
/// Deepest object nesting followed.
const MAX_DEPTH: usize = 64;
/// Budget for decoded numbers (held as f64) and parsed header bytes: bounds memory and the work
/// a file whose groups are shared many times over can cause.
const MAX_DECODED_BYTES: usize = 1 << 31;
/// Objects visited (bounds files whose object groups are shared many times over).
const MAX_OBJECT_VISITS: usize = 1_000_000;
/// Sample hash prefix of every data sample.
const KEY_BYTES: usize = 16;
/// Object header blocks end with the properties and children hashes.
const OBJECT_HASH_BYTES: usize = 32;

/// Material of polygons outside every face set.
const DEFAULT_MATERIAL: &str = "Default";

/// The Ogawa tree over the file bytes.
struct Ogawa<'a> {
    b: &'a [u8],
    budget: usize,
    /// Archive-wide metadata strings properties and objects refer to by index (0 is empty).
    indexed_meta: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum PropKind {
    Compound,
    Scalar,
    Array,
}

/// A property header and where its container keeps it (`H`).
struct Prop<H> {
    name: String,
    meta: String,
    kind: PropKind,
    pod: u32,
    extent: usize,
    at: H,
}

/// What the mesh walk reads from an Alembic container: objects, their properties and the first
/// sample of a property.
trait Archive {
    /// An object or a property inside the container.
    type Handle;

    /// Child objects of `object` in order: name, metadata and handle.
    fn children(&mut self, object: &Self::Handle) -> Result<Vec<(String, String, Self::Handle)>>;

    /// The properties of `object`'s top compound property.
    fn properties(&mut self, object: &Self::Handle) -> Result<Vec<Prop<Self::Handle>>>;

    /// The properties inside compound property `property`.
    fn compound(&mut self, property: &Prop<Self::Handle>) -> Result<Vec<Prop<Self::Handle>>>;

    /// The first sample of a scalar or array property as numbers (`extent` per item).
    fn values(&mut self, property: &Prop<Self::Handle>) -> Result<Vec<f64>>;
}

/// Numbers of a POD type, little-endian (Alembic's POD numbering: 0 bool, 1 u8, 2 i8, 3 u16,
/// 4 i16, 5 u32, 6 i32, 7 u64, 8 i64, 10 f32, 11 f64); `None` for other types.
fn pod_values(pod: u32, raw: &[u8]) -> Option<Vec<f64>> {
    Some(match pod {
        0 | 1 => raw.iter().map(|v| *v as f64).collect(),
        2 => raw.iter().map(|v| *v as i8 as f64).collect(),
        3 => raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c) as f64).collect(),
        4 => raw.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c) as f64).collect(),
        5 => raw.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c) as f64).collect(),
        6 => raw.as_chunks::<4>().0.iter().map(|c| i32::from_le_bytes(*c) as f64).collect(),
        7 => raw.as_chunks::<8>().0.iter().map(|c| u64::from_le_bytes(*c) as f64).collect(),
        8 => raw.as_chunks::<8>().0.iter().map(|c| i64::from_le_bytes(*c) as f64).collect(),
        10 => raw.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c) as f64).collect(),
        11 => raw.as_chunks::<8>().0.iter().map(|c| f64::from_le_bytes(*c)).collect(),
        _ => return None,
    })
}

/// Bytes per value of a POD type.
fn pod_bytes(pod: u32) -> Option<usize> {
    match pod {
        0..=2 => Some(1),
        3 | 4 => Some(2),
        5 | 6 | 10 => Some(4),
        7 | 8 | 11 => Some(8),
        _ => None,
    }
}

/// Value of `key` in Alembic metadata (`key=value;key=value`).
fn meta_get<'m>(meta: &'m str, key: &str) -> Option<&'m str> {
    meta.split(';').filter_map(|kv| kv.split_once('=')).find(|(k, _)| *k == key).map(|(_, v)| v)
}

/// Little-endian reads from a header block.
struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self.pos.checked_add(n).and_then(|end| self.b.get(self.pos..end)).ok_or_else(|| malformed(KIND, "header block ends early"))?;
        self.pos += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?.first().copied().unwrap_or(0))
    }

    fn u32(&mut self) -> Result<u32> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    /// A u8, u16 or u32 by the header's size hint.
    fn hinted(&mut self, hint: u32) -> Result<usize> {
        Ok(match hint {
            0 => self.u8()? as usize,
            1 => {
                let s = self.take(2)?;
                u16::from_le_bytes([s[0], s[1]]) as usize
            }
            2 => self.u32()? as usize,
            _ => return Err(malformed(KIND, format!("bad size hint {hint}"))),
        })
    }

    fn string(&mut self, n: usize) -> Result<String> {
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
}

impl<'a> Ogawa<'a> {
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.budget =
            self.budget.checked_sub(bytes).ok_or_else(|| FormatError::TooLarge(format!("the Alembic file decodes to more than {MAX_DECODED_BYTES} bytes")))?;
        Ok(())
    }

    fn u64_at(&self, pos: u64) -> Result<u64> {
        let p = usize::try_from(pos).map_err(|_| malformed(KIND, "offset out of range"))?;
        let s = p.checked_add(8).and_then(|e| self.b.get(p..e)).ok_or_else(|| malformed(KIND, format!("offset {pos} is past the end of the file")))?;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Ok(u64::from_le_bytes(a))
    }

    /// Child references of the group at `r` (offset 0 is the empty group).
    fn group(&mut self, r: u64) -> Result<Vec<u64>> {
        if r & DATA_BIT != 0 {
            return Err(malformed(KIND, "expected a group, found a data block"));
        }
        if r == 0 {
            return Ok(Vec::new());
        }
        let n = self.u64_at(r)?;
        let room = (self.b.len() as u64).saturating_sub(r.saturating_add(8)) / 8;
        if n > room {
            return Err(malformed(KIND, format!("group at {r} claims {n} children")));
        }
        self.charge(n as usize * 8)?;
        (0..n).map(|i| self.u64_at(r + 8 + 8 * i)).collect()
    }

    /// The bytes of the data block at `r` (offset 0 is the empty block).
    fn data(&self, r: u64) -> Result<&'a [u8]> {
        if r & DATA_BIT == 0 {
            return Err(malformed(KIND, "expected a data block, found a group"));
        }
        let off = r & !DATA_BIT;
        if off == 0 {
            return Ok(&[]);
        }
        let size = self.u64_at(off)?;
        let start = usize::try_from(off + 8).map_err(|_| malformed(KIND, "offset out of range"))?;
        usize::try_from(size)
            .ok()
            .and_then(|s| start.checked_add(s))
            .and_then(|end| self.b.get(start..end))
            .ok_or_else(|| malformed(KIND, format!("data block at {off} ({size} bytes) overruns the file")))
    }

    fn meta_at(&self, index: u8) -> String {
        self.indexed_meta.get(index as usize).cloned().unwrap_or_default()
    }

    /// The properties of the compound property group at `r`.
    fn compound_at(&mut self, r: u64) -> Result<Vec<Prop<u64>>> {
        let children = self.group(r)?;
        let Some(&last) = children.last().filter(|c| **c & DATA_BIT != 0) else { return Ok(Vec::new()) };
        let block = self.data(last)?;
        self.charge(block.len())?;
        let mut c = Cursor { b: block, pos: 0 };
        let mut out = Vec::new();
        while c.pos < block.len() {
            let info = c.u32()?;
            let hint = (info & 0xc) >> 2;
            let kind = match info & 0x3 {
                0 => PropKind::Compound,
                1 => PropKind::Scalar,
                _ => PropKind::Array,
            };
            let (pod, extent) = ((info & 0xf0) >> 4, ((info & 0xff000) >> 12) as usize);
            if kind != PropKind::Compound {
                let _samples = c.hinted(hint)?;
                if info & 0x200 != 0 {
                    c.hinted(hint)?;
                    c.hinted(hint)?;
                }
                if info & 0x100 != 0 {
                    c.hinted(hint)?;
                }
            }
            let name_len = c.hinted(hint)?;
            let name = c.string(name_len)?;
            let meta_index = ((info & 0xff0_0000) >> 20) as u8;
            let meta = if meta_index == 0xff {
                let n = c.hinted(hint)?;
                c.string(n)?
            } else {
                self.meta_at(meta_index)
            };
            let group = children.get(out.len()).copied().ok_or_else(|| malformed(KIND, format!("property \"{name}\" has no group")))?;
            out.push(Prop { name, meta, kind, pod, extent, at: group });
        }
        Ok(out)
    }

    /// The first sample of a scalar or array property as numbers (`extent` per item).
    fn first_sample(&mut self, property: &Prop<u64>) -> Result<Vec<f64>> {
        let samples = self.group(property.at)?;
        let Some(&first) = samples.first() else { return Ok(Vec::new()) };

        let raw = self.data(first)?;
        let raw = if raw.is_empty() {
            raw
        } else {
            raw.get(KEY_BYTES..).ok_or_else(|| malformed(KIND, format!("sample of \"{}\" is shorter than its key", property.name)))?
        };

        let unsupported = || FormatError::Unsupported(format!("Alembic property \"{}\" of data type {}", property.name, property.pod));
        let value_bytes = pod_bytes(property.pod).ok_or_else(unsupported)?;
        self.charge((raw.len() / value_bytes).saturating_mul(8))?;
        pod_values(property.pod, raw).ok_or_else(unsupported)
    }

    /// Object headers (name, metadata) of the object group's last child.
    fn object_headers(&mut self, children: &[u64]) -> Result<Vec<(String, String)>> {
        let Some(&last) = children.last().filter(|c| **c & DATA_BIT != 0) else { return Ok(Vec::new()) };
        let block = self.data(last)?;
        self.charge(block.len())?;
        let body = block.get(..block.len().saturating_sub(OBJECT_HASH_BYTES)).unwrap_or_default();
        let mut c = Cursor { b: body, pos: 0 };
        let mut out = Vec::new();
        while c.pos < body.len() {
            let n = c.u32()? as usize;
            let name = c.string(n)?;
            let index = c.u8()?;
            let meta = if index == 0xff {
                let n = c.u32()? as usize;
                c.string(n)?
            } else {
                self.meta_at(index)
            };
            out.push((name, meta));
        }
        Ok(out)
    }
}

impl Archive for Ogawa<'_> {
    /// A group reference.
    type Handle = u64;

    fn children(&mut self, object: &u64) -> Result<Vec<(String, String, u64)>> {
        let children = self.group(*object)?;
        let headers = self.object_headers(&children)?;
        headers
            .into_iter()
            .enumerate()
            .map(|(i, (name, meta))| {
                let group = children.get(i + 1).copied().filter(|g| g & DATA_BIT == 0);
                let group = group.ok_or_else(|| malformed(KIND, format!("object \"{name}\" has no group")))?;
                Ok((name, meta, group))
            })
            .collect()
    }

    fn properties(&mut self, object: &u64) -> Result<Vec<Prop<u64>>> {
        match self.group(*object)?.first() {
            Some(&p) if p & DATA_BIT == 0 => self.compound_at(p),
            _ => Ok(Vec::new()),
        }
    }

    fn compound(&mut self, property: &Prop<u64>) -> Result<Vec<Prop<u64>>> {
        self.compound_at(property.at)
    }

    fn values(&mut self, property: &Prop<u64>) -> Result<Vec<f64>> {
        self.first_sample(property)
    }
}

pub(crate) fn read(bytes: &[u8]) -> Result<RawMesh> {
    crate::guard(KIND, || read_inner(bytes))
}

fn read_inner(bytes: &[u8]) -> Result<RawMesh> {
    if !bytes.starts_with(OGAWA_MAGIC) && hdf5::superblock_at(bytes).is_some() {
        let mut archive = hdf5_archive::H5Archive::open(bytes)?;
        let top = archive.top()?;
        return walk(archive, &top);
    }
    if !bytes.starts_with(OGAWA_MAGIC) || bytes.len() < 16 {
        return Err(malformed(KIND, "not an Alembic file (neither the Ogawa nor the HDF5 container)"));
    }
    let mut o = Ogawa { b: bytes, budget: MAX_DECODED_BYTES, indexed_meta: vec![String::new()] };
    let root_ref = o.u64_at(8)?;
    let root = o.group(root_ref)?;
    let is_data = |i: usize| root.get(i).is_some_and(|r| r & DATA_BIT != 0);
    if root.len() < 6 || !(0..6).all(|i| is_data(i) != (i == 2)) {
        return Err(malformed(KIND, "the archive group does not have the Alembic layout"));
    }
    // Indexed metadata: a u8 length before each string.
    let block = o.data(root[5])?;
    let mut c = Cursor { b: block, pos: 0 };
    while c.pos < block.len() {
        let n = c.u8()? as usize;
        o.indexed_meta.push(c.string(n)?);
    }
    walk(o, &root[2])
}

/// Every mesh under the top object `top` of `archive`.
fn walk<A: Archive>(archive: A, top: &A::Handle) -> Result<RawMesh> {
    let mut sink = PolygonSink::default();
    let mut walk = Walk { a: archive, sink: &mut sink, visits: 0 };
    walk.object(top, Mat4::IDENTITY, "", 0)?;
    Ok(sink.finish())
}

struct Walk<'b, A> {
    a: A,
    sink: &'b mut PolygonSink,
    visits: usize,
}

/// Which items a geometry parameter's values are per.
#[derive(Clone, Copy)]
enum Scope {
    Constant,
    Uniform,
    Vertex,
    FaceVarying,
}

/// A geometry parameter's values and optional index list.
struct Param {
    vals: Vec<f64>,
    indices: Option<Vec<f64>>,
    scope: Scope,
}

impl Param {
    /// The `W` numbers of polygon corner `k` (point `p`, face `f`).
    fn get<const W: usize>(&self, k: usize, p: usize, f: usize) -> Option<[f64; W]> {
        let i = match self.scope {
            Scope::Constant => 0,
            Scope::Uniform => f,
            Scope::Vertex => p,
            Scope::FaceVarying => k,
        };
        let i = match &self.indices {
            Some(ix) => Some(*ix.get(i)?).filter(|v| *v >= 0.0)? as usize,
            None => i,
        };
        let start = i.checked_mul(W)?;
        self.vals.get(start..start.checked_add(W)?)?.try_into().ok()
    }
}

fn find<'p, H>(props: &'p [Prop<H>], name: &str) -> Option<&'p Prop<H>> {
    props.iter().find(|p| p.name == name)
}

/// The schema compound of an object: the compound whose metadata names `schema`, else `name`.
fn schema_compound<'p, H>(props: &'p [Prop<H>], schema: &str, name: &str) -> Option<&'p Prop<H>> {
    let compounds = || props.iter().filter(|p| p.kind == PropKind::Compound);
    compounds().find(|p| meta_get(&p.meta, "schema") == Some(schema)).or_else(|| compounds().find(|p| p.name == name))
}

impl<A: Archive> Walk<'_, A> {
    /// The children of object `r`, placed by `parent` (world matrix); `xform` names the Xform
    /// object `r` is (empty otherwise): meshes take their transform's name, as DCC exports
    /// expect (`pCube1` / `pCubeShape1`, object / mesh data).
    fn object(&mut self, r: &A::Handle, parent: Mat4, xform: &str, depth: usize) -> Result<()> {
        self.visits += 1;
        if depth > MAX_DEPTH || self.visits > MAX_OBJECT_VISITS {
            return Err(malformed(KIND, "objects are nested too deeply or shared too often"));
        }
        for (name, meta, child) in self.a.children(r)? {
            let schema = meta_get(&meta, "schema").unwrap_or_default();
            let props = self.a.properties(&child)?;
            let mut world = parent;
            if schema.starts_with("AbcGeom_Xform") {
                if let Some(xf) = schema_compound(&props, schema, ".xform") {
                    let (local, inherits) = self.xform(xf)?;
                    world = if inherits { parent * local } else { local };
                }
            } else if schema.starts_with("AbcGeom_PolyMesh")
                && let Some(geom) = schema_compound(&props, schema, ".geom")
            {
                self.sink.part = if xform.is_empty() { name.clone() } else { xform.to_string() };
                let sets = self.face_sets(&child)?;
                self.mesh(geom, world, &sets)?;
            }
            if !schema.starts_with("AbcGeom_FaceSet") {
                let owner = if schema.starts_with("AbcGeom_Xform") { name.as_str() } else { "" };
                self.object(&child, world, owner, depth + 1)?;
            }
        }
        Ok(())
    }

    /// Local matrix and `.inherits` of an Xform schema compound.
    fn xform(&mut self, xf: &Prop<A::Handle>) -> Result<(Mat4, bool)> {
        let props = self.a.compound(xf)?;
        let ops = match find(&props, ".ops") {
            Some(p) => self.a.values(p)?,
            None => Vec::new(),
        };
        let vals = match find(&props, ".vals") {
            Some(p) => self.a.values(p)?,
            None => Vec::new(),
        };
        let inherits = match find(&props, ".inherits") {
            Some(p) => self.a.values(p)?.first().is_none_or(|v| *v != 0.0),
            None => true,
        };
        let mut m = Mat4::IDENTITY;
        let mut ch = 0usize;
        for op in ops {
            let kind = (op as u32) >> 4;
            let n = match kind {
                0 | 1 => 3,
                2 => 4,
                3 => 16,
                4..=6 => 1,
                _ => return Err(malformed(KIND, format!("unknown xform operation {kind}"))),
            };
            let Some(v) = vals.get(ch..ch + n) else { return Err(malformed(KIND, "xform has fewer values than its operations need")) };
            ch += n;
            let v: Vec<f32> = v.iter().map(|x| *x as f32).collect();
            let v3 = |i: usize| Vec3::new(v.get(i).copied().unwrap_or(0.0), v.get(i + 1).copied().unwrap_or(0.0), v.get(i + 2).copied().unwrap_or(0.0));
            let angle = |i: usize| v.get(i).copied().unwrap_or(0.0).to_radians();
            // Ops apply right to left (the first op is outermost), angles in degrees.
            let op_m = match kind {
                0 => Mat4::from_scale(v3(0)),
                1 => Mat4::from_translation(v3(0)),
                2 => v3(0).try_normalize().map_or(Mat4::IDENTITY, |axis| Mat4::from_axis_angle(axis, angle(3))),
                // Row-major with row vectors: read as columns it is the column-vector matrix.
                3 => Mat4::from_cols_slice(&v),
                4 => Mat4::from_rotation_x(angle(0)),
                5 => Mat4::from_rotation_y(angle(0)),
                _ => Mat4::from_rotation_z(angle(0)),
            };
            m *= op_m;
        }
        if !m.is_finite() {
            return Err(malformed(KIND, "xform has non-finite values"));
        }
        Ok((m, inherits))
    }

    /// Face set children of a PolyMesh object: (name, faces), in header order.
    fn face_sets(&mut self, mesh: &A::Handle) -> Result<Vec<(String, Vec<f64>)>> {
        let mut out = Vec::new();
        for (name, meta, child) in self.a.children(mesh)? {
            let schema = meta_get(&meta, "schema").unwrap_or_default();
            if !schema.starts_with("AbcGeom_FaceSet") {
                continue;
            }
            let props = self.a.properties(&child)?;
            let Some(fs) = schema_compound(&props, schema, ".faceset") else { continue };
            let inner = self.a.compound(fs)?;
            if let Some(faces) = find(&inner, ".faces") {
                out.push((name, self.a.values(faces)?));
            }
        }
        Ok(out)
    }

    /// A geometry parameter `name`: an array property, or a compound of `.vals` and `.indices`.
    fn param(&mut self, props: &[Prop<A::Handle>], name: &str, counts: [usize; 3]) -> Result<Option<Param>> {
        let Some(p) = find(props, name) else { return Ok(None) };
        let (vals, indices) = match p.kind {
            PropKind::Array => (self.a.values(p)?, None),
            PropKind::Compound => {
                let inner = self.a.compound(p)?;
                let Some(v) = find(&inner, ".vals") else { return Ok(None) };
                let vals = self.a.values(v)?;
                let indices = match find(&inner, ".indices") {
                    Some(ix) => Some(self.a.values(ix)?),
                    None => None,
                };
                (vals, indices)
            }
            PropKind::Scalar => return Ok(None),
        };
        let width = p.extent.max(1);
        let items = indices.as_ref().map_or(vals.len() / width, Vec::len);
        let [points, faces, corners] = counts;
        let scope = match meta_get(&p.meta, "geoScope") {
            Some("con") => Scope::Constant,
            Some("uni") => Scope::Uniform,
            Some("vtx" | "var") => Scope::Vertex,
            Some("fvr") => Scope::FaceVarying,
            // No scope written: guess from the item count.
            _ if items == corners => Scope::FaceVarying,
            _ if items == points => Scope::Vertex,
            _ if items == faces => Scope::Uniform,
            _ => Scope::Constant,
        };
        Ok(Some(Param { vals, indices, scope }))
    }

    fn mesh(&mut self, geom: &Prop<A::Handle>, world: Mat4, sets: &[(String, Vec<f64>)]) -> Result<()> {
        let properties = self.a.compound(geom)?;
        let get = |a: &mut A, name: &str| -> Result<Vec<f64>> {
            match find(&properties, name) {
                Some(p) => a.values(p),
                None => Err(malformed(KIND, format!("PolyMesh has no {name}"))),
            }
        };

        let p = get(&mut self.a, "P")?;
        let indices = get(&mut self.a, ".faceIndices")?;
        let counts = get(&mut self.a, ".faceCounts")?;
        check_vertices(p.len() / 3)?;
        let points: Vec<Vec3> = p.as_chunks::<3>().0.iter().map(|c| world.transform_point3(Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32))).collect();
        let n = [points.len(), counts.len(), indices.len()];
        let normals = self.param(&properties, "N", n)?;
        let uvs = self.param(&properties, "uv", n)?;

        let mut material = vec![None::<usize>; counts.len()];
        for (s, (_, faces)) in sets.iter().enumerate() {
            for f in faces {
                if let Some(m) = material.get_mut(*f as usize).filter(|m| m.is_none() && *f >= 0.0) {
                    *m = Some(s);
                }
            }
        }
        let normal_transform = Mat3::from_mat4(world).inverse().transpose();

        // Alembic winds polygons clockwise; a mirroring transform flips them once more
        let keep_order = world.determinant() < 0.0;
        let (mut k, mut corners, mut bad_uvs) = (0usize, Vec::new(), 0usize);
        for (f, &count) in counts.iter().enumerate() {
            let end = (count >= 0.0).then(|| k.checked_add(count as usize)).flatten().filter(|e| *e <= indices.len());
            let end = end.ok_or_else(|| malformed(KIND, format!("face {f} has {count} corners past the index list")))?;
            corners.clear();
            for (corner_index, &point_index) in indices.iter().enumerate().take(end).skip(k) {
                let p = points
                    .get(point_index as usize)
                    .filter(|_| point_index >= 0.0)
                    .ok_or_else(|| malformed(KIND, format!("point index {point_index} out of range")))?;
                let point_index = point_index as usize;
                let n = normals
                    .as_ref()
                    .and_then(|l| l.get::<3>(corner_index, point_index, f))
                    .map(|[x, y, z]| normal_transform * Vec3::new(x as f32, y as f32, z as f32));
                let uv = uvs.as_ref().map(|l| match l.get::<2>(corner_index, point_index, f) {
                    Some([u, v]) => Vec2::new(u as f32, 1.0 - v as f32),
                    None => {
                        bad_uvs += 1;
                        Vec2::ZERO
                    }
                });
                corners.push(Corner { p: *p, n, uv });
            }
            k = end;
            if !keep_order {
                corners.reverse();
            }
            let name = material.get(f).copied().flatten().and_then(|s| sets.get(s)).map_or(DEFAULT_MATERIAL, |(n, _)| n.as_str());
            self.sink.polygon(name, &corners)?;
        }

        if bad_uvs > 0 {
            self.sink.warn(format!("{bad_uvs} polygon corners have no UV in the Alembic uv parameter (set to 0, 0)"));
        }

        Ok(())
    }
}
