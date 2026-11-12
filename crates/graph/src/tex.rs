// SPDX-License-Identifier: MIT OR Apache-2.0
//! Float images with wrapped (tileable) sampling: RGBA ([`Tex`]) or grey ([`Grey`], one value per pixel standing for
//! the opaque RGBA pixel `[v, v, v, 1]`).

/// An image of `w` x `h` pixels of type `P`, row by row.
#[derive(Clone, Debug)]
pub struct Image<P> {
    pub w: usize,
    pub h: usize,
    pub px: Vec<P>,
}

/// A float RGBA image.
pub type Tex = Image<[f32; 4]>;

/// A grey, opaque float image: pixel `v` is the RGBA pixel `[v, v, v, 1]`. Every operation on it applies to `v` the same
/// arithmetic the RGBA form applies to each colour channel, so expanding a grey result gives bit for bit what the RGBA
/// computation gives on the expanded inputs.
pub type Grey = Image<f32>;

/// A pixel of an [`Image`]: RGBA, or a grey value.
pub trait Pixel: Copy + Send + Sync + std::fmt::Debug + 'static {
    /// What a read past the end of an image gives: transparent black (a grey image reads 0).
    const NONE: Self;
    /// The same function on each pair of channels.
    fn zip(self, o: Self, f: impl Fn(f32, f32) -> f32) -> Self;
    /// The same function on each channel.
    fn each(self, f: impl Fn(f32) -> f32) -> Self;
    /// As an RGBA pixel.
    fn rgba(self) -> [f32; 4];
    /// How many channels it has.
    const CH: usize;
    /// Channel `c` (0 past the last).
    fn ch(self, c: usize) -> f32;
    /// A pixel of channels `f(0)`, `f(1)`, ..
    fn from_ch(f: impl Fn(usize) -> f32) -> Self;
}

impl Pixel for [f32; 4] {
    const NONE: Self = [0.0; 4];
    #[inline(always)]
    fn zip(self, o: Self, f: impl Fn(f32, f32) -> f32) -> Self {
        zip4(self, o, f)
    }
    #[inline(always)]
    fn each(self, f: impl Fn(f32) -> f32) -> Self {
        [f(self[0]), f(self[1]), f(self[2]), f(self[3])]
    }
    #[inline(always)]
    fn rgba(self) -> [f32; 4] {
        self
    }
    const CH: usize = 4;
    #[inline(always)]
    fn ch(self, c: usize) -> f32 {
        get(&self, c)
    }
    #[inline(always)]
    fn from_ch(f: impl Fn(usize) -> f32) -> Self {
        [f(0), f(1), f(2), f(3)]
    }
}

impl Pixel for f32 {
    const NONE: Self = 0.0;
    #[inline(always)]
    fn zip(self, o: Self, f: impl Fn(f32, f32) -> f32) -> Self {
        f(self, o)
    }
    #[inline(always)]
    fn each(self, f: impl Fn(f32) -> f32) -> Self {
        f(self)
    }
    #[inline(always)]
    fn rgba(self) -> [f32; 4] {
        [self, self, self, 1.0]
    }
    const CH: usize = 1;
    #[inline(always)]
    fn ch(self, c: usize) -> f32 {
        if c == 0 { self } else { 0.0 }
    }
    #[inline(always)]
    fn from_ch(f: impl Fn(usize) -> f32) -> Self {
        f(0)
    }
}

/// The bits of 1.0: a grey pixel's alpha.
const ONE_BITS: u32 = 0x3f80_0000;

/// Whether `p` is a grey, opaque pixel bit for bit (the same three colour channels, alpha exactly 1).
#[inline(always)]
pub fn is_grey_px(p: [f32; 4]) -> bool {
    p[0].to_bits() == p[1].to_bits() && p[1].to_bits() == p[2].to_bits() && p[3].to_bits() == ONE_BITS
}

