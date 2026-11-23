// P1, fill sampling and projection (gpu_composite_projection.rs). Owned by the P1 agent.
// Compiled into the composite shader when the program needs it, else replaced by the stub
// string in the Rust module (keep both signatures identical). composite_projection_shared.wgsl
// comes first in both.
//
// A fill on P1's path ports the CPU evaluator's fill texel (compose eval.rs `fill_texel`,
// `symmetric`, `cover`, `source`) with the projection's numbers from the side table. It reads
// images and patterns through `value` and `procedural_value` with a copy of the operation that
// carries the projected points, so it calls nothing the image and pattern stubs lack.

// Operation.ext.y (FLAG_* in gpu_composite_projection.rs; the sampling bits are shared)
const P1_TILE_COVER: u32 = 32u;
const P1_TURNS: u32 = 64u;
const P1_CROP: u32 = 128u;
const P1_BACKFACE: u32 = 256u;
const P1_SYMMETRY: u32 = 512u;
const P1_DEPTH: u32 = 1024u;
const P1_FRAMED: u32 = 2048u;
const P1_IMAGE_ALPHA: u32 = 4096u;
const P1_FRAME_VALID: u32 = 8192u;
const P1_UV_PLACED: u32 = 16384u;

// Side-table words (SLOT_* in gpu_composite_projection.rs)
const P1_KIND: u32 = 0u;
const P1_AXIS: u32 = 1u;
const P1_SCALE: u32 = 2u;
const P1_SHARPNESS: u32 = 4u;
const P1_OFFSET: u32 = 5u;
const P1_ROTATION: u32 = 8u;
const P1_TO_LOCAL: u32 = 10u;
const P1_STRETCH: u32 = 19u;
const P1_UV_TILING: u32 = 22u;
const P1_UV_ROTATION: u32 = 24u;
const P1_UV_OFFSET: u32 = 26u;
const P1_CYLINDER_ANGLE: u32 = 28u;
const P1_BACKFACE_THRESHOLD: u32 = 29u;
const P1_BACKFACE_WIDTH: u32 = 30u;
const P1_FRAME_ORIGIN: u32 = 31u;
const P1_FRAME_RIGHT: u32 = 34u;
const P1_FRAME_UP: u32 = 37u;
const P1_FRAME_NORMAL: u32 = 40u;
const P1_FRAME_SIZE: u32 = 43u;
const P1_FRAME_DEPTH: u32 = 45u;
const P1_FRAME_FLIP: u32 = 46u;
const P1_FRAME_MIRROR_AXIS: u32 = 47u;
const P1_FRAME_MIRROR_OFFSET: u32 = 48u;
const P1_FRAME_MIRROR_FLIP: u32 = 49u;
const P1_WARP_CORNERS: u32 = 50u;
const P1_SYMMETRY_KIND: u32 = 58u;
const P1_SYMMETRY_AXIS: u32 = 59u;
const P1_SYMMETRY_COUNT: u32 = 60u;
const P1_SYMMETRY_POSITION: u32 = 61u;
const P1_SYMMETRY_STEP: u32 = 64u;
const P1_SYMMETRY_SPAN: u32 = 65u;
const P1_SYMMETRY_FLIP: u32 = 66u;
const P1_DEPTH_HARDNESS: u32 = 67u;
const P1_DEPTH_DIRECTORY: u32 = 68u;

// Projection kinds in the side table (KIND_* in gpu_composite_projection.rs)
const P1_KIND_UV: u32 = 0u;
const P1_KIND_UV_TILE: u32 = 1u;
const P1_KIND_TRIPLANAR: u32 = 2u;
const P1_KIND_PLANAR: u32 = 3u;
const P1_KIND_SPHERICAL: u32 = 4u;
const P1_KIND_CYLINDRICAL: u32 = 5u;
const P1_KIND_PLACED: u32 = 6u;
const P1_KIND_WARP: u32 = 7u;

