/// How a channel's image is made from the file chosen for a map.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Take {
    /// The file as it is.
    Whole,
    /// One colour channel (0 = red) of a packed image.
    Channel(usize),
    /// 1 − red: glossiness as roughness.
    Inverted,
}
