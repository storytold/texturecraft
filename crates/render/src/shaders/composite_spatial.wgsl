// Each scratch texel holds four float components followed by UV coverage.
fn field_word(slot: u32, local: vec2<u32>) -> u32 {
    let size = params.filter_region.zw;
    return params.filter_stage.x + (slot * size.x * size.y + local.y * size.x + local.x) * 5u;
}

fn filter_value(slot: u32, pixel: vec2<u32>) -> vec4<f32> {
    if any(pixel < params.filter_region.xy) {
        return vec4(0.0);
    }
    let local = pixel - params.filter_region.xy;
    if any(local >= params.filter_region.zw) {
        return vec4(0.0);
    }
    let start = field_word(slot, local);
    return bitcast<vec4<f32>>(vec4(output[start], output[start + 1u], output[start + 2u], output[start + 3u]));
}

// The most recent prepared field already contains all the effects below it.
fn previous_field(start: u32, count: u32) -> u32 {
    for (var i = count; i > 0u; i--) {
        if layers[start + i - 1u].ids.y == SPATIAL_FILTER_OPCODE {
            return i;
        }
    }
    return 0u;
}

@compute @workgroup_size(8, 8)
fn prepare_filter(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.filter_chunk.zw) {
        return;
    }
    let local = params.filter_chunk.xy + id.xy;
    let pixel = params.filter_region.xy + local;
    var prepared = vec4(0.0);
    var geometric_weight = 0.0;
    var covered = all(pixel < params.source.xy);
    if covered && params.coverage.x != 0u {
        covered = uv_texel_covered(pixel);
    }
    if covered {
        let surface = surface_at(pixel);
        let uv = (vec2<f32>(pixel) + 0.5) / vec2<f32>(params.source.xy);
        var projected = ProjectedBrush(vec4(0.0), 0.0);
        if (params.coverage.w & PROJECTED_BRUSH) != 0u {
            projected = sample_brush_projection(pixel, surface);
        }
        var baked = vec3(1.0, 0.5, 1.0);
        if (params.coverage.w & 2u) != 0u {
            baked = bitcast<vec3<f32>>(baked_at(pixel).xyz);
        }
        let texel = LayerTexel(pixel, uv, surface, baked, projected);
        let layer = layers[params.filter_stage.z];
        let count = params.filter_stage.w & 0x0fffffffu;
        if (params.filter_stage.w & 0x40000000u) != 0u {
            let generator = layers[params.program.x + layer.effects.x + count];
            prepared = vec4(generator_ramp(generator, texel), 0.0, 0.0, 1.0);
        } else if enable_anchor_reads && (params.filter_stage.w & 0x30000000u) != 0u {
            var triangle = 0xffffffffu;
            if (params.coverage.w & GEOMETRY_MASKS) != 0u {
                triangle = textureLoad(surface_uv, vec2<i32>(pixel), 0).x;
            }
            if (params.filter_stage.w & 0x20000000u) != 0u {
                prepared = vec4(capture_material_anchor(texel, params.filter_stage.z, triangle), 1.0);
            } else if geometry_allows(layer.constraints.x, triangle) {
                prepared = vec4(vec3(layer_mask(layer, texel, count)), 1.0);
            }
        } else if (params.filter_stage.w & 0x80000000u) != 0u {
            prepared = vec4(layer_mask(layer, texel, count), 0.0, 0.0, 1.0);
        } else {
            let content = layer_content(layer, texel, count);
            prepared = vec4(content.rgb * content.a, content.a);
        }
        geometric_weight = 1.0;
    }
    let start = field_word(params.filter_stage.y, local);
    let words = bitcast<vec4<u32>>(prepared);
    output[start] = words.x;
    output[start + 1u] = words.y;
    output[start + 2u] = words.z;
    output[start + 3u] = words.w;
    output[start + 4u] = bitcast<u32>(geometric_weight);
}
