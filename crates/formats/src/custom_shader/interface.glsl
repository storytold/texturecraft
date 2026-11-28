#version 450
// TextureCraft GPU interface, original work derived from our Globals layout.
layout(set=0,binding=0,std140) uniform TCGlobals {
    mat4 view_proj;
    mat4 inv_view_proj;
    vec4 eye;
    vec4 env;
    vec4 sh[9];
    vec4 mode;
    vec4 bg;
    vec4 uv_xform;
    vec4 light;
    mat4 light_vp;
    vec4 shadow;
    vec4 material;
    vec4 scatter;
    vec4 fwd;
    vec4 disp;
    vec4 refr;
    vec4 highlight;
    vec4 highlight_shift;
    vec4 set_hover;
    vec4 bake_high;
    vec4 bake_matching_error;
    vec4 bake_cage;
    vec4 bake_cage_wire;
    vec4 bake_missing_seam;
    vec4 bake_neutral_color;
    vec4 bake_neutral;
    vec4 bake_bent_normal;
    vec4 uv_rotation;
} g;
layout(set=0,binding=5,std140) uniform TCParameters { vec4 values[128]; } tc_params;
