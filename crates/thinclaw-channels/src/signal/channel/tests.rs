use super::*;
use crate::signal::attachments::signal_attachment_dir;

use std::sync::Mutex;

use tempfile::tempdir;

static SIGNAL_ATTACHMENTS_ENV_LOCK: Mutex<()> = Mutex::new(());

fn make_config() -> SignalConfig {
    SignalConfig {
        http_url: "http://127.0.0.1:8686".to_string(),
        account: "+1234567890".to_string(),
        allow_from: vec!["+1111111111".to_string()],
        allow_from_groups: vec![],
        dm_policy: "allowlist".to_string(),
        group_policy: "disabled".to_string(),
        group_allow_from: vec![],
        ignore_attachments: false,
        ignore_stories: false,
    }
}

/// Create a config that allows a specific group (and all senders).
fn make_config_with_allowed_group(group_id: &str) -> SignalConfig {
    SignalConfig {
        http_url: "http://127.0.0.1:8686".to_string(),
        account: "+1234567890".to_string(),
        allow_from: vec!["*".to_string()],
        allow_from_groups: vec![group_id.to_string()],
        dm_policy: "allowlist".to_string(),
        group_policy: "allowlist".to_string(),
        group_allow_from: vec![],
        ignore_attachments: true,
        ignore_stories: true,
    }
}

fn make_channel() -> Result<SignalChannel, ChannelError> {
    SignalChannel::new(make_config())
}

fn make_channel_with_allowed_group(group_id: &str) -> Result<SignalChannel, ChannelError> {
    SignalChannel::new(make_config_with_allowed_group(group_id))
}

fn make_envelope(source_number: Option<&str>, message: Option<&str>) -> Envelope {
    Envelope {
        source: source_number.map(String::from),
        source_number: source_number.map(String::from),
        source_name: None,
        source_uuid: None,
        data_message: message.map(|m| DataMessage {
            message: Some(m.to_string()),
            timestamp: Some(1_700_000_000_000),
            group_info: None,
            attachments: None,
        }),
        story_message: None,
        timestamp: Some(1_700_000_000_000),
    }
}

#[test]
fn creates_with_correct_fields() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    assert_eq!(ch.config.http_url, "http://127.0.0.1:8686");
    assert_eq!(ch.config.account, "+1234567890");
    assert_eq!(ch.config.allow_from.len(), 1);
    assert!(ch.config.allow_from_groups.is_empty());
    assert!(!ch.config.ignore_attachments);
    assert!(!ch.config.ignore_stories);
    Ok(())
}

#[test]
fn strips_trailing_slash() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.http_url = "http://127.0.0.1:8686/".to_string();
    let ch = SignalChannel::new(config)?;
    assert_eq!(ch.config.http_url, "http://127.0.0.1:8686");
    Ok(())
}

#[test]
fn debug_mode_disabled_by_default() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    assert!(!ch.is_debug());
    Ok(())
}

#[test]
fn debug_mode_toggle() -> Result<(), ChannelError> {
    let ch = make_channel()?;

    // Initially disabled
    assert!(!ch.is_debug());

    // Toggle on
    let new_state = ch.toggle_debug();
    assert!(new_state);
    assert!(ch.is_debug());

    // Toggle off
    let new_state = ch.toggle_debug();
    assert!(!new_state);
    assert!(!ch.is_debug());

    Ok(())
}

#[test]
fn debug_mode_persists_across_toggles() -> Result<(), ChannelError> {
    let ch = make_channel()?;

    // Multiple toggles
    ch.toggle_debug();
    assert!(ch.is_debug());
    ch.toggle_debug();
    assert!(!ch.is_debug());
    ch.toggle_debug();
    assert!(ch.is_debug());
    ch.toggle_debug();
    assert!(!ch.is_debug());

    Ok(())
}

#[test]
fn wildcard_allows_anyone() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.allow_from = vec!["*".to_string()];
    let ch = SignalChannel::new(config)?;
    assert!(ch.is_sender_allowed("+9999999999"));
    Ok(())
}

#[test]
fn specific_sender_allowed() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    assert!(ch.is_sender_allowed("+1111111111"));
    Ok(())
}

#[test]
fn unknown_sender_denied() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    assert!(!ch.is_sender_allowed("+9999999999"));
    Ok(())
}

#[test]
fn empty_allowlist_denies_all() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.allow_from = vec![];
    let ch = SignalChannel::new(config)?;
    assert!(!ch.is_sender_allowed("+1111111111"));
    Ok(())
}

#[test]
fn uuid_prefix_in_allowlist() -> Result<(), ChannelError> {
    let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    let mut config = make_config();
    config.allow_from = vec![format!("uuid:{uuid}")];
    let ch = SignalChannel::new(config)?;
    assert!(ch.is_sender_allowed(uuid));
    // Should not match phone numbers.
    assert!(!ch.is_sender_allowed("+1111111111"));
    Ok(())
}

#[test]
fn bare_uuid_in_allowlist() -> Result<(), ChannelError> {
    let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    let mut config = make_config();
    config.allow_from = vec![uuid.to_string()];
    let ch = SignalChannel::new(config)?;
    assert!(ch.is_sender_allowed(uuid));
    Ok(())
}

