// Original TextureCraft implementation of the documented surface API.
layout(set=1,binding=0) uniform texture2D tc_base;
layout(set=1,binding=1) uniform texture2D tc_orm;
layout(set=1,binding=2) uniform texture2D tc_normal;
layout(set=1,binding=3) uniform texture2D tc_height;
layout(set=1,binding=4) uniform texture2D tc_emissive;
layout(set=1,binding=8) uniform texture2D tc_maps;
layout(set=1,binding=9) uniform sampler tc_sampler;
layout(set=0,binding=3) uniform texture2D tc_shadow_map;
layout(set=0,binding=4) uniform samplerShadow tc_shadow_sampler;
layout(set=0,binding=6) uniform texture2DArray tc_inputs;
layout(set=0,binding=1) uniform texture2D tc_environment;
layout(set=0,binding=2) uniform sampler tc_env_sampler;
layout(location=0) in vec3 tc_position;
layout(location=1) in vec3 tc_vertex_normal;
layout(location=2) in vec4 tc_tangent;
layout(location=3) in vec2 tc_uv;
layout(location=4) flat in uint tc_hover;
layout(location=0) out vec4 tc_color;

const float M_PI = 3.141592653589793;
const float M_2PI = 6.283185307179586;
const float M_INV_PI = 0.318309886183791;
struct SamplerSparse { int channel; };
struct TCInputTexture { int layer; };
vec4 texture(TCInputTexture input_tex, vec2 uv) {
    if (input_tex.layer < 0) return texture(sampler2D(tc_environment,tc_env_sampler),uv);
    return texture(sampler2DArray(tc_inputs,tc_sampler),vec3(uv,float(input_tex.layer)));
}
vec4 textureLod(TCInputTexture input_tex, vec2 uv, float lod) {
    if (input_tex.layer < 0) return textureLod(sampler2D(tc_environment,tc_env_sampler),uv,lod);
    return textureLod(sampler2DArray(tc_inputs,tc_sampler),vec3(uv,float(input_tex.layer)),0.0);
}
struct SparseCoord { vec2 tex_coord; vec2 dx; vec2 dy; };
struct V2F { vec3 position; vec3 normal; vec3 tangent; vec3 bitangent; vec2 tex_coord; SparseCoord sparse_coord; };
struct LocalVectors { vec3 normal; vec3 tangent; vec3 bitangent; vec3 eye; };
bool isBackFace() { return !gl_FrontFacing; }
mat4 tc_camera_view() {
    return tc_params.camera_view;
}
vec3 tc_diffuse = vec3(0.0);
vec3 tc_specular = vec3(0.0);
vec3 tc_emission = vec3(0.0);
vec3 tc_albedo = vec3(1.0);
float tc_alpha = 1.0;

