struct Params {
    camera: mat4x4<f32>,
    eye: vec4<f32>, // world position, depth bias
    view_scale: vec4<f32>, // pixels per world unit, fade cosines, depth enabled
    mesh: vec4<f32>, // center and radius of the normalized UV surface
    ortho: vec4<f32>, // orthographic ray direction and enable
    shape: vec4<f32>, // alpha extents, roundness, contrast
    alpha_rotation: vec4<f32>, // sine and cosine of the alpha angle
    flags: vec4<u32>, // UV mode, culling, invert brush tip, quick-mask mode
    stamp_blending: u32,
}
struct Vertex {
    position: vec4<f32>,
    normal: vec4<f32>,
}
struct Dab {
    center: vec4<f32>,
    normal: vec4<f32>,
    tangent: vec4<f32>,
    bitangent: vec4<f32>,
    seed: u32,
    // A symmetry copy's way back to its source stamp (StampSource): origin, w 1 on a copy
    source: vec4<f32>,
    // The columns of StampSource::to_source
    to_source: array<vec4<f32>, 3>,
}
// Corresponds to the BrushAlpha mapping in BrushPipeline::pipeline.
override alpha_kind: u32 = 0u;
// Match texturecraft_paint::StampBlending.
const BLEND_LIGHTEN: u32 = 1u;
const BLEND_ADD: u32 = 2u;
const LIBRARY_ALPHA_KIND: u32 = 7u;
const IMAGE_ALPHA_KIND: u32 = 8u;
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(3) var uv_surface: texture_2d<u32>;
@group(0) @binding(4) var<storage, read> vertices: array<Vertex>;
@group(0) @binding(5) var alpha_in: texture_2d<u32>;
@group(0) @binding(6) var depth: texture_2d<f32>;
@group(0) @binding(7) var<storage, read> dabs: array<Dab>;
@group(0) @binding(8) var<storage, read> tiles: array<u32>;
@group(0) @binding(9) var<storage, read_write> alpha_out: array<u32>;
@group(0) @binding(10) var tip: texture_2d<f32>;

fn accept(i: u32, direction: vec3<f32>) -> bool { return true; }

fn unit(v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let l = length(v);
    return select(fallback, v / max(l, 1e-20), l > 1e-20);
}

// Visible from the conservative near depth, and uncertain where the BVH must resolve it.
fn depth_test(point: vec3<f32>) -> vec2<bool> {
    let clip = params.camera * vec4(point, 1.0);
    if clip.w <= 1e-6 || any(abs(clip.xy) > vec2(clip.w)) {
        return vec2(false);
    }
    let uv = clip.xy / clip.w * vec2(0.5, -0.5) + 0.5;
    let size = vec2<i32>(textureDimensions(depth));
    let pixel = min(vec2<i32>(uv * vec2<f32>(size)), size - 1);
    var near = 3.402823466e38;
    var far = 0.0;
    var empty = false;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let p = pixel + vec2(x, y);
            if any(p < vec2(0)) || any(p >= size) { continue; }
            let z = textureLoad(depth, p, 0).x;
            near = min(near, z);
            if z == 3.402823466e38 { empty = true; }
            else { far = max(far, z); }
        }
    }
    let distance = length(point - params.eye.xyz);
    return vec2(distance <= near + params.eye.w, empty || distance <= far + params.eye.w + far * 0.002);
}

fn ray_visible(point: vec3<f32>, geometric: vec3<f32>, direction: vec3<f32>) -> bool {
    let distance = dot(params.eye.xyz - point, direction);
    if distance <= 0.0 { return false; }
    let lift = params.eye.w * 0.0025;
    if lift >= distance { return true; }
    let side = select(geometric, -geometric, dot(geometric, direction) < 0.0);
    return traverse_from(0u, point + (side + direction) * lift, direction, distance - lift, true).idx == NONE;
}

