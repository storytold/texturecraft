// TextureCraft viewport shaders: metal/roughness (or specular/glossiness) PBR with image-based
// lighting and optional parallax occlusion mapping, plus the optional lobes of the extra channels
// (clear coat with its own normal, sheen, anisotropy, subsurface scattering, transmission; packing
// in compose/src/display.rs), the G-buffer of the depth of field and subsurface blur passes, the
// 2D UV view,
// the background, debug views and the 3D wireframe. Output is scene-linear HDR, premultiplied by
// alpha = scene coverage over the studio backdrop; exposure, tone mapping and sRGB encoding
// happen in the post pass (post.wgsl). WebGPU/WebGL2-safe (fragment passes only).

struct Globals {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>, // background rays: a perspective camera even when the view is orthographic
    eye: vec4<f32>,      // xyz eye position, w -
    env: vec4<f32>,      // x rotation (radians), y max lod, z AO intensity, w lit single-channel view (1: the channel on a lit neutral material)
    sh: array<vec4<f32>, 9>,
    mode: vec4<f32>,     // x view mode (0 material, 1 channel, 2 mesh map, 3 UV checker), y channel index, z height strength, w normal Y sign
    bg: vec4<f32>,       // x environment opacity, y background blur lod, z UV view (1) or 3D (0), w wireframe opacity
    uv_xform: vec4<f32>, // UV view: scale xy, offset zw (clip space)
    light: vec4<f32>,    // xyz world direction to the key light (rotated with the environment), w intensity (0 = off)
    light_vp: mat4x4<f32>, // world -> shadow map clip space
    shadow: vec4<f32>,   // x shadows on (1), y shadow map texel (uv), z normal offset (world units), w shadow opacity (0..1)
    material: vec4<f32>, // x specular/glossiness shading (1), y parallax occlusion (1), z accumulation sample index, w IOR of transmission
    scatter: vec4<f32>,  // xyz per-channel diffuse wrap at scattering weight 1 (0 = Lambert), w G-buffer holds the scattering light (1)
    fwd: vec4<f32>,      // xyz camera view direction (unit), w -
    disp: vec4<f32>,     // x displacement on (1), y world units per unit of height at strength 1, z vertex spacing in UV units, w -
    refr: vec4<f32>,     // x screen-space refraction on (1), y lods of the scene behind, z refraction depth (world units), w -
    highlight: vec4<f32>, // rgb highlighted set's edge color in the scene (before exposure and tone mapping), a opacity
    highlight_shift: vec4<f32>, // xy sideways shift of each highlight line copy (clip units), zw -
    bake_high: vec4<f32>, // bake mode: rgb high poly color, a -
    bake_matching_error: vec4<f32>, // rgb color where the bake's rays miss the high poly, a its strength (0 hidden)
    bake_cage: vec4<f32>, // rgb cage surface color, a opacity
    bake_cage_wire: vec4<f32>, // rgb cage wireframe color, a opacity
    bake_missing_seam: vec4<f32>, // rgb color of hard edges without a UV seam, a opacity
    bake_neutral_color: vec4<f32>, // rgb the neutral material's color, a its reflection samples this frame
    bake_neutral: vec4<f32>, // x roughness, y metallic, z AO intensity of the neutral material, w on (1)
    bake_bent_normal: vec4<f32>, // x, y how much the bent normal (in the aux texture) turns the diffuse light and dims reflections, zw -
    uv_rotation: vec4<f32>, // UV view: x cos, y sin of the view rotation (counter-clockwise on screen), z view aspect (width / height), w -
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var env_tex: texture_2d<f32>;
@group(0) @binding(2) var env_samp: sampler;
@group(0) @binding(3) var shadow_map: texture_depth_2d;
@group(0) @binding(4) var shadow_samp: sampler_comparison;

@group(1) @binding(0) var t_base: texture_2d<f32>;
@group(1) @binding(1) var t_orm: texture_2d<f32>;
@group(1) @binding(2) var t_nrm: texture_2d<f32>;
@group(1) @binding(3) var t_height: texture_2d<f32>;
@group(1) @binding(4) var t_emis: texture_2d<f32>;
@group(1) @binding(5) var t_lobes: texture_2d<f32>;
@group(1) @binding(6) var t_sheen: texture_2d<f32>;
@group(1) @binding(7) var t_extra: texture_2d<f32>;
@group(1) @binding(8) var t_aux: texture_2d<f32>;
@group(1) @binding(9) var samp: sampler;

// The scene behind transmissive surfaces (screen-space refraction), black when off.
@group(2) @binding(0) var behind_tex: texture_2d<f32>;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) nrm: vec3<f32>,
    @location(2) tan: vec4<f32>,
    @location(3) uv: vec2<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) wpos: vec3<f32>,
    @location(1) nrm: vec3<f32>,
    @location(2) tan: vec4<f32>,
    @location(3) uv: vec2<f32>,
};

// The vertex moved along its normal by the height channel (displacement).
fn displaced(v: VIn) -> vec3<f32> {
    if (g.disp.x < 0.5) {
        return v.pos;
    }
    return displace_by_height(v.pos, v.nrm, v.uv, t_height, samp, g.mode.z * g.disp.y, g.disp.z);
}

