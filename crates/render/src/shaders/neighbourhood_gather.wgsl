// P5, the gather and whole-set filters (gpu_neighbourhood_gather.rs): kinds 32..48 (gather) and
// 48..56 (whole set). Owned by the P5 agent. Always part of the neighbourhood shader; extra
// passes (structure tensor, reductions) get their own entry points here, created in
// gather::Pipelines.

// The P5 kinds (gpu_neighbourhood_gather.rs)
const GATHER_FXAA: u32 = 32u;
const GATHER_SOURCE_WARP: u32 = 34u;
const GATHER_SOURCE_SLOPE_BLUR: u32 = 35u;
const GATHER_MIRROR: u32 = 36u;
const GATHER_TRANSFORM: u32 = 37u;

// FXAA (fxaa_texel in eval.rs): the edge search, the least contrast it smooths and the most a
// lone texel blends in
const GATHER_FXAA_SEARCH: i32 = 8;
const GATHER_FXAA_EDGE_MIN: f32 = 0.0312;
const GATHER_FXAA_EDGE_RELATIVE: f32 = 0.125;
const GATHER_FXAA_SUBPIXEL: f32 = 0.75;
// Flag bit of an FXAA stage's words[0]: Affect Alpha Channel
const GATHER_FXAA_ALPHA: u32 = 1u;

// Flag bits of a Mirror stage's words[0]: the axes it mirrors across, whether each copies its
// right (bottom) side, and Display Debug
const GATHER_MIRROR_X: u32 = 1u;
const GATHER_MIRROR_Y: u32 = 2u;
const GATHER_MIRROR_KEEP_RIGHT: u32 = 4u;
const GATHER_MIRROR_KEEP_BOTTOM: u32 = 8u;
const GATHER_MIRROR_DEBUG: u32 = 16u;

// Bit fields of a Warp's or Blur Slope's words[0]: the warp mode, the blending, the directions,
// the source's mode, whether the source repeats within the UV tile, and the read count
const GATHER_WARP_MODE_SHIFT: u32 = 0u;
const GATHER_BLENDING_SHIFT: u32 = 2u;
const GATHER_DIRECTION_COUNT_SHIFT: u32 = 4u;
const GATHER_SOURCE_MODE_SHIFT: u32 = 8u;
const GATHER_SOURCE_TILED: u32 = 1024u;
const GATHER_READ_COUNT_SHIFT: u32 = 16u;
const GATHER_WARP_MODE_WARP: u32 = 0u;
const GATHER_WARP_MODE_MULTI_DIRECTIONAL: u32 = 2u;
const GATHER_WARP_BLENDING_MAX: u32 = 1u;
const GATHER_WARP_BLENDING_MIN: u32 = 2u;
const GATHER_WARP_BLENDING_CHAIN: u32 = 3u;
const GATHER_SLOPE_BLENDING_BLUR: u32 = 0u;
const GATHER_SLOPE_BLENDING_MAX: u32 = 2u;
const GATHER_SOURCE_DEFAULT_NOISE: u32 = 0u;
const GATHER_SOURCE_CUSTOM_NOISE: u32 = 2u;
// Mid gray, the source value that moves nothing, and how far noise is stretched from it
// (WarpSource in core)
const GATHER_MID_GRAY: f32 = 0.5;
const GATHER_NOISE_STRETCH: f32 = 1.5;
// Octaves of a warp's default noise (texel_noise in eval.rs)
const GATHER_NOISE_OCTAVES: u32 = 3u;
// The source's slope (rise per UV tile) that moves a texel the whole way (SourceWarp::FULL_SLOPE)
const GATHER_FULL_SLOPE: f32 = 4.0;
// A source rising less than this across a texel is flat (slope_texel.rs)
const GATHER_FLAT_STEEPNESS: f32 = 1e-6;
// A content read covering less alpha than this has no colour (MIN_ALPHA in eval.rs)
const GATHER_MIN_ALPHA: f32 = 1e-5;
// The gray an image input reads until one is linked (ImageInput's default): linked inputs still
// composite on the CPU (effect_needs_cpu)
const GATHER_INPUT_DEFAULT: f32 = 0.5;

const GATHER_LARGEST_FINITE: f32 = 3.402823466e38;

fn gather_finite(value: f32) -> bool {
    return abs(value) <= GATHER_LARGEST_FINITE;
}

// Whether the stage filters content (four premultiplied components) rather than a mask's value
fn gather_is_content() -> bool {
    return stage_context() != STAGE_MASK;
}

