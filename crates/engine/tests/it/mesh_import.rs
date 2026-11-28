//! Importing a mesh file that embeds PBR textures: the textures become a fill layer at the
//! bottom of the material's texture set, so the surface is no longer the default gray.

use serde_json::json;
use texturecraft_engine::core::{Channel, Fill, LayerKind, Source};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, geom::Mesh};

/// A small uniform PNG image (4 × 2) of one RGBA color.
fn png(rgba: [u8; 4]) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(4, 2, image::Rgba(rgba));
    let mut png = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    png
}

/// A GLB of the built-in plane with one material "Rust" whose PBR slots are embedded images, in
/// the order base color, metallic-roughness, normal, occlusion, emissive (`None` leaves a slot out).
fn pbr_glb(images: [Option<Vec<u8>>; 5]) -> Vec<u8> {
    let mesh: Mesh = primitives::builtin("plane").unwrap();
    let mut bin = Vec::new();
    let mut view = |bytes: &[u8]| {
        let offset = bin.len();
        bin.extend_from_slice(bytes);
        bin.resize(bin.len().next_multiple_of(4), 0);
        json!({ "buffer": 0, "byteOffset": offset, "byteLength": bytes.len() })
    };
    let floats = |v: Vec<f32>| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let mut views = vec![
        view(&floats(mesh.positions.iter().flat_map(|p| [p.x, p.y, p.z]).collect())),
        view(&floats(mesh.uvs.iter().flat_map(|uv| [uv.x, uv.y]).collect())),
        view(&mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect::<Vec<u8>>()),
    ];
    let mut image_views = Vec::new();
    for image in images.iter().flatten() {
        image_views.push(view(image));
    }
    views.extend(image_views.iter().cloned());

    let (lo, hi) = mesh.bounds();
    // Each present image gets the next texture index; absent slots are left out
    let mut index = 0;
    let mut slot = |present: bool| {
        present.then(|| {
            index += 1;
            json!({ "index": index - 1 })
        })
    };
    let mut pbr = json!({});
    let mut material = json!({ "name": "Rust" });
    for (key, image, in_pbr) in [
        ("baseColorTexture", &images[0], true),
        ("metallicRoughnessTexture", &images[1], true),
        ("normalTexture", &images[2], false),
        ("occlusionTexture", &images[3], false),
        ("emissiveTexture", &images[4], false),
    ] {
        if let Some(texture) = slot(image.is_some()) {
            if in_pbr { pbr[key] = texture } else { material[key] = texture }
        }
    }
    material["pbrMetallicRoughness"] = pbr;

    let n_images = images.iter().filter(|i| i.is_some()).count();
    let doc = json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "name": "Plane", "mesh": 0 }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0, "TEXCOORD_0": 1 }, "indices": 2, "material": 0 }] }],
        "materials": [material],
        "textures": (0..n_images).map(|i| json!({ "source": i })).collect::<Vec<_>>(),
        "images": (0..n_images).map(|i| json!({ "bufferView": 3 + i, "mimeType": "image/png" })).collect::<Vec<_>>(),
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": views,
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": mesh.positions.len(), "type": "VEC3", "min": [lo.x, lo.y, lo.z], "max": [hi.x, hi.y, hi.z] },
            { "bufferView": 1, "componentType": 5126, "count": mesh.positions.len(), "type": "VEC2" },
            { "bufferView": 2, "componentType": 5125, "count": mesh.indices.len(), "type": "SCALAR" }
        ]
    });
    let mut text = serde_json::to_vec(&doc).unwrap();
    text.resize(text.len().next_multiple_of(4), b' ');
    let mut glb = Vec::new();
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&((12 + 8 + text.len() + 8 + bin.len()) as u32).to_le_bytes());
    for (chunk, kind) in [(&text, b"JSON"), (&bin, b"BIN\0")] {
        glb.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        glb.extend_from_slice(kind);
        glb.extend_from_slice(chunk);
    }
    glb
}

/// The fill layer at the bottom of texture set `ts`'s stack, if any.
fn bottom_fill(s: &Session, ts: usize) -> Option<(u64, Fill)> {
    let set = s.doc.project.texture_set(ts).ok()?;
    let id = *set.stack.first()?;
    let layer = s.doc.project.layer(id).ok()?;
    match &layer.kind {
        LayerKind::Fill(f) => Some((id.0, f.clone())),
        _ => None,
    }
}

