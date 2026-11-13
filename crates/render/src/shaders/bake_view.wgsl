// Bake mode's 3D view: the high poly, the cage around the low poly, the missing seams and the neutral material

struct BakeViewOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // 1 where the bake's rays miss the high poly, 0 where they reach it, negative where another texture set owns it (hidden)
    @location(2) missed: f32,
};

// Reflectance of the neutral material's dielectric part facing the camera
const NEUTRAL_F0: f32 = 0.04;

// Lowest roughness the neutral material shades with: below it the GGX lobe is too narrow for its samples
const MIN_NEUTRAL_ROUGHNESS: f32 = 0.045;

// How fast reflections fade as they dip below the mesh surface (larger = sooner)
const HORIZON_FADE: f32 = 1.3;

// Floor of the cosines that divide the reflection terms, so grazing angles stay finite
const MIN_COSINE: f32 = 1e-4;

// A normal closer than this to +-Z builds its tangent frame from X, where the cross product with Z would vanish
const TANGENT_HELPER_MAX_Z: f32 = 0.999;

// One mip coarser than each sample's share of the lobe: hides the gaps between few samples (filtered importance sampling)
const FILTERED_SAMPLING_LOD_BIAS: f32 = 1.0;

// A packed bent normal shorter than this is the unbaked gray, not a direction
const BENT_NORMAL_MIN_LENGTH: f32 = 0.5;

// Cosines between a reflection and the bent normal over which the reflection goes from occluded to open
const BENT_OCCLUSION_START: f32 = -0.25;
const BENT_OCCLUSION_END: f32 = 0.75;

// Floor of the GGX distribution's denominator, which reaches 0 at the peak of a mirror-like lobe
const MIN_GGX_DENOMINATOR: f32 = 1e-6;

// Fractional part of the golden ratio: shifts each accumulated frame's samples around the lobe
const GOLDEN_RATIO_FRACTION: f32 = 0.618034;

// High-poly surface where the interpolated `missed` falls below this belongs to a hidden texture set
const HIDDEN_BELOW: f32 = -0.5;

// Brightness of surfaces seen edge-on; facing the camera adds the rest
const HEADLIGHT_AMBIENT: f32 = 0.3;

// Fractions of the way to the camera each vertex moves, so near-coincident surfaces and lines draw in a fixed order
const HIGH_POLY_PULL_TOWARD_EYE: f32 = 0.001;
const CAGE_WIRE_PULL_TOWARD_EYE: f32 = 0.002;

// Lit from the camera: surfaces facing it are brightest
fn headlight(color: vec3<f32>, normal: vec3<f32>, world_position: vec3<f32>) -> vec3<f32> {
    let facing = abs(dot(normal, normalize(g.eye.xyz - world_position)));
    return color * (HEADLIGHT_AMBIENT + (1.0 - HEADLIGHT_AMBIENT) * facing);
}

fn pulled_toward_eye(world_position: vec3<f32>, fraction: f32) -> vec4<f32> {
    return g.view_proj * vec4<f32>(world_position + (g.eye.xyz - world_position) * fraction, 1.0);
}

// Pulled slightly toward the camera: where the high poly nearly touches the low poly it shows instead of flickering with it
@vertex
fn vs_bake_high(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(4) missed: f32) -> BakeViewOut {
    var out: BakeViewOut;
    out.clip = pulled_toward_eye(position, HIGH_POLY_PULL_TOWARD_EYE);
    out.world_position = position;
    out.normal = normal;
    out.missed = missed;
    return out;
}

@fragment
fn fs_bake_high(surface: BakeViewOut) -> @location(0) vec4<f32> {
    if (surface.missed < HIDDEN_BELOW) {
        discard;
    }

    let color = mix(g.bake_high.rgb, g.bake_matching_error.rgb, max(surface.missed, 0.0) * g.bake_matching_error.a);
    return vec4<f32>(headlight(color, normalize(surface.normal), surface.world_position), 1.0);
}

