// UV fill compositor. Noise and color conversions follow texel/src/{noise,tile}.rs.
struct Operation {
    ids: vec4<u32>,
    low: vec4<f32>,
    high: vec4<f32>,
    projection: vec4<f32>,
    uv: vec4<f32>,
    balance: vec4<f32>,
    effects: vec4<u32>,
    procedural: vec4<u32>,
}

struct Params {
    // Width, row count, top row, reserved texels per map.
    band: vec4<u32>,
    // Source width, source height, display sampling step, height map enabled.
    source: vec4<u32>,
    program: vec4<u32>,
    coverage: vec4<u32>,
    paint_window: vec4<u32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> layers: array<Operation>;
@group(0) @binding(2) var<storage, read> srgb: array<u32>;
@group(0) @binding(3) var<storage, read_write> output: array<u32>;
@group(0) @binding(4) var<storage, read> paint: array<u32>;
@group(0) @binding(5) var<storage, read> uv_coverage: array<u32>;
@group(0) @binding(6) var surface_uv: texture_2d<u32>;
struct SurfaceVertex {
    position: vec4<f32>,
    normal: vec4<f32>,
}
@group(0) @binding(7) var<storage, read> surface_vertices: array<SurfaceVertex>;
@group(0) @binding(8) var bake_maps: texture_2d_array<f32>;
@group(0) @binding(9) var brush_alpha: texture_2d<u32>;

const NO_PAINT_IMAGE = 0xffffffffu;

struct Surface {
    position: vec3<f32>,
    normal: vec3<f32>,
}

fn edge(a: vec2<f32>, b: vec2<f32>, p: vec2<f32>) -> f32 {
    let ab = b - a;
    let ap = p - a;
    return ab.x * ap.y - ab.y * ap.x;
}

fn surface_at(pixel: vec2<u32>) -> Surface {
    if (params.coverage.w & 1u) == 0u || any(pixel >= params.source.xy) {
        return Surface(vec3(0.0), vec3(0.0, 1.0, 0.0));
    }
    let hit = textureLoad(surface_uv, vec2<i32>(pixel), 0).xy;
    if hit.x == 0xffffffffu {
        return Surface(vec3(0.0), vec3(0.0, 1.0, 0.0));
    }
    let a = surface_vertices[hit.x * 3u];
    let b = surface_vertices[hit.x * 3u + 1u];
    let c = surface_vertices[hit.x * 3u + 2u];
    // Reconstruct the working position from the original UVs. Packed barycentrics can
    // cross a hard procedural cell boundary at high frequencies.
    let size = vec2<f32>(params.source.xy);
    let uv_a = vec2(a.position.w, a.normal.w) * size;
    let uv_b = vec2(b.position.w, b.normal.w) * size;
    let uv_c = vec2(c.position.w, c.normal.w) * size;
    let center = vec2<f32>(pixel) + 0.5;
    let area = edge(uv_a, uv_b, uv_c);
    var weight = unpack2x16unorm(hit.y);
    if abs(area) >= 1e-12 {
        weight = vec2(edge(uv_c, uv_a, center), edge(uv_a, uv_b, center)) * (1.0 / area);
    }
    let w = 1.0 - weight.x - weight.y;
    let position = a.position.xyz * w + b.position.xyz * weight.x + c.position.xyz * weight.y;
    let normal = a.normal.xyz * w + b.normal.xyz * weight.x + c.normal.xyz * weight.y;
    return Surface(position, select(normal / max(length(normal), 1e-6), vec3(0.0, 1.0, 0.0), dot(normal, normal) <= 1e-12));
}

fn baked_at(pixel: vec2<u32>, layer: i32) -> vec4<f32> {
    let size = textureDimensions(bake_maps);
    let uv = (vec2<f32>(pixel) + 0.5) / vec2<f32>(params.source.xy);
    let sample_pixel = min(vec2<u32>(uv * vec2<f32>(size)), size - 1u);
    return textureLoad(bake_maps, vec2<i32>(sample_pixel), layer, 0);
}

// The nearest squared distance can be clipped to one when no feature ID is needed.
fn cellular(p: vec3<f32>, seed: u32, details: bool) -> vec3<f32> {
    let cell = vec3<i32>(floor(p));
    let local = fract(p);
    var nearest = select(1.0, 4.0, details);
    var second = 1.0;
    var feature_id = 0u;
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let shift = vec3(x, y, z);
                let bits = hash(cell + shift, seed * 0xc2b2ae35u);
                let feature = vec3<f32>(bits) / 4294967296.0;
                let delta = vec3<f32>(shift) + feature - local;
                let distance = dot(delta, delta);
                if details {
                    if distance < nearest {
                        second = nearest;
                        nearest = distance;
                        feature_id = bits.x ^ bits.y;
                    } else {
                        second = min(second, distance);
                    }
                } else {
                    nearest = min(nearest, distance);
                }
            }
        }
    }
    return vec3(sqrt(min(1.0, nearest)), sqrt(min(1.0, second)), f32(feature_id) / 4294967296.0);
}

