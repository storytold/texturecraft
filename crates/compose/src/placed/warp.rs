// The Bézier patch evaluation (`BezierMesh`: its layout, `bernstein`, `locate`, `sample`) is copied from PhotoCraft,
// crates/geom/src/warp.rs, revision 5f65579, generalised to 3D control points with derivatives. See ATTRIBUTION.md.
//
// Copyright (c) 2026 ArtCraft Team and the PhotoCraft contributors.
// Licensed under either of the Apache License, Version 2.0 or the MIT license, at your option.

//! The warp mode of a placed projection ([`PlacedMode::Warp`](texturecraft_core::PlacedMode::Warp)): a grid of
//! `rows × columns` anchors with Bézier handles forms a smooth patch surface in the volume; each anchor carries an image
//! coordinate, so the image is laid on the surface and projected along the grid's z, up to the projection depth on
//! either side, faded by the depth falloff.
//!
//! Grid space runs −1 … 1 across the volume (twice its local coordinates). A regular grid has row 0 at the top (y = 1)
//! and column 0 on the left (x = −1) with image coordinates 0 … 1 the same way, so it shows the image as the planar
//! projection would. A zero handle is automatic: a third of the way towards the neighbour, smoothed through the anchor.
//!
//! The surface's inverse (which surface point lies over a point of the grid's x-y plane) is rasterised once per grid
//! ([`InverseLookup`]) and refined by Newton steps on the exact patch; surfaces are kept for the grids used last.

use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use texturecraft_core::{WarpGrid, WarpPoint};

/// The most rows or columns a grid may have.
pub const MAX_SIDE: u32 = 64;
/// Grid units per local unit of the volume.
pub const GRID_PER_LOCAL: f32 = 2.0;
/// Cells of the inverse lookup along the larger side of the grid.
const LOOKUP_RESOLUTION: usize = 256;
/// Tessellation steps per patch and direction for the lookup.
const STEPS_PER_PATCH: usize = 12;
/// Newton steps per inverse query.
const NEWTON_STEPS: usize = 10;
/// A query counts as on the surface when the patch point lies this close to it in x-y (grid units).
const ON_SURFACE: f64 = 1e-6;
/// Surfaces kept for reuse.
const CACHED: usize = 8;
/// Handle slots of [`WarpPoint::tangents`]: towards the previous and next anchor along u (columns), then along v (rows).
const U_PREV: usize = 0;
const U_NEXT: usize = 1;
const V_PREV: usize = 2;
const V_NEXT: usize = 3;

/// The regular grid of `rows × columns` anchors over −1 … 1 (handles automatic).
pub fn regular_points(rows: u32, columns: u32) -> Vec<WarpPoint> {
    let (r1, c1) = (rows.max(2) - 1, columns.max(2) - 1);
    let mut out = Vec::with_capacity((rows as usize).saturating_mul(columns as usize));
    for r in 0..rows {
        for c in 0..columns {
            let (u, v) = (c as f32 / c1 as f32, r as f32 / r1 as f32);
            out.push(WarpPoint { pos: [2.0 * u - 1.0, 1.0 - 2.0 * v, 0.0], uv: [u, v], tangents: [[0.0; 3]; 4] });
        }
    }
    out
}

/// The grid's anchors row by row (the regular grid when it stores none), or why it cannot be evaluated.
pub fn anchors(grid: &WarpGrid) -> Result<Cow<'_, [WarpPoint]>, String> {
    if !(2..=MAX_SIDE).contains(&grid.rows) || !(2..=MAX_SIDE).contains(&grid.columns) {
        return Err(format!("a warp grid of {} × {} points (2 … {MAX_SIDE} each)", grid.rows, grid.columns));
    }
    let expected = (grid.rows as usize) * (grid.columns as usize);
    if !grid.points.is_empty() && grid.points.len() != expected {
        return Err(format!("a warp grid with {} points instead of {expected}", grid.points.len()));
    }
    let finite = |p: &WarpPoint| p.pos.iter().chain(&p.uv).chain(p.tangents.iter().flatten()).all(|v| v.is_finite());
    if !grid.points.iter().all(finite) {
        return Err("a warp grid with a point that is not a number".to_string());
    }
    Ok(if grid.points.is_empty() { Cow::Owned(regular_points(grid.rows, grid.columns)) } else { Cow::Borrowed(&grid.points) })
}

