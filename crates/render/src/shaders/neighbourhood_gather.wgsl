// P5, the gather and whole-set filters (gpu_neighbourhood_gather.rs): kinds 32..48 (gather) and
// 48..56 (whole set). Owned by the P5 agent. Always part of the neighbourhood shader; extra
// passes (structure tensor, reductions) get their own entry points here, created in
// gather::Pipelines.

// The P5 kinds (gpu_neighbourhood_gather.rs)
const GATHER_FXAA: u32 = 32u;
const GATHER_KUWAHARA: u32 = 33u;
const GATHER_SOURCE_WARP: u32 = 34u;
const GATHER_SOURCE_SLOPE_BLUR: u32 = 35u;
const GATHER_MIRROR: u32 = 36u;
const GATHER_TRANSFORM: u32 = 37u;
const GATHER_COLOR_MATCH: u32 = 48u;

// FXAA (fxaa_texel in eval.rs): edge search, least contrast and most subpixel blend
const GATHER_FXAA_SEARCH: i32 = 8;
const GATHER_FXAA_EDGE_MIN: f32 = 0.0312;
const GATHER_FXAA_EDGE_RELATIVE: f32 = 0.125;
const GATHER_FXAA_SUBPIXEL: f32 = 0.75;
// Flag bit of an FXAA stage's words[0]: Affect Alpha Channel
const GATHER_FXAA_ALPHA: u32 = 1u;

// Flag bits of a Mirror stage's words[0]: axes, copied sides and Display Debug
const GATHER_MIRROR_X: u32 = 1u;
const GATHER_MIRROR_Y: u32 = 2u;
const GATHER_MIRROR_KEEP_RIGHT: u32 = 4u;
const GATHER_MIRROR_KEEP_BOTTOM: u32 = 8u;
const GATHER_MIRROR_DEBUG: u32 = 16u;

// Flag bits of a Color Match stage's words[0]: the source is the set's mean, and Custom Color Variation
const GATHER_COLOR_MATCH_AVERAGE: u32 = 1u;
const GATHER_COLOR_MATCH_CUSTOM_VARIATION: u32 = 2u;

// Anisotropic Kuwahara (kuwahara_texel in eval.rs): ellipse steps, sectors and weights
const GATHER_KUWAHARA_STEPS: i32 = 6;
const GATHER_KUWAHARA_SECTORS: u32 = 8u;
const GATHER_MIN_TENSOR: f32 = 1e-12;
const GATHER_KUWAHARA_FALLOFF: f32 = 2.0;
const GATHER_KUWAHARA_SPREAD_SCALE: f32 = 255.0;
// Flag bit of a Kuwahara stage's words[0]: Extract Direction Custom Input
const GATHER_KUWAHARA_CUSTOM_INPUT: u32 = 1u;
const GATHER_PI: f32 = 3.141592653589793;
const GATHER_HALF_PI: f32 = 1.5707963267948966;
const GATHER_TAU: f32 = 6.283185307179586;

// Bit fields of a Warp's or Blur Slope's words[0] (gather::source_warp, source_slope_blur)
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
// Mid gray moves nothing; noise is stretched from it (WarpSource in core)
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
// An unlinked image input's gray (ImageInput's default); linked inputs stay on the CPU
const GATHER_INPUT_DEFAULT: f32 = 0.5;

const GATHER_LARGEST_FINITE: f32 = 3.402823466e38;

fn gather_finite(value: f32) -> bool {
    return abs(value) <= GATHER_LARGEST_FINITE;
}

// Whether the stage filters content (four premultiplied components) rather than a mask's value
fn gather_is_content() -> bool {
    return stage_context() != STAGE_MASK;
}

// Offset of the texel's copy of the set from its home copy (wrapping programs' periodic fields)
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

// A read laid on the texel's own alpha `own` (keep_coverage in eval.rs; content only)
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

// FXAA at `home`: the read across its edge, no weight where nothing blends (fxaa_texel in eval.rs)
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