const P1_MISSING: u32 = 0xffffffffu;
const P1_ANCHOR_SOURCE: u32 = 15u;
// Operation.ids.w of an image read that returns the samples as stored (no decoding)
const P1_RAW_KIND: u32 = 3u;
const P1_LARGEST_FLOAT: f32 = 3.402823466e38;
// Pattern tiles round and down a spherical projection, and round a cylindrical one (compose pattern.rs)
const P1_SPHERE_PATTERN_TILES: vec2<f32> = vec2(4.0, 2.0);
const P1_CYLINDER_PATTERN_TILES: f32 = 4.0;
// Lighter triplanar planes than this cannot change a 0..1 pattern (compose pattern.rs TRIPLANAR_SKIP)
const P1_TRIPLANAR_SKIP: f32 = 1.0 / 131072.0;
const P1_FULL_TURN_DEGREES: f32 = 360.0;
const P1_MIN_CYLINDER_ANGLE: f32 = 1.0;

// Where a projection reads the texel (compose pattern.rs Coords): UV, position in mesh radii
// from the mesh's centre, normal
struct P1Coords {
    uv: vec2<f32>,
    pos: vec3<f32>,
    nrm: vec3<f32>,
}

// Up to three image or pattern points and their weights (one plane: weight 1 in x)
struct P1Samples {
    points: array<vec2<f32>, 3>,
    weights: vec3<f32>,
}

// One copy of a symmetric fill (compose fill_copy.rs): the texel samples the projection at
// pivot + turn * (position - pivot), its image turned over as `flip` says (bit 0 across, bit 1 down)
struct P1Copy {
    index: u32,
    moves: bool,
    turn: mat3x3<f32>,
    pivot: vec3<f32>,
    flip: u32,
}

fn p1_vec2(op: Operation, at: u32) -> vec2<f32> {
    return vec2(param(op, at), param(op, at + 1u));
}

fn p1_vec3(op: Operation, at: u32) -> vec3<f32> {
    return vec3(param(op, at), param(op, at + 1u), param(op, at + 2u));
}

fn p1_finite(v: f32) -> bool {
    return abs(v) <= P1_LARGEST_FLOAT;
}

// glam's try_normalize: `fallback` for a zero or non-finite length
fn p1_normalized(v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let reciprocal = 1.0 / length(v);
    if reciprocal > 0.0 && p1_finite(reciprocal) {
        return v * reciprocal;
    }
    return fallback;
}

fn p1_identity_copy() -> P1Copy {
    return P1Copy(0u, false, mat3x3(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)), vec3(0.0), 0u);
}

// `point` turned by the sine and cosine in `turn`, scaled by `tiling` and moved by `offset` (compose `placement`)
fn p1_place(point: vec2<f32>, tiling: vec2<f32>, turn: vec2<f32>, offset: vec2<f32>) -> vec2<f32> {
    return vec2(point.x * turn.y - point.y * turn.x, point.x * turn.x + point.y * turn.y) * tiling + offset;
}

// A 3D projection's UV transformation (compose `place_uv`); the default changes nothing
fn p1_place_uv(op: Operation, point: vec2<f32>) -> vec2<f32> {
    if (op.ext.y & P1_UV_PLACED) == 0u {
        return point;
    }
    return p1_place(point, p1_vec2(op, P1_UV_TILING), p1_vec2(op, P1_UV_ROTATION), p1_vec2(op, P1_UV_OFFSET));
}

// A frame's image placement, turned round the frame's centre (compose `place_frame_uv`)
fn p1_place_frame_uv(op: Operation, point: vec2<f32>) -> vec2<f32> {
    if (op.ext.y & P1_UV_PLACED) == 0u {
        return point;
    }
    let tiling = p1_vec2(op, P1_UV_TILING);
    return p1_place(point - vec2(0.5), tiling, p1_vec2(op, P1_UV_ROTATION), p1_vec2(op, P1_UV_OFFSET)) + vec2(0.5) * tiling;
}

fn p1_to_local(op: Operation) -> mat3x3<f32> {
    return mat3x3(p1_vec3(op, P1_TO_LOCAL), p1_vec3(op, P1_TO_LOCAL + 3u), p1_vec3(op, P1_TO_LOCAL + 6u));
}

