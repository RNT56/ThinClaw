pub(crate) fn floor_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    if s.is_char_boundary(pos) {
        return pos;
    }
    let mut i = pos;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Hard timeout for local `osascript`/`sqlite3` subprocesses used by the macOS
/// channels (iMessage, Apple Mail). Without it a hung child — a macOS
/// automation-consent prompt on an unattended machine, a modal dialog, or a
/// stuck database scan — wedges polling or sending forever.
pub(crate) const SUBPROCESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Run a subprocess to completion under [`SUBPROCESS_TIMEOUT`], returning a
/// human-readable error string on spawn failure or timeout.
pub(crate) async fn output_with_timeout(
    cmd: &mut tokio::process::Command,
    ctx: &str,
) -> Result<std::process::Output, String> {
    match tokio::time::timeout(SUBPROCESS_TIMEOUT, cmd.output()).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) => Err(format!("{ctx}: {e}")),
        Err(_) => Err(format!(
            "{ctx}: timed out after {}s",
            SUBPROCESS_TIMEOUT.as_secs()
        )),
    }
}
