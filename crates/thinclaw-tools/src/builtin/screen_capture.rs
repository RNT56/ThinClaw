//! Screen capture tool.
//!
//! Captures the screen (or a specific window) and saves it to a file.
//! Uses platform-native commands:
//! - macOS: `screencapture` CLI (built-in)
//! - Linux: `gnome-screenshot`, `scrot`, or `import` (ImageMagick)
//! - Windows: PowerShell snippet
//!
//! This replaces `ScreenCommands.swift` from the companion app.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::process::Command;

use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput};
use thinclaw_types::JobContext;

use super::capture_target::{CaptureFormat, CaptureTarget};
use crate::execution::bounded_command_output;

const CAPTURE_HELPER_TIMEOUT: Duration = Duration::from_secs(45);
const CAPTURE_STDOUT_LIMIT: usize = 256 * 1024;
const CAPTURE_STDERR_LIMIT: usize = 256 * 1024;

#[cfg(target_os = "windows")]
const WINDOWS_SCREENSHOT_SCRIPT: &str = r#"
$OutputPath = $args[0]
Add-Type -AssemblyName System.Windows.Forms
$screen = [System.Windows.Forms.Screen]::PrimaryScreen
$bitmap = New-Object System.Drawing.Bitmap($screen.Bounds.Width, $screen.Bounds.Height)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
try {
    $graphics.CopyFromScreen($screen.Bounds.Location, [System.Drawing.Point]::Empty, $screen.Bounds.Size)
    $bitmap.Save($OutputPath)
} finally {
    $graphics.Dispose()
    $bitmap.Dispose()
}
"#;

/// Screen capture tool.
pub struct ScreenCaptureTool;

impl Default for ScreenCaptureTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenCaptureTool {
    pub fn new() -> Self {
        Self
    }
}

impl std::fmt::Debug for ScreenCaptureTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScreenCaptureTool").finish()
    }
}

/// Determine the output path for a screenshot.
fn screenshot_path(custom: Option<&str>) -> PathBuf {
    if let Some(p) = custom {
        PathBuf::from(p)
    } else {
        let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        thinclaw_platform::state_paths()
            .screenshots_dir
            .join(format!("screen_{ts}_{}.png", uuid::Uuid::new_v4().simple()))
    }
}

/// Capture the screen on macOS using `screencapture`.
#[cfg(target_os = "macos")]
async fn capture_screen(
    path: &std::path::Path,
    interactive: bool,
    window: bool,
    delay_secs: Option<u32>,
) -> Result<(), ToolError> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Create screenshot dir: {e}")))?;
    }

    let mut cmd = Command::new("screencapture");

    if interactive {
        cmd.arg("-i"); // Interactive selection
    } else if window {
        cmd.arg("-w"); // Window selection
    }

    if let Some(delay) = delay_secs {
        cmd.arg("-T").arg(delay.to_string());
    }

    // Silence the shutter sound
    cmd.arg("-x");

    cmd.arg(path.to_string_lossy().as_ref());

    let output = bounded_command_output(
        &mut cmd,
        CAPTURE_HELPER_TIMEOUT,
        CAPTURE_STDOUT_LIMIT,
        CAPTURE_STDERR_LIMIT,
        "macOS screen capture",
    )
    .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ToolError::ExecutionFailed(format!(
            "screencapture failed: {stderr}"
        )));
    }

    // Verify the file was created
    if !path.exists() {
        return Err(ToolError::ExecutionFailed(
            "Screenshot was cancelled or failed".to_string(),
        ));
    }

    Ok(())
}

/// Capture the screen on Linux using available tools.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxScreenCaptureCommand {
    program: &'static str,
    args: Vec<String>,
}

#[cfg(target_os = "linux")]
fn linux_screen_capture_commands(
    path: &std::path::Path,
    interactive: bool,
    window: bool,
    delay_secs: Option<u32>,
) -> Vec<LinuxScreenCaptureCommand> {
    let output = path.to_string_lossy().to_string();
    let mut commands = Vec::new();

    let mut gnome_args = Vec::new();
    if interactive {
        gnome_args.push("-a".to_string());
    } else if window {
        gnome_args.push("-w".to_string());
    }
    if let Some(delay) = delay_secs {
        gnome_args.push("-d".to_string());
        gnome_args.push(delay.to_string());
    }
    gnome_args.push("-f".to_string());
    gnome_args.push(output.clone());
    commands.push(LinuxScreenCaptureCommand {
        program: "gnome-screenshot",
        args: gnome_args,
    });

    let mut scrot_args = Vec::new();
    if let Some(delay) = delay_secs {
        scrot_args.push("-d".to_string());
        scrot_args.push(delay.to_string());
    }
    if interactive || window {
        scrot_args.push("-s".to_string());
    }
    scrot_args.push(output.clone());
    commands.push(LinuxScreenCaptureCommand {
        program: "scrot",
        args: scrot_args,
    });

    if delay_secs.is_none() {
        let args = if interactive || window {
            vec![output]
        } else {
            vec!["-window".to_string(), "root".to_string(), output]
        };
        commands.push(LinuxScreenCaptureCommand {
            program: "import",
            args,
        });
    }

    commands
}