// An unlinked image input at `home`: (default, 1) where covered, else (0, 0)
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

// A warp's source at `position` before shaping, (value, found); `texel` picks the repeat's tile
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

// The per-component extreme of the covered reads (keep_extreme_read in warp_texel.rs)
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

// atan2 with Rust's results where y is zero, which WGSL leaves open at the origin
fn gather_atan2(y: f32, x: f32) -> f32 {
    if y == 0.0 {
        return select(0.0, GATHER_PI, x < 0.0);
    }

    return atan2(y, x);
}

// Kuwahara's direction gray at `home`: its custom input or the luma, (gray, found)
fn gather_kuwahara_gray(home: vec2<f32>) -> vec2<f32> {
    if (params.field_layout.z & GATHER_KUWAHARA_CUSTOM_INPUT) != 0u {
        return gather_input(home);
    }

    let found = gather_read(home);
    if found.coverage <= MIN_WEIGHT {
        return vec2(0.0);
    }

    return vec2(gather_luma(found.value / found.coverage), 1.0);
}

// The gray at `home` plus `offset`, or `own` where it covers nothing
fn gather_kuwahara_gray_or(home: vec2<f32>, offset: vec2<f32>, own: vec2<f32>) -> vec2<f32> {
    let gray = gather_kuwahara_gray(home + offset);
    return select(own, gray, gray.y > 0.0);
}

// The ellipse at `home` from the structure tensor: (cos, sin, along, across)
fn gather_kuwahara_ellipse(home: vec2<f32>) -> vec4<f32> {
    let radius = params.extra[0].x;
    let spread = params.extra[0].y * radius;
    var xx = 0.0;
    var xy = 0.0;
    var yy = 0.0;
    for (var grid_y = -1; grid_y <= 1; grid_y++) {
        for (var grid_x = -1; grid_x <= 1; grid_x++) {
            let at = home + vec2(f32(grid_x) * spread, f32(grid_y) * spread);
            let own = gather_kuwahara_gray(at);
            let left = gather_kuwahara_gray_or(at, vec2(-1.0, 0.0), own);
            let right = gather_kuwahara_gray_or(at, vec2(1.0, 0.0), own);
            let up = gather_kuwahara_gray_or(at, vec2(0.0, -1.0), own);
            let down = gather_kuwahara_gray_or(at, vec2(0.0, 1.0), own);
            if left.y <= 0.0 || right.y <= 0.0 || up.y <= 0.0 || down.y <= 0.0 {
                continue;
            }

            // Central differences over two texels, weighed by a 1-2-1 binomial kernel each way
            let dx = (right.x - left.x) * 0.5;
            let dy = (down.x - up.x) * 0.5;
            let weight = f32(4 >> u32(abs(grid_x) + abs(grid_y)));
            xx += weight * dx * dx;
            xy += weight * dx * dy;
            yy += weight * dy * dy;
        }
    }
    let root = sqrt((xx - yy) * (xx - yy) + 4.0 * xy * xy);
    var clear = 0.0;
    if xx + yy > GATHER_MIN_TENSOR {
        clear = root / (xx + yy);
    }

    // Strokes run across the steepest change; Anisotropy Angle turns the ellipse counterclockwise on screen
    let turned = 0.5 * gather_atan2(2.0 * xy, xx - yy) + GATHER_HALF_PI - params.extra[1].y;
    var frame = vec2(1.0, 0.0);
    if gather_finite(turned) {
        frame = vec2(cos(turned), sin(turned));
    }
    let stretch = params.extra[1].x * clamp(select(0.0, clear, gather_finite(clear)), 0.0, 1.0);
    return vec4(frame, radius * (1.0 + stretch), radius / (1.0 + stretch));
}

// Kuwahara's tensor pass: each covered texel's ellipse into buffers.z
@compute @workgroup_size(8, 8)
fn gather_kuwahara_frame(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.rect.z || id.y >= params.rect.w {
        return;
    }

    var ellipse = vec4(0.0);
    if original_at(id.xy).coverage > 0.0 {
        output_copy = copy_of(vec2<i32>(id.xy));
        ellipse = gather_kuwahara_ellipse(gather_begin(vec2<f32>(vec2<i32>(id.xy) + field_origin())));
    }
    write_at(params.buffers.z, id.xy, Texel(ellipse, 0.0));
}