// How far the texel being written lies from its home copy of the set, in texels. A wrapping
// program's fields hold periodic copies past the set's edges: positions are worked out at home
// (noise, UV tiles, mirrors) and read in the texel's own copy, as the CPU's wrapped lower stages
// are evaluated over the set and read periodically
var<private> gather_shift: vec2<f32> = vec2(0.0);

// The home position of the texel at absolute position `point`, which sets gather_shift
fn gather_begin(point: vec2<f32>) -> vec2<f32> {
    let local = vec2<i32>(point) - field_origin();
    gather_shift = vec2<f32>(copy_of(local) * (params.bounds.zw - params.bounds.xy));
    return point - gather_shift;
}

// A bilinear read at home position `home` added to `acc` (premultiplied components, then weight)
fn gather_sample(acc: Texel, home: vec2<f32>) -> Texel {
    return sample_field(acc, home + gather_shift);
}

fn gather_read(home: vec2<f32>) -> Texel {
    return gather_sample(EMPTY, home);
}

// The luma of premultiplied components (lum in core's blend.rs), or a mask's value
fn gather_luma(value: vec4<f32>) -> f32 {
    if gather_is_content() {
        return 0.3 * value.x + 0.59 * value.y + 0.11 * value.z;
    }
    return value.x;
}

// A read recoloured over the texel's own alpha `own` (keep_coverage in eval.rs; content only):
// no weight where the read carries no alpha
fn gather_keep_coverage(acc: Texel, own: f32) -> Texel {
    if !gather_is_content() {
        return acc;
    }
    if acc.value.w <= MIN_WEIGHT {
        return EMPTY;
    }

    let scale = own * acc.coverage / acc.value.w;
    return Texel(vec4(acc.value.xyz * scale, own * acc.coverage), acc.coverage);
}

// The luma at home position `home`; off the UV island the texel's own, so seams are not edges
fn gather_fxaa_luma(home: vec2<f32>, own: f32) -> f32 {
    let found = gather_read(home);
    if found.coverage <= MIN_WEIGHT {
        return own;
    }
    return gather_luma(found.value / found.coverage);
}

// Walk the edge's midline one way until its luma leaves the edge's: (steps, last difference)
fn gather_fxaa_walk(home: vec2<f32>, perp: vec2<f32>, along: vec2<f32>, way: f32, edge: f32, enough: f32, own: f32) -> vec2<f32> {
    var end = vec2(f32(GATHER_FXAA_SEARCH), 0.0);
    for (var step = 1; step <= GATHER_FXAA_SEARCH; step++) {
        let t = f32(step) * way;
        let delta = gather_fxaa_luma(home + (perp * 0.5 + along * t), own) - edge;
        end = vec2(f32(step), delta);
        if abs(delta) >= enough {
            break;
        }
    }
    return end;
}

// FXAA at home position `home` with premultiplied components `center`: the read across its edge
// by how near the edge's end is (fxaa_texel in eval.rs); no weight where nothing blends
fn gather_fxaa(home: vec2<f32>, center: vec4<f32>) -> Texel {
    let own = gather_luma(center);
    let n = gather_fxaa_luma(home + vec2(0.0, -1.0), own);
    let s = gather_fxaa_luma(home + vec2(0.0, 1.0), own);
    let w = gather_fxaa_luma(home + vec2(-1.0, 0.0), own);
    let e = gather_fxaa_luma(home + vec2(1.0, 0.0), own);
    let hi = max(max(max(max(own, n), s), w), e);
    let lo = min(min(min(min(own, n), s), w), e);
    let range = hi - lo;
    if !gather_finite(range) || range < max(GATHER_FXAA_EDGE_MIN, hi * GATHER_FXAA_EDGE_RELATIVE) {
        return EMPTY;
    }

    let nw = gather_fxaa_luma(home + vec2(-1.0, -1.0), own);
    let ne = gather_fxaa_luma(home + vec2(1.0, -1.0), own);
    let sw = gather_fxaa_luma(home + vec2(-1.0, 1.0), own);
    let se = gather_fxaa_luma(home + vec2(1.0, 1.0), own);

    // An edge running across (values changing down the rows) or down (values changing along them)
    let across = 2.0 * abs(n - 2.0 * own + s) + abs(nw - 2.0 * w + sw) + abs(ne - 2.0 * e + se);
    let down = 2.0 * abs(w - 2.0 * own + e) + abs(nw - 2.0 * n + ne) + abs(sw - 2.0 * s + se);
    let horizontal = across >= down;
    let before = select(w, n, horizontal);
    let after = select(e, s, horizontal);
    var toward = 1.0;
    var side = after;
    if abs(before - own) >= abs(after - own) {
        toward = -1.0;
        side = before;
    }
    let perp = select(vec2(toward, 0.0), vec2(0.0, toward), horizontal);
    let along = select(vec2(0.0, 1.0), vec2(1.0, 0.0), horizontal);

    // Walk the edge's midline both ways until its luma leaves the edge's
    let edge = 0.5 * (own + side);
    let enough = 0.25 * abs(side - own);
    let back = gather_fxaa_walk(home, perp, along, -1.0, edge, enough, own);
    let ahead = gather_fxaa_walk(home, perp, along, 1.0, edge, enough, own);
    let nearest = select(ahead, back, back.x < ahead.x);
    var edge_blend = 0.0;
    if (nearest.y < 0.0) != (own < edge) {
        edge_blend = 0.5 - nearest.x / (back.x + ahead.x);
    }

    // A texel unlike all its neighbours blends in by its own contrast (sides weigh twice the corners)
    let average = (2.0 * (n + s + w + e) + nw + ne + sw + se) / 12.0;
    let contrast = clamp(abs(average - own) / range, 0.0, 1.0);
    let eased = contrast * contrast * (3.0 - 2.0 * contrast);
    let blend = max(edge_blend, eased * eased * GATHER_FXAA_SUBPIXEL);
    if blend > 0.0 {
        return gather_read(home + perp * blend);
    }
    return EMPTY;
}

