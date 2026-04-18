//! Device information tool using the `sysinfo` crate.
//!
//! Provides system information: CPU, memory, disk, OS, uptime, hostname.
//! This is the Rust replacement for the Swift `DeviceCommands.swift` module.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Serialize;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolMetadata, ToolOutput, ToolRouteIntent};

/// System/device information tool.
///
/// Gathers system metrics without any external API calls or side effects.
pub struct DeviceInfoTool;

impl Default for DeviceInfoTool {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceInfoTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Serialize)]
struct CpuInfo {
    /// Name of the CPU (e.g., "Apple M2 Pro")
    name: String,
    /// Number of physical cores
    physical_cores: usize,
    /// Number of logical CPUs (threads)
    logical_cpus: usize,
    /// CPU architecture
    arch: String,
    /// Per-core CPU usage percentages (refreshed)
    usage_per_core: Vec<f32>,
    /// Average CPU usage across all cores
    average_usage: f32,
}

#[derive(Debug, Serialize)]
struct MemoryInfo {
    /// Total physical RAM in bytes
    total_bytes: u64,
    /// Used RAM in bytes
    used_bytes: u64,
    /// Available RAM in bytes
    available_bytes: u64,
    /// Human-readable total
    total: String,
    /// Human-readable used
    used: String,
    /// Human-readable available
    available: String,
    /// Usage percentage
    usage_percent: f64,
    /// Total swap in bytes
    swap_total_bytes: u64,
    /// Used swap in bytes
    swap_used_bytes: u64,
}

#[derive(Debug, Serialize)]
struct DiskInfo {
    name: String,
    mount_point: String,
    total_bytes: u64,
    available_bytes: u64,
    total: String,
    available: String,
    usage_percent: f64,
    file_system: String,
}

#[derive(Debug, Serialize)]
struct OsInfo {
    name: String,
    version: String,
    kernel_version: String,
    hostname: String,
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let minutes = (seconds % 3600) / 60;

    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

#[async_trait]
impl Tool for DeviceInfoTool {
    fn name(&self) -> &str {
        "device_info"
    }

