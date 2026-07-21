use super::*;
use axum::{body::Body, http::Request};
use futures::StreamExt;
use secrecy::SecretString;
use std::time::Duration;
use tokio::time::timeout;
use tower::ServiceExt;

#[test]
fn seen_guids_dedupes_and_evicts() {
    let mut seen = SeenGuids::default();
    assert!(seen.insert_new("a"), "first sighting is new");
    assert!(!seen.insert_new("a"), "repeat is a duplicate");
    assert!(seen.insert_new("b"), "different guid is new");
    // Overflow the ring so "a" is eventually evicted and seen fresh again.
    for i in 0..SeenGuids::CAP {
        seen.insert_new(&format!("fill-{i}"));
    }
    assert!(seen.insert_new("a"), "evicted guid is treated as new again");
}

#[test]
fn test_normalize_server_url() {
    assert_eq!(
        normalize_server_url("192.168.1.50:1234"),
        "http://192.168.1.50:1234"
    );
    assert_eq!(
        normalize_server_url("http://192.168.1.50:1234/"),
        "http://192.168.1.50:1234"
    );
    assert_eq!(
        normalize_server_url("https://my.tunnel.dev"),
        "https://my.tunnel.dev"
    );
    assert_eq!(normalize_server_url(""), "");
    assert_eq!(normalize_server_url("  "), "");
}

#[test]
fn test_extract_record_from_data_object() {
    let payload = serde_json::json!({
        "type": "new-message",
        "data": { "text": "hello", "isFromMe": false }
    });
    let record = extract_record(&payload).unwrap();
    assert_eq!(record.get("text").unwrap().as_str(), Some("hello"));
}

#[test]
fn test_extract_record_from_data_array() {
    let payload = serde_json::json!({
        "type": "new-message",
        "data": [{ "text": "hello" }]
    });
    let record = extract_record(&payload).unwrap();
    assert_eq!(record.get("text").unwrap().as_str(), Some("hello"));
}

#[test]
fn test_first_string() {
    let a = serde_json::json!("");
    let b = serde_json::json!("hello");
    assert_eq!(first_string(&[Some(&a), Some(&b)]), Some("hello".into()));
    assert_eq!(first_string(&[None, None]), None);
}

#[test]
fn test_split_message_short() {
    let chunks = split_message("Hello!", 4000);
    assert_eq!(chunks, vec!["Hello!"]);
}

#[test]
fn test_split_message_long() {
    let text = "a".repeat(5000);
    let chunks = split_message(&text, 4000);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].len(), 4000);
}

#[tokio::test]
async fn test_webhook_routes_forwards_allowed_inbound_message() {
    let config = BlueBubblesConfig {
        server_url: "http://127.0.0.1:1".to_string(),
        password: SecretString::from("bluebubbles-pass"),
        webhook_host: "127.0.0.1".to_string(),
        webhook_port: 8645,
        webhook_path: "/bluebubbles-webhook".to_string(),
        allow_from: vec!["alice@example.com".to_string()],
        send_read_receipts: false,
    };

    let channel = BlueBubblesChannel::new(config)
        .await
        .expect("channel should initialize");
    let mut stream = channel.start().await.expect("stream should start");

    let app = channel.webhook_routes();
    let payload = serde_json::json!({
        "type": "new-message",
        "text": "hello from iMessage",
        "chatGuid": "chat-1",
        "isFromMe": false,
        "chatIdentifier": "chat-1",
        "handle": { "address": "alice@example.com" },
        "isGroup": false,
    });
    let request = Request::builder()
        .method("POST")
        .uri("/bluebubbles-webhook?password=bluebubbles-pass")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&payload).expect("payload should serialize"),
        ))
        .expect("request should build");

    let response = app
        .oneshot(request)
        .await
        .expect("webhook request should be served");
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    let message = timeout(Duration::from_secs(1), stream.next())
        .await
        .expect("message should arrive")
        .expect("stream should yield one message");
    assert_eq!(message.channel, "bluebubbles");
    assert_eq!(message.user_id, "alice@example.com");
    assert_eq!(message.content, "hello from iMessage");
    assert_eq!(message.metadata["chat_guid"], "chat-1");
    assert_eq!(message.metadata["conversation_kind"], "direct");
    assert_eq!(message.metadata["attachment_count"], 0);
}

