// Registration hub of the GPU port packages in the composite shader (gpu_composite_packages.rs).
// Frozen after Package 0: a package defines its functions in its own file (or stub) and never
// edits this one. The opcode blocks and flag bytes match the Rust constants there.

const FIRST_EXTENDED_EFFECT: u32 = 128u;
const END_EXTENDED_EFFECT: u32 = 192u;
const POINT_OPCODES_START: u32 = 128u;
const POINT_OPCODES_END: u32 = 144u;
const STACK_OPCODES_START: u32 = 144u;
const STACK_OPCODES_END: u32 = 152u;
const LIGHTING_OPCODES_START: u32 = 152u;
const LIGHTING_OPCODES_END: u32 = 160u;
// Flag bytes of Operation.ext.z
const POINT_OP_BITS: u32 = 0x000000ffu;
const STACK_OP_BITS: u32 = 0x0000ff00u;
const LIGHTING_OP_BITS: u32 = 0x00ff0000u;
const BRUSH_OP_BITS: u32 = 0xff000000u;

// Fills, paint and patterns: below the built-in effects, and the extended patterns before the
// packages' effects. Generators (40..48) and effects are checked by their callers.
fn is_fill_opcode(kind: u32) -> bool {
    return kind < 20u || (kind >= 48u && kind < FIRST_EXTENDED_EFFECT);
}

fn is_extended_effect(kind: u32) -> bool {
    return kind >= FIRST_EXTENDED_EFFECT && kind < END_EXTENDED_EFFECT;
}

// Parameter `index` of an operation from the side table (ProceduralInputs::pack_params).
fn param(op: Operation, index: u32) -> f32 {
    return bitcast<f32>(srgb[op.ext.x + index]);
}

// Raw word `index` of an operation from the side table (ProceduralInputs::pack_words).
fn param_word(op: Operation, index: u32) -> u32 {
    return srgb[op.ext.x + index];
}

// scalar_filter's default branch: the packages' opcodes, and x for every other opcode.
fn extended_scalar_filter(op: Operation, x: f32) -> f32 {
    let kind = op.ids.y;
    if kind >= POINT_OPCODES_START && kind < POINT_OPCODES_END {
        return point_scalar_filter(op, x);
    }
    if kind >= STACK_OPCODES_START && kind < STACK_OPCODES_END {
        return stack_scalar_filter(op, x);
    }
    if kind >= LIGHTING_OPCODES_START && kind < LIGHTING_OPCODES_END {
        return lighting_scalar_filter(op, x);
    }
    return x;
}

// A packages' content effect in the layer_content loop: the content (colour, coverage) after the
// effect, opacity (effect.low.w) applied. channel is the destination slot, texel the full texel
// (pixel, UV, surface, bakes) for effects that read more than the colour or change coverage.
fn extended_content_effect(effect: Operation, content: vec4<f32>, channel: u32, texel: LayerTexel) -> vec4<f32> {
    let kind = effect.ids.y;
    if kind >= POINT_OPCODES_START && kind < POINT_OPCODES_END {
        return point_content_effect(effect, content, channel, texel);
    }
    if kind >= STACK_OPCODES_START && kind < STACK_OPCODES_END {
        return stack_content_effect(effect, content, channel, texel);
    }
    if kind >= LIGHTING_OPCODES_START && kind < LIGHTING_OPCODES_END {
        return lighting_content_effect(effect, content, channel, texel);
    }
    return content;
}

// A packages' mask effect in the layer_mask loop: the effect's result for mask value mask, which
// the loop then mixes in by the effect's opacity (effect.low.w).
fn extended_mask_effect(effect: Operation, mask: f32, texel: LayerTexel) -> f32 {
    let kind = effect.ids.y;
    if kind >= PROBE_OPCODES_START && kind < END_EXTENDED_EFFECT {
        return probe_mask_effect(effect, texel);
    }
    if kind >= POINT_OPCODES_START && kind < POINT_OPCODES_END {
        return point_mask_effect(effect, mask, texel);
    }
    if kind >= STACK_OPCODES_START && kind < STACK_OPCODES_END {
        return stack_mask_effect(effect, mask, texel);
    }
    if kind >= LIGHTING_OPCODES_START && kind < LIGHTING_OPCODES_END {
        return lighting_mask_effect(effect, mask, texel);
    }
    return mask;
}

// The channel's material at `pixel` that an effect's stack read asked for (Operation.spatial.w,
// bound by FilterPlan::bind_stack_reads): every layer below the effect's layer, or also its
// own operations before Base Color. Zero without a read.
fn stack_at(effect: Operation, pixel: vec2<u32>) -> vec3<f32> {
    if reading_own_layer && effect.effects.y == 1u {
        return channel_default((effect.effects.z & 0xffffu) - 1u);
    }
    if effect.spatial.w == 0u {
        return vec3(0.0);
    }
    return filter_value(effect.spatial.w - 1u, pixel).rgb;
}

// The material of the second channel a package effect asked for (Operation.spatial.z,
// FilterPlan::bind_stack_reads): P6's Metallic and Roughness, or Glossiness and Specular.
fn stack_at_second(effect: Operation, pixel: vec2<u32>) -> vec3<f32> {
    if reading_own_layer && effect.effects.y == 1u {
        return channel_default((effect.effects.z >> 16u) - 1u);
    }
    if effect.spatial.z == 0u {
        return vec3(0.0);
    }
    return filter_value(effect.spatial.z - 1u, pixel).rgb;
}