#[test]
fn group_allowlist_filtering() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.allow_from = vec!["*".to_string()];
    config.allow_from_groups = vec!["group123".to_string()];
    let ch = SignalChannel::new(config)?;
    assert!(ch.is_group_allowed("group123"));
    assert!(!ch.is_group_allowed("other_group"));
    Ok(())
}

#[test]
fn group_allowlist_wildcard() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.allow_from_groups = vec!["*".to_string()];
    let ch = SignalChannel::new(config)?;
    assert!(ch.is_group_allowed("any_group"));
    Ok(())
}

#[test]
fn group_allowlist_empty_denies_all() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.allow_from_groups = vec![];
    let ch = SignalChannel::new(config)?;
    assert!(!ch.is_group_allowed("any_group"));
    Ok(())
}

#[test]
fn name_returns_signal() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    assert_eq!(ch.name(), "signal");
    Ok(())
}

#[test]
fn process_envelope_dm_accepted_with_empty_allow_from_groups() -> Result<(), ChannelError> {
    // Empty allow_from_groups = DMs only. DMs should be accepted.
    let ch = make_channel()?;
    let env = make_envelope(Some("+1111111111"), Some("Hello!"));
    assert!(ch.process_envelope(&env).is_some());
    Ok(())
}

#[test]
fn process_envelope_group_denied_with_empty_allow_from_groups() -> Result<(), ChannelError> {
    // Empty allow_from_groups = DMs only. Group messages should be denied.
    let mut config = make_config();
    config.allow_from = vec!["*".to_string()];
    let ch = SignalChannel::new(config)?;

    let env = Envelope {
        source: Some("+1111111111".to_string()),
        source_number: Some("+1111111111".to_string()),
        source_name: None,
        source_uuid: None,
        data_message: Some(DataMessage {
            message: Some("hi".to_string()),
            timestamp: Some(1000),
            group_info: Some(GroupInfo {
                group_id: Some("group123".to_string()),
            }),
            attachments: None,
        }),
        story_message: None,
        timestamp: Some(1000),
    };
    assert!(ch.process_envelope(&env).is_none());
    Ok(())
}

#[test]
fn process_envelope_group_accepted_when_in_allow_from_groups() -> Result<(), ChannelError> {
    let ch = make_channel_with_allowed_group("group123")?;

    let env = Envelope {
        source: Some("+1111111111".to_string()),
        source_number: Some("+1111111111".to_string()),
        source_name: None,
        source_uuid: None,
        data_message: Some(DataMessage {
            message: Some("hi".to_string()),
            timestamp: Some(1000),
            group_info: Some(GroupInfo {
                group_id: Some("group123".to_string()),
            }),
            attachments: None,
        }),
        story_message: None,
        timestamp: Some(1000),
    };
    assert!(ch.process_envelope(&env).is_some());

    // Different group should be denied.
    let env2 = Envelope {
        source: Some("+1111111111".to_string()),
        source_number: Some("+1111111111".to_string()),
        source_name: None,
        source_uuid: None,
        data_message: Some(DataMessage {
            message: Some("hi".to_string()),
            timestamp: Some(1000),
            group_info: Some(GroupInfo {
                group_id: Some("other_group".to_string()),
            }),
            attachments: None,
        }),
        story_message: None,
        timestamp: Some(1000),
    };
    assert!(ch.process_envelope(&env2).is_none());
    Ok(())
}

#[test]
fn reply_target_dm() {
    let dm = DataMessage {
        message: Some("hi".to_string()),
        timestamp: Some(1000),
        group_info: None,
        attachments: None,
    };
    assert_eq!(
        SignalChannel::reply_target(&dm, "+1111111111"),
        "+1111111111"
    );
}

#[test]
fn reply_target_group() {
    let group = DataMessage {
        message: Some("hi".to_string()),
        timestamp: Some(1000),
        group_info: Some(GroupInfo {
            group_id: Some("group123".to_string()),
        }),
        attachments: None,
    };
    assert_eq!(
        SignalChannel::reply_target(&group, "+1111111111"),
        "group:group123"
    );
}

#[test]
fn parse_recipient_target_e164_is_direct() {
    assert_eq!(
        SignalChannel::parse_recipient_target("+1234567890"),
        RecipientTarget::Direct("+1234567890".to_string())
    );
}

#[test]
fn parse_recipient_target_prefixed_group_is_group() {
    assert_eq!(
        SignalChannel::parse_recipient_target("group:abc123"),
        RecipientTarget::Group("abc123".to_string())
    );
}

#[test]
fn parse_recipient_target_uuid_is_direct() {
    let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    assert_eq!(
        SignalChannel::parse_recipient_target(uuid),
        RecipientTarget::Direct(uuid.to_string())
    );
}

#[test]
fn parse_recipient_target_non_e164_plus_is_group() {
    assert_eq!(
        SignalChannel::parse_recipient_target("+abc123"),
        RecipientTarget::Group("+abc123".to_string())
    );
}

#[test]
fn is_e164_detects_valid_and_invalid() {
    assert!(SignalChannel::is_e164("+1234567890"));
    assert!(!SignalChannel::is_e164("+123456"));
    assert!(!SignalChannel::is_e164("1234567890"));
    assert!(!SignalChannel::is_e164("abc"));
}

