//! Compositing benchmarks at 4K and 8K (run with `--ignored --nocapture` in release).

use std::time::{Duration, Instant};

use rayon::prelude::*;
use serde_json::{Value, json};
use texturecraft_engine::compose::{DisplayKind, kinds_for, tile_bytes};
use texturecraft_engine::core::Channel;
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::texel::TILE;
use texturecraft_engine::{Session, execute};

// Exercise procedural fills, generators, a blurred mask, a folder and painted layers
fn stack(resolution: u32) -> Session {
    let mut session = Session::new_project("perf", primitives::sphere(1.0, 64, 32), "builtin:sphere", None, resolution).unwrap();
    session.viewport = Vec2::new(800.0, 800.0);

    let run = |session: &mut Session, id: &str, params: Value| execute(session, id, &params).unwrap();
    run(&mut session, "layer.add.fill", json!({"color": [0.6, 0.5, 0.4], "roughness": 0.6}));
    run(&mut session, "layer.fill.set", json!({"channel": "baseColor", "procedural": "clouds", "low": [0.2, 0.2, 0.2], "high": [0.8, 0.7, 0.6]}));
    run(&mut session, "layer.fill.set", json!({"channel": "height", "procedural": "cells", "low": [-0.2, -0.2, -0.2], "high": [0.2, 0.2, 0.2]}));
    run(&mut session, "layer.add.fill", json!({"color": [0.1, 0.3, 0.6]}));
    run(&mut session, "layer.fill.set", json!({"channel": "roughness", "procedural": "grunge"}));
    run(&mut session, "mask.add.black", Value::Null);
    run(&mut session, "effect.add.generator", json!({"generator": "position", "level": 0.5}));
    run(&mut session, "effect.add.filter", json!({"filter": "blur", "radius": 8}));
    run(&mut session, "layer.add.fill", json!({"roughness": 0.3}));
    run(&mut session, "layer.fill.set", json!({"channel": "baseColor", "procedural": "wood"}));
    run(&mut session, "layer.fill.projection", json!({"projection": "triplanar", "scale": 2}));
    run(&mut session, "mask.add.black", Value::Null);
    run(&mut session, "effect.add.fill", json!({"procedural": "scratches", "scale": 6}));
    run(&mut session, "layer.add.fill", json!({"color": [0.2, 0.2, 0.2], "metallic": 1.0}));
    run(&mut session, "mask.add.black", Value::Null);
    run(&mut session, "effect.add.generator", json!({"generator": "dirt"}));
    run(&mut session, "layer.add.fill", json!({"roughness": 0.8}));
    run(&mut session, "layer.fill.set", json!({"channel": "roughness", "procedural": "marble"}));
    run(&mut session, "layer.add.group", Value::Null);
    run(&mut session, "layer.add.fill", json!({"color": [0.9, 0.9, 0.2]}));
    run(&mut session, "mask.add.black", Value::Null);
    run(&mut session, "effect.add.generator", json!({"generator": "direction"}));
    run(&mut session, "effect.add.levels", json!({"inLow": 0.3, "inHigh": 0.7}));

    let top_layer = session.doc.project.texture_sets[0].stack.last().copied().unwrap();
    run(&mut session, "layer.select", json!({"layer": top_layer.0}));
    run(&mut session, "layer.add.paint", Value::Null);
    run(&mut session, "tool.brush", json!({"size": 6, "hardness": 80}));
    run(&mut session, "tool.stroke", json!({"points": [[200, 300], [600, 500]]}));
    run(&mut session, "layer.add.paint", Value::Null);

    session.update();
    session
}

