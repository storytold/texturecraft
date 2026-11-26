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
    preceding_field = 0xffffffffu;
    if enable_spatial_fields && layer.spatial.y != 0u {
        skip = previous_field(start, count);
    }
    var content = vec4(0.0);
    // The layer's own source (paint or fill) and its fill and paint effects go through one
    // base_content call: the driver inlines each call site, and a fill can be large (P1)
    var base = true;
    if skip != 0u {
        let field = layer.spatial.x + layers[start + skip - 1u].spatial.x;
        preceding_field = field;
        let filtered = filter_value(field, texel.pixel);
        let coverage = clamp(filtered.a, 0.0, 1.0);
        var color = vec3(0.0);
        if coverage > 1e-5 {
            color = filtered.rgb / coverage;
        }
        content = vec4(color, coverage);
        base = false;
        if content.a <= 0.0 && layer.spatial.w == 0u {
            return vec4(0.0);
        }
    }
    var i = skip;
    var fetched = false;
    loop {
        if !base && i >= count {
            break;
        }
        var effect = layer;
        if !base {
            effect = layers[start + i];
        }
        // A package effect's image input through the same base_content call, before the effect
        let fetch = !base && !fetched && is_extended_effect(effect.ids.y) && effect.effects.w != 0u;
        if base || fetch || is_fill_opcode(effect.ids.y) {
            var read = effect;
            if fetch {
                read = layers[effect.effects.w - 1u];
            }
            var source = base_content(read, texel);
            if fetch {
                effect_input_cache = source;
                fetched = true;
                continue;
            }
            if base {
                base = false;
                content = source;
                if passthrough {
                    // A pass-through layer lays its source over the stack beneath it
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
                if content.a <= 0.0 && layer.spatial.w == 0u {
                    return vec4(0.0);
                }
                continue;
            }
            // A graph effect that reads the stack keeps the layer's coverage
            if effect.ids.y == IMAGE_SOURCE_OPCODE && is_graph_image(effect.ids.w) && effect.procedural.x == 1u {
                source.a = content.a;
            }
            content = content_over(content, source, effect, layer.ids.x);
        } else if is_extended_effect(effect.ids.y) {
            content = extended_content_effect(effect, content, layer.ids.x, texel);
        } else if effect.ids.y != 39u {
            let filtered = effect_color(effect, content.rgb, layer.ids.x, texel.surface, texel.baked.x);
            content = vec4(content.rgb + (filtered - content.rgb) * effect.low.w, content.a);
        }
        fetched = false;
        i++;
    }
    return vec4(content.rgb, min(content.a, 1.0));
}
