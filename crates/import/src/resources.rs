//! The file's embedded resources and texture cache as images, and the image resources the imported project carries.
//!
//! The file refers to its own image resources as `/<name>?version=<id>`, where `<id>` names the dataset
//! `/resources/<id>` (sometimes with an extension appended). Shelf resources (`resource://<shelf>/<name>?version=<id>`)
//! are usually not stored, but the file often embeds the images it uses under the same `/resources/<id>`.
//! `resourcealias:<Map>` names one of the texture set's mesh map slots.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use texturecraft_compose::{ImageRes, ImageStore};
use texturecraft_import_reader::decode::image::ImageResource;
use texturecraft_import_reader::decode::pixels::{RowOrder, SourceFormat};
use texturecraft_import_reader::decode::texture_cache::CacheTexture;
use texturecraft_import_reader::model::MeshMapRef;
use texturecraft_import_reader::project::{CacheEntryRef, ProjectFile, paths};

use crate::error::PartError;
use crate::image::Rgba;

/// URL scheme of shelf resources (file-format data).
pub(crate) const SHELF_SCHEME: &str = "resource://";
/// URL scheme naming a texture set's mesh map slot (file-format data).
const ALIAS_SCHEME: &str = "resourcealias:";
/// What separates a resource's name from its id in a URL (file-format data).
const VERSION_MARK: &str = "?version=";

/// One decoded image resource.
#[derive(Debug, Clone)]
pub struct ResourceImage {
    /// Straight values in the stored encoding (float formats: linear), rows top first.
    pub image: Rgba,
    /// True for float formats.
    pub float: bool,
    /// Bytes the stored block was short by (padded with zeros).
    pub missing_bytes: usize,
}

/// One decoded texture-cache entry.
pub struct CacheImage {
    pub bit: u64,
    /// Straight values as TextureCraft holds them (sRGB entries made linear), coverage as alpha, rows top first.
    pub image: Rgba,
    /// True for entries stored sRGB-encoded (compared in that encoding).
    pub srgb: bool,
    pub missing_bytes: usize,
}

/// Splits `/<name>?version=<id>` into name and resource id; `None` for other URLs.
pub fn split_resource_url(url: &str) -> Option<(&str, &str)> {
    if url.starts_with(SHELF_SCHEME) || url.starts_with(ALIAS_SCHEME) {
        return None;
    }
    let (name, id) = url.strip_prefix('/').unwrap_or(url).rsplit_once(VERSION_MARK)?;
    (!id.is_empty()).then_some((name, id))
}

/// The version id of a shelf URL: the name of the dataset under `/resources` when the file embedded it.
pub fn shelf_version(url: &str) -> Option<&str> {
    let rest = url.strip_prefix(SHELF_SCHEME)?;
    let (_, id) = rest.rsplit_once(VERSION_MARK)?;
    (!id.is_empty() && !id.contains('/')).then_some(id)
}

/// The display name of a resource URL (no scheme, shelf path or version).
pub fn url_name(url: &str) -> &str {
    let path = url.split(VERSION_MARK).next().unwrap_or(url);
    let path = path.strip_prefix(ALIAS_SCHEME).unwrap_or(path);
    path.rsplit('/').next().unwrap_or(path)
}

