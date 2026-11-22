/// What a band did this frame
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BandResponse {
    /// The band is open: show its rows
    pub(crate) open: bool,

    /// Its Restore defaults button was clicked
    pub(crate) reset: bool,
}
