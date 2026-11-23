// The nearest squared distance can be clipped to one when no feature ID is needed.
fn cellular(p: vec3<f32>, seed: u32, details: bool) -> vec3<f32> {
    let cell = vec3<i32>(floor(p));
    let local = fract(p);
    var nearest = select(1.0, 4.0, details);
    var second = 1.0;
    var feature_id = 0u;
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let shift = vec3(x, y, z);
                let bits = hash(cell + shift, seed * 0xc2b2ae35u);
                let feature = vec3<f32>(bits) / 4294967296.0;
                let delta = vec3<f32>(shift) + feature - local;
                let distance = dot(delta, delta);
                if details {
                    if distance < nearest {
                        second = nearest;
                        nearest = distance;
                        feature_id = bits.x ^ bits.y;
                    } else {
                        second = min(second, distance);
                    }
                } else {
                    nearest = min(nearest, distance);
                }
            }
        }
    }
    return vec3(sqrt(min(1.0, nearest)), sqrt(min(1.0, second)), f32(feature_id) / 4294967296.0);
}

fn cells(p: vec3<f32>, seed: u32) -> f32 {
    return cellular(p, seed, false).x;
}

fn cell_value(cell: vec3<i32>, seed: u32) -> f32 {
    return f32(hash(cell, seed * 0x9e3779b9u).x) / 4294967296.0;
}

fn drip(p: vec3<f32>, x: f32, salt: i32, seed: u32) -> f32 {
    let cx = x * 6.0;
    let column = i32(floor(cx));
    let width = 0.08 + 0.25 * cell_value(vec3(column, 0, salt), seed);
    let across = 1.0 - smoothstep(width * 0.3, width, abs(fract(cx) - 0.5 - (cell_value(vec3(column, 1, salt), seed) - 0.5) * 0.5));
    let yy = p.y * (0.35 + 0.4 * cell_value(vec3(column, 2, salt), seed)) + cell_value(vec3(column, 3, salt), seed) * 7.0;
    let t = fract(yy);
    let on = cell_value(vec3(column, i32(floor(yy)), 3 + salt), seed) > 0.5;
    return select(0.0, across * pow(t, 1.6) * (1.0 - smoothstep(0.9, 1.0, t)), on);
}

fn pattern(kind: u32, p: vec3<f32>, seed: u32) -> f32 {
    switch kind {
        case 1u: { return fbm(p, 6u, seed, false); }
        case 2u: { return fbm(p, 4u, seed, true); }
        case 3u: { return noise(p * 16.0, seed, false); }
        case 4u: { return cells(p, seed); }
        case 5u: {
            let n = fbm(p, 6u, seed, true) * 0.7 + fbm(p * 3.7, 3u, seed ^ 0x55u, false) * 0.3;
            return clamp((n - 0.45) * 4.0 + 0.5, 0.0, 1.0);
        }
        case 6u: {
            let n = noise(p * vec3(0.15, 6.0, 6.0), seed, true);
            let line = 1.0 - min(abs(n - 0.5) * 40.0, 1.0);
            let gate = noise(p * 0.8, seed ^ 0x99u, false);
            return line * clamp((gate - 0.45) * 4.0, 0.0, 1.0);
        }
        case 7u: {
            let warp = fbm(p * vec3(0.18, 0.6, 1.0), 3u, seed, true);
            let t = fract((p.y + (warp - 0.5) * 0.9) * 5.0);
            let ring = 1.0 - smoothstep(0.45, 1.0, t);
            let fiber = noise(p * vec3(2.0, 40.0, 1.0), seed ^ 0x3cu, false);
            let tone = fbm(p * vec3(0.6, 3.0, 1.0), 2u, seed ^ 0x71u, false);
            return clamp(ring * 0.6 + fiber * 0.25 + tone * 0.15, 0.0, 1.0);
        }
        case 8u: {
            let turb = fbm(p, 5u, seed, true);
            let veins = pow(abs(sin((p.x + p.y * 0.5) * 2.5 + turb * 9.0)), 0.3);
            let cloud = fbm(p * 2.0, 3u, seed ^ 0x19u, false);
            return clamp(veins * 0.85 + cloud * 0.15, 0.0, 1.0);
        }
        case 11u: {
            let first = cells(p, seed);
            let second = cells(p * 2.7 + 13.0, seed ^ 0x6bu);
            return min(smoothstep(0.08, 0.16, first), smoothstep(0.06, 0.12, second));
        }
        case 12u: {
            let first = cells(p * 2.0, seed ^ 0x45u);
            return smoothstep(0.55, 0.58, fbm(p, 5u, seed, true) + (first - 0.5) * 0.25);
        }
        case 13u: {
            let fine = noise(p * vec3(37.0, 1.3, 1.0), seed ^ 0x2bu, false);
            return clamp(max(drip(p, p.x, 0, seed), drip(p, p.x + 0.37, 1, seed) * 0.8) * (0.6 + 0.4 * fine), 0.0, 1.0);
        }
        default: { return extended_pattern(kind, p, seed); }
    }
}

