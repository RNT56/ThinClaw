use crate::chat::AttachedDoc;
use crate::direct_assets::DirectAssetStore;
use crate::file_store::FileStore;
use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{FromRow, SqlitePool};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use tauri::{AppHandle, State};
use thinclaw_runtime_contracts::AssetRef;
use uuid::Uuid;

const DIRECT_SURFACE: &str = "direct_workbench";
const DIRECT_CHANNEL: &str = "desktop";
const DIRECT_USER_ID: &str = "local_user";
const LEGACY_HISTORY_MIGRATION: &str = "desktop-history-v1";
const LEGACY_ID_NAMESPACE: Uuid = Uuid::from_u128(0x6b95a854_1873_45c3_b077_e315b1031213);

fn migrated_id(record_kind: &str, legacy_id: &str) -> String {
    Uuid::new_v5(
        &LEGACY_ID_NAMESPACE,
        format!("{record_kind}:{legacy_id}").as_bytes(),
    )
    .to_string()
}

fn exact_id(id: &str) -> String {
    Uuid::parse_str(id)
        .map(|uuid| uuid.to_string())
        .unwrap_or_else(|_| id.to_string())
}

fn legacy_conversation_id(id: &str) -> String {
    migrated_id("conversation", id)
}

fn legacy_message_id(id: &str) -> String {
    migrated_id("message", id)
}

fn millis_to_timestamp(value: i64) -> String {
    Utc.timestamp_millis_opt(value)
        .single()
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn timestamp_to_millis(value: &str) -> i64 {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.timestamp_millis())
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .map(|timestamp| timestamp.and_utc().timestamp_millis())
        })
        .unwrap_or_default()
}

fn parse_json_or_null(value: Option<String>) -> Value {
    value
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or(Value::Null)
}

fn direct_metadata(metadata: &Value) -> Option<&serde_json::Map<String, Value>> {
    metadata.get(DIRECT_SURFACE)?.as_object()
}

fn set_direct_metadata_value(metadata: &mut Value, key: &str, value: Value) {
    if !metadata.is_object() {
        *metadata = json!({});
    }
    let root = metadata.as_object_mut().expect("metadata object");
    let direct = root
        .entry(DIRECT_SURFACE.to_string())
        .or_insert_with(|| json!({}));
    if !direct.is_object() {
        *direct = json!({});
    }
    direct
        .as_object_mut()
        .expect("direct metadata object")
        .insert(key.to_string(), value);
}

#[derive(Debug, FromRow)]
struct LegacyConversation {
    id: String,
    title: String,
    created_at: i64,
    updated_at: i64,
    project_id: Option<String>,
    sort_order: i32,
}

#[derive(Debug, FromRow)]
struct LegacyMessage {
    id: String,
    conversation_id: String,
    role: String,
    content: String,
    images: Option<String>,
    assets: Option<String>,
    attached_docs: Option<String>,
    web_search_results: Option<String>,
    created_at: i64,
}

/// One app-wide conversation service shared by Direct Workbench and the
/// embedded ThinClaw agent. Direct commands use the SQLx adapter while the
/// agent receives the exact same `Arc<dyn Database>`.
#[derive(Clone)]
pub struct SharedHistoryStore {
    pool: SqlitePool,
    runtime_store: Arc<dyn thinclaw_core::db::Database>,
}

