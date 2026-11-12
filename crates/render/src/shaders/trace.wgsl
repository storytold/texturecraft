// GPU path tracing: one compute invocation per pixel adds `spp` samples to the accumulation
// buffers. A line-by-line port of `trace.rs` (the CPU tracer is the oracle): the same random
// streams, thin-lens camera, BVH traversal, material conventions, lobes, transmission,
// next-event estimation with MIS, subsurface random walk, Russian roulette and firefly clamp.

const PI: f32 = 3.14159265358979;
const TAU: f32 = 6.28318530717959;
const BIG: f32 = 3.0e38;
const WALK_STEPS: u32 = 48u;
const MAX_SETS: u32 = 256u;

struct Params {
    inv_vp: mat4x4<f32>,
    forward: vec4<f32>, // xyz camera forward, w ray offset (eps)
    size: vec4<u32>,    // width, height, first sample index, samples this dispatch
    cfg: vec4<u32>,     // bounces, specular/glossiness, environment light sampling, texture sets
    env: vec4<f32>,     // sin and cos of the environment rotation
    sigma: vec4<f32>,   // subsurface extinction per color channel
    eye: vec4<f32>,     // camera position
    lens: vec4<f32>,    // focus depth along the view axis, lens radius (0: pinhole), transmission IOR
    // Per texture set (array layer): width, height, bit mask of 1×1 placeholder kinds, height strength.
    sets: array<vec4<f32>, MAX_SETS>,
};

@group(0) @binding(0) var<uniform> P: Params;
// nodes (binding 1) and tris (binding 2) are in rays.wgsl; here a triangle's w words are
// (texture layer, tangent sign, 0) and its last row (vertex indices, original index).
// Per vertex: (normal, u), (tangent, v).
@group(0) @binding(3) var<storage, read> verts: array<vec4<f32>>;
// Environment importance sampling: row CDF (h), column CDFs (w·h), pixel probabilities (w·h).
@group(0) @binding(4) var<storage, read> env_cdf: array<f32>;
@group(0) @binding(5) var<storage, read_write> acc_hit: array<vec4<f32>>; // radiance, luminance²
@group(0) @binding(6) var<storage, read_write> acc_alb: array<vec4<f32>>; // first-hit albedo, hits
@group(0) @binding(7) var<storage, read_write> acc_nrm: array<vec4<f32>>; // shading normal, depth
@group(0) @binding(8) var<storage, read_write> acc_env: array<vec4<f32>>; // environment of misses
@group(0) @binding(9) var env_tex: texture_2d<f32>;
// The renderer's display textures (compose::display packing), one layer per texture set.
@group(0) @binding(10) var t_base: texture_2d_array<f32>;
@group(0) @binding(11) var t_orm: texture_2d_array<f32>;
@group(0) @binding(12) var t_normal: texture_2d_array<f32>;
@group(0) @binding(13) var t_height: texture_2d_array<f32>;
@group(0) @binding(14) var t_emissive: texture_2d_array<f32>;
@group(0) @binding(15) var t_lobes: texture_2d_array<f32>;
@group(0) @binding(16) var t_sheen: texture_2d_array<f32>;
@group(0) @binding(17) var t_extra: texture_2d_array<f32>;

var<private> rng: u32;

fn pcg3d(a: vec3<u32>) -> vec3<u32> {
    var v = a * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v = v ^ (v >> vec3<u32>(16u));
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return v;
}

