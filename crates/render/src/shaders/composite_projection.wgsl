// P1 (gpu_composite_projection.rs, its stub otherwise): the CPU's fill texel (compose eval.rs fill_texel) from the side table, reading images and patterns through value

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

// Words per copy in a depth directory (gpu_depth_grid.rs): plane descriptor, origin, cell, side, unit, bias, slope bias limit, fade
const P1_DEPTH_ENTRY_WORDS: u32 = 10u;

// Projection kinds in the side table (KIND_* in gpu_composite_projection.rs)
const P1_KIND_UV: u32 = 0u;
const P1_KIND_UV_TILE: u32 = 1u;
const P1_KIND_TRIPLANAR: u32 = 2u;
const P1_KIND_PLANAR: u32 = 3u;
const P1_KIND_SPHERICAL: u32 = 4u;
const P1_KIND_CYLINDRICAL: u32 = 5u;
const P1_KIND_WARP: u32 = 7u;

const P1_MISSING: u32 = 0xffffffffu;
const P1_ANCHOR_SOURCE: u32 = 15u;
// Operation.ids.w of an image read that returns the samples as stored (no decoding)
const P1_RAW_KIND: u32 = 3u;
// An image descriptor's flag: its texels are sRGB encoded (gpu_tile_pattern.rs append_rgba)
const P1_SRGB_IMAGE: u32 = 256u;
const P1_LARGEST_FLOAT: f32 = 3.402823466e38;
// Floors at or beyond this reduce by the image's period before the integer conversion
const P1_LARGEST_TEXEL_INDEX: f32 = 2147483520.0;
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
// The word of a frame record's alpha image (gpu_image_source.rs decal_words)
const P1_DECAL_ALPHA_WORD: u32 = 11u;

// Where a projection reads the texel (compose pattern.rs Coords): UV, position in mesh radii from the mesh's centre, normal
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

// A symmetry copy (compose fill_copy.rs): pivot + turn * (position - pivot), its image flipped across (bit 0) and down (bit 1)
struct P1Copy {
    index: u32,
    moves: bool,
    turn: mat3x3<f32>,
    pivot: vec3<f32>,
    flip: u32,
}

// The surfaces of the next texels across and down, read once for every copy's footprint
struct P1Neighbours {
    across: Surface,
    down: Surface,
}

