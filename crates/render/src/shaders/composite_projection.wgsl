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

// A depth directory (gpu_depth_grid.rs): the copy count, then per copy its grid's plane
// descriptor (width, height, x, y, layer in the image atlas), origin, cell size, cells per side,
// nearness unit, bias, slope bias limit and fade width
const P1_DEPTH_ENTRY_WORDS: u32 = 10u;

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
// Where a frame's depth fade starts, as a share of its depth (compose pattern.rs DEPTH_FADE_START)
const P1_DEPTH_FADE_START: f32 = 0.75;
// Nearer a cylinder's axis than this a texel counts as facing out (compose projector_cover.rs)
const P1_MIN_AXIS_DISTANCE: f32 = 1e-6;
const P1_FLOAT_EPSILON: f32 = 1.1920929e-7;
// A warp quad's twist this small against its area counts as parallel sides (compose warp.rs RELATIVE_PARALLEL)
const P1_RELATIVE_PARALLEL: f32 = 1e-6;
// A neighbour farther than this share of the image away lies across a UV seam (compose eval.rs SEAM_JUMP_SHARE)
const P1_SEAM_JUMP_SHARE: f32 = 0.25;
// Symmetry copies stop once they cover this much (compose eval.rs COVERED)
const P1_COVERED: f32 = 0.9999;
// The word of a decal's alpha image in its frame words (gpu_image_source.rs decal_words)
const P1_DECAL_ALPHA_WORD: u32 = 11u;

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

