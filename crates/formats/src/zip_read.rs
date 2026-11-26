//! Reading files out of a zip archive held in memory (a downloaded texture pack).
//!
//! Entry names are only labels here, never joined to a path. Declared sizes can lie, so every
//! read goes through a limit; the number of entries listed is capped.

mod zip_files;

pub use zip_files::ZipFiles;

/// Maximum number of archive entries indexed for a texture pack.
pub const MAX_ENTRIES: u64 = 4096;

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    fn make(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (n, b) in files {
            archive.start_file(*n, options).unwrap();
            archive.write_all(b).unwrap();
        }
        archive.finish().unwrap().into_inner()
    }

    #[test]
    fn lists_reads_and_caps() {
        let mut archive = ZipFiles::open(make(&[("a/x.png", b"hello"), ("b.txt", &[7u8; 5000])])).unwrap();
        assert_eq!(archive.list(10), vec![("a/x.png".to_string(), 5), ("b.txt".to_string(), 5000)]);
        assert_eq!(archive.list(1).len(), 1);
        assert_eq!(archive.read("a/x.png", 100).unwrap(), b"hello");
        assert!(archive.read("b.txt", 100).is_err(), "over the cap");
        assert!(archive.read("nope", 100).is_err());
    }

    #[test]
    fn excessive_entry_count_is_rejected_before_indexing() {
        let names: Vec<String> = (0..4097).map(|index| format!("{index}.png")).collect();
        let entries: Vec<(&str, &[u8])> = names.iter().map(|name| (name.as_str(), b"".as_slice())).collect();
        let bytes = make(&entries);

        let result = ZipFiles::open(bytes);

        assert!(matches!(result, Err(crate::FormatError::TooLarge(_))));
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(ZipFiles::open(b"not a zip".to_vec()).is_err());
        assert!(ZipFiles::open(Vec::new()).is_err());
    }
}
