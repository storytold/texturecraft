//! The Resources Updater: compare image resources with the files they came from, read changed
//! files again and point a resource at another file. Each change is one undo step, and every
//! layer, mask and view using a changed resource recomposites; a mesh map imported from a changed
//! file is resampled while it still holds what the file gave it.

use serde_json::{Value, json};
use texturecraft_core::ResourceSource;

use crate::commands::{CommandSpec, always, bad};
use crate::resource_state::ResourceState;
use crate::resource_status::ResourceStatus;
use crate::{Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "resource.status",
            "Check resources on disk",
            [],
            None,
            "{} → {resources: [{name, status: outdated|missing|upToDate|noSource|material, path?, error?}] (outdated first; material: a stored MaterialCraft package), total, outdated}",
            always,
            |s, _| Ok(status(s))
        ),
        cmd!(
            "resource.update",
            "Update resources",
            [],
            None,
            "{names?: [resource name] (default: every outdated resource)} → {updated, failed: [{name, error}]} — reads their files again, one undo step",
            always,
            update
        ),
        cmd!(
            "resource.relink",
            "Relink resource",
            [],
            None,
            "{name: image resource, path — an image file (png, jpg, tga, tif, bmp, webp, exr, hdr)} → {name, path} — the resource takes the file's pixels and follows it from now on",
            always,
            relink
        ),
    ]
}

impl Session {
    /// Project images and linked MaterialCraft packages, ordered by status and then by name.
    /// Equal-size files are compared when their modification time changed or is unknown.
    /// A known time lets later checks reuse the verdict.
    pub fn resource_states(&mut self) -> Vec<ResourceState> {
        let mut states = self.compare_resources();
        let materials = self.material_packages().into_iter();
        states.extend(materials.map(|name| ResourceState { name, status: ResourceStatus::Material, path: None, error: None }));
        states.sort_by(|a, b| a.status.cmp(&b.status).then_with(|| a.name.cmp(&b.name)));
        states
    }

    /// The packages of linked MaterialCraft materials (fill layers and material effects) the project holds
    fn material_packages(&self) -> std::collections::BTreeSet<String> {
        use texturecraft_core::{EffectKind, Fill, LayerKind};

        let package_of = |fill: &Fill| fill.link.as_ref().map(|link| link.package.clone());
        let mut packages = std::collections::BTreeSet::new();
        for layer in self.doc.project.layers.values() {
            if let LayerKind::Fill(fill) = &layer.kind {
                packages.extend(package_of(fill));
            }
            for effect in layer.mask.iter().flat_map(|mask| &mask.effects).chain(&layer.content_effects) {
                if let EffectKind::Material(fill) = &effect.kind {
                    packages.extend(package_of(fill));
                }
            }
        }

        packages.retain(|name| self.doc.resources.contains_key(name));
        packages
    }

