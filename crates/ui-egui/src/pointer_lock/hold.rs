//! How the system lets a drag hold its pointer

/// How a drag holds the pointer on this platform
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hold {
    /// The pointer moves, and is put back where the press began after every frame; the put-back's own move arrives among the others (X11)
    PutBack,
    /// Put back the same way, but the system folds its waiting moves into one, read from where it put the pointer (Windows)
    PutBackFolded,
    /// The system keeps the pointer still and reports raw motion (macOS, Wayland, which cannot move a pointer)
    Locked,
    /// The pointer cannot be held (the browser): drags read it as it moves
    Free,
}

impl Hold {
    /// The way this platform holds a pointer; Wayland is told apart from X11 the way the window system picks it
    pub(crate) fn of_platform(ctx: &egui::Context) -> Hold {
        if cfg!(target_arch = "wasm32") {
            Hold::Free
        } else if cfg!(target_os = "macos") {
            Hold::Locked
        } else if cfg!(target_os = "windows") {
            Hold::PutBackFolded
        } else if !crate::uses_wayland(ctx) {
            Hold::PutBack
        } else {
            Hold::Locked
        }
    }

    /// The pointer is held by putting it back after every frame
    pub(crate) fn puts_back(self) -> bool {
        matches!(self, Hold::PutBack | Hold::PutBackFolded)
    }
}