// One stored level of an image: its first plane's origin word, size and encoding (gpu_tile_pattern.rs append_rgba)
struct P1ImageLevel {
    planes: u32,
    size: vec2<i32>,
    encoding: u32,
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

// compose organic.rs smoothstep
fn p1_smoothstep(start: f32, end: f32, value: f32) -> f32 {
    let t = clamp((value - start) / (end - start), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// The world position of a place in mesh radii (P1's band uniform holds the mesh's centre and radius)
fn p1_world(position: vec3<f32>) -> vec3<f32> {
    let frame = bitcast<vec4<f32>>(params.packages[0]);
    return position * frame.w + frame.xyz;
}

fn p1_identity_copy() -> P1Copy {
    return P1Copy(0u, false, mat3x3(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)), vec3(0.0), 0u);
}

// sRGB encoded values as linear ones (texel srgb_to_linear)
fn p1_linearized(rgb: vec3<f32>) -> vec3<f32> {
    let encoded = clamp(rgb, vec3(0.0), vec3(1.0));
    return select(pow((encoded + 0.055) / 1.055, vec3(2.4)), encoded / 12.92, encoded <= vec3(0.04045));
}

// One plane word of an image texel in the atlas
fn p1_plane_at(plane: u32, pixel: vec2<i32>) -> u32 {
    let origin = vec2<i32>(i32(srgb[plane]), i32(srgb[plane + 1u]));
    return textureLoad(tile_images, origin + pixel, i32(srgb[plane + 2u]), 0).x;
}

fn p1_base_level(image: u32) -> P1ImageLevel {
    return P1ImageLevel(image + 3u, vec2<i32>(i32(srgb[image]), i32(srgb[image + 1u])), srgb[image + 2u] & 255u);
}

// Where an image's level count sits: after its own planes, followed by one descriptor offset per level
fn p1_level_table(image: u32) -> u32 {
    return image + 3u + 3u * p1_base_level(image).encoding;
}

// The halved levels an image holds besides itself
fn p1_level_count(image: u32) -> u32 {
    if (srgb[image + 2u] & P1_IMAGE_LEVELS) == 0u {
        return 0u;
    }

    return srgb[p1_level_table(image)];
}

// Halved level `index` (1 is half the size): width, height, encoding, then one origin per plane
fn p1_halved_level(image: u32, index: u32) -> P1ImageLevel {
    let word = srgb[p1_level_table(image) + index];
    return P1ImageLevel(word + 3u, vec2<i32>(i32(srgb[word]), i32(srgb[word + 1u])), srgb[word + 2u]);
}

// Texel `pixel` of `level`: wrapped on the axes that repeat, the edge texel past the edge on the others
fn p1_level_texel(level: P1ImageLevel, pixel: vec2<i32>, clamps: vec2<bool>) -> vec4<f32> {
    let wrapped = vec2(wrap_coordinate(pixel.x, level.size.x), wrap_coordinate(pixel.y, level.size.y));
    let at = select(wrapped, clamp(pixel, vec2(0), level.size - 1), clamps);
    let first = p1_plane_at(level.planes, at);
    switch level.encoding {
        case 1u: { return unpack4x8unorm(first); }
        case 2u: { return vec4(unpack2x16unorm(first), unpack2x16unorm(p1_plane_at(level.planes + 3u, at))); }
        default: {
            return bitcast<vec4<f32>>(vec4(first, p1_plane_at(level.planes + 3u, at), p1_plane_at(level.planes + 6u, at), p1_plane_at(level.planes + 9u, at)));
        }
    }
}

// floor(t) as texel indices that never overflow: huge repeating ones reduced by the period, stopped ones kept just past the edge
fn p1_texel_index(t: vec2<f32>, size: vec2<i32>, clamps: vec2<bool>) -> vec2<i32> {
    let period = vec2<f32>(size);
    var base = floor(t);
    base = select(base, base % period, abs(base) >= vec2(P1_LARGEST_TEXEL_INDEX) & !clamps);
    base = select(base, clamp(base, vec2(-1.0), period), clamps);
    return vec2<i32>(base);
}

// The texel under `uv` (compose ImageRes::nearest)
fn p1_nearest(level: P1ImageLevel, uv: vec2<f32>, clamps: vec2<bool>) -> vec4<f32> {
    return p1_level_texel(level, p1_texel_index(uv * vec2<f32>(level.size), level.size, clamps), clamps);
}

// Bilinear sample of `level` (compose store.rs bilinear)
fn p1_bilinear(level: P1ImageLevel, uv: vec2<f32>, clamps: vec2<bool>) -> vec4<f32> {
    let grid = uv * vec2<f32>(level.size) - 0.5;
    let at = p1_texel_index(grid, level.size, clamps);
    let f = grid - floor(grid);
    let a = p1_level_texel(level, at, clamps);
    let b = p1_level_texel(level, at + vec2(1, 0), clamps);
    let c = p1_level_texel(level, at + vec2(0, 1), clamps);
    let d = p1_level_texel(level, at + vec2(1, 1), clamps);
    let top = a + (b - a) * f.x;
    let bottom = c + (d - c) * f.x;
    return top + (bottom - top) * f.y;
}

// Level `index` of an image read bilinearly; `full` (the image itself) at 0 and past the last level
fn p1_level_sample(image: u32, index: u32, uv: vec2<f32>, clamps: vec2<bool>, full: vec4<f32>) -> vec4<f32> {
    if index == 0u || index > p1_level_count(image) {
        return full;
    }

    return p1_bilinear(p1_halved_level(image, index), uv, clamps);
}

// An image at a projected point read as its sampling says (compose ImageRes::sample_filtered); the read carries footprint and linear flag in projection.xy
fn p1_image_at(image: u32, uv: vec2<f32>, layer: Operation) -> vec4<f32> {
    if !all(abs(uv) <= vec2(P1_LARGEST_FLOAT)) {
        return vec4(0.0);
    }

    let level = p1_base_level(image);
    let clamps = (vec2(layer.ext.y) & vec2(P1_CLAMP_U, P1_CLAMP_V)) != vec2(0u);
    if (layer.ext.y & P1_NEAREST) != 0u {
        return p1_nearest(level, uv, clamps);
    }

    let stored = p1_bilinear(level, uv, clamps);
    let full = select(stored, vec4(p1_linearized(stored.rgb), stored.a), layer.projection.y != 0.0);
    let footprint = layer.projection.x;
    if (layer.ext.y & P1_HQ) == 0u || !(footprint > 1.0 && p1_finite(footprint)) {
        return full;
    }

    // Blend the two levels round the footprint's level of detail, both through one
    // p1_level_sample call (the driver inlines each call site)
    let detail = min(log2(footprint), f32(p1_level_count(image)));
    let finer = u32(floor(detail));
    let toward_coarser = detail - f32(finer);
    let levels = select(2u, 1u, toward_coarser <= 0.0);
    var samples = array<vec4<f32>, 2>(full, full);
    for (var k = 0u; k < levels; k++) {
        samples[k] = p1_level_sample(image, finer + k, uv, clamps, full);
    }
    if levels == 1u {
        return samples[0];
    }
    return samples[0] + (samples[1] - samples[0]) * toward_coarser;
}

// `point` turned by the sine and cosine in `turn`, scaled by `tiling` and moved by `offset` (compose placement)
fn p1_place(point: vec2<f32>, tiling: vec2<f32>, turn: vec2<f32>, offset: vec2<f32>) -> vec2<f32> {
    return vec2(point.x * turn.y - point.y * turn.x, point.x * turn.x + point.y * turn.y) * tiling + offset;
}

// A 3D projection's UV transformation (compose place_uv); the default changes nothing
fn p1_place_uv(op: Operation, point: vec2<f32>) -> vec2<f32> {
    if (op.ext.y & P1_UV_PLACED) == 0u {
        return point;
    }

    return p1_place(point, p1_vec2(op, P1_UV_TILING), p1_vec2(op, P1_UV_ROTATION), p1_vec2(op, P1_UV_OFFSET));
}

// A frame's image placement, turned round the frame's centre (compose place_frame_uv)
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

// `coords` inside the projector turned and stretched by its rotation and scale (compose projector_local)
fn p1_local(op: Operation, coords: P1Coords) -> P1Coords {
    if (op.ext.y & P1_TURNS) == 0u {
        return coords;
    }

    let to_local = p1_to_local(op);
    let stretch = p1_vec3(op, P1_STRETCH);
    let nrm = p1_normalized(to_local * coords.nrm * stretch, coords.nrm);
    return P1Coords(coords.uv, to_local * coords.pos / stretch, nrm);
}

// 0..1 round a spherical projection's pole axis and down from its top (compose spherical_uv)
fn p1_sphere(op: Operation, pos: vec3<f32>) -> vec2<f32> {
    let direction = p1_normalized(pos + p1_vec3(op, P1_OFFSET), vec3(0.0, 1.0, 0.0));
    return vec2(azimuth(direction.z, direction.x), acos(clamp(direction.y, -1.0, 1.0)) / 3.141592654);
}

// 0..1 round a cylindrical projection's axis over its angle, and the height along it (compose cylinder_coords)
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

// Where the projection reads an image at `coords` (compose image_samples)
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

// Where the projection reads a pattern at `coords`, times the pattern scale (compose procedural_in)
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

// The image points as a symmetry copy sees them, its image turned over (compose FillCopy::uv)
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
    // `authored_image_value` at a projected point alone: the other projections and the patterns
    // would be inlined at every read
    if image == MISSING_IMAGE {
        return vec3(0.5);
    }
    let sampled = p1_image_at(image, point, read);
    if kind == P1_ALPHA_KIND {
        return vec3(sampled.a);
    }
    return decode_image_sample(read, sampled.rgb);
}

// An image source's value from its samples (compose Texel::source): read as stored, weighed, then decoded for the channel
fn p1_image_value(op: Operation, samples: P1Samples, surface: Surface, footprints: vec3<f32>) -> vec3<f32> {
    let image = op.ids.z;
    if image == P1_MISSING {
        return vec3(0.5);
    }

    let srgb_color = op.ids.w == 0u && (srgb[image + 2u] & P1_SRGB_IMAGE) != 0u;

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
        case 0u: { return select(rgb, p1_linearized(rgb), srgb_color && !linear); }
        case 1u: { return vec3(dot(rgb, vec3(0.3, 0.59, 0.11))); }
        case 2u: { return vec3(dot(rgb, vec3(0.3, 0.59, 0.11)) * 2.0 - 1.0); }
        case 4u: { return vec3(rgb.r, 1.0 - rgb.g, rgb.b); }
        default: { return rgb; }
    }
}

