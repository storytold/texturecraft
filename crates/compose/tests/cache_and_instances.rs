//! The hot-layer cache (the stack below and the tape above it, through folders) and blocked
//! compositing give exactly the unoptimized result; instance layers draw their source; masks
//! combine with height.

use std::collections::BTreeMap;

use texturecraft_compose::{
    BelowCache, Composite, Inputs, MaskGray, PaintTarget, PixelStore, StrokePreview, all_tiles, composite_tiles, composite_tiles_cached,
};
use texturecraft_core::project::ENV_RESOURCE_PREFIX;
use texturecraft_core::{
    BakedMap, BlendMode, Channel, ColorSelection, CompareOp, CompareSource, ContentsOffset, Effect, EffectKind, Fill, Filter, Generator, GeneratorKind,
    GrayscaleConversion, HeightMask, ImageInput, Layer, LayerId, LayerKind, Mask, MaskBackground, Procedural, ProceduralKind, Project, Projection, Source,
    TextureSet,
};
use texturecraft_geom::{Mesh, UvGBuffer, primitives};
use texturecraft_texel::{TILE, TileImage, to_u16};

#[path = "cache_and_instances/blur_on.rs"]
mod blur_on;

use blur_on::BlurOn;

const RES: u32 = 256;

struct Fixture {
    project: Project,
    mesh: Mesh,
    gbuf: UvGBuffer,
    pixels: PixelStore,
}

fn fixture() -> Fixture {
    let mesh = primitives::sphere(1.0, 32, 16);
    let gbuf = UvGBuffer::new(&mesh, 0, RES, RES);
    let mut project = Project::new("t", "builtin:sphere");
    project.texture_sets.push(TextureSet::new("A", 0, RES));
    project.texture_sets.push(TextureSet::new("B", 0, RES));
    Fixture { project, mesh, gbuf, pixels: PixelStore::default() }
}

fn procedural(kind: ProceduralKind, low: f32, high: f32) -> Source {
    Source::Procedural { pattern: Procedural::new(kind), low: [low; 3], high: [high; 3], ramp: Vec::new() }
}

fn add(f: &mut Fixture, ts: usize, kind: LayerKind, mask: Option<Mask>) -> LayerId {
    put(f, ts, None, kind, mask)
}

/// A layer on top of `folder`'s children, or of set `ts`'s stack.
fn put(f: &mut Fixture, ts: usize, folder: Option<LayerId>, kind: LayerKind, mask: Option<Mask>) -> LayerId {
    let id = f.project.alloc_id();
    let mut l = Layer::new(id, "l", kind, &Channel::DEFAULT_SET);
    l.mask = mask;
    f.project.insert_layer(ts, l, folder, folder.is_some()).unwrap()
}

fn fill(sources: &[(Channel, Source)]) -> LayerKind {
    let mut f = Fill::default();
    f.sources = sources.iter().cloned().collect();
    LayerKind::Fill(f)
}

fn effect(f: &mut Fixture, kind: EffectKind) -> Effect {
    Effect::new(f.project.alloc_id(), kind)
}

/// Procedural fills, a generator mask with a blur, a folder, painted pixels; returns the top
/// paint layer.
fn heavy_stack(f: &mut Fixture) -> LayerId {
    add(
        f,
        0,
        fill(&[(Channel::BaseColor, procedural(ProceduralKind::Clouds, 0.1, 0.9)), (Channel::Height, procedural(ProceduralKind::Cells, -0.3, 0.3))]),
        None,
    );
    let mut mask = Mask::new(MaskBackground::Black);
    mask.effects.push(effect(f, EffectKind::Generator(Generator::new(GeneratorKind::Position))));
    mask.effects.push(effect(f, EffectKind::Filter(Filter::Blur { radius: 40.0, wrap: false })));
    add(f, 0, fill(&[(Channel::Roughness, procedural(ProceduralKind::Grunge, 0.0, 1.0))]), Some(mask));
    let group = add(f, 0, LayerKind::Group { children: vec![], collapsed: false, contents: Default::default(), randomized: Vec::new() }, None);
    for b in f.project.layer_mut(group).unwrap().channels.values_mut() {
        b.mode = BlendMode::Passthrough;
    }
    let inner = f.project.alloc_id();
    let l = Layer::new(inner, "inner", fill(&[(Channel::Metallic, procedural(ProceduralKind::Scratches, 0.0, 1.0))]), &Channel::DEFAULT_SET);
    f.project.insert_layer(0, l, Some(group), true).unwrap();
    let mut blurred = Mask::new(MaskBackground::Black);
    blurred.effects.push(effect(
        f,
        EffectKind::Fill { source: procedural(ProceduralKind::Checker, 0.0, 1.0), projection: Projection::default(), sampling: Default::default() },
    ));
    blurred.effects.push(effect(f, EffectKind::Filter(Filter::Blur { radius: 24.0, wrap: false })));
    let top = add(f, 0, LayerKind::Paint, Some(blurred));
    let img = f.pixels.layer_mut(top, Channel::BaseColor, (RES, RES));
    for y in 40..200 {
        for x in 30..220 {
            img.set(x, y, &[to_u16(0.4), to_u16(0.1), 0, to_u16(0.5)]);
        }
    }
    top
}

fn inputs<'a>(f: &'a Fixture, ts: usize, images: &'a texturecraft_compose::ImageStore, baked: &'a texturecraft_compose::BakedMaps) -> Inputs<'a> {
    Inputs {
        project: &f.project,
        ts,
        mesh: &f.mesh,
        gbuf: &f.gbuf,
        pixels: &f.pixels,
        baked,
        block_scales: f.project.texture_sets[ts].block_scales(),
        preview: None,
        images,
        bounds: f.mesh.bounds(),
    }
}

fn run(f: &Fixture, ts: usize, tiles: &[(u32, u32)], below: Option<&mut BelowCache>) -> Composite {
    let (images, baked) = (Default::default(), Default::default());
    let mut out = Composite::new(&f.project.texture_sets[ts].channel_list(), RES);
    composite_tiles_cached(&inputs(f, ts, &images, &baked), &mut out, tiles, below);
    out
}

fn reference(f: &Fixture, ts: usize) -> Composite {
    let (images, baked) = (Default::default(), Default::default());
    let mut out = Composite::new(&f.project.texture_sets[ts].channel_list(), RES);
    composite_tiles(&inputs(f, ts, &images, &baked), &mut out, &all_tiles(RES, RES));
    out
}

#[test]
fn below_cache_and_blocks_match_the_full_composite_exactly() {
    let mut f = fixture();
    let top = heavy_stack(&mut f);
    let want = reference(&f, 0);
    // Tile by tile (each its own block and neighbourhood region) gives the same bits.
    let mut one_by_one = Composite::new(&f.project.texture_sets[0].channel_list(), RES);
    let (images, baked) = (Default::default(), Default::default());
    for t in all_tiles(RES, RES) {
        composite_tiles(&inputs(&f, 0, &images, &baked), &mut one_by_one, &[t]);
    }
    assert!(one_by_one == want, "per-tile composites differ from one pass");

    let mut cache = BelowCache::default();
    cache.set_hot(Some(top));
    let half: Vec<(u32, u32)> = all_tiles(RES, RES).into_iter().filter(|t| (t.0 + t.1) % 2 == 0).collect();
    // Cold, half warm and fully warm caches.
    let mut out = run(&f, 0, &half, Some(&mut cache));
    assert!(!cache.is_empty(), "the stack below the hot layer is cached");
    composite_tiles_cached(&inputs(&f, 0, &images, &baked), &mut out, &all_tiles(RES, RES), Some(&mut cache));
    assert!(out == want, "a partly cached composite differs");
    let warm = run(&f, 0, &all_tiles(RES, RES), Some(&mut cache));
    assert!(warm == want, "a cached composite differs");

    // Painting the hot layer keeps the cache; editing below it empties it.
    f.pixels.layer_mut(top, Channel::BaseColor, (RES, RES)).set(5, 5, &[to_u16(1.0), 0, 0, to_u16(1.0)]);
    let n = cache.len();
    assert!(run(&f, 0, &all_tiles(RES, RES), Some(&mut cache)) == reference(&f, 0));
    assert_eq!(cache.len(), n);
    let low = f.project.texture_sets[0].stack[0];
    if let LayerKind::Fill(fl) = &mut f.project.layer_mut(low).unwrap().kind {
        fl.sources.insert(Channel::Roughness, Source::Uniform([0.9; 3]));
    }
    assert!(run(&f, 0, &all_tiles(RES, RES), Some(&mut cache)) == reference(&f, 0), "a stale cache was used");
    // A hot layer inside a folder caches below the folder.
    let group = f.project.texture_sets[0].stack[2];
    let inner = f.project.layer(group).unwrap().children()[0];
    cache.set_hot(Some(inner));
    run(&f, 0, &all_tiles(RES, RES), Some(&mut cache));
    assert!(!cache.is_empty());
    assert!(run(&f, 0, &all_tiles(RES, RES), Some(&mut cache)) == reference(&f, 0));
}