fn cells(p: vec3<f32>, seed: u32) -> f32 {
    return cellular(p, seed, false).x;
}

fn cell_value(cell: vec3<i32>, seed: u32) -> f32 {
    return f32(hash(cell, seed * 0x9e3779b9u).x) / 4294967296.0;
}

fn drip(p: vec3<f32>, x: f32, salt: i32, seed: u32) -> f32 {
    let cx = x * 6.0;
    let column = i32(floor(cx));
    let width = 0.08 + 0.25 * cell_value(vec3(column, 0, salt), seed);
    let across = 1.0 - smoothstep(width * 0.3, width, abs(fract(cx) - 0.5 - (cell_value(vec3(column, 1, salt), seed) - 0.5) * 0.5));
    let yy = p.y * (0.35 + 0.4 * cell_value(vec3(column, 2, salt), seed)) + cell_value(vec3(column, 3, salt), seed) * 7.0;
    let t = fract(yy);
    let on = cell_value(vec3(column, i32(floor(yy)), 3 + salt), seed) > 0.5;
    return select(0.0, across * pow(t, 1.6) * (1.0 - smoothstep(0.9, 1.0, t)), on);
}

fn woven(p: vec3<f32>, seed: u32, twill: bool) -> f32 {
    let point = p.xy * 2.0;
    let cell = vec2<i32>(floor(point));
    let local = fract(point);
    let period = select(2, 4, twill);
    let warp_on_top = ((cell.x + cell.y) & (period - 1)) < period / 2;
    let thread_uv = select(local, local.yx, warp_on_top);
    let pi = 3.141592653589793;
    let thread = sqrt(max(0.0, sin(pi * thread_uv.x))) * (0.8 + 0.2 * sin(pi * thread_uv.y));
    let tone = select(1.0, 0.55, twill && !warp_on_top);
    let fiber = noise(p * vec3(40.0, 40.0, 1.0), seed, false);
    return clamp(thread * tone * (0.88 + 0.12 * fiber), 0.0, 1.0);
}