#[tokio::test]
async fn test_webhook_routes_rejects_unlisted_sender() {
    let config = BlueBubblesConfig {
        server_url: "http://127.0.0.1:1".to_string(),
        password: SecretString::from("bluebubbles-pass"),
        webhook_host: "127.0.0.1".to_string(),
        webhook_port: 8645,
        webhook_path: "/bluebubbles-webhook".to_string(),
        allow_from: vec!["trusted@example.com".to_string()],
        send_read_receipts: false,
    };

    let channel = BlueBubblesChannel::new(config)
        .await
        .expect("channel should initialize");
    let mut stream = channel.start().await.expect("stream should start");
    let app = channel.webhook_routes();

    let payload = serde_json::json!({
        "type": "new-message",
        "text": "ignore this",
        "chatGuid": "chat-2",
        "isFromMe": false,
        "handle": { "address": "unknown@example.com" },
    });
    let request = Request::builder()
        .method("POST")
        .uri("/bluebubbles-webhook?guid=bluebubbles-pass")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&payload).expect("payload should serialize"),
        ))
        .expect("request should build");

    let response = app
        .oneshot(request)
        .await
        .expect("webhook request should be served");
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    assert!(
        timeout(Duration::from_millis(200), stream.next())
            .await
            .is_err(),
        "unlisted sender should not emit a message"
    );
}

#[test]
fn test_redact_url() {
    assert_eq!(
        redact_url("http://192.168.1.50:1234/api/v1/ping"),
        "http://192.168.1.50:1234/***"
    );
    assert_eq!(redact_url("https://my.tunnel.dev"), "https://my.tunnel.dev");
}

#[test]
fn test_redact_pii() {
    assert_eq!(redact_pii("+4917612345678"), "[REDACTED]");
    assert_eq!(redact_pii("user@icloud.com"), "[REDACTED]");
    assert_eq!(redact_pii("hello"), "hello");
}

#[test]
fn test_message_events_filter() {
    assert!(MESSAGE_EVENTS.contains(&"new-message"));
    assert!(MESSAGE_EVENTS.contains(&"updated-message"));
    assert!(!MESSAGE_EVENTS.contains(&"typing-indicator"));
}

#[test]
fn test_tapback_codes_range() {
    // Added reactions: 2000-2005
    for code in 2000..=2005 {
        assert!(
            TAPBACK_CODES.contains(&code),
            "tapback code {code} should be present"
        );
    }
    // Removed reactions: 3000-3005
    for code in 3000..=3005 {
        assert!(
            TAPBACK_CODES.contains(&code),
            "tapback code {code} should be present"
        );
    }
    // Non-tapback codes should not match
    assert!(!TAPBACK_CODES.contains(&0));
    assert!(!TAPBACK_CODES.contains(&1999));
    assert!(!TAPBACK_CODES.contains(&2006));
}

#[tokio::test]
async fn test_webhook_filters_tapback_reactions() {
    let config = BlueBubblesConfig {
        server_url: "http://127.0.0.1:1".to_string(),
        password: SecretString::from("bluebubbles-pass"),
        webhook_host: "127.0.0.1".to_string(),
        webhook_port: 8645,
        webhook_path: "/bluebubbles-webhook".to_string(),
        allow_from: vec![],
        send_read_receipts: false,
    };

    let channel = BlueBubblesChannel::new(config)
        .await
        .expect("channel should initialize");
    let mut stream = channel.start().await.expect("stream should start");
    let app = channel.webhook_routes();

    // Send a tapback reaction (love = 2000)
    let payload = serde_json::json!({
        "type": "new-message",
        "data": {
            "text": "",
            "chatGuid": "chat-1",
            "isFromMe": false,
            "associatedMessageType": 2000,
            "handle": { "address": "alice@example.com" },
        }
    });
    let request = Request::builder()
        .method("POST")
        .uri("/bluebubbles-webhook?password=bluebubbles-pass")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).expect("serialize")))
        .expect("build request");

    let response = app.oneshot(request).await.expect("serve");
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    // No message should arrive — tapbacks are filtered
    assert!(
        timeout(Duration::from_millis(200), stream.next())
            .await
            .is_err(),
        "tapback reaction should not produce a message"
    );
}