#[test]
fn instances_draw_their_source_across_texture_sets() {
    let mut f = fixture();
    let top = heavy_stack(&mut f);
    // Set B instances every top-level layer of set A: same composite.
    for id in f.project.texture_sets[0].stack.clone() {
        add(&mut f, 1, LayerKind::Instance { source: id }, None);
    }
    let a = reference(&f, 0);
    assert!(reference(&f, 1).channels == a.channels, "instances draw like their sources");
    // An edit to the source shows in the instance.
    f.pixels.layer_mut(top, Channel::BaseColor, (RES, RES)).set(100, 100, &[to_u16(1.0), 0, 0, to_u16(1.0)]);
    let (a, b) = (reference(&f, 0), reference(&f, 1));
    assert!(b.channels == a.channels);
    // The instance's own mask overrides the source's.
    let inst = *f.project.texture_sets[1].stack.last().unwrap();
    f.project.layer_mut(inst).unwrap().mask = Some(Mask::new(MaskBackground::Black));
    let b = reference(&f, 1);
    let (x, y) = (100, 100);
    assert!((a.value(Channel::BaseColor, x, y)[0] - b.value(Channel::BaseColor, x, y)[0]).abs() > 0.1);
    // A hidden instance or a missing source draws nothing.
    let mut g = fixture();
    let src = add(&mut g, 0, fill(&[(Channel::Roughness, Source::Uniform([0.9; 3]))]), None);
    let inst = add(&mut g, 1, LayerKind::Instance { source: src }, None);
    assert!((reference(&g, 1).value(Channel::Roughness, 128, 128)[0] - 0.9).abs() < 1e-3);
    g.project.layer_mut(inst).unwrap().visible = false;
    assert!((reference(&g, 1).value(Channel::Roughness, 128, 128)[0] - 0.5).abs() < 1e-3);
    g.project.layer_mut(inst).unwrap().visible = true;
    g.project.layers.remove(&src);
    assert!((reference(&g, 1).value(Channel::Roughness, 128, 128)[0] - 0.5).abs() < 1e-3);
}

#[test]
fn mask_with_height_follows_the_layer_height() {
    let mut f = fixture();

    // Half masking makes the height fall from 0.25 at left to -0.25 at right
    let mut ramp = Procedural::new(ProceduralKind::Gradient);
    ramp.scale = 1.0;
    let height = Source::Procedural { pattern: ramp, low: [0.25; 3], high: [-0.25; 3], ramp: Vec::new() };
    let id = add(&mut f, 0, fill(&[(Channel::BaseColor, Source::Uniform([1.0; 3])), (Channel::Height, height)]), None);
    let mut mask = Mask::new(MaskBackground::Black);
    mask.effects.push(effect(&mut f, EffectKind::Fill { source: Source::Uniform([0.5; 3]), projection: Projection::default(), sampling: Default::default() }));
    f.project.layer_mut(id).unwrap().mask = Some(mask);
    let plain = reference(&f, 0);
    f.project.layer_mut(id).unwrap().height_mask = HeightMask { enabled: true, contrast: 1.0, offset: 0.0 };
    let hm = reference(&f, 0);
    let row = TILE * 2;
    let (left, right) = (16, RES - 16);
    let v = |c: &Composite, x| c.value(Channel::BaseColor, x, row)[0];

    assert!((v(&plain, left) - 0.75).abs() < 0.01 && (v(&plain, right) - 0.75).abs() < 0.01, "plain mask: 50 % white over gray");
    assert!(v(&hm, left) > 0.84, "high parts show first: {}", v(&hm, left));
    assert!(v(&hm, right) < 0.66, "low parts later: {}", v(&hm, right));

    // Without a mask there is nothing to combine
    f.project.layer_mut(id).unwrap().mask.as_mut().unwrap().enabled = false;

    assert!((v(&reference(&f, 0), left) - 1.0).abs() < 1e-3);
}

fn group(f: &mut Fixture, folder: Option<LayerId>, mode: BlendMode, opacity: f32, mask: Option<Mask>) -> LayerId {
    let id = put(f, 0, folder, LayerKind::Group { children: vec![], collapsed: false, contents: Default::default(), randomized: Vec::new() }, mask);
    blend(f, id, mode, opacity);
    id
}

fn blend(f: &mut Fixture, id: LayerId, mode: BlendMode, opacity: f32) {
    for b in f.project.layer_mut(id).unwrap().channels.values_mut() {
        b.mode = mode;
        b.opacity = opacity;
    }
}

/// A black mask revealed by a noisy generator, blurred when `blur` is set.
fn generator_mask(f: &mut Fixture, kind: GeneratorKind, blur: Option<f32>) -> Mask {
    let mut g = Generator::new(kind);
    g.noise_amount = 0.4;
    g.noise_scale = 3.0;
    let mut mask = Mask::new(MaskBackground::Black);
    mask.effects.push(effect(f, EffectKind::Generator(g)));
    if let Some(radius) = blur {
        mask.effects.push(effect(f, EffectKind::Filter(Filter::Blur { radius, wrap: false })));
    }
    mask
}

/// Paint a half-covered rectangle into a paint layer's base color and roughness.
fn paint_rect(f: &mut Fixture, id: LayerId, (x0, y0): (u32, u32), (w, h): (u32, u32), v: f32) {
    for (ch, px) in [(Channel::BaseColor, vec![to_u16(v * 0.5), to_u16(0.1), 0, to_u16(0.5)]), (Channel::Roughness, vec![to_u16(v * 0.5), to_u16(0.5)])] {
        let img = f.pixels.layer_mut(id, ch, (RES, RES));
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                img.set(x, y, &px);
            }
        }
    }
}

struct Nested {
    floor: LayerId,
    base: LayerId,
    g1: LayerId,
    inner: LayerId,
    g2: LayerId,
    deep: LayerId,
    g2_above: LayerId,
    g1_above: LayerId,
    top_fill: LayerId,
    top: LayerId,
}

/// Set 0, bottom to top: a painted layer; a procedural fill; a normal folder (generator mask, 80 %) holding a
/// fill, a pass-through folder (checker mask) holding [a fill with a blurred generator mask,
/// the `deep` paint layer, an overlay fill with a generator mask, an instance of the base fill]
/// and a screen fill with HSL and blur content effects; a multiply fill with a blurred
/// generator mask; a painted layer with an invert effect.
fn nested_stack(f: &mut Fixture) -> Nested {
    let floor = add(f, 0, LayerKind::Paint, None);
    paint_rect(f, floor, (0, 0), (256, 200), 0.6);
    let base = add(
        f,
        0,
        fill(&[(Channel::BaseColor, procedural(ProceduralKind::Clouds, 0.1, 0.9)), (Channel::Height, procedural(ProceduralKind::Cells, -0.3, 0.3))]),
        None,
    );
    let mask = generator_mask(f, GeneratorKind::Position, None);
    let g1 = group(f, None, BlendMode::Normal, 0.8, Some(mask));
    let inner =
        put(f, 0, Some(g1), fill(&[(Channel::Roughness, procedural(ProceduralKind::Grunge, 0.0, 1.0)), (Channel::Metallic, Source::Uniform([0.3; 3]))]), None);
    let mut checker = Mask::new(MaskBackground::Black);
    checker.effects.push(effect(
        f,
        EffectKind::Fill { source: procedural(ProceduralKind::Checker, 0.2, 1.0), projection: Projection::default(), sampling: Default::default() },
    ));
    let g2 = group(f, Some(g1), BlendMode::Passthrough, 1.0, Some(checker));
    let mask = generator_mask(f, GeneratorKind::Direction, Some(16.0));
    put(f, 0, Some(g2), fill(&[(Channel::BaseColor, procedural(ProceduralKind::Scratches, 0.0, 1.0))]), Some(mask));
    let deep = put(f, 0, Some(g2), LayerKind::Paint, None);
    paint_rect(f, deep, (20, 30), (150, 120), 0.8);
    let mask = generator_mask(f, GeneratorKind::Position, None);
    let g2_above =
        put(f, 0, Some(g2), fill(&[(Channel::BaseColor, Source::Uniform([0.2, 0.6, 0.3])), (Channel::Roughness, Source::Uniform([0.9; 3]))]), Some(mask));
    blend(f, g2_above, BlendMode::Overlay, 0.7);
    put(f, 0, Some(g2), LayerKind::Instance { source: base }, Some(Mask::new(MaskBackground::White)));
    let g1_above = put(f, 0, Some(g1), fill(&[(Channel::BaseColor, procedural(ProceduralKind::Wood, 0.2, 0.8))]), None);
    let hsl = effect(f, EffectKind::Filter(Filter::Hsl { hue: 0.2, saturation: 0.1, lightness: 0.0, perceptive: false }));
    let blur = effect(f, EffectKind::Filter(Filter::Blur { radius: 20.0, wrap: false }));
    f.project.layer_mut(g1_above).unwrap().content_effects.extend([hsl, blur]);
    blend(f, g1_above, BlendMode::Screen, 0.5);
    let mask = generator_mask(f, GeneratorKind::Position, Some(30.0));
    let top_fill = add(f, 0, fill(&[(Channel::BaseColor, Source::Uniform([0.9, 0.5, 0.1])), (Channel::Height, Source::Uniform([0.2; 3]))]), Some(mask));
    blend(f, top_fill, BlendMode::Multiply, 0.6);
    let top = add(f, 0, LayerKind::Paint, None);
    paint_rect(f, top, (100, 60), (120, 150), 0.3);
    let invert = effect(f, EffectKind::Filter(Filter::Invert));
    f.project.layer_mut(top).unwrap().content_effects.push(invert);
    Nested { floor, base, g1, inner, g2, deep, g2_above, g1_above, top_fill, top }
}

