//! Stanford PLY (ascii, binary little and big endian): vertex positions, normals, UVs and colors,
//! and faces of any size (fan-triangulated). Other elements and properties are skipped.

use texturecraft_geom::glam::{Vec2, Vec3, Vec4};

use super::{RawMesh, check_triangles, check_vertices};
use crate::{Result, malformed};

const KIND: &str = "PLY";
/// Longest polygon accepted (a list count above this is treated as corruption).
const MAX_FACE_VERTICES: usize = 4096;

use craft_mesh::ply::{Encoding as SharedEncoding, Header, Rows, Scalar as Ty};

fn parse_type(s: &str) -> Option<Ty> {
    Some(match s {
        "char" | "int8" => Ty::I8,
        "uchar" | "uint8" => Ty::U8,
        "short" | "int16" => Ty::I16,
        "ushort" | "uint16" => Ty::U16,
        "int" | "int32" => Ty::I32,
        "uint" | "uint32" => Ty::U32,
        "float" | "float32" => Ty::F32,
        "double" | "float64" => Ty::F64,
        _ => return None,
    })
}

struct Property {
    name: String,
    ty: Ty,
    /// Count type of a list property.
    list: Option<Ty>,
}

struct Element {
    name: String,
    count: usize,
    props: Vec<Property>,
}

#[derive(Clone, Copy, PartialEq)]
enum Encoding {
    Ascii,
    Le,
    Be,
}