#[tokio::test]
async fn test_webhook_form_encoded_fallback() {
    let config = BlueBubblesConfig {
        server_url: "http://127.0.0.1:1".to_string(),
        password: SecretString::from("bluebubbles-pass"),
        webhook_host: "127.0.0.1".to_string(),
        webhook_port: 8645,
        webhook_path: "/bluebubbles-webhook".to_string(),
        allow_from: vec![],
        send_read_receipts: false,
    };

    let channel = BlueBubblesChannel::new(config)
        .await
        .expect("channel should initialize");
    let mut stream = channel.start().await.expect("stream should start");
    let app = channel.webhook_routes();

    // Send form-encoded body: payload=<url-encoded JSON>
    let json_str = serde_json::json!({
        "type": "new-message",
        "data": {
            "text": "form-encoded hello",
            "chatGuid": "chat-form",
            "isFromMe": false,
            "handle": { "address": "bob@example.com" },
        }
    })
    .to_string();
    let form_body = format!("payload={}", urlencoding::encode(&json_str));

    let request = Request::builder()
        .method("POST")
        .uri("/bluebubbles-webhook?password=bluebubbles-pass")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(form_body))
        .expect("build request");

    let response = app.oneshot(request).await.expect("serve");
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    let message = timeout(Duration::from_secs(1), stream.next())
        .await
        .expect("message should arrive")
        .expect("stream should yield a message");
    assert_eq!(message.channel, "bluebubbles");
    assert_eq!(message.content, "form-encoded hello");
    assert_eq!(message.user_id, "bob@example.com");
}

#[test]
fn test_base64_decode_valid() {
    let result = base64_decode("SGVsbG8=").expect("should decode");
    assert_eq!(result, b"Hello");
}

#[test]
fn test_base64_decode_invalid() {
    assert!(base64_decode("!!!invalid!!!").is_err());
}

#[tokio::test]
async fn test_webhook_includes_reply_to_metadata() {
    let config = BlueBubblesConfig {
        server_url: "http://127.0.0.1:1".to_string(),
        password: SecretString::from("pass123"),
        webhook_host: "127.0.0.1".to_string(),
        webhook_port: 8645,
        webhook_path: "/bluebubbles-webhook".to_string(),
        allow_from: vec![],
        send_read_receipts: false,
    };

    let channel = BlueBubblesChannel::new(config)
        .await
        .expect("channel should initialize");
    let mut stream = channel.start().await.expect("stream should start");
    let app = channel.webhook_routes();

    let payload = serde_json::json!({
        "type": "new-message",
        "data": {
            "text": "replying to something",
            "chatGuid": "chat-reply",
            "isFromMe": false,
            "handle": { "address": "carol@example.com" },
            "threadOriginatorGuid": "original-msg-guid-123",
            "guid": "this-msg-guid-456",
        }
    });
    let request = Request::builder()
        .method("POST")
        .uri("/bluebubbles-webhook?password=pass123")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).expect("serialize")))
        .expect("build request");

    let response = app.oneshot(request).await.expect("serve");
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    let message = timeout(Duration::from_secs(1), stream.next())
        .await
        .expect("message should arrive")
        .expect("stream should yield");
    assert_eq!(message.metadata["message_id"], "this-msg-guid-456");
    assert_eq!(
        message.metadata["reply_to_message_id"],
        "original-msg-guid-123"
    );
}