// How far toward the eye a grazing point may be tested past its neighbours (back faces get none)
fn grazing_reach(point: vec3<f32>, facing: f32) -> f32 {
    if facing <= 0.0 { return 0.0; }
    let angle = clamp(facing, 0.05, 1.0);
    let clip_w = max(0.0, (params.camera * vec4(point, 1.0)).w);
    return clip_w * 3.0 * sqrt(1.0 - angle * angle) / (params.view_scale.x * angle);
}

fn visible(point: vec3<f32>, geometric: vec3<f32>, direction: vec3<f32>, facing: f32) -> bool {
    if params.view_scale.w == 0.0 { return true; }
    let reach = grazing_reach(point, facing);
    let test = depth_test(point + direction * reach);
    if test.x {
        return reach <= 0.0 || depth_test(point).x || ray_visible(point, geometric, direction);
    }
    return test.y && ray_visible(point, geometric, direction);
}

fn falloff(radius: f32, hardness: f32) -> f32 {
    return 1.0 - smoothstep(min(hardness, 0.999), 1.0, radius);
}

fn procedural_alpha(p: vec2<f32>, hardness: f32, seed: u32) -> f32 {
    let radius = length(p);
    switch alpha_kind {
        case 0u: { return falloff(mix(max(abs(p.x), abs(p.y)), radius, params.shape.z), hardness); }
        case 1u: { return falloff(max(abs(p.x), abs(p.y)), hardness); }
        case 5u: {
            if radius == 0.0 { return 1.0; }
            let k = abs(cos(atan2(p.y, p.x) * 5.0 / 2.0));
            return falloff(radius / (0.45 + 0.55 * k * k), hardness);
        }
        default: {}
    }

    let fall = falloff(radius, hardness);
    if fall == 0.0 { return 0.0; }
    let seed_position = f32(seed);
    switch alpha_kind {
        case 2u: {
            let n = noise(vec3(p * 14.0, seed_position * 0.37), seed, false);
            return select(0.0, fall, n > 0.72);
        }
        case 3u: {
            let n = fbm(vec3(p.x * 2.5 + seed_position * 0.13, p.y * 2.5, 0.5), 5u, seed, true);
            return fall * clamp((n - 0.42) * 5.0, 0.0, 1.0);
        }
        case 4u: {
            return fall * fbm(vec3(p * 1.8, seed_position * 0.21), 4u, seed, false);
        }
        case 6u: {
            let n = noise(vec3(p.x * 0.8, p.y * 18.0, seed_position * 0.7), seed, false);
            return fall * clamp((n - 0.35) * 3.0, 0.0, 1.0);
        }
        default: { return fall; }
    }
}

fn tip_texel(p: vec2<i32>, side: i32, mip: u32) -> f32 {
    if any(p < vec2(0)) || any(p >= vec2(side)) { return 0.0; }
    // The GLSL backend needs an i32 level here.
    return textureLoad(tip, p, i32(mip)).x;
}

fn library_alpha(local: vec2<f32>, radius: f32) -> f32 {
    let base = f32(textureDimensions(tip, 0).x);
    let diameter = radius * 2.0 * min(params.shape.x, params.shape.y);
    var mip = 0u;
    if diameter <= base {
        let requested = floor(log2(base / max(diameter, 1.0)));
        mip = u32(clamp(requested, 0.0, f32(textureNumLevels(tip) - 1u)));
    }
    let side = i32(textureDimensions(tip, mip).x);
    let p = (local * vec2(0.5, -0.5) + 0.5) * f32(side) - 0.5;
    if any(p <= vec2(-1.0)) || any(p >= vec2(f32(side))) { return 0.0; }
    // The weights from the same floor as the texels: llvmpipe may fuse the multiply-add for one of
    // floor and fract and round it for the other, which reads the next texel at full weight
    let cell = floor(p);
    let corner = vec2<i32>(cell);
    let t = clamp(p - cell, vec2(0.0), vec2(1.0));
    let top = mix(tip_texel(corner, side, mip), tip_texel(corner + vec2(1, 0), side, mip), t.x);
    let bottom = mix(tip_texel(corner + vec2(0, 1), side, mip), tip_texel(corner + vec2(1, 1), side, mip), t.x);
    return mix(top, bottom, t.y);
}

