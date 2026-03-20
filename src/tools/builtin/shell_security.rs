//! Shell command security validation.
//!
//! Extracted from `shell.rs` to separate security concerns from execution logic.
//! All validation functions, blocked command patterns, and safe-bins lists live here.

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

/// Commands that are always blocked for safety.
pub(super) static BLOCKED_COMMANDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        "rm -rf /",
        "rm -rf /*",
        ":(){ :|:& };:", // Fork bomb
        "dd if=/dev/zero",
        "mkfs",
        "chmod -R 777 /",
        "> /dev/sda",
        "curl | sh",
        "wget | sh",
        "curl | bash",
        "wget | bash",
    ])
});

/// Patterns that indicate potentially dangerous commands.
pub(super) static DANGEROUS_PATTERNS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "sudo ",
        "doas ",
        " | sh",
        " | bash",
        " | zsh",
        "eval ",
        "$(curl",
        "$(wget",
        "/etc/passwd",
        "/etc/shadow",
        "~/.ssh",
        ".bash_history",
        "id_rsa",
    ]
});

/// Patterns that should NEVER be auto-approved, even if the user chose "always approve"
/// for the shell tool. These require explicit per-invocation approval because they are
/// destructive or security-sensitive.
pub(super) static NEVER_AUTO_APPROVE_PATTERNS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "rm -rf",
        "rm -fr",
        "chmod -r 777",
        "chmod 777",
        "chown -r",
        "shutdown",
        "reboot",
        "poweroff",
        "init 0",
        "init 6",
        "iptables",
        "nft ",
        "useradd",
        "userdel",
        "passwd",
        "visudo",
        "crontab",
        "systemctl disable",
        "launchctl unload",
        "kill -9",
        "killall",
        "pkill",
        "docker rm",
        "docker rmi",
        "docker system prune",
        "git push --force",
        "git push -f",
        "git reset --hard",
        "git clean -f",
        "DROP TABLE",
        "DROP DATABASE",
        "TRUNCATE",
        "DELETE FROM",
    ]
});

/// Environment variables safe to forward to child processes.
///
/// When executing commands directly (no sandbox), we scrub the environment to
/// prevent API keys and secrets from leaking through `env`, `printenv`, or child
/// process inheritance (CWE-200). Only these well-known OS/toolchain variables
/// are forwarded.
pub(super) const SAFE_ENV_VARS: &[&str] = &[
    // Core OS
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "COLORTERM",
    // Locale
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    // Working directory (many tools depend on this)
    "PWD",
    // Temp directories
    "TMPDIR",
    "TMP",
    "TEMP",
    // XDG (Linux desktop/config paths)
    "XDG_RUNTIME_DIR",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    // Rust toolchain
    "CARGO_HOME",
    "RUSTUP_HOME",
    // Node.js
    "NODE_PATH",
    "NPM_CONFIG_PREFIX",
    // Editor (for git commit, etc.)
    "EDITOR",
    "VISUAL",
    // Windows (no-ops on Unix, but needed if we ever run on Windows)
    "SystemRoot",
    "SYSTEMROOT",
    "ComSpec",
    "PATHEXT",
    "APPDATA",
    "LOCALAPPDATA",
    "USERPROFILE",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "WINDIR",
];

/// Environment variables that indicate library injection attacks.
///
/// Commands that set these variables are blocked because they can be used to
/// hijack any process by preloading attacker-controlled shared libraries.
/// This covers both Linux (LD_*) and macOS (DYLD_*) variants.
pub(super) const DANGEROUS_ENV_VARS: &[&str] = &[
    // Linux
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    // macOS
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "DYLD_FRAMEWORK_PATH",
    "DYLD_FALLBACK_LIBRARY_PATH",
];

