//! Reading and writing saved projections, including decals in the shape saved before they were planar projections placed on the surface

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::saved_decal::SavedDecal;
use crate::{Decal, Projection};

/// A projection as saved: today's, or a decal from before decals were planar projections
#[derive(Deserialize)]
enum SavedProjection {
    Decal(Decal),
    #[serde(untagged)]
    Current(Projection),
}

/// Serde hook for projection fields: an older decal loads as the planar projection that looks the same
pub(crate) fn load<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Projection, D::Error> {
    Ok(match SavedProjection::deserialize(deserializer)? {
        SavedProjection::Decal(frame) => Projection::decal(frame),
        SavedProjection::Current(projection) => projection,
    })
}

/// Serde hook for projection fields: a planar projection with a decal's look saves as a decal, so earlier versions still read a decal
pub(crate) fn save<S: Serializer>(projection: &Projection, serializer: S) -> Result<S::Ok, S::Error> {
    match projection {
        Projection::Planar { placed: Some(frame), .. } if *projection == Projection::decal(*frame) => SavedDecal::Decal(frame).serialize(serializer),
        _ => projection.serialize(serializer),
    }
}
