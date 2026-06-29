//! lib: media.

use super::*;

pub(crate) fn should_ignore_update(update_id: i64) -> bool {
    if channel_host::workspace_read(IGNORE_UPDATES_UNTIL_ID_PATH)
        .and_then(|raw| raw.parse::<i64>().ok())
        .is_some_and(|upper_bound| update_id <= upper_bound)
    {
        return true;
    }

    // Migration/backward-compat: when setup has already persisted
    // `state/last_update_id` (next offset), suppress any webhook-delivered
    // update IDs below that offset.
    channel_host::workspace_read(POLLING_STATE_PATH)
        .and_then(|raw| raw.parse::<i64>().ok())
        .is_some_and(|next_offset| update_id < next_offset)
}

/// Process a single message.
pub(crate) fn handle_message(message: TelegramMessage) {
    // Use text or caption (for media messages)
    let content = message
        .text
        .as_deref()
        .filter(|t| !t.is_empty())
        .or_else(|| message.caption.as_deref().filter(|c| !c.is_empty()))
        .unwrap_or_default()
        .to_string();

    // Collect media descriptors: (file_id, mime_type, filename)
    let media_descriptors = collect_media_descriptors(&message);
    let has_media = !media_descriptors.is_empty();

    // Skip messages with no content AND no media
    if content.is_empty() && !has_media {
        return;
    }

    // Skip messages without a sender (channel posts)
    let from = match message.from {
        Some(f) => f,
        None => return,
    };

    // Skip bot messages to avoid loops
    if from.is_bot {
        return;
    }

    let is_private = message.chat.chat_type == "private";

    // Owner validation: when owner_id is set, only that user can message
    let owner_id_str = channel_host::workspace_read(OWNER_ID_PATH).filter(|s| !s.is_empty());

    if let Some(ref id_str) = owner_id_str {
        if let Ok(owner_id) = id_str.parse::<i64>() {
            if from.id != owner_id {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!(
                        "Dropping message from non-owner user {} (owner: {})",
                        from.id, owner_id
                    ),
                );
                return;
            }
        }
    } else if is_private {
        // No owner_id: apply dm_policy for private chats
        let dm_policy =
            channel_host::workspace_read(DM_POLICY_PATH).unwrap_or_else(|| "pairing".to_string());

        if dm_policy != "open" {
            // Build effective allow list: config allow_from + pairing store
            let mut allowed: Vec<String> = channel_host::workspace_read(ALLOW_FROM_PATH)
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            if let Ok(store_allowed) = channel_host::pairing_read_allow_from(CHANNEL_NAME) {
                allowed.extend(store_allowed);
            }

            let id_str = from.id.to_string();
            let username_opt = from.username.as_deref();
            let is_allowed = allowed.contains(&"*".to_string())
                || allowed.contains(&id_str)
                || username_opt.is_some_and(|u| allowed.contains(&u.to_string()));

            if !is_allowed {
                if dm_policy == "pairing" {
                    // Upsert pairing request and send reply
                    let normalized_thread_id =
                        normalized_message_thread_id(message.message_thread_id);
                    let meta = serde_json::json!({
                        "chat_id": message.chat.id,
                        "user_id": from.id,
                        "username": username_opt,
                        "display_name": if let Some(ref last) = from.last_name {
                            format!("{} {}", from.first_name, last)
                        } else {
                            from.first_name.clone()
                        },
                        "conversation_kind": conversation_kind(is_private),
                        "conversation_scope_id": conversation_scope_id(
                            message.chat.id,
                            normalized_thread_id,
                            is_private,
                        ),
                        "external_conversation_key": external_conversation_key(
                            message.chat.id,
                            normalized_thread_id,
                            is_private,
                        ),
                        "raw_sender_id": from.id.to_string(),
                        "stable_sender_id": from.id.to_string(),
                    })
                    .to_string();

                    match channel_host::pairing_upsert_request(CHANNEL_NAME, &id_str, &meta) {
                        Ok(result) => {
                            channel_host::log(
                                channel_host::LogLevel::Info,
                                &format!(
                                    "Pairing request for user {} (chat {}): code {}",
                                    from.id, message.chat.id, result.code
                                ),
                            );
                            if result.created {
                                let _ = send_pairing_reply(message.chat.id, &result.code);
                            }
                        }
                        Err(e) => {
                            channel_host::log(
                                channel_host::LogLevel::Error,
                                &format!("Pairing upsert failed: {}", e),
                            );
                        }
                    }
                }
                return;
            }
        }
    }

    // For group chats, only respond if bot was mentioned or respond_to_all is enabled
    if !is_private {
        let respond_to_all = channel_host::workspace_read(RESPOND_TO_ALL_GROUP_PATH)
            .as_deref()
            .unwrap_or("false")
            == "true";

        if !respond_to_all {
            let has_command = content.starts_with('/');
            let bot_username = channel_host::workspace_read(BOT_USERNAME_PATH).unwrap_or_default();
            let has_bot_mention = if bot_username.is_empty() {
                content.contains('@')
            } else {
                let mention = format!("@{}", bot_username);
                content.to_lowercase().contains(&mention.to_lowercase())
            };

            // In groups: need command, mention, or direct reply to bot
            if !has_command && !has_bot_mention {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!("Ignoring group message without mention: {}", content),
                );
                return;
            }
        }
    }

    // Build user display name
    let user_name = if let Some(ref last) = from.last_name {
        format!("{} {}", from.first_name, last)
    } else {
        from.first_name.clone()
    };

    // Build metadata for response routing
    // Telegram private chats can now run in threaded mode. When Telegram
    // supplies a message_thread_id for a private chat topic, preserve it so
    // typing, replies, and WebUI sync stay inside that specific topic.
    let normalized_thread_id = normalized_message_thread_id(message.message_thread_id);
    let stable_sender_id = from.id.to_string();
    let metadata = TelegramMessageMetadata {
        chat_id: message.chat.id,
        message_id: message.message_id,
        user_id: from.id,
        is_private,
        message_thread_id: normalized_thread_id,
        conversation_kind: Some(conversation_kind(is_private).to_string()),
        conversation_scope_id: Some(conversation_scope_id(
            message.chat.id,
            normalized_thread_id,
            is_private,
        )),
        external_conversation_key: Some(external_conversation_key(
            message.chat.id,
            normalized_thread_id,
            is_private,
        )),
        raw_sender_id: Some(stable_sender_id.clone()),
        stable_sender_id: Some(stable_sender_id),
        subagent_session_mode: None,
    };

    let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());

    // Download media attachments
    let attachments = download_media_attachments(&media_descriptors);

    // Determine content to emit
    let content_to_emit = if content.is_empty() {
        // Media-only message — provide a default prompt so the agent sees it
        if has_media {
            "[Media received — please analyze the attached content]".to_string()
        } else {
            return; // Should not reach here (checked above)
        }
    } else {
        let bot_username = channel_host::workspace_read(BOT_USERNAME_PATH).unwrap_or_default();
        match content_to_emit_for_agent(
            &content,
            if bot_username.is_empty() {
                None
            } else {
                Some(bot_username.as_str())
            },
        ) {
            Some(value) => value,
            None => return,
        }
    };

    // Emit the message to the agent
    // Use message_thread_id as the thread_id for forum topic threading
    channel_host::log(
        channel_host::LogLevel::Info,
        &format!(
            "handle_message: chat_id={}, message_id={}, message_thread_id={:?}, is_private={}, chat_type={}",
            message.chat.id,
            message.message_id,
            normalized_thread_id,
            is_private,
            message.chat.chat_type
        ),
    );

    channel_host::emit_message(&EmittedMessage {
        user_id: from.id.to_string(),
        user_name: Some(user_name),
        content: content_to_emit,
        thread_id: incoming_session_thread_id(message.chat.id, normalized_thread_id, is_private),
        metadata_json,
        attachments,
    });

    // Persist the active thread for this chat so that sendChatAction can
    // target the correct forum topic even when Telegram omits the thread ID
    // (e.g., the General topic).  Any incoming message updates this, so the
    // typing indicator always targets the most recent thread.
    if let Some(thread_id) = normalized_thread_id {
        write_workspace_state(
            &format!("{}{}", LAST_ACTIVE_THREAD_PREFIX, message.chat.id),
            &thread_id.to_string(),
        );
    }

    channel_host::log(
        channel_host::LogLevel::Info,
        &format!(
            "Emitted message from user {} in chat {} (thread: {:?}, attachments: {})",
            from.id,
            message.chat.id,
            normalized_thread_id,
            media_descriptors.len()
        ),
    );
}