/// Convert dirty tiles for a new set and report bytes for selected and all display kinds.
/// Previously sent kinds can add conversion work when a set keeps its GPU textures.
fn upload_bytes(session: &mut Session) -> (usize, usize) {
    let tiles = session.take_uploads(0);
    let empty = Default::default();
    let baked = session.doc.baked.get(&0).unwrap_or(&empty);
    let composite = &session.caches[0].composite;
    let channels: Vec<Channel> = composite.channels.keys().copied().collect();
    let kinds: Vec<DisplayKind> = kinds_for(&channels).collect();
    let selected_bytes = tiles
        .par_iter()
        .flat_map_iter(|&(tile_x, tile_y)| kinds.iter().map(move |&kind| (kind, tile_x, tile_y)))
        .map(|(kind, tile_x, tile_y)| tile_bytes(composite, baked, kind, tile_x, tile_y).len())
        .sum();
    let all_bytes_per_tile: usize = DisplayKind::ALL.iter().map(|kind| kind.bytes_per_texel() * (TILE * TILE) as usize).sum();

    (selected_bytes, tiles.len() * all_bytes_per_tile)
}

fn mb(b: usize) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

fn bench(res: u32) {
    let t = Instant::now();
    let mut s = stack(res);
    println!("[{res}²] setup {:.2?} ({} layers, {} covered tiles)", t.elapsed(), s.doc.project.walk(0).len(), s.caches[0].covered_tiles.len());
    s.caches[0].invalidate_all();
    let t = Instant::now();
    s.update();
    println!("[{res}²] full recomposite (Session::update) {:.2?}", t.elapsed());
    let t = Instant::now();
    let (up, all) = upload_bytes(&mut s);
    println!("[{res}²] full display tile conversion {:.2?} ({:.0} MB needed kinds; {:.0} MB all kinds)", t.elapsed(), mb(up), mb(all));

    // The UI's budgeted update: a full recomposite spread over frames.
    s.caches[0].invalidate_all();
    let clock = Instant::now();
    let ms = || clock.elapsed().as_secs_f64() * 1000.0;
    let (mut frames, mut worst) = (0, Duration::ZERO);
    while s.pending_tiles() > 0 {
        let t = Instant::now();
        s.update_for(30.0, ms);
        worst = worst.max(t.elapsed());
        frames += 1;
    }
    println!("[{res}²] budgeted full recomposite: {frames} frames, worst frame {worst:.2?}, total {:.2?}", clock.elapsed());

    // Strokes on the top paint layer, twice over the same path: per-frame dirty-tile composite
    // + display conversion, then the commit. Brush sizes are relative to the mesh: the largest
    // one covers a third of the texture per stamp.
    for (size, brush) in [(0.5, "largest brush"), (0.02, "default brush")] {
        s.tool.brush.size = size;
        for pass in ["1st stroke", "2nd stroke"] {
            let (mut comp, mut conv, mut worst, mut tiles) = (Duration::ZERO, Duration::ZERO, Duration::ZERO, 0usize);
            s.begin_stroke(Vec2::new(250.0, 400.0), s.viewport, 1.0).unwrap();
            let frames = 40;
            for i in 0..frames {
                let p = Vec2::new(250.0 + i as f32 * 8.0, 400.0 + (i as f32 * 0.3).sin() * 40.0);
                s.continue_stroke(p, 1.0);
                tiles += s.caches[0].dirty.len();
                let t = Instant::now();
                s.update();
                let c = t.elapsed();
                let t = Instant::now();
                upload_bytes(&mut s);
                let u = t.elapsed();
                comp += c;
                conv += u;
                worst = worst.max(c + u);
            }
            s.end_stroke().unwrap();
            let t = Instant::now();
            s.update();
            let commit = t.elapsed();
            let f = frames as u32;
            println!(
                "[{res}²] {brush}, {pass}: frame composite {:.2?} + display {:.2?} avg, worst {worst:.2?} ({:.0} dirty tiles/frame); commit {commit:.2?}",
                comp / f,
                conv / f,
                tiles as f64 / frames as f64
            );
        }
    }
    let m = s.memory();
    println!("[{res}²] memory: layer pixels {:.1} MB, composites {:.1} MB, hot-layer cache {:.1} MB", mb(m.pixels), mb(m.composites), mb(m.cache));
}

#[test]
#[ignore]
fn bench_4k() {
    bench(4096);
}

#[test]
#[ignore]
fn bench_8k() {
    bench(8192);
}

