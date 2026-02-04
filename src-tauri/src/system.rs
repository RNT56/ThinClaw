use serde::Serialize;
use specta::Type;
use std::sync::{Mutex, OnceLock};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
use tauri::command;

static SYS: OnceLock<Mutex<System>> = OnceLock::new();

fn get_sys() -> &'static Mutex<System> {
    SYS.get_or_init(|| {
        let mut sys = System::new_with_specifics(
            RefreshKind::new()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        );
        sys.refresh_all();
        Mutex::new(sys)
    })
}

#[derive(Debug, Serialize, Type)]
pub struct SystemSpecs {
    pub total_memory: f64,
    pub used_memory: f64,
    pub cpu_brand: String,
    pub cpu_usage: f32,
    pub cpu_cores: u16,
    pub platform: String,
    pub app_memory: f64,
    pub memory_bandwidth_gbps: f32,
}

#[command]
#[specta::specta]
pub fn get_system_specs() -> SystemSpecs {
    let mut sys = get_sys().lock().unwrap();

    // Refresh CPU usage
    sys.refresh_cpu();

    let cpu = sys.cpus().first();
    let cpu_brand = cpu
        .map(|c| c.brand().to_string())
        .unwrap_or_else(|| "Unknown".to_string());
    let cpu_usage = sys.global_cpu_info().cpu_usage();

    let platform = System::name().unwrap_or("Unknown".to_string());

    // Get cumulative memory of app + sidecars
    let my_pid = sysinfo::get_current_pid().unwrap();
    sys.refresh_processes();

    let mut total_app_memory = 0.0;
    if let Some(process) = sys.process(my_pid) {
        total_app_memory += process.memory() as f64;
    }

    // Include child processes (sidecars)
    for p in sys.processes().values() {
        if p.parent() == Some(my_pid) {
            total_app_memory += p.memory() as f64;
        }
    }

    SystemSpecs {
        total_memory: sys.total_memory() as f64,
        used_memory: sys.used_memory() as f64,
        memory_bandwidth_gbps: detect_bandwidth(&cpu_brand),
        cpu_brand,
        cpu_usage,
        cpu_cores: sys.cpus().len() as u16,
        platform,
        app_memory: total_app_memory,
    }
}

fn detect_bandwidth(brand: &str) -> f32 {
    let brand_lower = brand.to_lowercase();

    // Apple Silicon Bandwidth (Approximate/Conservative)
    if brand_lower.contains("apple")
        || brand_lower.contains("m1")
        || brand_lower.contains("m2")
        || brand_lower.contains("m3")
    {
        if brand_lower.contains("ultra") {
            return 800.0;
        }
        if brand_lower.contains("max") {
            return 400.0;
        }
        if brand_lower.contains("pro") {
            return 200.0;
        }
        return 100.0; // Base chips
    }

    // Fallback for PC (DDR4/DDR5 Dual Channel is typically 50-80 GB/s)
    60.0
}
