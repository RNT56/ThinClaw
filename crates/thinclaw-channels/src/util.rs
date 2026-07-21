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

/// Decode text selected through SQLite's `hex(...)` function. Channel queries
/// use this representation so embedded pipes, CR/LF, and NUL bytes cannot
/// forge additional CLI rows or shift columns.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn decode_sqlite_hex(value: &str) -> Option<String> {
    String::from_utf8(hex::decode(value).ok()?).ok()
}

/// Hard timeout for local `osascript`/`sqlite3` subprocesses used by the macOS
/// channels (iMessage, Apple Mail). Without it a hung child — a macOS
/// automation-consent prompt on an unattended machine, a modal dialog, or a
/// stuck database scan — wedges polling or sending forever.
#[cfg(target_os = "macos")]
pub(crate) const SUBPROCESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Enough for fifty maximum-size Mail summaries plus query overhead, while
/// still preventing a corrupted database or hostile local executable from
/// exhausting the daemon's memory.
#[cfg(target_os = "macos")]
const SUBPROCESS_STDOUT_LIMIT: usize = 8 * 1024 * 1024;

#[cfg(target_os = "macos")]
const SUBPROCESS_STDERR_LIMIT: usize = 256 * 1024;

/// Run a subprocess to completion under [`SUBPROCESS_TIMEOUT`], returning a
/// human-readable error string on spawn failure or timeout.
#[cfg(target_os = "macos")]
pub(crate) async fn output_with_timeout(
    cmd: &mut tokio::process::Command,
    ctx: &str,
) -> Result<std::process::Output, String> {
    let output = thinclaw_platform::bounded_command_output(
        cmd,
        SUBPROCESS_TIMEOUT,
        SUBPROCESS_STDOUT_LIMIT,
        SUBPROCESS_STDERR_LIMIT,
    )
    .await
    .map_err(|error| format!("{ctx}: {error}"))?;
    Ok(std::process::Output {
        status: output.status,
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_hex_round_trips_delimiters_and_newlines() {
        let value = "one|two\nthree\r\0🙂";
        assert_eq!(
            decode_sqlite_hex(&hex::encode(value)),
            Some(value.to_string())
        );
        assert_eq!(decode_sqlite_hex("not-hex"), None);
    }
}
