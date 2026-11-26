//! The built-in alpha library: brush stamp shapes generated in code (no image files). Each alpha
//! is rendered once, on first use, into a mip-mapped grayscale image that strokes sample
//! bilinearly at the level matching the stamp size, so a library alpha costs no more per texel
//! than a procedural one and nothing is allocated per stamp.

use std::collections::{BTreeMap, VecDeque};
use std::f32::consts::{PI, TAU};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use texturecraft_texel::noise;

use crate::brush::smoothstep;

/// Size in pixels of the finest mip level.
pub const BASE: usize = 512;
/// Size of the coarsest mip level.
const MIN_LEVEL: usize = 4;
/// Edge anti-aliasing width in stamp units (about 1.5 base pixels; the stamp spans −1..1).
const AA: f32 = 3.0 / BASE as f32;
const SEED: u32 = 0x7C0A;

/// A shape over the stamp square: local coordinates −1..1 (y up) → coverage 0..1.
type Gen = Box<dyn Fn(f32, f32) -> f32 + Send + Sync>;

/// Generator settings saved with the selected library alpha in a brush.
pub type LibraryAlphaParams = BTreeMap<AlphaControl, f32>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AlphaControl {
    Seed,
    Scale,
    Density,
    Detail,
    Breakup,
    Count,
    Thickness,
    Softness,
}

impl AlphaControl {
    pub fn key(self) -> &'static str {
        match self {
            Self::Seed => "seed",
            Self::Scale => "scale",
            Self::Density => "density",
            Self::Detail => "detail",
            Self::Breakup => "breakup",
            Self::Count => "count",
            Self::Thickness => "thickness",
            Self::Softness => "softness",
        }
    }

    pub fn whole_number(self) -> bool {
        matches!(self, Self::Seed | Self::Detail | Self::Count)
    }
}

/// The meaning, range and default of a single library generator control.
#[derive(Clone, Copy, Debug)]
pub struct AlphaOption {
    pub control: AlphaControl,
    pub label: &'static str,
    pub minimum: f32,
    pub maximum: f32,
    pub default: f32,
}

impl AlphaOption {
    const fn new(control: AlphaControl, label: &'static str, minimum: f32, maximum: f32, default: f32) -> Self {
        Self { control, label, minimum, maximum, default }
    }

    pub fn value(self, params: &LibraryAlphaParams) -> f32 {
        let value = params.get(&self.control).copied().filter(|value| value.is_finite()).unwrap_or(self.default);
        let value = value.clamp(self.minimum, self.maximum);
        if self.control.whole_number() { value.round() } else { value }
    }
}

const SEED_OPTION: AlphaOption = AlphaOption::new(AlphaControl::Seed, "Seed", 0.0, 65535.0, 0.0);
const SCALE: AlphaOption = AlphaOption::new(AlphaControl::Scale, "Detail scale", 0.25, 4.0, 1.0);
const DENSITY: AlphaOption = AlphaOption::new(AlphaControl::Density, "Density", 0.25, 2.0, 1.0);
const BREAKUP: AlphaOption = AlphaOption::new(AlphaControl::Breakup, "Breakup", 0.0, 2.0, 1.0);
const THICKNESS: AlphaOption = AlphaOption::new(AlphaControl::Thickness, "Thickness", 0.25, 2.0, 1.0);
const SOFTNESS: AlphaOption = AlphaOption::new(AlphaControl::Softness, "Edge softness", 0.25, 4.0, 1.0);
const DOT_ROWS: AlphaOption = AlphaOption::new(AlphaControl::Count, "Dots per row", 1.0, 12.0, 4.0);
const SCRATCH_COUNT: AlphaOption = AlphaOption::new(AlphaControl::Count, "Scratch count", 1.0, 64.0, 16.0);
const SPLAT_COUNT: AlphaOption = AlphaOption::new(AlphaControl::Count, "Drop count", 0.0, 96.0, 28.0);
const FUR_COUNT: AlphaOption = AlphaOption::new(AlphaControl::Count, "Strand count", 1.0, 96.0, 40.0);
const VEIN_COUNT: AlphaOption = AlphaOption::new(AlphaControl::Count, "Vein count", 1.0, 24.0, 6.0);
const STITCH_COUNT: AlphaOption = AlphaOption::new(AlphaControl::Count, "Stitch count", 1.0, 20.0, 6.0);
const VENT_COUNT: AlphaOption = AlphaOption::new(AlphaControl::Count, "Slot count", 1.0, 16.0, 5.0);
const CHIP_DETAIL: AlphaOption = AlphaOption::new(AlphaControl::Detail, "Noise octaves", 1.0, 7.0, 3.0);
const SPOT_DETAIL: AlphaOption = AlphaOption::new(AlphaControl::Detail, "Noise octaves", 1.0, 7.0, 5.0);
const BOLT_SIDES: AlphaOption = AlphaOption::new(AlphaControl::Count, "Head sides", 3.0, 12.0, 6.0);

enum Maker {
    Shape(fn(&LibraryAlphaParams) -> Gen),
    /// Strokes of a stencil glyph (see [`GLYPHS`]).
    Glyph(&'static str),
}

/// One alpha of the library.
pub struct LibraryAlpha {
    pub id: String,
    pub label: String,
    /// Shelf group ("Basic", "Grunge", …).
    pub category: &'static str,
    maker: Maker,
    options: &'static [AlphaOption],
    image: OnceLock<Arc<AlphaImage>>,
}

impl LibraryAlpha {
    pub fn options(&self) -> &'static [AlphaOption] {
        self.options
    }

    /// Keep only this alpha's valid non-default settings.
    pub fn sanitized_parameters(&self, params: &LibraryAlphaParams) -> LibraryAlphaParams {
        self.options
            .iter()
            .filter_map(|option| {
                let value = option.value(params);
                (value != option.default).then_some((option.control, value))
            })
            .collect()
    }