// A procedural source read at its pattern points: the pattern code weighs and shapes them
// (`value` of the returned operation)
fn p1_procedural_planes(op: Operation, samples: P1Samples) -> Operation {
    var planes = op;
    planes.procedural.w = P1_PLANES_PROJECTION;
    planes.projection = vec4(samples.points[0], samples.points[1]);
    planes.constraints = vec4(bitcast<vec2<u32>>(samples.points[2]), 0u, 0u);
    planes.balance = vec4(op.balance.x, samples.weights);
    return planes;
}

// A world point in the turned and stretched frame: across and up in frame sizes, out in world units; w 0 if not finite (compose frame_local)
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

// How squarely a surface with normal `nrm` faces the turned frame (compose frame_sample)
fn p1_frame_facing(op: Operation, nrm: vec3<f32>) -> f32 {
    let normal = p1_vec3(op, P1_FRAME_NORMAL);
    if (op.ext.y & P1_TURNS) == 0u {
        return dot(nrm, normal);
    }

    let turned = vec3(dot(nrm, p1_vec3(op, P1_FRAME_RIGHT)), dot(nrm, p1_vec3(op, P1_FRAME_UP)), dot(nrm, normal));
    return p1_normalized(p1_to_local(op) * turned * p1_vec3(op, P1_STRETCH), turned).z;
}