/// What one interactive painting frame costs in the app, stage by stage, on the startup sample
/// project (2K material ball with a smart material folder): stamping, compositing the dirty
/// tiles, display conversion; then the commit at release. Painted twice: a new layer inside the
/// smart material folder (on top of its fills), then the bottom layer (everything above it).
#[test]
#[ignore]
fn bench_sample_painting() {
    let mut s = texturecraft_engine::project_cmds::sample_session(2048, 512).unwrap();
    s.viewport = Vec2::new(1000.0, 880.0);
    let bottom = s.doc.project.texture_sets[0].stack[0];
    execute(&mut s, "layer.add.paint", &Value::Null).unwrap();
    s.update();
    upload_bytes(&mut s);
    for place in ["in the smart material folder", "at the bottom of the stack"] {
        if place == "at the bottom of the stack" {
            execute(&mut s, "layer.select", &json!({"layer": bottom.0})).unwrap();
        }
        for (size, brush) in [(0.02, "default brush"), (0.08, "large brush")] {
            s.tool.brush.size = size;
            let (mut stamp, mut comp, mut conv, mut worst, mut tiles) = (Duration::ZERO, Duration::ZERO, Duration::ZERO, Duration::ZERO, 0usize);
            let t = Instant::now();
            s.begin_stroke(Vec2::new(350.0, 400.0), s.viewport, 1.0).unwrap();
            let begin = t.elapsed();
            let frames = 60;
            for i in 0..frames {
                let p = Vec2::new(350.0 + i as f32 * 5.0, 400.0 + (i as f32 * 0.2).sin() * 60.0);
                let t = Instant::now();
                s.continue_stroke(p, 1.0);
                let a = t.elapsed();
                tiles += s.caches.iter().map(|c| c.dirty.len()).sum::<usize>();
                let t = Instant::now();
                s.update();
                let b = t.elapsed();
                let t = Instant::now();
                upload_bytes(&mut s);
                let c = t.elapsed();
                stamp += a;
                comp += b;
                conv += c;
                worst = worst.max(a + b + c);
            }
            let t = Instant::now();
            s.end_stroke().unwrap();
            let end = t.elapsed();
            let committed = s.caches[0].dirty.len();
            let t = Instant::now();
            s.update();
            let after = t.elapsed();
            let f = frames as u32;
            let below = &s.caches[0].below;
            println!(
                "[sample] {place}, {brush}: begin {begin:.2?}; per frame stamp {:.2?} + composite {:.2?} ({:.2?}/tile) + display {:.2?}, worst {worst:.2?} ({:.0} dirty tiles/frame); end_stroke {end:.2?} + update {after:.2?} ({committed} tiles); cache {} tiles, {:.1} MB, {:.0} KB/tile",
                stamp / f,
                comp / f,
                comp / tiles.max(1) as u32,
                conv / f,
                tiles as f64 / frames as f64,
                below.len(),
                mb(below.bytes()),
                below.tile_bytes().unwrap_or(0) as f64 / 1024.0
            );
        }
    }
}

/// Per-texel overhead of the compositor on a one-paint-layer project.
#[test]
#[ignore]
fn bench_minimal_stack() {
    let mut s = Session::new_project("min", primitives::sphere(1.0, 64, 32), "builtin:sphere", None, 2048).unwrap();
    s.viewport = Vec2::new(800.0, 800.0);
    s.update();
    for label in ["empty set", "one paint layer"] {
        if label == "one paint layer" {
            execute(&mut s, "layer.add.paint", &Value::Null).unwrap();
            execute(&mut s, "tool.stroke", &json!({"points": [[200, 300], [600, 500]]})).unwrap();
            s.update();
        }
        s.caches[0].invalidate_all();
        let n = s.caches[0].dirty.len();
        let t = Instant::now();
        s.update();
        println!("[min] {label}: {n} tiles in {:.2?} ({:.2?}/tile)", t.elapsed(), t.elapsed() / n.max(1) as u32);
    }
}