// xorshift32, as `Rng::next`.
fn rnd() -> f32 {
    rng ^= rng << 13u;
    rng ^= rng >> 17u;
    rng ^= rng << 5u;
    return f32(rng >> 8u) / 16777216.0;
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn finite(x: f32) -> bool {
    return (bitcast<u32>(x) & 0x7f800000u) != 0x7f800000u;
}

fn finite3(c: vec3<f32>) -> bool {
    return finite(c.x) && finite(c.y) && finite(c.z);
}

fn normalize_or(v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let l2 = dot(v, v);
    if (l2 > 1e-30 && finite(l2)) {
        return v * inverseSqrt(l2);
    }
    return fallback;
}

// ---- Environment ----

fn dir_of(u: f32, v: f32) -> vec3<f32> {
    let theta = v * PI;
    let phi = u * TAU;
    return vec3<f32>(sin(theta) * cos(phi), cos(theta), sin(theta) * sin(phi));
}

// atan2 for the elevation too: GPU acos is too coarse near the poles of a texel row.
fn uv_of(d: vec3<f32>) -> vec2<f32> {
    let u = atan2(d.z, d.x) / TAU;
    return vec2<f32>(u - floor(u), atan2(length(d.xz), d.y) / PI);
}

fn to_env(d: vec3<f32>) -> vec3<f32> {
    let s = P.env.x;
    let c = P.env.y;
    return vec3<f32>(c * d.x - s * d.z, d.y, s * d.x + c * d.z);
}

fn env_texel(x: i32, y: i32) -> vec3<f32> {
    let dims = vec2<i32>(textureDimensions(env_tex));
    let xx = ((x % dims.x) + dims.x) % dims.x;
    let yy = clamp(y, 0, dims.y - 1);
    return textureLoad(env_tex, vec2<i32>(xx, yy), 0).rgb;
}

// Bilinear lookup in environment space (`EnvImage::sample`).
fn env_local(d: vec3<f32>) -> vec3<f32> {
    let dims = vec2<f32>(textureDimensions(env_tex));
    let uv = uv_of(d);
    let f = uv * dims - 0.5;
    let x0 = floor(f.x);
    let y0 = floor(f.y);
    let t = f - vec2<f32>(x0, y0);
    let xi = i32(x0);
    let yi = i32(y0);
    let a = mix(env_texel(xi, yi), env_texel(xi + 1, yi), t.x);
    let b = mix(env_texel(xi, yi + 1), env_texel(xi + 1, yi + 1), t.x);
    return mix(a, b, t.y);
}

fn env(d: vec3<f32>) -> vec3<f32> {
    return env_local(to_env(d));
}

fn env_pdf_local(d: vec3<f32>) -> f32 {
    let dims = textureDimensions(env_tex);
    let uv = uv_of(d);
    return texel_pdf(min(u32(uv.x * f32(dims.x)), dims.x - 1u), min(u32(uv.y * f32(dims.y)), dims.y - 1u));
}

// Solid-angle density of sampling a direction in environment texel (x, y).
fn texel_pdf(x: u32, y: u32) -> f32 {
    let dims = textureDimensions(env_tex);
    let w = dims.x;
    let h = dims.y;
    let s = sin(PI * (f32(y) + 0.5) / f32(h));
    let p = env_cdf[h + w * h + y * w + x];
    if (s <= 1e-6) {
        return 0.0;
    }
    return p * f32(w * h) / (2.0 * PI * PI * s);
}

fn env_pdf(d: vec3<f32>) -> f32 {
    return env_pdf_local(to_env(d));
}

// First index in env_cdf[start, start + n) whose value is >= goal (`partition_point`).
fn lower_bound(start: u32, n: u32, goal: f32) -> u32 {
    var lo = 0u;
    var hi = n;
    loop {
        if (lo >= hi) {
            break;
        }
        let mid = (lo + hi) / 2u;
        if (env_cdf[start + mid] < goal) {
            lo = mid + 1u;
        } else {
            hi = mid;
        }
    }
    return lo;
}

// A world direction toward the environment chosen by brightness, and its density (0: none).
fn sample_env() -> vec4<f32> {
    let u0 = rnd();
    let u1 = rnd();
    let u2 = rnd();
    let u3 = rnd();
    let dims = textureDimensions(env_tex);
    let w = dims.x;
    let h = dims.y;
    let y = min(lower_bound(0u, h, u0 * env_cdf[h - 1u]), h - 1u);
    let row = h + y * w;
    let x = min(lower_bound(row, w, u1 * env_cdf[row + w - 1u]), w - 1u);
    let e = dir_of((f32(x) + u2) / f32(w), (f32(y) + u3) / f32(h));
    // The texel's own density (round-tripping the direction through atan2 could land next door).
    let pdf = texel_pdf(x, y);
    let s = P.env.x;
    let c = P.env.y;
    return vec4<f32>(c * e.x + s * e.z, e.y, -s * e.x + c * e.z, pdf);
}

// ---- Ray casts ----

fn accept(i: u32, d: vec3<f32>) -> bool {
    return true;
}

fn traverse(o: vec3<f32>, d: vec3<f32>, tmax: f32, any_hit: bool) -> Hit {
    return traverse_from(0u, o, d, tmax, any_hit);
}

fn occluded(o: vec3<f32>, d: vec3<f32>) -> bool {
    return traverse(o, d, BIG, true).idx != NONE;
}

// ---- Materials ----

// Texel of `uv` (nearest, wrapped) in kind `k` of layer `layer`, offset by (dx, dy).
fn texel(layer: u32, k: u32, uv: vec2<f32>, dx: i32, dy: i32) -> vec2<i32> {
    let info = P.sets[layer];
    let small = ((u32(info.z) >> k) & 1u) == 1u;
    let dims = select(max(vec2<i32>(info.xy), vec2<i32>(1)), vec2<i32>(1), small);
    let x = i32(floor(uv.x * f32(dims.x))) + dx;
    let y = i32(floor(uv.y * f32(dims.y))) + dy;
    return vec2<i32>(((x % dims.x) + dims.x) % dims.x, ((y % dims.y) + dims.y) % dims.y);
}

struct Surface {
    pos: vec3<f32>,
    geo: vec3<f32>,
    n: vec3<f32>,
    tangent: vec3<f32>,
    diffuse: vec3<f32>,
    f0: vec3<f32>,
    roughness: f32,
    emissive: vec3<f32>,
    opacity: f32,
    anisotropy: f32,
    coat: f32,
    coat_roughness: f32,
    coat_n: vec3<f32>,
    sheen: vec3<f32>,
    sheen_roughness: f32,
    scatter: f32,
    // Transmissive part of the base, its tint, and whether the ray enters the mesh here.
    transmission: f32,
    base: vec3<f32>,
    front: bool,
};

fn vertex_normal(i: u32, fallback: vec3<f32>) -> vec3<f32> {
    let n = verts[i * 2u].xyz;
    return select(n, fallback, dot(n, n) == 0.0);
}

// Tangent-space normal blending (reoriented normal mapping).
fn rnm(base: vec3<f32>, detail: vec3<f32>) -> vec3<f32> {
    let t = base + vec3<f32>(0.0, 0.0, 1.0);
    let u = detail * vec3<f32>(-1.0, -1.0, 1.0);
    return normalize_or(t * dot(t, u) - u * t.z, vec3<f32>(0.0, 0.0, 1.0));
}

fn surface(hit: Hit, o: vec3<f32>, d: vec3<f32>) -> Surface {
    let p0 = tri_p(hit.idx, 0u);
    let p1 = tri_p(hit.idx, 1u);
    let p2 = tri_p(hit.idx, 2u);
    let ix = tris[hit.idx * 4u + 3u].xyz;
    let layer_in = tris[hit.idx * 4u].w;
    let layer = select(P.cfg.w, layer_in, layer_in < P.cfg.w);
    let tsign = select(1.0, -1.0, bitcast<f32>(tris[hit.idx * 4u + 1u].w) < 0.0);
    let u = hit.u;
    let v = hit.v;
    let w0 = 1.0 - u - v;
    var geo = normalize_or(cross(p1 - p0, p2 - p0), vec3<f32>(0.0, 1.0, 0.0));
    let nrm = vertex_normal(ix.x, geo) * w0 + vertex_normal(ix.y, geo) * u + vertex_normal(ix.z, geo) * v;
    var n_geo = normalize_or(nrm, geo);
    let front = dot(geo, d) <= 0.0;
    if (!front) {
        geo = -geo;
    }
    if (dot(n_geo, geo) < 0.0) {
        n_geo = -n_geo;
    }
    let a0 = verts[ix.x * 2u + 1u];
    let a1 = verts[ix.y * 2u + 1u];
    let a2 = verts[ix.z * 2u + 1u];
    let uv = vec2<f32>(verts[ix.x * 2u].w, a0.w) * w0 + vec2<f32>(verts[ix.y * 2u].w, a1.w) * u + vec2<f32>(verts[ix.z * 2u].w, a2.w) * v;
    let tw = a0.xyz * w0 + a1.xyz * u + a2.xyz * v;

    let base_t = textureLoad(t_base, texel(layer, 0u, uv, 0, 0), layer, 0);
    let orm = textureLoad(t_orm, texel(layer, 1u, uv, 0, 0), layer, 0);
    let nrm_t = textureLoad(t_normal, texel(layer, 2u, uv, 0, 0), layer, 0);
    let hgt = textureLoad(t_height, texel(layer, 3u, uv, 0, 0), layer, 0);
    let emi = textureLoad(t_emissive, texel(layer, 4u, uv, 0, 0), layer, 0);
    let lobes = textureLoad(t_lobes, texel(layer, 5u, uv, 0, 0), layer, 0);
    let sheen_t = textureLoad(t_sheen, texel(layer, 6u, uv, 0, 0), layer, 0);
    let extra = textureLoad(t_extra, texel(layer, 7u, uv, 0, 0), layer, 0);

    let info = P.sets[layer];
    // A UV tile smaller than its atlas block: Extra alpha holds 32 × log2(1 / its share), as in mesh.wgsl.
    let k = info.w * info.x / 16.0 * exp2(-extra.a * 255.0 / 32.0);
    var n_height = vec3<f32>(0.0, 0.0, 1.0);
    if (k != 0.0) {
        let hx = textureLoad(t_height, texel(layer, 3u, uv, 1, 0), layer, 0).r - textureLoad(t_height, texel(layer, 3u, uv, -1, 0), layer, 0).r;
        let hy = textureLoad(t_height, texel(layer, 3u, uv, 0, 1), layer, 0).r - textureLoad(t_height, texel(layer, 3u, uv, 0, -1), layer, 0).r;
        n_height = normalize_or(vec3<f32>(-hx * 0.5 * k, hy * 0.5 * k, 1.0), vec3<f32>(0.0, 0.0, 1.0));
    }
    // The signed normal in half floats, as in mesh.wgsl (flat is exactly +z)
    let detail = normalize_or(clamp(nrm_t.rgb, vec3<f32>(-1.0), vec3<f32>(1.0)), vec3<f32>(0.0, 0.0, 1.0));
    let n_ts = rnm(n_height, detail);
    let tt = normalize_or(tw - n_geo * dot(n_geo, tw), onb_t(n_geo));
    let b = cross(n_geo, tt) * tsign;
    var n = normalize_or(tt * n_ts.x + b * n_ts.y + n_geo * n_ts.z, n_geo);
    if (dot(n, -d) <= 0.0) {
        n = n_geo;
    }
    let angle = clamp(lobes.a, 0.0, 1.0) * PI;
    let turned = tt * cos(angle) + b * sin(angle);
    let tangent = normalize_or(turned - n * dot(n, turned), onb_t(n));

    let base = base_t.rgb;
    var diffuse: vec3<f32>;
    var f0: vec3<f32>;
    var metallic = 0.0;
    if (P.cfg.y != 0u) {
        f0 = clamp(hgt.gba, vec3<f32>(0.0), vec3<f32>(1.0));
        diffuse = base * (1.0 - max3(f0));
    } else {
        metallic = clamp(orm.b, 0.0, 1.0);
        let level = clamp(orm.a, 0.0, 1.0);
        diffuse = base * (1.0 - metallic);
        f0 = mix(vec3<f32>(0.08 * level), base, metallic);
    }
    // The coat normal packs x and y as 128 + 127·n (flat: exactly 0); z is rebuilt.
    let coat_base = select(geo, n_geo, dot(n_geo, -d) > 0.0);
    let cxy = (extra.xy * 255.0 - 128.0) / 127.0;
    let c = normalize_or(vec3<f32>(cxy, sqrt(max(1.0 - dot(cxy, cxy), 0.0))), vec3<f32>(0.0, 0.0, 1.0));
    let coat_turned = normalize_or(tt * c.x + b * c.y + coat_base * c.z, coat_base);
    var s: Surface;
    s.pos = o + d * hit.t;
    s.geo = geo;
    s.n = n;
    s.tangent = tangent;
    s.diffuse = diffuse;
    s.f0 = f0;
    s.roughness = clamp(orm.g, 0.045, 1.0);
    s.emissive = emi.rgb;
    s.opacity = clamp(base_t.a, 0.0, 1.0);
    s.anisotropy = clamp(lobes.b, 0.0, 1.0);
    s.coat = clamp(lobes.r, 0.0, 1.0);
    s.coat_roughness = max(clamp(lobes.g, 0.0, 1.0), 0.045);
    s.coat_n = select(coat_base, coat_turned, dot(coat_turned, -d) > 0.0);
    s.sheen = clamp(sheen_t.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * clamp(sheen_t.a, 0.0, 1.0);
    s.sheen_roughness = max(clamp(nrm_t.a, 0.0, 1.0), 0.07);
    s.scatter = clamp(emi.a, 0.0, 1.0);
    s.transmission = clamp(extra.b, 0.0, 1.0) * (1.0 - metallic);
    s.base = clamp(base, vec3<f32>(0.0), vec3<f32>(1.0));
    s.front = front;
    return s;
}

// ---- BSDF ----

fn schlick(f0: vec3<f32>, cos_t: f32) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - clamp(cos_t, 0.0, 1.0), 5.0);
}

