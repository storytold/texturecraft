// Tile Generator parameters and placements follow gpu_tile_pattern.rs in the packed
// compositor lookup buffer. Shapes, intersections and overlap blending run per texel.
const TILE_NO_IMAGE = 0xffffffffu;
const TILE_EPSILON = 0.000001;
const TILE_PLACEMENT_WORDS = 11u;

struct TileShapeSettings {
    kind: u32,
    samples: vec2<f32>,
    specific: f32,
    tiling: u32,
    size: vec2<f32>,
    rotation: vec2<f32>,
    diagonal: bool,
}

fn tile_float(word: u32) -> f32 {
    return bitcast<f32>(srgb[word]);
}

fn tile_pair(word: u32) -> vec2<f32> {
    return vec2(tile_float(word), tile_float(word + 1u));
}

fn tile_mod(n: i32, d: i32) -> i32 {
    return wrap_coordinate(n, d);
}

fn tile_image_texel(origin: vec2<i32>, layer: i32, size: vec2<i32>, pixel: vec2<i32>) -> f32 {
    let wrapped = vec2(tile_mod(pixel.x, size.x), tile_mod(pixel.y, size.y));
    return bitcast<f32>(textureLoad(tile_images, origin + wrapped, layer, 0).x);
}

fn tile_image_at(image: u32, uv: vec2<f32>) -> f32 {
    let origin = vec2<i32>(i32(srgb[image]), i32(srgb[image + 1u]));
    let layer = i32(srgb[image + 2u]);
    let size = vec2<i32>(i32(srgb[image + 3u]), i32(srgb[image + 4u]));
    let grid = uv * vec2<f32>(size) - 0.5;
    let base = vec2<i32>(floor(grid));
    let f = fract(grid);
    let a = tile_image_texel(origin, layer, size, base);
    let b = tile_image_texel(origin, layer, size, base + vec2(1, 0));
    let c = tile_image_texel(origin, layer, size, base + vec2(0, 1));
    let d = tile_image_texel(origin, layer, size, base + vec2(1, 1));
    let top = a + (b - a) * f.x;
    let bottom = c + (d - c) * f.x;
    return clamp(top + (bottom - top) * f.y, 0.0, 1.0);
}

