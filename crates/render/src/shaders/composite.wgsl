// UV fill compositor. Noise and color conversions follow texel/src/{noise,tile}.rs.
struct Operation {
    ids: vec4<u32>,
    low: vec4<f32>,
    high: vec4<f32>,
    projection: vec4<f32>,
    uv: vec4<f32>,
    balance: vec4<f32>,
    effects: vec4<u32>,
    procedural: vec4<u32>,
    // Layer: first content field slot, content and mask field flags. Marker:
    // relative content slot or absolute mask field slot.
    spatial: vec4<u32>,
    // Geometry lookup offset, Height operation plus one, contrast and offset.
    constraints: vec4<u32>,
}

struct Params {
    // Width, row count, top row, reserved texels per map.
    band: vec4<u32>,
    // Source width, source height, display sampling step, output mode (roughness, glossiness, mask).
    source: vec4<u32>,
    // Layer count, scratch row pitch, DisplayKind output bits, left column.
    program: vec4<u32>,
    coverage: vec4<u32>,
    // Origin and size in tiles, or a sentinel in x for per-image windows.
    paint_window: vec4<u32>,
    // Source origin and size of the neighborhood fields.
    filter_region: vec4<u32>,
    // Field word offset, destination slot, layer index, first excluded effect.
    // The high bit of the last word selects the mask stack.
    filter_stage: vec4<u32>,
    filter_chunk: vec4<u32>,
    // Original ID bake dimensions, both zero when absent.
    id_bake: vec4<u32>,
    // Channel, start of anchor owner's draws, content cut, mask preview draw.
    anchor: vec4<u32>,
    anchor_default: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> layers: array<Operation>;
@group(0) @binding(2) var<storage, read> srgb: array<u32>;
@group(0) @binding(3) var<storage, read_write> output: array<u32>;
@group(0) @binding(4) var<storage, read> paint: array<u32>;
@group(0) @binding(5) var<storage, read> uv_coverage: array<u32>;
@group(0) @binding(6) var surface_uv: texture_2d<u32>;
struct SurfaceVertex {
    position: vec4<f32>,
    normal: vec4<f32>,
}
@group(0) @binding(7) var<storage, read> surface_vertices: array<SurfaceVertex>;
@group(0) @binding(8) var bake_maps: texture_2d<u32>;
@group(0) @binding(9) var brush_alpha: texture_2d<u32>;
@group(0) @binding(11) var brush_stencil_image: texture_2d<f32>;
@group(0) @binding(12) var<uniform> brush_stencil: StencilProjection;

const NO_PAINT_IMAGE = 0xffffffffu;
const PROJECTED_BRUSH: u32 = 4u;
const GEOMETRY_MASKS: u32 = 8u;
const SPATIAL_FILTER_OPCODE: u32 = 36u;
const COLOR_SELECTION_OPCODE: u32 = 38u;

struct Surface {
    position: vec3<f32>,
    normal: vec3<f32>,
    valid: bool,
}

fn edge(a: vec2<f32>, b: vec2<f32>, p: vec2<f32>) -> f32 {
    let ab = b - a;
    let ap = p - a;
    return ab.x * ap.y - ab.y * ap.x;
}

fn surface_at(pixel: vec2<u32>) -> Surface {
    if (params.coverage.w & 1u) == 0u || any(pixel >= params.source.xy) {
        return Surface(vec3(0.0), vec3(0.0, 1.0, 0.0), false);
    }
    let hit = textureLoad(surface_uv, vec2<i32>(pixel), 0).xy;
    if hit.x == 0xffffffffu {
        return Surface(vec3(0.0), vec3(0.0, 1.0, 0.0), false);
    }
    let a = surface_vertices[hit.x * 3u];
    let b = surface_vertices[hit.x * 3u + 1u];
    let c = surface_vertices[hit.x * 3u + 2u];
    // Reconstruct the working position from the original UVs. Packed barycentrics can
    // cross a hard procedural cell boundary at high frequencies.
    let size = vec2<f32>(params.source.xy);
    let uv_a = vec2(a.position.w, a.normal.w) * size;
    let uv_b = vec2(b.position.w, b.normal.w) * size;
    let uv_c = vec2(c.position.w, c.normal.w) * size;
    let center = vec2<f32>(pixel) + 0.5;
    let area = edge(uv_a, uv_b, uv_c);
    var weight = unpack2x16unorm(hit.y);
    if abs(area) >= 1e-12 {
        weight = vec2(edge(uv_c, uv_a, center), edge(uv_a, uv_b, center)) * (1.0 / area);
    }
    let w = 1.0 - weight.x - weight.y;
    let position = a.position.xyz * w + b.position.xyz * weight.x + c.position.xyz * weight.y;
    let normal = a.normal.xyz * w + b.normal.xyz * weight.x + c.normal.xyz * weight.y;
    return Surface(position, select(normal / max(length(normal), 1e-6), vec3(0.0, 1.0, 0.0), dot(normal, normal) <= 1e-12), true);
}

fn bake_pixel_at(pixel: vec2<u32>) -> vec2<u32> {
    let size = textureDimensions(bake_maps);
    let uv = (vec2<f32>(pixel) + 0.5) / vec2<f32>(params.source.xy);
    return min(vec2<u32>(uv * vec2<f32>(size)), size - 1u);
}

fn baked_at(pixel: vec2<u32>) -> vec4<u32> {
    return textureLoad(bake_maps, vec2<i32>(bake_pixel_at(pixel)), 0);
}

fn unpack_baked_normal(packed: u32) -> vec3<f32> {
    let components = vec3<f32>(vec3(packed, packed >> 10u, packed >> 20u) & vec3(1023u)) / 1022.0;
    return vec3(components.xy, components.z * components.z);
}

// The nearest squared distance can be clipped to one when no feature ID is needed.
fn cellular(p: vec3<f32>, seed: u32, details: bool) -> vec3<f32> {
    let cell = vec3<i32>(floor(p));
    let local = fract(p);
    var nearest = select(1.0, 4.0, details);
    var second = 1.0;
    var feature_id = 0u;
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let shift = vec3(x, y, z);
                let bits = hash(cell + shift, seed * 0xc2b2ae35u);
                let feature = vec3<f32>(bits) / 4294967296.0;
                let delta = vec3<f32>(shift) + feature - local;
                let distance = dot(delta, delta);
                if details {
                    if distance < nearest {
                        second = nearest;
                        nearest = distance;
                        feature_id = bits.x ^ bits.y;
                    } else {
                        second = min(second, distance);
                    }
                } else {
                    nearest = min(nearest, distance);
                }
            }
        }
    }
    return vec3(sqrt(min(1.0, nearest)), sqrt(min(1.0, second)), f32(feature_id) / 4294967296.0);
}

fn cells(p: vec3<f32>, seed: u32) -> f32 {
    return cellular(p, seed, false).x;
}

fn cell_value(cell: vec3<i32>, seed: u32) -> f32 {
    return f32(hash(cell, seed * 0x9e3779b9u).x) / 4294967296.0;
}

fn drip(p: vec3<f32>, x: f32, salt: i32, seed: u32) -> f32 {
    let cx = x * 6.0;
    let column = i32(floor(cx));
    let width = 0.08 + 0.25 * cell_value(vec3(column, 0, salt), seed);
    let across = 1.0 - smoothstep(width * 0.3, width, abs(fract(cx) - 0.5 - (cell_value(vec3(column, 1, salt), seed) - 0.5) * 0.5));
    let yy = p.y * (0.35 + 0.4 * cell_value(vec3(column, 2, salt), seed)) + cell_value(vec3(column, 3, salt), seed) * 7.0;
    let t = fract(yy);
    let on = cell_value(vec3(column, i32(floor(yy)), 3 + salt), seed) > 0.5;
    return select(0.0, across * pow(t, 1.6) * (1.0 - smoothstep(0.9, 1.0, t)), on);
}