fn coat_fresnel(cos_t: f32) -> f32 {
    return schlick(vec3<f32>(0.04), cos_t).x;
}

fn cosine_local(u1: f32, u2: f32) -> vec3<f32> {
    let r = sqrt(u1);
    let phi = TAU * u2;
    return vec3<f32>(r * cos(phi), r * sin(phi), sqrt(max(1.0 - u1, 0.0)));
}

fn cosine_around(n: vec3<f32>) -> vec3<f32> {
    let u1 = rnd();
    let u2 = rnd();
    let l = cosine_local(u1, u2);
    return onb_t(n) * l.x + onb_b(n) * l.y + n * l.z;
}

fn smith_g1(w: vec3<f32>, at: f32, ab: f32) -> f32 {
    let z = max(w.z, 1e-6);
    return 2.0 * z / (z + sqrt(at * at * w.x * w.x + ab * ab * w.y * w.y + z * z));
}

fn sample_vndf(ve: vec3<f32>, at: f32, ab: f32, u1: f32, u2: f32) -> vec3<f32> {
    let vh = normalize_or(vec3<f32>(at * ve.x, ab * ve.y, ve.z), vec3<f32>(0.0, 0.0, 1.0));
    let len2 = vh.x * vh.x + vh.y * vh.y;
    let t1 = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-vh.y, vh.x, 0.0) / sqrt(len2), len2 > 0.0);
    let t2 = cross(vh, t1);
    let r = sqrt(u1);
    let phi = TAU * u2;
    let p1 = r * cos(phi);
    let s = 0.5 * (1.0 + vh.z);
    let p2 = (1.0 - s) * sqrt(max(1.0 - p1 * p1, 0.0)) + s * r * sin(phi);
    let nh = t1 * p1 + t2 * p2 + vh * sqrt(max(1.0 - p1 * p1 - p2 * p2, 0.0));
    return normalize_or(vec3<f32>(at * nh.x, ab * nh.y, max(nh.z, 1e-6)), vec3<f32>(0.0, 0.0, 1.0));
}

