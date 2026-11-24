// P6, lighting and material-reading effects (gpu_composite_lighting.rs). Owned by the P6 agent.
// Compiled into the composite shader when the program needs it, else replaced by the stub
// string in the Rust module (keep both signatures identical).

// Opcodes and flags as in gpu_composite_lighting.rs
const LIGHTING_STYLIZED: u32 = 152u;
const LIGHTING_ENVIRONMENT: u32 = 153u;
const LIGHTING_PBR_VALIDATE: u32 = 154u;
const LIGHTING_SPECULAR_GLOSSINESS: u32 = 1u;
const LIGHTING_DIFFUSE: u32 = 2u;
const LIGHTING_SPECULAR: u32 = 4u;
const LIGHTING_DETAIL_NORMAL: u32 = 8u;
const LIGHTING_CHECKS_ALBEDO: u32 = 1u;
const LIGHTING_CHECKS_METAL: u32 = 2u;

// Stylized side-table words (gpu_composite_lighting.rs `stylized`): three lights of seven words
const LIGHTING_LIGHT_WORDS: u32 = 7u;
const LIGHTING_SKY_INTENSITY: u32 = 21u;
const LIGHTING_SKY_COLOR: u32 = 22u;
const LIGHTING_HORIZON_COLOR: u32 = 25u;
const LIGHTING_GROUND_COLOR: u32 = 28u;
const LIGHTING_DIFFUSE_AO: u32 = 31u;
const LIGHTING_SPECULAR_AO: u32 = 32u;
const LIGHTING_DIFFUSE_CAVITY: u32 = 33u;
const LIGHTING_SPECULAR_CAVITY: u32 = 34u;
const LIGHTING_EDGES_INTENSITY: u32 = 35u;
const LIGHTING_CAVITY_SMOOTHNESS: u32 = 36u;
const LIGHTING_EDGES_SMOOTHNESS: u32 = 37u;
const LIGHTING_DIELECTRIC: u32 = 38u;
const LIGHTING_RELIEF: u32 = 39u;

// Environment side-table words: the environment block's offset, then the settings
const LIGHTING_ENVIRONMENT_BLOCK: u32 = 0u;
const LIGHTING_TURN: u32 = 1u;
const LIGHTING_TILT_SIN: u32 = 2u;
const LIGHTING_TILT_COS: u32 = 3u;
const LIGHTING_EXPOSURE: u32 = 4u;
const LIGHTING_ENVIRONMENT_RELIEF: u32 = 5u;
const LIGHTING_DIFFUSE_OCCLUSION: u32 = 6u;
const LIGHTING_SPECULAR_OCCLUSION: u32 = 7u;
const LIGHTING_ENVIRONMENT_DIELECTRIC: u32 = 8u;
// The environment block: nine RGB coefficients, then the viewport's rotation (radians)
const LIGHTING_SH_COEFFICIENTS: u32 = 9u;
const LIGHTING_SH_ROTATION: u32 = 27u;

// The constants of compose's eval.rs and core's stylized_shading.rs
const LIGHTING_HEIGHT_RELIEF: f32 = 0.03125;
const LIGHTING_MIN_NORMAL_LENGTH_SQUARED: f32 = 1e-6;
const LIGHTING_FLAT_CURVATURE: f32 = 0.5;
const LIGHTING_CURVATURE_SIDE_SCALE: f32 = 2.0;
const LIGHTING_COOK_TORRANCE_DENOMINATOR: f32 = 4.0;
const LIGHTING_MIN_ROUGHNESS: f32 = 0.08;
const LIGHTING_MIN_SMOOTHNESS: f32 = 1e-3;
const LIGHTING_MIN_LENGTH: f32 = 1e-6;
const LIGHTING_PI: f32 = 3.14159265358979;

// A P6 opcode (152..160) on a scalar channel. Masks skip lighting.
fn lighting_scalar_filter(op: Operation, x: f32) -> f32 {
    return x;
}

