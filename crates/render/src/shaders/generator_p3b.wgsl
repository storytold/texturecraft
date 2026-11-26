// P3b: the whole-map generator fields (gpu_generator_p3b.rs), built as compose's uv_islands.rs,
// uv_border_distance.rs and dripping_rust.rs build them. Concatenated after noise.wgsl.

struct Params {
    // Working atlas width and height, UV tile block width and height
    size: vec4<u32>,
    // The kernel's words (each kernel's comment names them)
    words: vec4<u32>,
    // The tile grid of a UV tile atlas (columns, rows, column, row; columns 0 without one)
    grid: vec4<f32>,
    // The occlusion bake's width and height, 1 when there is one, and the block side the tile
    // scales are kept by (BlockScales::block; 0 when every tile fills its block)
    bake: vec4<u32>,
    // The kernel's values (each kernel's comment names them)
    values: array<vec4<f32>, 5>,
    // Dripping Rust's taps: share of the drip's length and fade, two per vector
    taps: array<vec4<f32>, 16>,
}

// f32 bits of the mesh position and UV, then the triangle's UV island and padding
struct Corner {
    position: vec4<u32>,
    extra: vec4<u32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var surface_uv: texture_2d<u32>;
@group(0) @binding(2) var<storage, read> corners: array<Corner>;
@group(0) @binding(3) var<storage, read> islands: array<vec4<f32>>;
@group(0) @binding(4) var first: texture_2d<f32>;
@group(0) @binding(5) var second: texture_2d<f32>;
@group(0) @binding(6) var third: texture_2d<f32>;
@group(0) @binding(7) var output: texture_storage_2d<r32float, write>;
@group(0) @binding(8) var occlusion_bake: texture_2d<f32>;
// Each block's tile scale row by row (TextureSet::block_scales), and the box radius of each
// block, row by row over the working atlas's blocks, for the box passes
@group(0) @binding(9) var<storage, read> block_scales: array<f32>;
@group(0) @binding(10) var<storage, read> block_radii: array<u32>;

const NO_TRIANGLE: u32 = 0xffffffffu;
const NO_ISLAND: u32 = 0xffffffffu;
const WORKGROUP: u32 = 8u;
const F32_EPSILON: f32 = 1.1920929e-7;
const MIN_DOUBLE_AREA: f32 = 1e-12;

struct Hit {
    covered: bool,
    island: u32,
    uv: vec2<f32>,
    position: vec3<f32>,
}

fn edge(a: vec2<f32>, b: vec2<f32>, p: vec2<f32>) -> f32 {
    return (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
}

fn size() -> vec2<u32> {
    return params.size.xy;
}

fn block() -> vec2<u32> {
    return params.size.zw;
}

fn covered(pixel: vec2<u32>) -> bool {
    return textureLoad(surface_uv, vec2<i32>(pixel), 0).x != NO_TRIANGLE;
}

// The texel's surface as UvGBuffer::new and surface_at work it out: the barycentrics at the
// texel centre from the triangle's UVs, then the interpolated mesh UV and position
fn hit_at(pixel: vec2<u32>) -> Hit {
    let triangle = textureLoad(surface_uv, vec2<i32>(pixel), 0).x;
    let centre = vec2<f32>(pixel) + 0.5;
    if triangle == NO_TRIANGLE {
        return Hit(false, NO_ISLAND, centre / vec2<f32>(size()), vec3(0.0));
    }
    let a = corners[triangle * 3u];
    let b = corners[triangle * 3u + 1u];
    let c = corners[triangle * 3u + 2u];
    let extent = vec2<f32>(size());
    let uv_a = bitcast<vec2<f32>>(vec2(a.position.w, a.extra.x));
    let uv_b = bitcast<vec2<f32>>(vec2(b.position.w, b.extra.x));
    let uv_c = bitcast<vec2<f32>>(vec2(c.position.w, c.extra.x));
    let texel_a = uv_a * extent;
    let texel_b = uv_b * extent;
    let texel_c = uv_c * extent;
    let double_area = edge(texel_a, texel_b, texel_c);
    var weights = vec2(0.0);
    if abs(double_area) >= MIN_DOUBLE_AREA {
        let orientation = select(-1.0, 1.0, double_area > 0.0);
        let inverse_area = 1.0 / abs(double_area);
        weights = vec2(edge(texel_c, texel_a, centre) * orientation * inverse_area, edge(texel_a, texel_b, centre) * orientation * inverse_area);
    }
    let w = 1.0 - weights.x - weights.y;
    let position = bitcast<vec3<f32>>(a.position.xyz) * w + bitcast<vec3<f32>>(b.position.xyz) * weights.x + bitcast<vec3<f32>>(c.position.xyz) * weights.y;
    let uv = uv_a * w + uv_b * weights.x + uv_c * weights.y;
    return Hit(true, a.extra.y, uv, position);
}

// The tile scale of the block holding working texel (x, y) (BlockScales::at): 1 without scales
fn scale_at(x: u32, y: u32) -> f32 {
    if params.bake.w == 0u {
        return 1.0;
    }
    let column = x / params.bake.w;
    let columns = u32(params.grid.x);
    let index = (y / params.bake.w) * columns + column;
    if column >= columns || index >= arrayLength(&block_scales) {
        return 1.0;
    }
    return block_scales[index];
}

// The texels of the UV tile holding the texel: the block's width at its tile scale
fn tile_texels(pixel: vec2<u32>) -> f32 {
    return f32(block().x) * scale_at(pixel.x, pixel.y);
}

// texel_coords' UV: a UV tile atlas's projections see the mesh's own UVs (TextureSet::from_atlas,
// which undoes a tile's own scale)
fn mesh_uv(hit: Hit) -> vec2<f32> {
    if params.grid.x == 0.0 {
        return hit.uv;
    }
    let atlas = vec2(hit.uv.x * params.grid.x + params.grid.z, hit.uv.y * params.grid.y - params.grid.w);
    if params.bake.w == 0u {
        return atlas;
    }
    let corner = floor(atlas);
    // The UDIM tile at the corner (udim_at), and its block in the grid (TileGrid::udim)
    let udim_row = -corner.y;
    let column = corner.x - params.grid.z;
    let row = params.grid.w - udim_row;
    var scale = 1.0;
    let is_udim = corner.x >= 0.0 && corner.x <= 9.0 && udim_row >= 0.0 && udim_row <= 99.0;
    if is_udim && column >= 0.0 && column < params.grid.x && row >= 0.0 && row < params.grid.y {
        scale = block_scales[u32(row) * u32(params.grid.x) + u32(column)];
    }
    return vec2(corner.x + (atlas.x - corner.x) / scale, corner.y + (atlas.y - corner.y) / scale);
}

// BlockScales::bake_uv: the uniform bake's UV of a working atlas UV, invalid (w 0) where a tile
// smaller than its block leaves the block empty
fn bake_uv(uv: vec2<f32>) -> vec3<f32> {
    if params.bake.w == 0u {
        return vec3(uv, 1.0);
    }
    let block_side = f32(params.bake.w);
    let position = uv * vec2<f32>(size()) / block_side;
    let origin = floor(position);
    let scale = scale_at(u32(origin.x * block_side), u32(origin.y * block_side));
    let local = (position - origin) / scale;
    if any(local < vec2(0.0)) || any(local >= vec2(1.0)) {
        return vec3(0.0);
    }
    return vec3((origin + local) * block_side / vec2<f32>(size()), 1.0);
}

// Rust's f32::round: halves away from zero (WGSL's round goes to even)
fn round_away(value: f32) -> f32 {
    let whole = trunc(value);
    return select(whole, whole + sign(value), abs(value - whole) >= 0.5);
}

// Rust's f32::rem_euclid
fn rem_euclid(value: f32, divisor: f32) -> f32 {
    let remainder = value % divisor;
    return select(remainder, remainder + abs(divisor), remainder < 0.0);
}

fn in_size(pixel: vec2<u32>) -> bool {
    return all(pixel < size());
}

fn store_value(pixel: vec2<u32>, value: f32) {
    textureStore(output, vec2<i32>(pixel), vec4(value, 0.0, 0.0, 0.0));
}

fn read_first(pixel: vec2<u32>) -> f32 {
    return textureLoad(first, vec2<i32>(pixel), 0).x;
}

// UV Random Color: the island's gray from the island table (0 outside the islands)
@compute @workgroup_size(WORKGROUP, WORKGROUP)
fn random_color(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = id.xy;
    if !in_size(pixel) { return; }
    let hit = hit_at(pixel);
    var value = 0.0;
    if hit.covered && hit.island != NO_ISLAND {
        value = islands[hit.island].x;
    }
    store_value(pixel, value);
}

// The texel's line through its block, along its row or its column: the texel's place on it, the
// line's start and its length
fn block_line(pixel: vec2<u32>, along_columns: bool) -> vec3<u32> {
    let origin = (pixel / block()) * block();
    let extent = min(block(), size() - origin);
    if along_columns {
        return vec3(pixel.y - origin.y, origin.y, extent.y);
    }
    return vec3(pixel.x - origin.x, origin.x, extent.x);
}

fn along(pixel: vec2<u32>, along_columns: bool, position: u32) -> vec2<u32> {
    return select(vec2(position, pixel.y), vec2(pixel.x, position), along_columns);
}

// The squared distance along its block's row to the nearest uncovered texel, the block's edges
// counting as uncovered (one invocation per row and block, a run at a time)
@compute @workgroup_size(64)
fn distance_rows(@builtin(global_invocation_id) id: vec3<u32>) {
    let blocks = (size().x + block().x - 1u) / block().x;
    let row = id.x / blocks;
    if row >= size().y { return; }
    let left = (id.x % blocks) * block().x;
    let columns = min(block().x, size().x - left);
    var start = 0u;
    for (var column = 0u; column <= columns; column++) {
        if column < columns && covered(vec2(left + column, row)) { continue; }
        // The run [start, column) is covered: the ring texels sit at start - 1 and column
        for (var inside = start; inside < column; inside++) {
            let span = f32(min(inside + 1u - start, column - inside));
            store_value(vec2(left + inside, row), span * span);
        }
        if column < columns { store_value(vec2(left + column, row), 0.0); }
        start = column + 1u;
    }
}

// The distance to the nearest uncovered texel of the block: the least squared row distance plus
// the squared rows between, the block's top and bottom edges counting as uncovered. values[0].x is
// the squared reach the generators read: a texel farther away gets that reach instead, which every
// reader takes like any farther distance, and the search ends there
@compute @workgroup_size(WORKGROUP, WORKGROUP)
fn distance_columns(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = id.xy;
    if !in_size(pixel) { return; }
    let line = block_line(pixel, true);
    let row = line.x;
    let top = line.y;
    let rows = line.z;
    var best = min(read_first(pixel), params.values[0].x);
    for (var rows_away = 1u; f32(rows_away * rows_away) < best; rows_away++) {
        let squared = f32(rows_away * rows_away);
        if rows_away > row { best = min(best, f32((row + 1u) * (row + 1u))); } else {
            best = min(best, squared + read_first(vec2(pixel.x, top + row - rows_away)));
        }
        if row + rows_away >= rows { best = min(best, f32((rows - row) * (rows - row))); } else {
            best = min(best, squared + read_first(vec2(pixel.x, top + row + rows_away)));
        }
    }
    store_value(pixel, sqrt(best));
}

// UV Border Distance's gradient from the distances in `first`: values[0].x is the reach per
// texel of the UV tile
@compute @workgroup_size(WORKGROUP, WORKGROUP)
fn border_gradient(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = id.xy;
    if !in_size(pixel) { return; }
    let reach = params.values[0].x * tile_texels(pixel);
    var value = 0.0;
    if covered(pixel) && reach > 0.0 {
        value = clamp(1.0 - read_first(pixel) / reach, 0.0, 1.0);
    }
    store_value(pixel, value);
}

// The CPU's fixed-point scale for box blurs (eval.rs FIX): values become integers of 2^-24
const BOX_FIX: f32 = 16777216.0;

// A signed 64-bit integer as (low, high) words, two's complement: the CPU sums box windows in i64
fn wide_add(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let low = a.x + b.x;
    return vec2(low, a.y + b.y + select(0u, 1u, low < a.x));
}

fn wide_negate(a: vec2<u32>) -> vec2<u32> {
    return wide_add(~a, vec2(1u, 0u));
}

fn wide_from(value: i32) -> vec2<u32> {
    return vec2(bitcast<u32>(value), select(0u, 0xffffffffu, value < 0));
}

// `value` / `divisor` for a non-negative `value` whose quotient fits 32 bits and a divisor below
// 2^16, sixteen bits at a time
fn wide_divide(value: vec2<u32>, divisor: u32) -> u32 {
    var remainder = value.y % divisor;
    let upper = (remainder << 16u) | (value.x >> 16u);
    let upper_quotient = upper / divisor;
    remainder = upper % divisor;
    let lower = (remainder << 16u) | (value.x & 0xffffu);
    return (upper_quotient << 16u) | (lower / divisor);
}

// The CPU's window mean (box_line): (2 sum + n) divided by 2n, rounded towards negative infinity
fn box_mean(sum: vec2<u32>, taps: u32) -> i32 {
    let numerator = wide_add(wide_add(sum, sum), vec2(taps, 0u));
    let divisor = 2u * taps;
    if (numerator.y & 0x80000000u) == 0u {
        return bitcast<i32>(wide_divide(numerator, divisor));
    }
    let magnitude = wide_add(wide_negate(numerator), vec2(divisor - 1u, 0u));
    return -bitcast<i32>(wide_divide(magnitude, divisor));
}

// A value in fixed point as the CPU rounds it (f64::round, half away from zero); `value` × 2^24 is exact
fn box_fixed(value: f32) -> i32 {
    let scaled = value * BOX_FIX;
    let whole = trunc(scaled);
    let rounded = whole + select(0.0, sign(scaled), abs(scaled - whole) >= 0.5);
    return i32(rounded);
}

// The texel at `position` along line `line` of a box pass, along rows or columns
fn box_texel(along_columns: bool, line: u32, position: u32) -> vec2<u32> {
    return select(vec2(position, line), vec2(line, position), along_columns);
}

// A box pass's input texel as the CPU's fixed-point integer: the first pass converts, later passes
// read the integers the pass before stored (exact in f32 for magnitudes up to 2^24)
fn box_read(along_columns: bool, line: u32, position: u32) -> vec2<u32> {
    let value = read_first(box_texel(along_columns, line, position));
    return wide_from(select(i32(value), box_fixed(value), params.words.z != 0u));
}

// Texels one box-pass invocation writes along its line: enough work to keep the running sum, small
// enough to keep the GPU busy (BOX_CHUNK in gpu_generator_p3b_fields_gpu.rs)
const BOX_CHUNK: u32 = 64u;

// One box pass of its block's radius along the block's rows (words.y 0) or columns (1), reading
// black past the block's edges, exactly as the CPU's fixed-point blur_blocks. One invocation per
// chunk of a line within a block: it sums its first window, then keeps a running i64 sum, so a
// texel costs two reads whatever the radius, and integer sums make the chunks exact.
// words.z marks the first of the six passes (it converts to fixed point), words.w the last (it
// converts back); a block of radius 0 the CPU leaves as it is
@compute @workgroup_size(64)
fn box_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    let along_columns = params.words.y != 0u;
    let blocks = (size() + block() - 1u) / block();
    let segments = select(blocks.x, blocks.y, along_columns);
    let lines = select(size().y, size().x, along_columns);
    let block_length = select(block().x, block().y, along_columns);
    let chunks = (block_length + BOX_CHUNK - 1u) / BOX_CHUNK;
    // Neighbouring invocations take neighbouring lines, so their reads lie side by side
    if id.x >= lines * segments * chunks { return; }
    let line = id.x % lines;
    let segment = (id.x / lines) % segments;
    let start = segment * block_length;
    let length = i32(min(block_length, select(size().x, size().y, along_columns) - start));
    let first = i32((id.x / (lines * segments)) * BOX_CHUNK);
    if first >= length { return; }
    let last = min(first + i32(BOX_CHUNK), length);
    let block_index = select(vec2(segment, line / block().y), vec2(line / block().x, segment), along_columns);
    let radius = i32(block_radii[block_index.y * blocks.x + block_index.x]);
    if radius == 0 {
        for (var at = first; at < last; at++) {
            let texel = box_texel(along_columns, line, start + u32(at));
            store_value(texel, read_first(texel));
        }
        return;
    }

    let taps = u32(2 * radius + 1);
    var sum = vec2(0u);
    for (var offset = max(first - radius, 0); offset <= min(first + radius, length - 1); offset++) {
        sum = wide_add(sum, box_read(along_columns, line, start + u32(offset)));
    }
    for (var at = first; at < last; at++) {
        let mean = box_mean(sum, taps);
        let value = select(f32(mean), f32(mean) / BOX_FIX, params.words.w != 0u);
        store_value(box_texel(along_columns, line, start + u32(at)), value);
        if at + radius + 1 < length {
            sum = wide_add(sum, box_read(along_columns, line, start + u32(at + radius + 1)));
        }
        if at - radius >= 0 {
            sum = wide_add(sum, wide_negate(box_read(along_columns, line, start + u32(at - radius))));
        }
    }
}

// `first` where the texel is covered, else 0 (values[0].y 1: also clamped to 0..1)
@compute @workgroup_size(WORKGROUP, WORKGROUP)
fn keep_covered(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = id.xy;
    if !in_size(pixel) { return; }
    var value = 0.0;
    if covered(pixel) {
        value = read_first(pixel);
        if params.values[0].y != 0.0 { value = clamp(value, 0.0, 1.0); }
    }
    store_value(pixel, value);
}

// 1 on covered texels, 0 elsewhere: the weights a blur over the cover divides by
@compute @workgroup_size(WORKGROUP, WORKGROUP)
fn cover(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = id.xy;
    if !in_size(pixel) { return; }
    store_value(pixel, select(0.0, 1.0, covered(pixel)));
}

// blur_over_cover's result: the blurred sums in `second` over the blurred weights in `third`
// where the texel is covered and has weight, else the unblurred value in `first`
@compute @workgroup_size(WORKGROUP, WORKGROUP)
fn over_cover(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = id.xy;
    if !in_size(pixel) { return; }
    var value = read_first(pixel);
    let weight = textureLoad(third, vec2<i32>(pixel), 0).x;
    if covered(pixel) && weight > params.values[0].x {
        value = textureLoad(second, vec2<i32>(pixel), 0).x / weight;
    }
    store_value(pixel, value);
}

// The rust before any blur: from the occlusion, broken up by the grunge. values[0] holds the
// floor, gain, occlusion and power, values[1] the grunge's repeats, mean, spread and weight;
// words.z is the seed
@compute @workgroup_size(WORKGROUP, WORKGROUP)
fn rust_seed(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = id.xy;
    if !in_size(pixel) { return; }
    let hit = hit_at(pixel);
    if !hit.covered {
        store_value(pixel, 0.0);
        return;
    }
    var occlusion = 1.0;
    if params.bake.z != 0u {
        let uv = bake_uv((vec2<f32>(pixel) + 0.5) / vec2<f32>(size()));
        if uv.z != 0.0 {
            let at = min(vec2<u32>(uv.xy * vec2<f32>(params.bake.xy)), params.bake.xy - 1u);
            occlusion = textureLoad(occlusion_bake, vec2<i32>(at), 0).x;
        }
    }
    let shape = params.values[0];
    let base = shape.x + shape.y * pow(max(shape.z - occlusion, 0.0), shape.w);
    let grunge = params.values[1];
    let point = vec3(mesh_uv(hit) * grunge.x, 0.0);
    let n = fbm(point, 6u, params.words.z, true) * 0.7 + fbm(point * 3.7, 3u, params.words.z ^ 0x55u, false) * 0.3;
    let pattern = clamp((n - 0.45) * 4.0 + 0.5, 0.0, 1.0);
    // The pattern's own shape at contrast 1 and balance 0 (pattern.rs shape)
    let shaped = clamp((pattern - 0.5) + 0.5, 0.0, 1.0);
    store_value(pixel, base + grunge.w * ((shaped - grunge.y) / grunge.z));
}

struct DripSurface {
    island: u32,
    axis: f32,
}

// The texel's island and its place along the drip axis (words.x) across the mesh's bounds
// (values[0].x the low end, .y the extent); island NO_ISLAND outside the islands
fn drip_surface(x: i32, y: i32) -> DripSurface {
    if x < 0 || y < 0 || x >= i32(size().x) || y >= i32(size().y) {
        return DripSurface(NO_ISLAND, 0.0);
    }
    let hit = hit_at(vec2(u32(x), u32(y)));
    if !hit.covered || hit.island == NO_ISLAND {
        return DripSurface(NO_ISLAND, 0.0);
    }
    return DripSurface(hit.island, (hit.position[params.words.x] - params.values[0].x) / params.values[0].y);
}

// One side of the finite difference within an island: (present, value)
fn axis_at(x: i32, y: i32, island: u32) -> vec2<f32> {
    let surface = drip_surface(x, y);
    return vec2(select(0.0, 1.0, surface.island == island), surface.axis);
}

fn slope(before: vec2<f32>, here: f32, after: vec2<f32>) -> f32 {
    if before.x != 0.0 && after.x != 0.0 { return (after.y - before.y) * 0.5; }
    if after.x != 0.0 { return after.y - here; }
    if before.x != 0.0 { return here - before.y; }
    return 0.0;
}

fn tap(index: u32) -> vec2<f32> {
    let pair = params.taps[index / 2u];
    return select(pair.xy, pair.zw, index % 2u == 1u);
}

// Each texel takes the most rust found uphill along the drip axis, fading with how far up it
// lies (dripping_rust.rs drips): values[0].z is the reach per texel of the UV tile times the 2K
// UV tile's texels (values[0].w), words.y the samples
@compute @workgroup_size(WORKGROUP, WORKGROUP)
fn drips(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = id.xy;
    if !in_size(pixel) { return; }
    let here = read_first(pixel);
    let x = i32(pixel.x);
    let y = i32(pixel.y);
    let surface = drip_surface(x, y);
    let island = surface.island;
    if island == NO_ISLAND {
        store_value(pixel, here);
        return;
    }
    let axis_here = surface.axis;
    let dx = slope(axis_at(x - 1, y, island), axis_here, axis_at(x + 1, y, island));
    let dy = slope(axis_at(x, y - 1, island), axis_here, axis_at(x, y + 1, island));
    let line_length = sqrt(dx * dx + dy * dy);
    // The CPU returns here for a non-finite length or one at most f32::EPSILON
    if !(line_length > F32_EPSILON) || line_length > 3.4e38 {
        store_value(pixel, here);
        return;
    }
    let reach = params.values[0].z * tile_texels(pixel) / params.values[0].w;
    var most = here;
    var current = vec2(x, y);
    for (var index = 0u; index < params.words.y; index++) {
        let taken = tap(index);
        let span = reach * taken.x;
        let tap_at = vec2(i32(round_away(f32(x) + span * dx / line_length)), i32(round_away(f32(y) + span * dy / line_length)));
        // A sparse tap must still have continuous coverage along the ray
        var broken = false;
        loop {
            if all(current == tap_at) { break; }
            current = next_ray_texel(vec2(x, y), current, tap_at, vec2(dx, dy));
            if axis_at(current.x, current.y, island).x == 0.0 {
                broken = true;
                break;
            }
        }
        if broken { break; }
        // A tap beyond a ridge contributes only if it is higher than the receiving texel
        let other = axis_at(tap_at.x, tap_at.y, island);
        if other.x != 0.0 && other.y >= axis_here {
            most = max(most, read_first(vec2<u32>(tap_at)) * taken.y);
        }
    }
    store_value(pixel, most);
}

fn next_ray_texel(origin: vec2<i32>, at: vec2<i32>, goal: vec2<i32>, direction: vec2<f32>) -> vec2<i32> {
    let stride = clamp(goal - at, vec2(-1), vec2(1));
    if stride.x == 0 || stride.y == 0 {
        return at + stride;
    }
    // Compare crossing distances without dividing by either component
    let crossing_x = (f32(abs(at.x - origin.x)) + 0.5) * abs(direction.y);
    let crossing_y = (f32(abs(at.y - origin.y)) + 0.5) * abs(direction.x);
    return at + stride * vec2(select(0, 1, crossing_x <= crossing_y), select(0, 1, crossing_y <= crossing_x));
}

// The light or dark checker square's colour at `at` in texels of a 2K UV tile with the grid laid
// over it (uv_islands.rs checker_color): values[1] holds the square, the minor line period, the
// line's half width and the grid's opacity; values[3] the dark shade and the two line shades
fn checker_color(light: vec3<f32>, at: vec2<f32>, footprint: f32) -> vec3<f32> {
    let grid = params.values[1];
    let square = grid.x;
    let minor = grid.y;
    // Include the texel footprint so thin grid lines survive coarse sampling
    let reach = grid.z + footprint / 2.0;
    let opacity = grid.w;
    let shades = params.values[3];
    let is_light = rem_euclid(floor(at.x / square) + floor(at.y / square), 2.0) >= 1.0;
    let base = select(light * shades.x, light, is_light);
    var lines = array<bool, 4>(
        on_line(at.y, minor, reach) && !on_line(at.y, square, reach),
        on_line(at.x, minor, reach) && !on_line(at.x, square, reach),
        on_line(at.y, square, reach),
        on_line(at.x, square, reach),
    );
    var shade = 1.0;
    for (var index = 0u; index < 4u; index++) {
        if lines[index] {
            let line = select(shades.z, shades.y, index < 2u);
            shade = shade + (line - shade) * opacity;
        }
    }
    return base * shade;
}

fn on_line(position: f32, period: f32, reach: f32) -> bool {
    return abs(position - round_away(position / period) * period) < reach;
}

// UV Checker: the gray of its colour (uv_islands.rs checker_field). values[0] holds the light
// colour and 1 for Random Colors, values[2] the border colour and the border width in 2K
// texels, values[4].x the 2K UV tile's texels; `first` holds the distances and words.x is 1
// with a border
@compute @workgroup_size(WORKGROUP, WORKGROUP)
fn checker(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = id.xy;
    if !in_size(pixel) { return; }
    let hit = hit_at(pixel);
    if !hit.covered {
        store_value(pixel, 0.0);
        return;
    }
    let texels_2k = params.values[4].x / tile_texels(pixel);
    let footprint = max(texels_2k - 1.0, 0.0);
    // Measure from the texel footprint so coarse texels still show the border they cover
    let on_border = params.words.x != 0u && read_first(pixel) * texels_2k - footprint <= params.values[2].w;
    var color = params.values[2].xyz;
    if !on_border {
        var light = params.values[0].xyz;
        if params.values[0].w != 0.0 && hit.island != NO_ISLAND {
            light = islands[hit.island].yzw;
        }
        let uv = mesh_uv(hit);
        let reference = params.values[4].x;
        color = checker_color(light, vec2(rem_euclid(uv.x, 1.0) * reference, rem_euclid(uv.y, 1.0) * reference), footprint);
    }
    store_value(pixel, (color.x + color.y + color.z) / 3.0);
}
