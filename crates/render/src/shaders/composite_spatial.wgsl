// Each scratch texel holds four float components followed by UV coverage, and a sixth word (the
// stage's input, FilterStage::input in Rust) when params.field_layout.y is set.
fn field_word(slot: u32, local: vec2<u32>) -> u32 {
    let size = params.filter_region.zw;
    return params.filter_stage.x + (slot * size.x * size.y + local.y * size.x + local.x) * params.field_layout.x;
}

// The fields' origin in source texels. It lies below zero when the program has a wrapping
// stage (FilterPlan::wraps in Rust), so read it signed.
fn field_origin() -> vec2<i32> {
    return bitcast<vec2<i32>>(params.filter_region.xy);
}

// The texel of a field at source pixel `pixel`, or None (u32 max) outside the field.
fn field_local(pixel: vec2<u32>) -> vec2<u32> {
    let local = vec2<i32>(pixel) - field_origin();
    if any(local < vec2(0)) || any(local >= vec2<i32>(params.filter_region.zw)) {
        return vec2(0xffffffffu);
    }
    return vec2<u32>(local);
}

fn filter_value(slot: u32, pixel: vec2<u32>) -> vec4<f32> {
    let local = field_local(pixel);
    if local.x == 0xffffffffu {
        return vec4(0.0);
    }
    let start = field_word(slot, local);
    return bitcast<vec4<f32>>(vec4(output[start], output[start + 1u], output[start + 2u], output[start + 3u]));
}

// The field the layer_content or layer_mask loop resumed from (its preceding stage), or none.
var<private> preceding_field: u32 = 0xffffffffu;

// Word 4 of the preceding stage's output field at `pixel`: a stage's output keeps its coverage
// there, which no reader uses, so a stage can leave a second value in it (P4's pre-blur gray
// for P2's gradient after `precede_with_stage`). Zero outside the field or without a stage.
fn preceding_stage_word(pixel: vec2<u32>) -> f32 {
    let local = field_local(pixel);
    if preceding_field == 0xffffffffu || local.x == 0xffffffffu {
        return 0.0;
    }
    return bitcast<f32>(output[field_word(preceding_field, local) + 4u]);
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
    // A field can start below zero; a wrapping stage (field_layout.w bit 0) reads the set
    // periodically there, any other stage reads empty texels outside the set.
    let position = field_origin() + vec2<i32>(local);
    var pixel = vec2<u32>(max(position, vec2(0)));
    var covered = all(position >= vec2(0)) && all(pixel < params.source.xy);
    if (params.field_layout.w & 1u) != 0u {
        pixel = vec2<u32>(vec2(wrap_coordinate(position.x, i32(params.source.x)), wrap_coordinate(position.y, i32(params.source.y))));
        covered = true;
    }
    var prepared = vec4(0.0);
    var geometric_weight = 0.0;
    var stage_input = 0.0;
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
        if params.field_layout.z != 0u {
            stage_input = stage_input_value(layers[params.field_layout.z - 1u], texel);
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
    if params.field_layout.y != 0u {
        output[start + 5u] = bitcast<u32>(stage_input);
    }
}