@vertex
fn vs_bake_cage(@location(0) position: vec3<f32>) -> BakeViewOut {
    var out: BakeViewOut;
    out.clip = g.view_proj * vec4<f32>(position, 1.0);
    out.world_position = position;
    out.normal = vec3<f32>(0.0, 1.0, 0.0);
    out.missed = 0.0;
    return out;
}

// The cage's own faces: their normal comes from the screen-space change of the position
@fragment
fn fs_bake_cage(surface: BakeViewOut) -> @location(0) vec4<f32> {
    let face_normal = normalize(cross(dpdx(surface.world_position), dpdy(surface.world_position)));
    return vec4<f32>(headlight(g.bake_cage.rgb, face_normal, surface.world_position), g.bake_cage.a);
}

// Pulled toward the camera like the 3D wireframe, so the lines win against the cage's faces
@vertex
fn vs_bake_cage_wire(@location(0) position: vec3<f32>) -> BakeViewOut {
    var out: BakeViewOut;
    out.clip = pulled_toward_eye(position, CAGE_WIRE_PULL_TOWARD_EYE);
    out.world_position = position;
    out.normal = vec3<f32>(0.0, 1.0, 0.0);
    out.missed = 0.0;
    return out;
}

@fragment
fn fs_wire_bake_cage(surface: BakeViewOut) -> @location(0) vec4<f32> {
    return g.bake_cage_wire;
}

@fragment
fn fs_wire_missing_seam(surface: VOut) -> @location(0) vec4<f32> {
    return g.bake_missing_seam;
}

// The missing seams along the cage, whose vertices carry the bake view's outputs
@fragment
fn fs_wire_cage_missing_seam(surface: BakeViewOut) -> @location(0) vec4<f32> {
    return g.bake_missing_seam;
}

// The neutral material at the texture set's normal `n` (over the mesh normal `n_geo`) seen from direction `v`, darkened by its occlusion `ao_channel`, lit along the packed bent normal `packed_bent` when it is on
fn neutral_color(n: vec3<f32>, n_geo: vec3<f32>, v: vec3<f32>, world_position: vec3<f32>, ao_channel: f32, packed_bent: vec3<f32>) -> vec4<f32> {
    let albedo = g.bake_neutral_color.rgb;
    let rough = clamp(g.bake_neutral.x, MIN_NEUTRAL_ROUGHNESS, 1.0);
    let metal = g.bake_neutral.y;
    let ao = mix(1.0, ao_channel, g.bake_neutral.z);
    let f0 = mix(vec3<f32>(NEUTRAL_F0), albedo, metal);
    let diffuse_col = albedo * (1.0 - metal);
    let nov = clamp(abs(dot(n, v)), MIN_COSINE, 1.0);

    // An unbaked bent normal reads as the neutral gray (length 0): the surface normal stands in
    var bent = packed_bent * 2.0 - 1.0;
    bent = select(n, normalize(bent), dot(bent, bent) > BENT_NORMAL_MIN_LENGTH * BENT_NORMAL_MIN_LENGTH);
    let diffuse = sh_irradiance(normalize(mix(n, bent, g.bake_bent_normal.x)), vec3<f32>(0.0)) * diffuse_col * ao;
    let r = reflect(-v, n);
    let horizon = clamp(1.0 + HORIZON_FADE * dot(r, n_geo), 0.0, 1.0);
    // Reflections pointing away from the open direction are occluded
    let bent_occlusion = mix(1.0, smoothstep(BENT_OCCLUSION_START, BENT_OCCLUSION_END, dot(bent, r)), g.bake_bent_normal.y);
    let spec = sampled_reflection(n, v, rough, f0, nov) * ao * bent_occlusion * horizon * horizon;

    var direct = vec3<f32>(0.0);
    if (g.light.w > 0.0) {
        let l = normalize(g.light.xyz);
        let h = normalize(l + v);
        let nol = clamp(dot(n, l), 0.0, 1.0);
        let f = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - clamp(dot(v, h), 0.0, 1.0), 5.0);
        direct = ((vec3<f32>(1.0) - f) * diffuse_col / PI + ggx_dv(rough * rough, nov, nol, clamp(dot(n, h), 0.0, 1.0)) * f) * nol * g.light.w;
        if (g.shadow.x > 0.5 && nol > 0.0) {
            direct *= shadow_factor(world_position, n_geo);
        }
    }
    let color = diffuse + spec + direct;
    return vec4<f32>(select(color, vec3<f32>(0.0), any(color != color)), 1.0);
}

