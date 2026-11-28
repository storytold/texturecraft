void main() {
    vec3 n = normalize(tc_vertex_normal);
    if (!gl_FrontFacing) n = -n;
    vec3 t = normalize(tc_tangent.xyz);
    V2F inputs = V2F(tc_position,n,t,cross(n,t)*tc_tangent.w,tc_uv,SparseCoord(tc_uv,dFdx(tc_uv),dFdy(tc_uv)));
    tc_alpha = textureSparse(SamplerSparse(7),tc_uv).r;
    shade(inputs);
    vec3 color = tc_albedo * tc_diffuse + tc_specular + tc_emission;
    if (tc_hover == 1u) color = mix(color,g.set_hover.rgb,g.set_hover.a);
    tc_color = vec4(color * tc_alpha,tc_alpha);
}
