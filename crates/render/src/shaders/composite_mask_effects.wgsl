// Operation.ext.z flag of a soft comparison or colour selection (SOFT_MASK_EFFECT in gpu_composite_stack.rs)
const SOFT_MASK_EFFECT: u32 = 0x00000100u;

// The fields of a mask stack sit at its first slot (Operation.spatial.z, plus one) and the
// stack's effects hold slots relative to it, so a layer can have a second copy of its mask
// stack: the lit Base Color's material reads the one at params.mask_view.y while preparing
// (FilterPlan::add_lit_mask_views).
fn mask_base(layer: Operation) -> u32 {
    if params.mask_view.x != 0u && layer.spatial.z == params.mask_view.x {
        mask_view_active = true;
        return params.mask_view.y;
    }
    mask_view_active = false;
    return layer.spatial.z;
}

fn layer_mask(layer: Operation, texel: LayerTexel, count: u32) -> f32 {
    let start = params.program.x + layer.effects.x;
    var skip = 0u;
    var mask = layer.balance.y;
    preceding_field = 0xffffffffu;
    let base = mask_base(layer);
    if enable_spatial_fields && base != 0u {
        skip = previous_field(start, count);
        if skip != 0u {
            let field = base - 1u + layers[start + skip - 1u].spatial.x;
            preceding_field = field;
            mask = clamp(filter_value(field, texel.pixel).x, 0.0, 1.0);
        }
    }
    var i = skip;
    var fetched = false;
    loop {
        if i >= count {
            break;
        }
        let e = layers[start + i];
        // A package effect's image input through the same base_content call, before the effect
        let fetch = !fetched && is_extended_effect(e.ids.y) && e.effects.w != 0u;
        var stay = false;
        if e.ids.y == 16u {
            continue;
        }
        var weight = e.low.w;
        var result: f32;
        if fetch || is_fill_opcode(e.ids.y) {
            // Paint and fills through one base_content call: the driver inlines each call site
            var read = e;
            if fetch {
                read = layers[e.effects.w - 1u];
            }
            let sampled = base_content(read, texel);
            if fetch {
                effect_input_cache = sampled;
                stay = true;
                continue;
            }
            result = blend_scalar(u32(e.balance.z), mask, sampled.x);
            weight *= sampled.a;
        } else if enable_generators && e.ids.y >= 40u && e.ids.y < 48u {
            var ramp: f32;
            if !enable_spatial_fields || e.spatial.y == 0u {
                ramp = generator_ramp(e, texel);
            } else {
                ramp = filter_value(base - 1u + e.spatial.y - 1u, texel.pixel).x;
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
                let selected = stack_color_selection(e, texel);
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

        continuing {
            fetched = stay;
            if !stay {
                i++;
            }
        }
    }
    return clamp(mask, 0.0, 1.0);
}
