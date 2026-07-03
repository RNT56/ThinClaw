use super::*;

fn blank_message(message_type: &str) -> WhatsAppMessage {
    WhatsAppMessage {
        id: "wamid.test".to_string(),
        from: "15551234567".to_string(),
        timestamp: "1234567890".to_string(),
        message_type: message_type.to_string(),
        text: None,
        image: None,
        audio: None,
        video: None,
        document: None,
        sticker: None,
        location: None,
        contacts: None,
        interactive: None,
        reaction: None,
        context: None,
    }
}

#[test]
fn test_parse_webhook_payload() {
    let json = r#"{
        "object": "whatsapp_business_account",
        "entry": [{
            "id": "123456789",
            "changes": [{
                "field": "messages",
                "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                        "display_phone_number": "+1234567890",
                        "phone_number_id": "987654321"
                    },
                    "contacts": [{
                        "wa_id": "15551234567",
                        "profile": {
                            "name": "John Doe"
                        }
                    }],
                    "messages": [{
                        "id": "wamid.abc123",
                        "from": "15551234567",
                        "timestamp": "1234567890",
                        "type": "text",
                        "text": {
                            "body": "Hello!"
                        }
                    }]
                }
            }]
        }]
    }"#;

    let payload: WebhookPayload = serde_json::from_str(json).unwrap();
    assert_eq!(payload.object, "whatsapp_business_account");
    assert_eq!(payload.entry.len(), 1);

    let change = &payload.entry[0].changes[0];
    assert_eq!(change.field, "messages");
    assert_eq!(change.value.metadata.phone_number_id, "987654321");

    let message = &change.value.messages[0];
    assert_eq!(message.from, "15551234567");
    assert_eq!(message.text.as_ref().unwrap().body, "Hello!");
}

#[test]
fn test_parse_status_update() {
    let json = r#"{
        "object": "whatsapp_business_account",
        "entry": [{
            "id": "123456789",
            "changes": [{
                "field": "messages",
                "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                        "display_phone_number": "+1234567890",
                        "phone_number_id": "987654321"
                    },
                    "statuses": [{
                        "id": "wamid.abc123",
                        "status": "delivered",
                        "timestamp": "1234567890",
                        "recipient_id": "15551234567"
                    }]
                }
            }]
        }]
    }"#;

    let payload: WebhookPayload = serde_json::from_str(json).unwrap();
    let value = &payload.entry[0].changes[0].value;

    // Should have status but no messages
    assert!(value.messages.is_empty());
    assert_eq!(value.statuses.len(), 1);
    assert_eq!(value.statuses[0].status, "delivered");
}

#[test]
fn test_metadata_roundtrip() {
    let metadata = WhatsAppMessageMetadata {
        phone_number_id: "123456".to_string(),
        sender_phone: "15551234567".to_string(),
        message_id: Some("wamid.abc".to_string()),
        reply_to_message_id: Some("wamid.abc".to_string()),
        timestamp: Some("1234567890".to_string()),
        conversation_kind: Some("direct".to_string()),
        conversation_scope_id: Some("whatsapp:direct:123456:15551234567".to_string()),
        external_conversation_key: Some("whatsapp://direct/123456/15551234567".to_string()),
        raw_sender_id: Some("15551234567".to_string()),
        stable_sender_id: Some("15551234567".to_string()),
        inbound_message_type: Some("text".to_string()),
        context_message_id: None,
        event_details: None,
        response_attachments: Vec::new(),
    };

    let json = serde_json::to_string(&metadata).unwrap();
    let parsed: WhatsAppMessageMetadata = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.phone_number_id, "123456");
    assert_eq!(parsed.sender_phone, "15551234567");
    assert_eq!(parsed.conversation_kind.as_deref(), Some("direct"));
}

#[test]
fn test_split_message_short() {
    let chunks = split_message("hello", WHATSAPP_MAX_MESSAGE_LENGTH);
    assert_eq!(chunks, vec!["hello"]);
}

#[test]
fn test_split_message_prefers_newline_boundaries() {
    let text = format!("{}\n{}", "a".repeat(2500), "b".repeat(2500));
    let chunks = split_message(&text, WHATSAPP_MAX_MESSAGE_LENGTH);

    assert_eq!(chunks.len(), 2);
    assert!(chunks[0].len() <= WHATSAPP_MAX_MESSAGE_LENGTH);
    assert!(chunks[1].len() <= WHATSAPP_MAX_MESSAGE_LENGTH);
    assert!(chunks[0].starts_with('a'));
    assert!(chunks[1].starts_with('b'));
}

#[test]
fn test_split_message_is_unicode_safe() {
    let text = "🙂".repeat(5000);
    let chunks = split_message(&text, WHATSAPP_MAX_MESSAGE_LENGTH);

    assert_eq!(chunks.len(), 2);
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.chars().count())
            .sum::<usize>(),
        5000
    );
}

#[test]
fn test_normalize_location_message() {
    let mut message = blank_message("location");
    message.location = Some(WhatsAppLocation {
        latitude: 52.52,
        longitude: 13.405,
        name: Some("Berlin".to_string()),
        address: Some("Alexanderplatz".to_string()),
        url: None,
    });

    let (summary, details) = normalized_message_content(&message);
    assert_eq!(summary, "Shared location: Berlin (Alexanderplatz)");
    assert_eq!(details.unwrap()["type"], "location");
}

