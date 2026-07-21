use serde::Serialize;
use specta::Type;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex, OnceLock,
};
use std::time::Duration;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, ProcessesToUpdate, RefreshKind, System};
use tauri::{command, State};

static SYS: OnceLock<Mutex<System>> = OnceLock::new();
static STARTUP_READY_MS: AtomicU64 = AtomicU64::new(0);
const STARTUP_BUDGET_MS: u64 = 8_000;
const BYTES_PER_GIB: f64 = 1024.0 * 1024.0 * 1024.0;

fn get_sys() -> &'static Mutex<System> {
    SYS.get_or_init(|| {
        let mut sys = System::new_with_specifics(
            RefreshKind::nothing()
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
    /// Resident memory of the Tauri desktop process alone.
    pub desktop_memory: f64,
    /// Resident memory of all descendant sidecar processes.
    pub sidecar_memory: f64,
    /// Desktop + descendant sidecar resident memory.
    pub app_memory: f64,
    /// Configured app+sidecar memory budget (zero when disabled).
    pub memory_ceiling: f64,
    pub memory_budget_exceeded: bool,
    /// Backend initialization time captured immediately before the event loop.
    pub startup_ready_ms: u64,
    pub startup_budget_ms: u64,
    pub startup_budget_exceeded: bool,
    pub memory_bandwidth_gbps: f32,
}

#[command]
#[specta::specta]
pub fn get_system_specs(config: State<'_, crate::config::ConfigManager>) -> SystemSpecs {
    let mut sys = get_sys().lock().unwrap_or_else(|e| e.into_inner());

    // Refresh CPU usage
    sys.refresh_cpu_all();

    let cpu = sys.cpus().first();
    let cpu_brand = cpu
        .map(|c| c.brand().to_string())
        .unwrap_or_else(|| "Unknown".to_string());
    let cpu_usage = sys.global_cpu_usage();

    let platform = System::name().unwrap_or("Unknown".to_string());

    // Get cumulative memory of app + sidecars. `get_current_pid()` can fail on
    // unsupported platforms; degrade to reporting 0 app memory rather than
    // panicking the Tauri command (which would surface as a hard error in the UI).
    let my_pid = sysinfo::get_current_pid().ok();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let mut desktop_memory = 0.0;
    let mut sidecar_memory = 0.0;
    if let Some(my_pid) = my_pid {
        if let Some(process) = sys.process(my_pid) {
            desktop_memory = process.memory() as f64;
        }

        // Include every descendant, not only direct children: Python and model
        // launchers commonly fork the actual inference worker one level deeper.
        for (pid, process) in sys.processes() {
            if *pid != my_pid && is_descendant_of(&sys, *pid, my_pid) {
                sidecar_memory += process.memory() as f64;
            }
        }
    }
    let total_app_memory = desktop_memory + sidecar_memory;
    let user_config = config.get_config();
    let memory_ceiling = if user_config.enable_memory_reservation {
        user_config.memory_reservation_gb as f64 * BYTES_PER_GIB
    } else {
        0.0
    };
    let startup_ready_ms = STARTUP_READY_MS.load(Ordering::Relaxed);

    SystemSpecs {
        total_memory: sys.total_memory() as f64,
        used_memory: sys.used_memory() as f64,
        memory_bandwidth_gbps: detect_bandwidth(&cpu_brand),
        cpu_brand,
        cpu_usage,
        cpu_cores: sys.cpus().len() as u16,
        platform,
        desktop_memory,
        sidecar_memory,
        app_memory: total_app_memory,
        memory_ceiling,
        memory_budget_exceeded: exceeds_memory_budget(total_app_memory, memory_ceiling),
        startup_ready_ms,
        startup_budget_ms: STARTUP_BUDGET_MS,
        startup_budget_exceeded: startup_ready_ms > STARTUP_BUDGET_MS,
    }
}

pub fn record_startup_ready(duration: Duration) {
    STARTUP_READY_MS.store(
        duration.as_millis().min(u64::MAX as u128) as u64,
        Ordering::Relaxed,
    );
}

fn is_descendant_of(system: &System, pid: sysinfo::Pid, ancestor: sysinfo::Pid) -> bool {
    let mut current = system.process(pid).and_then(|process| process.parent());
    for _ in 0..64 {
        let Some(parent) = current else { return false };
        if parent == ancestor {
            return true;
        }
        current = system.process(parent).and_then(|process| process.parent());
    }
    false
}

fn exceeds_memory_budget(usage_bytes: f64, ceiling_bytes: f64) -> bool {
    ceiling_bytes > 0.0 && usage_bytes > ceiling_bytes
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_memory_budget_is_strictly_bounded() {
        let ceiling = 8.0 * BYTES_PER_GIB;
        assert!(!exceeds_memory_budget(7.9 * BYTES_PER_GIB, ceiling));
        assert!(!exceeds_memory_budget(ceiling, ceiling));
        assert!(exceeds_memory_budget(8.1 * BYTES_PER_GIB, ceiling));
        assert!(!exceeds_memory_budget(100.0 * BYTES_PER_GIB, 0.0));
    }
}
