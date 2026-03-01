//! Network quality detection for adaptive sync strategies.
//!
//! Detects network conditions and recommends a sync strategy:
//! - **FullSync**: Upload everything immediately (good connection)
//! - **DeferLargeFiles**: Upload small files now, defer large ones (mediocre)
//! - **OfflineQueue**: Queue all uploads for later (offline/metered)
//!
//! # Detection Strategy
//!
//! Uses a lightweight HTTP HEAD request to the cloud endpoint to measure
//! latency. Falls back to optimistic (FullSync) if detection fails.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

// ── Types ────────────────────────────────────────────────────────────────

/// Detected network quality.
#[derive(Debug, Clone)]
pub struct NetworkQuality {
    /// Round-trip latency to cloud endpoint (ms)
    pub latency_ms: u32,
    /// Detected connection type
    pub connection_type: ConnectionType,
    /// Whether the connection appears metered
    pub is_metered: bool,
    /// Whether the device is online
    pub is_online: bool,
}

impl Default for NetworkQuality {
    fn default() -> Self {
        Self {
            latency_ms: 0,
            connection_type: ConnectionType::Unknown,
            is_metered: false,
            is_online: true,
        }
    }
}

/// Connection type (best-effort detection).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionType {
    Wifi,
    Ethernet,
    Cellular,
    Unknown,
}

impl std::fmt::Display for ConnectionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionType::Wifi => write!(f, "WiFi"),
            ConnectionType::Ethernet => write!(f, "Ethernet"),
            ConnectionType::Cellular => write!(f, "Cellular"),
            ConnectionType::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Recommended sync strategy based on network quality.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncStrategy {
    /// Upload everything immediately — good connection detected.
    FullSync,
    /// Upload small files (<10 MB) now, defer large files to next cycle.
    DeferLargeFiles,
    /// Queue all uploads for later — offline or metered connection.
    OfflineQueue,
}

impl SyncStrategy {
    /// Size threshold for "large file" in DeferLargeFiles mode.
    pub const LARGE_FILE_THRESHOLD: u64 = 10 * 1024 * 1024; // 10 MB

    /// Whether a file of the given size should be synced in this strategy.
    pub fn should_sync(&self, file_size: u64) -> bool {
        match self {
            SyncStrategy::FullSync => true,
            SyncStrategy::DeferLargeFiles => file_size < Self::LARGE_FILE_THRESHOLD,
            SyncStrategy::OfflineQueue => false,
        }
    }
}

impl std::fmt::Display for SyncStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncStrategy::FullSync => write!(f, "Full sync"),
            SyncStrategy::DeferLargeFiles => write!(f, "Defer large files"),
            SyncStrategy::OfflineQueue => write!(f, "Offline queue"),
        }
    }
}

// ── Detection ────────────────────────────────────────────────────────────

/// Detect network quality by probing a URL.
///
/// Sends a lightweight HTTP HEAD request to measure latency.
/// If the probe fails, returns offline status.
pub async fn detect_quality(probe_url: Option<&str>) -> NetworkQuality {
    let url = probe_url.unwrap_or("https://www.google.com/generate_204");

    debug!("[cloud/network] Probing: {}", url);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    let start = Instant::now();
    match client.head(url).send().await {
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as u32;
            let is_online = resp.status().is_success()
                || resp.status().is_redirection()
                || resp.status().as_u16() == 204;

            let quality = NetworkQuality {
                latency_ms,
                connection_type: detect_connection_type(),
                is_metered: false, // Conservative: assume not metered
                is_online,
            };

            debug!(
                "[cloud/network] Probe result: {}ms, online={}, type={}",
                quality.latency_ms, quality.is_online, quality.connection_type
            );

            quality
        }
        Err(e) => {
            let latency_ms = start.elapsed().as_millis() as u32;

            if e.is_timeout() {
                warn!(
                    "[cloud/network] Probe timed out after {}ms — treating as slow",
                    latency_ms
                );
                NetworkQuality {
                    latency_ms,
                    connection_type: detect_connection_type(),
                    is_metered: false,
                    is_online: true, // Timeout != offline
                }
            } else {
                warn!("[cloud/network] Probe failed: {} — treating as offline", e);
                NetworkQuality {
                    latency_ms: 0,
                    connection_type: ConnectionType::Unknown,
                    is_metered: false,
                    is_online: false,
                }
            }
        }
    }
}

