//! An image file read again for a resource, checked before the project changes

use texturecraft_core::ResourceSource;
use texturecraft_formats::DecodedImage;

use crate::Result;
use crate::resource_cmds::{MAX_BYTES, read_image_file};
use crate::resource_update_cmds::file_source;

/// A resource's new bytes, their decoded pixels and the file they came from
pub(crate) struct ReloadedFile {
    pub name: String,
    pub bytes: Vec<u8>,
    pub image: DecodedImage,
    pub source: ResourceSource,
}

impl ReloadedFile {
    pub(crate) fn read(cmd: &str, name: String, path: &str) -> Result<Self> {
        let source = file_source(path);
        let bytes = read_image_file(cmd, path, MAX_BYTES as u64)?;

        Self::decode(cmd, name, source, bytes)
    }

    pub(crate) fn read_changed(cmd: &str, name: &str, previous: &ResourceSource, stored: &[u8], decoded: bool) -> Result<Option<Self>> {
        let mut source = file_source(&previous.path);
        let bytes = read_image_file(cmd, &previous.path, MAX_BYTES as u64)?;
        let unchanged = bytes == stored;
        if unchanged && decoded {
            return Ok(None);
        }

        source.mesh_map = previous.mesh_map;
        let file = Self::decode(cmd, name.to_owned(), source, bytes)?;
        Ok((!unchanged).then_some(file))
    }

    fn decode(cmd: &str, name: String, source: ResourceSource, bytes: Vec<u8>) -> Result<Self> {
        let image = texturecraft_formats::decode_image(&bytes).map_err(|error| crate::commands::bad(cmd, format!("{}: {error}", source.path)))?;

        Ok(Self { name, bytes, image, source })
    }
}