// Set while the prepare pass evaluates the lit Base Color as its mask reads it (OWN_VIEW): the
// CPU evaluates a mask with no lit material (Texel::lit is None while node_mask runs), so the
// layer's own-material reads (Operation.effects.y 1) give the channels' defaults there
// (channel_default; their channel slots plus one in effects.z).
var<private> reading_own_layer: bool = false;

// Set by layer_mask while it evaluates the lit layer's first mask (params.mask_view)
var<private> mask_view_active: bool = false;

// The effect's layer's own content of the read channel, with its coverage in alpha: Compare's
// This Layer mixes it over stack_at. It reads the field of its This Layer stage (slot plus one in
// Operation.effects.z, FilterPlan::add_this_layer_reads), prepared from the whole content stack
// before the mask: no content code inline in the mask's.
fn this_layer_content(effect: Operation, texel: LayerTexel) -> vec4<f32> {
    var slot = effect.effects.z;
    // The lit layer's first mask reads its Base Color without fields or material (mask_view)
    if mask_view_active && slot == params.mask_view.w {
        slot = params.mask_view.z;
    }
    if slot == 0u {
        return vec4(0.0);
    }
    let field = filter_value(slot - 1u, texel.pixel);
    let coverage = clamp(field.a, 0.0, 1.0);
    return vec4(select(vec3(0.0), field.rgb / coverage, coverage > 1e-5), coverage);
}

// A package effect's image input at the texel, read through layer_content's or layer_mask's one
// base_content call right before the effect (the driver inlines each call site)
var<private> effect_input_cache: vec4<f32> = vec4(0.0);

// The effect's image input at the texel (Operation.effects.w, ProceduralInputs::register_input),
// as P3's stage_input_value reads it: its base content's x.
fn effect_input(effect: Operation, texel: LayerTexel) -> f32 {
    if effect.effects.w == 0u {
        return 0.0;
    }
    return effect_input_cache.x;
}

// The effect's image input as colour (ProceduralInputs::register_color_input).
fn effect_input_color(effect: Operation, texel: LayerTexel) -> vec3<f32> {
    if effect.effects.w == 0u {
        return vec3(0.0);
    }
    return effect_input_cache.rgb;
}

// An image's stored texel under `uv`, wrapping, as ImageRes::sample_nearest reads it (an ID
// map's colours must not blend). `image` is the record offset ProceduralInputs::pack_image's
// side-table word holds (u32 max: missing).
fn image_texel_nearest(image: u32, uv: vec2<f32>) -> vec4<f32> {
    if image == 0xffffffffu {
        return vec4(0.0);
    }
    let size = vec2<i32>(i32(srgb[image]), i32(srgb[image + 1u]));
    let pixel = vec2<i32>(floor(uv * vec2<f32>(size)));
    return material_image_texel(image, pixel, size);
}

// An image's stored texels under `uv` blended bilinearly without wrapping: past an edge it keeps
// the edge texels, as ImageRes::sample_filtered with BilinearSharp and UvWrap::None reads it
// (P7: the brush mask image, MaskGray::at). `image` as for image_texel_nearest.
fn image_texel_bilinear_clamped(image: u32, uv: vec2<f32>) -> vec4<f32> {
    if image == 0xffffffffu {
        return vec4(0.0);
    }
    let size = vec2<i32>(i32(srgb[image]), i32(srgb[image + 1u]));
    let at = uv * vec2<f32>(size) - 0.5;
    let corner = floor(at);
    let t = at - corner;
    let last = size - 1;
    let low = clamp(vec2<i32>(corner), vec2(0), last);
    let high = clamp(vec2<i32>(corner) + 1, vec2(0), last);
    let top = mix(material_image_texel(image, low, size), material_image_texel(image, vec2(high.x, low.y), size), t.x);
    let bottom = mix(material_image_texel(image, vec2(low.x, high.y), size), material_image_texel(image, high, size), t.x);
    return mix(top, bottom, t.y);
}

// Test probes of the hooks above: composite_probes.wgsl, compiled into the render crate's tests
// only (a stub elsewhere, shader_sources in gpu_composite_packages.rs).

// WGSL rejects recursion: a package function must not call anything that leads back to the hook
// that called it (scalar_filter, effect_color, layer_content and layer_mask reach the hooks above,
// fill_value reaches projection_fill_value, painted reaches live_brush_paint).

// effect_color for the packages' opcodes (is_extended_effect). channel is the destination slot
// (channel_slot in Rust: 0 Base Color, 6 Height, 7 Normal, …).
fn extended_effect_color(op: Operation, c: vec3<f32>, channel: u32, surface: Surface, baked_ao: f32) -> vec3<f32> {
    let kind = op.ids.y;
    if kind >= POINT_OPCODES_START && kind < POINT_OPCODES_END {
        return point_effect_color(op, c, channel, surface, baked_ao);
    }
    if kind >= STACK_OPCODES_START && kind < STACK_OPCODES_END {
        return stack_effect_color(op, c, channel, surface, baked_ao);
    }
    if kind >= LIGHTING_OPCODES_START && kind < LIGHTING_OPCODES_END {
        return lighting_effect_color(op, c, channel, surface, baked_ao);
    }
    return c;
}
