struct Params {
    // Source and destination dimensions, followed by source and output tile windows.
    sizes: vec4<u32>,
    source: vec4<u32>,
    dest: vec4<u32>,
    // Packed words per texel and the empty texel as two packed words.
    format: vec4<u32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> tiles: array<u32>;
@group(0) @binding(2) var<storage, read_write> output: array<u32>;

fn source_word(at: vec2<u32>, word: u32) -> u32 {
    let pixel = min(at, params.sizes.xy - 1u);
    let tile = pixel / 64u - params.source.xy;
    if any(tile >= params.source.zw) {
        return params.format[word + 1u];
    }
    let addr = tiles[tiles[0] + tile.y * params.source.z + tile.x];
    if addr == 0u {
        return params.format[word + 1u];
    }
    let local = pixel % 64u;
    return tiles[addr + (local.y * 64u + local.x) * params.format.x + word];
}

const ONE = 65536u;
const LOW = ONE - 1u;

fn channels(word: u32) -> vec2<u32> {
    return vec2(word & LOW, word >> 16u);
}

// Horizontal intermediates have 16 fractional bits. Split them into high/low words
// before the vertical blend and round once, as in TileImage::resampled_tile.
fn bilinear(a: vec2<u32>, b: vec2<u32>, c: vec2<u32>, d: vec2<u32>, w: vec2<u32>) -> vec2<u32> {
    let top = a * (ONE - w.x) + b * w.x;
    let bottom = c * (ONE - w.x) + d * w.x;
    let bits = vec2(16u);
    let high = (top >> bits) * (ONE - w.y) + (bottom >> bits) * w.y;
    let low = (top & vec2(LOW)) * (ONE - w.y) + (bottom & vec2(LOW)) * w.y;
    return (high + (low >> bits) + (ONE / 2u)) >> bits;
}

@compute @workgroup_size(8, 8)
fn resize(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = params.dest.xy * 64u + id.xy;
    let tile = id.xy / 64u;
    let local = id.xy % 64u;
    let offset = ((tile.y * params.dest.z + tile.x) * 4096u + local.y * 64u + local.x) * params.format.x;
    if any(pixel >= params.sizes.zw) {
        for (var i = 0u; i < params.format.x; i++) { output[offset + i] = 0u; }
        return;
    }
    let dest_size = params.sizes.zw;
    let denominator = 2u * dest_size;
    let center = max((2u * pixel + 1u) * params.sizes.xy, dest_size) - dest_size;
    let base = center / denominator;
    let w = ((center % denominator) * ONE + dest_size) / denominator;
    for (var i = 0u; i < params.format.x; i++) {
        let result = bilinear(
            channels(source_word(base, i)),
            channels(source_word(base + vec2(1u, 0u), i)),
            channels(source_word(base + vec2(0u, 1u), i)),
            channels(source_word(base + 1u, i)),
            w,
        );
        output[offset + i] = result.x | (result.y << 16u);
    }
}
