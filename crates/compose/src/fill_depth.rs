use texturecraft_core::FillSymmetry;
use texturecraft_geom::{Mesh, glam::Vec3};

use crate::fill_copies::FillCopies;
use crate::projector_depth::{CELL_BUDGET, GRID_SIDE, ProjectorDepth};
use crate::projector_view::ProjectorView;

pub(crate) type FillDepthKey = (ProjectorView, Option<(FillSymmetry, Vec3, f32)>);

/// Depth coverage in the original and optional symmetric views of one fill.
pub(crate) enum FillDepth {
    Single(ProjectorDepth),
    Copies(Box<[Option<ProjectorDepth>]>),
}

impl FillDepth {
    pub(crate) fn build(mesh: &Mesh, (view, symmetry): FillDepthKey) -> Option<Self> {
        let Some((symmetry, center, radius)) = symmetry else {
            return ProjectorDepth::build(view, mesh).map(Self::Single);
        };

        let copies = FillCopies::new(&symmetry, center, center, radius);
        let count = symmetry.copies() as usize;

        // Share one grid's storage and raster budget across the copy views
        let grid_side = (GRID_SIDE * GRID_SIDE / count).isqrt();
        let cell_budget = CELL_BUDGET / count;
        let maps: Box<_> =
            (0..count).map(|index| ProjectorDepth::build_for_copy(view, mesh, Some(copies.copy(index as u32)), grid_side, cell_budget)).collect();

        maps.iter().any(Option::is_some).then_some(Self::Copies(maps))
    }

    /// Read coverage at the point already transformed into the selected copy's space.
    pub(crate) fn cover(&self, copy_index: usize, position: Vec3, hardness: f32) -> f32 {
        let depth = match self {
            Self::Single(depth) => Some(depth),
            Self::Copies(maps) => maps.get(copy_index).and_then(Option::as_ref),
        };

        depth.map_or(1.0, |depth| depth.cover(position, hardness))
    }
}
