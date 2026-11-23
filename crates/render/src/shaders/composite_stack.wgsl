// P3, stack plumbing and mask effects (gpu_composite_stack.rs). Owned by the P3 agent.
// Compiled into the composite shader when the program needs it, else replaced by the stub
// string in the Rust module (keep both signatures identical).

// A P3 opcode (144..152) on a scalar channel or a mask value.
fn stack_scalar_filter(op: Operation, x: f32) -> f32 {
    return x;
}

// A P3 opcode on a content channel; channel is the destination slot.
fn stack_effect_color(op: Operation, c: vec3<f32>, channel: u32, surface: Surface, baked_ao: f32) -> vec3<f32> {
    return c;
}

// The sixth word of a stage field (FilterStage::input): the input operation's value at the
// texel, written by prepare_filter in composite_spatial.wgsl.
fn stage_input_value(op: Operation, texel: LayerTexel) -> f32 {
    return base_content(op, texel).x;
}

// A stack opcode in the layer_content loop: the content after the effect, opacity applied. The
// default mixes stack_effect_color by the opacity; take over for coverage or texel reads.
fn stack_content_effect(effect: Operation, content: vec4<f32>, channel: u32, texel: LayerTexel) -> vec4<f32> {
    let filtered = stack_effect_color(effect, content.rgb, channel, texel.surface, texel.baked.x);
    return vec4(content.rgb + (filtered - content.rgb) * effect.low.w, content.a);
}

// A stack opcode in the layer_mask loop: the result the loop mixes in by the opacity.
fn stack_mask_effect(effect: Operation, mask: f32, texel: LayerTexel) -> f32 {
    return stack_scalar_filter(effect, mask);
}
