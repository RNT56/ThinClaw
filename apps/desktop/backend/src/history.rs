use crate::chat::AttachedDoc;
use crate::direct_assets::DirectAssetStore;
use crate::file_store::FileStore;
use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::SqlitePool;
use tauri::{AppHandle, State};
use thinclaw_runtime_contracts::AssetRef;

const MAX_CONVERSATION_TITLE_BYTES: usize = 512;
const MAX_MESSAGE_CONTENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_MESSAGE_METADATA_BYTES: usize = 1024 * 1024;
const MAX_MESSAGE_ATTACHMENTS: usize = 100;
const MAX_HISTORY_PAGE_SIZE: i32 = 200;

fn unix_timestamp_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn validate_identifier(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        Err(format!("{label} identifier is invalid"))
    } else {
        Ok(())
    }
}

fn validate_title(title: &str) -> Result<(), String> {
    if title.trim().is_empty()
        || title.len() > MAX_CONVERSATION_TITLE_BYTES
        || title.chars().any(|character| character == '\0')
    {
        Err("Conversation title is invalid".to_string())
    } else {
        Ok(())
    }
}

fn serialize_optional_metadata<T: Serialize>(
    value: Option<T>,
    label: &str,
) -> Result<Option<String>, String> {
    value
        .map(|value| {
            let encoded = serde_json::to_string(&value)
                .map_err(|error| format!("Failed to encode {label}: {error}"))?;
            if encoded.len() > MAX_MESSAGE_METADATA_BYTES {
                return Err(format!("{label} exceeds the supported size limit"));
            }
            Ok(encoded)
        })
        .transpose()
}

fn generate_id() -> String {
    use rand::{distributions::Alphanumeric, Rng};
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(16)
        .map(char::from)
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, sqlx::FromRow)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    #[specta(type = f64)]
    pub created_at: i64,
    #[specta(type = f64)]
    pub updated_at: i64,
    pub project_id: Option<String>,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, sqlx::FromRow)]
pub struct MessageEntry {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub images: Option<String>,
    pub assets: Option<String>,
    pub attached_docs: Option<String>,
    pub web_search_results: Option<String>, // New column
    #[specta(type = f64)]
    pub created_at: i64,
}