// finite_unit in core: clamped to 0..1, 0 when not finite
fn gather_finite_unit(value: f32) -> f32 {
    return select(0.0, clamp(value, 0.0, 1.0), gather_finite(value));
}

// An image input's gray at home position `home` (the input closure of gather in eval.rs):
// bilinear over the covered texels, (gray, 1), or (0, 0) where it covers nothing. Until one is
// linked an input reads its default everywhere it covers
fn gather_input(home: vec2<f32>) -> vec2<f32> {
    if gather_read(home).coverage > MIN_WEIGHT {
        return vec2(GATHER_INPUT_DEFAULT, 1.0);
    }
    return vec2(0.0);
}

// The direction of a warp's `index`-th move, laid out in pairs in `extra`
fn gather_direction(index: u32) -> vec2<f32> {
    let pair = params.extra[index / 2u];
    return select(pair.xy, pair.zw, index % 2u == 1u);
}

// A warp's or slope blur's source at `position` before it is shaped (WarpSourceSample::raw):
// (value, 1), or (0, 0) where the values it reads cover nothing. `texel` is the home position of
// the texel being written, whose UV tile a repeating source stays in
fn gather_source_raw(texel: vec2<f32>, position: vec2<f32>) -> vec2<f32> {
    let flags = params.field_layout.z;
    let mode = (flags >> GATHER_SOURCE_MODE_SHIFT) & 3u;
    if mode == GATHER_SOURCE_DEFAULT_NOISE {
        let height = fbm(vec3(position * params.weights.z, 0.0), GATHER_NOISE_OCTAVES, params.mode.z, true);
        if !gather_finite(height) {
            return vec2(GATHER_MID_GRAY, 1.0);
        }
        return vec2(clamp(GATHER_MID_GRAY + (height - GATHER_MID_GRAY) * GATHER_NOISE_STRETCH, 0.0, 1.0), 1.0);
    }

    // Keep the bilinear taps in this UV tile at the ends of each repeat
    var at = position;
    if (flags & GATHER_SOURCE_TILED) != 0u {
        let side = vec2<f32>(params.bounds.zw - params.bounds.xy);
        let origin = floor(texel / side) * side;
        let repeated = (position - origin + 0.5) * params.weights.z;
        let wrapped = vec2(gather_rem_euclid(repeated.x, side.x), gather_rem_euclid(repeated.y, side.y)) - 0.5;
        at = origin + clamp(wrapped, vec2(0.0), side - 1.0);
    }
    if mode == GATHER_SOURCE_CUSTOM_NOISE {
        return gather_input(at);
    }

    // The filtered values' luminance; content stays premultiplied by its own coverage
    let found = gather_read(at);
    if gather_is_content() {
        if found.value.w > GATHER_MIN_ALPHA {
            let color = found.value.xyz / found.value.w;
            return vec2(0.3 * color.x + 0.59 * color.y + 0.11 * color.z, 1.0);
        }
        return vec2(0.0);
    }
    if found.coverage > MIN_WEIGHT {
        return vec2(found.value.x / found.coverage, 1.0);
    }
    return vec2(0.0);
}

