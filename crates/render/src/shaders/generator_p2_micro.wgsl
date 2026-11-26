// P2 of the generator GPU port: a generator's Micro Details over the bake's layout, as compose's
// micro_detail/mod.rs builds them. Each UV tile block samples the micro height and normal on its
// own; Standard and Sobel add the divergence of the height's tilted normal, Smooth the height
// above its blurred copies, and the occlusion is the horizon the height raises. Blocks of one size
// read alike, so one block of each size is computed and placed into every block of that size.

struct Params {
    // Block width and height, the normal slot's first word in `slots`, the curvature type (0
    // Standard or Sobel, 1 Smooth)
    block: vec4<u32>,
    // The height image's width and height, the normal image's (0: a uniform slot)
    slots: vec4<u32>,
    // A uniform height's gray, a uniform normal's x and y, the normal image's green sign
    uniforms: vec4<f32>,
    // The first block origin of this size and the step between origins
    origins: vec4<u32>,
    // Origins across and down, the output layer
    place: vec4<u32>,
    // The line axis (0 rows, 1 columns), the box radius (0: the kernel), the kernel reach, and
    // the fields read and written (`read_line`)
    lines: vec4<u32>,
    // The type's constant, its gain on the height, its gain on the normal, the height's tilt
    gains: vec4<f32>,
    // Texels per texel at 2K, the occlusion depth, the occlusion steps per direction, and which
    // slots are read (bit 0 the height, bit 1 the normal)
    scales: vec4<f32>,
    // The height intensity Smooth's detail is scaled by
    intensity: vec4<f32>,
    // An anchor height slot: the map's width and rows (0: no anchor), the working atlas's size
    anchor: vec4<u32>,
    // The working atlas's block side, and the block's tile scale (0: the working UV is the bake's)
    anchor_block: vec4<f32>,
    kernel: array<vec4<f32>, 4>,
    directions: array<vec4<f32>, 16>,
    // Each direction's steps: texel offset x and y, distance
    taps: array<vec4<f32>, 256>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> slots: array<f32>;
@group(0) @binding(2) var<storage, read_write> heights: array<f32>;
@group(0) @binding(3) var<storage, read_write> first: array<f32>;
@group(0) @binding(4) var<storage, read_write> second: array<f32>;
@group(0) @binding(5) var<storage, read_write> detail: array<f32>;
@group(0) @binding(6) var<storage, read_write> normals: array<vec2<f32>>;
@group(0) @binding(7) var<storage, read_write> tilted: array<vec2<f32>>;
@group(0) @binding(8) var maps_out: texture_storage_2d_array<r32float, write>;
@group(0) @binding(9) var bake: texture_2d<u32>;
@group(0) @binding(10) var maps_in: texture_2d_array<f32>;
@group(0) @binding(11) var bake_out: texture_storage_2d<rgba32uint, write>;
@group(0) @binding(12) var capture: texture_2d<f32>;

const OCCLUSION_DIRECTIONS: u32 = 16u;
const OCCLUSION_STRENGTH: f32 = 0.3232;
const SMOOTH_SIGMAS: f32 = 7.0;
const READS_HEIGHT: u32 = 1u;
const READS_NORMAL: u32 = 2u;

fn block_size() -> vec2<i32> {
    return vec2<i32>(params.block.xy);
}

fn wrapped(at: vec2<i32>) -> u32 {
    let size = block_size();
    let x = ((at.x % size.x) + size.x) % size.x;
    let y = ((at.y % size.y) + size.y) % size.y;
    return u32(y * size.x + x);
}

// One channel of an image in `slots` from `start`, `channels` floats per texel, read as
// ImageRes::sample reads it: wrapped, bilinear between the four texel centres around `uv`
fn slot_sample(start: u32, size: vec2<u32>, channels: u32, channel: u32, uv: vec2<f32>) -> f32 {
    let texel = uv * vec2<f32>(size) - 0.5;
    let corner = floor(texel);
    let weight = texel - corner;
    let extent = vec2<i32>(size);
    let left = vec2<i32>(corner);
    let low = ((left % extent) + extent) % extent;
    let high = (((left + 1) % extent) + extent) % extent;
    let read = start + channel;
    let top_left = slots[read + u32(low.y * extent.x + low.x) * channels];
    let top_right = slots[read + u32(low.y * extent.x + high.x) * channels];
    let bottom_left = slots[read + u32(high.y * extent.x + low.x) * channels];
    let bottom_right = slots[read + u32(high.y * extent.x + high.x) * channels];
    let upper = top_left + (top_right - top_left) * weight.x;
    let lower = bottom_left + (bottom_right - bottom_left) * weight.x;
    return upper + (lower - upper) * weight.y;
}

// An anchor point's height under bake texel `texel` (AnchorHeights::at): the capture program's
// texel at the working UV of the texel's bake UV, 0 outside the atlas
fn anchor_height(texel: vec2<u32>) -> f32 {
    let uv = (vec2<f32>(texel) + 0.5) / vec2<f32>(params.anchor.xy);
    if any(uv < vec2(0.0)) || any(uv >= vec2(1.0)) {
        return 0.0;
    }
    let working = vec2<f32>(params.anchor.zw);
    var at = uv;
    let scale = params.anchor_block.y;
    if scale != 0.0 {
        let block = params.anchor_block.x;
        let position = uv * working / block;
        let origin = floor(position);
        at = (origin + (position - origin) * scale) * block / working;
    }
    let pixel = min(vec2<u32>(at * working), params.anchor.zw - 1u);
    return textureLoad(capture, vec2<i32>(pixel), 0).x;
}

// The micro height and normal at each texel of the block; the detail starts at zero
@compute @workgroup_size(8, 8)
fn sample_slots(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.block.xy) {
        return;
    }
    let index = id.y * params.block.x + id.x;
    let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(params.block.xy);
    var height = params.uniforms.x;
    if params.slots.x != 0u {
        height = slot_sample(0u, params.slots.xy, 1u, 0u, uv);
    }
    // An anchor's height, its block computed alone
    if params.anchor.x != 0u {
        height = anchor_height(params.origins.xy + id.xy);
    }
    var normal = params.uniforms.yz;
    if params.slots.z != 0u {
        let red = slot_sample(params.block.z, params.slots.zw, 2u, 0u, uv);
        let green = slot_sample(params.block.z, params.slots.zw, 2u, 1u, uv);
        normal = vec2(red * 2.0 - 1.0, (green * 2.0 - 1.0) * params.uniforms.w);
    }
    heights[index] = height;
    normals[index] = normal;
    detail[index] = 0.0;
}