/// What a bitmap URL naming one of the set's mesh map slots reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshMapAlias<'a> {
    /// Not a slot name: the URL itself.
    NotAlias,
    /// The slot holds no map.
    Unassigned(&'a str),
    /// The URL of the map assigned to the slot.
    Assigned(&'a str),
    /// A slot name we don't know, or a slot the set doesn't list.
    Unknown,
}

/// Resolves `resourcealias:<Map>` through the set's mesh map slots.
pub fn mesh_map_alias<'a>(url: &'a str, maps: &'a [MeshMapRef]) -> MeshMapAlias<'a> {
    let Some(name) = url.strip_prefix(ALIAS_SCHEME) else { return MeshMapAlias::NotAlias };
    // File-format data: the slot names of `additionalMaps`, by usage (`normalMapBitmap` for the normal map).
    let usage = match name {
        "ID" => Some(0),
        "AmbientOcclusion" => Some(1),
        "Curvature" => Some(2),
        "NormalWorldSpace" => Some(3),
        "Position" => Some(4),
        "Thickness" => Some(5),
        "Normal" => None,
        _ => return MeshMapAlias::Unknown,
    };
    match maps.iter().find(|m| m.usage == usage) {
        Some(m) if m.url.starts_with(ALIAS_SCHEME) => MeshMapAlias::Unassigned(name),
        Some(m) => MeshMapAlias::Assigned(&m.url),
        None => MeshMapAlias::Unknown,
    }
}

/// `/resources/<id>`, possibly with an extension the file appended.
pub fn resource_dataset(project: &ProjectFile, id: &str) -> Result<String, PartError> {
    let exact = format!("{}/{id}", paths::RESOURCES);
    let layout = project.layout();
    if layout.dataset(&exact).is_some() {
        return Ok(exact);
    }
    let prefix = format!("{exact}.");
    layout
        .datasets_under(paths::RESOURCES)
        .find(|d| d.path.starts_with(&prefix))
        .map(|d| d.path.to_string())
        .ok_or_else(|| PartError::Missing(format!("resource {id}")))
}

/// Reads and decodes the image resource `/resources/<id>`.
pub fn read_image(project: &ProjectFile, id: &str) -> Result<ResourceImage, PartError> {
    let dataset = resource_dataset(project, id)?;
    let data = project.read(&dataset)?;
    let res = ImageResource::parse(&data)?;
    let format = SourceFormat::from_image(&res.format).ok_or_else(|| PartError::UnknownFormat(res.format.name().to_string()))?;
    let pixels = res.decompress_padded()?;
    let float = matches!(format, SourceFormat::Rgba16F | SourceFormat::Rgba32F);
    Ok(ResourceImage { image: Rgba::decode(format, RowOrder::TopFirst, res.width, res.height, &pixels.bytes)?, float, missing_bytes: pixels.missing })
}

/// Reads and decodes one texture-cache entry (stored bottom row first, as textures are).
pub fn read_cache(project: &ProjectFile, entry: &CacheEntryRef) -> Result<CacheImage, PartError> {
    let data = project.read(&entry.path)?;
    let texture = CacheTexture::parse(&data)?;
    let format = SourceFormat::from_gl(texture.gl_internal_format).ok_or_else(|| PartError::UnknownFormat(texture.gl_format_name().to_string()))?;
    let pixels = texture.decompress_padded()?;
    let image = Rgba::decode(format, RowOrder::BottomFirst, texture.width, texture.height, &pixels.bytes)?;
    Ok(CacheImage { bit: entry.channel_bit, image, srgb: format == SourceFormat::CacheSrgb8Alpha8, missing_bytes: pixels.missing })
}

/// A resource of the imported project: its file bytes, and the image the compositor samples while it is held (see
/// [`ProjectImages::release`]).
type Resource = (Vec<u8>, Option<Arc<ImageRes>>);

/// A bitmap loaded for the imported project: its resource name there and its pixels.
#[derive(Clone)]
pub struct Loaded {
    pub name: String,
    pub image: Arc<ResourceImage>,
}

/// The project's images as the import uses them: each bitmap URL read once, and the resources the imported project
/// carries (by name, as files) with the decoded images the safety net composites with. The decoded images are held
/// until [`ProjectImages::release`] (after each texture set), and read from the file again when a later set uses them.
pub struct ProjectImages<'a> {
    /// `None` for a stack built without a file (tests): no bitmap can be read.
    project: Option<&'a ProjectFile>,
    /// By URL: the resource name, or why it cannot be read.
    loaded: Mutex<HashMap<String, Result<String, String>>>,
    /// By URL: the decoded images held.
    held: Mutex<HashMap<String, Arc<ResourceImage>>>,
    out: Mutex<BTreeMap<String, Resource>>,
}

