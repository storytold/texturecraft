/// Whether a Baking Visualization section starts open, as the reference shows the panel at first launch (HP and Cage open, UV seams and Project mesh closed)
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum SectionOpening {
    Open,
    Closed,
}