/// The decoded first texel of the image resource a channel's source reads.
fn source_texel(s: &Session, fill: &Fill, channel: Channel) -> [f32; 4] {
    let Some(Source::Image { name, .. }) = fill.sources.get(&channel) else { panic!("{channel:?} is not an image source: {:?}", fill.sources.get(&channel)) };
    let bytes = s.doc.resources.get(name).unwrap_or_else(|| panic!("resource {name} is missing"));
    let img = image::load_from_memory(bytes).unwrap().into_rgba8();
    let p = img.get_pixel(0, 0).0;
    p.map(|v| f32::from(v) / 255.0)
}

/// The composited value of `channel` at the middle of texture set 0 (linear; scalars repeated).
fn composite(s: &mut Session, channel: Channel) -> [f32; 3] {
    s.update();
    let res = s.doc.project.texture_set(0).unwrap().resolution;
    s.caches[0].composite.value(channel, res / 2, res / 2)
}

#[test]
fn glb_embedded_pbr_textures_become_a_fill_layer() {
    let base_color = png([200, 90, 40, 255]);
    let metallic_roughness = png([0, 160, 220, 255]);
    let normal = png([128, 128, 255, 255]);
    let occlusion = png([77, 0, 0, 255]);
    let glb = pbr_glb([Some(base_color.clone()), Some(metallic_roughness.clone()), Some(normal.clone()), Some(occlusion.clone()), None]);

    let mut s = Session::from_mesh_bytes("rost.glb", &glb, 128, "rost").unwrap();

    // The stack has the imported fill at the bottom, the empty paint layer on top of it.
    let set = s.doc.project.texture_set(0).unwrap();
    assert_eq!(set.stack.len(), 2, "one fill below the paint layer: {:?}", set.stack);
    assert!(matches!(s.doc.project.layer(set.stack[1]).unwrap().kind, LayerKind::Paint));
    let (_, fill) = bottom_fill(&s, 0).expect("the embedded textures made a fill layer");

    // Base color keeps the embedded image as its source.
    let texel = source_texel(&s, &fill, Channel::BaseColor);
    assert!((texel[0] - 200.0 / 255.0).abs() < 1e-3 && (texel[1] - 90.0 / 255.0).abs() < 1e-3 && (texel[2] - 40.0 / 255.0).abs() < 1e-3, "{texel:?}");

    // The packed metallic-roughness image is split: roughness reads green, metallic blue.
    let rough = source_texel(&s, &fill, Channel::Roughness)[0];
    let metal = source_texel(&s, &fill, Channel::Metallic)[0];
    assert!((rough - 160.0 / 255.0).abs() < 0.01, "roughness from the green channel: {rough}");
    assert!((metal - 220.0 / 255.0).abs() < 0.01, "metallic from the blue channel: {metal}");

    // The normal map is applied as-is (glTF's OpenGL convention) and occlusion reads red.
    let n = source_texel(&s, &fill, Channel::Normal);
    assert!((n[0] - 128.0 / 255.0).abs() < 1e-3 && (n[2] - 1.0).abs() < 1e-3, "{n:?}");
    let ao = source_texel(&s, &fill, Channel::AmbientOcclusion)[0];
    assert!((ao - 77.0 / 255.0).abs() < 0.01, "occlusion from the red channel: {ao}");
    assert!(s.doc.project.texture_set(0).unwrap().has_channel(Channel::AmbientOcclusion), "the occlusion channel was added");

    // The composited surface is the embedded base color, not the default gray.
    let base = composite(&mut s, Channel::BaseColor);
    let gray = [0.5, 0.5, 0.5];
    assert!(base.iter().zip(gray).any(|(v, g)| (v - g).abs() > 0.05), "not the default gray: {base:?}");
    let lin = |v: f32| texturecraft_engine::texel::srgb_to_linear(v);
    assert!(
        (base[0] - lin(200.0 / 255.0)).abs() < 0.02 && (base[1] - lin(90.0 / 255.0)).abs() < 0.02 && (base[2] - lin(40.0 / 255.0)).abs() < 0.02,
        "{base:?}"
    );
    let rough = composite(&mut s, Channel::Roughness)[0];
    let metal = composite(&mut s, Channel::Metallic)[0];
    assert!((rough - 160.0 / 255.0).abs() < 0.02 && (metal - 220.0 / 255.0).abs() < 0.02, "rough {rough} metal {metal}");

    // Saved and reopened, the fill and its resources are still there and still composite.
    let reopened = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();
    let (_, fill) = bottom_fill(&reopened, 0).expect("the fill layer survives a save and reopen");
    assert!(fill.sources.contains_key(&Channel::BaseColor) && fill.sources.contains_key(&Channel::Metallic));
    let mut reopened = reopened;
    let base = composite(&mut reopened, Channel::BaseColor);
    assert!(base.iter().zip(gray).any(|(v, g)| (v - g).abs() > 0.05), "still not gray after reopen: {base:?}");
}

