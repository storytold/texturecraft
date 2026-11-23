// Match the CPU neighbourhood filters in compose/src/eval.rs. Keep the
// material alpha separate from the geometric UV weight throughout the filter.
struct Params {
    rect: vec4<u32>,       // Absolute origin and extent in source texels.
    buffers: vec4<u32>,    // Original, input, output word offsets, and vertical flag.
    mode: vec4<u32>,       // Radius, filter kind, seed, and first box pass.
    weights: vec4<f32>,    // Opacity, amount, noise frequency, and high-pass middle.
    direction: vec4<f32>,
    // Words per field texel (5, or 6 with the input word), input word present, and two words of
    // the GPU port packages (NeighbourhoodFilter::words).
    field_layout: vec4<u32>,
    extra: array<vec4<f32>, 2>, // The packages' parameters (NeighbourhoodFilter::extra).
    // What the stage filters (stage_context), then reserved.
    stage: vec4<u32>,
}

struct Texel {
    value: vec4<f32>,      // Premultiplied RGB and material alpha.
    coverage: f32,       // Geometric UV coverage, or the blurred weight between box passes.
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> fields: array<u32>;

const MIN_WEIGHT: f32 = 1e-6;
const FIX: f32 = 16777216.0;
const EMPTY: Texel = Texel(vec4(0.0), 0.0);

fn word_at(offset: u32, position: vec2<u32>) -> u32 {
    return offset + (position.y * params.rect.z + position.x) * params.field_layout.x;
}

// The optional sixth word of an original-field texel (FilterStage::input in Rust): an input's
// gray or the layer's Height, zero when the program's fields have no input word.
fn input_at(position: vec2<u32>) -> f32 {
    if params.field_layout.y == 0u { return 0.0; }
    return bitcast<f32>(fields[word_at(params.buffers.x, position) + 5u]);
}

// What the stage filters (NeighbourhoodFilter::context): a mask, or content by channel kind.
const STAGE_MASK: u32 = 0u;
const STAGE_COLOR: u32 = 1u;
const STAGE_SCALAR: u32 = 2u;
const STAGE_SIGNED: u32 = 3u;
const STAGE_NORMAL_MAP: u32 = 4u;

fn stage_context() -> u32 {
    return params.stage.x;
}

// The field's origin in source texels. It can lie below zero (a wrapping stage's field: the
// prepare pass then fills it periodically), so read it signed.
fn field_origin() -> vec2<i32> {
    return bitcast<vec2<i32>>(params.rect.xy);
}

// Kinds 16..32 are P4's (neighbourhood_box.wgsl), 32..56 P5's (neighbourhood_gather.wgsl).
// Whether a kind runs the box passes; NeighbourhoodFilter::uses_box in Rust must agree.
fn uses_box_kind(kind: u32) -> bool {
    if kind <= 4u { return true; }
    if kind >= 16u && kind < 32u { return box_family_uses_box(kind); }
    if kind >= 32u && kind < 56u { return gather_family_uses_box(kind); }
    return false;
}

fn read_at(offset: u32, position: vec2<u32>) -> Texel {
    let i = word_at(offset, position);
    return Texel(bitcast<vec4<f32>>(vec4(fields[i], fields[i + 1u], fields[i + 2u], fields[i + 3u])), bitcast<f32>(fields[i + 4u]));
}

fn write_at(offset: u32, position: vec2<u32>, texel: Texel) {
    let i = word_at(offset, position);
    let value = bitcast<vec4<u32>>(texel.value);
    fields[i] = value.x;
    fields[i + 1u] = value.y;
    fields[i + 2u] = value.z;
    fields[i + 3u] = value.w;
    fields[i + 4u] = bitcast<u32>(texel.coverage);
}

fn original_at(position: vec2<u32>) -> Texel {
    let texel = read_at(params.buffers.x, position);
    if texel.coverage <= 0.0 { return EMPTY; }
    return texel;
}

fn round_fixed(value: vec4<f32>) -> vec4<f32> {
    let whole = floor(value);
    return whole + select(vec4(0.0), vec4(1.0), value - whole >= vec4(0.5));
}

fn box_sample(position: vec2<u32>) -> Texel {
    if params.mode.w == 0u { return read_at(params.buffers.y, position); }
    let original = original_at(position);
    let finite = abs(original.value) <= vec4(3.402823466e38);
    let value = select(vec4(0.0), clamp(original.value, vec4(-1e6), vec4(1e6)), finite);
    return Texel(sign(value) * round_fixed(abs(value) * FIX) / FIX, original.coverage);
}

struct BoxSum {
    sum: Texel,
    correction: Texel,
    nonzero: vec4<i32>,
    covered: i32,
}

// Compensated updates keep the sliding sum stable even where only tiny
// geometric support remains at the edge of an island.
fn add_box_sample(acc: BoxSum, texel: Texel, weight: f32) -> BoxSum {
    let rgb = texel.value * weight - acc.correction.value;
    let coverage = texel.coverage * weight - acc.correction.coverage;
    let sum = Texel(acc.sum.value + rgb, acc.sum.coverage + coverage);
    let nonzero = acc.nonzero + select(vec4(0), vec4(i32(weight)), texel.value != vec4(0.0));
    let covered = acc.covered + select(0, i32(weight), texel.coverage != 0.0);
    return BoxSum(sum, Texel((sum.value - acc.sum.value) - rgb, (sum.coverage - acc.sum.coverage) - coverage), nonzero, covered);
}

// One running window per line. Its initial sum visits at most the length of
// the line, and every subsequent pixel adds and removes at most one sample.
@compute @workgroup_size(64)
fn box_blur(@builtin(global_invocation_id) id: vec3<u32>) {
    let vertical = params.buffers.w != 0u;
    let size = select(params.rect.zw, params.rect.wz, vertical);
    let line = id.x;
    if line >= size.y { return; }
    let radius = params.mode.x;
    let reciprocal = 1.0 / f32(radius * 2u + 1u);
    var acc = BoxSum(EMPTY, EMPTY, vec4(0), 0);
    for (var index = 0u; index < min(radius + 1u, size.x); index++) {
        let point = vec2(index, line);
        acc = add_box_sample(acc, box_sample(select(point, point.yx, vertical)), 1.0);
    }
    for (var index = 0u; index < size.x; index++) {
        let point = vec2(index, line);
        // Preserve exact zeros outside the filter footprint. A residual in alpha
        // would make sparse Replace blend an otherwise empty texel.
        let value = select(vec4(0.0), round_fixed(acc.sum.value * reciprocal * FIX) / FIX, acc.nonzero > vec4(0));
        let coverage = select(0.0, round_fixed(vec4(max(acc.sum.coverage, 0.0) * reciprocal * FIX)).x / FIX, acc.covered > 0);
        write_at(params.buffers.z, select(point, point.yx, vertical), Texel(value, coverage));
        let next = index + radius + 1u;
        if next < size.x {
            let added = vec2(next, line);
            acc = add_box_sample(acc, box_sample(select(added, added.yx, vertical)), 1.0);
        }
        if index >= radius {
            let previous = vec2(index - radius, line);
            acc = add_box_sample(acc, box_sample(select(previous, previous.yx, vertical)), -1.0);
        }
    }
}

fn filtered_box(original: vec4<f32>, blurred: vec4<f32>) -> vec4<f32> {
    switch params.mode.y {
        case 1u: { return original + params.weights.y * (original - blurred); }
        case 2u: { return vec4(original.rgb - blurred.rgb + params.weights.w * original.a, original.a); }
        case 3u: {
            let side = sign(original.rgb);
            let distance = abs(original.rgb);
            return vec4(side * min(distance, max(2.0 * blurred.rgb * side - distance, vec3(0.0))), original.a);
        }
        case 4u: { return vec4(original.rgb + params.weights.y * blurred.rgb, original.a); }
        case 0u: { return blurred; }
        default: {
            let kind = params.mode.y;
            if kind >= 16u && kind < 32u { return box_family_filtered(original, blurred); }
            if kind >= 32u && kind < 56u { return gather_family_filtered(original, blurred); }
            return blurred;
        }
    }
}

fn add_weighted(acc: Texel, point: vec2<i32>, weight: f32) -> Texel {
    if weight <= 0.0 || any(point < vec2(0)) || any(point >= vec2<i32>(params.rect.zw)) { return acc; }
    let texel = original_at(vec2<u32>(point));
    return Texel(acc.value + texel.value * weight, acc.coverage + texel.coverage * weight);
}

// Bilinear samples use absolute texel positions so a cropped dispatch or a
// paint-window split does not shift filter effects.
fn sample_field(acc: Texel, point: vec2<f32>) -> Texel {
    if !all(abs(point) <= vec2(3.402823466e38)) { return acc; }
    let base = vec2<i32>(floor(point)) - field_origin();
    let fraction = fract(point);
    let rest = 1.0 - fraction;
    var result = add_weighted(acc, base, rest.x * rest.y);
    result = add_weighted(result, base + vec2(1, 0), fraction.x * rest.y);
    result = add_weighted(result, base + vec2(0, 1), rest.x * fraction.y);
    return add_weighted(result, base + 1, fraction.x * fraction.y);
}

fn normalized_or(sample: Texel, original: vec4<f32>) -> vec4<f32> {
    if sample.coverage > MIN_WEIGHT { return sample.value / sample.coverage; }
    return original;
}

fn height_noise(point: vec2<f32>, delta: vec2<f32>, seed: u32) -> f32 {
    return fbm(vec3(point * params.weights.z + delta, 0.0), 3u, seed, true);
}

fn warp_offset(point: vec2<f32>, seed: u32, radius: f32) -> f32 {
    return clamp((height_noise(point, vec2(0.0), seed) - 0.5) * 3.0, -1.0, 1.0) * radius;
}

fn gathered(point: vec2<f32>, original: vec4<f32>) -> vec4<f32> {
    let radius = f32(params.mode.x);
    let n = i32(clamp(params.mode.x, 1u, 24u));
    let step = radius / f32(n);
    let seed = params.mode.z;
    var acc = EMPTY;
    switch params.mode.y {
        case 5u: {
            for (var i = -n; i <= n; i++) {
                acc = sample_field(acc, point + params.direction.xy * (f32(i) * step));
            }
        }
        case 6u: {
            let offset = vec2(warp_offset(point, seed, radius), warp_offset(point, seed ^ 0x5bd1e995u, radius));
            acc = sample_field(acc, point + offset);
        }
        case 7u: {
            acc = sample_field(acc, point + params.direction.xy * warp_offset(point, seed, radius));
        }
        case 8u: {
            let left = normalized_or(sample_field(EMPTY, point - vec2(radius, 0.0)), original);
            let right = normalized_or(sample_field(EMPTY, point + vec2(radius, 0.0)), original);
            let top = normalized_or(sample_field(EMPTY, point - vec2(0.0, radius)), original);
            let bottom = normalized_or(sample_field(EMPTY, point + vec2(0.0, radius)), original);
            let dx = right.rgb - left.rgb;
            let dy = bottom.rgb - top.rgb;
            let gradient = min(sqrt(dx * dx + dy * dy), vec3(1.0));
            return vec4(gradient * original.a, original.a);
        }
        case 9u: {
            let d = vec2(
                height_noise(point, vec2(-0.01, 0.0), seed) - height_noise(point, vec2(0.01, 0.0), seed),
                height_noise(point, vec2(0.0, -0.01), seed) - height_noise(point, vec2(0.0, 0.01), seed),
            );
            let direction = d / max(length(d), 1e-20);
            for (var i = 0; i <= n; i++) {
                acc = sample_field(acc, point + direction * (f32(i) * step));
            }
        }
        case 10u: {
            let center = (floor(point / radius) + 0.5) * radius - 0.5;
            acc = sample_field(acc, center);
        }
        default: {
            let kind = params.mode.y;
            if kind >= 16u && kind < 32u { return box_family_gathered(point, original); }
            if kind >= 32u && kind < 56u { return gather_family_gathered(point, original); }
            return original;
        }
    }
    return normalized_or(acc, original);
}

@compute @workgroup_size(8, 8)
fn apply_neighbourhood(@builtin(global_invocation_id) id: vec3<u32>) {
    let point = id.xy;
    if any(point >= params.rect.zw) { return; }
    let original = original_at(point);
    var filtered = original.value;
    if original.coverage > 0.0 {
        if uses_box_kind(params.mode.y) {
            let blurred = read_at(params.buffers.y, point);
            if blurred.coverage > MIN_WEIGHT {
                filtered = filtered_box(original.value, blurred.value / blurred.coverage);
            }
        } else {
            filtered = gathered(vec2<f32>(vec2<i32>(point) + field_origin()), original.value);
        }
        filtered = original.value + (filtered - original.value) * params.weights.x;
    }
    write_at(params.buffers.z, point, Texel(filtered, original.coverage));
}

fn first_cell() -> vec2<u32> {
    return params.rect.xy / params.mode.x;
}

fn cell_extent() -> vec2<u32> {
    let cell = params.mode.x;
    return (params.rect.xy + params.rect.zw + vec2(cell - 1u)) / cell - first_cell();
}

@compute @workgroup_size(8, 8)
fn average_cells(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= cell_extent()) { return; }
    let cell = params.mode.x;
    let origin = (first_cell() + id.xy) * cell;
    let first = max(origin, params.rect.xy) - params.rect.xy;
    let end = min(origin + vec2(cell), params.rect.xy + params.rect.zw) - params.rect.xy;
    var sum = 0.0;
    var count = 0u;
    for (var y = first.y; y < end.y; y++) {
        for (var x = first.x; x < end.x; x++) {
            let sample = original_at(vec2(x, y));
            if sample.coverage > 0.0 {
                sum += sample.value.x;
                count += 1u;
            }
        }
    }
    let value = sum / f32(max(count, 1u));
    write_at(params.buffers.y, id.xy, Texel(vec4(value, 0.0, 0.0, 1.0), select(0.0, 1.0, count != 0u)));
}

