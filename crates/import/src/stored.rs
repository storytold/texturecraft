//! The original's stored results, per texture set and channel, read for comparing (`import.verify`): the texture
//! cache the original keeps of every set's final channels, bound to texture sets exactly as the import binds them, so
//! a set of the imported project finds its stored result by name.
//!
//! [`StoredChannel::final_look`] gives a stored texel as the original shows it (its value over the channel's uncovered
//! default) in the stored encoding, and [`StoredChannel::encode`] puts TextureCraft's value in the same encoding: the
//! same comparison as the import's safety net ([`crate::safety_net::score`]).

use std::collections::BTreeSet;

use texturecraft_core::Channel;
use texturecraft_import_reader::decode::assbin::AssbinScene;
use texturecraft_import_reader::project::{ProjectFile, paths};
use texturecraft_texel::linear_to_srgb;

use crate::ImportError;
use crate::channels::{self, uncovered_default};
use crate::resources::{CacheImage, read_cache};
use crate::sets::SetBinding;

/// One channel's stored result.
pub struct StoredChannel {
    pub channel: Channel,
    /// The original's name for the channel.
    pub label: String,
    pub image: CacheImage,
    /// What the original shows where the stored result is uncovered (as TextureCraft holds it).
    pub default: [f32; 3],
}

/// One texture set's stored results.
pub struct StoredSet {
    /// The set's name, as the import names the texture set.
    pub name: String,
    pub channels: Vec<StoredChannel>,
    /// Channels left out (no TextureCraft channel, unreadable).
    pub notes: Vec<String>,
}

/// Every texture set of a file that has stored results.
pub struct StoredResults {
    pub sets: Vec<StoredSet>,
    /// Texture sets without a stored result and other file-wide notes.
    pub notes: Vec<String>,
}

impl StoredResults {
    pub fn set(&self, name: &str) -> Option<&StoredSet> {
        self.sets.iter().find(|s| s.name == name)
    }

    pub fn set_mut(&mut self, name: &str) -> Option<&mut StoredSet> {
        self.sets.iter_mut().find(|s| s.name == name)
    }
}

/// Reads the stored results of the project file `bytes`.
pub fn read_bytes(bytes: Vec<u8>) -> Result<StoredResults, ImportError> {
    read(&ProjectFile::from_bytes(bytes)?)
}

/// Reads the stored results of the project file at `path` (read lazily).
#[cfg(not(target_arch = "wasm32"))]
pub fn read_path(path: &std::path::Path) -> Result<StoredResults, ImportError> {
    read(&ProjectFile::open(path)?)
}

/// Reads the stored results of `file`, set by set as the import binds them.
pub fn read(file: &ProjectFile) -> Result<StoredResults, ImportError> {
    let scene = file.read(paths::SCENE)?;
    let materials = AssbinScene::parse(&scene).map_err(|source| ImportError::Decode { part: "mesh", source })?.materials;
    let records = file.document_materials().unwrap_or_default();
    let entries = file.cache_entries();
    let cache_sets: BTreeSet<String> = entries.iter().map(|e| e.set.clone()).collect();
    let binding = SetBinding::bind(&materials, &records, &cache_sets);
    // the stack's shader model decides one default (base colour); without the layer document the older one applies
    let model = file.model().ok().map(|(document, _)| document);
    let mut out = StoredResults { sets: Vec::new(), notes: Vec::new() };
    for source in &binding.sets {
        let name = source.record.name.clone();
        let Some(folder) = &source.cache_set else {
            out.notes.push(format!("texture set \"{name}\": no stored result (the original never rendered it)"));
            continue;
        };
        let shader_model = source.record.scene_uid.filter(|_| source.in_document).and_then(|uid| model.as_ref()?.texture_set(uid)).and_then(|s| s.shader_model);
        let mut set = StoredSet { name, channels: Vec::new(), notes: Vec::new() };
        let (mut mine, other_tiles) = crate::uv_tiles::one_tile(entries.iter().filter(|e| &e.set == folder).collect());
        if other_tiles > 0 {
            set.notes.push(crate::uv_tiles::other_tiles_note(other_tiles));
        }
        mine.sort_by_key(|e| e.channel_bit);
        for e in mine {
            let label = channels::project_channel_name(e.channel_bit);
            let Some(channel) = channels::channel_for_bit(e.channel_bit) else {
                set.notes.push(format!("channel {label}: no matching channel in TextureCraft; not compared"));
                continue;
            };
            match read_cache(file, e) {
                Ok(image) => set.channels.push(StoredChannel { channel, label, image, default: uncovered_default(channel, shader_model) }),
                Err(err) => set.notes.push(format!("channel {label}: unreadable stored result ({err}); not compared")),
            }
        }
        out.sets.push(set);
    }
    Ok(out)
}