    /// The default mip-mapped image (rendered on first use).
    pub fn image(&self) -> &AlphaImage {
        self.default_image()
    }

    fn default_image(&self) -> &Arc<AlphaImage> {
        self.image.get_or_init(|| Arc::new(AlphaImage::new(BASE, self.render(BASE, 1))))
    }

    /// Render once per recent customized alpha, and share the result across strokes and previews.
    pub fn configured_image(&self, params: &LibraryAlphaParams) -> Arc<AlphaImage> {
        let params = self.sanitized_parameters(params);
        if params.is_empty() {
            return self.default_image().clone();
        }
        let result = {
            let mut recent = CONFIGURED_IMAGES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let existing = recent.iter().position(|(id, existing, _)| id == &self.id && existing == &params);
            let entry = existing.and_then(|i| recent.remove(i)).unwrap_or_else(|| (self.id.clone(), params.clone(), Arc::new(OnceLock::new())));
            let result = entry.2.clone();
            recent.push_front(entry);
            recent.truncate(CONFIGURED_IMAGES_KEPT);
            result
        };
        result.get_or_init(|| Arc::new(AlphaImage::new(BASE, self.render_configured(BASE, 1, &params)))).clone()
    }

    /// Render the default alpha at `size`² pixels (row-major, top row first) with `ss`² samples per pixel.
    pub fn render(&self, size: usize, ss: usize) -> Vec<f32> {
        self.render_configured(size, ss, &LibraryAlphaParams::new())
    }

    /// As [`Self::render`], with generator parameters applied before mip-mapping or sampling.
    pub fn render_configured(&self, size: usize, ss: usize, params: &LibraryAlphaParams) -> Vec<f32> {
        let g = match self.maker {
            Maker::Shape(make) => make(params),
            Maker::Glyph(strokes) => glyph(strokes, THICKNESS.value(params)),
        };
        let (size, ss) = (size.clamp(1, 4096), ss.clamp(1, 4));
        let mut px = vec![0.0f32; size * size];
        px.par_chunks_mut(size).enumerate().for_each(|(y, row)| {
            for (x, out) in row.iter_mut().enumerate() {
                let mut sum = 0.0;
                for i in 0..ss * ss {
                    let sx = (x as f32 + ((i % ss) as f32 + 0.5) / ss as f32) / size as f32;
                    let sy = (y as f32 + ((i / ss) as f32 + 0.5) / ss as f32) / size as f32;
                    sum += g(sx * 2.0 - 1.0, 1.0 - sy * 2.0);
                }
                let v = sum / (ss * ss) as f32;
                *out = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
            }
        });
        px
    }
}

// Sliders can produce many intermediate settings. Keep a small number of custom images globally
// and let active strokes keep their own reference across eviction.
const CONFIGURED_IMAGES_KEPT: usize = 4;
type CachedAlpha = (String, LibraryAlphaParams, Arc<OnceLock<Arc<AlphaImage>>>);
static CONFIGURED_IMAGES: Mutex<VecDeque<CachedAlpha>> = Mutex::new(VecDeque::new());

/// A grayscale alpha with its mip chain (level 0 is the finest).
pub struct AlphaImage {
    /// (size, row-major pixels) per level.
    levels: Vec<(usize, Vec<f32>)>,
}

impl AlphaImage {
    fn new(size: usize, px: Vec<f32>) -> AlphaImage {
        let mut levels = vec![(size, px)];
        while let Some((n, prev)) = levels.last()
            && n / 2 >= MIN_LEVEL
        {
            let (n, h) = (*n, n / 2);
            let at = |x: usize, y: usize| prev.get(y * n + x).copied().unwrap_or(0.0);
            let next = (0..h * h).map(|i| (i % h * 2, i / h * 2)).map(|(x, y)| (at(x, y) + at(x + 1, y) + at(x, y + 1) + at(x + 1, y + 1)) * 0.25).collect();
            levels.push((h, next));
        }
        AlphaImage { levels }
    }

