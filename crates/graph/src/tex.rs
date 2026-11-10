// SPDX-License-Identifier: MIT OR Apache-2.0
//! Float RGBA texture with wrapped (tileable) sampling.

#[derive(Clone, Debug)]
pub struct Tex {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[f32; 4]>,
}

/// floor() without a libm call: the baseline x86-64 target has no SSE4.1 rounding instruction, so `f32::floor` is a
/// function call there. Exact for |x| < 2^31 (callers use texture coordinates).
#[inline(always)]
pub fn ffloor(x: f32) -> f32 {
    let i = x as i32 as f32;
    if i > x { i - 1.0 } else { i }
}

/// `f64::floor` without a libm call (see [`ffloor`]), the same for every input: below 2^62 in size by truncation,
/// anything else (larger, infinite, NaN) through `floor` itself.
#[inline(always)]
pub fn floor64(x: f64) -> f64 {
    if x == 0.0 {
        // keeps the sign of a zero, as floor() does
        x
    } else if x.abs() < 4.611_686_018_427_388e18 {
        let i = x as i64 as f64;
        if i > x { i - 1.0 } else { i }
    } else {
        x.floor()
    }
}

pub fn lum(p: [f32; 4]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

fn san(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

/// Item `i` of `s`, the default (zero) past its end: an index that can't panic.
#[inline(always)]
pub fn get<T: Copy + Default>(s: &[T], i: usize) -> T {
    s.get(i).copied().unwrap_or_default()
}

/// `[f(a[0], b[0]), .. f(a[3], b[3])]`: a per-channel operation on two pixels, without indexing by a variable.
#[inline(always)]
pub fn zip4<A: Copy, B: Copy, R>(a: [A; 4], b: [B; 4], f: impl Fn(A, B) -> R) -> [R; 4] {
    [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2]), f(a[3], b[3])]
}

impl Tex {
    pub fn solid(w: usize, h: usize, c: [f32; 4]) -> Tex {
        Tex { w, h, px: vec![c; w * h] }
    }

    /// Build a texture from a per-pixel function of (u, v) in [0,1). Runs multi-threaded.
    pub fn from_fn<F: Fn(f32, f32) -> [f32; 4] + Sync>(w: usize, h: usize, f: F) -> Tex {
        Tex::from_fn_with(w, h, || (), |_, u, v| f(u, v))
    }