fn woven(p: vec3<f32>, seed: u32, twill: bool) -> f32 {
    let point = p.xy * 2.0;
    let cell = vec2<i32>(floor(point));
    let local = fract(point);
    let period = select(2, 4, twill);
    let warp_on_top = ((cell.x + cell.y) & (period - 1)) < period / 2;
    let thread_uv = select(local, local.yx, warp_on_top);
    let pi = 3.141592653589793;
    let thread = sqrt(max(0.0, sin(pi * thread_uv.x))) * (0.8 + 0.2 * sin(pi * thread_uv.y));
    let tone = select(1.0, 0.55, twill && !warp_on_top);
    let fiber = noise(p * vec3(40.0, 40.0, 1.0), seed, false);
    return clamp(thread * tone * (0.88 + 0.12 * fiber), 0.0, 1.0);
}

fn pattern(kind: u32, p: vec3<f32>, seed: u32) -> f32 {
    switch kind {
        case 1u: { return fbm(p, 6u, seed, false); }
        case 2u: { return fbm(p, 4u, seed, true); }
        case 3u: { return noise(p * 16.0, seed, false); }
        case 4u: { return cells(p, seed); }
        case 5u: {
            let n = fbm(p, 6u, seed, true) * 0.7 + fbm(p * 3.7, 3u, seed ^ 0x55u, false) * 0.3;
            return clamp((n - 0.45) * 4.0 + 0.5, 0.0, 1.0);
        }
        case 6u: {
            let n = noise(p * vec3(0.15, 6.0, 6.0), seed, true);
            let line = 1.0 - min(abs(n - 0.5) * 40.0, 1.0);
            let gate = noise(p * 0.8, seed ^ 0x99u, false);
            return line * clamp((gate - 0.45) * 4.0, 0.0, 1.0);
        }
        case 7u: {
            let warp = fbm(p * vec3(0.18, 0.6, 1.0), 3u, seed, true);
            let t = fract((p.y + (warp - 0.5) * 0.9) * 5.0);
            let ring = 1.0 - smoothstep(0.45, 1.0, t);
            let fiber = noise(p * vec3(2.0, 40.0, 1.0), seed ^ 0x3cu, false);
            let tone = fbm(p * vec3(0.6, 3.0, 1.0), 2u, seed ^ 0x71u, false);
            return clamp(ring * 0.6 + fiber * 0.25 + tone * 0.15, 0.0, 1.0);
        }
        case 8u: {
            let turb = fbm(p, 5u, seed, true);
            let veins = pow(abs(sin((p.x + p.y * 0.5) * 2.5 + turb * 9.0)), 0.3);
            let cloud = fbm(p * 2.0, 3u, seed ^ 0x19u, false);
            return clamp(veins * 0.85 + cloud * 0.15, 0.0, 1.0);
        }
        case 11u: {
            let first = cells(p, seed);
            let second = cells(p * 2.7 + 13.0, seed ^ 0x6bu);
            return min(smoothstep(0.08, 0.16, first), smoothstep(0.06, 0.12, second));
        }
        case 12u: {
            let first = cells(p * 2.0, seed ^ 0x45u);
            return smoothstep(0.55, 0.58, fbm(p, 5u, seed, true) + (first - 0.5) * 0.25);
        }
        case 13u: {
            let fine = noise(p * vec3(37.0, 1.3, 1.0), seed ^ 0x2bu, false);
            return clamp(max(drip(p, p.x, 0, seed), drip(p, p.x + 0.37, 1, seed) * 0.8) * (0.6 + 0.4 * fine), 0.0, 1.0);
        }
        case 48u: {
            let cell = vec3<i32>(floor(p));
            return select(0.0, 1.0, ((cell.x + cell.y + cell.z) & 1) == 0);
        }
        case 49u: { return select(0.0, 1.0, fract(p.x) < 0.5); }
        case 50u: {
            let row = i32(floor(p.y * 2.0));
            let x = p.x + select(0.0, 0.5, (row & 1) == 1);
            let local = fract(vec2(x, p.y * 2.0));
            let d = min(min(local.x, 1.0 - local.x), min(local.y, 1.0 - local.y) * 0.5);
            let tone = cell_value(vec3(i32(floor(x)), row, 0), seed) * 0.6 + noise(p * 9.0, seed, false) * 0.4;
            return smoothstep(0.02, 0.045, d) * (0.85 + 0.15 * tone);
        }
        case 51u: {
            let cell = vec2<i32>(floor(p.xy));
            let local = fract(p.xy);
            let d = min(min(local.x, 1.0 - local.x), min(local.y, 1.0 - local.y));
            return smoothstep(0.015, 0.035, d) * (0.9 + 0.1 * cell_value(vec3(cell, 0), seed));
        }
        case 52u: { return woven(p, seed, false); }
        case 53u: { return woven(p, seed, true); }
        case 54u: {
            let w = noise(p * 0.7, seed ^ 0x2du, true) - 0.5;
            let features = cellular(p + vec3(w * 0.4, -w * 0.4, 0.0), seed, true);
            let crease = smoothstep(0.0, 0.16, features.y - features.x);
            let pebble = 1.0 - features.x * 0.5;
            let fine = noise(p * 7.0, seed ^ 0x13u, false);
            return clamp(crease * pebble * (0.85 + 0.15 * fine), 0.0, 1.0);
        }
        case 55u: {
            let first = min(cells(p, seed) * 1.25, 1.0);
            let second = min(cells(p * 2.3 + 5.1, seed ^ 0x5au) * 1.25, 1.0);
            return clamp(first * first * 0.75 + second * second * 0.25, 0.0, 1.0);
        }
        case 56u: {
            let a = noise(p * vec3(1.5, 60.0, 1.0), seed, false);
            let b = noise(p * vec3(0.4, 18.0, 1.0), seed ^ 0x77u, false);
            return clamp(a * 0.6 + b * 0.4, 0.0, 1.0);
        }
        case 57u: { return cellular(p, seed, true).z; }
        case 58u: {
            let w = noise(p * 1.3, seed ^ 0x61u, true) - 0.5;
            let q = p * vec3(1.15, 0.85, 1.0) + vec3(w * 0.3, w * 0.3, 0.0);
            let features = cellular(q, seed, true);
            let tone = features.z;
            let print = select(0.0, 1.0 - smoothstep(0.18 + 0.1 * tone, 0.3 + 0.12 * tone, features.x), tone > 0.4);
            let ridges = 0.5 + 0.5 * sin(features.x * 190.0 + w * 12.0);
            let smudge = clamp((fbm(p * 1.7, 4u, seed ^ 0x33u, false) - 0.55) * 3.0, 0.0, 1.0);
            return clamp(print * ridges * 0.9 + smudge * 0.4, 0.0, 1.0);
        }
        default: { return periodic_pattern(kind, p, seed); }
    }
}

fn raw_procedural(layer: Operation, p: vec3<f32>) -> f32 {
    if layer.ids.y == 80u {
        return tile_generator(u32(layer.high.w), p.xy);
    }
    return pattern(layer.ids.y, p, layer.ids.z);
}

fn sample_procedural(layer: Operation, p: vec3<f32>) -> f32 {
    let reduced = layer.procedural.x != 0u;
    let cells = f32(max(layer.procedural.x, 1u)) / layer.uv.z;
    let grid = p.xy * cells - 0.5;
    let base = floor(grid) + 0.5;
    var values = vec4(0.0);
    for (var i = 0u; i < select(1u, 4u, reduced); i++) {
        let offset = vec2<f32>(vec2(i % 2u, i / 2u));
        let point = select(p.xy, (base + offset) / cells, reduced);
        let value = raw_procedural(layer, vec3(point, p.z));
        // Keeping the write at constant indices also lets FXC keep this loop rolled.
        switch i {
            case 0u: { values.x = value; }
            case 1u: { values.y = value; }
            case 2u: { values.z = value; }
            default: { values.w = value; }
        }
    }
    let t = fract(grid);
    return select(values.x, mix(mix(values.x, values.y, t.x), mix(values.z, values.w, t.x), t.y), reduced);
}

fn paint_ramp_color(word: u32) -> vec3<f32> {
    return bitcast<vec3<f32>>(vec3(srgb[word], srgb[word + 1u], srgb[word + 2u]));
}

