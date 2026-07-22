use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use super::{Result, SandboxError};

pub(crate) struct HostCommandOutput {
    pub exit_code: i64,
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
    pub truncated: bool,
}

/// Execute an explicitly full-access host command while retaining ownership of
/// its descendant process tree. Output is drained continuously but only a
/// bounded prefix is retained.
pub(crate) async fn execute_host_command(
    command: &str,
    cwd: &Path,
    env: HashMap<String, String>,
    timeout: Duration,
    max_output_bytes: usize,
) -> Result<HostCommandOutput> {
    let start = std::time::Instant::now();
    let mut cmd = if cfg!(target_os = "windows") {
        let mut command_process = Command::new("cmd");
        command_process.args(["/C", command]);
        command_process
    } else {
        let mut shell = Command::new("sh");
        shell.args(["-c", command]);
        shell
    };
    cmd.current_dir(cwd)
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = thinclaw_platform::process::OwnedChild::spawn(&mut cmd).map_err(|error| {
        SandboxError::ExecutionFailed {
            reason: format!("failed to retain ownership of host command descendants: {error}"),
        }
    })?;
    let mut stdout = child
        .take_stdout()
        .ok_or_else(|| SandboxError::ExecutionFailed {
            reason: "host command stdout pipe was not created".to_string(),
        })?;
    let mut stderr = child
        .take_stderr()
        .ok_or_else(|| SandboxError::ExecutionFailed {
            reason: "host command stderr pipe was not created".to_string(),
        })?;

    let half_max = max_output_bytes / 2;
    let output = tokio::time::timeout(timeout, async {
        let (stdout_result, stderr_result, status_result) = tokio::join!(
            capture_bounded_pipe(&mut stdout, half_max),
            capture_bounded_pipe(&mut stderr, half_max),
            child.wait(),
        );
        let (stdout_bytes, stdout_truncated) =
            stdout_result.map_err(|error| SandboxError::ExecutionFailed {
                reason: format!("failed reading host command stdout: {error}"),
            })?;
        let (stderr_bytes, stderr_truncated) =
            stderr_result.map_err(|error| SandboxError::ExecutionFailed {
                reason: format!("failed reading host command stderr: {error}"),
            })?;
        let status = status_result.map_err(|error| SandboxError::ExecutionFailed {
            reason: error.to_string(),
        })?;
        Ok::<_, SandboxError>((
            stdout_bytes,
            stderr_bytes,
            status,
            stdout_truncated || stderr_truncated,
        ))
    })
    .await;
    let (stdout_bytes, stderr_bytes, status, truncated) = match output {
        Ok(result) => result?,
        Err(_) => {
            let _ = child.kill().await;
            return Err(SandboxError::Timeout(timeout));
        }
    };

    Ok(HostCommandOutput {
        exit_code: status.code().unwrap_or(-1) as i64,
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        duration: start.elapsed(),
        truncated,
    })
}

async fn capture_bounded_pipe<R>(mut pipe: R, max_bytes: usize) -> std::io::Result<(Vec<u8>, bool)>
where
    R: AsyncRead + Unpin,
{
    let mut retained = Vec::with_capacity(max_bytes.min(8 * 1024));
    let mut chunk = [0_u8; 8 * 1024];
    let mut truncated = false;
    loop {
        let read = pipe.read(&mut chunk).await?;
        if read == 0 {
            return Ok((retained, truncated));
        }
        truncated |= append_bounded(&mut retained, &chunk[..read], max_bytes);
    }
}

fn append_bounded(target: &mut Vec<u8>, chunk: &[u8], max_bytes: usize) -> bool {
    let remaining = max_bytes.saturating_sub(target.len());
    let retained = remaining.min(chunk.len());
    target.extend_from_slice(&chunk[..retained]);
    retained < chunk.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn retained_output_is_bounded_while_stream_is_drained() {
        let output = execute_host_command(
            "yes x | head -c 200000",
            Path::new("/tmp"),
            HashMap::new(),
            Duration::from_secs(5),
            1024,
        )
        .await
        .expect("command should complete");
        assert!(output.truncated);
        assert!(output.stdout.len() <= 512);
        assert!(output.stderr.len() <= 512);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_descendant_process_group() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("child.pid");
        let mut env = HashMap::new();
        env.insert(
            "THINCLAW_TEST_CHILD_PID".to_string(),
            marker.display().to_string(),
        );
        let result = execute_host_command(
            "sleep 30 & child=$!; printf '%s' \"$child\" > \"$THINCLAW_TEST_CHILD_PID\"; wait",
            temp.path(),
            env,
            Duration::from_millis(100),
            1024,
        )
        .await;
        assert!(matches!(result, Err(SandboxError::Timeout(_))));

        let pid: libc::pid_t = std::fs::read_to_string(marker).unwrap().parse().unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            // SAFETY: signal 0 performs an existence check and does not alter
            // the target process.
            let exists = unsafe { libc::kill(pid, 0) } == 0;
            if !exists {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "descendant survived timeout"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
