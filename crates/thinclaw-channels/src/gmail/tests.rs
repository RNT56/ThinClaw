use super::*;

fn test_config() -> GmailConfig {
    GmailConfig {
        enabled: true,
        project_id: "test-project".into(),
        subscription_id: "test-sub".into(),
        topic_id: "test-topic".into(),
        oauth_token: Some("test-token".into()),
        ..Default::default()
    }
}

#[test]
fn test_new_requires_configuration() {
    let config = GmailConfig::default();
    let result = GmailChannel::new(config);
    assert!(result.is_err());
}

#[test]
fn can_refresh_token_requires_refresh_token_and_client_id() {
    let base = test_config();
    assert!(!base.can_refresh_token(), "no refresh creds yet");

    let full = GmailConfig {
        refresh_token: Some("r".into()),
        client_id: Some("c".into()),
        client_secret: Some("s".into()),
        ..test_config()
    };
    assert!(full.can_refresh_token());
    assert!(
        GmailConfig {
            client_secret: None,
            ..full.clone()
        }
        .can_refresh_token()
    );

    // Refresh token and client id are mandatory; PKCE public clients do
    // not necessarily have a client secret.
    for partial in [
        GmailConfig {
            refresh_token: None,
            ..full.clone()
        },
        GmailConfig {
            client_id: Some(String::new()),
            ..full.clone()
        },
    ] {
        assert!(!partial.can_refresh_token());
    }
}

#[test]
fn is_auth_error_detects_expired_token_signals() {
    assert!(GmailChannel::is_auth_error(&ChannelError::AuthFailed {
        name: "gmail".into(),
        reason: "nope".into(),
    }));
    assert!(GmailChannel::is_auth_error(&ChannelError::SendFailed {
        name: "gmail".into(),
        reason: "HTTP 401 Unauthorized: invalid_token".into(),
    }));
    assert!(!GmailChannel::is_auth_error(&ChannelError::SendFailed {
        name: "gmail".into(),
        reason: "HTTP 500 Internal Server Error".into(),
    }));
}

