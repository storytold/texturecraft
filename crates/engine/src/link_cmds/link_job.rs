use std::sync::{Arc, atomic::AtomicBool};

use super::{CookReservation, CookedMaterial, EngineError, Result};
use serde_json::{Map, Value};
use texturecraft_core::LayerId;

#[cfg(not(target_arch = "wasm32"))]
use {
    super::{CookedMap, LinkMemoryBudget, MaterialInterface, cook_report::CookReport, materialcraft_tool, quiet},
    std::{io::Read, path::Path},
    texturecraft_core::{BakedMap, Channel},
};

/// A re-cook of a linked layer, to run anywhere (a worker thread) and hand back to
/// [`crate::Session::apply_link_cook`].
#[derive(Clone, Debug)]
pub struct LinkJob {
    pub layer: LayerId,
    pub package: Arc<Vec<u8>>,
    /// The source package extension (`mcmat`, `mcraft` or `sbs`).
    pub source_ext: &'static str,
    /// The named graph in a linked material; direct package imports use its default.
    pub graph: Option<String>,
    /// Pixels across: the layer's picked resolution (default: the texture set's)
    pub size: u32,
    /// Every parameter's value, by id, in MaterialCraft's typed form.
    pub values: Map<String, Value>,
    /// Graph inputs matched to mesh maps and encoded one at a time in the worker.
    pub input_identifiers: Vec<String>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) baked: Option<texturecraft_compose::BakedMaps>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) budget: LinkMemoryBudget,
    pub canceled: Arc<AtomicBool>,
}

#[cfg(not(target_arch = "wasm32"))]
fn io(error: impl std::fmt::Display) -> EngineError {
    EngineError::Other(error.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path).map_err(io)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(io(format!("invalid or oversized cook output: {}", path.display())));
    }

    let mut bytes = Vec::new();
    let capacity = usize::try_from(metadata.len()).map_err(io)?;
    bytes.try_reserve_exact(capacity).map_err(io)?;
    #[cfg(unix)]
    let mut file = {
        use rustix::fs::{Mode, OFlags, open};

        // The generated output must stay a regular file between inspection and open
        let descriptor = open(path, OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty()).map_err(io)?;
        std::fs::File::from(descriptor)
    };

    #[cfg(not(unix))]
    let mut file = std::fs::File::open(path).map_err(io)?;
    if !file.metadata().map_err(io)?.is_file() {
        return Err(io(format!("invalid cook output: {}", path.display())));
    }

    file.by_ref().take(metadata.len()).read_to_end(&mut bytes).map_err(io)?;
    if file.read(&mut [0u8]).map_err(io)? != 0 {
        return Err(io("cook output grew while reading"));
    }

    if bytes.len() as u64 > limit {
        return Err(io("cook output exceeds the remaining budget"));
    }

    Ok(bytes)
}

#[cfg(not(target_arch = "wasm32"))]
const CANCEL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(20);

#[cfg(windows)]
#[path = "windows_cook.rs"]
mod windows_cook;

#[cfg(not(target_arch = "wasm32"))]
fn invoke(command: std::process::Command, dir: &Path, budget: LinkMemoryBudget, canceled: &AtomicBool) -> Result<Vec<u8>> {
    use std::sync::atomic::Ordering;

    const MAX_DIAGNOSTIC_BYTES: u64 = 1 << 20;
    const MAX_REPORT_BYTES: u64 = 8 << 20;
    if canceled.load(Ordering::Relaxed) {
        return Err(io("MaterialCraft cook canceled"));
    }

    let stdout = dir.join("reply.json");
    let stderr = dir.join("diagnostic.txt");
    let child_allowance = budget.child_allowance();
    let (success, status) = run_bounded(&command, &stdout, &stderr, child_allowance, canceled)?;
    if !success {
        let diagnostic = read_bounded(&stderr, MAX_DIAGNOSTIC_BYTES.min(budget.remaining()))?;
        return Err(io(format!(
            "MaterialCraft cook failed ({status}, {child_allowance} bytes of RAM available within the configured cap): {}",
            String::from_utf8_lossy(&diagnostic).trim()
        )));
    }

    read_bounded(&stdout, MAX_REPORT_BYTES.min(budget.remaining()))
}

