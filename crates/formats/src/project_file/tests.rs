use std::sync::Arc;

use texturecraft_core::{BakedMap, BlendMode, Effect, EffectKind, Fill, GeometryMask, Layer, LayerKind, Mask, MaskBackground, TextureSet};

use super::*;

struct Sample {
    pf: ProjectFile,
    paint: LayerId,
}

fn sample() -> Sample {
    let mut p = Project::new("Demo", "mesh.glb");
    p.texture_sets.push(TextureSet::new("A", 0, 128));
    p.texture_sets.push(TextureSet::new("B", 1, 64));
    let layer = Layer::new(p.alloc_id(), "Paint", LayerKind::Paint, &Channel::DEFAULT_SET);
    let paint = p.insert_layer(0, layer, None, false).unwrap();
    let mut masked = Layer::new(p.alloc_id(), "Masked", LayerKind::Fill(Fill::default()), &Channel::DEFAULT_SET);
    let effect = p.alloc_id();
    masked.mask = Some(Mask { effects: vec![Effect::new(effect, EffectKind::Paint)], ..Mask::new(MaskBackground::Black) });
    p.insert_layer(1, masked, None, false).unwrap();
    let mut pixels = PixelStore::default();
    let img = pixels.layer_mut(paint, Channel::BaseColor, (128, 128));
    img.set(0, 0, &[1, 2, 3, 4]);
    img.set(127, 127, &[65535, 0, 9, 65535]);
    pixels.layer_mut(paint, Channel::Height, (128, 128)).set(70, 3, &[40000, 65535]);
    pixels.mask_mut(effect, (64, 64)).set(10, 60, &[30000, 65535]);
    let ao: Vec<f32> = (0..256).map(|i| i as f32 / 255.0).collect();
    let baked = BakedMaps {
        resolution: 16,
        maps: [(BakedMap::AmbientOcclusion, Arc::new(ao.into())), (BakedMap::Normal, Arc::new(vec![0.5; 16 * 16 * 3].into()))].into(),
    };
    let pf = ProjectFile {
        project: p,
        pixels,
        stroke_replay: None,
        baked: [(0, baked)].into(),
        mesh: Some(("glb".into(), vec![1, 2, 3, 4, 5])),
        high_poly: vec![("b_high.obj".into(), vec![7; 9]), ("a_high.ply".into(), vec![6])],
        cage: Some(("cage.obj".into(), vec![5, 5])),
        bake_paint: [((0, texturecraft_core::BakePaintTarget::Skew), BakePaintData { width: 3, height: 2, values: vec![0.0, 0.25, 0.5, 0.75, 1.0, 0.125] })]
            .into(),
        thumbnail: Some(vec![137, 80, 78, 71]),
        resources: [("bricks.png".to_string(), vec![9, 8, 7])].into(),
        graphs: Default::default(),
        strokes: BTreeMap::new(),
        warnings: Vec::new(),
    };
    Sample { pf, paint }
}

fn assert_same(a: &ProjectFile, b: &ProjectFile) {
    assert_eq!(a.project, b.project);
    assert_eq!(a.pixels.layers.len(), b.pixels.layers.len());
    for (k, img) in &a.pixels.layers {
        assert_eq!(Some(img), b.pixels.layers.get(k), "{k:?}");
    }
    assert_eq!(a.pixels.masks.len(), b.pixels.masks.len());
    for (k, img) in &a.pixels.masks {
        assert_eq!(Some(img), b.pixels.masks.get(k), "{k:?}");
    }
    assert_eq!(a.baked, b.baked);
    assert_eq!(a.mesh, b.mesh);
    assert_eq!((&a.high_poly, &a.cage), (&b.high_poly, &b.cage), "bake meshes keep their order");
    assert_eq!(a.bake_paint, b.bake_paint);
    assert_eq!(a.thumbnail, b.thumbnail);
    assert_eq!(a.resources, b.resources);
}

fn is_guard_error(e: &FormatError) -> bool {
    e.to_string().contains("parser failed")
}

#[test]
fn round_trip_is_identical_and_deterministic() {
    let s = sample();
    let bytes = save_project_bytes(&s.pf).unwrap();
    let back = load_project_bytes(&bytes).unwrap();
    assert!(back.warnings.is_empty());
    assert_same(&s.pf, &back);
    assert_eq!(back.pixels.layer(s.paint, Channel::BaseColor).unwrap().get(127, 127), [65535, 0, 9, 65535]);
    assert_eq!(save_project_bytes(&back).unwrap(), bytes);
    // Built-in mesh projects carry no mesh entry.
    let mut builtin = s.pf.clone();
    builtin.mesh = None;
    builtin.thumbnail = None;
    builtin.project.mesh = "builtin:sphere".into();
    assert_same(&builtin, &load_project_bytes(&save_project_bytes(&builtin).unwrap()).unwrap());
}

#[test]
fn shared_project_schema_round_trips_the_app_document() {
    let project = sample().pf.project;
    let original = serde_json::to_value(&project).unwrap();
    let document: craft_tcraft::ProjectDocument = serde_json::from_value(original.clone()).unwrap();
    assert_eq!(document.format_version, project.format_version);
    assert_eq!(serde_json::to_value(document).unwrap(), original);
}

#[test]
fn legacy_bakes_load_as_is_without_layout_conversion() {
    let mut sample = sample();
    sample.pf.project.format_version = 1;
    sample.pf.pixels.layers.clear();
    let set = &mut sample.pf.project.texture_sets[0];
    set.resolution = 256;
    set.uv_tiles = vec![1001, 1002];
    set.tile_resolutions.insert(1002, 128);
    let pixels: Vec<f32> = (0..16).flat_map(|y| (0..32).map(move |x| (y * 32 + x) as f32)).collect();
    sample.pf.baked.insert(0, BakedMaps { resolution: 32, maps: [(BakedMap::AmbientOcclusion, Arc::new(pixels.into()))].into() });

    let loaded = load_project_bytes(&save_project_bytes(&sample.pf).unwrap()).unwrap();
    assert_eq!(loaded.project.format_version, FORMAT_VERSION);
    assert_eq!(loaded.baked, sample.pf.baked, "old bake pixels are accepted without preserving their former layout");
    assert_eq!(loaded.bake_paint, sample.pf.bake_paint, "correction paint remains in the working layout");
    let reopened = load_project_bytes(&save_project_bytes(&loaded).unwrap()).unwrap();
    assert_same(&loaded, &reopened);
}

#[test]
fn a_bent_normal_declared_without_pixels_in_an_older_project_is_dropped_and_reported() {
    let mut sample = sample();
    sample.pf.project.texture_sets[0].baked.insert(BakedMap::BentNormal);
    assert!(!sample.pf.baked[&0].maps.contains_key(&BakedMap::BentNormal));
    sample.pf.project.format_version = TANGENT_BENT_NORMAL_VERSION - 1;
    let older = load_project_bytes(&save_project_bytes(&sample.pf).unwrap()).unwrap();
    assert!(!older.project.texture_sets[0].baked.contains(&BakedMap::BentNormal));
    assert_eq!(older.warnings.len(), 1, "{:?}", older.warnings);
    assert!(older.warnings[0].contains("\"A\""), "{}", older.warnings[0]);
}

#[test]
fn a_bent_normal_with_pixels_but_no_declaration_in_an_older_project_is_dropped_and_reported() {
    let mut sample = sample();
    let bent = Arc::new([0.5, 1.0, 0.5].repeat(16 * 16).into());
    sample.pf.baked.get_mut(&0).unwrap().maps.insert(BakedMap::BentNormal, bent);
    assert!(!sample.pf.project.texture_sets[0].baked.contains(&BakedMap::BentNormal));
    sample.pf.project.format_version = TANGENT_BENT_NORMAL_VERSION - 1;
    let older = load_project_bytes(&save_project_bytes(&sample.pf).unwrap()).unwrap();
    assert!(!older.baked[&0].maps.contains_key(&BakedMap::BentNormal));
    assert_eq!(older.warnings.len(), 1, "{:?}", older.warnings);
    assert!(older.warnings[0].contains("\"A\""), "{}", older.warnings[0]);
}

