// P6, lighting and material-reading effects (gpu_composite_lighting.rs). Owned by the P6 agent.
// The dispatcher and helpers; each lighting kind's file follows in full or as its stub
// (gpu_composite_lighting.rs `source`), so a program compiles only the kinds it uses.

// Opcodes and flags as in gpu_composite_lighting.rs
const LIGHTING_STYLIZED: u32 = 152u;
const LIGHTING_ENVIRONMENT: u32 = 153u;
const LIGHTING_PBR_VALIDATE: u32 = 154u;
const LIGHTING_SPECULAR_GLOSSINESS: u32 = 1u;
const LIGHTING_DIFFUSE: u32 = 2u;
const LIGHTING_SPECULAR: u32 = 4u;
const LIGHTING_DETAIL_NORMAL: u32 = 8u;

// A P6 opcode (152..160) on a scalar channel. Masks skip lighting
fn lighting_scalar_filter(op: Operation, x: f32) -> f32 {
    return x;
}

// A P6 opcode on a content channel; channel is the destination slot (0 Base Color)
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

// A lighting opcode in the layer_content loop: the content after the effect, opacity applied. The effects light the base color only; the material and the lit normal are read once here
fn lighting_content_effect(effect: Operation, content: vec4<f32>, channel: u32, texel: LayerTexel) -> vec4<f32> {
    if channel != 0u {
        return content;
    }
    let kind = effect.ids.y;
    let material = stack_at(effect, texel.pixel);
    var lit = content.rgb;
    if kind == LIGHTING_PBR_VALIDATE {
        lit = lighting_pbr_validate(effect, content.rgb, material.x);
    } else {
        let normal = lighting_normal(effect, texel);
        let ao = select(1.0, clamp(texel.baked.x, 0.0, 1.0), lighting_finite(texel.baked.x));
        if kind == LIGHTING_STYLIZED {
            lit = lighting_stylized(effect, content.rgb, normal, vec2(ao, texel.baked.y), material, stack_at_second(effect, texel.pixel));
        } else if kind == LIGHTING_ENVIRONMENT {
            lit = lighting_environment(effect, content.rgb, normal, ao, material.x);
        }
    }
    return vec4(lighting_mix3(content.rgb, lit, effect.low.w), content.a);
}

// A lighting opcode in the layer_mask loop: the mask as it is (compile_mask skips lighting and PBR Validate, as the CPU does)
fn lighting_mask_effect(effect: Operation, mask: f32, texel: LayerTexel) -> f32 {
    return lighting_scalar_filter(effect, mask);
}