// ============================================================================
// Media Download Helpers
// ============================================================================

/// Collect all downloadable media descriptors from the message.
pub(crate) fn collect_media_descriptors(message: &TelegramMessage) -> Vec<MediaDescriptor> {
    let mut descriptors = Vec::new();

    // Photo: take the largest resolution (last element)
    if let Some(ref photos) = message.photo {
        if let Some(largest) = photos.last() {
            // Skip photos that are clearly too large
            if largest.file_size.unwrap_or(0) <= MAX_DOWNLOAD_SIZE {
                descriptors.push(MediaDescriptor {
                    file_id: largest.file_id.clone(),
                    mime_type: "image/jpeg".to_string(), // Telegram always serves photos as JPEG
                    filename: Some(format!("photo_{}.jpg", largest.file_unique_id)),
                });
            }
        }
    }

    // Voice message (OGG/Opus)
    if let Some(ref voice) = message.voice {
        if voice.file_size.unwrap_or(0) <= MAX_DOWNLOAD_SIZE {
            descriptors.push(MediaDescriptor {
                file_id: voice.file_id.clone(),
                mime_type: voice
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| "audio/ogg".to_string()),
                filename: Some(format!("voice_{}.ogg", voice.file_unique_id)),
            });
        }
    }

    // Audio file (music)
    if let Some(ref audio) = message.audio {
        if audio.file_size.unwrap_or(0) <= MAX_DOWNLOAD_SIZE {
            descriptors.push(MediaDescriptor {
                file_id: audio.file_id.clone(),
                mime_type: audio
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| "audio/mpeg".to_string()),
                filename: audio.file_name.clone(),
            });
        }
    }

    // Document (general file)
    if let Some(ref doc) = message.document {
        if doc.file_size.unwrap_or(0) <= MAX_DOWNLOAD_SIZE {
            descriptors.push(MediaDescriptor {
                file_id: doc.file_id.clone(),
                mime_type: doc
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| "application/octet-stream".to_string()),
                filename: doc.file_name.clone(),
            });
        }
    }

    // Video
    if let Some(ref video) = message.video {
        if video.file_size.unwrap_or(0) <= MAX_DOWNLOAD_SIZE {
            descriptors.push(MediaDescriptor {
                file_id: video.file_id.clone(),
                mime_type: video
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| "video/mp4".to_string()),
                filename: video.file_name.clone(),
            });
        }
    }

    // Video note (round video)
    if let Some(ref vn) = message.video_note {
        if vn.file_size.unwrap_or(0) <= MAX_DOWNLOAD_SIZE {
            descriptors.push(MediaDescriptor {
                file_id: vn.file_id.clone(),
                mime_type: "video/mp4".to_string(),
                filename: Some(format!("video_note_{}.mp4", vn.file_unique_id)),
            });
        }
    }

    // Sticker (as image — skip animated/video stickers)
    if let Some(ref sticker) = message.sticker {
        let is_static = !sticker.is_animated.unwrap_or(false) && !sticker.is_video.unwrap_or(false);
        if is_static && sticker.file_size.unwrap_or(0) <= MAX_DOWNLOAD_SIZE {
            descriptors.push(MediaDescriptor {
                file_id: sticker.file_id.clone(),
                mime_type: "image/webp".to_string(),
                filename: Some(format!("sticker_{}.webp", sticker.file_unique_id)),
            });
        }
    }

    descriptors
}

