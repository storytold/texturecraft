// Operation.ext.z flag of a soft comparison or colour selection (SOFT_MASK_EFFECT in gpu_composite_stack.rs)
const SOFT_MASK_EFFECT: u32 = 0x00000100u;

fn layer_mask(layer: Operation, texel: LayerTexel, count: u32) -> f32 {
    let start = params.program.x + layer.effects.x;
    var skip = 0u;
    var mask = layer.balance.y;
    if enable_spatial_fields && layer.spatial.z != 0u {
        skip = previous_field(start, count);
        if skip != 0u {
            let field = layers[start + skip - 1u].spatial.x;
            mask = clamp(filter_value(field, texel.pixel).x, 0.0, 1.0);
        }
    }
    for (var i = skip; i < count; i++) {
        let e = layers[start + i];
        if e.ids.y == 16u {
            continue;
        }
        var weight = e.low.w;
        var result: f32;
        if e.ids.y == 10u {
            let sampled = painted(e, texel.pixel, texel.projected);
            result = blend_scalar(u32(e.balance.z), mask, sampled.x);
            weight *= sampled.w;
        } else if is_fill_opcode(e.ids.y) {
            let sampled = fill_value(e, texel.uv, texel.surface);
            result = blend_scalar(u32(e.balance.z), mask, sampled.x);
            weight *= sampled.a;
        } else if enable_generators && e.ids.y >= 40u && e.ids.y < 48u {
            var ramp: f32;
            if !enable_spatial_fields || e.spatial.y == 0u {
                ramp = generator_ramp(e, texel);
            } else {
                ramp = filter_value(e.spatial.y - 1u, texel.pixel).x;
            }
            if e.spatial.z != 0u {
                // A generator's region is stored as mask values before later effects read it.
                mask = clamp(mask, 0.0, 1.0);
                result = blend_scalar(u32(e.balance.z), mask, finish_generator(e, ramp));
                mask = clamp(lerp(mask, result, weight), 0.0, 1.0);
                continue;
            }
            result = blend_scalar(u32(e.balance.z), mask, finish_generator(e, ramp));
        } else if is_extended_effect(e.ids.y) {
            result = extended_mask_effect(e, mask, texel);
        } else if e.ids.y == COLOR_SELECTION_OPCODE {
            if (e.ext.z & SOFT_MASK_EFFECT) != 0u {
                // Output, soft edge and background; a transparent background covers only the selection
                let selected = stack_color_selection(e, texel.pixel);
                result = blend_scalar(u32(e.balance.z), mask, selected.x);
                weight *= selected.y;
            } else {
                result = blend_scalar(u32(e.balance.z), mask, color_selection(e, texel.pixel));
            }
        } else if e.ids.y == 37u {
            // A comparison blends its pass or fail into the mask like a fill
            var compared = scalar_filter(e, mask);
            if (e.ext.z & SOFT_MASK_EFFECT) != 0u {
                compared = stack_soft_compare(e, mask);
            }
            result = blend_scalar(u32(e.balance.z), mask, compared);
        } else {
            result = scalar_filter(e, mask);
        }
        mask = lerp(mask, result, weight);
    }
    return clamp(mask, 0.0, 1.0);
}
