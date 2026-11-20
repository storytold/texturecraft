//! Image resources: import (PNG, JPEG, TGA, TIFF, BMP, WEBP, EXR, HDR), list, remove, and use as
//! fill sources.

use std::sync::Arc;

use serde_json::{Value, json};
use texturecraft_compose::ImageRes;
use texturecraft_core::{Channel, Fill, Layer, LayerKind, Source};
use texturecraft_formats::{MAX_RESOURCE_NAME_LEN, sanitize_resource_name};

use crate::commands::{CommandSpec, always, bad, channel_param, strp};
use crate::{EngineError, Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "resource.import",
            "Import resources",
            ["File"],
            None,
            "{path — an image file (png, jpg, tga, tif, bmp, webp, exr, hdr), prefix?: put in front of the resource name (anything outside A-Z a-z 0-9 _ . - becomes _)}",
            always,
            import
        ),
        cmd!("resource.list", "List resources", [], None, "{}", always, |s, _| Ok(list(s))),
        cmd!("resource.remove", "Remove resource", [], None, "{name}", always, remove),
        cmd!("file.removeUnused", "Remove unused resources...", ["File"], None, "{}", always, remove_unused),
        cmd!(
            "layer.add.image",
            "Add fill layer from image",
            [],
            None,
            "{name (exact resource name with extension or unique stem), channel?: baseColor, projection?: uv|triplanar|planar, scale?}",
            always,
            add_image_fill
        ),
    ]
}

/// Largest accepted resource file.
pub(crate) const MAX_BYTES: usize = 512 << 20;
pub(crate) const EXR_SIGNATURE: &[u8] = &[0x76, 0x2f, 0x31, 0x01];

pub(crate) fn decoded(d: texturecraft_formats::DecodedImage) -> Arc<ImageRes> {
    Arc::new(ImageRes { width: d.width, height: d.height, rgba: d.rgba, srgb: d.srgb })
}

impl Session {
    /// Add an image resource from bytes (one undo step); returns its unique, sanitized name
    pub fn import_resource_bytes(&mut self, file_name: &str, bytes: Vec<u8>) -> Result<String> {
        self.import_resource_bytes_prefixed("", file_name, bytes)
    }

    /// Like [`Self::import_resource_bytes`], with `prefix` put in front of the resource's name
    pub fn import_resource_bytes_prefixed(&mut self, prefix: &str, file_name: &str, bytes: Vec<u8>) -> Result<String> {
        self.edit("Import resource", |session| session.add_image_resource_prefixed(prefix, file_name, bytes, texturecraft_formats::MAX_RESOURCE_TEXELS))
    }

    /// Add an image within a decoded texel budget, inside the caller's edit
    pub(crate) fn add_image_resource(&mut self, file_name: &str, bytes: Vec<u8>, max_texels: u64) -> Result<String> {
        self.add_image_resource_prefixed("", file_name, bytes, max_texels)
    }

    fn add_image_resource_prefixed(&mut self, prefix: &str, file_name: &str, bytes: Vec<u8>, max_texels: u64) -> Result<String> {
        if bytes.len() > MAX_BYTES {
            return Err(EngineError::Other(format!("{file_name} is larger than {} MB", MAX_BYTES >> 20)));
        }

        let image = texturecraft_formats::decode_image_within(&bytes, max_texels).map_err(|error| EngineError::Other(format!("{file_name}: {error}")))?;
        let leaf = std::path::Path::new(file_name).file_name().and_then(|name| name.to_str()).unwrap_or("image");
        let mut base = sanitize_resource_name(leaf);

        // The file name has priority over the optional prefix
        let prefix_room = MAX_RESOURCE_NAME_LEN - base.len();
        let prefix = prefix.trim();
        if !prefix.is_empty() && prefix_room > 0 {
            let mut prefix = sanitize_resource_name(prefix);
            prefix.truncate(prefix_room);
            base.insert_str(0, &prefix);
        }

        let (stem, extension) = base.rfind('.').map_or((base.as_str(), ""), |extension_start| base.split_at(extension_start));
        let mut name = base.clone();
        let mut duplicate_index = 2;
        while self.doc.resources.contains_key(&name) {
            let discriminator = format!("{duplicate_index}_");
            let available = MAX_RESOURCE_NAME_LEN - discriminator.len();

            // Leave room for a stem even for names that end in an unusually long extension
            let (stem, extension) = if extension.len() < available { (stem, extension) } else { (base.as_str(), "") };
            let stem = stem.get(..stem.len().min(available - extension.len())).unwrap_or(stem);
            name = format!("{discriminator}{stem}{extension}");
            duplicate_index += 1;
        }

        self.doc.resources.insert(name.clone(), Arc::new(bytes));
        self.images.insert(name.clone(), decoded(image));

        Ok(name)
    }

