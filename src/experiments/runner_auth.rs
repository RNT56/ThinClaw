//! Private authentication transport for the internal experiment runner.

use std::fs::OpenOptions;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use secrecy::SecretString;
use serde::Deserialize;
use uuid::Uuid;

pub const MAX_RUNNER_AUTH_ENVELOPE_BYTES: usize = 4096;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRunnerAuthEnvelope {
    schema_version: u8,
    lease_id: Uuid,
    token: SecretString,
}

pub struct RunnerAuthEnvelope {
    pub lease_id: Uuid,
    pub token: SecretString,
}

impl std::fmt::Debug for RunnerAuthEnvelope {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunnerAuthEnvelope")
            .field("schema_version", &1)
            .field("lease_id", &self.lease_id)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

pub fn read_runner_auth(
    auth_stdin: bool,
    auth_file: Option<&Path>,
) -> anyhow::Result<RunnerAuthEnvelope> {
    let bytes = match (auth_stdin, auth_file) {
        (true, None) => read_auth_stdin()?,
        (false, Some(path)) => read_private_auth_file(path)?,
        _ => bail!("exactly one of --auth-stdin or --auth-file is required"),
    };
    let raw: RawRunnerAuthEnvelope = serde_json::from_slice(&bytes)
        .context("runner authentication envelope is not valid versioned JSON")?;
    if raw.schema_version != 1 {
        bail!("unsupported runner authentication envelope version");
    }
    if secrecy::ExposeSecret::expose_secret(&raw.token)
        .trim()
        .is_empty()
    {
        bail!("runner authentication envelope contains an empty token");
    }
    Ok(RunnerAuthEnvelope {
        lease_id: raw.lease_id,
        token: raw.token,
    })
}

fn read_auth_stdin() -> anyhow::Result<Vec<u8>> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        bail!("--auth-stdin refuses to read authentication from an interactive terminal");
    }
    read_bounded(stdin.lock()).context("failed to read runner authentication from stdin")
}

fn read_private_auth_file(path: &Path) -> anyhow::Result<Vec<u8>> {
    if !path.is_absolute() {
        bail!("--auth-file must be an absolute path");
    }
    let before = std::fs::symlink_metadata(path).context("failed to inspect --auth-file")?;
    if before.file_type().is_symlink() || !before.is_file() {
        bail!("--auth-file must be a non-symlink regular file");
    }
    if before.len() > MAX_RUNNER_AUTH_ENVELOPE_BYTES as u64 {
        bail!("runner authentication envelope exceeds the size limit");
    }
    validate_private_metadata(&before)?;

    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).context("failed to open --auth-file")?;
    let opened = file
        .metadata()
        .context("failed to inspect opened --auth-file")?;
    validate_private_metadata(&opened)?;
    ensure_same_file(&before, &opened)?;
    let bytes = read_bounded(file).context("failed to read --auth-file")?;

    if !before.permissions().readonly() {
        std::fs::remove_file(path).context("failed to consume one-use --auth-file")?;
    }
    Ok(bytes)
}

fn read_bounded(mut reader: impl Read) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(MAX_RUNNER_AUTH_ENVELOPE_BYTES);
    reader
        .by_ref()
        .take((MAX_RUNNER_AUTH_ENVELOPE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_RUNNER_AUTH_ENVELOPE_BYTES {
        bail!("runner authentication envelope exceeds the size limit");
    }
    Ok(bytes)
}

#[cfg(unix)]
fn validate_private_metadata(metadata: &std::fs::Metadata) -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt;
    if metadata.nlink() != 1 {
        bail!("--auth-file must have exactly one hard link");
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        bail!("--auth-file must be owned by the current user");
    }
    if metadata.mode() & 0o077 != 0 {
        bail!("--auth-file permissions must deny group and other access");
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_metadata(metadata: &std::fs::Metadata) -> anyhow::Result<()> {
    if !metadata.is_file() {
        bail!("--auth-file must be a regular file");
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_same_file(before: &std::fs::Metadata, opened: &std::fs::Metadata) -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt;
    if before.dev() != opened.dev() || before.ino() != opened.ino() {
        bail!("--auth-file was replaced while it was being opened");
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_same_file(
    _before: &std::fs::Metadata,
    _opened: &std::fs::Metadata,
) -> anyhow::Result<()> {
    Ok(())
}

pub fn resolve_workspace_root(path: Option<PathBuf>) -> anyhow::Result<Option<PathBuf>> {
    let Some(path) = path else {
        return Ok(None);
    };
    if !path.is_absolute() {
        bail!("--workspace-root must be an absolute path");
    }
    let metadata =
        std::fs::symlink_metadata(&path).context("failed to inspect --workspace-root")?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("--workspace-root must be a non-symlink directory");
    }
    Ok(Some(
        path.canonicalize()
            .context("failed to canonicalize --workspace-root")?,
    ))
}
