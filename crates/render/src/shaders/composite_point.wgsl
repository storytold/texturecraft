// P2, point filters and gradients (gpu_composite_point.rs). Owned by the P2 agent.
// Compiled into the composite shader when the program needs it, else replaced by the stub
// string in the Rust module (keep both signatures identical).

// Opcodes of P2's block that the loop-level hooks test (the constants in gpu_composite_point.rs)
const POINT_POSTERIZE_ALPHA: u32 = 129u;

// Histogram Scan's narrowest soft cut (SCAN_MIN_WIDTH in Rust)
const POINT_SCAN_MIN_WIDTH: f32 = 1e-3;

// Destination slots that hold no colour (channel_slot in Rust)
const POINT_HEIGHT_SLOT: u32 = 6u;
const POINT_NORMAL_SLOT: u32 = 7u;
const POINT_COAT_NORMAL_SLOT: u32 = 11u;

// Base Color and the colour lobes (ChannelKind::Color in Rust)
fn point_is_color(channel: u32) -> bool {
    return channel == 0u || (channel >= 8u && channel <= 10u);
}

// The luminance weights of lum in Rust
fn point_luma(c: vec3<f32>) -> f32 {
    return 0.3 * c.r + 0.59 * c.g + 0.11 * c.b;
}

// The scalar form on each component: signed values through 0..1, normals left alone (per_unit in Rust)
fn point_per_unit(op: Operation, c: vec3<f32>, channel: u32) -> vec3<f32> {
    if channel == POINT_NORMAL_SLOT || channel == POINT_COAT_NORMAL_SLOT {
        return c;
    }
    if point_is_color(channel) {
        return vec3(point_scalar_filter(op, c.r), point_scalar_filter(op, c.g), point_scalar_filter(op, c.b));
    }
    if channel == POINT_HEIGHT_SLOT {
        return vec3(point_scalar_filter(op, c.x * 0.5 + 0.5) * 2.0 - 1.0);
    }
    return vec3(point_scalar_filter(op, c.x));
}

// WGSL has no cube root; the OKLab cone responses of clamped colours are never negative
fn point_cube_root(x: f32) -> f32 {
    return select(0.0, pow(x, 1.0 / 3.0), x > 0.0);
}

// perceptive_adjust in Rust: op.low.yz saturation and lightness, op.high.xy sine and cosine of the hue turn
fn point_perceptive_adjust(op: Operation, c: vec3<f32>) -> vec3<f32> {
    let rgb = vec3(filter_unit(c.r), filter_unit(c.g), filter_unit(c.b));
    let l = point_cube_root(0.41222146 * rgb.r + 0.53633255 * rgb.g + 0.051445995 * rgb.b);
    let m = point_cube_root(0.2119035 * rgb.r + 0.6806995 * rgb.g + 0.10739696 * rgb.b);
    let s = point_cube_root(0.08830246 * rgb.r + 0.28171885 * rgb.g + 0.6299787 * rgb.b);
    let lightness = 0.21045426 * l + 0.7936178 * m - 0.004072047 * s;
    let a = 1.9779985 * l - 2.4285922 * m + 0.4505937 * s;
    let b = 0.025904037 * l + 0.78277177 * m - 0.80867577 * s;

    let chroma = 1.0 + clamp(op.low.y, -1.0, 1.0);
    let turned_a = (a * op.high.y - b * op.high.x) * chroma;
    let turned_b = (a * op.high.x + b * op.high.y) * chroma;
    let shifted = shift_unit(clamp(lightness, 0.0, 1.0), op.low.z);

    let cone_l = shifted + 0.39633778 * turned_a + 0.21580376 * turned_b;
    let cone_m = shifted - 0.105561346 * turned_a - 0.06385417 * turned_b;
    let cone_s = shifted - 0.08948418 * turned_a - 1.2914855 * turned_b;
    let l3 = cone_l * cone_l * cone_l;
    let m3 = cone_m * cone_m * cone_m;
    let s3 = cone_s * cone_s * cone_s;
    let red = 4.0767417 * l3 - 3.3077116 * m3 + 0.23096994 * s3;
    let green = -1.268438 * l3 + 2.6097574 * m3 - 0.34131938 * s3;
    let blue = -0.0041960863 * l3 - 0.7034186 * m3 + 1.7076147 * s3;
    return vec3(filter_unit(red), filter_unit(green), filter_unit(blue));
}