// A P6 opcode on a content channel; channel is the destination slot (0 Base Color).
fn lighting_effect_color(op: Operation, c: vec3<f32>, channel: u32, surface: Surface, baked_ao: f32) -> vec3<f32> {
    return c;
}

// Neither infinite nor not a number, as f32::is_finite
fn lighting_finite(x: f32) -> bool {
    return (bitcast<u32>(x) & 0x7f800000u) != 0x7f800000u;
}

// core's finite_unit: x clamped to 0..1, 0 when not finite
fn lighting_finite_unit(x: f32) -> f32 {
    return select(0.0, clamp(x, 0.0, 1.0), lighting_finite(x));
}

fn lighting_finite_unit3(v: vec3<f32>) -> vec3<f32> {
    return vec3(lighting_finite_unit(v.x), lighting_finite_unit(v.y), lighting_finite_unit(v.z));
}

// core's mix3: a + (b - a) w
fn lighting_mix3(a: vec3<f32>, b: vec3<f32>, w: f32) -> vec3<f32> {
    return a + (b - a) * w;
}

fn lighting_param3(effect: Operation, index: u32) -> vec3<f32> {
    return vec3(param(effect, index), param(effect, index + 1u), param(effect, index + 2u));
}

// The baked height at `pixel` from the bake rows
fn lighting_baked_height(pixel: vec2<u32>) -> f32 {
    return bitcast<f32>(baked_block_at(pixel, 1u).x);
}

// The baked height's change per UV unit along U and V at `pixel` (central differences inside the
// set), as the CPU's height_slope; a component that is not finite makes the caller skip the tilt
fn lighting_height_slope(pixel: vec2<u32>) -> vec2<f32> {
    let size = params.source.xy;
    let low = max(pixel, vec2(1u)) - 1u;
    let high = min(pixel + 1u, size - 1u);
    let across = lighting_baked_height(vec2(high.x, pixel.y)) - lighting_baked_height(vec2(low.x, pixel.y));
    let down = lighting_baked_height(vec2(pixel.x, high.y)) - lighting_baked_height(vec2(pixel.x, low.y));
    return vec2(across / f32(max(high.x - low.x, 1u)) * f32(size.x), down / f32(max(high.y - low.y, 1u)) * f32(size.y));
}

// The baked world normal (the mesh's without one) tilted by the baked height's slope times
// `relief` along the surface tangent, as the CPU's detail_normal
fn lighting_detail_normal(texel: LayerTexel, relief: f32) -> vec3<f32> {
    var normal = texel.surface.normal;
    if bake_has(BAKED_WORLD_NORMAL) {
        let detail = bitcast<vec3<f32>>(baked_block_at(texel.pixel, 1u).yzw) * 2.0 - 1.0;
        let length_squared = dot(detail, detail);
        if length_squared > LIGHTING_MIN_NORMAL_LENGTH_SQUARED {
            normal = detail * (1.0 / sqrt(length_squared));
        }
    }
    if !texel.surface.valid || !bake_has(BAKED_HEIGHT) || any(texel.pixel >= params.source.xy) {
        return normal;
    }

    let slope = lighting_height_slope(texel.pixel);
    if !lighting_finite(slope.x) || !lighting_finite(slope.y) {
        return normal;
    }
    let tangent = texel.surface.tangent.xyz;
    let bitangent = cross(normal, tangent) * texel.surface.tangent.w;
    let tilted = normal - (tangent * slope.x + bitangent * slope.y) * LIGHTING_HEIGHT_RELIEF * relief;
    let reciprocal = 1.0 / sqrt(dot(tilted, tilted));
    if lighting_finite(reciprocal) && reciprocal > 0.0 {
        return tilted * reciprocal;
    }
    return normal;
}

// `amount` (clamped 0..1) reaching full over `smoothness`: a step at or below the step limit
fn lighting_fade_in(amount: f32, smoothness: f32) -> f32 {
    let clamped = clamp(amount, 0.0, 1.0);
    if smoothness <= LIGHTING_MIN_SMOOTHNESS {
        return select(0.0, 1.0, clamped > 0.0);
    }
    return min(clamped / smoothness, 1.0);
}