    fn description(&self) -> &str {
        "Inspect the current machine's hardware and OS state. Use this for questions \
         about CPU, memory, disks, uptime, hostname, or operating-system details, \
         especially when troubleshooting local environment issues."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "include": {
                    "type": "array",
                    "description": "Optional filter — which sections to include. Default: all. Options: cpu, memory, disks, os, uptime",
                    "items": {
                        "type": "string",
                        "enum": ["cpu", "memory", "disks", "os", "uptime"]
                    }
                }
            },
            "required": []
        })
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata::live_authoritative(ToolRouteIntent::LocalState)
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let include: Option<Vec<String>> = params
            .get("include")
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        let include_all = include.is_none();
        let includes = |section: &str| -> bool {
            include_all
                || include
                    .as_ref()
                    .is_some_and(|list| list.iter().any(|s| s == section))
        };

        use sysinfo::System;

        let mut sys = System::new();

        // Build the result object conditionally
        let mut result = serde_json::Map::new();

        // OS info
        if includes("os") {
            result.insert(
                "os".to_string(),
                serde_json::to_value(OsInfo {
                    name: System::name().unwrap_or_else(|| "Unknown".to_string()),
                    version: System::os_version().unwrap_or_else(|| "Unknown".to_string()),
                    kernel_version: System::kernel_version()
                        .unwrap_or_else(|| "Unknown".to_string()),
                    hostname: System::host_name().unwrap_or_else(|| "Unknown".to_string()),
                })
                .unwrap_or_default(),
            );
        }

        // CPU info
        if includes("cpu") {
            sys.refresh_cpu_all();
            // Sleep briefly to get meaningful CPU usage (sysinfo needs two refreshes)
            tokio::time::sleep(Duration::from_millis(200)).await;
            sys.refresh_cpu_all();

            let cpus = sys.cpus();
            let usage_per_core: Vec<f32> = cpus.iter().map(|c| c.cpu_usage()).collect();
            let average_usage = if cpus.is_empty() {
                0.0
            } else {
                usage_per_core.iter().sum::<f32>() / cpus.len() as f32
            };

            result.insert(
                "cpu".to_string(),
                serde_json::to_value(CpuInfo {
                    name: cpus
                        .first()
                        .map(|c| c.brand().to_string())
                        .unwrap_or_else(|| "Unknown".to_string()),
                    physical_cores: System::physical_core_count().unwrap_or(0),
                    logical_cpus: cpus.len(),
                    arch: std::env::consts::ARCH.to_string(),
                    usage_per_core,
                    average_usage,
                })
                .unwrap_or_default(),
            );
        }

        // Memory info
        if includes("memory") {
            sys.refresh_memory();

            let total = sys.total_memory();
            let used = sys.used_memory();
            let available = sys.available_memory();
            let usage_percent = if total > 0 {
                (used as f64 / total as f64) * 100.0
            } else {
                0.0
            };

            result.insert(
                "memory".to_string(),
                serde_json::to_value(MemoryInfo {
                    total_bytes: total,
                    used_bytes: used,
                    available_bytes: available,
                    total: format_bytes(total),
                    used: format_bytes(used),
                    available: format_bytes(available),
                    usage_percent: (usage_percent * 10.0).round() / 10.0,
                    swap_total_bytes: sys.total_swap(),
                    swap_used_bytes: sys.used_swap(),
                })
                .unwrap_or_default(),
            );
        }

        // Disk info
        if includes("disks") {
            use sysinfo::Disks;
            let disks = Disks::new_with_refreshed_list();

            let disk_infos: Vec<DiskInfo> = disks
                .iter()
                .map(|d| {
                    let total = d.total_space();
                    let available = d.available_space();
                    let used = total.saturating_sub(available);
                    let usage_percent = if total > 0 {
                        ((used as f64 / total as f64) * 1000.0).round() / 10.0
                    } else {
                        0.0
                    };

                    DiskInfo {
                        name: d.name().to_string_lossy().to_string(),
                        mount_point: d.mount_point().to_string_lossy().to_string(),
                        total_bytes: total,
                        available_bytes: available,
                        total: format_bytes(total),
                        available: format_bytes(available),
                        usage_percent,
                        file_system: d.file_system().to_string_lossy().to_string(),
                    }
                })
                .collect();

            result.insert(
                "disks".to_string(),
                serde_json::to_value(disk_infos).unwrap_or_default(),
            );
        }

        // Uptime
        if includes("uptime") {
            let uptime_secs = System::uptime();
            result.insert("uptime_seconds".to_string(), serde_json::json!(uptime_secs));
            result.insert(
                "uptime_human".to_string(),
                serde_json::json!(format_uptime(uptime_secs)),
            );
        }

        Ok(ToolOutput::success(
            serde_json::Value::Object(result),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false // Local data only, no external content
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_info_tool_schema() {
        let tool = DeviceInfoTool::new();
        assert_eq!(tool.name(), "device_info");
        assert!(!tool.description().is_empty());

        let schema = tool.parameters_schema();
        assert!(schema.get("properties").is_some());
    }

    #[tokio::test]
    async fn test_device_info_returns_data() {
        let tool = DeviceInfoTool::new();
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"include": ["os", "memory"]}), &ctx)
            .await
            .unwrap();

        let obj = result.result.as_object().unwrap();
        assert!(obj.contains_key("os"));
        assert!(obj.contains_key("memory"));
        // Should not include cpu/disks when not requested
        assert!(!obj.contains_key("cpu"));
        assert!(!obj.contains_key("disks"));
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1_073_741_824), "1.0 GB");
        assert_eq!(format_bytes(1_099_511_627_776), "1.0 TB");
    }

    #[test]
    fn test_format_uptime() {
        assert_eq!(format_uptime(30), "0m");
        assert_eq!(format_uptime(60), "1m");
        assert_eq!(format_uptime(3661), "1h 1m");
        assert_eq!(format_uptime(90061), "1d 1h 1m");
    }
}
