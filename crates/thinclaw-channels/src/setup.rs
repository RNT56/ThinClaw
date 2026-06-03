//! Pure setup helpers shared by root setup adapters.

use rand::RngCore;
use uuid::Uuid;

pub const WEBHOOK_SECRET_BYTES: usize = 32;

pub const TUNNEL_PROVIDER_NGROK: &str = "ngrok";
pub const TUNNEL_PROVIDER_CLOUDFLARE: &str = "cloudflare";
pub const TUNNEL_PROVIDER_TAILSCALE: &str = "tailscale";
pub const TUNNEL_PROVIDER_CUSTOM: &str = "custom";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramSetupUpdate {
    pub update_id: i64,
    pub message: Option<TelegramSetupMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramSetupMessage {
    pub from: Option<TelegramSetupUser>,
    pub chat: Option<TelegramSetupChat>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramSetupUser {
    pub id: i64,
    pub first_name: String,
    pub username: Option<String>,
    pub is_bot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramSetupChat {
    pub chat_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramOwnerCandidate {
    pub user_id: i64,
    pub display_name: String,
    pub username: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramOwnerCapture {
    pub candidate: TelegramOwnerCandidate,
    pub acknowledged_offset: i64,
    pub ignored_update_upper_bound: i64,
}

impl TelegramOwnerCandidate {
    pub fn summary(&self) -> String {
        if let Some(username) = self.username.as_deref() {
            format!("@{} (ID: {})", username, self.user_id)
        } else {
            format!("{} (ID: {})", self.display_name, self.user_id)
        }
    }
}

pub fn extract_telegram_owner_capture(
    updates: &[TelegramSetupUpdate],
) -> Option<TelegramOwnerCapture> {
    updates.iter().find_map(|update| {
        let message = update.message.as_ref()?;
        let from = message.from.as_ref()?;
        let chat = message.chat.as_ref()?;
        if from.is_bot || chat.chat_type != "private" {
            return None;
        }

        let display_name = from
            .username
            .as_ref()
            .map(|username| format!("@{}", username))
            .unwrap_or_else(|| from.first_name.clone());

        let candidate = TelegramOwnerCandidate {
            user_id: from.id,
            display_name,
            username: from.username.clone(),
        };
        let acknowledged_offset = next_telegram_update_offset(updates)
            .unwrap_or_else(|| update.update_id.saturating_add(1));
        let ignored_update_upper_bound = acknowledged_offset.saturating_sub(1);

        Some(TelegramOwnerCapture {
            candidate,
            acknowledged_offset,
            ignored_update_upper_bound,
        })
    })
}

pub fn next_telegram_update_offset(updates: &[TelegramSetupUpdate]) -> Option<i64> {
    updates
        .iter()
        .map(|update| update.update_id.saturating_add(1))
        .max()
}

pub fn validate_e164_account(account: &str) -> Result<(), String> {
    if !account.starts_with('+') {
        return Err("E.164 account must start with '+'".to_string());
    }
    let digits = &account[1..];
    if digits.is_empty() {
        return Err("E.164 account must have digits after '+'".to_string());
    }
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err("E.164 account must contain only digits after '+'".to_string());
    }
    if digits.len() < 7 || digits.len() > 15 {
        return Err("E.164 account must be 7-15 digits after '+'".to_string());
    }
    Ok(())
}

pub fn validate_allow_from_list(list: &str) -> Result<(), String> {
    if list.is_empty() {
        return Ok(());
    }
    for (i, item) in list.split(',').enumerate() {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "*" {
            continue;
        }
        if let Some(uuid_part) = trimmed.strip_prefix("uuid:") {
            if Uuid::parse_str(uuid_part).is_err() {
                return Err(format!(
                    "allow_from[{}]: '{}' is not a valid UUID (after 'uuid:' prefix)",
                    i, trimmed
                ));
            }
            continue;
        }
        if validate_e164_account(trimmed).is_ok() {
            continue;
        }
        if Uuid::parse_str(trimmed).is_ok() {
            continue;
        }
        return Err(format!(
            "allow_from[{}]: '{}' must be '*', E.164 phone number, UUID, or 'uuid:<id>'",
            i, trimmed
        ));
    }
    Ok(())
}

pub fn validate_allow_from_groups_list(list: &str) -> Result<(), String> {
    if list.is_empty() {
        return Ok(());
    }
    for item in list.split(',') {
        let trimmed = item.trim();
        if trimmed.is_empty() || trimmed == "*" {
            continue;
        }
    }
    Ok(())
}

pub fn generate_webhook_secret() -> String {
    generate_secret_with_length(WEBHOOK_SECRET_BYTES)
}

pub fn generate_secret_with_length(length: usize) -> String {
    let mut rng = rand::thread_rng();
    let mut bytes = vec![0u8; length];
    rng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn normalize_static_tunnel_url(raw_url: &str) -> Result<String, String> {
    if !raw_url.starts_with("https://") {
        return Err("Invalid tunnel URL: must use HTTPS".to_string());
    }

    Ok(raw_url.trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        TelegramOwnerCandidate, TelegramSetupChat, TelegramSetupMessage, TelegramSetupUpdate,
        TelegramSetupUser, extract_telegram_owner_capture, generate_secret_with_length,
        generate_webhook_secret, next_telegram_update_offset, normalize_static_tunnel_url,
        validate_allow_from_list, validate_e164_account,
    };

    #[test]
    fn generate_webhook_secret_uses_32_bytes_as_hex() {
        let secret = generate_webhook_secret();
        assert_eq!(secret.len(), 64);
        assert!(secret.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_secret_with_length_uses_hex_encoding() {
        let s = generate_secret_with_length(16);
        assert_eq!(s.len(), 32);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));

        let s2 = generate_secret_with_length(1);
        assert_eq!(s2.len(), 2);
    }

    #[test]
    fn extract_telegram_owner_candidate_uses_first_private_human_message() {
        let updates = vec![
            telegram_update(10, 42, "private", "Ada", Some("ada"), false),
            telegram_update(11, 99, "private", "Grace", Some("grace"), false),
        ];

        assert_eq!(
            extract_telegram_owner_capture(&updates).map(|capture| capture.candidate),
            Some(TelegramOwnerCandidate {
                user_id: 42,
                display_name: "@ada".to_string(),
                username: Some("ada".to_string()),
            })
        );
    }

    #[test]
    fn extract_telegram_owner_candidate_ignores_group_messages_and_bots() {
        let updates = vec![
            telegram_update(7, 13, "group", "Ignored Group", Some("group_user"), false),
            telegram_update(8, 21, "private", "Ignored Bot", Some("helper_bot"), true),
            telegram_update(9, 34, "private", "Owner", None, false),
        ];

        assert_eq!(
            extract_telegram_owner_capture(&updates).map(|capture| capture.candidate),
            Some(TelegramOwnerCandidate {
                user_id: 34,
                display_name: "Owner".to_string(),
                username: None,
            })
        );
    }

    #[test]
    fn extract_telegram_owner_capture_tracks_acknowledged_offset() {
        let updates = vec![
            telegram_update(21, 17, "private", "Owner", Some("owner"), false),
            telegram_update(24, 88, "private", "Another", Some("another"), false),
        ];

        let capture = extract_telegram_owner_capture(&updates)
            .expect("expected capture from private message");
        assert_eq!(
            capture.candidate,
            TelegramOwnerCandidate {
                user_id: 17,
                display_name: "@owner".to_string(),
                username: Some("owner".to_string()),
            }
        );
        assert_eq!(capture.acknowledged_offset, 25);
        assert_eq!(capture.ignored_update_upper_bound, 24);
    }

    #[test]
    fn next_telegram_update_offset_tracks_latest_update_plus_one() {
        let updates = vec![
            telegram_update(4, 1, "private", "One", None, false),
            telegram_update(12, 2, "private", "Two", None, false),
            telegram_update(9, 3, "private", "Three", None, false),
        ];

        assert_eq!(next_telegram_update_offset(&updates), Some(13));
    }

    #[test]
    fn telegram_owner_candidate_summary_prefers_username() {
        let with_username = TelegramOwnerCandidate {
            user_id: 42,
            display_name: "@ada".to_string(),
            username: Some("ada".to_string()),
        };
        assert_eq!(with_username.summary(), "@ada (ID: 42)");

        let without_username = TelegramOwnerCandidate {
            user_id: 99,
            display_name: "Grace".to_string(),
            username: None,
        };
        assert_eq!(without_username.summary(), "Grace (ID: 99)");
    }

    #[test]
    fn validate_e164_account_accepts_only_portable_phone_shape() {
        assert!(validate_e164_account("+15551234567").is_ok());
        assert_eq!(
            validate_e164_account("15551234567"),
            Err("E.164 account must start with '+'".to_string())
        );
        assert_eq!(
            validate_e164_account("+1555x34567"),
            Err("E.164 account must contain only digits after '+'".to_string())
        );
        assert_eq!(
            validate_e164_account("+123456"),
            Err("E.164 account must be 7-15 digits after '+'".to_string())
        );
    }

    #[test]
    fn validate_allow_from_list_accepts_expected_identifiers() {
        assert!(validate_allow_from_list("").is_ok());
        assert!(validate_allow_from_list("*, +15551234567").is_ok());
        assert!(validate_allow_from_list("550e8400-e29b-41d4-a716-446655440000").is_ok());
        assert!(validate_allow_from_list("uuid:550e8400-e29b-41d4-a716-446655440000").is_ok());

        assert_eq!(
            validate_allow_from_list("uuid:not-a-uuid"),
            Err(
                "allow_from[0]: 'uuid:not-a-uuid' is not a valid UUID (after 'uuid:' prefix)"
                    .to_string()
            )
        );
        assert_eq!(
            validate_allow_from_list("not-valid"),
            Err(
                "allow_from[0]: 'not-valid' must be '*', E.164 phone number, UUID, or 'uuid:<id>'"
                    .to_string()
            )
        );
    }

    #[test]
    fn normalize_static_tunnel_url_requires_https_and_trims_trailing_slashes() {
        assert_eq!(
            normalize_static_tunnel_url("https://example.test///"),
            Ok("https://example.test".to_string())
        );
        assert_eq!(
            normalize_static_tunnel_url("http://example.test"),
            Err("Invalid tunnel URL: must use HTTPS".to_string())
        );
    }

    fn telegram_update(
        update_id: i64,
        user_id: i64,
        chat_type: &str,
        first_name: &str,
        username: Option<&str>,
        is_bot: bool,
    ) -> TelegramSetupUpdate {
        TelegramSetupUpdate {
            update_id,
            message: Some(TelegramSetupMessage {
                from: Some(TelegramSetupUser {
                    id: user_id,
                    first_name: first_name.to_string(),
                    username: username.map(str::to_string),
                    is_bot,
                }),
                chat: Some(TelegramSetupChat {
                    chat_type: chat_type.to_string(),
                }),
            }),
        }
    }
}
