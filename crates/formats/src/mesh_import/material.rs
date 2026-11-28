//! A material's PBR inputs as a mesh file embeds them, as the maps of a fill layer.

use texturecraft_core::Channel;
use texturecraft_geom::glam::{Vec3, Vec4};
use texturecraft_texel::{linear_to_srgb, srgb_to_linear};

use crate::Result;

/// A material's PBR inputs as the file gives them: linear factors and the encoded images (glTF
/// conventions: roughness in the green and metallic in the blue channel of one texture,
/// occlusion in the red, OpenGL (+Y) normal maps). The default is glTF's default material.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialTextures {
    pub material: String,
    /// Linear RGBA base color factor.
    pub base_color: Vec4,
    pub base_color_image: Option<Vec<u8>>,
    pub metallic: f32,
    pub roughness: f32,
    pub metallic_roughness_image: Option<Vec<u8>>,
    pub normal_image: Option<Vec<u8>>,
    /// glTF `normalTexture.scale` (1: the map as it is).
    pub normal_scale: f32,
    pub occlusion_image: Option<Vec<u8>>,
    /// glTF `occlusionTexture.strength`.
    pub occlusion_strength: f32,
    /// Linear RGB emissive factor (the emissive strength included).
    pub emissive: Vec3,
    pub emissive_image: Option<Vec<u8>>,
}

impl Default for MaterialTextures {
    /// glTF's defaults: white base color, metallic and roughness 1, unscaled normal and full
    /// occlusion strength, no emission.
    fn default() -> Self {
        MaterialTextures {
            material: String::new(),
            base_color: Vec4::ONE,
            base_color_image: None,
            metallic: 1.0,
            roughness: 1.0,
            metallic_roughness_image: None,
            normal_image: None,
            normal_scale: 1.0,
            occlusion_image: None,
            occlusion_strength: 1.0,
            emissive: Vec3::ZERO,
            emissive_image: None,
        }
    }
}

/// What a fill layer needs for one material: encoded images (with their file extension) and
/// constant linear values per channel, plus the textures that could not be used.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MaterialMaps {
    pub images: Vec<(Channel, Vec<u8>, &'static str)>,
    pub values: Vec<(Channel, [f32; 3])>,
    pub warnings: Vec<String>,
}

impl MaterialTextures {
    /// True when the material carries at least one texture.
    pub fn has_images(&self) -> bool {
        [&self.base_color_image, &self.metallic_roughness_image, &self.normal_image, &self.occlusion_image, &self.emissive_image].iter().any(|i| i.is_some())
    }

    /// The fill maps of the material: textures with their factors applied (decoded only where a
    /// factor or a channel split changes them), factors without a texture as constant values.
    pub fn maps(&self) -> MaterialMaps {
        let mut out = MaterialMaps::default();
        let name = &self.material;
        let image = |out: &mut MaterialMaps, c: Channel, r: Result<(Vec<u8>, &'static str)>| match r {
            Ok((bytes, ext)) => out.images.push((c, bytes, ext)),
            Err(e) => out.warnings.push(format!("the {} texture of {name}: {e}", c.id())),
        };
        let pass = |bytes: &[u8]| Ok((bytes.to_vec(), extension_of(bytes)));

        // Base color: the image as it is, or scaled by its linear factor.
        match &self.base_color_image {
            Some(b) if self.base_color == Vec4::ONE => image(&mut out, Channel::BaseColor, pass(b)),
            Some(b) => image(&mut out, Channel::BaseColor, scaled_color(b, self.base_color)),
            None => out.values.push((Channel::BaseColor, self.base_color.truncate().to_array())),
        }

        // glTF packs roughness in green and metallic in blue of one texture; each factor scales
        // its own channel.
        match &self.metallic_roughness_image {
            Some(b) => match decode(b) {
                Ok(img) => {
                    image(&mut out, Channel::Metallic, channel_png(&img, 2, self.metallic));
                    image(&mut out, Channel::Roughness, channel_png(&img, 1, self.roughness));
                }
                Err(e) => out.warnings.push(format!("the metallic/roughness texture of {name}: {e}")),
            },
            None => {
                out.values.push((Channel::Metallic, [self.metallic; 3]));
                out.values.push((Channel::Roughness, [self.roughness; 3]));
            }
        }

        if let Some(b) = &self.normal_image {
            image(&mut out, Channel::Normal, scaled_normal(b, self.normal_scale));
        }
        if let Some(b) = &self.occlusion_image {
            match decode(b) {
                // Occlusion is the red channel of its texture, blended toward none (white) by its
                // strength: glTF's 1 + strength * (red - 1)
                Ok(img) => image(&mut out, Channel::AmbientOcclusion, occlusion_png(&img, self.occlusion_strength)),
                Err(e) => out.warnings.push(format!("the occlusion texture of {name}: {e}")),
            }
        }

        if self.emissive_image.is_some() && self.emissive.max_element() > 1.0 {
            out.warnings.push(format!("the emissive strength of {name} is above 1: the emissive texture stores at most 1, so brighter texels are clipped"));
        }
        match &self.emissive_image {
            Some(b) if self.emissive == Vec3::ONE => image(&mut out, Channel::Emissive, pass(b)),
            Some(b) => image(&mut out, Channel::Emissive, scaled_color(b, self.emissive.extend(1.0))),
            None if self.emissive != Vec3::ZERO => out.values.push((Channel::Emissive, self.emissive.to_array())),
            None => {}
        }
        out
    }
}

