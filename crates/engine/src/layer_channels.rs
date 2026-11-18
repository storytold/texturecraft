//! Layer menu commands that act on a whole layer: lock it against painting, write its channels
//! to image files, and switch its mask between gamma-corrected and plain grey values.

use serde_json::{Value, json};
use texturecraft_core::LayerId;

#[cfg(not(target_arch = "wasm32"))]
use crate::commands::strp;
use crate::commands::{CommandSpec, bad, boolp, has_layer, layer_param};
use crate::{EngineError, Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "layer.lock",
            "Lock layer",
            ["Layers"],
            None,
            "{layer?, locked?: bool (default: toggle)} → {locked}. A locked layer refuses paint strokes and fills on its content and its mask (the quick mask is not part of the layer); everything else still edits it",
            has_layer,
            lock
        ),
        cmd!(
            "mask.gammaCorrected",
            "Gamma corrected alpha/mask",
            ["Layers", "Mask"],
            None,
            "{layer?, enabled?: bool (default: toggle)} → {enabled}. A gamma corrected mask reads its grey values as sRGB encoded and converts them to linear when it is applied (an instance edits its source's mask unless it has its own)",
            has_layer,
            gamma_corrected
        ),
        cmd!(
            "layer.exportChannels",
            "Export channels to files...",
            ["Layers"],
            None,
            "{layer?, path: file name, format?: png|tga|tif|exr|... (default: the extension of `path`, else png)} → {files, warnings}. Writes the layer alone (the other layers of its texture set hidden, its mask applied) as one image per channel it holds: `<name>_<channel>.<ext>`, with the UV tile number before the extension for a UV-tiled set. Writes at the texture set's resolution",
            has_layer,
            export_channels
        ),
    ]
}

fn lock(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, "layer.lock", p)?;
    let now = boolp(p, "locked").unwrap_or(!s.doc.project.layer(id)?.locked);
    let label = if now { "Lock layer" } else { "Unlock layer" };
    s.edit(label, |s| {
        s.doc.project.layer_mut(id)?.locked = now;
        Ok(())
    })?;
    Ok(json!({ "locked": now }))
}

fn gamma_corrected(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, "mask.gammaCorrected", p)?;
    let owner = s.doc.project.mask_owner(id);
    let layer = s.doc.project.layer(owner)?;
    let mask = layer.mask.as_ref().ok_or_else(|| EngineError::Other(format!("\"{}\" has no mask", layer.name)))?;
    let now = boolp(p, "enabled").unwrap_or(!mask.gamma_corrected);
    s.edit("Gamma corrected mask", |s| {
        if let Some(mask) = s.doc.project.layer_mut(owner)?.mask.as_mut() {
            mask.gamma_corrected = now;
        }
        Ok(())
    })?;
    Ok(json!({ "enabled": now }))
}

