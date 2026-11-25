/// What a drag on the projector does
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RoundGrab {
    /// The centre square: slide it across the view
    Slide,
    /// Along the projector's X (0), Y (1) or Z (2) direction
    Axis(usize),
    /// Turn it round its X, Y or Z direction
    Ring(usize),
    /// A size dot: scale it along the directions marked
    Size([bool; 3]),
}