    /// An alpha from a `width × height` grey image (rows top first, values clamped to 0..1, NaN as 0), stretched over
    /// the stamp square at [`BASE`] pixels (box-filtered where the image is finer, bilinear where it is coarser), with
    /// its mip chain. For shapes made outside the library, such as an imported brush's tip; `None` for an empty image
    /// or a pixel count that does not match.
    pub fn from_pixels(width: usize, height: usize, px: &[f32]) -> Option<AlphaImage> {
        if width == 0 || height == 0 || width.checked_mul(height) != Some(px.len()) {
            return None;
        }
        let at = |x: usize, y: usize| px.get(y.min(height - 1) * width + x.min(width - 1)).map_or(0.0, |v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
        let n = BASE;
        let (sx, sy) = (width as f32 / n as f32, height as f32 / n as f32);
        let mut out = vec![0.0f32; n * n];
        out.par_chunks_mut(n).enumerate().for_each(|(y, row)| {
            for (x, o) in row.iter_mut().enumerate() {
                let (x0, x1, y0, y1) = (x as f32 * sx, (x + 1) as f32 * sx, y as f32 * sy, (y + 1) as f32 * sy);
                *o = if sx >= 1.0 || sy >= 1.0 {
                    // the source pixels this pixel covers
                    let (ax, ay) = (x0.floor() as usize, y0.floor() as usize);
                    let (bx, by) = ((x1.ceil() as usize).max(ax + 1), (y1.ceil() as usize).max(ay + 1));
                    let (mut sum, mut count) = (0.0, 0.0);
                    for py in ay..by {
                        for px in ax..bx {
                            sum += at(px, py);
                            count += 1.0;
                        }
                    }
                    sum / count
                } else {
                    let (fx, fy) = (((x0 + x1) * 0.5 - 0.5).max(0.0), ((y0 + y1) * 0.5 - 0.5).max(0.0));
                    let (ix, iy) = (fx.floor(), fy.floor());
                    let (tx, ty) = (fx - ix, fy - iy);
                    let (ix, iy) = (ix as usize, iy as usize);
                    let top = at(ix, iy) + (at(ix + 1, iy) - at(ix, iy)) * tx;
                    let bottom = at(ix, iy + 1) + (at(ix + 1, iy + 1) - at(ix, iy + 1)) * tx;
                    top + (bottom - top) * ty
                };
            }
        });
        Some(AlphaImage::new(n, out))
    }

    pub fn levels(&self) -> usize {
        self.levels.len()
    }

    /// Grayscale texels of each mip, finest first, for the renderer's tip texture.
    pub fn mip_pixels(&self) -> impl Iterator<Item = (u32, &[f32])> {
        self.levels.iter().map(|(size, pixels)| (*size as u32, pixels.as_slice()))
    }

    /// The coarsest level still at least as fine as a stamp of `radius` texels.
    pub fn level_for(&self, radius: f32) -> usize {
        let d = radius * 2.0;
        if !d.is_finite() {
            return 0;
        }
        let (mut level, mut size) = (0, BASE as f32);
        while level + 1 < self.levels.len() && size * 0.5 >= d {
            size *= 0.5;
            level += 1;
        }
        level
    }

    /// Bilinear sample of `level` at local stamp coordinates (−1..1, y up); 0 outside.
    pub fn sample(&self, level: usize, lx: f32, ly: f32) -> f32 {
        let Some((n, px)) = self.levels.get(level).or(self.levels.last()) else { return 0.0 };
        let nf = *n as f32;
        let fx = (lx * 0.5 + 0.5) * nf - 0.5;
        let fy = (0.5 - ly * 0.5) * nf - 0.5;
        if !(fx > -1.0 && fx < nf && fy > -1.0 && fy < nf) {
            return 0.0;
        }
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let n = *n as isize;
        let at = |x: isize, y: isize| if x < 0 || y < 0 || x >= n || y >= n { 0.0 } else { px.get((y * n + x) as usize).copied().unwrap_or(0.0) };
        let top = at(x0, y0) + (at(x0 + 1, y0) - at(x0, y0)) * tx;
        let bottom = at(x0, y0 + 1) + (at(x0 + 1, y0 + 1) - at(x0, y0 + 1)) * tx;
        top + (bottom - top) * ty
    }
}

static LIBRARY: LazyLock<Vec<LibraryAlpha>> = LazyLock::new(|| {
    let entry = |id: String, label: String, category, options, maker| LibraryAlpha { id, label, category, maker, options, image: OnceLock::new() };
    let shapes = SHAPES.iter().map(|&(id, label, cat, options, make)| entry(id.into(), label.into(), cat, options, Maker::Shape(make)));
    let glyphs = GLYPHS
        .iter()
        .map(|&(c, strokes)| entry(format!("letter_{}", c.to_ascii_lowercase()), format!("Stencil {c}"), LETTERS, &[THICKNESS], Maker::Glyph(strokes)));
    shapes.chain(glyphs).collect()
});

/// Every library alpha, in shelf order.
pub fn library() -> &'static [LibraryAlpha] {
    &LIBRARY
}

/// A library alpha by id or label (case, spaces, `_` and `-` ignored).
pub fn find(name: &str) -> Option<&'static LibraryAlpha> {
    if let Some(alpha) = library().iter().find(|alpha| alpha.id == name) {
        return Some(alpha);
    }
    let n = |x: &str| x.to_ascii_lowercase().replace([' ', '_', '-'], "");
    let want = n(name);
    library().iter().find(|a| n(&a.id) == want || n(&a.label) == want)
}

const BASIC: &str = "Basic";
const GRUNGE: &str = "Grunge";
const ORGANIC: &str = "Organic";
const HARD: &str = "Hard surface";
const LETTERS: &str = "Stencil letters";

/// (id, label, category, generator).
type ShapeDef = (&'static str, &'static str, &'static str, &'static [AlphaOption], fn(&LibraryAlphaParams) -> Gen);