impl SharedHistoryStore {
    pub async fn open(state_dir: &Path, legacy_pool: &SqlitePool) -> Result<Self, String> {
        use thinclaw_core::db::Database as _;

        let runtime_path = crate::thinclaw::runtime_builder::runtime_db_path(state_dir);
        let backend = thinclaw_db::libsql::LibSqlBackend::new_local(&runtime_path)
            .await
            .map_err(|error| error.to_string())?;
        backend
            .run_migrations()
            .await
            .map_err(|error| error.to_string())?;
        let runtime_store: Arc<dyn thinclaw_core::db::Database> = Arc::new(backend);

        let database_url = format!("sqlite://{}?mode=rwc", runtime_path.to_string_lossy());
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("PRAGMA foreign_keys = ON")
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("PRAGMA busy_timeout = 5000")
                        .execute(&mut *connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(&database_url)
            .await
            .map_err(|error| error.to_string())?;

        let store = Self {
            pool,
            runtime_store,
        };
        store.migrate_legacy_history(legacy_pool).await?;
        Ok(store)
    }

    pub fn runtime_store(&self) -> Arc<dyn thinclaw_core::db::Database> {
        Arc::clone(&self.runtime_store)
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    async fn migrate_legacy_history(&self, legacy_pool: &SqlitePool) -> Result<(), String> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS desktop_history_migrations (\
             id TEXT PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&self.pool)
        .await
        .map_err(|error| error.to_string())?;

        let already_applied: Option<String> = sqlx::query_scalar(
            "SELECT id FROM desktop_history_migrations WHERE id = ?",
        )
        .bind(LEGACY_HISTORY_MIGRATION)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| error.to_string())?;
        if already_applied.is_some() {
            return Ok(());
        }

        let conversations = sqlx::query_as::<_, LegacyConversation>(
            "SELECT id, title, created_at, updated_at, project_id, sort_order \
             FROM conversations ORDER BY created_at ASC",
        )
        .fetch_all(legacy_pool)
        .await
        .map_err(|error| error.to_string())?;
        let messages = sqlx::query_as::<_, LegacyMessage>(
            "SELECT id, conversation_id, role, content, images, assets, attached_docs, \
             web_search_results, created_at FROM messages ORDER BY created_at ASC",
        )
        .fetch_all(legacy_pool)
        .await
        .map_err(|error| error.to_string())?;
        let legacy_conversation_ids: HashSet<&str> =
            conversations.iter().map(|row| row.id.as_str()).collect();

        let mut transaction = self.pool.begin().await.map_err(|error| error.to_string())?;
        for conversation in &conversations {
            let id = legacy_conversation_id(&conversation.id);
            let metadata = json!({
                "direct_workbench": {
                    "legacy_id": conversation.id,
                    "title": conversation.title,
                    "project_id": conversation.project_id,
                    "sort_order": conversation.sort_order,
                    "created_at": conversation.created_at,
                    "updated_at": conversation.updated_at,
                }
            });
            sqlx::query(
                "INSERT INTO conversations (\
                 id, channel, user_id, actor_id, conversation_scope_id, conversation_kind, \
                 surface, thread_id, stable_external_conversation_key, started_at, last_activity, metadata\
                 ) VALUES (?, ?, ?, ?, ?, 'direct', ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(id) DO NOTHING",
            )
            .bind(&id)
            .bind(DIRECT_CHANNEL)
            .bind(DIRECT_USER_ID)
            .bind(DIRECT_USER_ID)
            .bind(&id)
            .bind(DIRECT_SURFACE)
            .bind(&conversation.id)
            .bind(format!("{DIRECT_SURFACE}:{}", conversation.id))
            .bind(millis_to_timestamp(conversation.created_at))
            .bind(millis_to_timestamp(conversation.updated_at))
            .bind(metadata.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(|error| error.to_string())?;
        }

        for message in &messages {
            if !legacy_conversation_ids.contains(message.conversation_id.as_str()) {
                tracing::warn!(
                    legacy_message_id = %message.id,
                    legacy_conversation_id = %message.conversation_id,
                    "Skipping orphaned legacy Direct Workbench message"
                );
                continue;
            }
            let metadata = json!({
                "direct_workbench": {
                    "legacy_id": message.id,
                    "images": parse_json_or_null(message.images.clone()),
                    "assets": parse_json_or_null(message.assets.clone()),
                    "attached_docs": parse_json_or_null(message.attached_docs.clone()),
                    "web_search_results": parse_json_or_null(message.web_search_results.clone()),
                }
            });
            sqlx::query(
                "INSERT INTO conversation_messages (\
                 id, conversation_id, role, content, actor_id, actor_display_name, \
                 raw_sender_id, metadata, created_at\
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO NOTHING",
            )
            .bind(legacy_message_id(&message.id))
            .bind(legacy_conversation_id(&message.conversation_id))
            .bind(&message.role)
            .bind(&message.content)
            .bind((message.role == "user").then_some(DIRECT_USER_ID))
            .bind((message.role == "user").then_some("Local user"))
            .bind((message.role == "user").then_some(DIRECT_USER_ID))
            .bind(metadata.to_string())
            .bind(millis_to_timestamp(message.created_at))
            .execute(&mut *transaction)
            .await
            .map_err(|error| error.to_string())?;
        }

        sqlx::query("INSERT INTO desktop_history_migrations (id) VALUES (?)")
            .bind(LEGACY_HISTORY_MIGRATION)
            .execute(&mut *transaction)
            .await
            .map_err(|error| error.to_string())?;
        transaction
            .commit()
            .await
            .map_err(|error| error.to_string())?;

        tracing::info!(
            conversations = conversations.len(),
            messages = messages.len(),
            "Merged legacy Direct Workbench history into the shared conversation store"
        );
        Ok(())
    }

    async fn require_direct_conversation(&self, id: &str) -> Result<String, String> {
        let exact = exact_id(id);
        let migrated = legacy_conversation_id(id);
        let resolved: Option<String> = sqlx::query_scalar(
            "SELECT id FROM conversations WHERE (id = ? OR id = ?) AND surface = ? \
             ORDER BY CASE WHEN id = ? THEN 0 ELSE 1 END LIMIT 1",
        )
        .bind(&exact)
        .bind(&migrated)
        .bind(DIRECT_SURFACE)
        .bind(&exact)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| error.to_string())?;
        resolved.ok_or_else(|| "Direct Workbench conversation not found".to_string())
    }