// The source at `position` after its blur, Balance and Contrast (WarpSourceSample::at)
fn gather_source_at(texel: vec2<f32>, position: vec2<f32>) -> f32 {
    let half = params.direction.y;
    var value = GATHER_MID_GRAY;
    if half > 0.0 {
        var sum = 0.0;
        var count = 0.0;
        for (var corner = 0u; corner < 4u; corner++) {
            let offset = select(vec2(-half), vec2(half), vec2(corner & 1u, corner >> 1u) != vec2(0u));
            let raw = gather_source_raw(texel, position + offset);
            if raw.y > 0.0 {
                sum += raw.x;
                count += 1.0;
            }
        }
        if count > 0.0 {
            value = sum / count;
        }
    } else {
        let raw = gather_source_raw(texel, position);
        if raw.y > 0.0 {
            value = raw.x;
        }
    }

    // Balance and Contrast (shape_gray in core's gradient_input.rs); a steepness of 0 is a step
    let balanced = gather_finite_unit(value) + params.weights.w - GATHER_MID_GRAY;
    let steepness = bitcast<f32>(params.field_layout.w);
    if steepness == 0.0 {
        return select(0.0, 1.0, balanced >= GATHER_MID_GRAY);
    }
    return gather_finite_unit((balanced - GATHER_MID_GRAY) / steepness + GATHER_MID_GRAY);
}

// How far (-1..1 of the move) a source value moves a texel along a direction (SourceWarp::amount)
fn gather_warp_amount(value: f32) -> f32 {
    return select(0.0, clamp(2.0 * value - 1.0, -1.0, 1.0), gather_finite(value));
}

// The largest (or smallest) of reads, component by component (keep_extreme_read in
// warp_texel.rs): reads that cover nothing are skipped
struct GatherExtreme {
    picked: vec4<f32>,
    found: bool,
}

fn gather_extreme_add(extreme: GatherExtreme, home: vec2<f32>, larger: bool) -> GatherExtreme {
    let found = gather_read(home);
    if found.coverage <= MIN_WEIGHT {
        return extreme;
    }

    let value = found.value / found.coverage;
    if !extreme.found {
        return GatherExtreme(value, true);
    }
    return GatherExtreme(select(min(extreme.picked, value), max(extreme.picked, value), larger), true);
}

fn gather_extreme_texel(extreme: GatherExtreme) -> Texel {
    if !extreme.found {
        return EMPTY;
    }
    return Texel(extreme.picked, 1.0);
}

// Warp at home position `home` (warp_texel.rs): where its source sends the texel, or those reads blended
fn gather_source_warp(home: vec2<f32>) -> Texel {
    let flags = params.field_layout.z;
    let mode = (flags >> GATHER_WARP_MODE_SHIFT) & 3u;
    let blending = (flags >> GATHER_BLENDING_SHIFT) & 3u;
    let count = (flags >> GATHER_DIRECTION_COUNT_SHIFT) & 7u;
    let reach = params.weights.y;
    if mode == GATHER_WARP_MODE_WARP {
        let probe = params.direction.x;
        let rise = vec2(
            gather_source_at(home, home + vec2(probe, 0.0)) - gather_source_at(home, home - vec2(probe, 0.0)),
            gather_source_at(home, home + vec2(0.0, probe)) - gather_source_at(home, home - vec2(0.0, probe)),
        );

        // The slope per UV tile against the one that moves the whole way, at most the whole move
        let across = params.extra[0].x;
        let slope = vec2(
            select(0.0, rise.x / across / GATHER_FULL_SLOPE, gather_finite(rise.x)),
            select(0.0, rise.y / across / GATHER_FULL_SLOPE, gather_finite(rise.y)),
        );
        let slope_length = sqrt(slope.x * slope.x + slope.y * slope.y);
        let fit = select(1.0, 1.0 / slope_length, slope_length > 1.0);
        return gather_read(home + vec2(slope.x * fit * reach, slope.y * fit * reach));
    }

    // One move after the other, each from where the last one landed
    let multi = mode == GATHER_WARP_MODE_MULTI_DIRECTIONAL;
    if multi && blending == GATHER_WARP_BLENDING_CHAIN {
        var at = home;
        for (var index = 0u; index < count; index++) {
            let moved = gather_warp_amount(gather_source_at(home, at)) * reach;
            at = at + gather_direction(index) * moved;
        }
        return gather_read(at);
    }

    // Independent directions share the source at the original texel
    let moved = gather_warp_amount(gather_source_at(home, home)) * reach;
    if multi && (blending == GATHER_WARP_BLENDING_MAX || blending == GATHER_WARP_BLENDING_MIN) {
        var extreme = GatherExtreme(vec4(0.0), false);
        for (var index = 0u; index < count; index++) {
            extreme = gather_extreme_add(extreme, home + gather_direction(index) * moved, blending == GATHER_WARP_BLENDING_MAX);
        }
        return gather_extreme_texel(extreme);
    }
    var acc = EMPTY;
    for (var index = 0u; index < count; index++) {
        acc = gather_sample(acc, home + gather_direction(index) * moved);
    }
    return acc;
}

