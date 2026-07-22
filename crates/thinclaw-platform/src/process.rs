//! Owned, bounded execution for trusted argv-constructed host utilities.

use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};

/// A child process plus ownership of every descendant it creates.
///
/// Unix children run in a fresh process group. Windows children are attached
/// to a kill-on-close Job Object. Dropping this value therefore cannot detach
/// a subprocess tree into the host runtime.
pub struct OwnedChild {
    child: Child,
    ownership: DescendantOwnership,
}

/// Synchronous-process counterpart to [`OwnedChild`].
pub struct OwnedStdChild {
    child: std::process::Child,
    ownership: DescendantOwnership,
}

impl std::fmt::Debug for OwnedStdChild {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnedStdChild")
            .field("id", &self.child.id())
            .finish_non_exhaustive()
    }
}

impl OwnedStdChild {
    pub fn spawn(command: &mut std::process::Command) -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
        }
        let mut child = command.spawn()?;
        let ownership = match DescendantOwnership::attach_std(&child) {
            Ok(ownership) => ownership,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        Ok(Self { child, ownership })
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn take_stdin(&mut self) -> Option<std::process::ChildStdin> {
        self.child.stdin.take()
    }

    pub fn take_stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.child.stdout.take()
    }

    pub fn take_stderr(&mut self) -> Option<std::process::ChildStderr> {
        self.child.stderr.take()
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        if status.is_some() {
            self.ownership.terminate();
        }
        Ok(status)
    }

    pub fn wait(&mut self) -> std::io::Result<ExitStatus> {
        let status = self.child.wait()?;
        self.ownership.terminate();
        Ok(status)
    }

    pub fn kill(&mut self) -> std::io::Result<()> {
        self.ownership.terminate();
        match self.child.kill() {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {}
            Err(error) => return Err(error),
        }
        let _ = self.child.wait()?;
        Ok(())
    }
}

impl std::fmt::Debug for OwnedChild {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnedChild")
            .field("id", &self.child.id())
            .finish_non_exhaustive()
    }
}

impl OwnedChild {
    /// Spawn a command in a descendant-owned process boundary.
    pub fn spawn(command: &mut Command) -> std::io::Result<Self> {
        command.kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn()?;
        let ownership = match DescendantOwnership::attach(&child) {
            Ok(ownership) => ownership,
            Err(error) => {
                let _ = child.start_kill();
                return Err(error);
            }
        };
        Ok(Self { child, ownership })
    }

    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        if status.is_some() {
            self.ownership.terminate();
        }
        Ok(status)
    }

    pub async fn wait(&mut self) -> std::io::Result<ExitStatus> {
        let status = self.child.wait().await?;
        self.ownership.terminate();
        Ok(status)
    }

    pub async fn kill(&mut self) -> std::io::Result<()> {
        self.ownership.terminate();
        match self.child.start_kill() {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {}
            Err(error) => return Err(error),
        }
        let _ = self.child.wait().await?;
        Ok(())
    }
}

/// Captured output from a bounded subprocess.
#[derive(Debug)]
pub struct BoundedProcessOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// One newline-delimited record retained within a fixed memory bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedLine {
    pub bytes: Vec<u8>,
    pub truncated: bool,
}

impl BoundedLine {
    /// Decode the retained bytes without allowing malformed subprocess output
    /// to escape the bounded transport as an error.
    pub fn into_lossy_text(self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

/// Read and drain one newline-delimited record while retaining at most
/// `max_bytes`. Draining the full record prevents a producer from blocking on
/// a full pipe, while the retained allocation remains bounded.
pub async fn read_bounded_line<R>(
    reader: &mut R,
    max_bytes: usize,
) -> std::io::Result<Option<BoundedLine>>
where
    R: AsyncBufRead + Unpin,
{
    let mut retained = Vec::with_capacity(max_bytes.min(8 * 1024));
    let mut saw_input = false;
    let mut truncated = false;

    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if !saw_input {
                return Ok(None);
            }
            break;
        }
        saw_input = true;

        let newline = available.iter().position(|byte| *byte == b'\n');
        let data_len = newline.unwrap_or(available.len());
        let remaining = max_bytes.saturating_sub(retained.len());
        let keep = data_len.min(remaining);
        retained.extend_from_slice(&available[..keep]);
        truncated |= keep < data_len;
        let consumed = newline.map_or(available.len(), |index| index + 1);
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }

