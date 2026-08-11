use settings::macros::define_settings_group;
use settings::{SupportedPlatforms, SyncToCloud};
use warp_core::features::FeatureFlag;
use warpui::platform::GraphicsBackend;

fn default_to_windows_high_performance_gpu() -> bool {
    cfg!(windows) && FeatureFlag::WindowsHighPerformanceGpuDefault.is_enabled()
}

define_settings_group!(GPUSettings, settings: [
    prefer_low_power_gpu: PreferLowPowerGPU {
        type: bool,
        // Opt for the low power (integrated) GPU on Windows / Linux since discrete GPUs tend to be
        // more unstable.
        default: cfg!(any(target_os = "linux", target_os = "freebsd"))
            || (cfg!(windows) && !default_to_windows_high_performance_gpu()),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "system.prefer_low_power_gpu",
        description: "Whether to prefer the integrated (low-power) GPU.",
    },
    preferred_backend: PreferredGraphicsBackend {
        type: Option<GraphicsBackend>,
        default: default_to_windows_high_performance_gpu().then_some(GraphicsBackend::Vulkan),
        supported_platforms: SupportedPlatforms::WINDOWS,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "system.preferred_graphics_backend",
        description: "The preferred graphics backend on Windows.",
    },
]);