fn gradient_color(op: Operation, t: f32) -> vec3<f32> {
    let last = op.procedural.z - 1u;
    let pos = filter_unit(t) * f32(last);
    let index = min(u32(pos), last);
    let offset = op.procedural.y;
    let first = paint_ramp_color(offset + index * 3u);
    let second = paint_ramp_color(offset + min(index + 1u, last) * 3u);
    return first + (second - first) * (pos - f32(index));
}

fn azimuth(y: f32, x: f32) -> f32 {
    if x == 0.0 && y == 0.0 { return 0.5; }
    return atan2(y, x) / 6.283185307 + 0.5;
}
fn value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec3<f32> {
    if layer.ids.y == 15u {
        let pixel = vec2<u32>(uv * vec2<f32>(params.source.xy));
        var color = layer.low.rgb;
        if layer.ids.z != 0u {
            let captured = filter_value(layer.ids.z - 1u, pixel);
            color = select(color, captured.rgb, captured.a > 0.0);
        }
        if layer.procedural.z > 0u {
            let lightness = luminance(color);
            let t = select(lightness, lightness * 0.5 + 0.5, (layer.ids.w & 2u) != 0u);
            return gradient_color(layer, t);
        }
        return select(color, vec3(luminance(color)), (layer.ids.w & 1u) != 0u);
    }
    if layer.ids.y == 0u {
        return layer.low.rgb;
    }
    if layer.ids.y == IMAGE_SOURCE_OPCODE {
        if layer.ids.z == MISSING_IMAGE {
            return vec3(0.5);
        }
        return decode_image_sample(layer, image_projection(layer, uv, surface).rgb);
    }
    let mode = layer.procedural.w;
    let p = surface.position * layer.projection.w + layer.projection.xyz;
    var points = array<vec2<f32>, 3>(p.zy, p.xz, p.xy);
    var w = vec3(1.0, 0.0, 0.0);
    if mode == 1u {
        let weighted = pow(abs(surface.normal), vec3(layer.balance.w));
        let weights = weighted / max(weighted.x + weighted.y + weighted.z, 1e-6);
        let minimum = 1.0 / (131072.0 * max(layer.uv.w, 1.0));
        let retained = select(vec3(0.0), weights, weights >= vec3(minimum));
        w = select(retained, weights, all(retained == vec3(0.0)));
    } else {
        switch mode {
            case 2u: {}
            case 3u: { points[0] = p.xz; }
            case 4u: { points[0] = p.xy; }
            case 5u: {
                var d = vec3(0.0, 1.0, 0.0);
                if dot(surface.position, surface.position) > 0.0 { d = normalize(surface.position); }
                points[0] = vec2(azimuth(d.z, d.x) * 4.0, acos(clamp(d.y, -1.0, 1.0)) * (2.0 / 3.141592654)) * layer.projection.w;
            }
            case 6u, 7u, 8u: {
                let pos = surface.position;
                let axis = select(select(pos.xyz, pos.yxz, mode == 6u), pos.xzy, mode == 8u);
                points[0] = vec2(azimuth(axis.z, axis.x) * 4.0, axis.y) * layer.projection.w;
            }
            case DECAL_PROJECTION: { points[0] = uv * layer.uv.z; }
            default: {
                let rotation = layer.projection.xy;
                let rotated = vec2(uv.x * rotation.x - uv.y * rotation.y, uv.x * rotation.y + uv.y * rotation.x);
                points[0] = (rotated * layer.projection.zw + layer.uv.xy) * layer.uv.z;
            }
        }
    }
    var n = 0.0;
    for (var plane = 0u; plane < select(1u, 3u, mode == 1u); plane++) {
        if w[plane] > 0.0 {
            n += w[plane] * sample_procedural(layer, vec3(points[plane], 0.0));
        }
    }
    let shaped = clamp((n - 0.5) * layer.uv.w + 0.5 + layer.balance.x, 0.0, 1.0);
    let t = select(shaped, 1.0 - shaped, layer.ids.w != 0u);
    if layer.procedural.z > 0u {
        return gradient_color(layer, t);
    }
    return layer.low.rgb + (layer.high.rgb - layer.low.rgb) * t;
}

fn fill_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> {
    if layer.procedural.w != DECAL_PROJECTION {
        return vec4(value(layer, uv, surface), 1.0);
    }
    let frame = u32(layer.uv.x);
    let decal = project_decal(frame, surface);
    if decal.z <= 0.0 {
        return vec4(0.0);
    }
    let alpha_image = srgb[frame + 11u];
    let sample_uv = select(decal.xy, uv, layer.ids.y == 15u);
    if alpha_image != MISSING_IMAGE {
        let sampled = material_image_at(alpha_image, decal.xy);
        let coverage = decal.z * clamp(sampled.a, 0.0, 1.0);
        if layer.ids.y == IMAGE_SOURCE_OPCODE && layer.ids.z == alpha_image {
            return vec4(decode_image_sample(layer, sampled.rgb), coverage);
        }
        return vec4(value(layer, sample_uv, surface), coverage);
    }
    return vec4(value(layer, sample_uv, surface), decal.z);
}

fn stored_sample(layer: Operation, table: u32, origin: vec2<u32>, size: vec2<u32>, pixel: vec2<u32>, empty: vec4<f32>) -> vec4<f32> {
    let atlas_tile = pixel / 64u;
    if any(atlas_tile < origin) {
        return empty;
    }
    let tile = atlas_tile - origin;
    if any(tile >= size) {
        return empty;
    }
    let local = pixel % 64u;
    let offset = paint[table + tile.y * size.x + tile.x];
    if offset == 0u {
        return empty;
    }
    let texel = offset + (local.y * 64u + local.x) * (layer.ids.w / 2u);
    let first = paint[texel];
    if layer.ids.w == 4u {
        let second = paint[texel + 1u];
        return vec4<f32>(vec4(first & 65535u, first >> 16u, second & 65535u, second >> 16u));
    }
    return vec4(vec3(f32(first & 65535u)), f32(first >> 16u));
}

fn resampled_sample(layer: Operation, image: u32, pixel: vec2<u32>, empty: vec4<f32>) -> vec4<f32> {
    let texels = vec2(paint[image], paint[image + 1u]);
    let origin = vec2(paint[image + 2u], paint[image + 3u]);
    let window = vec2(paint[image + 4u], paint[image + 5u]);
    let table = image + 6u;
    if all(texels == params.source.xy) {
        return stored_sample(layer, table, origin, window, pixel, empty);
    }
    let pos = max((vec2<f32>(pixel) + 0.5) * vec2<f32>(texels) / vec2<f32>(params.source.xy) - 0.5, vec2(0.0));
    let base = vec2<u32>(pos);
    let weights = fract(pos);
    let last = texels - 1u;
    let a = stored_sample(layer, table, origin, window, min(base, last), empty);
    let b = stored_sample(layer, table, origin, window, min(base + vec2(1u, 0u), last), empty);
    let c = stored_sample(layer, table, origin, window, min(base + vec2(0u, 1u), last), empty);
    let d = stored_sample(layer, table, origin, window, min(base + 1u, last), empty);
    let top = a * (1.0 - weights.x) + b * weights.x;
    let bottom = c * (1.0 - weights.x) + d * weights.x;
    return min(floor(top * (1.0 - weights.y) + bottom * weights.y + 0.5), vec4(65535.0));
}

fn stored_paint(layer: Operation, pixel: vec2<u32>) -> vec4<f32> {
    let signed = layer.ids.x == 6u;
    // An empty signed texel holds the rounded 16-bit encoding of signed zero.
    let empty = vec4(vec3(select(0.0, 32768.0, signed)), 0.0);
    var raw = empty;
    if layer.ids.z != NO_PAINT_IMAGE {
        let image = paint[layer.ids.z];
        if params.paint_window.x == 0xffffffffu {
            raw = resampled_sample(layer, image, pixel, empty);
        } else {
            raw = stored_sample(layer, image, params.paint_window.xy, params.paint_window.zw, pixel, empty);
        }
    }
    let p = raw / 65535.0;
    return vec4(select(p.rgb, p.rgb * 2.0 - 1.0, signed), p.a);
}

struct ProjectedBrush {
    rgba: vec4<f32>,
    coverage: f32,
}

fn brush_coverage(pixel: vec2<u32>) -> f32 {
    return f32(textureLoad(brush_alpha, vec2<i32>(pixel), 0).x) / 65535.0;
}

