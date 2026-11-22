fn content_over(content: vec4<f32>, source: vec4<f32>, effect: Operation, channel: u32) -> vec4<f32> {
    let alpha = source.a * effect.low.w;
    if alpha <= 0.0 { return content; }
    var blended = source.rgb;
    if content.a > 0.0 {
        let mode = u32(effect.balance.z);
        if measured_blend {
            blended = layer_blended_color(mode, content.rgb, source.rgb, channel);
        } else if group_color_slot(channel) {
            blended = blend_rgb(mode, content.rgb, source.rgb);
        } else {
            blended = vec3(blend_scalar(mode, content.x, source.x));
        }
    }
    let coverage = alpha + content.a * (1.0 - alpha);
    let amount = alpha / max(coverage, 1e-5);
    return vec4(content.rgb + (blended - content.rgb) * amount, coverage);
}

fn layer_content(layer: Operation, texel: LayerTexel, count: u32) -> vec4<f32> {
    return layer_content_from(layer, texel, count, vec4(0.0), false);
}

fn layer_content_from(layer: Operation, texel: LayerTexel, count: u32, previous: vec4<f32>, passthrough: bool) -> vec4<f32> {
    let start = params.program.x + layer.effects.z;
    var skip = 0u;
    if enable_spatial_fields && layer.spatial.y != 0u {
        skip = previous_field(start, count);
    }
    var content: vec4<f32>;
    if skip != 0u {
        let field = layer.spatial.x + layers[start + skip - 1u].spatial.x;
        let filtered = filter_value(field, texel.pixel);
        let coverage = clamp(filtered.a, 0.0, 1.0);
        var color = vec3(0.0);
        if coverage > 1e-5 {
            color = filtered.rgb / coverage;
        }
        content = vec4(color, coverage);
    } else {
        content = base_content(layer, texel);
        if passthrough {
            if measured_blend {
                let empty = empty_channel(layer.ids.x);
                var straight = empty;
                if previous.a > 0.0 { straight = (previous.rgb - empty * (1.0 - previous.a)) / previous.a; }
                var addition = layer;
                addition.low.w = 1.0;
                addition.balance.z = 0.0;
                content = content_over(vec4(straight, previous.a), content, addition, layer.ids.x);
            } else {
                content = vec4(mix(previous.rgb, content.rgb, clamp(content.a, 0.0, 1.0)), 1.0);
            }
        }
    }
    if content.a <= 0.0 && layer.spatial.w == 0u {
        return vec4(0.0);
    }
    for (var i = skip; i < count; i++) {
        let effect = layers[start + i];
        if effect.ids.y == 10u {
            let paint = painted(effect, texel.pixel, texel.projected);
            content = content_over(content, paint, effect, layer.ids.x);
        } else if effect.ids.y < 20u || effect.ids.y >= 48u {
            var fill = content_fill_value(effect, texel.uv, texel.surface);
            if effect.ids.y == IMAGE_SOURCE_OPCODE && effect.ids.w >= 8u && effect.procedural.x == 1u {
                fill.a = content.a;
            }
            content = content_over(content, fill, effect, layer.ids.x);
        } else if effect.ids.y != 39u {
            let filtered = effect_color(effect, content.rgb, layer.ids.x, texel.surface, texel.baked.x);
            content = vec4(content.rgb + (filtered - content.rgb) * effect.low.w, content.a);
        }
    }
    return vec4(content.rgb, min(content.a, 1.0));
}
