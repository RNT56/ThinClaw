//! Prompt-surface PII redaction helpers.

use std::ops::Range;

use regex::Regex;
use sha2::{Digest, Sha256};

/// Hash a user identifier into a stable prompt-safe token.
pub fn hash_user_id(raw_id: &str) -> String {
    let digest = Sha256::digest(raw_id.as_bytes());
    let hex = hex::encode(digest);
    format!("user_{}", &hex[..12])
}

/// Whether prompt surfaces for a channel should redact user identifiers.
pub fn should_redact(platform: &str) -> bool {
    !platform.trim().eq_ignore_ascii_case("discord")
}

/// Render a user identifier for prompt injection.
pub fn redact_for_prompt(actor_id: &str, platform: &str) -> String {
    if should_redact(platform) {
        hash_user_id(actor_id)
    } else {
        actor_id.to_string()
    }
}

/// Redact prompt-bound sensitive text while preserving deterministic labels.
///
/// Contact details, local paths, and likely secrets are always redacted on prompt
/// surfaces. User/account IDs are redacted only for non-Discord channels so
/// Discord identity labels can remain human-readable where needed.
pub fn redact_prompt_text(content: &str, platform: Option<&str>) -> String {
    let redact_user_ids = platform.is_some_and(should_redact);
    let mut ranges = Vec::new();

    collect_regex_ranges(
        content,
        r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b",
        "email",
        &mut ranges,
        |_, _| true,
    );
    collect_regex_ranges(
        content,
        r"\+?\d[\d .()/-]{8,}\d",
        "phone",
        &mut ranges,
        |value, range| {
            if !redact_user_ids && has_user_id_label_prefix(content, range.start) {
                return false;
            }
            let digits = value.chars().filter(|ch| ch.is_ascii_digit()).count();
            (10..=15).contains(&digits)
        },
    );
    collect_regex_ranges(
        content,
        r#"(?i)\b(?:api[_-]?key|access[_-]?token|auth(?:orization)?|bearer|secret|client[_-]?secret|password|passwd|private[_-]?key)\b\s*[:=]\s*["']?[^"'\s,;]{8,}"#,
        "secret",
        &mut ranges,
        |_, _| true,
    );
    collect_regex_ranges(
        content,
        r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{20,}",
        "secret",
        &mut ranges,
        |_, _| true,
    );
    collect_regex_ranges(
        content,
        r"\b(?:sk-[A-Za-z0-9]{20,}|gh[pousr]_[A-Za-z0-9_]{20,}|xox[baprs]-[A-Za-z0-9-]{20,}|[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,})\b",
        "secret",
        &mut ranges,
        |_, _| true,
    );
    collect_regex_ranges(
        content,
        r#"(?m)(?:~|/Users|/home|/var/folders|/tmp|/private/tmp|/Volumes)/[^\s`"'<>)\]]+"#,
        "path",
        &mut ranges,
        |_, _| true,
    );
    collect_regex_ranges(
        content,
        r#"(?i)\b[A-Z]:\\[^\s`"'<>)\]]+"#,
        "path",
        &mut ranges,
        |_, _| true,
    );

    if redact_user_ids {
        collect_regex_ranges(
            content,
            r"(?i)\b(?:actor|user|sender|principal|account|telegram|signal)[_-]?(?:id|user)?\s*[:=]\s*[A-Za-z0-9@._:+-]{5,}",
            "user-id",
            &mut ranges,
            |_, _| true,
        );
    }

    apply_redactions(content, ranges)
}

#[derive(Debug, Clone)]
struct PiiRedaction {
    range: Range<usize>,
    replacement: String,
}