vec4 textureSparse(SamplerSparse s, SparseCoord coord) {
    vec2 uv = coord.tex_coord;
    if (s.channel >= 16) return texture(TCInputTexture(s.channel-16),uv);
    if (s.channel == 0) { return textureGrad(sampler2D(tc_base, tc_sampler), uv, coord.dx, coord.dy); }
    if (s.channel == 1) { return vec4(textureGrad(sampler2D(tc_orm, tc_sampler), uv, coord.dx, coord.dy).g); }
    if (s.channel == 2) { return vec4(textureGrad(sampler2D(tc_orm, tc_sampler), uv, coord.dx, coord.dy).b); }
    if (s.channel == 3) { return vec4(textureGrad(sampler2D(tc_orm, tc_sampler), uv, coord.dx, coord.dy).r); }
    if (s.channel == 4) { return textureGrad(sampler2D(tc_normal, tc_sampler), uv, coord.dx, coord.dy); }
    if (s.channel == 5) { return textureGrad(sampler2D(tc_height, tc_sampler), uv, coord.dx, coord.dy); }
    if (s.channel == 6) { return textureGrad(sampler2D(tc_emissive, tc_sampler), uv, coord.dx, coord.dy); }
    if (s.channel == 7) { return vec4(textureGrad(sampler2D(tc_base, tc_sampler), uv, coord.dx, coord.dy).a); }
    if (s.channel == 8) { return vec4(textureGrad(sampler2D(tc_orm, tc_sampler), uv, coord.dx, coord.dy).a); }
    if (s.channel == 9) { return vec4(textureGrad(sampler2D(tc_maps, tc_sampler), uv, coord.dx, coord.dy).r); }
    if (s.channel == 10) { return vec4(textureGrad(sampler2D(tc_maps, tc_sampler), uv, coord.dx, coord.dy).g); }
    if (s.channel == 11) { return vec4(textureGrad(sampler2D(tc_maps, tc_sampler), uv, coord.dx, coord.dy).b); }
    return vec4(0.0);
}
vec4 textureSparse(SamplerSparse s, vec2 uv) { return textureSparse(s, SparseCoord(uv, dFdx(uv), dFdy(uv))); }
vec3 getBaseColor(vec4 value) { return value.rgb; }
vec3 getBaseColor(SamplerSparse s,SparseCoord c) { return textureSparse(s,c).rgb; }
float getRoughness(vec4 value) { return value.r; }
float getRoughness(SamplerSparse s,SparseCoord c) { return textureSparse(s,c).r; }
float getMetallic(vec4 value) { return value.r; }
float getMetallic(SamplerSparse s,SparseCoord c) { return textureSparse(s,c).r; }
float getAO(vec4 value) { return value.r; }
float getOpacity(vec4 value) { return value.r; }
float getOpacity(SamplerSparse s,SparseCoord c) { return textureSparse(s,c).r; }
float getHeight(vec4 value) { return value.r; }
float getHeight(SamplerSparse s,SparseCoord c) { return textureSparse(s,c).r; }
float getSpecularLevel(vec4 value) { return value.r; }
float getSpecularLevel(SamplerSparse s,SparseCoord c) { return textureSparse(s,c).r; }
float getGlossiness(vec4 value) { return value.r; }
float getGlossiness(SamplerSparse s,SparseCoord c) { return textureSparse(s,c).r; }
vec3 getDiffuse(vec4 value) { return value.rgb; }
vec3 getDiffuse(SamplerSparse s,SparseCoord c) { return textureSparse(s,c).rgb; }
vec3 getSpecularColor(vec4 value) { return value.rgb; }
vec3 getSpecularColor(SamplerSparse s,SparseCoord c) { return textureSparse(s,c).rgb; }
vec3 getEmissive(vec4 value) { return value.rgb; }
vec3 pbrComputeEmissive(SamplerSparse s,SparseCoord c) { return textureSparse(s,c).rgb; }
vec3 getSpecularColor(vec3 base, float metal, float specular) { return mix(vec3(0.08 * specular), base, metal); }
vec3 getDiffuseColor(vec3 base, float metal) { return base * (1.0 - metal); }
vec3 generateDiffuseColor(vec3 base,float metal) { return base*(1.0-metal); }
vec3 generateSpecularColor(float specular,vec3 base,float metal) { return mix(vec3(0.08*specular),base,metal); }
vec3 generateSpecularColor(vec3 base,float metal) { return mix(vec3(0.04),base,metal); }
float specularOcclusionCorrection(float diffuse_occlusion,float metallic,float roughness) {
    // Narrow reflections see less of a diffuse visibility cone; rough metallic reflections approach diffuse occlusion.
    return mix(1.0,clamp(diffuse_occlusion,0.0,1.0),clamp(roughness,0.0,1.0)*mix(0.5,1.0,clamp(metallic,0.0,1.0)));
}
vec2 tc_env_uv(vec3 direction) {
    float c=cos(g.env.x),s=sin(g.env.x);
    vec3 d=vec3(c*direction.x-s*direction.z,direction.y,s*direction.x+c*direction.z);
    return vec2(fract(atan(d.z,d.x)/M_2PI+1.0),acos(clamp(d.y,-1.0,1.0))/M_PI);
}
vec3 tc_irradiance(vec3 n) {
    float c=cos(g.env.x),s=sin(g.env.x);
    n=vec3(c*n.x-s*n.z,n.y,s*n.x+c*n.z);
    float x=n.x,y=n.y,z=n.z;
    vec3 result=g.sh[0].xyz*0.282095;
    result+=(g.sh[1].xyz*y+g.sh[2].xyz*z+g.sh[3].xyz*x)*0.488603;
    result+=(g.sh[4].xyz*x*y+g.sh[5].xyz*y*z+g.sh[7].xyz*x*z)*1.092548;
    result+=g.sh[6].xyz*(3.0*z*z-1.0)*0.315392+g.sh[8].xyz*(x*x-y*y)*0.546274;
    return max(result,vec3(0.0));
}
vec3 pbrComputeDiffuse(LocalVectors frame,vec3 color) { return color*tc_irradiance(frame.normal); }
vec3 pbrComputeDiffuse(LocalVectors frame,vec3 color,float occlusion) { return pbrComputeDiffuse(frame,color)*occlusion; }
vec3 pbrComputeSpecular(LocalVectors frame,vec3 color,float roughness,float occlusion,float bentNormalSpecularAmount) {
    // TextureCraft's split-sum environment approximation; bent normals are unavailable.
    vec3 reflected=reflect(-frame.eye,frame.normal);
    vec3 light=textureLod(sampler2D(tc_environment,tc_env_sampler),tc_env_uv(reflected),clamp(roughness,0.0,1.0)*g.env.y).rgb;
    float nv=max(dot(frame.normal,frame.eye),0.0);
    vec4 fit=clamp(roughness,0.0,1.0)*vec4(-1.0,-0.0275,-0.572,0.022)+vec4(1.0,0.0425,1.04,-0.04);
    float a004=min(fit.x*fit.x,exp2(-9.28*nv))*fit.x+fit.y;
    vec2 brdf=vec2(-1.04,1.04)*a004+fit.zw;
    return light*(color*brdf.x+brdf.y)*occlusion;
}
vec3 pbrComputeSpecular(LocalVectors frame,vec3 color,float roughness) { return pbrComputeSpecular(frame,color,roughness,1.0,0.0); }
vec3 pbrComputeSpecular(LocalVectors frame,vec3 f0,vec3 f82,float roughness,float occlusion,float bentNormalSpecularAmount) {
    float grazing=pow(1.0-max(dot(frame.normal,frame.eye),0.0),5.0);
    return pbrComputeSpecular(frame,mix(f0,f82,grazing),roughness,occlusion,bentNormalSpecularAmount);
}
vec3 pbrComputeSpecular(LocalVectors frame,vec3 f0,vec3 f82,float roughness) { return pbrComputeSpecular(frame,f0,f82,roughness,1.0,0.0); }
vec3 normalUnpack(vec4 value,float y_coeff) {
    if (value.a == 0.0) return vec3(0.0,0.0,1.0);
    return normalize((value.xyz*2.0-1.0)*vec3(1.0,y_coeff,1.0));
}
vec3 normalUnpack(vec4 value) { return normalUnpack(value,g.mode.w); }
vec3 normalBlend(vec3 base,vec3 over) { return normalize(vec3(base.xy+over.xy,base.z*over.z)); }
vec3 normalBlendOriented(vec3 base,vec3 over) {
    vec3 t=base+vec3(0.0,0.0,1.0);vec3 u=over*vec3(-1.0,-1.0,1.0);
    return normalize(t*dot(t,u)/max(t.z,0.00001)-u);
}
vec3 normalFade(vec3 normal,float attenuation) { return normalize(mix(vec3(0.0,0.0,1.0),normal,attenuation)); }
vec3 normalFromHeight(SparseCoord coord,float force) {
    vec2 step_uv=1.0/vec2(textureSize(sampler2D(tc_height,tc_sampler),0));
    float left=texture(sampler2D(tc_height,tc_sampler),coord.tex_coord-vec2(step_uv.x,0.0)).r;
    float right=texture(sampler2D(tc_height,tc_sampler),coord.tex_coord+vec2(step_uv.x,0.0)).r;
    float above=texture(sampler2D(tc_height,tc_sampler),coord.tex_coord-vec2(0.0,step_uv.y)).r;
    float below=texture(sampler2D(tc_height,tc_sampler),coord.tex_coord+vec2(0.0,step_uv.y)).r;
    return normalize(vec3((left-right)*force,(below-above)*force,1.0));
}
vec3 getTSNormal(SparseCoord c) { return normalize(texture(sampler2D(tc_normal,tc_sampler),c.tex_coord).xyz*vec3(1.0,g.mode.w,1.0)); }
vec3 getTSNormal(SparseCoord c,SamplerSparse s) { return s.channel==4 ? getTSNormal(c) : normalUnpack(textureSparse(s,c)); }
vec3 getTSNormal(SparseCoord c,vec3 height_normal) { return normalBlendOriented(height_normal,getTSNormal(c)); }
vec3 getTSNormal(SparseCoord c,SamplerSparse s,vec3 height_normal) { return normalBlendOriented(height_normal,getTSNormal(c,s)); }
vec3 computeWSBaseNormal(SparseCoord c,vec3 t,vec3 b,vec3 n) { return normalize(mat3(t,b,n)*getTSNormal(c)); }
vec3 computeWSNormal(SparseCoord c,vec3 t,vec3 b,vec3 n) { return normalize(mat3(t,b,n)*getTSNormal(c,normalFromHeight(c,g.mode.z))); }
vec3 computeNormal(V2F i) {
    vec3 n = normalize(i.normal);
    vec3 t = normalize(i.tangent - n * dot(i.tangent,n));
    vec3 b = normalize(i.bitangent);
    vec3 map = texture(sampler2D(tc_normal,tc_sampler),i.tex_coord).xyz;
    map.y *= g.mode.w;
    return normalize(t * map.x + b * map.y + n * map.z);
}
LocalVectors computeLocalFrame(V2F i) { return LocalVectors(computeNormal(i),normalize(i.tangent),normalize(i.bitangent),normalize(g.eye.xyz-i.position)); }
LocalVectors computeLocalFrame(V2F i, vec3 n) { return LocalVectors(n,normalize(i.tangent),normalize(i.bitangent),normalize(g.eye.xyz-i.position)); }
LocalVectors computeLocalFrame(V2F i,vec3 n,float angle) { float c=cos(angle);float s=sin(angle);return LocalVectors(n,normalize(i.tangent*c+i.bitangent*s),normalize(i.bitangent*c-i.tangent*s),normalize(g.eye.xyz-i.position)); }
vec3 tangentSpaceToWorldSpace(vec3 v,V2F i) { return mat3(i.tangent,i.bitangent,i.normal)*v; }
vec3 worldSpaceToTangentSpace(vec3 v,V2F i) { return transpose(mat3(i.tangent,i.bitangent,i.normal))*v; }
vec3 getEyeVec(vec3 position) { return normalize(g.eye.xyz-position); }
vec3 getEyeVec(V2F i) { return getEyeVec(i.position); }
float getAO(SparseCoord coord) { return textureSparse(SamplerSparse(3),coord).r; }
float getShadowFactor(vec3 position) {
    if (g.shadow.x < 0.5) return 1.0;
    vec4 clip = g.light_vp * vec4(position,1.0);
    vec3 projected = clip.xyz / clip.w;
    vec2 uv = projected.xy * vec2(0.5,-0.5) + 0.5;
    if (projected.z < 0.0 || projected.z > 1.0 || any(lessThan(uv,vec2(0.0))) || any(greaterThan(uv,vec2(1.0)))) return 1.0;
    float visible = textureLod(sampler2DShadow(tc_shadow_map,tc_shadow_sampler),vec3(uv,projected.z),0.0);
    return mix(1.0,visible,g.shadow.w);
}
float getShadowFactor() { return getShadowFactor(tc_position); }
float getOpacity(SparseCoord coord) { return textureSparse(SamplerSparse(7),coord).r; }
vec3 getEmissive(SparseCoord coord) { return textureSparse(SamplerSparse(6),coord).rgb; }
void diffuseShadingOutput(vec3 color) { tc_diffuse = color; }
void specularShadingOutput(vec3 color) { tc_specular = color; }
void emissiveColorOutput(vec3 color) { tc_emission = color; }
void albedoOutput(vec3 color) { tc_albedo = color; }
void alphaOutput(float alpha) { tc_alpha = alpha; }
void alphaKill(float alpha) { if (alpha < 0.5) discard; }
vec3 linear_to_srgb(vec3 color) { return mix(12.92*color,1.055*pow(max(color,vec3(0.0)),vec3(1.0/2.4))-0.055,step(vec3(0.0031308),color)); }
vec3 srgb_to_linear(vec3 color) { return mix(color/12.92,pow((color+0.055)/1.055,vec3(2.4)),step(vec3(0.04045),color)); }