fn sample_brush_projection(pixel: vec2<u32>, surface: Surface) -> ProjectedBrush {
    let coverage = brush_coverage(pixel);
    if coverage <= 0.0 { return ProjectedBrush(vec4(0.0), 0.0); }
    return ProjectedBrush(stencil_sample(brush_stencil_image, brush_stencil, surface.position), coverage);
}

fn painted(layer: Operation, pixel: vec2<u32>, projected: ProjectedBrush) -> vec4<f32> {
    var stored = stored_paint(layer, pixel);
    if layer.projection.w > 0.0 {
        var m = projected.coverage * layer.uv.w * projected.rgba.a;
        if (params.coverage.w & PROJECTED_BRUSH) == 0u {
            m = brush_coverage(pixel) * layer.uv.w;
        }
        if m > 0.0 {
            // Match the paint commit's premultiplied over and 16-bit channel encoding.
            let value = select(layer.uv.xxx, layer.uv.xyz, layer.ids.w == 4u);
            let pigment = select(value, projected.rgba.rgb, layer.projection.w == 2.0);
            let remaining = 1.0 - m;
            let premul = pigment * m + stored.rgb * remaining;
            let signed = layer.ids.x == 6u;
            let encode = select(premul, premul * 0.5 + 0.5, signed);
            let quantized = floor(clamp(encode, vec3(0.0), vec3(1.0)) * 65535.0 + 0.5) / 65535.0;
            let rgb = select(quantized, quantized * 2.0 - 1.0, signed);
            let coverage = f32(to_u16(m + stored.a * remaining)) / 65535.0;
            stored = vec4(rgb, coverage);
        }
    }
    if stored.a == 0.0 {
        return vec4(0.0);
    }
    return vec4(stored.rgb / stored.a, stored.a);
}

fn shift_unit(x: f32, amount: f32) -> f32 {
    let a = clamp(amount, -1.0, 1.0);
    return select(x * (1.0 + a), x + (1.0 - x) * a, a >= 0.0);
}