fn pattern(kind: u32, p: vec3<f32>, seed: u32) -> f32 {
    switch kind {
        case 1u: { return fbm(p, 6u, seed, false); }
        case 2u: { return fbm(p, 4u, seed, true); }
        case 3u: { return noise(p * 16.0, seed, false); }
        case 4u: { return cells(p, seed); }
        case 5u: {
            let n = fbm(p, 6u, seed, true) * 0.7 + fbm(p * 3.7, 3u, seed ^ 0x55u, false) * 0.3;
            return clamp((n - 0.45) * 4.0 + 0.5, 0.0, 1.0);
        }
        case 6u: {
            let n = noise(p * vec3(0.15, 6.0, 6.0), seed, true);
            let line = 1.0 - min(abs(n - 0.5) * 40.0, 1.0);
            let gate = noise(p * 0.8, seed ^ 0x99u, false);
            return line * clamp((gate - 0.45) * 4.0, 0.0, 1.0);
        }
        case 7u: {
            let warp = fbm(p * vec3(0.18, 0.6, 1.0), 3u, seed, true);
            let t = fract((p.y + (warp - 0.5) * 0.9) * 5.0);
            let ring = 1.0 - smoothstep(0.45, 1.0, t);
            let fiber = noise(p * vec3(2.0, 40.0, 1.0), seed ^ 0x3cu, false);
            let tone = fbm(p * vec3(0.6, 3.0, 1.0), 2u, seed ^ 0x71u, false);
            return clamp(ring * 0.6 + fiber * 0.25 + tone * 0.15, 0.0, 1.0);
        }
        case 8u: {
            let turb = fbm(p, 5u, seed, true);
            let veins = pow(abs(sin((p.x + p.y * 0.5) * 2.5 + turb * 9.0)), 0.3);
            let cloud = fbm(p * 2.0, 3u, seed ^ 0x19u, false);
            return clamp(veins * 0.85 + cloud * 0.15, 0.0, 1.0);
        }
        case 11u: {
            let first = cells(p, seed);
            let second = cells(p * 2.7 + 13.0, seed ^ 0x6bu);
            return min(smoothstep(0.08, 0.16, first), smoothstep(0.06, 0.12, second));
        }
        case 12u: {
            let first = cells(p * 2.0, seed ^ 0x45u);
            return smoothstep(0.55, 0.58, fbm(p, 5u, seed, true) + (first - 0.5) * 0.25);
        }
        case 13u: {
            let fine = noise(p * vec3(37.0, 1.3, 1.0), seed ^ 0x2bu, false);
            return clamp(max(drip(p, p.x, 0, seed), drip(p, p.x + 0.37, 1, seed) * 0.8) * (0.6 + 0.4 * fine), 0.0, 1.0);
        }
        case 48u: {
            let cell = vec3<i32>(floor(p));
            return select(0.0, 1.0, ((cell.x + cell.y + cell.z) & 1) == 0);
        }
        case 49u: { return select(0.0, 1.0, fract(p.x) < 0.5); }
        case 50u: {
            let row = i32(floor(p.y * 2.0));
            let x = p.x + select(0.0, 0.5, (row & 1) == 1);
            let local = fract(vec2(x, p.y * 2.0));
            let d = min(min(local.x, 1.0 - local.x), min(local.y, 1.0 - local.y) * 0.5);
            let tone = cell_value(vec3(i32(floor(x)), row, 0), seed) * 0.6 + noise(p * 9.0, seed, false) * 0.4;
            return smoothstep(0.02, 0.045, d) * (0.85 + 0.15 * tone);
        }
        case 51u: {
            let cell = vec2<i32>(floor(p.xy));
            let local = fract(p.xy);
            let d = min(min(local.x, 1.0 - local.x), min(local.y, 1.0 - local.y));
            return smoothstep(0.015, 0.035, d) * (0.9 + 0.1 * cell_value(vec3(cell, 0), seed));
        }
        case 52u: { return woven(p, seed, false); }
        case 53u: { return woven(p, seed, true); }
        case 54u: {
            let w = noise(p * 0.7, seed ^ 0x2du, true) - 0.5;
            let features = cellular(p + vec3(w * 0.4, -w * 0.4, 0.0), seed, true);
            let crease = smoothstep(0.0, 0.16, features.y - features.x);
            let pebble = 1.0 - features.x * 0.5;
            let fine = noise(p * 7.0, seed ^ 0x13u, false);
            return clamp(crease * pebble * (0.85 + 0.15 * fine), 0.0, 1.0);
        }
        case 55u: {
            let first = min(cells(p, seed) * 1.25, 1.0);
            let second = min(cells(p * 2.3 + 5.1, seed ^ 0x5au) * 1.25, 1.0);
            return clamp(first * first * 0.75 + second * second * 0.25, 0.0, 1.0);
        }
        case 56u: {
            let a = noise(p * vec3(1.5, 60.0, 1.0), seed, false);
            let b = noise(p * vec3(0.4, 18.0, 1.0), seed ^ 0x77u, false);
            return clamp(a * 0.6 + b * 0.4, 0.0, 1.0);
        }
        case 57u: { return cellular(p, seed, true).z; }
        case 58u: {
            let w = noise(p * 1.3, seed ^ 0x61u, true) - 0.5;
            let q = p * vec3(1.15, 0.85, 1.0) + vec3(w * 0.3, w * 0.3, 0.0);
            let features = cellular(q, seed, true);
            let tone = features.z;
            let print = select(0.0, 1.0 - smoothstep(0.18 + 0.1 * tone, 0.3 + 0.12 * tone, features.x), tone > 0.4);
            let ridges = 0.5 + 0.5 * sin(features.x * 190.0 + w * 12.0);
            let smudge = clamp((fbm(p * 1.7, 4u, seed ^ 0x33u, false) - 0.55) * 3.0, 0.0, 1.0);
            return clamp(print * ridges * 0.9 + smudge * 0.4, 0.0, 1.0);
        }
        default: { return fract(p.x); }
    }
}

fn raw_procedural(layer: Operation, p: vec3<f32>) -> f32 {
    return pattern(layer.ids.y, p, layer.ids.z);
}

fn sample_procedural(layer: Operation, p: vec3<f32>) -> f32 {
    if layer.procedural.x == 0u {
        return raw_procedural(layer, p);
    }
    let cells = f32(layer.procedural.x) / layer.uv.z;
    let grid = p.xy * cells - 0.5;
    let base = floor(grid) + 0.5;
    let t = fract(grid);
    let a = raw_procedural(layer, vec3(base / cells, p.z));
    let b = raw_procedural(layer, vec3((base + vec2(1.0, 0.0)) / cells, p.z));
    let c = raw_procedural(layer, vec3((base + vec2(0.0, 1.0)) / cells, p.z));
    let d = raw_procedural(layer, vec3((base + vec2(1.0)) / cells, p.z));
    return mix(mix(a, b, t.x), mix(c, d, t.x), t.y);
}