const SHAPES: [ShapeDef; 31] = [
    ("round_soft", "Soft round", BASIC, &[SOFTNESS], round_soft),
    ("round_hard", "Hard round", BASIC, &[SOFTNESS], round_hard),
    ("square", "Square", BASIC, &[SOFTNESS], square),
    ("rectangle", "Rectangle", BASIC, &[SOFTNESS], rectangle),
    ("triangle", "Triangle", BASIC, &[SOFTNESS], triangle),
    ("hexagon", "Hexagon", BASIC, &[SOFTNESS], hexagon),
    ("ring", "Ring", BASIC, &[THICKNESS], ring),
    ("cross", "Cross", BASIC, &[THICKNESS], cross),
    ("star_5", "Star", BASIC, &[THICKNESS], star5),
    ("star_4", "Sparkle", BASIC, &[THICKNESS], star4),
    ("star_6", "Star 6", BASIC, &[THICKNESS], star6),
    ("arrow", "Arrow", BASIC, &[THICKNESS], arrow),
    ("dots_grid", "Dots grid", BASIC, &[DOT_ROWS, THICKNESS], dots_grid),
    ("scratches", "Scratches", GRUNGE, &[SEED_OPTION, SCRATCH_COUNT, THICKNESS, BREAKUP], scratches),
    ("chips", "Chips", GRUNGE, &[SEED_OPTION, SCALE, DENSITY, CHIP_DETAIL, BREAKUP], chips),
    ("splatter", "Splatter", GRUNGE, &[SEED_OPTION, SPLAT_COUNT, THICKNESS, BREAKUP], splatter),
    ("sponge", "Sponge", GRUNGE, &[SEED_OPTION, SCALE, DENSITY, BREAKUP], sponge),
    ("grunge_spots", "Grunge spots", GRUNGE, &[SEED_OPTION, SCALE, DENSITY, SPOT_DETAIL], grunge_spots),
    ("cracks", "Cracks", GRUNGE, &[SEED_OPTION, SCALE, THICKNESS, BREAKUP], cracks),
    ("speckle", "Speckle", GRUNGE, &[SEED_OPTION, SCALE, DENSITY], speckle),
    ("fur", "Fur strands", ORGANIC, &[SEED_OPTION, FUR_COUNT, THICKNESS], fur),
    ("leaf", "Leaf", ORGANIC, &[VEIN_COUNT, THICKNESS], leaf),
    ("wood_knot", "Wood knot", ORGANIC, &[SEED_OPTION, SCALE, BREAKUP], wood_knot),
    ("rivet", "Rivet", HARD, &[SOFTNESS], rivet),
    ("bolt_head", "Bolt head", HARD, &[BOLT_SIDES, SOFTNESS], bolt_head),
    ("screw_slot", "Screw slot", HARD, &[THICKNESS], screw_slot),
    ("screw_cross", "Screw cross", HARD, &[THICKNESS], screw_cross),
    ("stitches", "Stitches", HARD, &[STITCH_COUNT, THICKNESS], stitches),
    ("vent", "Vent slots", HARD, &[VENT_COUNT, THICKNESS], vent),
    ("fabric", "Fabric stamp", HARD, &[SCALE], fabric),
    ("bricks", "Bricks stamp", HARD, &[SEED_OPTION, SCALE, THICKNESS], bricks),
];

// ---------------------------------------------------------------------------------------------
// Helpers

/// Coverage of a signed distance (negative inside), anti-aliased over [`AA`].
fn fill(d: f32) -> f32 {
    soft_fill(d, 1.0)
}

fn soft_fill(d: f32, softness: f32) -> f32 {
    (0.5 - d / (AA * softness)).clamp(0.0, 1.0)
}

fn len(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

/// Distance from (px, py) to segment a–b, and the position 0..1 of the closest point along it.
fn seg(px: f32, py: f32, a: [f32; 2], b: [f32; 2]) -> (f32, f32) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let t = (((px - a[0]) * dx + (py - a[1]) * dy) / (dx * dx + dy * dy).max(1e-12)).clamp(0.0, 1.0);
    (len(px - a[0] - dx * t, py - a[1] - dy * t), t)
}

/// Signed distance to a regular `n`-gon with apothem `a`, its first edge normal at `phase`.
fn ngon(x: f32, y: f32, n: u32, a: f32, phase: f32) -> f32 {
    (0..n).map(|k| phase + k as f32 * TAU / n as f32).map(|t| x * t.cos() + y * t.sin()).fold(f32::MIN, f32::max) - a
}

/// A `k`-pointed star (one point up): edge radius from `inner` (valleys) to `outer` (points),
/// `curve` > 1 makes thinner rays.
fn star(x: f32, y: f32, k: u32, inner: f32, outer: f32, curve: f32) -> f32 {
    let t = ((x.atan2(y) / (TAU / k as f32)).rem_euclid(1.0) - 0.5).abs() * 2.0;
    fill(len(x, y) - (inner + (outer - inner) * t.powf(curve)))
}

/// Soft radial fade towards the stamp edge (keeps grunge alphas from cutting off square).
fn fade(x: f32, y: f32) -> f32 {
    1.0 - smoothstep(0.55, 0.97, len(x, y))
}

/// A dome (rivet/screw head profile): a hard rim rising to 1 at the center.
fn dome(r: f32, radius: f32, softness: f32) -> f32 {
    let q = r / radius;
    0.35 * soft_fill(r - radius, softness) + 0.65 * (1.0 - q * q).max(0.0).sqrt()
}

/// Deterministic random 0..1 for item `i`, property `k`.
fn random_seed(params: &LibraryAlphaParams) -> u32 {
    SEED + SEED_OPTION.value(params) as u32
}

fn rnd(i: u32, k: u32, seed: u32) -> f32 {
    noise::cell_hash(i as i32, k as i32, 0, seed)
}

fn fbm(x: f32, y: f32, z: f32, octaves: u32, seed: u32) -> f32 {
    noise::fbm([x, y, z], octaves, seed, noise::perlin)
}

// ---------------------------------------------------------------------------------------------
// Basic shapes

fn round_soft(params: &LibraryAlphaParams) -> Gen {
    let softness = SOFTNESS.value(params);
    Box::new(move |x, y| {
        let coverage = (1.0 - (x * x + y * y)).max(0.0);
        if softness == 1.0 { coverage.powi(2) } else { coverage.powf(2.0 / softness) }
    })
}

fn round_hard(params: &LibraryAlphaParams) -> Gen {
    let softness = SOFTNESS.value(params);
    Box::new(move |x, y| soft_fill(len(x, y) - 0.95, softness))
}

fn square(params: &LibraryAlphaParams) -> Gen {
    let softness = SOFTNESS.value(params);
    Box::new(move |x, y| soft_fill(x.abs().max(y.abs()) - 0.9, softness))
}

fn rectangle(params: &LibraryAlphaParams) -> Gen {
    let softness = SOFTNESS.value(params);
    Box::new(move |x, y| soft_fill((x.abs() - 0.95).max(y.abs() - 0.45), softness))
}

fn triangle(params: &LibraryAlphaParams) -> Gen {
    // Point up, centered on its bounding box.
    let softness = SOFTNESS.value(params);
    Box::new(move |x, y| soft_fill(ngon(x, y + 0.24, 3, 0.475, -PI / 2.0), softness))
}

