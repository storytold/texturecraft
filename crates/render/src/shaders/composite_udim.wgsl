// P8, UDIM texture sets and baked layouts (gpu_composite_udim.rs). Owned by the P8 agent.
// Compiled into the composite shader when the program needs it. P8 defines its per-tile
// functions here; the stub string in the Rust module must declare the same signatures.

// A UV tile atlas's record in the side table (udim::bind in Rust), at params.packages[7].x (0 for
// a classic set): its grid, block side, flags and stage level table, then each block's scale (f32
// bits) and level, row by row
const UDIM_COLS: u32 = 0u;
const UDIM_ROWS: u32 = 1u;
const UDIM_COL: u32 = 2u;
const UDIM_ROW: u32 = 3u;
const UDIM_BLOCK: u32 = 4u;
const UDIM_FLAGS: u32 = 5u;
const UDIM_SLOTS: u32 = 6u;
const UDIM_SCALES: u32 = 7u;
// UDIM_FLAGS: a tile has a resolution of its own (TextureSet::from_atlas undoes its scale)
const UDIM_SCALED: u32 = 1u;
// The anchor source opcode (ANCHOR_SOURCE in Rust): it reads its field at the texel
const UDIM_ANCHOR_SOURCE: u32 = 15u;
const UDIM_NONE: u32 = 0xffffffffu;
// The bake maps of separate heights (BakedInputs::separate in Rust): 4 built-in maps, then 2 row maps
const UDIM_BUILT_IN_MAPS: u32 = 4u;
const UDIM_ROW_MAPS: u32 = 2u;

fn udim_record() -> u32 {
    return params.packages[7].x;
}

// Rust's f32::round for the positive values here (WGSL's round goes to even)
fn udim_round(value: f32) -> f32 {
    let whole = floor(value);
    return whole + select(0.0, 1.0, value - whole >= 0.5);
}

// The working width of one UV tile's block, or of a classic texture set
fn udim_block_resolution() -> u32 {
    let record = udim_record();
    return select(params.source.x, srgb[record + UDIM_BLOCK], record != 0u);
}

// The atlas block holding `pixel`, row by row, or UDIM_NONE past the grid (BlockScales::at)
fn udim_block_index(pixel: vec2<u32>) -> u32 {
    let record = udim_record();
    let cols = srgb[record + UDIM_COLS];
    let cell = pixel / max(srgb[record + UDIM_BLOCK], 1u);
    if record == 0u || cell.x >= cols || cell.y >= srgb[record + UDIM_ROWS] {
        return UDIM_NONE;
    }
    return cell.y * cols + cell.x;
}

// The tile scale of the block holding `pixel` (1 past the grid)
fn udim_scale_at(pixel: vec2<u32>) -> f32 {
    let index = udim_block_index(pixel);
    if index == UDIM_NONE {
        return 1.0;
    }
    return bitcast<f32>(srgb[udim_record() + UDIM_SCALES + index]);
}

// The level (distinct tile scale, largest first) of the block holding `pixel`
fn udim_level_at(pixel: vec2<u32>) -> u32 {
    let index = udim_block_index(pixel);
    if index == UDIM_NONE {
        return 0u;
    }
    let record = udim_record();
    let blocks = srgb[record + UDIM_COLS] * srgb[record + UDIM_ROWS];
    return srgb[record + UDIM_SCALES + blocks + index];
}

// Atlas UV to mesh UV of the grid alone (TileGrid::from_atlas)
fn udim_grid_uv(uv: vec2<f32>) -> vec2<f32> {
    let record = udim_record();
    let across = uv.x * f32(srgb[record + UDIM_COLS]) + f32(srgb[record + UDIM_COL]);
    let down = uv.y * f32(srgb[record + UDIM_ROWS]) - f32(srgb[record + UDIM_ROW]);
    return vec2(across, down);
}

// The scale of the UV tile holding mesh UV `mesh` (TextureSet::tile_scale of udim_at): its
// block's, 1 outside the grid
fn udim_tile_scale(mesh: vec2<f32>) -> f32 {
    let record = udim_record();
    let cols = srgb[record + UDIM_COLS];
    let cell = vec2<i32>(vec2(floor(mesh.x) - f32(srgb[record + UDIM_COL]), f32(srgb[record + UDIM_ROW]) + floor(mesh.y)));
    if any(cell < vec2(0)) || cell.x >= i32(cols) || cell.y >= i32(srgb[record + UDIM_ROWS]) {
        return 1.0;
    }
    return bitcast<f32>(srgb[record + UDIM_SCALES + u32(cell.y) * cols + u32(cell.x)]);
}

// The mesh's own UV at atlas UV `uv` (TextureSet::from_atlas): projections and patterns see it,
// so they repeat per UV tile (Texel::tile_uv)
fn udim_mesh_uv(uv: vec2<f32>) -> vec2<f32> {
    let record = udim_record();
    if record == 0u {
        return uv;
    }
    let mesh = udim_grid_uv(uv);
    if (srgb[record + UDIM_FLAGS] & UDIM_SCALED) == 0u {
        return mesh;
    }

    let corner = floor(mesh);
    return corner + (mesh - corner) / udim_tile_scale(mesh);
}

// The UV a fill's value reads at atlas UV `uv`: the mesh's own, except for an anchor, which reads
// its field at the texel
fn udim_value_uv(layer: Operation, uv: vec2<f32>) -> vec2<f32> {
    if layer.ids.y == UDIM_ANCHOR_SOURCE {
        return uv;
    }
    return udim_mesh_uv(uv);
}