struct Ggx {
    t: vec3<f32>,
    b: vec3<f32>,
    n: vec3<f32>,
    v: vec3<f32>,
    at: f32,
    ab: f32,
};

fn ggx_new(t: vec3<f32>, n: vec3<f32>, v: vec3<f32>, at: f32, ab: f32) -> Ggx {
    let b = cross(n, t);
    return Ggx(t, b, n, vec3<f32>(dot(v, t), dot(v, b), max(dot(v, n), 1e-4)), max(at, 1e-3), max(ab, 1e-3));
}

fn ggx_local(g: Ggx, w: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(w, g.t), dot(w, g.b), dot(w, g.n));
}

fn ggx_d(g: Ggx, h: vec3<f32>) -> f32 {
    let k = (h.x / g.at) * (h.x / g.at) + (h.y / g.ab) * (h.y / g.ab) + h.z * h.z;
    return 1.0 / max(PI * g.at * g.ab * k * k, 1e-12);
}

// (D·G / (4 cos θv), density of `ggx_sample`, cos(l, h), 1) toward `w`; w = 0 when below.
fn ggx_eval(g: Ggx, w: vec3<f32>) -> vec4<f32> {
    let l = ggx_local(g, w);
    if (l.z <= 0.0) {
        return vec4<f32>(0.0);
    }
    let h = normalize_or(l + g.v, vec3<f32>(0.0, 0.0, 1.0));
    let pdf = smith_g1(g.v, g.at, g.ab) * ggx_d(g, h) / (4.0 * g.v.z);
    return vec4<f32>(pdf * smith_g1(l, g.at, g.ab), pdf, dot(l, h), 1.0);
}

