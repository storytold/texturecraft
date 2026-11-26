/// Which shape the projector shows
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Shape {
    Box,
    Sphere,
    /// Round the axis 0, 1 or 2
    Cylinder(usize),
}