/// Safe binaries allowed when `IRONCLAW_SAFE_BINS_ONLY=true`.
///
/// When this mode is active, only commands whose first token (the binary name)
/// matches one of these entries are allowed. Additional binaries can be added
/// via the `IRONCLAW_EXTRA_BINS` env var (comma-separated).
pub(super) const SAFE_BINS: &[&str] = &[
    // File inspection
    "ls",
    "cat",
    "head",
    "tail",
    "less",
    "more",
    "wc",
    "file",
    "stat",
    "find",
    "tree",
    "du",
    "df",
    // Text processing
    "grep",
    "rg",
    "ag",
    "awk",
    "sed",
    "sort",
    "uniq",
    "cut",
    "tr",
    "diff",
    "patch",
    "jq",
    "yq",
    // Build tools
    "cargo",
    "rustc",
    "rustfmt",
    "clippy-driver",
    "npm",
    "npx",
    "node",
    "yarn",
    "pnpm",
    "bun",
    "deno",
    "python3",
    "python",
    "pip3",
    "pip",
    "uv",
    "go",
    "make",
    "cmake",
    "gcc",
    "g++",
    "clang",
    "clang++",
    // Version control
    "git",
    // Shell utilities
    "echo",
    "printf",
    "date",
    "which",
    "whereis",
    "whoami",
    "env",
    "printenv",
    "true",
    "false",
    "test",
    "mkdir",
    "cp",
    "mv",
    "touch",
    "ln",
    // Networking (read-only)
    "curl",
    "wget",
    "ping",
    "dig",
    "nslookup",
    // Archival
    "tar",
    "zip",
    "unzip",
    "gzip",
    "gunzip",
    // Container
    "docker",
    "podman",
    // Documentation
    "man",
    // Desktop (open files in default app)
    "open",     // macOS
    "xdg-open", // Linux
    // Clipboard
    "pbcopy",  // macOS
    "pbpaste", // macOS
    "xclip",   // Linux
    // Common utilities
    "tee",
    "xargs",
    "chmod",
    "realpath",
    "basename",
    "dirname",
];

/// Check whether safe-bins-only mode is enabled.
pub(super) fn is_safe_bins_only() -> bool {
    // IC-007: Use optional_env to see bridge-injected vars (not just real env)
    crate::config::helpers::optional_env("IRONCLAW_SAFE_BINS_ONLY")
        .ok()
        .flatten()
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false)
}

/// Extract the binary name from a command string (first token, basename only).
pub(super) fn extract_binary_name(cmd: &str) -> Option<String> {
    let trimmed = cmd.trim();
    // Skip leading env assignments (e.g., "FOO=bar command")
    let mut tokens = trimmed.split_whitespace();
    for token in tokens.by_ref() {
        if !token.contains('=') {
            // This is the actual command — extract basename
            let basename = token.rsplit('/').next().unwrap_or(token);
            return Some(basename.to_string());
        }
    }
    None
}

/// Detect library injection via environment variable manipulation.
///
/// Returns a reason string if the command sets any dangerous env vars
/// (LD_PRELOAD, DYLD_INSERT_LIBRARIES, etc.).
pub fn detect_library_injection(cmd: &str) -> Option<&'static str> {
    let upper = cmd.to_uppercase();
    for var in DANGEROUS_ENV_VARS {
        // Match patterns: VAR=value, export VAR=, env VAR=, set VAR=
        let var_assign = format!("{var}=");
        let var_export = format!("export {var}");
        if upper.contains(var_assign.as_str()) || upper.contains(var_export.as_str()) {
            return Some("library injection via environment variable");
        }
    }
    None
}

/// Check whether a command's binary is in the safe bins allowlist.
///
/// Returns None if allowed, Some(reason) if blocked.
pub(super) fn check_safe_bins(cmd: &str) -> Option<String> {
    if !is_safe_bins_only() {
        return None;
    }

    let binary = match extract_binary_name(cmd) {
        Some(b) => b,
        None => return Some("could not determine binary name".to_string()),
    };

    // Check built-in list
    if SAFE_BINS.contains(&binary.as_str()) {
        return None;
    }

    // Check user-extended list
    if let Ok(extra) = std::env::var("IRONCLAW_EXTRA_BINS") {
        let extras: Vec<&str> = extra.split(',').map(|s| s.trim()).collect();
        if extras.contains(&binary.as_str()) {
            return None;
        }
    }

    Some(format!(
        "binary '{}' not in safe bins allowlist (set IRONCLAW_EXTRA_BINS to extend)",
        binary
    ))
}

