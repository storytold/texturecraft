// Each scratch texel holds four float components followed by UV coverage, and a sixth word (the
// stage's input, FilterStage::input in Rust) when params.field_layout.y is set, or three words
// (the input's colour) when the stride is eight.
// A whole-set pre-pass keeps only its own stages and the working fields (whole_compact): a slot
// outside them has no field there (u32 max).
fn field_word(slot: u32, local: vec2<u32>) -> u32 {
    let size = params.filter_region.zw;
    var compact = slot;
    let table = params.whole_compact;
    if table.w != 0u {
        if slot >= table.z {
            compact = table.y + slot - table.z;
        } else if slot >= table.x && slot - table.x < table.y {
            compact = slot - table.x;
        } else {
            return 0xffffffffu;
        }
    }
    return params.filter_stage.x + (compact * size.x * size.y + local.y * size.x + local.x) * params.field_layout.x;
}

// The fields' origin in source texels. It lies below zero when the program has a wrapping
// stage (FilterPlan::wraps in Rust), so read it signed.
fn field_origin() -> vec2<i32> {
    return bitcast<vec2<i32>>(params.filter_region.xy);
}

// The field texel a periodic prepare pass writes (u32 max otherwise): there every field read
// (lower stages, anchors, generator ramps) takes the same copy of the set, not the pixel's
// position inside the set.
var<private> periodic_local: vec2<u32> = vec2(0xffffffffu);

// The texel of a field at source pixel `pixel`, or None (u32 max) outside the field.
fn field_local(pixel: vec2<u32>) -> vec2<u32> {
    if periodic_local.x != 0xffffffffu {
        return periodic_local;
    }
    let local = vec2<i32>(pixel) - field_origin();
    if any(local < vec2(0)) || any(local >= vec2<i32>(params.filter_region.zw)) {
        return vec2(0xffffffffu);
    }
    return vec2<u32>(local);
}

// The first word of stage `slot` at source pixel `pixel`, or u32 max outside its field. A
// whole-set stage (whole_slots) reads the pre-pass result at the pixel, also in a periodic
// prepare pass; any other stage its band field.
fn slot_word(stage_slot: u32, pixel: vec2<u32>) -> u32 {
    // P8: a texel of a smaller UV tile reads its tile scale's level stage
    let slot = udim_level_slot(stage_slot, pixel);
    if params.whole_slots.x != 0u {
        for (var k = 0u; k < 4u; k++) {
            if params.whole_slots[k] == stage_slot + 1u {
                let local = vec2<i32>(pixel) - bitcast<vec2<i32>>(params.whole_rect.xy);
                if any(local < vec2(0)) || any(local >= vec2<i32>(params.whole_rect.zw)) {
                    return 0xffffffffu;
                }
                // P8: a level stage's result follows its base stage's
                let result = (slot - stage_slot) * params.whole_rect.z * params.whole_rect.w;
                return params.whole_bases[k] + (result + u32(local.y) * params.whole_rect.z + u32(local.x)) * params.field_layout.x;
            }
        }
    }
    let local = field_local(pixel);
    if local.x == 0xffffffffu {
        return 0xffffffffu;
    }
    return field_word(slot, local);
}

fn filter_value(slot: u32, pixel: vec2<u32>) -> vec4<f32> {
    let start = slot_word(slot, pixel);
    if start == 0xffffffffu {
        return vec4(0.0);
    }
    return bitcast<vec4<f32>>(vec4(output[start], output[start + 1u], output[start + 2u], output[start + 3u]));
}

// The field the layer_content or layer_mask loop resumed from (its preceding stage), or none.
var<private> preceding_field: u32 = 0xffffffffu;

