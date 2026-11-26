// The wear, section and light controls (compose's wear.rs, wear_scan.rs and sections.rs), of any
// kind. An operation on opcodes 40..46 with a side-table offset in ext.x computes with them;
// gpu_generator_p1.rs packs its words (the P1_* offsets below).

const P1_WEAR: u32 = 1u;
const P1_SECTION: u32 = 2u;
const P1_LIGHT: u32 = 3u;

// Words of every controls operation
const P1_MODEL: u32 = 0u;
const P1_FLAGS: u32 = 1u;
const P1_SOURCE_COUNT: u32 = 3u;
// The source of each role (relative index plus one, 0: none): wear curvature and occlusion,
// the section's signal input, the light's normal and position
const P1_ROLES: u32 = 4u;
const P1_PARAMETERS: u32 = 7u;

// P1_FLAGS bits: the section adjusts after the generator's stage, the blurred curvature levels apply
const P1_LATE_ADJUST: u32 = 1u;
const P1_LEVELS: u32 = 2u;
// The section adjusts between two stages; the bits from P1_STAGE_DISTANCE_SHIFT count the stages
// from the first to the second (gpu_generator_p1.rs)
const P1_TWO_STAGES: u32 = 4u;
const P1_STAGE_DISTANCE_SHIFT: u32 = 8u;
// The most sources an operation reads
const P1_MAX_SOURCES: u32 = 2u;
// The narrower curvature level when it is the map itself
const P1_NO_LEVEL: u32 = 0xffffffffu;

// Wear variants (P1_PARAMETERS + 0)
const P1_EDGE_WEAR: u32 = 0u;
const P1_FIBER_GLASS: u32 = 1u;
const P1_DIRT: u32 = 2u;

fn p1_word(op: Operation, index: u32) -> u32 {
    return srgb[op.ext.x + index];
}

fn p1_param(op: Operation, index: u32) -> f32 {
    return bitcast<f32>(srgb[op.ext.x + index]);
}

// The operation's sources (replaced inputs) for generator_read_sources, which reads every source
// of a generator at one fill_value call site
fn p1_source_ops(op: Operation, count: u32) -> u32 {
    if op.ids.y > 46u || op.ext.x == 0u {
        return count;
    }
    var added = count;
    let sources = min(p1_word(op, P1_SOURCE_COUNT), P1_MAX_SOURCES);
    for (var i = 0u; i < sources; i++) {
        added = generator_add_read(op.effects.z + i, added);
    }
    return added;
}

// The operation's sources as generator_read_sources read them
fn p1_sources(op: Operation, texel: LayerTexel) -> array<vec4<f32>, 2> {
    var values = array<vec4<f32>, 2>(vec4(0.0), vec4(0.0));
    let count = min(p1_word(op, P1_SOURCE_COUNT), P1_MAX_SOURCES);
    for (var i = 0u; i < count; i++) {
        values[i] = vec4(generator_read(op.effects.z + i), 0.0);
    }
    return values;
}

// Source of role `role`, or `fallback` without one
fn p1_role(op: Operation, sources: array<vec4<f32>, 2>, role: u32, fallback: vec4<f32>) -> vec4<f32> {
    let slot = p1_word(op, P1_ROLES + role);
    var read = sources;
    return select(fallback, read[min(max(slot, 1u), P1_MAX_SOURCES) - 1u], slot != 0u);
}

// This texel's world position across the mesh's bounds, 0..1 on each axis (0 where uncovered), as
// compose's bounds_position: low.xyz + position × high.xyz (generator_p3a::bind_bounds)
fn generator_bounds_position(op: Operation, texel: LayerTexel) -> vec3<f32> {
    if !texel.surface.valid {
        return vec3(0.0);
    }
    return clamp(op.low.xyz + texel.surface.position * op.high.xyz, vec3(0.0), vec3(1.0));
}

// The generator's world normal (Texel::generator_input_color of World Normal): World Space
// Normals' input, else the stored world normal when baked, else the surface's
fn p1_normal(op: Operation, texel: LayerTexel, sources: array<vec4<f32>, 2>) -> vec3<f32> {
    let own_input = p1_word(op, P1_MODEL) == P1_SECTION && p1_word(op, P1_PARAMETERS + 17u) == 3u && p1_word(op, P1_ROLES) != 0u;
    if own_input {
        return p1_role(op, sources, 0u, vec4(0.0)).xyz * 2.0 - 1.0;
    }
    if bake_has(BAKED_WORLD_NORMAL) && udim_bake_valid(texel.pixel) {
        return bitcast<vec3<f32>>(baked_block_at(texel.pixel, 1u).yzw) * 2.0 - 1.0;
    }
    return texel.surface.normal;
}