#[test]
fn conversation_key_helpers() {
    assert_eq!(SignalChannel::conversation_kind(false), "direct");
    assert_eq!(SignalChannel::conversation_kind(true), "group");
    assert_eq!(
        SignalChannel::conversation_scope_id(true, "+111", "stable", Some("group-1")),
        "signal:group:group-1"
    );
    assert_eq!(
        SignalChannel::conversation_scope_id(false, "+111", "stable", Some("group-1")),
        "signal:direct:stable"
    );
    assert_eq!(
        SignalChannel::external_conversation_key(true, "+111", "stable", Some("group-1")),
        "signal://group/group-1"
    );
    assert_eq!(
        SignalChannel::external_conversation_key(false, "+111", "stable", Some("group-1")),
        "signal://direct/stable"
    );
}

#[test]
fn build_rpc_params_static_respects_fields() {
    let direct = SignalChannel::build_rpc_params_static(
        "http://127.0.0.1:8686",
        "acct",
        &RecipientTarget::Direct("+111".into()),
        Some("hello"),
    );
    assert_eq!(direct["recipient"], serde_json::json!(["+111"]));
    assert_eq!(direct["account"], "acct");
    assert_eq!(direct["message"], "hello");

    let group = SignalChannel::build_rpc_params_static(
        "http://127.0.0.1:8686",
        "acct",
        &RecipientTarget::Group("group-1".into()),
        None,
    );
    assert_eq!(group["groupId"], "group-1");
    assert_eq!(group["account"], "acct");
    assert!(group.get("message").is_none());
}

#[test]
fn redact_url_removes_auth() {
    let redacted = SignalChannel::redact_url("https://user:password@example.com/api/v1/rpc");
    assert!(redacted.contains("**REDACTED**"));
    assert!(!redacted.contains("password"));
}

#[test]
fn redact_url_invalid_url() {
    assert_eq!(SignalChannel::redact_url("not a url"), "<invalid-url>");
}

#[test]
fn is_uuid_valid() {
    assert!(SignalChannel::is_uuid(
        "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
    ));
    assert!(SignalChannel::is_uuid(
        "00000000-0000-0000-0000-000000000000"
    ));
}

#[test]
fn is_uuid_invalid() {
    assert!(!SignalChannel::is_uuid("+1234567890"));
    assert!(!SignalChannel::is_uuid("not-a-uuid"));
    assert!(!SignalChannel::is_uuid("group:abc123"));
    assert!(!SignalChannel::is_uuid(""));
}

#[test]
fn thread_id_from_identifier_is_deterministic() {
    let id1 = SignalChannel::thread_id_from_identifier("+1234567890");
    let id2 = SignalChannel::thread_id_from_identifier("+1234567890");
    assert_eq!(id1, id2, "same input should produce same UUID");
}

#[test]
fn thread_id_from_identifier_is_valid_uuid() {
    let id = SignalChannel::thread_id_from_identifier("+1234567890");
    assert!(Uuid::parse_str(&id).is_ok(), "should be a valid UUID");
}

#[test]
fn thread_id_from_identifier_different_inputs() {
    let id1 = SignalChannel::thread_id_from_identifier("+1234567890");
    let id2 = SignalChannel::thread_id_from_identifier("+9876543210");
    assert_ne!(id1, id2, "different inputs should produce different UUIDs");
}

#[test]
fn sender_prefers_source_number() {
    let env = Envelope {
        source: Some("uuid-123".to_string()),
        source_number: Some("+1111111111".to_string()),
        source_name: None,
        source_uuid: None,
        data_message: None,
        story_message: None,
        timestamp: Some(1000),
    };
    assert_eq!(SignalChannel::sender(&env), Some("+1111111111".to_string()));
}

#[test]
fn sender_falls_back_to_source() {
    let env = Envelope {
        source: Some("a1b2c3d4-e5f6-7890-abcd-ef1234567890".to_string()),
        source_number: None,
        source_name: None,
        source_uuid: None,
        data_message: None,
        story_message: None,
        timestamp: Some(1000),
    };
    assert_eq!(
        SignalChannel::sender(&env),
        Some("a1b2c3d4-e5f6-7890-abcd-ef1234567890".to_string())
    );
}

#[test]
fn sender_none_when_both_missing() {
    let env = Envelope {
        source: None,
        source_number: None,
        source_name: None,
        source_uuid: None,
        data_message: None,
        story_message: None,
        timestamp: None,
    };
    assert_eq!(SignalChannel::sender(&env), None);
}

#[test]
fn process_envelope_valid_dm() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    let env = make_envelope(Some("+1111111111"), Some("Hello!"));
    let (msg, target) = ch.process_envelope(&env).unwrap();
    assert_eq!(msg.content, "Hello!");
    assert_eq!(msg.user_id, "+1111111111");
    assert_eq!(msg.channel, "signal");
    assert_eq!(target, "+1111111111");
    Ok(())
}

#[test]
fn process_envelope_denied_sender() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    let env = make_envelope(Some("+9999999999"), Some("Hello!"));
    assert!(ch.process_envelope(&env).is_none());
    Ok(())
}