/// Cubic Bernstein weights at `t`.
fn bernstein(t: f64) -> [f64; 4] {
    let s = 1.0 - t;
    [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t]
}

/// Derivatives of the cubic Bernstein weights at `t`.
fn bernstein_d(t: f64) -> [f64; 4] {
    let s = 1.0 - t;
    [-3.0 * s * s, 3.0 * s * s - 6.0 * s * t, 6.0 * s * t - 3.0 * t * t, 3.0 * t * t]
}

/// A point on the surface with its partial derivatives over the whole parameter square.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SurfaceSample {
    pos: [f64; 3],
    ds: [f64; 3],
    dt: [f64; 3],
}

/// A grid of bicubic Bézier patches with 3D control points over the unit square of `(s, t)`: `us`/`vs` are the patch
/// boundaries (0 … 1), the control points a `(3·(us.len()−1)+1) × (3·(vs.len()−1)+1)` grid, rows along `t`.
/// Indexing never panics: a malformed mesh evaluates to the origin.
#[derive(Debug, Clone, PartialEq)]
struct BezierMesh {
    us: Vec<f64>,
    vs: Vec<f64>,
    points: Vec<[f64; 3]>,
}

impl BezierMesh {
    fn nx(&self) -> usize {
        3 * self.us.len().saturating_sub(1) + 1
    }

    fn ny(&self) -> usize {
        3 * self.vs.len().saturating_sub(1) + 1
    }

    fn is_valid(&self) -> bool {
        let knots_ok = |k: &[f64]| k.len() >= 2 && k.first() == Some(&0.0) && k.last() == Some(&1.0) && k.windows(2).all(|w| matches!(w, [a, b] if b > a));
        knots_ok(&self.us) && knots_ok(&self.vs) && self.points.len() == self.nx() * self.ny() && self.points.iter().all(|p| p.iter().all(|v| v.is_finite()))
    }

    /// The patch containing `s`, the position inside it (0 … 1) and the patch's width.
    fn locate(knots: &[f64], s: f64) -> (usize, f64, f64) {
        let n = knots.len().saturating_sub(1);
        let s = s.clamp(0.0, 1.0);
        let knot = |i: usize| knots.get(i).copied().unwrap_or(1.0);
        let mut i = 0;
        while i + 1 < n && s > knot(i + 1) {
            i += 1;
        }
        let w = knot(i + 1) - knot(i);
        (i, if w > 0.0 { (s - knot(i)) / w } else { 0.0 }, w)
    }

    fn point(&self, i: usize, j: usize) -> [f64; 3] {
        self.points.get(j * self.nx() + i).copied().unwrap_or([0.0; 3])
    }

