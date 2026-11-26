fn generator_ramp(effect: Operation, texel: LayerTexel) -> f32 {
    // Every source the generator reads, at one call site (generator_read_sources)
    generator_read_sources(effect, texel);
    var input = vec3(0.0);
    if effect.effects.w != 0u {
        input = generator_read(effect.effects.w);
    }
    // P1: the wear, section and light controls (generator_p1.wgsl)
    if effect.ids.y <= 46u && effect.ext.x != 0u {
        return p1_generator_ramp(effect, texel);
    }
    // The generators with their own controls: P3b's whole-map kinds read their field (5 and up), P3a's the texel's inputs
    if effect.ids.y == GENERATOR_KIND_OPCODE {
        if effect.procedural.x >= 5u {
            return p3b_field_ramp(effect, texel.pixel);
        }
        return generator_p3a_ramp(effect, texel, input);
    }
    let surface = texel.surface;
    var baked = texel.baked;
    if effect.effects.w != 0u {
        switch effect.ids.y {
            case 40u, 43u: { baked.y = input.x; }
            case 41u, 44u: { baked.x = input.x; }
            case 42u: { baked.z = input.x; }
            default: {}
        }
    }
    // P2: what the generator's micro details add to its curvature and occlusion
    baked = generator_micro(effect, texel.pixel, baked);
    let detail =select(0u, u32(effect.uv.y), enable_curvature_detail);
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
    var value = p1_finish_signal(effect, ramp);
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

// The most sources one generator reads: P1's replaced inputs, or an earlier or P3a input, and
// P2's texture slot and grunge (its pattern and breakup, or its custom slot)
const GENERATOR_MAX_READS: u32 = 6u;
var<private> generator_read_ops: array<u32, 6>;
var<private> generator_read_values: array<vec3<f32>, 6>;
var<private> generator_read_count: u32 = 0u;

// Read every source operation `effect` reads in one loop: the driver inlines fill_value at each
// call site, and with anchor reads more copies overflow lavapipe's shader stack
fn generator_read_sources(effect: Operation, texel: LayerTexel) {
    var count = 0u;
    if effect.effects.w != 0u {
        generator_read_ops[count] = effect.effects.w;
        count++;
    }
    count = p1_source_ops(effect, count);
    count = p2_source_ops(effect, count);
    generator_read_count = min(count, GENERATOR_MAX_READS);
    for (var i = 0u; i < generator_read_count; i++) {
        generator_read_values[i] = generator_source_value(generator_read_ops[i] - 1u, texel);
    }
}

// Append source operation `op` (index plus one) to the reads, unless the table is full
fn generator_add_read(op: u32, count: u32) -> u32 {
    if op == 0u || count >= GENERATOR_MAX_READS {
        return count;
    }
    generator_read_ops[count] = op;
    return count + 1u;
}

// What source operation `op` (index plus one) read at this texel (generator_read_sources)
fn generator_read(op: u32) -> vec3<f32> {
    for (var i = 0u; i < generator_read_count; i++) {
        if generator_read_ops[i] == op {
            return generator_read_values[i];
        }
    }
    return vec3(0.0);
}