/// Recommend a sync strategy based on detected network quality.
pub fn recommend_strategy(quality: &NetworkQuality) -> SyncStrategy {
    if !quality.is_online {
        info!("[cloud/network] Strategy: OfflineQueue (device offline)");
        return SyncStrategy::OfflineQueue;
    }

    if quality.is_metered {
        info!("[cloud/network] Strategy: OfflineQueue (metered connection)");
        return SyncStrategy::OfflineQueue;
    }

    if quality.connection_type == ConnectionType::Cellular {
        info!("[cloud/network] Strategy: DeferLargeFiles (cellular)");
        return SyncStrategy::DeferLargeFiles;
    }

    let strategy = match quality.latency_ms {
        0..=100 => SyncStrategy::FullSync,
        101..=500 => SyncStrategy::DeferLargeFiles,
        _ => SyncStrategy::OfflineQueue,
    };

    info!(
        "[cloud/network] Strategy: {} (latency: {}ms, type: {})",
        strategy, quality.latency_ms, quality.connection_type
    );

    strategy
}

/// Detect connection type (best-effort, platform-dependent).
fn detect_connection_type() -> ConnectionType {
    // On macOS we could use SCNetworkReachability or NWPathMonitor,
    // but those require Objective-C bridging. For now, default to Unknown.
    // The latency-based strategy works well enough without this.
    ConnectionType::Unknown
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recommend_strategy_fast() {
        let q = NetworkQuality {
            latency_ms: 30,
            connection_type: ConnectionType::Wifi,
            is_metered: false,
            is_online: true,
        };
        assert_eq!(recommend_strategy(&q), SyncStrategy::FullSync);
    }

    #[test]
    fn test_recommend_strategy_medium() {
        let q = NetworkQuality {
            latency_ms: 200,
            connection_type: ConnectionType::Wifi,
            is_metered: false,
            is_online: true,
        };
        assert_eq!(recommend_strategy(&q), SyncStrategy::DeferLargeFiles);
    }

    #[test]
    fn test_recommend_strategy_slow() {
        let q = NetworkQuality {
            latency_ms: 1500,
            connection_type: ConnectionType::Unknown,
            is_metered: false,
            is_online: true,
        };
        assert_eq!(recommend_strategy(&q), SyncStrategy::OfflineQueue);
    }

    #[test]
    fn test_recommend_strategy_offline() {
        let q = NetworkQuality {
            latency_ms: 0,
            connection_type: ConnectionType::Unknown,
            is_metered: false,
            is_online: false,
        };
        assert_eq!(recommend_strategy(&q), SyncStrategy::OfflineQueue);
    }

    #[test]
    fn test_recommend_strategy_metered() {
        let q = NetworkQuality {
            latency_ms: 20,
            connection_type: ConnectionType::Wifi,
            is_metered: true,
            is_online: true,
        };
        assert_eq!(recommend_strategy(&q), SyncStrategy::OfflineQueue);
    }

    #[test]
    fn test_recommend_strategy_cellular() {
        let q = NetworkQuality {
            latency_ms: 50,
            connection_type: ConnectionType::Cellular,
            is_metered: false,
            is_online: true,
        };
        assert_eq!(recommend_strategy(&q), SyncStrategy::DeferLargeFiles);
    }

    #[test]
    fn test_sync_strategy_should_sync() {
        // FullSync syncs everything
        assert!(SyncStrategy::FullSync.should_sync(100_000_000));

        // DeferLargeFiles: small files yes, large files no
        assert!(SyncStrategy::DeferLargeFiles.should_sync(1_000_000)); // 1 MB
        assert!(!SyncStrategy::DeferLargeFiles.should_sync(50_000_000)); // 50 MB

        // OfflineQueue: nothing
        assert!(!SyncStrategy::OfflineQueue.should_sync(100));
    }

    #[test]
    fn test_connection_type_display() {
        assert_eq!(format!("{}", ConnectionType::Wifi), "WiFi");
        assert_eq!(format!("{}", ConnectionType::Ethernet), "Ethernet");
        assert_eq!(format!("{}", ConnectionType::Cellular), "Cellular");
        assert_eq!(format!("{}", ConnectionType::Unknown), "Unknown");
    }

    #[test]
    fn test_sync_strategy_display() {
        assert_eq!(format!("{}", SyncStrategy::FullSync), "Full sync");
        assert_eq!(
            format!("{}", SyncStrategy::DeferLargeFiles),
            "Defer large files"
        );
        assert_eq!(format!("{}", SyncStrategy::OfflineQueue), "Offline queue");
    }
}
