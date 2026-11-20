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

fn channels(word: u32) -> vec2<f32> {
    return vec2<f32>(f32(word & 65535u), f32(word >> 16u));
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
    let source = max((vec2<f32>(pixel) + 0.5) * vec2<f32>(params.sizes.xy) / vec2<f32>(params.sizes.zw) - 0.5, vec2(0.0));
    let base = vec2<u32>(source);
    let w = fract(source);
    for (var i = 0u; i < params.format.x; i++) {
        let top = channels(source_word(base, i)) * (1.0 - w.x) + channels(source_word(base + vec2(1u, 0u), i)) * w.x;
        let bottom = channels(source_word(base + vec2(0u, 1u), i)) * (1.0 - w.x) + channels(source_word(base + 1u, i)) * w.x;
        let result = vec2<u32>(min(top * (1.0 - w.y) + bottom * w.y + 0.5, vec2(65535.0)));
        output[offset + i] = result.x | (result.y << 16u);
    }
}