fn hexagon(params: &LibraryAlphaParams) -> Gen {
    let softness = SOFTNESS.value(params);
    Box::new(move |x, y| soft_fill(ngon(x, y, 6, 0.82, 0.0), softness))
}

fn ring(params: &LibraryAlphaParams) -> Gen {
    let half_width = 0.2 * THICKNESS.value(params);
    Box::new(move |x, y| fill((len(x, y) - 0.72).abs() - half_width))
}

fn cross(params: &LibraryAlphaParams) -> Gen {
    let half_width = 0.22 * THICKNESS.value(params);
    Box::new(move |x, y| fill((x.abs() - half_width).max(y.abs() - 0.92).min((x.abs() - 0.92).max(y.abs() - half_width))))
}

fn star5(params: &LibraryAlphaParams) -> Gen {
    let inner = (0.4 * THICKNESS.value(params)).min(0.9);
    Box::new(move |x, y| star(x, y + 0.06, 5, inner, 0.97, 1.0))
}

fn star4(params: &LibraryAlphaParams) -> Gen {
    let inner = (0.1 * THICKNESS.value(params)).min(0.9);
    Box::new(move |x, y| star(x, y, 4, inner, 0.97, 3.0))
}

fn star6(params: &LibraryAlphaParams) -> Gen {
    let inner = (0.55 * THICKNESS.value(params)).min(0.9);
    Box::new(move |x, y| star(x, y, 6, inner, 0.95, 1.0))
}

fn arrow(params: &LibraryAlphaParams) -> Gen {
    let half_width = 0.16 * THICKNESS.value(params);
    Box::new(move |x, y| fill((x.abs() - half_width).max(y - 0.15).max(-0.9 - y).min(ngon(x, y - 0.35, 3, 0.25, -PI / 2.0))))
}

fn dots_grid(params: &LibraryAlphaParams) -> Gen {
    let count = DOT_ROWS.value(params);
    let step = 1.8 / count;
    let radius = (0.14 * THICKNESS.value(params)).min(step * 0.48);
    let offset = if count == DOT_ROWS.default { 0.675 } else { 0.9 - step * 0.5 };
    let center = move |v: f32| ((v + 0.9) / step).floor().clamp(0.0, count - 1.0) * step - offset;
    Box::new(move |x, y| fill(len(x - center(x), y - center(y)) - radius))
}

// ---------------------------------------------------------------------------------------------
// Grunge

fn scratches(params: &LibraryAlphaParams) -> Gen {
    let seed = random_seed(params);
    let count = SCRATCH_COUNT.value(params) as u32;
    let width = THICKNESS.value(params);
    let breakup = BREAKUP.value(params);
    // [a, b, half width, intensity] of thin lines along a dominant direction.
    let lines: Vec<([f32; 2], [f32; 2], f32, f32)> = (0..count)
        .map(|i| {
            let (cx, cy) = (rnd(i, 0, seed) * 1.2 - 0.6, rnd(i, 1, seed) * 1.2 - 0.6);
            let ang = 0.5 + (rnd(i, 2, seed) - 0.5) * 0.6 * breakup;
            let half = 0.15 + rnd(i, 3, seed) * 0.4;
            let (dx, dy) = (ang.cos() * half, ang.sin() * half);
            ([cx - dx, cy - dy], [cx + dx, cy + dy], (0.004 + rnd(i, 4, seed) * 0.012) * width, 0.5 + rnd(i, 5, seed) * 0.5)
        })
        .collect();
    Box::new(move |x, y| {
        let m = lines.iter().fold(0.0f32, |m, &(a, b, w, k)| {
            let (d, t) = seg(x, y, a, b);
            // Tapered ends.
            m.max(fill(d - w * (1.0 - (t * 2.0 - 1.0).powi(2)).sqrt()) * k)
        });
        m * fade(x, y)
    })
}

fn chips(params: &LibraryAlphaParams) -> Gen {
    let seed = random_seed(params);
    let scale = SCALE.value(params);
    let density = DENSITY.value(params);
    let detail = CHIP_DETAIL.value(params) as u32;
    let breakup = BREAKUP.value(params);
    Box::new(move |x, y| {
        let (u, v) = (x * scale, y * scale);
        let w = fbm(u * 3.0, v * 3.0, 1.7, detail, seed) - 0.5;
        let (f1, f2, id) = noise::cellular([u * 3.5 + w * 0.8 * breakup, v * 3.5 + w * 0.8 * breakup, 0.5], seed);
        if id >= 0.45 * density {
            return 0.0;
        }
        let rough = noise::fbm([u * 14.0, v * 14.0, 2.3], 3, seed + 1, noise::value);
        fill(0.1 + 0.12 * rough - (f2 - f1)) * fade(x, y)
    })
}

fn splatter(params: &LibraryAlphaParams) -> Gen {
    let seed = random_seed(params);
    let count = SPLAT_COUNT.value(params) as u32;
    let thickness = THICKNESS.value(params);
    let breakup = BREAKUP.value(params);
    // Droplets [x, y, radius] around a ragged central blob.
    let drops: Vec<[f32; 3]> = (0..count)
        .map(|i| {
            let (a, d) = (rnd(i, 0, seed) * TAU, 0.42 + rnd(i, 1, seed).powf(0.7) * 0.45);
            [a.cos() * d, a.sin() * d, (0.015 + rnd(i, 2, seed).powi(2) * 0.07) * thickness]
        })
        .collect();
    Box::new(move |x, y| {
        let a = y.atan2(x);
        let edge = 0.36 + 0.3 * (fbm(a.cos() * 1.5 + 4.0, a.sin() * 1.5, 0.0, 3, seed) - 0.5) * breakup;
        let blob = fill(len(x, y) - edge);
        drops.iter().fold(blob, |m, d| m.max(fill(len(x - d[0], y - d[1]) - d[2])))
    })
}

