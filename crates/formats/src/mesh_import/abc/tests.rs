//! Alembic Ogawa files built by a small writer (the layout the reader expects), then imported.

use texturecraft_geom::glam::Vec3;

use super::*;
use crate::mesh_import::finish;

/// Builds an Ogawa byte stream: data blocks and groups appended, the root set last.
struct Writer {
    b: Vec<u8>,
}

impl Writer {
    fn new() -> Writer {
        Writer { b: vec![0; 16] }
    }

    fn data(&mut self, bytes: &[u8]) -> u64 {
        let at = self.b.len() as u64;
        self.b.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        self.b.extend_from_slice(bytes);
        at | DATA_BIT
    }

    fn group(&mut self, children: &[u64]) -> u64 {
        let at = self.b.len() as u64;
        self.b.extend_from_slice(&(children.len() as u64).to_le_bytes());
        for c in children {
            self.b.extend_from_slice(&c.to_le_bytes());
        }
        at
    }

    fn finish(mut self, root: u64) -> Vec<u8> {
        self.b[..5].copy_from_slice(OGAWA_MAGIC);
        self.b[5] = 0xff;
        self.b[6..8].copy_from_slice(&[0, 1]);
        self.b[8..16].copy_from_slice(&root.to_le_bytes());
        self.b
    }
}

/// A property for [`Abc::compound`]: name, metadata, kind and group.
struct P {
    name: &'static str,
    meta: String,
    info: u32,
    group: u64,
}

/// Alembic structure on top of [`Writer`].
struct Abc {
    w: Writer,
}

const F32: u32 = 10;
const I32: u32 = 6;
const U32: u32 = 5;
const U8: u32 = 1;
const F64: u32 = 11;

impl Abc {
    fn sample(&mut self, raw: &[u8]) -> u64 {
        let mut v = vec![7u8; KEY_BYTES];
        v.extend_from_slice(raw);
        self.w.data(&v)
    }

    fn array(&mut self, name: &'static str, meta: &str, pod: u32, extent: u32, raw: &[u8]) -> P {
        let (d, dims) = (self.sample(raw), self.w.data(&[]));
        let group = self.w.group(&[d, dims]);
        // Array, size hint 2 (u32 fields), inline metadata.
        P { name, meta: meta.into(), info: 2 | (2 << 2) | (pod << 4) | (extent << 12), group }
    }

    fn scalar(&mut self, name: &'static str, pod: u32, extent: u32, raw: &[u8]) -> P {
        let d = self.sample(raw);
        let group = self.w.group(&[d]);
        P { name, meta: String::new(), info: 1 | (2 << 2) | (pod << 4) | (extent << 12), group }
    }

    fn compound(&mut self, name: &'static str, meta: &str, props: Vec<P>) -> P {
        let mut header = Vec::new();
        for p in &props {
            let info = p.info | (0xff << 20);
            header.extend_from_slice(&info.to_le_bytes());
            if info & 3 != 0 {
                header.extend_from_slice(&1u32.to_le_bytes()); // samples
            }
            header.extend_from_slice(&(p.name.len() as u32).to_le_bytes());
            header.extend_from_slice(p.name.as_bytes());
            header.extend_from_slice(&(p.meta.len() as u32).to_le_bytes());
            header.extend_from_slice(p.meta.as_bytes());
        }
        let mut children: Vec<u64> = props.iter().map(|p| p.group).collect();
        children.push(self.w.data(&header));
        let group = self.w.group(&children);
        P { name, meta: meta.into(), info: 2 << 2, group }
    }

    /// An object group: properties and children (name, metadata, group). `indexed` children use
    /// metadata index 1.
    fn object(&mut self, props: Vec<P>, children: &[(&str, &str, u64)]) -> u64 {
        let props = self.compound("", "", props).group;
        let mut header = Vec::new();
        for (name, meta, _) in children {
            header.extend_from_slice(&(name.len() as u32).to_le_bytes());
            header.extend_from_slice(name.as_bytes());
            if meta.is_empty() {
                header.push(1);
            } else {
                header.push(0xff);
                header.extend_from_slice(&(meta.len() as u32).to_le_bytes());
                header.extend_from_slice(meta.as_bytes());
            }
        }
        header.extend_from_slice(&[0; OBJECT_HASH_BYTES]);
        let mut kids = vec![props];
        kids.extend(children.iter().map(|c| c.2));
        kids.push(self.w.data(&header));
        self.w.group(&kids)
    }

