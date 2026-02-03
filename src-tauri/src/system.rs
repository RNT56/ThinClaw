use serde::Serialize;
use specta::Type;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
use tauri::command;

#[derive(Debug, Serialize, Type)]
pub struct SystemSpecs {
    pub total_memory: f64,
    pub used_memory: f64,
    pub cpu_brand: String,
    pub cpu_usage: f32,
    pub cpu_cores: u16,
    pub platform: String,
}

#[command]
#[specta::specta]
pub fn get_system_specs() -> SystemSpecs {
    let mut sys = System::new_with_specifics(
        RefreshKind::new()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything()),
    );

    // Wait a bit to ensure CPU usage is accurate (needs two measurements usually, but for brand/cores it's fine)
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu();
    sys.refresh_memory();

    let cpu = sys.cpus().first();
    let cpu_brand = cpu
        .map(|c| c.brand().to_string())
        .unwrap_or_else(|| "Unknown".to_string());
    let cpu_usage = sys.global_cpu_info().cpu_usage();

    let platform = System::name().unwrap_or("Unknown".to_string());

    SystemSpecs {
        total_memory: sys.total_memory() as f64,
        used_memory: sys.used_memory() as f64,
        cpu_brand,
        cpu_usage,
        cpu_cores: sys.cpus().len() as u16,
        platform,
    }
}