fn paint_ramp_color(word: u32) -> vec3<f32> {
    return bitcast<vec3<f32>>(vec3(srgb[word], srgb[word + 1u], srgb[word + 2u]));
}

fn azimuth(y: f32, x: f32) -> f32 {
    if x == 0.0 && y == 0.0 { return 0.5; }
    return atan2(y, x) / 6.283185307 + 0.5;
}

fn value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec3<f32> {
    if layer.ids.y == 0u {
        return layer.low.rgb;
    }
    let mode = layer.procedural.w;
    let p = surface.position * layer.projection.w + layer.projection.xyz;
    var n = 0.0;
    if mode == 1u {
        let weighted = pow(abs(surface.normal), vec3(layer.balance.w));
        let weights = weighted / max(weighted.x + weighted.y + weighted.z, 1e-6);
        let minimum = 1.0 / (131072.0 * max(layer.uv.w, 1.0));
        let retained = select(vec3(0.0), weights, weights >= vec3(minimum));
        let w = select(retained, weights, all(retained == vec3(0.0)));
        if w.x > 0.0 { n += w.x * sample_procedural(layer, vec3(p.zy, 0.0)); }
        if w.y > 0.0 { n += w.y * sample_procedural(layer, vec3(p.xz, 0.0)); }
        if w.z > 0.0 { n += w.z * sample_procedural(layer, vec3(p.xy, 0.0)); }
    } else {
        var q = vec2(0.0);
        switch mode {
            case 2u: { q = p.zy; }
            case 3u: { q = p.xz; }
            case 4u: { q = p.xy; }
            case 5u: {
                var d = vec3(0.0, 1.0, 0.0);
                if dot(surface.position, surface.position) > 0.0 { d = normalize(surface.position); }
                q = vec2(azimuth(d.z, d.x) * 4.0, acos(clamp(d.y, -1.0, 1.0)) * (2.0 / 3.141592654)) * layer.projection.w;
            }
            case 6u, 7u, 8u: {
                let pos = surface.position;
                let axis = select(select(pos.xyz, pos.yxz, mode == 6u), pos.xzy, mode == 8u);
                q = vec2(azimuth(axis.z, axis.x) * 4.0, axis.y) * layer.projection.w;
            }
            default: {
                let rotation = layer.projection.xy;
                let rotated = vec2(uv.x * rotation.x - uv.y * rotation.y, uv.x * rotation.y + uv.y * rotation.x);
                q = (rotated * layer.projection.zw + layer.uv.xy) * layer.uv.z;
            }
        }
        n = sample_procedural(layer, vec3(q, 0.0));
    }
    let shaped = clamp((n - 0.5) * layer.uv.w + 0.5 + layer.balance.x, 0.0, 1.0);
    let t = select(shaped, 1.0 - shaped, layer.ids.w != 0u);
    if layer.procedural.z > 0u {
        let last = layer.procedural.z - 1u;
        let pos = t * f32(last);
        let index = min(u32(pos), last);
        let offset = layer.procedural.y;
        return mix(paint_ramp_color(offset + index * 3u), paint_ramp_color(offset + min(index + 1u, last) * 3u), pos - f32(index));
    }
    return layer.low.rgb + (layer.high.rgb - layer.low.rgb) * t;
}

fn stored_paint(layer: Operation, pixel: vec2<u32>) -> vec4<f32> {
    // An empty signed texel stores zero as 32768.
    let empty = vec4(vec3(select(0.0, 1.0 / 65535.0, layer.ids.x == 6u)), 0.0);
    if layer.ids.z == NO_PAINT_IMAGE {
        return empty;
    }
    let atlas_tile = pixel / 64u;
    if any(atlas_tile < params.paint_window.xy) {
        return empty;
    }
    let tile = atlas_tile - params.paint_window.xy;
    if any(tile >= params.paint_window.zw) {
        return empty;
    }
    let local = pixel % 64u;
    let offset = paint[paint[layer.ids.z] + tile.y * params.paint_window.z + tile.x];
    if offset == 0u {
        return empty;
    }
    let rgba = layer.ids.w == 4u;
    let texel = offset + (local.y * 64u + local.x) * (layer.ids.w / 2u);
    let first = paint[texel];
    var coverage = f32(first >> 16u) / 65535.0;
    var rgb = vec3(f32(first & 65535u) / 65535.0);
    if rgba {
        let second = paint[texel + 1u];
        rgb.y = f32(first >> 16u) / 65535.0;
        rgb.z = f32(second & 65535u) / 65535.0;
        coverage = f32(second >> 16u) / 65535.0;
    }
    if layer.ids.x == 6u {
        rgb = rgb * 2.0 - 1.0;
    }
    return vec4(rgb, coverage);
}