// Word 4 of the preceding stage's output field at `pixel`: a stage's output keeps its coverage
// there, which no reader uses, so a stage can leave a second value in it (P4's pre-blur gray
// for P2's gradient after `precede_with_stage`). Zero outside the field or without a stage.
fn preceding_stage_word(pixel: vec2<u32>) -> f32 {
    if preceding_field == 0xffffffffu {
        return 0.0;
    }
    let start = slot_word(preceding_field, pixel);
    if start == 0xffffffffu {
        return 0.0;
    }
    return bitcast<f32>(output[start + 4u]);
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

// Bits of the prepare pass's params.field_layout.w besides bit 0 (the stage wraps): how it reads
// its input (FilterStage::input_kind in Rust)
const INPUT_LAYER: u32 = 2u;
const INPUT_COLOR: u32 = 4u;
const OWN_VIEW: u32 = 8u;
const NO_FIELDS: u32 = 32u;
const INPUT2: u32 = 16u;

@compute @workgroup_size(8, 8)
fn prepare_filter(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.filter_chunk.zw) {
        return;
    }
    let local = params.filter_chunk.xy + id.xy;
    // A field can start below zero. In a wrapping program (field_layout.w bit 0) every stage's
    // field holds periodic copies of the set there, and reads the lower fields at the same copy;
    // otherwise the field lies inside the set.
    let position = field_origin() + vec2<i32>(local);
    var pixel = vec2<u32>(max(position, vec2(0)));
    var covered = all(position >= vec2(0)) && all(pixel < params.source.xy);
    if (params.field_layout.w & 1u) != 0u {
        pixel = vec2<u32>(vec2(wrap_coordinate(position.x, i32(params.source.x)), wrap_coordinate(position.y, i32(params.source.y))));
        covered = true;
        periodic_local = local;
    }
    var prepared = vec4(0.0);
    var geometric_weight = 0.0;
    var stage_input = vec3(0.0);
    var stage_input2 = vec3(0.0);
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
        // Every mask and every content goes through one layer_mask and one layer_content call
        // below, which the driver inlines once each (the call sites, not the paths, set the size)
        let flags = params.filter_stage.w;
        let capture = enable_anchor_reads && (flags & 0x20000000u) != 0u;
        let anchor_mask = enable_anchor_reads && !capture && (flags & 0x10000000u) != 0u;
        let generator = !capture && !anchor_mask && (flags & 0x40000000u) != 0u;
        let mask = anchor_mask || (!capture && !generator && (flags & 0x80000000u) != 0u);
        var triangle = 0xffffffffu;
        if (capture || anchor_mask) && (params.coverage.w & GEOMETRY_MASKS) != 0u {
            triangle = textureLoad(surface_uv, vec2<i32>(pixel), 0).x;
        }
        if generator {
            let source = layers[params.program.x + layer.effects.x + count];
            prepared = vec4(generator_ramp(source, texel), 0.0, 0.0, 1.0);
        }
        // One loop evaluates what the stage needs: a capture's layers below (their masks and
        // contents blended as the anchor capture does), the stage's mask or its content, and a
        // layer operation's content as its input (Height To Normal's own Height, INPUT_LAYER)
        let input_layer = params.field_layout.z != 0u && (params.field_layout.w & INPUT_LAYER) != 0u;
        var first = params.filter_stage.z;
        var end = first + select(1u, 0u, generator);
        if capture {
            first = 0u;
            end = params.filter_stage.z;
        }
        let input_step = end;
        if input_layer {
            end += 1u;
        }
        let channel = params.anchor.x;
        let color_channel = channel == 0u || (channel >= 7u && channel <= 11u);
        var result = params.anchor_default.rgb;
        var parents: array<vec3<f32>, 17>;
        var group_masks: array<f32, 17>;
        var depth = 0u;
        for (var i = first; i < end; i++) {
            let input = i == input_step;
            var source = layers[select(i, params.field_layout.z - 1u, input)];
            if capture && !input {
                if composite_groups && source.ids.y == END_GROUP {
                    depth -= 1u;
                    let weight = group_masks[depth] * bitcast<f32>(srgb[source.ids.z + channel]);
                    result = mix(parents[depth], result, weight);
                    continue;
                }
                if !(composite_groups && source.ids.y == BEGIN_GROUP) && source.ids.x != channel {
                    continue;
                }
            }
            // The mask: a capture's folders and layers (with their height masks), or the stage's
            var value = 1.0;
            if !input && (capture || mask) {
                value = 0.0;
                if !(capture || anchor_mask) || geometry_allows(source.constraints.x, triangle) {
                    value = layer_mask(source, texel, select(count, source.effects.y, capture));
                    if capture {
                        value = mask_with_height(source, texel, value);
                    }
                }
                if mask {
                    prepared = select(vec4(value, 0.0, 0.0, 1.0), vec4(vec3(value), 1.0), anchor_mask);
                    continue;
                }
                if composite_groups && source.ids.y == BEGIN_GROUP {
                    parents[depth] = result;
                    group_masks[depth] = value;
                    depth += 1u;
                    continue;
                }
                if value <= 0.0 {
                    continue;
                }
            }
            // The content
            var source_count = count;
            if input {
                source_count = source.effects.w;
            } else if capture {
                source_count = select(source.effects.w, min(source.effects.w, params.anchor.z), i >= params.anchor.y);
            } else {
                // A view of the layer for its mask (This Layer): its own material as the
                // defaults (OWN_VIEW), its stages as identity (NO_FIELDS)
                reading_own_layer = (params.field_layout.w & OWN_VIEW) != 0u;
                if (params.field_layout.w & NO_FIELDS) != 0u {
                    source.spatial.y = 0u;
                }
            }
            let content = layer_content(source, texel, source_count);
            reading_own_layer = false;
            if input {
                // A layer operation's content times its coverage
                stage_input = vec3(content.x * content.a);
            } else if capture {
                let mode = u32(source.balance.z);
                let weight = source.low.w * value * select(content.a, 1.0, mode == 2u && content.a > 0.0);
                if weight > 0.0 {
                    if color_channel {
                        result = mix(result, blend_rgb(mode, result, content.rgb), weight);
                    } else {
                        result = vec3(lerp(result.x, blend_scalar(mode, result.x, content.x), weight));
                    }
                }
            } else {
                prepared = vec4(content.rgb * content.a, content.a);
            }
        }
        if capture {
            prepared = vec4(result, 1.0);
        }
        // The input sources: the first (unless a layer operation), and a second one (INPUT2,
        // its colour); one base_content call (P3's stage_input_value is its base content's x)
        let second = (params.field_layout.w & INPUT2) != 0u;
        let sources = select(1u, 2u, second);
        for (var k = select(0u, 1u, params.field_layout.z == 0u || input_layer); k < sources; k++) {
            let source = base_content(layers[select(params.field_layout.z, params.anchor.z, k == 1u) - 1u], texel);
            if k == 1u {
                stage_input2 = source.rgb;
            } else {
                stage_input = select(vec3(source.x), source.rgb, (params.field_layout.w & INPUT_COLOR) != 0u);
            }
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
        output[start + 5u] = bitcast<u32>(stage_input.x);
        if params.field_layout.x >= 8u {
            output[start + 6u] = bitcast<u32>(stage_input.y);
            output[start + 7u] = bitcast<u32>(stage_input.z);
        }
        if params.field_layout.x >= 11u {
            output[start + 8u] = bitcast<u32>(stage_input2.x);
            output[start + 9u] = bitcast<u32>(stage_input2.y);
            output[start + 10u] = bitcast<u32>(stage_input2.z);
        }
    }
}