// A world direction reflected off a visible normal; w = 0 when it points below.
fn ggx_sample(g: Ggx, u1: f32, u2: f32) -> vec4<f32> {
    let h = sample_vndf(g.v, g.at, g.ab, u1, u2);
    let l = h * 2.0 * dot(g.v, h) - g.v;
    if (l.z <= 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(g.t * l.x + g.b * l.y + g.n * l.z, 1.0);
}

fn d_charlie(a: f32, noh: f32) -> f32 {
    let inv = 1.0 / a;
    return (2.0 + inv) * pow(max(1.0 - noh * noh, 1e-4), inv * 0.5) / TAU;
}

fn sheen_l(x: f32, o: f32) -> f32 {
    let a = mix(21.5473, 25.3245, o);
    let b = mix(3.82987, 3.32435, o);
    let c = mix(0.19823, 0.16801, o);
    let d = mix(-1.97760, -1.27393, o);
    let e = mix(-4.32054, -4.85967, o);
    return a / (1.0 + b * pow(max(x, 1e-4), c)) + d * x + e;
}

fn sheen_lambda(cos_t: f32, a: f32) -> f32 {
    let o = (1.0 - a) * (1.0 - a);
    if (cos_t < 0.5) {
        return exp(sheen_l(cos_t, o));
    }
    return exp(2.0 * sheen_l(0.5, o) - sheen_l(1.0 - cos_t, o));
}

fn sheen_vis(nov: f32, nol: f32, a: f32) -> f32 {
    return clamp(1.0 / ((1.0 + sheen_lambda(nov, a) + sheen_lambda(nol, a)) * 4.0 * nov * max(nol, 1e-4)), 0.0, 1.0);
}

fn sheen_albedo(nov: f32, r: f32) -> f32 {
    let a = 1.2416 - 1.7580 * r + 1.7164 * r * r;
    let b = 6.2757 / (r + 0.2077) - 8.6381 + 4.4979 * r;
    let q = 1.2287 - 0.5161 * r;
    let c = 0.1121 * r - 0.3840 * r * r;
    return clamp(a * exp(-b * pow(max(nov, 0.0), q)) + c, 0.0, 1.0);
}

struct Bsdf {
    spec: Ggx,
    coat: Ggx,
    f0: vec3<f32>,
    diffuse: vec3<f32>,
    fv: vec3<f32>,
    coat_weight: f32,
    coat_t: f32,
    sheen: vec3<f32>,
    sheen_a: f32,
    sheen_r: f32,
    sheen_ev: f32,
    p: vec4<f32>,
    walk_albedo: vec3<f32>,
    p_walk: f32,
};

fn bsdf_new(s: Surface, v: vec3<f32>) -> Bsdf {
    let a = max(s.roughness * s.roughness, 1e-3);
    let spec = ggx_new(s.tangent, s.n, v, a * (1.0 + s.anisotropy), a * (1.0 - s.anisotropy));
    let coat_a = s.coat_roughness * s.coat_roughness;
    let coat = ggx_new(onb_t(s.coat_n), s.coat_n, v, coat_a, coat_a);
    let fv = schlick(s.f0, spec.v.z);
    let coat_t = 1.0 - s.coat * coat_fresnel(coat.v.z);
    let sheen_ev = sheen_albedo(spec.v.z, s.sheen_roughness);
    let base = coat_t * (1.0 - max3(s.sheen) * sheen_ev);
    let diffuse = s.diffuse * (1.0 - s.scatter);
    let walk_albedo = s.diffuse * s.scatter * (vec3<f32>(1.0) - fv) * base;
    var w = vec4<f32>(luminance(fv) * base, luminance(diffuse * (vec3<f32>(1.0) - fv)) * base, luminance(s.sheen) * sheen_ev * coat_t, s.coat * coat_fresnel(coat.v.z));
    let sum = w.x + w.y + w.z + w.w;
    w = select(vec4<f32>(0.0), max(w, vec4<f32>(0.1 * sum)), w > vec4<f32>(1e-5));
    let total = w.x + w.y + w.z + w.w;
    let p = select(vec4<f32>(1.0, 0.0, 0.0, 0.0), w / total, total > 0.0);
    let walk = luminance(walk_albedo);
    let p_walk = select(0.0, clamp(walk / (walk + sum), 0.05, 0.95), walk > 1e-5);
    return Bsdf(spec, coat, s.f0, diffuse, fv, s.coat, coat_t, s.sheen, s.sheen_roughness * s.sheen_roughness, s.sheen_roughness, sheen_ev, p, walk_albedo, p_walk);
}

// BRDF × cos toward `w` (xyz) and the density of `bsdf_sample` picking it (w).
fn bsdf_eval(b: Bsdf, w: vec3<f32>) -> vec4<f32> {
    var f = vec3<f32>(0.0);
    var pdf = 0.0;
    let l = ggx_local(b.spec, w);
    if (l.z > 0.0) {
        let m = max3(b.sheen);
        var base = b.coat_t;
        if (m > 0.0) {
            base *= min(1.0 - m * b.sheen_ev, 1.0 - m * sheen_albedo(l.z, b.sheen_r));
        }
        let e = ggx_eval(b.spec, w);
        if (e.w > 0.0) {
            f += schlick(b.f0, e.z) * e.x * base;
            pdf += b.p.x * e.y;
        }
        f += b.diffuse * (vec3<f32>(1.0) - b.fv) * (l.z / PI * base);
        pdf += (b.p.y + b.p.z) * l.z / PI;
        if (m > 0.0) {
            let h = normalize_or(l + b.spec.v, vec3<f32>(0.0, 0.0, 1.0));
            f += b.sheen * (d_charlie(b.sheen_a, h.z) * sheen_vis(b.spec.v.z, l.z, b.sheen_a) * l.z * b.coat_t);
        }
    }
    if (b.coat_weight > 0.0) {
        let e = ggx_eval(b.coat, w);
        if (e.w > 0.0) {
            f += vec3<f32>(b.coat_weight * coat_fresnel(e.z) * e.x);
            pdf += b.p.w * e.y;
        }
    }
    return vec4<f32>(f, pdf);
}

fn bsdf_sample(b: Bsdf) -> vec4<f32> {
    let u0 = rnd();
    let u1 = rnd();
    let u2 = rnd();
    if (u0 < b.p.x) {
        return ggx_sample(b.spec, u1, u2);
    }
    if (u0 < b.p.x + b.p.y + b.p.z) {
        let l = cosine_local(u1, u2);
        return vec4<f32>(b.spec.t * l.x + b.spec.b * l.y + b.spec.n * l.z, 1.0);
    }
    return ggx_sample(b.coat, u1, u2);
}

// ---- Transmission ----

// Unpolarized Fresnel reflectance of a dielectric interface; eta = n(incident) / n(other side).
fn fresnel_dielectric(cos_i: f32, eta: f32) -> f32 {
    let sin2_t = eta * eta * max(1.0 - cos_i * cos_i, 0.0);
    if (sin2_t >= 1.0) {
        return 1.0;
    }
    let cos_t = sqrt(1.0 - sin2_t);
    let rs = (eta * cos_i - cos_t) / (eta * cos_i + cos_t);
    let rp = (cos_i - eta * cos_t) / (cos_i + eta * cos_t);
    return 0.5 * (rs * rs + rp * rp);
}

// Density standing in for a lobe next-event estimation never samples.
const UNSAMPLED_PDF: f32 = 1e15;

struct Glass {
    dir: vec3<f32>,
    w: vec3<f32>,
    ok: bool,
};

// The transmissive base as a rough dielectric interface: reflect with the Fresnel at the IOR,
// else refract into (or out of) the mesh, tinted by the base color on the way in.
fn dielectric(s: Surface, v: vec3<f32>) -> Glass {
    let a = max(s.roughness * s.roughness, 1e-3);
    let lobe = ggx_new(s.tangent, s.n, v, a, a);
    let u1 = rnd();
    let u2 = rnd();
    let hl = sample_vndf(lobe.v, a, a, u1, u2);
    let h = lobe.t * hl.x + lobe.b * hl.y + lobe.n * hl.z;
    let cos_i = max(dot(v, h), 1e-6);
    let ior = max(P.lens.z, 1.0);
    let eta = select(ior, 1.0 / ior, s.front);
    if (rnd() < fresnel_dielectric(cos_i, eta)) {
        let l = h * 2.0 * cos_i - v;
        let ll = ggx_local(lobe, l);
        return Glass(l, vec3<f32>(smith_g1(ll, a, a)), dot(l, s.geo) > 0.0 && ll.z > 0.0);
    }
    let cos_t = sqrt(max(1.0 - eta * eta * (1.0 - cos_i * cos_i), 0.0));
    let l = normalize_or(h * (eta * cos_i - cos_t) - v * eta, -v);
    let ll = ggx_local(lobe, l);
    let tint = select(vec3<f32>(1.0), s.base, s.front);
    return Glass(l, tint * smith_g1(-ll, a, a), dot(l, s.geo) < 0.0 && ll.z < 0.0);
}

// ---- Transport ----

struct Walk {
    exit: vec3<f32>,
    out: vec3<f32>,
    w: vec3<f32>,
    ok: bool,
};

fn random_walk(s: Surface) -> Walk {
    let sigma = P.sigma.xyz;
    var p = s.pos - s.geo * P.forward.w;
    var d = cosine_around(-s.geo);
    var w = vec3<f32>(1.0);
    for (var i = 0u; i < WALK_STEPS; i++) {
        let c = min(u32(rnd() * 3.0), 2u);
        let t = -log(max(1.0 - rnd(), 1e-12)) / sigma[c];
        let hit = traverse(p, d, t, false);
        if (hit.idx != NONE) {
            let tr = exp(-sigma * hit.t);
            w *= tr / max((tr.x + tr.y + tr.z) / 3.0, 1e-30);
            let q0 = tri_p(hit.idx, 0u);
            let n = normalize_or(cross(tri_p(hit.idx, 1u) - q0, tri_p(hit.idx, 2u) - q0), -d);
            let out = select(-n, n, dot(n, d) > 0.0);
            return Walk(p + d * hit.t, out, w, finite3(w));
        }
        let tr = exp(-sigma * t);
        w *= sigma * tr / max(dot(sigma * tr, vec3<f32>(1.0 / 3.0)), 1e-30);
        p += d * t;
        let z = 1.0 - 2.0 * rnd();
        let phi = TAU * rnd();
        let r = sqrt(max(1.0 - z * z, 0.0));
        d = vec3<f32>(r * cos(phi), r * sin(phi), z);
    }
    return Walk(vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), false);
}