    fn archive(mut self, top: u64, indexed_meta: &[&str]) -> Vec<u8> {
        let mut im = Vec::new();
        for m in indexed_meta {
            im.push(m.len() as u8);
            im.extend_from_slice(m.as_bytes());
        }
        let kids = [
            self.w.data(&0i32.to_le_bytes()),
            self.w.data(&10709i32.to_le_bytes()),
            top,
            self.w.data(b"_ai_Application=test"),
            self.w.data(&[]),
            self.w.data(&im),
        ];
        let root = self.w.group(&kids);
        self.w.finish(root)
    }
}

fn f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}
fn i32s(v: &[i32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// A unit quad (two faces of a split square, in face sets "left" / "right") under an Xform
/// translated by (1, 2, 3) and scaled by 2, plus an un-set triangle mesh under the root.
fn sample_file(with_tri: bool) -> Vec<u8> {
    let mut a = Abc { w: Writer::new() };
    // Points of a 2×1 strip in the XY plane, faces wound clockwise seen from +Z (Alembic).
    let p = f32s(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 2.0, 1.0, 0.0]);
    let indices = i32s(&[0, 3, 4, 1, 1, 4, 5, 2]);
    let counts = i32s(&[4, 4]);
    let pp = a.array("P", "interpretation=point", F32, 3, &p);
    let fi = a.array(".faceIndices", "", I32, 1, &indices);
    let fc = a.array(".faceCounts", "", I32, 1, &counts);
    // Indexed face-varying UVs (u = x / 2, v = y).
    let vals = a.array(".vals", "", F32, 2, &f32s(&[0.0, 0.0, 0.5, 0.0, 1.0, 0.0, 0.0, 1.0, 0.5, 1.0, 1.0, 1.0]));
    let uvi = a.array(".indices", "", U32, 1, &i32s(&[0, 3, 4, 1, 1, 4, 5, 2]));
    let uv = a.compound("uv", "geoScope=fvr;podName=float32_t;podExtent=2", vec![vals, uvi]);
    let n = a.array("N", "geoScope=vtx", F32, 3, &f32s(&[0.0, 0.0, 1.0].repeat(6)));
    let geom = a.compound(".geom", "schema=AbcGeom_PolyMesh_v1", vec![pp, fi, fc, uv, n]);
    let left_faces = a.array(".faces", "", I32, 1, &i32s(&[0]));
    let left_fs = a.compound(".faceset", "schema=AbcGeom_FaceSet_v1", vec![left_faces]);
    let left = a.object(vec![left_fs], &[]);
    let right_faces = a.array(".faces", "", I32, 1, &i32s(&[1]));
    let right_fs = a.compound(".faceset", "schema=AbcGeom_FaceSet_v1", vec![right_faces]);
    let right = a.object(vec![right_fs], &[]);
    let fs_meta = "schema=AbcGeom_FaceSet_v1;schemaObjTitle=AbcGeom_FaceSet_v1:.faceset";
    let mesh = a.object(vec![geom], &[("left", fs_meta, left), ("right", fs_meta, right)]);
    // Translate (1, 2, 3) then scale 2: ops apply right to left.
    let ops = a.scalar(".ops", U8, 2, &[0x10, 0x00]);
    let xv: Vec<u8> = [1.0f64, 2.0, 3.0, 2.0, 2.0, 2.0].iter().flat_map(|x| x.to_le_bytes()).collect();
    let vals = a.scalar(".vals", F64, 6, &xv);
    let xform = a.compound(".xform", "schema=AbcGeom_Xform_v3", vec![ops, vals]);
    let xf = a.object(vec![xform], &[("stripShape", "schema=AbcGeom_PolyMesh_v1;schemaObjTitle=AbcGeom_PolyMesh_v1:.geom", mesh)]);
    // A lone triangle without UVs or face sets, metadata through the index.
    let tp = a.array("P", "", F32, 3, &f32s(&[0.0, 0.0, 5.0, 0.0, 1.0, 5.0, 1.0, 0.0, 5.0]));
    let ti = a.array(".faceIndices", "", I32, 1, &i32s(&[0, 1, 2]));
    let tc = a.array(".faceCounts", "", I32, 1, &i32s(&[3]));
    let tgeom = a.compound(".geom", "schema=AbcGeom_PolyMesh_v1", vec![tp, ti, tc]);
    let tri = a.object(vec![tgeom], &[]);
    let mut children = vec![("strip", "schema=AbcGeom_Xform_v3", xf), ("tri", "", tri)];
    children.truncate(if with_tri { 2 } else { 1 });
    let top = a.object(vec![], &children);
    a.archive(top, &["schema=AbcGeom_PolyMesh_v1"])
}

#[test]
fn imports_polymesh_through_xforms_with_face_sets_uvs_and_normals() {
    let raw = read(&sample_file(true)).unwrap();
    let mut names: Vec<(&str, &str)> = raw.groups.iter().map(|(m, p, _)| (m.as_str(), p.as_str())).collect();
    names.sort();
    assert_eq!(names, vec![("Default", "tri"), ("left", "strip"), ("right", "strip")], "meshes take their transform's name");
    assert_eq!(raw.triangle_count(), 5);
    // UVs exist only on the strip, so the whole mesh is unwrapped (with a warning).
    assert!(raw.uvs.is_none() && !raw.warnings.is_empty());
    // World positions: scaled by 2, then translated.
    let (lo, hi) = raw.positions.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    assert_eq!((lo, hi), (Vec3::new(0.0, 0.0, 3.0), Vec3::new(5.0, 4.0, 5.0)));
    // Clockwise Alembic faces become counter-clockwise: the strip faces +Z.
    for (m, _, tris) in &raw.groups {
        if m == "Default" {
            continue;
        }
        for t in tris.chunks(3) {
            let p = |i: u32| raw.positions[i as usize];
            assert!((p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0])).z > 0.0);
        }
    }
    let mesh = finish(raw).unwrap().mesh;
    assert_eq!(mesh.materials.len(), 3);
    // The strip alone keeps its indexed face-varying UVs: u = x / 2 in the file's own space.
    let strip = finish(read(&sample_file(false)).unwrap()).unwrap();
    assert!(!strip.report.uvs_generated);
    for (p, uv) in strip.mesh.positions.iter().zip(&strip.mesh.uvs) {
        let local = (*p - Vec3::new(1.0, 2.0, 3.0)) / 2.0;
        assert!((uv.x - local.x / 2.0).abs() < 1e-6 && (uv.y - (1.0 - local.y)).abs() < 1e-6, "{p} {uv}");
    }
}

