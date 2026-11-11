//! The original's polygon fill (picks of triangles, polygons, connected pieces or UV islands, by click or dragged
//! rectangle) as pixels of a paint effect, as TextureCraft's own polygon fill tool leaves them. The file stores the
//! hits, not the triangles: [`pick`] resolves each on the set's mesh ([`geometry::SetGeometry`]); [`runs`] collects the
//! value each hit writes and the triangles it covers, in order (later hits paint over earlier ones); [`FillJob::paint`]
//! lays them into the effect's pixels at full coverage on every texel whose triangle (from the UV G-buffer) was picked.

pub mod geometry;
pub mod pick;

use std::collections::BTreeSet;

use texturecraft_core::{Channel, LayerId};
use texturecraft_geom::{NO_TRI, UvGBuffer};
use texturecraft_import_reader::blockers::Blocker;
use texturecraft_import_reader::model::{PolygonFillEffect, StrokeView};
use texturecraft_texel::{TileImage, to_u16};

use crate::status::Status;
use geometry::SetGeometry;

/// One value and the mesh triangles it is written on.
#[derive(Debug, Clone, PartialEq)]
pub struct FillRun {
    /// The hits' `color` (a grey in masks: its first component).
    pub color: [f32; 3],
    /// Mesh triangle indices.
    pub triangles: BTreeSet<u32>,
}

/// The runs of one polygon fill and how exactly they reproduce it.
#[derive(Debug, Clone, PartialEq)]
pub struct FillRuns {
    pub runs: Vec<FillRun>,
    pub status: Status,
}

impl FillRuns {
    /// True when the picks resolved and cover no triangle at all (the original saves zero-width rectangles for clicks
    /// with a little mouse jitter): the fill writes nothing.
    pub fn picks_nothing(&self) -> bool {
        self.status.is_computable() && self.runs.iter().all(|r| r.triangles.is_empty())
    }
}

/// Resolves the fill's hits on `geometry` (`None`: the set's mesh is not available). Consecutive hits writing the same
/// value share a run.
pub fn runs(geometry: Option<&SetGeometry>, fill: &PolygonFillEffect) -> FillRuns {
    let Some(geometry) = geometry else {
        let why = Blocker::feature("polygon-fill-without-mesh", "polygon fill without the set's mesh");
        return FillRuns { runs: Vec::new(), status: Status::not_computable(why) };
    };
    let mut status = Status::Exact;
    if fill.unread > 0 {
        status.and(&Status::not_computable(Blocker::feature("polygon-fill-unread-picks", format!("polygon fill: {} picks not read", fill.unread))));
    }
    let mut runs: Vec<FillRun> = Vec::new();
    for hit in &fill.hits {
        if hit.view == StrokeView::Uv && hit.duplicate_count > 0 {
            // the original adds symmetry copies to 2D-view picks too, by a rule not known yet (3D-view copies are exact)
            status.and(&Status::approximated("polygon fill picked in the 2D view with symmetry"));
        }
        let picked = match pick::pick(geometry, hit) {
            Ok(p) => p,
            Err(e) => {
                status.and(&Status::not_computable(Blocker::feature("polygon-fill-pick", format!("polygon fill pick: {e:?}"))));
                continue;
            }
        };
        let color = hit.color.map(|v| if v.is_finite() { v } else { 0.0 });
        if runs.last().is_none_or(|r| r.color != color) {
            runs.push(FillRun { color, triangles: BTreeSet::new() });
        }
        let Some(run) = runs.last_mut() else { continue };
        for (p, tris) in picked {
            if let Some(part) = geometry.parts.get(p) {
                run.triangles.extend(tris.iter().map(|t| (part.first as u32).saturating_add(*t)));
            }
        }
    }
    FillRuns { runs, status }
}

/// What a polygon fill paints: the paint effect `effect` gets `runs`, laid over each other in order.
#[derive(Debug, Clone, PartialEq)]
pub struct FillJob {
    pub effect: LayerId,
    /// `None` for a paint effect in a mask; the channel of a content paint effect.
    pub channel: Option<Channel>,
    pub runs: Vec<FillRun>,
    /// The layer's path, for the report.
    pub label: String,
}

