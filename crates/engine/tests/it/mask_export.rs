//! `mask.export` and `Session::mask_bitmaps`: the evaluated mask as a bitmap at the texture
//! set's resolution, oriented like the exported textures.

use std::path::PathBuf;

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, LayerId};
use texturecraft_engine::geom::{NO_TRI, primitives};
use texturecraft_engine::{Session, execute};

/// A new folder for one test's files.
fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texturecraft-mask-export-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A session of `mesh` with a fill layer whose mask is `value` wherever the mesh covers the texture.
fn session_with_mask(mesh: texturecraft_engine::geom::Mesh, name: &str, resolution: u32, value: f32) -> (Session, LayerId) {
    let mut s = Session::new_project("t", mesh, name, None, resolution).unwrap();
    let layer = LayerId(execute(&mut s, "layer.add.fill", &json!({"color": [1.0, 1.0, 1.0]})).unwrap()["layer"].as_u64().unwrap());
    execute(&mut s, "mask.add.black", &Value::Null).unwrap();
    execute(&mut s, "effect.add.fill", &json!({"value": value})).unwrap();
    s.update();
    (s, layer)
}

fn covered(s: &Session, x: u32, y: u32) -> bool {
    let g = &s.caches[0].gbuf;
    g.tri[(y * g.width + x) as usize] != NO_TRI
}

#[test]
fn the_bitmap_is_the_mask_the_composite_is_drawn_with_in_the_same_orientation() {
    let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 256).unwrap();
    let layer = LayerId(execute(&mut s, "layer.add.fill", &json!({"color": [1.0, 1.0, 1.0]})).unwrap()["layer"].as_u64().unwrap());

    // A white layer gives output = base + (1 - base) * mask, so its mask can be recovered
    execute(&mut s, "layer.visible", &json!({"layer": layer.0, "visible": false})).unwrap();
    s.update();
    let base: Vec<f32> = (0..256 * 256).map(|i| s.caches[0].composite.value(Channel::BaseColor, i % 256, i / 256)[0]).collect();
    execute(&mut s, "layer.visible", &json!({"layer": layer.0, "visible": true})).unwrap();
    execute(&mut s, "mask.add.black", &Value::Null).unwrap();
    execute(&mut s, "effect.add.fill", &json!({"procedural": "clouds", "seed": 3})).unwrap();
    s.update();

    let bitmaps = s.mask_bitmaps(layer, true).unwrap();
    assert_eq!(bitmaps.len(), 1, "a classic set gives one bitmap");
    let bitmap = &bitmaps[0];
    assert_eq!((bitmap.tile, bitmap.width, bitmap.height), (None, 256, 256));
    assert_eq!(bitmap.values.len(), 256 * 256);

    let (mut covered_texels, mut distinct) = (0, std::collections::BTreeSet::new());
    for y in 0..256u32 {
        for x in 0..256u32 {
            let value = bitmap.values[(y * 256 + x) as usize];
            if !covered(&s, x, y) {
                assert_eq!(value, 0.0, "uncovered texel ({x}, {y}) is black");
                continue;
            }

            covered_texels += 1;
            distinct.insert((value * 100.0) as u32);
            let base = base[(y * 256 + x) as usize];
            let shown = s.caches[0].composite.value(Channel::BaseColor, x, y)[0];
            assert!((base + (1.0 - base) * value - shown).abs() < 3e-3, "texel ({x}, {y}): mask {value} does not give the composite {shown} over {base}");
        }
    }
    assert!(covered_texels > 1000, "the mesh covers part of the texture");
    assert!(distinct.len() > 20, "the mask varies across the texture, so a mirrored bitmap would not match the composite");
}

#[test]
fn inverting_changes_the_bitmap_and_a_switched_off_or_missing_mask_is_an_error() {
    let (mut s, layer) = session_with_mask(primitives::material_ball(), "builtin:material_ball", 128, 0.25);
    execute(&mut s, "mask.invert", &json!({"layer": layer.0})).unwrap();
    let bitmap = s.mask_bitmaps(layer, false).unwrap().remove(0);
    let (x, y) = (0..128 * 128).map(|i| (i % 128, i / 128)).find(|&(x, y)| covered(&s, x, y)).unwrap();
    assert!((bitmap.values[(y * 128 + x) as usize] - 0.75).abs() < 1e-3, "inverting 0.25 gives 0.75");
    execute(&mut s, "mask.toggle", &json!({"layer": layer.0, "enabled": false})).unwrap();
    assert!(s.mask_bitmaps(layer, false).is_err(), "a switched-off mask has nothing to export");
    execute(&mut s, "mask.remove", &json!({"layer": layer.0})).unwrap();
    assert!(s.mask_bitmaps(layer, false).is_err(), "so has a layer without a mask");
}

