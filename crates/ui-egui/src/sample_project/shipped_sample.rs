use std::io::{self, ErrorKind, Write as _};
use std::path::Path;

/// A built-in sample project and its installed filename.
pub(super) struct ShippedSample {
    pub(super) file_name: &'static str,
    pub(super) bytes: &'static [u8],
}

impl ShippedSample {
    pub(super) fn install_missing(&self, folder: &Path, destination: &Path) -> io::Result<()> {
        let mut staged_sample = tempfile::NamedTempFile::new_in(folder)?;
        staged_sample.write_all(self.bytes)?;
        staged_sample.as_file().sync_all()?;

        // A user can save to the destination while the bundled copy is being written
        match staged_sample.persist_noclobber(destination) {
            Ok(_) => Ok(()),
            Err(error) if error.error.kind() == ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error.error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ShippedSample;

    #[test]
    fn a_sample_that_appears_during_install_is_kept() {
        let folder = tempfile::tempdir().unwrap();
        let sample = ShippedSample { file_name: "Sample.tcraft", bytes: b"bundled sample" };
        let destination = folder.path().join(sample.file_name);
        assert!(!destination.exists());
        std::fs::write(&destination, b"user project").unwrap();

        sample.install_missing(folder.path(), &destination).unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), b"user project");
        assert_eq!(std::fs::read_dir(folder.path()).unwrap().count(), 1, "the temporary copy was removed");
    }
}