@vertex
fn vs_mesh(v: VIn) -> VOut {
    var o: VOut;
    let p = displaced(v);
    o.clip = g.view_proj * vec4<f32>(p, 1.0);
    o.wpos = p;
    o.nrm = v.nrm;
    o.tan = v.tan;
    o.uv = v.uv;
    return o;
}

@vertex
fn vs_uv(v: VIn) -> VOut {
    var o: VOut;
    let scaled = vec2<f32>(v.uv.x * 2.0 - 1.0, 1.0 - v.uv.y * 2.0) * g.uv_xform.xy;

    // The rotation turns the square in pixels, not in clip space, so it stays square on a wide view
    let aspect = g.uv_rotation.z;
    let c = g.uv_rotation.x;
    let s = g.uv_rotation.y;
    let turned = vec2<f32>((c * scaled.x * aspect - s * scaled.y) / aspect, s * scaled.x * aspect + c * scaled.y);
    let p = turned + g.uv_xform.zw;
    o.clip = vec4<f32>(p, 0.5, 1.0);
    o.wpos = v.pos;
    o.nrm = v.nrm;
    o.tan = v.tan;
    o.uv = v.uv;
    return o;
}

const PI: f32 = 3.14159265;

fn env_uv(d: vec3<f32>) -> vec2<f32> {
    let c = cos(g.env.x);
    let s = sin(g.env.x);
    let r = vec3<f32>(c * d.x - s * d.z, d.y, s * d.x + c * d.z);
    let phi = atan2(r.z, r.x);
    return vec2<f32>(fract(phi / (2.0 * PI) + 1.0), acos(clamp(r.y, -1.0, 1.0)) / PI);
}

fn env_rotate(d: vec3<f32>) -> vec3<f32> {
    let c = cos(g.env.x);
    let s = sin(g.env.x);
    return vec3<f32>(c * d.x - s * d.z, d.y, s * d.x + c * d.z);
}

// Diffuse irradiance from the SH environment. `wrap` (per channel, 0..1) convolves with the
// wrapped-Lambert kernel max(cos + w, 0) / (1 + w)^2 instead of the cosine: its bands 1 and 2
// scale by exactly 1 - w/2 and (1 - w)^2 (w = 0 is plain Lambert).
fn sh_irradiance(n0: vec3<f32>, wrap: vec3<f32>) -> vec3<f32> {
    let n = env_rotate(n0);
    let x = n.x;
    let y = n.y;
    let z = n.z;
    let f1 = 1.0 - 0.5 * wrap;
    let f2 = (1.0 - wrap) * (1.0 - wrap);
    var r = g.sh[0].xyz * 0.282095;
    r += f1 * (g.sh[1].xyz * (0.488603 * y));
    r += f1 * (g.sh[2].xyz * (0.488603 * z));
    r += f1 * (g.sh[3].xyz * (0.488603 * x));
    r += f2 * (g.sh[4].xyz * (1.092548 * x * y));
    r += f2 * (g.sh[5].xyz * (1.092548 * y * z));
    r += f2 * (g.sh[6].xyz * (0.315392 * (3.0 * z * z - 1.0)));
    r += f2 * (g.sh[7].xyz * (1.092548 * x * z));
    r += f2 * (g.sh[8].xyz * (0.546274 * (x * x - y * y)));
    return max(r, vec3<f32>(0.0));
}

fn max3(c: vec3<f32>) -> f32 {
    return max(c.r, max(c.g, c.b));
}

fn schlick(f0: f32, cos_t: f32) -> f32 {
    return f0 + (1.0 - f0) * pow(1.0 - cos_t, 5.0);
}

// GGX distribution × height-correlated Smith visibility (isotropic, alpha = roughness^2).
fn ggx_dv(a: f32, nov: f32, nol: f32, noh: f32) -> f32 {
    let a2 = a * a;
    let d = a2 / (PI * pow(noh * noh * (a2 - 1.0) + 1.0, 2.0));
    let vis = 0.5 / max(nol * sqrt(nov * nov * (1.0 - a2) + a2) + nov * sqrt(nol * nol * (1.0 - a2) + a2), 1e-5);
    return d * vis;
}

// Anisotropic GGX × height-correlated Smith visibility (Heitz 2014). `tb_*`: the (tangent,
// bitangent) components of h, v and l in the anisotropy frame.
fn ggx_dv_aniso(at: f32, ab: f32, tb_h: vec2<f32>, noh: f32, tb_v: vec2<f32>, nov: f32, tb_l: vec2<f32>, nol: f32) -> f32 {
    let a2 = at * ab;
    let dd = vec3<f32>(ab * tb_h.x, at * tb_h.y, a2 * noh);
    let w2 = a2 / dot(dd, dd);
    let d = a2 * w2 * w2 / PI;
    let lv = nol * length(vec3<f32>(at * tb_v.x, ab * tb_v.y, nov));
    let ll = nov * length(vec3<f32>(at * tb_l.x, ab * tb_l.y, nol));
    return d * 0.5 / max(lv + ll, 1e-5);
}

// Sheen: the "Charlie" distribution and the visibility of Estevez & Kulla, "Production Friendly
// Microfacet Sheen BRDF" (2017), alpha = sheen roughness^2. Same math as trace.rs.
fn d_charlie(a: f32, noh: f32) -> f32 {
    let inv = 1.0 / a;
    let sin2 = max(1.0 - noh * noh, 1e-4);
    return (2.0 + inv) * pow(sin2, inv * 0.5) / (2.0 * PI);
}

