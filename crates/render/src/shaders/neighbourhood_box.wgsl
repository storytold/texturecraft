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
// Glow and Drop Shadow
const BOX_HALO: u32 = 21u;
const BOX_OUTLINE: u32 = 22u;
const BOX_DISTANCE_BEVEL: u32 = 24u;
const BOX_BEVEL_SMOOTH: u32 = 25u;
const BOX_DIRECTIONAL_DISTANCE: u32 = 26u;

// Highpass in sRGB from ACEScg (params.field_layout.z; 0 is the working space, 1 sRGB from linear)
const BOX_SPACE_SRGB_FROM_ACESCG: u32 = 2u;
// Below this alpha content has no straight colour (MIN_ALPHA in eval.rs)
const BOX_MIN_ALPHA: f32 = 1e-5;
// Word 4 of a field texel: its coverage, the pre-blur's gray in a stage output, scratch in the distance passes
const BOX_SECOND_WORD: u32 = 4u;
// Flags in params.field_layout.z
const HALO_INNER: u32 = 1u;
const HALO_PRECISE: u32 = 2u;
// The blurred value a precise halo is solid from
const BOX_PRECISE_EDGE: f32 = 0.5;
// The largest finite f32
const BOX_LARGEST_FINITE: f32 = 3.402823466e38;
// Flags in params.field_layout.z
const OUTLINE_FROM_BLACK: u32 = 1u;
const OUTLINE_COVERS_INSIDE: u32 = 2u;
const OUTLINE_COVERS_OUTSIDE: u32 = 4u;
const OUTLINE_BAND_OFF: u32 = 8u;
// The shapes' blurred share at their edge
const BOX_OUTLINE_EDGE_SHARE: f32 = 0.5;
// Flags in params.field_layout.z
const DISTANCE_ANGULAR: u32 = 1u;
const DISTANCE_INWARD: u32 = 2u;
const DISTANCE_NON_UNIFORM: u32 = 4u;
const SMOOTH_IN: u32 = 8u;
const SMOOTH_IN_OUT: u32 = 16u;
// From a texel's centre to the edge between it and the nearest texel across
const BOX_EDGE_OFFSET: f32 = 0.5;

