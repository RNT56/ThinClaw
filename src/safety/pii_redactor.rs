//! Prompt-surface PII redaction helpers.

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
}