/// The file extension of an encoded image.
fn extension_of(bytes: &[u8]) -> &'static str {
    craft_image_codecs::detect(bytes).and_then(|f| f.extensions().first().copied()).unwrap_or("png")
}

/// Decode an embedded image within the resource texel budget.
fn decode(bytes: &[u8]) -> Result<crate::DecodedImage> {
    crate::decode_image_within(bytes, crate::MAX_RESOURCE_TEXELS)
}

/// One channel of `img` times `factor`, as a 16-bit gray PNG (glTF packs roughness in green
/// and metallic in blue).
fn channel_png(img: &crate::DecodedImage, channel: usize, factor: f32) -> Result<(Vec<u8>, &'static str)> {
    let factor = if factor.is_finite() { factor.clamp(0.0, 1.0) } else { 1.0 };
    let values = img.rgba.iter().map(|p| p.get(channel).copied().unwrap_or(0.0) * factor);
    Ok((crate::gray16_png(img.width, img.height, values)?, "png"))
}

/// The occlusion (red channel) of `img` blended toward no occlusion by glTF's `strength`, as a
/// 16-bit gray PNG: `1 + strength * (red - 1)`.
fn occlusion_png(img: &crate::DecodedImage, strength: f32) -> Result<(Vec<u8>, &'static str)> {
    let strength = if strength.is_finite() { strength.clamp(0.0, 1.0) } else { 1.0 };
    let values = img.rgba.iter().map(|p| 1.0 + strength * (p.first().copied().unwrap_or(1.0) - 1.0));
    Ok((crate::gray16_png(img.width, img.height, values)?, "png"))
}

/// An sRGB color texture times a linear `factor` (alpha times its alpha), as an RGBA8 PNG.
fn scaled_color(bytes: &[u8], factor: Vec4) -> Result<(Vec<u8>, &'static str)> {
    let img = decode(bytes)?;
    // Not clamped to 1: an emissive strength scales the texture up (the stored texel still is)
    let f = factor.to_array().map(|v| if v.is_finite() { v.max(0.0) } else { 1.0 });
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut pixels = Vec::with_capacity(img.rgba.len());
    for p in &img.rgba {
        let mut out = [0u8; 4];
        for (k, out) in out.iter_mut().enumerate().take(3) {
            let linear = if img.srgb { srgb_to_linear(p[k]) } else { p[k] };
            let scaled = linear * f[k];
            *out = byte(if img.srgb { linear_to_srgb(scaled) } else { scaled });
        }
        // Alpha is linear in every encoding.
        out[3] = byte(p[3] * f[3]);
        pixels.push(out);
    }
    Ok((crate::rgba8_png(img.width, img.height, &pixels)?, "png"))
}

/// A normal map with glTF's `scale` applied to its tangent-space x and y (stored packed as
/// `n * 0.5 + 0.5`), as an RGBA8 PNG; scale 1 keeps the original bytes.
fn scaled_normal(bytes: &[u8], scale: f32) -> Result<(Vec<u8>, &'static str)> {
    if scale == 1.0 {
        return Ok((bytes.to_vec(), extension_of(bytes)));
    }
    let img = decode(bytes)?;
    let s = if scale.is_finite() { scale.max(0.0) } else { 1.0 };
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut pixels = Vec::with_capacity(img.rgba.len());
    for p in &img.rgba {
        // glTF: normalize((stored * 2 - 1) * (scale, scale, 1)), so the direction survives a
        // scale above 1. The data map is read raw (no sRGB linearization); alpha passes through.
        let n = Vec3::new((p[0] * 2.0 - 1.0) * s, (p[1] * 2.0 - 1.0) * s, p[2] * 2.0 - 1.0).try_normalize().unwrap_or(Vec3::Z);
        pixels.push([byte(n.x * 0.5 + 0.5), byte(n.y * 0.5 + 0.5), byte(n.z * 0.5 + 0.5), byte(p[3])]);
    }
    Ok((crate::rgba8_png(img.width, img.height, &pixels)?, "png"))
}
