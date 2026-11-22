struct Params {
    dimensions: vec4<u32>, // output width, band rows, input width, opcode (0 enlarges, 1 reduces)
    blend: vec4<u32>,      // opacity bits, whole-image input instead of a stripe with wrap padding
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> front: array<u32>;
@group(0) @binding(2) var<storage, read> back: array<u32>;
@group(0) @binding(3) var<storage, read_write> output: array<u32>;

fn even(word: u32) -> f32 {
    return decode(word & 0xffffu);
}

fn odd(word: u32) -> f32 {
    return decode(word >> 16u);
}

fn source(x: u32, y: u32) -> f32 {
    let word = front[(y * params.dimensions.z + x) / 2u];
    return select(even(word), odd(word), (x & 1u) != 0u);
}

fn doubled(x: u32, y: u32) -> vec2<f32> {
    let w = params.dimensions.z;
    let whole_image = params.blend.y != 0u;
    let main = y / 2u + select(1u, 0u, whole_image);
    let side_offset = select(w - 1u, 1u, (y & 1u) != 0u);
    var side = (main + side_offset) & (w - 1u);
    if !whole_image {
        side = select(main - 1u, main + 1u, (y & 1u) != 0u);
    }
    let left = (x + w - 1u) & (w - 1u);
    let right = (x + 1u) & (w - 1u);
    let main_sample = vec2(source(left, main), source(right, main));
    let side_sample = vec2(source(left, side), source(right, side));
    let main_row = source(x, main) * 0.75 + main_sample * 0.25;
    let side_row = source(x, side) * 0.75 + side_sample * 0.25;
    return main_row * 0.75 + side_row * 0.25;
}

fn halved(x: u32, y: u32) -> vec2<f32> {
    let stride = params.dimensions.z / 2u;
    let i = 2u * (y * stride + x);
    let top = vec2(front[i], front[i + 1u]);
    let bottom = vec2(front[i + stride], front[i + stride + 1u]);
    // Accumulate in CPU row order. A packed integer average changes ties because the CPU sums decoded f32 values.
    var sum = vec2(even(top.x), even(top.y));
    sum += vec2(odd(top.x), odd(top.y));
    sum += vec2(even(bottom.x), even(bottom.y));
    sum += vec2(odd(bottom.x), odd(bottom.y));
    return sum * 0.25;
}

fn blended(i: u32) -> vec2<f32> {
    let a = front[i];
    let b = back[i];
    let f = vec2(even(a), odd(a));
    let bg = vec2(even(b), odd(b));
    let opacity = bitcast<f32>(params.blend.x);
    switch params.dimensions.w {
        case 3u: { return bg + (f - bg) * opacity; }
        case 32u: { return bg + f * opacity; }
        case 33u: { return bg - f * opacity; }
        default: { return bg + (2.0 * f - 1.0) * opacity; }
    }
}

fn lookup_level(sample: u32) -> u32 {
    return (back[sample / 2u] >> ((sample & 1u) * 16u)) & 0xffffu;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let width = params.dimensions.x / 2u;
    if gid.x >= width || gid.y >= params.dimensions.y {
        return;
    }
    let i = gid.y * width + gid.x;
    var words: vec2<u32>;
    if params.dimensions.w <= 1u {
        var value: vec2<f32>;
        if params.dimensions.w == 0u {
            value = doubled(gid.x, gid.y);
        } else {
            value = halved(gid.x, gid.y);
        }
        words = vec2(rounded_read(value.x), rounded_read(value.y));
    } else if params.dimensions.w == 2u {
        let packed = front[i];
        words = vec2(lookup_level(packed & 0xffffu), lookup_level(packed >> 16u));
    } else {
        words = compiled_16bit(blended(i));
    }
    output[i] = words.x | (words.y << 16u);
}