/// Same as `check_safe_bins` but always enforced (for sandbox base_dir mode).
///
/// When `ShellTool::base_dir` is set, the safe bins allowlist is mandatory —
/// the `IRONCLAW_SAFE_BINS_ONLY` env var check is skipped.
/// Returns `true` if the command should be BLOCKED.
pub(super) fn check_safe_bins_forced(cmd: &str) -> bool {
    let binary = match extract_binary_name(cmd) {
        Some(b) => b,
        None => return true, // Can't determine binary → block
    };

    // Check built-in list
    if SAFE_BINS.contains(&binary.as_str()) {
        return false;
    }

    // Check user-extended list
    if let Ok(extra) = std::env::var("IRONCLAW_EXTRA_BINS") {
        let extras: Vec<&str> = extra.split(',').map(|s| s.trim()).collect();
        if extras.contains(&binary.as_str()) {
            return false;
        }
    }

    true // Not in allowlist → block
}

/// Scan a command string for absolute paths outside the sandbox base directory.
///
/// This is a best-effort heuristic — it catches obvious cases like:
/// - `cat /etc/passwd`
/// - `ls /usr/local/bin`
/// - `cp file.txt /tmp/leaked`
///
/// It does NOT catch:
/// - Subshell expansions (`cat $(echo /etc/passwd)`)
/// - Paths constructed in code (`python -c "open('/etc/passwd')"`)
/// - Variable expansions (`cat $HOME/.ssh/id_rsa`)
///
/// Those are handled by the safe-bins allowlist + user approval system.
pub(super) fn detect_path_escape(cmd: &str, base_dir: &Path) -> Option<String> {
    let base_str = base_dir.to_string_lossy();

    // Tokenize the command by whitespace and common shell delimiters
    // We look for tokens that start with `/` (absolute paths) or contain `..`
    for token in cmd.split(|c: char| {
        c.is_whitespace() || c == ';' || c == '|' || c == '&' || c == '>' || c == '<'
    }) {
        let token = token.trim_matches(|c: char| c == '\'' || c == '"' || c == '(' || c == ')');
        if token.is_empty() {
            continue;
        }

        // Check for `..` traversal (e.g., `cat ../../etc/passwd`, `./../../secrets`)
        // This catches relative path escapes that don't start with `/`
        if token.contains("..") {
            // Allow `..` in non-path contexts (e.g., range syntax `1..10`)
            // A `..` is suspicious when preceded/followed by `/` or at token boundaries
            if token.contains("../")
                || token.contains("/..")
                || (token.contains("..") && token.starts_with('.'))
            {
                return Some(format!("path traversal: {}", token));
            }
        }

        // Only check absolute path tokens from here on
        if !token.starts_with('/') {
            continue;
        }

        // Allow the base_dir itself and anything under it
        if token.starts_with(base_str.as_ref()) {
            continue;
        }

        // Allow `/dev/null` and `/dev/stdin` etc. (common harmless redirects)
        if token.starts_with("/dev/") {
            continue;
        }

        // Allow `/tmp` (frequently needed for temp files in builds)
        if token.starts_with("/tmp") {
            continue;
        }

        // Allow common tool paths that are invoked, not accessed
        // (e.g., `/usr/bin/env python3` or `/bin/sh -c ...`)
        if token.starts_with("/usr/bin/")
            || token.starts_with("/bin/")
            || token.starts_with("/usr/local/bin/")
        {
            continue;
        }

        // This is an absolute path outside the workspace
        return Some(token.to_string());
    }

    None
}

/// Check whether a shell command contains patterns that must never be auto-approved.
///
/// Even when the user has chosen "always approve" for the shell tool, these commands
/// require explicit per-invocation approval because they are destructive.
pub fn requires_explicit_approval(command: &str) -> bool {
    let lower = command.to_lowercase();
    NEVER_AUTO_APPROVE_PATTERNS
        .iter()
        .any(|p| lower.contains(&p.to_lowercase()))
}