fn mis(a: f32, b: f32) -> f32 {
    let a2 = a * a;
    let b2 = b * b;
    return select(0.0, a2 / (a2 + b2), a2 + b2 > 0.0);
}

fn clamp_firefly(c: vec3<f32>) -> vec3<f32> {
    let lum = luminance(c);
    return select(c, c * (32.0 / lum), lum > 32.0);
}

// Next-event estimation toward the environment from `pos` (geometric normal `geo`), through
// the BSDF `b` (its sampling density scaled by `keep`) or, with `lambert`, a Lambertian exit.
fn next_event(pos: vec3<f32>, geo: vec3<f32>, thr: vec3<f32>, b: Bsdf, keep: f32, lambert: bool) -> vec3<f32> {
    if (P.cfg.z == 0u) {
        return vec3<f32>(0.0);
    }
    let e = sample_env();
    let wl = e.xyz;
    let pdf_env = e.w;
    if (pdf_env <= 0.0 || dot(wl, geo) <= 0.0) {
        return vec3<f32>(0.0);
    }
    var f: vec3<f32>;
    var pdf_brdf: f32;
    if (lambert) {
        let c = max(dot(wl, geo), 0.0) / PI;
        f = vec3<f32>(c);
        pdf_brdf = c;
    } else {
        let r = bsdf_eval(b, wl);
        f = r.xyz;
        pdf_brdf = r.w * keep;
    }
    if (max3(f) <= 0.0 || occluded(pos + geo * P.forward.w, wl)) {
        return vec3<f32>(0.0);
    }
    return clamp_firefly(thr * f * env(wl) * (mis(pdf_env, pdf_brdf) / pdf_env));
}

