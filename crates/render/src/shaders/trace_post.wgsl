// GPU path tracing, after the samples: the edge-avoiding à-trous denoiser on demodulated radiance
// and the display resolve (tone mapping over the studio backdrop or the environment), ports of
// `Tracer::denoise` and `Tracer::rgba8`.

struct Post {
    size: vec4<u32>,    // width, height, samples, à-trous step
    display: vec4<f32>, // exposure multiplier, ACES (1) or linear (0), environment opacity, denoise (1)
    back_top: vec4<f32>, // rgb background color at the top (linear), a -
    back_bottom: vec4<f32>, // rgb background color at the bottom (linear), a -
};

@group(0) @binding(0) var<uniform> Q: Post;
@group(0) @binding(1) var<storage, read> acc_hit: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> acc_alb: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> acc_nrm: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read> acc_env: array<vec4<f32>>;
// Demodulated radiance and its variance: read from `src`, written to `dst`.
@group(0) @binding(5) var<storage, read> src: array<vec4<f32>>;
@group(0) @binding(6) var<storage, read_write> dst: array<vec4<f32>>;
@group(0) @binding(7) var out: texture_storage_2d<rgba8unorm, write>;

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn normalize_or(v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let l2 = dot(v, v);
    return select(fallback, v * inverseSqrt(l2), l2 > 1e-30);
}

struct Guide {
    albedo: vec3<f32>,
    normal: vec3<f32>,
    depth: f32,
    hit: bool,
};

fn guide(i: u32) -> Guide {
    let a = acc_alb[i];
    let n = max(a.w, 1.0);
    let nd = acc_nrm[i];
    return Guide(max(a.rgb / n, vec3<f32>(0.02)), normalize_or(nd.xyz, vec3<f32>(0.0, 0.0, 1.0)), nd.w / n, a.w > 0.0);
}

@compute @workgroup_size(8, 8)
fn denoise_init(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= Q.size.x || id.y >= Q.size.y) {
        return;
    }
    let i = id.y * Q.size.x + id.x;
    let g = guide(i);
    let h = acc_hit[i];
    let n = max(acc_alb[i].w, 1.0);
    let mean = h.rgb / n;
    let var_l = max(h.w / n - luminance(mean) * luminance(mean), 0.0) / n;
    let a = max(luminance(g.albedo), 0.02);
    dst[i] = vec4<f32>(mean / g.albedo, var_l / (a * a));
}

@compute @workgroup_size(8, 8)
fn denoise_step(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = Q.size.x;
    let h = Q.size.y;
    if (id.x >= w || id.y >= h) {
        return;
    }
    let i = id.y * w + id.x;
    let p = guide(i);
    let ip = src[i];
    if (!p.hit) {
        dst[i] = ip;
        return;
    }
    let step = i32(Q.size.w);
    var kernel = array<f32, 5>(1.0 / 16.0, 1.0 / 4.0, 3.0 / 8.0, 1.0 / 4.0, 1.0 / 16.0);
    let sigma_l = 4.0 * sqrt(ip.w) + 1e-3;
    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    var vsum = 0.0;
    for (var j = 0; j < 5; j++) {
        for (var k = 0; k < 5; k++) {
            let qx = i32(id.x) + (k - 2) * step;
            let qy = i32(id.y) + (j - 2) * step;
            if (qx < 0 || qy < 0 || qx >= i32(w) || qy >= i32(h)) {
                continue;
            }
            let qi = u32(qy) * w + u32(qx);
            let q = guide(qi);
            if (!q.hit) {
                continue;
            }
            let iq = src[qi];
            let wn = pow(max(dot(p.normal, q.normal), 0.0), 64.0);
            let wz = exp(-abs(p.depth - q.depth) / (0.01 * max(p.depth, 1e-3) * f32(step)));
            let wl = exp(-abs(luminance(ip.rgb) - luminance(iq.rgb)) / sigma_l);
            let wt = kernel[k] * kernel[j] * wn * wz * wl;
            sum += iq.rgb * wt;
            wsum += wt;
            vsum += wt * wt * iq.w;
        }
    }
    if (wsum > 1e-8) {
        dst[i] = vec4<f32>(sum / wsum, vsum / (wsum * wsum));
    } else {
        dst[i] = ip;
    }
}

// The viewport's plain background (matches post.wgsl).
fn backdrop(uv: vec2<f32>) -> vec3<f32> {
    return mix(Q.back_top.rgb, Q.back_bottom.rgb, uv.y);
}

fn tonemap(hdr: vec3<f32>) -> vec3<f32> {
    let x = max(hdr, vec3<f32>(0.0)) * Q.display.x;
    if (Q.display.y < 0.5) {
        return min(x, vec3<f32>(1.0));
    }
    let v = x * 0.6;
    return clamp((v * (v * 2.51 + 0.03)) / (v * (v * 2.43 + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let v = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(v, vec3<f32>(1.0 / 2.4)) - 0.055, v * 12.92, v <= vec3<f32>(0.0031308));
}

// Display pixels: the mesh (denoised when `src` holds the filtered radiance) over the backdrop.
@compute @workgroup_size(8, 8)
fn resolve(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = Q.size.x;
    let h = Q.size.y;
    if (id.x >= w || id.y >= h) {
        return;
    }
    let i = id.y * w + id.x;
    let n = f32(max(Q.size.z, 1u));
    let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(f32(w), f32(h));
    let hits = acc_alb[i].w;
    let misses = n - hits;
    var back = backdrop(uv);
    if (misses > 0.0) {
        back = mix(back, tonemap(acc_env[i].rgb / misses), clamp(Q.display.z, 0.0, 1.0));
    }
    var mean = acc_hit[i].rgb / max(hits, 1.0);
    if (Q.display.w > 0.5) {
        mean = src[i].rgb * guide(i).albedo;
    }
    let scene = select(vec3<f32>(0.0), tonemap(mean), hits > 0.0);
    let c = mix(back, scene, hits / n);
    textureStore(out, vec2<i32>(id.xy), vec4<f32>(to_srgb(c), 1.0));
}