// The environment's GGX reflection at normal `n`, importance sampled with this frame's share of the neutral material's quality; each accumulated frame takes other samples
fn sampled_reflection(n: vec3<f32>, v: vec3<f32>, rough: f32, f0: vec3<f32>, nov: f32) -> vec3<f32> {
    let count = max(u32(g.bake_neutral_color.w), 1u);
    let a = rough * rough;
    let a2 = a * a;
    let env_size = vec2<f32>(textureDimensions(env_tex));
    let texel_solid_angle = 4.0 * PI / (env_size.x * env_size.y);
    let side = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n.z) < TANGENT_HELPER_MAX_Z);
    let tangent = normalize(cross(side, n));
    let bitangent = cross(n, tangent);
    let frame_offset = u32(g.material.z) * count;
    // Schlick-GGX geometry term at k = a / 2
    let k = a * 0.5;
    let view_visibility = nov / (nov * (1.0 - k) + k);

    var sum = vec3<f32>(0.0);
    for (var index = 0u; index < count; index++) {
        let xi = vec2<f32>(fract((f32(index) + 0.5) / f32(count) + f32(g.material.z) * GOLDEN_RATIO_FRACTION), radical_inverse(index + frame_offset));
        let phi = 2.0 * PI * xi.x;
        // Clamped: near roughness 0 the ratio rounds a hair past 1, and the sine's square root would be NaN (a black pixel once frames accumulate)
        let cos_theta = clamp(sqrt((1.0 - xi.y) / (1.0 + (a2 - 1.0) * xi.y)), 0.0, 1.0);
        let sin_theta = sqrt(1.0 - cos_theta * cos_theta);
        let h = normalize(tangent * (cos(phi) * sin_theta) + bitangent * (sin(phi) * sin_theta) + n * cos_theta);
        let l = reflect(-v, h);
        let nol = min(dot(n, l), 1.0);
        if (nol <= 0.0) {
            continue;
        }

        // Capped at 1: a dot product rounding past it makes pow(1 - voh, 5) NaN on GPUs that compute pow through log2
        let noh = clamp(dot(n, h), MIN_COSINE, 1.0);
        let voh = clamp(dot(v, h), MIN_COSINE, 1.0);
        let denominator = max(noh * noh * (a2 - 1.0) + 1.0, MIN_GGX_DENOMINATOR);
        let pdf = a2 / (PI * denominator * denominator) * noh / (4.0 * voh);
        // Filtered importance sampling: each sample reads the mip that covers its share of the lobe
        let lod = clamp(0.5 * log2(1.0 / (f32(count) * pdf * texel_solid_angle)) + FILTERED_SAMPLING_LOD_BIAS, 0.0, g.env.y);
        let light = textureSampleLevel(env_tex, env_samp, env_uv(l), lod).rgb;
        let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - voh, 5.0);
        let visibility = view_visibility * nol / (nol * (1.0 - k) + k);
        let contribution = light * fresnel * (visibility * voh / (noh * nov));
        // A sample that still came out NaN (x != x) is dropped: the frame average would carry it, and post effects spread it over the image
        sum += select(contribution, vec3<f32>(0.0), any(contribution != contribution));
    }
    return sum / f32(count);
}

// Van der Corput: the bits of `index` mirrored behind the binary point (swapped by hand: WebGL2 lacks a bit reverse)
fn radical_inverse(index: u32) -> f32 {
    var bits = (index << 16u) | (index >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return f32(bits) / 4294967296.0;
}
