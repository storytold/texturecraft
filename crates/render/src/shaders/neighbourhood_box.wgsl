// P4, the box family and distance filters (gpu_neighbourhood_box.rs): kinds 16..24 (box) and
// 24..32 (distance). Owned by the P4 agent. Always part of the neighbourhood shader; extra
// passes (row and column distance transforms) get their own entry points here, created in
// box_family::Pipelines. Box sums use the 2^24 fixed point of box_blur, as the CPU's i64 sums.

// Pixelate laid on each texel's own material alpha (box_family::PIXELATE_KEEPING_ALPHA)
const PIXELATE_KEEPING_ALPHA: u32 = 16u;
// Blur and Bevel with Wrap over a field the prepare pass fills periodically
const BOX_WRAPPED_BLUR: u32 = 17u;
const BOX_WRAPPED_BEVEL: u32 = 18u;
// Highpass in an sRGB space or of the alpha
const BOX_HIGHPASS: u32 = 19u;
// The Gradient's input blur for P2's gradient
const BOX_PRE_BLUR: u32 = 20u;

// Highpass in sRGB from ACEScg (params.field_layout.z; 0 is the working space, 1 sRGB from linear)
const BOX_SPACE_SRGB_FROM_ACESCG: u32 = 2u;
// Below this alpha content has no straight colour (MIN_ALPHA in eval.rs)
const BOX_MIN_ALPHA: f32 = 1e-5;
// Word 4 of a field texel: its coverage, the pre-blur's gray in a stage output, scratch in the distance passes
const BOX_SECOND_WORD: u32 = 4u;

// Whether a P4 kind runs the three box passes first (box_family::uses_box must agree).
fn box_family_uses_box(kind: u32) -> bool {
    var uses = kind == BOX_WRAPPED_BLUR || kind == BOX_WRAPPED_BEVEL;
    uses = uses || kind == BOX_HIGHPASS;
    uses = uses || kind == BOX_PRE_BLUR;
    return uses;
}

// A P4 kind after the box passes: the original and the box-blurred field, unpremultiplied
// by coverage like filtered_box.
fn box_family_filtered(original: vec4<f32>, blurred: vec4<f32>) -> vec4<f32> {
    // Away from 0 by at most the value itself, as the built-in Bevel
    if params.mode.y == BOX_WRAPPED_BEVEL {
        let side = sign(original.rgb);
        let distance = abs(original.rgb);
        return vec4(side * min(distance, max(2.0 * blurred.rgb * side - distance, vec3(0.0))), original.a);
    }
    return blurred;
}

// A P4 kind without the box passes, at absolute texel position point.
fn box_family_gathered(point: vec2<f32>, original: vec4<f32>) -> vec4<f32> {
    if params.mode.y == PIXELATE_KEEPING_ALPHA {
        return pixelate_keeping_alpha(point, original);
    }
    return original;
}

// The cell centre's sample with its colour laid on the texel's own alpha (keep_coverage in eval.rs)
// A mask's alpha is 1 wherever it is covered, so a mask pixelates as with Alpha on
fn pixelate_keeping_alpha(point: vec2<f32>, original: vec4<f32>) -> vec4<f32> {
    let radius = f32(params.mode.x);
    let center = (floor(point / radius) + 0.5) * radius - 0.5;
    let sample = sample_field(EMPTY, center);

    // No material alpha in the cell leaves the texel as it was
    if sample.value.w <= MIN_WEIGHT || sample.coverage <= MIN_WEIGHT {
        return original;
    }

    let scale = original.w * sample.coverage / sample.value.w;
    return vec4(sample.value.rgb * scale, original.w * sample.coverage) / sample.coverage;
}

// The kind's flags or mode (NeighbourhoodFilter::words[0])
fn box_flags() -> u32 {
    return params.field_layout.z;
}

fn box_srgb_from_linear(value: f32) -> f32 {
    let v = clamp(value, 0.0, 1.0);
    if v <= 0.0031308 {
        return v * 12.92;
    }
    return 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}

fn box_linear_from_srgb(value: f32) -> f32 {
    let v = clamp(value, 0.0, 1.0);
    if v <= 0.04045 {
        return v / 12.92;
    }
    return pow((v + 0.055) / 1.055, 2.4);
}

// Matrix rows times a column, summed in the CPU's order (times in highpass_space.rs)
fn box_times_rows(first: vec3<f32>, second: vec3<f32>, third: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    return vec3(
        first.x * v.x + first.y * v.y + first.z * v.z,
        second.x * v.x + second.y * v.y + second.z * v.z,
        third.x * v.x + third.y * v.y + third.z * v.z,
    );
}

// HighpassSpace::encode of a straight colour
fn box_highpass_encode(rgb: vec3<f32>) -> vec3<f32> {
    var linear_rgb = rgb;
    if box_flags() == BOX_SPACE_SRGB_FROM_ACESCG {
        // AP1_TO_REC709 in highpass_space.rs
        linear_rgb = box_times_rows(vec3(1.70505, -0.62179, -0.08326), vec3(-0.13026, 1.14080, -0.01055), vec3(-0.02400, -0.12897, 1.15297), rgb);
    }
    return vec3(box_srgb_from_linear(linear_rgb.x), box_srgb_from_linear(linear_rgb.y), box_srgb_from_linear(linear_rgb.z));
}

