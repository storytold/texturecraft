//! The 3D Distance and 3D Linear Gradient generators: gradients around a point and between two points across the mesh's bounds

use texturecraft_core::{DistanceParameters, GeneratorGlobal, LinearGradientParameters};
use texturecraft_geom::glam::Vec3;

use crate::histogram_scan::histogram_scan;

/// The value at `position` (0..1 across the bounds, the longest side spanning 0..1): 1 at the point, 0 at the radius, then scanned by the offset and contrast
pub(crate) fn distance_value(distance: &DistanceParameters, position: Vec3) -> f32 {
    let from_point = position.distance(Vec3::from(distance.point));
    let gradient = if distance.radius > 0.0 { (1.0 - from_point / distance.radius).clamp(0.0, 1.0) } else { 0.0 };
    histogram_scan(gradient, distance.offset, distance.contrast)
}

/// Smallest squared start-to-end span the gradient divides by
const MIN_GRADIENT_LENGTH_SQUARED: f32 = 1e-12;

/// The value at `position`: how far along from the start to the end it lies (0 at the start, 1 at the end), balanced and contrasted like a generator's global controls
pub(crate) fn linear_gradient_value(gradient: &LinearGradientParameters, position: Vec3) -> f32 {
    let (start, end) = (Vec3::from(gradient.start), Vec3::from(gradient.end));
    let span = end - start;
    let along = if span.length_squared() > MIN_GRADIENT_LENGTH_SQUARED { (position - start).dot(span) / span.length_squared() } else { 0.0 };
    GeneratorGlobal { balance: gradient.balance, contrast: gradient.contrast, ..GeneratorGlobal::default() }.apply(along.clamp(0.0, 1.0))
}
