//! Graphics adapter selection and multi-GPU configuration (#49).
//!
//! On machines with multiple GPUs (e.g. hybrid laptops or dual-GPU desktops with Intel/AMD iGPU and
//! NVIDIA/AMD dGPU), TextureCraft must allow users to select which graphics processor renders the
//! 3D viewport, and default to the high-performance discrete GPU for real-time PBR shading.
//!
//! Selection priority:
//! 1. Window surface support: adapters that cannot present to the surface are ranked last.
//! 2. Explicit filter: `--gpu <name>` CLI option or `WGPU_ADAPTER_NAME` environment variable (case-insensitive substring).
//! 3. Hardware before software: physical GPUs before software/CPU rasterizers (llvmpipe, lavapipe, WARP).
//! 4. Power preference: [`wgpu::PowerPreference::HighPerformance`] (default for 3D painting) prefers discrete GPUs,
//!    [`wgpu::PowerPreference::LowPower`] prefers integrated GPUs, and [`wgpu::PowerPreference::None`] preserves system order.
//! 5. Backend preference: native backends (Vulkan, Metal, DX12) before OpenGL.
//!    On Windows, prefer Vulkan when the candidates otherwise rank equally.

use std::sync::Arc;

use eframe::NativeOptions;
use eframe::egui_wgpu::WgpuSetup;
use eframe::wgpu::{self, Backend, DeviceType, PowerPreference};

/// Environment variable for choosing an adapter by name or substring.
pub const ADAPTER_NAME_ENV: &str = "WGPU_ADAPTER_NAME";

/// Environment variable for power preference (`high`, `low`, `none`).
pub const POWER_PREF_ENV: &str = "WGPU_POWER_PREF";

/// Parses a power preference string from CLI or environment variable.
pub fn parse_power_preference(s: &str) -> Result<PowerPreference, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "high" | "highperformance" | "high-performance" => Ok(PowerPreference::HighPerformance),
        "low" | "lowpower" | "low-power" => Ok(PowerPreference::LowPower),
        "none" | "default" => Ok(PowerPreference::None),
        other => Err(format!("invalid power preference `{other}`: expected `high`, `low`, or `none`")),
    }
}

/// A graphics adapter candidate inspected during adapter selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    pub vendor: u32,
    pub device: u32,
    pub device_type: DeviceType,
    pub backend: Backend,
    pub surface_supported: bool,
}

impl Candidate {
    pub fn new(adapter: &wgpu::Adapter, surface: Option<&wgpu::Surface<'_>>) -> Self {
        let info = adapter.get_info();
        Self {
            name: info.name,
            vendor: info.vendor,
            device: info.device,
            device_type: info.device_type,
            backend: info.backend,
            surface_supported: surface.is_none_or(|s| adapter.is_surface_supported(s)),
        }
    }

    /// Whether this candidate matches the given search filter (by name or backend).
    pub fn matches_filter(&self, filter: &str) -> bool {
        let filter_lower = filter.to_ascii_lowercase();
        self.name.to_ascii_lowercase().contains(&filter_lower) || format!("{:?}", self.backend).to_ascii_lowercase().contains(&filter_lower)
    }

    /// Whether this is a CPU or software rasterizer.
    pub fn is_software(&self) -> bool {
        self.device_type == DeviceType::Cpu
            || self.name.to_ascii_lowercase().contains("llvmpipe")
            || self.name.to_ascii_lowercase().contains("lavapipe")
            || self.name.to_ascii_lowercase().contains("warp")
    }
}

/// Chooses the best adapter index from the candidates based on filter, power preference, and surface support.
pub fn pick_adapter_index(candidates: &[Candidate], named_filter: Option<&str>, power: PowerPreference) -> Option<usize> {
    if candidates.is_empty() {
        return None;
    }

    candidates
        .iter()
        .enumerate()
        .min_by_key(|(orig_idx, c)| {
            // Surface support is required; unsupported adapters rank last.
            let surface_penalty = !c.surface_supported;

            // Explicit named filter (e.g. --gpu nvidia, WGPU_ADAPTER_NAME=3080).
            let name_penalty = match named_filter {
                Some(filter) => !c.matches_filter(filter),
                None => false,
            };

            // Hardware GPUs before software/CPU rasterizers.
            let software_penalty = c.is_software();

            // Native backends before OpenGL (except when preserving system order).
            let backend_rank = match (power, c.backend) {
                (PowerPreference::None, _) => 0,
                (_, Backend::Dx12) if cfg!(windows) => 1,
                (_, Backend::Vulkan | Backend::Metal | Backend::Dx12) => 0,
                (_, Backend::Gl) => 2,
                _ => 3,
            };

            // Power preference ranking.
            let power_rank = match power {
                PowerPreference::HighPerformance => match c.device_type {
                    DeviceType::DiscreteGpu => 0,
                    DeviceType::IntegratedGpu => 1,
                    DeviceType::Other => 2,
                    DeviceType::VirtualGpu => 3,
                    DeviceType::Cpu => 4,
                },
                PowerPreference::LowPower => match c.device_type {
                    DeviceType::IntegratedGpu => 0,
                    DeviceType::DiscreteGpu => 1,
                    DeviceType::Other => 2,
                    DeviceType::VirtualGpu => 3,
                    DeviceType::Cpu => 4,
                },
                PowerPreference::None => match c.device_type {
                    DeviceType::Cpu => 1,
                    _ => 0,
                },
            };

            (surface_penalty, name_penalty, software_penalty, power_rank, backend_rank, *orig_idx)
        })
        .map(|(idx, _)| idx)
}

