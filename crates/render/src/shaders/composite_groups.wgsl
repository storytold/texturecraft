// Folder-local state uses spare space after the display and spatial filter
// outputs in the band buffer. The flat material pipeline specializes this off.
override composite_groups: bool = false;
const BEGIN_GROUP: u32 = 200u;
const END_GROUP: u32 = 201u;
const DISPLAY_CHANNELS: u32 = 21u;
const PASSTHROUGH_GROUP: u32 = 0xffffffffu;

struct MaterialValues {
    base: vec3<f32>,
    normal: vec3<f32>,
    scalars: array<f32, 6>,
    lobe_colors: array<vec3<f32>, 4>,
    lobe_values: array<f32, 9>,
    coverage: array<f32, DISPLAY_CHANNELS>,
}

fn group_word(depth: u32, channel_word: u32, index: u32) -> u32 {
    // Interleave adjacent pixels so a workgroup writes contiguous words.
    return params.filter_stage.y + (depth * params.filter_stage.z + channel_word) * params.band.w + index;
}

fn group_color_slot(slot: u32) -> bool {
    return slot == 0u || (slot >= 7u && slot < 12u);
}

fn material_value(state: MaterialValues, slot: u32) -> vec3<f32> {
    if slot == 0u { return state.base; }
    if slot == 7u { return state.normal; }
    if slot >= 12u { return vec3(state.lobe_values[slot - 12u]); }
    if slot >= 8u { return state.lobe_colors[slot - 8u]; }
    return vec3(state.scalars[slot - 1u]);
}

fn set_material_value(state: ptr<function, MaterialValues>, slot: u32, color: vec3<f32>) {
    if slot == 0u {
        (*state).base = color;
    } else if slot == 7u {
        (*state).normal = color;
    } else if slot >= 12u {
        (*state).lobe_values[slot - 12u] = color.x;
    } else if slot >= 8u {
        (*state).lobe_colors[slot - 8u] = color;
    } else {
        (*state).scalars[slot - 1u] = color.x;
    }
}

fn save_group_parent(depth: u32, index: u32, weights: u32, mask: f32, parent: MaterialValues) -> MaterialValues {
    var inner = parent;
    output[group_word(depth, 0u, index)] = bitcast<u32>(mask);
    var channels = params.filter_stage.w;
    var word = 1u;
    while channels != 0u {
        let slot = firstTrailingBit(channels);
        channels &= channels - 1u;
        let value = bitcast<vec3<u32>>(material_value(parent, slot));
        output[group_word(depth, word, index)] = value.x;
        word += 1u;
        if group_color_slot(slot) {
            output[group_word(depth, word, index)] = value.y;
            output[group_word(depth, word + 1u, index)] = value.z;
            word += 2u;
        }
        if measured_blend {
            output[group_word(depth, word, index)] = bitcast<u32>(parent.coverage[slot]);
            word += 1u;
            let mode = srgb[weights + DISPLAY_CHANNELS + slot];
            if mode != PASSTHROUGH_GROUP && mode != 1u {
                set_material_value(&inner, slot, empty_channel(slot));
                inner.coverage[slot] = 0.0;
            }
        }
    }
    return inner;
}

fn mix_group_parent(depth: u32, index: u32, weights: u32, inner: MaterialValues) -> MaterialValues {
    var result = inner;
    let mask = bitcast<f32>(output[group_word(depth, 0u, index)]);
    var channels = params.filter_stage.w;
    var word = 1u;
    while channels != 0u {
        let slot = firstTrailingBit(channels);
        channels &= channels - 1u;
        let weight = mask * bitcast<f32>(srgb[weights + slot]);
        let red = bitcast<f32>(output[group_word(depth, word, index)]);
        var outer = vec3(red);
        word += 1u;
        if group_color_slot(slot) {
            outer.y = bitcast<f32>(output[group_word(depth, word, index)]);
            outer.z = bitcast<f32>(output[group_word(depth, word + 1u, index)]);
            word += 2u;
        }
        let color = material_value(inner, slot);
        if measured_blend {
            let outer_coverage = bitcast<f32>(output[group_word(depth, word, index)]);
            word += 1u;
            let outer_values = vec4(outer, outer_coverage);
            let inner_coverage = inner.coverage[slot];
            let mode = srgb[weights + DISPLAY_CHANNELS + slot];
            var mixed = vec4(color, inner_coverage);
            if mode == PASSTHROUGH_GROUP {
                mixed = mix(outer_values, mixed, weight);
            } else {
                if mode != 2u && inner_coverage > 0.0 {
                    mixed = vec4((color - empty_channel(slot) * (1.0 - inner_coverage)) / inner_coverage, inner_coverage);
                }
                mixed = measured_over(mode, outer_values, mixed, weight, slot);
            }
            set_material_value(&result, slot, mixed.rgb);
            result.coverage[slot] = mixed.a;
        } else {
            set_material_value(&result, slot, outer + (color - outer) * weight);
        }
    }
    return result;
}
