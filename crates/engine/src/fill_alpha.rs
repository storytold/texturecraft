//! A library alpha as a fill source: the source picker's Alphas put the alpha's picture in the project as an image resource the channel reads

use std::sync::Arc;

use texturecraft_paint::alphas::{self, LibraryAlpha};

use crate::resource_cmds::decoded;
use crate::{EngineError, Result, Session};

/// Resource names of library alphas start with this, so the picker and the slot can tell them from imported images
const PREFIX: &str = "alpha_";
/// Resource names of library alphas end with this: they are stored as PNG
const EXTENSION: &str = ".png";
/// Samples per pixel side when the alpha is rendered, for smooth edges
const SUPERSAMPLING: usize = 2;

/// The resource name a library alpha takes in a project
pub fn resource_name(alpha: &LibraryAlpha) -> String {
    texturecraft_formats::sanitize_resource_name(&format!("{PREFIX}{}{EXTENSION}", alpha.id))
}

/// Matches a reserved library name, including numbered alternatives for projects with a pre-existing import.
pub fn library_alpha_of(name: &str) -> Option<&'static LibraryAlpha> {
    let id = name.strip_prefix(PREFIX)?.strip_suffix(EXTENSION)?;
    let id = id.rsplit_once('.').map_or(id, |(id, discriminator)| if discriminator.parse::<usize>().is_ok() { id } else { "" });
    alphas::library().iter().find(|alpha| alpha.id == id)
}

impl Session {
    /// A resource identified as a generated library picture when it was added or decoded.
    pub fn library_alpha(&self, name: &str) -> Option<&'static LibraryAlpha> {
        if self.library_alphas.contains(name) { library_alpha_of(name) } else { None }
    }

    /// Puts library alpha `alpha` in the project as an image resource unless it is there already, inside the caller's edit; returns its name
    pub(crate) fn add_library_alpha(&mut self, alpha: &LibraryAlpha) -> Result<String> {
        let mut name = resource_name(alpha);
        let mut discriminator = 2;
        while self.doc.resources.contains_key(&name) {
            if self.library_alpha(&name).is_some() {
                return Ok(name);
            }

            name = format!("{PREFIX}{}.{discriminator}{EXTENSION}", alpha.id);
            discriminator += 1;
        }

        let bytes = png(alpha)?;
        let image = texturecraft_formats::decode_image(&bytes).map_err(|error| EngineError::Other(format!("{name}: {error}")))?;

        self.doc.resources.insert(name.clone(), Arc::new(bytes));
        self.images.insert(name.clone(), decoded(image));
        self.library_alphas.insert(name.clone());

        Ok(name)
    }
}

/// Recognize legacy generated pixels without treating a same-named import as a library picture.
pub(crate) fn matches_saved_picture(name: &str, image: &texturecraft_formats::DecodedImage) -> bool {
    let Some(alpha) = library_alpha_of(name) else { return false };
    if image.width as usize != alphas::BASE || image.height as usize != alphas::BASE || !image.srgb {
        return false;
    }

    let samples = alpha.render(alphas::BASE, SUPERSAMPLING);
    samples.len() == image.rgba.len()
        && samples.into_iter().zip(&image.rgba).all(|(sample, &[red, green, blue, opacity])| {
            opacity == 1.0 && red == green && red == blue && texturecraft_texel::to_u16(red) == texturecraft_texel::to_u16(sample)
        })
}

/// The alpha's picture as a 16-bit grayscale PNG at the library's base size
fn png(alpha: &LibraryAlpha) -> Result<Vec<u8>> {
    let size = alphas::BASE;
    let side = u32::try_from(size).map_err(|_| EngineError::Other("alpha size out of range".into()))?;
    texturecraft_formats::mcmat::mesh_map_png(side, 1, &alpha.render(size, SUPERSAMPLING))
        .map_err(|error| EngineError::Other(format!("alpha {}: {error}", alpha.id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_library_alpha_is_found_by_the_resource_name_it_takes() {
        for alpha in alphas::library() {
            let name = resource_name(alpha);
            assert_eq!(name, format!("{PREFIX}{}{EXTENSION}", alpha.id), "the id is a safe resource name as it is");
            assert_eq!(library_alpha_of(&name).map(|found| &found.id), Some(&alpha.id));
        }

        assert!(library_alpha_of("alpha_.png").is_none());
    }
}