// How much a face turned `facing` (the cosine of its turn from the projection) keeps (compose backface_cover)
fn p1_backface(op: Operation, facing: f32) -> f32 {
    let threshold = param(op, P1_BACKFACE_THRESHOLD);
    let width = param(op, P1_BACKFACE_WIDTH);
    if width <= P1_FLOAT_EPSILON {
        return select(0.0, 1.0, facing > threshold);
    }

    return p1_smoothstep(threshold, threshold + width, facing);
}

// 0 past `depth` (at least the smallest frame extent) in front of or behind a frame, fading near it (compose depth_fade)
fn p1_depth_fade(z: f32, depth: f32) -> f32 {
    let distance = abs(z);
    if distance > depth {
        return 0.0;
    }

    return 1.0 - p1_smoothstep(P1_DEPTH_FADE_START * depth, depth, distance);
}

// A placed frame at `world` with normal `nrm`: uv, cover, and 1 in w (0 where not finite) (compose placed_cover)
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

// A placed frame and its mirrored copy: whichever covers the point more (compose placed_cover_symmetric)
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

// How far across the warp's quad a point lies at height `v`, on the axis the edge spans most: u, and 1 in y if finite (compose warp.rs across)
fn p1_warp_across(top: vec2<f32>, left: vec2<f32>, twist: vec2<f32>, relative: vec2<f32>, v: f32) -> vec2<f32> {
    let span = top + twist * v;
    var u = (relative.y - left.y * v) / span.y;
    if abs(span.x) >= abs(span.y) {
        u = (relative.x - left.x * v) / span.x;
    }
    return vec2(u, select(0.0, 1.0, p1_finite(u)));
}

// How far `uv` lies outside the image's 0..1 square (compose warp.rs outside)
fn p1_outside(uv: vec2<f32>) -> f32 {
    let past = uv - clamp(uv, vec2(0.0), vec2(1.0));
    return dot(past, past);
}

// A frame point inside the bilinear quad of the warp's corners: uv, and 1 in z where there is one (compose warp.rs warp_uv)
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

        // Avoid subtracting near-equal values; the root takes linear's sign bit, as copysign does
        let root = sqrt(discriminant);
        let root_term = -0.5 * (linear + select(root, -root, (bitcast<u32>(linear) >> 31u) != 0u));
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

// A warp at `world`: uv in the quad, how much the quad and its reach cover it when cropped, 0 in w where nothing lands (compose warp_cover)
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

// A framed projection at `world` with normal `nrm`: uv, cover, and 0 in w where nothing lands (compose framed_cover)
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

