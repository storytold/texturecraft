//! A decal written in the shape saved before decals were planar projections

use serde::Serialize;

use crate::Decal;

/// A decal in the shape saved before decals were planar projections
#[derive(Serialize)]
pub(crate) enum SavedDecal<'a> {
    Decal(&'a Decal),
}