// HighpassSpace::decode of a straight colour
fn box_highpass_decode(rgb: vec3<f32>) -> vec3<f32> {
    let linear_rgb = vec3(box_linear_from_srgb(rgb.x), box_linear_from_srgb(rgb.y), box_linear_from_srgb(rgb.z));
    if box_flags() == BOX_SPACE_SRGB_FROM_ACESCG {
        // REC709_TO_AP1 in highpass_space.rs
        return box_times_rows(vec3(0.61310, 0.33952, 0.04737), vec3(0.07019, 0.91635, 0.01345), vec3(0.02062, 0.10957, 0.86981), linear_rgb);
    }
    return linear_rgb;
}

// Premultiplied content with its straight colour moved into or out of the Highpass space (recode in eval.rs)
fn box_highpass_recode(value: vec4<f32>, encode: bool) -> vec4<f32> {
    if box_flags() == 0u || value.w <= MIN_WEIGHT {
        return value;
    }
    let straight = value.rgb / value.w;
    if encode {
        return vec4(box_highpass_encode(straight) * value.w, value.w);
    }
    return vec4(box_highpass_decode(straight) * value.w, value.w);
}

// Highpass in its space and of the alpha when params.field_layout.w is set (filter_region and recover in eval.rs)
fn box_highpass_texel(original: vec4<f32>, blurred: Texel) -> vec4<f32> {
    let with_alpha = params.field_layout.w != 0u;
    let encoded = box_highpass_recode(original, true);
    var result = encoded;
    if blurred.coverage > MIN_WEIGHT {
        let average = blurred.value / blurred.coverage;
        let rgb = encoded.rgb - average.rgb + params.weights.w * encoded.w;
        var alpha = encoded.w;
        if with_alpha {
            alpha = clamp(encoded.w - average.w + 0.5, 0.0, 1.0);
        }
        result = encoded + (vec4(rgb, alpha) - encoded) * params.weights.x;
    }

    // The colours laid on the new coverage, mid gray where there was none
    if with_alpha {
        if original.w > MIN_WEIGHT {
            result = vec4(result.rgb * (result.w / original.w), result.w);
        } else {
            result = vec4(vec3(params.weights.w * result.w), result.w);
        }
    }
    return box_highpass_recode(result, false);
}

// Luminance as core::lum weighs it
fn box_lum(color: vec3<f32>) -> f32 {
    return 0.3 * color.x + 0.59 * color.y + 0.11 * color.z;
}

// Premultiplied content's straight colour, black without alpha
fn box_straight_color(value: vec4<f32>) -> vec3<f32> {
    return select(vec3(0.0), value.rgb / value.w, value.w > BOX_MIN_ALPHA);
}

// GrayscaleMode::gray of a straight colour, the mode in params.field_layout.z
fn box_gray_of(rgb: vec3<f32>) -> f32 {
    let mode = params.field_layout.z;
    if mode == 0u {
        return (max(max(rgb.x, rgb.y), rgb.z) + min(min(rgb.x, rgb.y), rgb.z)) / 2.0;
    }
    if mode == 1u {
        return box_lum(rgb);
    }
    if mode == 2u {
        return (rgb.x + rgb.y + rgb.z) / 3.0;
    }
    if mode == 3u {
        return max(max(rgb.x, rgb.y), rgb.z);
    }
    return min(min(rgb.x, rgb.y), rgb.z);
}

// The pre-blur's output (map_blurred in eval.rs): the original texel in words 0-3, the blurred gray in word 4
// A normal map's gradient is the identity, so there the blur is mixed in here and word 4 stays 0
fn box_pre_blur_texel(point: vec2<u32>, original: Texel, blurred: Texel) {
    var average = original.value;
    if blurred.coverage > MIN_WEIGHT {
        average = blurred.value / blurred.coverage;
    }
    var value = original.value;
    var gray = average.x;
    let context = stage_context();
    if context != STAGE_MASK {
        let straight = box_straight_color(average);
        gray = straight.x;
        if context == STAGE_COLOR {
            gray = box_gray_of(straight);
        } else if context == STAGE_NORMAL_MAP {
            let before = box_straight_color(original.value);
            value = vec4((before + (straight - before) * params.weights.x) * original.value.w, original.value.w);
            gray = 0.0;
        }
    }
    write_at(params.buffers.z, point, Texel(value, original.coverage));
    fields[word_at(params.buffers.z, point) + BOX_SECOND_WORD] = bitcast<u32>(gray);
}

// The apply pass of the kinds with passes of their own: the original at buffers.x, the filtered field at buffers.y, the result to buffers.z
@compute @workgroup_size(8, 8)
fn box_family_apply(@builtin(global_invocation_id) id: vec3<u32>) {
    let point = id.xy;
    if any(point >= params.rect.zw) {
        return;
    }
    let original = original_at(point);
    let filtered = read_at(params.buffers.y, point);
    let kind = params.mode.y;

    // Uncovered texels stay empty
    if original.coverage <= 0.0 {
        write_at(params.buffers.z, point, original);
        return;
    }

    var result = original.value;
    if kind == BOX_HIGHPASS {
        result = box_highpass_texel(original.value, filtered);
    }
    if kind == BOX_PRE_BLUR {
        box_pre_blur_texel(point, original, filtered);
        return;
    }
    write_at(params.buffers.z, point, Texel(result, original.coverage));
}

// What the box passes of a kind with passes of its own blur, from the original at buffers.x into buffers.z
@compute @workgroup_size(8, 8)
fn box_family_prepare(@builtin(global_invocation_id) id: vec3<u32>) {
    let point = id.xy;
    if any(point >= params.rect.zw) {
        return;
    }
    let kind = params.mode.y;
    var prepared = EMPTY;

    // Highpass blurs the colour in its space
    if kind == BOX_HIGHPASS {
        let original = original_at(point);
        prepared = Texel(box_highpass_recode(original.value, true), original.coverage);
    }
    write_at(params.buffers.z, point, prepared);
}