#[test]
fn process_envelope_empty_message() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    let env = make_envelope(Some("+1111111111"), Some(""));
    assert!(ch.process_envelope(&env).is_none());
    Ok(())
}

#[test]
fn process_envelope_no_data_message() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    let env = make_envelope(Some("+1111111111"), None);
    assert!(ch.process_envelope(&env).is_none());
    Ok(())
}

#[test]
fn process_envelope_skips_stories() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.allow_from = vec!["*".to_string()];
    config.ignore_stories = true;
    let ch = SignalChannel::new(config)?;
    let mut env = make_envelope(Some("+1111111111"), Some("story text"));
    env.story_message = Some(serde_json::json!({}));
    assert!(ch.process_envelope(&env).is_none());
    Ok(())
}

#[test]
fn process_envelope_skips_attachment_only() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.allow_from = vec!["*".to_string()];
    config.ignore_attachments = true;
    let ch = SignalChannel::new(config)?;
    let env = Envelope {
        source: Some("+1111111111".to_string()),
        source_number: Some("+1111111111".to_string()),
        source_name: None,
        source_uuid: None,
        data_message: Some(DataMessage {
            message: None,
            timestamp: Some(1_700_000_000_000),
            group_info: None,
            attachments: Some(vec![SignalAttachment {
                content_type: Some("image/png".to_string()),
                filename: None,
                size: None,
                id: None,
            }]),
        }),
        story_message: None,
        timestamp: Some(1_700_000_000_000),
    };
    assert!(ch.process_envelope(&env).is_none());
    Ok(())
}

#[test]
fn process_envelope_uuid_sender_dm() -> Result<(), ChannelError> {
    let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    let mut config = make_config();
    config.allow_from = vec!["*".to_string()];
    let ch = SignalChannel::new(config)?;

    let env = Envelope {
        source: Some(uuid.to_string()),
        source_number: None,
        source_name: Some("Privacy User".to_string()),
        source_uuid: None,
        data_message: Some(DataMessage {
            message: Some("Hello from privacy user".to_string()),
            timestamp: Some(1_700_000_000_000),
            group_info: None,
            attachments: None,
        }),
        story_message: None,
        timestamp: Some(1_700_000_000_000),
    };
    let (msg, target) = ch.process_envelope(&env).unwrap();
    assert_eq!(msg.user_id, uuid);
    assert_eq!(msg.user_name.as_deref(), Some("Privacy User"));
    assert_eq!(msg.content, "Hello from privacy user");
    assert_eq!(target, uuid);

    // Verify reply routing: UUID sender in DM should route as Direct.
    let parsed = SignalChannel::parse_recipient_target(&target);
    assert_eq!(parsed, RecipientTarget::Direct(uuid.to_string()));
    Ok(())
}

#[test]
fn process_envelope_uuid_sender_in_group() -> Result<(), ChannelError> {
    let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    let mut config = make_config_with_allowed_group("testgroup");
    config.ignore_attachments = false;
    config.ignore_stories = false;
    let ch = SignalChannel::new(config)?;

    let env = Envelope {
        source: Some(uuid.to_string()),
        source_number: None,
        source_name: None,
        source_uuid: None,
        data_message: Some(DataMessage {
            message: Some("Group msg from privacy user".to_string()),
            timestamp: Some(1_700_000_000_000),
            group_info: Some(GroupInfo {
                group_id: Some("testgroup".to_string()),
            }),
            attachments: None,
        }),
        story_message: None,
        timestamp: Some(1_700_000_000_000),
    };
    let (msg, target) = ch.process_envelope(&env).unwrap();
    assert_eq!(msg.user_id, uuid);
    assert_eq!(target, "group:testgroup");
    // Groups now use deterministic UUID derived from group ID
    let expected_thread_id = SignalChannel::thread_id_from_identifier("group:testgroup");
    assert_eq!(msg.thread_id, Some(expected_thread_id));

    // Verify reply routing: group message should still route as Group.
    let parsed = SignalChannel::parse_recipient_target(&target);
    assert_eq!(parsed, RecipientTarget::Group("testgroup".to_string()));
    Ok(())
}

#[test]
fn process_envelope_group_not_in_allow_from_groups() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.allow_from = vec!["*".to_string()];
    config.allow_from_groups = vec!["allowed_group".to_string()];
    let ch = SignalChannel::new(config)?;

    let env = Envelope {
        source: Some("+1111111111".to_string()),
        source_number: Some("+1111111111".to_string()),
        source_name: None,
        source_uuid: None,
        data_message: Some(DataMessage {
            message: Some("Hi".to_string()),
            timestamp: Some(1_700_000_000_000),
            group_info: Some(GroupInfo {
                group_id: Some("other_group".to_string()),
            }),
            attachments: None,
        }),
        story_message: None,
        timestamp: Some(1_700_000_000_000),
    };
    assert!(ch.process_envelope(&env).is_none());
    Ok(())
}