pub(crate) fn read(bytes: &[u8]) -> Result<RawMesh> {
    let (elements, enc, body_start) = header(bytes)?;
    // Keep the strict app header grammar and mesh policy; delegate only wire scalars.
    let encoding = match enc {
        Encoding::Ascii => SharedEncoding::Ascii,
        Encoding::Le => SharedEncoding::Little,
        Encoding::Be => SharedEncoding::Big,
    };
    let shared_header = Header { encoding, elements: Vec::new(), body: body_start, comments: Vec::new() };
    let mut body = Rows::new(&shared_header, bytes);
    let mut raw = RawMesh::default();
    let (mut normals, mut uvs, mut colors) = (Vec::new(), Vec::new(), Vec::new());
    // Elements may appear in any order, so take the vertex count from the header.
    let nverts: usize = elements.iter().filter(|e| e.name == "vertex").map(|e| e.count).fold(0, usize::saturating_add);
    // The shared ASCII reader counts the delimiter after a value as read; the previous reader
    // left it in the remaining bytes. Only an element without properties can notice the difference.
    let ascii_slack = usize::from(enc == Encoding::Ascii);
    for el in &elements {
        // An element without properties reads nothing, so its rows are skipped. Its count stays
        // bounded by the bytes left, as the previous reader counted them.
        if el.props.is_empty() {
            if el.count > body.remaining().saturating_add(ascii_slack) {
                return Err(malformed(KIND, format!("element \"{}\" claims {} items but the file is too short", el.name, el.count)));
            }
            // A vertex element still needs positions, as below.
            if el.name == "vertex" {
                return Err(malformed(KIND, "vertices have no x, y, z"));
            }
            continue;
        }
        // Every item takes at least one byte, so the count can never exceed what is left.
        if el.count > body.remaining() {
            return Err(malformed(KIND, format!("element \"{}\" claims {} items but the file is too short", el.name, el.count)));
        }
        let find = |names: &[&str]| el.props.iter().position(|p| p.list.is_none() && names.contains(&p.name.as_str()));
        let (px, py, pz) = (find(&["x"]), find(&["y"]), find(&["z"]));
        let (nx, ny, nz) = (find(&["nx"]), find(&["ny"]), find(&["nz"]));
        let (tu, tv) = (find(&["s", "u", "texture_u"]), find(&["t", "v", "texture_v"]));
        let (cr, cg, cb, ca) = (find(&["red", "r"]), find(&["green", "g"]), find(&["blue", "b"]), find(&["alpha", "a"]));
        let has_color = cr.is_some() && cg.is_some() && cb.is_some();
        let vertex = el.name == "vertex";
        let has_n = nx.is_some() && ny.is_some() && nz.is_some();
        let has_uv = tu.is_some() && tv.is_some();
        if vertex {
            if px.is_none() || py.is_none() || pz.is_none() {
                return Err(malformed(KIND, "vertices have no x, y, z"));
            }
            check_vertices(el.count)?;
            raw.positions.reserve(el.count);
        }
        let face_list =
            (el.name == "face").then(|| el.props.iter().position(|p| p.list.is_some() && (p.name == "vertex_indices" || p.name == "vertex_index"))).flatten();
        let mut vals = vec![0.0f64; el.props.len()];
        let mut poly: Vec<u32> = Vec::new();
        for _ in 0..el.count {
            for (k, p) in el.props.iter().enumerate() {
                match p.list {
                    None => {
                        if let Some(v) = vals.get_mut(k) {
                            *v = body.value(p.ty).map_err(|e| malformed(KIND, e.to_string()))?;
                        }
                    }
                    Some(ct) => {
                        let len = body.value(ct).map_err(|e| malformed(KIND, e.to_string()))?;
                        if !(0.0..=MAX_FACE_VERTICES as f64).contains(&len) {
                            return Err(malformed(KIND, format!("bad list length {len}")));
                        }
                        poly.clear();
                        for _ in 0..len as usize {
                            let v = body.value(p.ty).map_err(|e| malformed(KIND, e.to_string()))?;
                            if face_list == Some(k) {
                                if !(0.0..nverts as f64).contains(&v) {
                                    return Err(malformed(KIND, format!("face index {v} out of range")));
                                }
                                poly.push(v as u32);
                            }
                        }
                        if face_list == Some(k) && poly.len() >= 3 {
                            check_triangles(raw.triangle_count(), poly.len() - 2)?;
                            raw.polygon("Default", "", &poly);
                        }
                    }
                }
            }
            if vertex {
                let g = |i: Option<usize>| i.and_then(|i| vals.get(i)).copied().unwrap_or(0.0) as f32;
                raw.positions.push(Vec3::new(g(px), g(py), g(pz)));
                if has_n {
                    normals.push(Vec3::new(g(nx), g(ny), g(nz)));
                }
                if has_uv {
                    // PLY texture coordinates follow OpenGL (origin bottom-left).
                    uvs.push(Vec2::new(g(tu), 1.0 - g(tv)));
                }
                if has_color {
                    // Integer channels are normalized by their type's maximum; floats are 0..1.
                    let c = |i: Option<usize>, default: f32| match i.and_then(|i| el.props.get(i)) {
                        Some(p) => g(i) / color_scale(p.ty),
                        None => default,
                    };
                    colors.push(Vec4::new(c(cr, 1.0), c(cg, 1.0), c(cb, 1.0), c(ca, 1.0)));
                }
            }
        }
    }
    let n = raw.positions.len();
    raw.normals = (normals.len() == n).then_some(normals);
    raw.uvs = (uvs.len() == n).then_some(uvs);
    raw.colors = (colors.len() == n).then_some(colors);
    Ok(raw)
}

fn color_scale(ty: Ty) -> f32 {
    match ty {
        Ty::U8 | Ty::I8 => 255.0,
        Ty::U16 | Ty::I16 => 65535.0,
        Ty::U32 | Ty::I32 => u32::MAX as f32,
        Ty::F32 | Ty::F64 => 1.0,
    }
}

