//! Log query and filter CLI commands.
//!
//! Subcommands:
//! - `logs tail`     — stream recent logs (like `tail -f`)
//! - `logs search`   — search logs by pattern
//! - `logs show`     — show logs for a time range
//! - `logs levels`   — list available log levels and targets

use std::io::{Read as _, Seek as _};
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, Utc};
use clap::Subcommand;

const MAX_LOG_QUERY_ENTRIES: usize = 50_000;
const MAX_LOG_DIRECTORY_ENTRIES: usize = 4096;
const MAX_LOG_TAIL_BYTES_PER_FILE: u64 = 16 * 1024 * 1024;
const MAX_LOG_FILTER_BYTES: usize = 4096;

/// Default log directory.
fn default_log_dir() -> PathBuf {
    crate::platform::state_paths().logs_dir
}

#[derive(Subcommand, Debug, Clone)]
pub enum LogCommand {
    /// Stream recent log entries (like `tail -f`)
    Tail {
        /// Number of lines to show initially (default: 50)
        #[arg(short = 'n', long, default_value = "50")]
        lines: usize,

        /// Filter by log level (trace, debug, info, warn, error)
        #[arg(short, long)]
        level: Option<String>,

        /// Filter by component/target (e.g. "thinclaw::agent", "thinclaw::hooks")
        #[arg(short, long)]
        target: Option<String>,

        /// Custom log directory
        #[arg(long)]
        log_dir: Option<PathBuf>,
    },

    /// Search logs by pattern
    Search {
        /// Search pattern (substring match, case-insensitive)
        pattern: String,

        /// Maximum results to show (default: 100)
        #[arg(short = 'n', long, default_value = "100")]
        max_results: usize,

        /// Filter by log level
        #[arg(short, long)]
        level: Option<String>,

        /// Start time (ISO 8601, e.g. "2026-03-03T10:00:00")
        #[arg(long)]
        since: Option<String>,

        /// End time (ISO 8601)
        #[arg(long)]
        until: Option<String>,

        /// Custom log directory
        #[arg(long)]
        log_dir: Option<PathBuf>,
    },

    /// Show logs for a time range
    Show {
        /// Start time (ISO 8601 or relative: "1h", "30m", "1d")
        #[arg(long, default_value = "1h")]
        since: String,

        /// End time (ISO 8601, default: now)
        #[arg(long)]
        until: Option<String>,

        /// Filter by log level
        #[arg(short, long)]
        level: Option<String>,

        /// Filter by component/target
        #[arg(short, long)]
        target: Option<String>,

        /// Output format: text (default) or json
        #[arg(long, default_value = "text")]
        format: String,

        /// Custom log directory
        #[arg(long)]
        log_dir: Option<PathBuf>,
    },

    /// List available log levels and known targets
    Levels,
}

/// A parsed log entry.
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// Timestamp of the log entry.
    pub timestamp: Option<DateTime<Utc>>,
    /// Log level (INFO, WARN, ERROR, etc.).
    pub level: String,
    /// Component/target (e.g. "thinclaw::agent").
    pub target: String,
    /// The log message.
    pub message: String,
    /// Raw line content.
    pub raw: String,
}

