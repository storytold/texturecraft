// P2, point filters and gradients (gpu_composite_point.rs). Owned by the P2 agent.
// Compiled into the composite shader when the program needs it, else replaced by the stub
// string in the Rust module (keep both signatures identical).

// Opcodes of P2's block (the constants in gpu_composite_point.rs)
const POINT_HSL_PERCEPTIVE: u32 = 128u;

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

// A P2 opcode (128..144) on a scalar channel or a mask value.
fn point_scalar_filter(op: Operation, x: f32) -> f32 {
    switch op.ids.y {
        // HSL Perceptive: grays and masks shift their lightness only, as opcode 31
        case 128u: { return shift_unit(filter_unit(x), op.low.z); }
        default: { return x; }
    }
}

// A P2 opcode on a content channel; channel is the destination slot (0 Base Color, 6 Height,
// 7 Normal, 8..10 colour lobes; channel_slot in Rust).
fn point_effect_color(op: Operation, c: vec3<f32>, channel: u32, surface: Surface, baked_ao: f32) -> vec3<f32> {
    if point_is_color(channel) && op.ids.y == POINT_HSL_PERCEPTIVE {
        return point_perceptive_adjust(op, c);
    }
    return point_per_unit(op, c, channel);
}

// A point opcode in the layer_content loop: the content after the effect, opacity applied. The
// default mixes point_effect_color by the opacity; take over for coverage or texel reads.
fn point_content_effect(effect: Operation, content: vec4<f32>, channel: u32, texel: LayerTexel) -> vec4<f32> {
    let filtered = point_effect_color(effect, content.rgb, channel, texel.surface, texel.baked.x);
    return vec4(content.rgb + (filtered - content.rgb) * effect.low.w, content.a);
}

// A point opcode in the layer_mask loop: the result the loop mixes in by the opacity.
fn point_mask_effect(effect: Operation, mask: f32, texel: LayerTexel) -> f32 {
    return point_scalar_filter(effect, mask);
}