impl Grey {
    /// As an RGBA image: each pixel `[v, v, v, 1]`.
    pub fn expand(&self) -> Tex {
        let mut px = vec![[0.0f32; 4]; self.px.len()];
        crate::par::for_each_run_mut(&mut px, 1 << 15, |first, run| {
            for (i, d) in run.iter_mut().enumerate() {
                *d = get(&self.px, first + i).rgba();
            }
        });
        Tex { w: self.w, h: self.h, px }
    }

    /// `t` as a grey image when every pixel of it is grey and opaque bit for bit ([`is_grey_px`]).
    pub fn of(t: &Tex) -> Option<Grey> {
        // (the first pixel tells most colour images apart at once)
        if !t.px.first().is_none_or(|p| is_grey_px(*p)) {
            return None;
        }
        let mut px = vec![0.0f32; t.px.len()];
        let colour = std::sync::atomic::AtomicBool::new(false);
        crate::par::for_each_run_mut(&mut px, 1 << 15, |first, run| {
            for (i, d) in run.iter_mut().enumerate() {
                let p = t.px_at(first + i);
                if !is_grey_px(p) {
                    colour.store(true, std::sync::atomic::Ordering::Relaxed);
                    return;
                }
                *d = p[0];
            }
        });
        (!colour.load(std::sync::atomic::Ordering::Relaxed)).then_some(Grey { w: t.w, h: t.h, px })
    }
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

impl<P: Pixel> Image<P> {
    pub fn solid(w: usize, h: usize, c: P) -> Image<P> {
        Image { w, h, px: vec![c; w * h] }
    }

    /// Build a texture from a per-pixel function of (u, v) in [0,1). Runs multi-threaded.
    pub fn from_fn<F: Fn(f32, f32) -> P + Sync>(w: usize, h: usize, f: F) -> Image<P> {
        Image::from_fn_with(w, h, || (), |_, u, v| f(u, v))
    }

    /// As [`Tex::from_fn`], with working state: `init` makes it for each band of rows (one per thread), `f` gets it for
    /// each pixel.
    /// What a pixel gets must not depend on the pixels computed with the same state before it.
    pub fn from_fn_with<S, I: Fn() -> S + Sync, F: Fn(&mut S, f32, f32) -> P + Sync>(w: usize, h: usize, init: I, f: F) -> Image<P> {
        let w = w.max(1);
        let h = h.max(1);
        let mut px = vec![P::NONE; w * h];
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
                *p = o.each(san);
            }
        });
        Image { w, h, px }
    }

    /// Pixel `i` (counted row by row), [`Pixel::NONE`] past the end.
    #[inline]
    pub fn px_at(&self, i: usize) -> P {
        self.px.get(i).copied().unwrap_or(P::NONE)
    }

    /// Sets pixel `i` (counted row by row); past the end, nothing.
    #[inline]
    pub fn set_px(&mut self, i: usize, v: P) {
        if let Some(p) = self.px.get_mut(i) {
            *p = v;
        }
    }

    pub fn at(&self, x: i64, y: i64) -> P {
        Texels::at(self, x, y)
    }

    /// Bilinear, wrapped sampling in uv space.
    #[inline]
    pub fn sample(&self, u: f32, v: f32) -> P {
        Texels::sample(self, u, v)
    }

    pub fn map<F: Fn(P) -> P + Sync>(&self, f: F) -> Image<P> {
        let mut out = self.clone();
        par_rows(&mut out.px, self.w.max(1), |_, chunk| {
            for p in chunk.iter_mut() {
                *p = f(*p);
            }
        });
        out
    }
}

