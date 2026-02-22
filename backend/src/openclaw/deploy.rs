use std::process::Stdio;
use tauri::Emitter;
use tauri::{AppHandle, Manager, State};
use tokio::io::{AsyncBufReadExt, BufReader}; // For simple cross-platform spawning

#[tauri::command]
#[specta::specta]
pub async fn openclaw_deploy_remote(
    app: AppHandle,
    _state: State<'_, super::commands::OpenClawManager>,
    ip: String,
    user: String,
) -> Result<(), String> {
    // Resolve script path
    // Assuming deploy-remote.sh is bundled as a resource or present in src-tauri
    // Development vs Production path handling is tricky with loose scripts.
    // Best practice: sidecar. But sidecar must be binary.
    // Hack for Dev: use relative path from CWD or resource dir.

    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    // In dev: backend/openclaw-engine/deploy-remote.sh
    // In prod: resources/openclaw-engine/deploy-remote.sh (if configured in tauri.conf.json resources)

    let mut script_path = resource_dir
        .join("openclaw-engine")
        .join("deploy-remote.sh");

    // Fallback for dev environment if resource dir logic differs
    if !script_path.exists() {
        if let Ok(cwd) = std::env::current_dir() {
            let dev_path = cwd
                .join("backend")
                .join("openclaw-engine")
                .join("deploy-remote.sh");
            if dev_path.exists() {
                script_path = dev_path;
            }
        }
    }

    if !script_path.exists() {
        return Err(format!("Deployment script not found at: {:?}", script_path));
    }

    // We use Tokio Command to spawn asynchronously and capture output
    let mut child = tokio::process::Command::new("sh")
        .arg(script_path)
        .arg(&ip)
        .arg(&user)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Important: Inherit PATH so it finds 'ansible', 'git', 'ssh', etc.
        // On macOS GUI apps, PATH might be limited.
        // We can attempt to augment it.
        .env(
            "PATH",
            format!(
                "{}:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .spawn()
        .map_err(|e| format!("Failed to start deployment script: {}", e))?;

    let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;
    let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;

    let app_handle = app.clone();

    // Spawn task to read stdout
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = app_handle.emit("deploy-log", format!("[stdout] {}", line));
        }
    });

    let app_handle_err = app.clone();
    // Spawn task to read stderr
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = app_handle_err.emit("deploy-log", format!("[stderr] {}", line));
        }
    });

    // Wait for completion in background to not block the command return
    // But update status on completion
    let app_handle_status = app.clone();
    tokio::spawn(async move {
        match child.wait().await {
            Ok(status) => {
                if status.success() {
                    let _ = app_handle_status.emit("deploy-status", "success");
                } else {
                    let _ = app_handle_status
                        .emit("deploy-status", format!("failed: code {:?}", status.code()));
                }
            }
            Err(e) => {
                let _ = app_handle_status.emit("deploy-status", format!("error: {}", e));
            }
        }
    });

    Ok(())
}