// Anisotropic Kuwahara at `home`: its sectors' means weighed by their calm (kuwahara_texel in eval.rs)
fn gather_kuwahara(home: vec2<f32>, ellipse: vec4<f32>) -> Texel {
    let content = gather_is_content();
    let sector_exponent = params.extra[0].z;
    let sharpness = params.extra[0].w;
    var steps = 1;
    if gather_finite(ellipse.z) {
        steps = clamp(i32(ceil(ellipse.z)), 1, GATHER_KUWAHARA_STEPS);
    }

    // Each sector's weighted sum, squared sum and weight
    var sums: array<vec4<f32>, 8>;
    var squares: array<f32, 8>;
    var weights: array<f32, 8>;
    for (var row = -steps; row <= steps; row++) {
        for (var column = -steps; column <= steps; column++) {
            let u = f32(column) / f32(steps);
            let v = f32(row) / f32(steps);
            let spread = u * u + v * v;
            if spread > 1.0 {
                continue;
            }

            let across = u * ellipse.z;
            let along = v * ellipse.w;
            let found = gather_read(home + vec2(across * ellipse.x - along * ellipse.y, across * ellipse.y + along * ellipse.x));
            if found.coverage <= MIN_WEIGHT {
                continue;
            }

            var value = found.value / found.coverage;
            var squared = value.x * value.x;
            if content {
                squared = value.x * value.x + value.y * value.y + value.z * value.z;
            } else {
                value = vec4(value.x, 0.0, 0.0, 0.0);
            }
            let falloff = exp(-GATHER_KUWAHARA_FALLOFF * spread);
            let heading = gather_atan2(v, u);
            for (var sector = 0u; sector < GATHER_KUWAHARA_SECTORS; sector++) {
                // The center contributes to every sector while other weights taper at sector boundaries
                let facing = cos(heading - f32(sector) * GATHER_TAU / f32(GATHER_KUWAHARA_SECTORS));
                var weight = falloff;
                if spread > 0.0 {
                    if facing <= 0.0 {
                        continue;
                    }

                    weight = falloff * pow(facing, sector_exponent);
                }
                sums[sector] += weight * value;
                weights[sector] += weight;
                squares[sector] += weight * squared;
            }
        }
    }

    // Weight low-variance sectors more and use logarithms to avoid overflow
    var logs: array<f32, 8>;
    var top = 0.0;
    var any_sector = false;
    for (var sector = 0u; sector < GATHER_KUWAHARA_SECTORS; sector++) {
        let weight = weights[sector];
        if weight <= MIN_WEIGHT {
            continue;
        }

        let mean = sums[sector] / weight;
        var mean_squared = mean.x * mean.x;
        if content {
            mean_squared = mean.x * mean.x + mean.y * mean.y + mean.z * mean.z;
        }
        let deviation = sqrt(max(squares[sector] / weight - mean_squared, 0.0));
        logs[sector] = -sharpness * log(1.0 + GATHER_KUWAHARA_SPREAD_SCALE * deviation);
        top = select(logs[sector], max(top, logs[sector]), any_sector);
        any_sector = true;
    }
    if !any_sector {
        return EMPTY;
    }

    var total = EMPTY;
    for (var sector = 0u; sector < GATHER_KUWAHARA_SECTORS; sector++) {
        let weight = weights[sector];
        if weight <= MIN_WEIGHT {
            continue;
        }

        let share = exp(logs[sector] - top);
        total = Texel(total.value + share * sums[sector] / weight, total.coverage + share);
    }
    return total;
}

// A compensated sum (Kahan): the CPU sums the mean in f64
struct GatherSum {
    sum: vec4<f32>,
    correction: vec4<f32>,
}

