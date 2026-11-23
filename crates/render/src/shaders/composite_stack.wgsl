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

// A mask comparison with a soft edge (CompareOp::mask): low.z is the ramp's width past the edge.
fn stack_soft_compare(op: Operation, mask: f32) -> f32 {
    let width = op.low.z;
    if op.ids.z == 0u {
        return clamp((op.low.x - mask) / width, 0.0, 1.0);
    }
    if op.ids.z == 2u {
        return clamp((mask - op.low.x) / width, 0.0, 1.0);
    }
    return clamp(1.0 + (op.low.y - abs(mask - op.low.x)) / width, 0.0, 1.0);
}

// A colour selection's gray and how much of it covers the mask below (ColorSelection::value).
// The side table holds the output, the edge width and the background (below 0: transparent).
fn stack_color_selection(effect: Operation, pixel: vec2<u32>) -> vec2<f32> {
    let output = param(effect, 0u);
    let width = param(effect, 1u);
    let background = param(effect, 2u);
    var amount = 0.0;
    if params.id_bake.x != 0u && effect.procedural.z > 0u {
        let color = id_color_at(pixel);
        var distance = 3.402823466e38;
        for (var i = 0u; i < effect.procedural.z; i++) {
            let at = effect.procedural.y + i * 3u;
            let picked = bitcast<vec3<f32>>(vec3(srgb[at], srgb[at + 1u], srgb[at + 2u]));
            let gap = abs(picked - color);
            distance = min(distance, max(gap.x, max(gap.y, gap.z)));
        }
        let past = distance - effect.low.x;
        if past <= 0.0 {
            amount = 1.0;
        } else if width > 0.0 {
            amount = clamp(1.0 - past / width, 0.0, 1.0);
        }
    }
    if background < 0.0 {
        return vec2(output, amount);
    }
    return vec2(background + (output - background) * amount, 1.0);
}