// The ambient light from the sky, horizon and ground reaching a surface facing `normal`
fn lighting_ambient(effect: Operation, normal: vec3<f32>) -> vec3<f32> {
    let up = clamp(normal.y, -1.0, 1.0);
    let horizon = lighting_param3(effect, LIGHTING_HORIZON_COLOR);
    if up >= 0.0 {
        return lighting_mix3(horizon, lighting_param3(effect, LIGHTING_SKY_COLOR), up);
    }
    return lighting_mix3(horizon, lighting_param3(effect, LIGHTING_GROUND_COLOR), -up);
}

// The GGX distribution of microfacets facing `normal_dot_half` at `roughness`
fn lighting_ggx(normal_dot_half: f32, roughness: f32) -> f32 {
    let squared = roughness * roughness;
    let alpha_squared = squared * squared;
    let denominator = normal_dot_half * normal_dot_half * (alpha_squared - 1.0) + 1.0;
    return alpha_squared / (LIGHTING_PI * denominator * denominator);
}

// Schlick's Fresnel of reflectance `f0` at cosine `cosine`
fn lighting_schlick(f0: vec3<f32>, cosine: f32) -> vec3<f32> {
    let rest = 1.0 - clamp(cosine, 0.0, 1.0);
    let fifth = rest * rest * rest * rest * rest;
    return f0 + (1.0 - f0) * fifth;
}

// `v` at unit length; a vector too short to have a direction points up
fn lighting_normalize(v: vec3<f32>) -> vec3<f32> {
    let magnitude = sqrt(dot(v, v));
    if magnitude > LIGHTING_MIN_LENGTH {
        return v / magnitude;
    }
    return vec3(0.0, 1.0, 0.0);
}

fn lighting_dot(a: vec3<f32>, b: vec3<f32>) -> f32 {
    return a.x * b.x + a.y * b.y + a.z * b.z;
}