/// A live stroke over the middle of the texture painting `target`.
fn stroke(target: LayerId) -> StrokePreview {
    let mut alpha = TileImage::new(RES, RES, 1);
    for y in 90..170 {
        for x in 10..240 {
            alpha.set(x, y, &[to_u16(((x + y) % 7) as f32 / 6.0)]);
        }
    }
    StrokePreview {
        target: PaintTarget::Layer(target),
        alpha,
        opacity: 0.9,
        erase: false,
        values: [(Channel::BaseColor, [1.0, 0.1, 0.1]), (Channel::Roughness, [0.2; 3])].into_iter().collect(),
        mask_gray: MaskGray::Value(1.0),
        stencil: None,
        carried: BTreeMap::new(),
        carried_mask: None,
    }
}

fn run_with(f: &Fixture, ts: usize, tiles: &[(u32, u32)], below: Option<&mut BelowCache>, preview: Option<&StrokePreview>) -> Composite {
    let (images, baked) = (Default::default(), Default::default());
    let mut inp = inputs(f, ts, &images, &baked);
    inp.preview = preview;
    let mut out = Composite::new(&f.project.texture_sets[ts].channel_list(), RES);
    composite_tiles_cached(&inp, &mut out, tiles, below);
    out
}

/// With `hot` cached, cold, partly warm and warm composites (and, for paint layers, a live
/// stroke on it) match the full composite bit for bit. Returns the warm cache.
fn check_cached(f: &Fixture, ts: usize, hot: LayerId) -> BelowCache {
    let all = all_tiles(RES, RES);
    let want = reference(f, ts);
    let mut cache = BelowCache::default();
    cache.set_hot(Some(hot));
    let half: Vec<(u32, u32)> = all.iter().copied().filter(|t| (t.0 + t.1) % 2 == 0).collect();
    let mut out = run(f, ts, &half, Some(&mut cache));
    let (images, baked) = (Default::default(), Default::default());
    composite_tiles_cached(&inputs(f, ts, &images, &baked), &mut out, &all, Some(&mut cache));
    assert!(out == want, "a partly cached composite differs (hot {hot:?})");
    assert!(run(f, ts, &all, Some(&mut cache)) == want, "a warm composite differs (hot {hot:?})");
    if matches!(f.project.layer(hot).unwrap().kind, LayerKind::Paint) {
        let pv = stroke(hot);
        let n = cache.len();
        assert!(run_with(f, ts, &all, Some(&mut cache), Some(&pv)) == run_with(f, ts, &all, None, Some(&pv)), "a live stroke differs (hot {hot:?})");
        assert_eq!(cache.len(), n, "the live stroke on the hot layer keeps the cache");
    }
    cache
}

#[test]
fn hot_layers_anywhere_in_nested_folders_composite_exactly() {
    let mut f = fixture();
    let n = nested_stack(&mut f);
    for hot in [n.deep, n.inner, n.g2, n.g1_above, n.top_fill, n.top, n.floor] {
        let cache = check_cached(&f, 0, hot);
        assert!(!cache.is_empty(), "hot {hot:?} is cached");
    }
    // The base fill is drawn twice (an instance in a folder above): nothing is cached.
    assert!(check_cached(&f, 0, n.base).is_empty());
    // Committing the stroke (painting the hot layer) keeps the cache and stays exact.
    let mut cache = check_cached(&f, 0, n.deep);
    let len = cache.len();
    paint_rect(&mut f, n.deep, (60, 100), (100, 40), 0.1);
    assert!(run(&f, 0, &all_tiles(RES, RES), Some(&mut cache)) == reference(&f, 0));
    assert_eq!(cache.len(), len);
}

#[test]
fn instanced_folders_cache_and_hot_layers_drawn_twice_fall_back() {
    let mut f = fixture();
    let n = nested_stack(&mut f);
    // Set B draws set A's top level through instances: the hot layer sits in an instanced folder.
    for id in f.project.texture_sets[0].stack.clone() {
        add(&mut f, 1, LayerKind::Instance { source: id }, None);
    }
    for hot in [n.deep, n.g1, n.top] {
        let cache = check_cached(&f, 1, hot);
        assert!(!cache.is_empty(), "hot {hot:?} is cached through its instance");
    }
    // An instance of the hot layer above it: drawn twice, nothing is cached.
    put(&mut f, 0, Some(n.g1), LayerKind::Instance { source: n.deep }, None);
    assert!(check_cached(&f, 0, n.deep).is_empty());
}

#[test]
fn anchor_readers_fall_back_to_full_evaluation() {
    let mut f = fixture();
    let n = nested_stack(&mut f);
    let anchor = effect(&mut f, EffectKind::Anchor);
    let source = Source::Anchor { anchor: anchor.id, channel: Channel::Height, ramp: Vec::new() };
    f.project.layer_mut(n.base).unwrap().content_effects.push(anchor);

    // A mask above the hot layer reads the stack below it
    let reader = effect(&mut f, EffectKind::Fill { source, projection: Projection::default(), sampling: Default::default() });
    f.project.layer_mut(n.g2_above).unwrap().mask.as_mut().unwrap().effects.push(reader);
    for hot in [n.deep, n.top] {
        assert!(check_cached(&f, 0, hot).is_empty(), "anchors read the stack: nothing cached");
    }
}

#[test]
fn edits_off_the_hot_layer_invalidate_and_edits_to_it_do_not() {
    let mut f = fixture();
    let n = nested_stack(&mut f);
    let all = all_tiles(RES, RES);
    let mut cache = check_cached(&f, 0, n.deep);
    // One tile composited after an edit: a cache emptied by the edit holds just that tile.
    let mut emptied = |f: &Fixture| {
        run(f, 0, &[(1, 1)], Some(&mut cache));
        let emptied = cache.len() == 1;
        assert!(run(f, 0, &all, Some(&mut cache)) == reference(f, 0), "a stale cache was used");
        emptied
    };
    paint_rect(&mut f, n.deep, (0, 0), (200, 200), 0.5);
    assert!(!emptied(&f), "painting the hot layer");
    blend(&mut f, n.deep, BlendMode::Multiply, 0.5);
    f.project.layer_mut(n.deep).unwrap().mask = Some(Mask::new(MaskBackground::White));
    assert!(!emptied(&f), "the hot layer's blending and mask");
    if let LayerKind::Fill(fl) = &mut f.project.layer_mut(n.g2_above).unwrap().kind {
        fl.sources.insert(Channel::Roughness, Source::Uniform([0.1; 3]));
    }
    assert!(emptied(&f), "a fill value above");
    if let EffectKind::Generator(g) = &mut f.project.layer_mut(n.g2_above).unwrap().mask.as_mut().unwrap().effects[0].kind {
        g.level = 0.2;
    }
    assert!(emptied(&f), "a mask effect parameter above");
    f.project.layer_mut(n.top_fill).unwrap().visible = false;
    assert!(emptied(&f), "visibility above");
    f.project.move_layer(n.g1_above, 0).unwrap();
    assert!(emptied(&f), "a reorder");
    blend(&mut f, n.g1, BlendMode::Normal, 0.4);
    assert!(emptied(&f), "a folder holding the hot layer");
    paint_rect(&mut f, n.top, (0, 0), (30, 30), 0.9);
    assert!(emptied(&f), "pixels above");
}