impl LogEntry {
    /// Parse a structured log line.
    ///
    /// Expected format: `2026-03-03T12:00:00Z  INFO thinclaw::agent: message here`
    pub fn parse(line: &str) -> Self {
        let parts: Vec<&str> = line.splitn(4, ' ').collect();

        // Only treat as structured if the first field looks like an ISO timestamp
        // and the second field is a known log level
        if parts.len() >= 4 {
            let maybe_ts = parts[0].trim();
            let maybe_level = parts[1].trim().to_uppercase();
            let is_level = matches!(
                maybe_level.as_str(),
                "TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR"
            );

            if is_level && (maybe_ts.contains('T') || maybe_ts.contains('-')) {
                let timestamp = DateTime::parse_from_rfc3339(maybe_ts)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc));

                let target = parts[2].trim().trim_end_matches(':').to_string();
                let message = parts[3..].join(" ");

                return Self {
                    timestamp,
                    level: maybe_level,
                    target,
                    message,
                    raw: line.to_string(),
                };
            }
        }

        Self {
            timestamp: None,
            level: String::new(),
            target: String::new(),
            message: line.to_string(),
            raw: line.to_string(),
        }
    }

    /// Check if this entry matches a level filter.
    pub fn matches_level(&self, filter: &str) -> bool {
        self.level.eq_ignore_ascii_case(filter)
    }

    /// Check if this entry matches a target filter.
    pub fn matches_target(&self, filter: &str) -> bool {
        self.target.to_lowercase().contains(&filter.to_lowercase())
    }

    /// Check if this entry contains a pattern (case-insensitive).
    pub fn contains_pattern(&self, pattern: &str) -> bool {
        self.raw.to_lowercase().contains(&pattern.to_lowercase())
    }

    /// Check if this entry is within a time range.
    pub fn in_time_range(
        &self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> bool {
        match self.timestamp {
            Some(ts) => {
                if let Some(s) = since
                    && ts < s
                {
                    return false;
                }
                if let Some(u) = until
                    && ts > u
                {
                    return false;
                }
                true
            }
            None => true, // include entries without timestamps
        }
    }
}

/// Parse a relative time string ("1h", "30m", "1d") into a DateTime.
pub fn parse_relative_time(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();

    // Try ISO 8601 first
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }

    // Try NaiveDateTime (no timezone)
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(DateTime::from_naive_utc_and_offset(dt, Utc));
    }

    // Relative format: "1h", "30m", "1d"
    let (num_str, unit) = s.split_at(s.len().saturating_sub(1));
    let num: i64 = num_str.parse().ok()?;
    if num < 0 {
        return None;
    }

    let duration = match unit {
        "s" => chrono::Duration::try_seconds(num)?,
        "m" => chrono::Duration::try_minutes(num)?,
        "h" => chrono::Duration::try_hours(num)?,
        "d" => chrono::Duration::try_days(num)?,
        "w" => chrono::Duration::try_weeks(num)?,
        _ => return None,
    };

    Utc::now().checked_sub_signed(duration)
}