// StylizedShading::shade of linear `albedo` at the texel, its material from the stack reads
fn lighting_stylized(effect: Operation, albedo: vec3<f32>, texel: LayerTexel) -> vec3<f32> {
    let flags = effect.ids.z;
    var normal = texel.surface.normal;
    if (flags & LIGHTING_DETAIL_NORMAL) != 0u {
        normal = lighting_detail_normal(texel, param(effect, LIGHTING_RELIEF));
    }
    let ao = select(1.0, clamp(texel.baked.x, 0.0, 1.0), lighting_finite(texel.baked.x));
    let curvature = select(LIGHTING_FLAT_CURVATURE, texel.baked.y, lighting_finite(texel.baked.y));
    let cavity = lighting_fade_in((LIGHTING_FLAT_CURVATURE - curvature) * LIGHTING_CURVATURE_SIDE_SCALE, param(effect, LIGHTING_CAVITY_SMOOTHNESS));
    let compute_diffuse = (flags & LIGHTING_DIFFUSE) != 0u;
    let compute_specular = (flags & LIGHTING_SPECULAR) != 0u;
    var edge_light = 0.0;
    if compute_diffuse {
        let edge_amount = (curvature - LIGHTING_FLAT_CURVATURE) * LIGHTING_CURVATURE_SIDE_SCALE;
        edge_light = lighting_fade_in(edge_amount, param(effect, LIGHTING_EDGES_SMOOTHNESS)) * param(effect, LIGHTING_EDGES_INTENSITY);
    }

    // Highlights read Glossiness and Specular, or Metallic and Roughness
    var roughness: f32;
    var f0: vec3<f32>;
    var diffuse_albedo = albedo;
    if (flags & LIGHTING_SPECULAR_GLOSSINESS) != 0u {
        roughness = 1.0 - lighting_finite_unit(stack_at(effect, texel.pixel).x);
        f0 = lighting_finite_unit3(stack_at_second(effect, texel.pixel));
    } else {
        let metal = lighting_finite_unit(stack_at(effect, texel.pixel).x);
        f0 = lighting_mix3(vec3(param(effect, LIGHTING_DIELECTRIC)), albedo, metal);
        var kept = 1.0;
        if compute_specular {
            kept = 1.0 - metal;
        }
        roughness = lighting_finite_unit(stack_at_second(effect, texel.pixel).x);
        diffuse_albedo = albedo * kept;
    }
    roughness = max(roughness, LIGHTING_MIN_ROUGHNESS);
    let sky = lighting_ambient(effect, normal);
    let sky_intensity = param(effect, LIGHTING_SKY_INTENSITY);
    let diffuse_ao = 1.0 + (ao - 1.0) * param(effect, LIGHTING_DIFFUSE_AO);
    let specular_ao = 1.0 + (ao - 1.0) * param(effect, LIGHTING_SPECULAR_AO);
    let diffuse_cavity = 1.0 - cavity * param(effect, LIGHTING_DIFFUSE_CAVITY);
    let specular_cavity = 1.0 - cavity * param(effect, LIGHTING_SPECULAR_CAVITY);

    var diffuse = vec3(0.0);
    if compute_diffuse {
        diffuse = sky * sky_intensity * diffuse_ao * diffuse_cavity;
    }
    var specular = vec3(0.0);
    for (var light = 0u; light < 3u; light++) {
        let base = light * LIGHTING_LIGHT_WORDS;
        let direction = lighting_param3(effect, base);
        let intensity = param(effect, base + 3u);
        let color = lighting_param3(effect, base + 4u);
        let facing = lighting_dot(normal, direction);
        if facing <= 0.0 || intensity <= 0.0 {
            continue;
        }

        if compute_diffuse {
            diffuse += color * intensity * facing * diffuse_cavity;
        }
        if compute_specular {
            let halfway = lighting_normalize(normal + direction);
            let normal_dot_half = max(lighting_dot(normal, halfway), 0.0);
            let highlight = lighting_ggx(normal_dot_half, roughness) / LIGHTING_COOK_TORRANCE_DENOMINATOR * facing * specular_cavity;
            specular += color * intensity * highlight * lighting_schlick(f0, normal_dot_half);
        }
    }
    if compute_specular {
        specular += f0 * sky * sky_intensity * specular_ao * specular_cavity;
    }

    var lit = vec3(0.0);
    if compute_diffuse {
        lit = diffuse_albedo * diffuse;
    }
    let shaded = lighting_finite_unit3(lit + specular);
    return lighting_finite_unit3(shaded + (1.0 - shaded) * edge_light);
}

// The environment's diffuse light reaching a surface facing `normal`, tilted and turned as
// EnvironmentSh::irradiance does, from the late-bound block at `block`
fn lighting_irradiance(effect: Operation, block: u32, normal: vec3<f32>) -> vec3<f32> {
    let sin_tilt = param(effect, LIGHTING_TILT_SIN);
    let cos_tilt = param(effect, LIGHTING_TILT_COS);
    let tilted = vec3(normal.x, cos_tilt * normal.y - sin_tilt * normal.z, sin_tilt * normal.y + cos_tilt * normal.z);
    let turn = bitcast<f32>(srgb[block + LIGHTING_SH_ROTATION]) + param(effect, LIGHTING_TURN);
    let sin_turn = sin(turn);
    let cos_turn = cos(turn);
    let d = vec3(cos_turn * tilted.x - sin_turn * tilted.z, tilted.y, sin_turn * tilted.x + cos_turn * tilted.z);

    // environment_sky.rs sh_basis
    var basis = array<f32, 9>(
        0.282095,
        0.488603 * d.y,
        0.488603 * d.z,
        0.488603 * d.x,
        1.092548 * d.x * d.y,
        1.092548 * d.y * d.z,
        0.315392 * (3.0 * d.z * d.z - 1.0),
        1.092548 * d.x * d.z,
        0.546274 * (d.x * d.x - d.y * d.y),
    );
    var light = vec3(0.0);
    for (var i = 0u; i < LIGHTING_SH_COEFFICIENTS; i++) {
        let at = block + i * 3u;
        light += bitcast<vec3<f32>>(vec3(srgb[at], srgb[at + 1u], srgb[at + 2u])) * basis[i];
    }
    return max(light, vec3(0.0));
}

