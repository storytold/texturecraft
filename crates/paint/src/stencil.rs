//! A stencil a stroke paints through: a grey image laid on the view. Once the stroke's stamps are laid, its coverage at
//! each texel is multiplied by the stencil where that texel's surface point lies on the view
//! ([`crate::StrokeCanvas::scale`]), so the stencil limits the stroke as a whole, not each stamp.

use std::sync::Arc;

use texturecraft_geom::glam::{Mat3, Vec2, Vec3};

/// The stencil's grey and where it lies on the view.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewStencil {
    /// One value everywhere (inside and outside the image), or the image.
    grey: Grey,
    /// From the view's normalised device coordinates (−1..1, y up) to the image's (0..1, y up).
    to_image: Mat3,
}

#[derive(Clone, Debug, PartialEq)]
enum Grey {
    Uniform(f32),
    /// Rows top first.
    Image {
        width: usize,
        height: usize,
        values: Arc<[f32]>,
    },
}

fn unit(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

impl ViewStencil {
    /// A `width × height` grey image (rows top first, values 0..1) placed by `to_image` (column by column, from the
    /// view's normalised device coordinates to the image's 0..1 with y up); `None` when the sizes do not match or the
    /// matrix is not finite.
    pub fn image(width: usize, height: usize, values: Arc<[f32]>, to_image: [f32; 9]) -> Option<ViewStencil> {
        let to_image = Mat3::from_cols_array(&to_image);
        (width > 0 && height > 0 && width.checked_mul(height) == Some(values.len()) && to_image.is_finite())
            .then_some(ViewStencil { grey: Grey::Image { width, height, values }, to_image })
    }

    /// A stencil of one grey everywhere.
    pub fn uniform(value: f32) -> ViewStencil {
        ViewStencil { grey: Grey::Uniform(unit(value)), to_image: Mat3::IDENTITY }
    }

    /// What the stencil lets through at the view's normalised device coordinates `ndc` (y up): the image's bilinear grey
    /// there, 0 outside it.
    pub fn at(&self, ndc: Vec2) -> f32 {
        let (width, height, values) = match &self.grey {
            Grey::Uniform(v) => return *v,
            Grey::Image { width, height, values } => (*width, *height, values),
        };
        let p = self.to_image * Vec3::new(ndc.x, ndc.y, 1.0);
        // the image's rows run down
        let (u, v) = (p.x, 1.0 - p.y);
        if !((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)) {
            return 0.0;
        }
        let (x, y) = (u * width as f32 - 0.5, v * height as f32 - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |a: f32, b: f32| -> f32 {
            if a < 0.0 || b < 0.0 || a >= width as f32 || b >= height as f32 {
                return 0.0;
            }
            values.get(b as usize * width + a as usize).copied().map_or(0.0, unit)
        };
        let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1.0, y0) * fx;
        let bottom = at(x0, y0 + 1.0) * (1.0 - fx) + at(x0 + 1.0, y0 + 1.0) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_image_is_read_through_its_matrix_with_rows_down() {
        // top half white, bottom half black, spanning the view
        let values: Arc<[f32]> = Arc::from(vec![1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0]);
        let s = ViewStencil::image(2, 4, values, [0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.5, 0.5, 1.0]).unwrap();
        assert!(s.at(Vec2::new(0.0, 0.5)) > 0.99, "the top of the view reads the image's top rows");
        assert!(s.at(Vec2::new(0.0, -0.5)) < 0.01);
        assert_eq!(s.at(Vec2::new(0.0, 2.5)), 0.0, "nothing outside the image");
        assert_eq!(s.at(Vec2::new(f32::NAN, 0.0)), 0.0);
        // a uniform stencil lets its value through everywhere
        assert_eq!(ViewStencil::uniform(0.4).at(Vec2::new(9.0, 9.0)), 0.4);
        assert!(ViewStencil::image(2, 2, Arc::from(vec![0.0; 3]), [0.0; 9]).is_none());
    }
}
