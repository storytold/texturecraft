// P6's Baked Lighting Stylized (gpu_composite_lighting.rs `stylized`)

// Side-table words: three lights of seven words (direction toward it, intensity, color), then the rest
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

// The constants of core's stylized_shading.rs
const LIGHTING_FLAT_CURVATURE: f32 = 0.5;
const LIGHTING_CURVATURE_SIDE_SCALE: f32 = 2.0;
const LIGHTING_COOK_TORRANCE_DENOMINATOR: f32 = 4.0;
const LIGHTING_MIN_ROUGHNESS: f32 = 0.08;
const LIGHTING_MIN_SMOOTHNESS: f32 = 1e-3;
const LIGHTING_MIN_LENGTH: f32 = 1e-6;
const LIGHTING_PI: f32 = 3.14159265358979;

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

// StylizedShading::shade of linear `albedo` facing `normal` with the baked AO and curvature in `baked`; `first` and `second` are the material's Metallic and Roughness, or Glossiness and Specular
fn lighting_stylized(effect: Operation, albedo: vec3<f32>, normal: vec3<f32>, baked: vec2<f32>, first: vec3<f32>, second: vec3<f32>) -> vec3<f32> {
    let flags = effect.ids.z;
    let ao = baked.x;
    let curvature = select(LIGHTING_FLAT_CURVATURE, baked.y, lighting_finite(baked.y));
    let cavity = lighting_fade_in((LIGHTING_FLAT_CURVATURE - curvature) * LIGHTING_CURVATURE_SIDE_SCALE, param(effect, LIGHTING_CAVITY_SMOOTHNESS));
    let compute_diffuse = (flags & LIGHTING_DIFFUSE) != 0u;
    let compute_specular = (flags & LIGHTING_SPECULAR) != 0u;
    var edge_light = 0.0;
    if compute_diffuse {
        let edge_amount = (curvature - LIGHTING_FLAT_CURVATURE) * LIGHTING_CURVATURE_SIDE_SCALE;
        edge_light = lighting_fade_in(edge_amount, param(effect, LIGHTING_EDGES_SMOOTHNESS)) * param(effect, LIGHTING_EDGES_INTENSITY);
    }

    var roughness: f32;
    var f0: vec3<f32>;
    var diffuse_albedo = albedo;
    if (flags & LIGHTING_SPECULAR_GLOSSINESS) != 0u {
        roughness = 1.0 - lighting_finite_unit(first.x);
        f0 = lighting_finite_unit3(second);
    } else {
        let metal = lighting_finite_unit(first.x);
        f0 = lighting_mix3(vec3(param(effect, LIGHTING_DIELECTRIC)), albedo, metal);
        var kept = 1.0;
        if compute_specular {
            kept = 1.0 - metal;
        }
        roughness = lighting_finite_unit(second.x);
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
