// Published library interface names, bound to TextureCraft's viewport data.
#ifdef TC_LIB_VECTORS
#define camera_pos (g.eye.xyz)
#define camera_dir (g.fwd.xyz)
#define is2DView (g.bg.z > 0.5)
#define is_perspective (length(vec3(g.view_proj[0].w,g.view_proj[1].w,g.view_proj[2].w)) > 0.0)
#define facing 0
#endif
#ifdef TC_LIB_NORMAL
#define height_texture SamplerSparse(5)
#define normal_texture SamplerSparse(4)
#define base_normal_texture SamplerSparse(4)
#define base_normal_y_coeff (g.mode.w)
#endif
#ifdef TC_LIB_PBR
#define maxLod (g.env.y)
#endif
#ifdef TC_LIB_SAMPLER
#define ao_intensity (g.env.z)
#define base_ao_tex SamplerSparse(11)
#define ao_tex SamplerSparse(3)
#endif
#ifdef TC_LIB_ENV
#define environment_texture TCInputTexture(-1)
#endif
#ifdef TC_LIB_EMISSIVE
#define emissive_tex SamplerSparse(6)
#endif
