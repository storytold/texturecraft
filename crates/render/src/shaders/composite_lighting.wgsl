// P6, lighting and material-reading effects (gpu_composite_lighting.rs). Owned by the P6 agent.
// Compiled into the composite shader when the program needs it, else replaced by the stub
// string in the Rust module (keep both signatures identical).

const LIGHTING_PBR_VALIDATE: u32 = 154u;
// PBR Validate's mode flags in ids.z
const LIGHTING_CHECKS_ALBEDO: u32 = 1u;
const LIGHTING_CHECKS_METAL: u32 = 2u;

// A P6 opcode (152..160) on a scalar channel. Masks skip lighting.
fn lighting_scalar_filter(op: Operation, x: f32) -> f32 {
    return x;
}

// A P6 opcode on a content channel; channel is the destination slot (0 Base Color).
fn lighting_effect_color(op: Operation, c: vec3<f32>, channel: u32, surface: Surface, baked_ao: f32) -> vec3<f32> {
    return c;
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
    let srgb = lighting_linear_to_srgb(clamp(luminance(color), 0.0, 1.0));
    var valid: bool;
    if metal >= effect.high.w {
        valid = (effect.ids.z & LIGHTING_CHECKS_METAL) == 0u || srgb >= effect.low.y;
    } else {
        valid = (effect.ids.z & LIGHTING_CHECKS_ALBEDO) == 0u || (srgb >= effect.low.x && srgb <= effect.low.z);
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
    if effect.ids.y == LIGHTING_PBR_VALIDATE {
        lit = lighting_pbr_validate(effect, content.rgb, stack_at(effect, texel.pixel).x);
    }
    return vec4(content.rgb + (lit - content.rgb) * effect.low.w, content.a);
}

// A lighting opcode in the layer_mask loop: the result the loop mixes in by the opacity. A mask
// has no base color, so PBR Validate leaves it as it is, as on the CPU.
fn lighting_mask_effect(effect: Operation, mask: f32, texel: LayerTexel) -> f32 {
    return lighting_scalar_filter(effect, mask);
}