fn gather_add(total: GatherSum, value: vec4<f32>) -> GatherSum {
    let added = value - total.correction;
    let sum = total.sum + added;
    return GatherSum(sum, (sum - total.sum) - added);
}

fn gather_write_words(start: u32, value: vec4<f32>) {
    let words = bitcast<vec4<u32>>(value);
    fields[start] = words.x;
    fields[start + 1u] = words.y;
    fields[start + 2u] = words.z;
    fields[start + 3u] = words.w;
}

fn gather_read_words(start: u32) -> vec4<f32> {
    return bitcast<vec4<f32>>(vec4(fields[start], fields[start + 1u], fields[start + 2u], fields[start + 3u]));
}

// Color Match's mean, pass 1: each row's premultiplied sums over the set, four words per row
@compute @workgroup_size(64)
fn gather_color_match_rows(@builtin(global_invocation_id) id: vec3<u32>) {
    let row = id.x;
    if row >= params.rect.w {
        return;
    }

    var total = GatherSum(vec4(0.0), vec4(0.0));
    let in_set = i32(row) >= params.bounds.y && i32(row) < params.bounds.w;
    let first = u32(clamp(params.bounds.x, 0, i32(params.rect.z)));
    let end = u32(clamp(params.bounds.z, 0, i32(params.rect.z)));
    for (var column = first; in_set && column < end; column++) {
        let texel = original_at(vec2(column, row));
        if texel.value.w > 0.0 && gather_finite(texel.value.w) {
            total = gather_add(total, texel.value);
        }
    }
    gather_write_words(params.buffers.z + row * 4u, total.sum);
}

// Color Match's mean, pass 2: the rows totalled into the mean and whether any texel had alpha
@compute @workgroup_size(1)
fn gather_color_match_total(@builtin(global_invocation_id) id: vec3<u32>) {
    var total = GatherSum(vec4(0.0), vec4(0.0));
    for (var row = 0u; row < params.rect.w; row++) {
        total = gather_add(total, gather_read_words(params.buffers.y + row * 4u));
    }

    var mean = vec4(0.0);
    if total.sum.w > 0.0 {
        mean = vec4(total.sum.xyz / total.sum.w, 1.0);
    }
    gather_write_words(params.buffers.z, mean);
}

// Linear sRGB to OKLab (from_linear in core's oklab.rs)
fn gather_cube_root(value: f32) -> f32 {
    return select(0.0, pow(value, 1.0 / 3.0), value > 0.0);
}

fn gather_oklab(color: vec3<f32>) -> vec3<f32> {
    let l = gather_cube_root(0.41222146 * color.x + 0.53633255 * color.y + 0.051445995 * color.z);
    let m = gather_cube_root(0.2119035 * color.x + 0.6806995 * color.y + 0.10739696 * color.z);
    let s = gather_cube_root(0.08830246 * color.x + 0.28171885 * color.y + 0.6299787 * color.z);
    return vec3(
        0.21045426 * l + 0.7936178 * m - 0.004072047 * s,
        1.9779985 * l - 2.4285922 * m + 0.4505937 * s,
        0.025904037 * l + 0.78277177 * m - 0.80867577 * s,
    );
}

// OKLab back to linear sRGB, unclamped (to_linear in core's oklab.rs)
fn gather_linear(lab: vec3<f32>) -> vec3<f32> {
    let l = lab.x + 0.39633778 * lab.y + 0.21580376 * lab.z;
    let m = lab.x - 0.105561346 * lab.y - 0.06385417 * lab.z;
    let s = lab.x - 0.08948418 * lab.y - 1.2914855 * lab.z;
    let l3 = l * l * l;
    let m3 = m * m * m;
    let s3 = s * s * s;
    return vec3(
        4.0767417 * l3 - 3.3077116 * m3 + 0.23096994 * s3,
        -1.268438 * l3 + 2.6097574 * m3 - 0.34131938 * s3,
        -0.0041960863 * l3 - 0.7034186 * m3 + 1.7076147 * s3,
    );
}