    async fn set_conversation_metadata_value(
        &self,
        id: &str,
        key: &str,
        value: Value,
    ) -> Result<(), String> {
        let id = self.require_direct_conversation(id).await?;
        let mut transaction = self.pool.begin().await.map_err(|error| error.to_string())?;
        let raw: String = sqlx::query_scalar("SELECT metadata FROM conversations WHERE id = ?")
            .bind(&id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|error| error.to_string())?;
        let mut metadata: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
        set_direct_metadata_value(&mut metadata, key, value);
        sqlx::query("UPDATE conversations SET metadata = ? WHERE id = ? AND surface = ?")
            .bind(metadata.to_string())
            .bind(id)
            .bind(DIRECT_SURFACE)
            .execute(&mut *transaction)
            .await
            .map_err(|error| error.to_string())?;
        transaction
            .commit()
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn message_texts(&self, id: &str) -> Result<Vec<(String, String)>, String> {
        #[derive(FromRow)]
        struct TextMessage {
            role: String,
            content: String,
        }

        let id = self.require_direct_conversation(id).await?;
        sqlx::query_as::<_, TextMessage>(
            "SELECT role, content FROM conversation_messages \
             WHERE conversation_id = ? ORDER BY created_at ASC, rowid ASC",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .map(|messages| {
            messages
                .into_iter()
                .map(|message| (message.role, message.content))
                .collect()
        })
        .map_err(|error| error.to_string())
    }

    pub async fn conversation_project_id(&self, id: &str) -> Result<Option<String>, String> {
        let exact = exact_id(id);
        let migrated = legacy_conversation_id(id);
        let metadata: Option<String> = sqlx::query_scalar(
            "SELECT metadata FROM conversations WHERE (id = ? OR id = ?) AND surface = ? \
             ORDER BY CASE WHEN id = ? THEN 0 ELSE 1 END LIMIT 1",
        )
        .bind(&exact)
        .bind(migrated)
        .bind(DIRECT_SURFACE)
        .bind(&exact)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| error.to_string())?;
        Ok(metadata
            .and_then(|metadata| serde_json::from_str::<Value>(&metadata).ok())
            .and_then(|value| {
                direct_metadata(&value)?
                    .get("project_id")?
                    .as_str()
                    .map(str::to_string)
            }))
    }

    pub async fn delete_project_conversations(&self, project_id: &str) -> Result<(), String> {
        sqlx::query(
            "DELETE FROM conversations WHERE surface = ? \
             AND json_extract(metadata, '$.direct_workbench.project_id') = ?",
        )
        .bind(DIRECT_SURFACE)
        .bind(project_id)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
    }

    pub async fn delete_all_direct(&self) -> Result<(), String> {
        sqlx::query("DELETE FROM conversations WHERE surface = ?")
            .bind(DIRECT_SURFACE)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
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

#[derive(Debug, Clone, FromRow)]
struct SharedConversationRow {
    id: String,
    metadata: String,
    started_at: String,
    last_activity: String,
}

#[derive(Debug, Clone, FromRow)]
struct MessageEntry {
    id: String,
    conversation_id: String,
    role: String,
    content: String,
    metadata: String,
    created_at: String,
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

fn conversation_from_row(row: SharedConversationRow) -> Conversation {
    let metadata: Value = serde_json::from_str(&row.metadata).unwrap_or_else(|_| json!({}));
    let direct = direct_metadata(&metadata);
    Conversation {
        id: row.id,
        title: direct
            .and_then(|value| value.get("title"))
            .and_then(Value::as_str)
            .unwrap_or("New conversation")
            .to_string(),
        created_at: direct
            .and_then(|value| value.get("created_at"))
            .and_then(Value::as_i64)
            .unwrap_or_else(|| timestamp_to_millis(&row.started_at)),
        updated_at: direct
            .and_then(|value| value.get("updated_at"))
            .and_then(Value::as_i64)
            .unwrap_or_else(|| timestamp_to_millis(&row.last_activity)),
        project_id: direct
            .and_then(|value| value.get("project_id"))
            .and_then(Value::as_str)
            .map(str::to_string),
        sort_order: direct
            .and_then(|value| value.get("sort_order"))
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or_default(),
    }
}

fn message_from_entry(entry: MessageEntry) -> FrontendMessage {
    let metadata: Value = serde_json::from_str(&entry.metadata).unwrap_or_else(|_| json!({}));
    let direct = direct_metadata(&metadata);
    let images = direct
        .and_then(|value| value.get("images"))
        .filter(|value| !value.is_null())
        .and_then(|value| serde_json::from_value(value.clone()).ok());
    let attached_docs = direct
        .and_then(|value| value.get("attached_docs"))
        .filter(|value| !value.is_null())
        .and_then(|value| {
            if let Ok(documents) = serde_json::from_value::<Vec<AttachedDoc>>(value.clone()) {
                Some(documents)
            } else if let Ok(documents) = serde_json::from_value::<Vec<String>>(value.clone()) {
                Some(
                    documents
                        .into_iter()
                        .map(|document| {
                            let parts: Vec<&str> = document.splitn(2, ':').collect();
                            if parts.len() == 2 {
                                AttachedDoc {
                                    id: parts[0].to_string(),
                                    name: parts[1].to_string(),
                                    asset_ref: Some(DirectAssetStore::direct_ref(parts[0])),
                                }
                            } else {
                                AttachedDoc {
                                    asset_ref: Some(DirectAssetStore::direct_ref(&document)),
                                    id: document,
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
    let assets = direct
        .and_then(|value| value.get("assets"))
        .filter(|value| !value.is_null())
        .and_then(|value| serde_json::from_value::<Vec<AssetRef>>(value.clone()).ok())
        .or_else(|| {
            DirectAssetStore::refs_for_message(None, images.as_deref(), attached_docs.as_deref())
        });
    let web_search_results = direct
        .and_then(|value| value.get("web_search_results"))
        .filter(|value| !value.is_null())
        .and_then(|value| serde_json::from_value(value.clone()).ok());

    FrontendMessage {
        id: entry.id,
        conversation_id: entry.conversation_id,
        role: entry.role,
        content: entry.content,
        images,
        assets,
        attached_docs,
        web_search_results,
        created_at: timestamp_to_millis(&entry.created_at),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_get_conversations(
    state: State<'_, SharedHistoryStore>,
) -> Result<Vec<Conversation>, String> {
    sqlx::query_as::<_, SharedConversationRow>(
        "SELECT id, metadata, started_at, last_activity FROM conversations \
         WHERE surface = ? ORDER BY \
         CAST(COALESCE(json_extract(metadata, '$.direct_workbench.sort_order'), 0) AS INTEGER) ASC, \
         last_activity DESC",
    )
    .bind(DIRECT_SURFACE)
    .fetch_all(state.pool())
    .await
    .map(|rows| rows.into_iter().map(conversation_from_row).collect())
    .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_create_conversation(
    state: State<'_, SharedHistoryStore>,
    title: String,
    project_id: Option<String>,
) -> Result<Conversation, String> {
    let id = Uuid::new_v4().to_string();
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

    let metadata = json!({
        "direct_workbench": {
            "title": conv.title,
            "project_id": conv.project_id,
            "sort_order": conv.sort_order,
            "created_at": conv.created_at,
            "updated_at": conv.updated_at,
        }
    });
    sqlx::query(
        "INSERT INTO conversations (\
         id, channel, user_id, actor_id, conversation_scope_id, conversation_kind, \
         surface, thread_id, stable_external_conversation_key, started_at, last_activity, metadata\
         ) VALUES (?, ?, ?, ?, ?, 'direct', ?, ?, ?, ?, ?, ?)",
    )
    .bind(&conv.id)
    .bind(DIRECT_CHANNEL)
    .bind(DIRECT_USER_ID)
    .bind(DIRECT_USER_ID)
    .bind(&conv.id)
    .bind(DIRECT_SURFACE)
    .bind(&conv.id)
    .bind(format!("{DIRECT_SURFACE}:{}", conv.id))
    .bind(millis_to_timestamp(conv.created_at))
    .bind(millis_to_timestamp(conv.updated_at))
    .bind(metadata.to_string())
    .execute(state.pool())
    .await
    .map_err(|e| e.to_string())?;

    Ok(conv)
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_delete_conversation(
    state: State<'_, SharedHistoryStore>,
    id: String,
) -> Result<(), String> {
    let id = state.require_direct_conversation(&id).await?;
    sqlx::query("DELETE FROM conversations WHERE id = ? AND surface = ?")
        .bind(id)
        .bind(DIRECT_SURFACE)
        .execute(state.pool())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_update_conversation_title(
    state: State<'_, SharedHistoryStore>,
    id: String,
    title: String,
) -> Result<(), String> {
    state
        .set_conversation_metadata_value(&id, "title", Value::String(title))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_update_conversation_project(
    state: State<'_, SharedHistoryStore>,
    id: String,
    project_id: Option<String>,
) -> Result<(), String> {
    state
        .set_conversation_metadata_value(
            &id,
            "project_id",
            project_id.map(Value::String).unwrap_or(Value::Null),
        )
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_update_conversations_order(
    state: State<'_, SharedHistoryStore>,
    orders: Vec<(String, i32)>,
) -> Result<(), String> {
    let mut tx = state.pool().begin().await.map_err(|e| e.to_string())?;
    for (id, order) in orders {
        let exact = exact_id(&id);
        let migrated = legacy_conversation_id(&id);
        let resolved: Option<(String, String)> = sqlx::query_as(
            "SELECT id, metadata FROM conversations WHERE (id = ? OR id = ?) AND surface = ? \
             ORDER BY CASE WHEN id = ? THEN 0 ELSE 1 END LIMIT 1",
        )
        .bind(&exact)
        .bind(&migrated)
        .bind(DIRECT_SURFACE)
        .bind(&exact)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        let Some((id, raw)) = resolved else {
            return Err("Direct Workbench conversation not found".to_string());
        };
        let mut metadata: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
        set_direct_metadata_value(&mut metadata, "sort_order", json!(order));
        sqlx::query("UPDATE conversations SET metadata = ? WHERE id = ? AND surface = ?")
            .bind(metadata.to_string())
            .bind(id)
            .bind(DIRECT_SURFACE)
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
    state: State<'_, SharedHistoryStore>,
    conversation_id: String,
    limit: Option<i32>,
    before_created_at: Option<f64>,
) -> Result<Vec<FrontendMessage>, String> {
    let limit = limit.unwrap_or(50) as i64;
    let conversation_id = state.require_direct_conversation(&conversation_id).await?;
    let before = before_created_at
        .map(|timestamp| millis_to_timestamp(timestamp as i64))
        .unwrap_or_else(|| "9999-12-31T23:59:59.999Z".to_string());

    let entries = sqlx::query_as::<_, MessageEntry>(
        "SELECT id, conversation_id, role, content, metadata, created_at \
         FROM conversation_messages WHERE conversation_id = ? AND created_at < ? \
         ORDER BY created_at DESC, rowid DESC LIMIT ?",
    )
    .bind(conversation_id)
    .bind(before)
    .bind(limit)
    .fetch_all(state.pool())
    .await
    .map_err(|e| e.to_string())?;

    // We fetch DESC for pagination logic (getting the *previous* messages), but frontend expects ASC (chronological)
    // so we reverse them before returning.
    let mut msgs_asc = entries;
    msgs_asc.reverse();

    let msgs = msgs_asc.into_iter().map(message_from_entry).collect();

    Ok(msgs)
}

#[tauri::command]
#[specta::specta]
// Tauri commands intentionally expose flat arguments for generated bindings.
#[allow(clippy::too_many_arguments)]
pub async fn direct_history_save_message(
    state: State<'_, SharedHistoryStore>,
    conversation_id: String,
    role: String,
    content: String,
    images: Option<Vec<String>>,
    assets: Option<Vec<AssetRef>>,
    attached_docs: Option<Vec<AttachedDoc>>,
    web_search_results: Option<Vec<crate::web_search::WebSearchResult>>,
) -> Result<String, String> {
    let conversation_id = state.require_direct_conversation(&conversation_id).await?;
    let id = Uuid::new_v4().to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let merged_assets =
        DirectAssetStore::refs_for_message(assets, images.as_deref(), attached_docs.as_deref());
    let metadata = json!({
        "direct_workbench": {
            "images": images,
            "assets": merged_assets,
            "attached_docs": attached_docs,
            "web_search_results": web_search_results,
        }
    });
    let now_timestamp = millis_to_timestamp(now);
    let mut tx = state.pool().begin().await.map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO conversation_messages (\
         id, conversation_id, role, content, actor_id, actor_display_name, raw_sender_id, metadata, created_at\
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&id)
    .bind(&conversation_id)
    .bind(&role)
    .bind(&content)
    .bind((role == "user").then_some(DIRECT_USER_ID))
    .bind((role == "user").then_some("Local user"))
    .bind((role == "user").then_some(DIRECT_USER_ID))
    .bind(metadata.to_string())
    .bind(&now_timestamp)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let raw: String = sqlx::query_scalar("SELECT metadata FROM conversations WHERE id = ?")
        .bind(&conversation_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let mut conversation_metadata: Value =
        serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
    set_direct_metadata_value(&mut conversation_metadata, "updated_at", json!(now));
    sqlx::query(
        "UPDATE conversations SET last_activity = ?, metadata = ? WHERE id = ? AND surface = ?",
    )
    .bind(&now_timestamp)
    .bind(conversation_metadata.to_string())
    .bind(&conversation_id)
    .bind(DIRECT_SURFACE)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;

    Ok(id)
}

#[tauri::command]
#[specta::specta]
pub async fn direct_history_edit_message(
    state: State<'_, SharedHistoryStore>,
    message_id: String,
    new_content: String,
) -> Result<(), String> {
    let mut tx = state.pool().begin().await.map_err(|e| e.to_string())?;
    let exact_message_id = exact_id(&message_id);
    let migrated_message_id = legacy_message_id(&message_id);

    // 1. Get the message's created_at and conversation_id
    let msg: MessageEntry = sqlx::query_as(
        "SELECT m.id, m.conversation_id, m.role, m.content, m.metadata, m.created_at \
         FROM conversation_messages m JOIN conversations c ON c.id = m.conversation_id \
         WHERE (m.id = ? OR m.id = ?) AND c.surface = ? \
         ORDER BY CASE WHEN m.id = ? THEN 0 ELSE 1 END LIMIT 1",
    )
    .bind(&exact_message_id)
    .bind(&migrated_message_id)
    .bind(DIRECT_SURFACE)
    .bind(&exact_message_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Message not found")?;
    let message_id = msg.id.clone();

    // 2. Update content
    sqlx::query("UPDATE conversation_messages SET content = ? WHERE id = ?")
        .bind(&new_content)
        .bind(&message_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    // 3. Delete subsequent messages (use >= to catch millisecond collisions,
    //    but exclude the edited message itself by ID)
    sqlx::query(
        "DELETE FROM conversation_messages \
         WHERE conversation_id = ? AND created_at >= ? AND id != ?",
    )
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

    let raw: String = sqlx::query_scalar("SELECT metadata FROM conversations WHERE id = ?")
        .bind(&msg.conversation_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let mut metadata: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
    set_direct_metadata_value(&mut metadata, "updated_at", json!(now));
    sqlx::query(
        "UPDATE conversations SET last_activity = ?, metadata = ? WHERE id = ? AND surface = ?",
    )
    .bind(millis_to_timestamp(now))
    .bind(metadata.to_string())
    .bind(&msg.conversation_id)
    .bind(DIRECT_SURFACE)
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
    history: State<'_, SharedHistoryStore>,
    vector_manager: State<'_, crate::vector_store::VectorStoreManager>,
    file_store: State<'_, FileStore>,
) -> Result<(), String> {
    println!("[history] direct_history_delete_all_history: clearing database...");
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    history.delete_all_direct().await?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn legacy_pool(temp: &TempDir) -> SqlitePool {
        let path = temp.path().join("legacy.db");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&format!("sqlite://{}?mode=rwc", path.to_string_lossy()))
            .await
            .expect("open legacy database");
        sqlx::query(
            "CREATE TABLE conversations (\
             id TEXT PRIMARY KEY, title TEXT NOT NULL, created_at INTEGER NOT NULL, \
             updated_at INTEGER NOT NULL, project_id TEXT, sort_order INTEGER DEFAULT 0)",
        )
        .execute(&pool)
        .await
        .expect("create legacy conversations");
        sqlx::query(
            "CREATE TABLE messages (\
             id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL, role TEXT NOT NULL, \
             content TEXT NOT NULL, images TEXT, assets TEXT, attached_docs TEXT, \
             web_search_results TEXT, created_at INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .expect("create legacy messages");
        pool
    }

    #[tokio::test]
    async fn legacy_merge_is_deterministic_idempotent_and_lossless() {
        let temp = TempDir::new().expect("temp dir");
        let legacy = legacy_pool(&temp).await;
        sqlx::query(
            "INSERT INTO conversations \
             (id, title, created_at, updated_at, project_id, sort_order) \
             VALUES ('legacy-chat', 'Imported chat', 1700000000000, 1700000001000, 'project-1', 7)",
        )
        .execute(&legacy)
        .await
        .expect("insert legacy conversation");
        let legacy_uuid = "8dbef749-6711-4be7-a9bf-56218dc66f69";
        sqlx::query(
            "INSERT INTO conversations \
             (id, title, created_at, updated_at, project_id, sort_order) \
             VALUES (?, 'UUID legacy chat', 1700000002000, 1700000003000, NULL, 0)",
        )
        .bind(legacy_uuid)
        .execute(&legacy)
        .await
        .expect("insert UUID legacy conversation");
        sqlx::query(
            "INSERT INTO messages \
             (id, conversation_id, role, content, images, assets, attached_docs, \
              web_search_results, created_at) \
             VALUES ('legacy-message', 'legacy-chat', 'user', 'hello', '[\"image.png\"]', \
                     '[{\"namespace\":\"direct_workbench\",\"id\":\"asset-1\"}]', \
                     '[{\"id\":\"doc-1\",\"name\":\"Doc\"}]', \
                     '[{\"title\":\"Result\",\"url\":\"https://example.com\",\"snippet\":\"S\"}]', \
                     1700000000500)",
        )
        .execute(&legacy)
        .await
        .expect("insert legacy message");
        sqlx::query(
            "INSERT INTO messages \
             (id, conversation_id, role, content, created_at) \
             VALUES ('orphan-message', 'missing-chat', 'user', 'orphan', 1700000000600)",
        )
        .execute(&legacy)
        .await
        .expect("insert orphaned legacy message");

        let store = SharedHistoryStore::open(temp.path(), &legacy)
            .await
            .expect("open shared store");
        store
            .migrate_legacy_history(&legacy)
            .await
            .expect("idempotent second migration");

        let expected_conversation_id = legacy_conversation_id("legacy-chat");
        let expected_message_id = legacy_message_id("legacy-message");
        let conversation: (String, String, String) = sqlx::query_as(
            "SELECT id, surface, metadata FROM conversations WHERE id = ?",
        )
        .bind(&expected_conversation_id)
        .fetch_one(store.pool())
        .await
        .expect("migrated conversation");
        assert_eq!(conversation.0, expected_conversation_id);
        assert_eq!(conversation.1, DIRECT_SURFACE);
        let metadata: Value = serde_json::from_str(&conversation.2).expect("conversation metadata");
        let direct = direct_metadata(&metadata).expect("direct metadata");
        assert_eq!(direct.get("title").and_then(Value::as_str), Some("Imported chat"));
        assert_eq!(direct.get("project_id").and_then(Value::as_str), Some("project-1"));
        assert_eq!(direct.get("sort_order").and_then(Value::as_i64), Some(7));

        let message: (String, String, String) = sqlx::query_as(
            "SELECT id, content, metadata FROM conversation_messages WHERE conversation_id = ?",
        )
        .bind(&expected_conversation_id)
        .fetch_one(store.pool())
        .await
        .expect("migrated message");
        assert_eq!(message.0, expected_message_id);
        assert_eq!(message.1, "hello");
        let metadata: Value = serde_json::from_str(&message.2).expect("message metadata");
        let direct = direct_metadata(&metadata).expect("direct metadata");
        assert_eq!(direct["images"][0], "image.png");
        assert_eq!(direct["assets"][0]["id"], "asset-1");
        assert_eq!(direct["attached_docs"][0]["id"], "doc-1");
        assert_eq!(direct["web_search_results"][0]["title"], "Result");

        let namespaced_uuid_id = legacy_conversation_id(legacy_uuid);
        assert_ne!(namespaced_uuid_id, legacy_uuid);
        let uuid_conversation_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM conversations WHERE id = ? AND surface = ?",
        )
        .bind(namespaced_uuid_id)
        .bind(DIRECT_SURFACE)
        .fetch_one(store.pool())
        .await
        .expect("UUID legacy conversation count");
        assert_eq!(uuid_conversation_count, 1);

        let conversation_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE surface = ?")
                .bind(DIRECT_SURFACE)
                .fetch_one(store.pool())
                .await
                .expect("conversation count");
        let message_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM conversation_messages")
                .fetch_one(store.pool())
                .await
                .expect("message count");
        assert_eq!(conversation_count, 2);
        assert_eq!(message_count, 1);
    }

    #[tokio::test]
    async fn direct_history_deletion_preserves_agent_surface() {
        let temp = TempDir::new().expect("temp dir");
        let legacy = legacy_pool(&temp).await;
        let store = SharedHistoryStore::open(temp.path(), &legacy)
            .await
            .expect("open shared store");

        for (id, surface) in [("agent-chat", "agent_cockpit"), ("direct-chat", DIRECT_SURFACE)] {
            sqlx::query(
                "INSERT INTO conversations (\
                 id, channel, user_id, conversation_kind, surface, metadata\
                 ) VALUES (?, 'desktop', 'local_user', 'direct', ?, \
                    '{\"direct_workbench\":{\"project_id\":\"project-1\"}}')",
            )
            .bind(id)
            .bind(surface)
            .execute(store.pool())
            .await
            .expect("insert surface conversation");
            sqlx::query(
                "INSERT INTO conversation_messages (id, conversation_id, role, content) \
                 VALUES (?, ?, 'user', 'hello')",
            )
            .bind(format!("{id}-message"))
            .bind(id)
            .execute(store.pool())
            .await
            .expect("insert surface message");
        }

        store
            .delete_project_conversations("project-1")
            .await
            .expect("delete Direct project history");
        sqlx::query(
            "INSERT INTO conversations (\
             id, channel, user_id, conversation_kind, surface, metadata\
             ) VALUES ('direct-chat-2', 'desktop', 'local_user', 'direct', ?, '{}')",
        )
        .bind(DIRECT_SURFACE)
        .execute(store.pool())
        .await
        .expect("insert second Direct conversation");
        store.delete_all_direct().await.expect("delete Direct history");
        let remaining: Vec<(String, String)> =
            sqlx::query_as("SELECT id, surface FROM conversations ORDER BY id")
                .fetch_all(store.pool())
                .await
                .expect("remaining surfaces");
        assert_eq!(remaining, vec![("agent-chat".to_string(), "agent_cockpit".to_string())]);
        let remaining_messages: Vec<(String, String)> =
            sqlx::query_as("SELECT id, conversation_id FROM conversation_messages ORDER BY id")
                .fetch_all(store.pool())
                .await
                .expect("remaining messages");
        assert_eq!(
            remaining_messages,
            vec![(
                "agent-chat-message".to_string(),
                "agent-chat".to_string()
            )]
        );
    }
}