fn collect_regex_ranges(
    content: &str,
    pattern: &str,
    kind: &'static str,
    ranges: &mut Vec<PiiRedaction>,
    keep: impl Fn(&str, Range<usize>) -> bool,
) {
    let regex = Regex::new(pattern).expect("constant PII regex pattern must compile");
    for mat in regex.find_iter(content) {
        let matched = mat.as_str();
        let range = mat.start()..mat.end();
        if keep(matched, range.clone()) {
            ranges.push(PiiRedaction {
                range,
                replacement: pii_placeholder(kind, matched),
            });
        }
    }
}

fn has_user_id_label_prefix(content: &str, match_start: usize) -> bool {
    let prefix = &content[..match_start];
    let tail = prefix
        .chars()
        .rev()
        .take(48)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    let normalized = tail.trim_end().to_ascii_lowercase();
    [
        "actor_id:",
        "actor-id:",
        "actor id:",
        "user_id:",
        "user-id:",
        "user id:",
        "sender_id:",
        "sender-id:",
        "sender id:",
        "principal_id:",
        "principal-id:",
        "principal id:",
        "account_id:",
        "account-id:",
        "account id:",
        "telegram_id:",
        "telegram-id:",
        "telegram id:",
        "signal_id:",
        "signal-id:",
        "signal id:",
    ]
    .iter()
    .any(|label| normalized.ends_with(label))
}

fn pii_placeholder(kind: &'static str, matched: &str) -> String {
    format!("[redacted {kind}:{}]", short_hash(matched))
}

fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let hex = hex::encode(digest);
    hex[..10].to_string()
}

fn apply_redactions(content: &str, mut ranges: Vec<PiiRedaction>) -> String {
    if ranges.is_empty() {
        return content.to_string();
    }

    ranges.sort_by_key(|redaction| redaction.range.start);
    let mut merged: Vec<PiiRedaction> = Vec::new();
    for redaction in ranges {
        if let Some(previous) = merged.last_mut()
            && redaction.range.start < previous.range.end
        {
            if redaction.range.end > previous.range.end {
                previous.range.end = redaction.range.end;
            }
            continue;
        }
        merged.push(redaction);
    }

    let mut output = String::with_capacity(content.len());
    let mut cursor = 0;
    for redaction in merged {
        output.push_str(&content[cursor..redaction.range.start]);
        output.push_str(&redaction.replacement);
        cursor = redaction.range.end;
    }
    output.push_str(&content[cursor..]);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_user_id_is_deterministic_and_prefixed() {
        let hashed = hash_user_id("12345");
        assert_eq!(hashed, hash_user_id("12345"));
        assert!(hashed.starts_with("user_"));
        assert_eq!(hashed.len(), 17);
        assert!(hashed[5..].chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn should_redact_non_discord_platforms() {
        assert!(should_redact("telegram"));
        assert!(should_redact("signal"));
    }

    #[test]
    fn should_not_redact_discord() {
        assert!(!should_redact("discord"));
        assert!(!should_redact("Discord"));
    }

    #[test]
    fn redacts_contacts_secrets_and_paths_deterministically() {
        let raw = "Email me@example.com, call +1 (555) 123-4567, token=sk-abcdefghijklmnopqrst, path /Users/alex/project/.env";
        let redacted = redact_prompt_text(raw, Some("discord"));

        assert_eq!(redacted, redact_prompt_text(raw, Some("discord")));
        assert!(!redacted.contains("me@example.com"));
        assert!(!redacted.contains("555"));
        assert!(!redacted.contains("sk-abcdefghijklmnopqrst"));
        assert!(!redacted.contains("/Users/alex"));
        assert!(redacted.contains("[redacted email:"));
        assert!(redacted.contains("[redacted phone:"));
        assert!(redacted.contains("[redacted secret:"));
        assert!(redacted.contains("[redacted path:"));
    }

    #[test]
    fn redacts_labeled_user_ids_only_for_non_discord_channels() {
        let raw = "actor_id: 15551234567";

        assert!(redact_prompt_text(raw, Some("signal")).contains("[redacted user-id:"));
        assert!(redact_prompt_text(raw, Some("discord")).contains("actor_id: 15551234567"));
    }
}