// The generator's Position input, else the position across the mesh's bounds
fn p1_position(op: Operation, texel: LayerTexel, sources: array<vec4<f32>, 2>, role: u32) -> vec3<f32> {
    if p1_word(op, P1_ROLES + role) != 0u {
        return p1_role(op, sources, role, vec4(0.0)).xyz;
    }
    return generator_bounds_position(op, texel);
}

// The signal before the section adjustment (wear and light values are final)
fn p1_generator_ramp(op: Operation, texel: LayerTexel) -> f32 {
    let sources = p1_sources(op, texel);
    var value = 0.0;
    switch p1_word(op, P1_MODEL) {
        case 1u: { value = p1_wear(op, texel, sources); }
        case 3u: { value = p1_light(op, texel, sources); }
        default: {
            // The section adjusts in the second of two stages (from the first's field), after its one
            // stage (in p1_finish_signal), or here; P2's texture blends over the adjusted signal
            let flags = p1_word(op, P1_FLAGS);
            var adjust = (flags & P1_LATE_ADJUST) == 0u;
            if (flags & P1_TWO_STAGES) != 0u && generator_second_stage {
                let second = mask_base(layers[params.filter_stage.z]) - 1u + op.spatial.y - 1u;
                value = filter_value(second - (flags >> P1_STAGE_DISTANCE_SHIFT), texel.pixel).x;
            } else {
                value = p1_section_signal(op, texel, sources);
                adjust = adjust && (flags & P1_TWO_STAGES) == 0u;
            }
            if adjust {
                value = generator_texture(op, texel, p1_adjusted(op, value), generator_bounds_position(op, texel), p1_normal(op, texel, sources));
            }
        }
    }
    return value;
}

// A ramp read back from the generator's stage, adjusted when its section acts after the stage
fn p1_finish_signal(op: Operation, ramp: f32) -> f32 {
    if op.ids.y > 46u || op.ext.x == 0u || (p1_word(op, P1_FLAGS) & P1_LATE_ADJUST) == 0u {
        return ramp;
    }
    // A texture runs a second stage instead (gpu_generator_p1.rs), so none blends here
    return p1_adjusted(op, ramp);
}

// Wear: compose's wear_ramp and wear_value
fn p1_wear(op: Operation, texel: LayerTexel, sources: array<vec4<f32>, 2>) -> f32 {
    let base = P1_PARAMETERS;
    let variant = p1_word(op, base);
    // The inputs with what P2's micro details add
    let read = vec3(p1_role(op, sources, 1u, vec4(texel.baked.x)).x, p1_role(op, sources, 0u, vec4(texel.baked.y)).x, texel.baked.z);
    let inputs = generator_micro(op, texel.pixel, read);
    let curvature = inputs.y - 0.5;
    let occlusion = inputs.x;
    let blurred = p1_blurred_curvature(op, texel.pixel, curvature);
    let grunge = generator_grunge(op, texel, generator_bounds_position(op, texel), p1_normal(op, texel, sources));

    let level = p1_param(op, base + 4u);
    let contrast = p1_param(op, base + 5u);
    let amount = p1_param(op, base + 6u);
    let ao_masking = p1_param(op, base + 7u);
    let curvature_weight = p1_param(op, base + 8u);
    let edges_masking = p1_param(op, base + 9u);
    let smoothed = (1.0 - 0.5) * curvature + 0.5 * blurred;
    var signal = 0.0;
    var scan = vec4(0.0);
    switch variant {
        case 2u: {
            signal = 0.486 * (0.869 - occlusion) - edges_masking * 0.817 * max(blurred, 0.0) + 0.203 * max(-blurred, 0.0) + 0.241 * amount
                + 0.0895 * amount * grunge;
            scan = vec4(0.593, -0.088, 1.386, 1.42);
        }
        case 1u: {
            let edges = smoothed * curvature_weight * 1.02;
            let open = p1_open_area(occlusion, ao_masking, vec3(0.144, 0.726, 0.669));
            signal = (edges * (1.0 - 0.989 * amount) + 0.167 * amount + 0.094 * p1_power(amount, 1.64) * grunge) * open;
            scan = vec4(0.162, 0.029, 0.358, 0.872);
        }
        default: {
            let edges = smoothed * curvature_weight;
            let open = p1_open_area(occlusion, ao_masking, vec3(-0.779, 2.244, 0.935));
            signal = edges * open * (1.0 + 0.031 * amount) - 0.009 * amount + 0.057 * p1_power(amount, 0.731) * grunge;
            scan = vec4(0.284, -0.009, 0.504, 0.95);
        }
    }

    // WearScan::apply: the level brings the edge down and narrows it, the contrast narrows it further
    let remaining = max(1.0 - level, 0.0);
    let width = max(remaining * scan.z * p1_power(max(1.0 - contrast, 0.0), scan.w), 1e-4);
    return clamp((signal - scan.y - remaining * scan.x) / width + 0.5, 0.0, 1.0);
}