/// Paint a half-covered rectangle (base color and roughness `v`) into a paint layer stored at
/// `size` texels.
fn paint_rect_at(f: &mut Fixture, id: LayerId, size: u32, (x0, y0): (u32, u32), (w, h): (u32, u32), v: f32) {
    for (ch, px) in [(Channel::BaseColor, vec![to_u16(v * 0.5), to_u16(0.1), 0, to_u16(0.5)]), (Channel::Roughness, vec![to_u16(v * 0.5), to_u16(0.5)])] {
        let img = f.pixels.layer_mut(id, ch, (size, size));
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                img.set(x, y, &px);
            }
        }
    }
}

#[test]
fn instances_resample_sources_painted_at_another_resolution() {
    let mut f = fixture();
    let half = RES / 2;
    f.project.texture_sets[1].resolution = half;
    // The source lives in set B at half resolution, masked by half-resolution mask pixels.
    let src = add(&mut f, 1, LayerKind::Paint, None);
    paint_rect_at(&mut f, src, half, (20, 30), (60, 50), 0.9);
    let paint = effect(&mut f, EffectKind::Paint);
    let mask_id = paint.id;
    f.project.layer_mut(src).unwrap().mask = Some(Mask { effects: vec![paint], ..Mask::new(MaskBackground::Black) });
    let m = f.pixels.mask_mut(mask_id, (half, half));
    for y in 0..half {
        for x in 0..50 {
            m.set(x, y, &[to_u16(1.0), to_u16(1.0)]);
        }
    }
    add(&mut f, 0, fill(&[(Channel::Metallic, procedural(ProceduralKind::Clouds, 0.0, 1.0))]), None);
    let inst = add(&mut f, 0, LayerKind::Instance { source: src }, None);
    let top = add(&mut f, 0, LayerKind::Paint, None);
    paint_rect(&mut f, top, (0, 0), (40, 40), 0.2);
    let out = reference(&f, 0);
    let rough = |x, y| out.value(Channel::Roughness, x, y)[0];
    // Painted roughness 0.9 at half coverage over the default 0.5, where the mask lets it through.
    assert!((rough(70, 120) - 0.7).abs() < 2e-3, "inside: {}", rough(70, 120));
    assert!((rough(180, 120) - 0.5).abs() < 2e-3, "masked out: {}", rough(180, 120));
    assert!((rough(70, 200) - 0.5).abs() < 2e-3, "not painted: {}", rough(70, 200));
    for hot in [top, inst] {
        check_cached(&f, 0, hot);
    }
    assert!(!check_cached(&f, 0, top).is_empty());
}

#[test]
fn anchors_inside_a_content_stack_record_the_content_up_to_them() {
    let mut f = fixture();
    let low = add(&mut f, 0, fill(&[(Channel::Height, Source::Uniform([0.4; 3]))]), None);
    f.project.layer_mut(low).unwrap().channels.retain(|c, _| *c == Channel::Height);
    blend(&mut f, low, BlendMode::Normal, 0.5);
    let (mid, end) = (effect(&mut f, EffectKind::Anchor), effect(&mut f, EffectKind::Anchor));
    let (mid_id, end_id) = (mid.id, end.id);
    let mut raise = Fill::default();
    raise.sources.insert(Channel::Height, Source::Uniform([0.8; 3]));
    let raise = effect(&mut f, EffectKind::Material(raise));
    f.project.layer_mut(low).unwrap().content_effects.extend([mid, raise, end]);
    let top = add(&mut f, 0, LayerKind::Paint, None);
    paint_rect(&mut f, top, (0, 0), (60, 60), 0.3);

    // Without readers the stack is cached (anchors are still recorded) and exact
    assert!(!check_cached(&f, 0, top).is_empty());

    // Readers: roughness reads the anchor inside the stack, metallic the one at its end
    for (c, anchor) in [(Channel::Roughness, mid_id), (Channel::Metallic, end_id)] {
        let reader = add(&mut f, 0, fill(&[(c, Source::Anchor { anchor, channel: Channel::Height, ramp: Vec::new() })]), None);
        f.project.layer_mut(reader).unwrap().channels.retain(|x, _| *x == c);
    }
    let out = reference(&f, 0);
    let at = |c| out.value(c, 128, 128)[0];

    // Half opacity over the default height 0: 0.4 before the raise, 0.8 after it
    assert!((at(Channel::Roughness) - 0.2).abs() < 2e-3, "inside the stack: {}", at(Channel::Roughness));
    assert!((at(Channel::Metallic) - 0.4).abs() < 2e-3, "at the end: {}", at(Channel::Metallic));
    assert!((at(Channel::Height) - 0.4).abs() < 2e-3);
    assert!(check_cached(&f, 0, top).is_empty(), "anchor readers evaluate everything");

    // Behind a blur the cut reads the filtered field; before it, the unfiltered content
    let blur = effect(&mut f, EffectKind::Filter(Filter::Blur { radius: 30.0, wrap: false }));
    f.project.layer_mut(low).unwrap().content_effects.insert(0, blur);
    let blurred = reference(&f, 0);
    assert!((blurred.value(Channel::Roughness, 128, 128)[0] - 0.2).abs() < 2e-3, "a flat height stays flat");

    let mut one_by_one = Composite::new(&f.project.texture_sets[0].channel_list(), RES);
    let (images, baked) = (Default::default(), Default::default());
    for t in all_tiles(RES, RES) {
        composite_tiles(&inputs(&f, 0, &images, &baked), &mut one_by_one, &[t]);
    }

    assert!(one_by_one == blurred);
}

#[test]
fn gathering_filters_composite_identically_in_any_grouping_and_cached() {
    let mut f = fixture();
    let base = add(
        &mut f,
        0,
        fill(&[(Channel::BaseColor, procedural(ProceduralKind::Clouds, 0.1, 0.9)), (Channel::Roughness, procedural(ProceduralKind::Checker, 0.0, 1.0))]),
        None,
    );
    let mut slope = Mask::new(MaskBackground::Black);
    slope.effects.push(effect(
        &mut f,
        EffectKind::Fill { source: procedural(ProceduralKind::Checker, 0.0, 1.0), projection: Projection::default(), sampling: Default::default() },
    ));
    slope.effects.push(effect(&mut f, EffectKind::Filter(Filter::SlopeBlur { radius: 24.0, scale: 3.0, seed: 1, divider: 1 })));
    let streaks = add(&mut f, 0, fill(&[(Channel::Metallic, procedural(ProceduralKind::Stripes, 0.0, 1.0))]), Some(slope));
    let top = add(&mut f, 0, LayerKind::Paint, None);
    paint_rect(&mut f, top, (30, 40), (150, 100), 0.4);
    let plain = reference(&f, 0);
    let warp = effect(&mut f, EffectKind::Filter(Filter::Warp { intensity: 30.0, scale: 4.0, seed: 3, divider: 1 }));
    f.project.layer_mut(base).unwrap().content_effects.push(warp);
    let line = effect(&mut f, EffectKind::Filter(Filter::DirectionalBlur { radius: 40.0, angle: 30.0 }));
    f.project.layer_mut(streaks).unwrap().content_effects.push(line);
    let want = reference(&f, 0);
    for c in [Channel::BaseColor, Channel::Roughness, Channel::Metallic] {
        assert!(want.channels[&c] != plain.channels[&c], "{c:?} changed by its filter");
    }
    let mut one_by_one = Composite::new(&f.project.texture_sets[0].channel_list(), RES);
    let (images, baked) = (Default::default(), Default::default());
    for t in all_tiles(RES, RES) {
        composite_tiles(&inputs(&f, 0, &images, &baked), &mut one_by_one, &[t]);
    }

    assert!(one_by_one == want, "per-tile composites differ from one pass");

    for hot in [top, streaks] {
        assert!(!check_cached(&f, 0, hot).is_empty());
    }
}

/// Composite every tile in chunks of `chunk`, telling the cache the `ahead` tiles that follow each chunk, as a pass spread over frames does.
fn run_sliced(fixture: &Fixture, cache: &mut BelowCache, chunk: usize, ahead: usize) -> Composite {
    let all = all_tiles(RES, RES);
    let (images, baked) = (Default::default(), Default::default());
    let mut out = Composite::new(&fixture.project.texture_sets[0].channel_list(), RES);

    for (index, tiles) in all.chunks(chunk).enumerate() {
        let done = (index * chunk + tiles.len()).min(all.len());
        cache.ahead = all.iter().skip(done).take(ahead).copied().collect();
        composite_tiles_cached(&inputs(fixture, 0, &images, &baked), &mut out, tiles, Some(&mut *cache));
        assert_eq!(cache.holds_fields(), !cache.ahead.is_empty(), "fields are kept exactly while tiles follow (chunk {chunk}, ahead {ahead})");
    }

    out
}