#[test]
fn token_refresh_response_defaults_expiry() {
    let parsed: GmailTokenRefreshResponse =
        serde_json::from_str(r#"{"access_token":"abc"}"#).unwrap();
    assert_eq!(parsed.access_token, "abc");
    assert_eq!(parsed.expires_in, 3600);

    let parsed: GmailTokenRefreshResponse =
        serde_json::from_str(r#"{"access_token":"abc","expires_in":1799}"#).unwrap();
    assert_eq!(parsed.expires_in, 1799);
}

#[test]
fn test_new_succeeds_with_valid_config() {
    let config = test_config();
    let channel = GmailChannel::new(config).unwrap();
    assert_eq!(channel.name(), "gmail");
}

#[test]
fn test_extract_sender_standard() {
    let msg = GmailMessage {
        id: "1".into(),
        thread_id: None,
        snippet: None,
        label_ids: None,
        internal_date: None,
        payload: Some(GmailPayload {
            headers: Some(vec![GmailHeader {
                name: "From".into(),
                value: "Alice <alice@example.com>".into(),
            }]),
            body: None,
            parts: None,
            mime_type: None,
        }),
    };
    assert_eq!(
        GmailChannel::extract_sender(&msg),
        Some("alice@example.com".into())
    );
}

#[test]
fn test_extract_sender_bare_email() {
    let msg = GmailMessage {
        id: "2".into(),
        thread_id: None,
        snippet: None,
        label_ids: None,
        internal_date: None,
        payload: Some(GmailPayload {
            headers: Some(vec![GmailHeader {
                name: "From".into(),
                value: "bob@example.com".into(),
            }]),
            body: None,
            parts: None,
            mime_type: None,
        }),
    };
    assert_eq!(
        GmailChannel::extract_sender(&msg),
        Some("bob@example.com".into())
    );
}

#[test]
fn test_extract_sender_malformed_angle_brackets() {
    let msg = GmailMessage {
        id: "2a".into(),
        thread_id: None,
        snippet: None,
        label_ids: None,
        internal_date: None,
        payload: Some(GmailPayload {
            headers: Some(vec![GmailHeader {
                name: "From".into(),
                value: "Bob <bob@example.com".into(),
            }]),
            body: None,
            parts: None,
            mime_type: None,
        }),
    };
    assert_eq!(
        GmailChannel::extract_sender(&msg),
        Some("Bob <bob@example.com".into())
    );
}

#[test]
fn test_extract_sender_display_name_with_gt_before_lt_does_not_panic() {
    // RFC-legal quoted display name containing '>' before the real '<'.
    // The old first-`<`/first-`>` slice panicked with `begin > end`.
    let msg = GmailMessage {
        id: "2b".into(),
        thread_id: None,
        snippet: None,
        label_ids: None,
        internal_date: None,
        payload: Some(GmailPayload {
            headers: Some(vec![GmailHeader {
                name: "From".into(),
                value: "\"Doe > John\" <evil@example.com>".into(),
            }]),
            body: None,
            parts: None,
            mime_type: None,
        }),
    };
    assert_eq!(
        GmailChannel::extract_sender(&msg),
        Some("evil@example.com".into())
    );
}

#[test]
fn test_to_incoming_message_skips_sent_label() {
    // A message this account authored (SENT label) must never re-enter the
    // agent loop, even from an allowed sender — otherwise the agent answers
    // its own replies forever.
    let channel = GmailChannel::new(GmailConfig {
        allowed_senders: vec![],
        ..test_config()
    })
    .unwrap();
    let msg = GmailMessage {
        id: "sent-1".into(),
        thread_id: Some("t1".into()),
        snippet: Some("a reply the agent just sent".into()),
        label_ids: Some(vec!["SENT".into()]),
        internal_date: None,
        payload: Some(GmailPayload {
            headers: Some(vec![GmailHeader {
                name: "From".into(),
                value: "agent@example.com".into(),
            }]),
            body: None,
            parts: None,
            mime_type: None,
        }),
    };
    assert!(channel.to_incoming_message(&msg).is_none());
}

#[test]
fn test_send_reply_headers_are_crlf_sanitized() {
    // Header injection: a recipient/subject carrying CR/LF must not be able
    // to smuggle an extra header line.
    assert_eq!(
        sanitize_header_value("a@b.com\r\nBcc: victim@x.com"),
        "a@b.com__Bcc: victim@x.com"
    );
    assert_eq!(sanitize_header_value("plain subject"), "plain subject");
}

#[test]
fn test_extract_subject() {
    let msg = GmailMessage {
        id: "3".into(),
        thread_id: None,
        snippet: None,
        label_ids: None,
        internal_date: None,
        payload: Some(GmailPayload {
            headers: Some(vec![
                GmailHeader {
                    name: "From".into(),
                    value: "x@y.com".into(),
                },
                GmailHeader {
                    name: "Subject".into(),
                    value: "Hello World".into(),
                },
            ]),
            body: None,
            parts: None,
            mime_type: None,
        }),
    };
    assert_eq!(
        GmailChannel::extract_subject(&msg),
        Some("Hello World".into())
    );
}

#[test]
fn test_decode_base64url() {
    let encoded = "SGVsbG8gV29ybGQ"; // "Hello World" in base64url
    let decoded = GmailChannel::decode_base64url(encoded);
    assert_eq!(decoded, Some("Hello World".into()));
}

#[test]
fn test_decode_base64url_invalid() {
    assert_eq!(GmailChannel::decode_base64url("???"), None);
}

#[test]
fn test_extract_body_from_snippet() {
    let msg = GmailMessage {
        id: "4".into(),
        thread_id: None,
        snippet: Some("This is a test snippet".into()),
        label_ids: None,
        internal_date: None,
        payload: None,
    };
    assert_eq!(GmailChannel::extract_body(&msg), "This is a test snippet");
}

#[test]
fn test_extract_body_from_plain_payload() {
    let msg = GmailMessage {
        id: "5".into(),
        thread_id: None,
        snippet: Some("snippet".into()),
        label_ids: None,
        internal_date: None,
        payload: Some(GmailPayload {
            headers: None,
            body: Some(GmailBody {
                data: Some("SGVsbG8gZnJvbSBlbWFpbA".into()), // "Hello from email"
                size: None,
            }),
            parts: None,
            mime_type: Some("text/plain".into()),
        }),
    };
    assert_eq!(GmailChannel::extract_body(&msg), "Hello from email");
}

#[test]
fn test_to_incoming_message_filters_unauthorized() {
    let config = GmailConfig {
        allowed_senders: vec!["allowed@example.com".into()],
        ..test_config()
    };
    let channel = GmailChannel::new(config).unwrap();

    let msg = GmailMessage {
        id: "6".into(),
        thread_id: Some("thread-1".into()),
        snippet: Some("Hello".into()),
        label_ids: None,
        internal_date: None,
        payload: Some(GmailPayload {
            headers: Some(vec![GmailHeader {
                name: "From".into(),
                value: "stranger@evil.com".into(),
            }]),
            body: None,
            parts: None,
            mime_type: None,
        }),
    };

    assert!(channel.to_incoming_message(&msg).is_none());
}

#[test]
fn test_to_incoming_message_accepts_allowed() {
    let config = GmailConfig {
        allowed_senders: vec!["allowed@example.com".into()],
        ..test_config()
    };
    let channel = GmailChannel::new(config).unwrap();

    let msg = GmailMessage {
        id: "7".into(),
        thread_id: Some("thread-2".into()),
        snippet: Some("Hello from allowed".into()),
        label_ids: None,
        internal_date: None,
        payload: Some(GmailPayload {
            headers: Some(vec![GmailHeader {
                name: "From".into(),
                value: "allowed@example.com".into(),
            }]),
            body: None,
            parts: None,
            mime_type: None,
        }),
    };

    let incoming = channel.to_incoming_message(&msg).unwrap();
    assert_eq!(incoming.channel, "gmail");
    assert_eq!(incoming.user_id, "allowed@example.com");
    assert_eq!(incoming.content, "Hello from allowed");
    assert_eq!(incoming.thread_id, Some("thread-2".into()));
}

#[test]
fn test_to_incoming_message_empty_allowlist_accepts_all() {
    let config = test_config(); // empty allowed_senders
    let channel = GmailChannel::new(config).unwrap();

    let msg = GmailMessage {
        id: "8".into(),
        thread_id: None,
        snippet: Some("Hi there".into()),
        label_ids: None,
        internal_date: None,
        payload: Some(GmailPayload {
            headers: Some(vec![GmailHeader {
                name: "From".into(),
                value: "anyone@anywhere.com".into(),
            }]),
            body: None,
            parts: None,
            mime_type: None,
        }),
    };

    assert!(channel.to_incoming_message(&msg).is_some());
}

#[test]
fn test_to_incoming_message_skips_empty_body() {
    let config = test_config();
    let channel = GmailChannel::new(config).unwrap();

    let msg = GmailMessage {
        id: "9".into(),
        thread_id: None,
        snippet: Some("   ".into()), // whitespace only
        label_ids: None,
        internal_date: None,
        payload: Some(GmailPayload {
            headers: Some(vec![GmailHeader {
                name: "From".into(),
                value: "user@example.com".into(),
            }]),
            body: None,
            parts: None,
            mime_type: None,
        }),
    };

    assert!(channel.to_incoming_message(&msg).is_none());
}

#[test]
fn test_messages_processed_counter() {
    let config = test_config();
    let channel = GmailChannel::new(config).unwrap();
    assert_eq!(channel.messages_processed(), 0);
}

#[tokio::test]
async fn test_start_fails_without_token() {
    let config = GmailConfig {
        enabled: true,
        project_id: "p".into(),
        subscription_id: "s".into(),
        topic_id: "t".into(),
        oauth_token: None,
        ..Default::default()
    };
    let channel = GmailChannel::new(config).unwrap();
    let result = channel.start().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_shutdown() {
    let config = test_config();
    let channel = GmailChannel::new(config).unwrap();
    let result = channel.shutdown().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_health_check_not_started() {
    let config = test_config();
    let channel = GmailChannel::new(config).unwrap();
    let result = channel.health_check().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_set_access_token() {
    let config = test_config();
    let channel = GmailChannel::new(config).unwrap();
    channel.set_access_token("new-token").await.unwrap();
    let token = channel.state.access_token.read().await.clone();
    assert_eq!(token, "new-token");
}

#[test]
fn test_extract_body_from_multipart() {
    let msg = GmailMessage {
        id: "10".into(),
        thread_id: None,
        snippet: Some("snippet".into()),
        label_ids: None,
        internal_date: None,
        payload: Some(GmailPayload {
            headers: None,
            body: None,
            parts: Some(vec![
                GmailPart {
                    mime_type: Some("text/html".into()),
                    body: Some(GmailBody {
                        data: Some("PFBA".into()),
                        size: None,
                    }),
                    parts: None,
                },
                GmailPart {
                    mime_type: Some("text/plain".into()),
                    body: Some(GmailBody {
                        data: Some("UGxhaW4gdGV4dA".into()), // "Plain text"
                        size: None,
                    }),
                    parts: None,
                },
            ]),
            mime_type: Some("multipart/alternative".into()),
        }),
    };
    assert_eq!(GmailChannel::extract_body(&msg), "Plain text");
}

#[test]
fn test_extract_message_id_header() {
    let msg = GmailMessage {
        id: "11".into(),
        thread_id: None,
        snippet: Some("snippet".into()),
        label_ids: None,
        internal_date: None,
        payload: Some(GmailPayload {
            headers: Some(vec![
                GmailHeader {
                    name: "From".into(),
                    value: "alice@example.com".into(),
                },
                GmailHeader {
                    name: "Message-ID".into(),
                    value: "<id-1234@example.com>".into(),
                },
            ]),
            body: None,
            parts: None,
            mime_type: None,
        }),
    };
    assert_eq!(
        GmailChannel::extract_message_id_header(&msg),
        Some("<id-1234@example.com>".into())
    );
}

#[test]
fn test_extract_message_id_header_missing() {
    let msg = GmailMessage {
        id: "12".into(),
        thread_id: None,
        snippet: Some("snippet".into()),
        label_ids: None,
        internal_date: None,
        payload: Some(GmailPayload {
            headers: Some(vec![GmailHeader {
                name: "From".into(),
                value: "alice@example.com".into(),
            }]),
            body: None,
            parts: None,
            mime_type: None,
        }),
    };
    assert_eq!(GmailChannel::extract_message_id_header(&msg), None);
}

#[test]
fn test_extract_text_from_payload_nested_part() {
    let payload = GmailPayload {
        headers: None,
        body: None,
        parts: Some(vec![GmailPart {
            mime_type: Some("multipart/related".into()),
            body: None,
            parts: Some(vec![GmailPart {
                mime_type: Some("text/plain".into()),
                body: Some(GmailBody {
                    data: Some("UGxhaW4gbmVzdGVkIHRleHQ".into()), // "Plain nested text"
                    size: None,
                }),
                parts: None,
            }]),
        }]),
        mime_type: Some("multipart/mixed".into()),
    };
    let msg = GmailMessage {
        id: "13".into(),
        thread_id: None,
        snippet: Some("snippet".into()),
        label_ids: None,
        internal_date: None,
        payload: Some(payload),
    };
    assert_eq!(GmailChannel::extract_body(&msg), "Plain nested text");
}