#[test]
fn test_normalize_contacts_message() {
    let mut message = blank_message("contacts");
    message.contacts = Some(vec![WhatsAppContactCard {
        name: Some(WhatsAppContactCardName {
            formatted_name: Some("Ada Lovelace".to_string()),
            first_name: None,
            last_name: None,
        }),
        phones: vec![],
        emails: vec![],
        org: None,
    }]);

    let (summary, details) = normalized_message_content(&message);
    assert_eq!(summary, "Shared contact cards: Ada Lovelace");
    assert_eq!(details.unwrap()["type"], "contacts");
}

#[test]
fn test_normalize_interactive_reply() {
    let mut message = blank_message("interactive");
    message.interactive = Some(WhatsAppInteractive {
        interactive_type: "button_reply".to_string(),
        button_reply: Some(WhatsAppInteractiveButtonReply {
            id: Some("yes".to_string()),
            title: Some("Yes".to_string()),
        }),
        list_reply: None,
        nfm_reply: None,
    });

    let (summary, details) = normalized_message_content(&message);
    assert_eq!(summary, "Selected button reply: Yes");
    assert_eq!(details.unwrap()["type"], "interactive");
}

#[test]
fn test_normalize_reaction_message() {
    let mut message = blank_message("reaction");
    message.reaction = Some(WhatsAppReaction {
        message_id: Some("wamid.original".to_string()),
        emoji: Some("👍".to_string()),
    });

    let (summary, details) = normalized_message_content(&message);
    assert_eq!(summary, "Reacted with 👍");
    assert_eq!(details.unwrap()["type"], "reaction");
}

#[test]
fn test_media_without_caption_uses_media_placeholder_path() {
    let mut message = blank_message("image");
    message.image = Some(WhatsAppMedia {
        id: "media123".to_string(),
        mime_type: Some("image/jpeg".to_string()),
        caption: None,
    });

    let (summary, details) = normalized_message_content(&message);
    assert!(summary.is_empty());
    assert_eq!(details.unwrap()["type"], "media");
}

#[test]
fn test_unknown_message_type_has_safe_fallback() {
    let message = blank_message("order");
    let (summary, details) = normalized_message_content(&message);

    assert_eq!(summary, "[WhatsApp order message received]");
    assert_eq!(details.unwrap()["type"], "unsupported");
}

#[test]
fn test_outbound_media_kind_mapping() {
    assert_eq!(
        outbound_media_kind_for_mime("image/png"),
        OutboundMediaKind::Image
    );
    assert_eq!(
        outbound_media_kind_for_mime("audio/ogg"),
        OutboundMediaKind::Audio
    );
    assert_eq!(
        outbound_media_kind_for_mime("video/mp4"),
        OutboundMediaKind::Video
    );
    assert_eq!(
        outbound_media_kind_for_mime("image/webp"),
        OutboundMediaKind::Sticker
    );
    assert_eq!(
        outbound_media_kind_for_mime("application/pdf"),
        OutboundMediaKind::Document
    );
}

#[test]
fn test_graph_api_error_classification_template_required() {
    let error = classify_graph_api_error_details(
        "message send",
        400,
        Some(131047),
        "More than 24 hours have passed since the customer last replied to this number.",
        Some("OAuthException"),
    );
    assert!(error.contains("template required"));
}

#[test]
fn test_graph_api_error_classification_auth() {
    let error = classify_graph_api_error_details(
        "message send",
        401,
        Some(190),
        "Invalid OAuth access token.",
        Some("OAuthException"),
    );
    assert!(error.contains("auth or permission"));
}

#[test]
fn test_graph_api_error_classification_media_upload() {
    let error = classify_graph_api_error_details(
        "media upload",
        400,
        Some(131053),
        "Unable to upload the media used in the message",
        None,
    );
    assert!(error.contains("media upload failed"));
}

#[test]
fn test_graph_api_error_classification_invalid_routing() {
    let error = classify_graph_api_error_details(
        "message send",
        400,
        Some(100),
        "Invalid parameter: phone_number_id",
        None,
    );
    assert!(error.contains("invalid recipient or routing metadata"));
}

#[test]
fn test_media_lookup_url_uses_configured_version() {
    assert_eq!(
        media_lookup_url("v23.0", "media-123"),
        "https://graph.facebook.com/v23.0/media-123"
    );
}

#[test]
fn test_metadata_accepts_proactive_route_aliases() {
    let json = serde_json::json!({
        "phone_number_id": "123456",
        "recipient_phone": "15551234567",
        "reply_to_message_id": "wamid.reply",
        "response_attachments": [{
            "mime_type": "image/png",
            "filename": "image.png",
            "data": "aGVsbG8="
        }]
    })
    .to_string();

    let parsed: WhatsAppMessageMetadata = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.sender_phone, "15551234567");
    assert_eq!(parsed.reply_to_message_id.as_deref(), Some("wamid.reply"));
    assert_eq!(parsed.response_attachments.len(), 1);
}