fn hue_to_rgb(p: f32, q: f32, hue: f32) -> f32 {
    let t = fract(hue);
    if t < 1.0 / 6.0 { return p + (q - p) * 6.0 * t; }
    if t < 0.5 { return q; }
    if t < 2.0 / 3.0 { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
    return p;
}

fn hsl_adjust(rgb: vec3<f32>, args: vec3<f32>) -> vec3<f32> {
    let c = clamp(rgb, vec3(0.0), vec3(1.0));
    let maximum = max(c.r, max(c.g, c.b));
    let minimum = min(c.r, min(c.g, c.b));
    let d = maximum - minimum;
    let lightness = (maximum + minimum) * 0.5;
    let light = shift_unit(lightness, args.z);
    if d <= 1e-6 { return vec3(light); }
    let saturation = d / select(maximum + minimum, 2.0 - maximum - minimum, lightness > 0.5);
    let sat = shift_unit(saturation, args.y);
    if sat <= 0.0 { return vec3(light); }
    var hue = (c.r - c.g) / d + 4.0;
    if maximum == c.r {
        let turns = (c.g - c.b) / d / 6.0;
        hue = fract(turns) * 6.0;
    } else if maximum == c.g {
        hue = (c.b - c.r) / d + 2.0;
    }
    let h = hue / 6.0 + args.x;
    let q = select(light + sat - light * sat, light * (1.0 + sat), light < 0.5);
    let p = 2.0 * light - q;
    return vec3(hue_to_rgb(p, q, h + 1.0 / 3.0), hue_to_rgb(p, q, h), hue_to_rgb(p, q, h - 1.0 / 3.0));
}

fn filter_unit(x: f32) -> f32 {
    return select(0.0, clamp(x, 0.0, 1.0), abs(x) <= 3.402823466e38);
}

fn color_balance_shift(op: Operation, lightness: f32) -> vec3<f32> {
    let l = filter_unit(lightness);
    let s = 1.0 - l;
    return op.low.xyz * (s * s) + op.high.xyz * (2.0 * l * s) + op.projection.xyz * (l * l);
}

fn replace_color(op: Operation, c: vec3<f32>) -> vec3<f32> {
    let d = distance(c, op.low.xyz) / sqrt(3.0);
    let tolerance = op.projection.x;
    let softness = op.projection.y;
    var amount = 0.0;
    if d <= tolerance {
        amount = 1.0;
    } else if d < tolerance + softness {
        let x = 1.0 - (d - tolerance) / max(softness, 1e-6);
        amount = x * x * (3.0 - 2.0 * x);
    }
    let color = c + (op.high.xyz - op.low.xyz) * amount;
    return vec3(filter_unit(color.r), filter_unit(color.g), filter_unit(color.b));
}

fn scalar_filter(op: Operation, x: f32) -> f32 {
    switch op.ids.y {
        case 20u: {
            let t = clamp((x - op.low.x) / max(op.low.y - op.low.x, 1e-5), 0.0, 1.0);
            return lerp(op.high.x, op.high.y, pow(t, 1.0 / max(op.low.z, 1e-3)));
        }
        case 21u: { return 1.0 - x; }
        case 22u: { return select(0.0, 1.0, x >= op.low.x); }
        case 23u: { return clamp((x - 0.5) * (1.0 + clamp(op.low.x, -1.0, 1.0)) + 0.5 + op.low.y, 0.0, 1.0); }
        case 24u: {
            let steps = max(floor(clamp(op.low.x, 2.0, 64.0) + 0.5) - 1.0, 1.0);
            return floor(clamp(x, 0.0, 1.0) * steps + 0.5) / steps;
        }
        case 25u: { return clamp(x, op.low.x, op.low.y); }
        case 26u: {
            if op.low.y >= 1.0 { return select(0.0, 1.0, x >= op.low.x); }
            let width = max(1.0 - clamp(op.low.y, 0.0, 1.0), 1e-3);
            return clamp((x - op.low.x) / width + 0.5, 0.0, 1.0);
        }
        case 27u: { return clamp(op.low.y + (x - op.low.y) * op.low.x, 0.0, 1.0); }
        case 28u: {
            var t = clamp(x, 0.0, 1.0) + clamp(op.low.x, -1.0, 1.0);
            if t > 1.0 { t -= 1.0; }
            if t < 0.0 { t += 1.0; }
            return clamp(t, 0.0, 1.0);
        }
        case 31u: { return shift_unit(clamp(x, 0.0, 1.0), op.low.z); }
        case 32u: {
            let shift = color_balance_shift(op, x);
            return filter_unit(x + (shift.x + shift.y + shift.z) / 3.0);
        }
        case 33u: {
            if op.procedural.z > 0u { return luminance(gradient_color(op, x)); }
            return x;
        }
        case 37u: {
            var passes = abs(x - op.low.x) <= op.low.y;
            if op.ids.z == 0u { passes = x < op.low.x; }
            if op.ids.z == 2u { passes = x > op.low.x; }
            return select(0.0, 1.0, passes);
        }
        default: { return x; }
    }
}

fn effect_color(op: Operation, c: vec3<f32>, channel: u32, surface: Surface, baked_ao: f32) -> vec3<f32> {
    if op.ids.y == 35u {
        if channel != 0u { return c; }
        let facing = max(0.0, dot(surface.normal, op.high.xyz));
        let ao = select(1.0, clamp(baked_ao, 0.0, 1.0), abs(baked_ao) <= 3.402823466e38);
        let light = op.low.y * (1.0 + (ao - 1.0) * op.low.z) + op.low.x * facing;
        let factor = select(1.0, max(0.0, light), abs(light) <= 3.402823466e38);
        return clamp(c * factor, vec3(0.0), vec3(1.0));
    }
    if channel == 7u || channel == 11u {
        return select(c, vec3(c.x, 1.0 - c.y, c.z), op.ids.y == 30u);
    }
    if channel == 0u || (channel >= 8u && channel <= 10u) {
        if op.ids.y == 29u { return vec3(dot(c, vec3(0.3, 0.59, 0.11))); }
        if op.ids.y == 31u { return hsl_adjust(c, op.low.xyz); }
        if op.ids.y == 32u {
            let color = c + color_balance_shift(op, luminance(c));
            return vec3(filter_unit(color.r), filter_unit(color.g), filter_unit(color.b));
        }
        if op.ids.y == 33u {
            if op.procedural.z > 0u { return gradient_color(op, luminance(c)); }
            return c;
        }
        if op.ids.y == 34u { return replace_color(op, c); }
        return vec3(scalar_filter(op, c.r), scalar_filter(op, c.g), scalar_filter(op, c.b));
    }
    if channel == 6u {
        return vec3(scalar_filter(op, c.x * 0.5 + 0.5) * 2.0 - 1.0);
    }
    return vec3(scalar_filter(op, c.x));
}

fn burn(a: f32, b: f32) -> f32 {
    if a >= 1.0 { return 1.0; }
    if b <= 0.0 { return 0.0; }
    return 1.0 - min((1.0 - a) / b, 1.0);
}

fn dodge(a: f32, b: f32) -> f32 {
    if a <= 0.0 { return 0.0; }
    if b >= 1.0 { return 1.0; }
    return min(a / (1.0 - b), 1.0);
}

fn overlay(a: f32, b: f32) -> f32 {
    return select(1.0 - 2.0 * (1.0 - a) * (1.0 - b), 2.0 * a * b, a < 0.5);
}

fn blend_scalar(mode: u32, a: f32, b: f32) -> f32 {
    switch mode {
        case 1u: { return a; }
        case 3u: { return a * b; }
        case 4u: { return a / max(b, 1e-6); }
        case 5u: { return b / max(a, 1e-6); }
        case 6u: { return min(a, b); }
        case 7u: { return max(a, b); }
        case 8u: { return a + b; }
        case 9u: { return a - b; }
        case 10u: { return b - a; }
        case 11u: { return abs(a - b); }
        case 12u: { return a + b - 2.0 * a * b; }
        case 13u: { return a + 2.0 * b - 1.0; }
        case 14u: { return overlay(a, b); }
        case 15u: { return 1.0 - (1.0 - a) * (1.0 - b); }
        case 16u: { return a + b - 1.0; }
        case 17u: { return burn(a, b); }
        case 18u: { return dodge(a, b); }
        case 19u: {
            let x = clamp(a, 0.0, 1.0);
            let y = clamp(b, 0.0, 1.0);
            if y <= 0.5 { return x - (1.0 - 2.0 * y) * x * (1.0 - x); }
            let d = select(sqrt(x), ((16.0 * x - 12.0) * x + 4.0) * x, x <= 0.25);
            return x + (2.0 * y - 1.0) * (d - x);
        }
        case 20u: { return overlay(b, a); }
        case 21u: { return select(dodge(a, 2.0 * (b - 0.5)), burn(a, 2.0 * b), b < 0.5); }
        case 22u: { return select(max(a, 2.0 * b - 1.0), min(a, 2.0 * b), b < 0.5); }
        default: { return b; }
    }
}

fn luminance(c: vec3<f32>) -> f32 { return dot(c, vec3(0.3, 0.59, 0.11)); }
fn maximum(c: vec3<f32>) -> f32 { return max(c.x, max(c.y, c.z)); }
fn minimum(c: vec3<f32>) -> f32 { return min(c.x, min(c.y, c.z)); }

fn with_luminance(color: vec3<f32>, luminance_goal: f32) -> vec3<f32> {
    var c = color + luminance_goal - luminance(color);
    let l = luminance(c);
    let lo = minimum(c);
    let hi = maximum(c);
    if lo < 0.0 && l - lo > 1e-6 { c = l + (c - l) * l / (l - lo); }
    if hi > 1.0 && hi - l > 1e-6 { c = l + (c - l) * (1.0 - l) / (hi - l); }
    return c;
}

fn rnm(a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    let t = a * 2.0 - vec3(1.0, 1.0, 0.0);
    let u = (b * 2.0 - 1.0) * vec3(-1.0, -1.0, 1.0);
    let n = t * dot(t, u) - u * t.z;
    return n / max(length(n), 1e-6) * 0.5 + 0.5;
}

fn blend_rgb(mode: u32, a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    switch mode {
        case 23u: { return b * luminance(a); }
        case 24u: {
            let lo = minimum(a);
            let width = maximum(a) - lo;
            var saturated = vec3(0.0);
            if width > 1e-6 { saturated = (a - lo) * (maximum(b) - minimum(b)) / width; }
            return with_luminance(saturated, luminance(a));
        }
        case 25u: { return with_luminance(b, luminance(a)); }
        case 26u: {
            let max_a = maximum(a);
            let max_b = maximum(b);
            if max_a <= 1e-6 { return vec3(max_b); }
            return a * max_b / max_a;
        }
        case 27u: {
            let p = a * 2.0 - 1.0;
            let q = b * 2.0 - 1.0;
            let n = vec3(p.xy + q.xy, p.z * q.z);
            return n / max(length(n), 1e-6) * 0.5 + 0.5;
        }
        case 28u: { return rnm(a,b); }
        case 29u: { return rnm(b,a); }
        default: { return vec3(blend_scalar(mode, a.x,b.x), blend_scalar(mode, a.y,b.y), blend_scalar(mode, a.z,b.z)); }
    }
}

fn generator_curvature(effect: Operation, pixel: vec2<u32>, finest: f32, mode: u32, micro: f32) -> f32 {
    let bake_pixel = bake_pixel_at(pixel);
    let offset = u32(effect.uv.x);
    var edges = 0.0;
    var cavities = 0.0;
    for (var i = 0u; i < 7u; i++) {
        let weight = bitcast<f32>(srgb[offset + i]);
        if weight == 0.0 { continue; }
        var response = (finest - 0.5) * 2.0;
        if i > 0u {
            let size = f32(1u << (i - 1u));
            response = 0.0;
            if effect.effects.w == 0u {
                response = curvature_level_at(bake_pixel, i - 1u) * (2.8284271247461903 * size);
            }
        }
        edges = 1.0 - (1.0 - edges) * (1.0 - weight * clamp(response, 0.0, 1.0));
        cavities = 1.0 - (1.0 - cavities) * (1.0 - weight * clamp(-response, 0.0, 1.0));
    }
    edges = 1.0 - (1.0 - edges) * (1.0 - max(micro, 0.0));
    cavities = 1.0 - (1.0 - cavities) * (1.0 + min(micro, 0.0));
    switch mode {
        case 1u: { return edges; }
        case 2u: { return cavities; }
        case 3u: { return 1.0 - (1.0 - edges) * (1.0 - cavities); }
        default: { return 0.5 + 0.5 * (edges - cavities); }
    }
}

fn generator_ramp(effect: Operation, texel: LayerTexel) -> f32 {
    let surface = texel.surface;
    var baked = texel.baked;
    if effect.effects.w != 0u {
        let source = layers[effect.effects.w - 1u];
        let input = fill_value(source, texel.uv, surface).x;
        switch effect.ids.y {
            case 40u, 43u: { baked.y = input; }
            case 41u, 44u: { baked.x = input; }
            case 42u: { baked.z = input; }
            default: {}
        }
    }
    let detail = u32(effect.uv.y);
    var micro = 0.0;
    if effect.effects.z != 0u {
        let height = value(layers[effect.effects.z - 1u], texel.uv, surface).x;
        micro = clamp(height, -1.0, 1.0) * effect.uv.z;
    }
    var signal = 0.0;
    switch effect.ids.y {
        case 40u: { signal = (baked.y - 0.5) * 2.0; }
        case 41u: { signal = 1.0 - baked.x; }
        case 42u: { signal = 1.0 - baked.z; }
        case 43u: { signal = baked.y; }
        case 44u: { signal = baked.x; }
        case 45u: { signal = dot(surface.position, effect.high.xyz) * 0.5 + 0.5; }
        default: { signal = dot(surface.normal, effect.high.xyz) * 0.5 + 0.5; }
    }
    if detail != 0u {
        signal = generator_curvature(effect, texel.pixel, baked.y, detail, micro);
    } else {
        switch effect.ids.y {
            case 40u, 43u, 44u: { signal += micro; }
            case 41u: { signal -= micro; }
            default: {}
        }
    }
    if effect.low.z > 0.0 {
        let noise = fbm(surface.position * effect.projection.x, 4u, effect.ids.z, true);
        if detail != 0u {
            let neutral = select(0.0, 0.5, detail == 4u);
            signal = neutral + (signal - neutral) * max(0.0, 1.0 - noise * effect.low.z);
        } else {
            signal += (noise - 0.5) * effect.low.z;
        }
    }
    let threshold = 1.0 - clamp(effect.low.x, 0.0, 1.0);
    let ramp = clamp((signal - (threshold - effect.low.y)) / (2.0 * effect.low.y), 0.0, 1.0);
    return ramp * ramp * (3.0 - 2.0 * ramp);
}

fn finish_generator(effect: Operation, ramp: f32) -> f32 {
    var value = ramp;
    if effect.projection.z != 0.5 || effect.projection.w != 0.0 {
        let balance = clamp(effect.projection.z, 0.0, 1.0);
        let low = max(1.0 - 2.0 * balance, 0.0);
        let high = min(2.0 - 2.0 * balance, 1.0);
        let middle = (low + high) * 0.5;
        let keep = 1.0 - clamp(effect.projection.w, 0.0, 1.0);
        let first = middle - (middle - low) * keep;
        let last = middle + (high - middle) * keep;
        if last - first < 1e-6 {
            value = select(0.0, 1.0, value >= middle);
        } else {
            value = clamp((value - first) / (last - first), 0.0, 1.0);
        }
    }
    return clamp(select(value, 1.0 - value, effect.ids.w != 0u) + effect.projection.y, 0.0, 1.0);
}

// Each scratch texel holds four float components followed by UV coverage.
fn field_word(slot: u32, local: vec2<u32>) -> u32 {
    let size = params.filter_region.zw;
    return params.filter_stage.x + (slot * size.x * size.y + local.y * size.x + local.x) * 5u;
}

fn filter_value(slot: u32, pixel: vec2<u32>) -> vec4<f32> {
    if any(pixel < params.filter_region.xy) {
        return vec4(0.0);
    }
    let local = pixel - params.filter_region.xy;
    if any(local >= params.filter_region.zw) {
        return vec4(0.0);
    }
    let start = field_word(slot, local);
    return bitcast<vec4<f32>>(vec4(output[start], output[start + 1u], output[start + 2u], output[start + 3u]));
}

struct LayerTexel {
    pixel: vec2<u32>,
    uv: vec2<f32>,
    surface: Surface,
    baked: vec3<f32>,
    projected: ProjectedBrush,
}

// The most recent prepared field already contains all the effects below it.
fn previous_field(start: u32, count: u32) -> u32 {
    for (var i = count; i > 0u; i--) {
        if layers[start + i - 1u].ids.y == SPATIAL_FILTER_OPCODE {
            return i;
        }
    }
    return 0u;
}

fn color_selection(effect: Operation, pixel: vec2<u32>) -> f32 {
    if params.id_bake.x == 0u || effect.procedural.z == 0u { return 0.0; }
    let color = id_color_at(pixel);
    for (var i = 0u; i < effect.procedural.z; i++) {
        let at = effect.procedural.y + i * 3u;
        let picked = bitcast<vec3<f32>>(vec3(srgb[at], srgb[at + 1u], srgb[at + 2u]));
        if all(abs(color - picked) <= vec3(effect.low.x)) { return 1.0; }
    }
    return 0.0;
}

fn layer_mask(layer: Operation, texel: LayerTexel, count: u32) -> f32 {
    let start = params.program.x + layer.effects.x;
    var skip = 0u;
    var mask = layer.balance.y;
    if layer.spatial.z != 0u {
        skip = previous_field(start, count);
        if skip != 0u {
            let field = layers[start + skip - 1u].spatial.x;
            mask = clamp(filter_value(field, texel.pixel).x, 0.0, 1.0);
        }
    }
    for (var i = skip; i < count; i++) {
        let e = layers[start + i];
        if e.ids.y == 16u {
            continue;
        }
        var weight = e.low.w;
        var result: f32;
        if e.ids.y == 10u {
            let sampled = painted(e, texel.pixel, texel.projected);
            result = blend_scalar(u32(e.balance.z), mask, sampled.x);
            weight *= sampled.w;
        } else if e.ids.y < 20u || e.ids.y >= 48u {
            let sampled = fill_value(e, texel.uv, texel.surface);
            result = blend_scalar(u32(e.balance.z), mask, sampled.x);
            weight *= sampled.a;
        } else if e.ids.y >= 40u {
            var ramp: f32;
            if e.spatial.y == 0u {
                ramp = generator_ramp(e, texel);
            } else {
                ramp = filter_value(e.spatial.y - 1u, texel.pixel).x;
            }
            if e.spatial.z != 0u {
                // A generator's region is stored as mask values before later effects read it.
                mask = clamp(mask, 0.0, 1.0);
                result = blend_scalar(u32(e.balance.z), mask, finish_generator(e, ramp));
                mask = clamp(lerp(mask, result, weight), 0.0, 1.0);
                continue;
            }
            result = blend_scalar(u32(e.balance.z), mask, finish_generator(e, ramp));
        } else if e.ids.y == COLOR_SELECTION_OPCODE {
            result = blend_scalar(u32(e.balance.z), mask, color_selection(e, texel.pixel));
        } else {
            result = scalar_filter(e, mask);
        }
        mask = lerp(mask, result, weight);
    }
    return clamp(mask, 0.0, 1.0);
}

fn content_over(content: vec4<f32>, source: vec4<f32>, effect: Operation, channel: u32) -> vec4<f32> {
    let alpha = source.a * effect.low.w;
    if alpha <= 0.0 { return content; }
    var blended = source.rgb;
    if content.a > 0.0 {
        let mode = u32(effect.balance.z);
        if channel == 0u || (channel >= 7u && channel <= 11u) {
            blended = blend_rgb(mode, content.rgb, source.rgb);
        } else {
            blended = vec3(blend_scalar(mode, content.x, source.x));
        }
    }
    let coverage = alpha + content.a * (1.0 - alpha);
    let amount = alpha / max(coverage, 1e-5);
    return vec4(content.rgb + (blended - content.rgb) * amount, coverage);
}

fn layer_content(layer: Operation, texel: LayerTexel, count: u32) -> vec4<f32> {
    let start = params.program.x + layer.effects.z;
    var skip = 0u;
    if layer.spatial.y != 0u {
        skip = previous_field(start, count);
    }
    var content: vec4<f32>;
    if skip != 0u {
        let field = layer.spatial.x + layers[start + skip - 1u].spatial.x;
        let filtered = filter_value(field, texel.pixel);
        let coverage = clamp(filtered.a, 0.0, 1.0);
        var color = vec3(0.0);
        if coverage > 1e-5 {
            color = filtered.rgb / coverage;
        }
        content = vec4(color, coverage);
    } else if layer.ids.y == 10u {
        content = painted(layer, texel.pixel, texel.projected);
    } else {
        content = fill_value(layer, texel.uv, texel.surface);
    }
    if content.a <= 0.0 && layer.spatial.w == 0u {
        return vec4(0.0);
    }
    for (var i = skip; i < count; i++) {
        let effect = layers[start + i];
        if effect.ids.y == 10u {
            let paint = painted(effect, texel.pixel, texel.projected);
            content = content_over(content, paint, effect, layer.ids.x);
        } else if effect.ids.y < 20u || effect.ids.y >= 48u {
            let fill = fill_value(effect, texel.uv, texel.surface);
            content = content_over(content, fill, effect, layer.ids.x);
        } else if effect.ids.y != 39u {
            let filtered = effect_color(effect, content.rgb, layer.ids.x, texel.surface, texel.baked.x);
            content = vec4(content.rgb + (filtered - content.rgb) * effect.low.w, content.a);
        }
    }
    return vec4(content.rgb, min(content.a, 1.0));
}

fn geometry_allows(offset: u32, triangle: u32) -> bool {
    if offset == 0u || triangle == 0xffffffffu { return true; }
    return (srgb[offset + triangle / 32u] & (1u << (triangle % 32u))) != 0u;
}

fn mask_with_height(layer: Operation, texel: LayerTexel, mask: f32) -> f32 {
    let contrast = bitcast<f32>(layer.constraints.z);
    if contrast == 0.0 || mask <= 0.0 || mask >= 1.0 { return mask; }
    var height = 0.0;
    if layer.constraints.y != 0u {
        let height_op = layers[layer.constraints.y - 1u];
        height = layer_content(height_op, texel, height_op.effects.w).x;
    }
    if !(abs(height) <= 3.402823466e38) { return mask; }
    let offset = bitcast<f32>(layer.constraints.w);
    return clamp(mask + contrast * 4.0 * mask * (1.0 - mask) * (height + offset), 0.0, 1.0);
}

fn uv_texel_covered(pixel: vec2<u32>) -> bool {
    let word = uv_coverage[pixel.y * params.coverage.y + pixel.x / 32u];
    return (word & (1u << (pixel.x % 32u))) != 0u;
}

fn capture_material_anchor(texel: LayerTexel, end: u32, triangle: u32) -> vec3<f32> {
    let channel = params.anchor.x;
    let color_channel = channel == 0u || (channel >= 7u && channel <= 11u);
    var result = params.anchor_default.rgb;
    var parents: array<vec3<f32>, 17>;
    var group_masks: array<f32, 17>;
    var depth = 0u;
    for (var i = 0u; i < end; i++) {
        let layer = layers[i];
        if composite_groups && layer.ids.y == BEGIN_GROUP {
            parents[depth] = result;
            var group_mask = 0.0;
            if geometry_allows(layer.constraints.x, triangle) {
                group_mask = mask_with_height(layer, texel, layer_mask(layer, texel, layer.effects.y));
            }
            group_masks[depth] = group_mask;
            depth += 1u;
            continue;
        }
        if composite_groups && layer.ids.y == END_GROUP {
            depth -= 1u;
            let weight = group_masks[depth] * bitcast<f32>(srgb[layer.ids.z + channel]);
            result = mix(parents[depth], result, weight);
            continue;
        }
        if layer.ids.x != channel || !geometry_allows(layer.constraints.x, triangle) {
            continue;
        }
        let count = select(layer.effects.w, min(layer.effects.w, params.anchor.z), i >= params.anchor.y);
        let content = layer_content(layer, texel, count);
        let mask = mask_with_height(layer, texel, layer_mask(layer, texel, layer.effects.y));
        let mode = u32(layer.balance.z);
        let weight = layer.low.w * mask * select(content.a, 1.0, mode == 2u && content.a > 0.0);
        if weight > 0.0 {
            if color_channel {
                result = mix(result, blend_rgb(mode, result, content.rgb), weight);
            } else {
                result = vec3(lerp(result.x, blend_scalar(mode, result.x, content.x), weight));
            }
        }
    }
    return result;
}

@compute @workgroup_size(8, 8)
fn prepare_filter(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.filter_chunk.zw) {
        return;
    }
    let local = params.filter_chunk.xy + id.xy;
    let pixel = params.filter_region.xy + local;
    var prepared = vec4(0.0);
    var geometric_weight = 0.0;
    var covered = all(pixel < params.source.xy);
    if covered && params.coverage.x != 0u {
        covered = uv_texel_covered(pixel);
    }
    if covered {
        let surface = surface_at(pixel);
        let uv = (vec2<f32>(pixel) + 0.5) / vec2<f32>(params.source.xy);
        var projected = ProjectedBrush(vec4(0.0), 0.0);
        if (params.coverage.w & PROJECTED_BRUSH) != 0u {
            projected = sample_brush_projection(pixel, surface);
        }
        var baked = vec3(1.0, 0.5, 1.0);
        if (params.coverage.w & 2u) != 0u {
            baked = bitcast<vec3<f32>>(baked_at(pixel).xyz);
        }
        let texel = LayerTexel(pixel, uv, surface, baked, projected);
        let layer = layers[params.filter_stage.z];
        let count = params.filter_stage.w & 0x0fffffffu;
        if (params.filter_stage.w & 0x40000000u) != 0u {
            let generator = layers[params.program.x + layer.effects.x + count];
            prepared = vec4(generator_ramp(generator, texel), 0.0, 0.0, 1.0);
        } else if (params.filter_stage.w & 0x30000000u) != 0u {
            var triangle = 0xffffffffu;
            if (params.coverage.w & GEOMETRY_MASKS) != 0u {
                triangle = textureLoad(surface_uv, vec2<i32>(pixel), 0).x;
            }
            if (params.filter_stage.w & 0x20000000u) != 0u {
                prepared = vec4(capture_material_anchor(texel, params.filter_stage.z, triangle), 1.0);
            } else if geometry_allows(layer.constraints.x, triangle) {
                prepared = vec4(vec3(layer_mask(layer, texel, count)), 1.0);
            }
        } else if (params.filter_stage.w & 0x80000000u) != 0u {
            prepared = vec4(layer_mask(layer, texel, count), 0.0, 0.0, 1.0);
        } else {
            let content = layer_content(layer, texel, count);
            prepared = vec4(content.rgb * content.a, content.a);
        }
        geometric_weight = 1.0;
    }
    let start = field_word(params.filter_stage.y, local);
    let words = bitcast<vec4<u32>>(prepared);
    output[start] = words.x;
    output[start + 1u] = words.y;
    output[start + 2u] = words.z;
    output[start + 3u] = words.w;
    output[start + 4u] = bitcast<u32>(geometric_weight);
}