fn painted(layer: Operation, pixel: vec2<u32>) -> vec4<f32> {
    var stored = stored_paint(layer, pixel);
    if layer.projection.w == 1.0 {
        let m = f32(textureLoad(brush_alpha, vec2<i32>(pixel), 0).x) / 65535.0 * layer.uv.w;
        if m > 0.0 {
            // Match the paint commit's premultiplied over and 16-bit channel encoding.
            let pigment = select(layer.uv.xxx, layer.uv.xyz, layer.ids.w == 4u);
            let remaining = 1.0 - m;
            let premul = pigment * m + stored.rgb * remaining;
            let signed = layer.ids.x == 6u;
            let encode = select(premul, premul * 0.5 + 0.5, signed);
            let quantized = floor(clamp(encode, vec3(0.0), vec3(1.0)) * 65535.0 + 0.5) / 65535.0;
            let rgb = select(quantized, quantized * 2.0 - 1.0, signed);
            let coverage = f32(to_u16(m + stored.a * remaining)) / 65535.0;
            stored = vec4(rgb, coverage);
        }
    }
    if stored.a == 0.0 {
        return vec4(0.0);
    }
    return vec4(stored.rgb / stored.a, stored.a);
}

fn shift_unit(x: f32, amount: f32) -> f32 {
    let a = clamp(amount, -1.0, 1.0);
    return select(x * (1.0 + a), x + (1.0 - x) * a, a >= 0.0);
}

