//! [`Metrics`]: one channel of ours against the stored result, over the texels of an [`IslandMask`](crate::IslandMask).
//!
//! Values come in the stored encoding, 0..1 (sRGB for colours stored sRGB), and differences are counted in 8-bit steps
//! of it (1 step = 1/255), per sample: every component of every texel compared is one sample. [`Tally`] accumulates
//! texel by texel, so a caller never needs both images whole.

use serde::Serialize;

/// Slack on the step thresholds: 8-bit values read as floats land a hair beside whole steps.
const STEP_SLACK: f64 = 1e-3;

/// The finished measurement of one channel.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Metrics {
    /// Mean absolute difference over all samples, in 8-bit steps.
    pub mean: f64,
    /// Largest difference of any sample, in 8-bit steps.
    pub max: f64,
    /// Share of samples within 1 step, in percent.
    pub within1: f64,
    /// Share of samples within 2 steps, in percent.
    pub within2: f64,
    /// Peak signal-to-noise ratio in dB for a peak of 1; `None` when the images are identical.
    pub psnr: Option<f64>,
    /// Texels compared (on the mesh).
    pub texels: usize,
    /// Samples compared (texels × components).
    pub samples: usize,
    /// `stored ≈ gain × ours + offset` over the first component: shows range, scale and inversion differences.
    pub fit: Option<Fit>,
}

/// A least-squares line `stored ≈ gain × ours + offset`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Fit {
    pub gain: f64,
    pub offset: f64,
}

/// Running sums for [`Metrics`].
#[derive(Debug, Clone, Default)]
pub struct Tally {
    sum: f64,
    squares: f64,
    max: f64,
    within1: usize,
    within2: usize,
    samples: usize,
    texels: usize,
    // the fit's sums over the first component (x ours, y stored)
    n: f64,
    sx: f64,
    sy: f64,
    sxx: f64,
    sxy: f64,
}

impl Tally {
    /// Adds one texel: the components of ours and of the stored result (as many as the shorter slice holds). A
    /// non-finite difference counts as the full range (255 steps).
    pub fn add(&mut self, ours: &[f32], stored: &[f32]) {
        self.texels += 1;
        for (a, b) in ours.iter().zip(stored) {
            let d = (f64::from(*a) - f64::from(*b)).abs() * 255.0;
            let d = if d.is_finite() { d } else { 255.0 };
            self.sum += d;
            self.squares += d * d;
            self.max = self.max.max(d);
            self.within1 += usize::from(d <= 1.0 + STEP_SLACK);
            self.within2 += usize::from(d <= 2.0 + STEP_SLACK);
            self.samples += 1;
        }
        if let (Some(a), Some(b)) = (ours.first(), stored.first())
            && a.is_finite()
            && b.is_finite()
        {
            let (x, y) = (f64::from(*a), f64::from(*b));
            self.n += 1.0;
            self.sx += x;
            self.sy += y;
            self.sxx += x * x;
            self.sxy += x * y;
        }
    }

    pub fn finish(&self) -> Metrics {
        let samples = self.samples.max(1) as f64;
        let mse = self.squares / samples / (255.0 * 255.0);
        let share = |k: usize| if self.samples == 0 { 100.0 } else { 100.0 * k as f64 / samples };
        Metrics {
            mean: self.sum / samples,
            max: self.max,
            within1: share(self.within1),
            within2: share(self.within2),
            psnr: (mse > 0.0).then(|| -10.0 * mse.log10()),
            texels: self.texels,
            samples: self.samples,
            fit: self.fit(),
        }
    }

    fn fit(&self) -> Option<Fit> {
        if self.n == 0.0 {
            return None;
        }
        let var = self.n * self.sxx - self.sx * self.sx;
        if var.abs() < 1e-12 {
            // ours is flat: no slope to measure, the offset is the stored mean
            return Some(Fit { gain: 0.0, offset: self.sy / self.n });
        }
        let gain = (self.n * self.sxy - self.sx * self.sy) / var;
        Some(Fit { gain, offset: (self.sy - gain * self.sx) / self.n })
    }
}

