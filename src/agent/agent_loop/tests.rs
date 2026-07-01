use super::truncate_for_preview;
use thinclaw_agent::startup_hooks::{
    heartbeat_gateway_fallback_identity_from_diagnostics,
    heartbeat_routine_owner_from_gateway_defaults, telegram_startup_thread_id,
};

#[test]
fn test_truncate_short_input() {
    assert_eq!(truncate_for_preview("hello", 10), "hello");
}

#[test]
fn test_truncate_empty_input() {
    assert_eq!(truncate_for_preview("", 10), "");
}

#[test]
fn test_truncate_exact_length() {
    assert_eq!(truncate_for_preview("hello", 5), "hello");
}

#[test]
fn test_truncate_over_limit() {
    let result = truncate_for_preview("hello world, this is long", 10);
    assert!(result.ends_with("..."));
    // "hello worl" = 10 chars + "..."
    assert_eq!(result, "hello worl...");
}

#[test]
fn test_truncate_collapses_newlines() {
    let result = truncate_for_preview("line1\nline2\nline3", 100);
    assert!(!result.contains('\n'));
    assert_eq!(result, "line1 line2 line3");
}

#[test]
fn test_truncate_collapses_whitespace() {
    let result = truncate_for_preview("hello   world", 100);
    assert_eq!(result, "hello world");
}

#[test]
fn test_truncate_multibyte_utf8() {
    // Each emoji is 4 bytes. Truncating at char boundary must not panic.
    let input = "😀😁😂🤣😃😄😅😆😉😊";
    let result = truncate_for_preview(input, 5);
    assert!(result.ends_with("..."));
    // First 5 chars = 5 emoji
    assert_eq!(result, "😀😁😂🤣😃...");
}

#[test]
fn test_truncate_cjk_characters() {
    // CJK chars are 3 bytes each in UTF-8.
    let input = "你好世界测试数据很长的字符串";
    let result = truncate_for_preview(input, 4);
    assert_eq!(result, "你好世界...");
}

#[test]
fn test_truncate_mixed_multibyte_and_ascii() {
    let input = "hello 世界 foo";
    let result = truncate_for_preview(input, 8);
    // 'h','e','l','l','o',' ','世','界' = 8 chars
    assert_eq!(result, "hello 世界...");
}

#[test]
fn test_telegram_startup_thread_id_routes_first_run_boots_to_onboarding() {
    assert_eq!(
        telegram_startup_thread_id("boot", "telegram", true),
        Some("bootstrap")
    );
    assert_eq!(
        telegram_startup_thread_id("bootstrap", "telegram", true),
        Some("bootstrap")
    );
    assert_eq!(
        telegram_startup_thread_id("boot", "telegram", false),
        Some("boot")
    );
    assert_eq!(telegram_startup_thread_id("bootstrap", "web", true), None);
}

#[test]
fn test_heartbeat_gateway_fallback_identity_prefers_gateway_identity() {
    let diagnostics = serde_json::json!({
        "user_id": "household-user",
        "actor_id": "desk-actor",
    });

    let (user_id, actor_id) =
        heartbeat_gateway_fallback_identity_from_diagnostics(Some(&diagnostics), "fallback-user");

    assert_eq!(user_id, "household-user");
    assert_eq!(actor_id, "desk-actor");
}

#[test]
fn test_heartbeat_gateway_fallback_identity_falls_back_to_workspace_user() {
    let diagnostics = serde_json::json!({
        "user_id": "",
        "actor_id": "",
    });

    let (user_id, actor_id) =
        heartbeat_gateway_fallback_identity_from_diagnostics(Some(&diagnostics), "fallback-user");

    assert_eq!(user_id, "fallback-user");
    assert_eq!(actor_id, "fallback-user");
}

#[test]
fn test_heartbeat_routine_owner_uses_inferred_gateway_principal_when_default() {
    let (user_id, actor_id) =
        heartbeat_routine_owner_from_gateway_defaults("default", "default", Some("684480568"));

    assert_eq!(user_id, "684480568");
    assert_eq!(actor_id, "684480568");
}

#[test]
fn test_heartbeat_routine_owner_preserves_distinct_gateway_actor() {
    let (user_id, actor_id) = heartbeat_routine_owner_from_gateway_defaults(
        "default",
        "desk-actor",
        Some("household-user"),
    );

    assert_eq!(user_id, "household-user");
    assert_eq!(actor_id, "desk-actor");
}