// The balance that keeps a gray, and the steepest soft contrast (gradient_input.rs)
const POINT_NEUTRAL_BALANCE: f32 = 0.5;
const POINT_MIN_STEEPNESS: f32 = 1e-3;

// Gradient Curve's Stairs steps and Triangular teeth (curve_preset.rs)
const POINT_STAIRS_STEPS: f32 = 4.0;
const POINT_TRIANGULAR_TEETH: f32 = 2.0;
const POINT_PI: f32 = 3.14159265358979;

// Smoothstep bounds closer than this are a step (MIN_SMOOTHSTEP_SPAN in Rust)
const POINT_MIN_SMOOTHSTEP_SPAN: f32 = 1e-6;

// Color Correct's [contrast, luminosity, saturation] at a lightness: shadows in low, midtones in high, highlights in projection
fn point_tone(op: Operation, lightness: f32) -> vec3<f32> {
    let l = filter_unit(lightness);
    return op.low.xyz * ((1.0 - l) * (1.0 - l)) + op.high.xyz * (2.0 * l * (1.0 - l)) + op.projection.xyz * (l * l);
}

// Color Correct's contrast around mid gray and luminosity shift
fn point_correct(x: f32, tone: vec3<f32>) -> f32 {
    return (x - 0.5) * (1.0 + clamp(tone.x, -1.0, 1.0)) + 0.5 + clamp(tone.y, -1.0, 1.0);
}

// Color Correct on a colour: each component corrected, then the saturation scales them around their gray
fn point_color_correct(op: Operation, c: vec3<f32>) -> vec3<f32> {
    let tone = point_tone(op, point_luma(c));
    let corrected = vec3(point_correct(c.r, tone), point_correct(c.g, tone), point_correct(c.b, tone));
    let gray = point_luma(corrected);
    let gain = 2.0 * clamp(tone.z, 0.0, 1.0);
    return vec3(filter_unit(gray + (corrected.r - gray) * gain), filter_unit(gray + (corrected.g - gray) * gain), filter_unit(gray + (corrected.b - gray) * gain));
}

// A gray balanced, contrasted around mid gray and maybe inverted (shape_gray in Rust)
fn point_shape_gray(x: f32, balance: f32, contrast: f32, invert: bool) -> f32 {
    let balanced = filter_unit(x) + filter_unit(balance) - POINT_NEUTRAL_BALANCE;
    let steepness = filter_unit(contrast);
    var shaped = select(0.0, 1.0, balanced >= POINT_NEUTRAL_BALANCE);
    if steepness != 1.0 {
        shaped = filter_unit((balanced - POINT_NEUTRAL_BALANCE) / max(1.0 - steepness, POINT_MIN_STEEPNESS) + POINT_NEUTRAL_BALANCE);
    }
    return select(shaped, 1.0 - shaped, invert);
}

// A Gradient Curve preset at t, by the shape curve_shape picks in Rust (CurvePreset::at)
fn point_curve(shape: u32, x: f32) -> f32 {
    let t = filter_unit(x);
    let peak = 1.0 - abs(2.0 * t - 1.0);
    var y = t;
    switch shape {
        case 0u: { y = sqrt(1.0 - (1.0 - t) * (1.0 - t)); }
        case 1u: { y = 1.0 - sqrt(1.0 - t * t); }
        case 2u: { y = min(floor(t * POINT_STAIRS_STEPS) / (POINT_STAIRS_STEPS - 1.0), 1.0); }
        case 3u: { y = 1.0 - (1.0 - t) * (1.0 - t); }
        case 4u: { y = t * t * (3.0 - 2.0 * t); }
        case 5u: { y = t * t; }
        case 6u: { y = sqrt(peak); }
        case 7u: { y = peak; }
        case 8u: { y = peak * peak; }
        case 9u: { y = sin(POINT_PI * t); }
        case 10u: { y = abs(sin(2.0 * POINT_PI * t)); }
        default: { y = 1.0 - abs((2.0 * POINT_TRIANGULAR_TEETH * t) % 2.0 - 1.0); }
    }
    return clamp(y, 0.0, 1.0);
}