impl<'a> ProjectImages<'a> {
    pub fn new(project: &'a ProjectFile) -> Self {
        ProjectImages { project: Some(project), loaded: Mutex::default(), held: Mutex::default(), out: Mutex::default() }
    }

    /// Images of no file: every bitmap is missing.
    pub fn none() -> ProjectImages<'static> {
        ProjectImages { project: None, loaded: Mutex::default(), held: Mutex::default(), out: Mutex::default() }
    }

    /// The bitmap a `urlToBitmapRes` URL names, added to the project's resources; or why it cannot be read.
    pub fn load(&self, url: &str) -> Result<Loaded, String> {
        let hit = self.loaded.lock().ok().and_then(|l| l.get(url).cloned());
        let name = match hit {
            Some(Err(why)) => return Err(why),
            Some(Ok(name)) => name,
            None => {
                let result = self.read(url);
                if let Ok(mut l) = self.loaded.lock() {
                    l.insert(url.to_string(), result.as_ref().map(|l| l.name.clone()).map_err(String::clone));
                }
                return result;
            }
        };
        // read before and let go since: read again (the same pixels)
        let image = self.held(url)?;
        if let Ok(mut out) = self.out.lock()
            && let Some(resource) = out.get_mut(&name)
            && resource.1.is_none()
        {
            resource.1 = Some(Arc::new(image.image.image_res(!image.float)));
        }
        Ok(Loaded { name, image })
    }

    /// The decoded image `url` names: the one held, or read again.
    fn held(&self, url: &str) -> Result<Arc<ResourceImage>, String> {
        if let Some(image) = self.held.lock().ok().and_then(|h| h.get(url).cloned()) {
            return Ok(image);
        }
        let image = Arc::new(self.decode(url)?.1);
        if let Ok(mut h) = self.held.lock() {
            h.insert(url.to_string(), image.clone());
        }
        Ok(image)
    }

    /// The bitmap a `urlToBitmapRes` URL names, decoded but not added to the project's resources (a brush tip); or why
    /// it cannot be read.
    pub fn peek(&self, url: &str) -> Result<Arc<ResourceImage>, String> {
        if let Some(image) = self.held.lock().ok().and_then(|h| h.get(url).cloned()) {
            return Ok(image);
        }
        if let Some(Err(why)) = self.loaded.lock().ok().and_then(|l| l.get(url).cloned()) {
            return Err(why);
        }
        self.decode(url).map(|(_, read)| Arc::new(read))
    }

    /// Lets go of the decoded images (a texture set is done); the resources' files stay, and a later set that uses an
    /// image reads it again.
    pub fn release(&self) {
        if let Ok(mut h) = self.held.lock() {
            h.clear();
        }
        if let Ok(mut out) = self.out.lock() {
            for resource in out.values_mut() {
                resource.1 = None;
            }
        }
    }

    /// The bitmap `url` names and its dataset id.
    fn decode(&self, url: &str) -> Result<(String, ResourceImage), String> {
        let (id, shelf) = match split_resource_url(url) {
            Some((_, id)) => (id, false),
            None => match shelf_version(url) {
                Some(id) => (id, true),
                None => return Err(format!("bitmap \"{}\" is a shelf resource, not stored in the file", url_name(url))),
            },
        };
        let Some(project) = self.project else { return Err(format!("bitmap \"{}\" is not available", url_name(url))) };
        let read = read_image(project, id).map_err(|e| match e {
            PartError::Missing(_) if shelf => format!("bitmap \"{}\" is a shelf resource, not stored in the file", url_name(url)),
            e => format!("bitmap \"{}\": {e}", url_name(url)),
        })?;
        if read.missing_bytes > 0 {
            return Err(format!("bitmap \"{}\": stored data {} bytes short", url_name(url), read.missing_bytes));
        }
        Ok((id.to_string(), read))
    }

    fn read(&self, url: &str) -> Result<Loaded, String> {
        let (id, read) = self.decode(url)?;
        let name = resource_name(url_name(url), &id, read.float);
        let bytes = read.image.resource_bytes(!read.float).map_err(|e| format!("bitmap \"{}\": {e}", url_name(url)))?;
        let res = Arc::new(read.image.image_res(!read.float));
        let image = Arc::new(read);
        if let Ok(mut out) = self.out.lock() {
            out.insert(name.clone(), (bytes, Some(res)));
        }
        if let Ok(mut h) = self.held.lock() {
            h.insert(url.to_string(), image.clone());
        }
        Ok(Loaded { name, image })
    }

    /// The resources the imported project carries: name → file bytes.
    pub fn files(&self) -> BTreeMap<String, Vec<u8>> {
        self.out.lock().map(|o| o.iter().map(|(k, (b, _))| (k.clone(), b.clone())).collect()).unwrap_or_default()
    }

    /// The resources decoded and held (those the texture set being built uses), as the compositor samples them.
    pub fn store(&self) -> ImageStore {
        self.out.lock().map(|o| o.iter().filter_map(|(k, (_, r))| Some((k.clone(), r.clone()?))).collect()).unwrap_or_default()
    }
}