// `value` to the power `exponent` (0 at 0, which WGSL's pow leaves undefined)
fn p1_power(value: f32, exponent: f32) -> f32 {
    return select(0.0, pow(value, exponent), value > 0.0);
}

// 1 where the area is open enough to wear (compose's open_area)
fn p1_open_area(occlusion: f32, masking: f32, fit: vec3<f32>) -> f32 {
    let fully_open = -fit.z * 0.5;
    var cut_off = fit.x + fit.y * masking;
    if masking < 0.5 && fit.x < fully_open {
        cut_off = fully_open + (fit.x + fit.y * 0.5 - fully_open) * masking / 0.5;
    }
    return clamp((occlusion - cut_off) / fit.z + 0.5, 0.0, 1.0);
}

// The curvature deviation blurred between the two levels around the wear's sigma (compose's blurred_curvature)
fn p1_blurred_curvature(op: Operation, pixel: vec2<u32>, curvature: f32) -> f32 {
    if (p1_word(op, P1_FLAGS) & P1_LEVELS) == 0u || !udim_bake_valid(pixel) {
        return curvature;
    }
    let base = P1_PARAMETERS;
    let narrower_level = p1_word(op, base + 1u);
    let wider_level = p1_word(op, base + 2u);
    let weight = p1_param(op, base + 3u);
    let bake_pixel = bake_pixel_at(pixel);
    var narrower = curvature;
    if narrower_level != P1_NO_LEVEL {
        if !curvature_level_present(narrower_level) {
            return curvature;
        }
        narrower = generator_level_at(op, bake_pixel, narrower_level);
    }
    if !curvature_level_present(wider_level) {
        return curvature;
    }
    let wider = generator_level_at(op, bake_pixel, wider_level);
    return narrower + (wider - narrower) * weight;
}

// The section's signal: curvature at its detail sizes, occlusion, the axes of the position or
// the world normal, or 0 for the kinds without one (P1_PARAMETERS + 17)
fn p1_section_signal(op: Operation, texel: LayerTexel, sources: array<vec4<f32>, 2>) -> f32 {
    let base = P1_PARAMETERS;
    switch p1_word(op, base + 17u) {
        case 0u: {
            let replaced = p1_word(op, P1_ROLES) != 0u;
            let finest = generator_micro(op, texel.pixel, vec3(1.0, p1_role(op, sources, 0u, vec4(texel.baked.y)).x, 1.0)).y;
            var detail = op;
            detail.uv.x = p1_param(op, base + 6u);
            detail.effects.w = select(0u, 1u, replaced);
            return generator_curvature(detail, texel.pixel, finest, p1_word(op, base + 7u), 0.0);
        }
        case 1u: {
            return generator_micro(op, texel.pixel, vec3(p1_role(op, sources, 0u, vec4(texel.baked.x)).x, 0.5, 1.0)).x;
        }
        case 2u: {
            return p1_axes_signal(op, p1_position(op, texel, sources, 0u));
        }
        case 3u: {
            // The stored world normal when baked, else the surface's, as 0..1 channels
            var channels = texel.surface.normal * 0.5 + 0.5;
            if p1_word(op, P1_ROLES) != 0u {
                channels = p1_role(op, sources, 0u, vec4(0.0)).xyz;
            } else if bake_has(BAKED_WORLD_NORMAL) && udim_bake_valid(texel.pixel) {
                channels = bitcast<vec3<f32>>(baked_block_at(texel.pixel, 1u).yzw);
            }
            return p1_axes_signal(op, channels);
        }
        default: {
            return 0.0;
        }
    }
}

