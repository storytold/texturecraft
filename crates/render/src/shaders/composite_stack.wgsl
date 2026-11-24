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
    if effect.ids.y == STACK_LAYER_COMPARE {
        // A comparison blends its pass or fail into the mask like a fill
        return blend_scalar(u32(effect.balance.z), mask, stack_layer_compare(effect, mask, texel));
    }
    return stack_scalar_filter(effect, mask);
}

// A mask comparison reading layers (LAYER_COMPARE in gpu_composite_stack.rs)
const STACK_LAYER_COMPARE: u32 = 144u;

// One side of a layer comparison (compare_side in the CPU's eval.rs), its code in four bits of
// ids.w: 0 the mask so far, 1 the constant, 2 the layers below, 3 this layer over them
fn stack_compare_side(op: Operation, side: u32, mask: f32, texel: LayerTexel) -> f32 {
    if side == 0u {
        return mask;
    }
    if side == 1u {
        return op.low.x;
    }

    // Where the layer is empty or thin it shows the stack below it
    var read = stack_at(op, texel.pixel);
    if side == 3u {
        let own = this_layer_content(op, texel);
        read = read + (own.rgb - read) * own.a;
    }

    // The channel's kind in ids.w from bit 8: colour, scalar, signed, normal map
    let kind = (op.ids.w >> 8u) & 255u;
    if kind == 1u {
        return read.x;
    }
    if kind == 2u {
        return read.x * 0.5 + 0.5;
    }
    return luminance(read);
}

// A comparison of two sides (CompareOp::mask): ids.z the comparison, low.y the tolerance, low.z
// the ramp's width past the edge (0: hard)
fn stack_layer_compare(op: Operation, mask: f32, texel: LayerTexel) -> f32 {
    let left = stack_compare_side(op, op.ids.w & 15u, mask, texel);
    let right = stack_compare_side(op, (op.ids.w >> 4u) & 15u, mask, texel);
    var past = op.low.y - abs(left - right);
    if op.ids.z == 0u {
        past = right - left;
    }
    if op.ids.z == 2u {
        past = left - right;
    }
    let width = op.low.z;
    if width <= 0.0 {
        let passes = select(past > 0.0, past >= 0.0, op.ids.z == 1u);
        return select(0.0, 1.0, passes);
    }
    var ramp = past / width;
    if op.ids.z == 1u {
        ramp += 1.0;
    }
    return clamp(ramp, 0.0, 1.0);
}

// A mask comparison with a soft edge (CompareOp::mask), low.z the ramp's width past the edge
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

// The ID map flag of a colour selection (ID_MAP_IMAGE in gpu_composite_stack.rs)
const STACK_ID_MAP_IMAGE: u32 = 0x00000200u;

// A colour selection's gray and cover (ColorSelection::value): side table image record, output,
// edge width and background (below 0: transparent)
fn stack_color_selection(effect: Operation, texel: LayerTexel) -> vec2<f32> {
    let output = param(effect, 1u);
    let width = param(effect, 2u);
    let background = param(effect, 3u);

    // The ID colour: the image's stored texel, or the baked map's; none where either is missing
    var has_id = params.id_bake.x != 0u;
    var color = vec3(0.0);
    if (effect.ext.z & STACK_ID_MAP_IMAGE) != 0u {
        let record = param_word(effect, 0u);
        has_id = record != 0xffffffffu;
        if has_id {
            color = image_texel_nearest(record, texel.uv).rgb;
        }
    } else if has_id {
        color = id_color_at(texel.pixel);
    }
    var amount = 0.0;
    if has_id && effect.procedural.z > 0u {
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
