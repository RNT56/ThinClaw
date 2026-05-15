use crate::chat::AttachedDoc;
use crate::direct_assets::DirectAssetStore;
use crate::file_store::FileStore;
use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::SqlitePool;
use tauri::{AppHandle, State};
use thinclaw_runtime_contracts::AssetRef;

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
    let id = generate_id();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let conv = Conversation {
        id: id.clone(),
        title,
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
    id: String,
) -> Result<(), String> {
    sqlx::query("DELETE FROM conversations WHERE id = ?")
        .bind(id)
        .execute(&*state)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_update_conversation_title(
    state: State<'_, SqlitePool>,
    id: String,
    title: String,
) -> Result<(), String> {
    sqlx::query("UPDATE conversations SET title = ? WHERE id = ?")
        .bind(title)
        .bind(id)
        .execute(&*state)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_update_conversation_project(
    state: State<'_, SqlitePool>,
    id: String,
    project_id: Option<String>,
) -> Result<(), String> {
    sqlx::query("UPDATE conversations SET project_id = ? WHERE id = ?")
        .bind(project_id)
        .bind(id)
        .execute(&*state)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_update_conversations_order(
    state: State<'_, SqlitePool>,
    orders: Vec<(String, i32)>,
) -> Result<(), String> {
    let mut tx = state.begin().await.map_err(|e| e.to_string())?;
    for (id, order) in orders {
        sqlx::query("UPDATE conversations SET sort_order = ? WHERE id = ?")
            .bind(order)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
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
    let limit = limit.unwrap_or(50) as i64;
    let before = before_created_at.map(|t| t as i64).unwrap_or(i64::MAX);

    let entries = sqlx::query_as::<_, MessageEntry>(
        "SELECT id, conversation_id, role, content, images, assets, attached_docs, web_search_results, created_at FROM messages WHERE conversation_id = ? AND created_at < ? ORDER BY created_at DESC LIMIT ?",
    )
    .bind(conversation_id)
    .bind(before)
    .bind(limit)
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
    let id = generate_id();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let merged_assets =
        DirectAssetStore::refs_for_message(assets, images.as_deref(), attached_docs.as_deref());
    let images_json = images.map(|v| serde_json::to_string(&v).unwrap_or("[]".to_string()));
    let assets_json = merged_assets.map(|v| serde_json::to_string(&v).unwrap_or("[]".to_string()));
    let docs_json = attached_docs.map(|v| serde_json::to_string(&v).unwrap_or("[]".to_string()));
    let search_json =
        web_search_results.map(|v| serde_json::to_string(&v).unwrap_or("[]".to_string()));

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
    .execute(&*state)
    .await
    .map_err(|e| e.to_string())?;

    // Update conversation timestamp
    sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(&conversation_id)
        .execute(&*state)
        .await
        .map_err(|e| e.to_string())?;

    Ok(id)
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_edit_message(
    state: State<'_, SqlitePool>,
    message_id: String,
    new_content: String,
) -> Result<(), String> {
    let mut tx = state.begin().await.map_err(|e| e.to_string())?;

    // 1. Get the message's created_at and conversation_id
    let msg: MessageEntry = sqlx::query_as("SELECT * FROM messages WHERE id = ?")
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

    // 3. Delete subsequent messages (use >= to catch millisecond collisions,
    //    but exclude the edited message itself by ID)
    sqlx::query("DELETE FROM messages WHERE conversation_id = ? AND created_at >= ? AND id != ?")
        .bind(&msg.conversation_id)
        .bind(msg.created_at)
        .bind(&message_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    // 4. Update conversation timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(&msg.conversation_id)
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

    // 3. Clear Files via FileStore
    if file_store.exists("documents").await {
        let docs_path = file_store.resolve_path("documents").await;
        let _ = tokio::fs::remove_dir_all(&docs_path).await;
        file_store
            .create_dir_all("documents")
            .await
            .map_err(|e| e.to_string())?;
    }

    if file_store.exists("images").await {
        let images_path = file_store.resolve_path("images").await;
        let _ = tokio::fs::remove_dir_all(&images_path).await;
        file_store
            .create_dir_all("images")
            .await
            .map_err(|e| e.to_string())?;
    }

    println!("[history] All history deleted and vector stores reset.");
    Ok(())
}
