// P3b: the whole-map generator fields gpu_generator_p3b.rs builds before the composite, one array
// layer per field (`Operation.procedural.y`), read at the texel and finished as eval.rs's
// generator_ramp finishes them.
@group(0) @binding(16) var generator_fields: texture_2d_array<f32>;

const P3B_BORDER_DISTANCE: u32 = 5u;
const P3B_DRIPPING_RUST: u32 = 6u;

// GeneratorGlobal::apply: the balance shifts the range, the contrast narrows it
fn p3b_balance_contrast(value: f32, balance: f32, contrast: f32) -> f32 {
    if balance == 0.5 && contrast == 0.0 {
        return value;
    }
    let balanced = clamp(balance, 0.0, 1.0);
    let low = max(1.0 - 2.0 * balanced, 0.0);
    let high = min(2.0 - 2.0 * balanced, 1.0);
    let middle = (low + high) * 0.5;
    let keep = 1.0 - clamp(contrast, 0.0, 1.0);
    let first = middle - (middle - low) * keep;
    let last = middle + (high - middle) * keep;
    if last - first < 1e-6 {
        return select(0.0, 1.0, value >= middle);
    }
    return clamp((value - first) / (last - first), 0.0, 1.0);
}

// histogram_scan.rs: a ramp `1 - contrast` wide slid through the values by the position
fn p3b_histogram_scan(value: f32, position: f32, contrast: f32) -> f32 {
    if position <= 0.0 { return 0.0; }
    if position >= 1.0 { return 1.0; }
    let width = 1.0 - clamp(contrast, 0.0, 1.0);
    let low = (1.0 - position) * (1.0 + width) - width;
    if width <= 1.1920929e-7 {
        return select(0.0, 1.0, value >= low);
    }
    return clamp((value - low) / width, 0.0, 1.0);
}

fn p3b_field_ramp(effect: Operation, pixel: vec2<u32>) -> f32 {
    let extent = textureDimensions(generator_fields);
    if any(pixel >= extent) {
        return 0.0;
    }
    let field = textureLoad(generator_fields, vec2<i32>(pixel), i32(effect.procedural.y), 0).x;
    switch effect.procedural.x {
        case P3B_BORDER_DISTANCE: { return p3b_balance_contrast(field, effect.high.x, effect.high.y); }
        case P3B_DRIPPING_RUST: { return p3b_histogram_scan(field, effect.high.x, effect.high.y); }
        default: { return field; }
    }
}