#[test]
fn world_space_bent_normals_of_older_projects_are_dropped_for_a_rebake() {
    let mut sample = sample();
    let bent = Arc::new([0.5, 1.0, 0.5].repeat(16 * 16).into());
    sample.pf.baked.get_mut(&0).unwrap().maps.insert(BakedMap::BentNormal, bent);
    sample.pf.project.texture_sets[0].baked.insert(BakedMap::BentNormal);

    // A current project keeps its tangent-space bent normals
    let current = load_project_bytes(&save_project_bytes(&sample.pf).unwrap()).unwrap();
    assert!(current.warnings.is_empty(), "{:?}", current.warnings);
    assert_same(&sample.pf, &current);

    // One saved before tangent space held world vectors: they go, the other maps stay
    sample.pf.project.format_version = TANGENT_BENT_NORMAL_VERSION - 1;
    let older = load_project_bytes(&save_project_bytes(&sample.pf).unwrap()).unwrap();
    assert_eq!(older.warnings.len(), 1, "{:?}", older.warnings);
    assert!(older.warnings[0].contains("\"A\"") && older.warnings[0].contains("Bent Normals"), "{}", older.warnings[0]);
    let maps = &older.baked[&0].maps;
    assert!(!maps.contains_key(&BakedMap::BentNormal));
    assert!(maps.contains_key(&BakedMap::AmbientOcclusion) && maps.contains_key(&BakedMap::Normal));
    assert!(!older.project.texture_sets[0].baked.contains(&BakedMap::BentNormal));
    assert_eq!(older.project.texture_sets[0].bake, sample.pf.project.texture_sets[0].bake, "the bake settings stay for the re-bake");

    // Saved again, the project is current and opens without the warning
    let reopened = load_project_bytes(&save_project_bytes(&older).unwrap()).unwrap();
    assert!(reopened.warnings.is_empty(), "{:?}", reopened.warnings);
    assert_same(&older, &reopened);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn save_and_load_through_files() {
    let s = sample();
    let dir = std::env::temp_dir().join(format!("tc-formats-project-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("demo.tcraft");
    save_project(&path, &s.pf).unwrap();
    save_project(&path, &s.pf).unwrap(); // overwrite in place
    assert!(!dir.join("demo.tcraft.tmp").exists());
    assert_same(&s.pf, &load_project(&path).unwrap());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn newer_format_is_refused() {
    let mut s = sample();
    s.pf.project.format_version = FORMAT_VERSION + 1;
    let err = load_project_bytes(&save_project_bytes(&s.pf).unwrap()).unwrap_err();
    assert!(matches!(err, FormatError::NewerVersion { found, .. } if found == FORMAT_VERSION + 1), "{err}");
    assert!(err.to_string().contains("newer"));
}

#[test]
fn malformed_project_files_are_errors() {
    let bytes = save_project_bytes(&sample().pf).unwrap();
    for len in 0..bytes.len() {
        let e = load_project_bytes(&bytes[..len]).unwrap_err();
        assert!(!is_guard_error(&e), "prefix {len}: {e}");
    }
    for i in (0..bytes.len()).step_by(3) {
        let mut b = bytes.clone();
        b[i] ^= 0x5A;
        if let Err(e) = load_project_bytes(&b) {
            assert!(!is_guard_error(&e), "flip at {i}: {e}");
        }
    }
    assert!(load_project_bytes(b"PK\x03\x04 definitely not a zip").is_err());
}

/// A project zip with `project.json` from `sample()` plus raw extra entries.
fn zip_with(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let s = sample();
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("project.json", SimpleFileOptions::default()).unwrap();
    zip.write_all(&serde_json::to_vec(&s.pf.project).unwrap()).unwrap();
    for (name, data) in entries {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn tile_bin(res: u32, comps: usize, tiles: &[(u32, u32)]) -> Vec<u8> {
    let mut b = header(res, res, comps, tiles.len());
    for (tx, ty) in tiles {
        b.extend(tx.to_le_bytes());
        b.extend(ty.to_le_bytes());
        b.extend(vec![7u8; TILE_AREA * comps * 2]);
    }
    b
}

#[test]
fn tiles_are_validated_against_the_texture_set() {
    let paint = sample().paint;
    let name = format!("pixels/layer_{}_baseColor.bin", paint.0);
    let ok = load_project_bytes(&zip_with(&[(&name, tile_bin(128, 4, &[(1, 1)]))])).unwrap();
    assert_eq!(ok.pixels.layer(paint, Channel::BaseColor).unwrap().get(64, 64), [0x0707; 4]);
    for (what, bin) in [
        ("tile out of range", tile_bin(128, 4, &[(2, 0)])),
        ("repeated tile", tile_bin(128, 4, &[(0, 0), (0, 0)])),
        ("wrong resolution", tile_bin(64, 4, &[(0, 0)])),
        ("wrong components", tile_bin(128, 3, &[(0, 0)])),
        ("truncated", tile_bin(128, 4, &[(0, 0)])[..100].to_vec()),
        ("trailing data", [tile_bin(128, 4, &[]), vec![1]].concat()),
    ] {
        assert!(load_project_bytes(&zip_with(&[(&name, bin)])).is_err(), "{what}");
    }
    let mut newer = tile_bin(128, 4, &[]);
    newer[4] = 9;
    assert!(matches!(load_project_bytes(&zip_with(&[(&name, newer)])), Err(FormatError::NewerVersion { .. })));
    // Pixels of a layer that is in no stack are dropped, not an error.
    let orphan = load_project_bytes(&zip_with(&[("pixels/layer_999_baseColor.bin", tile_bin(128, 4, &[(0, 0)]))])).unwrap();
    assert!(orphan.pixels.layers.is_empty());
    assert!(load_project_bytes(&zip_with(&[("pixels/layer_x_baseColor.bin", vec![])])).is_err());
    assert!(load_project_bytes(&zip_with(&[("baked/0_ambientOcclusion.bin", header(16, 16, 1, 0))])).is_err());
    assert!(load_project_bytes(&zip_with(&[("baked/0_ambientOcclusion.bin", header(100_000, 100_000, 1, 0))])).is_err());
}

#[test]
fn user_channels_round_trip_with_their_names_and_pixels() {
    let mut s = sample();
    let plain = String::from_utf8_lossy(&serde_json::to_vec(&s.pf.project).unwrap()).to_string();
    assert!(!plain.contains("channel_names"), "files without named channels look as before");
    let set = &mut s.pf.project.texture_sets[0];
    set.channels.push((Channel::User3, texturecraft_core::ChannelFormat::L16));
    set.channel_names.insert(Channel::User3, "Wetness".into());
    if let Some(layer) = s.pf.project.layers.get_mut(&s.paint) {
        layer.channels.insert(Channel::User3, Default::default());
    }
    s.pf.pixels.layer_mut(s.paint, Channel::User3, (128, 128)).set(5, 6, &[100, 200, 300, 65535]);
    let back = load_project_bytes(&save_project_bytes(&s.pf).unwrap()).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    assert_same(&s.pf, &back);
    assert_eq!(back.project.texture_sets[0].channel_label(Channel::User3), "Wetness");
    assert_eq!(back.project.texture_sets[0].channel_label(Channel::User4), "User 4");
    assert_eq!(back.pixels.layer(s.paint, Channel::User3).unwrap().get(5, 6), [100, 200, 300, 65535]);
}

#[test]
fn uv_tiled_sets_round_trip_their_atlas_pixels_and_bakes() {
    let mut s = sample();
    s.pf.project.texture_sets[0].uv_tiles = vec![1001, 1002];
    let mut pixels = PixelStore::default();
    pixels.layer_mut(s.paint, Channel::BaseColor, (256, 128)).set(200, 100, &[1, 2, 3, 4]);
    s.pf.pixels = pixels;
    let baked = BakedMaps { resolution: 32, maps: [(BakedMap::AmbientOcclusion, Arc::new(vec![0.25; 32 * 16].into()))].into() };
    s.pf.baked = [(0, baked)].into();
    let back = load_project_bytes(&save_project_bytes(&s.pf).unwrap()).unwrap();
    assert_same(&s.pf, &back);
    assert_eq!(back.baked[&0].height(BakedMap::AmbientOcclusion), 16);
    assert_eq!(back.pixels.layer(s.paint, Channel::BaseColor).unwrap().get(200, 100), [1, 2, 3, 4]);
    // A square image no longer fits the tiled set.
    let mut square = s.pf.clone();
    square.project.texture_sets[0].uv_tiles.clear();
    assert!(load_project_bytes(&save_project_bytes(&square).unwrap()).is_err());
}

#[test]
fn duplicate_entry_names_are_made_unique_on_save() {
    let mut pf = sample().pf;
    pf.high_poly = vec![("model.obj".into(), vec![1]), ("model.obj".into(), vec![2])];
    pf.resources = [("a b".to_string(), vec![3]), ("a_b".to_string(), vec![4])].into();
    let bytes = save_project_bytes(&pf).expect("save with duplicate entry names");
    let back = load_project_bytes(&bytes).unwrap();
    assert_eq!(back.high_poly.len(), 2);
    let mut hp: Vec<_> = back.high_poly.iter().map(|(_, b)| b.clone()).collect();
    hp.sort();
    assert_eq!(hp, vec![vec![1], vec![2]]);
    assert_eq!(back.resources.len(), 2);
    let mut res: Vec<_> = back.resources.values().cloned().collect();
    res.sort();
    assert_eq!(res, vec![vec![3], vec![4]]);
}

/// Every entry of a zip in file order: (name, method, crc, sizes, the compressed bytes as stored).
/// The zip headers around them (version fields, zip64 size placeholders) are not part of this:
/// copying an entry between zips rewrites them.
#[allow(clippy::type_complexity)]
fn payloads(zip: &[u8]) -> Vec<(String, CompressionMethod, u32, u64, u64, Vec<u8>)> {
    let mut a = ZipArchive::new(Cursor::new(zip)).unwrap();
    (0..a.len())
        .map(|i| {
            let mut f = a.by_index_raw(i).unwrap();
            let mut raw = Vec::new();
            f.read_to_end(&mut raw).unwrap();
            (f.name().unwrap().into_owned(), f.compression(), f.crc32(), f.size(), f.compressed_size(), raw)
        })
        .collect()
}

/// A project with several baked maps and pixel layers, big enough that entries stream in chunks.
fn big_sample() -> ProjectFile {
    let mut s = sample();
    let mut x = 12345u32;
    let mut noise = |n: usize| -> Vec<f32> {
        (0..n)
            .map(|i| {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                (i as f32 * 0.001).sin() * 0.5 + 0.5 + (x >> 24) as f32 * 1e-4
            })
            .collect()
    };
    let r = 300usize;
    let maps = [BakedMap::AmbientOcclusion, BakedMap::Normal, BakedMap::Curvature, BakedMap::Position]
        .into_iter()
        .map(|m| (m, Arc::new(noise(r * r * m.components()).into())))
        .collect();
    s.pf.baked.insert(1, BakedMaps { resolution: r as u32, maps });
    s.pf.mesh = Some(("obj".into(), noise(5000).iter().map(|v| (v * 200.0) as u8).collect()));
    s.pf
}

fn serial_bytes(pf: &ProjectFile) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    entries::write_serial(&mut zip, entries(pf).unwrap()).unwrap();
    zip.finish().unwrap().into_inner()
}

#[cfg(not(target_arch = "wasm32"))]
fn parallel_bytes(pf: &ProjectFile, workers: usize, budget: u64) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    entries::parallel::write(&mut zip, entries(pf).unwrap(), workers, budget).unwrap();
    zip.finish().unwrap().into_inner()
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn save_plan_follows_the_reserved_cores_and_the_ram_share() {
    // No cap of 8 workers any more, only the entry count and the cores.
    assert_eq!(entries::plan(64, 1, 100, 1000, 50), (63, 500));
    assert_eq!(entries::plan(64, 1, 20, 1000, 50).0, 20);
    // 0 reserved uses every core; more reserved than cores still leaves one worker.
    assert_eq!(entries::plan(16, 0, 100, 1000, 100), (16, 1000));
    assert_eq!(entries::plan(4, 9, 100, 1000, 100).0, 1);
    assert_eq!(entries::plan(1, 0, 100, 1000, 100).0, 1);
    // The share is of the RAM given, clamped to 1..=100 percent, and never overflows.
    assert_eq!(entries::plan(2, 1, 5, 1000, 0).1, 10);
    assert_eq!(entries::plan(2, 1, 5, 1000, 250).1, 1000);
    assert_eq!(entries::plan(2, 1, 5, u64::MAX, 100).1, entries::MAX_SAVE_IN_FLIGHT);
    // A large machine's share is capped: the save runs beside a full project.
    assert_eq!(entries::plan(2, 1, 5, 32 << 30, 50).1, entries::MAX_SAVE_IN_FLIGHT);
    assert_eq!(entries::plan(2, 1, 5, 2 << 30, 50).1, 1 << 30);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn parallel_save_with_any_reserved_cores_and_ram_share_matches_serial() {
    let pf = big_sample();
    let want = payloads(&serial_bytes(&pf));
    let n = want.len();
    // (cpus, reserved cores, available RAM, percent): none reserved, all reserved, a tiny and a huge share.
    for (cpus, reserved, ram, percent) in [(8, 0, 1u64 << 40, 100u8), (8, 1, 1 << 40, 50), (8, 99, 1 << 40, 50), (16, 0, 1000, 1), (6, 1, u64::MAX, 100)] {
        let (workers, budget) = entries::plan(cpus, reserved, n, ram, percent);
        let got = parallel_bytes(&pf, workers, budget);
        assert!(payloads(&got) == want, "{cpus} cpus, {reserved} reserved, {ram} bytes at {percent}%");
        assert_same(&pf, &load_project_bytes(&got).unwrap());
    }
    // The setting is read by the public save; extreme values still save the same entries.
    for (reserved, percent) in [(0, 100), (1000, 1), (1, 50)] {
        set_save_resources(reserved, percent);
        assert!(payloads(&save_project_bytes(&pf).unwrap()) == want, "{reserved} reserved, {percent}%");
    }
    set_save_resources(DEFAULT_RESERVED_CORES, DEFAULT_RAM_PERCENT);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn parallel_save_writes_the_same_entries_as_serial() {
    let pf = big_sample();
    let serial = serial_bytes(&pf);
    let want = payloads(&serial);
    assert!(want.len() > 10);
    for (workers, budget) in [(0, 1), (2, u64::MAX), (4, u64::MAX), (8, 1), (3, 100_000)] {
        let got = parallel_bytes(&pf, workers, budget);
        assert!(payloads(&got) == want, "{workers} workers, budget {budget}");
        assert_same(&pf, &load_project_bytes(&got).unwrap());
    }
    // The public entry point (threads chosen by the machine) agrees too, and loads back.
    assert!(payloads(&save_project_bytes(&pf).unwrap()) == want);
    assert_same(&pf, &load_project_bytes(&serial).unwrap());
}

/// The files written before entries were compressed in parallel: each entry written in one go.
fn reference_one_shot(pf: &ProjectFile) -> Vec<u8> {
    let deflate = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).large_file(true);
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("project.json", deflate).unwrap();
    zip.write_all(&serde_json::to_vec_pretty(&pf.project).unwrap()).unwrap();
    for (ts, maps) in &pf.baked {
        for (m, data) in &maps.maps {
            zip.start_file(format!("baked/{ts}_{}.bin", m.id()), deflate).unwrap();
            zip.write_all(&header(maps.resolution, maps.height(*m), m.components(), 0)).unwrap();
            let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_le_bytes()).collect();
            zip.write_all(&bytes).unwrap();
        }
    }
    zip.finish().unwrap().into_inner()
}

/// Streaming a baked map into the compressor in chunks may place deflate block boundaries a few
/// bytes differently than one big write, so the stored bytes can differ slightly; what the entry
/// holds (name, method, CRC, size) must not, and the compressed size stays within 0.01%.
#[test]
fn baked_entries_hold_what_the_one_shot_writer_wrote() {
    let mut pf = big_sample();
    (pf.mesh, pf.high_poly, pf.cage, pf.bake_paint, pf.thumbnail, pf.resources) = (None, vec![], None, BTreeMap::new(), None, BTreeMap::new());
    pf.pixels = PixelStore::default();
    let (new, old) = (payloads(&save_project_bytes(&pf).unwrap()), payloads(&reference_one_shot(&pf)));
    assert_eq!(new.len(), old.len());
    for (n, o) in new.iter().zip(&old) {
        assert_eq!((&n.0, n.1, n.2, n.3), (&o.0, o.1, o.2, o.3));
        assert!(n.4.abs_diff(o.4) <= 64 + o.4 / 10_000, "{}: {} vs {}", n.0, n.4, o.4);
    }
}

#[test]
fn bake_model_options_survive_the_project_archive() {
    use texturecraft_core::{BackfaceMode, CageType, HeightNormalization};

    let mut project_file = sample().pf;
    let settings = &mut project_file.project.texture_sets[0].bake;
    settings.common.resolution_height = Some(texturecraft_core::BakeResolution::R256);
    settings.cage.kind = CageType::CustomFile;
    settings.maps.ao.parameters.ignore_backface = BackfaceMode::ByMeshName;
    settings.maps.ao.parameters.rays.min_distance = 0.02;
    settings.maps.ao.parameters.ground_plane.enabled = true;
    settings.maps.curvature.parameters.mesh.sampling_radius = 0.002;
    settings.maps.height.parameters.normalization = HeightNormalization::Manual;
    settings.maps.height.parameters.scaling_divisor = 4.0;
    settings.skew.edge_protection = true;
    settings.skew.edge_distance = 0.3;

    let bytes = save_project_bytes(&project_file).unwrap();
    assert_same(&project_file, &load_project_bytes(&bytes).unwrap());
}

fn edit_archive(project_file: &ProjectFile, mut edit: impl FnMut(&mut String, &mut Vec<u8>)) -> Vec<u8> {
    let bytes = save_project_bytes(project_file).unwrap();
    let mut source = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut output = ZipWriter::new(Cursor::new(Vec::new()));

    for index in 0..source.len() {
        let mut entry = source.by_index(index).unwrap();
        let mut name = entry.name().unwrap().into_owned();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        edit(&mut name, &mut data);
        output.start_file(name, SimpleFileOptions::default()).unwrap();
        output.write_all(&data).unwrap();
    }

    output.finish().unwrap().into_inner()
}

fn edit_project_json(project_file: &ProjectFile, mut edit: impl FnMut(&mut serde_json::Value)) -> Vec<u8> {
    edit_archive(project_file, |name, data| {
        if name == "project.json" {
            let mut json = serde_json::from_slice(data).unwrap();
            edit(&mut json);
            *data = serde_json::to_vec(&json).unwrap();
        }
    })
}

#[test]
fn recovery_loads_legacy_bake_settings_and_map_names() {
    let mut expected = sample().pf;
    let maps = expected.baked.get_mut(&0).unwrap();
    maps.maps.insert(BakedMap::WorldNormal, Arc::new(vec![0.5; 16 * 16 * 3].into()));
    maps.maps.insert(BakedMap::BentNormal, Arc::new(vec![0.5; 16 * 16 * 3].into()));
    expected.project.texture_sets[0].baked = maps.maps.keys().copied().collect();
    expected.project.texture_sets[1].bake.common.dilation = 12;

    let bytes = edit_archive(&expected, |name, data| {
        if name == "project.json" {
            let mut json: serde_json::Value = serde_json::from_slice(data).unwrap();
            json["texture_sets"][0]["bake"] = serde_json::json!({
                "enabled": true, "resolution": 1024, "antialiasing": 1, "dilation": 16,
                "maps": ["Normal", "WorldNormal", "Id", "AmbientOcclusion", "Curvature", "Position", "Thickness"],
                "ao": {"rays": 64, "max_distance": 0.1}, "use_low_as_high": true
            });
            json["texture_sets"][0]["baked"].as_array_mut().unwrap().extend(serde_json::json!(["ColorMap", "WorldDirection"]).as_array().unwrap().clone());
            *data = serde_json::to_vec(&json).unwrap();
        }

        *name = name
            .replace("_normal.bin", "_normal_base.bin")
            .replace("_worldNormal.bin", "_world_space_normals.bin")
            .replace("_ambientOcclusion.bin", "_ambient_occlusion.bin")
            .replace("_bentNormal.bin", "_bent_normals.bin");
    });

    let recovered = load_project_bytes(&bytes).unwrap();
    assert_eq!(recovered.warnings.len(), 1);
    assert_same(&expected, &recovered);
    let reopened = load_project_bytes(&save_project_bytes(&recovered).unwrap()).unwrap();
    assert_same(&expected, &reopened);
    assert!(reopened.warnings.is_empty());
}

#[test]
fn recovery_keeps_readable_bake_fields_and_project_content() {
    let mut expected = sample().pf;
    let settings = &mut expected.project.texture_sets[0].bake;
    settings.common.dilation = 19;
    settings.maps.height.is_enabled = true;
    settings.maps.ao.parameters.rays.rays = 96;
    settings.maps.ao.parameters.ground_plane.enabled = true;
    settings.high_poly.low_suffix = "_lo".into();
    expected.project.texture_sets[1].bake.common.dilation = 7;

    let bytes = edit_project_json(&expected, |json| {
        let bake = &mut json["texture_sets"][0]["bake"];
        bake["common"]["antialiasing"] = serde_json::json!("unknown");
        bake["common"]["resolution_height"] = serde_json::json!(false);
        bake["maps"]["normal"] = serde_json::json!("Normal");
        bake["maps"]["ao"]["parameters"]["rays"]["max_distance"] = serde_json::json!("bad");
        bake["maps"]["opacity"]["is_enabled"] = serde_json::json!(null);
        bake["cage"] = serde_json::json!([1, 2]);
    });

    assert_same(&expected, &load_project_bytes(&bytes).unwrap());
}

#[test]
fn recovery_defaults_unreadable_bake_blocks() {
    let expected = sample().pf;
    for invalid in [serde_json::Value::Null, serde_json::json!(false), serde_json::json!("old"), serde_json::json!(["Normal"])] {
        let bytes = edit_project_json(&expected, |json| json["texture_sets"][0]["bake"] = invalid.clone());
        assert_same(&expected, &load_project_bytes(&bytes).unwrap());
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn a_failing_entry_is_an_error_not_a_hang() {
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for fail_at in [0usize, 3, 9] {
        let list: Vec<Entry<'_>> = (0..10)
            .map(|i| Entry {
                name: format!("e{i}"),
                options: opts,
                weight: 10,
                body: Box::new(move |w| {
                    if i == fail_at {
                        return Err(std::io::Error::other("boom"));
                    }
                    if i == 5 && fail_at == 9 {
                        panic!("worker panic");
                    }
                    w.write_all(&[i as u8; 100])
                }),
            })
            .collect();
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        assert!(entries::parallel::write(&mut zip, list, 4, 25).is_err(), "fail at {fail_at}");
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn a_late_failing_entry_after_good_ones_is_its_own_error() {
    // Only the last entry fails and nothing panics, so the error must come from that entry.
    for (workers, budget) in [(4usize, 25u64), (4, u64::MAX), (2, 1)] {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            let list: Vec<Entry<'_>> = (0..10)
                .map(|i| Entry {
                    name: format!("e{i}"),
                    options: opts,
                    weight: 10,
                    body: Box::new(move |w| if i == 9 { Err(std::io::Error::other("late boom")) } else { w.write_all(&[i as u8; 100]) }),
                })
                .collect();
            let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
            let res = entries::parallel::write(&mut zip, list, workers, budget);
            let _ = tx.send(res.map_err(|e| matches!(&e, FormatError::Io(io) if io.to_string() == "late boom")));
        });
        let res =
            rx.recv_timeout(std::time::Duration::from_secs(60)).unwrap_or_else(|e| panic!("save did not finish ({e}): {workers} workers, budget {budget}"));
        assert_eq!(res, Err(true), "{workers} workers, budget {budget}");
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn assert_entry_streams_to_destination(method: CompressionMethod, budget: u64) {
    use std::sync::atomic::{AtomicBool, Ordering};

    let path = std::env::temp_dir().join(format!("texturecraft-entry-streaming-{}-{method:?}.zip", std::process::id()));
    let destination = std::fs::File::create(&path).unwrap();
    let observed_destination = destination.try_clone().unwrap();
    let reached_destination = AtomicBool::new(false);
    let mut random = 12345u32;
    let payload: Vec<u8> = (0..131_072)
        .map(|_| {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            random as u8
        })
        .collect();
    let compressed = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut list: Vec<Entry<'_>> = (0..2)
        .map(|index| Entry { name: format!("before{index}"), options: compressed, weight: 128, body: Box::new(|writer| writer.write_all(&[7; 128])) })
        .collect();
    list.push(Entry {
        name: "target".into(),
        options: SimpleFileOptions::default().compression_method(method),
        weight: payload.len() as u64,
        body: Box::new(|writer| {
            let before_bytes = observed_destination.metadata()?.len();
            writer.write_all(&payload)?;
            writer.flush()?;
            reached_destination.store(observed_destination.metadata()?.len() > before_bytes, Ordering::Relaxed);
            Ok(())
        }),
    });
    list.push(Entry { name: "after".into(), options: compressed, weight: 128, body: Box::new(|writer| writer.write_all(&[9; 128])) });
    let mut writer = ZipWriter::new(destination);

    let result = entries::parallel::write(&mut writer, list, 4, budget);
    drop(writer.finish().unwrap());
    drop(observed_destination);
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();

    result.unwrap();
    assert!(reached_destination.load(Ordering::Relaxed), "{method:?} was buffered instead of streamed");
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let names: Vec<_> = (0..archive.len()).map(|index| archive.by_index(index).unwrap().name().unwrap().into_owned()).collect();
    assert_eq!(names, ["before0", "before1", "target", "after"]);
    let mut actual = Vec::new();
    archive.by_name("target").unwrap().read_to_end(&mut actual).unwrap();
    assert_eq!(actual, payload);
    let mut after = Vec::new();
    archive.by_name("after").unwrap().read_to_end(&mut after).unwrap();
    assert_eq!(after, [9; 128]);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn stored_entries_reach_the_destination_during_their_body() {
    assert_entry_streams_to_destination(CompressionMethod::Stored, 1 << 20);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn over_budget_entries_reach_the_destination_during_their_body() {
    assert_entry_streams_to_destination(CompressionMethod::Deflated, 4096);
}

#[test]
fn recovery_skips_unsupported_baked_map_entries() {
    let expected = sample().pf;
    let bytes = edit_archive(&expected, |name, _| {
        if name == "thumbnail.png" {
            *name = "baked/0_color_map.bin".into();
        }
    });
    let mut expected = expected;
    expected.thumbnail = None;
    let recovered = load_project_bytes(&bytes).unwrap();
    assert_same(&expected, &recovered);
    assert_eq!(recovered.warnings.len(), 1);
}

#[test]
fn recovery_does_not_hide_invalid_project_structure_or_newer_versions() {
    let project_file = sample().pf;
    for (field, value) in [("resolution", serde_json::json!("bad")), ("stack", serde_json::json!([999])), ("channels", serde_json::json!(null))] {
        let bytes = edit_project_json(&project_file, |json| {
            json["texture_sets"][0]["bake"] = serde_json::json!("old");
            json["texture_sets"][0][field] = value.clone();
        });
        assert!(load_project_bytes(&bytes).is_err(), "{field}");
    }

    let bytes = edit_project_json(&project_file, |json| {
        json["format_version"] = serde_json::json!(FORMAT_VERSION + 1);
        json["texture_sets"][0]["bake"] = serde_json::json!("old");
    });
    assert!(matches!(load_project_bytes(&bytes), Err(FormatError::NewerVersion { .. })));
}

#[test]
fn recovery_still_rejects_corrupt_pixels() {
    let project_file = sample().pf;
    let bytes = edit_archive(&project_file, |name, data| {
        if name == "project.json" {
            let mut json: serde_json::Value = serde_json::from_slice(data).unwrap();
            json["texture_sets"][0]["bake"]["maps"] = serde_json::json!(["Normal"]);
            *data = serde_json::to_vec(&json).unwrap();
        } else if name.starts_with("pixels/") {
            data.truncate(10);
        }
    });

    let error = load_project_bytes(&bytes).unwrap_err();
    assert!(error.to_string().contains("pixels/"), "{error}");
}

#[test]
fn recovery_does_not_accept_unrelated_json_errors() {
    let bytes = edit_archive(&sample().pf, |name, data| {
        if name == "project.json" {
            data.splice(1..1, br#""name":"duplicate", "#.iter().copied());
        }
    });

    let error = load_project_bytes(&bytes).unwrap_err();
    assert!(error.to_string().contains("duplicate field"), "{error}");
}

#[test]
fn project_open_limit_counts_every_loaded_entry_kind() {
    let paint = sample().paint;
    let baked = [header(16, 16, 1, 0), vec![0; 16 * 16 * 4]].concat();
    let entries = [
        ("resources/test.bin".to_string(), vec![1; 1024]),
        ("mesh.obj".to_string(), vec![2; 1024]),
        ("highpoly/test.obj".to_string(), vec![3; 1024]),
        ("cage/test.obj".to_string(), vec![4; 1024]),
        ("thumbnail.png".to_string(), vec![5; 1024]),
        (format!("pixels/layer_{}_baseColor.bin", paint.0), tile_bin(128, 4, &[(0, 0)])),
        (format!("pixels/mask_{}.bin", paint.0), tile_bin(128, 2, &[(0, 0)])),
        ("baked/0_ambientOcclusion.bin".to_string(), baked.clone()),
        ("bake_paint/0_skew.bin".to_string(), baked),
    ];

    for (name, data) in entries {
        let bytes = zip_with(&[(&name, data)]);
        let mut archive = ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let total_bytes: u64 = (0..archive.len()).map(|index| archive.by_index(index).unwrap().size()).sum();

        let accepted = load_with_budget(Cursor::new(&bytes), LoadBudget::new(total_bytes), None).unwrap();
        let error = load_with_budget(Cursor::new(&bytes), LoadBudget::new(total_bytes - 1), None).unwrap_err();
        let raised_limit = load_with_budget(Cursor::new(&bytes), LoadBudget::new(total_bytes + 1), None).unwrap();

        assert!(matches!(error, FormatError::TooLarge(_)), "{name}: {error}");
        assert!(error.to_string().contains(&format!("{} bytes", total_bytes - 1)), "{name}: {error}");
        assert_same(&accepted, &raised_limit);
    }
}

#[test]
fn project_open_limit_is_shared_across_the_whole_archive() {
    let project = sample().pf;
    let bytes = serial_bytes(&project);
    let mut archive = ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let total_bytes: u64 = (0..archive.len()).map(|index| archive.by_index(index).unwrap().size()).sum();

    let accepted = load_with_budget(Cursor::new(&bytes), LoadBudget::new(total_bytes), None).unwrap();
    let error = load_with_budget(Cursor::new(&bytes), LoadBudget::new(total_bytes - 1), None).unwrap_err();

    assert_same(&project, &accepted);
    assert!(matches!(error, FormatError::TooLarge(_)), "{error}");
}

#[test]
fn entry_reads_stop_at_the_smaller_of_the_entry_and_project_limits() {
    for (entry_limit, project_limit, expected_read) in [(8, 100, 9), (100, 8, 9)] {
        let mut reader = Cursor::new(vec![1; 1000]);
        let mut budget = LoadBudget::new(project_limit);

        let error = read_all(&mut reader, entry_limit, &mut budget, "resource").unwrap_err();

        assert!(matches!(error, FormatError::TooLarge(_)), "{error}");
        assert_eq!(reader.position(), expected_read);
    }
}

#[test]
fn oversized_baked_data_is_rejected_before_reading_its_payload() {
    let mut reader = Cursor::new(header(16, 16, 1, 0));
    let mut budget = LoadBudget::new(BIN_HEADER_BYTES as u64 + 16 * 16 * 4 - 1);

    let error = read_baked(&mut reader, BIN_HEADER_BYTES as u64 + 16 * 16 * 4, 1, &mut budget, "baked map").unwrap_err();

    assert!(matches!(error, FormatError::TooLarge(_)), "the size guard must run before the truncated read: {error}");
    assert_eq!(reader.position(), BIN_HEADER_BYTES as u64);
}

#[test]
fn archive_preflight_counts_ignored_entries_and_allows_the_exact_limit() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for name in ["project.json", "unknown/future-data"] {
        writer.start_file(name, options).unwrap();
        writer.write_all(&[0; 16]).unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();

    check_archive(&mut archive, 32, false).unwrap();
    let error = check_archive(&mut archive, 31, false).unwrap_err();

    assert!(matches!(error, FormatError::TooLarge(_)), "{error}");
}

#[test]
fn overlapping_archive_entries_are_rejected_before_decompression() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file("resource", SimpleFileOptions::default()).unwrap();
    writer.write_all(&[0; 16]).unwrap();
    writer.shallow_copy_file("resource", "copy").unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();

    let error = check_archive(&mut archive, 32, false).unwrap_err();

    assert!(error.to_string().contains("overlapping ZIP entries"), "{error}");
}

#[test]
fn actual_decompression_is_bounded_even_when_the_declared_size_is_small() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file("resource", SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)).unwrap();
    writer.write_all(&vec![0; 1 << 20]).unwrap();
    let mut bytes = writer.finish().unwrap().into_inner();
    let directory = ZipArchive::new(Cursor::new(&bytes)).unwrap().central_directory_start() as usize;
    // The central header stores the uncompressed size at byte 24
    bytes.get_mut(directory + 24..directory + 28).unwrap().copy_from_slice(&1u32.to_le_bytes());
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    check_archive(&mut archive, 1024, false).unwrap();
    let mut entry = archive.by_index(0).unwrap();
    let mut budget = LoadBudget::new(1024);

    let error = read_all(&mut entry, MAX_RESOURCE, &mut budget, "resource").unwrap_err();

    assert!(
        matches!(error, FormatError::TooLarge(_)) || error.to_string().contains("larger than its declared uncompressed size"),
        "actual bytes must be bounded independently of ZIP sizes: {error}"
    );
}

#[test]
fn baked_dimensions_must_match_the_entry_before_reserving_payload_bytes() {
    let mut reader = Cursor::new(header(1024, 1024, 3, 0));
    let mut budget = LoadBudget::new(16 << 20);

    let error = read_baked(&mut reader, BIN_HEADER_BYTES as u64, 3, &mut budget, "baked map").unwrap_err();

    assert!(error.to_string().contains("24 bytes for a 1024×1024×3 map"), "{error}");
    assert_eq!(budget.remaining(), (16 << 20) - BIN_HEADER_BYTES as u64);
    assert_eq!(reader.position(), BIN_HEADER_BYTES as u64);
}

#[test]
fn streamed_float_chunks_preserve_bits_across_the_chunk_boundary() {
    let values: Vec<u32> = (0..BAKED_CHUNK + 3).map(|index| (index as u32).wrapping_mul(0x9e3779b9)).collect();
    let mut bytes = header(values.len() as u32, 1, 1, 0);
    bytes.extend(values.iter().flat_map(|bits| bits.to_le_bytes()));
    let declared = bytes.len() as u64;
    let mut reader = Cursor::new(bytes);
    let mut budget = LoadBudget::new(declared);

    let (_, _, decoded) = read_baked(&mut reader, declared, 1, &mut budget, "baked map").unwrap();

    assert_eq!(decoded.into_iter().map(f32::to_bits).collect::<Vec<_>>(), values);
    assert_eq!(reader.position(), declared);
    assert_eq!(budget.remaining(), 0);
}

#[test]
fn archive_indexing_does_not_scan_large_payloads_and_rejects_large_metadata() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file("resource", SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
    writer.write_all(&vec![7; 1 << 20]).unwrap();
    let bytes = writer.finish().unwrap().into_inner();

    let mut archive = open_archive(Cursor::new(bytes), 4096).unwrap();
    let mut first_byte = [0; 1];
    archive.by_index(0).unwrap().read_exact(&mut first_byte).unwrap();

    assert_eq!(first_byte, [7]);

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file("a".repeat(5000), SimpleFileOptions::default()).unwrap();
    writer.write_all(&[7]).unwrap();
    let bytes = writer.finish().unwrap().into_inner();

    assert!(open_archive(Cursor::new(bytes), 4096).is_err());
}

#[test]
fn zip64_entry_counts_remain_supported_and_bounded() {
    for count in [65_536, MAX_ENTRIES + 1] {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for index in 0..count {
            writer.start_file(format!("e{index}"), SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
        }
        let bytes = writer.finish().unwrap().into_inner();

        let mut archive = open_archive(Cursor::new(bytes), MAX_ZIP_METADATA_BYTES).unwrap();
        let result = check_archive(&mut archive, 0, false);

        assert_eq!(archive.len(), count);
        if count <= MAX_ENTRIES {
            result.unwrap();
        } else {
            assert!(matches!(result, Err(FormatError::TooLarge(_))), "{result:?}");
        }
    }
}

/// A project.json written by format version 2 (before locks, placeholders, the import report, UV-tile geometry masks
/// and the imported blend modes), for `sample()` plus a layer with a part mask, a Tint channel and a levels effect.
const VERSION_2_PROJECT: &str = include_str!("fixtures/version2.json");

#[test]
fn version_2_files_still_load_and_resave_as_version_3() {
    let Sample { pf: sample, paint } = sample();
    let bytes = edit_archive(&sample, |name, data| {
        if name == "project.json" {
            *data = VERSION_2_PROJECT.as_bytes().to_vec();
        }
    });
    let loaded = load_project_bytes(&bytes).unwrap();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    assert_eq!(loaded.project.format_version, FORMAT_VERSION);
    assert_eq!(FORMAT_VERSION, 3);
    let parts = loaded.project.layers.values().find(|l| l.name == "Parts only").unwrap();
    assert_eq!(parts.geometry_mask, Some(GeometryMask::Parts(vec!["Body".into(), "Lid".into()])));
    assert_eq!(parts.channels[&Channel::BaseColor].mode, BlendMode::Tint);
    assert!(loaded.project.layers.values().all(|l| l.lock.is_none() && l.placeholders().next().is_none()));
    assert_eq!(loaded.project.import, None);
    assert_eq!(loaded.pixels.layer(paint, Channel::BaseColor).map(|i| i.get(127, 127)), Some([65535, 0, 9, 65535]));
    // Saved again as version 3, a part mask keeps its version 2 form and nothing else is added.
    let resaved = save_project_bytes(&loaded).unwrap();
    let json = project_json(&resaved);
    assert_eq!(json["format_version"], 3);
    let stored = json["layers"].as_object().unwrap().values().find(|l| l["name"] == "Parts only").unwrap();
    assert_eq!(stored["geometry_mask"], serde_json::json!(["Body", "Lid"]));
    assert!(json.get("import").is_none() && json["layers"].as_object().unwrap().values().all(|l| l.get("lock").is_none()));
    assert_same(&loaded, &load_project_bytes(&resaved).unwrap());
}

fn project_json(bytes: &[u8]) -> serde_json::Value {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut json = String::new();
    zip.by_name("project.json").unwrap().read_to_string(&mut json).unwrap();
    serde_json::from_str(&json).unwrap()
}

#[test]
fn version_3_features_round_trip_identically() {
    use texturecraft_core::import_report::original_result_layer;
    use texturecraft_core::{Fidelity, ImportReport, LayerReport, LayerRole, Lock, LockReason, SafetyFlag, SafetyFlagKind};

    let mut s = sample();
    let p = &mut s.pf.project;
    let channels = p.texture_sets[0].channel_list();
    // A locked layer with placeholders in its mask and content, every imported blend mode and a UV-tile mask.
    let mut locked = Layer::new(p.alloc_id(), "Imported", LayerKind::Paint, &channels);
    locked.content_effects.push(Effect::placeholder(p.alloc_id(), "Filter \"warp\"", "compiled graphs are not computed yet"));
    locked.mask = Some(Mask { effects: vec![Effect::placeholder(p.alloc_id(), "Generator \"dirt\"", "not computed")], ..Mask::new(MaskBackground::White) });
    locked.lock = Some(Lock::import(vec![LockReason::new("graph:warp", "content filter \"warp\""), LockReason::new("", "brush strokes")]));
    locked.geometry_mask = Some(GeometryMask::UvTiles(vec![1001, 1002]));
    for (c, m) in channels.iter().zip(BlendMode::IMPORTED) {
        if let Some(b) = locked.channels.get_mut(c) {
            b.mode = m;
        }
    }
    let locked = p.insert_layer(0, locked, None, false).unwrap();
    let original = p.alloc_id();
    p.insert_layer(0, original_result_layer(original, &channels, false), None, false).unwrap();
    s.pf.pixels.layer_mut(original, Channel::BaseColor, (128, 128)).set(5, 6, &[1, 2, 3, 65535]);
    let mut report = ImportReport::new("robot.ext (saved with version 1.2)", "texturecraft-import 0.1.0");
    report.add("texture set \"A\" / layer stack", Fidelity::Approximated, "2 layers imported");
    report.add("texture set \"A\" / brush strokes", Fidelity::Baked, "");
    report.layers.insert(original, LayerReport { role: LayerRole::OriginalResult, fidelity: Fidelity::Baked, ..LayerReport::default() });
    let flags = vec![
        SafetyFlag::new(SafetyFlagKind::UnhandledSetting, "Class.field", "setting Class.field = 2 not handled"),
        SafetyFlag::new(SafetyFlagKind::MaskNotComputable, "", "mask generator not computable"),
    ];
    report.layers.insert(locked, LayerReport { role: LayerRole::Imported, fidelity: Fidelity::Approximated, path: "Folder / Imported".into(), flags });
    p.import = Some(report);

    let bytes = save_project_bytes(&s.pf).unwrap();
    let back = load_project_bytes(&bytes).unwrap();
    assert!(back.warnings.is_empty());
    assert_same(&s.pf, &back);
    assert_eq!(save_project_bytes(&back).unwrap(), bytes, "saving is deterministic");
    let json = project_json(&bytes);
    assert_eq!(json["format_version"], 3);
    let stored = &json["layers"][locked.0.to_string()];
    assert_eq!(stored["geometry_mask"], serde_json::json!({"UvTiles": [1001, 1002]}));
    assert_eq!(stored["lock"]["source"], "Project import");
    assert_eq!(json["import"]["layers"][original.0.to_string()]["role"], "OriginalResult");
    // A stored fidelity this version does not know is an error, not a silent default.
    let mut bad = json.clone();
    bad["import"]["entries"][0]["fidelity"] = serde_json::json!("KeptRaw");
    let bytes = edit_project_json(&s.pf, |j| *j = bad.clone());
    assert!(load_project_bytes(&bytes).is_err());
}

const GREY: &str = "format 3\nsize 8\nnode 1 color r=0.5 g=0.5 b=0.5\noutput grey 1\nexpose gain 1 r 0 1\n";
const UNUSED: &str = "format 3\nsize 8\nnode 1 color r=1 g=0 b=0\noutput red 1\n";

/// A sample with a graph source, a nested graph in a mask generator, and one cook the source's use read.
fn graph_sample() -> (ProjectFile, String) {
    use texturecraft_core::{GraphInput, GraphRef, Projection, Source};
    let mut pf = sample().pf;
    let grey = pf.graphs.add_text(GREY);
    pf.graphs.add_text(UNUSED);
    let mut g = GraphRef::new(grey.clone(), "grey");
    g.output = Some("grey".into());
    let mut fill = Fill::default();
    fill.sources.insert(Channel::Roughness, Source::Graph(Box::new(g.clone())));
    let id = pf.project.alloc_id();
    let mut layer = Layer::new(id, "Graph fill", LayerKind::Fill(fill), &[Channel::Roughness]);
    let mut outer = GraphRef::new(texturecraft_core::text_key("absent outer"), "outer");
    outer.inputs.insert(0, GraphInput::Nested(Box::new(g)));
    layer.mask = Some(Mask {
        effects: vec![Effect::new(pf.project.alloc_id(), EffectKind::Graph { graph: outer, projection: Projection::default() })],
        ..Mask::new(MaskBackground::White)
    });
    pf.project.insert_layer(0, layer, None, false).unwrap();
    let t = texturecraft_compose::graphs::Tex { w: 70, h: 70, px: (0..4900).map(|i| [(i % 70) as f32 / 70.0, 0.25, 0.5, 1.0]).collect() };
    let key = texturecraft_core::StableHash::of(b"a cook").key();
    pf.graphs.insert_cooked(
        key.clone(),
        Cooked::from_textures([("grey".to_string(), t.clone()), ("odd name/x".to_string(), t)].into(), ["grey".to_string()].into()),
    );
    pf.graphs.insert_cooked(texturecraft_core::StableHash::of(b"unused cook").key(), Cooked::from_textures(BTreeMap::new(), Default::default()));
    pf.graphs.set_uses([(GraphUse { set: 0, id, channel: Some(Channel::Roughness) }, key.clone())].into());
    (pf, key)
}

#[test]
fn saving_a_parameter_edit_does_not_reattach_the_old_graph_pixels() {
    use texturecraft_compose::ImageStore;
    use texturecraft_compose::graphs::RenderEnv;
    use texturecraft_core::{GraphValue, Source};

    let (original, key) = graph_sample();
    let loaded = load_project_bytes(&save_project_bytes(&original).unwrap()).unwrap();
    let mut edited = loaded.clone();
    let use_ = loaded.graphs.uses().into_keys().next().unwrap();
    let LayerKind::Fill(fill) = &mut edited.project.layers.get_mut(&use_.id).unwrap().kind else { panic!("graph fill") };
    let Source::Graph(graph) = fill.sources.get_mut(&Channel::Roughness).unwrap() else { panic!("graph source") };
    graph.params.insert("gain".into(), GraphValue::Float(0.75));
    let updated = graph.as_ref().clone();

    let reopened = load_project_bytes(&save_project_bytes(&edited).unwrap()).unwrap();
    assert_eq!(reopened.graphs.use_cook_key(&use_), None, "the older result is not current");
    assert!(reopened.graphs.cooked(&key).is_none(), "a cook held only by an older state is not written");
    assert_eq!(loaded.graphs.use_cook_key(&use_), Some(key.clone()), "the existing use is still available to Undo");
    assert_eq!(save_project_bytes(&loaded).unwrap(), save_project_bytes(&original).unwrap());

    let env = RenderEnv { baked: &BakedMaps::default(), images: &ImageStore::default(), side: 8 };
    edited.graphs.cook_for(use_.clone(), &updated, &env, &[]).unwrap();
    let fresh = edited.graphs.use_cook_key(&use_).unwrap();
    assert_ne!(fresh, key);
    let reopened = load_project_bytes(&save_project_bytes(&edited).unwrap()).unwrap();
    assert_eq!(reopened.graphs.use_cook_key(&use_), Some(fresh));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_project_opened_from_its_file_reads_cooks_when_they_are_needed() {
    let (pf, key) = graph_sample();
    let dir = std::env::temp_dir().join(format!("tc-formats-lazy-cooks-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lazy.tcraft");
    save_project(&path, &pf).unwrap();
    let back = load_project(&path).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    assert_eq!(back.graphs.held_bytes(), 0, "no cook is held after opening");
    assert_eq!(back.graphs.used_cook_keys(), vec![key.clone()]);
    assert_eq!(*back.graphs.cooked(&key).unwrap(), *pf.graphs.cooked(&key).unwrap(), "read from the file as it was saved");
    // saved again (over itself and elsewhere) it holds the same cooks, byte for byte
    let bytes = save_project_bytes(&pf).unwrap();
    assert_eq!(save_project_bytes(&back).unwrap(), bytes);
    save_project(&path, &back).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(*back.graphs.cooked(&key).unwrap(), *pf.graphs.cooked(&key).unwrap(), "still read after the save over the file");
    // A lazy snapshot can still read its original cook after the path is removed.
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(*back.graphs.cooked(&key).unwrap(), *pf.graphs.cooked(&key).unwrap());
    assert_eq!(back.graphs.held_bytes(), 0, "reading from the retained file does not pin the cook");
}

#[test]
fn graph_texts_and_cooks_round_trip_byte_for_byte() {
    let (pf, key) = graph_sample();
    let bytes = save_project_bytes(&pf).unwrap();
    let back = load_project_bytes(&bytes).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    assert_same(&pf, &back);
    // only the referenced text that the project holds is saved (the outer one is missing: it is just not there)
    assert_eq!(back.graphs.texts.keys().collect::<Vec<_>>(), [&texturecraft_core::text_key(GREY)]);
    assert_eq!(back.graphs.uses(), pf.graphs.uses());
    let cooked = back.graphs.cooked(&key).unwrap();
    assert_eq!(*cooked, *pf.graphs.cooked(&key).unwrap(), "cooks load as they were saved, linear outputs too");
    assert_eq!(back.graphs.used_cooks().len(), 1, "cooks no use reads are not saved");
    assert_eq!(save_project_bytes(&back).unwrap(), bytes, "re-saving gives the same bytes");
    // version 3 files without graphs load as before
    let plain = sample().pf;
    let back = load_project_bytes(&save_project_bytes(&plain).unwrap()).unwrap();
    assert!(back.graphs.texts.is_empty() && back.graphs.uses().is_empty());
}

#[test]
fn hostile_graph_entries_are_refused_or_left_out() {
    let (pf, key) = graph_sample();
    let grey = texturecraft_core::text_key(GREY);
    // a text that does not hash to its name
    let bytes = edit_archive(&pf, |name, data| {
        if name.starts_with("graphs/") {
            data.extend_from_slice(b"node 9 color\n");
        }
    });
    assert!(load_project_bytes(&bytes).unwrap_err().to_string().contains("does not match its key"));
    // a graph entry that is not named by a key
    let bytes = edit_archive(&pf, |name, _| {
        if name.starts_with("graphs/") {
            *name = "graphs/../../x.txt".into();
        }
    });
    assert!(load_project_bytes(&bytes).is_err());
    // a reference whose key is not a key
    let bytes = edit_project_json(&pf, |json| {
        let text = json.to_string().replace(&grey, "../etc");
        *json = serde_json::from_str(&text).unwrap();
    });
    assert!(load_project_bytes(&bytes).is_err());
    // an unreadable index: no cooks, a note, the project still opens
    let bytes = edit_archive(&pf, |name, data| {
        if name == "cooked/index.json" {
            *data = b"{not json".to_vec();
        }
    });
    let back = load_project_bytes(&bytes).unwrap();
    assert!(back.graphs.cooked(&key).is_none() && back.warnings.len() == 1);
    // a cook whose output entry is broken is left out whole
    let bytes = edit_archive(&pf, |name, data| {
        if name.starts_with("cooked/") && name.ends_with(".bin") {
            data.truncate(30);
        }
    });
    let back = load_project_bytes(&bytes).unwrap();
    assert!(back.graphs.cooked(&key).is_none() && back.warnings.iter().any(|w| w.contains("rendered again")));
    assert_eq!(back.graphs.texts.len(), 1);
}

/// A record with every kind of part, keyed by nothing yet.
fn sample_record(seed: u32) -> texturecraft_core::StrokeRecord {
    use texturecraft_core::strokes::*;
    let tip = StrokeTip::Image { width: 2, height: 1, values: Arc::from(vec![0.25, f32::NAN]) };
    let brush = StrokeBrush {
        radius: 0.05,
        spacing: 0.1,
        flow: 0.8,
        size_pressure: Some(0.2),
        flow_pressure: None,
        size_jitter: 0.1,
        flow_jitter: 0.0,
        position_jitter: 0.3,
        position_both: true,
        position_normal: false,
        rotation: 1.5,
        rotation_jitter: 0.25,
        follow: true,
        alignment: StrokeTipAlignment::Camera,
        culling: Some(0.5),
        tip: StrokeTip::Round { hardness: 0.9 },
        cycle: vec![tip.clone(), tip],
        cycle_drawn_start: true,
        up: Some([0.0, 1.0, 0.0]),
    };
    let graph = texturecraft_core::GraphRef { output: Some("basecolor".into()), seed: 3, ..texturecraft_core::GraphRef::new("k".repeat(16), "grunge") };
    let stroke = RecordedStroke {
        space: StrokeSpace::Mesh { view: [1.0; 16], projection: [-0.0; 16] },
        points: vec![StrokePoint { position: [0.1, -0.2], pressure: 0.5 }, StrokePoint { position: [0.3, 0.4], pressure: 1.0 }],
        seed,
        lazy: 0.02,
        copies: 1,
        copy_matrix: [2.0; 16],
    };
    let path = PathStroke {
        pieces: vec![PathPiece { points: [[0.0; 3], [1.0; 3], [2.0; 3], [3.0; 3]], normals: [[0.0, 0.0, 1.0]; 2], size: [0.5, 1.0] }],
        anchor: None,
        seed: seed + 1,
        copies: 0,
        copy_matrix: [0.0; 16],
    };
    let step = StrokeStep {
        mode: StrokeMode::Paint,
        values: vec![(Channel::BaseColor, [1.0, 0.5, 0.0], 1.0)],
        sources: vec![(Channel::Roughness, StrokeSource::Graph(Box::new(graph)), 0.5), (Channel::Metallic, StrokeSource::NotKept("an image".into()), 1.0)],
        mask_value: 0.0,
        mask_source: None,
        opacity: 0.75,
        brush,
        stencil: Some(StrokeStencil { image: StencilValues::Image { width: 1, height: 2, values: Arc::from(vec![0.0, 1.0]) }, matrix: [0.5; 9] }),
        projection: Some(StrokeStencil { image: StencilValues::Uniform(0.3), matrix: [1.0; 9] }),
        placed: Some([0.25; 9]),
        strokes: vec![stroke.clone(), RecordedStroke { space: StrokeSpace::Uv, ..stroke }],
        paths: vec![path],
        ribbon: Some(StrokeRibbon { width: 0.1, copy: 0.4, tiling: RibbonTiling::Caps { guides: [0.2, 0.8] }, depth: 0.05 }),
        particles: Some(StrokeParticles {
            per_point: 2.0,
            per_length: 0.5,
            budget: 64.0,
            speed: 0.3,
            speed_rand: 0.2,
            life: 0.5,
            life_rand: 0.1,
            gravity: 1.0,
            friction: 0.5,
            wander: 0.1,
            curl: 0.2,
            fan: 1.0,
            heading: 0.3,
            away: true,
            kink_time: 0.1,
            kink_angle: 0.4,
            kink_branches: 2.0,
            kink_speed: 1.0,
            stop_speed: 0.01,
            view_bias: 0.2,
            spread: 1.0,
            scatter: 0.1,
            fade_size: 0.2,
            fade_size_rand: 0.1,
            fade_flow: 0.3,
            fade_flow_rand: 0.1,
            size: 0.8,
            flow: 0.9,
            step: 0.01,
            file_units: false,
        }),
    };
    let smudge = StrokeStep { mode: StrokeMode::Smudge, sources: vec![], ..step.clone() };
    StrokeRecord { target: StrokeTarget::Content, label: "Layer 1 / brush strokes".into(), steps: vec![step, smudge] }
}

fn mask_effect(pf: &ProjectFile) -> LayerId {
    pf.project.layers.values().find_map(|l| l.mask.as_ref()?.effects.first().map(|e| e.id)).unwrap()
}

fn stroke_entries(bytes: &[u8]) -> Vec<String> {
    payloads(bytes).into_iter().map(|p| p.0).filter(|n| n.starts_with("strokes/")).collect()
}

fn stroke_image_entries(bytes: &[u8]) -> Vec<String> {
    payloads(bytes).into_iter().map(|p| p.0).filter(|n| n.starts_with(strokes::IMAGE_DIR)).collect()
}

fn encode(record: &texturecraft_core::StrokeRecord) -> Vec<u8> {
    strokes::encode(record, &mut strokes::ImageTable::default()).unwrap()
}

#[test]
fn stroke_records_round_trip_bit_for_bit() {
    let mut s = sample();
    let native_strokes = br#"{"version":2,"targets":[]}"#;
    s.pf.stroke_replay = Some(StrokeReplayData::loaded(native_strokes.to_vec()));
    let mask = mask_effect(&s.pf);
    let record = sample_record(7);
    s.pf.strokes.insert(s.paint, record.clone().into());
    let mut on_mask = sample_record(9);
    on_mask.target = texturecraft_core::strokes::StrokeTarget::Mask;
    s.pf.strokes.insert(mask, on_mask.into());
    let bytes = save_project_bytes(&s.pf).unwrap();
    assert_eq!(stroke_entries(&bytes), [format!("strokes/{}.bin", s.paint.0), format!("strokes/{}.bin", mask.0)]);
    // the tip (laid by four stamps of two records) and the stencil are kept once each
    assert_eq!(stroke_image_entries(&bytes).len(), 2);
    let back = load_project_bytes(&bytes).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    assert_same(&s.pf, &back);
    assert_eq!(back.stroke_replay.as_ref().unwrap().bytes().unwrap().as_ref(), native_strokes);
    // floats keep their bits (NaN, -0.0), so the encodings agree too
    let read = back.strokes.get(&s.paint).and_then(StoredStrokes::record).unwrap();
    assert_eq!(encode(read), encode(&record));
    assert_eq!(back.strokes.len(), 2);
    // read back, the records share one copy of each image
    let other = back.strokes.get(&mask).and_then(StoredStrokes::record).unwrap();
    assert!(Arc::ptr_eq(read.images()[0], other.images()[0]));
    assert_eq!(save_project_bytes(&back).unwrap(), bytes, "saving again writes the same file");
}

#[test]
fn projects_without_stroke_records_are_written_as_before() {
    let s = sample();
    let bytes = save_project_bytes(&s.pf).unwrap();
    assert!(stroke_entries(&bytes).is_empty());
    let names: Vec<String> = payloads(&bytes).into_iter().map(|p| p.0).collect();
    let mut with = s.pf.clone();
    with.strokes.insert(s.paint, sample_record(1).into());
    let more: Vec<String> = payloads(&save_project_bytes(&with).unwrap())
        .into_iter()
        .map(|p| p.0)
        .filter(|n| !n.starts_with("strokes/") && !n.starts_with(strokes::IMAGE_DIR))
        .collect();
    // a record adds its entry (and the text of a graph it paints when the store holds it), nothing else changes
    assert_eq!(names, more);
    assert!(load_project_bytes(&bytes).unwrap().strokes.is_empty());
}

#[test]
fn records_of_removed_pixels_are_neither_saved_nor_loaded() {
    let mut s = sample();
    s.pf.strokes.insert(LayerId(9999), sample_record(1).into());
    assert!(stroke_entries(&save_project_bytes(&s.pf).unwrap()).is_empty());
    let record = strokes::encode_inline(&sample_record(1)).unwrap();
    let back = load_project_bytes(&zip_with(&[("strokes/9999.bin", record.clone()), (&format!("strokes/{}.bin", s.paint.0), record)])).unwrap();
    assert_eq!(back.strokes.keys().copied().collect::<Vec<_>>(), [s.paint]);
    assert!(matches!(load_project_bytes(&zip_with(&[("strokes/x.bin", vec![])])), Err(FormatError::Malformed { .. })));
}

#[test]
fn unreadable_stroke_records_are_kept_as_their_bytes() {
    let s = sample();
    let good = strokes::encode_inline(&sample_record(1)).unwrap();
    let mut newer = good.clone();
    newer[4] = 3;
    let mut trailing = good.clone();
    trailing.push(0);
    for (bytes, why) in [
        (newer, "version 3"),
        (encode(&sample_record(1)), "missing"),
        (trailing, "after the stroke record"),
        (b"TCSR".to_vec(), "cut short"),
        (b"PNG!....".to_vec(), "not a stroke"),
    ] {
        let back = load_project_bytes(&zip_with(&[(&format!("strokes/{}.bin", s.paint.0), bytes.clone())])).unwrap();
        let Some(StoredStrokes::Unreadable(u)) = back.strokes.get(&s.paint) else { panic!("unreadable: {:?}", back.strokes) };
        assert!(u.reason.contains(why), "{why}: {}", u.reason);
        assert_eq!(*u.bytes, bytes);
        assert!(back.warnings.iter().any(|w| w.contains("stroke record")), "{:?}", back.warnings);
        // saved again unchanged
        let again = load_project_bytes(&save_project_bytes(&back).unwrap()).unwrap();
        assert_eq!(again.strokes.get(&s.paint), back.strokes.get(&s.paint));
    }
}

#[test]
fn damaged_stroke_records_never_panic() {
    let mut table = strokes::ImageTable::default();
    let good = strokes::encode(&sample_record(1), &mut table).unwrap();
    let images: std::collections::HashMap<String, Arc<[f32]>> = table.by_key.into_iter().collect();
    let decode = |b: &[u8]| strokes::decode(b, &mut images.clone());
    // (NaN tips: compared by their encodings)
    assert_eq!(encode(&decode(&good).unwrap()), good);
    for n in 0..good.len() {
        assert!(decode(&good[..n]).is_err(), "cut at {n}");
    }
    let mut state = 0x9e37_79b9u32;
    for i in 0..4000 {
        let mut b = good.clone();
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let at = 8 + state as usize % (b.len() - 8);
        b[at] ^= (i % 255 + 1) as u8;
        let _ = decode(&b);
    }
    // a length that claims far more than the entry holds
    let mut huge = good[..8].to_vec();
    huge.extend_from_slice(&[0, 0, 0xff, 0xff, 0xff, 0xff, 0x0f]);
    assert!(decode(&huge).is_err());
    // an image entry that is cut, damaged or not of its key is not read
    let (key, values) = images.iter().next().unwrap();
    let bytes = strokes::encode_image(values);
    let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&strokes::decode_image(key, &bytes).unwrap()), bits(values), "(NaN tips: compared by their bits)");
    assert!(strokes::decode_image(key, &bytes[..bytes.len() - 1]).is_err());
    assert!(strokes::decode_image(&"0".repeat(32), &bytes).is_err());
    assert!(strokes::decode_image(key, b"TCSI").is_err());
}

#[test]
fn stroke_records_of_the_older_layout_are_still_read() {
    let s = sample();
    let record = sample_record(1);
    let old = strokes::encode_inline(&record).unwrap();
    let back = load_project_bytes(&zip_with(&[(&format!("strokes/{}.bin", s.paint.0), old)])).unwrap();
    let read = back.strokes.get(&s.paint).and_then(StoredStrokes::record).unwrap();
    assert_eq!(encode(read), encode(&record));
    // its copies of one image are shared once read
    let images = read.images();
    assert!(Arc::ptr_eq(images[0], images[1]));
    // and saved in the newer layout, the images apart
    let mut pf = s.pf.clone();
    pf.strokes = back.strokes.clone();
    assert_eq!(stroke_image_entries(&save_project_bytes(&pf).unwrap()).len(), 2);
}

#[test]
fn graphs_painted_by_stroke_records_keep_their_texts() {
    let mut s = sample();
    let text = "graph text the brush paints".to_string();
    let key = texturecraft_core::text_key(&text);
    let mut record = sample_record(1);
    if let texturecraft_core::strokes::StrokeSource::Graph(g) = &mut record.steps[0].sources[0].1 {
        g.key = key.clone();
    }
    s.pf.strokes.insert(s.paint, record.into());
    s.pf.graphs.texts.insert(key.clone(), Arc::from(text.as_str()));
    let back = load_project_bytes(&save_project_bytes(&s.pf).unwrap()).unwrap();
    assert_eq!(back.graphs.texts.get(&key).map(|t| t.to_string()), Some(text));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_project_opened_from_its_file_reads_pixels_and_baked_maps_after_its_archive_is_replaced() {
    let Sample { pf, .. } = sample();
    let dir = std::env::temp_dir().join(format!("tc-formats-lazy-parts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (path, moved) = (dir.join("lazy.tcraft"), dir.join("moved.tcraft"));
    save_project(&path, &pf).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut back = load_project(&path).unwrap();
    assert!(!back.pixels.layers.is_empty() && !back.baked.is_empty());
    assert!(back.pixels.layers.values().chain(back.pixels.masks.values()).all(|img| !img.is_loaded()), "no pixels read at open");
    assert!(back.baked.values().flat_map(|b| b.maps.values()).all(|m| m.bytes() == 0), "no baked map read at open");
    // saved without reading them into memory, the same file
    save_project(&path, &back).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(back.pixels.layers.values().all(|img| !img.is_loaded()));
    assert_same(&pf, &back);
    for img in back.pixels.layers.values_mut().chain(back.pixels.masks.values_mut()) {
        img.release();
    }
    // Simulate saving a document after removing a layer still referenced by undo history.
    let mut edited = back.clone();
    edited.pixels.layers.clear();
    save_project(&path, &edited).unwrap();
    assert!(load_project(&path).unwrap().pixels.layers.is_empty());
    save_project(&moved, &edited).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_same(&pf, &back);
    assert_eq!(save_project_bytes(&back).unwrap(), bytes);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_lazy_open_charges_lazily_read_entries_their_header_only() {
    let project = sample().pf;
    let dir = std::env::temp_dir().join(format!("tc-formats-lazy-budget-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("budget.tcraft");
    save_project(&path, &project).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut archive = ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let sizes: Vec<(String, u64)> = (0..archive.len())
        .map(|index| {
            let entry = archive.by_index(index).unwrap();
            (entry.name().unwrap().into_owned(), entry.size())
        })
        .collect();
    let total: u64 = sizes.iter().map(|(_, size)| size).sum();
    let held: u64 = sizes.iter().map(|(name, size)| if read_lazily(name) { (*size).min(BIN_HEADER_BYTES as u64) } else { *size }).sum();
    assert!(held < total, "the sample holds layer pixels read lazily");

    // the eager open needs the whole file; the lazy one only what it holds
    let eager = load_with_budget(Cursor::new(&bytes), LoadBudget::new(held), None).unwrap_err();
    assert!(matches!(eager, FormatError::TooLarge(_)), "{eager}");
    let origin = Origin::new(&path).unwrap();
    let lazy = load_with_budget(origin.reader(), LoadBudget::new(held), Some(&origin)).unwrap();
    assert_same(&project, &lazy);
    let short = load_with_budget(origin.reader(), LoadBudget::new(held - 1), Some(&origin)).unwrap_err();
    assert!(matches!(short, FormatError::TooLarge(_)), "{short}");
    std::fs::remove_dir_all(&dir).unwrap();
}