fn to_u16(v: f32) -> u32 {
    return u32(clamp(v, 0.0, 1.0) * 65535.0 + 0.5);
}

fn quantized_color(color: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(vec3<u32>(to_u16(color.r), to_u16(color.g), to_u16(color.b))) / 65535.0;
}

fn pack_srgb(color: vec3<f32>, alpha: f32) -> u32 {
    return srgb[to_u16(color.r) >> 4u]
        | (srgb[to_u16(color.g) >> 4u] << 8u)
        | (srgb[to_u16(color.b) >> 4u] << 16u)
        | ((to_u16(alpha) >> 8u) << 24u);
}

fn writes_display(kind: u32) -> bool {
    return (params.program.z & (1u << kind)) != 0u;
}

fn display_index(kind: u32, index: u32) -> u32 {
    // Normal (2) and Height (3) use two words per pixel. The remaining display
    // outputs use one. The Rust band layout omits all inactive outputs.
    let earlier = params.program.z & ((1u << kind) - 1u);
    let offset = countOneBits(earlier) + countOneBits(earlier & 12u);
    return offset * params.band.w + select(index, 2u * index, kind == 2u || kind == 3u);
}

// Find a covered source within the padding radius. The CPU padding's breadth-first
// search breaks equal Chebyshev distances in row-major source order.
fn padded_source(pixel: vec2<u32>) -> vec2<u32> {
    let pitch = params.coverage.y;
    let radius = params.coverage.z;
    if uv_texel_covered(pixel) {
        return pixel;
    }

    let left = pixel.x - min(pixel.x, radius);
    let right = min(pixel.x + radius, params.source.x - 1u);
    let start_word = left / 32u;
    let start_bit = left % 32u;
    let center = pixel.x - left;
    let column_mask = (1u << (right - left + 1u)) - 1u;
    let lower_columns = (1u << (center + 1u)) - 1u;
    let top = pixel.y - min(pixel.y, radius);
    let bottom = min(pixel.y + radius, params.source.y - 1u);
    var best = params.source.xy;
    var distance = radius + 1u;

    for (var row = top; row <= bottom; row++) {
        let dy = max(row, pixel.y) - min(row, pixel.y);
        if dy >= distance {
            continue;
        }
        let index = row * pitch + start_word;
        var bits = uv_coverage[index] >> start_bit;
        if right / 32u != start_word {
            bits |= uv_coverage[index + 1u] << (32u - start_bit);
        }
        bits &= column_mask;
        if bits == 0u {
            continue;
        }

        let lower = bits & lower_columns;
        let upper = bits & (0xffffffffu << center);
        var dx = radius + 1u;
        if lower != 0u {
            dx = center - firstLeadingBit(lower);
        }
        if upper != 0u {
            dx = min(dx, firstTrailingBit(upper) - center);
        }
        let candidate = max(dx, dy);
        if candidate < distance {
            distance = candidate;
            let first = center - min(center, distance);
            best = vec2(left + firstTrailingBit(bits & (0xffffffffu << first)), row);
        }
    }
    return best;
}