#[test]
fn filter_fields_kept_across_the_chunks_of_a_pass_composite_exactly() {
    let mut fixture = fixture();
    let top = heavy_stack(&mut fixture);
    let blurred_fill = fixture.project.texture_sets[0].stack[1];
    let expected = reference(&fixture, 0);

    for hot in [None, Some(top), Some(blurred_fill)] {
        // Exercise reuse within and recomputation beyond retained runs
        for (chunk, ahead) in [(1, 1), (1, 5), (3, 4), (5, 16), (2, 0)] {
            let mut cache = BelowCache::default();
            cache.set_hot(hot);
            assert!(run_sliced(&fixture, &mut cache, chunk, ahead) == expected, "a cold sliced pass differs (hot {hot:?}, chunk {chunk}, ahead {ahead})");

            // A warm hot-layer cache needs fewer fields
            assert!(run_sliced(&fixture, &mut cache, chunk, ahead) == expected, "a warm sliced pass differs (hot {hot:?}, chunk {chunk}, ahead {ahead})");
        }
    }

    // Evicted tiles need fields from every layer again
    let mut full = BelowCache::default();
    full.set_hot(Some(top));
    run(&fixture, 0, &all_tiles(RES, RES), Some(&mut full));
    let tile_bytes = full.tile_bytes().unwrap();
    for tiles_held in [2, 5, 9] {
        for (chunk, ahead) in [(1, 5), (3, 4), (2, 16)] {
            let mut cache = BelowCache::default();
            cache.budget = tiles_held * tile_bytes;
            cache.set_hot(Some(top));
            for pass in 0..3 {
                assert!(
                    run_sliced(&fixture, &mut cache, chunk, ahead) == expected,
                    "pass {pass} differs with {tiles_held} cached tiles (chunk {chunk}, ahead {ahead})"
                );
            }
        }
    }
}

#[test]
fn kept_filter_fields_are_not_used_after_an_edit_or_under_a_live_stroke() {
    let mut fixture = fixture();
    let top = heavy_stack(&mut fixture);
    let all = all_tiles(RES, RES);
    let (first, rest) = all.split_at(3);
    let mut cache = BelowCache::default();
    cache.set_hot(Some(top));

    // Keep fields for the remaining tiles
    cache.ahead = rest.to_vec();
    let mut out = run(&fixture, 0, first, Some(&mut cache));

    assert!(cache.holds_fields());

    // Changing the hot mask must invalidate the retained fields
    let mask = fixture.project.layer_mut(top).unwrap().mask.as_mut().unwrap();
    let blur = mask.effects.iter_mut().find(|effect| matches!(effect.kind, EffectKind::Filter(Filter::Blur { .. }))).unwrap();
    blur.kind = EffectKind::Filter(Filter::Blur { radius: 6.0, wrap: false });
    let (images, baked) = (Default::default(), Default::default());
    cache.ahead = Vec::new();
    composite_tiles_cached(&inputs(&fixture, 0, &images, &baked), &mut out, &all, Some(&mut cache));

    assert!(out == reference(&fixture, 0), "fields kept from before an edit were used after it");
    assert!(!cache.holds_fields());

    // A live stroke must bypass retained fields
    let preview = stroke(top);
    cache.ahead = rest.to_vec();
    let mut live = run_with(&fixture, 0, first, Some(&mut cache), Some(&preview));
    assert!(!cache.holds_fields());

    let mut inp = inputs(&fixture, 0, &images, &baked);
    inp.preview = Some(&preview);
    composite_tiles_cached(&inp, &mut live, rest, Some(&mut cache));

    assert!(live == run_with(&fixture, 0, &all, None, Some(&preview)), "a live stroke over a sliced pass differs");
}

/// Kept fields hold a layer off the hot path only over the tiles the cache lacked when they were built. A tile the cache drops
/// while they are in use needs that layer's fields again.
#[test]
fn tiles_the_cache_drops_during_a_kept_run_are_filtered_again() {
    const SIZE: u32 = 512;
    let mesh = primitives::sphere(1.0, 32, 16);
    let gbuf = UvGBuffer::new(&mesh, 0, SIZE, SIZE);
    let mut project = Project::new("t", "builtin:sphere");
    project.texture_sets.push(TextureSet::new("A", 0, SIZE));
    let mut fixture = Fixture { project, mesh, gbuf, pixels: PixelStore::default() };
    add(&mut fixture, 0, fill(&[(Channel::BaseColor, procedural(ProceduralKind::Clouds, 0.1, 0.9))]), None);

    // Pattern detail exposes any missing blur field
    let mut blurred = Mask::new(MaskBackground::Black);
    blurred.effects.push(effect(
        &mut fixture,
        EffectKind::Fill { source: procedural(ProceduralKind::Checker, 0.0, 1.0), projection: Projection::default(), sampling: Default::default() },
    ));
    blurred.effects.push(effect(&mut fixture, EffectKind::Filter(Filter::Blur { radius: 40.0, wrap: false })));
    add(&mut fixture, 0, fill(&[(Channel::Roughness, procedural(ProceduralKind::Grunge, 0.0, 1.0))]), Some(blurred));

    // Keep the blurred layer visible under the hot layer
    let hot = add(&mut fixture, 0, LayerKind::Paint, None);
    paint_rect_at(&mut fixture, hot, SIZE, (10, 10), (40, 40), 0.2);

    let mut all = all_tiles(SIZE, SIZE);
    all.sort_by_key(|&(x, y)| (y, x));
    let channels = fixture.project.texture_sets[0].channel_list();
    let (images, baked) = (Default::default(), Default::default());
    let mut expected = Composite::new(&channels, SIZE);
    composite_tiles(&inputs(&fixture, 0, &images, &baked), &mut expected, &all);

    // Prime the cache with only the two bottom rows
    let rows = (SIZE / TILE) as usize;
    let (above, bottom) = all.split_at(all.len() - 2 * rows);
    let mut cache = BelowCache::default();
    cache.set_hot(Some(hot));
    let mut out = Composite::new(&channels, SIZE);
    composite_tiles_cached(&inputs(&fixture, 0, &images, &baked), &mut out, bottom, Some(&mut cache));

    assert_eq!(cache.len(), bottom.len());

    cache.budget = cache.bytes();

    // Cache eviction during the run must rebuild missing lower-layer fields
    let mut out = Composite::new(&channels, SIZE);
    for (index, tile) in all.iter().enumerate() {
        cache.ahead = all.iter().skip(index + 1).copied().collect();
        composite_tiles_cached(&inputs(&fixture, 0, &images, &baked), &mut out, &[*tile], Some(&mut cache));
        if index == 0 {
            assert!(cache.holds_fields());
        }
    }

    assert!(above.len() > bottom.len());
    assert!(out == expected, "a tile dropped from the cache was composited with fields that lack the layers off the hot path");
}

/// A stroke that starts while fields are kept must not leave them allocated for as long as it lasts, even though tiles still follow.
#[test]
fn a_live_stroke_releases_the_fields_kept_before_it_started() {
    let mut fixture = fixture();
    let top = heavy_stack(&mut fixture);
    let all = all_tiles(RES, RES);
    let (first, rest) = all.split_at(3);
    let mut cache = BelowCache::default();
    cache.set_hot(Some(top));

    cache.ahead = rest.to_vec();
    run(&fixture, 0, first, Some(&mut cache));
    assert!(cache.holds_fields());

    // A stroke must release fields even while tiles remain
    let preview = stroke(top);
    let mut live = run_with(&fixture, 0, first, Some(&mut cache), Some(&preview));
    assert!(!cache.holds_fields(), "fields kept before the stroke stay allocated while it lasts");

    let (images, baked) = (Default::default(), Default::default());
    let mut inp = inputs(&fixture, 0, &images, &baked);
    inp.preview = Some(&preview);
    composite_tiles_cached(&inp, &mut live, rest, Some(&mut cache));
    assert!(live == run_with(&fixture, 0, &all, None, Some(&preview)), "a stroke that starts during a sliced pass differs");
}