// Whether a P4 kind runs the three box passes first (box_family::uses_box must agree).
fn box_family_uses_box(kind: u32) -> bool {
    var uses = kind == BOX_WRAPPED_BLUR || kind == BOX_WRAPPED_BEVEL;
    uses = uses || kind == BOX_HIGHPASS;
    uses = uses || kind == BOX_PRE_BLUR;
    uses = uses || kind == BOX_HALO;
    uses = uses || kind == BOX_OUTLINE;
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

fn box_extra(index: u32) -> f32 {
    return params.extra[index / 4u][index % 4u];
}

fn box_is_mask() -> bool {
    return stage_context() == STAGE_MASK;
}

fn box_smoothstep(start: f32, end: f32, value: f32) -> f32 {
    let t = clamp((value - start) / (end - start), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Glow or drop shadow in its tint over the texel (halo in eval.rs): tint in extra 0-2, hardness 3, strength 4
fn box_halo_texel(original: vec4<f32>, blurred: Texel) -> vec4<f32> {
    var shape = 0.0;
    if blurred.coverage > MIN_WEIGHT {
        shape = clamp(blurred.value.x / blurred.coverage, 0.0, 1.0);
    }
    if (box_flags() & HALO_PRECISE) != 0u {
        shape = box_smoothstep(0.0, BOX_PRECISE_EDGE, shape);
    }
    let tint = vec3(box_extra(0u), box_extra(1u), box_extra(2u));
    let glow = min(shape / box_extra(3u), 1.0) * box_extra(4u);
    let inner = (box_flags() & HALO_INNER) != 0u;
    var result = original;
    if box_is_mask() {
        // A mask's own value is its cover
        let cover = clamp(original.x, 0.0, 1.0);
        if inner {
            result.x += (tint.x - result.x) * (glow * cover);
        } else {
            result.x += tint.x * (glow * (1.0 - cover));
        }
    } else {
        let cover = clamp(original.w, 0.0, 1.0);
        if inner {
            result = vec4(result.rgb + (tint * cover - result.rgb) * glow, result.w);
        } else {
            let under = glow * (1.0 - cover);
            result = vec4(result.rgb + tint * under, result.w + under);
        }
    }
    return original + (result - original) * params.weights.x;
}

// What the shape filters threshold: a mask's value, content's luminance of its straight colour
fn box_shape_value(value: vec4<f32>) -> f32 {
    if box_is_mask() {
        return value.x;
    }
    if value.w > BOX_MIN_ALPHA {
        return box_lum(value.rgb / value.w);
    }
    return 0.0;
}

// Whether a covered texel lies inside the shapes (the threshold in extra 0, tolerance included)
fn box_inside_shapes(texel: Texel) -> bool {
    return texel.coverage > 0.0 && box_shape_value(texel.value) >= box_extra(0u);
}

// A ramp moved by offset and bent by exponent (RampCurve::apply)
fn box_ramp_curve(along: f32, offset: f32, exponent: f32) -> f32 {
    let moved = clamp(along + offset, 0.0, 1.0);
    if exponent == 1.0 || moved <= 0.0 {
        return moved;
    }
    return pow(moved, exponent);
}

// The texel's colours (a mask's value) moved to `after` and mixed in by the opacity in straight colour; the coverage stays
fn box_laid_on(value: vec4<f32>, after: f32) -> vec4<f32> {
    let opacity = params.weights.x;
    if box_is_mask() {
        return vec4(value.x + (after - value.x) * opacity, value.yzw);
    }
    let before = box_straight_color(value);
    return vec4((before + (vec3(after) - before) * opacity) * value.w, value.w);
}

// Mask Outline's band over the texel (mask_outline in eval.rs): blur in extra 1, fade exponent in extra 2
fn box_outline_texel(original: Texel, blurred: Texel) -> vec4<f32> {
    let inside = box_inside_shapes(original);
    var share = 0.0;
    if blurred.coverage > MIN_WEIGHT {
        share = clamp(blurred.value.x / blurred.coverage, 0.0, 1.0);
    }
    let closeness = clamp(select(share, 1.0 - share, inside) / BOX_OUTLINE_EDGE_SHARE, 0.0, 1.0);
    let covers = (box_flags() & select(OUTLINE_COVERS_OUTSIDE, OUTLINE_COVERS_INSIDE, inside)) != 0u;
    var band = 0.0;
    if covers && closeness > 0.0 && (box_flags() & OUTLINE_BAND_OFF) == 0u {
        band = 1.0;
        if box_extra(1u) > 0.0 {
            band = box_ramp_curve(closeness / box_extra(1u), 0.0, box_extra(2u));
        }
    }

    // From black the band alone, else the band over the value
    let value = original.value;
    if (box_flags() & OUTLINE_FROM_BLACK) != 0u {
        return box_laid_on(value, band);
    }
    let opacity = params.weights.x;
    if box_is_mask() {
        return vec4(value.x + (max(value.x, band) - value.x) * opacity, value.yzw);
    }
    let before = box_straight_color(value);
    return vec4((before + (max(before, vec3(band)) - before) * opacity) * value.w, value.w);
}

// Bevel::apply on the slope: an inward slope scales the shapes, an outward one rises around them
fn box_bevel_texel(original: Texel, slope: f32) -> vec4<f32> {
    let inside = box_inside_shapes(original);
    let inward = (box_flags() & DISTANCE_INWARD) != 0u;
    let value = original.value;
    let opacity = params.weights.x;
    if box_is_mask() {
        var after = value.x;
        if inward && inside {
            after = value.x * slope;
        } else if !inward && !inside {
            after = max(value.x, slope);
        }
        return vec4(value.x + (after - value.x) * opacity, value.yzw);
    }
    let before = box_straight_color(value);
    var after = before;
    if inward && inside {
        after = before * slope;
    } else if !inward && !inside {
        after = max(before, vec3(slope));
    }
    return vec4((before + (after - before) * opacity) * value.w, value.w);
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
    if kind == BOX_HALO {
        result = box_halo_texel(original.value, filtered);
    }
    if kind == BOX_OUTLINE {
        result = box_outline_texel(original, filtered);
    }
    if kind == BOX_DISTANCE_BEVEL {
        // buffers.z holds the unsmoothed slope; without smoothing (params.field_layout.w) buffers.y is that slope too
        let sharp = read_at(params.buffers.z, point).value.x;
        var slope = filtered.value.x;
        if params.field_layout.w > 0u && (box_flags() & DISTANCE_NON_UNIFORM) != 0u {
            // Keep the foot sharp and blur the top at full strength
            slope = sharp + (filtered.value.x - sharp) * sharp;
        }
        result = box_bevel_texel(original, slope);
    }
    if kind == BOX_BEVEL_SMOOTH {
        result = box_laid_on(original.value, filtered.value.x);
    }
    if kind == BOX_DIRECTIONAL_DISTANCE {
        result = box_laid_on(original.value, filtered.value.x);
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

    // A halo blurs the shapes moved by the shadow's offset: content's coverage or a mask's value, outside them for an inner glow
    if kind == BOX_HALO {
        let moved_from = vec2<i32>(point) - vec2<i32>(params.direction.xy);
        let segment = copy_segment(vec2<i32>(point));
        if all(moved_from >= segment.xy) && all(moved_from < segment.zw) {
            let source = original_at(vec2<u32>(moved_from));
            var shape = select(source.value.w, source.value.x, box_is_mask());
            shape = select(0.0, clamp(shape, 0.0, 1.0), abs(shape) <= BOX_LARGEST_FINITE);
            if (box_flags() & HALO_INNER) != 0u {
                shape = 1.0 - shape;
            }
            prepared = Texel(vec4(shape * source.coverage, 0.0, 0.0, 0.0), source.coverage);
        }
    }

    // Mask Outline blurs the shapes
    if kind == BOX_OUTLINE {
        let original = original_at(point);
        prepared = Texel(vec4(select(0.0, original.coverage, box_inside_shapes(original)), 0.0, 0.0, 0.0), original.coverage);
    }

    // Directional Distance walks over the inside flags
    if kind == BOX_DIRECTIONAL_DISTANCE {
        prepared = Texel(vec4(select(0.0, 1.0, box_inside_shapes(original_at(point))), 0.0, 0.0, 0.0), 1.0);
    }
    write_at(params.buffers.z, point, prepared);
}

// Floor division (WGSL's rounds towards zero)
fn box_floor_div(value: i32, divisor: i32) -> i32 {
    let quotient = value / divisor;
    if value % divisor != 0 && (value < 0) != (divisor < 0) {
        return quotient - 1;
    }
    return quotient;
}

// Where a line without a feature starts: beyond the field, squared or chessboard (shape_distance.rs)
fn box_distance_far(angular: bool) -> f32 {
    let side = f32(params.rect.z + params.rect.w);
    if angular {
        return side;
    }
    return side * side;
}

fn box_field_float(offset: u32, position: vec2<u32>, word: u32) -> f32 {
    return bitcast<f32>(fields[word_at(offset, position) + word]);
}

fn box_field_word(offset: u32, position: vec2<u32>, word: u32) -> u32 {
    return fields[word_at(offset, position) + word];
}

fn box_set_field_word(offset: u32, position: vec2<u32>, word: u32, value: u32) {
    fields[word_at(offset, position) + word] = value;
}

// One invocation per row: the distance along it to the nearest texel inside (word 0) and outside (word 1) into buffers.z, the inside flag in word 4
// A stage that does not wrap measures each copy of the set apart, as the CPU's region stops at the set's edges
@compute @workgroup_size(64)
fn distance_rows(@builtin(global_invocation_id) id: vec3<u32>) {
    let y = id.x;
    if y >= params.rect.w {
        return;
    }
    let width = i32(params.rect.z);
    var edges = array<i32, 4>(0, clamp(params.bounds.x, 0, width), clamp(params.bounds.z, 0, width), width);
    for (var segment = 0; segment < 3; segment++) {
        let start = select(edges[segment], 0, stage_wraps());
        let end = select(edges[segment + 1], width, stage_wraps());
        if start < end && (segment == 0 || !stage_wraps()) {
            box_row_segment(y, start, end);
        }
    }
}

// Texels [start, end) of row y as one line
fn box_row_segment(y: u32, start: i32, end: i32) {
    let angular = (box_flags() & DISTANCE_ANGULAR) != 0u;
    let far = box_distance_far(angular);
    let rows = params.buffers.z;

    // The nearest texel of each side to the left first
    var last_inside = -1;
    var last_outside = -1;
    for (var x = start; x < end; x++) {
        let position = vec2(u32(x), y);
        let inside = box_inside_shapes(original_at(position));
        if inside {
            last_inside = x;
        } else {
            last_outside = x;
        }
        box_set_field_word(rows, position, 0u, bitcast<u32>(last_inside));
        box_set_field_word(rows, position, 1u, bitcast<u32>(last_outside));
        box_set_field_word(rows, position, BOX_SECOND_WORD, select(0u, 1u, inside));
    }

    // Then the nearer of it and the one to the right
    var next_inside = -1;
    var next_outside = -1;
    for (var x = end - 1; x >= start; x--) {
        let position = vec2(u32(x), y);
        if box_field_word(rows, position, BOX_SECOND_WORD) != 0u {
            next_inside = x;
        } else {
            next_outside = x;
        }
        for (var side = 0u; side < 2u; side++) {
            let left = bitcast<i32>(box_field_word(rows, position, side));
            let right = select(next_outside, next_inside, side == 0u);
            var nearest = -1;
            if left >= 0 {
                nearest = x - left;
            }
            if right >= 0 && (nearest < 0 || right - x < nearest) {
                nearest = right - x;
            }
            var distance = far;
            if nearest >= 0 {
                distance = f32(nearest);
                if !angular {
                    distance = distance * distance;
                }
            }
            box_set_field_word(rows, position, side, bitcast<u32>(distance));
        }
    }
}

// A parabola (a cone for chessboard distances) of apex `apex` and height `height` at `place`
fn box_envelope_value(place: i32, apex: i32, height: u32, angular: bool) -> u32 {
    let across = u32(abs(place - apex));
    if angular {
        return max(across, height);
    }
    return across * across + height;
}

// The last place where the parabola of `apex` stays at or below that of `later` (Meijster's Sep)
fn box_envelope_separation(apex: i32, later: i32, height_apex: u32, height_later: u32, angular: bool) -> i32 {
    if angular {
        let middle = box_floor_div(apex + later, 2);
        if height_apex <= height_later {
            return max(apex + i32(height_later), middle);
        }
        return min(later - i32(height_apex), middle);
    }
    let lifted = (later * later + i32(height_later)) - (apex * apex + i32(height_apex));
    return box_floor_div(lifted, 2 * (later - apex));
}

// A column segment's lower envelope in exact integers (Meijster), where the CPU crosses the parabolas in f64
// Rows [first, end) of column x: heights in word `side` of buffers.z, apexes and starts in buffers.y, the result in word side + 2 of buffers.z
fn box_column_envelope(x: u32, side: u32, angular: bool, first: i32, end: i32) {
    let height = end - first;
    let heights = params.buffers.z;
    let stack = params.buffers.y;
    let apex_word = side * 2u;
    let start_word = side * 2u + 1u;
    let at = vec2(x, u32(first));
    var top = 0;
    box_set_field_word(stack, at, apex_word, 0u);
    box_set_field_word(stack, at, start_word, 0u);
    for (var u = 1; u < height; u++) {
        let height_u = u32(box_field_float(heights, vec2(x, u32(first + u)), side));
        loop {
            if top < 0 {
                break;
            }
            let apex = i32(box_field_word(stack, vec2(x, u32(first + top)), apex_word));
            let start = i32(box_field_word(stack, vec2(x, u32(first + top)), start_word));
            let height_apex = u32(box_field_float(heights, vec2(x, u32(first + apex)), side));
            if box_envelope_value(start, apex, height_apex, angular) > box_envelope_value(start, u, height_u, angular) {
                top -= 1;
            } else {
                break;
            }
        }
        if top < 0 {
            top = 0;
            box_set_field_word(stack, at, apex_word, u32(u));
            box_set_field_word(stack, at, start_word, 0u);
        } else {
            let apex = i32(box_field_word(stack, vec2(x, u32(first + top)), apex_word));
            let height_apex = u32(box_field_float(heights, vec2(x, u32(first + apex)), side));
            let start = 1 + box_envelope_separation(apex, u, height_apex, height_u, angular);
            if start < height {
                top += 1;
                box_set_field_word(stack, vec2(x, u32(first + top)), apex_word, u32(u));
                box_set_field_word(stack, vec2(x, u32(first + top)), start_word, u32(start));
            }
        }
    }
    for (var u = height - 1; u >= 0; u--) {
        let apex = i32(box_field_word(stack, vec2(x, u32(first + top)), apex_word));
        let start = i32(box_field_word(stack, vec2(x, u32(first + top)), start_word));
        let height_apex = box_field_float(heights, vec2(x, u32(first + apex)), side);
        var distance: f32;
        if angular {
            distance = max(f32(abs(u - apex)), height_apex);
        } else {
            // As DistanceEnvelope::lower computes it, in f32
            let across = f32(u) - f32(apex);
            distance = across * across + height_apex;
        }
        box_set_field_word(heights, vec2(x, u32(first + u)), side + 2u, bitcast<u32>(distance));
        if u == start {
            top -= 1;
        }
    }
}

// One invocation per column: both sides' envelopes over the row distances in buffers.z, each copy of the set apart unless the stage wraps
@compute @workgroup_size(64)
fn distance_columns(@builtin(global_invocation_id) id: vec3<u32>) {
    let x = id.x;
    if x >= params.rect.z {
        return;
    }
    let angular = (box_flags() & DISTANCE_ANGULAR) != 0u;
    let height = i32(params.rect.w);
    var edges = array<i32, 4>(0, clamp(params.bounds.y, 0, height), clamp(params.bounds.w, 0, height), height);
    for (var segment = 0; segment < 3; segment++) {
        let start = select(edges[segment], 0, stage_wraps());
        let end = select(edges[segment + 1], height, stage_wraps());
        if start < end && (segment == 0 || !stage_wraps()) {
            box_column_envelope(x, 0u, angular, start, end);
            box_column_envelope(x, 1u, angular, start, end);
        }
    }
}

// The sum of two unsigned 64-bit integers, each two u32 words with the high one first
fn box_wide_add(first: vec2<u32>, second: vec2<u32>) -> vec2<u32> {
    let low = first.y + second.y;
    return vec2(first.x + second.x + select(0u, 1u, low < first.y), low);
}

// The product of two integers below 2^26 as an unsigned 64-bit integer
fn box_wide_product(first: u32, second: u32) -> vec2<u32> {
    let first_high = first >> 13u;
    let first_low = first & 0x1fffu;
    let second_high = second >> 13u;
    let second_low = second & 0x1fffu;
    let high = first_high * second_high;
    let cross_term = first_high * second_low + first_low * second_high;
    let low = first_low * second_low;
    let upper = box_wide_add(vec2(high >> 6u, high << 26u), vec2(cross_term >> 19u, cross_term << 13u));
    return box_wide_add(upper, vec2(0u, low));
}

// `value` times 2^shift as an unsigned 64-bit integer
fn box_wide_shifted(value: u32, shift: u32) -> vec2<u32> {
    if shift >= 32u {
        return vec2(value << (shift - 32u), 0u);
    }
    if shift == 0u {
        return vec2(0u, value);
    }
    return vec2(value >> (32u - shift), value << shift);
}

fn box_wide_less(first: vec2<u32>, second: vec2<u32>) -> bool {
    return first.x < second.x || (first.x == second.x && first.y < second.y);
}

// Whether integer `value` lies below the square of odd · 2^exponent (exponent below zero)
fn box_below_square(value: u32, odd: u32, exponent: i32) -> bool {
    return box_wide_less(box_wide_shifted(value, u32(-2 * exponent)), box_wide_product(odd, odd));
}

// The correctly rounded square root of an integer-valued f32 below 2^32, as the CPU's sqrt gives it (WGSL's may be off)
// A root of an integer never lies on a midpoint between two floats unless it is whole, so the midpoints decide
fn box_exact_sqrt(squared: f32) -> f32 {
    let value = u32(squared);
    if value == 0u {
        return 0.0;
    }
    var root = sqrt(squared);
    for (var attempt = 0; attempt < 4; attempt++) {
        let bits = bitcast<u32>(root);
        let exponent = i32(bits >> 23u) - 127;
        let mantissa = (bits & 0x7fffffu) | 0x800000u;
        if !box_below_square(value, 2u * mantissa + 1u, exponent - 24) {
            root = bitcast<f32>(bits + 1u);
            continue;
        }

        // Below a power of two the next float down is half as far
        var below_lower: bool;
        if (bits & 0x7fffffu) == 0u {
            below_lower = box_below_square(value, 4u * mantissa - 1u, exponent - 25);
        } else {
            below_lower = box_below_square(value, 2u * mantissa - 1u, exponent - 24);
        }
        if !below_lower {
            break;
        }
        root = bitcast<f32>(bits - 1u);
    }
    return root;
}

// The Distance Map at the texel (the stage's input word, 0..1), else what the CPU reads without one: 1 on the mesh, 0 off it
// FilterStage::input is set for every Bevel Smooth and Directional Distance stage, so the word is this stage's whenever the fields carry it
fn box_distance_map(point: vec2<u32>) -> f32 {
    if params.field_layout.y != 0u {
        return clamp(input_at(point), 0.0, 1.0);
    }
    return select(0.0, 1.0, original_at(point).coverage > 0.0);
}

// The edge distance (word 1) and the ramp a distance filter makes of it (word 0) into buffers.z from the envelopes in buffers.y
// Coverage 1 everywhere, so the smoothing blurs every texel alike as the CPU's blur does; the reach in extra 1
@compute @workgroup_size(8, 8)
fn distance_edges(@builtin(global_invocation_id) id: vec3<u32>) {
    let point = id.xy;
    if any(point >= params.rect.zw) {
        return;
    }
    let distances = params.buffers.y;
    let inside = box_field_word(distances, point, BOX_SECOND_WORD) != 0u;
    let flags = box_flags();
    var distance = box_field_float(distances, point, select(2u, 3u, inside));
    if (flags & DISTANCE_ANGULAR) == 0u {
        distance = box_exact_sqrt(distance);
    }
    let edge = max(distance - BOX_EDGE_OFFSET, 0.0);
    var reach = box_extra(1u);

    // Bevel Smooth's Distance Map scales its reach texel by texel
    if params.mode.y == BOX_BEVEL_SMOOTH {
        reach = reach * box_distance_map(point);
    }
    var along = 1.0;
    if reach > 0.0 {
        along = clamp(edge / reach, 0.0, 1.0);
    }
    var ramp: f32;
    if params.mode.y == BOX_DISTANCE_BEVEL {
        // Bevel::slope
        let inward = (flags & DISTANCE_INWARD) != 0u;
        ramp = select(select(0.0, along, inside), select(1.0 - along, 1.0, inside), !inward);
    } else {
        // BevelSmooth::value, bent by its curve (offset in extra 2, exponent in extra 3)
        let curved = box_ramp_curve(along, box_extra(2u), box_extra(3u));
        if (flags & SMOOTH_IN) != 0u {
            ramp = select(0.0, curved, inside);
        } else if (flags & SMOOTH_IN_OUT) != 0u {
            ramp = select(0.5 - 0.5 * curved, 0.5 + 0.5 * curved, inside);
        } else {
            ramp = select(1.0 - curved, 1.0, inside);
        }
    }
    write_at(params.buffers.z, point, Texel(vec4(ramp, edge, 0.0, 0.0), 1.0));
}

// The IEEE f32 product of two normal floats, rounded to nearest even in integers: a driver cannot fuse it into an addition
fn box_exact_product(first: f32, second: f32) -> f32 {
    if first == 0.0 || second == 0.0 {
        return 0.0;
    }
    let first_bits = bitcast<u32>(first);
    let second_bits = bitcast<u32>(second);
    let sign = (first_bits ^ second_bits) & 0x80000000u;
    let product = box_wide_product((first_bits & 0x7fffffu) | 0x800000u, (second_bits & 0x7fffffu) | 0x800000u);
    var exponent = i32((first_bits >> 23u) & 0xffu) + i32((second_bits >> 23u) & 0xffu) - 127;

    // The 48-bit product of two 24-bit mantissas keeps its top 24 bits; the rest rounds
    var shift = 23u;
    if product.x >= 0x8000u {
        shift = 24u;
        exponent += 1;
    }
    var mantissa = (product.x << (32u - shift)) | (product.y >> shift);
    let rest = product.y & ((1u << shift) - 1u);
    let half = 1u << (shift - 1u);
    if rest > half || (rest == half && (mantissa & 1u) != 0u) {
        mantissa += 1u;
    }
    if mantissa == 0x1000000u {
        mantissa = 0x800000u;
        exponent += 1;
    }
    return bitcast<f32>(sign | (u32(exponent) << 23u) | (mantissa & 0x7fffffu));
}

// Directional Distance: each texel walks back against the direction over the inside flags in buffers.y (trail_distances)
// The trail into word 0 of buffers.z, the step that found a shape (-1: none) into word 1; reach in extra 1, step length 2, 1 - contrast 3
@compute @workgroup_size(8, 8)
fn directional_trail(@builtin(global_invocation_id) id: vec3<u32>) {
    let point = id.xy;
    if any(point >= params.rect.zw) {
        return;
    }
    // The texel's copy of the set is the CPU's region
    let segment = copy_segment(vec2<i32>(point));
    let size = segment.zw - segment.xy;
    let direction = params.direction.xy;
    let reach = box_extra(1u);
    let step_length = box_extra(2u);
    var found = -1;
    for (var sample = 0u; sample <= params.field_layout.z; sample++) {
        let back = f32(sample) * step_length;

        // Rounded half up as trail_steps rounds, the products rounded exactly as the CPU's
        let moved = floor(vec2(0.5) - vec2(box_exact_product(direction.x, back), box_exact_product(direction.y, back)));
        if !all(abs(moved) <= vec2(BOX_LARGEST_FINITE)) {
            break;
        }

        // A step as long as the field leaves it from every texel, one past its top or bottom for good
        let offset = vec2<i32>(moved);
        if abs(offset.x) >= size.x || abs(offset.y) >= size.y {
            break;
        }
        let source = vec2<i32>(point) + offset;
        if source.y < segment.y || source.y >= segment.w {
            break;
        }
        if source.x >= segment.x && source.x < segment.z && box_field_float(params.buffers.y, vec2<u32>(source), 0u) != 0.0 {
            found = i32(sample);
            break;
        }
    }

    // DirectionalDistance::value: no shape in reach leaves black
    // The Distance Map shortens the texel's reach: a share of the full reach is a larger share of a shorter one, none where it is black
    var trail = 0.0;
    if found >= 0 {
        var along = 0.0;
        if reach > 0.0 {
            along = f32(found) * step_length / reach;
        }
        let map = box_distance_map(point);
        var in_reach = true;
        if along > 0.0 {
            in_reach = map > 0.0;
            along = along / map;
        }
        if in_reach && along <= 1.0 {
            let fade = 1.0 - clamp(along, 0.0, 1.0);
            trail = clamp(0.5 + (fade - 0.5) / box_extra(3u), 0.0, 1.0);
        }
    }
    write_at(params.buffers.z, point, Texel(vec4(trail, f32(found), 0.0, 0.0), 1.0));
}