fn sheen_l(x: f32, a: f32) -> f32 {
    let o = (1.0 - a) * (1.0 - a);
    let p = mix(vec4<f32>(21.5473, 3.82987, 0.19823, -1.97760), vec4<f32>(25.3245, 3.32435, 0.16801, -1.27393), o);
    let e = mix(-4.32054, -4.85967, o);
    return p.x / (1.0 + p.y * pow(max(x, 1e-4), p.z)) + p.w * x + e;
}

fn sheen_lambda(c: f32, a: f32) -> f32 {
    if (c < 0.5) {
        return exp(sheen_l(c, a));
    }
    return exp(2.0 * sheen_l(0.5, a) - sheen_l(1.0 - c, a));
}

fn sheen_vis(nov: f32, nol: f32, a: f32) -> f32 {
    return clamp(1.0 / ((1.0 + sheen_lambda(nov, a) + sheen_lambda(nol, a)) * 4.0 * nov * max(nol, 1e-4)), 0.0, 1.0);
}

// Directional albedo of the sheen lobe: a fit of its numerical integral (error under 0.03 for
// sheen roughness 0.2..1 away from grazing angles; trace.rs tests it). It lights the sheen from
// the environment and scales the layers below so base + sheen stays energy preserving.
fn sheen_albedo(nov: f32, r: f32) -> f32 {
    let a = 1.2416 - 1.7580 * r + 1.7164 * r * r;
    let b = 6.2757 / (r + 0.2077) - 8.6381 + 4.4979 * r;
    let q = 1.2287 - 0.5161 * r;
    let c = 0.1121 * r - 0.3840 * r * r;
    return clamp(a * exp(-b * pow(nov, q)) + c, 0.0, 1.0);
}

// Analytic split-sum environment BRDF (Karis, "Physically Based Shading on Mobile").
fn env_brdf(f0: vec3<f32>, rough: f32, nov: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = rough * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * nov)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

