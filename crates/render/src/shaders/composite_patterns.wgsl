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
        default: { return periodic_pattern(kind, p, seed); }
    }
}

fn raw_procedural(layer: Operation, p: vec3<f32>) -> f32 {
    if layer.ids.y == 80u {
        return tile_generator(u32(layer.high.w), p.xy);
    }
    return pattern(layer.ids.y, p, layer.ids.z);
}

fn sample_procedural(layer: Operation, p: vec3<f32>) -> f32 {
    let reduced = layer.procedural.x != 0u;
    let cells = f32(max(layer.procedural.x, 1u)) / layer.uv.z;
    let grid = p.xy * cells - 0.5;
    let base = floor(grid) + 0.5;
    var values = vec4(0.0);
    for (var i = 0u; i < select(1u, 4u, reduced); i++) {
        let offset = vec2<f32>(vec2(i % 2u, i / 2u));
        let point = select(p.xy, (base + offset) / cells, reduced);
        let value = raw_procedural(layer, vec3(point, p.z));
        // Keeping the write at constant indices also lets FXC keep this loop rolled.
        switch i {
            case 0u: { values.x = value; }
            case 1u: { values.y = value; }
            case 2u: { values.z = value; }
            default: { values.w = value; }
        }
    }
    let t = fract(grid);
    return select(values.x, mix(mix(values.x, values.y, t.x), mix(values.z, values.w, t.x), t.y), reduced);
}

fn procedural_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec3<f32> {
    let mode = layer.procedural.w;
    let p = surface.position * layer.projection.w + layer.projection.xyz;
    var points = array<vec2<f32>, 3>(p.zy, p.xz, p.xy);
    var w = vec3(1.0, 0.0, 0.0);
    if mode == 1u {
        let weighted = pow(abs(surface.normal), vec3(layer.balance.w));
        let weights = weighted / max(weighted.x + weighted.y + weighted.z, 1e-6);
        let minimum = 1.0 / (131072.0 * max(layer.uv.w, 1.0));
        let retained = select(vec3(0.0), weights, weights >= vec3(minimum));
        w = select(retained, weights, all(retained == vec3(0.0)));
    } else {
        switch mode {
            case 2u: {}
            case 3u: { points[0] = p.xz; }
            case 4u: { points[0] = p.xy; }
            case 5u: {
                var d = vec3(0.0, 1.0, 0.0);
                if dot(surface.position, surface.position) > 0.0 { d = normalize(surface.position); }
                points[0] = vec2(azimuth(d.z, d.x) * 4.0, acos(clamp(d.y, -1.0, 1.0)) * (2.0 / 3.141592654)) * layer.projection.w;
            }
            case 6u, 7u, 8u: {
                let pos = surface.position;
                let axis = select(select(pos.xyz, pos.yxz, mode == 6u), pos.xzy, mode == 8u);
                points[0] = vec2(azimuth(axis.z, axis.x) * 4.0, axis.y) * layer.projection.w;
            }
            case DECAL_PROJECTION: { points[0] = uv * layer.uv.z; }
            default: {
                let rotation = layer.projection.xy;
                let rotated = vec2(uv.x * rotation.x - uv.y * rotation.y, uv.x * rotation.y + uv.y * rotation.x);
                points[0] = (rotated * layer.projection.zw + layer.uv.xy) * layer.uv.z;
            }
        }
    }
    var n = 0.0;
    for (var plane = 0u; plane < select(1u, 3u, mode == 1u); plane++) {
        if w[plane] > 0.0 {
            n += w[plane] * sample_procedural(layer, vec3(points[plane], 0.0));
        }
    }
    let shaped = clamp((n - 0.5) * layer.uv.w + 0.5 + layer.balance.x, 0.0, 1.0);
    let t = select(shaped, 1.0 - shaped, layer.ids.w != 0u);
    if layer.procedural.z > 0u {
        return gradient_color(layer, t);
    }
    return layer.low.rgb + (layer.high.rgb - layer.low.rgb) * t;
}