#[test]
fn strip_alone_keeps_uvs_flipped_to_v_down() {
    let mut a = Abc { w: Writer::new() };
    let pp = a.array("P", "", F32, 3, &f32s(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0]));
    let fi = a.array(".faceIndices", "", I32, 1, &i32s(&[0, 1, 2]));
    let fc = a.array(".faceCounts", "", I32, 1, &i32s(&[3]));
    // Vertex-scope UVs without geoScope metadata: guessed from the count.
    let uv = a.array("uv", "", F32, 2, &f32s(&[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]));
    let geom = a.compound(".geom", "schema=AbcGeom_PolyMesh_v1", vec![pp, fi, fc, uv]);
    let mesh = a.object(vec![geom], &[]);
    let top = a.object(vec![], &[("m", "", mesh)]);
    let bytes = a.archive(top, &["schema=AbcGeom_PolyMesh_v1"]);
    let imported = crate::import_mesh_bytes("m.abc", &bytes).unwrap();
    assert!(!imported.report.uvs_generated);
    let m = &imported.mesh;
    for (p, uv) in m.positions.iter().zip(&m.uvs) {
        assert_eq!(*uv, texturecraft_geom::glam::Vec2::new(p.x, 1.0 - p.y));
    }
}

#[test]
fn hdf5_and_garbage_are_clear_errors() {
    let e = read(b"\x89HDF\r\n\x1a\n rest of an hdf5 file").err().unwrap().to_string();
    assert!(e.contains("HDF5") && e.contains("Ogawa"), "{e}");
    assert!(read(b"not alembic at all").is_err());
    assert!(read(b"").is_err());
}