// Data views bypass exposure and tone mapping: linear color, clamped.
fn data_out(c: vec3<f32>) -> vec4<f32> {
    return vec4<f32>(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

// A value shown as-is on screen (normals, height, masks): undo the post pass's sRGB encoding.
fn raw_out(v: vec3<f32>) -> vec4<f32> {
    return vec4<f32>(to_linear(clamp(v, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
}

// Key-light visibility from the shadow map: 3x3 hardware-filtered PCF taps.
fn shadow_factor(wpos: vec3<f32>, n_geo: vec3<f32>) -> f32 {
    let p = g.light_vp * vec4<f32>(wpos + n_geo * g.shadow.z, 1.0);
    let ndc = p.xyz / p.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || ndc.z >= 1.0) {
        return 1.0;
    }
    let depth = ndc.z - 0.0005;
    var s = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            s += textureSampleCompareLevel(shadow_map, shadow_samp, uv + vec2<f32>(f32(x), f32(y)) * g.shadow.y * 1.5, depth);
        }
    }
    return s / 9.0;
}

// Reoriented normal mapping of a detail normal onto a base normal (tangent space, unpacked).
fn rnm(base: vec3<f32>, detail: vec3<f32>) -> vec3<f32> {
    let t = base + vec3<f32>(0.0, 0.0, 1.0);
    let u = detail * vec3<f32>(-1.0, -1.0, 1.0);
    return normalize(t * dot(t, u) - u * t.z);
}

// Byte 128 decodes to zero so the coat normal in Extra stays flat
fn snorm8(v: vec3<f32>) -> vec3<f32> {
    return clamp((v * 255.0 - 128.0) / 127.0, vec3<f32>(-1.0), vec3<f32>(1.0));
}

// Coat normal (tangent space around the mesh normal) from the Extra texture's signed bytes.
fn coat_normal_ts(extra: vec4<f32>) -> vec3<f32> {
    let xy = snorm8(extra.rgb).xy;
    return vec3<f32>(xy, sqrt(max(1.0 - dot(xy, xy), 0.0)));
}

// Channel views of the extra lobes (`Channel::ALL` indices 11..20).
fn lobe_view(ch: i32, lobes: vec4<f32>, sheen: vec4<f32>, sheen_rough: f32, scatter: f32, extra: vec4<f32>) -> vec4<f32> {
    switch ch {
        case 19: { return raw_out(vec3<f32>(extra.b)); }
        case 20: { return raw_out(coat_normal_ts(extra) * 0.5 + 0.5); }
        case 11: { return raw_out(vec3<f32>(lobes.r)); }
        case 12: { return raw_out(vec3<f32>(lobes.g)); }
        case 13: { return raw_out(vec3<f32>(sheen.a)); }
        case 14: { return data_out(sheen.rgb); }
        case 15: { return raw_out(vec3<f32>(sheen_rough)); }
        case 16: { return raw_out(vec3<f32>(lobes.b)); }
        case 17: { return raw_out(vec3<f32>(lobes.a)); }
        default: { return raw_out(vec3<f32>(scatter)); }
    }
}

fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

const PARALLAX_STEPS: i32 = 16;

// The share of its atlas block the UV tile at `uv` fills (Extra alpha, packed in
// compose/src/display.rs): a smaller tile has fewer texels per UV unit, so its height slopes per
// texel shrink with it. Read unfiltered, so neighbouring blocks and mips never blend scales.
fn tile_scale(uv: vec2<f32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(t_extra));
    let p = clamp(vec2<i32>(floor(uv * vec2<f32>(dims))), vec2<i32>(0), dims - 1);
    return exp2(-textureLoad(t_extra, p, 0).a * 255.0 / 32.0);
}

// Parallax occlusion mapping: march the view ray down through the signed height field (the mesh
// surface at height 0; height 1 lies strength/16 UV units above it, the same scale as
// height-to-normal) and return the UV where it first goes below the heights. `v_ts`: tangent-space
// direction to the eye (+y = up the image, so v grows along -y). A fixed step count keeps the
// loop free of data-dependent exits (derivatives stay valid after it).
fn parallax_uv(uv: vec2<f32>, v_ts: vec3<f32>) -> vec2<f32> {
    let dx = dpdx(uv);
    let dy = dpdy(uv);
    // UV shift per unit of height along the ray (grazing angles capped).
    let shift = vec2<f32>(v_ts.x, -v_ts.y) / max(v_ts.z, 0.15) * (g.mode.z / 16.0) * tile_scale(uv);
    var prev_z = 1.0;
    var prev_d = 1.0 - textureSampleGrad(t_height, samp, uv + shift, dx, dy).r;
    // Heights at the top of the range already hide the ray at entry.
    var found = prev_d <= 0.0;
    var hit = select(-1.0, 1.0, found);
    for (var k = 1; k <= PARALLAX_STEPS; k++) {
        let z = 1.0 - 2.0 * f32(k) / f32(PARALLAX_STEPS);
        let d = z - textureSampleGrad(t_height, samp, uv + shift * z, dx, dy).r;
        if (!found && d <= 0.0) {
            // Linear refinement between the last step above and the first below the surface.
            hit = mix(prev_z, z, prev_d / max(prev_d - d, 1e-5));
            found = true;
        }
        prev_z = z;
        prev_d = d;
    }
    return uv + shift * hit;
}

// The shaded color, and the scattering part of its diffuse light (blurred by the subsurface pass).
struct Shaded {
    color: vec4<f32>,
    sss: vec3<f32>,
};

fn unlit(c: vec4<f32>) -> Shaded {
    return Shaded(c, vec3<f32>(0.0));
}

fn mesh_color(i: VOut, front: bool) -> vec4<f32> {
    let o = shade_mesh(i, front);
    if (g.scatter.w > 0.5) {
        // The subsurface blur pass adds the scattering light back, blurred (fx.wgsl).
        return o.color - vec4<f32>(o.sss, 0.0);
    }
    return o.color;
}

@fragment
fn fs_mesh(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return mesh_color(i, front);
}

// The scene behind transmissive surfaces: everything but them (what they refract).
@fragment
fn fs_opaque(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let trans = textureSample(t_extra, samp, i.uv).b * (1.0 - textureSample(t_orm, samp, i.uv).b);
    let c = mesh_color(i, front);
    if (trans > 0.0) {
        discard;
    }
    return c;
}

// The depth of field / subsurface G-buffer: rgb the scattering diffuse light (when the blur is
// on), a the view depth (the clear value 0 marks the background).
@fragment
fn fs_gbuf(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let z = max(dot(i.wpos - g.eye.xyz, g.fwd.xyz), 1e-4);
    var sss = vec3<f32>(0.0);
    if (g.scatter.w > 0.5) {
        sss = shade_mesh(i, front).sss;
    } else if (textureSample(t_base, samp, i.uv).a < hash12(i.clip.xy)) {
        discard;
    }
    return vec4<f32>(sss, z);
}

// Roughness floors keep specular lobes finite
const SPECULAR_ROUGHNESS_FLOOR: f32 = 0.045;
const SHEEN_ROUGHNESS_FLOOR: f32 = 0.07;

// Lower bounds prevent degenerate lobes and normalization divisions
const GRAZING_COSINE_FLOOR: f32 = 1e-4;
const ANISOTROPIC_ALPHA_FLOOR: f32 = 0.002;
const TANGENT_LENGTH_SQUARED_FLOOR: f32 = 1e-8;
const REFRACTION_LENGTH_SQUARED_FLOOR: f32 = 1e-6;
const PROJECTION_W_FLOOR: f32 = 1e-6;

// Ignore coat tilts below half a signed 8-bit step
const COAT_TILT_THRESHOLD: f32 = 0.004;

// Tune the anisotropic reflection bend and geometric horizon fade
const ANISOTROPY_BEND_GAIN: f32 = 5.0;
const HORIZON_FADE_STRENGTH: f32 = 1.3;

const UV_CHECKER_CELLS: f32 = 32.0;
const UV_CHECKER_FINE_CELLS: f32 = 256.0;

// Offset alpha dithering between accumulated frames
const DITHER_FRAME_STEP: vec2<f32> = vec2<f32>(17.0, 59.0);

fn shade_mesh(i: VOut, front: bool) -> Shaded {
    var n_geo = normalize(i.nrm);
    if (!front) {
        n_geo = -n_geo;
    }

    let t = normalize(i.tan.xyz - n_geo * dot(n_geo, i.tan.xyz));
    let b = cross(n_geo, t) * i.tan.w;
    let v = normalize(g.eye.xyz - i.wpos);
    var uv = i.uv;
    if (g.material.y > 0.5) {
        uv = parallax_uv(uv, vec3<f32>(dot(v, t), dot(v, b), dot(v, n_geo)));
    }

    // A lit single-channel view puts the channel on a plain lit material: the other channels' lobes are left out
    let solo_lit = g.env.w > 0.5 && i32(g.mode.x + 0.5) == 1;
    var base_s = textureSample(t_base, samp, uv);
    var orm = textureSample(t_orm, samp, uv);
    let nrm_t = textureSample(t_nrm, samp, uv);
    let nrm_s = nrm_t.xyz;
    var emis_t = textureSample(t_emis, samp, uv);
    var lobes = textureSample(t_lobes, samp, uv);
    var sheen_t = textureSample(t_sheen, samp, uv);
    var extra = textureSample(t_extra, samp, uv);
    if (solo_lit) {
        // Base color shows itself; metallic and roughness show as a grey base color; normal and height show on neutral grey
        var shown = vec3<f32>(0.5);
        switch i32(g.mode.y + 0.5) {
            case 0: { shown = base_s.rgb; }
            case 1: { shown = vec3<f32>(orm.b); }
            case 2: { shown = vec3<f32>(orm.g); }
            default: {}
        }
        base_s = vec4<f32>(shown, base_s.a);
        orm = vec4<f32>(1.0, 0.5, 0.0, 0.5);
        emis_t = vec4<f32>(0.0);
        lobes = vec4<f32>(0.0);
        sheen_t = vec4<f32>(0.0);
        extra = vec4<f32>(0.0);
    }
    let emis = emis_t.rgb;
    let aux = textureSample(t_aux, samp, uv);
    let dims = vec2<f32>(textureDimensions(t_height));
    let ts = 1.0 / dims;
    let hgt = textureSample(t_height, samp, uv);
    let hc = hgt.r;
    let hl = textureSample(t_height, samp, uv - vec2<f32>(ts.x, 0.0)).r;
    let hr = textureSample(t_height, samp, uv + vec2<f32>(ts.x, 0.0)).r;
    let hu = textureSample(t_height, samp, uv - vec2<f32>(0.0, ts.y)).r;
    let hd = textureSample(t_height, samp, uv + vec2<f32>(0.0, ts.y)).r;

    // Match export: a unit height over 1/16 of the width tilts 45°
    // Combine the height gradient with the tangent-space normal
    let k = g.mode.z * dims.x / 16.0 * tile_scale(uv);
    let n_height = normalize(vec3<f32>(-(hr - hl) * 0.5 * k, (hd - hu) * 0.5 * k, 1.0));

    // The Normal texture holds the signed normal in half floats (flat is exactly +z)
    var n_detail = clamp(nrm_s, vec3<f32>(-1.0), vec3<f32>(1.0));
    n_detail.y *= g.mode.w;
    let n_ts = rnm(n_height, normalize(n_detail));
    let n = normalize(t * n_ts.x + b * n_ts.y + n_geo * n_ts.z);

    let mode = i32(g.mode.x + 0.5);
    if (mode == 1 && !solo_lit) {
        let ch = i32(g.mode.y + 0.5);
        if (ch >= 11) {
            return unlit(lobe_view(ch, lobes, sheen_t, nrm_t.a, emis_t.a, extra));
        }

        switch ch {
            case 0: { return unlit(data_out(base_s.rgb)); }
            case 1: { return unlit(data_out(vec3<f32>(orm.b))); }
            case 2: { return unlit(data_out(vec3<f32>(orm.g))); }
            case 3: { return unlit(raw_out(n_ts * 0.5 + 0.5)); }
            case 4: { return unlit(raw_out(vec3<f32>(hc * 0.5 + 0.5))); }
            case 5: { return unlit(data_out(emis)); }
            case 6: { return unlit(raw_out(vec3<f32>(base_s.a))); }
            case 7: { return unlit(raw_out(vec3<f32>(orm.r))); }
            case 9: { return unlit(data_out(hgt.gba)); }
            case 10: { return unlit(data_out(vec3<f32>(1.0 - orm.g))); }
            default: { return unlit(raw_out(vec3<f32>(orm.a))); }
        }
    }

    if (mode == 2) {
        return unlit(raw_out(aux.rgb));
    }

    if (mode == 4) {
        // The normal the shading uses (mesh, height slope and normal channel), as a world-space normal
        return unlit(raw_out(n * 0.5 + 0.5));
    }

    if (mode == 3) {
        let c = floor(uv * UV_CHECKER_CELLS);
        let chk = (c.x + c.y) - 2.0 * floor((c.x + c.y) * 0.5);
        let fine = floor(uv * UV_CHECKER_FINE_CELLS);
        let f2 = (fine.x + fine.y) - 2.0 * floor((fine.x + fine.y) * 0.5);
        let col = mix(vec3<f32>(0.85, 0.85, 0.85), vec3<f32>(0.18, 0.2, 0.24), chk) * (0.92 + 0.08 * f2);
        let tint = vec3<f32>(fract(uv.x), fract(uv.y), 0.5) * 0.25;
        let diff = max(dot(n, normalize(vec3<f32>(0.4, 0.8, 0.5))), 0.0) * 0.6 + 0.4;
        return unlit(data_out((col + tint) * diff));
    }

    if (g.bake_neutral.w > 0.5) {
        return Shaded(neutral_color(n, n_geo, t, b, v, i.wpos, orm.r, aux.rgb), vec3<f32>(0.0));
    }

    // Change the alpha dither each frame so accumulation smooths it
    if (base_s.a < hash12(i.clip.xy + g.material.z * DITHER_FRAME_STEP)) {
        discard;
    }

    let albedo = base_s.rgb;
    let ao = mix(1.0, orm.r, g.env.z);
    let rough = clamp(orm.g, SPECULAR_ROUGHNESS_FLOOR, 1.0);
    let metal = orm.b;
    let nov = clamp(abs(dot(n, v)), GRAZING_COSINE_FLOOR, 1.0);
    var f0 = mix(vec3<f32>(0.08 * orm.a), albedo, metal);
    var diffuse_col = albedo * (1.0 - metal);
    if (g.material.x > 0.5 && !solo_lit) {
        // Specular color gives F0 (a solo-lit channel view keeps its neutral material); remove its energy from diffuse
        f0 = hgt.gba;
        diffuse_col = albedo * (1.0 - max(f0.r, max(f0.g, f0.b)));
    }

    // Transmission removes energy from the dielectric diffuse base
    let trans = extra.b * (1.0 - metal);
    diffuse_col *= 1.0 - trans;

    // Zero lobe weights preserve the base shading
    let coat_w = lobes.r;
    let coat_rough = clamp(lobes.g, SPECULAR_ROUGHNESS_FLOOR, 1.0);
    let aniso = lobes.b;
    let sheen_col = sheen_t.rgb * sheen_t.a;
    let sheen_max = max3(sheen_col);
    let sheen_rough = clamp(nrm_t.a, SHEEN_ROUGHNESS_FLOOR, 1.0);

    // Scatter color controls each channel's diffuse wrap
    let wrap = emis_t.a * g.scatter.xyz;

    // Stretch GGX along the tangent rotated by 0..π
    let a = rough * rough;
    let at = max(a * (1.0 + aniso), ANISOTROPIC_ALPHA_FLOOR);
    let ab = max(a * (1.0 - aniso), ANISOTROPIC_ALPHA_FLOOR);
    let ang = lobes.a * PI;
    let ta0 = t * cos(ang) + b * sin(ang);
    let ta1 = ta0 - n * dot(n, ta0);
    let ta = ta1 * inverseSqrt(max(dot(ta1, ta1), TANGENT_LENGTH_SQUARED_FLOOR));
    let ba = cross(n, ta);

    // Coat normals perturb the geometric normal independently of base detail
    var n_c = n_geo;
    let cn = coat_normal_ts(extra);
    if (coat_w > 0.0 && max(abs(cn.x), abs(cn.y)) > COAT_TILT_THRESHOLD) {
        n_c = normalize(t * cn.x + b * cn.y + n_geo * cn.z);
    }

    let nov_c = clamp(abs(dot(n_c, v)), GRAZING_COSINE_FLOOR, 1.0);
    let coat_t = 1.0 - coat_w * schlick(0.04, nov_c);

    // Coat transmission and sheen albedo limit energy reaching the base
    var base_scale = coat_t;
    var sheen_e = 0.0;
    if (sheen_max > 0.0) {
        sheen_e = sheen_albedo(nov, sheen_rough);
        base_scale *= 1.0 - sheen_max * sheen_e;
    }

    let diffuse = sh_irradiance(n, wrap) * diffuse_col * ao;
    var r = reflect(-v, n);
    if (aniso > 0.0) {
        // Bend the reflection to approximate anisotropic IBL
        let bent = cross(cross(ba, v), ba);
        r = reflect(-v, normalize(mix(n, bent, aniso * clamp(ANISOTROPY_BEND_GAIN * rough, 0.0, 1.0))));
    }

    let pre = textureSampleLevel(env_tex, env_samp, env_uv(r), rough * g.env.y).rgb;

    // Fade reflections below the geometric surface
    let horizon = clamp(1.0 + HORIZON_FADE_STRENGTH * dot(r, n_geo), 0.0, 1.0);
    let spec = pre * env_brdf(f0, rough, nov) * ao * horizon * horizon;
    var layers = vec3<f32>(0.0);
    if (sheen_max > 0.0) {
        let pre_s = textureSampleLevel(env_tex, env_samp, env_uv(reflect(-v, n)), sheen_rough * g.env.y).rgb;
        layers += pre_s * sheen_col * (sheen_e * ao * coat_t);
    }

    if (coat_w > 0.0) {
        let pre_c = textureSampleLevel(env_tex, env_samp, env_uv(reflect(-v, n_c)), coat_rough * g.env.y).rgb;
        layers += pre_c * env_brdf(vec3<f32>(0.04), coat_rough, nov_c) * (coat_w * ao);
    }

    if (trans > 0.0) {
        // Approximate one refraction interface; the path tracer also handles the exit
        var td = refract(-v, n, 1.0 / max(g.material.w, 1.0));
        if (dot(td, td) < REFRACTION_LENGTH_SQUARED_FLOOR) {
            td = reflect(-v, n);
        }

        var behind = textureSampleLevel(env_tex, env_samp, env_uv(td), rough * g.env.y).rgb;
        if (g.refr.x > 0.5) {
            // Composite the refracted screen sample over the environment fallback
            let p = g.view_proj * vec4<f32>(i.wpos + td * g.refr.z, 1.0);
            let s = p.xy / max(p.w, PROJECTION_W_FLOOR);
            let suv = vec2<f32>(s.x * 0.5 + 0.5, 0.5 - s.y * 0.5);
            if (p.w > 0.0 && all(suv >= vec2<f32>(0.0)) && all(suv <= vec2<f32>(1.0))) {
                let lod = g.refr.y * rough * clamp(g.material.w * 2.0 - 2.0, 0.0, 1.0);
                let seen = textureSampleLevel(behind_tex, samp, suv, lod);
                behind = seen.rgb + behind * (1.0 - seen.a);
            }
        }
        layers += behind * albedo * (vec3<f32>(1.0) - env_brdf(f0, rough, nov)) * (trans * base_scale);
    }

    var direct = vec3<f32>(0.0);
    var direct_diffuse = vec3<f32>(0.0);
    if (g.light.w > 0.0) {
        let l = normalize(g.light.xyz);
        let h = normalize(l + v);
        let nol_raw = dot(n, l);
        let nol = max(nol_raw, 0.0);
        let noh = max(dot(n, h), 0.0);
        let voh = max(dot(v, h), 0.0);
        var dv = 0.0;
        if (aniso > 0.0) {
            dv = ggx_dv_aniso(at, ab, vec2<f32>(dot(ta, h), dot(ba, h)), noh, vec2<f32>(dot(ta, v), dot(ba, v)), nov, vec2<f32>(dot(ta, l), dot(ba, l)), nol);
        } else {
            dv = ggx_dv(a, nov, nol, noh);
        }

        let f = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - voh, 5.0);

        // Normalize wrapped Lambert to conserve energy
        let diff_nol = max((nol_raw + wrap) / ((1.0 + wrap) * (1.0 + wrap)), vec3<f32>(0.0));
        direct_diffuse = (vec3<f32>(1.0) - f) * diffuse_col / PI * diff_nol * (g.light.w * base_scale);
        direct = ((vec3<f32>(1.0) - f) * diffuse_col / PI * diff_nol + dv * f * nol) * (g.light.w * base_scale);
        if (sheen_max > 0.0) {
            let sa = sheen_rough * sheen_rough;
            direct += sheen_col * (d_charlie(sa, noh) * sheen_vis(nov, nol, sa) * nol * g.light.w * coat_t);
        }

        if (coat_w > 0.0) {
            let nol_c = max(dot(n_c, l), 0.0);
            direct += vec3<f32>(ggx_dv(coat_rough * coat_rough, nov_c, nol_c, max(dot(n_c, h), 0.0)) * schlick(0.04, voh) * coat_w * nol_c * g.light.w);
        }

        if (g.shadow.x > 0.5 && nol > 0.0) {
            let sh = mix(1.0, shadow_factor(i.wpos, n_geo), g.shadow.w);
            direct *= sh;
            direct_diffuse *= sh;
        }
    }

    return Shaded(vec4<f32>((diffuse + spec) * base_scale + layers + direct + emis, 1.0), (diffuse * base_scale + direct_diffuse) * emis_t.a);
}