    fn sample(&self, s: f64, t: f64) -> SurfaceSample {
        let (pi, a, wa) = Self::locate(&self.us, s);
        let (pj, b, wb) = Self::locate(&self.vs, t);
        let (ba, bb, da, db) = (bernstein(a), bernstein(b), bernstein_d(a), bernstein_d(b));
        let mut out = SurfaceSample { pos: [0.0; 3], ds: [0.0; 3], dt: [0.0; 3] };
        for (j, (wj, wdj)) in bb.iter().zip(db).enumerate() {
            for (i, (wi, wdi)) in ba.iter().zip(da).enumerate() {
                let p = self.point(3 * pi + i, 3 * pj + j);
                for (k, c) in p.iter().enumerate() {
                    if let (Some(o), Some(ds), Some(dt)) = (out.pos.get_mut(k), out.ds.get_mut(k), out.dt.get_mut(k)) {
                        *o += c * wi * wj;
                        *ds += c * wdi * wj;
                        *dt += c * wi * wdj;
                    }
                }
            }
        }
        let (ka, kb) = (if wa > 0.0 { 1.0 / wa } else { 0.0 }, if wb > 0.0 { 1.0 / wb } else { 0.0 });
        out.ds = out.ds.map(|v| v * ka);
        out.dt = out.dt.map(|v| v * kb);
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Cell {
    st: [f32; 2],
    z: f32,
}

/// The surface's parameters rasterised over the grid's x-y plane: each cell whose centre a tessellated triangle covers
/// holds the interpolated `(s, t)`, the layer nearest +z winning where the surface folds over itself.
#[derive(Debug, Clone, PartialEq)]
struct InverseLookup {
    origin: [f64; 2],
    cell: f64,
    width: usize,
    height: usize,
    cells: Vec<Option<Cell>>,
}

impl InverseLookup {
    fn empty() -> InverseLookup {
        InverseLookup { origin: [0.0; 2], cell: 1.0, width: 0, height: 0, cells: Vec::new() }
    }

    fn build(mesh: &BezierMesh, resolution: usize) -> InverseLookup {
        let resolution = resolution.clamp(8, 2048);
        let (ns, nt) = (mesh.us.len().saturating_sub(1) * STEPS_PER_PATCH, mesh.vs.len().saturating_sub(1) * STEPS_PER_PATCH);
        if ns == 0 || nt == 0 || !mesh.is_valid() {
            return InverseLookup::empty();
        }
        let mut verts = Vec::with_capacity((ns + 1) * (nt + 1));
        for j in 0..=nt {
            for i in 0..=ns {
                let (s, t) = (i as f64 / ns as f64, j as f64 / nt as f64);
                verts.push(([s, t], mesh.sample(s, t).pos));
            }
        }
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for (_, p) in &verts {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]);
        if extent.is_nan() || extent <= 1e-9 || !extent.is_finite() {
            return InverseLookup::empty();
        }
        let cell = extent / resolution as f64;
        let width = (((hi[0] - lo[0]) / cell).ceil() as usize).clamp(1, resolution) + 1;
        let height = (((hi[1] - lo[1]) / cell).ceil() as usize).clamp(1, resolution) + 1;
        let mut lookup = InverseLookup { origin: [lo[0] - 0.5 * cell, lo[1] - 0.5 * cell], cell, width, height, cells: vec![None; width * height] };
        let vert = |i: usize, j: usize| verts.get(j * (ns + 1) + i).copied();
        for j in 0..nt {
            for i in 0..ns {
                let (Some(a), Some(b), Some(c), Some(d)) = (vert(i, j), vert(i + 1, j), vert(i + 1, j + 1), vert(i, j + 1)) else { continue };
                lookup.triangle([a, b, c]);
                lookup.triangle([a, c, d]);
            }
        }
        lookup
    }

    fn triangle(&mut self, tri: [([f64; 2], [f64; 3]); 3]) {
        let [(sa, a), (sb, b), (sc, c)] = tri;
        let det = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
        if det.abs() < 1e-18 {
            return;
        }
        let to_cell = |v: f64, origin: f64| (v - origin) / self.cell - 0.5;
        let lo = |v: f64, origin: f64| to_cell(v, origin).floor().max(0.0) as usize;
        let hi = |v: f64, origin: f64, n: usize| (to_cell(v, origin).ceil().max(0.0) as usize).min(n.saturating_sub(1));
        let (x0, x1) = (lo(a[0].min(b[0]).min(c[0]), self.origin[0]), hi(a[0].max(b[0]).max(c[0]), self.origin[0], self.width));
        let (y0, y1) = (lo(a[1].min(b[1]).min(c[1]), self.origin[1]), hi(a[1].max(b[1]).max(c[1]), self.origin[1], self.height));
        let eps = -1e-9;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let p = [self.origin[0] + (x as f64 + 0.5) * self.cell, self.origin[1] + (y as f64 + 0.5) * self.cell];
                let wb = ((p[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (p[1] - a[1])) / det;
                let wc = ((b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1])) / det;
                let wa = 1.0 - wb - wc;
                if wa < eps || wb < eps || wc < eps {
                    continue;
                }
                let st = [(wa * sa[0] + wb * sb[0] + wc * sc[0]) as f32, (wa * sa[1] + wb * sb[1] + wc * sc[1]) as f32];
                let z = (wa * a[2] + wb * b[2] + wc * c[2]) as f32;
                if let Some(slot) = self.cells.get_mut(y * self.width + x)
                    && slot.is_none_or(|old| z > old.z)
                {
                    *slot = Some(Cell { st, z });
                }
            }
        }
    }