struct Sample {
    radiance: vec3<f32>,
    albedo: vec3<f32>,
    normal: vec3<f32>,
    depth: f32,
    hit: bool,
};

fn trace(o_in: vec3<f32>, d_in: vec3<f32>) -> Sample {
    let eps = P.forward.w;
    var o = o_in;
    var d = d_in;
    var out = Sample(vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), 0.0, false);
    var thr = vec3<f32>(1.0);
    var l = vec3<f32>(0.0);
    var bounce = 0u;
    var brdf_pdf = -1.0;
    let bounces = P.cfg.x;
    for (var it = 0u; it < bounces + 24u; it++) {
        let hit = traverse(o, d, BIG, false);
        if (hit.idx == NONE) {
            if (brdf_pdf < 0.0) {
                return Sample(vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), 0.0, false);
            }
            var w = 1.0;
            if (P.cfg.z != 0u) {
                w = mis(brdf_pdf, env_pdf(d));
            }
            l += clamp_firefly(thr * env(d) * w);
            break;
        }
        let s = surface(hit, o, d);
        if (rnd() > s.opacity) {
            o = s.pos - s.geo * eps;
            continue;
        }
        if (!out.hit) {
            out.hit = true;
            out.albedo = min(s.diffuse + s.f0, vec3<f32>(1.0));
            out.normal = s.n;
            out.depth = length(s.pos - o_in);
        }
        l += thr * s.emissive;
        if (bounce >= bounces) {
            break;
        }
        // Transmission is picked by its weight (a glass event, without next-event estimation).
        let glass = s.transmission > 0.0 && rnd() < s.transmission;
        let b = bsdf_new(s, -d);
        let keep = 1.0 - b.p_walk;
        if (!glass) {
            l += next_event(s.pos, s.geo, thr, b, keep, false);
        }
        var wi: vec3<f32>;
        var geo: vec3<f32>;
        if (glass) {
            let g = dielectric(s, -d);
            if (!g.ok) {
                break;
            }
            thr *= g.w;
            brdf_pdf = UNSAMPLED_PDF;
            o = s.pos;
            wi = g.dir;
            geo = s.geo;
        } else if (b.p_walk > 0.0 && rnd() < b.p_walk) {
            let walk = random_walk(s);
            if (!walk.ok) {
                break;
            }
            thr *= b.walk_albedo * walk.w / b.p_walk;
            l += next_event(walk.exit, walk.out, thr, b, keep, true);
            wi = cosine_around(walk.out);
            brdf_pdf = max(dot(wi, walk.out), 1e-6) / PI;
            o = walk.exit;
            geo = walk.out;
        } else {
            let smp = bsdf_sample(b);
            if (smp.w == 0.0) {
                break;
            }
            wi = smp.xyz;
            let e = bsdf_eval(b, wi);
            if (e.w <= 0.0) {
                break;
            }
            thr *= e.xyz / (e.w * keep);
            brdf_pdf = e.w * keep;
            o = s.pos;
            geo = s.geo;
        }
        if (bounce >= 2u) {
            let p = clamp(max3(thr), 0.05, 0.95);
            if (rnd() > p) {
                break;
            }
            thr /= p;
        }
        o += geo * eps * select(-1.0, 1.0, dot(wi, geo) >= 0.0);
        d = wi;
        bounce += 1u;
    }
    if (!out.hit) {
        return out;
    }
    out.radiance = select(vec3<f32>(0.0), l, finite3(l));
    return out;
}

