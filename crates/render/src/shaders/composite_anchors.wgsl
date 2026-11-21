fn capture_material_anchor(texel: LayerTexel, end: u32, triangle: u32) -> vec3<f32> {
    let channel = params.anchor.x;
    let color_channel = channel == 0u || (channel >= 7u && channel <= 11u);
    var result = params.anchor_default.rgb;
    var parents: array<vec3<f32>, 17>;
    var group_masks: array<f32, 17>;
    var depth = 0u;
    for (var i = 0u; i < end; i++) {
        let layer = layers[i];
        if composite_groups && layer.ids.y == BEGIN_GROUP {
            parents[depth] = result;
            var group_mask = 0.0;
            if geometry_allows(layer.constraints.x, triangle) {
                group_mask = mask_with_height(layer, texel, layer_mask(layer, texel, layer.effects.y));
            }
            group_masks[depth] = group_mask;
            depth += 1u;
            continue;
        }
        if composite_groups && layer.ids.y == END_GROUP {
            depth -= 1u;
            let weight = group_masks[depth] * bitcast<f32>(srgb[layer.ids.z + channel]);
            result = mix(parents[depth], result, weight);
            continue;
        }
        if layer.ids.x != channel || !geometry_allows(layer.constraints.x, triangle) {
            continue;
        }
        let count = select(layer.effects.w, min(layer.effects.w, params.anchor.z), i >= params.anchor.y);
        let content = layer_content(layer, texel, count);
        let mask = mask_with_height(layer, texel, layer_mask(layer, texel, layer.effects.y));
        let mode = u32(layer.balance.z);
        let weight = layer.low.w * mask * select(content.a, 1.0, mode == 2u && content.a > 0.0);
        if weight > 0.0 {
            if color_channel {
                result = mix(result, blend_rgb(mode, result, content.rgb), weight);
            } else {
                result = vec3(lerp(result.x, blend_scalar(mode, result.x, content.x), weight));
            }
        }
    }
    return result;
}
