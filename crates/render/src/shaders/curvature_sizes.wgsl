struct Params {
    dimensions: vec4<u32>,  // Bake width/height, working width/height.
    mode: vec4<u32>,        // Radius, step, vertical flag, input representation.
    level: vec4<u32>,       // Destination atlas origin and size.
    block: vec4<u32>,       // Bake block width/height, working block side, block columns (0: no tile scales).
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var source: texture_2d<u32>;
@group(0) @binding(2) var destination_image: texture_storage_2d<r32uint, write>;
@group(0) @binding(3) var<storage, read> coverage: array<u32>;
@group(0) @binding(4) var bake: texture_2d<u32>;
// The coarsening's edge weights; in the coverage scan each block's tile scale
@group(0) @binding(5) var<storage, read> edge_weights: array<f32>;

const FIX: f32 = 16777216.0;

fn working_covered(point: vec2<u32>) -> bool {
    let pitch = (params.dimensions.z + 31u) / 32u;
    let bits = coverage[point.y * pitch + point.x / 32u];
    return (bits & (1u << (point.x % 32u))) != 0u;
}

// A bake texel of a UV tile atlas whose tiles have scales: covered when a covered working texel
// of its block reaches it through BlockScales::bake_uv, by its footprint (compose's covered_texels)
fn scaled_bake_covered(point: vec2<u32>) -> bool {
    let bake_size = params.dimensions.xy;
    let working_size = params.dimensions.zw;
    let bake_block = params.block.xy;
    let working_block = params.block.z;
    let cell = point / bake_block;
    let scale = edge_weights[cell.y * params.block.w + cell.x];
    let origin = cell * working_block;
    let end_of_block = min(origin + working_block, working_size);
    let per_bake = f32(working_block) * scale / vec2<f32>(bake_block);
    let local = vec2<f32>(point - cell * bake_block);
    let first = origin + vec2<u32>(max(floor(local * per_bake) - 1.0, vec2(0.0)));
    let end = min(origin + vec2<u32>(ceil((local + 1.0) * per_bake)) + 1u, end_of_block);
    let half_width = 0.5 * vec2<f32>(bake_size) / (vec2<f32>(working_size) * scale);
    for (var y = first.y; y < end.y; y++) {
        for (var x = first.x; x < end.x; x++) {
            if !working_covered(vec2(x, y)) { continue; }
            let uv = (vec2<f32>(vec2(x, y)) + 0.5) / vec2<f32>(working_size);
            let position = uv * vec2<f32>(working_size) / f32(working_block);
            let corner = floor(position);
            let tile_uv = (position - corner) / scale;
            if any(tile_uv < vec2(0.0)) || any(tile_uv >= vec2(1.0)) { continue; }
            let center = (corner + tile_uv) * f32(working_block) / vec2<f32>(working_size) * vec2<f32>(bake_size);
            if all(vec2<f32>(point) >= floor(center - half_width)) && all(vec2<f32>(point) < ceil(center + half_width)) {
                return true;
            }
        }
    }
    return false;
}

fn bake_covered(point: vec2<u32>) -> bool {
    let bake_size = params.dimensions.xy;
    let working_size = params.dimensions.zw;
    if params.block.w != 0u { return scaled_bake_covered(point); }
    if all(bake_size == working_size) { return working_covered(point); }

    // Invert the working-texel footprints, then verify their original float
    // bounds. The extra candidate on each end allows for roundoff at a bake edge.
    let scale = vec2<f32>(working_size) / vec2<f32>(bake_size);
    let first = vec2<u32>(max(floor(vec2<f32>(point) * scale) - 1.0, vec2(0.0)));
    let end = min(working_size, vec2<u32>(ceil(vec2<f32>(point + 1u) * scale)) + 1u);
    let half_width = 0.5 * vec2<f32>(bake_size) / vec2<f32>(working_size);
    for (var y = first.y; y < end.y; y++) {
        for (var x = first.x; x < end.x; x++) {
            if !working_covered(vec2(x, y)) { continue; }
            let center = ((vec2<f32>(vec2(x, y)) + 0.5) / vec2<f32>(working_size)) * vec2<f32>(bake_size);
            if all(vec2<f32>(point) >= floor(center - half_width)) && all(vec2<f32>(point) < ceil(center + half_width)) {
                return true;
            }
        }
    }
    return false;
}

// Build the summed-area table of the projected coverage, one scan per line.
@compute @workgroup_size(64)
fn scan_rows(@builtin(global_invocation_id) id: vec3<u32>) {
    let y = id.x;
    if y >= params.dimensions.y { return; }
    var sum = 0u;
    for (var x = 0u; x < params.dimensions.x; x++) {
        sum += u32(bake_covered(vec2(x, y)));
        textureStore(destination_image, vec2<i32>(vec2(x, y)), vec4(sum, 0u, 0u, 0u));
    }
}

@compute @workgroup_size(64)
fn scan_columns(@builtin(global_invocation_id) id: vec3<u32>) {
    let x = id.x;
    if x >= params.dimensions.x { return; }
    var sum = 0u;
    for (var y = 0u; y < params.dimensions.y; y++) {
        let point = vec2<i32>(vec2(x, y));
        sum += textureLoad(source, point, 0).x;
        textureStore(destination_image, point, vec4(sum, 0u, 0u, 0u));
    }
}

fn prefix(point: vec2<i32>) -> u32 {
    if any(point < vec2(0)) { return 0u; }
    return textureLoad(source, point, 0).x;
}

fn covered_between(first: vec2<u32>, last: vec2<u32>) -> bool {
    let lo = vec2<i32>(first) - 1;
    let hi = vec2<i32>(last);
    return prefix(hi) + prefix(lo) > prefix(vec2(lo.x, hi.y)) + prefix(vec2(hi.x, lo.y));
}

// The first and last texel of the UV tile block holding `point`: padding stays within it
fn block_first(point: vec2<u32>) -> vec2<u32> {
    return point / params.block.xy * params.block.xy;
}

fn block_last(point: vec2<u32>) -> vec2<u32> {
    return min(block_first(point) + params.block.xy, params.dimensions.xy) - 1u;
}

fn inside_radius(point: vec2<u32>, radius: u32) -> bool {
    let first = block_first(point);
    return covered_between(max(point - min(point, vec2(radius)), first), min(point + radius, block_last(point)));
}

fn nearest(point: vec2<u32>) -> vec2<u32> {
    if covered_between(point, point) { return point; }
    let size = block_last(point) + 1u;
    let origin = block_first(point);
    var low = 1u;
    var high = 1u;
    let limit = max(size.x - origin.x, size.y - origin.y);
    // Most uncovered texels are near an island. Find a tight upper bound first.
    while high < limit && !inside_radius(point, high) {
        low = high + 1u;
        high = min(high * 2u, limit);
    }
    // The first square that contains a texel gives the exact Chebyshev distance.
    while low < high {
        let mid = (low + high) / 2u;
        if inside_radius(point, mid) {
            high = mid;
        } else {
            low = mid + 1u;
        }
    }
    let first = max(point - min(point, vec2(low)), origin);
    let last = min(point + low, size - 1u);
    // Among those at the same distance, the CPU resolves ties in source row-major order.
    low = first.y;
    high = last.y;
    while low < high {
        let mid = (low + high) / 2u;
        if covered_between(first, vec2(last.x, mid)) { high = mid; }
        else { low = mid + 1u; }
    }
    let y = low;
    low = first.x;
    high = last.x;
    while low < high {
        let mid = (low + high) / 2u;
        if covered_between(vec2(first.x, y), vec2(mid, y)) { high = mid; }
        else { low = mid + 1u; }
    }
    return vec2(low, y);
}

@compute @workgroup_size(8, 8)
fn pad_curvature(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = params.dimensions.xy;
    if any(id.xy >= size) { return; }
    var deviation = 0.0;
    // A block without a covered texel has no source: it stays flat
    if covered_between(block_first(id.xy), block_last(id.xy)) {
        deviation = bitcast<f32>(textureLoad(bake, vec2<i32>(nearest(id.xy)), 0).y) - 0.5;
    }
    textureStore(destination_image, vec2<i32>(id.xy), vec4(bitcast<u32>(deviation), 0u, 0u, 0u));
}

fn quantize_box(value: f32) -> f32 {
    let scaled = value * FIX;
    let lower = floor(scaled);
    // Keep the half-unit comparison separate to avoid rounding twice at
    // values whose fixed-point representation is already a whole float.
    return (lower + select(0.0, 1.0, scaled - lower >= 0.5)) / FIX;
}

fn blur_sample(point: vec2<u32>) -> f32 {
    var value: f32;
    if params.mode.w == 2u {
        value = bitcast<f32>(textureLoad(bake, vec2<i32>(point), 0).y) - 0.5;
    } else {
        value = bitcast<f32>(textureLoad(source, vec2<i32>(point), 0).x);
    }
    if params.mode.w != 0u {
        let finite = select(0.0, clamp(value, -1e6, 1e6), abs(value) <= 3.402823466e38);
        return sign(finite) * quantize_box(abs(finite));
    }
    return value;
}

fn add_compensated(total: vec2<f32>, value: f32) -> vec2<f32> {
    let delta = value - total.y;
    let sum = total.x + delta;
    return vec2(sum, (sum - total.x) - delta);
}

// Three row passes and then three column passes, using one running sum per
// line rather than reading a whole window per output texel.
@compute @workgroup_size(64)
fn blur_curvature(@builtin(global_invocation_id) id: vec3<u32>) {
    let vertical = params.mode.z != 0u;
    let size = select(params.dimensions.xy, params.dimensions.yx, vertical);
    let line = id.x;
    if line >= size.y { return; }
    let radius = params.mode.x;
    let divisor = f32(radius * 2u + 1u);
    // Each UV tile block of the line on its own, reading nothing past its ends
    let segment = max(select(params.block.x, params.block.y, vertical), 1u);
    for (var start = 0u; start < size.x; start += segment) {
        let end = min(start + segment, size.x);
        var sum = vec2(0.0);
        for (var i = start; i < min(start + radius + 1u, end); i++) {
            let point = vec2(i, line);
            sum = add_compensated(sum, blur_sample(select(point, point.yx, vertical)));
        }
        for (var i = start; i < end; i++) {
            let point = vec2(i, line);
            let result = quantize_box(sum.x / divisor);
            textureStore(destination_image, vec2<i32>(select(point, point.yx, vertical)), vec4(bitcast<u32>(result), 0u, 0u, 0u));
            let next = i + radius + 1u;
            if next < end {
                let p = vec2(next, line);
                sum = add_compensated(sum, blur_sample(select(p, p.yx, vertical)));
            }
            if i >= start + radius {
                let p = vec2(i - radius, line);
                sum = add_compensated(sum, -blur_sample(select(p, p.yx, vertical)));
            }
        }
    }
}

@compute @workgroup_size(8, 8)
fn coarsen_curvature(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.level.zw) { return; }
    let step = params.mode.y;
    let size = params.dimensions.xy;
    let begin = id.xy * step;
    let end = min(begin + step, size);
    var sum = 0.0;
    for (var y = begin.y; y < end.y; y++) {
        for (var x = begin.x; x < end.x; x++) {
            let blurred = bitcast<f32>(textureLoad(source, vec2<i32>(vec2(x, y)), 0).x);
            sum += blurred / (edge_weights[x] * edge_weights[size.x + y]);
        }
    }
    let scaled = sum / f32((end.x - begin.x) * (end.y - begin.y)) * 65534.0;
    let stored = i32(clamp(sign(scaled) * floor(abs(scaled) + 0.5), -32768.0, 32767.0));
    textureStore(destination_image, vec2<i32>(params.level.xy + id.xy), vec4(bitcast<u32>(stored), 0u, 0u, 0u));
}