/// Full-composite cost of every built-in material on the sample ball (1K, baked), slowest first.
#[test]
#[ignore]
fn bench_materials() {
    let mut rows = Vec::new();
    for m in texturecraft_engine::materials::MATERIALS {
        let mut s = texturecraft_engine::project_cmds::sample_session(1024, 256).unwrap();
        let top = s.doc.project.texture_sets[0].stack.clone();
        for id in top {
            let _ = execute(&mut s, "layer.remove", &json!({"layer": id.0}));
        }
        execute(&mut s, "material.apply", &json!({"material": m.id})).unwrap();
        s.caches[0].invalidate_all();
        let n = s.caches[0].dirty.len();
        let t = Instant::now();
        s.update();
        rows.push((t.elapsed() / n.max(1) as u32, m.id));
    }
    rows.sort();
    rows.reverse();
    for (per_tile, id) in rows {
        println!("[materials] {id:<28} {per_tile:.2?}/tile");
    }
}

/// Per-tile cost of single-layer stacks, to find fixed per-texel overhead (run single-threaded).
#[test]
#[ignore]
fn bench_single_layers() {
    let cases: &[(&str, &[(&str, Value)])] = &[
        ("uniform fill", &[("layer.add.fill", json!({"color": [0.5, 0.5, 0.5], "roughness": 0.4}))]),
        ("fill + clouds", &[("layer.add.fill", json!({})), ("layer.fill.set", json!({"channel": "baseColor", "procedural": "clouds"}))]),
        (
            "fill + triplanar clouds",
            &[
                ("layer.add.fill", json!({})),
                ("layer.fill.set", json!({"channel": "baseColor", "procedural": "clouds"})),
                ("layer.fill.projection", json!({"projection": "triplanar"})),
            ],
        ),
        (
            "fill + black mask + edge wear",
            &[("layer.add.fill", json!({})), ("mask.add.black", Value::Null), ("effect.add.generator", json!({"generator": "edgeWear"}))],
        ),
        ("fill + black mask + dirt", &[("layer.add.fill", json!({})), ("mask.add.black", Value::Null), ("effect.add.generator", json!({"generator": "dirt"}))]),
    ];
    for (label, cmds) in cases {
        let mut s = texturecraft_engine::project_cmds::sample_session(1024, 256).unwrap();
        for id in s.doc.project.texture_sets[0].stack.clone() {
            let _ = execute(&mut s, "layer.remove", &json!({"layer": id.0}));
        }
        for (id, p) in cmds.iter() {
            execute(&mut s, id, p).unwrap();
        }
        s.update();
        s.caches[0].invalidate_all();
        let n = s.caches[0].dirty.len();
        let t = Instant::now();
        s.update();
        println!("[single] {label:<32} {:.2?}/tile ({n} tiles)", t.elapsed() / n.max(1) as u32);
    }
}

/// The frame where an 8K stroke ends: `end_stroke` and the first budgeted update after it.
#[test]
#[ignore]
fn bench_8k_stroke_end() {
    let mut s = texturecraft_engine::project_cmds::sample_session(2048, 256).unwrap();
    s.viewport = Vec2::new(1700.0, 1230.0);
    execute(&mut s, "textureSet.resolution", &json!({"resolution": 8192})).unwrap();
    execute(&mut s, "layer.add.paint", &Value::Null).unwrap();
    s.update();
    upload_bytes(&mut s);
    for size in [0.02, 0.08] {
        s.tool.brush.size = size;
        for rep in 0..3 {
            s.begin_stroke(Vec2::new(500.0, 500.0 + rep as f32 * 40.0), s.viewport, 1.0).unwrap();
            for i in 0..60 {
                let p = Vec2::new(500.0 + i as f32 * 10.0, 500.0 + rep as f32 * 40.0 + i as f32 * 3.3);
                s.continue_stroke(p, 1.0);
                s.update();
                upload_bytes(&mut s);
            }
            let t = Instant::now();
            s.end_stroke().unwrap();
            let end = t.elapsed();
            let tiles = s.caches[0].dirty.len();
            let t = Instant::now();
            s.update();
            let after = t.elapsed();
            let t = Instant::now();
            let (up, all) = upload_bytes(&mut s);
            println!(
                "[8k] size {size}: end_stroke {end:.2?}, update {after:.2?} ({tiles} tiles), display {:.2?} ({:.1} MB needed kinds; {:.1} MB all kinds)",
                t.elapsed(),
                mb(up),
                mb(all)
            );
        }
    }
}