/// Configures `options` with adapter selection honoring `--gpu`, `WGPU_ADAPTER_NAME`, and `WGPU_POWER_PREF`.
pub fn configure(options: &mut NativeOptions, cli_gpu: Option<String>, cli_power: Option<PowerPreference>) -> Result<(), String> {
    let effective_filter = cli_gpu.or_else(|| std::env::var(ADAPTER_NAME_ENV).ok());
    let effective_power = match cli_power {
        Some(p) => p,
        None => match std::env::var(POWER_PREF_ENV) {
            Ok(val) => parse_power_preference(&val)?,
            Err(_) => PowerPreference::HighPerformance,
        },
    };

    if let WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        create.power_preference = effective_power;
        let device_descriptor = create.device_descriptor.clone();
        create.device_descriptor = Arc::new(move |adapter| {
            let mut descriptor = device_descriptor(adapter);
            let available = adapter.limits();
            let requested = &mut descriptor.required_limits;
            // Wide 8K filters need a larger source region even when the output band is small.
            // Negotiating the limit does not allocate a buffer. Each band still sizes its own
            // working set against the resulting device limit.
            if requested.max_storage_buffer_binding_size > 0 {
                let filter_limit = (512 << 20).min(available.max_storage_buffer_binding_size).min(available.max_buffer_size);
                requested.max_storage_buffer_binding_size = requested.max_storage_buffer_binding_size.max(filter_limit);
                requested.max_buffer_size = requested.max_buffer_size.max(filter_limit);
            }
            descriptor
        });

        let filter = effective_filter.clone();
        create.native_adapter_selector = Some(Arc::new(move |adapters, surface| {
            let candidates: Vec<Candidate> = adapters.iter().map(|a| Candidate::new(a, surface)).collect();

            for (idx, c) in candidates.iter().enumerate() {
                log::info!("graphics adapter #{idx}: {} ({:?}, {:?}, surface supported: {})", c.name, c.device_type, c.backend, c.surface_supported);
            }

            let chosen_idx = pick_adapter_index(&candidates, filter.as_deref(), effective_power)
                .ok_or_else(|| "no graphics adapter found that can present to the window".to_string())?;

            let chosen = &candidates[chosen_idx];
            if !chosen.surface_supported {
                return Err("no graphics adapter found that can present to the window".to_string());
            }

            if let Some(ref f) = filter
                && !chosen.matches_filter(f)
            {
                log::warn!("no graphics adapter matched filter `{f}`; fell back to {}", chosen.name);
            }

            log::info!("selected graphics adapter: {} ({:?}, {:?})", chosen.name, chosen.device_type, chosen.backend);

            adapters.get(chosen_idx).cloned().ok_or_else(|| "selected adapter index out of range".to_string())
        }));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_candidate(name: &str, device_type: DeviceType, backend: Backend, surface_supported: bool) -> Candidate {
        Candidate { name: name.to_string(), vendor: 0, device: 0, device_type, backend, surface_supported }
    }

    #[test]
    fn parses_power_preferences_correctly() {
        assert_eq!(parse_power_preference("high").unwrap(), PowerPreference::HighPerformance);
        assert_eq!(parse_power_preference("high-performance").unwrap(), PowerPreference::HighPerformance);
        assert_eq!(parse_power_preference("low").unwrap(), PowerPreference::LowPower);
        assert_eq!(parse_power_preference("low-power").unwrap(), PowerPreference::LowPower);
        assert_eq!(parse_power_preference("none").unwrap(), PowerPreference::None);
        assert_eq!(parse_power_preference("default").unwrap(), PowerPreference::None);
        assert!(parse_power_preference("invalid").is_err());
    }

    #[test]
    fn high_performance_default_prefers_discrete_gpu() {
        let candidates = vec![
            dummy_candidate("AMD Radeon 660M", DeviceType::IntegratedGpu, Backend::Vulkan, true),
            dummy_candidate("NVIDIA GeForce RTX 2050", DeviceType::DiscreteGpu, Backend::Vulkan, true),
        ];

        let selected = pick_adapter_index(&candidates, None, PowerPreference::HighPerformance);
        assert_eq!(selected, Some(1));
    }

    #[test]
    fn low_power_prefers_integrated_gpu() {
        let candidates = vec![
            dummy_candidate("AMD Radeon 660M", DeviceType::IntegratedGpu, Backend::Vulkan, true),
            dummy_candidate("NVIDIA GeForce RTX 2050", DeviceType::DiscreteGpu, Backend::Vulkan, true),
        ];

        let selected = pick_adapter_index(&candidates, None, PowerPreference::LowPower);
        assert_eq!(selected, Some(0));
    }

    #[test]
    fn none_power_preference_preserves_enumerated_order() {
        let candidates = vec![
            dummy_candidate("GPU A (GL)", DeviceType::DiscreteGpu, Backend::Gl, true),
            dummy_candidate("GPU B (Vulkan)", DeviceType::DiscreteGpu, Backend::Vulkan, true),
        ];

        let selected = pick_adapter_index(&candidates, None, PowerPreference::None);
        assert_eq!(selected, Some(0));
    }

    #[cfg(windows)]
    #[test]
    fn windows_prefers_vulkan_and_keeps_dx12_available() {
        let dx12 = dummy_candidate("Discrete GPU", DeviceType::DiscreteGpu, Backend::Dx12, true);
        let vulkan = dummy_candidate("Discrete GPU", DeviceType::DiscreteGpu, Backend::Vulkan, true);
        let mut candidates = [dx12, vulkan];
        let power = PowerPreference::HighPerformance;
        assert_eq!(pick_adapter_index(&candidates, None, power), Some(1));
        assert_eq!(pick_adapter_index(&candidates, Some("dx12"), power), Some(0));
        assert_eq!(pick_adapter_index(&candidates, None, PowerPreference::None), Some(0));
        candidates[1].surface_supported = false;
        assert_eq!(pick_adapter_index(&candidates, None, power), Some(0));
    }

    #[test]
    fn named_filter_selects_specific_gpu() {
        let candidates = vec![
            dummy_candidate("Intel(R) UHD Graphics 770", DeviceType::IntegratedGpu, Backend::Vulkan, true),
            dummy_candidate("NVIDIA GeForce RTX 3080", DeviceType::DiscreteGpu, Backend::Vulkan, true),
            dummy_candidate("AMD Radeon RX 7900 XTX", DeviceType::DiscreteGpu, Backend::Vulkan, true),
        ];

        // Match by "3080"
        let selected = pick_adapter_index(&candidates, Some("3080"), PowerPreference::HighPerformance);
        assert_eq!(selected, Some(1));

        // Match by "intel"
        let selected_intel = pick_adapter_index(&candidates, Some("intel"), PowerPreference::HighPerformance);
        assert_eq!(selected_intel, Some(0));

        // Match by "7900"
        let selected_amd = pick_adapter_index(&candidates, Some("7900"), PowerPreference::HighPerformance);
        assert_eq!(selected_amd, Some(2));
    }

    #[test]
    fn unsupported_surface_is_ranked_last() {
        let candidates = vec![
            dummy_candidate("NVIDIA GeForce RTX 3080", DeviceType::DiscreteGpu, Backend::Vulkan, false),
            dummy_candidate("Intel(R) UHD Graphics 770", DeviceType::IntegratedGpu, Backend::Vulkan, true),
        ];

        // Even with HighPerformance, RTX 3080 cannot present to the surface, so Intel is chosen.
        let selected = pick_adapter_index(&candidates, None, PowerPreference::HighPerformance);
        assert_eq!(selected, Some(1));
    }

    #[test]
    fn hardware_gpu_preferred_over_software() {
        let candidates = vec![
            dummy_candidate("llvmpipe (LLVM 18.1.8, 256 bits)", DeviceType::Cpu, Backend::Vulkan, true),
            dummy_candidate("Intel(R) UHD Graphics 620", DeviceType::IntegratedGpu, Backend::Vulkan, true),
        ];

        let selected = pick_adapter_index(&candidates, None, PowerPreference::HighPerformance);
        assert_eq!(selected, Some(1));
    }
}