    /// Starting parameters for a query at `xy`: the cell under it, then its neighbours.
    fn seeds(&self, xy: [f64; 2]) -> [Option<[f64; 2]>; 9] {
        let mut out = [None; 9];
        if self.cells.is_empty() || !xy.iter().all(|v| v.is_finite()) {
            return out;
        }
        let fx = (xy[0] - self.origin[0]) / self.cell;
        let fy = (xy[1] - self.origin[1]) / self.cell;
        if fx < -1.0 || fy < -1.0 || fx > self.width as f64 + 1.0 || fy > self.height as f64 + 1.0 {
            return out;
        }
        let (cx, cy) = (fx.floor() as isize, fy.floor() as isize);
        const ORDER: [(isize, isize); 9] = [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, 1), (1, -1), (-1, -1)];
        for (slot, (dx, dy)) in out.iter_mut().zip(ORDER) {
            let (x, y) = (cx + dx, cy + dy);
            if x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height {
                *slot = self.cells.get(y as usize * self.width + x as usize).copied().flatten().map(|c| c.st.map(f64::from));
            }
        }
        out
    }
}

/// Where a query lands on the surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceHit {
    /// The surface point (grid units).
    pub pos: [f64; 3],
    /// Image coordinate there.
    pub uv: [f64; 2],
    /// Image units per grid unit across x-y (the larger direction), for footprints.
    pub image_per_grid: f64,
}

/// A grid as a patch surface with the image laid on it, and its inverse.
#[derive(Debug, Clone, PartialEq)]
pub struct WarpSurface {
    mesh: BezierMesh,
    rows: usize,
    columns: usize,
    uvs: Vec<[f64; 2]>,
    lookup: InverseLookup,
}