// How much the shape crop and backface culling of a tri-planar, planar, spherical or cylindrical projection let through (compose projector_cover)
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

// The surfaces of the texels across and down from `uv`, for the footprints (invalid without Bilinear HQ)
fn p1_neighbours(op: Operation, uv: vec2<f32>) -> P1Neighbours {
    let invalid = NO_SURFACE;
    let kind = param_word(op, P1_KIND);
    if (op.ext.y & P1_HQ) == 0u || kind == P1_KIND_UV || kind == P1_KIND_UV_TILE {
        return P1Neighbours(invalid, invalid);
    }

    let pixel = vec2<u32>(uv * vec2<f32>(params.source.xy));
    return P1Neighbours(surface_at(pixel + vec2(1u, 0u)), surface_at(pixel + vec2(0u, 1u)));
}

// Image texels per surface texel in each plane of an image `size` texels large: the UV step, or the next texels' points (compose Texel::footprints)
fn p1_footprints(op: Operation, samples: P1Samples, uv: vec2<f32>, neighbours: P1Neighbours, copy: P1Copy, size: vec2<f32>) -> vec3<f32> {
    let source = vec2<f32>(params.source.xy);
    let kind = param_word(op, P1_KIND);
    if kind == P1_KIND_UV || kind == P1_KIND_UV_TILE {
        // The current tile's affine mapping, even where a neighbour wraps or is uncovered
        // P8: in a UV tile atlas, the mesh UV step of the texel's own tile
        let uv_step = udim_mesh_uv_step(vec2<u32>(uv * source));
        var span = max(uv_step.x * size.x, uv_step.y * size.y);
        if kind == P1_KIND_UV {
            let scale = p1_vec2(op, P1_SCALE);
            let turn = p1_vec2(op, P1_ROTATION);
            let across = length(p1_place(vec2(uv_step.x, 0.0), scale, turn, vec2(0.0)) * size);
            let down = length(p1_place(vec2(0.0, uv_step.y), scale, turn, vec2(0.0)) * size);
            span = max(across, down);
        }
        return vec3(select(1.0, max(span, 1.0), p1_finite(span)), 1.0, 1.0);
    }

    var points = samples.points;
    var footprints = vec3(1.0);
    let pixel = vec2<u32>(uv * source);
    let seam_limit = max(size.x, size.y) * P1_SEAM_JUMP_SHARE;
    for (var down = 0u; down < 2u; down++) {
        let next_pixel = pixel + vec2(1u - down, down);
        var neighbour = neighbours.across;
        if down == 1u {
            neighbour = neighbours.down;
        }
        neighbour = p1_copied(neighbour, copy);
        if !neighbour.valid {
            continue;
        }

        // P8: the neighbour's mesh UV on a UV tile atlas
        var coords = P1Coords(udim_mesh_uv((vec2<f32>(next_pixel) + 0.5) / source), neighbour.position, neighbour.normal);
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

// 1 where the image point `uv` lies inside its 0..1 tile on every axis that does not repeat (compose Texel::tile_cover)
fn p1_tile_cover(op: Operation, uv: vec2<f32>) -> f32 {
    if (op.ext.y & P1_TILE_COVER) == 0u {
        return 1.0;
    }

    let inside = uv >= vec2(0.0) & uv < vec2(1.0);
    let repeats = (vec2(op.ext.y) & vec2(P1_CLAMP_U, P1_CLAMP_V)) == vec2(0u);
    return select(0.0, 1.0, all(inside | repeats));
}

// The alpha of a cropped frame's image (its frame record) at the first image point, with the fill's sampling (compose Texel::shape_cover)
fn p1_frame_alpha(op: Operation, samples: P1Samples, uv: vec2<f32>, surface: Surface, neighbours: P1Neighbours, copy: P1Copy) -> f32 {
    let alpha_image = srgb[u32(op.uv.x) + P1_DECAL_ALPHA_WORD];
    if alpha_image == P1_MISSING {
        return 1.0;
    }

    // Read at the image's own footprint so shrunk colour and coverage agree
    var footprint = 1.0;
    if (op.ext.y & P1_HQ) != 0u {
        footprint = p1_footprints(op, samples, uv, neighbours, copy, vec2<f32>(vec2(srgb[alpha_image], srgb[alpha_image + 1u]))).x;
    }
    return clamp(p1_read_image(op, alpha_image, samples.points[0], surface, footprint, false, P1_ALPHA_KIND).x, 0.0, 1.0);
}

// How much depth culling leaves at `position` (mesh radii, as copy `copy_index` sees it) through that copy's grid (compose ProjectorDepth::cover)
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

    // The cell's nearest surface: nearness, then its slope across and up, in three texels of a row
    let texel = vec2<i32>(i32(srgb[plane + 2u] + cell_at.x * 3u), i32(srgb[plane + 3u] + cell_at.y));
    let layer = i32(srgb[plane + 4u]);
    let near = bitcast<f32>(textureLoad(tile_images, texel, layer, 0).x);
    if !p1_finite(near) {
        return 1.0;
    }

    let slope = bitcast<vec2<f32>>(vec2(textureLoad(tile_images, texel + vec2(1, 0), layer, 0).x, textureLoad(tile_images, texel + vec2(2, 0), layer, 0).x));

    // Extend the cell's plane so a surface never culls itself, with a slope bias for curvature across triangles
    let centre = origin + (vec2<f32>(cell_at) + 0.5) * cell;
    let behind = near + dot(slope, view.xy - centre) - view.z;
    let bias = bitcast<f32>(srgb[entry + 7u]) * unit + min(dot(abs(slope) * cell, vec2(1.0)), bitcast<f32>(srgb[entry + 8u]) * unit);
    let width = (1.0 - clamp(param(op, P1_DEPTH_HARDNESS), 0.0, 1.0)) * bitcast<f32>(srgb[entry + 9u]) * unit;
    if width <= P1_FLOAT_EPSILON {
        return select(0.0, 1.0, behind <= bias);
    }

    return 1.0 - p1_smoothstep(bias, bias + width, behind);
}

// How much of the texel the fill's shape covers, the surface as the copy sees it (compose Texel::shape_cover)
fn p1_shape_cover(op: Operation, coords: P1Coords, projected: P1Projected, uv: vec2<f32>, surface: Surface, neighbours: P1Neighbours, copy: P1Copy) -> f32 {
    let flags = op.ext.y;
    if (flags & P1_FRAMED) != 0u {
        let samples = p1_flipped(p1_image_points(op, projected.coords), copy);
        var cover = projected.cover;
        if (flags & P1_IMAGE_ALPHA) != 0u && cover > 0.0 {
            cover *= p1_frame_alpha(op, samples, uv, surface, neighbours, copy);
        }
        return cover * p1_tile_cover(op, samples.points[0]);
    }

    if (flags & (P1_TILE_COVER | P1_CROP | P1_BACKFACE)) == 0u {
        return 1.0;
    }

    return p1_tile_cover(op, p1_flipped(p1_image_points(op, coords), copy).points[0]) * p1_projector_cover(op, coords);
}

// How much of the texel the fill covers: its shape cover, then depth culling where any is left (compose Texel::cover)
fn p1_cover(op: Operation, coords: P1Coords, projected: P1Projected, uv: vec2<f32>, surface: Surface, neighbours: P1Neighbours, copy: P1Copy) -> f32 {
    let shape = p1_shape_cover(op, coords, projected, uv, surface, neighbours, copy);
    if (op.ext.y & P1_DEPTH) == 0u || shape <= 0.0 || !surface.valid {
        return shape;
    }

    return shape * p1_depth_cover(op, copy.index, surface.position);
}

// The surface as copy `copy` sees it (compose Texel::surface)
fn p1_copied(surface: Surface, copy: P1Copy) -> Surface {
    if !copy.moves || !surface.valid {
        return surface;
    }

    // The CPU's copy turns the point and the normals, not the tangent (Texel::surface)
    return Surface(copy.pivot + copy.turn * (surface.position - copy.pivot), copy.turn * surface.normal, true, surface.tangent);
}

// The fill sampled once through `copy`: its value and its coverage (compose Texel::fill_texel)
fn p1_fill_once(op: Operation, uv: vec2<f32>, surface: Surface, neighbours: P1Neighbours, copy: P1Copy) -> vec4<f32> {
    let copied = p1_copied(surface, copy);
    // P8: projections see the mesh's own UV on a UV tile atlas, so they repeat per tile
    let coords = P1Coords(udim_mesh_uv(uv), copied.position, copied.normal);
    let projected = p1_projected(op, coords, copied);
    let kind = op.ids.y;
    var rgb: vec3<f32>;
    if kind == IMAGE_SOURCE_OPCODE {
        let samples = p1_flipped(p1_image_points(op, projected.coords), copy);
        var footprints = vec3(1.0);
        if (op.ext.y & P1_HQ) != 0u && op.ids.z != P1_MISSING {
            footprints = p1_footprints(op, samples, uv, neighbours, copy, vec2<f32>(vec2(srgb[op.ids.z], srgb[op.ids.z + 1u])));
        }
        rgb = p1_image_value(op, samples, surface, footprints);
    } else {
        // Patterns and plain sources through one `value` call (the driver inlines each call site)
        var source = op;
        if kind != 0u && kind != P1_ANCHOR_SOURCE {
            source = p1_procedural_planes(op, p1_pattern_points(op, projected.coords));
        }
        rgb = value(source, uv, surface);
    }
    return vec4(rgb, p1_cover(op, coords, projected, uv, copied, neighbours, copy));
}

// The radial copy nearest a texel `turned` degrees round the axis, of `count` copies `step` degrees apart over `span` (compose nearest_copy)
fn p1_nearest_copy(turned: f32, step: f32, count: u32, span: f32) -> u32 {
    // A span of nothing (or no step at all) stacks every copy on the original
    if !(step > 0.0) {
        return 0u;
    }

    // Halves round away from zero, as Rust's round does
    let steps = turned / step;
    let below = floor(steps);
    let by_step = u32(select(below, below + 1.0, steps - below >= 0.5));
    if span >= P1_FULL_TURN_DEGREES {
        return by_step % count;
    }

    let last = min(by_step, count - 1u);
    let past_last = abs(turned - f32(last) * step);
    let before_first = P1_FULL_TURN_DEGREES - turned;
    return select(last, 0u, before_first <= past_last);
}

// Copy `index` of the fill's symmetry about `pivot`: the original at 0, then the mirror or the radial copies (compose FillCopies::copy)
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

// Every copy of a symmetric fill laid over the others, the texel's own copy on top (compose Texel::symmetric_copies, in mesh radii)
// `symmetric` false: the fill once, as it is (one p1_fill_once call site for both: the driver
// inlines each call site)
fn p1_symmetric(op: Operation, uv: vec2<f32>, surface: Surface, neighbours: P1Neighbours, symmetric: bool) -> vec4<f32> {
    var pivot = vec3(0.0);
    var axis = 0u;
    var count = 1u;
    if symmetric {
        pivot = p1_vec3(op, P1_SYMMETRY_POSITION);
        axis = param_word(op, P1_SYMMETRY_AXIS);
        count = param_word(op, P1_SYMMETRY_COUNT);
    }
    let from_pivot = surface.position - pivot;

    // The copy whose half (mirror) or wedge (radial) holds the texel comes first
    var first = select(0u, 1u, symmetric && from_pivot[axis] < 0.0);
    if symmetric && param_word(op, P1_SYMMETRY_KIND) != 0u {
        let turned = degrees(atan2(from_pivot[(axis + 2u) % 3u], from_pivot[(axis + 1u) % 3u]));
        first = p1_nearest_copy(select(turned, turned + P1_FULL_TURN_DEGREES, turned < 0.0), param(op, P1_SYMMETRY_STEP), count, param(op, P1_SYMMETRY_SPAN));
    }

    var color = vec3(0.0);
    var cover = 0.0;
    for (var next = 0u; next < count; next++) {
        var copy = p1_identity_copy();
        if symmetric {
            copy = p1_copy(op, (first + next) % count, pivot);
        }
        let one = p1_fill_once(op, uv, surface, neighbours, copy);
        if !symmetric {
            return one;
        }
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

// A fill with projection or sampling flags (Operation.ext.y != 0): value, and coverage in alpha; fill_value calls it first
fn projection_fill_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> {
    let neighbours = p1_neighbours(layer, uv);

    // An uncovered texel has no place to copy, so it samples the fill once
    return p1_symmetric(layer, uv, surface, neighbours, (layer.ext.y & P1_SYMMETRY) != 0u && surface.valid);
}
