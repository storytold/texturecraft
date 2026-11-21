// Periodic geometry, organic, fabric and wood patterns.
// CPU formulas are in compose/src/{pattern,organic,fabric,timber}.rs.

const PATTERN_PI: f32 = 3.141592653589793;
const PATTERN_TAU: f32 = 6.283185307179586;

fn wrap_cell(cell: vec2<i32>, period: vec2<i32>) -> vec2<i32> {
    return vec2(wrap_coordinate(cell.x, period.x), wrap_coordinate(cell.y, period.y));
}

fn pattern_hash(cell: vec2<i32>, component: i32, seed: u32) -> f32 {
    return cell_value(vec3(cell, component), seed);
}

fn pattern_step(start: f32, end: f32, value: f32) -> f32 {
    // The CPU's smoothstep also supports a descending interval.
    let t = clamp((value - start) / (end - start), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn lattice_noise(at: vec2<f32>, cells: vec2<i32>, seed: u32) -> f32 {
    let point = at * vec2<f32>(cells);
    let cell = vec2<i32>(floor(point));
    let local = fract(point);
    let weight = local * local * (3.0 - 2.0 * local);
    let top = lerp(
        pattern_hash(wrap_cell(cell, cells), 0, seed),
        pattern_hash(wrap_cell(cell + vec2(1, 0), cells), 0, seed),
        weight.x,
    );
    let bottom = lerp(
        pattern_hash(wrap_cell(cell + vec2(0, 1), cells), 0, seed),
        pattern_hash(wrap_cell(cell + vec2(1, 1), cells), 0, seed),
        weight.x,
    );
    return lerp(top, bottom, weight.y);
}

fn lattice_octave(at: vec2<f32>, cells: i32, seed: u32, octave: u32) -> f32 {
    return lattice_noise(at, vec2(cells), seed ^ (octave * 0x9e37u));
}

fn lattice_fbm2(at: vec2<f32>, cells: vec2<i32>, weights: vec2<f32>, seed: u32) -> f32 {
    return weights.x * lattice_octave(at, cells.x, seed, 0u)
         + weights.y * lattice_octave(at, cells.y, seed, 1u);
}

fn lattice_fbm3(at: vec2<f32>, cells: vec3<i32>, weights: vec3<f32>, seed: u32) -> f32 {
    return weights.x * lattice_octave(at, cells.x, seed, 0u)
         + weights.y * lattice_octave(at, cells.y, seed, 1u)
         + weights.z * lattice_octave(at, cells.z, seed, 2u);
}

fn round_pattern(p: vec2<f32>) -> vec2<f32> {
    // Rust round resolves half-integers away from zero.
    return sign(p) * floor(abs(p) + 0.5);
}

fn hexagon_tiles(uv: vec2<f32>, seed: u32) -> f32 {
    let repeat = vec2(1.0, 1.7320508);
    let point = uv * vec2(2.0, repeat.y);
    let shifted = point - repeat * 0.5;
    let first = point - repeat * round_pattern(point / repeat);
    let second = shifted - repeat * round_pattern(shifted / repeat);
    let local = select(second, first, dot(first, first) < dot(second, second));
    let reach = max(abs(local.x), max(abs(local.x * 0.5 + local.y * 0.8660254), abs(local.x * 0.5 - local.y * 0.8660254)));
    let joint = smoothstep(0.012, 0.045, 0.5 - reach);
    let center = point - local;
    let cell = vec2<i32>(round_pattern(vec2(center.x * 2.0, center.y / repeat.y * 2.0)));
    let tone = pattern_hash(wrap_cell(cell, vec2(4, 2)), 0, seed);
    return joint * (0.8 + 0.2 * tone);
}

fn wave_scales(uv: vec2<f32>) -> f32 {
    let row = i32(floor(uv.y / 0.25));
    for (var offset = 2; offset >= -2; offset--) {
        let circle_row = row + offset;
        let dy = uv.y - f32(circle_row) * 0.25;
        if abs(dy) > 0.5 {
            continue;
        }
        let shift = select(0.0, 0.5, (circle_row & 1) == 1);
        let shifted = uv.x - shift;
        let dx = uv.x - (sign(shifted) * floor(abs(shifted) + 0.5) + shift);
        let radial = length(vec2(dx, dy)) / 0.5;
        if radial <= 1.0 {
            let arcs = 0.5 + 0.5 * cos(PATTERN_TAU * 3.5 * radial);
            let rim = 1.0 - 0.7 * smoothstep(0.9, 1.0, radial);
            return clamp((0.3 + 0.7 * arcs) * rim, 0.0, 1.0);
        }
    }
    return 0.0;
}

fn herringbone_planks(uv: vec2<f32>, seed: u32) -> f32 {
    let point = uv * 6.0;
    let cell = vec2<i32>(floor(point));
    let local = fract(point);
    let phase = (cell.x - cell.y + 6) % 6;
    let step = 5 - phase;
    let horizontal = phase < 3;
    let origin = select(cell - vec2(0, step), cell - vec2(phase, 0), horizontal);
    let along = select(f32(step) + local.y, f32(phase) + local.x, horizontal) / 3.0;
    let across = select(local.x, local.y, horizontal);
    let edge = min(min(across, 1.0 - across), min(along, 1.0 - along) * 3.0);
    let body = smoothstep(0.015, 0.06, edge);
    let tone = pattern_hash(wrap_cell(origin, vec2(6)), select(1, 0, horizontal), seed);
    let grain = noise(vec3(along * 3.0, across * 28.0, tone * 31.0), seed ^ 0x7bu, false);
    return body * (0.55 + 0.25 * tone + 0.2 * grain);
}

fn halftone_dots(uv: vec2<f32>, seed: u32) -> f32 {
    let point = uv * 16.0;
    let cell = floor(vec2(point.x + point.y, point.x - point.y) * 0.5);
    let corners = array(vec2(0.0), vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(1.0));
    var coverage = 0.0;
    for (var i = 0u; i < 4u; i++) {
        let corner = cell + corners[i];
        let center = vec2(corner.x + corner.y, corner.x - corner.y);
        let distance = length(point - center);
        if distance < 0.12 + 0.6 + 0.05 {
            let tone = smoothstep(0.25, 0.75, lattice_fbm3(center / 16.0, vec3(3, 6, 12), vec3(0.55, 0.3, 0.15), seed));
            let radius = 0.12 + 0.6 * tone;
            coverage = max(coverage, 1.0 - smoothstep(radius - 0.05, radius + 0.05, distance));
        }
    }
    return coverage;
}

fn diamond_brocade(uv: vec2<f32>) -> f32 {
    let point = uv * 4.0;
    let local = abs(fract(point) - 0.5);
    let distance = local.x + local.y;
    let ribs = 0.5 + 0.5 * sin(PATTERN_TAU * 40.0 * (point.x + point.y) / 4.0);
    let ground = 0.22 + 0.12 * ribs;
    if distance >= 0.46 {
        return ground;
    }

    let inside = 1.0 - smoothstep(0.46 - 0.03, 0.46, distance);
    let rings = 0.5 + 0.5 * cos(PATTERN_TAU * 2.5 * distance / 0.46);
    let rise = max(sin(PATTERN_PI * 0.5 * (1.0 - distance / 0.46)), 0.0);
    let motif = (0.45 + 0.35 * rise + 0.2 * rings) * (0.9 + 0.1 * ribs);
    return clamp(lerp(ground, motif, inside), 0.0, 1.0);
}

fn truchet_arcs(uv: vec2<f32>, seed: u32) -> f32 {
    let point = uv * 8.0;
    let cell = vec2<i32>(floor(point));
    let local = fract(point);
    let flipped = pattern_hash(wrap_cell(cell, vec2(8)), 0, seed) > 0.5;
    let corner = select(vec2(0.0), vec2(1.0, 0.0), flipped);
    let distance = min(abs(length(local - corner) - 0.5), abs(length(local - (1.0 - corner)) - 0.5));
    let across = min(distance / 0.13, 1.0);
    return 0.05 + 0.9 * sqrt(1.0 - across * across);
}

fn smooth_fur(uv: vec2<f32>, seed: u32) -> f32 {
    let point = uv * 6.0;
    let cell = vec2<i32>(floor(point));
    var height = 0.0;
    for (var row = -5; row <= 1; row++) {
        for (var column = -5; column <= 5; column++) {
            let root_cell = cell + vec2(column, row);
            let delta_center = point - (vec2<f32>(root_cell) + 0.5);
            // A root moves by at most 0.35 on each axis. Discard unreachable roots
            // before hashing their positions and tangents.
            let nearest_root = max(abs(delta_center) - 0.35, vec2(0.0));
            if dot(nearest_root, nearest_root) > 16.0 {
                continue;
            }
            let wrapped = wrap_cell(root_cell, vec2(6));
            let salt = seed ^ 0x6du;
            let jitter = (vec2(pattern_hash(wrapped, 1, salt), pattern_hash(wrapped, 2, salt)) - 0.5) * 0.7;
            let delta = delta_center - jitter;
            if dot(delta, delta) > 16.0 {
                continue;
            }

            let angle = 0.1 + (pattern_hash(wrapped, 3, salt) * 2.0 - 1.0) * 0.45;
            let sine = sin(angle);
            let cosine = cos(angle);
            let local = vec2(cosine * delta.x + sine * delta.y, -sine * delta.x + cosine * delta.y);
            let lock_length = 3.0 * (1.0 + (pattern_hash(wrapped, 4, salt) * 2.0 - 1.0) * 0.27);
            if local.y < -1.55 * 0.5 || local.y > lock_length {
                continue;
            }

            let curl = -0.8 * (0.8 + pattern_hash(wrapped, 5, salt) * 0.4);
            let arc_radius = lock_length / curl;
            let along = atan2(local.y / arc_radius, (arc_radius - local.x) / arc_radius) / curl;
            if along < 0.0 || along > 1.0 {
                continue;
            }
            let radius = 1.55 * 0.5 * pow(1.0 - along, 0.65);
            let offset = length(vec2(local.x - arc_radius, local.y)) - abs(arc_radius);
            if radius <= 1e-6 || abs(offset) >= radius {
                continue;
            }

            let across = abs(offset) / radius;
            let profile = sqrt(1.0 - across * across);
            let strength = 0.8 + pattern_hash(wrapped, 6, salt) * 0.2;
            let lock_height = strength * profile * smoothstep(0.0, 1.0, along / 0.06)
                           * smoothstep(0.0, 1.0, (1.0 - along) / 0.025) * (0.28 + 0.72 * along);
            height = max(height, lock_height);
        }
    }
    return clamp(height, 0.0, 1.0);
}

fn mackerel_sky(uv: vec2<f32>, seed: u32) -> f32 {
    let point = uv * 6.0;
    let cell = vec2<i32>(floor(point));
    let salt = seed ^ 0x5fu;
    var sum = 0.0;
    for (var row = -2; row <= 2; row++) {
        for (var column = -2; column <= 2; column++) {
            let cloud = cell + vec2(column, row);
            let wrapped = wrap_cell(cloud, vec2(6));
            let center = vec2<f32>(cloud) + vec2(pattern_hash(wrapped, 1, salt), pattern_hash(wrapped, 2, salt));
            let radial = length(point - center) / (0.65 + 0.45 * pattern_hash(wrapped, 3, salt));
            if radial < 1.2 {
                let dome = pattern_step(1.2, 0.2, radial) * (0.85 + 0.15 * sqrt(max(1.0 - radial * radial, 0.0)));
                sum += exp(14.0 * dome * (0.6 + 0.4 * pattern_hash(wrapped, 4, salt))) - 1.0;
            }
        }
    }
    let value = log(1.0 + sum) / 14.0;
    let mottle = lattice_fbm3(uv, vec3(12, 24, 48), vec3(0.5, 0.3, 0.2), seed ^ 0x3au);
    return clamp(value * (0.88 + 0.12 * mottle), 0.0, 1.0);
}

fn pattern_warp(uv: vec2<f32>, seed: u32) -> vec2<f32> {
    let cells = vec3(2, 4, 8);
    let weights = vec3(0.5, 0.3, 0.2);
    return vec2(lattice_fbm3(uv, cells, weights, seed ^ 0x11u), lattice_fbm3(uv, cells, weights, seed ^ 0x22u)) - 0.5;
}

fn billow(uv: vec2<f32>, seed: u32) -> f32 {
    let warped = uv + pattern_warp(uv, seed) * 0.2;
    let body = lattice_fbm3(warped, vec3(3, 6, 12), vec3(0.3, 0.25, 0.2), seed)
             + 0.15 * lattice_octave(warped, 24, seed, 3u)
             + 0.1 * lattice_octave(warped, 48, seed, 4u);
    let angle = PATTERN_TAU * lattice_fbm2(warped, vec2(2, 4), vec2(0.6, 0.4), seed ^ 0x33u);
    let flow = vec2(cos(angle), sin(angle));
    var strands = 0.0;
    for (var sample = -5; sample <= 5; sample++) {
        strands += lattice_noise(warped + flow * (f32(sample) * 0.0045), vec2(64), seed ^ 0x44u);
    }
    let speckle = lattice_noise(warped, vec2(32), seed ^ 0x55u);
    return clamp(0.5 + (body - 0.5) * 1.7 + (strands / 11.0 - 0.5) * 0.45 + (speckle - 0.5) * 0.1, 0.0, 1.0);
}

fn reptile_scales(uv: vec2<f32>, seed: u32) -> f32 {
    let point = uv * vec2(8.0, 12.0);
    let cell = vec2<i32>(floor(point));
    let salt = seed ^ 0x2bu;
    var top_row = 0x7fffffffi;
    var top = vec2(0.0);
    var top_rim = 0.0;
    var tone = 0.0;
    var ridge = 0.0;
    var shadow = 0.0;
    for (var row = -2; row <= 2; row++) {
        let scale_row = cell.y + row;
        let stagger = select(0.0, 0.5, (scale_row & 1) == 1);
        for (var column = -1; column <= 2; column++) {
            let scale_cell = cell + vec2(column, row);
            let wrapped = wrap_cell(scale_cell, vec2(8, 12));
            let jitter = (vec2(pattern_hash(wrapped, 1, salt), pattern_hash(wrapped, 2, salt)) - 0.5) * vec2(0.3, 0.4);
            let center = vec2<f32>(scale_cell) + vec2(0.5 + stagger, 0.5) + jitter;
            let size = vec2(0.62, 1.3) * (0.92 + 0.16 * vec2(pattern_hash(wrapped, 3, salt), pattern_hash(wrapped, 4, salt)));
            let delta = point - center;
            let narrowing = 1.0 - 0.3 * clamp(delta.y / size.y, 0.0, 1.0);
            let local = delta / (size * vec2(narrowing, 1.0));
            let rim = length(local);
            if rim < 1.0 {
                if scale_row < top_row || (scale_row == top_row && pattern_hash(wrapped, 5, salt) > 0.5) {
                    top_row = scale_row;
                    top = local;
                    top_rim = rim;
                    tone = pattern_hash(wrapped, 6, salt);
                    ridge = pattern_hash(wrapped, 7, salt);
                }
            } else if rim < 1.35 && delta.y > 0.0 {
                shadow = max(shadow, pattern_step(1.35, 1.0, rim));
            }
        }
    }
    if top_row == 0x7fffffffi {
        return 0.1;
    }

    let height = sqrt(max(1.0 - top_rim * top_rim, 0.0));
    let keel = exp(-(top.x * 3.2) * (top.x * 3.2)) * (0.1 + 0.25 * ridge);
    let normal = normalize(vec3(top * 0.9, height + keel));
    let lit = max(dot(normal, normalize(vec3(-0.35, -0.5, 0.8))), 0.0);
    let grain = lattice_noise(uv, vec2(96), seed ^ 0x3cu);
    let body = (0.25 + 0.75 * tone) * (0.35 + 0.65 * lit) * (1.0 - 0.5 * shadow) * (0.86 + 0.14 * grain);
    return clamp(body * (0.45 + 0.55 * pattern_step(1.0, 0.8, top_rim)), 0.0, 1.0);
}

fn bark(uv: vec2<f32>, seed: u32) -> f32 {
    let wobble = vec2(lattice_noise(uv, vec2(5), seed ^ 0x31u), lattice_noise(uv, vec2(5), seed ^ 0x32u)) - 0.5;
    let warped = uv + wobble * vec2(0.06, 0.14);
    let point = warped * vec2(7.0, 3.0);
    let cell = vec2<i32>(floor(point));
    let salt = seed ^ 0x27u;
    var nearest = 3.402823466e+38;
    var second = nearest;
    var tone = 0.0;
    for (var row = -2; row <= 2; row++) {
        for (var column = -1; column <= 1; column++) {
            let plate = cell + vec2(column, row);
            let wrapped = wrap_cell(plate, vec2(7, 3));
            let jitter = (vec2(pattern_hash(wrapped, 1, salt), pattern_hash(wrapped, 2, salt)) - 0.5) * 0.85;
            let delta = point - (vec2<f32>(plate) + 0.5 + jitter);
            let distance = length(vec2(delta.x / 7.0, delta.y / 3.0 * 0.38));
            if distance < nearest {
                second = nearest;
                nearest = distance;
                tone = pattern_hash(wrapped, 3, salt);
            } else {
                second = min(second, distance);
            }
        }
    }
    let ridge = sqrt(smoothstep(0.0, 0.4, (second - nearest) * 7.0));
    let fibre = lattice_noise(warped, vec2(96, 6), seed ^ 0x52u);
    return clamp(ridge * (0.5 + 0.3 * tone + 0.2 * fibre), 0.0, 1.0);
}

fn flesh_vein(warped: vec2<f32>, cells: vec2<i32>, seed: u32, sharpness: f32) -> f32 {
    let line = lattice_fbm2(warped, cells, vec2(0.6, 0.4), seed);
    return pow(max(1.0 - abs(2.0 * line - 1.0), 0.0), sharpness);
}

fn flesh(uv: vec2<f32>, seed: u32) -> f32 {
    let warped = uv + pattern_warp(uv, seed) * 0.24;
    let body_seed = seed ^ 0x33u;
    let body = lattice_fbm3(warped, vec3(3, 6, 12), vec3(0.35, 0.3, 0.2), body_seed)
             + 0.15 * lattice_octave(warped, 24, body_seed, 3u);
    let fold = lattice_fbm2(warped, vec2(5, 10), vec2(0.6, 0.4), seed ^ 0x44u);
    let crease = pow(max(1.0 - abs(2.0 * fold - 1.0), 0.0), 4.0);
    let vein = max(flesh_vein(warped, vec2(3, 7), seed ^ 0x55u, 14.0), 0.7 * flesh_vein(warped, vec2(6, 13), seed ^ 0x56u, 20.0));
    let healthy = (0.2 + 0.7 * smoothstep(0.2, 0.8, body)) * (1.0 - 0.7 * crease) + 0.3 * vein;
    let decay = smoothstep(0.46, 0.58, lattice_fbm3(warped, vec3(2, 5, 10), vec3(0.5, 0.3, 0.2), seed ^ 0x66u));
    if decay == 0.0 {
        return clamp(healthy, 0.0, 1.0);
    }
    let mottle = lattice_fbm2(warped, vec2(24, 48), vec2(0.5), seed ^ 0x77u);
    let point = warped * 14.0;
    let cell = vec2<i32>(floor(point));
    let salt = seed ^ 0x88u;
    var blister = 0.0;
    for (var row = -1; row <= 1; row++) {
        for (var column = -1; column <= 1; column++) {
            let blister_cell = cell + vec2(column, row);
            let wrapped = wrap_cell(blister_cell, vec2(14));
            if pattern_hash(wrapped, 0, salt) > 0.55 {
                continue;
            }
            let center = vec2<f32>(blister_cell) + vec2(pattern_hash(wrapped, 1, salt), pattern_hash(wrapped, 2, salt));
            let radial = length(point - center) / (0.3 + 0.4 * pattern_hash(wrapped, 3, salt));
            blister = max(blister, pattern_step(1.0, 0.0, radial));
        }
    }
    let rotten = healthy * (0.3 + 0.4 * mottle) + 0.35 * blister;
    return clamp(lerp(healthy, rotten, decay), 0.0, 1.0);
}

fn camouflage_blotches(uv: vec2<f32>, cells: vec2<i32>, seed: u32) -> f32 {
    return 0.55 * lattice_noise(uv, cells, seed)
         + 0.3 * lattice_noise(uv, cells * 2, seed ^ 0x9eu)
         + 0.15 * lattice_noise(uv, cells * 4, seed ^ 0x3fu);
}

fn camouflage(uv: vec2<f32>, seed: u32) -> f32 {
    let ragged = (lattice_fbm2(uv, vec2(24, 48), vec2(0.6, 0.4), seed ^ 0x71u) - 0.5) * 0.14;
    let low = camouflage_blotches(uv, vec2(3, 2), seed ^ 0x11u) + ragged;
    let middle = camouflage_blotches(uv, vec2(4, 3), seed ^ 0x22u) + ragged;
    let high = camouflage_blotches(uv, vec2(6, 4), seed ^ 0x33u) + ragged;
    let first = lerp(0.05, 0.35, smoothstep(0.46 - 0.012, 0.46 + 0.012, low));
    let second = lerp(first, 0.65, smoothstep(0.54 - 0.012, 0.54 + 0.012, middle));
    return clamp(lerp(second, 0.95, smoothstep(0.6 - 0.012, 0.6 + 0.012, high)), 0.0, 1.0);
}

fn thread_profile(across: f32) -> f32 {
    let centered = 2.0 * across - 1.0;
    return sqrt(max(1.0 - centered * centered, 0.0));
}

fn thread_bulge(along: f32) -> f32 {
    return 0.55 + 0.45 * sin(PATTERN_PI * along);
}

fn knit(uv: vec2<f32>, seed: u32) -> f32 {
    let point = uv * vec2(10.0, 14.0);
    let cell = vec2<i32>(floor(point));
    var height = 0.0;
    var tone = 0.5;
    for (var row = -1; row <= 1; row++) {
        for (var column = -1; column <= 1; column++) {
            let stitch = cell + vec2(column, row);
            let yarn = 0.85 + 0.15 * pattern_hash(wrap_cell(stitch, vec2(10, 14)), 0, seed ^ 0x4bu);
            let delta = point - vec2<f32>(stitch);
            for (var side = -1; side <= 1; side += 2) {
                let start = vec2(select(0.1, 0.9, side > 0), -0.05);
                let direction = vec2(0.5, 1.05) - start;
                let along = clamp(dot(delta - start, direction) / dot(direction, direction), 0.0, 1.0);
                let distance = length(delta - (start + direction * along));
                if distance >= 0.21 {
                    continue;
                }
                let across = distance / 0.21;
                let twist = 0.5 + 0.5 * cos(PATTERN_TAU * (along * 2.5 + f32(side) * across * 0.3));
                let leg = sqrt(1.0 - across * across) * (0.82 + 0.18 * twist) * yarn;
                if leg > height {
                    height = leg;
                    tone = yarn;
                }
            }
        }
    }
    return clamp(0.12 + 0.88 * height * (0.8 + 0.2 * tone), 0.0, 1.0);
}

fn basket_weave(uv: vec2<f32>, seed: u32) -> f32 {
    let point = uv * 4.0;
    let cell = vec2<i32>(floor(point));
    let local = fract(point);
    let horizontal = ((cell.x + cell.y) & 1) == 0;
    let along = select(local.y, local.x, horizontal);
    let position = select(local.x, local.y, horizontal) * 2.0;
    let thread = i32(floor(position));
    let across = fract(position);
    let tone = pattern_hash(wrap_cell(vec2(cell.x * 2 + thread, cell.y), vec2(8, 4)), i32(horizontal), seed ^ 0x2du);
    let fibre = 0.9 + 0.1 * cos(PATTERN_TAU * across * 6.0);
    let crest = thread_profile(across) * thread_bulge(along) * fibre;
    return clamp((0.1 + 0.9 * crest) * (0.75 + 0.25 * tone) * select(0.9, 1.0, horizontal), 0.0, 1.0);
}

fn goose_warp_over(cell: vec2<i32>) -> bool {
    let from_turn = abs(wrap_cell(cell, vec2(12)) - 6);
    return (from_turn.x + from_turn.y) % 4 < 2;
}

fn goose_tuck(along: f32, before: bool, after: bool) -> f32 {
    let start = select(0.55 + 0.45 * smoothstep(0.0, 0.3, along), 1.0, before);
    let end = select(0.55 + 0.45 * pattern_step(1.0, 0.7, along), 1.0, after);
    return start * end;
}

fn goose_eye(uv: vec2<f32>, seed: u32) -> f32 {
    let q = uv * 24.0;
    let cell = vec2<i32>(floor(q));
    let local = fract(q);
    let tone = pattern_hash(wrap_cell(cell, vec2(24)), 0, seed ^ 0x3eu);
    var crest: f32;
    var brightness: f32;
    if goose_warp_over(cell) {
        let before = goose_warp_over(cell - vec2(0, 1));
        let after = goose_warp_over(cell + vec2(0, 1));
        crest = thread_profile(local.x) * goose_tuck(local.y, before, after);
        brightness = 1.0;
    } else {
        let before = !goose_warp_over(cell - vec2(1, 0));
        let after = !goose_warp_over(cell + vec2(1, 0));
        crest = thread_profile(local.y) * goose_tuck(local.x, before, after);
        brightness = 0.4;
    }
    return clamp((0.1 + 0.9 * crest) * brightness * (0.9 + 0.1 * tone), 0.0, 1.0);
}

fn silk_slub(thread: i32, along_unit: f32, warp: bool, seed: u32) -> f32 {
    let along = along_unit * 7.0;
    let cell = i32(floor(along));
    let fraction = fract(along);
    let salt = seed ^ 0x5au;
    let start = pattern_hash(wrap_cell(vec2(thread, cell), vec2(36, 7)), i32(warp), salt);
    let end = pattern_hash(wrap_cell(vec2(thread, cell + 1), vec2(36, 7)), i32(warp), salt);
    return smoothstep(0.74, 0.93, start + (end - start) * fraction * fraction * (3.0 - 2.0 * fraction));
}

fn slub_silk(uv: vec2<f32>, seed: u32) -> f32 {
    let q = uv * 36.0;
    let cell = vec2<i32>(floor(q));
    let local = fract(q);
    let warp_slub = silk_slub(cell.x, uv.y, true, seed);
    let weft_slub = silk_slub(cell.y, uv.x, false, seed);
    let warp_over = warp_slub > 0.5 || (weft_slub <= 0.5 && ((cell.x + cell.y) & 1) == 0);
    let thread = select(cell.y, cell.x, warp_over);
    let slub = select(weft_slub, warp_slub, warp_over);
    let across = select(local.y, local.x, warp_over);
    let along = select(local.x, local.y, warp_over);
    let sheen = 0.45 + 0.25 * pattern_hash(vec2(thread % 36, 99), i32(warp_over), seed ^ 0x6bu);
    let tuck = select(thread_bulge(along), 1.0, slub > 0.5);
    let crest = thread_profile(across) * tuck * (0.75 + 0.45 * slub);
    return clamp((0.25 + 0.75 * crest) * (sheen + 0.28 * slub), 0.0, 1.0);
}

fn strip_flooring(uv: vec2<f32>, seed: u32) -> f32 {
    let y = uv.y * 6.0;
    let row = i32(floor(y)) % 6;
    let across = fract(y);
    let salt = seed ^ 0x13u;
    let row_offset = pattern_hash(vec2(row, 0), 0, salt);
    let x = (uv.x + row_offset) * 2.0;
    let piece = i32(floor(x)) % 2;
    let along = fract(x);
    let board = vec2(row, piece);
    let to_edge = min(across, 1.0 - across) / 6.0;
    let to_joint = min(along, 1.0 - along) / 2.0;
    let gap = smoothstep(0.0012, 0.0035, min(to_edge, to_joint));
    let warp = lattice_fbm2(uv, vec2(3, 7), vec2(0.6, 0.4), seed ^ 0x24u) - 0.5;
    let rings = across * (1.6 + 1.2 * pattern_hash(board, 1, salt)) + pattern_hash(board, 2, salt) * 7.0 + warp * 2.2;
    let ring = 0.5 + 0.5 * cos(PATTERN_TAU * rings);
    let fibre = lattice_noise(uv, vec2(12, 240), seed ^ 0x35u);
    let tone = 0.42 + 0.4 * pattern_hash(board, 3, salt);
    return clamp(gap * tone * (0.68 + 0.32 * pow(ring, 1.4)) * (0.86 + 0.14 * fibre), 0.0, 1.0);
}

fn knotty_wood(uv: vec2<f32>, seed: u32) -> f32 {
    let wander = (lattice_fbm2(uv, vec2(2, 5), vec2(0.6, 0.4), seed ^ 0x14u) - 0.5) * 1.4;
    let cell = vec2<i32>(floor(uv * 3.0));
    let salt = seed ^ 0x47u;
    var phase = uv.y * 9.0 + wander;
    var knot_core = 0.0;
    var ring_noise = 0.0;
    var with_noise = false;
    for (var row = -1; row <= 1; row++) {
        for (var column = -1; column <= 1; column++) {
            let knot = cell + vec2(column, row);
            let wrapped = wrap_cell(knot, vec2(3));
            if pattern_hash(wrapped, 0, salt) > 0.45 {
                continue;
            }
            let jitter = vec2(pattern_hash(wrapped, 1, salt), pattern_hash(wrapped, 2, salt)) * 0.6;
            let center = vec2<f32>(knot) + 0.2 + jitter;
            let radius = 0.04 + 0.035 * pattern_hash(wrapped, 3, salt);
            let delta = uv - center / 3.0;
            let distance = length(delta * vec2(0.5, 1.0)) / radius;
            if distance >= 2.2 {
                continue;
            }
            if !with_noise {
                ring_noise = (lattice_noise(uv, vec2(24), seed ^ 0x66u) - 0.5) * 0.8;
                with_noise = true;
            }
            let circle = distance * 1.9 + pattern_hash(wrapped, 4, salt) * 3.0 + ring_noise;
            let home_y = (f32(wrapped.y) + 0.2 + jitter.y) / 3.0;
            let ring_here = (home_y + delta.y) * 9.0 + wander;
            phase += (circle - ring_here) * pattern_step(2.2, 0.8, distance);
            knot_core = max(knot_core, pattern_step(0.5, 0.2, distance));
        }
    }
    let ring = pow(0.5 + 0.5 * cos(PATTERN_TAU * phase), 1.6);
    let fibre = lattice_noise(uv, vec2(10, 260), seed ^ 0x58u);
    let base = 0.34 + 0.5 * ring;
    return clamp(base * (0.86 + 0.14 * fibre) * (1.0 - 0.8 * knot_core), 0.0, 1.0);
}

fn periodic_pattern(kind: u32, p: vec3<f32>, seed: u32) -> f32 {
    let uv = fract(p.xy);
    switch kind {
        case 60u: { return hexagon_tiles(uv, seed); }
        case 61u: { return wave_scales(uv); }
        case 62u: { return herringbone_planks(uv, seed); }
        case 63u: { return halftone_dots(uv, seed); }
        case 64u: { return diamond_brocade(uv); }
        case 65u: { return truchet_arcs(uv, seed); }
        case 66u: { return smooth_fur(uv, seed); }
        case 67u: { return mackerel_sky(uv, seed); }
        case 68u: { return billow(uv, seed); }
        case 69u: { return reptile_scales(uv, seed); }
        case 70u: { return bark(uv, seed); }
        case 71u: { return flesh(uv, seed); }
        case 72u: { return camouflage(uv, seed); }
        case 73u: { return knit(uv, seed); }
        case 74u: { return basket_weave(uv, seed); }
        case 75u: { return goose_eye(uv, seed); }
        case 76u: { return slub_silk(uv, seed); }
        case 77u: { return strip_flooring(uv, seed); }
        case 78u: { return knotty_wood(uv, seed); }
        default: { return uv.x; }
    }
}