fn tile_bevel(value: f32) -> f32 {
    let t = clamp(value, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn tile_fourth(value: f32) -> f32 {
    let squared = value * value;
    return squared * squared;
}

fn tile_bell(r: f32) -> f32 {
    return tile_fourth(clamp(1.0 - r * r, 0.0, 1.0));
}

fn tile_analytic_shape(kind: u32, p: vec2<f32>, specific: f32) -> f32 {
    if any(abs(p) > vec2(1.0)) { return 0.0; }
    let r = length(p);
    switch kind {
        case 1u: { return 1.0; }
        case 2u: { return select(0.0, 1.0, r <= 1.0); }
        case 3u: { return clamp(1.0 - r * r, 0.0, 1.0); }
        case 4u: { return tile_bell(r); }
        case 5u: { return exp2(-16.0 * r * r); }
        case 6u: { return tile_fourth(clamp(1.0 - r, 0.0, 1.0)); }
        case 7u: { return clamp(1.0 - max(abs(p.x), abs(p.y)), 0.0, 1.0); }
        case 8u: {
            if specific <= TILE_EPSILON { return 1.0; }
            return tile_bevel((1.0 - abs(p.x)) / specific) * tile_bevel((1.0 - abs(p.y)) / specific);
        }
        case 9u: { return (p.x + 1.0) * 0.5 * (1.0 - specific); }
        case 10u: {
            let cycles = 1.5 + 6.0 * specific;
            return clamp(1.0 - r * r, 0.0, 1.0) * (0.5 + 0.5 * cos(6.283185307179586 * cycles * p.x));
        }
        case 11u: {
            let width = 1.24 * (1.0 - specific);
            var cut = 1.0;
            if p.x > 0.0 && width > TILE_EPSILON { cut = tile_bell(p.x / width); }
            if p.x > 0.0 && width <= TILE_EPSILON { cut = 0.0; }
            return tile_bell(r) * cut;
        }
        case 12u: {
            let ridge = 1.0 - abs(p.x);
            return tile_bell(r) * ridge * ridge;
        }
        case 13u: {
            if specific >= 1.0 { return 0.0; }
            let shifted = (p.x + 1.0) / (1.0 - specific) - 1.0;
            let ring = clamp(1.0 - r * r, 0.0, 1.0);
            return 6.75 * r * r * ring * ring * tile_bevel((1.0 - shifted) * 0.5);
        }
        case 14u: {
            let radius = 0.2225;
            let half_length = specific * (1.0 - radius);
            let distance = length(vec2(max(abs(p.x) - half_length, 0.0), p.y)) / radius;
            return clamp(1.0 - distance * distance, 0.0, 1.0);
        }
        case 15u: { return clamp(1.0 - r, 0.0, 1.0); }
        default: { return 0.0; }
    }
}

fn tile_unrotate(p: vec2<f32>, sin_cos: vec2<f32>) -> vec2<f32> {
    return vec2(sin_cos.y * p.x + sin_cos.x * p.y, -sin_cos.x * p.x + sin_cos.y * p.y);
}

fn tile_shape_sample(shape: TileShapeSettings, point: vec2<f32>, image: u32) -> f32 {
    if any(abs(point) > vec2(1.0)) || any(shape.size <= vec2(TILE_EPSILON)) { return 0.0; }
    var local = point;
    if shape.tiling > 1u {
        let shown = f32((shape.tiling + 1u) / 2u - 1u);
        let cell = (local + 1.0) * 0.5 * f32(shape.tiling);
        if any(floor(cell) != vec2(shown)) { return 0.0; }
        local = (cell - shown) * 2.0 - 1.0;
    }
    local = tile_unrotate(local, shape.rotation) / shape.size;
    if shape.diagonal {
        local = tile_unrotate(local, vec2(0.7071067811865476)) * 1.4142135623730951;
    }
    if shape.kind == 0u {
        if image == TILE_NO_IMAGE || any(abs(local) > vec2(1.0)) { return 0.0; }
        return tile_image_at(image, (local + 1.0) * 0.5);
    }
    let grid = (local + 1.0) * 0.5 * shape.samples - 0.5;
    let base = floor(grid);
    let fraction = grid - base;
    let a = tile_analytic_shape(shape.kind, (base + vec2(0.5, 0.5)) / shape.samples * 2.0 - 1.0, shape.specific);
    let b = tile_analytic_shape(shape.kind, (base + vec2(1.5, 0.5)) / shape.samples * 2.0 - 1.0, shape.specific);
    let c = tile_analytic_shape(shape.kind, (base + vec2(0.5, 1.5)) / shape.samples * 2.0 - 1.0, shape.specific);
    let d = tile_analytic_shape(shape.kind, (base + vec2(1.5, 1.5)) / shape.samples * 2.0 - 1.0, shape.specific);
    let top = a + (b - a) * fraction.x;
    let bottom = c + (d - c) * fraction.x;
    return top + (bottom - top) * fraction.y;
}

fn tile_placement(shape: TileShapeSettings, offset: u32, point: vec2<f32>) -> f32 {
    let size = tile_pair(offset + 2u);
    if any(size <= vec2(TILE_EPSILON)) { return 0.0; }
    let distance = point - tile_pair(offset);
    // The CPU uses Rust's round-to-nearest with ties away from zero.
    let delta = distance - sign(distance) * floor(abs(distance) + 0.5);
    let radius = tile_pair(offset + 4u);
    let first = vec2<i32>(max(ceil(-radius - delta), vec2(-3.0)));
    let last = vec2<i32>(min(floor(radius - delta), vec2(3.0)));
    let sin_cos = tile_pair(offset + 6u);
    let flags = srgb[offset + 8u];
    let image = srgb[offset + 9u];
    var result = 0.0;
    for (var y = first.y; y <= last.y; y++) {
        for (var x = first.x; x <= last.x; x++) {
            let shifted = delta + vec2(f32(x), f32(y));
            if any(abs(shifted) > radius) { continue; }
            var local = tile_unrotate(shifted, sin_cos) * 2.0 / size;
            switch flags & 3u {
                case 1u: { local = vec2(local.y, -local.x); }
                case 2u: { local = -local; }
                case 3u: { local = vec2(-local.y, local.x); }
                default: {}
            }
            if (flags & 4u) != 0u { local.x = -local.x; }
            if (flags & 8u) != 0u { local.y = -local.y; }
            result = max(result, tile_shape_sample(shape, local, image));
        }
    }
    return result * tile_float(offset + 10u);
}

// First cell and number of cells to visit, counting each wrapped tile once.
fn tile_near(position: f32, reach: f32, count: i32) -> vec2<i32> {
    let first = floor(position - reach - 0.5);
    let last = ceil(position + reach - 0.5);
    return vec2(i32(first), i32(clamp(last - first + 1.0, 1.0, f32(count))));
}

fn tile_generator(offset: u32, unwrapped: vec2<f32>) -> f32 {
    let counts = vec2(i32(srgb[offset]), i32(srgb[offset + 1u]));
    let reach = tile_pair(offset + 2u);
    let vertical = srgb[offset + 4u] != 0u;
    let shifts = srgb[offset + 5u];
    let placements = srgb[offset + 6u];
    let shape = TileShapeSettings(
        srgb[offset + 7u], vec2(f32(srgb[offset + 8u]), f32(srgb[offset + 9u])),
        tile_float(offset + 10u), srgb[offset + 11u], tile_pair(offset + 12u),
        tile_pair(offset + 14u), srgb[offset + 16u] != 0u,
    );
    let point = fract(unwrapped);
    var background = tile_float(offset + 17u);
    let background_image = srgb[offset + 18u];
    if background_image != TILE_NO_IMAGE { background = tile_image_at(background_image, point); }
    let color = tile_float(offset + 19u);
    let blending = srgb[offset + 20u];
    let opacity = tile_float(offset + 21u);
    let middle = tile_float(offset + 22u);
    let half = tile_float(offset + 23u);
    let cell = (point - tile_pair(offset + 24u)) * vec2<f32>(counts);
    let ordered_cell = select(cell.yx, cell, vertical);
    let ordered_reach = select(reach.yx, reach, vertical);
    let ordered_counts = select(counts.yx, counts, vertical);
    let lines = tile_near(ordered_cell.x, ordered_reach.x, ordered_counts.x);
    var sum = 0.0;
    var colored = 0.0;
    for (var line_index = 0; line_index < lines.y; line_index++) {
        let line = tile_mod(lines.x + line_index, ordered_counts.x);
        let shift = tile_float(shifts + u32(line));
        let row = tile_near(ordered_cell.y - shift, ordered_reach.y, ordered_counts.y);
        for (var other_index = 0; other_index < row.y; other_index++) {
            let other = tile_mod(row.x + other_index, ordered_counts.y);
            let index = select(other * counts.y + line, line * counts.y + other, vertical);
            let value = tile_placement(shape, placements + u32(index) * TILE_PLACEMENT_WORDS, point);
            if blending == 0u {
                sum = max(sum, value);
                colored = max(colored, value * color);
            } else {
                sum += value;
                colored += value * select(color, 2.0 * color - 1.0, blending == 2u);
            }
        }
    }
    let alpha = clamp(sum, 0.0, 1.0) * opacity;
    let value = clamp(background * (1.0 - alpha) + clamp(colored, 0.0, 1.0) * opacity, 0.0, 1.0);
    if half <= TILE_EPSILON { return select(0.0, 1.0, value >= middle); }
    return clamp((value - (middle - half)) / (2.0 * half), 0.0, 1.0);
}