// The Sobel kernels: horizontal on `horizontal`, vertical on `vertical` (sobel_components)
fn sobel_heights(at: vec2<i32>) -> vec2<f32> {
    let left = at.x - 1;
    let right = at.x + 1;
    let top = at.y - 1;
    let bottom = at.y + 1;
    let column_right = heights[wrapped(vec2(right, top))] + 2.0 * heights[wrapped(vec2(right, at.y))] + heights[wrapped(vec2(right, bottom))];
    let column_left = heights[wrapped(vec2(left, top))] + 2.0 * heights[wrapped(vec2(left, at.y))] + heights[wrapped(vec2(left, bottom))];
    let row_bottom = heights[wrapped(vec2(left, bottom))] + 2.0 * heights[wrapped(vec2(at.x, bottom))] + heights[wrapped(vec2(right, bottom))];
    let row_top = heights[wrapped(vec2(left, top))] + 2.0 * heights[wrapped(vec2(at.x, top))] + heights[wrapped(vec2(right, top))];
    return vec2((column_right - column_left) / 8.0, (row_bottom - row_top) / 8.0);
}

// The divergence of a normal field (`tilted` when `of_tilted`, else `normals`), per texel at 2K
fn divergence(at: vec2<i32>, of_tilted: bool) -> f32 {
    let left = at.x - 1;
    let right = at.x + 1;
    let top = at.y - 1;
    let bottom = at.y + 1;
    var field: array<vec2<f32>, 9>;
    for (var row = 0; row < 3; row++) {
        for (var column = 0; column < 3; column++) {
            let index = wrapped(vec2(at.x + column - 1, at.y + row - 1));
            field[row * 3 + column] = select(normals[index], tilted[index], of_tilted);
        }
    }
    let column_right = field[2].x + 2.0 * field[5].x + field[8].x;
    let column_left = field[0].x + 2.0 * field[3].x + field[6].x;
    let row_bottom = field[6].y + 2.0 * field[7].y + field[8].y;
    let row_top = field[0].y + 2.0 * field[1].y + field[2].y;
    let dx = (column_right - column_left) / 8.0;
    let dy = (row_bottom - row_top) / 8.0;
    return (dx + dy) * params.scales.x;
}

