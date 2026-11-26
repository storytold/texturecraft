use x11rb::protocol::xproto::Cursor;

use super::SwatchSide;

/// The disc the X grab's pointer shows now and that pointer
pub(super) struct ShownDisc {
    pub(super) look: ([u8; 3], SwatchSide),
    pub(super) pointer: Cursor,
}
