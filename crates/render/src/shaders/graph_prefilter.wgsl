// Graph fills use the same graph-output mip chain and projection footprints as the CPU compositor.
fn graph_finite(value: f32) -> bool {
    return abs(value) <= 3.402823466e38;
}

fn graph_prefilters(layer: Operation) -> bool {
    return layer.ids.y == IMAGE_SOURCE_OPCODE && is_graph_image(layer.ids.w) && layer.ext.x != 0u;
}

fn graph_uv_footprint(layer: Operation) -> f32 {
    if !graph_prefilters(layer) { return 0.0; }
    let scale = abs(layer.projection.zw);
    return max(scale.x, scale.y) / f32(max(udim_block_resolution(), 1u));
}

fn graph_surface_axes(surface: Surface) -> GraphSurfaceAxes {
    let empty = GraphSurfaceAxes(vec3(0.0), vec3(0.0), 0.0);
    if !surface.valid { return empty; }

    let vertices = surface.triangle * 3u;
    let a = surface_vertices[vertices];
    let b = surface_vertices[vertices + 1u];
    let c = surface_vertices[vertices + 2u];
    let e1 = b.position.xyz - a.position.xyz;
    let e2 = c.position.xyz - a.position.xyz;
    let d1 = vec2(b.position.w - a.position.w, b.normal.w - a.normal.w);
    let d2 = vec2(c.position.w - a.position.w, c.normal.w - a.normal.w);
    let det = d1.x * d2.y - d2.x * d1.y;
    if !(abs(det) >= 1e-14 && graph_finite(det)) { return empty; }

    let size = vec2<f32>(max(params.source.xy, vec2(1u)));
    let across = (e1 * d2.y - e2 * d1.y) / (det * size.x);
    let down = (e2 * d1.x - e1 * d2.x) / (det * size.y);
    let span = sqrt(length(cross(e1, e2)) / abs(det)) / size.x;
    let limit = vec3(3.402823466e38);
    if !all(abs(across) <= limit) || !all(abs(down) <= limit) || !graph_finite(span) { return empty; }
    return GraphSurfaceAxes(across, down, span);
}

fn graph_projection_footprint(layer: Operation, surface: Surface) -> f32 {
    if !graph_prefilters(layer) { return 0.0; }
    let mode = layer.procedural.w;
    if mode == 0u { return graph_uv_footprint(layer); }
    if mode > 4u { return 0.0; }

    let scale = abs(layer.projection.w) * select(0.5, 1.0, mode == 1u);
    return graph_surface_axes(surface).length * scale;
}

fn graph_local_axes(frame: u32, vector: vec3<f32>) -> vec3<f32> {
    return vec3(dot(decal_vector(frame), vector), dot(decal_vector(frame + 4u), vector), dot(decal_vector(frame + 8u), vector));
}

fn graph_local_length(frame: u32, world_length: f32) -> f32 {
    if world_length <= 0.0 { return 1.0 / f32(max(udim_block_resolution(), 1u)); }
    let x = length(decal_vector(frame));
    let y = length(decal_vector(frame + 4u));
    let z = length(decal_vector(frame + 8u));
    return world_length * max(max(x, y), z);
}

fn graph_image_axis(frame: u32, axis: vec2<f32>) -> vec2<f32> {
    let u = bitcast<vec2<f32>>(vec2(srgb[frame + 12u], srgb[frame + 13u]));
    let v = bitcast<vec2<f32>>(vec2(srgb[frame + 14u], srgb[frame + 15u]));
    return vec2(dot(u, axis), dot(v, axis));
}

fn graph_image_scale(frame: u32) -> f32 {
    let u = bitcast<vec2<f32>>(vec2(srgb[frame + 12u], srgb[frame + 13u]));
    let v = bitcast<vec2<f32>>(vec2(srgb[frame + 14u], srgb[frame + 15u]));
    return max(length(u), length(v));
}