// Where a framed projection reads the texel and how much its frame lets through (compose Texel::projected)
struct P1Projected {
    coords: P1Coords,
    cover: f32,
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

// compose organic.rs `smoothstep`
fn p1_smoothstep(start: f32, end: f32, value: f32) -> f32 {
    let t = clamp((value - start) / (end - start), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// The texel's world position from its place in mesh radii (P1's band uniform: the mesh's centre and radius)
fn p1_world(position: vec3<f32>) -> vec3<f32> {
    let frame = bitcast<vec4<f32>>(params.packages[0]);
    return position * frame.w + frame.xyz;
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

// World point `world` in the frame turned and stretched by the projector: across and up in frame
// sizes, out along its normal in world units; w is 0 where it is not finite (compose `frame_local`)
fn p1_frame_local(op: Operation, world: vec3<f32>) -> vec4<f32> {
    let from_origin = world - p1_vec3(op, P1_FRAME_ORIGIN);
    var local = vec3(dot(from_origin, p1_vec3(op, P1_FRAME_RIGHT)), dot(from_origin, p1_vec3(op, P1_FRAME_UP)), dot(from_origin, p1_vec3(op, P1_FRAME_NORMAL)));
    if (op.ext.y & P1_TURNS) != 0u {
        local = p1_to_local(op) * local / p1_vec3(op, P1_STRETCH);
    }
    let size = p1_vec2(op, P1_FRAME_SIZE);
    let placed = vec3(local.x / size.x, local.y / size.y, local.z);
    return vec4(placed, select(0.0, 1.0, all(abs(placed) <= vec3(P1_LARGEST_FLOAT))));
}

// How squarely a surface with normal `nrm` faces the turned frame (compose `frame_sample`)
fn p1_frame_facing(op: Operation, nrm: vec3<f32>) -> f32 {
    let normal = p1_vec3(op, P1_FRAME_NORMAL);
    if (op.ext.y & P1_TURNS) == 0u {
        return dot(nrm, normal);
    }
    let turned = vec3(dot(nrm, p1_vec3(op, P1_FRAME_RIGHT)), dot(nrm, p1_vec3(op, P1_FRAME_UP)), dot(nrm, normal));
    return p1_normalized(p1_to_local(op) * turned * p1_vec3(op, P1_STRETCH), turned).z;
}

// How much a face turned `facing` (the cosine of its turn from the projection) keeps (compose `backface_cover`)
fn p1_backface(op: Operation, facing: f32) -> f32 {
    let threshold = param(op, P1_BACKFACE_THRESHOLD);
    let width = param(op, P1_BACKFACE_WIDTH);
    if width <= P1_FLOAT_EPSILON {
        return select(0.0, 1.0, facing > threshold);
    }
    return p1_smoothstep(threshold, threshold + width, facing);
}

// 0 past `depth` (already at least the smallest frame extent) in front of or behind a frame,
// fading near it (compose `depth_fade`)
fn p1_depth_fade(z: f32, depth: f32) -> f32 {
    let distance = abs(z);
    if distance > depth {
        return 0.0;
    }
    return 1.0 - p1_smoothstep(P1_DEPTH_FADE_START * depth, depth, distance);
}

// A placed frame at world point `world` with normal `nrm`: its uv, how much it covers, and 1 in w
// (0 in w where the point is not finite) (compose `placed_cover`)
fn p1_placed_cover(op: Operation, world: vec3<f32>, nrm: vec3<f32>) -> vec4<f32> {
    let local = p1_frame_local(op, world);
    if local.w == 0.0 {
        return vec4(0.0);
    }
    var cover = 1.0;
    if (op.ext.y & P1_BACKFACE) != 0u {
        cover = p1_backface(op, p1_frame_facing(op, nrm));
    }
    if (op.ext.y & P1_CROP) != 0u {
        if abs(local.x) > 0.5 || abs(local.y) > 0.5 {
            cover = 0.0;
        } else {
            cover *= p1_depth_fade(local.z, param(op, P1_FRAME_DEPTH));
        }
    }
    let uv = vec2(local.x + 0.5, 0.5 - local.y);
    let flip = (vec2(param_word(op, P1_FRAME_FLIP)) & vec2(1u, 2u)) != vec2(0u);
    return vec4(select(uv, vec2(1.0) - uv, flip), cover, 1.0);
}

// A placed frame and its mirrored copy: whichever covers the point more (compose `placed_cover_symmetric`)
fn p1_placed(op: Operation, world: vec3<f32>, nrm: vec3<f32>) -> vec4<f32> {
    let direct = p1_placed_cover(op, world, nrm);
    let mirror = param_word(op, P1_FRAME_MIRROR_AXIS);
    if mirror == 0u {
        return direct;
    }
    let axis = mirror - 1u;
    let center = bitcast<vec4<f32>>(params.packages[0]).xyz;
    var position = world;
    var normal = nrm;
    position[axis] = 2.0 * (center[axis] + param(op, P1_FRAME_MIRROR_OFFSET)) - world[axis];
    normal[axis] = -nrm[axis];
    var mirrored = p1_placed_cover(op, position, normal);
    switch param_word(op, P1_FRAME_MIRROR_FLIP) {
        case 1u: { mirrored.x = 1.0 - mirrored.x; }
        case 2u: { mirrored.y = 1.0 - mirrored.y; }
        default: {}
    }
    if direct.w != 0.0 && mirrored.w != 0.0 {
        return select(direct, mirrored, mirrored.z > direct.z);
    }
    return select(mirrored, direct, direct.w != 0.0);
}

fn p1_perp(a: vec2<f32>, b: vec2<f32>) -> f32 {
    return a.x * b.y - a.y * b.x;
}

// How far across the warp's quad a point lies at height `v`, solved on the axis the quad's edge
// spans most: u, and 1 in y where it is finite (compose warp.rs `across`)
fn p1_warp_across(top: vec2<f32>, left: vec2<f32>, twist: vec2<f32>, relative: vec2<f32>, v: f32) -> vec2<f32> {
    let span = top + twist * v;
    var u = (relative.y - left.y * v) / span.y;
    if abs(span.x) >= abs(span.y) {
        u = (relative.x - left.x * v) / span.x;
    }
    return vec2(u, select(0.0, 1.0, p1_finite(u)));
}

// How far `uv` lies outside the image's 0..1 square (compose warp.rs `outside`)
fn p1_outside(uv: vec2<f32>) -> f32 {
    let past = uv - clamp(uv, vec2(0.0), vec2(1.0));
    return dot(past, past);
}

// Texture coordinates of a point in frame units: the inverse of the bilinear quad the warp's
// corners make, 1 in z where there is one (compose warp.rs `warp_uv`)
fn p1_warp_uv(op: Operation, point: vec2<f32>) -> vec3<f32> {
    let top_left = p1_vec2(op, P1_WARP_CORNERS);
    let top_right = p1_vec2(op, P1_WARP_CORNERS + 2u);
    let bottom_right = p1_vec2(op, P1_WARP_CORNERS + 4u);
    let bottom_left = p1_vec2(op, P1_WARP_CORNERS + 6u);
    let top = top_right - top_left;
    let left = bottom_left - top_left;
    let twist = top_left - top_right + bottom_right - bottom_left;
    let relative = point - top_left;
    let base_area = p1_perp(top, left);
    let quadratic = p1_perp(twist, left);
    let linear = base_area + p1_perp(relative, twist);
    let constant = p1_perp(relative, top);

    var uv: vec3<f32>;
    if abs(quadratic) <= P1_RELATIVE_PARALLEL * abs(base_area) {
        // Parallel sides make the quad's equation linear in v
        let v = -constant / linear;
        let u = p1_warp_across(top, left, twist, relative, v);
        uv = vec3(u.x, v, u.y);
    } else {
        let discriminant = linear * linear - 4.0 * constant * quadratic;
        if discriminant < 0.0 {
            return vec3(0.0);
        }

        // Avoid subtracting near-equal values when the quad is nearly a parallelogram
        let root = sqrt(discriminant);
        let root_term = -0.5 * (linear + select(root, -root, linear < 0.0));
        let first_v = root_term / quadratic;
        let second_v = constant / root_term;
        let first_u = p1_warp_across(top, left, twist, relative, first_v);
        let second_u = p1_warp_across(top, left, twist, relative, second_v);
        let first = vec3(first_u.x, first_v, first_u.y);
        let second = vec3(second_u.x, second_v, second_u.y);
        if first.z == 0.0 {
            uv = second;
        } else if second.z == 0.0 {
            uv = first;
        } else {
            uv = select(first, second, p1_outside(second.xy) < p1_outside(first.xy));
        }
    }
    if uv.z == 0.0 || !p1_finite(uv.x) || !p1_finite(uv.y) {
        return vec3(0.0);
    }
    return uv;
}

// A warp at world point `world`: its uv in the quad, and how much of the quad and its reach cover
// it when cropped; 0 in w where nothing lands (compose `warp_cover`)
fn p1_warp_cover(op: Operation, world: vec3<f32>) -> vec4<f32> {
    let local = p1_frame_local(op, world);
    if local.w == 0.0 {
        return vec4(0.0);
    }
    let uv = p1_warp_uv(op, local.xy);
    if uv.z == 0.0 {
        return vec4(0.0);
    }
    if (op.ext.y & P1_CROP) == 0u {
        return vec4(uv.xy, 1.0, 1.0);
    }
    let inside = all(uv.xy >= vec2(0.0) & uv.xy <= vec2(1.0));
    return vec4(uv.xy, select(0.0, p1_depth_fade(local.z, param(op, P1_FRAME_DEPTH)), inside), 1.0);
}

// A framed projection at world point `world` with normal `nrm`: uv, cover, and 0 in w where
// nothing lands (compose `framed_cover`)
fn p1_framed(op: Operation, world: vec3<f32>, nrm: vec3<f32>) -> vec4<f32> {
    if (op.ext.y & P1_FRAME_VALID) == 0u {
        return vec4(0.0);
    }
    if param_word(op, P1_KIND) == P1_KIND_WARP {
        return p1_warp_cover(op, world);
    }
    return p1_placed(op, world, nrm);
}

// Where the projection reads the texel at `coords` on `surface` (compose Texel::projected)
fn p1_projected(op: Operation, coords: P1Coords, surface: Surface) -> P1Projected {
    if (op.ext.y & P1_FRAMED) == 0u {
        return P1Projected(coords, 1.0);
    }
    if !surface.valid {
        return P1Projected(coords, 0.0);
    }
    let framed = p1_framed(op, p1_world(surface.position), coords.nrm);
    if framed.w == 0.0 {
        return P1Projected(coords, 0.0);
    }
    return P1Projected(P1Coords(framed.xy, coords.pos, coords.nrm), framed.z);
}

// How much the shape crop and backface culling of a tri-planar, planar, spherical or cylindrical
// projection let through (compose `projector_cover`)
fn p1_projector_cover(op: Operation, coords: P1Coords) -> f32 {
    let flags = op.ext.y;
    if (flags & (P1_CROP | P1_BACKFACE)) == 0u {
        return 1.0;
    }
    let local = p1_local(op, coords);
    let offset = p1_vec3(op, P1_OFFSET);
    var inside = true;
    var facing = 1.0;
    var faces = false;
    switch param_word(op, P1_KIND) {
        case P1_KIND_TRIPLANAR: {
            let reach = abs(local.pos);
            inside = max(reach.x, max(reach.y, reach.z)) <= 1.0;
        }
        case P1_KIND_PLANAR: {
            // The frame across the axis, and the side the image reads the right way round from
            let scaled = local.pos * param(op, P1_SCALE) + offset;
            var frame = max(abs(scaled.x), abs(scaled.y));
            facing = local.nrm.z;
            switch param_word(op, P1_AXIS) {
                case 0u: {
                    frame = max(abs(scaled.z), abs(scaled.y));
                    facing = -local.nrm.x;
                }
                case 1u: {
                    frame = max(abs(scaled.x), abs(scaled.z));
                    facing = local.nrm.y;
                }
                default: {}
            }
            inside = frame <= 1.0;
            faces = true;
        }
        case P1_KIND_SPHERICAL: { inside = length(local.pos + offset) <= 1.0; }
        case P1_KIND_CYLINDRICAL: {
            let position = local.pos + offset;
            var axes = position;
            var normal = local.nrm.xz;
            switch param_word(op, P1_AXIS) {
                case 0u: {
                    axes = vec3(position.y, position.x, position.z);
                    normal = local.nrm.yz;
                }
                case 2u: {
                    axes = vec3(position.x, position.z, position.y);
                    normal = local.nrm.xy;
                }
                default: {}
            }
            let radius = length(axes.xz);
            facing = select(1.0, (normal.x * axes.x + normal.y * axes.z) / radius, radius > P1_MIN_AXIS_DISTANCE);
            inside = radius <= 1.0 && abs(axes.y) <= 1.0;
            faces = true;
        }
        default: {}
    }
    if (flags & P1_CROP) != 0u && !inside {
        return 0.0;
    }
    if faces && (flags & P1_BACKFACE) != 0u {
        return p1_backface(op, facing);
    }
    return 1.0;
}

// A cropped placed frame takes the shape of its image too (compose Texel::shape_cover): the alpha
// of the frame's image (from its decal words) at the first image point, read with the fill's
// sampling and footprint so shrunk colour and coverage agree
fn p1_frame_alpha(op: Operation, samples: P1Samples, uv: vec2<f32>, surface: Surface, copy: P1Copy) -> f32 {
    let alpha_image = srgb[u32(op.uv.x) + P1_DECAL_ALPHA_WORD];
    if alpha_image == P1_MISSING {
        return 1.0;
    }
    var footprint = 1.0;
    if (op.ext.y & P1_HQ) != 0u {
        footprint = p1_footprints(op, samples, uv, copy, vec2<f32>(vec2(srgb[alpha_image], srgb[alpha_image + 1u]))).x;
    }
    return clamp(p1_read_image(op, alpha_image, samples.points[0], surface, footprint, false, P1_ALPHA_KIND).x, 0.0, 1.0);
}

// Image texels per surface texel in each image plane of an image `size` texels large (compose
// Texel::footprints): from the UV step on UV projections, else from where the next texels across
// and down land
fn p1_footprints(op: Operation, samples: P1Samples, uv: vec2<f32>, copy: P1Copy, size: vec2<f32>) -> vec3<f32> {
    let source = vec2<f32>(params.source.xy);
    let kind = param_word(op, P1_KIND);
    if kind == P1_KIND_UV || kind == P1_KIND_UV_TILE {
        // The current tile's affine mapping, even where a neighbour wraps or is uncovered
        let step = 1.0 / source;
        var span = max(step.x * size.x, step.y * size.y);
        if kind == P1_KIND_UV {
            let scale = p1_vec2(op, P1_SCALE);
            let turn = p1_vec2(op, P1_ROTATION);
            let across = length(p1_place(vec2(step.x, 0.0), scale, turn, vec2(0.0)) * size);
            let down = length(p1_place(vec2(0.0, step.y), scale, turn, vec2(0.0)) * size);
            span = max(across, down);
        }
        return vec3(select(1.0, max(span, 1.0), p1_finite(span)), 1.0, 1.0);
    }

    var points = samples.points;
    var footprints = vec3(1.0);
    let pixel = vec2<u32>(uv * source);
    let seam_limit = max(size.x, size.y) * P1_SEAM_JUMP_SHARE;
    for (var across = 0u; across < 2u; across++) {
        let next_pixel = pixel + vec2(1u - across, across);
        let neighbour = p1_copied(surface_at(next_pixel), copy);
        if !neighbour.valid {
            continue;
        }
        var coords = P1Coords((vec2<f32>(next_pixel) + 0.5) / source, neighbour.position, neighbour.normal);
        if (op.ext.y & P1_FRAMED) != 0u {
            let framed = p1_framed(op, p1_world(neighbour.position), neighbour.normal);
            if framed.w == 0.0 {
                continue;
            }
            coords.uv = framed.xy;
        }
        var next = p1_flipped(p1_image_points(op, coords), copy).points;
        for (var plane = 0u; plane < 3u; plane++) {
            // Neighbours can land on disconnected surfaces at a UV seam
            let span = length((next[plane] - points[plane]) * size);
            if p1_finite(span) && span < seam_limit {
                footprints[plane] = max(footprints[plane], span);
            }
        }
    }
    return footprints;
}

// 1 where the image point `uv` lies inside its 0..1 tile on every axis that does not repeat
// (compose Texel::tile_cover)
fn p1_tile_cover(op: Operation, uv: vec2<f32>) -> f32 {
    if (op.ext.y & P1_TILE_COVER) == 0u {
        return 1.0;
    }
    let inside = uv >= vec2(0.0) & uv < vec2(1.0);
    let repeats = (vec2(op.ext.y) & vec2(P1_CLAMP_U, P1_CLAMP_V)) == vec2(0u);
    return select(0.0, 1.0, all(inside | repeats));
}

// How much depth culling leaves of the projection at `position` (in mesh radii, as copy
// `copy_index` sees it) through that copy's grid: 0 behind the surface nearest the projector,
// 1 on it or where nothing is in front (compose ProjectorDepth::cover)
fn p1_depth_cover(op: Operation, copy_index: u32, position: vec3<f32>) -> f32 {
    let directory = param_word(op, P1_DEPTH_DIRECTORY);
    if directory == 0u || copy_index >= srgb[directory] {
        return 1.0;
    }
    let entry = directory + 1u + copy_index * P1_DEPTH_ENTRY_WORDS;
    let plane = srgb[entry];
    if plane == P1_MISSING {
        return 1.0;
    }

    // The point in the projector's view: across, up and how near the projector (compose ProjectorView::local)
    var view: vec3<f32>;
    if param_word(op, P1_KIND) == P1_KIND_PLANAR {
        var local = position;
        if (op.ext.y & P1_TURNS) != 0u {
            local = p1_to_local(op) * position / p1_vec3(op, P1_STRETCH);
        }
        switch param_word(op, P1_AXIS) {
            case 0u: { view = vec3(local.z, local.y, -local.x); }
            case 1u: { view = vec3(local.x, local.z, local.y); }
            default: { view = local; }
        }
    } else {
        let local = p1_frame_local(op, p1_world(position));
        if local.w == 0.0 {
            return 1.0;
        }
        view = local.xyz;
    }
    if !all(abs(view) <= vec3(P1_LARGEST_FLOAT)) {
        return 1.0;
    }

    let origin = bitcast<vec2<f32>>(vec2(srgb[entry + 1u], srgb[entry + 2u]));
    let cell = bitcast<vec2<f32>>(vec2(srgb[entry + 3u], srgb[entry + 4u]));
    let side = srgb[entry + 5u];
    let unit = bitcast<f32>(srgb[entry + 6u]);
    let at = (view.xy - origin) / cell;
    if !(at.x >= 0.0 && at.y >= 0.0 && at.x <= f32(side) && at.y <= f32(side)) {
        return 1.0;
    }
    let cell_at = min(vec2<u32>(at), vec2(side - 1u));

    // The cell's nearest surface (nearness, then its slope across and up) in three texels of a row
    let texel = vec2<i32>(i32(srgb[plane + 2u] + cell_at.x * 3u), i32(srgb[plane + 3u] + cell_at.y));
    let layer = i32(srgb[plane + 4u]);
    let near = bitcast<f32>(textureLoad(tile_images, texel, layer, 0).x);
    if !p1_finite(near) {
        return 1.0;
    }
    let slope = bitcast<vec2<f32>>(vec2(textureLoad(tile_images, texel + vec2(1, 0), layer, 0).x, textureLoad(tile_images, texel + vec2(2, 0), layer, 0).x));

    // Extend the cell's plane to avoid culling a surface against itself, with a slope bias for
    // curvature across adjacent triangles
    let centre = origin + (vec2<f32>(cell_at) + 0.5) * cell;
    let behind = near + dot(slope, view.xy - centre) - view.z;
    let bias = bitcast<f32>(srgb[entry + 7u]) * unit + min(dot(abs(slope) * cell, vec2(1.0)), bitcast<f32>(srgb[entry + 8u]) * unit);
    let width = (1.0 - clamp(param(op, P1_DEPTH_HARDNESS), 0.0, 1.0)) * bitcast<f32>(srgb[entry + 9u]) * unit;
    if width <= P1_FLOAT_EPSILON {
        return select(0.0, 1.0, behind <= bias);
    }
    return 1.0 - p1_smoothstep(bias, bias + width, behind);
}

// How much of the texel the fill covers (compose Texel::cover): its shape cover, then depth
// culling where any is left of it
fn p1_cover(op: Operation, coords: P1Coords, projected: P1Projected, uv: vec2<f32>, surface: Surface, copy: P1Copy) -> f32 {
    let shape = p1_shape_cover(op, coords, projected, uv, surface, copy);
    if (op.ext.y & P1_DEPTH) == 0u || shape <= 0.0 || !surface.valid {
        return shape;
    }
    return shape * p1_depth_cover(op, copy.index, surface.position);
}

// How much of the texel the fill's shape covers (compose Texel::shape_cover), the surface as the
// copy sees it
fn p1_shape_cover(op: Operation, coords: P1Coords, projected: P1Projected, uv: vec2<f32>, surface: Surface, copy: P1Copy) -> f32 {
    let flags = op.ext.y;
    if (flags & P1_FRAMED) != 0u {
        let samples = p1_flipped(p1_image_points(op, projected.coords), copy);
        var cover = projected.cover;
        if (flags & P1_IMAGE_ALPHA) != 0u && cover > 0.0 {
            cover *= p1_frame_alpha(op, samples, uv, surface, copy);
        }
        return cover * p1_tile_cover(op, samples.points[0]);
    }
    if (flags & (P1_TILE_COVER | P1_CROP | P1_BACKFACE)) == 0u {
        return 1.0;
    }
    return p1_tile_cover(op, p1_flipped(p1_image_points(op, coords), copy).points[0]) * p1_projector_cover(op, coords);
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
    let projected = p1_projected(op, coords, copied);
    let kind = op.ids.y;
    var rgb: vec3<f32>;
    if kind == IMAGE_SOURCE_OPCODE {
        let samples = p1_flipped(p1_image_points(op, projected.coords), copy);
        var footprints = vec3(1.0);
        if (op.ext.y & P1_HQ) != 0u && op.ids.z != P1_MISSING {
            footprints = p1_footprints(op, samples, uv, copy, vec2<f32>(vec2(srgb[op.ids.z], srgb[op.ids.z + 1u])));
        }
        rgb = p1_image_value(op, samples, surface, footprints);
    } else if kind != 0u && kind != P1_ANCHOR_SOURCE {
        rgb = p1_procedural_value(op, p1_pattern_points(op, projected.coords), uv, surface);
    } else {
        rgb = value(op, uv, surface);
    }
    return vec4(rgb, p1_cover(op, coords, projected, uv, copied, copy));
}

// The radial copy nearest a texel `turned` degrees round the axis, of `count` copies `step`
// degrees apart over `span` degrees (compose fill_copies.rs `nearest_copy`)
fn p1_nearest_copy(turned: f32, step: f32, count: u32, span: f32) -> u32 {
    // A span of nothing (or no step at all) stacks every copy on the original
    if !(step > 0.0) {
        return 0u;
    }
    let by_step = u32(floor(turned / step + 0.5));
    if span >= P1_FULL_TURN_DEGREES {
        return by_step % count;
    }
    let last = min(by_step, count - 1u);
    let past_last = abs(turned - f32(last) * step);
    let before_first = P1_FULL_TURN_DEGREES - turned;
    return select(last, 0u, before_first <= past_last);
}

// Copy `index` of the fill's symmetry about `pivot`: the original at 0, then the mirror or the
// radial copies in turn (compose `FillCopies::copy`)
fn p1_copy(op: Operation, index: u32, pivot: vec3<f32>) -> P1Copy {
    var copy = p1_identity_copy();
    copy.index = index;
    copy.moves = true;
    copy.pivot = pivot;
    if index == 0u {
        return copy;
    }
    copy.flip = param_word(op, P1_SYMMETRY_FLIP);
    let axis = param_word(op, P1_SYMMETRY_AXIS);
    if param_word(op, P1_SYMMETRY_KIND) == 0u {
        var scale = vec3(1.0);
        scale[axis] = -1.0;
        copy.turn = mat3x3(vec3(scale.x, 0.0, 0.0), vec3(0.0, scale.y, 0.0), vec3(0.0, 0.0, scale.z));
        return copy;
    }

    // Copy `index` is the original turned by `index` steps, so a texel samples the original turned back
    let angle = -radians(f32(index) * param(op, P1_SYMMETRY_STEP));
    let s = sin(angle);
    let c = cos(angle);
    switch axis {
        case 0u: { copy.turn = mat3x3(vec3(1.0, 0.0, 0.0), vec3(0.0, c, s), vec3(0.0, -s, c)); }
        case 1u: { copy.turn = mat3x3(vec3(c, 0.0, -s), vec3(0.0, 1.0, 0.0), vec3(s, 0.0, c)); }
        default: { copy.turn = mat3x3(vec3(c, s, 0.0), vec3(-s, c, 0.0), vec3(0.0, 0.0, 1.0)); }
    }
    return copy;
}

// Every copy of a symmetric fill laid over the others, the texel's own copy on top (compose
// Texel::symmetric_copies; positions in mesh radii, where the pivot is the symmetry's position)
fn p1_symmetric(op: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> {
    let pivot = p1_vec3(op, P1_SYMMETRY_POSITION);
    let axis = param_word(op, P1_SYMMETRY_AXIS);
    let count = param_word(op, P1_SYMMETRY_COUNT);
    let from_pivot = surface.position - pivot;

    // The copy whose half (mirror) or wedge (radial) holds the texel comes first
    var first = select(0u, 1u, from_pivot[axis] < 0.0);
    if param_word(op, P1_SYMMETRY_KIND) != 0u {
        let turned = degrees(atan2(from_pivot[(axis + 2u) % 3u], from_pivot[(axis + 1u) % 3u]));
        first = p1_nearest_copy(select(turned, turned + P1_FULL_TURN_DEGREES, turned < 0.0), param(op, P1_SYMMETRY_STEP), count, param(op, P1_SYMMETRY_SPAN));
    }

    var color = vec3(0.0);
    var cover = 0.0;
    for (var next = 0u; next < count; next++) {
        let one = p1_fill_once(op, uv, surface, p1_copy(op, (first + next) % count, pivot));
        let weight = clamp(one.a, 0.0, 1.0) * (1.0 - cover);
        color += one.rgb * weight;
        cover += weight;
        if cover >= P1_COVERED {
            break;
        }
    }
    if cover <= 0.0 {
        return vec4(0.0);
    }
    return vec4(color / cover, cover);
}

// A fill whose operation has projection or sampling flags (Operation.ext.y != 0): the value and
// its coverage in alpha. fill_value in composite.wgsl calls it before its own paths.
fn projection_fill_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> {
    // An uncovered texel has no place to copy, so it samples the fill once
    if (layer.ext.y & P1_SYMMETRY) != 0u && surface.valid {
        return p1_symmetric(layer, uv, surface);
    }
    return p1_fill_once(layer, uv, surface, p1_identity_copy());
}