#[test]
fn sse_envelope_deserializes() {
    let json = r#"{
            "envelope": {
                "source": "+1111111111",
                "sourceNumber": "+1111111111",
                "sourceName": "Test User",
                "timestamp": 1700000000000,
                "dataMessage": {
                    "message": "Hello Signal!",
                    "timestamp": 1700000000000
                }
            }
        }"#;
    let sse: SseEnvelope = serde_json::from_str(json).unwrap();
    let env = sse.envelope.unwrap();
    assert_eq!(env.source_number.as_deref(), Some("+1111111111"));
    assert_eq!(env.source_name.as_deref(), Some("Test User"));
    let dm = env.data_message.unwrap();
    assert_eq!(dm.message.as_deref(), Some("Hello Signal!"));
}

#[test]
fn sse_envelope_deserializes_group() {
    let json = r#"{
            "envelope": {
                "sourceNumber": "+2222222222",
                "dataMessage": {
                    "message": "Group msg",
                    "groupInfo": {
                        "groupId": "abc123"
                    }
                }
            }
        }"#;
    let sse: SseEnvelope = serde_json::from_str(json).unwrap();
    let env = sse.envelope.unwrap();
    let dm = env.data_message.unwrap();
    assert_eq!(
        dm.group_info.as_ref().unwrap().group_id.as_deref(),
        Some("abc123")
    );
}

#[test]
fn envelope_defaults() {
    let json = r#"{}"#;
    let env: Envelope = serde_json::from_str(json).unwrap();
    assert!(env.source.is_none());
    assert!(env.source_number.is_none());
    assert!(env.source_name.is_none());
    assert!(env.data_message.is_none());
    assert!(env.story_message.is_none());
    assert!(env.timestamp.is_none());
}

#[test]
fn normalize_allow_entry_strips_uuid_prefix() {
    assert_eq!(
        SignalChannel::normalize_allow_entry("uuid:abc-123"),
        "abc-123"
    );
    assert_eq!(
        SignalChannel::normalize_allow_entry("+1234567890"),
        "+1234567890"
    );
    assert_eq!(SignalChannel::normalize_allow_entry("*"), "*");
}

// ── build_rpc_params tests ──────────────────────────────────────

#[test]
fn build_rpc_params_direct_with_message() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    let target = RecipientTarget::Direct("+5555555555".to_string());
    let params = ch.build_rpc_params(&target, Some("Hello!"));
    assert_eq!(params["recipient"], serde_json::json!(["+5555555555"]));
    assert_eq!(params["account"], "+1234567890");
    assert_eq!(params["message"], "Hello!");
    // Direct targets must NOT include groupId.
    assert!(params.get("groupId").is_none());
    Ok(())
}

#[test]
fn build_rpc_params_direct_without_message() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    let target = RecipientTarget::Direct("+5555555555".to_string());
    let params = ch.build_rpc_params(&target, None);
    assert_eq!(params["recipient"], serde_json::json!(["+5555555555"]));
    assert_eq!(params["account"], "+1234567890");
    // No message key should be present for typing indicators.
    assert!(params.get("message").is_none());
    Ok(())
}

#[test]
fn build_rpc_params_group_with_message() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    let target = RecipientTarget::Group("abc123".to_string());
    let params = ch.build_rpc_params(&target, Some("Group msg"));
    assert_eq!(params["groupId"], "abc123");
    assert_eq!(params["account"], "+1234567890");
    assert_eq!(params["message"], "Group msg");
    // Group targets must NOT include recipient.
    assert!(params.get("recipient").is_none());
    Ok(())
}

#[test]
fn build_rpc_params_group_without_message() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    let target = RecipientTarget::Group("abc123".to_string());
    let params = ch.build_rpc_params(&target, None);
    assert_eq!(params["groupId"], "abc123");
    assert_eq!(params["account"], "+1234567890");
    assert!(params.get("message").is_none());
    Ok(())
}

#[test]
fn build_rpc_params_uuid_direct_target() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    let target = RecipientTarget::Direct(uuid.to_string());
    let params = ch.build_rpc_params(&target, Some("hi"));
    assert_eq!(params["recipient"], serde_json::json!([uuid]));
    Ok(())
}

// ── metadata assertion tests ────────────────────────────────────

#[test]
fn process_envelope_metadata_has_signal_fields() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    let env = make_envelope(Some("+1111111111"), Some("Hello!"));
    let (msg, _) = ch.process_envelope(&env).unwrap();
    assert_eq!(msg.metadata["signal_sender"], "+1111111111");
    assert_eq!(msg.metadata["signal_target"], "+1111111111");
    assert_eq!(msg.metadata["signal_timestamp"], 1_700_000_000_000_u64);
    Ok(())
}

#[test]
fn process_envelope_metadata_group_target() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.allow_from = vec!["*".to_string()];
    config.allow_from_groups = vec!["*".to_string()];
    config.group_policy = "allowlist".to_string();
    let ch = SignalChannel::new(config)?;

    let env = Envelope {
        source: Some("+2222222222".to_string()),
        source_number: Some("+2222222222".to_string()),
        source_name: None,
        source_uuid: None,
        data_message: Some(DataMessage {
            message: Some("In the group".to_string()),
            timestamp: Some(1_700_000_000_000),
            group_info: Some(GroupInfo {
                group_id: Some("mygroup".to_string()),
            }),
            attachments: None,
        }),
        story_message: None,
        timestamp: Some(1_700_000_000_000),
    };
    let (msg, _) = ch.process_envelope(&env).unwrap();
    assert_eq!(msg.metadata["signal_target"], "group:mygroup");
    assert_eq!(msg.metadata["signal_sender"], "+2222222222");
    Ok(())
}