// Baked Lighting Environment's lit base color (EnvironmentSh::shade) at the texel
fn lighting_environment(effect: Operation, albedo: vec3<f32>, texel: LayerTexel) -> vec3<f32> {
    let normal = lighting_detail_normal(texel, param(effect, LIGHTING_ENVIRONMENT_RELIEF));
    let incoming = lighting_irradiance(effect, param_word(effect, LIGHTING_ENVIRONMENT_BLOCK), normal) * param(effect, LIGHTING_EXPOSURE);
    let ao = select(1.0, clamp(texel.baked.x, 0.0, 1.0), lighting_finite(texel.baked.x));
    let metal = lighting_finite_unit(stack_at(effect, texel.pixel).x);
    let f0 = lighting_mix3(vec3(param(effect, LIGHTING_ENVIRONMENT_DIELECTRIC)), albedo, metal);
    let diffuse = 1.0 + (ao - 1.0) * param(effect, LIGHTING_DIFFUSE_OCCLUSION);
    let specular = 1.0 + (ao - 1.0) * param(effect, LIGHTING_SPECULAR_OCCLUSION);
    return lighting_finite_unit3(albedo * (1.0 - metal) * incoming * diffuse + f0 * incoming * specular);
}

// The sRGB value of linear v, as the CPU's linear_to_srgb
fn lighting_linear_to_srgb(v: f32) -> f32 {
    let x = clamp(v, 0.0, 1.0);
    if x <= 0.0031308 {
        return x * 12.92;
    }
    return 1.055 * pow(x, 1.0 / 2.4) - 0.055;
}

// PbrValidate::check: the color when valid or not checked, the invalid color (high.xyz) when not.
// low.xyz: darkest valid non-metal, dimmest valid metal, brightest valid non-metal (sRGB);
// high.w: the metal threshold
fn lighting_pbr_validate(effect: Operation, color: vec3<f32>, metal: f32) -> vec3<f32> {
    let srgb_value = lighting_linear_to_srgb(clamp(luminance(color), 0.0, 1.0));
    var valid: bool;
    if metal >= effect.high.w {
        valid = (effect.ids.z & LIGHTING_CHECKS_METAL) == 0u || srgb_value >= effect.low.y;
    } else {
        valid = (effect.ids.z & LIGHTING_CHECKS_ALBEDO) == 0u || (srgb_value >= effect.low.x && srgb_value <= effect.low.z);
    }
    return select(effect.high.xyz, color, valid);
}

// A lighting opcode in the layer_content loop: the content after the effect, opacity applied.
// The effects light the base color only and read the layer's material at the texel.
fn lighting_content_effect(effect: Operation, content: vec4<f32>, channel: u32, texel: LayerTexel) -> vec4<f32> {
    if channel != 0u {
        return content;
    }
    var lit = content.rgb;
    if effect.ids.y == LIGHTING_STYLIZED {
        lit = lighting_stylized(effect, content.rgb, texel);
    } else if effect.ids.y == LIGHTING_ENVIRONMENT {
        lit = lighting_environment(effect, content.rgb, texel);
    } else if effect.ids.y == LIGHTING_PBR_VALIDATE {
        lit = lighting_pbr_validate(effect, content.rgb, stack_at(effect, texel.pixel).x);
    }
    return vec4(lighting_mix3(content.rgb, lit, effect.low.w), content.a);
}

// A lighting opcode in the layer_mask loop: the result the loop mixes in by the opacity. Masks
// skip lighting and PBR Validate (compile_mask), as the CPU does.
fn lighting_mask_effect(effect: Operation, mask: f32, texel: LayerTexel) -> f32 {
    return lighting_scalar_filter(effect, mask);
}