fn raw_procedural(layer: Operation, p: vec3<f32>) -> f32 {
    if layer.ids.y == 80u {
        return tile_generator(u32(layer.high.w), p.xy);
    }
    return pattern(layer.ids.y, p, layer.ids.z);
}

fn sample_procedural(layer: Operation, p: vec3<f32>) -> f32 {
    let reduced = layer.procedural.x != 0u;
    let cells = f32(max(layer.procedural.x, 1u)) / layer.uv.z;
    let grid = p.xy * cells - 0.5;
    let base = floor(grid) + 0.5;
    var values = vec4(0.0);
    for (var i = 0u; i < select(1u, 4u, reduced); i++) {
        let offset = vec2<f32>(vec2(i % 2u, i / 2u));
        let point = select(p.xy, (base + offset) / cells, reduced);
        let value = raw_procedural(layer, vec3(point, p.z));
        // Keeping the write at constant indices also lets FXC keep this loop rolled.
        switch i {
            case 0u: { values.x = value; }
            case 1u: { values.y = value; }
            case 2u: { values.z = value; }
            default: { values.w = value; }
        }
    }
    let t = fract(grid);
    return select(values.x, mix(mix(values.x, values.y, t.x), mix(values.z, values.w, t.x), t.y), reduced);
}

fn procedural_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec3<f32> {
    let mode = layer.procedural.w;
    let p = surface.position * layer.projection.w + layer.projection.xyz;
    var points = array<vec2<f32>, 3>(p.zy, p.xz, p.xy);
    var w = vec3(1.0, 0.0, 0.0);
    if mode == 1u {
        let weights = triplanar_weights(surface.normal, layer.balance.w);
        let minimum = 1.0 / (131072.0 * max(layer.uv.w, 1.0));
        w = select(vec3(0.0), weights, weights >= vec3(minimum));
    } else if mode == P1_PLANES_PROJECTION {
        // P1 placed the points and weighed the planes (composite_projection.wgsl)
        points = array<vec2<f32>, 3>(layer.projection.xy, layer.projection.zw, bitcast<vec2<f32>>(layer.constraints.xy));
        w = layer.balance.yzw;
    } else {
        switch mode {
            case 2u: {}
            case 3u: { points[0] = p.xz; }
            case 4u: { points[0] = p.xy; }
            case 5u: {
                let position = surface.position + layer.projection.xyz;
                var d = vec3(0.0, 1.0, 0.0);
                if dot(position, position) > 0.0 { d = normalize(position); }
                points[0] = vec2(azimuth(d.z, d.x) * 4.0, acos(clamp(d.y, -1.0, 1.0)) * (2.0 / 3.141592654)) * layer.projection.w;
            }
            case 6u, 7u, 8u: {
                let pos = surface.position + layer.projection.xyz;
                let axis = select(select(pos.xyz, pos.yxz, mode == 6u), pos.xzy, mode == 8u);
                points[0] = vec2(azimuth(axis.z, axis.x) * 4.0, axis.y) * layer.projection.w;
            }
            case DECAL_PROJECTION: { points[0] = uv * layer.uv.z; }
            case UV_TILE_PROJECTION: { points[0] = fract(uv) * layer.uv.z; }
            default: {
                let rotation = layer.projection.xy;
                let rotated = vec2(uv.x * rotation.x - uv.y * rotation.y, uv.x * rotation.y + uv.y * rotation.x);
                points[0] = (rotated * layer.projection.zw + layer.uv.xy) * layer.uv.z;
            }
        }
    }
    var n = 0.0;
    for (var plane = 0u; plane < select(1u, 3u, mode == 1u || mode == P1_PLANES_PROJECTION); plane++) {
        if w[plane] > 0.0 {
            n += w[plane] * sample_procedural(layer, vec3(points[plane], 0.0));
        }
    }
    let shaped = clamp((n - 0.5) * layer.uv.w + 0.5 + layer.balance.x, 0.0, 1.0);
    let t = select(shaped, 1.0 - shaped, layer.ids.w != 0u);
    if layer.procedural.z > 0u {
        return gradient_color(layer, t);
    }
    return layer.low.rgb + (layer.high.rgb - layer.low.rgb) * t;
}