// ── attachment-with-text tests ──────────────────────────────────

#[test]
fn process_envelope_attachment_with_text_not_skipped() -> Result<(), ChannelError> {
    // Even with ignore_attachments=true, messages that have BOTH text
    // and attachments should be processed (only attachment-only are skipped).
    let mut config = make_config();
    config.allow_from = vec!["*".to_string()];
    config.ignore_attachments = true;
    let ch = SignalChannel::new(config)?;

    let env = Envelope {
        source: Some("+1111111111".to_string()),
        source_number: Some("+1111111111".to_string()),
        source_name: None,
        source_uuid: None,
        data_message: Some(DataMessage {
            message: Some("Check this out".to_string()),
            timestamp: Some(1_700_000_000_000),
            group_info: None,
            attachments: Some(vec![SignalAttachment {
                content_type: Some("image/png".to_string()),
                filename: None,
                size: None,
                id: None,
            }]),
        }),
        story_message: None,
        timestamp: Some(1_700_000_000_000),
    };
    let result = ch.process_envelope(&env);
    assert!(
        result.is_some(),
        "Message with text + attachment should not be skipped"
    );
    let (msg, _) = result.unwrap();
    assert_eq!(msg.content, "Check this out");
    Ok(())
}

#[test]
fn process_envelope_attachment_only_not_skipped_when_ignore_disabled() -> Result<(), ChannelError> {
    // With ignore_attachments=false, attachment-only messages should be
    // processed with the "[Attachment]" placeholder text.
    let mut config = make_config();
    config.allow_from = vec!["*".to_string()];
    config.ignore_attachments = false;
    let ch = SignalChannel::new(config)?;

    let env = Envelope {
        source: Some("+1111111111".to_string()),
        source_number: Some("+1111111111".to_string()),
        source_name: None,
        source_uuid: None,
        data_message: Some(DataMessage {
            message: None,
            timestamp: Some(1_700_000_000_000),
            group_info: None,
            attachments: Some(vec![SignalAttachment {
                content_type: Some("image/png".to_string()),
                filename: None,
                size: None,
                id: None,
            }]),
        }),
        story_message: None,
        timestamp: Some(1_700_000_000_000),
    };
    // With ignore_attachments=false, attachment-only messages are now
    // processed with a media analysis prompt.
    let result = ch.process_envelope(&env);
    assert!(
        result.is_some(),
        "Attachment-only should be processed when ignore_attachments=false"
    );
    let (msg, _) = result.unwrap();
    assert_eq!(
        msg.content,
        "[Media received \u{2014} please analyze the attached content]"
    );
    Ok(())
}

// ── source_name / display name tests ────────────────────────────

#[test]
fn process_envelope_source_name_sets_user_name() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.allow_from = vec!["*".to_string()];
    let ch = SignalChannel::new(config)?;

    let env = Envelope {
        source: Some("+3333333333".to_string()),
        source_number: Some("+3333333333".to_string()),
        source_name: Some("Alice".to_string()),
        source_uuid: None,
        data_message: Some(DataMessage {
            message: Some("Hey".to_string()),
            timestamp: Some(1_700_000_000_000),
            group_info: None,
            attachments: None,
        }),
        story_message: None,
        timestamp: Some(1_700_000_000_000),
    };
    let (msg, _) = ch.process_envelope(&env).unwrap();
    assert_eq!(msg.user_name.as_deref(), Some("Alice"));
    Ok(())
}

#[test]
fn process_envelope_empty_source_name_not_set() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.allow_from = vec!["*".to_string()];
    let ch = SignalChannel::new(config)?;

    let env = Envelope {
        source: Some("+3333333333".to_string()),
        source_number: Some("+3333333333".to_string()),
        source_name: Some("".to_string()),
        source_uuid: None,
        data_message: Some(DataMessage {
            message: Some("Hey".to_string()),
            timestamp: Some(1_700_000_000_000),
            group_info: None,
            attachments: None,
        }),
        story_message: None,
        timestamp: Some(1_700_000_000_000),
    };
    let (msg, _) = ch.process_envelope(&env).unwrap();
    assert!(
        msg.user_name.is_none(),
        "Empty source_name should not set user_name"
    );
    Ok(())
}

#[test]
fn process_envelope_no_source_name_not_set() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    let env = make_envelope(Some("+1111111111"), Some("hi"));
    let (msg, _) = ch.process_envelope(&env).unwrap();
    assert!(msg.user_name.is_none());
    Ok(())
}

// ── thread_id tests ─────────────────────────────────────────────────────────────────

#[test]
fn process_envelope_dm_sets_thread_id_to_uuid() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    let env = make_envelope(Some("+1111111111"), Some("DM"));
    let (msg, _) = ch.process_envelope(&env).unwrap();
    // DMs now set thread_id to a deterministic UUID derived from phone number
    let expected_thread_id = SignalChannel::thread_id_from_identifier("+1111111111");
    assert_eq!(
        msg.thread_id,
        Some(expected_thread_id),
        "DMs should set thread_id to UUID"
    );
    Ok(())
}