    /// Decode resources that are not decoded yet and forget removed ones (after undo, redo, load).
    pub fn sync_images(&mut self) {
        self.images.retain(|name, _| self.doc.resources.contains_key(name));

        let mut linked_packages = std::collections::HashSet::new();
        let mut linked_images = std::collections::HashMap::new();
        for layer in self.doc.project.layers.values() {
            let LayerKind::Fill(Fill { link: Some(link), sources, .. }) = &layer.kind else { continue };

            linked_packages.insert(link.package.as_str());
            for source in sources.values() {
                if let Source::Image { name, .. } = source {
                    linked_images.entry(name.as_str()).and_modify(|size: &mut u32| *size = (*size).max(link.size)).or_insert(link.size);
                }
            }
        }

        for (name, bytes) in &self.doc.resources {
            if self.images.contains_key(name) || linked_packages.contains(name.as_str()) {
                continue;
            }

            // EXR may need a full float decode buffer even when encoded project bytes are small
            let image = match linked_images.get(name.as_str()) {
                Some(&size) if size > 0 && bytes.starts_with(EXR_SIGNATURE) => {
                    let texels = (u64::from(size) * u64::from(size)).min(texturecraft_formats::MAX_RESOURCE_TEXELS);
                    let decoder_bytes = texels * std::mem::size_of::<[f32; 4]>() as u64;
                    texturecraft_formats::decode_cooked_image(bytes, [size; 2], decoder_bytes).or_else(|_| texturecraft_formats::decode_image(bytes))
                }
                _ => texturecraft_formats::decode_image(bytes),
            };
            match image {
                Ok(image) => {
                    self.images.insert(name.clone(), decoded(image));
                }
                Err(error) => self.log.warn(crate::log::LIBRARY_SOURCE, format!("resource {name}: {error}")),
            }
        }

        self.drop_missing_image_view();
    }

    /// The "Other..." view cannot show an image that is gone (removed or renamed): the material comes back
    pub(crate) fn drop_missing_image_view(&mut self) {
        if self.view == crate::ViewMode::Image && !self.view_image.as_ref().is_some_and(|name| self.images.contains_key(name)) {
            self.view = crate::ViewMode::Material;
            self.view_image = None;
            self.revision = self.revision.wrapping_add(1);
        }
    }
}