impl Tex {
    pub fn gray_fn<F: Fn(f32, f32) -> f32 + Sync>(w: usize, h: usize, f: F) -> Tex {
        Tex::from_fn(w, h, |u, v| {
            let g = f(u, v);
            [g, g, g, 1.0]
        })
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

    pub fn gray_at(&self, x: i64, y: i64) -> f32 {
        lum(self.at(x, y))
    }
}

/// Run `f(first_index, chunk)` on consecutive pieces of `px` in parallel. Pieces are 16 whole rows of width `w`, and a
/// cancelled evaluation stops between pieces, so it gives the CPU back quickly.
pub fn par_rows<T: Send, F: Fn(usize, &mut [T]) + Sync>(px: &mut [T], w: usize, f: F) {
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

/// Images read pixel by pixel: [`Image`]s, and grey images held in steps ([`Grey16`]) read as their values. The reads
/// here are the same arithmetic for every kind.
pub trait Texels: Sync {
    type P: Pixel;
    fn width(&self) -> usize;
    fn height(&self) -> usize;
    /// Pixel `i` (counted row by row), [`Pixel::NONE`] past the end.
    fn texel(&self, i: usize) -> Self::P;

    /// Pixel (x, y), wrapped.
    fn at(&self, x: i64, y: i64) -> Self::P {
        // a branch instead of a division when the pixel is inside, as most are
        let wrap = |i: i64, n: usize| if i >= 0 && (i as usize) < n { i as usize } else { i.rem_euclid((n as i64).max(1)) as usize };
        self.texel(wrap(y, self.height()) * self.width() + wrap(x, self.width()))
    }

    /// Bilinear, wrapped sampling in uv space.
    #[inline]
    fn sample(&self, u: f32, v: f32) -> Self::P {
        let u = if u.is_finite() { u } else { 0.0 };
        let v = if v.is_finite() { v } else { 0.0 };
        let x = u * self.width() as f32 - 0.5;
        let y = v * self.height() as f32 - 0.5;
        let x0 = ffloor(x);
        let y0 = ffloor(y);
        let fx = x - x0;
        let fy = y - y0;
        let (w, h) = (self.width() as i64, self.height() as i64);
        // wrap the two columns and the two rows once (a branch instead of a division in the common case)
        let wrap = |i: i64, n: i64| if i >= 0 && i < n { i } else { i.rem_euclid(n.max(1)) };
        let (xa, ya) = (wrap(x0 as i64, w), wrap(y0 as i64, h));
        let xb = if xa + 1 == w { 0 } else { xa + 1 };
        let yb = if ya + 1 == h { 0 } else { ya + 1 };
        let (r0, r1) = ((ya * w) as usize, (yb * w) as usize);
        let a = self.texel(r0 + xa as usize);
        let b = self.texel(r0 + xb as usize);
        let c = self.texel(r1 + xa as usize);
        let d = self.texel(r1 + xb as usize);
        let top = a.zip(b, |a, b| a + (b - a) * fx);
        let bot = c.zip(d, |c, d| c + (d - c) * fx);
        top.zip(bot, |top, bot| top + (bot - top) * fy)
    }
}

impl<P: Pixel> Texels for Image<P> {
    type P = P;
    #[inline(always)]
    fn width(&self) -> usize {
        self.w
    }
    #[inline(always)]
    fn height(&self) -> usize {
        self.h
    }
    #[inline(always)]
    fn texel(&self, i: usize) -> P {
        self.px_at(i)
    }
}

impl Texels for Grey16 {
    type P = f32;
    #[inline(always)]
    fn width(&self) -> usize {
        self.w
    }
    #[inline(always)]
    fn height(&self) -> usize {
        self.h
    }
    #[inline(always)]
    fn texel(&self, i: usize) -> f32 {
        match self.px.get(i) {
            Some(k) => f32::from(*k) / self.div,
            None => 0.0,
        }
    }
}

/// A grey image held in whole steps: pixel `k` is the grey value `k as f32 / div` (an image quantised to 8 or 16 bits
/// holds nothing else, so it is kept in a eighth of the memory of RGBA floats).
#[derive(Clone, Debug)]
pub struct Grey16 {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u16>,
    pub div: f32,
}

impl Grey16 {
    /// `g` in steps of `1 / div`, when every value of it is exactly `k as f32 / div` for a whole `k` (bit for bit).
    pub(crate) fn of(g: &Grey, div: f32) -> Option<Grey16> {
        let mut px = vec![0u16; g.px.len()];
        let inexact = std::sync::atomic::AtomicBool::new(false);
        crate::par::for_each_run_mut(&mut px, 1 << 15, |first, run| {
            for (i, d) in run.iter_mut().enumerate() {
                let v = get(&g.px, first + i);
                let k = (v * div).round();
                let q = if (0.0..=65535.0).contains(&k) { k as u16 } else { 0 };
                if !(0.0..=65535.0).contains(&k) || (f32::from(q) / div).to_bits() != v.to_bits() {
                    inexact.store(true, std::sync::atomic::Ordering::Relaxed);
                    return;
                }
                *d = q;
            }
        });
        (!inexact.load(std::sync::atomic::Ordering::Relaxed)).then_some(Grey16 { w: g.w, h: g.h, px, div })
    }

    /// The grey values.
    pub(crate) fn widen(&self) -> Grey {
        let mut px = vec![0.0f32; self.px.len()];
        let div = self.div;
        crate::par::for_each_run_mut(&mut px, 1 << 15, |first, run| {
            for (i, d) in run.iter_mut().enumerate() {
                *d = f32::from(get(&self.px, first + i)) / div;
            }
        });
        Grey { w: self.w, h: self.h, px }
    }

    /// As an RGBA image: each pixel `[v, v, v, 1]`.
    pub(crate) fn expand(&self) -> Tex {
        let mut px = vec![[0.0f32; 4]; self.px.len()];
        let div = self.div;
        crate::par::for_each_run_mut(&mut px, 1 << 15, |first, run| {
            for (i, d) in run.iter_mut().enumerate() {
                *d = (f32::from(get(&self.px, first + i)) / div).rgba();
            }
        });
        Tex { w: self.w, h: self.h, px }
    }
}

/// A node result as the runtime holds it: RGBA, or grey (a quarter of the memory) when every pixel is grey and opaque,
/// in whole 8- or 16-bit steps (an eighth) when its format quantises it.
#[derive(Clone, Debug)]
pub(crate) enum Val {
    Rgba(std::sync::Arc<Tex>),
    Grey(std::sync::Arc<Grey>),
    Grey16(std::sync::Arc<Grey16>),
}

impl Val {
    /// `t`, held grey when it is grey bit for bit.
    pub(crate) fn of(t: Tex) -> Val {
        match Grey::of(&t) {
            Some(g) => Val::Grey(std::sync::Arc::new(g)),
            None => Val::Rgba(std::sync::Arc::new(t)),
        }
    }

    /// As an RGBA image (a grey one expanded: a new image).
    pub(crate) fn tex(&self) -> std::sync::Arc<Tex> {
        match self {
            Val::Rgba(t) => t.clone(),
            Val::Grey(g) => std::sync::Arc::new(g.expand()),
            Val::Grey16(g) => std::sync::Arc::new(g.expand()),
        }
    }

    /// Held in steps of `1 / div` when it is grey and exactly that; else as it is.
    pub(crate) fn quantised(self, div: f32) -> Val {
        match &self {
            Val::Grey(g) => match Grey16::of(g, div) {
                Some(q) => Val::Grey16(std::sync::Arc::new(q)),
                None => self,
            },
            _ => self,
        }
    }

    pub(crate) fn dims(&self) -> (usize, usize) {
        match self {
            Val::Rgba(t) => (t.w, t.h),
            Val::Grey(g) => (g.w, g.h),
            Val::Grey16(g) => (g.w, g.h),
        }
    }

    /// Bytes of pixels held.
    pub(crate) fn bytes(&self) -> usize {
        match self {
            Val::Rgba(t) => t.px.len() * 16,
            Val::Grey(g) => g.px.len() * 4,
            Val::Grey16(g) => g.px.len() * 2,
        }
    }

    /// As an image a function reads.
    pub(crate) fn img_ref(&self) -> ImgRef<'_> {
        match self {
            Val::Rgba(t) => ImgRef::Rgba(t),
            Val::Grey(g) => ImgRef::Grey(g),
            Val::Grey16(g) => ImgRef::Grey16(g),
        }
    }

    /// The grey values, however they are held; `None` for RGBA.
    pub(crate) fn grey_in(&self) -> Option<GreyIn<'_>> {
        match self {
            Val::Grey(g) => Some(GreyIn::F(g)),
            Val::Grey16(g) => Some(GreyIn::Q(g)),
            Val::Rgba(_) => None,
        }
    }
}

