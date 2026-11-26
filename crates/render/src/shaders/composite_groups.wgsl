// Folder-local state uses spare space after the display and spatial filter
// outputs in the band buffer. The flat material pipeline specializes this off.
override composite_groups: bool = false;
const BEGIN_GROUP: u32 = 200u;
const END_GROUP: u32 = 201u;

struct MaterialValues {
    base: vec3<f32>,
    normal: vec3<f32>,
    scalars: array<f32, 6>,
    lobe_colors: array<vec3<f32>, 4>,
    lobe_values: array<f32, 9>,
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

fn save_group_parent(depth: u32, index: u32, mask: f32, parent: MaterialValues) {
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
    }
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
        let color = outer + (material_value(inner, slot) - outer) * weight;
        if slot == 0u {
            result.base = color;
        } else if slot == 7u {
            result.normal = color;
        } else if slot >= 12u {
            result.lobe_values[slot - 12u] = color.x;
        } else if slot >= 8u {
            result.lobe_colors[slot - 8u] = color;
        } else {
            result.scalars[slot - 1u] = color.x;
        }
    }
    return result;
}
