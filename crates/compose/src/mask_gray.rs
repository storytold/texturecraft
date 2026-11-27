//! The gray a mask stroke paints

use std::sync::Arc;

use texturecraft_core::{Filtering, Sampling, UvWrap, lum};

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
                let sampling = Sampling { filtering: Filtering::BilinearSharp, wrap: UvWrap::None, ..Sampling::SAVED_BEFORE };
                let [r, g, b, _] = image.sample_filtered(u, v, sampling, 1.0);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mask_image_does_not_wrap_at_texture_edges() {
        let black = [0.0, 0.0, 0.0, 1.0];
        let white = [1.0; 4];
        let image = Arc::new(ImageRes::new(2, 2, vec![black, white, white, white], false));
        let brush = MaskGray::Image { name: "brush.png".to_owned(), image };

        let near_corner = brush.at(0, 0, (4, 4));
        let inside = brush.at(1, 1, (4, 4));
        let far_corner = brush.at(3, 3, (4, 4));

        assert!(near_corner < 0.01, "opposite edges must leave the black corner black, got {near_corner}");
        assert!(near_corner < inside && inside < far_corner, "sampling between image texels stays bilinear");
    }
}