// The 2D UV view shows unlit color or a debug channel over the UV layout
@fragment
fn fs_uv(i: VOut) -> @location(0) vec4<f32> {
    let base_s = textureSample(t_base, samp, i.uv);
    let orm = textureSample(t_orm, samp, i.uv);
    let mode = i32(g.mode.x + 0.5);
    if (mode == 1) {
        let ch = i32(g.mode.y + 0.5);
        if (ch >= 11) {
            let lobes = textureSample(t_lobes, samp, i.uv);
            let sheen = textureSample(t_sheen, samp, i.uv);
            return lobe_view(ch, lobes, sheen, textureSample(t_nrm, samp, i.uv).a, textureSample(t_emis, samp, i.uv).a, textureSample(t_extra, samp, i.uv));
        }

        switch ch {
            case 1: { return data_out(vec3<f32>(orm.b)); }
            case 2: { return data_out(vec3<f32>(orm.g)); }
            case 3: { return raw_out(textureSample(t_nrm, samp, i.uv).rgb * 0.5 + 0.5); }
            case 4: { return raw_out(vec3<f32>(textureSample(t_height, samp, i.uv).r * 0.5 + 0.5)); }
            case 5: { return data_out(textureSample(t_emis, samp, i.uv).rgb); }
            case 6: { return raw_out(vec3<f32>(base_s.a)); }
            case 7: { return raw_out(vec3<f32>(orm.r)); }
            case 8: { return raw_out(vec3<f32>(orm.a)); }
            case 9: { return data_out(textureSample(t_height, samp, i.uv).gba); }
            case 10: { return data_out(vec3<f32>(1.0 - orm.g)); }
            default: { return data_out(base_s.rgb); }
        }
    }

    if (mode == 2) {
        return raw_out(textureSample(t_aux, samp, i.uv).rgb);
    }

    if (mode == 4) {
        // The tangent-space normal the shading uses: the height slope turned by the normal channel
        let dims = vec2<f32>(textureDimensions(t_height));
        let ts = 1.0 / dims;
        let hl = textureSample(t_height, samp, i.uv - vec2<f32>(ts.x, 0.0)).r;
        let hr = textureSample(t_height, samp, i.uv + vec2<f32>(ts.x, 0.0)).r;
        let hu = textureSample(t_height, samp, i.uv - vec2<f32>(0.0, ts.y)).r;
        let hd = textureSample(t_height, samp, i.uv + vec2<f32>(0.0, ts.y)).r;
        let k = g.mode.z * dims.x / 16.0 * tile_scale(i.uv);
        let n_height = normalize(vec3<f32>(-(hr - hl) * 0.5 * k, (hd - hu) * 0.5 * k, 1.0));
        var n_detail = clamp(textureSample(t_nrm, samp, i.uv).rgb, vec3<f32>(-1.0), vec3<f32>(1.0));
        n_detail.y *= g.mode.w;
        return raw_out(rnm(n_height, normalize(n_detail)) * 0.5 + 0.5);
    }

    if (mode == 3) {
        let c = floor(i.uv * UV_CHECKER_CELLS);
        let chk = (c.x + c.y) - 2.0 * floor((c.x + c.y) * 0.5);
        return data_out(mix(vec3<f32>(0.85), vec3<f32>(0.18, 0.2, 0.24), chk));
    }

    // Light AO shading keeps the flat 2D material view readable
    return data_out(base_s.rgb * mix(1.0, orm.r, 0.5));
}