#[test]
fn cached_stack_tracks_the_images_a_tile_generator_reads() {
    use std::sync::Arc;
    use texturecraft_compose::{ImageRes, ImageStore};

    let mut fixture = fixture();
    let hot_layer = heavy_stack(&mut fixture);
    let bottom_layer = fixture.project.texture_sets[0].stack[0];
    let LayerKind::Fill(fill) = &mut fixture.project.layer_mut(bottom_layer).unwrap().kind else { panic!("expected fill") };
    let mut pattern = Procedural::new(ProceduralKind::TileGenerator);
    pattern.tile.get_or_insert_with(Box::default).background_image = Some("plate".into());
    fill.sources.insert(Channel::Roughness, Source::Procedural { pattern, low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() });

    let image_with_value = |value| Arc::new(ImageRes::new(RES, RES, vec![[value; 4]; (RES * RES) as usize], false));
    let mut images = ImageStore::default();
    let baked = Default::default();
    let tiles = all_tiles(RES, RES);
    let mut cache = BelowCache::default();
    cache.set_hot(Some(hot_layer));
    let mut output = Composite::new(&fixture.project.texture_sets[0].channel_list(), RES);

    for (stage, background) in [("cold", None), ("added", Some(1.0)), ("replaced", Some(0.4))] {
        if let Some(value) = background {
            images.insert("plate".into(), image_with_value(value));
        }

        fixture.pixels.layer_mut(hot_layer, Channel::BaseColor, (RES, RES)).set(5, 5, &[stage.len() as u16, 0, 0, u16::MAX]);
        let input = inputs(&fixture, 0, &images, &baked);
        composite_tiles_cached(&input, &mut output, &tiles[..1], Some(&mut cache));

        assert!(!cache.is_empty(), "{stage}: first tile should be cached");

        let retained = cache.len() - 1;
        composite_tiles_cached(&input, &mut output, &tiles[1..], Some(&mut cache));

        assert_eq!(retained, 0, "{stage}: the layers below are drawn again with the new image");

        let mut expected = Composite::new(&fixture.project.texture_sets[0].channel_list(), RES);
        composite_tiles(&input, &mut expected, &tiles);

        assert!(output == expected, "{stage}");
    }
}

#[test]
fn cached_stack_tracks_referenced_images_only() {
    use std::sync::Arc;
    use texturecraft_compose::{ImageRes, ImageStore};

    let mut fixture = fixture();
    let hot_layer = heavy_stack(&mut fixture);
    let bottom_layer = fixture.project.texture_sets[0].stack[0];
    let LayerKind::Fill(fill) = &mut fixture.project.layer_mut(bottom_layer).unwrap().kind else { panic!("expected fill") };

    fill.sources.insert(Channel::Roughness, Source::Image { name: "used".into(), normal_format: Default::default() });
    let image_with_value = |value| Arc::new(ImageRes::new(RES, RES, vec![[value; 4]; (RES * RES) as usize], false));
    let mut images = ImageStore::default();
    let baked = Default::default();
    let tiles = all_tiles(RES, RES);
    let mut cache = BelowCache::default();
    cache.set_hot(Some(hot_layer));
    let mut output = Composite::new(&fixture.project.texture_sets[0].channel_list(), RES);
    let mut previous_image = None;

    for stage in ["cold", "resolve", "unused1", "unused2", "unused3", "replace", "remove"] {
        match stage {
            "resolve" => {
                let image = image_with_value(1.0);
                previous_image = Some(Arc::downgrade(&image));
                images.insert("used".into(), image);
            }
            "replace" => {
                let replacement = image_with_value(0.4);
                let replacement_weak = Arc::downgrade(&replacement);
                images.insert("used".into(), replacement);

                assert!(previous_image.as_ref().is_some_and(|old| old.upgrade().is_none()), "cached key must not retain replaced image pixels");

                previous_image = Some(replacement_weak);
            }
            "remove" => {
                images.remove("used");

                assert!(previous_image.as_ref().is_some_and(|old| old.upgrade().is_none()), "cached key must not retain removed image pixels");
            }
            "cold" => {}
            _ => {
                images.insert(stage.into(), image_with_value(0.5));
            }
        }

        fixture.pixels.layer_mut(hot_layer, Channel::BaseColor, (RES, RES)).set(5, 5, &[stage.len() as u16, 0, 0, u16::MAX]);
        let input = inputs(&fixture, 0, &images, &baked);
        composite_tiles_cached(&input, &mut output, &tiles[..1], Some(&mut cache));

        assert!(!cache.is_empty(), "{stage}: first tile should be cached");

        let retained = cache.len() - 1;
        composite_tiles_cached(&input, &mut output, &tiles[1..], Some(&mut cache));

        assert_eq!(retained, if stage.starts_with("unused") { tiles.len() - 1 } else { 0 }, "{stage}");

        let mut expected = Composite::new(&fixture.project.texture_sets[0].channel_list(), RES);
        composite_tiles(&input, &mut expected, &tiles);

        assert!(output == expected, "{stage}");
    }
}

#[test]
fn generator_input_image_changes_invalidate_stack_key() {
    use std::sync::Arc;
    use texturecraft_compose::{ImageRes, ImageStore, StackKey};

    let mut fixture = fixture();
    heavy_stack(&mut fixture);
    let generated_layer = fixture.project.texture_sets[0].stack[1];
    let mask = fixture.project.layer_mut(generated_layer).unwrap().mask.as_mut().unwrap();
    let EffectKind::Generator(generator) = &mut mask.effects[0].kind else { panic!("expected generator") };
    *generator = Generator::new(GeneratorKind::Dirt);
    generator.inputs.insert(BakedMap::AmbientOcclusion, Source::image("used"));

    let image = |value| Arc::new(ImageRes::new(RES, RES, vec![[value; 4]; (RES * RES) as usize], false));
    let mut images = ImageStore::default();
    images.insert("used".into(), image(0.1));
    let baked = Default::default();
    let roots = &fixture.project.texture_sets[0].stack;
    let key = |images: &ImageStore| StackKey::new(&fixture.project, &fixture.pixels, &baked, images, 0, roots).unwrap();
    let original = key(&images);

    images.insert("unrelated".into(), image(0.5));

    assert_eq!(key(&images), original, "an unrelated image leaves the key unchanged");

    let old_image = Arc::downgrade(&images["used"]);
    images.insert("used".into(), image(0.9));

    assert!(old_image.upgrade().is_none(), "the cache must not retain the old image pixels");
    assert!(key(&images) != original, "replacing a generator input must invalidate the cache");

    images.remove("used");

    assert!(key(&images) != original, "removing a generator input must invalidate the cache");
}

#[test]
fn compare_masks_reading_layers_fall_back_to_full_evaluation() {
    let mut f = fixture();
    let n = nested_stack(&mut f);
    let compare = effect(
        &mut f,
        EffectKind::Compare {
            op: CompareOp::GreaterThan,
            value: 0.5,
            tolerance: 0.0,
            left: CompareSource::LayersBelow,
            right: CompareSource::ThisLayer,
            channel: Channel::Roughness,
            hardness: 1.0,
        },
    );

    // A mask above the hot layer compares the stack below it
    f.project.layer_mut(n.g2_above).unwrap().mask.as_mut().unwrap().effects.push(compare);

    for hot in [n.deep, n.top] {
        assert!(check_cached(&f, 0, hot).is_empty(), "compare masks read the stack: nothing cached");
    }
}

/// The key of set 0's whole stack over `images`
fn stack_key(fixture: &Fixture, images: &texturecraft_compose::ImageStore) -> texturecraft_compose::StackKey {
    let roots = &fixture.project.texture_sets[0].stack;
    texturecraft_compose::StackKey::new(&fixture.project, &fixture.pixels, &Default::default(), images, 0, roots).unwrap()
}

/// A flat image resource of `value`
fn flat_image(value: f32) -> std::sync::Arc<texturecraft_compose::ImageRes> {
    std::sync::Arc::new(texturecraft_compose::ImageRes::new(RES, RES, vec![[value; 4]; (RES * RES) as usize], false))
}

#[test]
fn filter_input_and_id_map_image_changes_invalidate_stack_key() {
    let mut fixture = fixture();
    heavy_stack(&mut fixture);
    let bottom_layer = fixture.project.texture_sets[0].stack[0];
    let conversion = GrayscaleConversion { custom_input: true, ..GrayscaleConversion::LIBRARY };
    let mut filter = effect(&mut fixture, EffectKind::Filter(Filter::GrayscaleConversion(conversion)));
    filter.inputs.insert(ImageInput::CUSTOM_INPUT.id.into(), Source::image("input"));
    let mut selection = ColorSelection::new(vec![[1.0, 0.0, 0.0]], 0.1);
    selection.id_map = Some("id".into());
    let selection = effect(&mut fixture, EffectKind::ColorSelection(selection));
    let layer = fixture.project.layer_mut(bottom_layer).unwrap();
    layer.content_effects.push(filter);
    layer.mask = Some(Mask::new(MaskBackground::White));
    layer.mask.as_mut().unwrap().effects.push(selection);
    let mut images = texturecraft_compose::ImageStore::default();
    images.insert("input".into(), flat_image(0.1));
    images.insert("id".into(), flat_image(0.1));

    for name in ["input", "id"] {
        let original = stack_key(&fixture, &images);
        images.insert(name.into(), flat_image(0.9));

        assert!(stack_key(&fixture, &images) != original, "replacing the {name} image must invalidate the cache");
    }
}

