struct Params {
    dimensions: vec4<u32>, // output width, band rows, input width, blend opcode or zero for a doubling
    blend: vec4<u32>,      // opacity bits, whole-image input instead of a stripe with wrap padding
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> front: array<u32>;
@group(0) @binding(2) var<storage, read> back: array<u32>;
@group(0) @binding(3) var<storage, read_write> output: array<u32>;

fn decode(value: u32) -> f32 {
    if value == 0u || value == 65535u {
        return select(0.0, 1.0, value != 0u);
    }
    // The normalized significand of q / 65535 is (q << (23 - leading_bit)) * 65536 / 65535.
    let leading = firstLeadingBit(value);
    let base = value << (23u - leading);
    let significand = base + (base + 32767u) / 65535u;
    return bitcast<f32>(((leading + 111u) << 23u) | (significand & 0x7fffffu));
}

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

fn rounded_read(input: f32) -> u32 {
    // Match the CPU's half-up read using integer high and low words for the exact product of the
    // f32 significand and 65535. A direct f32 multiply would round twice around the midpoint.
    let bits = bitcast<u32>(clamp(input, 0.0, 1.0));
    let exponent = bits >> 23u;
    if exponent < 110u {
        return 0u;
    }
    let shift = 150u - exponent;
    let significand = (bits & 0x7fffffu) | 0x800000u;
    let left = significand << 16u;
    let low = left - significand;
    let high = (significand >> 16u) - select(0u, 1u, left < significand);
    if shift > 32u {
        return (high + (1u << (shift - 33u))) >> (shift - 32u);
    }
    let rounded = low + (1u << (shift - 1u));
    let carry = high + select(0u, 1u, rounded < low);
    if shift == 32u {
        return carry;
    }
    return (carry << (32u - shift)) | (rounded >> shift);
}

fn blended(i: u32) -> vec2<f32> {
    let a = front[i];
    let b = back[i];
    let f = vec2(even(a), odd(a));
    let bg = vec2(even(b), odd(b));
    let opacity = bitcast<f32>(params.blend.x);
    switch params.dimensions.w {
        case 32u: { return bg + f * opacity; }
        case 33u: { return bg - f * opacity; }
        default: { return bg + (2.0 * f - 1.0) * opacity; }
    }
}

fn compiled_16bit(value: vec2<f32>) -> vec2<u32> {
    let fine = vec2<u32>(clamp(value, vec2(0.0), vec2(1.0)) * 1048576.0);
    // (65535 * fine + 524287) >> 20, split so no intermediate needs more than 32 bits.
    let low = vec2<i32>((fine & vec2<u32>(15u)) << vec2<u32>(16u)) + 524287 - vec2<i32>(fine);
    return vec2<u32>(vec2<i32>(fine >> vec2<u32>(4u)) + (low >> vec2<u32>(20u)));
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let width = params.dimensions.x / 2u;
    if gid.x >= width || gid.y >= params.dimensions.y {
        return;
    }
    let i = gid.y * width + gid.x;
    var words: vec2<u32>;
    if params.dimensions.w == 0u {
        let value = doubled(gid.x, gid.y);
        words = vec2(rounded_read(value.x), rounded_read(value.y));
    } else {
        words = compiled_16bit(blended(i));
    }
    output[i] = words.x | (words.y << 16u);
}