impl Session {
    /// The error a paint stroke on layer `id` gets when the layer is locked.
    pub(crate) fn locked_error(&self, id: LayerId) -> Option<EngineError> {
        let layer = self.doc.project.layers.get(&id).filter(|l| l.locked)?;
        Some(EngineError::Other(format!("\"{}\" is locked: unlock it to paint on it", layer.name)))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn export_channels(s: &mut Session, p: &Value) -> Result<Value> {
    use texturecraft_formats::export::{Dest, ExportOptions, ExportPreset, FileFormat, MapSpec, SourceRef, sanitize_file_stem};
    use texturecraft_formats::write_files;

    let path = strp(p, "path").filter(|path| !path.trim().is_empty()).ok_or_else(|| bad("layer.exportChannels", "`path` is required"))?;
    let id = layer_param(s, "layer.exportChannels", p)?;
    let (ts, _, _) = s.doc.project.locate(id).ok_or_else(|| bad("layer.exportChannels", "the layer is not in a stack"))?;
    let path = std::path::Path::new(path);
    let ext = path.extension().and_then(|e| e.to_str());
    let format = match strp(p, "format").or(ext) {
        Some(name) => FileFormat::from_extension(name).ok_or_else(|| bad("layer.exportChannels", format!("unknown format `{name}`")))?,
        None => FileFormat::Png,
    };
    let stem = path.file_stem().and_then(|n| n.to_str()).map(sanitize_file_stem).unwrap_or_else(|| "layer".into());

    // One map per channel the layer holds
    let layer = s.doc.project.layer(id)?;
    let maps: Vec<MapSpec> = layer
        .channels
        .keys()
        .map(|&channel| {
            let channels = if channel.components() == 3 {
                [Dest::R, Dest::G, Dest::B].into_iter().enumerate().map(|(i, d)| (d, SourceRef::Channel { channel, component: i as u8 })).collect()
            } else {
                vec![(Dest::L, SourceRef::Channel { channel, component: 0 })]
            };
            MapSpec { file: format!("{stem}_$channel"), format, bit_depth: 16, channels, inverted: Vec::new() }
        })
        .collect();
    let mut opts = ExportOptions::new(ExportPreset { name: "Layer channels".into(), maps });
    opts.format_override = Some(format);

    let files = render_alone(s, ts, id, &opts)?;
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| bad("layer.exportChannels", format!("cannot create {}: {e}", dir.display())))?;
    let warnings: Vec<String> = files.iter().flat_map(|f| f.warnings.iter().cloned()).collect();
    let written = write_files(&files, dir).map_err(|e| bad("layer.exportChannels", e.to_string()))?;
    let written: Vec<String> = written.iter().map(|p| p.display().to_string()).collect();
    s.log.push(format!("Exported the channels of the layer to {}", written.join(", ")));
    Ok(json!({ "files": written, "warnings": warnings }))
}

#[cfg(target_arch = "wasm32")]
fn export_channels(_: &mut Session, _: &Value) -> Result<Value> {
    Err(bad("layer.exportChannels", "writing files is not available in the browser"))
}

/// The export files of layer `id` composited alone: the view-only solo project, so the other
/// layers of its texture set are hidden. The document and the viewport are not touched.
#[cfg(not(target_arch = "wasm32"))]
fn render_alone(s: &mut Session, ts: usize, id: LayerId, opts: &texturecraft_formats::ExportOptions) -> Result<Vec<texturecraft_formats::ExportedFile>> {
    use texturecraft_compose::{BakedMaps, Composite, Inputs, composite_tiles, pad_tiles, solo_project};
    use texturecraft_formats::ExportInput;

    s.update();
    let view = solo_project(&s.doc.project, id, false).ok_or_else(|| bad("layer.exportChannels", "the layer is not in a stack"))?;
    let cache = s.caches.get(ts).ok_or_else(|| EngineError::Other(format!("no texture set {ts}")))?;
    let empty = BakedMaps::default();
    let baked = s.doc.baked.get(&ts).unwrap_or(&empty);
    let inputs = Inputs {
        project: &view,
        ts,
        mesh: &s.mesh,
        gbuf: &cache.gbuf,
        pixels: &s.doc.pixels,
        baked,
        block_scales: cache.composite.block_scales.clone(),
        preview: None,
        images: &s.images,
        bounds: s.mesh.bounds(),
    };
    let set = view.texture_set(ts).map_err(|e| EngineError::Other(e.to_string()))?;
    let mut composite = Composite { block_scales: inputs.block_scales.clone(), ..Composite::with_size(&set.channel_list(), cache.size.0, cache.size.1) };
    composite_tiles(&inputs, &mut composite, &cache.covered_tiles);
    pad_tiles(&mut composite, &cache.dilation, &cache.covered_tiles);
    let coverage = cache.gbuf.coverage_mask();
    let mesh_name =
        std::path::Path::new(s.doc.project.mesh.trim_start_matches("builtin:")).file_stem().and_then(|e| e.to_str()).unwrap_or("Untitled").to_string();
    let input = ExportInput { project: &view, ts, composite: &composite, baked, coverage: &coverage, mesh_name: &mesh_name };
    texturecraft_formats::render_maps(&input, opts).map_err(|e| EngineError::Other(e.to_string()))
}