fn header(bytes: &[u8]) -> Result<(Vec<Element>, Encoding, usize)> {
    if !bytes.starts_with(b"ply") {
        return Err(malformed(KIND, "missing \"ply\" signature"));
    }
    let mut pos = 0usize;
    let mut enc = None;
    let mut elements: Vec<Element> = Vec::new();
    loop {
        let rest = bytes.get(pos..).unwrap_or_default();
        let Some(nl) = rest.iter().position(|b| *b == b'\n') else { return Err(malformed(KIND, "header has no end_header")) };
        let line = std::str::from_utf8(rest.get(..nl).unwrap_or_default()).map_err(|_| malformed(KIND, "header is not text"))?.trim();
        pos += nl + 1;
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["end_header"] => break,
            ["format", f, ..] => {
                enc = Some(match *f {
                    "ascii" => Encoding::Ascii,
                    "binary_little_endian" => Encoding::Le,
                    "binary_big_endian" => Encoding::Be,
                    other => return Err(malformed(KIND, format!("unknown format {other}"))),
                })
            }
            ["element", name, count] => {
                let count = count.parse::<usize>().map_err(|_| malformed(KIND, format!("bad element count {count}")))?;
                elements.push(Element { name: name.to_string(), count, props: Vec::new() });
            }
            ["property", "list", ct, ty, name] => {
                let el = elements.last_mut().ok_or_else(|| malformed(KIND, "property before element"))?;
                let (ct, ty) = (parse_type(ct), parse_type(ty));
                let (Some(ct), Some(ty)) = (ct, ty) else { return Err(malformed(KIND, format!("bad list types in \"{line}\""))) };
                el.props.push(Property { name: name.to_string(), ty, list: Some(ct) });
            }
            ["property", ty, name] => {
                let el = elements.last_mut().ok_or_else(|| malformed(KIND, "property before element"))?;
                let ty = parse_type(ty).ok_or_else(|| malformed(KIND, format!("bad property type {ty}")))?;
                el.props.push(Property { name: name.to_string(), ty, list: None });
            }
            ["ply"] | [] | ["comment", ..] | ["obj_info", ..] => {}
            _ => return Err(malformed(KIND, format!("unexpected header line \"{line}\""))),
        }
    }
    let enc = enc.ok_or_else(|| malformed(KIND, "header has no format line"))?;
    Ok((elements, enc, pos))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod shared_rows_tests {
    use super::*;

    #[test]
    fn trailing_propertyless_element_after_a_newline_still_opens() {
        let bytes = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nelement edge 1\nend_header\n0 0 0\n";
        assert_eq!(read(bytes).unwrap().positions, vec![Vec3::new(0.0, 0.0, 0.0)]);
        let huge = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nelement edge 3\nend_header\n0 0 0\n";
        assert!(read(huge).is_err());
    }

    #[test]
    fn propertyless_vertex_element_is_still_rejected() {
        let bytes = b"ply\nformat ascii 1.0\nelement vertex 1\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n3 0 0 0\n";
        let Err(e) = read(bytes) else { panic!("a vertex element without x, y, z must not open") };
        let err = e.to_string();
        assert!(err.contains("vertices have no x, y, z"), "{err}");
    }

    #[test]
    fn binary_scalar_endianness_matches_ascii() {
        for (format, big) in [("binary_little_endian", false), ("binary_big_endian", true)] {
            let mut bytes =
                format!("ply\nformat {format} 1.0\nelement vertex 1\nproperty short x\nproperty float y\nproperty double z\nend_header\n").into_bytes();
            bytes.extend(if big { (-123i16).to_be_bytes() } else { (-123i16).to_le_bytes() });
            bytes.extend(if big { 1.25f32.to_be_bytes() } else { 1.25f32.to_le_bytes() });
            bytes.extend(if big { (-2.5f64).to_be_bytes() } else { (-2.5f64).to_le_bytes() });
            assert_eq!(read(&bytes).unwrap().positions, vec![Vec3::new(-123.0, 1.25, -2.5)]);
            bytes.pop();
            assert!(read(&bytes).is_err());
        }
        let ascii = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty short x\nproperty float y\nproperty double z\nend_header\n -123 \t1.25\r\n -2.5  ";
        assert_eq!(read(ascii).unwrap().positions, vec![Vec3::new(-123.0, 1.25, -2.5)]);
    }

    #[test]
    fn strict_header_and_invalid_face_policy_remain_local() {
        for extra in ["unknown directive\n", "property nonsense x\n"] {
            let bytes = format!("ply\nformat ascii 1.0\nelement vertex 1\n{extra}end_header\n0");
            assert!(read(bytes.as_bytes()).is_err());
        }
        assert!(read(b"ply\nformat ascii 1.0\ncomment \xff\nend_header\n").is_err());
        let bytes = b"ply\nformat ascii 1.0\nelement face 1\nproperty list uchar int vertex_indices\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n3 0 0 1\n0 0 0\n";
        assert!(read(bytes).is_err()); // Do not turn a bad face into a warning/skip.
    }
}