fn sponge(params: &LibraryAlphaParams) -> Gen {
    let seed = random_seed(params);
    let scale = SCALE.value(params);
    let density = DENSITY.value(params);
    let breakup = BREAKUP.value(params);
    Box::new(move |x, y| {
        let (u, v) = (x * scale, y * scale);
        let holes = noise::worley([u * 7.0, v * 7.0, 0.3], seed);
        let fine = noise::worley([u * 15.0, v * 15.0, 1.1], seed + 3);
        let body = fill(len(x, y) - 0.75 - 0.4 * (fbm(u * 2.0, v * 2.0, 0.8, 3, seed) - 0.5) * breakup);
        ((holes - (0.28 - 0.2 * (density - 1.0))) * 5.0).clamp(0.0, 1.0) * ((fine - (0.2 - 0.15 * (density - 1.0))) * 4.0).clamp(0.0, 1.0) * body
    })
}

fn grunge_spots(params: &LibraryAlphaParams) -> Gen {
    let seed = random_seed(params);
    let scale = SCALE.value(params);
    let density = DENSITY.value(params);
    let detail = SPOT_DETAIL.value(params) as u32;
    Box::new(move |x, y| {
        let (u, v) = (x * scale, y * scale);
        let spots = ((fbm(u * 2.6, v * 2.6, 3.1, detail, seed) - (0.55 - 0.2 * (density - 1.0))) * 7.0).clamp(0.0, 1.0);
        let specks = ((noise::value([u * 30.0, v * 30.0, 0.5], seed) - (0.86 - 0.1 * (density - 1.0))) * 8.0).clamp(0.0, 0.6);
        (spots + specks).min(1.0) * fade(x, y)
    })
}

fn cracks(params: &LibraryAlphaParams) -> Gen {
    let seed = random_seed(params);
    let scale = SCALE.value(params);
    let thickness = THICKNESS.value(params);
    let breakup = BREAKUP.value(params);
    Box::new(move |x, y| {
        let (u, v) = (x * scale, y * scale);
        let w = fbm(u * 2.0, v * 2.0, 5.0, 3, seed) - 0.5;
        let (f1, f2, _) = noise::cellular([u * 2.6 + w, v * 2.6 + w * 0.7, 0.2], seed + 5);
        let line = 1.0 - smoothstep(0.015 * thickness, 0.045 * thickness, f2 - f1);
        let interruption = ((noise::fbm([u * 4.0, v * 4.0, 7.0], 3, seed + 2, noise::value) - 0.35 * breakup) * 4.0).clamp(0.0, 1.0);
        line * interruption * fade(x, y)
    })
}

fn speckle(params: &LibraryAlphaParams) -> Gen {
    let seed = random_seed(params);
    let scale = 40.0 * SCALE.value(params);
    let threshold = 0.78 - 0.15 * (DENSITY.value(params) - 1.0);
    Box::new(move |x, y| ((noise::value([x * scale, y * scale, 0.0], seed) - threshold) * 10.0).clamp(0.0, 1.0) * fade(x, y))
}

// ---------------------------------------------------------------------------------------------
// Organic

fn fur(params: &LibraryAlphaParams) -> Gen {
    let seed = random_seed(params);
    let count = FUR_COUNT.value(params) as u32;
    let thickness = THICKNESS.value(params);
    // Strands rising from the bottom and bending sideways: 4 points, half width, bounding box.
    let strands: Vec<([[f32; 2]; 4], f32, [f32; 4])> = (0..count)
        .map(|i| {
            let (x0, y0) = (rnd(i, 0, seed) * 1.4 - 0.7, -0.85 + rnd(i, 1, seed) * 0.3);
            let (length, bend) = (0.8 + rnd(i, 2, seed) * 0.7, (rnd(i, 3, seed) - 0.5) * 0.9);
            let p = [0.0f32, 1.0, 2.0, 3.0].map(|k| {
                let t = k / 3.0;
                [x0 + bend * t * t * length, (y0 + t * length).min(0.95)]
            });
            let w = (0.008 + rnd(i, 4, seed) * 0.01) * thickness;
            let bb = p.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, q| [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])]);
            (p, w, [bb[0] - w, bb[1] - w, bb[2] + w, bb[3] + w])
        })
        .collect();
    Box::new(move |x, y| {
        let mut m = 0.0f32;
        for (p, w, bb) in &strands {
            if x < bb[0] || y < bb[1] || x > bb[2] || y > bb[3] {
                continue;
            }
            for (k, pair) in p.windows(2).enumerate() {
                if let [a, b] = pair {
                    let (d, t) = seg(x, y, *a, *b);
                    m = m.max(fill(d - w * (1.0 - (k as f32 + t) / 3.0)));
                }
            }
        }
        m
    })
}

fn leaf(params: &LibraryAlphaParams) -> Gen {
    let count = VEIN_COUNT.value(params);
    let width = THICKNESS.value(params);
    let (s, c) = (PI / 6.0).sin_cos();
    Box::new(move |x, y| {
        // Leaf frame tilted 30°: u across, v along (base at −0.85, tip at 0.85).
        let (u, v) = (x * c - y * s, x * s + y * c);
        let stem = fill(seg(u, v, [0.0, -0.97], [0.0, -0.8]).0 - 0.015);
        let t = (v + 0.85) / 1.7;
        if !(0.0..=1.0).contains(&t) {
            return stem;
        }
        let half = 0.38 * (PI * t).sin().max(0.0).powf(0.8) * (1.0 - 0.3 * t);
        // Midrib and side veins (darker lines).
        let q = (v - u.abs() * 0.8) * count;
        let vein = fill(u.abs() - 0.012 * width).max(fill((q - q.round()).abs() / count - 0.006 * width));
        (fill(u.abs() - half) * (1.0 - 0.45 * vein)).max(stem)
    })
}