impl StoredChannel {
    pub fn size(&self) -> (u32, u32) {
        (self.image.image.width, self.image.image.height)
    }

    /// The stored result at `size` square, as the import resamples it for a set of that resolution.
    pub fn resampled(&mut self, size: u32) {
        if self.size() != (size, size) {
            self.image.image = crate::import::resample(&self.image.image, size);
        }
    }

    /// A TextureCraft value (colours linear) in the stored encoding: sRGB for channels stored sRGB.
    pub fn encode(&self, v: [f32; 3]) -> [f32; 3] {
        match self.image.srgb {
            true => v.map(|c| linear_to_srgb(c.clamp(0.0, 1.0))),
            false => v,
        }
    }

    /// The stored texel `(x, y)` as the original shows it, over the uncovered default, in the stored encoding.
    pub fn final_look(&self, x: u32, y: u32) -> [f32; 3] {
        let c = self.image.image.get(x, y);
        let a = if c[3].is_finite() { c[3].clamp(0.0, 1.0) } else { 0.0 };
        let d = self.default;
        self.encode([c[0] * a + (1.0 - a) * d[0], c[1] * a + (1.0 - a) * d[1], c[2] * a + (1.0 - a) * d[2]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::Rgba;

    fn stored(srgb: bool, v: [f32; 4], channel: Channel) -> StoredChannel {
        let image = CacheImage { bit: 0, image: Rgba::solid(4, 4, v), srgb, missing_bytes: 0 };
        StoredChannel { channel, label: "x".into(), image, default: uncovered_default(channel, None) }
    }

    #[test]
    fn uncovered_stored_texels_show_the_default_and_srgb_channels_compare_encoded() {
        let rough = stored(false, [0.8, 0.8, 0.8, 0.0], Channel::Roughness);
        assert_eq!(rough.final_look(0, 0), uncovered_default(Channel::Roughness, None));
        let half = stored(false, [0.8, 0.8, 0.8, 0.5], Channel::Metallic);
        assert!((half.final_look(1, 1)[0] - 0.4).abs() < 1e-6);
        let colour = stored(true, [0.25, 0.5, 1.0, 1.0], Channel::BaseColor);
        let look = colour.final_look(0, 0);
        assert!((look[0] - linear_to_srgb(0.25)).abs() < 1e-6 && (look[2] - 1.0).abs() < 1e-6);
        let clipped = colour.encode([2.0, -1.0, 0.0]);
        assert!((clipped[0] - 1.0).abs() < 1e-6 && clipped[1] == 0.0, "{clipped:?}");
        assert_eq!(rough.encode([2.0, -1.0, 0.0]), [2.0, -1.0, 0.0]);
    }

    #[test]
    fn resampling_follows_the_set_size() {
        let mut s = stored(false, [0.5, 0.5, 0.5, 1.0], Channel::Roughness);
        s.resampled(4);
        assert_eq!(s.size(), (4, 4));
        s.resampled(2);
        assert_eq!(s.size(), (2, 2));
        assert!((s.final_look(1, 1)[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_file_that_is_not_a_project_is_an_error_not_a_crash() {
        assert!(read_bytes(vec![0; 64]).is_err());
        assert!(read_bytes(Vec::new()).is_err());
    }
}
