//! The gray a mask stroke paints

use std::sync::Arc;

use texturecraft_core::lum;

use crate::ImageRes;

/// The gray a mask stroke paints: one value, or the gray of an image laid once over the texture by its UVs
#[derive(Clone, Debug, PartialEq)]
pub enum MaskGray {
    Value(f32),
    /// Image resource `name`, read as its gray like a mask fill reads an image
    Image {
        name: String,
        image: Arc<ImageRes>,
    },
}

impl MaskGray {
    /// The gray painted at texel (`x`, `y`) of a `width` × `height` texture (the image read at the texel's centre)
    pub fn at(&self, x: u32, y: u32, (width, height): (u32, u32)) -> f32 {
        match self {
            MaskGray::Value(value) => value.clamp(0.0, 1.0),
            MaskGray::Image { image, .. } => {
                let u = (x as f32 + 0.5) / width.max(1) as f32;
                let v = (y as f32 + 0.5) / height.max(1) as f32;
                let [r, g, b, _] = image.sample(u, v);
                lum([r, g, b]).clamp(0.0, 1.0)
            }
        }
    }

    /// The image resource painted, None for one value
    pub fn image_name(&self) -> Option<&str> {
        match self {
            MaskGray::Value(_) => None,
            MaskGray::Image { name, .. } => Some(name),
        }
    }
}