// The camera ray between the near and far points of a pixel, through a random point of the thin
// lens when the scene has one (`TraceScene::camera_ray`): (origin, direction).
fn camera_ray(near: vec3<f32>, far: vec3<f32>) -> array<vec3<f32>, 2> {
    let dir = normalize_or(far - near, P.forward.xyz);
    let fwd = P.forward.xyz;
    let along = dot(dir, fwd);
    if (P.lens.y <= 0.0 || along <= 1e-6) {
        return array<vec3<f32>, 2>(near, dir);
    }
    let focal = near + dir * ((P.lens.x - dot(near - P.eye.xyz, fwd)) / along);
    let right = normalize_or(cross(fwd, vec3<f32>(0.0, 1.0, 0.0)), vec3<f32>(1.0, 0.0, 0.0));
    let up = cross(right, fwd);
    let r = P.lens.y * sqrt(rnd());
    let phi = TAU * rnd();
    let origin = near + (right * cos(phi) + up * sin(phi)) * r;
    return array<vec3<f32>, 2>(origin, normalize_or(focal - origin, dir));
}

@compute @workgroup_size(8, 8)
fn trace_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = P.size.x;
    let h = P.size.y;
    if (id.x >= w || id.y >= h) {
        return;
    }
    let i = id.y * w + id.x;
    var hit = acc_hit[i];
    var alb = acc_alb[i];
    var nrm = acc_nrm[i];
    var bg = acc_env[i];
    for (var k = 0u; k < P.size.w; k++) {
        let pass_index = P.size.z + k;
        rng = pcg3d(vec3<u32>(id.x, id.y, pass_index * 7919u + 17u)).x | 1u;
        let jx = rnd();
        let jy = rnd();
        let ndc = vec2<f32>((f32(id.x) + jx) / f32(w) * 2.0 - 1.0, 1.0 - (f32(id.y) + jy) / f32(h) * 2.0);
        let a = P.inv_vp * vec4<f32>(ndc, 0.0, 1.0);
        let b = P.inv_vp * vec4<f32>(ndc, 1.0, 1.0);
        let ray = camera_ray(a.xyz / a.w, b.xyz / b.w);
        let origin = ray[0];
        let dir = ray[1];
        let s = trace(origin, dir);
        if (s.hit) {
            hit += vec4<f32>(s.radiance, luminance(s.radiance) * luminance(s.radiance));
            alb += vec4<f32>(s.albedo, 1.0);
            nrm += vec4<f32>(s.normal, s.depth);
        } else {
            bg += vec4<f32>(env(dir), 0.0);
        }
    }
    acc_hit[i] = hit;
    acc_alb[i] = alb;
    acc_nrm[i] = nrm;
    acc_env[i] = bg;
}
