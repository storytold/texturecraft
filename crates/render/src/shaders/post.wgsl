// Viewport post processing, appended to util.wgsl (uses `src`, `src_samp`, `vs_full`): the
// resolved scene (linear HDR, premultiplied by scene coverage) → studio backdrop, exposure,
// tone mapping, optional bloom / sharpen / vignette / grain, sRGB encoding.

struct Post {
    a: vec4<f32>, // x exposure multiplier (2^EV), y tone mapping (0 linear, 1 ACES), z data view (1: no exposure, tone mapping or effects), w bloom strength
    b: vec4<f32>, // x vignette, y sharpen, z grain, w bloom threshold
    c: vec4<f32>, // depth of field (fx.wgsl): x circle of confusion at infinity (pixels), y focus depth, z largest circle radius (pixels), w accumulation sample
    d: vec4<f32>, // subsurface blur (fx.wgsl): xyz mean free path per channel (world units), w pixels per world unit at depth 1
    e: vec4<f32>, // x orthographic camera (1: pixels per world unit don't fall off with depth), y accumulation sample, zw -
    back_top: vec4<f32>, // rgb background color at the top of the view (linear), a -
    back_bottom: vec4<f32>, // rgb background color at the bottom of the view (linear), a -
};

@group(0) @binding(3) var<uniform> post: Post;
// The bloom pyramid in the post pass; the first extra input of the effect passes (fx.wgsl).
@group(0) @binding(4) var fx_a: texture_2d<f32>;

// Narkowicz's fitted ACES filmic curve (pre-scaled so mid gray stays put).
fn aces(x: vec3<f32>) -> vec3<f32> {
    let v = x * 0.6;
    return clamp((v * (2.51 * v + 0.03)) / (v * (2.43 * v + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn tonemap(hdr: vec3<f32>) -> vec3<f32> {
    let x = max(hdr, vec3<f32>(0.0)) * post.a.x;
    if (post.a.y > 0.5) {
        return aces(x);
    }
    return min(x, vec3<f32>(1.0));
}

// The plain background behind the scene: the two colors of Settings > General > Background color,
// top to bottom (the same color at both ends is a flat background).
fn backdrop(uv: vec2<f32>) -> vec3<f32> {
    return mix(post.back_top.rgb, post.back_bottom.rgb, uv.y);
}

// Display-linear color of one pixel (scene over backdrop), without bloom.
fn shade_px(uv: vec2<f32>) -> vec3<f32> {
    let c = textureSampleLevel(src, src_samp, uv, 0.0);
    let a = clamp(c.a, 0.0, 1.0);
    let s = c.rgb / max(c.a, 1e-4);
    var m = tonemap(s);
    if (post.a.z > 0.5) {
        m = clamp(s, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return mix(backdrop(uv), m, a);
}

fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

@fragment
fn fs_post(i: FsOut) -> @location(0) vec4<f32> {
    var col: vec3<f32>;
    if (post.a.w > 0.0) {
        // Bloom adds light in scene space, so it is tone mapped with the pixel; over the backdrop
        // it glows on its own.
        let c = textureSampleLevel(src, src_samp, i.uv, 0.0);
        let glow = textureSampleLevel(fx_a, src_samp, i.uv, 0.0).rgb * post.a.w;
        col = mix(backdrop(i.uv) + tonemap(glow), tonemap(c.rgb / max(c.a, 1e-4) + glow), clamp(c.a, 0.0, 1.0));
    } else {
        col = shade_px(i.uv);
    }
    if (post.b.y > 0.0) {
        // Unsharp mask on the display image (no HDR halos around highlights).
        let t = 1.0 / vec2<f32>(textureDimensions(src));
        let blur = (shade_px(i.uv + vec2<f32>(t.x, 0.0)) + shade_px(i.uv - vec2<f32>(t.x, 0.0)) + shade_px(i.uv + vec2<f32>(0.0, t.y)) + shade_px(i.uv - vec2<f32>(0.0, t.y))) * 0.25;
        col = max(col + (shade_px(i.uv) - blur) * post.b.y * 2.0, vec3<f32>(0.0));
    }
    if (post.b.x > 0.0) {
        let q = length(i.uv * 2.0 - 1.0) * 0.70710678;
        col *= 1.0 - post.b.x * q * q * q;
    }
    var out = to_srgb(col);
    if (post.b.z > 0.0) {
        // Stable film grain, strongest in the mid tones.
        let l = dot(out, vec3<f32>(0.2126, 0.7152, 0.0722));
        out += (hash12(i.pos.xy) - 0.5) * post.b.z * 0.12 * (1.0 - abs(l * 2.0 - 1.0) * 0.6);
    }
    return vec4<f32>(out, 1.0);
}

// Bloom: a dual-filter blur pyramid (Bjørge, "Bandwidth-Efficient Rendering", SIGGRAPH 2015) over a
// soft-knee bright pass.
fn bright(c: vec3<f32>) -> vec3<f32> {
    let t = post.b.w;
    let knee = t * 0.5;
    let br = max(c.r, max(c.g, c.b)) * post.a.x;
    var soft = clamp(br - t + knee, 0.0, 2.0 * knee);
    soft = soft * soft / (4.0 * knee + 1e-4);
    return c * (max(soft, br - t) / max(br, 1e-4));
}

fn down(uv: vec2<f32>) -> vec3<f32> {
    let h = 1.0 / vec2<f32>(textureDimensions(src));
    var s = textureSampleLevel(src, src_samp, uv, 0.0).rgb * 4.0;
    s += textureSampleLevel(src, src_samp, uv - h, 0.0).rgb;
    s += textureSampleLevel(src, src_samp, uv + h, 0.0).rgb;
    s += textureSampleLevel(src, src_samp, uv + vec2<f32>(h.x, -h.y), 0.0).rgb;
    s += textureSampleLevel(src, src_samp, uv - vec2<f32>(h.x, -h.y), 0.0).rgb;
    return s / 8.0;
}

@fragment
fn fs_bloom_pre(i: FsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(bright(down(i.uv)), 1.0);
}

@fragment
fn fs_bloom_down(i: FsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(down(i.uv), 1.0);
}

// Upsample the smaller level (tent of 8 taps) and add it onto the larger one (additive blend).
@fragment
fn fs_bloom_up(i: FsOut) -> @location(0) vec4<f32> {
    let h = 1.0 / vec2<f32>(textureDimensions(src));
    let uv = i.uv;
    var s = textureSampleLevel(src, src_samp, uv + vec2<f32>(-2.0 * h.x, 0.0), 0.0).rgb;
    s += textureSampleLevel(src, src_samp, uv + vec2<f32>(2.0 * h.x, 0.0), 0.0).rgb;
    s += textureSampleLevel(src, src_samp, uv + vec2<f32>(0.0, -2.0 * h.y), 0.0).rgb;
    s += textureSampleLevel(src, src_samp, uv + vec2<f32>(0.0, 2.0 * h.y), 0.0).rgb;
    s += textureSampleLevel(src, src_samp, uv + h, 0.0).rgb * 2.0;
    s += textureSampleLevel(src, src_samp, uv - h, 0.0).rgb * 2.0;
    s += textureSampleLevel(src, src_samp, uv + vec2<f32>(h.x, -h.y), 0.0).rgb * 2.0;
    s += textureSampleLevel(src, src_samp, uv - vec2<f32>(h.x, -h.y), 0.0).rgb * 2.0;
    return vec4<f32>(s / 12.0, 1.0);
}
