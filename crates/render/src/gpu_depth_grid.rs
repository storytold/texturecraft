//! Projector depth grids for the GPU compositor's depth culling (P1): the CPU evaluator's own
//! grids (`texturecraft_compose::projector_depth`), uploaded as exact-f32 planes of the image
//! atlas so the shader culls with the same numbers.

use std::sync::{Arc, Mutex, Weak};

use texturecraft_compose::pattern::mesh_frame;
use texturecraft_compose::projector_depth::ProjectorDepth;
use texturecraft_compose::projector_view::ProjectorView;
use texturecraft_core::{FillSymmetry, Projection};
use texturecraft_geom::Mesh;
use texturecraft_geom::glam::{Vec2, Vec3};

use crate::gpu_tile_pattern::RawPlane;

/// Words of one copy's entry in a depth directory (`P1_DEPTH_ENTRY_WORDS` in composite_projection.wgsl)
pub(crate) const ENTRY_WORDS: usize = 10;

/// Views whose grids stay cached for the last mesh, as the CPU evaluator's cache keeps them
const MAX_CACHED_VIEWS: usize = 8;

/// A fill's projector view and, with symmetry on, its copies about the mesh's centre and radius
type ViewKey = (ProjectorView, Option<(FillSymmetry, Vec3, f32)>);

/// One grid per symmetry copy (one without symmetry), None where a copy sees nothing
pub(crate) type FillGrids = Arc<[Option<DepthGrid>]>;

/// Grids by view, newest first
type CachedViews = Vec<(ViewKey, Option<FillGrids>)>;

/// The last mesh's grids, so a recompiled program keeps its atlas planes
static CACHE: Mutex<(Weak<Mesh>, CachedViews)> = Mutex::new((Weak::new(), Vec::new()));

/// One projector depth grid as the shader reads it: its plane (per cell, row by row from the
/// lower corner: the nearest surface's nearness and its slope across and up) and the numbers
/// `ProjectorDepth::cover` reads it with.
#[derive(Clone, Debug)]
pub(crate) struct DepthGrid {
    pub(crate) plane: RawPlane,
    origin: Vec2,
    cell: Vec2,
    side: u32,
    unit: f32,
    bias: f32,
    max_slope_bias: f32,
    fade: f32,
}

impl DepthGrid {
    fn of(depth: &ProjectorDepth) -> Self {
        let grid = depth.grid();
        let side = grid.grid_side as u32;
        let texels: Vec<f32> = grid.front.iter().flatten().copied().collect();
        Self {
            plane: RawPlane { size: [side * 3, side], texels: Arc::new(texels) },
            origin: grid.origin,
            cell: grid.cell,
            side,
            unit: grid.nearness_unit,
            bias: grid.bias,
            max_slope_bias: grid.max_slope_bias,
            fade: grid.fade,
        }
    }

    /// The grid's directory entry with its plane at atlas descriptor `plane`
    /// (composite_projection.wgsl `p1_depth_cover`).
    pub(crate) fn entry(&self, plane: u32) -> [u32; ENTRY_WORDS] {
        [
            plane,
            self.origin.x.to_bits(),
            self.origin.y.to_bits(),
            self.cell.x.to_bits(),
            self.cell.y.to_bits(),
            self.side,
            self.unit.to_bits(),
            self.bias.to_bits(),
            self.max_slope_bias.to_bits(),
            self.fade.to_bits(),
        ]
    }
}

/// The directory entry of a copy without a grid: it culls nothing.
pub(crate) fn missing_entry() -> [u32; ENTRY_WORDS] {
    let mut entry = [0; ENTRY_WORDS];
    entry[0] = u32::MAX;
    entry
}

/// The depth grids of a depth-culled `projection` over `mesh`, one per copy of `symmetry`, as the
/// CPU evaluator builds them (compose `EvaluationInputs::projector_depth`); None where the
/// projection culls nothing.
pub(crate) fn depth_grids(mesh: &Arc<Mesh>, projection: &Projection, symmetry: Option<FillSymmetry>) -> Option<FillGrids> {
    let bounds = mesh.bounds();
    let view = ProjectorView::of(projection, bounds)?;
    let copies = symmetry.map(|symmetry| {
        let (center, radius) = mesh_frame(bounds);

        // Flipping the image leaves the projector's depth view unchanged
        (FillSymmetry { flip: [false; 2], ..symmetry }, center, radius)
    });
    let key = (view, copies);

    let mut cache = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.0.upgrade().is_none_or(|cached| !Arc::ptr_eq(&cached, mesh)) {
        *cache = (Arc::downgrade(mesh), Vec::new());
    }
    if let Some(index) = cache.1.iter().position(|(cached, _)| *cached == key) {
        let entry = cache.1.remove(index);
        let grids = entry.1.clone();
        cache.1.insert(0, entry);
        return grids;
    }

    let grids = build(mesh, key);
    cache.1.insert(0, (key, grids.clone()));
    cache.1.truncate(MAX_CACHED_VIEWS);
    grids
}

fn build(mesh: &Mesh, (view, copies): ViewKey) -> Option<FillGrids> {
    let Some((symmetry, center, radius)) = copies else {
        return ProjectorDepth::build(view, mesh).map(|depth| Arc::from([Some(DepthGrid::of(&depth))]));
    };
    let maps = ProjectorDepth::build_copies(view, mesh, &symmetry, center, radius);
    maps.iter().any(Option::is_some).then(|| maps.iter().map(|map| map.as_ref().map(DepthGrid::of)).collect())
}