    /// As [`Tex::from_fn`], with working state: `init` makes it for each band of rows (one per thread), `f` gets it for
    /// each pixel.
    /// What a pixel gets must not depend on the pixels computed with the same state before it.
    pub fn from_fn_with<S, I: Fn() -> S + Sync, F: Fn(&mut S, f32, f32) -> [f32; 4] + Sync>(w: usize, h: usize, init: I, f: F) -> Tex {
        let w = w.max(1);
        let h = h.max(1);
        let mut px = vec![[0.0f32; 4]; w * h];
        // one band of whole rows per thread: a state can be costly to make (a function's position-independent part
        // runs once per state), and a cancel check per row
        let rows_per = h.div_ceil(crate::par::helpers() + 1);
        let cancel = &crate::context::current_cancel();
        crate::par::for_each_run_mut(&mut px, rows_per * w, |first, chunk| {
            let mut state = init();
            for (i, p) in chunk.iter_mut().enumerate() {
                if i % w == 0 && crate::context::is_set(cancel) {
                    return;
                }
                let idx = first + i;
                let x = idx % w;
                let y = idx / w;
                let o = f(&mut state, (x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                *p = [san(o[0]), san(o[1]), san(o[2]), san(o[3])];
            }
        });
        Tex { w, h, px }
    }

    pub fn gray_fn<F: Fn(f32, f32) -> f32 + Sync>(w: usize, h: usize, f: F) -> Tex {
        Tex::from_fn(w, h, |u, v| {
            let g = f(u, v);
            [g, g, g, 1.0]
        })
    }

    /// Pixel `i` (counted row by row), transparent black past the end.
    #[inline]
    pub fn px_at(&self, i: usize) -> [f32; 4] {
        self.px.get(i).copied().unwrap_or([0.0; 4])
    }

    /// Sets pixel `i` (counted row by row); past the end, nothing.
    #[inline]
    pub fn set_px(&mut self, i: usize, v: [f32; 4]) {
        if let Some(p) = self.px.get_mut(i) {
            *p = v;
        }
    }

    pub fn at(&self, x: i64, y: i64) -> [f32; 4] {
        // a branch instead of a division when the pixel is inside, as most are
        let wrap = |i: i64, n: usize| if i >= 0 && (i as usize) < n { i as usize } else { i.rem_euclid((n as i64).max(1)) as usize };
        self.px_at(wrap(y, self.h) * self.w + wrap(x, self.w))
    }

    /// Bilinear, wrapped sampling in uv space.
    #[inline]
    pub fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let u = if u.is_finite() { u } else { 0.0 };
        let v = if v.is_finite() { v } else { 0.0 };
        let x = u * self.w as f32 - 0.5;
        let y = v * self.h as f32 - 0.5;
        let x0 = ffloor(x);
        let y0 = ffloor(y);
        let fx = x - x0;
        let fy = y - y0;
        let (w, h) = (self.w as i64, self.h as i64);
        // wrap the two columns and the two rows once (a branch instead of a division in the common case)
        let wrap = |i: i64, n: i64| if i >= 0 && i < n { i } else { i.rem_euclid(n.max(1)) };
        let (xa, ya) = (wrap(x0 as i64, w), wrap(y0 as i64, h));
        let xb = if xa + 1 == w { 0 } else { xa + 1 };
        let yb = if ya + 1 == h { 0 } else { ya + 1 };
        let (r0, r1) = ((ya * w) as usize, (yb * w) as usize);
        let a = self.px_at(r0 + xa as usize);
        let b = self.px_at(r0 + xb as usize);
        let c = self.px_at(r1 + xa as usize);
        let d = self.px_at(r1 + xb as usize);
        let top = zip4(a, b, |a, b| a + (b - a) * fx);
        let bot = zip4(c, d, |c, d| c + (d - c) * fx);
        zip4(top, bot, |top, bot| top + (bot - top) * fy)
    }

    /// Catmull-Rom (bicubic) wrapped sampling in uv space: sharper than bilinear for rotations, scaling and warps. The
    /// result stays inside the range of the four nearest texels, so it never overshoots.
    pub fn sample_cubic(&self, u: f32, v: f32) -> [f32; 4] {
        let u = if u.is_finite() { u } else { 0.0 };
        let v = if v.is_finite() { v } else { 0.0 };
        let x = u * self.w as f32 - 0.5;
        let y = v * self.h as f32 - 0.5;
        let (x0, y0) = (ffloor(x), ffloor(y));
        let (fx, fy) = (x - x0, y - y0);
        let (xi, yi) = (x0 as i64, y0 as i64);
        let wts = |t: f32| {
            [
                0.5 * (-t * t * t + 2.0 * t * t - t),
                0.5 * (3.0 * t * t * t - 5.0 * t * t + 2.0),
                0.5 * (-3.0 * t * t * t + 4.0 * t * t + t),
                0.5 * (t * t * t - t * t),
            ]
        };
        let (wx, wy) = (wts(fx), wts(fy));
        let mut o = [0.0f32; 4];
        let inside = xi >= 1 && yi >= 1 && xi + 2 < self.w as i64 && yi + 2 < self.h as i64;
        for (j, wyj) in wy.iter().enumerate() {
            for (i, wxi) in wx.iter().enumerate() {
                let p = if inside { self.px_at((yi as usize + j - 1) * self.w + xi as usize + i - 1) } else { self.at(xi + i as i64 - 1, yi + j as i64 - 1) };
                let w = wxi * wyj;
                for (o, p) in o.iter_mut().zip(p) {
                    *o += p * w;
                }
            }
        }
        let n = [self.at(xi, yi), self.at(xi + 1, yi), self.at(xi, yi + 1), self.at(xi + 1, yi + 1)];
        for (c, o) in o.iter_mut().enumerate() {
            let lo = n.iter().map(|p| get(p, c)).fold(f32::MAX, f32::min);
            let hi = n.iter().map(|p| get(p, c)).fold(f32::MIN, f32::max);
            *o = o.clamp(lo, hi);
        }
        o
    }

    pub fn map<F: Fn([f32; 4]) -> [f32; 4] + Sync>(&self, f: F) -> Tex {
        let mut out = self.clone();
        par_rows(&mut out.px, self.w.max(1), |_, chunk| {
            for p in chunk.iter_mut() {
                *p = f(*p);
            }
        });
        out
    }

    pub fn gray_at(&self, x: i64, y: i64) -> f32 {
        lum(self.at(x, y))
    }
}

/// Run `f(first_index, chunk)` on consecutive pieces of `px` in parallel. Pieces are 16 whole rows of width `w`, and a
/// cancelled evaluation stops between pieces, so it gives the CPU back quickly.
pub fn par_rows<F: Fn(usize, &mut [[f32; 4]]) + Sync>(px: &mut [[f32; 4]], w: usize, f: F) {
    let cancel = &crate::context::current_cancel();
    crate::par::for_each_run_mut(px, w.max(1) * 16, |first, piece| {
        if !crate::context::is_set(cancel) {
            f(first, piece);
        }
    });
}

/// Parallel map over 0..n, preserving order.
pub fn par_map<T: Send + Default + Clone, F: Fn(usize) -> T + Sync>(n: usize, f: F) -> Vec<T> {
    let mut out = vec![T::default(); n];
    if n == 0 {
        return out;
    }
    let cancel = &crate::context::current_cancel();
    // runs of at most 4096 items, so a cancelled evaluation stops soon
    let run = n.div_ceil((crate::par::helpers() + 1) * 4).clamp(1, 4096);
    crate::par::for_each_run_mut(&mut out, run, |first, chunk| {
        if crate::context::is_set(cancel) {
            return;
        }
        for (i, slot) in chunk.iter_mut().enumerate() {
            *slot = f(first + i);
        }
    });
    out
}

#[cfg(test)]
mod fast_path_tests {
    use super::*;