// Benchmark document clones and 100 undo-producing strokes
#[test]
#[ignore]
fn bench_history() {
    let resolution = 4096;
    let setup_started = Instant::now();
    let mut session = stack(resolution);
    for _ in 0..20 {
        execute(&mut session, "layer.add.paint", &Value::Null).unwrap();
        execute(&mut session, "tool.stroke", &json!({"points": [[200, 300], [260, 320]]})).unwrap();
    }
    println!("[{resolution}²] history setup {:.2?}", setup_started.elapsed());

    let clone_started = Instant::now();
    for _ in 0..100 {
        drop(std::hint::black_box(session.doc.clone()));
    }
    println!("[{resolution}²] document clone {:.2?} average over 100", clone_started.elapsed() / 100);

    let initial_history_depth = session.history_labels().len();
    let strokes_started = Instant::now();
    for stroke_index in 0..100 {
        // Keep strokes on the framed sphere within the 800-pixel viewport
        let row_y = 300 + (stroke_index % 20) * 10;
        execute(&mut session, "tool.stroke", &json!({"points": [[300, row_y], [360, row_y + 20]]})).unwrap();
    }
    let stroke_duration = strokes_started.elapsed();

    assert_eq!(session.history_labels().len(), initial_history_depth + 100, "every stroke paints the mesh and makes an undo step");
    println!("[{resolution}²] 100 strokes {stroke_duration:.2?} ({:.2?} per stroke)", stroke_duration / 100);

    let memory = session.memory();
    println!(
        "[{resolution}²] memory: layer pixels {:.1} MB, composites {:.1} MB, hot-layer cache {:.1} MB",
        mb(memory.pixels),
        mb(memory.composites),
        mb(memory.cache)
    );
}

/// Drain the pending tiles with `Session::update`, returning how long that took.
fn settle(s: &mut Session) -> Duration {
    let t = Instant::now();
    while s.pending_tiles() > 0 {
        s.update();
    }
    t.elapsed()
}

/// Undo and redo of one small stroke: the tiles left pending and the time to recomposite them.
/// `blur` keeps the stack's blurred mask to exercise neighbourhood invalidation.
fn bench_undo_at(res: u32, blur: bool) {
    use texturecraft_engine::core::{EffectKind, Filter};
    let t = Instant::now();
    let mut s = stack(res);
    if !blur {
        let blurs: Vec<_> = s
            .doc
            .project
            .layers
            .values()
            .filter_map(|l| l.mask.as_ref())
            .flat_map(|m| m.effects.iter())
            .filter(|e| matches!(e.kind, EffectKind::Filter(Filter::Blur { .. })))
            .map(|e| e.id.0)
            .collect();
        for id in blurs {
            execute(&mut s, "effect.remove", &json!({"effect": id})).unwrap();
        }
    }
    s.update();
    let label = if blur { "blur mask kept" } else { "no neighbourhood filters" };
    println!("[{res}², {label}] setup {:.2?} ({} covered tiles)", t.elapsed(), s.caches[0].covered_tiles.len());
    // Seed the layer image before measuring partial undo
    execute(&mut s, "tool.stroke", &json!({"points": [[500, 500], [520, 510]]})).unwrap();
    execute(&mut s, "tool.stroke", &json!({"points": [[300, 300], [330, 310]]})).unwrap();
    s.update();
    for (name, cmd) in [("undo", "edit.undo"), ("redo", "edit.redo")] {
        execute(&mut s, cmd, &Value::Null).unwrap();
        let pending = s.pending_tiles();
        println!("[{res}², {label}] {name}: {pending} tiles pending, settled in {:.2?}", settle(&mut s));
    }
}

#[test]
#[ignore]
fn bench_undo() {
    // The 8192 setup takes many minutes under load
    let res = 4096;
    bench_undo_at(res, true);
    bench_undo_at(res, false);
}