// Blur Slope at home position `home` (slope_texel.rs): the reads downhill of its source, blended
fn gather_source_slope_blur(home: vec2<f32>) -> Texel {
    let flags = params.field_layout.z;
    let probe = params.direction.x;
    let downhill = vec2(
        gather_source_at(home, home - vec2(probe, 0.0)) - gather_source_at(home, home + vec2(probe, 0.0)),
        gather_source_at(home, home - vec2(0.0, probe)) - gather_source_at(home, home + vec2(0.0, probe)),
    );
    let steepness = sqrt(downhill.x * downhill.x + downhill.y * downhill.y);

    // A flat spot keeps its value
    if !(steepness > GATHER_FLAT_STEEPNESS) {
        return gather_read(home);
    }

    let along = downhill / steepness;
    let reads = flags >> GATHER_READ_COUNT_SHIFT;
    let spacing = params.weights.y / f32(reads);
    let blending = (flags >> GATHER_BLENDING_SHIFT) & 3u;
    if blending == GATHER_SLOPE_BLENDING_BLUR {
        var acc = EMPTY;
        for (var taken = 0u; taken <= reads; taken++) {
            acc = gather_sample(acc, home + along * (f32(taken) * spacing));
        }
        return acc;
    }
    var extreme = GatherExtreme(vec4(0.0), false);
    for (var taken = 0u; taken <= reads; taken++) {
        extreme = gather_extreme_add(extreme, home + along * (f32(taken) * spacing), blending == GATHER_SLOPE_BLENDING_MAX);
    }
    return gather_extreme_texel(extreme);
}

// value.rem_euclid(modulus) as Rust computes it for a positive modulus: the truncated remainder,
// moved up by the modulus when below zero
fn gather_rem_euclid(value: f32, modulus: f32) -> f32 {
    let remainder = value - modulus * trunc(value / modulus);
    return select(remainder, remainder + modulus, remainder < 0.0);
}

// Rounding half away from zero, as Rust's f32::round (WGSL's round goes to even)
fn gather_round(value: f32) -> f32 {
    let whole = floor(abs(value));
    return sign(value) * (whole + select(0.0, 1.0, abs(value) - whole >= 0.5));
}

// The top left of the UV tile (`side` texels) a home position lies in
fn gather_tile_origin(home: vec2<f32>, side: f32) -> vec2<f32> {
    return floor(home / side) * side;
}

// A tile coordinate folded back into 0..1 at the tile's edges, as a mirror reflects (fold in
// core's mirror.rs)
fn gather_fold(at: f32) -> f32 {
    let wrapped = gather_rem_euclid(at, 2.0);
    return select(wrapped, 2.0 - wrapped, wrapped > 1.0);
}

// Where along one axis a mirror reads (Mirror::source): the copied side reads across the axis
fn gather_mirror_across(at: f32, axis: f32, keep_after: bool) -> f32 {
    let copied = select(at > axis, at < axis, keep_after);
    return gather_fold(select(at, 2.0 * axis - at, copied));
}