@compute @workgroup_size(8, 8)
fn compose(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.band.x || id.y >= params.band.y {
        return;
    }
    let display_pixel = vec2(id.x + params.program.w, id.y + params.band.z);
    let sample_pixel = display_pixel * params.source.z;
    var pixel = sample_pixel;
    if params.coverage.x != 0u {
        pixel = padded_source(pixel);
    }
    let covered = all(pixel < params.source.xy);
    let uv = (vec2<f32>(pixel) + 0.5) / vec2<f32>(params.source.xy);
    let surface = surface_at(pixel);
    var projected = ProjectedBrush(vec4(0.0), 0.0);
    if covered && (params.coverage.w & PROJECTED_BRUSH) != 0u {
        projected = sample_brush_projection(pixel, surface);
    }
    var mask_bakes = vec3(1.0, 0.5, 1.0);
    var display_ao = 1.0;
    var baked_normal = 0u;
    if (params.coverage.w & 2u) != 0u {
        let display_bake = baked_at(sample_pixel);
        let scalars = bitcast<vec3<f32>>(display_bake.xyz);
        display_ao = scalars.x;
        baked_normal = display_bake.w;
        if covered {
            mask_bakes = scalars;
            if any(pixel != sample_pixel) { mask_bakes = bitcast<vec3<f32>>(baked_at(pixel).xyz); }
        }
    }
    let index = id.y * params.program.y + id.x;
    let texel = LayerTexel(pixel, uv, surface, mask_bakes, projected);
    if params.source.w == 2u {
        var gray = 0u;
        if covered {
            let layer = layers[params.anchor.w];
            gray = u32(layer_mask(layer, texel, layer.effects.y) * 255.0 + 0.5);
        }
        output[index] = (gray * 0x00010101u) | 0xff000000u;
        return;
    }
    var base = vec3(0.5);
    var normal = vec3(0.5, 0.5, 1.0);
    var scalars = array<f32, 6>(1.0, 1.0, 0.5, 0.0, 0.5, 0.0);
    // Grouped by the destination slots assigned by Rust.
    var lobe_colors = array<vec3<f32>, 4>(vec3(1.0), vec3(0.04), vec3(0.0), vec3(0.5, 0.5, 1.0));
    var lobe_values = array<f32, 9>(0.0, 0.5, 0.0, 0.0, 0.1, 0.0, 0.0, 0.0, 0.5);
    var last_mask = 0xffffffffu;
    var mask_value = 1.0;
    var triangle = 0xffffffffu;
    if covered && (params.coverage.w & GEOMETRY_MASKS) != 0u {
        triangle = textureLoad(surface_uv, vec2<i32>(pixel), 0).x;
    }
    var group_depth = 0u;
    for (var i = 0u; covered && i < params.program.x; i++) {
        let layer = layers[i];
        if composite_groups {
            if layer.ids.y == BEGIN_GROUP {
                var mask = 0.0;
                if geometry_allows(layer.constraints.x, triangle) {
                    mask = mask_with_height(layer, texel, layer_mask(layer, texel, layer.effects.y));
                }
                if mask <= 0.0 {
                    // The instruction stores the first operation after its END_GROUP.
                    i = layer.ids.w - 1u;
                    continue;
                }
                save_group_parent(group_depth, index, mask, MaterialValues(base, normal, scalars, lobe_colors, lobe_values));
                group_depth += 1u;
                continue;
            }
            if layer.ids.y == END_GROUP {
                group_depth -= 1u;
                let mixed = mix_group_parent(group_depth, index, layer.ids.z, MaterialValues(base, normal, scalars, lobe_colors, lobe_values));
                base = mixed.base;
                normal = mixed.normal;
                scalars = mixed.scalars;
                lobe_colors = mixed.lobe_colors;
                lobe_values = mixed.lobe_values;
                continue;
            }
        }
        if layer.ids.x == 0xffffffffu { continue; }
        if !geometry_allows(layer.constraints.x, triangle) { continue; }
        let mode = u32(layer.balance.z);
        var weight = layer.low.w;
        if layer.effects.y == 0u {
            weight *= layer.balance.y;
        } else {
            if last_mask != layer.effects.x {
                last_mask = layer.effects.x;
                mask_value = mask_with_height(layer, texel, layer_mask(layer, texel, layer.effects.y));
            }
            weight *= mask_value;
        }
        if weight <= 0.0 { continue; }
        let content = layer_content(layer, texel, layer.effects.w);
        weight *= select(content.a, 1.0, mode == 2u && content.a > 0.0);
        if weight <= 0.0 { continue; }
        let source = content.rgb;
        let destination = layer.ids.x;
        if destination == 0u {
            base += (blend_rgb(mode, base, source) - base) * weight;
        } else if destination == 7u {
            normal += (blend_rgb(mode, normal, source) - normal) * weight;
        } else if destination >= 12u {
            let slot = destination - 12u;
            let old = lobe_values[slot];
            lobe_values[slot] = lerp(old, blend_scalar(mode, old, source.x), weight);
        } else if destination >= 8u {
            let slot = destination - 8u;
            let old = lobe_colors[slot];
            lobe_colors[slot] = mix(old, blend_rgb(mode, old, source), weight);
        } else {
            let old = scalars[destination - 1u];
            scalars[destination - 1u] = lerp(old, blend_scalar(mode, old, source.x), weight);
        }
    }
    output[index] = pack_srgb(base, scalars[0]);
    let ao = clamp(display_ao * f32(to_u16(scalars[1])) / 65535.0, 0.0, 1.0);
    let roughness = select(to_u16(scalars[2]) >> 8u, 255u - (to_u16(lobe_values[8]) >> 8u), params.source.w != 0u);
    output[index + params.band.w] = u32(ao * 255.0 + 0.5)
        | (roughness << 8u)
        | ((to_u16(scalars[3]) >> 8u) << 16u)
        | ((to_u16(scalars[4]) >> 8u) << 24u);
    if writes_display(2u) {
        var detail = quantized_color(normal);
        if (baked_normal & (1u << 30u)) != 0u {
            detail = rnm(unpack_baked_normal(baked_normal), detail);
        }
        let packed = detail * 2.0 - 1.0;
        let normal_index = display_index(2u, index);
        output[normal_index] = pack2x16float(packed.xy);
        output[normal_index + 1u] = pack2x16float(vec2(packed.z, f32(to_u16(lobe_values[1])) / 65535.0));
    }
    if writes_display(3u) {
        let height = f32(to_u16(scalars[5] * 0.5 + 0.5)) * (2.0 / 65535.0) - 1.0;
        let f0 = quantized_color(lobe_colors[1]);
        let height_index = display_index(3u, index);
        output[height_index] = pack2x16float(vec2(height, f0.r));
        output[height_index + 1u] = pack2x16float(f0.gb);
    }
    if writes_display(4u) {
        output[display_index(4u, index)] = pack_srgb(lobe_colors[2], lobe_values[2]);
    }
    if writes_display(5u) {
        output[display_index(5u, index)] = (to_u16(lobe_values[3]) >> 8u)
            | ((to_u16(lobe_values[4]) >> 8u) << 8u)
            | ((to_u16(lobe_values[5]) >> 8u) << 16u)
            | ((to_u16(lobe_values[6]) >> 8u) << 24u);
    }
    if writes_display(6u) {
        output[display_index(6u, index)] = pack_srgb(lobe_colors[0], lobe_values[0]);
    }
    if writes_display(7u) {
        // The GPU compositor uses one full-resolution UV block, so tile scale is zero.
        let signed_xy = quantized_color(lobe_colors[3]).xy * 2.0 - 1.0;
        let packed = vec2<u32>(clamp(128.0 + 127.0 * signed_xy, vec2(0.0), vec2(255.0)) + 0.5);
        output[display_index(7u, index)] = packed.x | (packed.y << 8u) | ((to_u16(lobe_values[7]) >> 8u) << 16u);
    }
}
