//! Windows: when the wait for the release of the press a pick took ends, decided apart from the system's calls

use std::time::{Duration, Instant};

/// How long a release is waited for when the button state never showed the taken press down: longer than any click
const UNSEEN_PRESS_GRACE: Duration = Duration::from_secs(1);

/// The wait for the release of the press a pick took, so the window under it sees no half click
#[derive(Clone, Copy, Debug)]
pub(super) struct ReleaseWait {
    pressed_at: Instant,
    /// The system's button state has shown the button down since the press
    seen_down: bool,
}

impl ReleaseWait {
    /// The wait for the release of a press taken at `pressed_at`
    pub(super) fn new(pressed_at: Instant) -> Self {
        ReleaseWait { pressed_at, seen_down: false }
    }

    /// Whether the wait is over at `now` with the button physically `button_down` (a removed hook never sees the release)
    pub(super) fn over(&mut self, button_down: bool, now: Instant) -> bool {
        if button_down {
            self.seen_down = true;
            return false;
        }

        // A taken press may never reach the button state: the button reads up from the start
        self.seen_down || now.saturating_duration_since(self.pressed_at) >= UNSEEN_PRESS_GRACE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_the_hook_never_saw_ends_the_wait_once_the_button_is_up() {
        let pressed_at = Instant::now();
        let mut wait = ReleaseWait::new(pressed_at);

        assert!(!wait.over(true, pressed_at), "the button is still down: the release is still to come");
        assert!(wait.over(false, pressed_at), "the button is up again: its release went past a removed hook");
    }

    #[test]
    fn a_press_the_button_state_never_showed_ends_the_wait_after_a_grace() {
        let pressed_at = Instant::now();
        let mut wait = ReleaseWait::new(pressed_at);

        assert!(!wait.over(false, pressed_at), "the taken press may not reach the button state: its release is still to come");
        assert!(wait.over(false, pressed_at + UNSEEN_PRESS_GRACE), "the wait never outlasts the grace");
    }
}
