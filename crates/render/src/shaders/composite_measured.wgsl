// Measured blend rules match core/src/rules.rs. Stack values are the color
// shown over the channel default, with separate coverage per channel.

fn measured_div(a: f32, b: f32) -> f32 {
    if abs(b) <= 1e-6 {
        return select(0.0, 1.0, a > 0.0);
    }
    return min(a / b, 1.0);
}

fn measured_burn(a: f32, b: f32) -> f32 {
    if b == 0.0 {
        return select(0.0, 1.0, a >= 1.0);
    }
    return max(0.0, 1.0 - (1.0 - a) / b);
}

fn measured_dodge(a: f32, b: f32) -> f32 {
    if b >= 1.0 {
        return select(0.0, 1.0, a > 0.0);
    }
    return min(1.0, a / (1.0 - b));
}

fn measured_scalar(mode: u32, a: f32, b: f32) -> f32 {
    switch mode {
        case 4u: { return measured_div(a, b); }
        case 5u: { return measured_div(b, a); }
        case 13u: {
            let value = a + 2.0 * b - 1.0;
            return select(min(1.0, value), max(0.0, value), b < 0.5);
        }
        case 17u: { return measured_burn(a, b); }
        case 18u: { return measured_dodge(a, b); }
        case 19u: {
            if b <= 0.5 { return 2.0 * a * b + a * a * (1.0 - 2.0 * b); }
            return 2.0 * a * (1.0 - b) + sqrt(max(a, 0.0)) * (2.0 * b - 1.0);
        }
        case 21u: {
            if b < 0.5 { return measured_burn(a, 2.0 * b); }
            return measured_dodge(a, 2.0 * b - 1.0);
        }
        default: { return blend_scalar(mode, a, b); }
    }
}

fn layer_blended_scalar(mode: u32, a: f32, b: f32, unit: bool) -> f32 {
    let blended = measured_scalar(mode, a, b);
    return select(blended, clamp(blended, 0.0, 1.0), unit && mode > 2u);
}

fn layer_blended_color(mode: u32, a: vec3<f32>, b: vec3<f32>, channel: u32) -> vec3<f32> {
    let unit = (bitcast<u32>(params.anchor_default.w) & (1u << channel)) != 0u;
    if !group_color_slot(channel) {
        return vec3(layer_blended_scalar(mode, a.x, b.x, unit));
    }
    if mode >= 23u && mode <= 29u {
        let blended = blend_rgb(mode, a, b);
        return select(blended, clamp(blended, vec3(0.0), vec3(1.0)), unit);
    }
    return vec3(
        layer_blended_scalar(mode, a.x, b.x, unit),
        layer_blended_scalar(mode, a.y, b.y, unit),
        layer_blended_scalar(mode, a.z, b.z, unit),
    );
}

fn empty_channel(channel: u32) -> vec3<f32> {
    switch channel {
        case 0u, 3u, 5u, 13u, 20u: { return vec3(0.5); }
        case 1u, 2u, 8u: { return vec3(1.0); }
        case 7u, 11u: { return vec3(0.5, 0.5, 1.0); }
        case 9u: { return vec3(0.04); }
        case 16u: { return vec3(0.1); }
        default: { return vec3(0.0); }
    }
}

fn unit_or_flat(v: vec3<f32>, epsilon: f32) -> vec3<f32> {
    let size = length(v);
    if size > epsilon && size <= 3.402823466e38 {
        return v / size;
    }
    return vec3(0.0, 0.0, 1.0);
}

fn pack_normal(v: vec3<f32>) -> vec3<f32> {
    return v / max(length(v), 1e-6) * 0.5 + 0.5;
}

fn measured_normal(mode: u32, below: vec3<f32>, top: vec3<f32>, weight: f32) -> vec3<f32> {
    let a = unit_or_flat(below * 2.0 - 1.0, 1e-8);
    let flat = vec3(0.0, 0.0, 1.0);
    let b = unit_or_flat(mix(flat, unit_or_flat(top * 2.0 - 1.0, 1e-8), weight), 1e-12);
    if mode == 27u {
        return pack_normal(vec3(a.xy + b.xy, a.z * b.z));
    }
    let inverse = mode == 29u;
    let t = select(a, b, inverse) + flat;
    let u = select(b, a, inverse) * vec3(-1.0, -1.0, 1.0);
    return pack_normal(t * dot(t, u) - u * t.z);
}

fn measured_over(mode: u32, below: vec4<f32>, top: vec4<f32>, strength: f32, channel: u32) -> vec4<f32> {
    if mode == 1u {
        return below;
    }
    if mode == 2u {
        return mix(below, top, strength);
    }
    let weight = top.a * strength;
    if weight <= 0.0 {
        return below;
    }
    let coverage = below.a + weight * (1.0 - below.a);
    if mode >= 27u && mode <= 29u {
        return vec4(measured_normal(mode, below.rgb, top.rgb, weight), coverage);
    }
    if below.a >= 1.0 {
        return vec4(mix(below.rgb, layer_blended_color(mode, below.rgb, top.rgb, channel), weight), 1.0);
    }
    let empty = empty_channel(channel);
    if below.a <= 0.0 {
        return vec4(mix(empty, top.rgb, weight), coverage);
    }
    let straight = (below.rgb - empty * (1.0 - below.a)) / below.a;
    let blended = layer_blended_color(mode, straight, top.rgb, channel);
    return vec4(
        top.rgb * weight * (1.0 - below.a) + straight * below.a * (1.0 - weight)
            + blended * weight * below.a + empty * (1.0 - coverage),
        coverage,
    );
}
