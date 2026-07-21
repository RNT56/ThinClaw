use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

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
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd.spawn().map_err(|error| SandboxError::ExecutionFailed {
        reason: error.to_string(),
    })?;
    let mut ownership =
        DescendantOwnership::attach(&child).map_err(|error| SandboxError::ExecutionFailed {
            reason: format!("failed to retain ownership of host command descendants: {error}"),
        })?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| SandboxError::ExecutionFailed {
            reason: "host command stdout pipe was not created".to_string(),
        })?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| SandboxError::ExecutionFailed {
            reason: "host command stderr pipe was not created".to_string(),
        })?;

    let half_max = max_output_bytes / 2;
    let mut stdout_bytes = Vec::with_capacity(half_max.min(8 * 1024));
    let mut stderr_bytes = Vec::with_capacity(half_max.min(8 * 1024));
    let mut stdout_buffer = [0_u8; 8 * 1024];
    let mut stderr_buffer = [0_u8; 8 * 1024];
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    let mut status = None;
    let mut truncated = false;
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);

    loop {
        if status.is_some() && stdout_eof && stderr_eof {
            break;
        }
        tokio::select! {
            _ = &mut deadline => {
                ownership.terminate();
                let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
                return Err(SandboxError::Timeout(timeout));
            }
            result = child.wait(), if status.is_none() => {
                status = Some(result.map_err(|error| SandboxError::ExecutionFailed {
                    reason: error.to_string(),
                })?);
                // A shell can exit after launching background children. Kill
                // the owned group/job before waiting for inherited pipes to
                // close so those descendants cannot outlive this call.
                ownership.terminate();
            }
            result = stdout.read(&mut stdout_buffer), if !stdout_eof => {
                let read = result.map_err(|error| SandboxError::ExecutionFailed {
                    reason: format!("failed reading host command stdout: {error}"),
                })?;
                if read == 0 {
                    stdout_eof = true;
                } else {
                    truncated |= append_bounded(&mut stdout_bytes, &stdout_buffer[..read], half_max);
                }
            }
            result = stderr.read(&mut stderr_buffer), if !stderr_eof => {
                let read = result.map_err(|error| SandboxError::ExecutionFailed {
                    reason: format!("failed reading host command stderr: {error}"),
                })?;
                if read == 0 {
                    stderr_eof = true;
                } else {
                    truncated |= append_bounded(&mut stderr_bytes, &stderr_buffer[..read], half_max);
                }
            }
        }
    }

    ownership.terminate();
    let status = status.ok_or_else(|| SandboxError::ExecutionFailed {
        reason: "host command exited without a process status".to_string(),
    })?;
    Ok(HostCommandOutput {
        exit_code: status.code().unwrap_or(-1) as i64,
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        duration: start.elapsed(),
        truncated,
    })
}

fn append_bounded(target: &mut Vec<u8>, chunk: &[u8], max_bytes: usize) -> bool {
    let remaining = max_bytes.saturating_sub(target.len());
    let retained = remaining.min(chunk.len());
    target.extend_from_slice(&chunk[..retained]);
    retained < chunk.len()
}

#[cfg(unix)]
struct DescendantOwnership {
    process_group: libc::pid_t,
    terminated: bool,
}

#[cfg(unix)]
impl DescendantOwnership {
    fn attach(child: &Child) -> std::io::Result<Self> {
        let pid = child.id().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "child process has no PID")
        })?;
        let process_group = libc::pid_t::try_from(pid).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "child PID exceeds pid_t")
        })?;
        Ok(Self {
            process_group,
            terminated: false,
        })
    }

    fn terminate(&mut self) {
        if self.terminated || self.process_group <= 1 {
            return;
        }
        self.terminated = true;
        // SAFETY: `process_group` is the positive PID returned for a child
        // spawned with process_group(0); negating it targets only that group.
        unsafe {
            libc::kill(-self.process_group, libc::SIGKILL);
        }
    }
}

#[cfg(unix)]
impl Drop for DescendantOwnership {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(windows)]
struct DescendantOwnership {
    job: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl DescendantOwnership {
    fn attach(child: &Child) -> std::io::Result<Self> {
        use std::os::windows::io::RawHandle;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        // SAFETY: every pointer passed below is either null as documented or
        // points to an initialized value for the duration of the call.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                std::mem::size_of_val(&info) as u32,
            ) == 0
            {
                let error = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(error);
            }
            let process = child.raw_handle().ok_or_else(|| {
                CloseHandle(job);
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "child process has no Windows process handle",
                )
            })? as RawHandle as windows_sys::Win32::Foundation::HANDLE;
            if AssignProcessToJobObject(job, process) == 0 {
                let error = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(error);
            }
            Ok(Self { job })
        }
    }

    fn terminate(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        if self.job.is_null() {
            return;
        }
        // SAFETY: `job` is an owned handle created by CreateJobObjectW.
        unsafe {
            TerminateJobObject(self.job, 1);
            CloseHandle(self.job);
        }
        self.job = std::ptr::null_mut();
    }
}

#[cfg(windows)]
impl Drop for DescendantOwnership {
    fn drop(&mut self) {
        self.terminate();
    }
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