#[test]
fn truncated_and_corrupted_files_never_panic() {
    let bytes = sample_file(true);
    for n in (0..bytes.len()).step_by(3) {
        let _ = read(&bytes[..n]);
    }
    let mut rng = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..4000 {
        let mut b = bytes.clone();
        for _ in 0..4 {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let i = (rng % b.len() as u64) as usize;
            b[i] = (rng >> 32) as u8;
        }
        if let Ok(raw) = read(&b) {
            let _ = finish(raw);
        }
    }
}

#[test]
fn self_referencing_groups_are_bounded() {
    // The top object lists itself as its own child: depth / visit limits stop the walk.
    let mut a = Abc { w: Writer::new() };
    let props = a.compound("", "", vec![]).group;
    let at = a.w.b.len() as u64;
    let mut header = Vec::new();
    header.extend_from_slice(&1u32.to_le_bytes());
    header.push(b'x');
    header.push(0);
    header.extend_from_slice(&[0; OBJECT_HASH_BYTES]);
    // Group: [props, itself, header data] — the header block follows the group.
    let data_at = at + 8 + 3 * 8;
    a.w.group(&[props, at, data_at | DATA_BIT]);
    a.w.data(&header);
    let bytes = a.archive(at, &[]);
    assert!(read(&bytes).is_err());
}

/// A triangle mesh under a chain of `xforms` nested Xforms with 63-character names, each
/// translating by +1 in x: the shape of exports with long transform hierarchies.
fn chain_file(xforms: usize) -> Vec<u8> {
    let mut a = Abc { w: Writer::new() };
    let tp = a.array("P", "", F32, 3, &f32s(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0]));
    let ti = a.array(".faceIndices", "", I32, 1, &i32s(&[0, 1, 2]));
    let tc = a.array(".faceCounts", "", I32, 1, &i32s(&[3]));
    let geom = a.compound(".geom", "schema=AbcGeom_PolyMesh_v1", vec![tp, ti, tc]);
    let mesh = a.object(vec![geom], &[]);
    let mut child = ("CubeShape".to_string(), "schema=AbcGeom_PolyMesh_v1;schemaObjTitle=AbcGeom_PolyMesh_v1:.geom", mesh);
    for level in 0..xforms {
        let ops = a.scalar(".ops", U8, 1, &[0x10]);
        let xv: Vec<u8> = [1.0f64, 0.0, 0.0].iter().flat_map(|x| x.to_le_bytes()).collect();
        let vals = a.scalar(".vals", F64, 3, &xv);
        let xform = a.compound(".xform", "schema=AbcGeom_Xform_v3", vec![ops, vals]);
        let group = a.object(vec![xform], &[(child.0.as_str(), child.1, child.2)]);
        child = (format!("{level:0>63}"), "schema=AbcGeom_Xform_v3;schemaObjTitle=AbcGeom_Xform_v3:.xform", group);
    }
    let top = a.object(vec![], &[(child.0.as_str(), child.1, child.2)]);
    a.archive(top, &[])
}

#[test]
fn long_transform_chains_import() {
    // 64 nested Xforms over a mesh (65 levels) used to trip the nesting guard.
    for xforms in [64, 200] {
        let raw = read(&chain_file(xforms)).unwrap_or_else(|e| panic!("{xforms} levels: {e}"));
        assert_eq!(raw.triangle_count(), 1);
        // Every level's translation is inherited.
        let min_x = raw.positions.iter().map(|p| p.x).fold(f32::MAX, f32::min);
        assert_eq!(min_x, xforms as f32);
    }
}

#[test]
fn nesting_past_the_limit_is_an_error() {
    let e = read(&chain_file(MAX_DEPTH + 1)).err().unwrap().to_string();
    assert!(e.contains("nested too deeply"), "{e}");
}
