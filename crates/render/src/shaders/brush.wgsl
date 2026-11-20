struct Params {
    camera: mat4x4<f32>,
    eye: vec4<f32>, // world position, depth bias
    view_scale: vec4<f32>, // pixels per world unit, fade cosines, depth enabled
    mesh: vec4<f32>, // center and radius of the normalized UV surface
    ortho: vec4<f32>, // orthographic ray direction and enable
    shape: vec4<f32>, // alpha extents, roundness, contrast
    flags: vec4<u32>, // UV mode, culling, invert
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
}
// Corresponds to the BrushAlpha mapping in BrushPipeline::pipeline.
override alpha_kind: u32 = 0u;
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(3) var uv_surface: texture_2d<u32>;
@group(0) @binding(4) var<storage, read> vertices: array<Vertex>;
@group(0) @binding(5) var alpha_in: texture_2d<u32>;
@group(0) @binding(6) var depth: texture_2d<f32>;
@group(0) @binding(7) var<storage, read> dabs: array<Dab>;
@group(0) @binding(8) var<storage, read> tiles: array<u32>;
@group(0) @binding(9) var<storage, read_write> alpha_out: array<u32>;

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

fn visible(point: vec3<f32>, geometric: vec3<f32>, direction: vec3<f32>, facing: f32) -> bool {
    if params.view_scale.w == 0.0 { return true; }
    var reach = 0.0;
    if facing > 0.0 {
        let angle = clamp(facing, 0.05, 1.0);
        let clip_w = max(0.0, (params.camera * vec4(point, 1.0)).w);
        reach = clip_w * 3.0 * sqrt(1.0 - angle * angle) / (params.view_scale.x * angle);
    }
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

fn stamp_shape(local: vec2<f32>, hardness: f32, seed: u32) -> f32 {
    let value = procedural_alpha(local / params.shape.xy, hardness, seed);
    let graded = clamp((value - 0.5) * params.shape.w + 0.5, 0.0, 1.0);
    if params.flags.z == 0u { return graded; }
    return select(0.0, 1.0 - graded, all(abs(local) <= vec2(1.0)));
}

fn stroke_alpha(pixel: vec2<u32>, first: u32, count: u32) -> u32 {
    var coverage = textureLoad(alpha_in, vec2<i32>(pixel), 0).x;
    let hit = textureLoad(uv_surface, vec2<i32>(pixel), 0).xy;
    if hit.x == NONE { return coverage; }

    let uv_mode = params.flags.x != 0u;
    var point = vec3(0.0);
    var normal = vec3(0.0, 1.0, 0.0);
    var geometric = normal;
    var direction = normal;
    var facing = 1.0;
    if !uv_mode {
        let xy = unpack2x16unorm(hit.y);
        let z = max(0.0, 1.0 - xy.x - xy.y);
        let v = hit.x * 3u;
        let a = vertices[v];
        let b = vertices[v + 1u];
        let c = vertices[v + 2u];
        point = (a.position.xyz * z + b.position.xyz * xy.x + c.position.xyz * xy.y) * params.mesh.w + params.mesh.xyz;
        normal = unit(a.normal.xyz * z + b.normal.xyz * xy.x + c.normal.xyz * xy.y, normal);
        geometric = unit(cross(b.position.xyz - a.position.xyz, c.position.xyz - a.position.xyz), normal);
        direction = select(unit(params.eye.xyz - point, normal), params.ortho.xyz, params.ortho.w != 0.0);
        facing = dot(normal, direction);
        if params.flags.y != 0u && facing <= 0.0 { return coverage; }
    }
    var tested_visibility = false;
    for (var i = 0u; i < count; i++) {
        let dab = dabs[tiles[first + i]];
        var local = vec2(0.0);
        var fade = 1.0;
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
            if dot(local, local) > 2.0 || abs(dot(offset, dab.normal.xyz)) > dab.center.w * dab.normal.w || dot(normal, dab.normal.xyz) <= 0.0 {
                continue;
            }
            if params.flags.y != 0u {
                let f = clamp((facing - params.view_scale.y) / max(1e-6, params.view_scale.z - params.view_scale.y), 0.0, 1.0);
                fade = f * f * (3.0 - 2.0 * f);
            }
        }
        let opacity = stamp_shape(local, dab.bitangent.w, dab.seed) * dab.tangent.w * fade;
        if opacity <= 0.0 { continue; }
        if !uv_mode && !tested_visibility {
            if !visible(point, geometric, direction, facing) { return coverage; }
            tested_visibility = true;
        }
        let old = f32(coverage) / 65535.0;
        coverage = u32(clamp(opacity + old * (1.0 - opacity), 0.0, 1.0) * 65535.0 + 0.5);
    }
    return coverage;
}

@compute @workgroup_size(8, 8)
fn brush(@builtin(global_invocation_id) id: vec3<u32>) {
    let header = id.z * 4u;
    let origin = vec2(tiles[header], tiles[header + 1u]) + vec2(id.x * 2u, id.y);
    let first = tiles[header + 2u];
    let count = tiles[header + 3u];
    let a = stroke_alpha(origin, first, count);
    let b = stroke_alpha(origin + vec2(1u, 0u), first, count);
    alpha_out[id.z * 4096u + id.y * 64u + id.x] = a | (b << 16u);
}