#[test]
#[ignore]
fn bench_8k_resize() {
    let mut s = texturecraft_engine::project_cmds::sample_session(2048, 256).unwrap();
    execute(&mut s, "layer.add.paint", &Value::Null).unwrap();
    execute(&mut s, "tool.stroke", &json!({"points": [[300, 300], [700, 500]]})).unwrap();
    s.update();
    let t = Instant::now();
    execute(&mut s, "textureSet.resolution", &json!({"resolution": 8192})).unwrap();
    println!("[resize] 2K → 8K command {:.2?}", t.elapsed());
    // The UI's way: the worker thread's part, then what the frame that applies it pays.
    let t = Instant::now();
    let out = s.resize_task(0, 4096).unwrap().run(&texturecraft_engine::bake::Progress::new()).unwrap();
    let worker = t.elapsed();
    let t = Instant::now();
    s.apply_resize(out).unwrap();
    println!("[resize] 8K → 4K worker {worker:.2?}, apply {:.2?}", t.elapsed());
    let t = Instant::now();
    let out = s.resize_task(0, 8192).unwrap().run(&texturecraft_engine::bake::Progress::new()).unwrap();
    let worker = t.elapsed();
    let t = Instant::now();
    s.apply_resize(out).unwrap();
    println!("[resize] 4K → 8K worker {worker:.2?}, apply {:.2?}", t.elapsed());
    let t = Instant::now();
    let gbuf = texturecraft_engine::geom::UvGBuffer::new(&s.mesh, 0, 8192, 8192);
    println!("[resize] gbuffer {:.2?}", t.elapsed());
    let t = Instant::now();
    let d = texturecraft_engine::texel::DilationMap::in_blocks(&gbuf.coverage_mask(), 8192, 8192, Some(8), 8192);
    println!("[resize] dilation {:.2?}", t.elapsed());
    drop(d);
}

/// Layer-panel thumbnails on the 4K stack: the snapshot, each kind of picture, and a whole panel.
#[test]
#[ignore]
fn bench_layer_thumbs_4k() {
    use texturecraft_engine::compose::ThumbTarget;
    use texturecraft_engine::core::Channel;
    use texturecraft_engine::thumbs::{GbufCache, ThumbSnapshot};
    let s = stack(4096);
    let ts = s.ts();
    let mut gbufs = GbufCache::default();
    let t = Instant::now();
    let snap = ThumbSnapshot::new(&s, ts, &mut gbufs).unwrap();
    let first = t.elapsed();
    let t = Instant::now();
    let _again = ThumbSnapshot::new(&s, ts, &mut gbufs).unwrap();
    let cached = t.elapsed();
    let p = &s.doc.project;
    let mut targets = Vec::new();
    for l in p.layers.values() {
        targets.push(("content", ThumbTarget::Content { layer: l.id, channel: Channel::BaseColor }));
        if l.mask.is_some() {
            targets.push(("mask", ThumbTarget::Mask { layer: l.id }));
        }
        for e in l.mask.iter().flat_map(|m| m.effects.iter()).chain(l.content_effects.iter()) {
            targets.push(("stage", ThumbTarget::Stage { effect: e.id, channel: Channel::BaseColor }));
        }
    }
    let mut per: std::collections::BTreeMap<&str, (Duration, usize, Duration)> = Default::default();
    let all = Instant::now();
    for (kind, target) in &targets {
        let t = Instant::now();
        let _ = snap.render(*target);
        let d = t.elapsed();
        let e = per.entry(kind).or_default();
        e.0 += d;
        e.1 += 1;
        e.2 = e.2.max(d);
    }
    let total = all.elapsed();
    eprintln!("layer thumbnails, 4096 set, {} layers, 64 px:", p.layers.len());
    eprintln!("  snapshot: {:.2} ms first (with the 64 px G-buffer), {:.2} ms after", first.as_secs_f64() * 1e3, cached.as_secs_f64() * 1e3);
    for (kind, (sum, n, max)) in per {
        eprintln!("  {kind:8} {n:3} × avg {:.2} ms, max {:.2} ms", sum.as_secs_f64() * 1e3 / n as f64, max.as_secs_f64() * 1e3);
    }
    eprintln!("  every target once: {:.1} ms ({} pictures)", total.as_secs_f64() * 1e3, targets.len());
}

