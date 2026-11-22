struct Params {
    dimensions: vec4<u32>, // output width, band rows, input width, opcode (0 enlarges, 1 reduces)
    blend: vec4<u32>,      // opacity bits, whole-image input instead of a stripe with wrap padding, has mask
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> front: array<u32>;
@group(0) @binding(2) var<storage, read> back: array<u32>;
@group(0) @binding(3) var<storage, read_write> output: array<u32>;
@group(0) @binding(4) var<storage, read> mask: array<u32>;

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

// The CPU rounds each product before adding. Canonicalizing signed zeros
// prevents contraction at ties without changing the quantized output.
fn individual_product(a: vec2f, b: vec2f) -> vec2f {
    let bits = bitcast<vec2u>(a * b);
    return bitcast<vec2f>(select(bits, vec2u(0u), bits == vec2u(0x80000000u)));
}

fn mask_weight(i: u32) -> vec2f {
    let word = mask[i];
    let v = vec2(even(word), odd(word));
    // Match the reference luminance of an RGB sample with three equal channels.
    return individual_product(v, vec2f(0.2126)) + individual_product(v, vec2f(0.7152)) + individual_product(v, vec2f(0.0722));
}

fn separable_blend(mode: u32, f: vec2f, b: vec2f) -> vec2f {
    switch mode {
        case 1u: { return f + b; }
        case 2u: { return b - f; }
        case 3u: { return individual_product(f, b); }
        case 4u: { return 1.0 - individual_product(1.0 - f, 1.0 - b); }
        case 5u, 13u: {
            let pivot = select(b, f, mode == 13u);
            let dark = individual_product(2.0 * f, b);
            let light = 1.0 - individual_product(2.0 * (1.0 - f), 1.0 - b);
            return select(light, dark, pivot < vec2f(0.5));
        }
        case 6u: { return max(f, b); }
        case 7u: { return min(f, b); }
        case 8u: { return abs(f - b); }
        case 9u: { return individual_product(individual_product(1.0 - 2.0 * f, b), b) + individual_product(2.0 * f, b); }
        case 10u: { return select(b / max(1.0 - f, vec2f(0.001)), vec2f(1.0), f >= vec2f(0.999)); }
        case 11u: { return select(1.0 - (1.0 - b) / max(f, vec2f(0.001)), vec2f(0.0), f <= vec2f(0.001)); }
        case 12u: { return f + b - 1.0; }
        case 14u: { return f + b - individual_product(2.0 * f, b); }
        case 15u: { return select(b / max(f, vec2f(0.001)), b, f <= vec2f(0.001)); }
        case 16u: { return b + 2.0 * f - 1.0; }
        case 20u: { return b + (f - 0.5) * 2.0; }
        case 36u: {
            let dark = b - individual_product(individual_product(1.0 - 2.0 * f, b), 1.0 - b);
            let light = b + individual_product(2.0 * f - 1.0, sqrt(b) - b);
            return select(light, dark, f <= vec2f(0.5));
        }
        default: { return f; }
    }
}

fn blended(i: u32) -> vec2<f32> {
    let a = front[i];
    let b = back[i];
    let f = vec2(even(a), odd(a));
    let bg = vec2(even(b), odd(b));
    var opacity = vec2f(bitcast<f32>(params.blend.x));
    if params.blend.z != 0u {
        opacity = individual_product(mask_weight(i), opacity);
    }
    let mode = params.dimensions.w - 3u; // Blend opcodes follow enlarge, reduce, and levels.
    switch mode {
        case 21u: { return bg + individual_product(f - bg, opacity); }
        case 32u: { return bg + individual_product(f, opacity); }
        case 33u: { return bg - individual_product(f, opacity); }
        case 34u: { return bg + individual_product(2.0 * f - 1.0, opacity); }
        case 35u: {
            let divided = select(bg / max(f, vec2f(0.000015)), vec2f(1.0), f == vec2f(0.0));
            return bg + individual_product(divided - bg, opacity);
        }
        default: {
            let color = clamp(separable_blend(mode, f, bg), vec2f(0.0), vec2f(1.0));
            return bg + individual_product(color - bg, opacity);
        }
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
