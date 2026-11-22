struct Params {
    extent: vec4u, // width, height, operation, blend mode
    formats: vec4u, // front, back, mask, mask present
    alphas: vec4f, // packed grayscale alphas, opacity
    weights: vec4f,
    alpha_mode: vec4u,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> front: array<u32>;
@group(0) @binding(2) var<storage, read> back: array<u32>;
@group(0) @binding(3) var<storage, read> mask: array<u32>;
@group(0) @binding(4) var<storage, read_write> output: array<u32>;

fn word(slot: u32, i: u32) -> u32 {
    switch slot {
        case 0u: { return front[i]; }
        case 1u: { return back[i]; }
        default: { return mask[i]; }
    }
}

fn pixel(slot: u32, i: u32) -> vec4f {
    let format = params.formats[slot];
    switch format {
        case 2u: {
            let offset = i * 4u;
            return bitcast<vec4f>(vec4u(word(slot, offset), word(slot, offset + 1u), word(slot, offset + 2u), word(slot, offset + 3u)));
        }
        case 3u: {
            let rg = word(slot, i * 2u);
            let ba = word(slot, i * 2u + 1u);
            return vec4f(decode(rg & 0xffffu), decode(rg >> 16u), decode(ba & 0xffffu), decode(ba >> 16u));
        }
        default: {
            let step = (word(slot, i / 2u) >> ((i & 1u) * 16u)) & 0xffffu;
            let value = decode(step * select(1u, 257u, format == 1u));
            return vec4f(vec3f(value), params.alphas[slot]);
        }
    }
}

fn mul(a: f32, b: f32) -> f32 {
    return individual_product(vec2f(a), vec2f(b)).x;
}

fn weighted(p: vec4f, w: vec4f) -> f32 {
    let rg = individual_product(p.xy, w.xy);
    let ba = individual_product(p.zw, w.zw);
    return rg.x + rg.y + ba.x + ba.y;
}

fn color(i: u32) -> vec4f {
    let fg = pixel(0u, i);
    let bg = pixel(1u, i);
    let mode = params.extent.w;
    let alpha_mode = params.alpha_mode.x;
    var opacity = params.alphas.w;
    if params.formats.w != 0u {
        opacity = mul(weighted(pixel(2u, i), vec4f(0.2126, 0.7152, 0.0722, 0.0)), opacity);
    }
    var weight = opacity;
    var alpha = bg.a;
    let use_alpha = alpha_mode == 0u || (alpha_mode >= 2u && alpha_mode <= 4u);
    if use_alpha {
        if mode == 21u {
            alpha = bg.a + mul(fg.a - bg.a, opacity);
        } else {
            weight = mul(fg.a, opacity);
            switch alpha_mode {
                case 0u: { alpha = bg.a + mul(fg.a - bg.a, opacity); }
                case 2u, 3u: { alpha = weight + mul(bg.a, 1.0 - weight); }
                default: {}
            }
        }
    }
    if alpha_mode == 3u && mode == 0u {
        let rg = individual_product(fg.xy, vec2f(opacity)) + individual_product(bg.xy, vec2f(1.0 - weight));
        let b = mul(fg.b, opacity) + mul(bg.b, 1.0 - weight);
        return vec4f(rg, b, alpha);
    }
    let rg = blend_channels(mode, fg.xy, bg.xy, vec2f(weight));
    let b = blend_channels(mode, vec2f(fg.b), vec2f(bg.b), vec2f(weight)).x;
    return vec4f(rg, b, alpha);
}

fn packed(steps: vec2u) -> u32 {
    return steps.x | (steps.y << 16u);
}

fn write_color(i: u32, p: vec4f) {
    output[i * 2u] = packed(compiled_16bit(p.xy));
    output[i * 2u + 1u] = packed(compiled_16bit(p.zw));
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.extent.x / 2u || id.y >= params.extent.y {
        return;
    }
    let i = id.y * params.extent.x + id.x * 2u;
    switch params.extent.z {
        case 0u: {
            write_color(i, color(i));
            write_color(i + 1u, color(i + 1u));
        }
        case 1u: {
            let first = weighted(pixel(0u, i), params.weights);
            let second = weighted(pixel(0u, i + 1u), params.weights);
            output[i / 2u] = packed(compiled_16bit(vec2f(first, second)));
        }
        default: {
            let a = pixel(0u, i);
            let b = pixel(0u, i + 1u);
            write_color(i, vec4f(vec3f(weighted(a, params.weights)), a.a));
            write_color(i + 1u, vec4f(vec3f(weighted(b, params.weights)), b.a));
        }
    }
}
