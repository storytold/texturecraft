use std::ops::Range;

/// A rectangle of output texels, from `min` (inclusive) to `max` (exclusive), in atlas pixels
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TexelBox {
    pub min: (u32, u32),
    pub max: (u32, u32),
}

impl TexelBox {
    /// Whether texel (`x`, `y`) lies inside the box
    pub fn contains(&self, x: usize, y: usize) -> bool {
        (self.min.0 as usize..self.max.0 as usize).contains(&x) && (self.min.1 as usize..self.max.1 as usize).contains(&y)
    }

    /// The items of texel row `y` outside the box, in a row of `len` items with `scale` items per texel: those before the box and those after it
    pub fn outside_row(&self, y: usize, scale: usize, len: usize) -> [Range<usize>; 2] {
        let (start, end) = match (self.min.1 as usize..self.max.1 as usize).contains(&y) {
            true => ((self.min.0 as usize * scale).min(len), (self.max.0 as usize * scale).min(len)),
            false => (len, len),
        };
        [0..start, end.max(start)..len]
    }

    /// The box grown by `texels` on every side, kept inside a `width` x `height` image
    pub fn grown(&self, texels: u32, width: u32, height: u32) -> TexelBox {
        TexelBox {
            min: (self.min.0.saturating_sub(texels), self.min.1.saturating_sub(texels)),
            max: (self.max.0.saturating_add(texels).min(width), self.max.1.saturating_add(texels).min(height)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_items_outside_a_row_are_the_ones_contains_leaves_out() {
        let (width, scale) = (6, 2);
        let boxes = [
            TexelBox { min: (1, 1), max: (4, 3) },
            TexelBox { min: (0, 0), max: (6, 4) },
            TexelBox { min: (3, 1), max: (9, 2) },
            TexelBox { min: (4, 1), max: (2, 3) },
        ];
        for texels in boxes {
            for y in 0..4 {
                let outside = texels.outside_row(y, scale, width * scale);
                for item in 0..width * scale {
                    let is_outside = outside.iter().any(|range| range.contains(&item));
                    assert_eq!(is_outside, !texels.contains(item / scale, y), "{texels:?} row {y} item {item}");
                }
            }
        }
    }
}
