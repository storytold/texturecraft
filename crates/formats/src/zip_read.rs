//! Reading files out of a zip archive held in memory (a downloaded texture pack).
//!
//! Entry names are only labels here, never joined to a path. Declared sizes can lie, so every
//! read goes through a limit; the number of entries listed is capped.

use std::io::Read as _;

use crate::{FormatError, Result, malformed};

/// An opened archive.
pub struct ZipFiles {
    archive: zip::ZipArchive<std::io::Cursor<Vec<u8>>>,
}

impl ZipFiles {
    pub fn open(bytes: Vec<u8>) -> Result<ZipFiles> {
        let archive = crate::guard("zip", || zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| malformed("zip", e)))?;
        Ok(ZipFiles { archive })
    }

    /// The files (not folders) as `(name with / separators, declared size)`, at most `max`.
    pub fn list(&mut self, max: usize) -> Vec<(String, u64)> {
        let mut out = Vec::new();
        for i in 0..self.archive.len().min(max) {
            if let Ok(e) = self.archive.by_index_raw(i)
                && e.is_file()
                && let Ok(name) = e.name()
            {
                out.push((name.replace('\\', "/"), e.size()));
            }
        }
        out
    }

    /// The bytes of the file `name` (as listed), refusing more than `cap` bytes.
    pub fn read(&mut self, name: &str, cap: u64) -> Result<Vec<u8>> {
        let index = (0..self.archive.len())
            .find(|i| self.archive.by_index_raw(*i).is_ok_and(|e| e.name().is_ok_and(|n| n.replace('\\', "/") == name)))
            .ok_or_else(|| FormatError::Invalid(format!("{name} is not in the archive")))?;
        let mut entry = crate::guard("zip", || self.archive.by_index(index).map_err(|e| malformed("zip", e)))?;
        if entry.size() > cap {
            return Err(FormatError::TooLarge(format!("{name} is over {} MB", cap >> 20)));
        }
        let mut out = Vec::new();
        (&mut entry).take(cap.saturating_add(1)).read_to_end(&mut out)?;
        if out.len() as u64 > cap {
            return Err(FormatError::TooLarge(format!("{name} is over {} MB", cap >> 20)));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    fn make(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (n, b) in files {
            z.start_file(*n, o).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    #[test]
    fn lists_reads_and_caps() {
        let mut z = ZipFiles::open(make(&[("a/x.png", b"hello"), ("b.txt", &[7u8; 5000])])).unwrap();
        assert_eq!(z.list(10), vec![("a/x.png".to_string(), 5), ("b.txt".to_string(), 5000)]);
        assert_eq!(z.list(1).len(), 1);
        assert_eq!(z.read("a/x.png", 100).unwrap(), b"hello");
        assert!(z.read("b.txt", 100).is_err(), "over the cap");
        assert!(z.read("nope", 100).is_err());
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(ZipFiles::open(b"not a zip".to_vec()).is_err());
        assert!(ZipFiles::open(Vec::new()).is_err());
    }
}
