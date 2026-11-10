//! [`IslandMask`]: the texels of a texture set that lie on the mesh (texel centre inside a triangle of the set's UV
//! islands). Only these are compared: everything else in a stored result is padding the original adds past the island
//! borders, which TextureCraft pads its own way.

use texturecraft_geom::{NO_TRI, UvGBuffer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IslandMask {
    pub width: u32,
    pub height: u32,
    inside: Vec<bool>,
}

impl IslandMask {
    /// The texels a UV G-buffer covers (its triangle under the texel centre).
    pub fn from_gbuf(gbuf: &UvGBuffer) -> IslandMask {
        let n = (gbuf.width as usize).saturating_mul(gbuf.height as usize);
        let inside = (0..n).map(|i| gbuf.tri.get(i).is_some_and(|t| *t != NO_TRI)).collect();
        IslandMask { width: gbuf.width, height: gbuf.height, inside }
    }

    /// Every texel of `width` × `height` for which `on(x, y)` holds (synthetic masks, tests).
    pub fn from_fn(width: u32, height: u32, on: impl Fn(u32, u32) -> bool) -> IslandMask {
        let inside = (0..height).flat_map(|y| (0..width).map(move |x| (x, y))).map(|(x, y)| on(x, y)).collect();
        IslandMask { width, height, inside }
    }

    pub fn contains(&self, x: u32, y: u32) -> bool {
        x < self.width && y < self.height && self.inside.get(y as usize * self.width as usize + x as usize).copied().unwrap_or(false)
    }

    /// Texels on the mesh.
    pub fn count(&self) -> usize {
        self.inside.iter().filter(|b| **b).count()
    }

    /// Leaves out texels within `steps` texels (4-neighbourhood) of the outside or the image border: where island
    /// borders meet padding, a texel can mix both on either side.
    pub fn shrink(&mut self, steps: u32) {
        let (w, h) = (self.width as usize, self.height as usize);
        for _ in 0..steps {
            let prev = self.inside.clone();
            let on = |x: usize, y: usize| prev.get(y * w + x).copied().unwrap_or(false);
            for y in 0..h {
                for x in 0..w {
                    let edge = x == 0 || y == 0 || x + 1 == w || y + 1 == h;
                    if on(x, y) && (edge || !on(x - 1, y) || !on(x + 1, y) || !on(x, y - 1) || !on(x, y + 1)) {
                        if let Some(t) = self.inside.get_mut(y * w + x) {
                            *t = false;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec2, Vec3};
    use texturecraft_geom::Mesh;

    #[test]
    fn a_half_square_covers_texels_under_its_triangle_and_shrinks_from_the_outside() {
        // one triangle over the upper-left half of UV space
        let mesh =
            Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y], None, vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)], vec![0, 1, 2], "m")
                .unwrap();
        let gbuf = UvGBuffer::new(&mesh, 0, 8, 8);
        let mut mask = IslandMask::from_gbuf(&gbuf);
        assert_eq!((mask.width, mask.height), (8, 8));
        assert!(mask.contains(0, 0) && !mask.contains(7, 7));
        // texel centres strictly inside x + y < 8: 7 + 6 + … + 0 → 28, plus the 8 on the diagonal edge = 36
        assert_eq!(mask.count(), 36);
        mask.shrink(1);
        assert!(!mask.contains(0, 0) && mask.contains(1, 1));
        assert!(!mask.contains(99, 0));
    }

    #[test]
    fn synthetic_masks_follow_the_predicate() {
        let mask = IslandMask::from_fn(4, 2, |x, _| x < 2);
        assert_eq!(mask.count(), 4);
        assert!(mask.contains(1, 1) && !mask.contains(2, 0));
        let mut all = IslandMask::from_fn(5, 5, |_, _| true);
        all.shrink(1);
        assert_eq!(all.count(), 9);
        all.shrink(10);
        assert_eq!(all.count(), 0);
    }
}