// Background: the blurred environment, premultiplied by its opacity (the post pass puts the flat
// studio backdrop behind it).
struct BgOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_bg(@builtin(vertex_index) vi: u32) -> BgOut {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -3.0), vec2<f32>(-1.0, 1.0), vec2<f32>(3.0, 1.0));
    var o: BgOut;
    o.pos = vec4<f32>(p[vi], 1.0, 1.0);
    o.ndc = p[vi];
    return o;
}

@fragment
fn fs_bg(i: BgOut) -> @location(0) vec4<f32> {
    let o = g.bg.x;
    if (g.bg.z > 0.5 || o <= 0.001) {
        return vec4<f32>(0.0);
    }
    let a = g.inv_view_proj * vec4<f32>(i.ndc, 0.0, 1.0);
    let b = g.inv_view_proj * vec4<f32>(i.ndc, 1.0, 1.0);
    let d = normalize(b.xyz / b.w - a.xyz / a.w);
    let env = textureSampleLevel(env_tex, env_samp, env_uv(d), g.bg.y).rgb;
    return vec4<f32>(env * o, o);
}

// UV wireframe in the 2D view.
@fragment
fn fs_wire(i: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(0.9, 0.2, 0.2, g.bg.w);
}

// 3D wireframe over the shaded mesh: depth-tested, pulled slightly toward the camera so the
// lines win against their own triangles.
fn wire3d(v: VIn) -> VOut {
    var o: VOut;
    o.wpos = v.pos;
    o.nrm = v.nrm;
    o.tan = v.tan;
    o.uv = v.uv;
    let p = displaced(v);
    o.clip = g.view_proj * vec4<f32>(p + (g.eye.xyz - p) * 0.002, 1.0);
    return o;
}

