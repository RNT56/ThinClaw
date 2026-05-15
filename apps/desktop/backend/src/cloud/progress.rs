//! Migration progress reporting via Tauri events.
//!
//! Emits structured `cloud_migration_progress` events that the frontend
//! uses to render the migration progress dialog.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;

/// Progress event emitted to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationProgress {
    /// Unique migration ID
    pub migration_id: String,
    /// Direction: "to_cloud" or "to_local"
    pub direction: String,
    /// Current phase
    pub phase: MigrationPhase,
    /// Overall percentage (0.0 - 100.0)
    pub overall_percent: f64,
    /// Current phase progress (0.0 - 100.0)
    pub phase_percent: f64,
    /// Files processed so far
    pub files_done: u32,
    /// Total files to process
    pub files_total: u32,
    /// Bytes transferred so far
    pub bytes_done: u64,
    /// Total bytes to transfer
    pub bytes_total: u64,
    /// Current transfer speed (bytes/sec)
    pub speed_bps: u64,
    /// Estimated time remaining (seconds)
    pub eta_seconds: Option<u64>,
    /// Human-readable status message
    pub message: String,
    /// Whether the migration is complete
    pub complete: bool,
    /// Error message if failed
    pub error: Option<String>,
}

/// Migration phases (displayed as a checklist in the UI).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MigrationPhase {
    Preflight,
    DatabaseSnapshot,
    EncryptingFiles,
    UploadingDatabase,
    UploadingDocuments,
    UploadingImages,
    UploadingGeneratedImages,
    UploadingVectors,
    UploadingAgentState,
    UploadingManifest,
    Verification,
    Cleanup,
    Complete,
    // Download phases (cloud → local)
    DownloadingManifest,
    DownloadingFiles,
    RestoringDatabase,
    RebuildingVectors,
}

impl MigrationPhase {
    /// Human-readable label
    pub fn label(&self) -> &str {
        match self {
            Self::Preflight => "Pre-flight checks",
            Self::DatabaseSnapshot => "Database snapshot",
            Self::EncryptingFiles => "Encrypting files",
            Self::UploadingDatabase => "Uploading database",
            Self::UploadingDocuments => "Uploading documents",
            Self::UploadingImages => "Uploading images",
            Self::UploadingGeneratedImages => "Uploading generated images",
            Self::UploadingVectors => "Uploading vector indices",
            Self::UploadingAgentState => "Uploading agent state",
            Self::UploadingManifest => "Uploading manifest",
            Self::Verification => "Verifying archive",
            Self::Cleanup => "Cleaning up",
            Self::Complete => "Complete",
            Self::DownloadingManifest => "Downloading manifest",
            Self::DownloadingFiles => "Downloading files",
            Self::RestoringDatabase => "Restoring database",
            Self::RebuildingVectors => "Rebuilding vector indices",
        }
    }
}

/// Tracks migration progress and emits events to the frontend.
pub struct ProgressTracker {
    app: AppHandle,
    migration_id: String,
    direction: String,
    files_total: u32,
    files_done: u32,
    bytes_total: u64,
    bytes_done: u64,
    phase: MigrationPhase,
    start_time: Instant,
    cancelled: Arc<RwLock<bool>>,
}

impl ProgressTracker {
    /// Create a new progress tracker.
    pub fn new(
        app: AppHandle,
        migration_id: String,
        direction: String,
        files_total: u32,
        bytes_total: u64,
    ) -> Self {
        Self {
            app,
            migration_id,
            direction,
            files_total,
            files_done: 0,
            bytes_total,
            bytes_done: 0,
            phase: MigrationPhase::Preflight,
            start_time: Instant::now(),
            cancelled: Arc::new(RwLock::new(false)),
        }
    }

    /// Get a clone of the cancellation flag (for the cancel command).
    pub fn cancel_flag(&self) -> Arc<RwLock<bool>> {
        self.cancelled.clone()
    }

    /// Check if the migration has been cancelled.
    pub async fn is_cancelled(&self) -> bool {
        *self.cancelled.read().await
    }

    /// Set the current phase and emit progress.
    pub fn set_phase(&mut self, phase: MigrationPhase) {
        self.phase = phase;
        self.emit();
    }

    /// Record a file transfer and emit progress.
    pub fn file_done(&mut self, bytes: u64) {
        self.files_done += 1;
        self.bytes_done += bytes;
        self.emit();
    }

    /// Add bytes without incrementing file count (for partial transfers).
    pub fn add_bytes(&mut self, bytes: u64) {
        self.bytes_done += bytes;
        self.emit();
    }

    /// Get the number of files processed.
    pub fn files_done(&self) -> u32 {
        self.files_done
    }

    /// Get the number of bytes processed.
    pub fn bytes_done(&self) -> u64 {
        self.bytes_done
    }

    /// Mark migration as complete.
    pub fn complete(&mut self) {
        self.phase = MigrationPhase::Complete;
        self.emit();
    }

    /// Mark migration as failed.
    pub fn fail(&mut self, error: String) {
        let progress = MigrationProgress {
            migration_id: self.migration_id.clone(),
            direction: self.direction.clone(),
            phase: self.phase.clone(),
            overall_percent: self.overall_percent(),
            phase_percent: 0.0,
            files_done: self.files_done,
            files_total: self.files_total,
            bytes_done: self.bytes_done,
            bytes_total: self.bytes_total,
            speed_bps: self.speed_bps(),
            eta_seconds: None,
            message: format!("Migration failed: {}", error),
            complete: false,
            error: Some(error),
        };

        let _ = self.app.emit("cloud_migration_progress", &progress);
    }

    /// Emit current progress as a Tauri event.
    fn emit(&self) {
        let overall = self.overall_percent();
        let speed = self.speed_bps();
        let eta = if speed > 0 && self.bytes_done < self.bytes_total {
            Some((self.bytes_total - self.bytes_done) / speed)
        } else {
            None
        };

        let progress = MigrationProgress {
            migration_id: self.migration_id.clone(),
            direction: self.direction.clone(),
            phase: self.phase.clone(),
            overall_percent: overall,
            phase_percent: if self.files_total > 0 {
                (self.files_done as f64 / self.files_total as f64) * 100.0
            } else {
                0.0
            },
            files_done: self.files_done,
            files_total: self.files_total,
            bytes_done: self.bytes_done,
            bytes_total: self.bytes_total,
            speed_bps: speed,
            eta_seconds: eta,
            message: format!(
                "{} ({}/{})",
                self.phase.label(),
                self.files_done,
                self.files_total
            ),
            complete: self.phase == MigrationPhase::Complete,
            error: None,
        };

        let _ = self.app.emit("cloud_migration_progress", &progress);
    }

    fn overall_percent(&self) -> f64 {
        if self.bytes_total == 0 {
            return 0.0;
        }
        (self.bytes_done as f64 / self.bytes_total as f64) * 100.0
    }

    fn speed_bps(&self) -> u64 {
        let elapsed = self.start_time.elapsed().as_secs();
        if elapsed == 0 {
            return 0;
        }
        self.bytes_done / elapsed
    }
}
