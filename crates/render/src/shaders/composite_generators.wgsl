fn generator_ramp(effect: Operation, texel: LayerTexel) -> f32 {
    // P3a and P3b: the generators with their own controls (gpu_generator_p3a.rs)
    if effect.ids.y == GENERATOR_KIND_OPCODE {
        return generator_p3a_ramp(effect, texel);
    }
    let surface = texel.surface;
    var baked = texel.baked;
    if effect.effects.w != 0u {
        let input = generator_source_value(effect.effects.w - 1u, texel);
        switch effect.ids.y {
            case 40u, 43u: { baked.y = input; }
            case 41u, 44u: { baked.x = input; }
            case 42u: { baked.z = input; }
            default: {}
        }
    }
    let detail = select(0u, u32(effect.uv.y), enable_curvature_detail);
    var micro = 0.0;
    if enable_anchor_reads && effect.effects.z != 0u {
        let height = value(layers[effect.effects.z - 1u], texel.uv, surface).x;
        micro = clamp(height, -1.0, 1.0) * effect.uv.z;
    }
    var signal = 0.0;
    switch effect.ids.y {
        case 40u: { signal = (baked.y - 0.5) * 2.0; }
        case 41u: { signal = 1.0 - baked.x; }
        case 42u: { signal = 1.0 - baked.z; }
        case 43u: { signal = baked.y; }
        case 44u: { signal = baked.x; }
        case 45u: { signal = dot(surface.position, effect.high.xyz) * 0.5 + 0.5; }
        default: { signal = dot(surface.normal, effect.high.xyz) * 0.5 + 0.5; }
    }
    if detail != 0u {
        signal = generator_curvature(effect, texel.pixel, baked.y, detail, micro);
    } else {
        switch effect.ids.y {
            case 40u, 43u, 44u: { signal += micro; }
            case 41u: { signal -= micro; }
            default: {}
        }
    }
    if effect.low.z > 0.0 {
        let noise = fbm(surface.position * effect.projection.x, 4u, effect.ids.z, true);
        if detail != 0u {
            let neutral = select(0.0, 0.5, detail == 4u);
            signal = neutral + (signal - neutral) * max(0.0, 1.0 - noise * effect.low.z);
        } else {
            signal += (noise - 0.5) * effect.low.z;
        }
    }
    let threshold = 1.0 - clamp(effect.low.x, 0.0, 1.0);
    let ramp = clamp((signal - (threshold - effect.low.y)) / (2.0 * effect.low.y), 0.0, 1.0);
    return ramp * ramp * (3.0 - 2.0 * ramp);
}

fn finish_generator(effect: Operation, ramp: f32) -> f32 {
    var value = ramp;
    if effect.projection.z != 0.5 || effect.projection.w != 0.0 {
        let balance = clamp(effect.projection.z, 0.0, 1.0);
        let low = max(1.0 - 2.0 * balance, 0.0);
        let high = min(2.0 - 2.0 * balance, 1.0);
        let middle = (low + high) * 0.5;
        let keep = 1.0 - clamp(effect.projection.w, 0.0, 1.0);
        let first = middle - (middle - low) * keep;
        let last = middle + (high - middle) * keep;
        if last - first < 1e-6 {
            value = select(0.0, 1.0, value >= middle);
        } else {
            value = clamp((value - first) / (last - first), 0.0, 1.0);
        }
    }
    return clamp(select(value, 1.0 - value, effect.ids.w != 0u) + effect.projection.y, 0.0, 1.0);
}