#[test]
fn process_envelope_group_sets_thread_id_to_uuid() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.allow_from = vec!["*".to_string()];
    config.allow_from_groups = vec!["*".to_string()];
    config.group_policy = "allowlist".to_string();
    let ch = SignalChannel::new(config)?;

    let env = Envelope {
        source: Some("+1111111111".to_string()),
        source_number: Some("+1111111111".to_string()),
        source_name: None,
        source_uuid: None,
        data_message: Some(DataMessage {
            message: Some("Group msg".to_string()),
            timestamp: Some(1_700_000_000_000),
            group_info: Some(GroupInfo {
                group_id: Some("grp999".to_string()),
            }),
            attachments: None,
        }),
        story_message: None,
        timestamp: Some(1_700_000_000_000),
    };
    let (msg, _) = ch.process_envelope(&env).unwrap();
    // Groups now set thread_id to a deterministic UUID derived from group ID
    let expected_thread_id = SignalChannel::thread_id_from_identifier("group:grp999");
    assert_eq!(
        msg.thread_id,
        Some(expected_thread_id),
        "Groups should set thread_id to UUID"
    );
    Ok(())
}

// ── timestamp edge cases ────────────────────────────────────────

#[test]
fn process_envelope_uses_data_message_timestamp() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.allow_from = vec!["*".to_string()];
    let ch = SignalChannel::new(config)?;

    let env = Envelope {
        source: Some("+1111111111".to_string()),
        source_number: Some("+1111111111".to_string()),
        source_name: None,
        source_uuid: None,
        data_message: Some(DataMessage {
            message: Some("hi".to_string()),
            timestamp: Some(9999),
            group_info: None,
            attachments: None,
        }),
        story_message: None,
        timestamp: Some(1111),
    };
    let (msg, _) = ch.process_envelope(&env).unwrap();
    // data_message timestamp takes priority.
    assert_eq!(msg.metadata["signal_timestamp"], 9999);
    Ok(())
}

#[test]
fn process_envelope_falls_back_to_envelope_timestamp() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.allow_from = vec!["*".to_string()];
    let ch = SignalChannel::new(config)?;

    let env = Envelope {
        source: Some("+1111111111".to_string()),
        source_number: Some("+1111111111".to_string()),
        source_name: None,
        source_uuid: None,
        data_message: Some(DataMessage {
            message: Some("hi".to_string()),
            timestamp: None,
            group_info: None,
            attachments: None,
        }),
        story_message: None,
        timestamp: Some(7777),
    };
    let (msg, _) = ch.process_envelope(&env).unwrap();
    assert_eq!(msg.metadata["signal_timestamp"], 7777);
    Ok(())
}

#[test]
fn process_envelope_generates_timestamp_when_missing() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.allow_from = vec!["*".to_string()];
    let ch = SignalChannel::new(config)?;

    let env = Envelope {
        source: Some("+1111111111".to_string()),
        source_number: Some("+1111111111".to_string()),
        source_name: None,
        source_uuid: None,
        data_message: Some(DataMessage {
            message: Some("hi".to_string()),
            timestamp: None,
            group_info: None,
            attachments: None,
        }),
        story_message: None,
        timestamp: None,
    };
    let (msg, _) = ch.process_envelope(&env).unwrap();
    // Should generate a timestamp (current time in millis), just verify it's positive.
    let ts = msg.metadata["signal_timestamp"].as_u64().unwrap();
    assert!(ts > 0, "Generated timestamp should be positive");
    Ok(())
}

// ── SSE envelope deserialization edge cases ─────────────────────

#[test]
fn sse_envelope_missing_envelope_field() {
    let json = r#"{"account": "+1234567890"}"#;
    let sse: SseEnvelope = serde_json::from_str(json).unwrap();
    assert!(sse.envelope.is_none());
}

#[test]
fn sse_envelope_with_story_message() {
    let json = r#"{
            "envelope": {
                "sourceNumber": "+1111111111",
                "storyMessage": {"allowsReplies": true},
                "dataMessage": {
                    "message": "story text"
                }
            }
        }"#;
    let sse: SseEnvelope = serde_json::from_str(json).unwrap();
    let env = sse.envelope.unwrap();
    assert!(env.story_message.is_some());
    assert!(env.data_message.is_some());
}

#[test]
fn sse_envelope_with_attachments() {
    let json = r#"{
            "envelope": {
                "sourceNumber": "+1111111111",
                "dataMessage": {
                    "message": "See attached",
                    "attachments": [
                        {"contentType": "image/jpeg", "filename": "photo.jpg"},
                        {"contentType": "application/pdf"}
                    ]
                }
            }
        }"#;
    let sse: SseEnvelope = serde_json::from_str(json).unwrap();
    let dm = sse.envelope.unwrap().data_message.unwrap();
    let attachments = dm.attachments.unwrap();
    assert_eq!(attachments.len(), 2);
}

// ── is_e164 tests ───────────────────────────────────────────────