fn wrapped_tip_texel(pixel: vec2<i32>, size: vec2<i32>) -> vec4<f32> {
    return textureLoad(tip, (pixel + size) % size, 0);
}

fn image_alpha(local: vec2<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(tip, 0));
    let aspect = f32(size.x) / f32(size.y);
    let p = local * vec2(max(1.0 / aspect, 1.0), max(aspect, 1.0));
    if !all(abs(p) <= vec2(1.0)) { return 0.0; }
    let grid = (p * vec2(0.5, -0.5) + 0.5) * vec2<f32>(size) - 0.5;
    // The weights from the same floor as the texels (library_alpha)
    let cell = floor(grid);
    let at = vec2<i32>(cell);
    let t = clamp(grid - cell, vec2(0.0), vec2(1.0));
    let top = mix(wrapped_tip_texel(at, size), wrapped_tip_texel(at + vec2(1, 0), size), t.x);
    let bottom = mix(wrapped_tip_texel(at + vec2(0, 1), size), wrapped_tip_texel(at + vec2(1, 1), size), t.x);
    let rgba = mix(top, bottom, t.y);
    return clamp(dot(rgba.rgb, vec3(0.2126, 0.7152, 0.0722)) * rgba.a, 0.0, 1.0);
}

fn stamp_shape(local: vec2<f32>, hardness: f32, seed: u32, radius: f32) -> f32 {
    let sin_angle = params.alpha_rotation.x;
    let cos_angle = params.alpha_rotation.y;
    let rotated = vec2(local.x * cos_angle + local.y * sin_angle, -local.x * sin_angle + local.y * cos_angle);
    let tip_point = rotated / params.shape.xy;
    var value = 0.0;
    if alpha_kind == LIBRARY_ALPHA_KIND {
        value = library_alpha(tip_point, radius);
    } else if alpha_kind == IMAGE_ALPHA_KIND {
        value = image_alpha(tip_point);
    } else {
        value = procedural_alpha(tip_point, hardness, seed);
    }
    let graded = clamp((value - 0.5) * params.shape.w + 0.5, 0.0, 1.0);
    if params.flags.z == 0u { return graded; }
    return select(0.0, 1.0 - graded, all(abs(local) <= vec2(1.0)));
}

fn edge(a: vec2<f32>, b: vec2<f32>, p: vec2<f32>) -> f32 {
    let ab = b - a;
    let ap = p - a;
    return ab.x * ap.y - ab.y * ap.x;
}

struct BrushView {
    geometric: vec3<f32>,
    direction: vec3<f32>,
    facing: f32,
}

fn brush_view(point: vec3<f32>, geometric: vec3<f32>, normal: vec3<f32>) -> BrushView {
    let direction = select(unit(params.eye.xyz - point, normal), params.ortho.xyz, params.ortho.w != 0.0);
    return BrushView(select(normal, geometric, dot(geometric, geometric) > 0.0), direction, dot(normal, direction));
}

fn source_direction(dab: Dab, direction: vec3<f32>) -> vec3<f32> {
    return mat3x3<f32>(dab.to_source[0].xyz, dab.to_source[1].xyz, dab.to_source[2].xyz) * direction;
}

fn source_point(dab: Dab, point: vec3<f32>) -> vec3<f32> {
    return dab.source.xyz + source_direction(dab, point - dab.source.xyz);
}

// A symmetry copy paints where its source stamp would, so it sees the texel from the source's side
fn copy_view(dab: Dab, point: vec3<f32>, geometric: vec3<f32>, normal: vec3<f32>) -> BrushView {
    return brush_view(source_point(dab, point), source_direction(dab, geometric), source_direction(dab, normal));
}