    if retained.last() == Some(&b'\r') {
        retained.pop();
    }
    Ok(Some(BoundedLine {
        bytes: retained,
        truncated,
    }))
}

/// Failure modes for bounded subprocess execution.
#[derive(Debug, thiserror::Error)]
pub enum BoundedProcessError {
    #[error("failed to spawn subprocess: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("subprocess pipe was not created: {0}")]
    MissingPipe(&'static str),
    #[error("subprocess I/O failed: {0}")]
    Io(#[source] std::io::Error),
    #[error("subprocess exceeded its {0:?} deadline")]
    Timeout(Duration),
    #[error("subprocess output exceeded its capture limit")]
    OutputLimit { stdout: bool, stderr: bool },
}

async fn capture_bounded_pipe<R>(mut pipe: R, limit: usize) -> std::io::Result<(Vec<u8>, bool)>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut captured = Vec::with_capacity(limit.min(64 * 1024));
    let mut exceeded = false;
    let mut chunk = [0_u8; 8192];
    loop {
        let read = pipe.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(captured.len());
        let retained = remaining.min(read);
        captured.extend_from_slice(&chunk[..retained]);
        exceeded |= retained < read;
        // Keep draining after the retention limit so a full pipe cannot block
        // the child while host memory remains bounded.
    }
    Ok((captured, exceeded))
}

/// Run a trusted argv-constructed host utility with descendant ownership, a
/// hard deadline, and strict stdout/stderr retention limits.
pub async fn bounded_command_output(
    command: &mut Command,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<BoundedProcessOutput, BoundedProcessError> {
    bounded_command_output_inner(command, None, timeout, stdout_limit, stderr_limit).await
}

/// Run a trusted argv-constructed host utility with bounded stdin, descendant
/// ownership, a hard deadline, and strict stdout/stderr retention limits.
///
/// Callers remain responsible for choosing and enforcing an appropriate input
/// size limit before invoking this function.
pub async fn bounded_command_output_with_input(
    command: &mut Command,
    input: &[u8],
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<BoundedProcessOutput, BoundedProcessError> {
    bounded_command_output_inner(command, Some(input), timeout, stdout_limit, stderr_limit).await
}

/// Synchronous counterpart to [`bounded_command_output`] for configuration and
/// service-management paths that cannot be made async. Output is drained on
/// fixed-memory reader threads while the caller polls a hard deadline.
pub fn bounded_std_command_output(
    command: &mut std::process::Command,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<BoundedProcessOutput, BoundedProcessError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(BoundedProcessError::Spawn)?;
    let mut ownership = match DescendantOwnership::attach_std(&child) {
        Ok(ownership) => ownership,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(BoundedProcessError::Spawn(error));
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            ownership.terminate();
            let _ = child.kill();
            let _ = child.wait();
            return Err(BoundedProcessError::MissingPipe("stdout"));
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            ownership.terminate();
            let _ = child.kill();
            let _ = child.wait();
            return Err(BoundedProcessError::MissingPipe("stderr"));
        }
    };
    let stdout_reader = std::thread::spawn(move || capture_bounded_std_pipe(stdout, stdout_limit));
    let stderr_reader = std::thread::spawn(move || capture_bounded_std_pipe(stderr, stderr_limit));

    let now = Instant::now();
    let deadline = now.checked_add(timeout).unwrap_or(now);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                ownership.terminate();
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(BoundedProcessError::Timeout(timeout));
            }
            Err(error) => {
                ownership.terminate();
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(BoundedProcessError::Io(error));
            }
        }
    };
    // A utility may leave descendants holding inherited pipes after its direct
    // process exits. Terminate the owned tree before joining the drainers.
    ownership.terminate();
    let (stdout, stdout_exceeded) = stdout_reader
        .join()
        .map_err(|_| BoundedProcessError::Io(std::io::Error::other("stdout reader panicked")))?
        .map_err(BoundedProcessError::Io)?;
    let (stderr, stderr_exceeded) = stderr_reader
        .join()
        .map_err(|_| BoundedProcessError::Io(std::io::Error::other("stderr reader panicked")))?
        .map_err(BoundedProcessError::Io)?;
    if stdout_exceeded || stderr_exceeded {
        return Err(BoundedProcessError::OutputLimit {
            stdout: stdout_exceeded,
            stderr: stderr_exceeded,
        });
    }
    Ok(BoundedProcessOutput {
        status,
        stdout,
        stderr,
    })
}

