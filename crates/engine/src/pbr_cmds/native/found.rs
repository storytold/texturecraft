use std::path::Path;

use super::Origin;
use crate::pbr_cmds::{FileList, MAX_FILE_BYTES};

pub(in crate::pbr_cmds) struct Found {
    pub(super) source: Origin,
    pub(in crate::pbr_cmds) files: FileList,
    /// A name for the material (folder, archive or shared file-name prefix).
    pub(in crate::pbr_cmds) name: Option<String>,
}

impl Found {
    /// Reads one file of the list. Archive entries are read through a limit, since their
    /// declared size can lie.
    pub(in crate::pbr_cmds) fn load(&self, name: &str) -> std::result::Result<Vec<u8>, String> {
        match &self.source {
            Origin::Folder { root } => read_regular(&root.join(name), MAX_FILE_BYTES),
            Origin::Files(paths) => {
                let path = paths.iter().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy() == name)).ok_or("no such file")?;
                read_regular(path, MAX_FILE_BYTES)
            }
            Origin::Zip(zip) => zip.borrow_mut().read(name, MAX_FILE_BYTES).map_err(|e| e.to_string()),
        }
    }
}

pub(super) fn read_regular(path: &Path, max_bytes: u64) -> std::result::Result<Vec<u8>, String> {
    use std::io::Read as _;

    // Reject links and special files before opening; check the opened handle again below
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("it is not a regular file".into());
    }

    #[cfg(unix)]
    let file = {
        use rustix::fs::{Mode, OFlags, open};

        // NONBLOCK avoids waiting if a regular file is replaced by a pipe during open
        let descriptor =
            open(path, OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty()).map_err(|error| error.to_string())?;
        std::fs::File::from(descriptor)
    };
    #[cfg(not(unix))]
    let file = {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;

            // FILE_FLAG_OPEN_REPARSE_POINT opens a replaced link itself instead of its target
            const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
            options.custom_flags(OPEN_REPARSE_POINT);
        }
        options.open(path).map_err(|error| error.to_string())?
    };
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("it is not a regular file".into());
    }

    if metadata.len() > max_bytes {
        return Err(format!("the file exceeds the limit of {max_bytes} bytes"));
    }

    // The file can grow after metadata was read, so bound actual bytes too
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1)).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!("the file exceeds the limit of {max_bytes} bytes"));
    }

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regular_reads_enforce_the_limit_and_reject_links() {
        let path = std::env::temp_dir().join(format!("tc_pbr_read_{}", std::process::id()));
        std::fs::write(&path, [1u8; 16]).unwrap();

        assert_eq!(read_regular(&path, 16).unwrap(), [1u8; 16]);
        assert!(read_regular(&path, 15).unwrap_err().contains("limit"));
        #[cfg(unix)]
        {
            let link = path.with_extension("link");
            std::os::unix::fs::symlink(&path, &link).unwrap();

            assert!(read_regular(&link, 16).unwrap_err().contains("regular file"));
            std::fs::remove_file(link).unwrap();
        }
        std::fs::remove_file(path).unwrap();
    }
}