#[cfg(windows)]
use windows_cook::run_bounded;

#[cfg(not(any(target_arch = "wasm32", windows)))]
fn run_bounded(command: &std::process::Command, stdout: &Path, stderr: &Path, remaining_bytes: u64, canceled: &AtomicBool) -> Result<(bool, String)> {
    use std::sync::atomic::Ordering;

    let mut command = bounded_cook_command(command, remaining_bytes)?;
    let mut child = command
        .stdout(std::fs::File::create(stdout).map_err(io)?)
        .stderr(std::fs::File::create(stderr).map_err(io)?)
        .spawn()
        .map_err(|error| io(format!("could not start the MaterialCraft cook: {error}. Install MaterialCraft or set MATERIALCRAFT_CLI")))?;

    loop {
        if canceled.load(Ordering::Relaxed) {
            let _ = child.kill();
            child.wait().map_err(io)?;
            return Err(io("MaterialCraft cook canceled"));
        }

        match child.try_wait() {
            Ok(Some(status)) => return Ok((status.success(), status.to_string())),
            Ok(None) => std::thread::sleep(CANCEL_POLL_INTERVAL),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io(error));
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn bounded_cook_command(command: &std::process::Command, remaining_bytes: u64) -> Result<std::process::Command> {
    // Cap address space before exec so a large allocation cannot race the parent
    let mut limited = quiet(Path::new("prlimit"));
    limited.arg(format!("--as={remaining_bytes}")).arg("--").arg(command.get_program()).args(command.get_args());
    Ok(limited)
}

#[cfg(any(target_os = "macos", target_os = "freebsd"))]
fn bounded_cook_command(command: &std::process::Command, remaining_bytes: u64) -> Result<std::process::Command> {
    #[cfg(target_os = "macos")]
    const SHELL: &str = "/bin/bash";
    #[cfg(target_os = "freebsd")]
    const SHELL: &str = "/bin/sh";

    // The shell applies the process limit before replacing itself with MaterialCraft
    let mut limited = quiet(Path::new(SHELL));
    limited
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .args(["-c", r#"ulimit -v "$1" || exit 1; shift; exec -- "$@""#, "texturecraft-cook"])
        .arg((remaining_bytes / 1024).to_string())
        .arg(command.get_program())
        .args(command.get_args());
    Ok(limited)
}

#[cfg(not(any(target_arch = "wasm32", target_os = "linux", target_os = "macos", target_os = "freebsd", windows)))]
fn bounded_cook_command(_: &std::process::Command, remaining_bytes: u64) -> Result<std::process::Command> {
    Err(io(format!("This platform cannot enforce the configured MaterialCraft RAM allowance ({remaining_bytes} bytes remaining)")))
}

#[cfg(not(target_arch = "wasm32"))]
fn channel_for_usage(usage: &str) -> Option<Channel> {
    if usage == "diffuse" {
        return Some(Channel::BaseColor);
    }

    Channel::ALL.into_iter().find(|channel| channel.id() == usage)
}

#[cfg(not(target_arch = "wasm32"))]
impl LinkJob {
    pub(super) fn run_reserved(&self, reservation: CookReservation) -> Result<CookedMaterial> {
        use std::sync::atomic::Ordering;

        let mut budget = self.budget;
        let pixels = u64::from(self.size) * u64::from(self.size);
        budget.check(CookedMap::decoded_bytes(pixels))?;

        let cli = materialcraft_tool("materialcraft-cli", "MATERIALCRAFT_CLI");
        let temp = tempfile::Builder::new().prefix("texturecraft-material-").tempdir().map_err(io)?;
        let dir = temp.path();
        let ext = match self.source_ext {
            "mcmat" | "sbs" | "mcraft" => self.source_ext,
            _ => return Err(io("unknown linked source format")),
        };

        let source = dir.join(format!("source.{ext}"));
        std::fs::write(&source, &*self.package).map_err(io)?;

        let out = dir.join("maps");
        let mut export = quiet(&cli);
        export.arg("export").arg(&source).arg(&out).args(["--profile", "maps", "--size"]).arg(format!("{}x{}", self.size, self.size));
        if self.source_ext != "mcmat"
            && let Some(graph) = &self.graph
        {
            export.arg("--graph-path").arg(graph);
        }

        for (id, value) in &self.values {
            export.arg("--set").arg(format!("{id}={value}"));
        }

        if let Some(bakes) = self.baked.as_ref().filter(|bakes| !bakes.maps.is_empty()) {
            let mut identifiers = self.input_identifiers.clone();
            if self.layer == LayerId(0) {
                let mut inspect = quiet(&cli);
                if ext == "mcmat" {
                    inspect.arg("info").arg(&source);
                } else {
                    inspect.arg("run").arg(&source).arg("material.info");
                }

                let interface: MaterialInterface = serde_json::from_slice(&invoke(inspect, dir, budget, &self.canceled)?).map_err(io)?;
                identifiers = interface.inputs.into_iter().map(|input| input.identifier).collect();
            }

            for (index, identifier) in identifiers.iter().enumerate() {
                let Some((map, data)) = BakedMap::parse(identifier).and_then(|map| Some((map, bakes.maps.get(&map)?))) else {
                    continue;
                };

                let temporary_bytes = texturecraft_formats::mcmat::mesh_map_png_working_bytes(bakes.resolution, bakes.height(map), map.components());
                budget.check(temporary_bytes)?;
                budget.check_available(temporary_bytes)?;
                if self.canceled.load(Ordering::Relaxed) {
                    return Err(io("MaterialCraft cook canceled"));
                }

                let file = dir.join(format!("input{index}.png"));
                let output = std::fs::File::create(&file).map_err(io)?;
                texturecraft_formats::mcmat::write_mesh_map_png(output, bakes.resolution, map.components(), data).map_err(io)?;
                export.arg("--input").arg(format!("{identifier}={}", file.display()));
            }
        }

        let reply: CookReport = serde_json::from_slice(&invoke(export, dir, budget, &self.canceled)?).map_err(io)?;
        let mut maps = Vec::<CookedMap>::new();
        let mut size = 0;
        let has_base_color = reply.maps.iter().any(|map| map.usage == Channel::BaseColor.id());
        for map in reply.maps.into_iter().rev() {
            if self.canceled.load(Ordering::Relaxed) {
                return Err(io("MaterialCraft cook canceled"));
            }

            let Some(channel) = channel_for_usage(&map.usage) else { continue };

            if (map.usage == "diffuse" && has_base_color) || maps.iter().any(|map| map.channel == channel) {
                continue;
            }

            let path = Path::new(&map.file);
            if path.file_name().and_then(|p| p.to_str()) != Some(&map.file) || map.file.contains(['\\', ':']) {
                return Err(io("invalid map filename in MaterialCraft's export report"));
            }

            let [width, height] = map.size;
            let texels = u64::from(width) * u64::from(height);
            if width == 0 || height == 0 || width > self.size || height > self.size || texels > texturecraft_formats::MAX_RESOURCE_TEXELS {
                return Err(io(format!(
                    "{} map dimensions {width}x{height} exceed the requested {} px bake or the project resource limit",
                    map.usage, self.size
                )));
            }

            let file = out.join(path);
            let metadata = std::fs::symlink_metadata(&file).map_err(io)?;
            let temporary_bytes = metadata.len().saturating_add(CookedMap::decoder_peak_bytes(texels));
            budget.check(temporary_bytes)?;
            budget.check_available(metadata.len())?;
            let bytes = read_bounded(&file, metadata.len().min(crate::resource_cmds::MAX_BYTES as u64))?;
            let decoded = CookedMap::decode(channel, bytes, map.size, &mut budget)?;
            size = size.max(width).max(height);
            maps.push(decoded);
        }

        if maps.is_empty() {
            return Err(io("MaterialCraft returned no paintable map usages"));
        }

        Ok(CookedMaterial::new(reply.material, maps, size).with_reservation(reservation))
    }
}

#[cfg(target_arch = "wasm32")]
impl LinkJob {
    pub(super) fn run_reserved(&self, _: CookReservation) -> Result<CookedMaterial> {
        Err(EngineError::Other("cooking a procedural material requires materialcraft-cli on a desktop".into()))
    }
}

impl LinkJob {
    pub fn run(&self) -> Result<CookedMaterial> {
        self.run_reserved(CookReservation::for_cook().map_err(|error| EngineError::Other(error.to_string()))?)
    }
}
