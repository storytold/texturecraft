//! Which surface points the paint camera can see: a software-rasterized eye-distance buffer,
//! rendered once at the start of a stroke.

use rayon::prelude::*;
use texturecraft_geom::glam::{Mat4, Vec2, Vec3};
use texturecraft_geom::{Mesh, NO_TRI, OrbitCamera};

const BAND: usize = 16;

/// Largest buffer side in pixels.
const MAX_SIZE: u32 = 4096;

/// Clip-space w below which a vertex counts as behind the camera.
pub const MIN_CLIP_W: f32 = 1e-5;

/// Screen area (pixels squared) below which a triangle is skipped as degenerate.
const MIN_SCREEN_AREA: f32 = 1e-9;

/// How far outside a triangle (in barycentric weight) a pixel center may lie and still count, closing seams between neighbours.
const EDGE_TOLERANCE: f32 = 1e-4;

/// [`EyeDepth::bias`] as a fraction of the scene radius.
const BIAS_PER_RADIUS: f32 = 0.004;

/// Distance from the eye to the nearest surface per screen pixel (f32::MAX where empty).
#[derive(Clone, Debug)]
pub struct EyeDepth {
    pub width: u32,
    pub height: u32,
    pub view_proj: Mat4,
    pub eye: Vec3,
    pub dist: Vec<f32>,
    /// The nearest triangle per screen pixel ([`NO_TRI`] where empty); empty unless rendered with [`EyeDepth::render_with_triangles`].
    pub triangle: Vec<u32>,
    /// Tolerance in world units (≈ 0.4% of the scene radius).
    pub bias: f32,
}

/// Where `position` lands in a `size` pixel view through `view_proj` (origin top-left), or None behind the camera.
pub fn screen_point(view_proj: Mat4, position: Vec3, size: Vec2) -> Option<Vec2> {
    let clip = view_proj * position.extend(1.0);
    (clip.w > MIN_CLIP_W).then(|| Vec2::new(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5) * size)
}

impl EyeDepth {
    /// Rasterize `mesh` seen by `camera` into a `width × height` buffer (sizes capped at [`MAX_SIZE`]), without [`EyeDepth::triangle`].
    pub fn render(mesh: &Mesh, camera: &OrbitCamera, width: u32, height: u32) -> EyeDepth {
        Self::rasterize(mesh, camera, width, height, false)
    }

    /// [`EyeDepth::render`] that also records the nearest triangle per pixel in [`EyeDepth::triangle`].
    pub fn render_with_triangles(mesh: &Mesh, camera: &OrbitCamera, width: u32, height: u32) -> EyeDepth {
        Self::rasterize(mesh, camera, width, height, true)
    }