// The raster's surface_visible for a copy: the source's point may lie up to the stamp depth off an asymmetric surface
fn copy_visible(dab: Dab, point: vec3<f32>, view: BrushView) -> bool {
    if params.view_scale.w == 0.0 { return true; }
    let seen = source_point(dab, point);
    let allowed = seen + view.direction * (dab.center.w * dab.normal.w);
    let reach = grazing_reach(seen, view.facing);
    let test = depth_test(allowed + view.direction * reach);
    if test.x {
        return reach <= 0.0 || depth_test(allowed).x || ray_visible(allowed, view.geometric, view.direction);
    }
    return test.y && ray_visible(allowed, view.geometric, view.direction);
}

fn quick_limit(pixel: vec2<u32>, offset: u32) -> f32 {
    if params.flags.w == 0u { return 1.0; }
    var coverage = 0.0;
    if offset != 0u {
        let local = pixel % 64u;
        let index = local.y * 64u + local.x;
        coverage = f32((tiles[offset + index / 2u] >> (index % 2u * 16u)) & 65535u) / 65535.0;
    }
    return select(coverage, 1.0 - coverage, params.flags.w == 2u);
}

fn stroke_alpha(pixel: vec2<u32>, first: u32, count: u32, mask: u32) -> u32 {
    var coverage = textureLoad(alpha_in, vec2<i32>(pixel), 0).x;
    let limit = quick_limit(pixel, mask);
    if limit == 0.0 { return coverage; }
    let hit = textureLoad(uv_surface, vec2<i32>(pixel), 0).xy;
    if hit.x == NONE { return coverage; }

    let uv_mode = params.flags.x != 0u;
    var point = vec3(0.0);
    var normal = vec3(0.0);
    var geometric = vec3(0.0);
    var back_facing = false;
    var view = BrushView(vec3(0.0), vec3(0.0), 1.0);
    var has_normal = false;
    var texels_per_unit = 1.0;
    if !uv_mode {
        let xy = unpack2x16unorm(hit.y);
        var weights = vec3(max(0.0, 1.0 - xy.x - xy.y), xy);
        let v = hit.x * 3u;
        let a = vertices[v];
        let b = vertices[v + 1u];
        let c = vertices[v + 2u];
        var interpolated = a.normal.xyz * weights.x + b.normal.xyz * weights.y + c.normal.xyz * weights.z;
        // Packed barycentrics can leave a residual normal where the vertex normals cancel.
        // Recover the full weights near cancellation before deciding whether a dab normal is needed.
        let packed_error = 4.0 / 65535.0;
        if dot(interpolated, interpolated) <= packed_error * packed_error {
            let size = vec2<f32>(textureDimensions(uv_surface));
            let uv_a = vec2(a.position.w, a.normal.w) * size;
            let uv_b = vec2(b.position.w, b.normal.w) * size;
            let uv_c = vec2(c.position.w, c.normal.w) * size;
            let center = vec2<f32>(pixel) + 0.5;
            let area = edge(uv_a, uv_b, uv_c);
            if abs(area) >= 1e-12 {
                let full = vec2(edge(uv_c, uv_a, center), edge(uv_a, uv_b, center)) * (1.0 / area);
                weights = vec3(1.0 - full.x - full.y, full);
                interpolated = a.normal.xyz * weights.x + b.normal.xyz * weights.y + c.normal.xyz * weights.z;
            }
        }
        point = (a.position.xyz * weights.x + b.position.xyz * weights.y + c.position.xyz * weights.z) * params.mesh.w + params.mesh.xyz;
        normal = unit(interpolated, vec3(0.0));
        let world_cross = cross(b.position.xyz - a.position.xyz, c.position.xyz - a.position.xyz);
        geometric = unit(world_cross, vec3(0.0));
        if alpha_kind == LIBRARY_ALPHA_KIND {
            let size = vec2<f32>(textureDimensions(uv_surface));
            let uv_a = vec2(a.position.w, a.normal.w);
            let uv_b = vec2(b.position.w, b.normal.w);
            let uv_c = vec2(c.position.w, c.normal.w);
            let area = abs(edge(uv_a, uv_b, uv_c)) * size.x * size.y;
            let world_area = length(world_cross) * params.mesh.w * params.mesh.w;
            texels_per_unit = sqrt(area / world_area);
        }
        has_normal = dot(normal, normal) > 0.0;
        if has_normal {
            view = brush_view(point, geometric, normal);
            // Symmetry copies see the texel from their source's side
            back_facing = params.flags.y != 0u && view.facing <= 0.0;
        }
    }
    var tested_visibility = false;
    var hidden = false;
    for (var i = 0u; i < count; i++) {
        let dab = dabs[tiles[first + i]];
        var local = vec2(0.0);
        var fade = 1.0;
        var dab_view = view;
        let symmetry_copy = !uv_mode && dab.source.w != 0.0;
        if uv_mode {
            let reach = dab.center.w * 1.42;
            let p = vec2<f32>(pixel);
            if any(p < floor(dab.center.xy - reach)) || any(p >= ceil(dab.center.xy + reach)) {
                continue;
            }
            let d = (p + 0.5 - dab.center.xy) * vec2(1.0, -1.0) / dab.center.w;
            local = vec2(dot(d, dab.tangent.xy), dot(d, vec2(-dab.tangent.y, dab.tangent.x)));
        } else {
            let offset = point - dab.center.xyz;
            local = vec2(dot(offset, dab.tangent.xyz), dot(offset, dab.bitangent.xyz)) / dab.center.w;
            if dot(local, local) > 2.0 || abs(dot(offset, dab.normal.xyz)) > dab.center.w * dab.normal.w {
                continue;
            }
            if has_normal && dot(normal, dab.normal.xyz) <= 0.0 { continue; }
            if symmetry_copy {
                dab_view = copy_view(dab, point, geometric, select(dab.normal.xyz, normal, has_normal));
                if params.flags.y != 0u && dab_view.facing <= 0.0 { continue; }
            } else if has_normal {
                if back_facing || hidden { continue; }
            } else {
                dab_view = brush_view(point, geometric, dab.normal.xyz);
                if params.flags.y != 0u && dab_view.facing <= 0.0 { continue; }
            }
            if params.flags.y != 0u {
                let f = clamp((dab_view.facing - params.view_scale.y) / max(1e-6, params.view_scale.z - params.view_scale.y), 0.0, 1.0);
                fade = f * f * (3.0 - 2.0 * f);
            }
        }
        let opacity = stamp_shape(local, dab.bitangent.w, dab.seed, dab.center.w * texels_per_unit) * dab.tangent.w * fade * limit;
        if opacity <= 0.0 { continue; }
        if symmetry_copy {
            if !copy_visible(dab, point, dab_view) { continue; }
        } else if !uv_mode && !tested_visibility {
            if !visible(point, dab_view.geometric, dab_view.direction, dab_view.facing) {
                hidden = has_normal;
                continue;
            }
            tested_visibility = has_normal;
        }
        let old = f32(coverage) / 65535.0;
        var blended = opacity + old * (1.0 - opacity);
        switch params.stamp_blending {
            case BLEND_LIGHTEN: { blended = max(old, opacity); }
            case BLEND_ADD: { blended = old + opacity; }
            default: {}
        }
        coverage = u32(clamp(blended, 0.0, 1.0) * 65535.0 + 0.5);
    }
    return coverage;
}

@compute @workgroup_size(8, 8)
fn brush(@builtin(global_invocation_id) id: vec3<u32>) {
    let header = id.z * 5u;
    let origin = vec2(tiles[header], tiles[header + 1u]) + vec2(id.x * 2u, id.y);
    let first = tiles[header + 2u];
    let count = tiles[header + 3u];
    let mask = tiles[header + 4u];
    let a = stroke_alpha(origin, first, count, mask);
    let b = stroke_alpha(origin + vec2(1u, 0u), first, count, mask);
    alpha_out[id.z * 4096u + id.y * 64u + id.x] = a | (b << 16u);
}