// Standard and Sobel: the height turned into a normal tilted by its slope
@compute @workgroup_size(8, 8)
fn tilt(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.block.xy) {
        return;
    }
    let slope = sobel_heights(vec2<i32>(id.xy));
    let strength = params.gains.w;
    let x = -strength * slope.x;
    let y = -strength * slope.y;
    let norm = sqrt(x * x + y * y + 1.0);
    tilted[id.y * params.block.x + id.x] = vec2(x / norm, y / norm);
}

// Field `which` of a blur: 0 the heights, 1 the first scratch, 2 the second
fn read_line(which: u32, index: u32) -> f32 {
    switch which {
        case 0u: { return heights[index]; }
        case 1u: { return first[index]; }
        default: { return second[index]; }
    }
}

fn write_line(which: u32, index: u32, value: f32) {
    if which == 1u {
        first[index] = value;
    } else {
        second[index] = value;
    }
}

// One blur pass along each line (rows or columns), wrapping around the block: the sampled kernel
// below a sigma of two, else one of three box passes as a running sum (blur_lines)
@compute @workgroup_size(64)
fn blur(@builtin(global_invocation_id) id: vec3<u32>) {
    let columns = params.lines.x == 1u;
    let line_length = select(params.block.x, params.block.y, columns);
    let count = select(params.block.y, params.block.x, columns);
    if id.x >= count {
        return;
    }
    let source = params.lines.w & 3u;
    let destination = params.lines.w >> 2u;
    let line = i32(id.x);
    let span = i32(line_length);
    let radius = i32(params.lines.y);
    if radius == 0 {
        let reach = i32(params.lines.z);
        for (var position = 0; position < span; position++) {
            var sum = 0.0;
            for (var offset = -reach; offset <= reach; offset++) {
                let along = (((position + offset) % span) + span) % span;
                let at = select(vec2(along, line), vec2(line, along), columns);
                let tap = u32(offset + reach);
                sum += read_line(source, wrapped(at)) * params.kernel[tap / 4u][tap % 4u];
            }
            write_line(destination, wrapped(select(vec2(position, line), vec2(line, position), columns)), sum);
        }
        return;
    }
    let window = f32(2 * radius + 1);
    var sum = 0.0;
    for (var offset = -radius; offset <= radius; offset++) {
        let along = ((offset % span) + span) % span;
        sum += read_line(source, wrapped(select(vec2(along, line), vec2(line, along), columns)));
    }
    for (var position = 0; position < span; position++) {
        write_line(destination, wrapped(select(vec2(position, line), vec2(line, position), columns)), sum / window);
        let entering = (((position + radius + 1) % span) + span) % span;
        let leaving = (((position - radius) % span) + span) % span;
        let added = read_line(source, wrapped(select(vec2(entering, line), vec2(line, entering), columns)));
        let removed = read_line(source, wrapped(select(vec2(leaving, line), vec2(line, leaving), columns)));
        sum += added - removed;
    }
}