// `coords` inside the projector turned and stretched by its rotation and scale (compose `projector_local`)
fn p1_local(op: Operation, coords: P1Coords) -> P1Coords {
    if (op.ext.y & P1_TURNS) == 0u {
        return coords;
    }
    let to_local = p1_to_local(op);
    let stretch = p1_vec3(op, P1_STRETCH);
    let nrm = p1_normalized(to_local * coords.nrm * stretch, coords.nrm);
    return P1Coords(coords.uv, to_local * coords.pos / stretch, nrm);
}

// 0..1 round a spherical projection's pole axis and down from its top (compose `spherical_uv`)
fn p1_sphere(op: Operation, pos: vec3<f32>) -> vec2<f32> {
    let direction = p1_normalized(pos + p1_vec3(op, P1_OFFSET), vec3(0.0, 1.0, 0.0));
    return vec2(azimuth(direction.z, direction.x), acos(clamp(direction.y, -1.0, 1.0)) / 3.141592654);
}

// 0..1 round a cylindrical projection's axis over its angle, and the height along it (compose `cylinder_coords`)
fn p1_cylinder(op: Operation, pos: vec3<f32>) -> vec2<f32> {
    let point = pos + p1_vec3(op, P1_OFFSET);
    var axes = point;
    switch param_word(op, P1_AXIS) {
        case 0u: { axes = vec3(point.y, point.x, point.z); }
        case 2u: { axes = vec3(point.x, point.z, point.y); }
        default: {}
    }
    let turn = azimuth(axes.z, axes.x);
    let angle = param(op, P1_CYLINDER_ANGLE);

    // A full turn reads the turn untouched, as the CPU does bit for bit
    if angle == P1_FULL_TURN_DEGREES {
        return vec2(turn, axes.y);
    }
    return vec2((turn - 0.5) * P1_FULL_TURN_DEGREES / max(angle, P1_MIN_CYLINDER_ANGLE) + 0.5, axes.y);
}

fn p1_one(point: vec2<f32>) -> P1Samples {
    return P1Samples(array<vec2<f32>, 3>(point, vec2(0.0), vec2(0.0)), vec3(1.0, 0.0, 0.0));
}

// Where the projection reads an image at `coords` (compose `image_samples`)
fn p1_image_points(op: Operation, coords: P1Coords) -> P1Samples {
    let scale = p1_vec2(op, P1_SCALE);
    let offset = p1_vec3(op, P1_OFFSET);
    switch param_word(op, P1_KIND) {
        case P1_KIND_UV_TILE: { return p1_one(coords.uv - floor(coords.uv)); }
        case P1_KIND_UV: { return p1_one(p1_place(coords.uv, scale, p1_vec2(op, P1_ROTATION), offset.xy)); }
        case P1_KIND_TRIPLANAR: {
            let local = p1_local(op, coords);
            let point = local.pos * scale.x + offset;

            // Float images can exceed 1, so even a light plane counts
            let weights = triplanar_weights(local.nrm, param(op, P1_SHARPNESS));
            let points = array<vec2<f32>, 3>(
                p1_place_uv(op, vec2(point.z, -point.y)),
                p1_place_uv(op, vec2(point.x, point.z)),
                p1_place_uv(op, vec2(point.x, -point.y)),
            );
            return P1Samples(points, weights);
        }
        case P1_KIND_PLANAR: {
            let point = p1_local(op, coords).pos * scale.x + offset;
            var plane = vec2(point.x, -point.y);
            switch param_word(op, P1_AXIS) {
                case 0u: { plane = vec2(point.z, -point.y); }
                case 1u: { plane = vec2(point.x, point.z); }
                default: {}
            }
            return p1_one(p1_place_uv(op, plane * 0.5 + vec2(0.5)));
        }
        case P1_KIND_SPHERICAL: { return p1_one(p1_place_uv(op, p1_sphere(op, p1_local(op, coords).pos) * scale.x)); }
        case P1_KIND_CYLINDRICAL: {
            let around = p1_cylinder(op, p1_local(op, coords).pos);
            return p1_one(p1_place_uv(op, vec2(around.x, 0.5 - around.y * 0.5) * scale.x));
        }
        default: { return p1_one(p1_place_frame_uv(op, coords.uv)); }
    }
}

