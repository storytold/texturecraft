//! The reference app's static output templates, in its order (alphabetical).

use texturecraft_core::{BakedMap, Channel};

use super::{baked, ch, output, virt};
use crate::export::{Dest, ExportPreset, FileFormat, SourceRef, VirtualMap};

pub(super) fn static_templates() -> Vec<ExportPreset> {
    let preset = |name: &str, maps| ExportPreset { name: name.into(), maps };
    vec![
        preset(
            "Amazon Lumberyard",
            vec![
                output("$mesh_$textureSet_diff(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2)), (Dest::A, ch(Channel::Opacity, 0))]),
                output("$mesh_$textureSet_spec(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Specular, 0)), (Dest::G, ch(Channel::Specular, 1)), (Dest::B, ch(Channel::Specular, 2))]),
                output("$mesh_$textureSet_ddna(_$colorSpace)(.$udim)", FileFormat::Png, 8, true, vec![(Dest::R, virt(VirtualMap::NormalDirectX, 0)), (Dest::G, virt(VirtualMap::NormalDirectX, 1)), (Dest::B, virt(VirtualMap::NormalDirectX, 2)), (Dest::A, ch(Channel::Glossiness, 0))]),
            ],
        ),
        preset(
            "Arnold (AiStandard)",
            vec![
                output("$mesh_$textureSet_BaseColor(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2))]),
                output("$mesh_$textureSet_Metalness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Metallic, 0))]),
                output("$mesh_$textureSet_Roughness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Roughness, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2)), (Dest::A, virt(VirtualMap::UvCoverage, 0))]),
                output("$mesh_$textureSet_Height(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::Height, 0))]),
                output("$mesh_$textureSet_Emissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2)), (Dest::A, virt(VirtualMap::UvCoverage, 0))]),
            ],
        ),
        preset(
            "Arnold UDIM Legacy (AiStandard)",
            vec![
                output("$mesh_BaseColor(_$colorSpace).$textureSet", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2))]),
                output("$mesh_Metalness(_$colorSpace).$textureSet", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Metallic, 0))]),
                output("$mesh_Roughness(_$colorSpace).$textureSet", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Roughness, 0))]),
                output("$mesh_Normal(_$colorSpace).$textureSet", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2)), (Dest::A, virt(VirtualMap::UvCoverage, 0))]),
                output("$mesh_Height(_$colorSpace).$textureSet", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::Height, 0))]),
                output("$mesh_Emissive(_$colorSpace).$textureSet", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2)), (Dest::A, virt(VirtualMap::UvCoverage, 0))]),
            ],
        ),
        preset(
            "Blender (Principled BSDF)",
            vec![
                output("$textureSet_BaseColor(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2))]),
                output("$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$textureSet_Metallic(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Metallic, 0)), (Dest::G, ch(Channel::Metallic, 0)), (Dest::B, ch(Channel::Metallic, 0))]),
                output("$textureSet_Roughness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Roughness, 0)), (Dest::G, ch(Channel::Roughness, 0)), (Dest::B, ch(Channel::Roughness, 0))]),
                output("$textureSet_Emission(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
                output("$textureSet_Alpha(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Opacity, 0)), (Dest::G, ch(Channel::Opacity, 0)), (Dest::B, ch(Channel::Opacity, 0))]),
                output("$textureSet_Displacement(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Height, 0)), (Dest::G, ch(Channel::Height, 0)), (Dest::B, ch(Channel::Height, 0))]),
            ],
        ),
        preset(
            "Corona",
            vec![
                output("$mesh_$textureSet_Diffuse(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, virt(VirtualMap::Diffuse, 0)), (Dest::G, virt(VirtualMap::Diffuse, 1)), (Dest::B, virt(VirtualMap::Diffuse, 2))]),
                output("$mesh_$textureSet_Reflection(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, virt(VirtualMap::Reflection, 0)), (Dest::G, virt(VirtualMap::Reflection, 1)), (Dest::B, virt(VirtualMap::Reflection, 2))]),
                output("$mesh_$textureSet_Glossiness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, virt(VirtualMap::Glossiness2, 0))]),
                output("$mesh_$textureSet_IOR(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, virt(VirtualMap::InverseIor, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalDirectX, 0)), (Dest::G, virt(VirtualMap::NormalDirectX, 1)), (Dest::B, virt(VirtualMap::NormalDirectX, 2)), (Dest::A, virt(VirtualMap::UvCoverage, 0))]),
                output("$mesh_$textureSet_Height(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::Height, 0))]),
                output("$mesh_$textureSet_Emissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2)), (Dest::A, virt(VirtualMap::UvCoverage, 0))]),
            ],
        ),
        preset(
            "CryEngine",
            vec![
                output("$mesh_$textureSet_diff(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2)), (Dest::A, ch(Channel::Opacity, 0))]),
                output("$mesh_$textureSet_spec(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Specular, 0)), (Dest::G, ch(Channel::Specular, 1)), (Dest::B, ch(Channel::Specular, 2))]),
                output("$mesh_$textureSet_ddna(_$colorSpace)(.$udim)", FileFormat::Png, 8, true, vec![(Dest::R, virt(VirtualMap::NormalDirectX, 0)), (Dest::G, virt(VirtualMap::NormalDirectX, 1)), (Dest::B, virt(VirtualMap::NormalDirectX, 2)), (Dest::A, ch(Channel::Glossiness, 0))]),
            ],
        ),
        preset(
            "Dota 2",
            vec![
                output("$mesh_$textureSet_color(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2))]),
                output("$mesh_$textureSet_transparency(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Opacity, 0))]),
                output("$mesh_$textureSet_normal(_$colorSpace)(.$udim)", FileFormat::Png, 8, true, vec![(Dest::R, virt(VirtualMap::NormalDirectX, 0)), (Dest::G, virt(VirtualMap::NormalDirectX, 1)), (Dest::B, virt(VirtualMap::NormalDirectX, 2))]),
                output("$mesh_$textureSet_detailmask(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, SourceRef::Constant(0.0))]),
                output("$mesh_$textureSet_diffusemask(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, SourceRef::Constant(0.0))]),
                output("$mesh_$textureSet_metalnessmask(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Metallic, 0))]),
                output("$mesh_$textureSet_selfillummask(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Emissive, 0))]),
                output("$mesh_$textureSet_specmask(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Specular, 0))]),
                output("$mesh_$textureSet_rimmask(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, SourceRef::Constant(0.0))]),
                output("$mesh_$textureSet_basetintmask(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, SourceRef::Constant(0.0))]),
                output("$mesh_$textureSet_specexp(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Glossiness, 0))]),
            ],
        ),
        preset(
            "KeyShot",
            vec![
                output("$mesh_$textureSet_Diffuse(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, virt(VirtualMap::Diffuse, 0)), (Dest::G, virt(VirtualMap::Diffuse, 1)), (Dest::B, virt(VirtualMap::Diffuse, 2))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalDirectX, 0)), (Dest::G, virt(VirtualMap::NormalDirectX, 1)), (Dest::B, virt(VirtualMap::NormalDirectX, 2))]),
                output("$mesh_$textureSet_Reflection(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, virt(VirtualMap::Reflection, 0)), (Dest::G, virt(VirtualMap::Reflection, 1)), (Dest::B, virt(VirtualMap::Reflection, 2))]),
                output("$mesh_$textureSet_Roughness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Roughness, 0))]),
                output("$mesh_$textureSet_Metallic(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Metallic, 0))]),
            ],
        ),
        preset(
            "KeyShot 9+",
            vec![
                output("$project.sp_$textureSet.ts_base_color(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2))]),
                output("$project.sp_$textureSet.ts_metallic(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Metallic, 0))]),
                output("$project.sp_$textureSet.ts_roughness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Roughness, 0))]),
                output("$project.sp_$textureSet.ts_specular(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::SpecularLevel, 0))]),
                output("$project.sp_$textureSet.ts_anisotropic(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::AnisotropyLevel, 0))]),
                output("$project.sp_$textureSet.ts_anisotropic_angle(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::AnisotropyAngle, 0))]),
                output("$project.sp_$textureSet.ts_normal(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$project.sp_$textureSet.ts_opacity(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Opacity, 0))]),
                output("$project.sp_$textureSet.ts_height(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::Height, 0))]),
                output("$project.sp_$textureSet.ts_emissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
            ],
        ),
        preset(
            "Lens Studio",
            vec![
                output("$mesh_$textureSet_base(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2)), (Dest::A, ch(Channel::Opacity, 0))]),
                output("$mesh_$textureSet_normal(_$colorSpace)(.$udim)", FileFormat::Png, 8, true, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$mesh_$textureSet_materialParams(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Metallic, 0)), (Dest::G, ch(Channel::Roughness, 0)), (Dest::B, virt(VirtualMap::AoMixed, 0))]),
            ],
        ),
        preset(
            "Maxwell (Metallic Roughness)",
            vec![
                output("$mesh_$textureSet_BaseColor(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2))]),
                output("$mesh_$textureSet_Roughness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Roughness, 0))]),
                output("$mesh_$textureSet_Metallic(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Metallic, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$mesh_$textureSet_Height(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::Height, 0))]),
                output("$mesh_$textureSet_AnisoLevel(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::AnisotropyLevel, 0))]),
                output("$mesh_$textureSet_AnisoAngle(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::AnisotropyAngle, 0))]),
                output("$mesh_$textureSet_AO(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, virt(VirtualMap::AoMixed, 0))]),
                output("$mesh_$textureSet_Opacity(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Opacity, 0))]),
                output("$mesh_$textureSet_SSS(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::ScatterWeight, 0))]),
                output("$mesh_$textureSet_Emission(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
            ],
        ),
        preset(
            "Maxwell (Specular Glossiness)",
            vec![
                output("$mesh_$textureSet_Diffuse(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2))]),
                output("$mesh_$textureSet_Specular(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Specular, 0)), (Dest::G, ch(Channel::Specular, 1)), (Dest::B, ch(Channel::Specular, 2))]),
                output("$mesh_$textureSet_Glossiness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Glossiness, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$mesh_$textureSet_Height(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::Height, 0))]),
                output("$mesh_$textureSet_AnisoLevel(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::AnisotropyLevel, 0))]),
                output("$mesh_$textureSet_AnisoAngle(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::AnisotropyAngle, 0))]),
                output("$mesh_$textureSet_AO(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, virt(VirtualMap::AoMixed, 0))]),
                output("$mesh_$textureSet_Opacity(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Opacity, 0))]),
                output("$mesh_$textureSet_SSS(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::ScatterWeight, 0))]),
                output("$mesh_$textureSet_Emission(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
            ],
        ),
        preset(
            "Mesh Maps",
            vec![
                output("$textureSet_normal_base(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, baked(BakedMap::Normal, 0)), (Dest::G, baked(BakedMap::Normal, 1)), (Dest::B, baked(BakedMap::Normal, 2))]),
                output("$textureSet_world_space_normals(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, baked(BakedMap::WorldNormal, 0)), (Dest::G, baked(BakedMap::WorldNormal, 1)), (Dest::B, baked(BakedMap::WorldNormal, 2))]),
                output("$textureSet_id(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, baked(BakedMap::Id, 0)), (Dest::G, baked(BakedMap::Id, 1)), (Dest::B, baked(BakedMap::Id, 2))]),
                output("$textureSet_position(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, baked(BakedMap::Position, 0)), (Dest::G, baked(BakedMap::Position, 1)), (Dest::B, baked(BakedMap::Position, 2))]),
                output("$textureSet_bent_normals_base(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, baked(BakedMap::BentNormal, 0)), (Dest::G, baked(BakedMap::BentNormal, 1)), (Dest::B, baked(BakedMap::BentNormal, 2))]),
                output("$textureSet_ambient_occlusion(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, baked(BakedMap::AmbientOcclusion, 0))]),
                output("$textureSet_curvature(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, baked(BakedMap::Curvature, 0))]),
                output("$textureSet_thickness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, baked(BakedMap::Thickness, 0))]),
                output("$textureSet_height_base(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, baked(BakedMap::Height, 0))]),
                output("$textureSet_opacity_base(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, baked(BakedMap::Opacity, 0))]),
            ],
        ),
        preset(
            "Non-PBR Specular Glossiness",
            vec![
                output("$mesh_$textureSet_Diffuse(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2)), (Dest::A, ch(Channel::Opacity, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 8, true, vec![(Dest::R, virt(VirtualMap::NormalDirectX, 0)), (Dest::G, virt(VirtualMap::NormalDirectX, 1)), (Dest::B, virt(VirtualMap::NormalDirectX, 2))]),
                output("$mesh_$textureSet_SpecularGlossiness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Specular, 0)), (Dest::G, ch(Channel::Specular, 1)), (Dest::B, ch(Channel::Specular, 2)), (Dest::A, ch(Channel::Glossiness, 0))]),
                output("$mesh_$textureSet_Emissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
            ],
        ),
        preset(
            "PBR Metallic Roughness",
            vec![
                output("$mesh_$textureSet_BaseColor(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2)), (Dest::A, ch(Channel::Opacity, 0))]),
                output("$mesh_$textureSet_Roughness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Roughness, 0))]),
                output("$mesh_$textureSet_Metallic(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Metallic, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalDirectX, 0)), (Dest::G, virt(VirtualMap::NormalDirectX, 1)), (Dest::B, virt(VirtualMap::NormalDirectX, 2))]),
                output("$mesh_$textureSet_Height(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::Height, 0))]),
                output("$mesh_$textureSet_Emissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
            ],
        ),
        preset(
            "PBR Metallic Roughness (folders or PSD groups)",
            vec![
                output("BaseColor_$textureSet_BaseColor(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2))]),
                output("Roughness_$textureSet_Roughness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Roughness, 0)), (Dest::G, ch(Channel::Roughness, 0)), (Dest::B, ch(Channel::Roughness, 0))]),
                output("Metallic_$textureSet_Metallic(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Metallic, 0)), (Dest::G, ch(Channel::Metallic, 0)), (Dest::B, ch(Channel::Metallic, 0))]),
                output("Normal_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalDirectX, 0)), (Dest::G, virt(VirtualMap::NormalDirectX, 1)), (Dest::B, virt(VirtualMap::NormalDirectX, 2))]),
                output("Height_$textureSet_Height(_$colorSpace)(_$colorSpace)(_$colorSpace)(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, ch(Channel::Height, 0)), (Dest::G, ch(Channel::Height, 0)), (Dest::B, ch(Channel::Height, 0))]),
                output("Emissive_$textureSet_Emissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
            ],
        ),
        preset(
            "PBR Specular Glossiness",
            vec![
                output("$mesh_$textureSet_Diffuse(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2)), (Dest::A, ch(Channel::Opacity, 0))]),
                output("$mesh_$textureSet_Specular(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Specular, 0)), (Dest::G, ch(Channel::Specular, 1)), (Dest::B, ch(Channel::Specular, 2))]),
                output("$mesh_$textureSet_Glossiness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Glossiness, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalDirectX, 0)), (Dest::G, virt(VirtualMap::NormalDirectX, 1)), (Dest::B, virt(VirtualMap::NormalDirectX, 2))]),
                output("$mesh_$textureSet_Height(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::Height, 0))]),
                output("$mesh_$textureSet_Emissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
            ],
        ),
        preset(
            "PBR Specular Glossiness (converted from Metallic Roughness)",
            vec![
                output("$mesh_$textureSet_Diffuse(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, virt(VirtualMap::Diffuse, 0)), (Dest::G, virt(VirtualMap::Diffuse, 1)), (Dest::B, virt(VirtualMap::Diffuse, 2)), (Dest::A, ch(Channel::Opacity, 0))]),
                output("$mesh_$textureSet_Specular(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, virt(VirtualMap::SpecularColor, 0)), (Dest::G, virt(VirtualMap::SpecularColor, 1)), (Dest::B, virt(VirtualMap::SpecularColor, 2))]),
                output("$mesh_$textureSet_Glossiness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, virt(VirtualMap::Glossiness, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalDirectX, 0)), (Dest::G, virt(VirtualMap::NormalDirectX, 1)), (Dest::B, virt(VirtualMap::NormalDirectX, 2))]),
                output("$mesh_$textureSet_Height(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::Height, 0))]),
                output("$mesh_$textureSet_Emissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
            ],
        ),
        preset(
            "Redshift (rsMaterial)",
            vec![
                output("$mesh_$textureSet_Color(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2)), (Dest::A, ch(Channel::Opacity, 0))]),
                output("$mesh_$textureSet_Roughness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Roughness, 0))]),
                output("$mesh_$textureSet_Metalness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Metallic, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$mesh_$textureSet_DisplaceHeightField(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::Height, 0))]),
                output("$mesh_$textureSet_EmissionColor(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
            ],
        ),
        preset(
            "Renderman (pxrDisney)",
            vec![
                output("$mesh_$textureSet_BaseColor(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2))]),
                output("$mesh_$textureSet_Roughness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Roughness, 0))]),
                output("$mesh_$textureSet_Metallic(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Metallic, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$mesh_$textureSet_Displacement(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::Height, 0))]),
                output("$mesh_$textureSet_Emissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
                output("$mesh_$textureSet_Presence(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Opacity, 0))]),
            ],
        ),
        preset(
            "Renderman (pxrSurface)",
            vec![
                output("$mesh_$textureSet_DiffuseColor(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, virt(VirtualMap::Diffuse, 0)), (Dest::G, virt(VirtualMap::Diffuse, 1)), (Dest::B, virt(VirtualMap::Diffuse, 2))]),
                output("$mesh_$textureSet_SpecularFaceColor(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, virt(VirtualMap::SpecularColor, 0)), (Dest::G, virt(VirtualMap::SpecularColor, 1)), (Dest::B, virt(VirtualMap::SpecularColor, 2))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$mesh_$textureSet_SpecularRoughness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Roughness, 0))]),
                output("$mesh_$textureSet_GlowColor(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
                output("$mesh_$textureSet_Presence(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Opacity, 0))]),
                output("$mesh_$textureSet_Displacement(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::Height, 0))]),
            ],
        ),
        preset(
            "Roblox (MaterialVariant)",
            vec![
                output("$textureSet_ALB", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2))]),
                output("$textureSet_NRM", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$textureSet_MET", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Metallic, 0)), (Dest::G, ch(Channel::Metallic, 0)), (Dest::B, ch(Channel::Metallic, 0))]),
                output("$textureSet_RGH", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Roughness, 0)), (Dest::G, ch(Channel::Roughness, 0)), (Dest::B, ch(Channel::Roughness, 0))]),
            ],
        ),
        preset(
            "Roblox (SurfaceAppearance)",
            vec![
                output("$textureSet_ColorMap", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2)), (Dest::A, ch(Channel::Opacity, 0))]),
                output("$textureSet_NormalMap", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$textureSet_MetalnessMap", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Metallic, 0)), (Dest::G, ch(Channel::Metallic, 0)), (Dest::B, ch(Channel::Metallic, 0))]),
                output("$textureSet_RoughnessMap", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Roughness, 0)), (Dest::G, ch(Channel::Roughness, 0)), (Dest::B, ch(Channel::Roughness, 0))]),
            ],
        ),
        preset(
            "Shade 3D",
            vec![
                output("$project_$textureSet_Diffuse(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, virt(VirtualMap::LegacyDiffuse, 0)), (Dest::G, virt(VirtualMap::LegacyDiffuse, 1)), (Dest::B, virt(VirtualMap::LegacyDiffuse, 2))]),
                output("$project_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$project_$textureSet_Roughness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Roughness, 0))]),
                output("$project_$textureSet_Specular(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, virt(VirtualMap::SpecularColor, 0))]),
            ],
        ),
        preset(
            "Spark AR Studio",
            vec![
                output("$textureSet_Color(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2)), (Dest::A, ch(Channel::Opacity, 0))]),
                output("$textureSet_ORM(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, virt(VirtualMap::AoMixed, 0)), (Dest::G, ch(Channel::Roughness, 0)), (Dest::B, ch(Channel::Metallic, 0))]),
                output("$textureSet_Emissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
                output("$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 8, true, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
            ],
        ),
        preset(
            "Unity HD Render Pipeline (Metallic Standard)",
            vec![
                output("$mesh_$textureSet_BaseMap(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2)), (Dest::A, ch(Channel::Opacity, 0))]),
                output("$mesh_$textureSet_MaskMap(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Metallic, 0)), (Dest::G, virt(VirtualMap::AoMixed, 0)), (Dest::B, SourceRef::Constant(0.0)), (Dest::A, virt(VirtualMap::Glossiness, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 8, true, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$mesh_$textureSet_Emissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
            ],
        ),
        preset(
            "Unity HD Render Pipeline (Specular)",
            vec![
                output("$mesh_$textureSet_BaseMap(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, virt(VirtualMap::Diffuse, 0)), (Dest::G, virt(VirtualMap::Diffuse, 1)), (Dest::B, virt(VirtualMap::Diffuse, 2)), (Dest::A, ch(Channel::Opacity, 0))]),
                output("$mesh_$textureSet_Specular(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, virt(VirtualMap::SpecularColor, 0)), (Dest::G, virt(VirtualMap::SpecularColor, 1)), (Dest::B, virt(VirtualMap::SpecularColor, 2))]),
                output("$mesh_$textureSet_MaskMap(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Metallic, 0)), (Dest::G, virt(VirtualMap::AoMixed, 0)), (Dest::B, SourceRef::Constant(0.0)), (Dest::A, virt(VirtualMap::Glossiness, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 8, true, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$mesh_$textureSet_Emissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
            ],
        ),
        preset(
            "Unity Universal Render Pipeline (Metallic Standard)",
            vec![
                output("$mesh_$textureSet_AlbedoTransparency(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2)), (Dest::A, ch(Channel::Opacity, 0))]),
                output("$mesh_$textureSet_MetallicSmoothness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Metallic, 0)), (Dest::G, ch(Channel::Metallic, 0)), (Dest::B, ch(Channel::Metallic, 0)), (Dest::A, virt(VirtualMap::Glossiness, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 8, true, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$mesh_$textureSet_Emission(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
            ],
        ),
        preset(
            "Unity Universal Render Pipeline (Specular)",
            vec![
                output("$mesh_$textureSet_AlbedoTransparency(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, virt(VirtualMap::Diffuse, 0)), (Dest::G, virt(VirtualMap::Diffuse, 1)), (Dest::B, virt(VirtualMap::Diffuse, 2)), (Dest::A, ch(Channel::Opacity, 0))]),
                output("$mesh_$textureSet_SpecularSmoothness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, virt(VirtualMap::SpecularColor, 0)), (Dest::G, virt(VirtualMap::SpecularColor, 1)), (Dest::B, virt(VirtualMap::SpecularColor, 2)), (Dest::A, virt(VirtualMap::Glossiness, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 8, true, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$mesh_$textureSet_Emission(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
            ],
        ),
        preset(
            "Unreal Engine (Packed)",
            vec![
                output("$mesh_$textureSet_BaseColor(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2)), (Dest::A, ch(Channel::Opacity, 0))]),
                output("$mesh_$textureSet_OcclusionRoughnessMetallic(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, virt(VirtualMap::AoMixed, 0)), (Dest::G, ch(Channel::Roughness, 0)), (Dest::B, ch(Channel::Metallic, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 8, true, vec![(Dest::R, virt(VirtualMap::NormalDirectX, 0)), (Dest::G, virt(VirtualMap::NormalDirectX, 1)), (Dest::B, virt(VirtualMap::NormalDirectX, 2))]),
                output("$mesh_$textureSet_Emissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
            ],
        ),
        preset(
            "Unreal Engine SSS (Packed)",
            vec![
                output("$mesh_$textureSet_BaseColor(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2)), (Dest::A, ch(Channel::ScatterWeight, 0))]),
                output("$mesh_$textureSet_OcclusionRoughnessMetallic(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, virt(VirtualMap::AoMixed, 0)), (Dest::G, ch(Channel::Roughness, 0)), (Dest::B, ch(Channel::Metallic, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 8, true, vec![(Dest::R, virt(VirtualMap::NormalDirectX, 0)), (Dest::G, virt(VirtualMap::NormalDirectX, 1)), (Dest::B, virt(VirtualMap::NormalDirectX, 2))]),
                output("$mesh_$textureSet_Emissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
            ],
        ),
        preset(
            "Vray Next (Metallic Roughness)",
            vec![
                output("$mesh_$textureSet_BaseColor(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2))]),
                output("$mesh_$textureSet_Roughness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Roughness, 0))]),
                output("$mesh_$textureSet_Metallic(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Metallic, 0))]),
                output("$mesh_$textureSet_Height(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::Height, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$mesh_$textureSet_Emissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
                output("$mesh_$textureSet_Transmissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Transmission, 0)), (Dest::G, ch(Channel::Transmission, 0)), (Dest::B, ch(Channel::Transmission, 0))]),
                output("$mesh_$textureSet_AnisotropyAngle(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::AnisotropyAngle, 0))]),
                output("$mesh_$textureSet_AnisotropyLevel(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::AnisotropyLevel, 0))]),
            ],
        ),
        preset(
            "Vray Next (Specular Glossiness)",
            vec![
                output("$mesh_$textureSet_Diffuse(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2))]),
                output("$mesh_$textureSet_Specular(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Specular, 0)), (Dest::G, ch(Channel::Specular, 1)), (Dest::B, ch(Channel::Specular, 2))]),
                output("$mesh_$textureSet_Glossiness(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Glossiness, 0))]),
                output("$mesh_$textureSet_Height(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::Height, 0))]),
                output("$mesh_$textureSet_Normal(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$mesh_$textureSet_Emissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
                output("$mesh_$textureSet_Transmissive(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Transmission, 0)), (Dest::G, ch(Channel::Transmission, 0)), (Dest::B, ch(Channel::Transmission, 0))]),
                output("$mesh_$textureSet_AnisotropyAngle(_$colorSpace)(.$udim)", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::AnisotropyAngle, 0))]),
                output("$mesh_$textureSet_AnisotropyLevel(_$colorSpace)(.$udim)", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::AnisotropyLevel, 0))]),
            ],
        ),
        preset(
            "Vray Next UDIM Legacy (Metallic Roughness)",
            vec![
                output("$mesh_BaseColor(_$colorSpace).$textureSet", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2))]),
                output("$mesh_Roughness(_$colorSpace).$textureSet", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Roughness, 0))]),
                output("$mesh_Metallic(_$colorSpace).$textureSet", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::Metallic, 0))]),
                output("$mesh_Height(_$colorSpace).$textureSet", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::Height, 0))]),
                output("$mesh_Normal(_$colorSpace).$textureSet", FileFormat::Png, 16, false, vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(VirtualMap::NormalOpenGl, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))]),
                output("$mesh_Emissive(_$colorSpace).$textureSet", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Emissive, 0)), (Dest::G, ch(Channel::Emissive, 1)), (Dest::B, ch(Channel::Emissive, 2))]),
                output("$mesh_Transmissive(_$colorSpace).$textureSet", FileFormat::Png, 8, false, vec![(Dest::R, ch(Channel::Transmission, 0)), (Dest::G, ch(Channel::Transmission, 0)), (Dest::B, ch(Channel::Transmission, 0))]),
                output("$mesh_AnisotropyAngle(_$colorSpace).$textureSet", FileFormat::Png, 16, false, vec![(Dest::L, ch(Channel::AnisotropyAngle, 0))]),
                output("$mesh_AnisotropyLevel(_$colorSpace).$textureSet", FileFormat::Png, 8, false, vec![(Dest::L, ch(Channel::AnisotropyLevel, 0))]),
            ],
        ),
    ]
}