#[cfg(target_os = "linux")]
async fn capture_screen(
    path: &std::path::Path,
    interactive: bool,
    window: bool,
    delay_secs: Option<u32>,
) -> Result<(), ToolError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Create screenshot dir: {e}")))?;
    }

    let mut attempted = Vec::new();
    let deadline = tokio::time::Instant::now() + CAPTURE_HELPER_TIMEOUT;
    for plan in linux_screen_capture_commands(path, interactive, window, delay_secs) {
        if !thinclaw_platform::executable_available(plan.program) {
            continue;
        }

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            attempted.push("capture helper deadline elapsed".to_string());
            break;
        }
        let mut command = Command::new(plan.program);
        command.args(&plan.args);
        match bounded_command_output(
            &mut command,
            remaining,
            CAPTURE_STDOUT_LIMIT,
            CAPTURE_STDERR_LIMIT,
            plan.program,
        )
        .await
        {
            Ok(output) if output.status.success() && path.exists() => {
                return Ok(());
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                attempted.push(if stderr.is_empty() {
                    format!("{} exited with {}", plan.program, output.status)
                } else {
                    format!("{}: {stderr}", plan.program)
                });
            }
            Err(error) => attempted.push(format!("{}: {error}", plan.program)),
        }
    }

    let mode = if interactive {
        "interactive"
    } else if window {
        "window"
    } else {
        "fullscreen"
    };
    let attempted = if attempted.is_empty() {
        "No compatible command was available for the requested mode.".to_string()
    } else {
        attempted.join("; ")
    };
    Err(ToolError::ExecutionFailed(format!(
        "Linux screen capture failed for mode={mode}. Install gnome-screenshot, scrot, or ImageMagick import. Details: {attempted}"
    )))
}

/// Capture the screen on Windows using PowerShell.
#[cfg(target_os = "windows")]
async fn capture_screen(
    path: &std::path::Path,
    _interactive: bool,
    _window: bool,
    _delay_secs: Option<u32>,
) -> Result<(), ToolError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Create screenshot dir: {e}")))?;
    }

    let mut command = Command::new("powershell");
    command
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(WINDOWS_SCREENSHOT_SCRIPT)
        .arg(path);
    let output = bounded_command_output(
        &mut command,
        CAPTURE_HELPER_TIMEOUT,
        CAPTURE_STDOUT_LIMIT,
        CAPTURE_STDERR_LIMIT,
        "Windows screen capture",
    )
    .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ToolError::ExecutionFailed(format!("Screenshot: {stderr}")));
    }

    Ok(())
}

#[async_trait]
impl Tool for ScreenCaptureTool {
    fn name(&self) -> &str {
        "screen_capture"
    }

    fn description(&self) -> &str {
        "Capture the screen and save to a PNG file. Supports full screen, \
         interactive region selection, or window capture where the host screenshot \
         command supports it. Can specify output path and delay."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["fullscreen", "interactive", "window"],
                    "description": "Capture mode. Default: fullscreen",
                    "default": "fullscreen"
                },
                "output_path": {
                    "type": "string",
                    "description": "Custom output file path. Default: ~/.thinclaw/screenshots/screen_<timestamp>.png"
                },
                "delay_seconds": {
                    "type": "integer",
                    "description": "Delay in seconds before capturing (supported by macOS, gnome-screenshot, and scrot)",
                    "minimum": 0,
                    "maximum": 30
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

        let mode = params
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("fullscreen");
        if !matches!(mode, "fullscreen" | "interactive" | "window") {
            return Err(ToolError::InvalidParameters(
                "mode must be fullscreen, interactive, or window".to_string(),
            ));
        }

        let custom_path = params.get("output_path").and_then(|v| v.as_str());
        let delay = params
            .get("delay_seconds")
            .and_then(|v| v.as_u64())
            .map(|d| d.min(30) as u32);

        let requested_path = screenshot_path(custom_path);
        let target = CaptureTarget::prepare(&requested_path, CaptureFormat::Png).await?;

        let interactive = mode == "interactive";
        let window = mode == "window";

        capture_screen(target.staging_path(), interactive, window, delay).await?;
        let (path, size_bytes) = target.publish().await?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "path": path.to_string_lossy(),
                "size_bytes": size_bytes,
                "mode": mode,
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_name() {
        let tool = ScreenCaptureTool::new();
        assert_eq!(tool.name(), "screen_capture");
    }

    #[test]
    fn test_screenshot_path_default() {
        let path = screenshot_path(None);
        assert!(path.to_string_lossy().contains("screen_"));
        assert!(path.to_string_lossy().ends_with(".png"));
    }

    #[test]
    fn test_screenshot_path_custom() {
        let path = screenshot_path(Some("/tmp/my_screenshot.png"));
        assert_eq!(path, PathBuf::from("/tmp/my_screenshot.png"));
    }

    #[test]
    fn test_approval() {
        let tool = ScreenCaptureTool::new();
        assert!(matches!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_command_selection_covers_modes_and_delay() {
        let path = std::path::Path::new("/tmp/thinclaw-screen.png");
        let full = linux_screen_capture_commands(path, false, false, None);
        assert!(full.iter().any(|cmd| cmd.program == "gnome-screenshot"));
        assert!(
            full.iter()
                .any(|cmd| cmd.args.contains(&"-window".to_string()))
        );

        let interactive = linux_screen_capture_commands(path, true, false, Some(2));
        let gnome = interactive
            .iter()
            .find(|cmd| cmd.program == "gnome-screenshot")
            .expect("gnome plan");
        assert!(gnome.args.contains(&"-a".to_string()));
        assert!(gnome.args.contains(&"-d".to_string()));
        assert!(!interactive.iter().any(|cmd| cmd.program == "import"));
    }
}