fn capture_bounded_std_pipe<R: std::io::Read>(
    mut pipe: R,
    limit: usize,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut captured = Vec::with_capacity(limit.min(64 * 1024));
    let mut exceeded = false;
    let mut chunk = [0_u8; 8192];
    loop {
        let read = std::io::Read::read(&mut pipe, &mut chunk)?;
        if read == 0 {
            break;
        }
        let retained = read.min(limit.saturating_sub(captured.len()));
        captured.extend_from_slice(&chunk[..retained]);
        exceeded |= retained < read;
    }
    Ok((captured, exceeded))
}

async fn bounded_command_output_inner(
    command: &mut Command,
    input: Option<&[u8]>,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<BoundedProcessOutput, BoundedProcessError> {
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = OwnedChild::spawn(command).map_err(BoundedProcessError::Spawn)?;
    let stdin = match input {
        Some(_) => match child.take_stdin() {
            Some(stdin) => Some(stdin),
            None => {
                let _ = child.kill().await;
                return Err(BoundedProcessError::MissingPipe("stdin"));
            }
        },
        None => None,
    };
    let stdout = match child.take_stdout() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill().await;
            return Err(BoundedProcessError::MissingPipe("stdout"));
        }
    };
    let stderr = match child.take_stderr() {
        Some(stderr) => stderr,
        None => {
            let _ = child.kill().await;
            return Err(BoundedProcessError::MissingPipe("stderr"));
        }
    };

    let result = tokio::time::timeout(timeout, async {
        let write_input = async move {
            if let (Some(mut stdin), Some(input)) = (stdin, input) {
                stdin.write_all(input).await?;
                stdin.shutdown().await?;
            }
            Ok::<(), std::io::Error>(())
        };
        let (stdin_result, stdout, stderr, status) = tokio::join!(
            write_input,
            capture_bounded_pipe(stdout, stdout_limit),
            capture_bounded_pipe(stderr, stderr_limit),
            child.wait(),
        );
        stdin_result?;
        Ok::<_, std::io::Error>((stdout?, stderr?, status?))
    })
    .await;

    let ((stdout, stdout_exceeded), (stderr, stderr_exceeded), status) = match result {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            let _ = child.kill().await;
            return Err(BoundedProcessError::Io(error));
        }
        Err(_) => {
            let _ = child.kill().await;
            return Err(BoundedProcessError::Timeout(timeout));
        }
    };

    if stdout_exceeded || stderr_exceeded {
        return Err(BoundedProcessError::OutputLimit {
            stdout: stdout_exceeded,
            stderr: stderr_exceeded,
        });
    }

    Ok(BoundedProcessOutput {
        status,
        stdout,
        stderr,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn output_limit_is_enforced_while_pipe_is_drained() {
        let mut command = Command::new("sh");
        command.args(["-c", "yes x | head -c 200000"]);
        let result = bounded_command_output(&mut command, Duration::from_secs(5), 1024, 1024).await;
        assert!(matches!(
            result,
            Err(BoundedProcessError::OutputLimit {
                stdout: true,
                stderr: false
            })
        ));
    }

    #[tokio::test]
    async fn timeout_kills_descendant_process_group() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("descendant.pid");
        let script = format!(
            "sleep 30 & child=$!; printf '%s' \"$child\" > '{}'; wait",
            marker.display()
        );
        let mut command = Command::new("sh");
        command.args(["-c", &script]);
        let result =
            bounded_command_output(&mut command, Duration::from_millis(150), 1024, 1024).await;
        assert!(matches!(result, Err(BoundedProcessError::Timeout(_))));

        let pid: libc::pid_t = std::fs::read_to_string(marker).unwrap().parse().unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            // SAFETY: signal zero checks existence without altering the target.
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

    #[tokio::test]
    async fn input_is_delivered_while_output_is_bounded() {
        let mut command = Command::new("sh");
        command.args(["-c", "read value; printf 'seen:%s' \"$value\""]);
        let output = bounded_command_output_with_input(
            &mut command,
            b"hello\n",
            Duration::from_secs(5),
            1024,
            1024,
        )
        .await
        .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"seen:hello");
    }

    #[test]
    fn synchronous_runner_enforces_deadline() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 30"]);
        let result =
            bounded_std_command_output(&mut command, Duration::from_millis(100), 1024, 1024);
        assert!(matches!(result, Err(BoundedProcessError::Timeout(_))));
    }
}