// Where the projection reads a pattern at `coords`, times the pattern scale (compose `procedural_in`)
fn p1_pattern_points(op: Operation, coords: P1Coords) -> P1Samples {
    let scale = p1_vec2(op, P1_SCALE);
    let offset = p1_vec3(op, P1_OFFSET);
    let pattern_scale = op.uv.z;
    switch param_word(op, P1_KIND) {
        case P1_KIND_UV_TILE: { return p1_one((coords.uv - floor(coords.uv)) * pattern_scale); }
        case P1_KIND_UV: { return p1_one(p1_place(coords.uv, scale, p1_vec2(op, P1_ROTATION), offset.xy) * pattern_scale); }
        case P1_KIND_TRIPLANAR: {
            let local = p1_local(op, coords);
            let point = local.pos * scale.x * pattern_scale + offset;

            // The pattern's contrast scales the blend, so planes too light to change it weigh 0
            let weights = triplanar_weights(local.nrm, param(op, P1_SHARPNESS));
            let skip = P1_TRIPLANAR_SKIP / max(op.uv.w, 1.0);
            let points = array<vec2<f32>, 3>(
                p1_place_uv(op, vec2(point.z, point.y)),
                p1_place_uv(op, vec2(point.x, point.z)),
                p1_place_uv(op, vec2(point.x, point.y)),
            );
            return P1Samples(points, select(weights, vec3(0.0), weights < vec3(skip)));
        }
        case P1_KIND_PLANAR: {
            let point = p1_local(op, coords).pos * scale.x * pattern_scale + offset;
            var plane = vec2(point.x, point.y);
            switch param_word(op, P1_AXIS) {
                case 0u: { plane = vec2(point.z, point.y); }
                case 1u: { plane = vec2(point.x, point.z); }
                default: {}
            }
            return p1_one(p1_place_uv(op, plane));
        }
        case P1_KIND_SPHERICAL: {
            let sphere = p1_sphere(op, p1_local(op, coords).pos);
            return p1_one(p1_place_uv(op, sphere * P1_SPHERE_PATTERN_TILES * scale.x * pattern_scale));
        }
        case P1_KIND_CYLINDRICAL: {
            let around = p1_cylinder(op, p1_local(op, coords).pos);
            return p1_one(p1_place_uv(op, vec2(around.x * P1_CYLINDER_PATTERN_TILES, around.y) * scale.x * pattern_scale));
        }
        default: { return p1_one(p1_place_frame_uv(op, coords.uv) * pattern_scale); }
    }
}

// The image points as a symmetry copy sees them, its image turned over (compose `FillCopy::uv`)
fn p1_flipped(samples: P1Samples, copy: P1Copy) -> P1Samples {
    var flipped = samples;
    let flip = (vec2(copy.flip) & vec2(1u, 2u)) != vec2(0u);
    for (var plane = 0u; plane < 3u; plane++) {
        flipped.points[plane] = select(flipped.points[plane], vec2(1.0) - flipped.points[plane], flip);
    }
    return flipped;
}

// One image read at an already projected point: as stored (P1_RAW_KIND), or its alpha
fn p1_read_image(op: Operation, image: u32, point: vec2<f32>, surface: Surface, footprint: f32, linear: bool, kind: u32) -> vec3<f32> {
    var read = op;
    read.ids.y = IMAGE_SOURCE_OPCODE;
    read.ids.z = image;
    read.ids.w = kind;
    read.procedural.w = P1_POINT_PROJECTION;
    read.projection = vec4(footprint, select(0.0, 1.0, linear), 0.0, 0.0);
    return value(read, point, surface);
}