/// Run a log CLI command.
pub async fn run_log_command(cmd: LogCommand) -> anyhow::Result<()> {
    match cmd {
        LogCommand::Tail {
            lines,
            level,
            target,
            log_dir,
        } => {
            let dir = log_dir.unwrap_or_else(default_log_dir);
            let lines = lines.min(MAX_LOG_QUERY_ENTRIES);
            let entries = read_log_entries(&dir, lines.saturating_mul(2))?;

            let filtered: Vec<&LogEntry> = entries
                .iter()
                .filter(|e| level.as_ref().is_none_or(|l| e.matches_level(l)))
                .filter(|e| target.as_ref().is_none_or(|t| e.matches_target(t)))
                .collect();

            let start = filtered.len().saturating_sub(lines);
            for entry in &filtered[start..] {
                println!("{}", entry.raw);
            }

            println!(
                "\n--- Showing {} of {} entries ({}) ---",
                filtered.len().min(lines),
                entries.len(),
                dir.display()
            );
        }

        LogCommand::Search {
            pattern,
            max_results,
            level,
            since,
            until,
            log_dir,
        } => {
            if pattern.len() > MAX_LOG_FILTER_BYTES {
                anyhow::bail!("log search pattern exceeds the size limit");
            }
            let dir = log_dir.unwrap_or_else(default_log_dir);
            let entries = read_log_entries(&dir, 10_000)?;

            let since_dt = since.as_deref().and_then(parse_relative_time);
            let until_dt = until.as_deref().and_then(parse_relative_time);

            let mut count = 0;
            for entry in &entries {
                if count >= max_results {
                    break;
                }
                if !entry.contains_pattern(&pattern) {
                    continue;
                }
                if let Some(ref l) = level
                    && !entry.matches_level(l)
                {
                    continue;
                }
                if !entry.in_time_range(since_dt, until_dt) {
                    continue;
                }
                println!("{}", entry.raw);
                count += 1;
            }

            println!("\n--- {} result(s) for '{}' ---", count, pattern);
        }

        LogCommand::Show {
            since,
            until,
            level,
            target,
            format,
            log_dir,
        } => {
            let dir = log_dir.unwrap_or_else(default_log_dir);
            let entries = read_log_entries(&dir, 50_000)?;

            let since_dt = parse_relative_time(&since);
            let until_dt = until.as_deref().and_then(parse_relative_time);

            let filtered: Vec<&LogEntry> = entries
                .iter()
                .filter(|e| e.in_time_range(since_dt, until_dt))
                .filter(|e| level.as_ref().is_none_or(|l| e.matches_level(l)))
                .filter(|e| target.as_ref().is_none_or(|t| e.matches_target(t)))
                .collect();

            if format == "json" {
                let json_entries: Vec<serde_json::Value> = filtered
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "timestamp": e.timestamp.map(|t| t.to_rfc3339()),
                            "level": e.level,
                            "target": e.target,
                            "message": e.message,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&json_entries)?);
            } else {
                for entry in &filtered {
                    println!("{}", entry.raw);
                }
            }

            println!("\n--- {} entries ---", filtered.len());
        }

        LogCommand::Levels => {
            println!("Available log levels:");
            println!("  TRACE — Very detailed debugging output");
            println!("  DEBUG — Detailed debugging output");
            println!("  INFO  — General informational messages");
            println!("  WARN  — Warning conditions");
            println!("  ERROR — Error conditions");
            println!();
            println!("Known targets:");
            println!("  thinclaw::agent      — Agent loop and dispatch");
            println!("  thinclaw::channels   — Channel I/O");
            println!("  thinclaw::hooks      — Hook execution");
            println!("  thinclaw::llm        — LLM provider calls");
            println!("  thinclaw::tools      — Tool execution");
            println!("  thinclaw::workspace  — Workspace and indexing");
            println!("  thinclaw::safety     — Safety layer");
            println!("  thinclaw::config     — Configuration loading");
            println!();
            println!("Usage:");
            println!("  thinclaw logs tail -l error");
            println!("  thinclaw logs search \"timeout\" --since 1h");
            println!("  thinclaw logs show --since 30m -t thinclaw::llm");
        }
    }

    Ok(())
}

/// Read log entries from a log directory. Returns up to `max` entries.
fn read_log_entries(dir: &std::path::Path, max: usize) -> anyhow::Result<Vec<LogEntry>> {
    let max = max.min(MAX_LOG_QUERY_ENTRIES);
    if max == 0 {
        return Ok(Vec::new());
    }
    let directory_metadata = match std::fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
        anyhow::bail!("log path is not a real directory");
    }

    // Find log files, sorted by name (newest last)
    let mut log_files = Vec::new();
    for (index, entry) in std::fs::read_dir(dir)?.enumerate() {
        if index >= MAX_LOG_DIRECTORY_ENTRIES {
            anyhow::bail!("log directory contains too many entries");
        }
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext == "log" || ext == "txt" || ext == "jsonl")
        {
            log_files.push(path);
        }
    }
    log_files.sort();

    let mut entries = Vec::new();

    // Read files in reverse order (newest first) until we have enough
    for file in log_files.iter().rev() {
        let content = read_log_tail(file, MAX_LOG_TAIL_BYTES_PER_FILE)?;
        let file_entries = content
            .lines()
            .rev()
            .filter(|line| !line.trim().is_empty())
            .map(LogEntry::parse);

        entries.extend(file_entries.take(max.saturating_sub(entries.len())));

        if entries.len() >= max {
            break;
        }
    }

    entries.reverse();
    Ok(entries)
}