#[cfg(test)]
mod bounded_line_tests {
    use super::*;

    #[tokio::test]
    async fn bounded_line_reader_drains_an_oversized_record() {
        let (mut writer, reader) = tokio::io::duplex(64);
        let writer_task = tokio::spawn(async move {
            writer.write_all(b"abcdef\r\nok\n").await.unwrap();
        });
        let mut reader = tokio::io::BufReader::new(reader);

        let first = read_bounded_line(&mut reader, 4)
            .await
            .unwrap()
            .expect("first line");
        assert_eq!(first.bytes, b"abcd");
        assert!(first.truncated);

        let second = read_bounded_line(&mut reader, 4)
            .await
            .unwrap()
            .expect("second line");
        assert_eq!(second.bytes, b"ok");
        assert!(!second.truncated);
        assert!(read_bounded_line(&mut reader, 4).await.unwrap().is_none());
        writer_task.await.unwrap();
    }
}

#[cfg(unix)]
struct DescendantOwnership {
    process_group: libc::pid_t,
    terminated: bool,
}

#[cfg(unix)]
impl DescendantOwnership {
    fn attach(child: &Child) -> std::io::Result<Self> {
        Self::attach_pid(child.id())
    }

    fn attach_std(child: &std::process::Child) -> std::io::Result<Self> {
        Self::attach_pid(Some(child.id()))
    }

    fn attach_pid(pid: Option<u32>) -> std::io::Result<Self> {
        let pid = pid.ok_or_else(|| {
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
        // SAFETY: this positive PID belongs to a child spawned with
        // process_group(0); its negation targets only that process group.
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

#[cfg(all(test, windows))]
mod windows_tests {
    use super::{DescendantOwnership, OwnedChild, OwnedStdChild};

    #[test]
    fn process_ownership_types_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<DescendantOwnership>();
        assert_send_sync::<OwnedChild>();
        assert_send_sync::<OwnedStdChild>();
    }
}

#[cfg(windows)]
struct DescendantOwnership {
    job: Option<std::os::windows::io::OwnedHandle>,
}

#[cfg(windows)]
impl DescendantOwnership {
    fn attach(child: &Child) -> std::io::Result<Self> {
        use std::os::windows::io::RawHandle;

        let process = child.raw_handle().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "child process has no Windows process handle",
            )
        })? as RawHandle as windows_sys::Win32::Foundation::HANDLE;
        Self::attach_handle(process)
    }

    fn attach_std(child: &std::process::Child) -> std::io::Result<Self> {
        use std::os::windows::io::AsRawHandle as _;
        Self::attach_handle(child.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE)
    }

    fn attach_handle(process: windows_sys::Win32::Foundation::HANDLE) -> std::io::Result<Self> {
        use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        // SAFETY: pointers are null where the API permits it or reference an
        // initialized structure for the duration of each call.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            // SAFETY: CreateJobObjectW returned a fresh owned handle. Wrapping
            // it immediately gives the handle standard Send-safe RAII
            // semantics on every return path.
            let job = OwnedHandle::from_raw_handle(job);
            let raw_job = job.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                raw_job,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                std::mem::size_of_val(&info) as u32,
            ) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            if AssignProcessToJobObject(raw_job, process) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(Self { job: Some(job) })
        }
    }

    fn terminate(&mut self) {
        use std::os::windows::io::AsRawHandle as _;
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        let Some(job) = self.job.take() else {
            return;
        };
        // SAFETY: `job` is an owned handle created by CreateJobObjectW.
        unsafe {
            TerminateJobObject(
                job.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE,
                1,
            );
        }
    }
}

#[cfg(windows)]
impl Drop for DescendantOwnership {
    fn drop(&mut self) {
        self.terminate();
    }
}
