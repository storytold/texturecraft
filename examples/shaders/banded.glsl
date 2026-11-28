// Original TextureCraft sample, MIT OR Apache-2.0.
import lib-sparse.glsl
import lib-normal.glsl
//: param auto channel_basecolor
uniform SamplerSparse base_tex;
//: param custom { "default": 3, "label": "Light bands", "group": "Lighting", "min": 2, "max": 8 }
uniform int bands;
//: param custom { "default": [0.25, 0.3, 0.45], "label": "Shadow tint", "group": "Lighting", "widget": "color", "min": 0, "max": 1 }
uniform vec3 shadow_tint;
//: param custom { "default": true, "label": "Use normal map", "group": "Lighting" }
uniform bool use_normal;
//: param custom { "default": "", "default_color": [1, 1, 1, 1], "label": "Tint texture", "group": "Lighting" }
uniform sampler2D tint_texture;
void shade(V2F inputs) {
    vec3 normal = use_normal ? computeNormal(inputs) : normalize(inputs.normal);
    vec3 light = normalize(vec3(0.5, 0.8, 0.4));
    float lambert = max(dot(normal, light), 0.0);
    float band = floor(lambert * float(bands)) / float(bands - 1);
    vec3 base = textureSparse(base_tex, inputs.sparse_coord).rgb * texture(tint_texture, inputs.tex_coord).rgb;
    diffuseShadingOutput(base * mix(shadow_tint, vec3(1.0), min(band, 1.0)));
}