// Mesh UV units per atlas texel within the UV tile at `pixel` (Inputs::mesh_uv_step)
fn udim_mesh_uv_step(pixel: vec2<u32>) -> vec2<f32> {
    let record = udim_record();
    let atlas_step = 1.0 / vec2<f32>(params.source.xy);
    if record == 0u {
        return atlas_step;
    }
    let grid_step = atlas_step * vec2(f32(srgb[record + UDIM_COLS]), f32(srgb[record + UDIM_ROWS]));
    if (srgb[record + UDIM_FLAGS] & UDIM_SCALED) == 0u {
        return grid_step;
    }

    // Locate the tile before undoing its scale
    let atlas_uv = (vec2<f32>(pixel) + 0.5) * atlas_step;
    return grid_step / udim_tile_scale(udim_grid_uv(atlas_uv));
}

// A fill with its reduced pattern's texels per repeat at the UV tile of atlas UV `uv`: P8's
// ext.w is the side-table table of them per level (udim::pattern_levels in Rust)
fn udim_placed(layer: Operation, uv: vec2<f32>) -> Operation {
    if layer.ext.w == 0u {
        return layer;
    }
    var placed = layer;
    placed.procedural.x = srgb[layer.ext.w + udim_level_at(vec2<u32>(uv * vec2<f32>(params.source.xy)))];
    return placed;
}

// The stage slot a texel at `pixel` reads for stage `slot`: the level stage of its block's tile
// scale where level stages follow the stage (FilterStage::level in Rust)
fn udim_level_slot(slot: u32, pixel: vec2<u32>) -> u32 {
    let record = udim_record();
    if record == 0u {
        return slot;
    }
    let table = srgb[record + UDIM_SLOTS];
    if table == 0u || slot >= srgb[table] || srgb[table + 1u + slot] == 0u {
        return slot;
    }
    return slot + udim_level_at(pixel);
}

// The UV tile's origin and sides at `pixel` (x, y, width, height), where a height slope stays
// (Texel::height_slope): the whole set for a classic set
fn udim_tile_frame(pixel: vec2<u32>) -> vec4<u32> {
    let record = udim_record();
    if record == 0u {
        return vec4(0u, 0u, params.source.xy);
    }
    let block = max(srgb[record + UDIM_BLOCK], 1u);
    let tile = max(u32(udim_round(f32(block) * udim_scale_at(pixel))), 1u);
    return vec4(pixel / block * block, tile, tile);
}

// Working atlas UV to bake atlas UV, without the part of a block a smaller UV tile leaves empty
// (BlockScales::bake_uv): z is 0 where no bake lies
fn udim_bake_uv(uv: vec2<f32>) -> vec3<f32> {
    let record = udim_record();
    if record == 0u || (srgb[record + UDIM_FLAGS] & UDIM_SCALED) == 0u {
        return vec3(uv, 1.0);
    }
    let block = f32(max(srgb[record + UDIM_BLOCK], 1u));
    let size = vec2<f32>(params.source.xy);
    let position = uv * size / block;
    let origin = floor(position);
    let local = (position - origin) / udim_scale_at(vec2<u32>(origin * block));
    if any(local < vec2(0.0)) || any(local >= vec2(1.0)) {
        return vec3(uv, 0.0);
    }
    return vec3((origin + local) * block / size, 1.0);
}

// Whether a bake lies at `pixel` (BakedMaps::sample_working is Some there)
fn udim_bake_valid(pixel: vec2<u32>) -> bool {
    return udim_bake_uv((vec2<f32>(pixel) + 0.5) / vec2<f32>(params.source.xy)).z != 0.0;
}

// Row block `block`'s texel where no bake lies: each map's value when it is not baked
// (GpuCompositor::baked_input in Rust)
fn udim_absent_bake(block: u32) -> vec4<u32> {
    if block == 1u {
        return vec4(0u);
    }
    return vec4(bitcast<u32>(1.0), bitcast<u32>(0.5), bitcast<u32>(1.0), 0u);
}

// Row block `block`'s texel at `pixel` when the baked maps have different heights: each map from
// the rows of its own height, found in the header row params.packages[7].y - 1
fn udim_separate_bake(pixel: vec2<u32>, block: u32) -> vec4<u32> {
    let header = params.packages[7].y - 1u;
    let width = textureDimensions(bake_maps).x;
    let uv = udim_bake_uv((vec2<f32>(pixel) + 0.5) / vec2<f32>(params.source.xy)).xy;
    var texel = udim_absent_bake(block);
    let first = select(0u, UDIM_BUILT_IN_MAPS, block == 1u);
    let count = select(UDIM_BUILT_IN_MAPS, UDIM_ROW_MAPS, block == 1u);
    for (var index = first; index < first + count; index++) {
        let entry = textureLoad(bake_maps, vec2<i32>(vec2(index % width, header + index / width)), 0);
        if entry.y == 0u {
            continue;
        }
        let size = vec2(width, entry.y);
        let at = min(vec2<u32>(uv * vec2<f32>(size)), size - 1u) + vec2(0u, entry.x);
        let stored = textureLoad(bake_maps, vec2<i32>(at), 0);
        // Each map keeps its component: AO x, Normal w, Curvature y, Thickness z; Height x, World Normal yzw
        switch index {
            case 0u, 4u: { texel.x = stored.x; }
            case 1u: { texel.w = stored.w; }
            case 2u: { texel.y = stored.y; }
            case 3u: { texel.z = stored.z; }
            default: { texel = vec4(texel.x, stored.yzw); }
        }
    }
    return texel;
}

// Extra's alpha at `pixel`: 32 × log2(1 / the UV tile's scale), which the viewport's height
// slopes read (encode_tile in compose's display.rs)
fn udim_extra_scale(pixel: vec2<u32>) -> u32 {
    return u32(clamp(udim_round(-32.0 * log2(udim_scale_at(pixel))), 0.0, 255.0));
}