fn graph_warp_footprint(frame: u32, surface: Surface, image_per_grid: f32) -> f32 {
    let grid_per_local = 2.0;
    let axes = graph_surface_axes(surface);
    return graph_local_length(frame, axes.length) * grid_per_local * image_per_grid * graph_image_scale(frame);
}

fn graph_plane_axis(vector: vec3<f32>, plane: u32, side: vec3<f32>) -> vec2<f32> {
    switch plane {
        case 0u: { return vec2(-vector.z * side.x, -vector.y); }
        case 1u: { return vec2(vector.x, vector.z * side.y); }
        default: { return vec2(vector.x * side.z, -vector.y); }
    }
}

fn graph_triplanar_axes(frame: u32, surface: Surface) -> GraphSurfaceAxes {
    let axes = graph_surface_axes(surface);
    let footprint = graph_local_length(frame, axes.length) * graph_image_scale(frame);
    return GraphSurfaceAxes(graph_local_axes(frame, axes.across), graph_local_axes(frame, axes.down), footprint);
}

fn graph_plane_tap(frame: u32, plane: u32, side: vec3<f32>, axes: GraphSurfaceAxes, point: vec2<f32>) -> GraphTap {
    if all(axes.across == vec3(0.0)) && all(axes.down == vec3(0.0)) {
        return GraphTap(point, axes.length, vec2(0.0), 1u);
    }

    let across = graph_image_axis(frame, graph_plane_axis(axes.across, plane, side));
    let down = graph_image_axis(frame, graph_plane_axis(axes.down, plane, side));
    let across_len = length(across);
    let down_len = length(down);
    let longer = max(across_len, down_len);
    let shorter = min(across_len, down_len);
    if !(longer > 0.0 && graph_finite(longer)) { return GraphTap(point, 0.0, vec2(0.0), 1u); }

    // Keep the same probe cap as compose::placed::MAX_ANISOTROPY
    let max_probes = 16.0;
    let probes = select(max_probes, clamp(ceil(longer / shorter), 1.0, max_probes), shorter > 0.0);
    let footprint = longer / probes;
    let major = select(down, across, across_len >= down_len);
    return GraphTap(point, footprint, major * max((longer - footprint) / longer, 0.0), u32(probes));
}

fn graph_image_sample(image: u32, tap: GraphTap, repeat: vec2<bool>) -> vec4<f32> {
    var table = 0u;
    var detail = 0.0;
    var probes = 1u;
    if tap.footprint > 0.0 {
        let side = f32(max(srgb[image], srgb[image + 1u]));
        let footprint = tap.footprint * side;
        let flags = srgb[image + 2u];
        if footprint > 1.0 && graph_finite(footprint) {
            probes = tap.probes;
            if (flags & 512u) != 0u {
                // The graph's base image has one gray plane or two planes of 16-bit RGBA
                let planes = select(2u, 1u, (flags & 255u) == 5u);
                table = image + 3u + 3u * planes;
                detail = min(log2(footprint), f32(srgb[table]));
            }
        }
    }

    let fine = u32(floor(detail));
    let blend = detail - f32(fine);
    let levels = select(1u, 2u, blend > 0.0);
    var rgba = vec4(0.0);
    var count = 0u;
    for (var probe = 0u; probe < probes; probe++) {
        let along = (f32(probe) + 0.5) / f32(probes) - 0.5;
        let uv = tap.uv + tap.span * along;
        if !placed_uv_covered(uv, repeat) { continue; }
        var samples = array<vec4<f32>, 2>(vec4(0.0), vec4(0.0));
        for (var level = 0u; level < levels; level++) {
            let index = fine + level;
            var descriptor = image;
            if index > 0u { descriptor = srgb[table + index]; }
            samples[level] = material_image_at_with_repeat(descriptor, uv, repeat);
        }
        rgba += samples[0] + (samples[1] - samples[0]) * blend;
        count += 1u;
    }
    return rgba / f32(max(count, 1u));
}
