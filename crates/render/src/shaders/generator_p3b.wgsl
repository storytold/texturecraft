// P3b: the whole-map generator fields (gpu_generator_p3b.rs), built as compose's uv_islands.rs,
// uv_border_distance.rs and dripping_rust.rs build them. Concatenated after noise.wgsl.

struct Params {
    // Working atlas width and height, UV tile block width and height
    size: vec4<u32>,
    // The kernel's words: box radius or axis, then drip samples
    words: vec4<u32>,
    // The tile grid of a UV tile atlas (columns, rows, column, row; columns 0 without one)
    grid: vec4<f32>,
    // The occlusion bake's width and height, and 1 when there is one
    bake: vec4<u32>,
    // The kernel's values (gpu_generator_p3b.rs names each)
    values: array<vec4<f32>, 5>,
    // Dripping Rust's taps: share of the drip's length and fade, two per vector
    taps: array<vec4<f32>, 16>,
}

struct Corner {
    // Mesh position and UV x
    position: vec4<f32>,
    // UV y, the triangle's UV island (on its first corner) and padding
    extra: vec4<f32>,
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
    let uv_a = vec2(a.position.w, a.extra.x);
    let uv_b = vec2(b.position.w, b.extra.x);
    let uv_c = vec2(c.position.w, c.extra.x);
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
    let position = a.position.xyz * w + b.position.xyz * weights.x + c.position.xyz * weights.y;
    let uv = uv_a * w + uv_b * weights.x + uv_c * weights.y;
    return Hit(true, bitcast<u32>(a.extra.y), uv, position);
}

// texel_coords' UV: a UV tile atlas's projections see the mesh's own UVs (TileGrid::from_atlas)
fn mesh_uv(hit: Hit) -> vec2<f32> {
    if params.grid.x == 0.0 {
        return hit.uv;
    }
    return vec2(hit.uv.x * params.grid.x + params.grid.z, hit.uv.y * params.grid.y - params.grid.w);
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

// UV Random Color: the island's gray (0 outside the islands); values[0].x is unused
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

// One line of a block (words.x: 0 rows, 1 columns): the texel's place on it, its start and its length
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
// the squared rows between, the block's top and bottom edges counting as uncovered
@compute @workgroup_size(WORKGROUP, WORKGROUP)
fn distance_columns(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = id.xy;
    if !in_size(pixel) { return; }
    let line = block_line(pixel, true);
    let row = line.x;
    let top = line.y;
    let rows = line.z;
    var best = read_first(pixel);
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

// UV Border Distance's gradient from the distances in `first`: values[0].x is the reach in texels
@compute @workgroup_size(WORKGROUP, WORKGROUP)
fn border_gradient(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = id.xy;
    if !in_size(pixel) { return; }
    let reach = params.values[0].x;
    var value = 0.0;
    if covered(pixel) && reach > 0.0 {
        value = clamp(1.0 - read_first(pixel) / reach, 0.0, 1.0);
    }
    store_value(pixel, value);
}

// One box pass of radius words.x along the block's rows (words.y 0) or columns (1), reading
// black past the block's edges (blur_blocks, box_line)
@compute @workgroup_size(WORKGROUP, WORKGROUP)
fn box_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = id.xy;
    if !in_size(pixel) { return; }
    let radius = i32(params.words.x);
    let along_columns = params.words.y != 0u;
    let line = block_line(pixel, along_columns);
    let at = i32(line.x);
    let start = line.y;
    let line_length = i32(line.z);
    var sum = 0.0;
    for (var offset = max(at - radius, 0); offset <= min(at + radius, line_length - 1); offset++) {
        sum += read_first(along(pixel, along_columns, start + u32(offset)));
    }
    store_value(pixel, sum / f32(2 * radius + 1));
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
        let uv = (vec2<f32>(pixel) + 0.5) / vec2<f32>(size());
        let at = min(vec2<u32>(uv * vec2<f32>(params.bake.xy)), params.bake.xy - 1u);
        occlusion = textureLoad(occlusion_bake, vec2<i32>(at), 0).x;
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

// The texel's island and its place along the drip axis (words.x) across the mesh's bounds
// (values[0].x the low end, .y the extent); island NO_ISLAND outside the islands
fn drip_surface(x: i32, y: i32) -> vec2<f32> {
    if x < 0 || y < 0 || x >= i32(size().x) || y >= i32(size().y) {
        return vec2(bitcast<f32>(NO_ISLAND), 0.0);
    }
    let hit = hit_at(vec2(u32(x), u32(y)));
    if !hit.covered || hit.island == NO_ISLAND {
        return vec2(bitcast<f32>(NO_ISLAND), 0.0);
    }
    return vec2(bitcast<f32>(hit.island), (hit.position[params.words.x] - params.values[0].x) / params.values[0].y);
}

// One side of the finite difference within an island: (present, value)
fn axis_at(x: i32, y: i32, island: u32) -> vec2<f32> {
    let surface = drip_surface(x, y);
    return vec2(select(0.0, 1.0, bitcast<u32>(surface.x) == island), surface.y);
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
// lies (dripping_rust.rs drips): values[0].z is the reach in texels, words.y the samples
@compute @workgroup_size(WORKGROUP, WORKGROUP)
fn drips(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = id.xy;
    if !in_size(pixel) { return; }
    let here = read_first(pixel);
    let x = i32(pixel.x);
    let y = i32(pixel.y);
    let surface = drip_surface(x, y);
    let island = bitcast<u32>(surface.x);
    if island == NO_ISLAND {
        store_value(pixel, here);
        return;
    }
    let axis_here = surface.y;
    let dx = slope(axis_at(x - 1, y, island), axis_here, axis_at(x + 1, y, island));
    let dy = slope(axis_at(x, y - 1, island), axis_here, axis_at(x, y + 1, island));
    let line_length = sqrt(dx * dx + dy * dy);
    // The CPU returns here for a non-finite length or one at most f32::EPSILON
    if !(line_length > F32_EPSILON) || line_length > 3.4e38 {
        store_value(pixel, here);
        return;
    }
    let reach = params.values[0].z;
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
// line reach and the grid's opacity; values[3] the dark shade and the two line shades
fn checker_color(light: vec3<f32>, at: vec2<f32>) -> vec3<f32> {
    let grid = params.values[1];
    let square = grid.x;
    let minor = grid.y;
    let reach = grid.z;
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
// texels, values[3].w the texels of a 2K UV tile one working texel spans, values[4].x the 2K UV
// tile's texels; `first` holds the distances and words.x is 1 with a border
@compute @workgroup_size(WORKGROUP, WORKGROUP)
fn checker(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = id.xy;
    if !in_size(pixel) { return; }
    let hit = hit_at(pixel);
    if !hit.covered {
        store_value(pixel, 0.0);
        return;
    }
    let texels_2k = params.values[3].w;
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
        color = checker_color(light, vec2(rem_euclid(uv.x, 1.0) * reference, rem_euclid(uv.y, 1.0) * reference));
    }
    store_value(pixel, (color.x + color.y + color.z) / 3.0);
}
