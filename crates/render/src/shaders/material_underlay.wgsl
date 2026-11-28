@group(0) @binding(19) var<storage, read> material_underlay: array<u32>;

const UNDERLAY_UNIFORM: u32 = 0x80000000u;

fn material_underlay_page(pixel: vec2<u32>) -> u32 {
    let tile = pixel / 64u;
    return material_underlay[1u + tile.y * material_underlay[0u] + tile.x];
}

fn underlay_value(page: u32, pixel: vec2<u32>, slot: u32, default_value: vec3<f32>) -> vec3<f32> {
    let channel = material_underlay[page + slot];
    if channel == 0u {
        return default_value;
    }

    let rgb = slot == 0u || (slot >= 7u && slot <= 11u);
    var index = channel & ~UNDERLAY_UNIFORM;
    if (channel & UNDERLAY_UNIFORM) == 0u {
        let local = pixel % 64u;
        index += (local.y * 64u + local.x) * select(1u, 3u, rgb);
    }
    let first = bitcast<f32>(material_underlay[index]);
    if rgb {
        return vec3(first, bitcast<f32>(material_underlay[index + 1u]), bitcast<f32>(material_underlay[index + 2u]));
    }
    return vec3(first);
}