// The three channels combined by the section's axes (compose's axes_signal)
fn p1_axes_signal(op: Operation, channels: vec3<f32>) -> f32 {
    let base = P1_PARAMETERS + 8u;
    var value = 0.0;
    // Top to bottom is the base, front to back goes last
    let order = vec3(1u, 0u, 2u);
    for (var step = 0u; step < 3u; step++) {
        let axis = order[step];
        let invert = p1_word(op, base + axis * 3u) != 0u;
        let blend = p1_word(op, base + axis * 3u + 1u);
        let weight = p1_param(op, base + axis * 3u + 2u);
        if weight <= 0.0 {
            continue;
        }
        let layer = select(channels[axis], 1.0 - channels[axis], invert);
        var blended = layer;
        switch blend {
            case 1u: { blended = value + layer; }
            case 2u: { blended = value - layer; }
            case 3u: { blended = value * layer; }
            case 4u: { blended = value + 2.0 * layer - 1.0; }
            case 5u: { blended = max(value, layer); }
            default: {}
        }
        value = clamp(value + (blended - value) * weight, 0.0, 1.0);
    }
    return value;
}

// The section's balance, contrast, brightness and invert (compose's sections::adjusted)
fn p1_adjusted(op: Operation, signal: f32) -> f32 {
    let base = P1_PARAMETERS;
    let flags = p1_word(op, base);
    var value = clamp(signal, 0.0, 1.0);
    if (flags & 1u) != 0u {
        let balance = p1_param(op, base + 1u);
        if (flags & 2u) != 0u {
            value = p1_balanced(value, balance);
        } else {
            value = p1_power(value, p1_param(op, base + 2u));
        }
    }

    let contrast = p1_param(op, base + 3u);
    if contrast > 0.0 {
        let half_range = 0.5 * (1.0 - contrast);
        if 2.0 * half_range < 1e-6 {
            value = select(0.0, 1.0, value > 0.5 + 4.6e-5);
        } else {
            value = clamp((value - (0.5 - half_range)) / (2.0 * half_range), 0.0, 1.0);
        }
    }

    if (flags & 4u) != 0u {
        let brightness = p1_param(op, base + 4u);
        value = select(value * (1.0 + brightness), value + brightness * (1.0 - value), brightness > 0.0);
    }
    return select(value, 1.0 - value, p1_word(op, base + 5u) != 0u);
}

// GeneratorGlobal::apply with a balance alone
fn p1_balanced(value: f32, balance: f32) -> f32 {
    if balance == 0.5 {
        return value;
    }
    let low = max(1.0 - 2.0 * balance, 0.0);
    let high = min(2.0 - 2.0 * balance, 1.0);
    if high - low < 1e-6 {
        return select(0.0, 1.0, value >= (low + high) * 0.5);
    }
    return clamp((value - low) / (high - low), 0.0, 1.0);
}

// The Light generator (compose's light_value)
fn p1_light(op: Operation, texel: LayerTexel, sources: array<vec4<f32>, 2>) -> f32 {
    let base = P1_PARAMETERS;
    var normal = texel.surface.normal;
    if p1_word(op, P1_ROLES) != 0u {
        normal = p1_role(op, sources, 0u, vec4(0.0)).xyz * 2.0 - 1.0;
    } else if bake_has(BAKED_WORLD_NORMAL) && udim_bake_valid(texel.pixel) {
        normal = bitcast<vec3<f32>>(baked_block_at(texel.pixel, 1u).yzw) * 2.0 - 1.0;
    }
    let position = p1_position(op, texel, sources, 1u);
    let direction = vec3(p1_param(op, base), p1_param(op, base + 1u), p1_param(op, base + 2u));
    let length_of_normal = length(normal);
    normal = select(vec3(0.0, 1.0, 0.0), normal * (1.0 / length_of_normal), length_of_normal > 1e-6);

    // Glossiness 0 lights the half facing away too, 0.5 is a plain diffuse falloff
    let threshold = p1_param(op, base + 3u);
    var diffuse = clamp((dot(normal, direction) - threshold) / p1_param(op, base + 4u), 0.0, 1.0);
    let highlight_level = p1_param(op, base + 5u);
    if highlight_level >= 1.0 {
        diffuse = select(0.0, 1.0, diffuse > 0.0);
    } else {
        diffuse = clamp(diffuse / p1_param(op, base + 6u), 0.0, 1.0);
    }

    // Depth from the tangent plane facing the light
    let depth = 0.5 - dot(position - vec3(0.5), direction);
    let attenuation = p1_param(op, base + 7u);
    var kept = clamp(1.0 - depth, 0.0, 1.0) * (2.0 - 2.0 * attenuation);
    if attenuation <= 0.5 {
        kept = clamp(1.0 - 2.0 * attenuation * depth, 0.0, 1.0);
    }
    return kept * select(diffuse, 1.0 - diffuse, p1_word(op, base + 8u) != 0u);
}