#[test]
fn is_e164_valid_numbers() {
    assert!(SignalChannel::is_e164("+12345678901"));
    assert!(SignalChannel::is_e164("+1234567")); // min 7 digits after +
    assert!(SignalChannel::is_e164("+123456789012345")); // max 15 digits
}

#[test]
fn is_e164_invalid_numbers() {
    assert!(!SignalChannel::is_e164("12345678901")); // no +
    assert!(!SignalChannel::is_e164("+1")); // too short (1 digit)
    assert!(!SignalChannel::is_e164("+1234567890123456")); // too long (16 digits)
    assert!(!SignalChannel::is_e164("+abc123")); // non-digit
    assert!(!SignalChannel::is_e164("")); // empty
    assert!(!SignalChannel::is_e164("+")); // plus only
}

// ── config edge cases ───────────────────────────────────────────

#[test]
fn multiple_allow_from() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.allow_from = vec![
        "+1111111111".to_string(),
        "+2222222222".to_string(),
        "a1b2c3d4-e5f6-7890-abcd-ef1234567890".to_string(),
    ];
    let ch = SignalChannel::new(config)?;
    assert!(ch.is_sender_allowed("+1111111111"));
    assert!(ch.is_sender_allowed("+2222222222"));
    assert!(ch.is_sender_allowed("a1b2c3d4-e5f6-7890-abcd-ef1234567890"));
    assert!(!ch.is_sender_allowed("+9999999999"));
    Ok(())
}

#[test]
fn multiple_allow_from_groups() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.allow_from_groups = vec!["group_a".to_string(), "group_b".to_string()];
    let ch = SignalChannel::new(config)?;
    assert!(ch.is_group_allowed("group_a"));
    assert!(ch.is_group_allowed("group_b"));
    assert!(!ch.is_group_allowed("group_c"));
    Ok(())
}

#[test]
fn uuid_prefix_normalization_in_allowlist() -> Result<(), ChannelError> {
    let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    let mut config = make_config();
    config.allow_from = vec![format!("uuid:{uuid}"), "+1111111111".to_string()];
    let ch = SignalChannel::new(config)?;
    // uuid:-prefixed entry should match bare UUID sender.
    assert!(ch.is_sender_allowed(uuid));
    // Phone numbers still work alongside UUID entries.
    assert!(ch.is_sender_allowed("+1111111111"));
    // Non-matching should fail.
    assert!(!ch.is_sender_allowed("+9999999999"));
    Ok(())
}

// ── stories behavior tests ──────────────────────────────────────

#[test]
fn process_envelope_stories_not_skipped_when_disabled() -> Result<(), ChannelError> {
    // With ignore_stories=false, story messages with a data_message
    // should still be processed.
    let mut config = make_config();
    config.allow_from = vec!["*".to_string()];
    config.ignore_stories = false;
    let ch = SignalChannel::new(config)?;

    let env = Envelope {
        source: Some("+1111111111".to_string()),
        source_number: Some("+1111111111".to_string()),
        source_name: None,
        source_uuid: None,
        data_message: Some(DataMessage {
            message: Some("story with text".to_string()),
            timestamp: Some(1_700_000_000_000),
            group_info: None,
            attachments: None,
        }),
        story_message: Some(serde_json::json!({})),
        timestamp: Some(1_700_000_000_000),
    };
    let result = ch.process_envelope(&env);
    assert!(
        result.is_some(),
        "Stories should not be skipped when ignore_stories=false"
    );
    Ok(())
}

// ── trailing slash variations ───────────────────────────────────

#[test]
fn strips_multiple_trailing_slashes() -> Result<(), ChannelError> {
    let mut config = make_config();
    config.http_url = "http://127.0.0.1:8686///".to_string();
    let ch = SignalChannel::new(config)?;
    assert_eq!(ch.config.http_url, "http://127.0.0.1:8686");
    Ok(())
}

#[test]
fn preserves_url_without_trailing_slash() -> Result<(), ChannelError> {
    let config = make_config();
    let ch = SignalChannel::new(config)?;
    assert_eq!(ch.config.http_url, "http://127.0.0.1:8686");
    Ok(())
}

#[test]
fn formatting_hints_describe_plain_text_only() -> Result<(), ChannelError> {
    let ch = make_channel()?;
    assert_eq!(
        ch.formatting_hints().as_deref(),
        Some(
            "Signal renders plain text only. Do not use markdown formatting. Keep messages concise."
        )
    );
    Ok(())
}

#[test]
fn signal_attachment_dir_prefers_override_env() {
    let _guard = SIGNAL_ATTACHMENTS_ENV_LOCK.lock().unwrap();
    let temp = tempdir().unwrap();

    // SAFETY: Tests serialize access to this process-global env var with a mutex.
    unsafe {
        std::env::set_var("SIGNAL_ATTACHMENTS_DIR", temp.path());
    }

    let resolved = signal_attachment_dir();

    // SAFETY: Protected by the same mutex as the matching set_var above.
    unsafe {
        std::env::remove_var("SIGNAL_ATTACHMENTS_DIR");
    }

    assert_eq!(resolved.as_deref(), Some(temp.path()));
}