/// With generated UVs the embedded textures would not line up, so they are not applied, and
/// the log says why.
#[test]
fn glb_textures_are_not_applied_on_generated_uvs() {
    let glb = pbr_glb([Some(png([200, 90, 40, 255])), None, None, None, None]);
    let s = Session::from_mesh_bytes_with_uvs("rost.glb", &glb, 128, "rost", None, true).unwrap();
    assert_eq!(s.doc.project.texture_set(0).unwrap().stack.len(), 1, "only the empty paint layer");
    assert!(s.log.iter().any(|l| l.contains("were not applied")), "{:?}", s.log);
}

/// Absent slots leave gaps: a base color and a normal map alone still both arrive.
#[test]
fn glb_with_some_slots_missing_applies_the_present_ones() {
    let glb = pbr_glb([Some(png([200, 90, 40, 255])), None, Some(png([128, 128, 255, 255])), None, None]);
    let s = Session::from_mesh_bytes("rost.glb", &glb, 128, "rost").unwrap();
    let (_, fill) = bottom_fill(&s, 0).expect("a fill layer");
    assert!(matches!(fill.sources.get(&Channel::BaseColor), Some(Source::Image { .. })), "{:?}", fill.sources);
    assert!(matches!(fill.sources.get(&Channel::Normal), Some(Source::Image { .. })), "{:?}", fill.sources);
}

#[test]
fn glb_material_with_only_factors_keeps_the_plain_import() {
    // No textures at all: the layer stack stays the empty paint layer (unchanged behaviour).
    let mesh: Mesh = primitives::builtin("plane").unwrap();
    let mut bin = Vec::new();
    let floats = |v: Vec<f32>| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    bin.extend_from_slice(&floats(mesh.positions.iter().flat_map(|p| [p.x, p.y, p.z]).collect()));
    bin.extend_from_slice(&floats(mesh.uvs.iter().flat_map(|uv| [uv.x, uv.y]).collect()));
    bin.extend_from_slice(&mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect::<Vec<u8>>());
    let (lo, hi) = mesh.bounds();
    let doc = json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0 }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0, "TEXCOORD_0": 1 }, "indices": 2, "material": 0 }] }],
        "materials": [{ "name": "Rust", "pbrMetallicRoughness": { "baseColorFactor": [0.8, 0.2, 0.1, 1.0] } }],
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": mesh.positions.len() * 12 },
            { "buffer": 0, "byteOffset": mesh.positions.len() * 12, "byteLength": mesh.positions.len() * 8 },
            { "buffer": 0, "byteOffset": mesh.positions.len() * 20, "byteLength": mesh.indices.len() * 4 }
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": mesh.positions.len(), "type": "VEC3", "min": [lo.x, lo.y, lo.z], "max": [hi.x, hi.y, hi.z] },
            { "bufferView": 1, "componentType": 5126, "count": mesh.positions.len(), "type": "VEC2" },
            { "bufferView": 2, "componentType": 5125, "count": mesh.indices.len(), "type": "SCALAR" }
        ]
    });
    let mut text = serde_json::to_vec(&doc).unwrap();
    text.resize(text.len().next_multiple_of(4), b' ');
    let mut glb = Vec::new();
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&((12 + 8 + text.len() + 8 + bin.len()) as u32).to_le_bytes());
    glb.extend_from_slice(&(text.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&text);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin);

    let mut s = Session::from_mesh_bytes("plain.glb", &glb, 128, "plain").unwrap();
    assert!(bottom_fill(&s, 0).is_none(), "a material without textures adds no fill layer");
    let base = composite(&mut s, Channel::BaseColor);
    assert!(base.iter().all(|v| (v - 0.5).abs() < 0.01), "still the default gray: {base:?}");
}