fn wood_knot(params: &LibraryAlphaParams) -> Gen {
    let seed = random_seed(params);
    let spacing = 30.0 * SCALE.value(params);
    let breakup = BREAKUP.value(params);
    Box::new(move |x, y| {
        let d = len(x / 0.55, y) + (fbm(x * 2.5, y * 2.5, 9.0, 3, seed) - 0.5) * 0.25 * breakup;
        let rings = 0.5 + 0.5 * (d * spacing).cos();
        let core = 1.0 - smoothstep(0.12, 0.22, d);
        (rings * 0.7).max(core) * (1.0 - smoothstep(0.7, 0.95, d))
    })
}

// ---------------------------------------------------------------------------------------------
// Hard surface

fn rivet(params: &LibraryAlphaParams) -> Gen {
    let softness = SOFTNESS.value(params);
    Box::new(move |x, y| dome(len(x, y), 0.7, softness))
}

fn bolt_head(params: &LibraryAlphaParams) -> Gen {
    let sides = BOLT_SIDES.value(params) as u32;
    let softness = SOFTNESS.value(params);
    Box::new(move |x, y| {
        let r = len(x, y);
        (soft_fill(r - 0.95, softness) * 0.4).max(soft_fill(ngon(x, y, sides, 0.7, 0.0), softness) * 0.75).max(soft_fill(r - 0.6, softness))
    })
}

fn screw_slot(params: &LibraryAlphaParams) -> Gen {
    let width = 0.08 * THICKNESS.value(params);
    Box::new(move |x, y| dome(len(x, y), 0.9, 1.0) * (1.0 - fill((y.abs() - width).max(x.abs() - 0.7))))
}

fn screw_cross(params: &LibraryAlphaParams) -> Gen {
    let width = 0.08 * THICKNESS.value(params);
    Box::new(move |x, y| {
        let cut = (y.abs() - width).max(x.abs() - 0.5).min((x.abs() - width).max(y.abs() - 0.5));
        dome(len(x, y), 0.9, 1.0) * (1.0 - fill(cut))
    })
}

fn stitches(params: &LibraryAlphaParams) -> Gen {
    let count = STITCH_COUNT.value(params) as u32;
    let step = 1.5 / count.saturating_sub(1).max(1) as f32;
    let width = (0.05 * THICKNESS.value(params)).min(step * 0.45);
    let offset = if count == 1 { 0.0 } else { -0.75 };
    Box::new(move |x, y| {
        (0..count).map(|i| offset + i as f32 * step).fold(0.0f32, |m, cx| m.max(fill(seg(x, y, [cx - 0.07, -0.2], [cx + 0.07, 0.2]).0 - width)))
    })
}

fn vent(params: &LibraryAlphaParams) -> Gen {
    let count = VENT_COUNT.value(params) as u32;
    let step = 1.2 / count.saturating_sub(1).max(1) as f32;
    let width = (0.07 * THICKNESS.value(params)).min(step * 0.45);
    let offset = if count == 1 { 0.0 } else { -0.6 };
    Box::new(move |x, y| (0..count).map(|i| offset + i as f32 * step).fold(0.0f32, |m, cy| m.max(fill(seg(x, y, [-0.7, cy], [0.7, cy]).0 - width))))
}

fn fabric(params: &LibraryAlphaParams) -> Gen {
    let scale = 7.0 * SCALE.value(params);
    Box::new(move |x, y| {
        // Plain weave: the thread on top alternates per cell; its profile runs across it.
        let (u, v) = (x * scale, y * scale);
        let over = (u.floor() as i32 + v.floor() as i32).rem_euclid(2) == 0;
        let thread = if over { (PI * v.rem_euclid(1.0)).sin() } else { (PI * u.rem_euclid(1.0)).sin() };
        thread * fill(x.abs().max(y.abs()) - 0.92)
    })
}

fn bricks(params: &LibraryAlphaParams) -> Gen {
    let scale = SCALE.value(params);
    let (sx, sy) = (1.6 * scale, 3.2 * scale);
    let width = THICKNESS.value(params);
    let seed = random_seed(params);
    Box::new(move |x, y| {
        let v = y * sy;
        let row = v.floor();
        let u = x * sx + if (row as i32).rem_euclid(2) == 0 { 0.5 } else { 0.0 };
        let (fu, fv) = (u.rem_euclid(1.0), v.rem_euclid(1.0));
        let d = (((fu - 0.5).abs() - (0.47 - 0.03 * (width - 1.0))) / sx).max(((fv - 0.5).abs() - (0.44 - 0.06 * (width - 1.0))) / sy);
        let tint = 0.7 + 0.3 * noise::cell_hash(u.floor() as i32, row as i32, 0, seed);
        fill(d) * tint * fill(x.abs().max(y.abs()) - 0.92)
    })
}

// ---------------------------------------------------------------------------------------------
// Stencil letters