fn hue_to_rgb(p: f32, q: f32, hue: f32) -> f32 {
    let t = fract(hue);
    if t < 1.0 / 6.0 { return p + (q - p) * 6.0 * t; }
    if t < 0.5 { return q; }
    if t < 2.0 / 3.0 { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
    return p;
}

fn hsl_adjust(rgb: vec3<f32>, args: vec3<f32>) -> vec3<f32> {
    let c = clamp(rgb, vec3(0.0), vec3(1.0));
    let maximum = max(c.r, max(c.g, c.b));
    let minimum = min(c.r, min(c.g, c.b));
    let d = maximum - minimum;
    let lightness = (maximum + minimum) * 0.5;
    let light = shift_unit(lightness, args.z);
    if d <= 1e-6 { return vec3(light); }
    let saturation = d / select(maximum + minimum, 2.0 - maximum - minimum, lightness > 0.5);
    let sat = shift_unit(saturation, args.y);
    if sat <= 0.0 { return vec3(light); }
    var hue = (c.r - c.g) / d + 4.0;
    if maximum == c.r {
        let turns = (c.g - c.b) / d / 6.0;
        hue = fract(turns) * 6.0;
    } else if maximum == c.g {
        hue = (c.b - c.r) / d + 2.0;
    }
    let h = hue / 6.0 + args.x;
    let q = select(light + sat - light * sat, light * (1.0 + sat), light < 0.5);
    let p = 2.0 * light - q;
    return vec3(hue_to_rgb(p, q, h + 1.0 / 3.0), hue_to_rgb(p, q, h), hue_to_rgb(p, q, h - 1.0 / 3.0));
}

fn scalar_filter(op: Operation, x: f32) -> f32 {
    switch op.ids.y {
        case 20u: {
            let t = clamp((x - op.low.x) / max(op.low.y - op.low.x, 1e-5), 0.0, 1.0);
            return lerp(op.high.x, op.high.y, pow(t, 1.0 / max(op.low.z, 1e-3)));
        }
        case 21u: { return 1.0 - x; }
        case 22u: { return select(0.0, 1.0, x >= op.low.x); }
        case 23u: { return clamp((x - 0.5) * (1.0 + clamp(op.low.x, -1.0, 1.0)) + 0.5 + op.low.y, 0.0, 1.0); }
        case 24u: {
            let steps = max(floor(clamp(op.low.x, 2.0, 64.0) + 0.5) - 1.0, 1.0);
            return floor(clamp(x, 0.0, 1.0) * steps + 0.5) / steps;
        }
        case 25u: { return clamp(x, op.low.x, op.low.y); }
        case 26u: {
            if op.low.y >= 1.0 { return select(0.0, 1.0, x >= op.low.x); }
            let width = max(1.0 - clamp(op.low.y, 0.0, 1.0), 1e-3);
            return clamp((x - op.low.x) / width + 0.5, 0.0, 1.0);
        }
        case 27u: { return clamp(op.low.y + (x - op.low.y) * op.low.x, 0.0, 1.0); }
        case 28u: {
            var t = clamp(x, 0.0, 1.0) + clamp(op.low.x, -1.0, 1.0);
            if t > 1.0 { t -= 1.0; }
            if t < 0.0 { t += 1.0; }
            return clamp(t, 0.0, 1.0);
        }
        case 31u: { return shift_unit(clamp(x, 0.0, 1.0), op.low.z); }
        default: { return x; }
    }
}

fn effect_color(op: Operation, c: vec3<f32>, channel: u32) -> vec3<f32> {
    if channel == 7u {
        return select(c, vec3(c.x, 1.0 - c.y, c.z), op.ids.y == 30u);
    }
    if channel == 0u {
        if op.ids.y == 29u { return vec3(dot(c, vec3(0.3, 0.59, 0.11))); }
        if op.ids.y == 31u { return hsl_adjust(c, op.low.xyz); }
        return vec3(scalar_filter(op, c.r), scalar_filter(op, c.g), scalar_filter(op, c.b));
    }
    if channel == 6u {
        return vec3(scalar_filter(op, c.x * 0.5 + 0.5) * 2.0 - 1.0);
    }
    return vec3(scalar_filter(op, c.x));
}

fn burn(a: f32, b: f32) -> f32 {
    if a >= 1.0 { return 1.0; }
    if b <= 0.0 { return 0.0; }
    return 1.0 - min((1.0 - a) / b, 1.0);
}

fn dodge(a: f32, b: f32) -> f32 {
    if a <= 0.0 { return 0.0; }
    if b >= 1.0 { return 1.0; }
    return min(a / (1.0 - b), 1.0);
}

fn overlay(a: f32, b: f32) -> f32 {
    return select(1.0 - 2.0 * (1.0 - a) * (1.0 - b), 2.0 * a * b, a < 0.5);
}

fn blend_scalar(mode: u32, a: f32, b: f32) -> f32 {
    switch mode {
        case 1u: { return a; }
        case 3u: { return a * b; }
        case 4u: { return a / max(b, 1e-6); }
        case 5u: { return b / max(a, 1e-6); }
        case 6u: { return min(a, b); }
        case 7u: { return max(a, b); }
        case 8u: { return a + b; }
        case 9u: { return a - b; }
        case 10u: { return b - a; }
        case 11u: { return abs(a - b); }
        case 12u: { return a + b - 2.0 * a * b; }
        case 13u: { return a + 2.0 * b - 1.0; }
        case 14u: { return overlay(a, b); }
        case 15u: { return 1.0 - (1.0 - a) * (1.0 - b); }
        case 16u: { return a + b - 1.0; }
        case 17u: { return burn(a, b); }
        case 18u: { return dodge(a, b); }
        case 19u: {
            let x = clamp(a, 0.0, 1.0);
            let y = clamp(b, 0.0, 1.0);
            if y <= 0.5 { return x - (1.0 - 2.0 * y) * x * (1.0 - x); }
            let d = select(sqrt(x), ((16.0 * x - 12.0) * x + 4.0) * x, x <= 0.25);
            return x + (2.0 * y - 1.0) * (d - x);
        }
        case 20u: { return overlay(b, a); }
        case 21u: { return select(dodge(a, 2.0 * (b - 0.5)), burn(a, 2.0 * b), b < 0.5); }
        case 22u: { return select(max(a, 2.0 * b - 1.0), min(a, 2.0 * b), b < 0.5); }
        default: { return b; }
    }
}

fn luminance(c: vec3<f32>) -> f32 { return dot(c, vec3(0.3, 0.59, 0.11)); }
fn maximum(c: vec3<f32>) -> f32 { return max(c.x, max(c.y, c.z)); }
fn minimum(c: vec3<f32>) -> f32 { return min(c.x, min(c.y, c.z)); }

fn with_luminance(color: vec3<f32>, luminance_goal: f32) -> vec3<f32> {
    var c = color + luminance_goal - luminance(color);
    let l = luminance(c);
    let lo = minimum(c);
    let hi = maximum(c);
    if lo < 0.0 && l - lo > 1e-6 { c = l + (c - l) * l / (l - lo); }
    if hi > 1.0 && hi - l > 1e-6 { c = l + (c - l) * (1.0 - l) / (hi - l); }
    return c;
}

fn rnm(a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    let t = a * 2.0 - vec3(1.0, 1.0, 0.0);
    let u = (b * 2.0 - 1.0) * vec3(-1.0, -1.0, 1.0);
    let n = t * dot(t, u) - u * t.z;
    return n / max(length(n), 1e-6) * 0.5 + 0.5;
}

fn blend_rgb(mode: u32, a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    switch mode {
        case 23u: { return b * luminance(a); }
        case 24u: {
            let lo = minimum(a);
            let width = maximum(a) - lo;
            var saturated = vec3(0.0);
            if width > 1e-6 { saturated = (a - lo) * (maximum(b) - minimum(b)) / width; }
            return with_luminance(saturated, luminance(a));
        }
        case 25u: { return with_luminance(b, luminance(a)); }
        case 26u: {
            let max_a = maximum(a);
            let max_b = maximum(b);
            if max_a <= 1e-6 { return vec3(max_b); }
            return a * max_b / max_a;
        }
        case 27u: {
            let p = a * 2.0 - 1.0;
            let q = b * 2.0 - 1.0;
            let n = vec3(p.xy + q.xy, p.z * q.z);
            return n / max(length(n), 1e-6) * 0.5 + 0.5;
        }
        case 28u: { return rnm(a,b); }
        case 29u: { return rnm(b,a); }
        default: { return vec3(blend_scalar(mode, a.x,b.x), blend_scalar(mode, a.y,b.y), blend_scalar(mode, a.z,b.z)); }
    }
}

fn generator(effect: Operation, surface: Surface, baked: vec3<f32>) -> f32 {
    var signal = 0.0;
    switch effect.ids.y {
        case 40u: { signal = (baked.y - 0.5) * 2.0; }
        case 41u: { signal = 1.0 - baked.x; }
        case 42u: { signal = 1.0 - baked.z; }
        case 43u: { signal = baked.y; }
        case 44u: { signal = baked.x; }
        case 45u: { signal = dot(surface.position, effect.high.xyz) * 0.5 + 0.5; }
        default: { signal = dot(surface.normal, effect.high.xyz) * 0.5 + 0.5; }
    }
    if effect.low.z > 0.0 {
        signal += (fbm(surface.position * effect.projection.x, 4u, effect.ids.z, true) - 0.5) * effect.low.z;
    }
    let threshold = 1.0 - clamp(effect.low.x, 0.0, 1.0);
    let value = smoothstep(threshold - effect.low.y, threshold + effect.low.y, signal);
    return clamp(select(value, 1.0 - value, effect.ids.w != 0u) + effect.projection.y, 0.0, 1.0);
}

fn layer_mask(layer: Operation, pixel: vec2<u32>, uv: vec2<f32>, surface: Surface, baked: vec3<f32>) -> f32 {
    var mask = layer.balance.y;
    for (var i = 0u; i < layer.effects.y; i++) {
        let e = layers[params.program.x + layer.effects.x + i];
        var weight = e.low.w;
        var result: f32;
        if e.ids.y == 10u {
            let sampled = painted(e, pixel);
            result = blend_scalar(u32(e.balance.z), mask, sampled.x);
            weight *= sampled.w;
        } else if e.ids.y < 20u || e.ids.y >= 48u {
            result = blend_scalar(u32(e.balance.z), mask, value(e, uv, surface).x);
        } else if e.ids.y >= 40u {
            result = blend_scalar(u32(e.balance.z), mask, generator(e, surface, baked));
        } else {
            result = scalar_filter(e, mask);
        }
        mask = lerp(mask, result, weight);
    }
    return clamp(mask, 0.0, 1.0);
}

fn to_u16(v: f32) -> u32 {
    return u32(clamp(v, 0.0, 1.0) * 65535.0 + 0.5);
}

// Find a covered source within the padding radius. The CPU padding's breadth-first
// search breaks equal Chebyshev distances in row-major source order.
fn padded_source(pixel: vec2<u32>) -> vec2<u32> {
    let pitch = params.coverage.y;
    let radius = params.coverage.z;
    let current = uv_coverage[pixel.y * pitch + pixel.x / 32u];
    if (current & (1u << (pixel.x % 32u))) != 0u {
        return pixel;
    }

    let left = pixel.x - min(pixel.x, radius);
    let right = min(pixel.x + radius, params.source.x - 1u);
    let start_word = left / 32u;
    let start_bit = left % 32u;
    let center = pixel.x - left;
    let column_mask = (1u << (right - left + 1u)) - 1u;
    let lower_columns = (1u << (center + 1u)) - 1u;
    let top = pixel.y - min(pixel.y, radius);
    let bottom = min(pixel.y + radius, params.source.y - 1u);
    var best = params.source.xy;
    var distance = radius + 1u;

    for (var row = top; row <= bottom; row++) {
        let dy = max(row, pixel.y) - min(row, pixel.y);
        if dy >= distance {
            continue;
        }
        let index = row * pitch + start_word;
        var bits = uv_coverage[index] >> start_bit;
        if right / 32u != start_word {
            bits |= uv_coverage[index + 1u] << (32u - start_bit);
        }
        bits &= column_mask;
        if bits == 0u {
            continue;
        }

        let lower = bits & lower_columns;
        let upper = bits & (0xffffffffu << center);
        var dx = radius + 1u;
        if lower != 0u {
            dx = center - firstLeadingBit(lower);
        }
        if upper != 0u {
            dx = min(dx, firstTrailingBit(upper) - center);
        }
        let candidate = max(dx, dy);
        if candidate < distance {
            distance = candidate;
            let first = center - min(center, distance);
            best = vec2(left + firstTrailingBit(bits & (0xffffffffu << first)), row);
        }
    }
    return best;
}

@compute @workgroup_size(8, 8)
fn compose(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.band.x || id.y >= params.band.y {
        return;
    }
    let display_pixel = vec2(id.x + params.program.w, id.y + params.band.z);
    let sample_pixel = display_pixel * params.source.z;
    var pixel = sample_pixel;
    if params.coverage.x != 0u {
        pixel = padded_source(pixel);
    }
    let covered = all(pixel < params.source.xy);
    let uv = (vec2<f32>(pixel) + 0.5) / vec2<f32>(params.source.xy);
    let surface = surface_at(pixel);
    var mask_bakes = vec3(1.0, 0.5, 1.0);
    var display_ao = 1.0;
    if (params.coverage.w & 2u) != 0u {
        let display_bake = baked_at(sample_pixel, 0);
        display_ao = display_bake.x;
        if covered {
            mask_bakes = display_bake.xyz;
            if any(pixel != sample_pixel) { mask_bakes = baked_at(pixel, 0).xyz; }
        }
    }
    var base = vec3(0.5);
    var normal = vec3(0.5, 0.5, 1.0);
    var scalars = array<f32, 6>(1.0, 1.0, 0.5, 0.0, 0.5, 0.0);
    var last_mask = 0xffffffffu;
    var mask_value = 1.0;
    for (var i = 0u; covered && i < params.program.x; i++) {
        let layer = layers[i];
        var source: vec3<f32>;
        let mode = u32(layer.balance.z);
        var weight = layer.low.w;
        if layer.effects.y == 0u {
            weight *= layer.balance.y;
        } else {
            if last_mask != layer.effects.x {
                last_mask = layer.effects.x;
                mask_value = layer_mask(layer, pixel, uv, surface, mask_bakes);
            }
            weight *= mask_value;
        }
        if weight <= 0.0 { continue; }
        if layer.ids.y == 10u {
            let sampled = painted(layer, pixel);
            source = sampled.rgb;
            weight *= select(sampled.a, 1.0, mode == 2u && sampled.a > 0.0);
            if weight <= 0.0 { continue; }
        } else {
            source = value(layer, uv, surface);
        }
        let destination = layer.ids.x;
        for (var step = 0u; step < layer.effects.w; step++) {
            let effect = layers[params.program.x + layer.effects.z + step];
            let filtered = effect_color(effect, source, destination);
            source += (filtered - source) * effect.low.w;
        }
        if destination == 0u {
            base += (blend_rgb(mode, base, source) - base) * weight;
        } else if destination == 7u {
            normal += (blend_rgb(mode, normal, source) - normal) * weight;
        } else {
            let old = scalars[destination - 1u];
            scalars[destination - 1u] = lerp(old, blend_scalar(mode, old, source.x), weight);
        }
    }
    let index = id.y * params.program.y + id.x;
    output[index] = srgb[to_u16(base.x) >> 4u]
        | (srgb[to_u16(base.y) >> 4u] << 8u)
        | (srgb[to_u16(base.z) >> 4u] << 16u)
        | ((to_u16(scalars[0]) >> 8u) << 24u);
    let ao = clamp(display_ao * f32(to_u16(scalars[1])) / 65535.0, 0.0, 1.0);
    output[index + params.band.w] = u32(ao * 255.0 + 0.5)
        | ((to_u16(scalars[2]) >> 8u) << 8u)
        | ((to_u16(scalars[3]) >> 8u) << 16u)
        | ((to_u16(scalars[4]) >> 8u) << 24u);
    if params.source.w != 0u {
        let height = f32(to_u16(scalars[5] * 0.5 + 0.5)) * (2.0 / 65535.0) - 1.0;
        let height_index = 2u * (index + params.band.w);
        output[height_index] = pack2x16float(vec2(height, 0.04));
        output[height_index + 1u] = pack2x16float(vec2(0.04));
    }
    if params.program.z != 0u {
        var detail = vec3<f32>(vec3<u32>(to_u16(normal.r), to_u16(normal.g), to_u16(normal.b))) / 65535.0;
        if (params.coverage.w & 4u) != 0u {
            let baked_normal = baked_at(sample_pixel, 1);
            if baked_normal.a > 0.0 { detail = rnm(baked_normal.rgb, detail); }
        }
        let packed = detail * 2.0 - 1.0;
        let normal_index = 2u * (index + 2u * params.band.w);
        output[normal_index] = pack2x16float(packed.xy);
        output[normal_index + 1u] = pack2x16float(vec2(packed.z, 0.5));
    }
}