// A colour's OKLab reference, polar with Custom Color Variation (PreparedColorMatch in core)
fn gather_color_reference(color: vec3<f32>, polar: bool) -> vec3<f32> {
    let lab = gather_oklab(max(color, vec3(0.0)));
    if !polar {
        return lab;
    }

    return vec3(lab.x, sqrt(lab.y * lab.y + lab.z * lab.z), gather_atan2(lab.z, lab.y));
}

// Color Match on colour content (color_match in eval.rs, ColorMatch::move_polar)
fn gather_color_match(original: vec4<f32>) -> vec4<f32> {
    let alpha = original.w;
    if stage_context() != STAGE_COLOR || !(alpha > 0.0) || !gather_finite(alpha) {
        return original;
    }

    let flags = params.field_layout.z;
    let polar = (flags & GATHER_COLOR_MATCH_CUSTOM_VARIATION) != 0u;
    let color = original.xyz / alpha;
    var source = params.extra[0].xyz;
    if (flags & GATHER_COLOR_MATCH_AVERAGE) != 0u {
        let mean = gather_read_words(params.buffers.y);
        if mean.w > 0.0 {
            source = gather_color_reference(mean.xyz, polar);
        }
    }
    let goal = params.extra[1].xyz;
    let lab = gather_oklab(max(color, vec3(0.0)));
    var moved = goal + lab - source;
    if polar {
        let texel = vec3(lab.x, sqrt(lab.y * lab.y + lab.z * lab.z), gather_atan2(lab.z, lab.y));
        let hue_limit = params.extra[0].w;
        let chroma_limit = params.extra[1].w;
        let luma_limit = params.weights.y;
        let turn = gather_rem_euclid(texel.z - source.z + GATHER_PI, GATHER_TAU) - GATHER_PI;
        let lightness = goal.x + clamp(texel.x - source.x, -luma_limit, luma_limit);
        let chroma = max(goal.y + clamp(texel.y - source.y, -chroma_limit, chroma_limit), 0.0);
        let hue = goal.z + clamp(turn, -hue_limit, hue_limit);
        moved = vec3(lightness, chroma * cos(hue), chroma * sin(hue));
    }

    // A move that leaves the colours gives the colour back
    let linear = gather_linear(moved);
    var result = color;
    if gather_finite(linear.x) && gather_finite(linear.y) && gather_finite(linear.z) {
        result = clamp(linear, vec3(0.0), vec3(1.0));
    }
    return vec4(result * alpha, alpha);
}

// Rust's rem_euclid for a positive modulus
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

// A tile coordinate folded back into 0..1, as a mirror reflects (fold in core's mirror.rs)
fn gather_fold(at: f32) -> f32 {
    let wrapped = gather_rem_euclid(at, 2.0);
    return select(wrapped, 2.0 - wrapped, wrapped > 1.0);
}

// Where along one axis a mirror reads (Mirror::source): the copied side reads across the axis
fn gather_mirror_across(at: f32, axis: f32, keep_after: bool) -> f32 {
    let copied = select(at > axis, at < axis, keep_after);
    return gather_fold(select(at, 2.0 * axis - at, copied));
}

// Mirror at `home`: its tile's kept side, or white on a debug axis (mirror_texel in eval.rs)
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

// Transform at `home`: bilinear over its tile, empty past it unless it repeats (transform_texel in eval.rs)
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
    if kind == GATHER_COLOR_MATCH {
        return gather_color_match(original);
    }

    let flags = params.field_layout.z;
    let home = gather_begin(point);
    var acc = EMPTY;
    if kind == GATHER_FXAA {
        acc = gather_fxaa(home, original);
        if (flags & GATHER_FXAA_ALPHA) == 0u {
            acc = gather_keep_coverage(acc, original.w);
        }
    } else if kind == GATHER_KUWAHARA {
        let local = vec2<u32>(vec2<i32>(point) - field_origin());
        acc = gather_kuwahara(home, read_at(params.buffers.y, local).value);
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