    /// Whether `name` is an image resource the Resources Updater lists
    fn is_image_resource(&self, name: &str) -> bool {
        self.doc.resources.contains_key(name) && (self.images.contains_key(name) || self.doc.project.resource_sources.contains_key(name))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn compare_resources(&mut self) -> Vec<ResourceState> {
        let mut checks = std::mem::take(&mut self.resource_checks);
        let names: Vec<String> = self.doc.resources.keys().filter(|name| self.is_image_resource(name)).cloned().collect();
        let mut states = Vec::with_capacity(names.len());
        for name in names {
            let (Some(bytes), source) = (self.doc.resources.get(&name), self.doc.project.resource_sources.get(&name)) else { continue };
            let (status, error) = source.map_or((ResourceStatus::NoSource, None), |source| compare_with_file(&name, source, bytes, &mut checks));
            states.push(ResourceState { path: source.map(|source| source.path.clone()), name, status, error });
        }

        checks.retain(|name, _| self.doc.resources.contains_key(name));
        self.resource_checks = checks;
        states
    }

    #[cfg(target_arch = "wasm32")]
    fn compare_resources(&mut self) -> Vec<ResourceState> {
        let sources = &self.doc.project.resource_sources;
        let browser = "file paths are not available in the browser";
        let names = self.doc.resources.keys().filter(|name| self.is_image_resource(name));
        names
            .map(|name| match sources.get(name) {
                Some(source) => {
                    ResourceState { name: name.clone(), status: ResourceStatus::Missing, path: Some(source.path.clone()), error: Some(browser.into()) }
                }
                None => ResourceState { name: name.clone(), status: ResourceStatus::NoSource, path: None, error: None },
            })
            .collect()
    }

    /// Stream changed files into one undo step. A mesh map is compared before its first tile is replaced.
    #[cfg(not(target_arch = "wasm32"))]
    fn install_reloaded(&mut self, label: &str, files: impl Iterator<Item = crate::reloaded_file::ReloadedFile>) -> Result<Vec<String>> {
        let mut files = files.peekable();
        if files.peek().is_none() {
            return Ok(Vec::new());
        }

        let mut updated = Vec::new();
        let result = self.edit(label, |session| {
            let mut slots = std::collections::BTreeMap::new();
            for file in files {
                if let Some(slot) = file.source.mesh_map {
                    slots.entry((slot.texture_set, slot.map)).or_insert_with(|| session.mesh_map_matches(slot.texture_set, slot.map));
                }

                let name = file.name;
                if crate::fill_alpha::matches_saved_picture(&name, &file.image) {
                    session.library_alphas.insert(name.clone());
                } else {
                    session.library_alphas.remove(&name);
                }

                session.images.insert(name.clone(), crate::resource_cmds::decoded(file.image));
                session.doc.resources.insert(name.clone(), std::sync::Arc::new(file.bytes));
                session.doc.project.resource_sources.insert(name.clone(), file.source);
                updated.push(name);
            }

            for ((texture_set, map), matches) in slots {
                if matches {
                    let images = session.mesh_map_images(texture_set, map).ok_or_else(|| bad("resource.update", "a mesh map source image is missing"))?;
                    crate::meshmap_cmds::use_images(session, "resource.update", texture_set, map, &images)?;
                }
            }
            Ok(())
        });

        if let Err(error) = result {
            // The edit restored the encoded bytes, so rebuild any cache entries replaced before rollback
            for name in &updated {
                self.images.remove(name);
            }
            self.sync_images();
            return Err(error);
        }

        Ok(updated)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn mesh_map_images(
        &self,
        texture_set: usize,
        map: texturecraft_core::BakedMap,
    ) -> Option<std::collections::BTreeMap<u32, std::sync::Arc<texturecraft_compose::ImageRes>>> {
        let mut images = std::collections::BTreeMap::new();
        for (name, source) in &self.doc.project.resource_sources {
            if let Some(slot) = source.mesh_map.filter(|slot| slot.texture_set == texture_set && slot.map == map) {
                images.insert(slot.tile, self.images.get(name)?.clone());
            }
        }

        Some(images)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn mesh_map_matches(&self, texture_set: usize, map: texturecraft_core::BakedMap) -> bool {
        let Some(current) = self.doc.baked.get(&texture_set).and_then(|baked| baked.maps.get(&map)) else { return false };
        let Some(images) = self.mesh_map_images(texture_set, map) else { return false };

        crate::mesh_map_sampler::MeshMapSampler::new(self, "resource.update", texture_set, map, &images).is_ok_and(|sampler| sampler.matches(current))
    }
}

/// How `stored` (a resource's bytes) compares with the file `source` names now, reusing the
/// remembered verdict in `checks` while the file's size and time stay the same
#[cfg(not(target_arch = "wasm32"))]
fn compare_with_file(
    name: &str,
    source: &ResourceSource,
    stored: &std::sync::Arc<Vec<u8>>,
    checks: &mut std::collections::HashMap<String, crate::resource_check::ResourceCheck>,
) -> (ResourceStatus, Option<String>) {
    let path = std::path::Path::new(&source.path);
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return (ResourceStatus::Missing, Some(format!("{} is not a regular file", source.path))),
        Err(error) => return (ResourceStatus::Missing, Some(format!("{}: {error}", source.path))),
    };

    let size = metadata.len();
    if size != stored.len() as u64 {
        return (ResourceStatus::Outdated, None);
    }

    let modified_ms = modified_ms(&metadata);
    if modified_ms != 0 && modified_ms == source.modified_ms {
        return (ResourceStatus::UpToDate, None);
    }

    if let Some(check) = checks.get(name)
        && check.holds_for(&source.path, size, modified_ms, stored)
    {
        return (check.status, None);
    }

    // A newer time alone is not a change: the file may have been saved again unchanged
    let status = match file_matches(path, stored) {
        Ok(true) => ResourceStatus::UpToDate,
        Ok(false) => ResourceStatus::Outdated,
        Err(error) => return (ResourceStatus::Missing, Some(error.to_string())),
    };
    if modified_ms == 0 {
        // An unknown time cannot establish that the next check sees the same file contents
        checks.remove(name);
    } else {
        let bytes = std::sync::Arc::downgrade(stored);
        checks.insert(name.to_string(), crate::resource_check::ResourceCheck { path: source.path.clone(), size, modified_ms, bytes, status });
    }

    (status, None)
}

/// Compare without allocating a second copy of the resource. Read to the checked end to detect I/O errors and growth.
#[cfg(not(target_arch = "wasm32"))]
fn file_matches(path: &std::path::Path, stored: &[u8]) -> Result<bool> {
    use std::io::{ErrorKind, Read};

    const CMD: &str = "resource.status";
    const CHUNK_BYTES: usize = 32 * 1024;
    let read_error = |error| bad(CMD, format!("cannot read {}: {error}", path.display()));
    let too_large = || bad(CMD, format!("{} is too large (limit {} bytes)", path.display(), crate::resource_cmds::MAX_BYTES));
    let mut file = crate::resource_cmds::open_resource_file(CMD, path)?;
    let len = file.metadata().map_err(read_error)?.len();
    if len > crate::resource_cmds::MAX_BYTES as u64 {
        return Err(too_large());
    }

    let mut chunk = [0; CHUNK_BYTES];
    let mut equal = len == stored.len() as u64;
    for start in (0..len as usize).step_by(CHUNK_BYTES) {
        let end = (start + CHUNK_BYTES).min(len as usize);
        let part = &mut chunk[..end - start];
        match file.read_exact(part) {
            Ok(()) => equal = equal && stored.get(start..end) == Some(part),
            Err(error) if error.kind() == ErrorKind::UnexpectedEof => return Ok(false),
            Err(error) => return Err(read_error(error)),
        }
    }

    if file.read(&mut [0]).map_err(read_error)? > 0 {
        return Err(too_large());
    }

    Ok(equal)
}

/// A file's modification time in milliseconds since the Unix epoch (0 when the system has none)
#[cfg(not(target_arch = "wasm32"))]
fn modified_ms(metadata: &std::fs::Metadata) -> u64 {
    let since_epoch = metadata.modified().ok().and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok());
    since_epoch.map_or(0, |elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
}

/// The record for an image about to be read from `path`: its absolute path and its current
/// modification time, taken before reading so a write during the read still shows as a change
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn file_source(path: &str) -> ResourceSource {
    let absolute = std::path::absolute(path).ok().and_then(|absolute| absolute.to_str().map(str::to_string)).unwrap_or_else(|| path.to_string());
    let modified_ms = std::fs::metadata(path).map_or(0, |metadata| modified_ms(&metadata));
    ResourceSource { path: absolute, modified_ms, mesh_map: None }
}

/// The browser reads no paths; the record keeps the name it was given
#[cfg(target_arch = "wasm32")]
pub(crate) fn file_source(path: &str) -> ResourceSource {
    ResourceSource { path: path.to_string(), modified_ms: 0, mesh_map: None }
}

fn status(s: &mut Session) -> Value {
    let states = s.resource_states();
    let outdated = states.iter().filter(|state| state.status == ResourceStatus::Outdated).count();
    let resources: Vec<Value> = states
        .iter()
        .map(|state| {
            let mut row = json!({"name": state.name, "status": state.status.id()});
            if let Some(path) = &state.path {
                row["path"] = json!(path);
            }
            if let Some(error) = &state.error {
                row["error"] = json!(error);
            }

            row
        })
        .collect();

    json!({"resources": resources, "total": states.len(), "outdated": outdated})
}

#[cfg(not(target_arch = "wasm32"))]
fn update(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "resource.update";

    let names: Vec<String> = match p.get("names") {
        None | Some(Value::Null) => s.resource_states().into_iter().filter(|state| state.status == ResourceStatus::Outdated).map(|state| state.name).collect(),
        Some(Value::Array(names)) => {
            names.iter().map(|name| name.as_str().map(str::to_string).ok_or_else(|| bad(CMD, "`names` must be resource names"))).collect::<Result<_>>()?
        }
        Some(_) => return Err(bad(CMD, "`names` must be an array of resource names")),
    };

    // Validate the complete selection before opening files
    let mut selected = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in names {
        let bytes = s.doc.resources.get(&name).ok_or_else(|| bad(CMD, format!("no resource `{name}`")))?;
        let Some(source) = s.doc.project.resource_sources.get(&name) else {
            let reason = if s.images.contains_key(&name) {
                format!("`{name}` has no source file: relink it to a file first")
            } else {
                format!("`{name}` has no source file: only image resources can be relinked")
            };
            return Err(bad(CMD, reason));
        };

        if seen.insert(name.clone()) {
            let decoded = s.images.contains_key(&name);
            selected.push((name, source.clone(), bytes.clone(), decoded));
        }
    }

    let mut failed = Vec::new();
    let files = selected.into_iter().filter_map(|(name, source, stored, decoded)| {
        match crate::reloaded_file::ReloadedFile::read_changed(CMD, &name, &source, &stored, decoded) {
            Ok(file) => file,
            Err(error) => {
                failed.push(json!({"name": name, "error": error.to_string()}));
                None
            }
        }
    });
    let updated = s.install_reloaded("Update resources", files)?;

    if updated.is_empty() && !failed.is_empty() {
        let reasons: Vec<&str> = failed.iter().filter_map(|failure| failure["error"].as_str()).collect();
        return Err(bad(CMD, format!("nothing was updated: {}", reasons.join("; "))));
    }

    Ok(json!({"updated": updated, "failed": failed}))
}

#[cfg(not(target_arch = "wasm32"))]
fn relink(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "resource.relink";

    let name = crate::commands::strp(p, "name").ok_or_else(|| bad(CMD, "`name` is required"))?.to_string();
    let path = crate::commands::strp(p, "path").ok_or_else(|| bad(CMD, "`path` is required"))?;
    if !s.is_image_resource(&name) {
        return Err(bad(CMD, format!("no image resource `{name}`")));
    }

    // A relinked mesh map file still feeds its mesh map
    let mut file = crate::reloaded_file::ReloadedFile::read(CMD, name.clone(), path)?;
    file.source.mesh_map = s.doc.project.resource_sources.get(&name).and_then(|source| source.mesh_map);
    let path = file.source.path.clone();
    s.install_reloaded("Relink resource", std::iter::once(file))?;

    Ok(json!({"name": name, "path": path}))
}

#[cfg(target_arch = "wasm32")]
fn update(_: &mut Session, _: &Value) -> Result<Value> {
    Err(bad("resource.update", "file paths are not available in the browser"))
}

#[cfg(target_arch = "wasm32")]
fn relink(_: &mut Session, _: &Value) -> Result<Value> {
    Err(bad("resource.relink", "file paths are not available in the browser"))
}
