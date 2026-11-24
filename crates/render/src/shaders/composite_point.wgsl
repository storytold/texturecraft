// P2, point filters and gradients (gpu_composite_point.rs). Owned by the P2 agent.
// Compiled into the composite shader when the program needs it, else replaced by the stub
// string in the Rust module (keep both signatures identical).

// Opcodes of P2's block that the loop-level hooks test (the constants in gpu_composite_point.rs)
const POINT_POSTERIZE_ALPHA: u32 = 129u;
const POINT_GRAYSCALE_CONVERSION: u32 = 136u;
const POINT_GRADIENT: u32 = 137u;

// A gradient's flags in Operation.ext.z (the constants in gpu_composite_point.rs)
const POINT_GRADIENT_EVEN_LINEAR: u32 = 1u;
const POINT_GRADIENT_SRGB: u32 = 2u;
const POINT_GRADIENT_NEUTRAL_INPUT: u32 = 4u;
const POINT_GRADIENT_DEBUG: u32 = 8u;
const POINT_GRADIENT_BLURRED: u32 = 16u;

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
        // Folded into -π..π, where GPU sines are accurate; |sin(2πt - π)| = |sin(2πt)|
        case 10u: { y = abs(sin(2.0 * POINT_PI * t - POINT_PI)); }
        default: { y = 1.0 - abs((2.0 * POINT_TRIANGULAR_TEETH * t) % 2.0 - 1.0); }
    }
    return clamp(y, 0.0, 1.0);
}

// A colour's gray: 0 desaturation, 1 luma, 2 average, 3 max, 4 min, else weights of red, green, blue and alpha
fn point_gray(mode: u32, rgb: vec3<f32>, alpha: f32, weights: vec4<f32>) -> f32 {
    switch mode {
        case 0u: { return (max(rgb.r, max(rgb.g, rgb.b)) + min(rgb.r, min(rgb.g, rgb.b))) / 2.0; }
        case 1u: { return point_luma(rgb); }
        case 2u: { return (rgb.r + rgb.g + rgb.b) / 3.0; }
        case 3u: { return max(rgb.r, max(rgb.g, rgb.b)); }
        case 4u: { return min(rgb.r, min(rgb.g, rgb.b)); }
        default: { return rgb.r * weights.x + rgb.g * weights.y + rgb.b * weights.z + alpha * weights.w; }
    }
}

// Grayscale Conversion's shaped gray: ids.z the gray, high the weights, low.xyz balance, contrast and 1 to invert
fn point_conversion_gray(op: Operation, rgb: vec3<f32>, alpha: f32) -> f32 {
    return point_shape_gray(point_gray(op.ids.z, rgb, alpha, op.high), op.low.x, op.low.y, op.low.z > 0.5);
}

// Grayscale Conversion on a channel whose coverage is alpha (GrayscaleConversion::channel_value in Rust)
fn point_conversion_channel(op: Operation, c: vec3<f32>, channel: u32, alpha: f32) -> vec3<f32> {
    if channel == POINT_NORMAL_SLOT || channel == POINT_COAT_NORMAL_SLOT {
        return c;
    }
    if point_is_color(channel) {
        return vec3(point_conversion_gray(op, c, alpha));
    }
    if channel == POINT_HEIGHT_SLOT {
        return vec3(point_conversion_gray(op, vec3(c.x * 0.5 + 0.5), alpha) * 2.0 - 1.0);
    }
    return vec3(point_conversion_gray(op, vec3(c.x), alpha));
}

// Grayscale Conversion's Custom Input at the texel: its source operation (effects.w, register_input) as a colour
fn point_custom_input(op: Operation, texel: LayerTexel) -> vec3<f32> {
    return effect_input_color(op, texel);
}

// Grayscale Conversion of a Custom Input onto a channel (GrayscaleConversion::channel_value with an input)
fn point_conversion_input(op: Operation, input: vec3<f32>, c: vec3<f32>, channel: u32) -> vec3<f32> {
    if channel == POINT_NORMAL_SLOT || channel == POINT_COAT_NORMAL_SLOT {
        return c;
    }

    let gray = point_conversion_gray(op, input, 1.0);
    if channel == POINT_HEIGHT_SLOT {
        return vec3(gray * 2.0 - 1.0);
    }
    return vec3(gray);
}

// The linear value of an sRGB component (srgb_to_linear in Rust)
fn point_srgb_to_linear(v: f32) -> f32 {
    let x = clamp(v, 0.0, 1.0);
    if x <= 0.04045 {
        return x / 12.92;
    }
    return pow((x + 0.055) / 1.055, 2.4);
}

// Stop index of a gradient in the side table: its place, then its colour
fn point_stop(op: Operation, index: u32) -> vec4<f32> {
    let at = index * 4u;
    return vec4(param(op, at), param(op, at + 1u), param(op, at + 2u), param(op, at + 3u));
}

// A gradient's colour at t from its procedural.z stops (at least one); evenly spaced linear ones read as opcode 33 does
fn point_ramp(op: Operation, t: f32) -> vec3<f32> {
    if (op.ext.z & POINT_GRADIENT_EVEN_LINEAR) != 0u {
        return gradient_color(op, t);
    }

    // The last stop at or before t and the first one after it; the stops are in place order
    let count = op.procedural.z;
    let position = filter_unit(t);
    var split = 0u;
    for (var index = 0u; index < count; index++) {
        if param(op, index * 4u) <= position {
            split = index + 1u;
        }
    }
    let before = point_stop(op, max(split, 1u) - 1u);
    let after = point_stop(op, min(split, count - 1u));

    var share = 0.0;
    if after.x > before.x {
        share = clamp((position - before.x) / (after.x - before.x), 0.0, 1.0);
    }
    let color = before.yzw + (after.yzw - before.yzw) * share;
    if (op.ext.z & POINT_GRADIENT_SRGB) == 0u {
        return color;
    }
    return vec3(point_srgb_to_linear(color.x), point_srgb_to_linear(color.y), point_srgb_to_linear(color.z));
}

