// Test probes of the hooks above (opcodes 188..192, emitted only by the render crate's tests).
const PROBE_OPCODES_START: u32 = 188u;
fn probe_mask_effect(effect: Operation, texel: LayerTexel) -> f32 {
    switch effect.ids.y {
        case 188u: { return this_layer_content(effect, texel).x; }
        case 189u: { return stack_at(effect, texel.pixel).x; }
        case 190u: { return image_texel_nearest(param_word(effect, 0u), texel.uv).x; }
        default: {
            if effect.ext.x != 0u {
                return probe_late_hooks(effect, texel);
            }
            return effect_input(effect, texel);
        }
    }
}

// Opcode 191 with a selector in its first side-table word probes the P0h hooks: 1..4 the surface
// tangent's x, y, z and sign as 0.5 + 0.5 v, 5 the second stack read, 6 environment word
// param_word(effect, 2u) of the block at param_word(effect, 1u), 7 the baked Height of the bake
// rows (0.25 without one), 8 the baked ambient occlusion where baked_at reads it, 9 the image
// "probe" read bilinearly without wrapping.
fn probe_late_hooks(effect: Operation, texel: LayerTexel) -> f32 {
    let selector = param_word(effect, 0u);
    switch selector {
        case 1u, 2u, 3u, 4u: { return 0.5 + 0.5 * texel.surface.tangent[selector - 1u]; }
        case 5u: { return stack_at_second(effect, texel.pixel).x; }
        case 6u: { return bitcast<f32>(srgb[param_word(effect, 1u) + param_word(effect, 2u)]); }
        case 8u: { return bitcast<f32>(baked_at(texel.pixel).x); }
        case 9u: { return image_texel_bilinear_clamped(param_word(effect, 1u), texel.uv).x; }
        default: {
            if !bake_has(BAKED_HEIGHT) {
                return 0.25;
            }
            return bitcast<f32>(baked_block_at(texel.pixel, 1u).x);
        }
    }
}