/// Download media files from Telegram and convert to WIT MediaAttachment format.
pub(crate) fn download_media_attachments(
    descriptors: &[MediaDescriptor],
) -> Vec<near::agent::channel_host::MediaAttachment> {
    use near::agent::channel_host::MediaAttachment;

    let mut attachments = Vec::new();
    let headers_json = serde_json::json!({"Accept": "*/*"}).to_string();

    for desc in descriptors {
        match download_telegram_file(&desc.file_id, &headers_json) {
            Ok(data) => {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!(
                        "Downloaded media: {} ({}, {} bytes)",
                        desc.filename.as_deref().unwrap_or("unnamed"),
                        desc.mime_type,
                        data.len()
                    ),
                );
                attachments.push(MediaAttachment {
                    mime_type: desc.mime_type.clone(),
                    data,
                    filename: desc.filename.clone(),
                });
            }
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!(
                        "Failed to download media {}: {}",
                        desc.filename.as_deref().unwrap_or("unnamed"),
                        e
                    ),
                );
            }
        }
    }

    attachments
}

/// Download a file from Telegram Bot API using getFile + file download.
///
/// Step 1: Call getFile to get the file_path
/// Step 2: Download from https://api.telegram.org/file/bot<token>/<file_path>
///
/// The host injects the bot token into the request automatically.
pub(crate) fn download_telegram_file(file_id: &str, headers_json: &str) -> Result<Vec<u8>, String> {
    // Step 1: getFile API call
    let get_file_url = format!(
        "https://api.telegram.org/bot{{TELEGRAM_BOT_TOKEN}}/getFile?file_id={}",
        file_id
    );

    let response =
        channel_host::http_request("GET", &get_file_url, headers_json, None, Some(10_000))
            .map_err(|e| format!("getFile HTTP failed: {}", e))?;

    if response.status != 200 {
        return Err(format!("getFile returned HTTP {}", response.status));
    }

    let api_response: TelegramApiResponse<TelegramFile> = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse getFile response: {}", e))?;

    if !api_response.ok {
        return Err(format!(
            "getFile API error: {}",
            api_response
                .description
                .unwrap_or_else(|| "unknown".to_string())
        ));
    }

    let file = api_response
        .result
        .ok_or_else(|| "getFile returned no result".to_string())?;

    let file_path = file
        .file_path
        .ok_or_else(|| "getFile returned no file_path".to_string())?;

    // Step 2: Download the actual file binary
    let download_url = format!(
        "https://api.telegram.org/file/bot{{TELEGRAM_BOT_TOKEN}}/{}",
        file_path
    );

    let download_response =
        channel_host::http_request("GET", &download_url, headers_json, None, Some(30_000))
            .map_err(|e| format!("File download HTTP failed: {}", e))?;

    if download_response.status != 200 {
        return Err(format!(
            "File download returned HTTP {}",
            download_response.status
        ));
    }

    if download_response.body.is_empty() {
        return Err("File download returned empty body".to_string());
    }

    Ok(download_response.body)
}