#[test]
fn the_environment_keys_the_stack_only_while_environment_lighting_reads_it() {
    let mut fixture = fixture();
    heavy_stack(&mut fixture);
    let bottom_layer = fixture.project.texture_sets[0].stack[0];
    let mut images = texturecraft_compose::ImageStore::default();
    images.insert("sky".into(), flat_image(0.5));
    fixture.project.display.environment = format!("{ENV_RESOURCE_PREFIX}sky");
    let unlit = stack_key(&fixture, &images);

    fixture.project.display.env_rotation += 90.0;

    assert_eq!(stack_key(&fixture, &images), unlit, "without Baked Lighting Environment the turn changes nothing");

    let lighting =
        Filter::EnvironmentLighting { horizontal: 0.0, vertical: 0.0, exposure: 0.0, height_intensity: 0.0, ambient_occlusion: 0.0, specular_occlusion: 0.0 };
    let lighting = effect(&mut fixture, EffectKind::Filter(lighting));
    fixture.project.layer_mut(bottom_layer).unwrap().content_effects.push(lighting);
    let lit = stack_key(&fixture, &images);
    fixture.project.display.env_rotation += 90.0;
    let turned = stack_key(&fixture, &images);
    assert!(turned != lit, "turning the environment invalidates the cache");
    assert_eq!(lit.changed_tiles(&turned), None, "and recomposites every tile");

    images.insert("sky".into(), flat_image(0.9));

    assert!(stack_key(&fixture, &images) != turned, "replacing the environment image invalidates the cache");
}

#[test]
fn painting_metallic_updates_validation_on_a_layer_above_it() {
    let mut fixture = fixture();
    let metal = add(&mut fixture, 0, LayerKind::Paint, None);
    let color = add(&mut fixture, 0, fill(&[(Channel::BaseColor, Source::Uniform([0.12; 3]))]), None);
    let color_layer = fixture.project.layer_mut(color).unwrap();
    color_layer.channels.retain(|channel, _| *channel == Channel::BaseColor);
    let validation = effect(&mut fixture, EffectKind::Filter(Filter::PbrValidate(texturecraft_core::PbrValidate::LIBRARY)));
    fixture.project.layer_mut(color).unwrap().content_effects.push(validation);

    let mut cache = BelowCache::default();
    cache.set_hot(Some(metal));
    let coordinate = RES / 2;
    let tiles = [(coordinate / TILE, coordinate / TILE)];
    let before = run(&fixture, 0, &tiles, Some(&mut cache)).value(Channel::BaseColor, coordinate, coordinate);

    fixture.pixels.layer_mut(metal, Channel::Metallic, (RES, RES)).set(coordinate, coordinate, &[u16::MAX; 2]);
    let fresh = run(&fixture, 0, &tiles, None).value(Channel::BaseColor, coordinate, coordinate);
    let updated = run(&fixture, 0, &tiles, Some(&mut cache)).value(Channel::BaseColor, coordinate, coordinate);

    assert_ne!(fresh, before, "the same color is valid for a dielectric and too dark for a metal");
    assert_eq!(updated, fresh, "replaying the stack must reevaluate material-dependent validation");
}

/// A folder of set `ts` holding a Clouds fill whose content or mask is blurred by 8; returns the folder and the fill
fn folder_of_blurred_clouds(f: &mut Fixture, ts: usize, folder: Option<LayerId>, blur_on: BlurOn) -> (LayerId, LayerId) {
    let held = put(f, ts, folder, LayerKind::Group { children: vec![], collapsed: false, contents: Default::default(), randomized: Vec::new() }, None);
    let clouds = put(f, ts, Some(held), fill(&[(Channel::BaseColor, procedural(ProceduralKind::Clouds, 0.1, 0.9))]), None);
    let blur = effect(f, EffectKind::Filter(Filter::Blur { radius: 8.0, wrap: false }));
    match blur_on {
        BlurOn::Content => f.project.layer_mut(clouds).unwrap().content_effects.push(blur),
        BlurOn::Mask => {
            let mut mask = Mask::new(MaskBackground::Black);
            let checker = procedural(ProceduralKind::Checker, 0.0, 1.0);
            mask.effects.push(effect(f, EffectKind::Fill { source: checker, projection: Projection::default(), sampling: Default::default() }));
            mask.effects.push(blur);
            f.project.layer_mut(clouds).unwrap().mask = Some(mask);
        }
    }
    (held, clouds)
}

/// A top-level folder of set 0 laying Scale 2 and Rotation 25 on the fills inside it
fn offset_folder(f: &mut Fixture) -> LayerId {
    let contents = ContentsOffset::new(2.0, 25.0);
    put(f, 0, None, LayerKind::Group { children: vec![], collapsed: false, contents, randomized: Vec::new() }, None)
}

#[test]
fn a_neighbourhood_filter_in_an_instanced_folder_under_an_offset_folder_reads_its_content_in_the_instances_space() {
    let mut mismatches = Vec::new();
    for blur_on in [BlurOn::Content, BlurOn::Mask] {
        let mut instanced = fixture();
        let (source, _) = folder_of_blurred_clouds(&mut instanced, 1, None, blur_on);
        let outer = offset_folder(&mut instanced);
        put(&mut instanced, 0, Some(outer), LayerKind::Instance { source }, None);

        let mut copied = fixture();
        let outer = offset_folder(&mut copied);
        folder_of_blurred_clouds(&mut copied, 0, Some(outer), blur_on);

        let (instanced, copied) = (reference(&instanced, 0), reference(&copied, 0));
        let texels = (0..RES).flat_map(|y| (0..RES).map(move |x| (x, y)));
        let differences: Vec<f32> = texels
            .map(|(x, y)| {
                let (a, b) = (instanced.value(Channel::BaseColor, x, y), copied.value(Channel::BaseColor, x, y));
                a.iter().zip(&b).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max)
            })
            .filter(|difference| *difference > 1e-5)
            .collect();
        let largest = differences.iter().copied().fold(0.0, f32::max);

        if !differences.is_empty() {
            mismatches.push(format!("{blur_on:?} blur: the instance differs from the copy by up to {largest} on {} texels", differences.len()));
        }
    }

    assert!(mismatches.is_empty(), "{}", mismatches.join("; "));
}

#[test]
fn a_hot_layer_with_a_neighbourhood_filter_in_an_instanced_folder_under_an_offset_folder_composites_exactly_cached() {
    for blur_on in [BlurOn::Content, BlurOn::Mask] {
        let mut instanced = fixture();
        let (source, clouds) = folder_of_blurred_clouds(&mut instanced, 1, None, blur_on);
        let outer = offset_folder(&mut instanced);
        put(&mut instanced, 0, Some(outer), LayerKind::Instance { source }, None);

        let cache = check_cached(&instanced, 0, clouds);

        assert!(!cache.is_empty(), "{blur_on:?} blur: the hot layer is cached through its instance");
    }
}

fn cache_after_changing_mesh(owned: bool) {
    use std::sync::Arc;
    use texturecraft_compose::{ProjectorDepthCache, composite_tiles_with_projectors};

    let mut fixture = fixture();
    let lower = add(&mut fixture, 0, fill(&[(Channel::BaseColor, Source::Uniform([1.0, 0.0, 0.0]))]), None);
    fixture.project.layer_mut(lower).unwrap().geometry_mask = Some(vec!["Lid".into()]);
    let hot = add(&mut fixture, 0, LayerKind::Paint, None);
    let mut named = fixture.mesh.clone();
    named.submeshes[0].name = "Lid".into();
    let first = Arc::new(named);
    let second = Arc::new(fixture.mesh.clone());
    let images = Default::default();
    let baked = Default::default();
    let mut projectors = ProjectorDepthCache::default();
    let mut cache = BelowCache::default();
    cache.set_hot(Some(hot));
    let mut render = |mesh: &Arc<Mesh>, cache: Option<&mut BelowCache>| {
        let mut input = inputs(&fixture, 0, &images, &baked);
        input.mesh = mesh;
        let mut output = Composite::new(&fixture.project.texture_sets[0].channel_list(), RES);
        if owned {
            composite_tiles_with_projectors(&input, mesh, &mut projectors, &mut output, &[(1, 1)], cache);
        } else {
            composite_tiles_cached(&input, &mut output, &[(1, 1)], cache);
        }
        output
    };

    let before = render(&first, Some(&mut cache));
    assert!(!cache.is_empty());
    let expected = render(&second, None);
    let actual = render(&second, Some(&mut cache));

    assert!(before != expected, "the lower fill applies only to the named part");
    assert!(actual == expected, "the previous mesh's cached fill was reused");
}