@vertex
fn vs_wire3d(v: VIn) -> VOut {
    return wire3d(v);
}

@fragment
fn fs_wire3d(i: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(0.35, 0.35, 0.35, g.bg.w);
}

// How far overlay quads move toward the eye, as a fraction of their distance (as the wireframe does)
const OVERLAY_TOWARD_EYE: f32 = 0.002;

// See-through overlay quads (the symmetry plane and the lines on the mesh): the color comes in the tangent slot, nudged toward the eye like the wireframe so lines on the surface win the depth test
@vertex
fn vs_overlay(v: VIn) -> VOut {
    var o: VOut;
    o.wpos = v.pos;
    o.nrm = v.nrm;
    o.tan = v.tan;
    o.uv = v.uv;
    o.clip = g.view_proj * vec4<f32>(v.pos + (g.eye.xyz - v.pos) * OVERLAY_TOWARD_EYE, 1.0);
    return o;
}

@fragment
fn fs_overlay(i: VOut) -> @location(0) vec4<f32> {
    return i.tan;
}

// The highlighted set's edges: four copies of each line, shifted to the corners of the extra width, make it wider than one pixel
@vertex
fn vs_wire_highlight(v: VIn, @builtin(instance_index) copy: u32) -> VOut {
    var o = wire3d(v);
    let corner = vec2<f32>(f32(copy & 1u), f32((copy >> 1u) & 1u)) * 2.0 - 1.0;
    o.clip = vec4<f32>(o.clip.xy + corner * g.highlight_shift.xy * o.clip.w, o.clip.zw);
    return o;
}

@fragment
fn fs_wire_highlight(i: VOut) -> @location(0) vec4<f32> {
    return g.highlight;
}
