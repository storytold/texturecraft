/// What a choice of the drop popup ("Which part of <map>?") reads from the dropped map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum SourcePick {
    /// "Red, green and blue": the map's R, G and B into the swatch's R, G and B.
    AllRgb,
    /// One component of the map in every channel of the swatch.
    Component(u8),
}