/// An image a function reads: RGBA, or grey (read as `[v, v, v, 1]`, the same values the expanded image gives).
#[derive(Clone, Copy, Debug)]
pub enum ImgRef<'a> {
    Rgba(&'a Tex),
    Grey(&'a Grey),
    Grey16(&'a Grey16),
}

impl<'a> From<&'a Tex> for ImgRef<'a> {
    fn from(t: &'a Tex) -> ImgRef<'a> {
        ImgRef::Rgba(t)
    }
}

impl ImgRef<'_> {
    pub fn w(&self) -> usize {
        match self {
            ImgRef::Rgba(t) => t.w,
            ImgRef::Grey(t) => t.w,
            ImgRef::Grey16(t) => t.w,
        }
    }

    pub fn h(&self) -> usize {
        match self {
            ImgRef::Rgba(t) => t.h,
            ImgRef::Grey(t) => t.h,
            ImgRef::Grey16(t) => t.h,
        }
    }

    /// Pixel (x, y), wrapped: [`Image::at`].
    pub fn at(&self, x: i64, y: i64) -> [f32; 4] {
        match self {
            ImgRef::Rgba(t) => t.at(x, y),
            ImgRef::Grey(t) => t.at(x, y).rgba(),
            ImgRef::Grey16(t) => Texels::at(*t, x, y).rgba(),
        }
    }

    /// Bilinear, wrapped: [`Image::sample`].
    pub fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        match self {
            ImgRef::Rgba(t) => t.sample(u, v),
            ImgRef::Grey(t) => t.sample(u, v).rgba(),
            ImgRef::Grey16(t) => Texels::sample(*t, u, v).rgba(),
        }
    }

    /// Every pixel in order, as RGBA.
    pub fn for_each_px(&self, mut f: impl FnMut([f32; 4])) {
        match self {
            ImgRef::Rgba(t) => t.px.iter().for_each(|p| f(*p)),
            ImgRef::Grey(t) => t.px.iter().for_each(|p| f(p.rgba())),
            ImgRef::Grey16(t) => (0..t.px.len()).for_each(|i| f(t.texel(i).rgba())),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            ImgRef::Rgba(t) => t.px.len(),
            ImgRef::Grey(t) => t.px.len(),
            ImgRef::Grey16(t) => t.px.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A grey image as held: floats, or steps.
#[derive(Clone, Copy, Debug)]
pub(crate) enum GreyIn<'a> {
    F(&'a Grey),
    Q(&'a Grey16),
}

impl GreyIn<'_> {
    pub(crate) fn len(&self) -> usize {
        match self {
            GreyIn::F(g) => g.px.len(),
            GreyIn::Q(g) => g.px.len(),
        }
    }

    /// As floats (a new image when held in steps).
    pub(crate) fn floats(&self) -> std::borrow::Cow<'_, Grey> {
        match self {
            GreyIn::F(g) => std::borrow::Cow::Borrowed(*g),
            GreyIn::Q(g) => std::borrow::Cow::Owned(g.widen()),
        }
    }
}

impl Texels for GreyIn<'_> {
    type P = f32;
    #[inline(always)]
    fn width(&self) -> usize {
        match self {
            GreyIn::F(g) => g.w,
            GreyIn::Q(g) => g.w,
        }
    }
    #[inline(always)]
    fn height(&self) -> usize {
        match self {
            GreyIn::F(g) => g.h,
            GreyIn::Q(g) => g.h,
        }
    }
    #[inline(always)]
    fn texel(&self, i: usize) -> f32 {
        match self {
            GreyIn::F(g) => g.px_at(i),
            GreyIn::Q(g) => g.texel(i),
        }
    }
}
