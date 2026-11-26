fn generator_curvature(effect: Operation, pixel: vec2<u32>, finest: f32, mode: u32, micro: f32) -> f32 {
    let bake_pixel = bake_pixel_at(pixel);
    let offset = u32(effect.uv.x);
    var edges = 0.0;
    var cavities = 0.0;
    // P8: no curvature where a smaller UV tile leaves its block empty, only the micro height
    let sizes = select(0u, 7u, udim_bake_valid(pixel));
    for (var i = 0u; i < sizes; i++) {
        let weight = bitcast<f32>(srgb[offset + i]);
        if weight == 0.0 { continue; }
        var response = (finest - 0.5) * 2.0;
        if i > 0u {
            let size = f32(1u << (i - 1u));
            response = 0.0;
            if effect.effects.w == 0u {
                response = curvature_level_at(bake_pixel, i - 1u) * (2.8284271247461903 * size);
            }
        }
        edges = 1.0 - (1.0 - edges) * (1.0 - weight * clamp(response, 0.0, 1.0));
        cavities = 1.0 - (1.0 - cavities) * (1.0 - weight * clamp(-response, 0.0, 1.0));
    }
    edges = 1.0 - (1.0 - edges) * (1.0 - max(micro, 0.0));
    cavities = 1.0 - (1.0 - cavities) * (1.0 + min(micro, 0.0));
    switch mode {
        case 1u: { return edges; }
        case 2u: { return cavities; }
        case 3u: { return 1.0 - (1.0 - edges) * (1.0 - cavities); }
        default: { return 0.5 + 0.5 * (edges - cavities); }
    }
}
