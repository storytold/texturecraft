/// How an eyedropper pick outside the app ended
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickOutcome {
    /// The click landed outside the app: the 8-bit sRGB colour of the one screen pixel under it
    Picked([u8; 3]),
    /// The system picker was dismissed (Escape in it): the pick ends with nothing changed
    Cancelled,
    /// This system cannot read the screen: the pick stays a pick on the model
    Unavailable,
}