impl Metrics {
    /// Measures two whole images of `components` per texel (row-major, the mask's size) on the mask's texels.
    /// `None` when a size disagrees.
    pub fn of_images(ours: &[[f32; 4]], stored: &[[f32; 4]], components: usize, mask: &crate::IslandMask) -> Option<Metrics> {
        let n = (mask.width as usize).checked_mul(mask.height as usize)?;
        if ours.len() != n || stored.len() != n {
            return None;
        }
        let c = components.clamp(1, 4);
        let mut tally = Tally::default();
        for y in 0..mask.height {
            for x in 0..mask.width {
                if !mask.contains(x, y) {
                    continue;
                }
                let i = y as usize * mask.width as usize + x as usize;
                if let (Some(a), Some(b)) = (ours.get(i), stored.get(i)) {
                    tally.add(a.get(..c)?, b.get(..c)?);
                }
            }
        }
        Some(tally.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IslandMask;

    const STEP: f32 = 1.0 / 255.0;

    #[test]
    fn identical_images_score_zero_everywhere() {
        let img = vec![[0.25, 0.5, 0.75, 1.0]; 16];
        let m = Metrics::of_images(&img, &img, 3, &IslandMask::from_fn(4, 4, |_, _| true)).unwrap();
        assert_eq!((m.mean, m.max, m.within1, m.within2, m.texels, m.samples), (0.0, 0.0, 100.0, 100.0, 16, 48));
        assert_eq!(m.psnr, None);
    }

    #[test]
    fn steps_shares_and_maximum_follow_the_differences() {
        // four texels, one component: 0, 1, 2 and 10 steps off
        let ours = vec![[0.5; 4]; 4];
        let stored: Vec<[f32; 4]> = [0.0, 1.0, 2.0, 10.0].iter().map(|k| [0.5 + k * STEP; 4]).collect();
        let m = Metrics::of_images(&ours, &stored, 1, &IslandMask::from_fn(2, 2, |_, _| true)).unwrap();
        assert!((m.mean - 13.0 / 4.0).abs() < 1e-3, "{m:?}");
        assert!((m.max - 10.0).abs() < 1e-3);
        assert_eq!((m.within1, m.within2), (50.0, 75.0));
        // mse = (0 + 1 + 4 + 100) / 4 steps²
        let psnr = -10.0 * (105.0 / 4.0 / (255.0f64 * 255.0)).log10();
        assert!((m.psnr.unwrap() - psnr).abs() < 1e-3);
    }

    #[test]
    fn texels_off_the_mesh_do_not_count() {
        let ours = vec![[0.5; 4]; 4];
        let mut stored = ours.clone();
        stored[3] = [1.0; 4];
        let all = Metrics::of_images(&ours, &stored, 3, &IslandMask::from_fn(2, 2, |_, _| true)).unwrap();
        assert!(all.max > 120.0);
        let on_mesh = Metrics::of_images(&ours, &stored, 3, &IslandMask::from_fn(2, 2, |x, y| (x, y) != (1, 1))).unwrap();
        assert_eq!((on_mesh.max, on_mesh.texels), (0.0, 3));
        // an empty mask measures nothing and divides by nothing
        let none = Metrics::of_images(&ours, &stored, 3, &IslandMask::from_fn(2, 2, |_, _| false)).unwrap();
        assert_eq!((none.mean, none.texels, none.within1), (0.0, 0, 100.0));
        assert_eq!(none.fit, None);
        // a size mismatch is refused, never read out of bounds
        assert!(Metrics::of_images(&ours, &stored[..3], 3, &IslandMask::from_fn(2, 2, |_, _| true)).is_none());
    }

    #[test]
    fn nan_counts_as_the_full_range_and_the_fit_reads_range_differences() {
        let mut t = Tally::default();
        t.add(&[f32::NAN], &[0.5]);
        assert_eq!(t.finish().max, 255.0);
        // stored = 0.5 × ours + 0.25 (a halved, offset range)
        let mut t = Tally::default();
        for v in [0.0f32, 0.2, 0.6, 1.0] {
            t.add(&[v], &[0.5 * v + 0.25]);
        }
        let fit = t.finish().fit.unwrap();
        assert!((fit.gain - 0.5).abs() < 1e-6 && (fit.offset - 0.25).abs() < 1e-6, "{fit:?}");
        let mut flat = Tally::default();
        flat.add(&[0.3], &[0.1]);
        flat.add(&[0.3], &[0.3]);
        let fit = flat.finish().fit.unwrap();
        assert!(fit.gain == 0.0 && (fit.offset - 0.2).abs() < 1e-6, "{fit:?}");
    }
}
