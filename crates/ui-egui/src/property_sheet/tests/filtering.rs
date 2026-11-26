//! The gallery's Filtering choice

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Filtering {
    BilinearHq,
    BilinearSharp,
    Nearest,
}

impl Filtering {
    pub(super) fn name(self) -> String {
        match self {
            Filtering::BilinearHq => "Bilinear | HQ",
            Filtering::BilinearSharp => "Bilinear | Sharp",
            Filtering::Nearest => "Nearest",
        }
        .to_owned()
    }
}