/// A full recomposite in one call against the same one spread over frames, as the UI runs it, with and without a blurred mask
/// in the stack: neighbourhood filters are what a frame slice pays for apart from its own tiles.
#[test]
#[ignore]
fn bench_sliced_filters_4k() {
    for blur in [false, true] {
        let mut session = Session::new_project("sliced", primitives::sphere(1.0, 64, 32), "builtin:sphere", None, 4096).unwrap();
        session.viewport = Vec2::new(800.0, 800.0);
        let run = |session: &mut Session, id: &str, params: Value| execute(session, id, &params).unwrap();
        run(&mut session, "layer.add.fill", json!({"color": [0.6, 0.5, 0.4], "roughness": 0.6}));
        run(&mut session, "layer.add.fill", json!({"color": [0.1, 0.3, 0.6]}));
        run(&mut session, "mask.add.black", Value::Null);
        run(&mut session, "effect.add.generator", json!({"generator": "position", "level": 0.5}));
        if blur {
            run(&mut session, "effect.add.filter", json!({"filter": "blur", "radius": 8}));
        }

        session.update();

        session.caches[0].invalidate_all();
        let started = Instant::now();
        session.update();
        let direct = started.elapsed();

        session.caches[0].invalidate_all();
        let clock = Instant::now();
        let elapsed_ms = || clock.elapsed().as_secs_f64() * 1000.0;
        let (mut frames, mut longest_frame) = (0, Duration::ZERO);
        while session.pending_tiles() > 0 {
            let started = Instant::now();
            session.update_for(30.0, elapsed_ms);
            longest_frame = longest_frame.max(started.elapsed());
            frames += 1;
        }

        let label = if blur { "blurred mask" } else { "no filter" };
        println!("[sliced 4096², {label}] one call {direct:.2?}; in 30 ms slices {frames} frames, worst {longest_frame:.2?}, total {:.2?}", clock.elapsed());
    }
}

/// Input to CPU display tiles ready for upload (excludes GPU submission and presentation).
#[test]
#[ignore]
fn bench_filtered_mask_edits() {
    let resolution = 1024;
    let mut s = Session::new_project("filtered edits", primitives::plane(1.0, 1), "builtin:plane", None, resolution).unwrap();
    let run = |s: &mut Session, command, params: Value| execute(s, command, &params).unwrap();
    let layer = run(&mut s, "layer.add.fill", json!({"color": [1.0, 1.0, 1.0]}))["layer"].clone();
    run(&mut s, "mask.add.black", Value::Null);
    let paint = run(&mut s, "effect.add.paint", json!({"target": "mask"}))["effect"].clone();
    run(&mut s, "effect.add.filter", json!({"target": "mask", "filter": "blur", "radius": 8}));
    run(&mut s, "layer.select", json!({"layer": layer, "target": paint}));
    run(&mut s, "tool.brush", json!({"size": 0.6, "hardness": 100}));
    s.begin_stroke_uv(Vec2::splat(0.1), 1.0).unwrap();
    s.end_stroke().unwrap();
    s.update();
    s.take_uploads(0);

    let mut samples = [Vec::new(), Vec::new(), Vec::new()];
    for i in 0..35 {
        let uv = Vec2::new((124.5 + (i % 7) as f32 * 128.0) / resolution as f32, (160.5 + (i / 7) as f32 * 128.0) / resolution as f32);
        for (phase, times) in samples.iter_mut().enumerate() {
            let start = Instant::now();
            match phase {
                0 => s.begin_stroke_uv(uv, 1.0).unwrap(),
                1 => {
                    s.undo().unwrap();
                }
                _ => {
                    s.redo().unwrap();
                }
            }
            let pending = s.pending_tiles();
            s.update();
            let tiles = s.take_uploads(0);
            let cache = &s.caches[0];
            let channels: Vec<_> = cache.composite.channels.keys().copied().collect();
            let fallback = Default::default();
            let baked = s.doc.baked.get(&0).unwrap_or(&fallback);
            tiles.par_iter().for_each(|&(x, y)| {
                for kind in kinds_for(&channels) {
                    std::hint::black_box(tile_bytes(&cache.composite, baked, kind, x, y));
                }
            });
            if i >= 3 {
                times.push((start.elapsed(), pending));
            }
            if phase == 0 {
                s.end_stroke().unwrap();
                s.update();
                s.take_uploads(0);
            }
        }
    }
    for (phase, mut times) in ["preview", "undo", "redo"].into_iter().zip(samples) {
        times.sort_unstable();
        let p50 = times[times.len().div_ceil(2) - 1];
        let p95 = times[(times.len() * 95).div_ceil(100) - 1];
        println!("{phase}: {} samples, CPU display ready P50 {:.2?}, P95 {:.2?}, {} pending tiles at median", times.len(), p50.0, p95.0, p50.1);
    }
}