fn read_log_tail(path: &Path, max_bytes: u64) -> anyhow::Result<String> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        anyhow::bail!("log entry is not a regular file");
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options.open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() {
        anyhow::bail!("log entry is not a regular file");
    }
    let start = opened.len().saturating_sub(max_bytes);
    file.seek(std::io::SeekFrom::Start(start))?;
    let capacity = usize::try_from(opened.len().saturating_sub(start)).unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity);
    file.take(max_bytes).read_to_end(&mut bytes)?;
    if start > 0
        && let Some(newline) = bytes.iter().position(|byte| *byte == b'\n')
    {
        bytes.drain(..=newline);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_log_entry_structured() {
        let line = "2026-03-03T12:00:00Z INFO thinclaw::agent: Processing message from user-1";
        let entry = LogEntry::parse(line);
        assert!(entry.timestamp.is_some());
        assert_eq!(entry.level, "INFO");
        assert_eq!(entry.target, "thinclaw::agent");
        assert!(entry.message.contains("Processing message"));
    }

    #[test]
    fn test_parse_log_entry_unstructured() {
        let line = "some random log line";
        let entry = LogEntry::parse(line);
        assert!(entry.timestamp.is_none());
        assert_eq!(entry.message, "some random log line");
    }

    #[test]
    fn test_matches_level() {
        let entry = LogEntry {
            timestamp: None,
            level: "WARN".to_string(),
            target: String::new(),
            message: String::new(),
            raw: String::new(),
        };
        assert!(entry.matches_level("warn"));
        assert!(entry.matches_level("WARN"));
        assert!(!entry.matches_level("error"));
    }

    #[test]
    fn test_matches_target() {
        let entry = LogEntry {
            timestamp: None,
            level: String::new(),
            target: "thinclaw::agent::heartbeat".to_string(),
            message: String::new(),
            raw: String::new(),
        };
        assert!(entry.matches_target("agent"));
        assert!(entry.matches_target("heartbeat"));
        assert!(!entry.matches_target("hooks"));
    }

    #[test]
    fn test_contains_pattern() {
        let entry = LogEntry {
            timestamp: None,
            level: String::new(),
            target: String::new(),
            message: String::new(),
            raw: "Error: connection timeout after 30s".to_string(),
        };
        assert!(entry.contains_pattern("timeout"));
        assert!(entry.contains_pattern("Timeout")); // case-insensitive
        assert!(!entry.contains_pattern("success"));
    }

    #[test]
    fn test_parse_relative_time() {
        // Valid relative times
        assert!(parse_relative_time("1h").is_some());
        assert!(parse_relative_time("30m").is_some());
        assert!(parse_relative_time("7d").is_some());

        // Invalid
        assert!(parse_relative_time("abc").is_none());
        assert!(parse_relative_time("-1h").is_none());
        assert!(parse_relative_time("9223372036854775807w").is_none());
    }

    #[test]
    fn test_read_empty_dir() {
        // Non-existent directory should return empty
        let entries = read_log_entries(std::path::Path::new("/tmp/nonexistent_thinclaw_logs"), 100);
        assert!(entries.is_ok());
        assert!(entries.unwrap().is_empty());
    }

    #[test]
    fn log_reader_returns_the_most_recent_lines_in_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("agent.log"), "one\ntwo\nthree\nfour\n").unwrap();

        let entries = read_log_entries(dir.path(), 2).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].raw, "three");
        assert_eq!(entries[1].raw, "four");
    }

    #[cfg(unix)]
    #[test]
    fn log_reader_does_not_follow_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), "secret\n").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("linked.log")).unwrap();

        assert!(read_log_entries(dir.path(), 10).unwrap().is_empty());
    }
}