/// Clean message text by removing bot commands and @mentions at the start.
/// When bot_username is set, only strips that specific mention; otherwise strips any leading @mention.
pub(crate) fn clean_message_text(text: &str, bot_username: Option<&str>) -> String {
    let mut result = text.trim().to_string();

    // Remove leading /command
    if result.starts_with('/') {
        if let Some(space_idx) = result.find(' ') {
            result = result[space_idx..].trim_start().to_string();
        } else {
            // Just a command with no text
            return String::new();
        }
    }

    // Remove leading @mention
    if result.starts_with('@') {
        if let Some(bot) = bot_username {
            let mention = format!("@{}", bot);
            let mention_lower = mention.to_lowercase();
            let result_lower = result.to_lowercase();
            if result_lower.starts_with(&mention_lower) {
                let rest = result[mention.len()..].trim_start();
                if rest.is_empty() {
                    return String::new();
                }
                result = rest.to_string();
            } else if let Some(space_idx) = result.find(' ') {
                // Different leading @mention - only strip if it's the bot
                let first_word = &result[..space_idx];
                if first_word.eq_ignore_ascii_case(&mention) {
                    result = result[space_idx..].trim_start().to_string();
                }
            }
        } else {
            // No bot_username: strip any leading @mention
            if let Some(space_idx) = result.find(' ') {
                result = result[space_idx..].trim_start().to_string();
            } else {
                return String::new();
            }
        }
    }

    result
}

/// Decide which user content should be emitted to the agent loop.
///
/// - `/start` emits a placeholder so the agent can greet the user
/// - bare slash commands are passed through for Submission parsing
/// - empty/mention-only messages are ignored
/// - otherwise cleaned text is emitted
pub(crate) fn content_to_emit_for_agent(
    content: &str,
    bot_username: Option<&str>,
) -> Option<String> {
    let cleaned_text = clean_message_text(content, bot_username);
    let trimmed_content = content.trim();

    if trimmed_content.eq_ignore_ascii_case("/start") {
        return Some("[User started the bot]".to_string());
    }

    if cleaned_text.is_empty() && trimmed_content.starts_with('/') {
        return Some(trimmed_content.to_string());
    }

    if cleaned_text.is_empty() {
        return None;
    }

    Some(cleaned_text)
}

// ============================================================================
// Utilities
// ============================================================================

/// Create a JSON HTTP response.
pub(crate) fn json_response(status: u16, value: serde_json::Value) -> OutgoingHttpResponse {
    let body = serde_json::to_vec(&value).unwrap_or_default();
    let headers = serde_json::json!({"Content-Type": "application/json"});

    OutgoingHttpResponse {
        status,
        headers_json: headers.to_string(),
        body,
    }
}

// Export the component

// ============================================================================
// Tests
// ============================================================================