// Smooth: the height above one blurred copy (in `second`) added to the detail
@compute @workgroup_size(8, 8)
fn accumulate(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.block.xy) {
        return;
    }
    let index = id.y * params.block.x + id.x;
    detail[index] += heights[index] - second[index];
}

// `value` at local texel `local` of every block of this size
fn place(local: vec2<u32>, value: f32) {
    for (var down = 0u; down < params.place.y; down++) {
        for (var across = 0u; across < params.place.x; across++) {
            let at = params.origins.xy + params.origins.zw * vec2(across, down) + local;
            textureStore(maps_out, vec2<i32>(at), i32(params.place.z), vec4(value, 0.0, 0.0, 0.0));
        }
    }
}

// The curvature the micro details add (curvature_term)
@compute @workgroup_size(8, 8)
fn term(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.block.xy) {
        return;
    }
    let at = vec2<i32>(id.xy);
    let reads = u32(params.scales.w);
    var value = params.gains.x;
    if (reads & READS_HEIGHT) != 0u {
        var height_detail = 0.0;
        if params.block.w == 1u {
            height_detail = detail[id.y * params.block.x + id.x] / SMOOTH_SIGMAS * params.intensity.x;
        } else {
            height_detail = divergence(at, true);
        }
        value += height_detail * params.gains.y;
    }
    if (reads & READS_NORMAL) != 0u {
        value += divergence(at, false) * params.gains.z;
    }
    place(id.xy, value);
}

fn sine(slope: f32) -> f32 {
    return slope / sqrt(1.0 + slope * slope);
}

// What the occlusion is multiplied by: the horizon the height raises above its slope in each
// direction (occlusion_factor); 1 where the radius reaches nothing (no steps)
@compute @workgroup_size(8, 8)
fn occlusion(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= params.block.xy) {
        return;
    }
    let steps = u32(params.scales.z);
    if steps == 0u {
        place(id.xy, 1.0);
        return;
    }
    let at = vec2<i32>(id.xy);
    let here = heights[wrapped(at)];
    let gradient = vec2(
        (heights[wrapped(at + vec2(1, 0))] - heights[wrapped(at - vec2(1, 0))]) / 2.0,
        (heights[wrapped(at + vec2(0, 1))] - heights[wrapped(at - vec2(0, 1))]) / 2.0,
    );
    let depth = params.scales.y;
    var occluded = 0.0;
    for (var direction = 0u; direction < OCCLUSION_DIRECTIONS; direction++) {
        var horizon = -3.402823466e38;
        for (var taken = 0u; taken < steps; taken++) {
            let tap = params.taps[direction * steps + taken];
            horizon = max(horizon, (heights[wrapped(at + vec2<i32>(tap.xy))] - here) / tap.z);
        }
        let way = params.directions[direction].xy;
        let tangent = gradient.x * way.x + gradient.y * way.y;
        occluded += max(sine(horizon * depth) - sine(tangent * depth), 0.0);
    }
    place(id.xy, 1.0 - OCCLUSION_STRENGTH * occluded / f32(OCCLUSION_DIRECTIONS));
}

// The bake's curvature rows (the size of `bake_out`) with the micro curvature term (layer
// place.z) added, which the detail sizes then blur (MicroCurvature::curvature)
@compute @workgroup_size(8, 8)
fn adjust_bake(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= textureDimensions(bake_out)) {
        return;
    }
    var texel = textureLoad(bake, vec2<i32>(id.xy), 0);
    let term = textureLoad(maps_in, vec2<i32>(id.xy), i32(params.place.z), 0).x;
    texel.y = bitcast<u32>(bitcast<f32>(texel.y) + term);
    textureStore(bake_out, vec2<i32>(id.xy), texel);
}