/// A resource name safe as a file name: `[A-Za-z0-9_.-]`, the bitmap's name and a short id, with the extension of the
/// file it is stored as.
pub fn resource_name(name: &str, id: &str, float: bool) -> String {
    let clean =
        |s: &str, n: usize| -> String { s.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') { c } else { '_' }).take(n).collect() };
    let stem = clean(name, 60);
    let id = clean(id.split('.').next().unwrap_or(id), 12);
    let ext = if float { "exr" } else { "png" };
    format!("{}_{id}.{ext}", if stem.is_empty() { "image" } else { &stem })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_embedded_urls_only() {
        assert_eq!(split_resource_url("/Curvature Map from Mesh Cup?version=81ae"), Some(("Curvature Map from Mesh Cup", "81ae")));
        assert_eq!(split_resource_url("resource://shelf/a?version=1.x"), None);
        assert_eq!(split_resource_url("resourcealias:Normal"), None);
        assert_eq!(split_resource_url("/x?version="), None);
        assert_eq!(shelf_version("resource://shelf/Cutlery1?version=c51e.image"), Some("c51e.image"));
        assert_eq!(shelf_version("resource://shelf/a"), None);
        assert_eq!(url_name("resource://shelf/dirt/dirt_a?version=1"), "dirt_a");
        assert_eq!(url_name("resourcealias:Curvature"), "Curvature");
    }

    #[test]
    fn mesh_map_slots_resolve() {
        let maps = vec![MeshMapRef { usage: Some(1), url: "/AO?version=1".into() }, MeshMapRef { usage: Some(5), url: "resourcealias:Thickness".into() }];
        assert_eq!(mesh_map_alias("resourcealias:AmbientOcclusion", &maps), MeshMapAlias::Assigned("/AO?version=1"));
        assert_eq!(mesh_map_alias("resourcealias:Thickness", &maps), MeshMapAlias::Unassigned("Thickness"));
        assert_eq!(mesh_map_alias("resourcealias:Curvature", &maps), MeshMapAlias::Unknown);
        assert_eq!(mesh_map_alias("resourcealias:Nonsense", &maps), MeshMapAlias::Unknown);
        assert_eq!(mesh_map_alias("/a?version=2", &maps), MeshMapAlias::NotAlias);
    }

    #[test]
    fn resource_names_are_file_safe() {
        assert_eq!(resource_name("My bitmap (2)", "81aeffffffffffffff.image", false), "My_bitmap__2__81aeffffffff.png");
        assert_eq!(resource_name("", "x", true), "image_x.exr");
    }
}
