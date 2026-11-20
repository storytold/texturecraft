// Deterministic lattice noise shared with the CPU kernels in texel/src/noise.rs.

fn pcg3d(cell: vec3<u32>) -> vec3<u32> {
    var v = cell * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v = v ^ (v >> vec3<u32>(16u));
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return v;
}

fn hash(cell: vec3<i32>, salt: u32) -> vec3<u32> {
    let bits = bitcast<vec3<u32>>(cell);
    return pcg3d(vec3<u32>(bits.x ^ salt, bits.yz));
}

const GRADIENTS = array<vec3<f32>, 12>(
    vec3(1.0, 1.0, 0.0), vec3(-1.0, 1.0, 0.0), vec3(1.0, -1.0, 0.0), vec3(-1.0, -1.0, 0.0),
    vec3(1.0, 0.0, 1.0), vec3(-1.0, 0.0, 1.0), vec3(1.0, 0.0, -1.0), vec3(-1.0, 0.0, -1.0),
    vec3(0.0, 1.0, 1.0), vec3(0.0, -1.0, 1.0), vec3(0.0, 1.0, -1.0), vec3(0.0, -1.0, -1.0),
);

fn lerp(a: f32, b: f32, w: f32) -> f32 {
    return a + (b - a) * w;
}

fn corner(cell: vec3<i32>, seed: u32, offset: vec3<i32>, local: vec3<f32>, gradient: bool) -> f32 {
    if gradient {
        return dot(GRADIENTS[hash(cell + offset, seed * 0x85ebca6bu).y % 12u], local - vec3<f32>(offset));
    }
    return f32(hash(cell + offset, seed * 0x9e3779b9u).x) / 4294967296.0;
}

fn noise(p: vec3<f32>, seed: u32, gradient: bool) -> f32 {
    let cell = vec3<i32>(floor(p));
    let local = fract(p);
    let w = local * local * local * (local * (local * 6.0 - 15.0) + 10.0);
    let xy0 = lerp(
        lerp(corner(cell, seed, vec3(0, 0, 0), local, gradient), corner(cell, seed, vec3(1, 0, 0), local, gradient), w.x),
        lerp(corner(cell, seed, vec3(0, 1, 0), local, gradient), corner(cell, seed, vec3(1, 1, 0), local, gradient), w.x),
        w.y);
    let xy1 = lerp(
        lerp(corner(cell, seed, vec3(0, 0, 1), local, gradient), corner(cell, seed, vec3(1, 0, 1), local, gradient), w.x),
        lerp(corner(cell, seed, vec3(0, 1, 1), local, gradient), corner(cell, seed, vec3(1, 1, 1), local, gradient), w.x),
        w.y);
    let value = lerp(xy0, xy1, w.z);
    return select(value, clamp(value * 0.5 + 0.5, 0.0, 1.0), gradient);
}

fn fbm(p: vec3<f32>, octaves: u32, seed: u32, gradient: bool) -> f32 {
    var position = p;
    var weight = 0.5;
    var normalization = 0.0;
    var sum = 0.0;
    for (var i = 0u; i < octaves; i++) {
        sum += weight * noise(position, seed + i * 31u, gradient);
        normalization += weight;
        position = position * 2.0 + 17.13;
        weight *= 0.5;
    }
    return sum / normalization;
}