/// First whole-material image after opening a saved project or resizing its texture set.
/// Compare overview sizes without warming the full-resolution layer cache.
#[test]
#[ignore = "material overview benchmark"]
fn bench_open_resize_overview() {
    let source = stack(1024).to_project_bytes().unwrap();
    let started = Instant::now();
    let mut session = Session::from_project_bytes(&source).unwrap();
    println!("open 1K document and build UV cache: {:?}", started.elapsed());
    for size in [1024, 4096] {
        if size != 1024 {
            let started = Instant::now();
            execute(&mut session, "textureSet.resolution", &json!({"resolution":size})).unwrap();
            println!("resize to {size} and build UV cache: {:?}", started.elapsed());
        }
        let pending = session.pending_tiles();
        for edge in [128, 256, 256, 128] {
            let started = Instant::now();
            let preview = session.material_overview(0, edge).unwrap();
            println!("{size} -> {edge} whole atlas: {:?}, {} bytes", started.elapsed(), preview.maps.iter().map(|(_, bytes)| bytes.len()).sum::<usize>());
            assert_eq!(pending, session.pending_tiles());
        }
        let started = Instant::now();
        session.update();
        println!("{size} full-resolution CPU composite: {:?}", started.elapsed());
    }
}

/// Batch the same 64px storage tiles into larger square CPU jobs for a cold 1K composite.
/// Each run reads the same layer stack, with no cross-job filter lookahead or GPU work.
#[test]
#[ignore = "compositing work size benchmark"]
fn bench_cold_work_regions() {
    use texturecraft_engine::compose::{BakedMaps, BelowCache, Composite, Inputs, composite_tiles_cached};
    let session = stack(1024);
    let cache = &session.caches[0];
    let set = &session.doc.project.texture_sets[0];
    let empty = BakedMaps::default();
    let inputs = Inputs {
        project: &session.doc.project,
        ts: 0,
        mesh: &session.mesh,
        gbuf: &cache.gbuf,
        pixels: &session.doc.pixels,
        baked: session.doc.baked.get(&0).unwrap_or(&empty),
        block_scales: set.block_scales(),
        preview: None,
        images: &session.images,
        graphs: &session.doc.graphs,
        bounds: session.mesh.bounds(),
    };
    let mut reference = None;
    for edge in [64, 128, 256, 256, 128, 64] {
        let mut composite = Composite::new(&set.channel_list(), 1024);
        let mut cache = BelowCache::default();
        cache.set_hot(session.selected_layer());
        cache.pass = (1024 / TILE).pow(2) as usize;
        let started = Instant::now();
        for y in (0..1024).step_by(edge) {
            for x in (0..1024).step_by(edge) {
                let x_tiles = x / TILE..(x + edge as u32) / TILE;
                let tiles: Vec<_> = (y / TILE..(y + edge as u32) / TILE).flat_map(|ty| x_tiles.clone().map(move |tx| (tx, ty))).collect();
                composite_tiles_cached(&inputs, &mut composite, &tiles, Some(&mut cache));
            }
        }
        println!("{edge}x{edge} work regions: {:?}", started.elapsed());
        if let Some(expected) = &reference {
            assert!(expected == &composite, "batch size changed the resulting pixels");
        } else {
            reference = Some(composite);
        }
    }
}