#[test]
fn mask_export_writes_a_16_bit_gray_png() {
    let (mut s, layer) = session_with_mask(primitives::material_ball(), "builtin:material_ball", 128, 0.5);
    let dir = temp_dir("png");
    let path = dir.join("ball_mask.png");
    let done = execute(&mut s, "mask.export", &json!({"layer": layer.0, "path": path.display().to_string()})).unwrap();
    assert_eq!(done["files"], json!([path.display().to_string()]));

    let image = image::open(&path).unwrap();
    assert!(matches!(image, image::DynamicImage::ImageLuma16(_)), "16-bit grayscale, got {:?}", image.color());
    let gray = image.into_luma16();
    assert_eq!(gray.dimensions(), (128, 128));
    let expected = s.mask_bitmaps(layer, true).unwrap().remove(0).gray16();
    assert_eq!(gray.as_raw(), &expected, "the file holds the bitmap, top row first");

    // 0.5 is a level 8-bit files cannot hold exactly
    assert!(gray.pixels().any(|p| p.0[0] == 32768), "the 16-bit level of 0.5");

    // The extension is added, and the layer defaults to the selected one
    let done = execute(&mut s, "mask.export", &json!({"path": dir.join("other").display().to_string()})).unwrap();
    assert_eq!(done["files"], json!([dir.join("other.png").display().to_string()]));
    assert!(dir.join("other.png").exists());
    assert!(execute(&mut s, "mask.export", &json!({"layer": layer.0})).is_err(), "a path is required");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_tiled_set_exports_one_file_per_tile_at_each_tiles_size() {
    let (mut s, layer) = session_with_mask(primitives::udim_plane(), "builtin:udim_plane", 256, 0.5);
    execute(&mut s, "textureSet.tile.resolution", &json!({"ts": 0, "tile": 1002, "resolution": 128})).unwrap();
    s.update();
    let (width, height) = s.caches[0].size;
    assert_eq!((width, height), (512, 256), "two blocks of the larger tile's size");

    let bitmaps = s.mask_bitmaps(layer, true).unwrap();
    assert_eq!(bitmaps.iter().map(|b| (b.tile, b.width, b.height)).collect::<Vec<_>>(), [(Some(1001), 256, 256), (Some(1002), 128, 128)]);

    // The smaller tile fills the top-left corner of its block
    for (bitmap, block_x) in bitmaps.iter().zip([0, 256]) {
        for y in 0..bitmap.height {
            for x in 0..bitmap.width {
                let expected = if covered(&s, block_x + x, y) { 0.5 } else { 0.0 };
                let value = bitmap.values[(y * bitmap.width + x) as usize];
                assert!((value - expected).abs() < 1e-3, "tile {:?} texel ({x}, {y}): {value}", bitmap.tile);
            }
        }
    }
    assert!(bitmaps.iter().all(|b| b.values.iter().any(|v| *v > 0.4)), "both tiles have mask");

    let atlas = s.mask_bitmaps(layer, false).unwrap();
    assert_eq!(atlas.len(), 1);
    assert_eq!((atlas[0].tile, atlas[0].width, atlas[0].height), (None, 512, 256), "the clipboard copy is the whole atlas");

    let dir = temp_dir("tiles");
    let path = dir.join("plane_mask.png");
    let done = execute(&mut s, "mask.export", &json!({"layer": layer.0, "path": path.display().to_string()})).unwrap();
    assert_eq!(done["files"], json!([dir.join("plane_mask.1001.png").display().to_string(), dir.join("plane_mask.1002.png").display().to_string()]));
    assert_eq!(image::open(dir.join("plane_mask.1001.png")).unwrap().into_luma16().dimensions(), (256, 256));
    assert_eq!(image::open(dir.join("plane_mask.1002.png")).unwrap().into_luma16().dimensions(), (128, 128));
    let _ = std::fs::remove_dir_all(dir);
}