fn f64x3(v: [f32; 3]) -> [f64; 3] {
    v.map(f64::from)
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scaled_diff(a: [f64; 3], b: [f64; 3], k: f64) -> [f64; 3] {
    [(a[0] - b[0]) * k, (a[1] - b[1]) * k, (a[2] - b[2]) * k]
}

/// The handle of anchor `here` towards `next` (`towards_next`) or `prev`: the stored offset, or automatic.
fn handle(stored: [f32; 3], here: [f64; 3], prev: Option<[f64; 3]>, next: Option<[f64; 3]>, towards_next: bool) -> [f64; 3] {
    if stored != [0.0; 3] {
        return f64x3(stored);
    }
    let forward = match (prev, next) {
        (Some(p), Some(n)) => scaled_diff(n, p, 1.0 / 6.0),
        (None, Some(n)) => scaled_diff(n, here, 1.0 / 3.0),
        (Some(p), None) => scaled_diff(here, p, 1.0 / 3.0),
        (None, None) => [0.0; 3],
    };
    if towards_next { forward } else { forward.map(|v| -v) }
}

/// The bicubic control net of `anchors` (`rows × columns`, row by row): each anchor at a patch corner, its handles the
/// control points next to it along the grid lines, the inner control point by a corner the anchor plus both handles.
fn control_net(anchors: &[WarpPoint], rows: usize, columns: usize) -> BezierMesh {
    let (nx, ny) = (3 * (columns - 1) + 1, 3 * (rows - 1) + 1);
    let mut points = vec![[0.0f64; 3]; nx * ny];
    let at = |r: isize, c: isize| -> Option<&WarpPoint> {
        (r >= 0 && c >= 0 && (r as usize) < rows && (c as usize) < columns).then(|| anchors.get(r as usize * columns + c as usize)).flatten()
    };
    let pos = |r: isize, c: isize| at(r, c).map(|p| f64x3(p.pos));
    for r in 0..rows as isize {
        for c in 0..columns as isize {
            let (Some(p), Some(here)) = (at(r, c), pos(r, c)) else { continue };
            let (left, right, up, down) = (pos(r, c - 1), pos(r, c + 1), pos(r - 1, c), pos(r + 1, c));
            let hu =
                |di: isize| if di < 0 { handle(p.tangents[U_PREV], here, left, right, false) } else { handle(p.tangents[U_NEXT], here, left, right, true) };
            let hv = |dj: isize| if dj < 0 { handle(p.tangents[V_PREV], here, up, down, false) } else { handle(p.tangents[V_NEXT], here, up, down, true) };
            let (i, j) = (3 * c, 3 * r);
            let mut put = |di: isize, dj: isize, v: [f64; 3]| {
                let (x, y) = (i + di, j + dj);
                if x >= 0
                    && y >= 0
                    && (x as usize) < nx
                    && (y as usize) < ny
                    && let Some(slot) = points.get_mut(y as usize * nx + x as usize)
                {
                    *slot = v;
                }
            };
            put(0, 0, here);
            for d in [-1, 1] {
                put(d, 0, add(here, hu(d)));
                put(0, d, add(here, hv(d)));
            }
            for dj in [-1, 1] {
                for di in [-1, 1] {
                    put(di, dj, add(add(here, hu(di)), hv(dj)));
                }
            }
        }
    }
    let knots = |n: usize| (0..n).map(|k| k as f64 / (n - 1) as f64).collect();
    BezierMesh { us: knots(columns), vs: knots(rows), points }
}

impl WarpSurface {
    /// The surface of `grid` with its inverse, or why the grid cannot be evaluated.
    pub fn new(grid: &WarpGrid) -> Result<WarpSurface, String> {
        let anchors = anchors(grid)?;
        let (rows, columns) = (grid.rows as usize, grid.columns as usize);
        let mesh = control_net(&anchors, rows, columns);
        let uvs = anchors.iter().map(|p| p.uv.map(f64::from)).collect();
        let lookup = InverseLookup::build(&mesh, LOOKUP_RESOLUTION);
        Ok(WarpSurface { mesh, rows, columns, uvs, lookup })
    }

    /// The image coordinate at `(s, t)` (bilinear between a cell's four anchors) and its derivatives along s and t.
    fn image_at(&self, s: f64, t: f64) -> ([f64; 2], [f64; 2], [f64; 2]) {
        let cell = |x: f64, n: usize| {
            let cells = n.saturating_sub(1).max(1);
            let f = x.clamp(0.0, 1.0) * cells as f64;
            let i = (f.floor() as usize).min(cells - 1);
            (i, f - i as f64, cells as f64)
        };
        let (c, a, kc) = cell(s, self.columns);
        let (r, b, kr) = cell(t, self.rows);
        let uv = |r: usize, c: usize| self.uvs.get(r * self.columns + c).copied().unwrap_or([0.0; 2]);
        let (p00, p01, p10, p11) = (uv(r, c), uv(r, c + 1), uv(r + 1, c), uv(r + 1, c + 1));
        let component = |q00: f64, q01: f64, q10: f64, q11: f64| {
            (
                (q00 * (1.0 - a) + q01 * a) * (1.0 - b) + (q10 * (1.0 - a) + q11 * a) * b,
                ((q01 - q00) * (1.0 - b) + (q11 - q10) * b) * kc,
                ((q10 - q00) * (1.0 - a) + (q11 - q01) * a) * kr,
            )
        };
        let (u, us, ut) = component(p00[0], p01[0], p10[0], p11[0]);
        let (v, vs, vt) = component(p00[1], p01[1], p10[1], p11[1]);
        ([u, v], [us, vs], [ut, vt])
    }

    /// The surface point over `xy` (grid units), or `None` off the surface.
    pub fn locate(&self, xy: [f64; 2]) -> Option<SurfaceHit> {
        self.lookup.seeds(xy).into_iter().flatten().find_map(|seed| self.refine(xy, seed))
    }

    fn refine(&self, xy: [f64; 2], seed: [f64; 2]) -> Option<SurfaceHit> {
        let [mut s, mut t] = seed;
        for _ in 0..NEWTON_STEPS {
            let p = self.mesh.sample(s, t);
            let (ex, ey) = (xy[0] - p.pos[0], xy[1] - p.pos[1]);
            if ex.abs() + ey.abs() < 1e-12 {
                break;
            }
            let (a, b, c, d) = (p.ds[0], p.dt[0], p.ds[1], p.dt[1]);
            let det = a * d - b * c;
            if det.abs() < 1e-14 || !det.is_finite() {
                break;
            }
            s = (s + (d * ex - b * ey) / det).clamp(0.0, 1.0);
            t = (t + (a * ey - c * ex) / det).clamp(0.0, 1.0);
        }
        let p = self.mesh.sample(s, t);
        let miss = ((xy[0] - p.pos[0]).powi(2) + (xy[1] - p.pos[1]).powi(2)).sqrt();
        if miss.is_nan() || miss > ON_SURFACE {
            return None;
        }
        let (uv, uv_s, uv_t) = self.image_at(s, t);
        // d(image)/d(xy) = d(image)/d(st) · d(st)/d(xy)
        let (a, b, c, d) = (p.ds[0], p.dt[0], p.ds[1], p.dt[1]);
        let det = a * d - b * c;
        let image_per_grid = if det.abs() > 1e-14 {
            let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
            let along_x = (uv_s[0] * ia + uv_t[0] * ic).hypot(uv_s[1] * ia + uv_t[1] * ic);
            let along_y = (uv_s[0] * ib + uv_t[0] * id).hypot(uv_s[1] * ib + uv_t[1] * id);
            along_x.max(along_y)
        } else {
            1.0
        };
        Some(SurfaceHit { pos: p.pos, uv, image_per_grid })
    }
}

static CACHE: Mutex<Vec<(WarpGrid, Arc<WarpSurface>)>> = Mutex::new(Vec::new());

/// The surface of `grid`, built once and shared while it stays among the grids used last.
pub fn surface_for(grid: &WarpGrid) -> Result<Arc<WarpSurface>, String> {
    // Asked once per texel: the grid used last on this thread answers without the shared lock or a copy of the grid
    // (the compositor's threads would otherwise queue on `CACHE` for every texel of a warped layer).
    let same = |k: &WarpGrid| k.rows == grid.rows && k.columns == grid.columns && k.points == grid.points;
    if let Some(s) = LAST.with(|last| last.borrow().as_ref().filter(|(k, _)| same(k)).map(|(_, s)| Arc::clone(s))) {
        return Ok(s);
    }
    let s = surface_shared(grid)?;
    LAST.with(|last| *last.borrow_mut() = Some((WarpGrid { depth: 0.0, ..grid.clone() }, Arc::clone(&s))));
    Ok(s)
}

thread_local! {
    /// The grid this thread asked for last, and its surface.
    static LAST: std::cell::RefCell<Option<(WarpGrid, Arc<WarpSurface>)>> = const { std::cell::RefCell::new(None) };
}

/// [`surface_for`] through the cache shared by all threads.
fn surface_shared(grid: &WarpGrid) -> Result<Arc<WarpSurface>, String> {
    let key = WarpGrid { depth: 0.0, ..grid.clone() };
    if let Ok(cache) = CACHE.lock()
        && let Some((_, s)) = cache.iter().find(|(k, _)| *k == key)
    {
        return Ok(Arc::clone(s));
    }
    let surface = Arc::new(WarpSurface::new(grid)?);
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|(k, _)| *k != key);
        cache.insert(0, (key, Arc::clone(&surface)));
        cache.truncate(CACHED);
    }
    Ok(surface)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_regular_grid_shows_the_image_as_planar() {
        let g = WarpGrid { rows: 3, columns: 3, depth: 0.1, points: Vec::new() };
        let s = surface_for(&g).unwrap();
        for (xy, uv) in [([0.0, 0.0], [0.5, 0.5]), ([-1.0, 1.0], [0.0, 0.0]), ([0.5, -0.5], [0.75, 0.75])] {
            let hit = s.locate(xy).unwrap();
            assert!((hit.uv[0] - uv[0]).abs() < 1e-6 && (hit.uv[1] - uv[1]).abs() < 1e-6, "{xy:?}: {hit:?}");
            assert!((hit.image_per_grid - 0.5).abs() < 1e-6 && hit.pos[2].abs() < 1e-9);
        }
        assert!(s.locate([1.5, 0.0]).is_none(), "off the grid");
    }

    #[test]
    fn a_grid_asked_again_shares_its_surface_and_another_grid_gets_its_own() {
        let a = WarpGrid { rows: 3, columns: 3, depth: 0.1, points: Vec::new() };
        let mut moved = regular_points(3, 3);
        if let Some(p) = moved.get_mut(4) {
            p.pos = [0.3, 0.2, 0.1];
        }
        let b = WarpGrid { rows: 3, columns: 3, depth: 0.1, points: moved };
        let (a1, b1) = (surface_for(&a).unwrap(), surface_for(&b).unwrap());
        let a2 = surface_for(&WarpGrid { depth: 0.4, ..a.clone() }).unwrap();
        assert!(Arc::ptr_eq(&a1, &a2), "the depth is not part of the surface");
        assert!(!Arc::ptr_eq(&a1, &b1));
        assert!(Arc::ptr_eq(&b1, &surface_for(&b).unwrap()));
        let hit = b1.locate([0.3, 0.2]).unwrap();
        assert!((hit.uv[0] - 0.5).abs() < 1e-5, "{hit:?}");
    }

    #[test]
    fn a_moved_anchor_carries_its_image_point() {
        let mut points = regular_points(3, 3);
        if let Some(p) = points.get_mut(4) {
            p.pos = [0.3, 0.2, 0.1];
        }
        let s = WarpSurface::new(&WarpGrid { rows: 3, columns: 3, depth: 0.1, points }).unwrap();
        let hit = s.locate([0.3, 0.2]).unwrap();
        assert!((hit.uv[0] - 0.5).abs() < 1e-5 && (hit.uv[1] - 0.5).abs() < 1e-5 && (hit.pos[2] - 0.1).abs() < 1e-5, "{hit:?}");
    }

    #[test]
    fn malformed_grids_are_refused() {
        assert!(WarpSurface::new(&WarpGrid { rows: 1, columns: 3, depth: 0.1, points: Vec::new() }).is_err());
        assert!(WarpSurface::new(&WarpGrid { rows: 65, columns: 3, depth: 0.1, points: Vec::new() }).is_err());
        assert!(WarpSurface::new(&WarpGrid { rows: 3, columns: 3, depth: 0.1, points: regular_points(2, 2) }).is_err());
        let mut nan = regular_points(3, 3);
        if let Some(p) = nan.get_mut(2) {
            p.uv[0] = f32::INFINITY;
        }
        assert!(WarpSurface::new(&WarpGrid { rows: 3, columns: 3, depth: 0.1, points: nan }).is_err());
        let broken = BezierMesh { us: vec![0.0], vs: Vec::new(), points: Vec::new() };
        assert!(!broken.is_valid());
        assert_eq!(broken.sample(0.5, 0.5).pos, [0.0; 3]);
    }
}