/// A tiny stroke font on a 5×7 grid: space-separated polylines of `xy` digit pairs, x 0..4
/// left to right, y 0..6 top to bottom.
const GLYPHS: [(char, &str); 36] = [
    ('A', "060110304146 0343"),
    ('B', "06003041423303 3344453606"),
    ('C', "4130100105163645"),
    ('D', "00304145360600"),
    ('E', "40000646 0333"),
    ('F', "400006 0333"),
    ('G', "41301001051636454323"),
    ('H', "0006 4046 0343"),
    ('I', "1030 2026 1636"),
    ('J', "4045361605"),
    ('K', "0006 400346"),
    ('L', "000646"),
    ('M', "0600234046"),
    ('N', "06004640"),
    ('O', "103041453616050110"),
    ('P', "06003041423303"),
    ('Q', "103041453616050110 3346"),
    ('R', "06003041423303 2346"),
    ('S', "413010010213334445361605"),
    ('T', "0040 2026"),
    ('U', "000516364540"),
    ('V', "002640"),
    ('W', "0016233640"),
    ('X', "0046 4006"),
    ('Y', "002340 2326"),
    ('Z', "00400646"),
    ('0', "103041453616050110 0541"),
    ('1', "112026 1636"),
    ('2', "01103041420646"),
    ('3', "01103041423313 334445361605"),
    ('4', "300343 3036"),
    ('5', "400002324345361605"),
    ('6', "413010010516364543321203"),
    ('7', "00404116"),
    ('8', "130201103041423313 130405163645443313"),
    ('9', "4313020110304145361605"),
];

/// A glyph from [`GLYPHS`] strokes, drawn with round caps.
fn glyph(strokes: &str, thickness: f32) -> Gen {
    const UNIT: f32 = 0.27;
    let point = |p: &[u8]| match p {
        [x, y] => Some([(x.wrapping_sub(b'0') as f32 - 2.0) * UNIT, (3.0 - y.wrapping_sub(b'0') as f32) * UNIT]),
        _ => None,
    };
    let segs: Vec<([f32; 2], [f32; 2])> = strokes
        .split_whitespace()
        .flat_map(|run| {
            let pts: Vec<[f32; 2]> = run.as_bytes().chunks(2).filter_map(point).collect();
            pts.windows(2).filter_map(|w| if let [a, b] = w { Some((*a, *b)) } else { None }).collect::<Vec<_>>()
        })
        .collect();
    Box::new(move |x, y| fill(segs.iter().map(|&(a, b)| seg(x, y, a, b).0).fold(f32::MAX, f32::min) - 0.11 * thickness))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_alpha_is_bounded_non_empty_and_deterministic() {
        assert!(library().len() >= 60);
        let mut ids = std::collections::BTreeSet::new();
        for a in library() {
            assert!(ids.insert(a.id.clone()), "duplicate {}", a.id);
            assert_eq!(find(&a.label).map(|f| f.id.as_str()), Some(a.id.as_str()));
            let px = a.render(64, 2);
            assert!(px.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)), "{}", a.id);
            let mean = px.iter().sum::<f32>() / px.len() as f32;
            assert!(mean > 0.01 && px.iter().any(|v| *v > 0.5), "{} is empty ({mean})", a.id);
            assert_eq!(px, a.render(64, 2), "{} is not deterministic", a.id);
            // The cached image agrees with a direct render and is zero outside the stamp.
            let img = a.image();
            assert!(img.levels() >= 6);
            assert_eq!(img.sample(0, 1.2, 0.0), 0.0);
            assert_eq!(img.sample(0, f32::NAN, 0.0), 0.0);
            let last = img.levels() - 1;
            assert!(img.sample(last, 0.0, 0.0).is_finite() && img.sample(99, 0.3, -0.2).is_finite());
        }
        assert!(find("STENCIL a").is_some() && find("letter_7").is_some() && find("nope").is_none());
    }

    #[test]
    fn levels_follow_the_stamp_size() {
        let img = find("round_hard").map(LibraryAlpha::image).unwrap();
        assert_eq!(img.level_for(f32::INFINITY), 0);
        assert_eq!(img.level_for(f32::NAN), 0);
        assert_eq!(img.level_for(BASE as f32), 0);
        assert_eq!(img.level_for(BASE as f32 / 4.0), 1);
        assert_eq!(img.level_for(0.0), img.levels() - 1);
        // A centered hard disc: full in the middle at every level, empty outside.
        for l in 0..img.levels() - 1 {
            assert!(img.sample(l, 0.0, 0.0) > 0.99, "level {l}");
        }
        for l in 0..4 {
            assert!(img.sample(l, 0.0, 0.99) < 0.05, "level {l}");
        }
    }

    #[test]
    fn an_alpha_from_pixels_is_stretched_over_the_stamp_and_refuses_bad_sizes() {
        // left half white, right half black, 4 x 2: stretched over the square
        let px = [1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, f32::NAN];
        let img = AlphaImage::from_pixels(4, 2, &px).unwrap();
        assert!(img.levels() > 4);
        assert!(img.sample(0, -0.5, 0.5) > 0.99 && img.sample(0, -0.5, -0.5) > 0.99);
        assert!(img.sample(0, 0.5, 0.5) < 0.01 && img.sample(0, 0.5, -0.5) < 0.01, "NaN reads as 0");
        assert_eq!(img.sample(0, 1.5, 0.0), 0.0);
        // a finer image is averaged
        let fine = vec![0.25; 1024 * 1024];
        assert!((AlphaImage::from_pixels(1024, 1024, &fine).unwrap().sample(0, 0.1, 0.2) - 0.25).abs() < 1e-4);
        assert!(AlphaImage::from_pixels(0, 2, &[]).is_none());
        assert!(AlphaImage::from_pixels(3, 3, &px).is_none());
    }

    /// Run with `cargo test -p texturecraft-paint --release -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_library_sampling() {
        let img = find("scratches").map(LibraryAlpha::image).unwrap();
        let t = std::time::Instant::now();
        let mut sum = 0.0;
        for i in 0..4_000_000u32 {
            let (x, y) = ((i % 2000) as f32 / 1000.0 - 1.0, (i / 2000) as f32 / 1000.0 - 1.0);
            sum += img.sample(1, x, y);
        }
        let ns = t.elapsed().as_nanos() as f64 / 4e6;
        println!("library alpha sample: {ns:.2} ns ({sum})");
        assert!(ns < 50.0);
    }
}