// Mirror at home position `home` (mirror_texel in eval.rs): the texel its tile's kept side gives,
// or white on a debug axis
fn gather_mirror(home: vec2<f32>, original: vec4<f32>) -> Texel {
    let flags = params.field_layout.z;
    let axes = params.extra[0].xy;
    let line = params.extra[0].z;
    let side = params.extra[0].w;
    let origin = gather_tile_origin(home, side);
    let local = home - origin;
    if local.x >= side || local.y >= side {
        return EMPTY;
    }

    let uv = (local + 0.5) / side;
    let on_x = (flags & GATHER_MIRROR_X) != 0u && abs(uv.x - axes.x) <= line;
    let on_y = (flags & GATHER_MIRROR_Y) != 0u && abs(uv.y - axes.y) <= line;
    if (flags & GATHER_MIRROR_DEBUG) != 0u && (on_x || on_y) {
        // Content stays premultiplied by its own coverage
        if gather_is_content() {
            return Texel(vec4(original.w), 1.0);
        }
        return Texel(vec4(1.0, original.yzw), 1.0);
    }

    var source = uv;
    if (flags & GATHER_MIRROR_X) != 0u {
        source.x = gather_mirror_across(uv.x, axes.x, (flags & GATHER_MIRROR_KEEP_RIGHT) != 0u);
    }
    if (flags & GATHER_MIRROR_Y) != 0u {
        source.y = gather_mirror_across(uv.y, axes.y, (flags & GATHER_MIRROR_KEEP_BOTTOM) != 0u);
    }

    // Whole texels, so an even mirror copies texels exactly
    let texel = vec2(gather_round(source.x * side - 0.5), gather_round(source.y * side - 0.5));
    return gather_read(origin + clamp(texel, vec2(0.0), vec2(side - 1.0)));
}

// Transform at home position `home` (transform_texel in eval.rs): bilinear between the four
// texels round where its affine reads in the tile, each wrapped round the tile; where it does not
// repeat, a read past the tile counts as empty
fn gather_transform(home: vec2<f32>) -> Texel {
    let columns = params.extra[0];
    let shift = params.extra[1].xy;
    let side = params.extra[1].z;
    let repeats = params.field_layout.z != 0u;
    let origin = gather_tile_origin(home, side);
    let local = home - origin;
    if local.x >= side || local.y >= side {
        return EMPTY;
    }

    // Texel space of the tile, centres at whole numbers
    let uv = (local + 0.5) / side;
    let at = vec2(columns.x * uv.x + columns.z * uv.y + shift.x, columns.y * uv.x + columns.w * uv.y + shift.y) * side - 0.5;
    if !gather_finite(at.x) || !gather_finite(at.y) {
        return EMPTY;
    }

    let base = floor(at);
    let t = at - base;
    var total = EMPTY;
    for (var corner = 0u; corner < 4u; corner++) {
        let offset = vec2(f32(corner & 1u), f32(corner >> 1u));
        let weights = select(1.0 - t, t, offset > vec2(0.5));
        let weight = weights.x * weights.y;
        if weight <= 0.0 {
            continue;
        }
        let texel = base + offset;
        let inside = texel.x >= 0.0 && texel.x < side && texel.y >= 0.0 && texel.y < side;
        if !repeats && !inside {
            // Empty: no value, full weight
            total.coverage += weight;
            continue;
        }

        // One whole texel: the read's own weights, so this matches its bilinear read
        let one = gather_read(origin + vec2(gather_rem_euclid(texel.x, side), gather_rem_euclid(texel.y, side)));
        total = Texel(total.value + one.value * weight, total.coverage + one.coverage * weight);
    }
    return total;
}

// Whether a P5 kind runs the three box passes first (gather::uses_box must agree).
fn gather_family_uses_box(kind: u32) -> bool {
    return false;
}

// A P5 kind after the box passes.
fn gather_family_filtered(original: vec4<f32>, blurred: vec4<f32>) -> vec4<f32> {
    return blurred;
}

// A P5 kind at absolute texel position point: sample_field and normalized_or read the original
// field, input_at the sixth word.
fn gather_family_gathered(point: vec2<f32>, original: vec4<f32>) -> vec4<f32> {
    let kind = params.mode.y;
    let flags = params.field_layout.z;
    let home = gather_begin(point);
    var acc = EMPTY;
    if kind == GATHER_FXAA {
        acc = gather_fxaa(home, original);
        if (flags & GATHER_FXAA_ALPHA) == 0u {
            acc = gather_keep_coverage(acc, original.w);
        }
    } else if kind == GATHER_SOURCE_WARP {
        acc = gather_source_warp(home);
    } else if kind == GATHER_SOURCE_SLOPE_BLUR {
        acc = gather_source_slope_blur(home);
    } else if kind == GATHER_MIRROR {
        acc = gather_mirror(home, original);
    } else if kind == GATHER_TRANSFORM {
        acc = gather_transform(home);
    } else {
        return original;
    }

    // A mask keeps its value alone (one component on the CPU)
    let result = normalized_or(acc, original);
    if gather_is_content() {
        return result;
    }
    return vec4(result.x, original.yzw);
}