// The ramp position of gray x after the gradient's balance, contrast and invert (GradientInput::shape in Rust)
fn point_gradient_position(op: Operation, x: f32) -> f32 {
    if (op.ext.z & POINT_GRADIENT_NEUTRAL_INPUT) != 0u {
        return x;
    }
    return point_shape_gray(x, op.low.x, op.low.y, op.low.z > 0.5);
}

// A gradient on a gray (a non-colour channel or a mask): its ramp's luminance; a ramp without colours leaves the gray
fn point_gradient_gray(op: Operation, gray: f32) -> f32 {
    if op.procedural.z == 0u {
        return gray;
    }
    return point_luma(point_ramp(op, point_gradient_position(op, gray)));
}

// A gradient after its input blur stage: gray is the stage's blurred gray (word 4), c the unblurred content (map_blurred in Rust)
fn point_blurred_gradient(op: Operation, c: vec3<f32>, channel: u32, gray: f32) -> vec3<f32> {
    // The CPU blurs a normal map's own value here, which the stage does not carry; the effect leaves normals as they are
    if channel == POINT_NORMAL_SLOT || channel == POINT_COAT_NORMAL_SLOT {
        return c;
    }
    if point_is_color(channel) {
        if op.procedural.z == 0u {
            return c;
        }
        return point_ramp(op, point_gradient_position(op, gray));
    }
    if channel == POINT_HEIGHT_SLOT {
        return vec3(point_gradient_gray(op, gray * 0.5 + 0.5) * 2.0 - 1.0);
    }
    return vec3(point_gradient_gray(op, gray));
}

// A gradient whose input blur ran as the preceding stage
fn point_is_gradient_blurred(op: Operation) -> bool {
    return op.ids.y == POINT_GRADIENT && (op.ext.z & POINT_GRADIENT_BLURRED) != 0u;
}

// A gradient's debug view: its ramp left to right across each UV tile in place of the mapped values
fn point_is_gradient_debug(op: Operation) -> bool {
    return op.ids.y == POINT_GRADIENT && (op.ext.z & POINT_GRADIENT_DEBUG) != 0u;
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
        // Grayscale Conversion: a mask weighs alpha as 1
        case 136u: { return point_conversion_gray(op, vec3(x), 1.0); }
        // A gradient on a gray reads its ramp's luminance; a ramp without colours leaves the gray
        case 137u: { return point_gradient_gray(op, x); }
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
            case 136u: { return vec3(point_conversion_gray(op, c, 1.0)); }
            // A gradient on a colour: ids.z the gray it reads
            case 137u: {
                if op.procedural.z == 0u { return c; }
                return point_ramp(op, point_gradient_position(op, point_gray(op.ids.z, c, 1.0, vec4(0.0))));
            }
            default: {}
        }
    }
    return point_per_unit(op, c, channel);
}

// A point opcode in the layer_content loop: the content after the effect, opacity applied. The
// default mixes point_effect_color by the opacity; take over for coverage or texel reads.
fn point_content_effect(effect: Operation, content: vec4<f32>, channel: u32, texel: LayerTexel) -> vec4<f32> {
    var filtered: vec3<f32>;
    if effect.ids.y == POINT_GRAYSCALE_CONVERSION && effect.effects.w != 0u {
        // A Custom Input gives its colour's gray, its alpha weighed as 1
        filtered = point_conversion_input(effect, point_custom_input(effect, texel), content.rgb, channel);
    } else if effect.ids.y == POINT_GRAYSCALE_CONVERSION {
        // Grayscale Conversion weighs the content's coverage as its alpha
        filtered = point_conversion_channel(effect, content.rgb, channel, content.a);
    } else if point_is_gradient_debug(effect) {
        // Every channel but a colour shows the ramp's luminance as it is, heights and normals too
        if effect.procedural.z == 0u {
            return content;
        }
        let ramp = point_ramp(effect, fract(texel.uv.x));
        filtered = select(vec3(point_luma(ramp)), ramp, point_is_color(channel));
    } else if point_is_gradient_blurred(effect) {
        // The stage keeps the unblurred texel, so the mix and the coverage stay the original's
        filtered = point_blurred_gradient(effect, content.rgb, channel, preceding_stage_word(texel.pixel));
    } else {
        filtered = point_effect_color(effect, content.rgb, channel, texel.surface, texel.baked.x);
    }
    var coverage = content.a;
    if effect.ids.y == POINT_POSTERIZE_ALPHA {
        coverage += (point_posterize(effect.low.x, content.a) - content.a) * effect.low.w;
    }
    return vec4(content.rgb + (filtered - content.rgb) * effect.low.w, coverage);
}

// A point opcode in the layer_mask loop: the result the loop mixes in by the opacity.
fn point_mask_effect(effect: Operation, mask: f32, texel: LayerTexel) -> f32 {
    if effect.ids.y == POINT_GRAYSCALE_CONVERSION && effect.effects.w != 0u {
        return point_conversion_gray(effect, point_custom_input(effect, texel), 1.0);
    }
    if point_is_gradient_debug(effect) {
        if effect.procedural.z == 0u {
            return mask;
        }
        return point_luma(point_ramp(effect, fract(texel.uv.x)));
    }
    if point_is_gradient_blurred(effect) {
        return point_gradient_gray(effect, preceding_stage_word(texel.pixel));
    }
    return point_scalar_filter(effect, mask);
}