fn cell_corner(point: vec2<i32>, weight: f32) -> vec2<f32> {
    let cell = vec2<u32>(clamp(point, vec2(0), vec2<i32>(cell_extent()) - 1));
    let sample = read_at(params.buffers.y, cell);
    return vec2(sample.value.x, 1.0) * (weight * sample.coverage);
}

@compute @workgroup_size(8, 8)
fn interpolate_cells(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.rect.zw) { return; }
    let original = original_at(id.xy);
    var value = original.value.x;
    if original.coverage > 0.0 {
        let position = (vec2<f32>(params.rect.xy + id.xy) + 0.5) / f32(params.mode.x) - 0.5 - vec2<f32>(first_cell());
        let cell = vec2<i32>(floor(position));
        let f = fract(position);
        let weighted = cell_corner(cell, (1.0 - f.x) * (1.0 - f.y))
            + cell_corner(cell + vec2(1, 0), f.x * (1.0 - f.y))
            + cell_corner(cell + vec2(0, 1), (1.0 - f.x) * f.y)
            + cell_corner(cell + vec2(1), f.x * f.y);
        if weighted.y > MIN_WEIGHT {
            value = weighted.x / weighted.y;
        }
    }
    write_at(params.buffers.z, id.xy, Texel(vec4(value, original.value.yzw), original.coverage));
}
