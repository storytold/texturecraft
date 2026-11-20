// UV fill compositor. Noise and color conversions follow texel/src/{noise,tile}.rs.
struct Operation {
    ids: vec4<u32>,
    low: vec4<f32>,
    high: vec4<f32>,
    projection: vec4<f32>,
    uv: vec4<f32>,
    balance: vec4<f32>,
    effects: vec4<u32>,
}

struct Params {
    // Width, row count, top row, reserved texels per map.
    band: vec4<u32>,
    // Source width, source height, display sampling step, height map enabled.
    source: vec4<u32>,
    program: vec4<u32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> layers: array<Operation>;
@group(0) @binding(2) var<storage, read> srgb: array<u32>;
@group(0) @binding(3) var<storage, read_write> output: array<u32>;
@group(0) @binding(4) var<storage, read> paint: array<u32>;

fn pcg3d(cell: vec3<u32>) -> vec3<u32> {
    var v = cell * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v = v ^ (v >> vec3<u32>(16u));
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return v;
}

fn hash(cell: vec3<i32>, salt: u32) -> vec3<u32> {
    let bits = bitcast<vec3<u32>>(cell);
    return pcg3d(vec3<u32>(bits.x ^ salt, bits.yz));
}

const GRADIENTS = array<vec3<f32>, 12>(
    vec3(1.0, 1.0, 0.0), vec3(-1.0, 1.0, 0.0), vec3(1.0, -1.0, 0.0), vec3(-1.0, -1.0, 0.0),
    vec3(1.0, 0.0, 1.0), vec3(-1.0, 0.0, 1.0), vec3(1.0, 0.0, -1.0), vec3(-1.0, 0.0, -1.0),
    vec3(0.0, 1.0, 1.0), vec3(0.0, -1.0, 1.0), vec3(0.0, 1.0, -1.0), vec3(0.0, -1.0, -1.0),
);

fn lerp(a: f32, b: f32, w: f32) -> f32 {
    return a + (b - a) * w;
}

fn corner(cell: vec3<i32>, seed: u32, offset: vec3<i32>, local: vec3<f32>, gradient: bool) -> f32 {
    if gradient {
        return dot(GRADIENTS[hash(cell + offset, seed * 0x85ebca6bu).y % 12u], local - vec3<f32>(offset));
    }
    return f32(hash(cell + offset, seed * 0x9e3779b9u).x) / 4294967296.0;
}

fn noise(p: vec3<f32>, seed: u32, gradient: bool) -> f32 {
    let cell = vec3<i32>(floor(p));
    let local = fract(p);
    let w = local * local * local * (local * (local * 6.0 - 15.0) + 10.0);
    let xy0 = lerp(
        lerp(corner(cell, seed, vec3(0, 0, 0), local, gradient), corner(cell, seed, vec3(1, 0, 0), local, gradient), w.x),
        lerp(corner(cell, seed, vec3(0, 1, 0), local, gradient), corner(cell, seed, vec3(1, 1, 0), local, gradient), w.x),
        w.y);
    let xy1 = lerp(
        lerp(corner(cell, seed, vec3(0, 0, 1), local, gradient), corner(cell, seed, vec3(1, 0, 1), local, gradient), w.x),
        lerp(corner(cell, seed, vec3(0, 1, 1), local, gradient), corner(cell, seed, vec3(1, 1, 1), local, gradient), w.x),
        w.y);
    let value = lerp(xy0, xy1, w.z);
    return select(value, clamp(value * 0.5 + 0.5, 0.0, 1.0), gradient);
}

fn fbm(p: vec3<f32>, octaves: u32, seed: u32, gradient: bool) -> f32 {
    var position = p;
    var weight = 0.5;
    var normalization = 0.0;
    var sum = 0.0;
    for (var i = 0u; i < octaves; i++) {
        sum += weight * noise(position, seed + i * 31u, gradient);
        normalization += weight;
        position = position * 2.0 + 17.13;
        weight *= 0.5;
    }
    return sum / normalization;
}

fn cells(p: vec3<f32>, seed: u32) -> f32 {
    let cell = vec3<i32>(floor(p));
    let local = fract(p);
    var nearest = 1.0;
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let shift = vec3(x, y, z);
                let feature = vec3<f32>(hash(cell + shift, seed * 0xc2b2ae35u)) / 4294967296.0;
                let delta = vec3<f32>(shift) + feature - local;
                nearest = min(nearest, dot(delta, delta));
            }
        }
    }
    return sqrt(nearest);
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
        default: { return fract(p.x); }
    }
}