// An image source's value from its samples (compose Texel::source): read as stored, weighed,
// then decoded for the channel
fn p1_image_value(op: Operation, samples: P1Samples, surface: Surface, footprints: vec3<f32>) -> vec3<f32> {
    let image = op.ids.z;
    if image == P1_MISSING {
        return vec3(0.5);
    }
    let srgb_color = op.ids.w == 0u && (srgb[image + 2u] & 256u) != 0u;

    // Planes blend in linear space when any of them reads a linear mip
    var linear = false;
    for (var plane = 0u; plane < 3u; plane++) {
        let footprint = footprints[plane];
        linear = linear || (srgb_color && samples.weights[plane] > 0.0 && footprint > 1.0 && p1_finite(footprint));
    }
    // Runtime indices reach arrays through a variable on every backend
    var points = samples.points;
    var rgb = vec3(0.0);
    for (var plane = 0u; plane < 3u; plane++) {
        let weight = samples.weights[plane];
        if weight > 0.0 {
            rgb += p1_read_image(op, image, points[plane], surface, footprints[plane], linear, P1_RAW_KIND) * weight;
        }
    }

    switch op.ids.w {
        case 0u: {
            if srgb_color && !linear {
                let encoded = clamp(rgb, vec3(0.0), vec3(1.0));
                return select(pow((encoded + 0.055) / 1.055, vec3(2.4)), encoded / 12.92, encoded <= vec3(0.04045));
            }
            return rgb;
        }
        case 1u: { return vec3(dot(rgb, vec3(0.3, 0.59, 0.11))); }
        case 2u: { return vec3(dot(rgb, vec3(0.3, 0.59, 0.11)) * 2.0 - 1.0); }
        case 4u: { return vec3(rgb.r, 1.0 - rgb.g, rgb.b); }
        default: { return rgb; }
    }
}

// A procedural source's value from its pattern points: the pattern code weighs and shapes them
fn p1_procedural_value(op: Operation, samples: P1Samples, uv: vec2<f32>, surface: Surface) -> vec3<f32> {
    var planes = op;
    planes.procedural.w = P1_PLANES_PROJECTION;
    planes.projection = vec4(samples.points[0], samples.points[1]);
    planes.constraints = vec4(bitcast<vec2<u32>>(samples.points[2]), 0u, 0u);
    planes.balance = vec4(op.balance.x, samples.weights);
    return procedural_value(planes, uv, surface);
}

// Image texels per surface texel in each image plane (compose Texel::footprints)
fn p1_footprints(op: Operation, samples: P1Samples, uv: vec2<f32>, copy: P1Copy, size: vec2<f32>) -> vec3<f32> {
    return vec3(1.0);
}

// How much of the texel the fill covers (compose Texel::cover), the surface as the copy sees it
fn p1_cover(op: Operation, coords: P1Coords, uv: vec2<f32>, surface: Surface, copy: P1Copy) -> f32 {
    return 1.0;
}

// The surface as copy `copy` sees it (compose Texel::surface)
fn p1_copied(surface: Surface, copy: P1Copy) -> Surface {
    if !copy.moves || !surface.valid {
        return surface;
    }
    return Surface(copy.pivot + copy.turn * (surface.position - copy.pivot), copy.turn * surface.normal, true);
}

// The fill sampled once through `copy`: its value and its coverage (compose Texel::fill_texel)
fn p1_fill_once(op: Operation, uv: vec2<f32>, surface: Surface, copy: P1Copy) -> vec4<f32> {
    let copied = p1_copied(surface, copy);
    let coords = P1Coords(uv, copied.position, copied.normal);
    let kind = op.ids.y;
    var rgb: vec3<f32>;
    if kind == IMAGE_SOURCE_OPCODE {
        let samples = p1_flipped(p1_image_points(op, coords), copy);
        var footprints = vec3(1.0);
        if (op.ext.y & P1_HQ) != 0u && op.ids.z != P1_MISSING {
            footprints = p1_footprints(op, samples, uv, copy, vec2<f32>(vec2(srgb[op.ids.z], srgb[op.ids.z + 1u])));
        }
        rgb = p1_image_value(op, samples, surface, footprints);
    } else if kind != 0u && kind != P1_ANCHOR_SOURCE {
        rgb = p1_procedural_value(op, p1_pattern_points(op, coords), uv, surface);
    } else {
        rgb = value(op, uv, surface);
    }
    return vec4(rgb, p1_cover(op, coords, uv, copied, copy));
}

// A fill whose operation has projection or sampling flags (Operation.ext.y != 0): the value and
// its coverage in alpha. fill_value in composite.wgsl calls it before its own paths.
fn projection_fill_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> {
    return p1_fill_once(layer, uv, surface, p1_identity_copy());
}