#[test]
fn a_borrowed_mesh_change_invalidates_cached_fills() {
    cache_after_changing_mesh(false);
}

#[test]
fn a_projector_rebind_without_below_cache_invalidates_cached_fills() {
    cache_after_changing_mesh(true);
}

fn roughness_compare(fixture: &mut Fixture, source: CompareSource) -> Effect {
    effect(
        fixture,
        EffectKind::Compare {
            op: CompareOp::GreaterThan,
            value: 0.7,
            tolerance: 0.0,
            left: source,
            right: CompareSource::Constant,
            channel: Channel::Roughness,
            hardness: 1.0,
        },
    )
}

#[test]
fn compare_uses_the_stack_below_each_draw_of_an_instanced_folder() {
    let mut fixture = fixture();
    let base = |roughness| fill(&[(Channel::BaseColor, Source::Uniform([0.0, 0.0, 1.0])), (Channel::Roughness, Source::Uniform([roughness; 3]))]);
    add(&mut fixture, 0, base(0.2), None);
    let folder = add(&mut fixture, 0, LayerKind::folder(), None);
    let mut mask = Mask::new(MaskBackground::Black);
    mask.effects.push(roughness_compare(&mut fixture, CompareSource::LayersBelow));
    mask.effects.push(effect(&mut fixture, EffectKind::Filter(Filter::Blur { radius: 32.0, wrap: false })));
    let red = fill(&[(Channel::BaseColor, Source::Uniform([1.0, 0.0, 0.0]))]);
    put(&mut fixture, 0, Some(folder), red, Some(mask));
    add(&mut fixture, 0, base(0.9), None);
    add(&mut fixture, 0, LayerKind::Instance { source: folder }, None);

    let color = run(&fixture, 0, &[(1, 1)], None).value(Channel::BaseColor, 96, 96);

    assert!(color[0] > color[2], "the second draw should read the rougher surface: {color:?}");
}

#[test]
fn compare_reads_a_retained_channel_after_it_is_removed_from_the_outputs() {
    let mut fixture = fixture();
    let roughness = fill(&[(Channel::Roughness, Source::Uniform([0.9; 3]))]);
    add(&mut fixture, 0, roughness, None);
    let mut mask = Mask::new(MaskBackground::Black);
    mask.effects.push(roughness_compare(&mut fixture, CompareSource::LayersBelow));
    let red = fill(&[(Channel::BaseColor, Source::Uniform([1.0, 0.0, 0.0]))]);
    add(&mut fixture, 0, red, Some(mask));
    let before = run(&fixture, 0, &[(1, 1)], None).value(Channel::BaseColor, 96, 96);

    fixture.project.texture_sets[0].channels.retain(|(channel, _)| *channel != Channel::Roughness);
    let after = run(&fixture, 0, &[(1, 1)], None);

    assert!(before[0] > before[1]);
    assert_eq!(after.value(Channel::BaseColor, 96, 96), before);
    assert!(!after.channels.contains_key(&Channel::Roughness));
}

#[test]
fn sparse_compare_mask_reads_filtered_content_through_the_full_blur_radius() {
    use texturecraft_compose::{mask_texels, mask_values};

    const SIZE: u32 = 512;
    let mut fixture = fixture();
    fixture.mesh = primitives::plane(2.0, 2);
    fixture.gbuf = UvGBuffer::new(&fixture.mesh, 0, SIZE, SIZE);
    fixture.project.texture_sets[0] = TextureSet::new("Plane", 0, SIZE);
    let mut mask = Mask::new(MaskBackground::Black);
    mask.effects.push(roughness_compare(&mut fixture, CompareSource::ThisLayer));
    mask.effects.push(effect(&mut fixture, EffectKind::Filter(Filter::Blur { radius: 256.0, wrap: false })));
    let roughness = fill(&[(Channel::Roughness, Source::Uniform([0.9; 3]))]);
    let layer = add(&mut fixture, 0, roughness, Some(mask));
    let edge = effect(&mut fixture, EffectKind::Filter(Filter::EdgeDetect { width: 8.0 }));
    fixture.project.layer_mut(layer).unwrap().content_effects.push(edge);
    let (images, baked) = (Default::default(), Default::default());
    let center = SIZE / 2;

    for (wrap, x) in [(false, center), (true, 0)] {
        let EffectKind::Filter(Filter::Blur { wrap: enabled, .. }) =
            &mut fixture.project.layer_mut(layer).unwrap().mask.as_mut().unwrap().effects.last_mut().unwrap().kind
        else {
            panic!("expected blur")
        };

        *enabled = wrap;
        let inputs = inputs(&fixture, 0, &images, &baked);

        let full = mask_texels(&inputs, layer).unwrap();
        let tile = mask_values(&inputs, layer, &[(x / TILE, center / TILE)]).unwrap();

        assert_eq!(tile.get(x, center)[0], to_u16(full[(center * SIZE + x) as usize]), "wrap={wrap}");
    }
}

#[test]
fn a_folder_thumbnail_reads_compare_inputs_from_its_backdrop() {
    use texturecraft_compose::{ThumbTarget, thumb};

    let mut fixture = fixture();
    let backdrop = fill(&[(Channel::Roughness, Source::Uniform([0.9; 3])), (Channel::BaseColor, Source::Uniform([0.0, 0.0, 1.0]))]);
    add(&mut fixture, 0, backdrop, None);
    let folder = add(&mut fixture, 0, LayerKind::folder(), None);
    let mut mask = Mask::new(MaskBackground::Black);
    mask.effects.push(roughness_compare(&mut fixture, CompareSource::LayersBelow));
    put(&mut fixture, 0, Some(folder), fill(&[(Channel::BaseColor, Source::Uniform([1.0, 0.0, 0.0]))]), Some(mask));
    let (images, baked) = (Default::default(), Default::default());
    let inputs = inputs(&fixture, 0, &images, &baked);

    let thumbnail = thumb::render(&inputs, ThumbTarget::Content { layer: folder, channel: Channel::BaseColor }).unwrap();
    let center = 4 * (96 * RES + 96) as usize;

    assert!(thumbnail.rgba[center] > thumbnail.rgba[center + 2], "visible red child");
}

#[test]
fn replacing_a_generator_texture_image_invalidates_the_cached_stack() {
    use std::sync::Arc;
    use texturecraft_compose::{ImageRes, ImageStore};
    use texturecraft_core::{TextureBlend, TextureLayer};

    let mut fixture = fixture();
    let hot_layer = heavy_stack(&mut fixture);
    let texture = TextureLayer { enabled: true, source: Some(Source::image("grunge")), opacity: 1.0, blend: TextureBlend::Normal, ..TextureLayer::default() };
    let mut mask = Mask::new(MaskBackground::Black);
    mask.effects
        .push(effect(&mut fixture, EffectKind::Generator(Generator { texture: Some(Box::new(texture)), ..Generator::new(GeneratorKind::AmbientOcclusion) })));
    let textured = add(&mut fixture, 0, fill(&[(Channel::Roughness, procedural(ProceduralKind::Clouds, 0.0, 1.0))]), Some(mask));
    let stack = &mut fixture.project.texture_sets[0].stack;
    stack.retain(|id| *id != textured);
    stack.insert(0, textured);

    let image_with_value = |value| Arc::new(ImageRes::new(4, 4, vec![[value, value, value, 1.0]; 16], false));
    let mut images = ImageStore::default();
    let baked = Default::default();
    let tiles = all_tiles(RES, RES);
    let mut cache = BelowCache::default();
    cache.set_hot(Some(hot_layer));
    let mut output = Composite::new(&fixture.project.texture_sets[0].channel_list(), RES);

    for (stage, value) in [("dark", 0.1), ("light", 0.9)] {
        images.insert("grunge".into(), image_with_value(value));
        let input = inputs(&fixture, 0, &images, &baked);
        composite_tiles_cached(&input, &mut output, &tiles[..1], Some(&mut cache));
        let retained = cache.len() - 1;
        composite_tiles_cached(&input, &mut output, &tiles[1..], Some(&mut cache));
        assert_eq!(retained, 0, "{stage}: a new texture image drops the tiles cached below the hot layer");

        let mut expected = Composite::new(&fixture.project.texture_sets[0].channel_list(), RES);
        composite_tiles(&input, &mut expected, &tiles);
        assert!(output == expected, "{stage}");
    }
}