// x in steps + 1 levels, as posterize in Rust
fn point_posterize(steps: f32, x: f32) -> f32 {
    return floor(filter_unit(x) * steps + 0.5) / steps;
}

// A P2 opcode (128..144) on a scalar channel or a mask value.
fn point_scalar_filter(op: Operation, x: f32) -> f32 {
    switch op.ids.y {
        // HSL Perceptive: grays and masks shift their lightness only, as opcode 31
        case 128u: { return shift_unit(filter_unit(x), op.low.z); }
        // Posterize with Apply to Alpha: the values as plain Posterize, low.x the steps
        case 129u: { return point_posterize(op.low.x, x); }
        // Histogram Scan from the other end: low.x is 1 - position, low.y the contrast
        case 130u: {
            if op.low.y >= 1.0 { return select(0.0, 1.0, x >= op.low.x); }
            let width = max(1.0 - clamp(op.low.y, 0.0, 1.0), POINT_SCAN_MIN_WIDTH);
            return filter_unit((x - op.low.x) / width + 0.5);
        }
        case 131u: { return filter_unit(point_correct(x, point_tone(op, x))); }
        // Smoothstep: low.xy the bounds in order
        case 132u: {
            let span = op.low.y - op.low.x;
            if span <= POINT_MIN_SMOOTHSTEP_SPAN { return select(0.0, 1.0, x >= op.low.y); }
            let t = clamp((x - op.low.x) / span, 0.0, 1.0);
            return t * t * (3.0 - 2.0 * t);
        }
        // Height Adjust: low.x 1 to invert, low.y the offset, low.z the multiplier
        case 133u: {
            let flipped = select(x, 1.0 - x, op.low.x > 0.5);
            return filter_unit((flipped - 0.5) * op.low.z + 0.5 + op.low.y);
        }
        // Gradient Curve: low.xyz balance, contrast and 1 to invert, ids.z the shape
        case 134u: { return point_curve(op.ids.z, point_shape_gray(x, op.low.x, op.low.y, op.low.z > 0.5)); }
        // Gradient Dynamic: high.x the colour's luminance
        case 135u: { return op.high.x; }
        default: { return x; }
    }
}

// A P2 opcode on a content channel; channel is the destination slot (0 Base Color, 6 Height,
// 7 Normal, 8..10 colour lobes; channel_slot in Rust).
fn point_effect_color(op: Operation, c: vec3<f32>, channel: u32, surface: Surface, baked_ao: f32) -> vec3<f32> {
    if point_is_color(channel) {
        switch op.ids.y {
            case 128u: { return point_perceptive_adjust(op, c); }
            case 131u: { return point_color_correct(op, c); }
            // Gradient Dynamic: low.xyz the colour
            case 135u: { return op.low.xyz; }
            default: {}
        }
    }
    return point_per_unit(op, c, channel);
}

// A point opcode in the layer_content loop: the content after the effect, opacity applied. The
// default mixes point_effect_color by the opacity; take over for coverage or texel reads.
fn point_content_effect(effect: Operation, content: vec4<f32>, channel: u32, texel: LayerTexel) -> vec4<f32> {
    let filtered = point_effect_color(effect, content.rgb, channel, texel.surface, texel.baked.x);
    var coverage = content.a;
    if effect.ids.y == POINT_POSTERIZE_ALPHA {
        coverage += (point_posterize(effect.low.x, content.a) - content.a) * effect.low.w;
    }
    return vec4(content.rgb + (filtered - content.rgb) * effect.low.w, coverage);
}

// A point opcode in the layer_mask loop: the result the loop mixes in by the opacity.
fn point_mask_effect(effect: Operation, mask: f32, texel: LayerTexel) -> f32 {
    return point_scalar_filter(effect, mask);
}
