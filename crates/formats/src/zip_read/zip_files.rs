use std::io::Read as _;

use crate::{FormatError, Result, malformed};

/// An opened archive.
pub struct ZipFiles {
    archive: zip::ZipArchive<std::io::Cursor<Vec<u8>>>,
}

impl ZipFiles {
    pub fn open(bytes: Vec<u8>) -> Result<ZipFiles> {
        // Bound central-directory allocation before the ZIP decoder indexes entries
        craft_zip::check_entry_count(&bytes, super::MAX_ENTRIES)
            .map_err(|error| if error.is_limit() { FormatError::TooLarge(error.to_string()) } else { malformed("zip", error) })?;

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
