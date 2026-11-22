// The host supplies the bicubic controls, anchor image coordinates and the inverse lookup seeds
// used by the reference projection. Refine a seed at the actual surface texel on the GPU.
struct WarpGrid {
    cells: vec2u,
    controls: u32,
    stride: u32,
    image: u32,
}

struct WarpPatch {
    cell: vec2u,
    within: vec2f,
}

struct WarpSample {
    point: vec3f,
    ds: vec2f,
    dt: vec2f,
}

fn warp_patch(grid: WarpGrid, st: vec2f) -> WarpPatch {
    let scaled = clamp(st, vec2f(0.0), vec2f(1.0)) * vec2f(grid.cells);
    let cell = min(vec2u(scaled), grid.cells - 1u);
    return WarpPatch(cell, scaled - vec2f(cell));
}

fn warp_bezier(t: f32) -> vec4f {
    let d = 1.0 - t;
    return vec4f(d * d * d, 3.0 * d * d * t, 3.0 * d * t * t, t * t * t);
}

fn warp_derivative(t: f32) -> vec4f {
    let d = 1.0 - t;
    return vec4f(-3.0 * d * d, 3.0 * d * d - 6.0 * d * t, 6.0 * d * t - 3.0 * t * t, 3.0 * t * t);
}

fn warp_sample(grid: WarpGrid, st: vec2f) -> WarpSample {
    let span = warp_patch(grid, st);
    let bx = warp_bezier(span.within.x);
    let by = warp_bezier(span.within.y);
    let dx = warp_derivative(span.within.x);
    let dy = warp_derivative(span.within.y);
    var sample = WarpSample(vec3f(0.0), vec2f(0.0), vec2f(0.0));
    for (var y = 0u; y < 4u; y++) {
        for (var x = 0u; x < 4u; x++) {
            let cp = 3u * span.cell + vec2u(x, y);
            let i = grid.controls + 3u * (cp.y * grid.stride + cp.x);
            let point = bitcast<vec3f>(vec3u(srgb[i], srgb[i + 1u], srgb[i + 2u]));
            sample.point += point * (bx[x] * by[y]);
            sample.ds += point.xy * (dx[x] * by[y]);
            sample.dt += point.xy * (bx[x] * dy[y]);
        }
    }
    sample.ds *= f32(grid.cells.x);
    sample.dt *= f32(grid.cells.y);
    return sample;
}

fn warp_uv_at(grid: WarpGrid, pos: vec2u) -> vec2f {
    let i = grid.image + 2u * (pos.y * (grid.cells.x + 1u) + pos.x);
    return bitcast<vec2f>(vec2u(srgb[i], srgb[i + 1u]));
}

fn warp_refine(grid: WarpGrid, position: vec2f, seed: vec2f) -> vec4f {
    var st = seed;
    for (var it = 0u; it < 10u; it++) {
        let sample = warp_sample(grid, st);
        let delta = position - sample.point.xy;
        if dot(delta, delta) < 1e-14 { break; }
        let det = sample.ds.x * sample.dt.y - sample.dt.x * sample.ds.y;
        if abs(det) <= 1e-14 { break; }
        let step = vec2f(sample.dt.y * delta.x - sample.dt.x * delta.y, sample.ds.x * delta.y - sample.ds.y * delta.x) / det;
        let next = clamp(st + step, vec2f(0.0), vec2f(1.0));
        if all(next == st) { break; }
        st = next;
    }
    let sample = warp_sample(grid, st);
    let miss = position - sample.point.xy;
    if !(dot(miss, miss) <= 1e-12) { return vec4f(0.0); }
    let span = warp_patch(grid, st);
    let a = warp_uv_at(grid, span.cell);
    let b = warp_uv_at(grid, span.cell + vec2u(1u, 0u));
    let c = warp_uv_at(grid, span.cell + vec2u(0u, 1u));
    let d = warp_uv_at(grid, span.cell + 1u);
    let uv = mix(mix(a, b, span.within.x), mix(c, d, span.within.x), span.within.y);
    return vec4f(uv, sample.point.z, 1.0);
}

fn warp_inverse(data: u32, position: vec2f) -> vec4f {
    let rows = srgb[data];
    let columns = srgb[data + 1u];
    let extent = vec2u(srgb[data + 2u], srgb[data + 3u]);
    let origin = bitcast<vec2f>(vec2u(srgb[data + 4u], srgb[data + 5u]));
    let cell_size = bitcast<f32>(srgb[data + 6u]);
    let f = (position - origin) / cell_size;
    if any(extent == vec2u(0u)) || !all(f >= vec2f(-1.0)) || !all(f <= vec2f(extent) + 1.0) {
        return vec4f(0.0);
    }
    let controls = data + 7u;
    let stride = 3u * (columns - 1u) + 1u;
    let image = controls + 3u * stride * (3u * (rows - 1u) + 1u);
    let seeds = image + 2u * rows * columns;
    let grid = WarpGrid(vec2u(columns - 1u, rows - 1u), controls, stride, image);
    let center = vec2i(floor(f));
    let offsets = array(
        vec2i(0, 0), vec2i(1, 0), vec2i(-1, 0), vec2i(0, 1), vec2i(0, -1),
        vec2i(1, 1), vec2i(-1, 1), vec2i(1, -1), vec2i(-1, -1),
    );
    for (var i = 0u; i < 9u; i++) {
        let cell = center + offsets[i];
        if any(cell < vec2i(0)) || any(cell >= vec2i(extent)) { continue; }
        let seed = seeds + 2u * (u32(cell.y) * extent.x + u32(cell.x));
        let st = bitcast<vec2f>(vec2u(srgb[seed], srgb[seed + 1u]));
        if st.x < 0.0 { continue; }
        let hit = warp_refine(grid, position, st);
        if hit.w != 0.0 { return hit; }
    }
    return vec4f(0.0);
}