/// Detect command injection and obfuscation attempts.
///
/// Catches patterns that indicate a prompt-injected LLM trying to exfiltrate
/// data or hide malicious intent through encoding. Returns a human-readable
/// reason if a pattern is detected.
///
/// These checks complement the existing BLOCKED_COMMANDS and DANGEROUS_PATTERNS
/// lists by catching obfuscation that simple substring matching would miss.
pub fn detect_command_injection(cmd: &str) -> Option<&'static str> {
    // Null bytes can bypass string matching in downstream tools
    if cmd.bytes().any(|b| b == 0) {
        return Some("null byte in command");
    }

    let lower = cmd.to_lowercase();

    // Base64 decode piped to shell execution (obfuscation of arbitrary commands)
    if (lower.contains("base64 -d") || lower.contains("base64 --decode"))
        && contains_shell_pipe(&lower)
    {
        return Some("base64 decode piped to shell");
    }

    // printf/echo with hex or octal escapes piped to shell
    if (lower.contains("printf") || lower.contains("echo -e") || lower.contains("echo $'"))
        && (lower.contains("\\x") || lower.contains("\\0"))
        && contains_shell_pipe(&lower)
    {
        return Some("encoded escape sequences piped to shell");
    }

    // xxd/od reverse (hex dump to binary) piped to shell.
    // Use has_command_token for "od" to avoid matching words like "method", "period".
    if (lower.contains("xxd -r") || has_command_token(&lower, "od ")) && contains_shell_pipe(&lower)
    {
        return Some("binary decode piped to shell");
    }

    // DNS exfiltration: dig/nslookup/host with command substitution.
    // Use has_command_token to avoid false positives on words containing
    // "host" (e.g., "ghost", "--host") or "dig" as substrings.
    if (has_command_token(&lower, "dig ")
        || has_command_token(&lower, "nslookup ")
        || has_command_token(&lower, "host "))
        && has_command_substitution(&lower)
    {
        return Some("potential DNS exfiltration via command substitution");
    }

    // Netcat with data piping (exfiltration channel).
    // Use has_command_token to avoid false positives on words containing
    // "nc" as a substring (e.g., "sync", "once", "fence").
    if (has_command_token(&lower, "nc ")
        || has_command_token(&lower, "ncat ")
        || has_command_token(&lower, "netcat "))
        && (lower.contains('|') || lower.contains('<'))
    {
        return Some("netcat with data piping");
    }

    // curl/wget posting file contents to a remote server.
    // Include both "-d @file" (with space) and "-d@file" (without space)
    // since curl accepts both forms.
    if lower.contains("curl")
        && (lower.contains("-d @")
            || lower.contains("-d@")
            || lower.contains("--data @")
            || lower.contains("--data-binary @")
            || lower.contains("--upload-file"))
    {
        return Some("curl posting file contents");
    }

    if lower.contains("wget") && lower.contains("--post-file") {
        return Some("wget posting file contents");
    }

    // Chained obfuscation: rev, tr, sed used to reconstruct hidden commands piped to shell
    if (lower.contains("| rev") || lower.contains("|rev")) && contains_shell_pipe(&lower) {
        return Some("string reversal piped to shell");
    }

    None
}

/// Check if a command string contains a pipe to a shell interpreter.
///
/// Uses word boundary checking so "| shell" or "| shift" don't false-positive
/// against "| sh".
pub(super) fn contains_shell_pipe(lower: &str) -> bool {
    has_pipe_to(lower, "sh")
        || has_pipe_to(lower, "bash")
        || has_pipe_to(lower, "zsh")
        || has_pipe_to(lower, "dash")
        || has_pipe_to(lower, "/bin/sh")
        || has_pipe_to(lower, "/bin/bash")
}

/// Check if the command pipes to a specific interpreter, with word boundary
/// validation so "| shift" doesn't match "| sh".
fn has_pipe_to(lower: &str, shell: &str) -> bool {
    for prefix in ["| ", "|"] {
        let pattern = format!("{prefix}{shell}");
        for (i, _) in lower.match_indices(&pattern) {
            let end = i + pattern.len();
            if end >= lower.len()
                || matches!(
                    lower.as_bytes()[end],
                    b' ' | b'\t' | b'\n' | b';' | b'|' | b'&' | b')'
                )
            {
                return true;
            }
        }
    }
    false
}

/// Check if a command string contains shell command substitution (`$(...)` or backticks).
fn has_command_substitution(s: &str) -> bool {
    s.contains("$(") || s.contains('`')
}

/// Check if `token` appears as a standalone command in `lower` (not as a substring
/// of another word).
///
/// A token is "standalone" if it appears at the start of the string or is preceded
/// by whitespace or a shell separator (`|`, `;`, `&`, `(`).
///
/// This prevents false positives like "sync " matching "nc " or "ghost " matching
/// "host ".
pub(super) fn has_command_token(lower: &str, token: &str) -> bool {
    for (i, _) in lower.match_indices(token) {
        if i == 0 {
            return true;
        }
        let before = lower.as_bytes()[i - 1];
        if matches!(before, b' ' | b'\t' | b'|' | b';' | b'&' | b'\n' | b'(') {
            return true;
        }
    }
    false
}
