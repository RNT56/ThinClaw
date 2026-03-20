//! Camera capture tool.
//!
//! Captures a photo from the system camera and saves it to a file.
//! Uses platform-native commands:
//! - macOS: `imagesnap` (Homebrew) or `ffmpeg`
//! - Linux: `fswebcam` or `ffmpeg`
//! - Windows: `ffmpeg`
//!
//! This replaces `CameraCommands.swift` from the companion app.

use std::path::PathBuf;
use std::time::Instant;

use async_trait::async_trait;
use tokio::process::Command;

use crate::context::JobContext;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput};

/// Camera capture tool.
pub struct CameraCaptureTool;

impl Default for CameraCaptureTool {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraCaptureTool {
    pub fn new() -> Self {
        Self
    }
}

impl std::fmt::Debug for CameraCaptureTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CameraCaptureTool").finish()
    }
}

/// Determine the output path for a camera capture.
fn capture_path(custom: Option<&str>) -> PathBuf {
    if let Some(p) = custom {
        PathBuf::from(p)
    } else {
        let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(format!(".ironclaw/camera/capture_{ts}.jpg"))
    }
}

/// Capture from camera on macOS.
#[cfg(target_os = "macos")]
async fn capture_camera(path: &std::path::Path, warmup_secs: f32) -> Result<String, ToolError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Create camera dir: {e}")))?;
    }

    // Try imagesnap first (most reliable on macOS)
    let imagesnap = Command::new("imagesnap")
        .arg("-w")
        .arg(format!("{warmup_secs}"))
        .arg(path.to_string_lossy().as_ref())
        .output()
        .await;

    if let Ok(output) = imagesnap
        && output.status.success()
        && path.exists()
    {
        return Ok("imagesnap".to_string());
    }

    // Fallback to ffmpeg
    let ffmpeg = Command::new("ffmpeg")
        .args([
            "-f",
            "avfoundation",
            "-framerate",
            "30",
            "-video_size",
            "1280x720",
            "-i",
            "0",
            "-frames:v",
            "1",
            "-y",
            &path.to_string_lossy(),
        ])
        .output()
        .await;

    if let Ok(output) = ffmpeg
        && output.status.success()
        && path.exists()
    {
        return Ok("ffmpeg".to_string());
    }

    Err(ToolError::ExecutionFailed(
        "No camera tool found. Install imagesnap (brew install imagesnap) or ffmpeg.".to_string(),
    ))
}

/// Capture from camera on Linux.
#[cfg(target_os = "linux")]
async fn capture_camera(path: &std::path::Path, _warmup_secs: f32) -> Result<String, ToolError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Create camera dir: {e}")))?;
    }

    // Try fswebcam first
    let fswebcam = Command::new("fswebcam")
        .args(["-r", "1280x720", "--no-banner", &path.to_string_lossy()])
        .output()
        .await;

    if let Ok(output) = fswebcam {
        if output.status.success() && path.exists() {
            return Ok("fswebcam".to_string());
        }
    }

    // Fallback to ffmpeg
    let ffmpeg = Command::new("ffmpeg")
        .args([
            "-f",
            "v4l2",
            "-video_size",
            "1280x720",
            "-i",
            "/dev/video0",
            "-frames:v",
            "1",
            "-y",
            &path.to_string_lossy(),
        ])
        .output()
        .await;

    if let Ok(output) = ffmpeg {
        if output.status.success() && path.exists() {
            return Ok("ffmpeg".to_string());
        }
    }

    Err(ToolError::ExecutionFailed(
        "No camera tool found. Install fswebcam or ffmpeg.".to_string(),
    ))
}

/// Capture from camera on Windows.
#[cfg(target_os = "windows")]
async fn capture_camera(path: &std::path::Path, _warmup_secs: f32) -> Result<String, ToolError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Create camera dir: {e}")))?;
    }

    let ffmpeg = Command::new("ffmpeg")
        .args([
            "-f",
            "dshow",
            "-video_size",
            "1280x720",
            "-i",
            "video=0",
            "-frames:v",
            "1",
            "-y",
            &path.to_string_lossy(),
        ])
        .output()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("ffmpeg: {e}")))?;

    if ffmpeg.status.success() && path.exists() {
        return Ok("ffmpeg".to_string());
    }

    Err(ToolError::ExecutionFailed(
        "Camera capture failed. Install ffmpeg.".to_string(),
    ))
}

#[async_trait]
impl Tool for CameraCaptureTool {
    fn name(&self) -> &str {
        "camera_capture"
    }

    fn description(&self) -> &str {
        "Capture a photo from the system camera and save to a JPEG file. \
         On macOS: uses imagesnap or ffmpeg. \
         On Linux: uses fswebcam or ffmpeg. \
         Returns the file path and size."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "output_path": {
                    "type": "string",
                    "description": "Custom output file path. Default: ~/.ironclaw/camera/capture_<timestamp>.jpg"
                },
                "warmup_seconds": {
                    "type": "number",
                    "description": "Camera warmup time in seconds (macOS only). Default: 1.0",
                    "default": 1.0,
                    "minimum": 0.0,
                    "maximum": 10.0
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let custom_path = params.get("output_path").and_then(|v| v.as_str());
        let warmup = params
            .get("warmup_seconds")
            .and_then(|v| v.as_f64())
            .map(|w| w.min(10.0) as f32)
            .unwrap_or(1.0);

        let path = capture_path(custom_path);
        let tool_used = capture_camera(&path, warmup).await?;

        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Read capture metadata: {e}")))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "path": path.to_string_lossy(),
                "size_bytes": metadata.len(),
                "tool_used": tool_used,
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Always // Camera access is privacy-sensitive
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_name() {
        let tool = CameraCaptureTool::new();
        assert_eq!(tool.name(), "camera_capture");
    }

    #[test]
    fn test_capture_path_default() {
        let path = capture_path(None);
        assert!(path.to_string_lossy().contains("capture_"));
        assert!(path.to_string_lossy().ends_with(".jpg"));
    }

    #[test]
    fn test_capture_path_custom() {
        let path = capture_path(Some("/tmp/cam.jpg"));
        assert_eq!(path, PathBuf::from("/tmp/cam.jpg"));
    }

    #[test]
    fn test_approval_always() {
        let tool = CameraCaptureTool::new();
        assert!(matches!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Always
        ));
    }
}
