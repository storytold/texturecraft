// P2, point filters and gradients (gpu_composite_point.rs). Owned by the P2 agent.
// Compiled into the composite shader when the program needs it, else replaced by the stub
// string in the Rust module (keep both signatures identical).

// A P2 opcode (128..144) on a scalar channel or a mask value.
fn point_scalar_filter(op: Operation, x: f32) -> f32 {
    return x;
}

// A P2 opcode on a content channel; channel is the destination slot (0 Base Color, 6 Height,
// 7 Normal, 8..10 colour lobes; channel_slot in Rust).
fn point_effect_color(op: Operation, c: vec3<f32>, channel: u32, surface: Surface, baked_ao: f32) -> vec3<f32> {
    return c;
}

// A point opcode in the layer_content loop: the content after the effect, opacity applied. The
// default mixes point_effect_color by the opacity; take over for coverage or texel reads.
fn point_content_effect(effect: Operation, content: vec4<f32>, channel: u32, texel: LayerTexel) -> vec4<f32> {
    let filtered = point_effect_color(effect, content.rgb, channel, texel.surface, texel.baked.x);
    return vec4(content.rgb + (filtered - content.rgb) * effect.low.w, content.a);
}

// A point opcode in the layer_mask loop: the result the loop mixes in by the opacity.
fn point_mask_effect(effect: Operation, mask: f32, texel: LayerTexel) -> f32 {
    return point_scalar_filter(effect, mask);
}