impl FillJob {
    /// The effect's pixels (value, coverage; premultiplied) at the G-buffer's size: every texel whose triangle a run
    /// picked gets the run's value at full coverage, later runs over earlier ones. A mask takes the grey (the colour's
    /// first component), a content channel the colour as stored (a one-value channel its mean). `None` when nothing
    /// is covered.
    pub fn paint(&self, gbuf: &UvGBuffer) -> Option<TileImage> {
        let comps = self.channel.map_or(1, Channel::components);
        let mut image = TileImage::new(gbuf.width, gbuf.height, comps + 1);
        let w = gbuf.width.max(1) as usize;
        let mut any = false;
        // the last run picking a triangle decides its value
        let mut value: std::collections::HashMap<u32, [f32; 3]> = std::collections::HashMap::new();
        for run in &self.runs {
            let v = match self.channel {
                None => [run.color[0]; 3],
                Some(c) => crate::channels::uniform_value(c, run.color),
            };
            for t in &run.triangles {
                value.insert(*t, v.map(|x| x.clamp(0.0, 1.0)));
            }
        }
        if value.is_empty() {
            return None;
        }
        for (i, t) in gbuf.tri.iter().enumerate() {
            if *t == NO_TRI {
                continue;
            }
            if let Some(v) = value.get(t) {
                let mut px: Vec<u16> = v.iter().take(comps).map(|x| to_u16(*x)).collect();
                px.push(u16::MAX);
                image.set((i % w) as u32, (i / w) as u32, &px);
                any = true;
            }
        }
        any.then_some(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec2, Vec3};
    use texturecraft_geom::{Mesh, SubMesh};
    use texturecraft_import_reader::model::{DecalHit, EffectHeader, HitGranularity, HitShape};
    use texturecraft_texel::from_u16;

    /// One quad over the whole UV square (two triangles, one polygon) and a second part outside it.
    fn mesh() -> Mesh {
        let mut m = Mesh { materials: vec!["m".into()], ..Mesh::default() };
        m.positions = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 0.0), Vec3::new(0.0, 1.0, 0.0)];
        m.normals = vec![Vec3::Z; 4];
        m.uvs = vec![Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 0.0)];
        m.indices = vec![0, 1, 2, 0, 2, 3];
        m.submeshes = vec![SubMesh { name: "quad".into(), material: 0, start: 0, count: 6 }];
        m
    }

    fn click(q: [f32; 2], grey: f32, granularity: HitGranularity) -> DecalHit {
        DecalHit {
            uid: None,
            shape: HitShape::Point(q),
            color: [grey; 3],
            granularity,
            view: StrokeView::Uv,
            proj_view_inv: [0.0; 16],
            duplicate_count: 0,
            duplicate_matrix: [0.0; 16],
            paint_through: false,
            collision: None,
            occlusion_culling: None,
        }
    }

    fn fill(hits: Vec<DecalHit>) -> PolygonFillEffect {
        PolygonFillEffect { header: EffectHeader::default(), hits, unread: 0 }
    }

    #[test]
    fn hits_become_runs_and_later_ones_paint_over_earlier_ones() {
        let m = mesh();
        let g = SetGeometry::new(&m, 0, &[vec![true, false]]);
        // white on the whole polygon, then black on its lower-right triangle (the file's UV, V up)
        let f = fill(vec![click([0.5, 0.5], 1.0, HitGranularity::Polygon), click([0.9, 0.1], 0.0, HitGranularity::Triangle)]);
        let r = runs(Some(&g), &f);
        assert!(r.status.is_exact());
        assert_eq!(r.runs.len(), 2);
        assert_eq!(r.runs[0].triangles, BTreeSet::from([0, 1]));
        assert_eq!(r.runs[1].triangles, BTreeSet::from([0]));
        let gbuf = UvGBuffer::new(&m, 0, 16, 16);
        let job = FillJob { effect: LayerId(5), channel: None, runs: r.runs, label: String::new() };
        let img = job.paint(&gbuf).unwrap();
        // texel rows run top → bottom like glTF v: the file's lower right (u 0.9, V 0.1) is texel (14, 14)
        let at = |x: u32, y: u32| img.get(x, y).iter().take(2).map(|v| from_u16(*v)).collect::<Vec<_>>();
        assert_eq!(at(14, 14), [0.0, 1.0]);
        assert_eq!(at(1, 1), [1.0, 1.0]);
    }

    #[test]
    fn missing_mesh_unread_picks_and_empty_picks() {
        let m = mesh();
        let g = SetGeometry::new(&m, 0, &[]);
        assert!(!runs(None, &fill(vec![click([0.5, 0.5], 1.0, HitGranularity::Triangle)])).status.is_computable());
        let mut f = fill(Vec::new());
        f.unread = 2;
        assert!(!runs(Some(&g), &f).status.is_computable());
        // a click beside the mesh picks nothing and writes nothing
        let r = runs(Some(&g), &fill(vec![click([2.0, 2.0], 1.0, HitGranularity::Mesh)]));
        assert!(r.picks_nothing());
        let job = FillJob { effect: LayerId(1), channel: None, runs: r.runs, label: String::new() };
        assert!(job.paint(&UvGBuffer::new(&m, 0, 8, 8)).is_none());
        // an unknown granularity is not computable
        assert!(!runs(Some(&g), &fill(vec![click([0.5, 0.5], 1.0, HitGranularity::Other(7))])).status.is_computable());
    }
}
