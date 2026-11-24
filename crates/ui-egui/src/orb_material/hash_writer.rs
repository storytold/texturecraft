use std::fmt;
use std::hash::Hasher;

/// Feeds formatted text straight into a hasher, so a `Debug` key needs no string
pub(super) struct HashWriter<'h, H: Hasher>(pub(super) &'h mut H);

impl<H: Hasher> fmt::Write for HashWriter<'_, H> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.0.write(text.as_bytes());
        Ok(())
    }
}