    /// The eye-distance buffer, plus the nearest triangle per pixel when `record_triangles` is set.
    fn rasterize(mesh: &Mesh, camera: &OrbitCamera, width: u32, height: u32, record_triangles: bool) -> EyeDepth {
        let (width, height) = (width.clamp(1, MAX_SIZE), height.clamp(1, MAX_SIZE));
        let size = Vec2::new(width as f32, height as f32);
        let view_proj = camera.view_proj(size.x / size.y);
        let eye = camera.eye();

        // Project every vertex once
        let screen: Vec<Option<Vec2>> = mesh.positions.iter().map(|position| screen_point(view_proj, *position, size)).collect();
        let vertex_distance: Vec<f32> = mesh.positions.iter().map(|position| position.distance(eye)).collect();

        // Bin each triangle into the bands of rows it covers
        let mut bands: Vec<Vec<u32>> = vec![Vec::new(); (height as usize).div_ceil(BAND)];
        for triangle in 0..mesh.triangle_count() {
            let Some([first, second, third]) = mesh.tri(triangle) else { continue };
            let (Some(Some(first)), Some(Some(second)), Some(Some(third))) = (screen.get(first), screen.get(second), screen.get(third)) else { continue };

            let top = first.y.min(second.y).min(third.y).floor().max(0.0);
            let bottom = first.y.max(second.y).max(third.y).ceil().min(size.y);
            if bottom <= top || !top.is_finite() || !bottom.is_finite() {
                continue;
            }
            let (first_band, last_band) = (top as usize / BAND, (bottom as usize).saturating_sub(1) / BAND);
            for band in bands.iter_mut().take(last_band + 1).skip(first_band) {
                band.push(triangle as u32);
            }
        }

        let mut dist = vec![f32::MAX; (width * height) as usize];
        let mut triangle = if record_triangles { vec![NO_TRI; (width * height) as usize] } else { Vec::new() };
        let band_len = BAND * width as usize;
        let mut triangle_bands: Vec<Option<&mut [u32]>> = triangle.chunks_mut(band_len).map(Some).collect();
        triangle_bands.resize_with(dist.len().div_ceil(band_len), || None);
        dist.par_chunks_mut(band_len).zip(triangle_bands.into_par_iter()).enumerate().for_each(|(band, (band_dist, mut band_triangles))| {
            let first_row = band * BAND;
            let row_count = band_dist.len() / width as usize;
            for &candidate in bands.get(band).map(Vec::as_slice).unwrap_or(&[]) {
                let Some(corners) = mesh.tri(candidate as usize) else { continue };
                let [first, second, third] = corners;
                let (Some(Some(first_point)), Some(Some(second_point)), Some(Some(third_point))) = (screen.get(first), screen.get(second), screen.get(third))
                else {
                    continue;
                };

                let corner_distance = corners.map(|vertex| vertex_distance.get(vertex).copied().unwrap_or(f32::MAX));
                let area = (*second_point - *first_point).perp_dot(*third_point - *first_point);
                if area.abs() < MIN_SCREEN_AREA {
                    continue;
                }

                let low = first_point.min(*second_point).min(*third_point).floor();
                let high = first_point.max(*second_point).max(*third_point).ceil();
                let (left, right) = (low.x.max(0.0) as usize, (high.x.min(size.x) as usize).min(width as usize));
                let (top, bottom) = (low.y.max(first_row as f32) as usize, high.y.min((first_row + row_count) as f32) as usize);
                for row in top.max(first_row)..bottom {
                    for column in left..right {
                        let sample = Vec2::new(column as f32 + 0.5, row as f32 + 0.5);
                        let first_weight = (*third_point - *second_point).perp_dot(sample - *second_point) / area;
                        let second_weight = (*first_point - *third_point).perp_dot(sample - *third_point) / area;
                        let third_weight = 1.0 - first_weight - second_weight;
                        if first_weight < -EDGE_TOLERANCE || second_weight < -EDGE_TOLERANCE || third_weight < -EDGE_TOLERANCE {
                            continue;
                        }

                        let distance = first_weight * corner_distance[0] + second_weight * corner_distance[1] + third_weight * corner_distance[2];
                        let at = (row - first_row) * width as usize + column;
                        if let Some(slot) = band_dist.get_mut(at)
                            && distance < *slot
                        {
                            *slot = distance;
                            if let Some(nearest) = band_triangles.as_deref_mut().and_then(|triangles| triangles.get_mut(at)) {
                                *nearest = candidate;
                            }
                        }
                    }
                }
            }
        });

        EyeDepth { width, height, view_proj, eye, dist, triangle, bias: mesh.radius() * BIAS_PER_RADIUS }
    }

    /// Whether world point `p` is visible (not hidden behind other geometry, inside the view).
    pub fn visible(&self, p: Vec3) -> bool {
        let c = self.view_proj * p.extend(1.0);
        if c.w <= 1e-6 {
            return false;
        }
        let (nx, ny) = (c.x / c.w, c.y / c.w);
        if nx.abs() > 1.0 || ny.abs() > 1.0 {
            return false;
        }
        let x = (((nx * 0.5 + 0.5) * self.width as f32) as u32).min(self.width - 1);
        let y = (((0.5 - ny * 0.5) * self.height as f32) as u32).min(self.height - 1);
        // Conservative: the nearest surface among the pixel and its neighbours.
        let mut near = f32::MAX;
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                let (sx, sy) = (x as i32 + dx, y as i32 + dy);
                if sx < 0 || sy < 0 || sx >= self.width as i32 || sy >= self.height as i32 {
                    continue;
                }
                if let Some(d) = self.dist.get((sy as u32 * self.width + sx as u32) as usize) {
                    near = near.min(*d);
                }
            }
        }
        // Allow a bias that grows with distance (depth buffer resolution).
        p.distance(self.eye) <= near + self.bias + near * 0.002
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_geom::primitives;

    #[test]
    fn front_of_sphere_is_visible_back_is_not() {
        let m = primitives::sphere(1.0, 64, 32);
        let mut cam = OrbitCamera::default();
        cam.yaw = 0.0;
        cam.pitch = 0.0;
        cam.frame(Vec3::splat(-1.0), Vec3::splat(1.0));
        let d = EyeDepth::render(&m, &cam, 256, 256);
        let toward = (cam.eye() - cam.target).normalize();
        assert!(d.visible(toward * 1.0));
        assert!(!d.visible(-toward * 1.0));
        assert!(!d.visible(Vec3::new(100.0, 0.0, 0.0)));
    }
}
