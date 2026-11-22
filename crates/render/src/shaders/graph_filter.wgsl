struct Params {
    source: vec4<u32>,  // width, height, grayscale divisor
    output: vec4<u32>,
    map: vec4<u32>,
    control: vec4<u32>, // filter, major axis, wrap x, wrap y
    values: vec4<f32>,
    direction: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> front: array<u32>;
@group(0) @binding(2) var<storage, read> height_map: array<u32>;
@group(0) @binding(3) var<storage, read_write> output: array<u32>;

fn limit(p: i32, n: i32, wrap: bool) -> u32 {
    if wrap {
        // The packed input axes are powers of two
        return u32(p) & u32(n - 1);
    }
    return u32(clamp(p, 0, n - 1));
}

fn address(p: vec2<i32>, size: vec2<u32>, wraps: vec2<bool>) -> u32 {
    let x = limit(p.x, i32(size.x), wraps.x);
    let y = limit(p.y, i32(size.y), wraps.y);
    return y * size.x + x;
}

fn unpack_grey(word: u32, i: u32) -> u32 {
    return (word >> ((i & 1u) * 16u)) & 0xffffu;
}

fn decode_grey(value: u32, div: u32) -> f32 {
    if div == 255u {
        return f32(value) / 255.0;
    }
    return decode(value);
}

fn front_at(p: vec2<i32>) -> f32 {
    let i = address(p, params.source.xy, params.control.zw != vec2<u32>(0u));
    return decode_grey(unpack_grey(front[i / 2u], i), params.source.z);
}

fn front_units(p: vec2<i32>) -> u32 {
    let i = address(p, params.source.xy, params.control.zw != vec2<u32>(0u));
    return unpack_grey(front[i / 2u], i) * select(1u, 257u, params.source.z == 255u);
}

fn map_at(p: vec2<i32>, wraps: vec2<bool>) -> f32 {
    let i = address(p, params.map.xy, wraps);
    return decode_grey(unpack_grey(height_map[i / 2u], i), params.map.z);
}

fn lerp(lo: f32, hi: f32, weight: f32) -> f32 {
    return lo + (hi - lo) * weight;
}

fn luminance(v: f32) -> f32 {
    return (v * 0.2126 + v * 0.7152) + v * 0.0722;
}

fn map_luminance(uv: vec2<f32>) -> f32 {
    let xy = uv * vec2<f32>(params.map.xy) - vec2<f32>(0.5);
    let base = vec2<i32>(floor(xy));
    let weight = fract(xy);
    let wraps = vec2<bool>(true);
    let top = lerp(map_at(base, wraps), map_at(base + vec2<i32>(1, 0), wraps), weight.x);
    let bottom = lerp(map_at(base + vec2<i32>(0, 1), wraps), map_at(base + vec2<i32>(1, 1), wraps), weight.x);
    return luminance(lerp(top, bottom, weight.y));
}

fn map_slope(p: vec2<i32>) -> vec2<f32> {
    let wraps = params.control.zw != vec2<u32>(0u);
    let center = luminance(map_at(p, wraps));
    let left = luminance(map_at(p - vec2<i32>(1, 0), wraps));
    let above = luminance(map_at(p - vec2<i32>(0, 1), wraps));
    return vec2<f32>(center - left, center - above) * vec2<f32>(params.map.xy);
}

fn lerp_slope(lo: vec2<f32>, hi: vec2<f32>, weight: f32) -> vec2<f32> {
    return lo + (hi - lo) * weight;
}

fn sampled_slope(uv: vec2<f32>) -> vec2<f32> {
    let xy = uv * vec2<f32>(params.map.xy) - vec2<f32>(0.5);
    let mask = vec2<i32>(params.map.xy) - vec2<i32>(1);
    let base = vec2<i32>(floor(xy)) & mask;
    let weight = fract(xy);
    if all(weight == vec2<f32>(0.0)) {
        return map_slope(base);
    }
    // The slope field's bilinear read wraps even when computing its differences clamps an edge
    let top = lerp_slope(map_slope(base), map_slope((base + vec2<i32>(1, 0)) & mask), weight.x);
    let bottom = lerp_slope(map_slope((base + vec2<i32>(0, 1)) & mask), map_slope((base + vec2<i32>(1, 1)) & mask), weight.x);
    return lerp_slope(top, bottom, weight.y);
}

fn stepped_front(uv: vec2<f32>) -> f32 {
    let size = vec2<f32>(params.source.xy);
    let half_texel = 0.5 / size;
    let coords = select(clamp(uv, half_texel, 1.0 - half_texel), uv, params.control.zw != vec2<u32>(0u));
    let scaled = coords * size;
    let whole = floor(scaled);
    let fraction = scaled - whole;
    // Subtract the half-texel in separate integer units so a large coordinate cannot move a rounding tie
    let previous = fraction < vec2<f32>(0.5);
    let base = vec2<i32>(whole) - select(vec2<i32>(0), vec2<i32>(1), previous);
    let steps = fraction * 256.0;
    let rounded = floor(steps) + select(vec2<f32>(0.0), vec2<f32>(1.0), fract(steps) >= vec2<f32>(0.5));
    let weights = (rounded + select(vec2<f32>(-128.0), vec2<f32>(128.0), previous)) / 256.0;
    let top = lerp(front_at(base), front_at(base + vec2<i32>(1, 0)), weights.x);
    let bottom = lerp(front_at(base + vec2<i32>(0, 1)), front_at(base + vec2<i32>(1, 1)), weights.x);
    return lerp(top, bottom, weights.y);
}

fn short_box(p: vec2<i32>) -> f32 {
    let radius = params.values.x;
    if radius <= 0.5 {
        return front_at(p);
    }
    let axis = select(vec2<i32>(1, 0), vec2<i32>(0, 1), params.control.y != 0u);
    let center = front_units(p);
    let inner = vec2<u32>(front_units(p - axis), front_units(p + axis));
    if radius <= 1.5 {
        return (f32(center) + (radius - 0.5) * f32(inner.x + inner.y)) / (2.0 * radius * 65535.0);
    }
    let outer = vec2<u32>(front_units(p - 2 * axis), front_units(p + 2 * axis));
    let edge = radius - 1.5;
    let wrap = select(params.control.z, params.control.w, params.control.y != 0u);
    if wrap == 0u {
        return (f32(center + inner.x + inner.y) + edge * f32(outer.x + outer.y)) / (2.0 * radius * 65535.0);
    }
    let weight = u32(params.values.y);
    let tap = (((256u - weight) * inner + weight * outer) + 128u) >> vec2<u32>(8u);
    return (f32(center) + (1.0 + edge) * f32(tap.x + tap.y)) / (2.0 * radius * 65535.0);
}

fn minor_read(p: vec2<i32>, minor: vec2<i32>, offset: f32) -> f32 {
    let base = floor(offset);
    let fractional = offset - base;
    let weight = floor(fractional * 256.0 + 0.5) / 256.0;
    let at = p + i32(base) * minor;
    return lerp(front_at(at), front_at(at + minor), weight);
}

fn short_line(p: vec2<i32>) -> f32 {
    let major = select(vec2<i32>(1, 0), vec2<i32>(0, 1), params.control.y != 0u);
    let minor = major.yx;
    let radius = params.values.x;
    let slope = params.values.y;
    let a = minor_read(p - major, minor, -slope);
    let b = minor_read(p + major, minor, slope);
    return (front_at(p) + (radius - 0.5) * (a + b)) / (2.0 * radius);
}

fn sharpen(p: vec2<i32>) -> f32 {
    let c = front_at(p);
    let l = front_at(p - vec2<i32>(1, 0));
    let r = front_at(p + vec2<i32>(1, 0));
    let t = front_at(p - vec2<i32>(0, 1));
    let b = front_at(p + vec2<i32>(0, 1));
    return clamp((c + params.values.x * ((2.0 * c - l) - r)) + params.values.y * ((2.0 * c - t) - b), 0.0, 1.0);
}

fn value_at(p: vec2<i32>) -> f32 {
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(params.output.xy);
    switch params.control.x {
        case 0u: {
            let distance = params.values.x * (map_luminance(uv) - params.values.y);
            return stepped_front(uv + (distance * params.direction.xy) * params.values.zw);
        }
        case 1u: { return stepped_front(uv + params.values.xy * sampled_slope(uv)); }
        case 2u: { return short_box(p); }
        case 3u: { return short_line(p); }
        default: { return sharpen(p); }
    }
}

fn compiled_8bit_sample(value: f32) -> u32 {
    return (255u * (rounded_read(value) >> 4u) + 2047u) >> 12u;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let width = params.output.x / 2u;
    if gid.x >= width || gid.y >= params.output.y {
        return;
    }
    let p = vec2<i32>(i32(gid.x * 2u), i32(gid.y));
    let values = vec2<f32>(value_at(p), value_at(p + vec2<i32>(1, 0)));
    var words: vec2<u32>;
    if params.output.z == 255u {
        words = vec2<u32>(compiled_8bit_sample(values.x), compiled_8bit_sample(values.y));
    } else if params.control.x < 2u {
        // Warps round reads to 16 bits before applying the node's compiled format
        words = vec2<u32>(rounded_read(values.x), rounded_read(values.y));
    } else {
        words = compiled_16bit(values);
    }
    output[gid.y * width + gid.x] = words.x | (words.y << 16u);
}