fn value(layer: Operation, uv: vec2<f32>) -> vec3<f32> {
    if layer.ids.y == 0u {
        return layer.low.rgb;
    }
    let rotation = layer.projection.xy;
    let rotated = vec2(uv.x * rotation.x - uv.y * rotation.y, uv.x * rotation.y + uv.y * rotation.x);
    let projected = (rotated * layer.projection.zw + layer.uv.xy) * layer.uv.z;
    let n = pattern(layer.ids.y, vec3(projected, 0.0), layer.ids.z);
    let shaped = clamp((n - 0.5) * layer.uv.w + 0.5 + layer.balance.x, 0.0, 1.0);
    let t = select(shaped, 1.0 - shaped, layer.ids.w != 0u);
    return layer.low.rgb + (layer.high.rgb - layer.low.rgb) * t;
}

fn painted(layer: Operation, pixel: vec2<u32>) -> vec4<f32> {
    let tile = pixel / 64u;
    let local = pixel % 64u;
    let offset = paint[layer.ids.z + tile.y * (params.source.x / 64u) + tile.x];
    if offset == 0u {
        return vec4(0.0);
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
    if coverage == 0.0 {
        return vec4(0.0);
    }
    if layer.ids.x == 6u {
        rgb = rgb * 2.0 - 1.0;
    }
    return vec4(rgb / coverage, coverage);
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

fn with_luminance(color: vec3<f32>, target: f32) -> vec3<f32> {
    var c = color + target - luminance(color);
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

fn layer_mask(layer: Operation, pixel: vec2<u32>, uv: vec2<f32>) -> f32 {
    var mask = layer.balance.y;
    for (var i = 0u; i < layer.effects.y; i++) {
        let e = layers[params.program.x + layer.effects.x + i];
        var weight = e.low.w;
        var result: f32;
        if e.ids.y == 10u {
            let sampled = painted(e, pixel);
            result = blend_scalar(u32(e.balance.z), mask, sampled.x);
            weight *= sampled.w;
        } else if e.ids.y < 10u {
            result = blend_scalar(u32(e.balance.z), mask, value(e, uv).x);
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

@compute @workgroup_size(8, 8)
fn compose(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.band.x || id.y >= params.band.y {
        return;
    }
    let pixel = vec2(id.x, id.y + params.band.z) * params.source.z;
    let uv = (vec2<f32>(pixel) + 0.5) / vec2<f32>(params.source.xy);
    var base = vec3(0.5);
    var normal = vec3(0.5, 0.5, 1.0);
    var scalars = array<f32, 6>(1.0, 1.0, 0.5, 0.0, 0.5, 0.0);
    for (var i = 0u; i < params.program.x; i++) {
        let layer = layers[i];
        var source: vec3<f32>;
        let mode = u32(layer.balance.z);
        var weight = layer.low.w;
        if layer.ids.y == 10u {
            let sampled = painted(layer, pixel);
            source = sampled.rgb;
            weight *= select(sampled.a, 1.0, mode == 2u && sampled.a > 0.0);
        } else {
            source = value(layer, uv);
        }
        let destination = layer.ids.x;
        for (var step = 0u; step < layer.effects.w; step++) {
            let effect = layers[params.program.x + layer.effects.z + step];
            let filtered = effect_color(effect, source, destination);
            source += (filtered - source) * effect.low.w;
        }
        weight *= layer_mask(layer, pixel, uv);
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
    output[index + params.band.w] = u32(f32(to_u16(scalars[1])) / 65535.0 * 255.0 + 0.5)
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
        let packed = vec3<f32>(to_u16(normal.r), to_u16(normal.g), to_u16(normal.b)) * (2.0 / 65535.0) - 1.0;
        let normal_index = 2u * (index + 2u * params.band.w);
        output[normal_index] = pack2x16float(packed.xy);
        output[normal_index + 1u] = pack2x16float(vec2(packed.z, 0.5));
    }
}