#[derive(Serialize, Type)]
pub struct FrontendMessage {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub images: Option<Vec<String>>,
    pub assets: Option<Vec<AssetRef>>,
    pub attached_docs: Option<Vec<AttachedDoc>>,
    pub web_search_results: Option<Vec<crate::web_search::WebSearchResult>>,
    #[specta(type = f64)]
    pub created_at: i64,
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_get_conversations(
    state: State<'_, SqlitePool>,
) -> Result<Vec<Conversation>, String> {
    sqlx::query_as::<_, Conversation>(
        "SELECT * FROM conversations ORDER BY sort_order ASC, updated_at DESC",
    )
    .fetch_all(&*state)
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_create_conversation(
    state: State<'_, SqlitePool>,
    title: String,
    project_id: Option<String>,
) -> Result<Conversation, String> {
    validate_title(&title)?;
    if let Some(project_id) = &project_id {
        validate_identifier("Project", project_id)?;
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?)")
            .bind(project_id)
            .fetch_one(&*state)
            .await
            .map_err(|error| format!("Failed to validate project: {error}"))?;
        if !exists {
            return Err("Selected project does not exist".to_string());
        }
    }
    let id = generate_id();
    let now = unix_timestamp_millis();

    let conv = Conversation {
        id: id.clone(),
        title: title.trim().to_string(),
        created_at: now,
        updated_at: now,
        project_id: project_id.clone(),
        sort_order: 0,
    };

    sqlx::query(
        "INSERT INTO conversations (id, title, created_at, updated_at, project_id, sort_order) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&conv.id)
    .bind(&conv.title)
    .bind(conv.created_at)
    .bind(conv.updated_at)
    .bind(&conv.project_id)
    .bind(conv.sort_order)
    .execute(&*state)
    .await
    .map_err(|e| e.to_string())?;

    Ok(conv)
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_delete_conversation(
    state: State<'_, SqlitePool>,
    vector_manager: State<'_, crate::vector_store::VectorStoreManager>,
    file_store: State<'_, FileStore>,
    id: String,
) -> Result<(), String> {
    validate_identifier("Conversation", &id)?;
    let update_guard = vector_manager.lock_updates().await;
    let paths: Vec<String> =
        sqlx::query_scalar("SELECT path FROM documents WHERE chat_id = ? AND project_id IS NULL")
            .bind(&id)
            .fetch_all(&*state)
            .await
            .map_err(|error| error.to_string())?;

    let mut transaction = state.begin().await.map_err(|error| error.to_string())?;
    sqlx::query(
        "DELETE FROM direct_assets WHERE namespace = 'direct_workbench' AND id IN (SELECT id FROM documents WHERE chat_id = ? AND project_id IS NULL)",
    )
    .bind(&id)
    .execute(&mut *transaction)
    .await
    .map_err(|error| error.to_string())?;
    sqlx::query(
        "DELETE FROM chunks WHERE document_id IN (SELECT id FROM documents WHERE chat_id = ? AND project_id IS NULL)",
    )
    .bind(&id)
    .execute(&mut *transaction)
    .await
    .map_err(|error| error.to_string())?;
    sqlx::query("DELETE FROM documents WHERE chat_id = ? AND project_id IS NULL")
        .bind(&id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
    sqlx::query("UPDATE documents SET chat_id = NULL WHERE chat_id = ? AND project_id IS NOT NULL")
        .bind(&id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
    sqlx::query("DELETE FROM messages WHERE conversation_id = ?")
        .bind(&id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
    sqlx::query("DELETE FROM chat_summaries WHERE conversation_id = ?")
        .bind(&id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
    sqlx::query("DELETE FROM conversations WHERE id = ?")
        .bind(&id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
    transaction
        .commit()
        .await
        .map_err(|error| error.to_string())?;

    vector_manager
        .delete_scope(&crate::vector_store::VectorScope::Chat(id))
        .map_err(|error| format!("Conversation deleted but vector cleanup failed: {error}"))?;
    drop(update_guard);
    let mut cleanup_failures = 0_usize;
    for path in paths {
        let still_referenced: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM documents WHERE path = ?)")
                .bind(&path)
                .fetch_one(&*state)
                .await
                .map_err(|error| error.to_string())?;
        if !still_referenced {
            if let Err(error) = file_store
                .delete_absolute(std::path::Path::new(&path))
                .await
            {
                cleanup_failures = cleanup_failures.saturating_add(1);
                tracing::warn!("[history] Could not remove deleted conversation asset: {error}");
            }
        }
    }
    if cleanup_failures > 0 {
        return Err(format!(
            "Conversation data was deleted, but {cleanup_failures} backing file(s) could not be removed"
        ));
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_update_conversation_title(
    state: State<'_, SqlitePool>,
    id: String,
    title: String,
) -> Result<(), String> {
    validate_identifier("Conversation", &id)?;
    validate_title(&title)?;
    let result = sqlx::query("UPDATE conversations SET title = ?, updated_at = ? WHERE id = ?")
        .bind(title.trim())
        .bind(unix_timestamp_millis())
        .bind(&id)
        .execute(&*state)
        .await
        .map_err(|e| e.to_string())?;
    if result.rows_affected() != 1 {
        return Err("Conversation does not exist".to_string());
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_update_conversation_project(
    state: State<'_, SqlitePool>,
    vector_manager: State<'_, crate::vector_store::VectorStoreManager>,
    id: String,
    project_id: Option<String>,
) -> Result<(), String> {
    validate_identifier("Conversation", &id)?;
    if let Some(project_id) = &project_id {
        validate_identifier("Project", project_id)?;
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?)")
            .bind(project_id)
            .fetch_one(&*state)
            .await
            .map_err(|error| format!("Failed to validate project: {error}"))?;
        if !exists {
            return Err("Selected project does not exist".to_string());
        }
    }

    let _update_guard = vector_manager.lock_updates().await;
    let mut transaction = state.begin().await.map_err(|error| error.to_string())?;
    let previous_project: Option<Option<String>> =
        sqlx::query_scalar("SELECT project_id FROM conversations WHERE id = ?")
            .bind(&id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| error.to_string())?;
    let Some(previous_project) = previous_project else {
        return Err("Conversation does not exist".to_string());
    };
    if previous_project == project_id {
        return Ok(());
    }

    let mut promoted_documents = 0_u64;
    if let Some(new_project_id) = &project_id {
        promoted_documents = sqlx::query(
            "UPDATE documents SET project_id = ?, updated_at = ? WHERE chat_id = ? AND project_id IS NULL",
        )
        .bind(new_project_id)
        .bind(unix_timestamp_millis())
        .bind(&id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| format!("Failed to move chat documents into the project: {error}"))?
        .rows_affected();
    }
    sqlx::query("UPDATE conversations SET project_id = ?, updated_at = ? WHERE id = ?")
        .bind(&project_id)
        .bind(unix_timestamp_millis())
        .bind(&id)
        .execute(&mut *transaction)
        .await
        .map_err(|e| e.to_string())?;
    transaction
        .commit()
        .await
        .map_err(|error| error.to_string())?;

    if promoted_documents > 0 {
        vector_manager
            .delete_scope(&crate::vector_store::VectorScope::Chat(id.clone()))
            .map_err(|error| {
                format!("Conversation moved but chat index cleanup failed: {error}")
            })?;
        let destination_project = project_id.ok_or_else(|| {
            "Conversation moved without a destination for promoted documents".to_string()
        })?;
        let new_scope = crate::vector_store::VectorScope::Project(destination_project);
        if let Err(error) =
            crate::rag::rebuild_vector_scope_with_lock_held(&state, &vector_manager, &new_scope)
                .await
        {
            let _ = vector_manager.delete_scope(&new_scope);
            return Err(format!(
                "Conversation moved, but the destination vector index could not be rebuilt: {error}"
            ));
        }
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_update_conversations_order(
    state: State<'_, SqlitePool>,
    orders: Vec<(String, i32)>,
) -> Result<(), String> {
    if orders.len() > 10_000 {
        return Err("Conversation order update exceeds the supported limit".to_string());
    }
    let mut seen = std::collections::HashSet::with_capacity(orders.len());
    for (id, order) in &orders {
        validate_identifier("Conversation", id)?;
        if *order < 0 || !seen.insert(id) {
            return Err("Conversation order entries are invalid or duplicated".to_string());
        }
    }
    let mut tx = state.begin().await.map_err(|e| e.to_string())?;
    for (id, order) in orders {
        let result = sqlx::query("UPDATE conversations SET sort_order = ? WHERE id = ?")
            .bind(order)
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        if result.rows_affected() != 1 {
            return Err(format!("Conversation {id} does not exist"));
        }
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_get_messages(
    state: State<'_, SqlitePool>,
    conversation_id: String,
    limit: Option<i32>,
    before_created_at: Option<f64>,
) -> Result<Vec<FrontendMessage>, String> {
    validate_identifier("Conversation", &conversation_id)?;
    let limit = limit.unwrap_or(50);
    if !(1..=MAX_HISTORY_PAGE_SIZE).contains(&limit) {
        return Err(format!(
            "History page size must be between 1 and {MAX_HISTORY_PAGE_SIZE}"
        ));
    }
    let before = match before_created_at {
        Some(timestamp)
            if timestamp.is_finite() && timestamp >= 0.0 && timestamp <= i64::MAX as f64 =>
        {
            timestamp as i64
        }
        Some(_) => return Err("History pagination timestamp is invalid".to_string()),
        None => i64::MAX,
    };

    let entries = sqlx::query_as::<_, MessageEntry>(
        "SELECT id, conversation_id, role, content, images, assets, attached_docs, web_search_results, created_at FROM messages WHERE conversation_id = ? AND created_at < ? ORDER BY created_at DESC LIMIT ?",
    )
    .bind(conversation_id)
    .bind(before)
    .bind(i64::from(limit))
    .fetch_all(&*state)
    .await
    .map_err(|e| e.to_string())?;

    // We fetch DESC for pagination logic (getting the *previous* messages), but frontend expects ASC (chronological)
    // so we reverse them before returning.
    let mut msgs_asc = entries;
    msgs_asc.reverse();

    let msgs = msgs_asc
        .into_iter()
        .map(|e| {
            let images = e.images.and_then(|s| serde_json::from_str(&s).ok());
            // Support both new (AttachedDoc) and old (String "id:name") formats
            let attached_docs = e.attached_docs.and_then(|s| {
                if let Ok(docs) = serde_json::from_str::<Vec<AttachedDoc>>(&s) {
                    Some(docs)
                } else if let Ok(strs) = serde_json::from_str::<Vec<String>>(&s) {
                    Some(
                        strs.into_iter()
                            .map(|str_val| {
                                let parts: Vec<&str> = str_val.splitn(2, ':').collect();
                                if parts.len() == 2 {
                                    AttachedDoc {
                                        id: parts[0].to_string(),
                                        name: parts[1].to_string(),
                                        asset_ref: Some(DirectAssetStore::direct_ref(parts[0])),
                                    }
                                } else {
                                    AttachedDoc {
                                        asset_ref: Some(DirectAssetStore::direct_ref(&str_val)),
                                        id: str_val,
                                        name: "Unknown".to_string(),
                                    }
                                }
                            })
                            .collect(),
                    )
                } else {
                    None
                }
            });

            let web_search_results = e
                .web_search_results
                .and_then(|s| serde_json::from_str(&s).ok());
            let assets = e
                .assets
                .and_then(|s| serde_json::from_str::<Vec<AssetRef>>(&s).ok())
                .or_else(|| {
                    DirectAssetStore::refs_for_message(
                        None,
                        images.as_deref(),
                        attached_docs.as_deref(),
                    )
                });

            FrontendMessage {
                id: e.id,
                conversation_id: e.conversation_id,
                role: e.role,
                content: e.content,
                images,
                assets,
                attached_docs,
                web_search_results,
                created_at: e.created_at,
            }
        })
        .collect();

    Ok(msgs)
}

#[tauri::command]
#[specta::specta]
// Tauri commands intentionally expose flat arguments for generated bindings.
#[allow(clippy::too_many_arguments)]
pub async fn direct_history_save_message(
    state: State<'_, SqlitePool>,
    conversation_id: String,
    role: String,
    content: String,
    images: Option<Vec<String>>,
    assets: Option<Vec<AssetRef>>,
    attached_docs: Option<Vec<AttachedDoc>>,
    web_search_results: Option<Vec<crate::web_search::WebSearchResult>>,
) -> Result<String, String> {
    validate_identifier("Conversation", &conversation_id)?;
    if !matches!(role.as_str(), "user" | "assistant" | "system" | "tool") {
        return Err("Message role is invalid".to_string());
    }
    if content.len() > MAX_MESSAGE_CONTENT_BYTES || content.contains('\0') {
        return Err("Message content exceeds the supported limit or contains NUL".to_string());
    }
    for (label, count) in [
        ("images", images.as_ref().map_or(0, Vec::len)),
        ("assets", assets.as_ref().map_or(0, Vec::len)),
        (
            "attached documents",
            attached_docs.as_ref().map_or(0, Vec::len),
        ),
        (
            "web search results",
            web_search_results.as_ref().map_or(0, Vec::len),
        ),
    ] {
        if count > MAX_MESSAGE_ATTACHMENTS {
            return Err(format!(
                "Message {label} exceed the {MAX_MESSAGE_ATTACHMENTS}-item limit"
            ));
        }
    }
    if images.as_ref().is_some_and(|images| {
        images
            .iter()
            .any(|path| path.len() > 16 * 1024 || path.contains('\0'))
    }) {
        return Err("Message image reference is invalid".to_string());
    }
    if assets.as_ref().is_some_and(|assets| {
        assets.iter().any(|asset| {
            asset.id.is_empty() || asset.id.len() > 128 || asset.id.chars().any(char::is_control)
        })
    }) {
        return Err("Message asset reference is invalid".to_string());
    }
    if attached_docs.as_ref().is_some_and(|documents| {
        documents.iter().any(|document| {
            document.id.is_empty()
                || document.id.len() > 128
                || document.id.chars().any(char::is_control)
                || document.name.is_empty()
                || document.name.len() > 512
                || document.name.contains('\0')
        })
    }) {
        return Err("Attached document metadata is invalid".to_string());
    }

    let id = generate_id();
    let now = unix_timestamp_millis();

    let merged_assets =
        DirectAssetStore::refs_for_message(assets, images.as_deref(), attached_docs.as_deref());
    if merged_assets
        .as_ref()
        .is_some_and(|assets| assets.len() > MAX_MESSAGE_ATTACHMENTS * 3)
    {
        return Err("Merged message assets exceed the supported limit".to_string());
    }
    if merged_assets.as_ref().is_some_and(|assets| {
        assets.iter().any(|asset| {
            asset.id.is_empty() || asset.id.len() > 128 || asset.id.chars().any(char::is_control)
        })
    }) {
        return Err("Merged message asset reference is invalid".to_string());
    }
    let images_json = serialize_optional_metadata(images, "message images")?;
    let assets_json = serialize_optional_metadata(merged_assets, "message assets")?;
    let docs_json = serialize_optional_metadata(attached_docs, "attached documents")?;
    let search_json = serialize_optional_metadata(web_search_results, "web search results")?;

    let mut transaction = state.begin().await.map_err(|error| error.to_string())?;
    let conversation_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversations WHERE id = ?)")
            .bind(&conversation_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|error| error.to_string())?;
    if !conversation_exists {
        return Err("Conversation does not exist".to_string());
    }
    sqlx::query(
        "INSERT INTO messages (id, conversation_id, role, content, images, assets, attached_docs, web_search_results, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&id)
    .bind(&conversation_id)
    .bind(&role)
    .bind(&content)
    .bind(&images_json)
    .bind(&assets_json)
    .bind(&docs_json)
    .bind(&search_json)
    .bind(now)
    .execute(&mut *transaction)
    .await
    .map_err(|e| e.to_string())?;

    // Update conversation timestamp
    let update = sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(&conversation_id)
        .execute(&mut *transaction)
        .await
        .map_err(|e| e.to_string())?;
    if update.rows_affected() != 1 {
        return Err("Conversation disappeared while saving the message".to_string());
    }
    transaction
        .commit()
        .await
        .map_err(|error| error.to_string())?;

    Ok(id)
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_edit_message(
    state: State<'_, SqlitePool>,
    message_id: String,
    new_content: String,
) -> Result<(), String> {
    validate_identifier("Message", &message_id)?;
    if new_content.len() > MAX_MESSAGE_CONTENT_BYTES || new_content.contains('\0') {
        return Err("Edited message exceeds the supported limit or contains NUL".to_string());
    }
    let mut tx = state.begin().await.map_err(|e| e.to_string())?;

    // 1. Get the message's created_at and conversation_id
    let msg: (String, i64, i64) =
        sqlx::query_as("SELECT conversation_id, created_at, rowid FROM messages WHERE id = ?")
            .bind(&message_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("Message not found")?;

    // 2. Update content
    sqlx::query("UPDATE messages SET content = ? WHERE id = ?")
        .bind(&new_content)
        .bind(&message_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    // 3. Delete only subsequent messages. `rowid` disambiguates messages that
    // share the same millisecond timestamp without deleting an earlier peer.
    sqlx::query(
        "DELETE FROM messages WHERE conversation_id = ? AND (created_at > ? OR (created_at = ? AND rowid > ?))",
    )
        .bind(&msg.0)
        .bind(msg.1)
        .bind(msg.1)
        .bind(msg.2)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM chat_summaries WHERE conversation_id = ?")
        .bind(&msg.0)
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;

    // 4. Update conversation timestamp
    let now = unix_timestamp_millis();

    sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(&msg.0)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_delete_all_history(
    _app: AppHandle,
    pool: State<'_, SqlitePool>,
    vector_manager: State<'_, crate::vector_store::VectorStoreManager>,
    file_store: State<'_, FileStore>,
) -> Result<(), String> {
    println!("[history] direct_history_delete_all_history: clearing database...");
    let update_guard = vector_manager.lock_updates().await;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM chunks")
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("failed to delete chunks: {}", e))?;
    println!("[history] - chunks cleared");

    sqlx::query("DELETE FROM documents")
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("failed to delete documents: {}", e))?;
    println!("[history] - documents cleared");

    sqlx::query("DELETE FROM messages")
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("failed to delete messages: {}", e))?;
    println!("[history] - messages cleared");

    sqlx::query("DELETE FROM conversations")
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("failed to delete conversations: {}", e))?;
    println!("[history] - conversations cleared");

    sqlx::query("DELETE FROM chat_summaries")
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("failed to delete summaries: {}", e))?;
    println!("[history] - summaries cleared");

    sqlx::query("DELETE FROM projects")
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("failed to delete projects: {}", e))?;
    println!("[history] - projects cleared");

    sqlx::query("DELETE FROM direct_assets WHERE namespace = 'direct_workbench'")
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("failed to delete direct assets: {}", e))?;
    println!("[history] - direct assets cleared");

    tx.commit()
        .await
        .map_err(|e| format!("failed to commit transaction: {}", e))?;
    println!("[history] database transaction committed");

    // 2. Reset all Vector Store indices
    vector_manager.reset_all().map_err(|e| e.to_string())?;
    drop(update_guard);

    // 3. Clear all history-backed file families through FileStore so cloud
    // copies receive deletions too and symlinks are never traversed.
    let mut cleanup_failures = Vec::new();
    for directory in ["documents", "images", "voice", "previews"] {
        if let Err(error) = file_store.delete_tree(directory).await {
            cleanup_failures.push(format!("{directory}: {error}"));
        }
    }
    if !cleanup_failures.is_empty() {
        return Err(format!(
            "History metadata was deleted, but backing-file cleanup failed: {}",
            cleanup_failures.join("; ")
        ));
    }

    println!("[history] All history deleted and vector stores reset.");
    Ok(())
}