fn list(s: &Session) -> Value {
    json!(
        s.doc
            .resources
            .iter()
            .map(|(k, v)| {
                let (w, h) = s.images.get(k).map(|i| (i.width, i.height)).unwrap_or((0, 0));
                json!({"name": k, "bytes": v.len(), "width": w, "height": h})
            })
            .collect::<Vec<_>>()
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let path = strp(p, "path").ok_or_else(|| bad("resource.import", "`path` is required"))?;
    let bytes = read_image_file("resource.import", path, MAX_BYTES as u64)?;
    let name = s.import_resource_bytes_prefixed(strp(p, "prefix").unwrap_or(""), path, bytes)?;
    Ok(json!({ "name": name }))
}

/// Read a regular image file within both the per-file cap and the caller's remaining byte budget
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_image_file(cmd: &str, path: &str, max_bytes: u64) -> Result<Vec<u8>> {
    read_resource_file(cmd, std::path::Path::new(path), max_bytes.min(MAX_BYTES as u64))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn open_resource_file(cmd: &str, path: &std::path::Path) -> Result<std::fs::File> {
    let display = path.display();
    let read_error = |error| bad(cmd, format!("cannot read {display}: {error}"));

    // Pipes can block on open, and devices may never reach EOF
    if !std::fs::metadata(path).map_err(read_error)?.is_file() {
        return Err(bad(cmd, format!("{display} is not a regular file")));
    }

    #[cfg(unix)]
    let file = {
        use rustix::fs::{Mode, OFlags, open};

        // Follow user-selected symlinks without waiting on a substituted pipe
        let descriptor = open(path, OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty()).map_err(|error| read_error(error.into()))?;
        std::fs::File::from(descriptor)
    };
    #[cfg(not(unix))]
    let file = std::fs::File::open(path).map_err(read_error)?;

    if !file.metadata().map_err(read_error)?.is_file() {
        return Err(bad(cmd, format!("{display} is not a regular file")));
    }

    Ok(file)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_resource_file(cmd: &str, path: &std::path::Path, max_bytes: u64) -> Result<Vec<u8>> {
    use std::io::Read;

    let display = path.display();
    let read_error = |error| bad(cmd, format!("cannot read {display}: {error}"));
    let too_large = || bad(cmd, format!("{display} is too large (limit {max_bytes} bytes)"));
    let mut file = open_resource_file(cmd, path)?;
    let metadata = file.metadata().map_err(read_error)?;
    if metadata.len() > max_bytes {
        return Err(too_large());
    }

    let mut bytes = Vec::new();
    let capacity = usize::try_from(metadata.len()).map_err(|_| too_large())?;
    bytes.try_reserve_exact(capacity).map_err(|error| bad(cmd, format!("cannot allocate {display}: {error}")))?;

    // A growing file must not trigger allocations beyond the checked size
    file.by_ref().take(metadata.len()).read_to_end(&mut bytes).map_err(read_error)?;
    if file.read(&mut [0u8]).map_err(read_error)? != 0 {
        return Err(too_large());
    }

    Ok(bytes)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn read_image_file(cmd: &str, _: &str, _: u64) -> Result<Vec<u8>> {
    Err(bad(cmd, "file paths are not available in the browser"))
}

#[cfg(target_arch = "wasm32")]
fn import(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Other("file paths are not available in the browser; use File > Import resources".into()))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let name = strp(p, "name").ok_or_else(|| bad("resource.remove", "`name` is required"))?.to_string();
    if !s.doc.resources.contains_key(&name) {
        return Err(bad("resource.remove", format!("no resource `{name}`")));
    }
    s.edit("Remove resource", |s| {
        s.doc.resources.remove(&name);
        Ok(())
    })?;
    s.sync_images();
    Ok(Value::Null)
}

/// Drop resources unused by layers, mask effects, the brush alpha, the stencil or the environment.
fn remove_unused(s: &mut Session, _: &Value) -> Result<Value> {
    let mut used = Vec::new();
    for set in &s.doc.project.texture_sets {
        for root in &set.stack {
            crate::material_lib::images_used(&s.doc.project, *root, &mut used, 0);
        }
    }
    used.extend(s.tool.stencil.as_ref().map(|st| st.image.clone()));
    used.extend(s.tool.brush.alpha_image.clone());
    used.extend(s.doc.project.display.environment_resource().map(str::to_string));
    let unused: Vec<String> = s.doc.resources.keys().filter(|k| !used.contains(k)).cloned().collect();
    if !unused.is_empty() {
        s.edit("Remove unused resources", |s| {
            s.doc.resources.retain(|k, _| !unused.contains(k));
            Ok(())
        })?;
        s.sync_images();
    }
    Ok(json!({ "removed": unused }))
}

#[cfg(not(target_arch = "wasm32"))]
const IMPORT_HINT: &str = "resource.import";
#[cfg(target_arch = "wasm32")]
const IMPORT_HINT: &str = "File > Import resources";

/// Resolve an image resource by exact name or unique file stem.
fn resolve_resource_name(s: &Session, name: &str) -> Result<String> {
    if s.images.contains_key(name) {
        return Ok(name.to_string());
    }
    let matches: Vec<&String> = s.images.keys().filter(|k| std::path::Path::new(k).file_stem().and_then(|stem| stem.to_str()) == Some(name)).collect();
    match matches.as_slice() {
        [single] => Ok((*single).clone()),
        [] => {
            let available: Vec<&str> = s.images.keys().map(String::as_str).collect();
            let hint =
                if available.is_empty() { format!(" (no resources imported: use {IMPORT_HINT})") } else { format!(" (available: {})", available.join(", ")) };
            Err(bad("layer.add.image", format!("no resource `{name}`{hint}")))
        }
        multiple => {
            let matches_str = multiple.iter().map(|m| format!("`{m}`")).collect::<Vec<_>>().join(", ");
            Err(bad("layer.add.image", format!("ambiguous resource `{name}` matches {matches_str}: give the full name with extension")))
        }
    }
}

fn add_image_fill(s: &mut Session, p: &Value) -> Result<Value> {
    let raw_name = strp(p, "name").ok_or_else(|| bad("layer.add.image", "`name` is required"))?;
    let name = resolve_resource_name(s, raw_name)?;
    let ch = channel_param("layer.add.image", p, "channel")?.unwrap_or(Channel::BaseColor);
    let projection = crate::layer_cmds::projection_from(p, "layer.add.image", None)?;
    let ts = s.ts();
    let label = std::path::Path::new(&name).file_stem().and_then(|n| n.to_str()).unwrap_or("Image").to_string();
    let layer_name = s.doc.project.next_layer_name(ts, &label);
    let id = s.edit("Add image fill", |s| {
        let id = s.doc.project.alloc_id();
        let mut fill = Fill { projection, ..Fill::default() };
        fill.sources.insert(ch, Source::image(name.clone()));
        crate::layer_cmds::insert_selected(s, Layer::new(id, layer_name, LayerKind::Fill(fill), &[ch]))
    })?;
    Ok(json!({ "layer": id.0 }))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn file_reads_obey_the_remaining_byte_budget() {
        let path = std::env::temp_dir().join(format!("texturecraft-read-image-{}.png", std::process::id()));
        std::fs::write(&path, b"1234").unwrap();
        let name = path.to_str().unwrap();
        assert!(read_image_file("test", name, 3).unwrap_err().to_string().contains("too large"));
        assert_eq!(read_image_file("test", name, 4).unwrap(), b"1234");
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn devices_are_rejected_before_reading() {
        assert!(read_image_file("test", "/dev/null", 16).unwrap_err().to_string().contains("not a regular file"));
    }

    #[test]
    fn duplicate_prefixed_image_names_survive_saving_and_reopening() {
        let mut session = crate::sample_session(128, 32).unwrap();
        let png = texturecraft_formats::gray16_png(1, 1, [0.5].into_iter()).unwrap();
        let file_name = format!("{}.png", "x".repeat(108));
        let prefix = "material";
        let mut names = Vec::new();

        for _ in 0..10 {
            let name = session.import_resource_bytes_prefixed(prefix, &file_name, png.clone()).unwrap();
            crate::execute(&mut session, "layer.add.image", &json!({"name": name})).unwrap();
            names.push(name);
        }

        let reopened = Session::from_project_bytes(&session.to_project_bytes().unwrap()).unwrap();

        assert_eq!(reopened.images.len(), names.len());
        for name in names {
            assert!(reopened.images.contains_key(&name), "missing image after reload: {name}");
            assert!(name.ends_with(".png"));
        }
    }

    #[test]
    fn removing_unused_resources_keeps_the_active_brush_alpha() {
        let mut session = crate::sample_session(128, 32).unwrap();
        let png = texturecraft_formats::gray16_png(1, 1, [0.5].into_iter()).unwrap();
        let alpha = session.import_resource_bytes("brush.png", png.clone()).unwrap();
        let unused = session.import_resource_bytes("unused.png", png).unwrap();
        crate::execute(&mut session, "tool.brush", &json!({"alpha": {"image": alpha}})).unwrap();

        let result = crate::execute(&mut session, "file.removeUnused", &Value::Null).unwrap();

        assert_eq!(result["removed"], json!([unused]));
        assert_eq!(session.tool.brush.alpha_image.as_deref(), Some(alpha.as_str()));
        assert!(session.images.contains_key(&alpha));
        assert!(!session.images.contains_key(&unused));
    }
}
