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
use std::time::Instant;

use async_trait::async_trait;
use tokio::process::Command;

use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput};
use thinclaw_types::JobContext;

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
            .join(format!("screen_{ts}.png"))
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

    let output = cmd
        .output()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("screencapture: {e}")))?;

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
    for plan in linux_screen_capture_commands(path, interactive, window, delay_secs) {
        if !thinclaw_platform::executable_available(plan.program) {
            continue;
        }

        match Command::new(plan.program).args(&plan.args).output().await {
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

    let ps_script = format!(
        r#"
        Add-Type -AssemblyName System.Windows.Forms
        $screen = [System.Windows.Forms.Screen]::PrimaryScreen
        $bitmap = New-Object System.Drawing.Bitmap($screen.Bounds.Width, $screen.Bounds.Height)
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $graphics.CopyFromScreen($screen.Bounds.Location, [System.Drawing.Point]::Empty, $screen.Bounds.Size)
        $bitmap.Save("{}")
        $graphics.Dispose()
        $bitmap.Dispose()
        "#,
        path.to_string_lossy()
    );

    let output = Command::new("powershell")
        .args(["-Command", &ps_script])
        .output()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("PowerShell: {e}")))?;

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

        let custom_path = params.get("output_path").and_then(|v| v.as_str());
        let delay = params
            .get("delay_seconds")
            .and_then(|v| v.as_u64())
            .map(|d| d.min(30) as u32);

        let path = screenshot_path(custom_path);

        let interactive = mode == "interactive";
        let window = mode == "window";

        capture_screen(&path, interactive, window, delay).await?;

        // Get file size
        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Read screenshot metadata: {e}")))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "path": path.to_string_lossy(),
                "size_bytes": metadata.len(),
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