    /// Values around every kind of edge: zeros of both signs, halves, integers, the cast's limits, specials, and a sweep.
    fn edges() -> Vec<f64> {
        let mut v = vec![
            0.0,
            -0.0,
            0.5,
            -0.5,
            1.0,
            -1.0,
            1e-300,
            -1e-300,
            f64::MIN_POSITIVE,
            f64::EPSILON,
            4.5e15,
            -4.5e15,
            4.611_686_018_427_388e18,
            -9.3e18,
            1e300,
            -1e300,
        ];
        v.extend([f64::INFINITY, f64::NEG_INFINITY, f64::NAN, f64::MAX, f64::MIN]);
        for k in -70i32..70 {
            let p = crate::math::powi(2f64, k);
            v.extend([p, -p, p + 0.5, -p - 0.5, p.next_up(), p.next_down(), -p.next_up(), -p.next_down()]);
        }
        let mut x = 0x1234_5678_9abc_def0u64;
        for _ in 0..200_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            v.push(f64::from_bits(x));
            v.push((x >> 11) as f64 / (1u64 << 40) as f64 - 4096.0);
        }
        v
    }

    #[test]
    fn floor64_is_floor_bit_for_bit() {
        for x in edges() {
            assert_eq!(floor64(x).to_bits(), x.floor().to_bits(), "{x:e}");
        }
    }

    #[test]
    fn tex_at_wraps_as_rem_euclid() {
        let t = Tex::from_fn(5, 3, |u, v| [u, v, u * v, 1.0]);
        for y in -7..8i64 {
            for x in -11..12i64 {
                assert_eq!(t.at(x, y), t.px[y.rem_euclid(3) as usize * 5 + x.rem_euclid(5) as usize], "{x} {y}");
            }
        }
    }
}
