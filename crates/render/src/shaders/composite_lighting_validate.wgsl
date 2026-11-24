// P6's PBR Validate (gpu_composite_lighting.rs `lower`)

// Its mode flags in ids.z
const LIGHTING_CHECKS_ALBEDO: u32 = 1u;
const LIGHTING_CHECKS_METAL: u32 = 2u;

// The sRGB value of linear v, as the CPU's linear_to_srgb
fn lighting_linear_to_srgb(v: f32) -> f32 {
    let x = clamp(v, 0.0, 1.0);
    if x <= 0.0031308 {
        return x * 12.92;
    }
    return 1.055 * pow(x, 1.0 / 2.4) - 0.055;
}

// PbrValidate::check: the color when valid or not checked, the invalid color (high.xyz) when not (low.xyz: darkest valid non-metal, dimmest valid metal, brightest valid non-metal; high.w: the metal threshold)
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
