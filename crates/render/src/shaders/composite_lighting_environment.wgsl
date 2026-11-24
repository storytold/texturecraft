// P6's Baked Lighting Environment (gpu_composite_lighting.rs `lower`)

// Side-table words: the environment block's offset, then the settings
const LIGHTING_ENVIRONMENT_BLOCK: u32 = 0u;
const LIGHTING_TURN: u32 = 1u;
const LIGHTING_TILT_SIN: u32 = 2u;
const LIGHTING_TILT_COS: u32 = 3u;
const LIGHTING_EXPOSURE: u32 = 4u;
const LIGHTING_DIFFUSE_OCCLUSION: u32 = 5u;
const LIGHTING_SPECULAR_OCCLUSION: u32 = 6u;
const LIGHTING_ENVIRONMENT_DIELECTRIC: u32 = 7u;
// The environment block: nine RGB coefficients, then the viewport's rotation (radians)
const LIGHTING_SH_ROTATION: u32 = 27u;

// Coefficient `index` of the environment block at `block`
fn lighting_sh(block: u32, index: u32) -> vec3<f32> {
    let at = block + index * 3u;
    return bitcast<vec3<f32>>(vec3(srgb[at], srgb[at + 1u], srgb[at + 2u]));
}

// The environment's diffuse light reaching a surface facing `normal`, tilted and turned as EnvironmentSh::irradiance does
fn lighting_irradiance(effect: Operation, block: u32, normal: vec3<f32>) -> vec3<f32> {
    let sin_tilt = param(effect, LIGHTING_TILT_SIN);
    let cos_tilt = param(effect, LIGHTING_TILT_COS);
    let tilted = vec3(normal.x, cos_tilt * normal.y - sin_tilt * normal.z, sin_tilt * normal.y + cos_tilt * normal.z);
    let turn = bitcast<f32>(srgb[block + LIGHTING_SH_ROTATION]) + param(effect, LIGHTING_TURN);
    let sin_turn = sin(turn);
    let cos_turn = cos(turn);
    let d = vec3(cos_turn * tilted.x - sin_turn * tilted.z, tilted.y, sin_turn * tilted.x + cos_turn * tilted.z);

    // environment_sky.rs sh_basis, summed in its order
    var light = lighting_sh(block, 0u) * 0.282095;
    light += lighting_sh(block, 1u) * (0.488603 * d.y);
    light += lighting_sh(block, 2u) * (0.488603 * d.z);
    light += lighting_sh(block, 3u) * (0.488603 * d.x);
    light += lighting_sh(block, 4u) * (1.092548 * d.x * d.y);
    light += lighting_sh(block, 5u) * (1.092548 * d.y * d.z);
    light += lighting_sh(block, 6u) * (0.315392 * (3.0 * d.z * d.z - 1.0));
    light += lighting_sh(block, 7u) * (1.092548 * d.x * d.z);
    light += lighting_sh(block, 8u) * (0.546274 * (d.x * d.x - d.y * d.y));
    return max(light, vec3(0.0));
}

// EnvironmentSh::shade of linear `albedo` facing `normal` with the baked AO and the material's metal
fn lighting_environment(effect: Operation, albedo: vec3<f32>, normal: vec3<f32>, ao: f32, metal_read: f32) -> vec3<f32> {
    let incoming = lighting_irradiance(effect, param_word(effect, LIGHTING_ENVIRONMENT_BLOCK), normal) * param(effect, LIGHTING_EXPOSURE);
    let metal = lighting_finite_unit(metal_read);
    let f0 = lighting_mix3(vec3(param(effect, LIGHTING_ENVIRONMENT_DIELECTRIC)), albedo, metal);
    let diffuse = 1.0 + (ao - 1.0) * param(effect, LIGHTING_DIFFUSE_OCCLUSION);
    let specular = 1.0 + (ao - 1.0) * param(effect, LIGHTING_SPECULAR_OCCLUSION);
    return lighting_finite_unit3(albedo * (1.0 - metal) * incoming * diffuse + f0 * incoming * specular);
}
