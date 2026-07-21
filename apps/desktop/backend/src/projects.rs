use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::{FromRow, SqlitePool};
use tauri::State;

const MAX_PROJECT_NAME_BYTES: usize = 256;
const MAX_PROJECT_DESCRIPTION_BYTES: usize = 16 * 1024;
const MAX_PROJECT_ORDER_UPDATES: usize = 10_000;

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

fn validate_project_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > MAX_PROJECT_NAME_BYTES || name.contains('\0') {
        Err("Project name is invalid".to_string())
    } else {
        Ok(())
    }
}

fn validate_project_description(description: &str) -> Result<(), String> {
    if description.len() > MAX_PROJECT_DESCRIPTION_BYTES || description.contains('\0') {
        Err("Project description exceeds the supported limit or contains NUL".to_string())
    } else {
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, Type, FromRow)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[specta(type = f64)]
    pub created_at: i64,
    #[specta(type = f64)]
    pub updated_at: i64,
    pub sort_order: i32,
}

#[derive(Debug, Serialize, Deserialize, Type, FromRow)]
pub struct Document {
    pub id: String,
    pub path: String,
    pub status: String,
    #[specta(type = f64)]
    pub created_at: i64,
    #[specta(type = f64)]
    pub updated_at: i64,
    pub project_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Type)]
pub struct CreateProjectRequest {
    pub name: String,
    pub description: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn create_project(
    pool: State<'_, SqlitePool>,
    mut request: CreateProjectRequest,
) -> Result<Project, String> {
    validate_project_name(&request.name)?;
    if let Some(description) = &request.description {
        validate_project_description(description)?;
    }
    request.name = request.name.trim().to_string();
    let id = uuid::Uuid::new_v4().to_string();
    let now = unix_timestamp_millis();

    let project = Project {
        id: id.clone(),
        name: request.name,
        description: request.description,
        created_at: now,
        updated_at: now,
        sort_order: 0,
    };

    sqlx::query("INSERT INTO projects (id, name, description, created_at, updated_at, sort_order) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&project.id)
        .bind(&project.name)
        .bind(&project.description)
        .bind(project.created_at)
        .bind(project.updated_at)
        .bind(project.sort_order)
        .execute(pool.inner())
        .await
        .map_err(|e| e.to_string())?;

    Ok(project)
}

#[tauri::command]
#[specta::specta]
pub async fn list_projects(pool: State<'_, SqlitePool>) -> Result<Vec<Project>, String> {
    let projects = sqlx::query_as::<_, Project>(
        "SELECT * FROM projects ORDER BY sort_order ASC, updated_at DESC LIMIT 10001",
    )
    .fetch_all(pool.inner())
    .await
    .map_err(|e| e.to_string())?;

    if projects.len() > 10_000 {
        return Err("Project count exceeds the supported limit".to_string());
    }

    Ok(projects)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_project(
    pool: State<'_, SqlitePool>,
    vector_manager: State<'_, crate::vector_store::VectorStoreManager>,
    file_store: State<'_, crate::file_store::FileStore>,
    id: String,
) -> Result<(), String> {
    validate_identifier("Project", &id)?;
    let update_guard = vector_manager.lock_updates().await;
    let conversation_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM conversations WHERE project_id = ? LIMIT 10001")
            .bind(&id)
            .fetch_all(pool.inner())
            .await
            .map_err(|error| error.to_string())?;
    if conversation_ids.len() > 10_000 {
        return Err("Project conversation count exceeds the deletion limit".to_string());
    }
    let document_paths: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT path FROM documents WHERE project_id = ? OR (project_id IS NULL AND chat_id IN (SELECT id FROM conversations WHERE project_id = ?))",
    )
    .bind(&id)
    .bind(&id)
    .fetch_all(pool.inner())
    .await
    .map_err(|error| error.to_string())?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    // 1. Delete messages associated with conversations in this project
    // This is important because while conversations have ON DELETE CASCADE from messages,
    // we want to be explicit about what's happening.
    sqlx::query("DELETE FROM messages WHERE conversation_id IN (SELECT id FROM conversations WHERE project_id = ?)")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM chat_summaries WHERE conversation_id IN (SELECT id FROM conversations WHERE project_id = ?)")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    // Delete project documents and any legacy chat-only documents owned by a
    // conversation that is about to be removed.
    sqlx::query(
        "DELETE FROM chunks WHERE document_id IN (SELECT id FROM documents WHERE project_id = ? OR (project_id IS NULL AND chat_id IN (SELECT id FROM conversations WHERE project_id = ?)))",
    )
    .bind(&id)
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "DELETE FROM direct_assets WHERE namespace = 'direct_workbench' AND id IN (SELECT id FROM documents WHERE project_id = ? OR (project_id IS NULL AND chat_id IN (SELECT id FROM conversations WHERE project_id = ?)))",
    )
    .bind(&id)
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "DELETE FROM documents WHERE project_id = ? OR (project_id IS NULL AND chat_id IN (SELECT id FROM conversations WHERE project_id = ?))",
    )
        .bind(&id)
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM conversations WHERE project_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    // 5. Delete the project itself
    sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;

    // 6. Delete the project's scoped vector index file
    let scope = crate::vector_store::VectorScope::Project(id.clone());
    vector_manager.delete_scope(&scope).map_err(|error| {
        format!("Project was deleted but its vector scope could not be removed: {error}")
    })?;
    for conversation_id in conversation_ids {
        vector_manager
            .delete_scope(&crate::vector_store::VectorScope::Chat(conversation_id))
            .map_err(|error| {
                format!("Project was deleted but a legacy chat index remains: {error}")
            })?;
    }
    drop(update_guard);

    let mut cleanup_failures = 0_usize;
    for path in document_paths {
        let still_referenced: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM documents WHERE path = ?)")
                .bind(&path)
                .fetch_one(pool.inner())
                .await
                .map_err(|error| error.to_string())?;
        if !still_referenced {
            if let Err(error) = file_store
                .delete_absolute(std::path::Path::new(&path))
                .await
            {
                cleanup_failures = cleanup_failures.saturating_add(1);
                tracing::warn!("[projects] Could not remove deleted project asset: {error}");
            }
        }
    }

    if cleanup_failures > 0 {
        return Err(format!(
            "Project data was deleted, but {cleanup_failures} backing file(s) could not be removed"
        ));
    }

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn update_project(
    pool: State<'_, SqlitePool>,
    id: String,
    name: Option<String>,
    description: Option<String>,
) -> Result<Project, String> {
    validate_identifier("Project", &id)?;
    if let Some(name) = &name {
        validate_project_name(name)?;
    }
    if let Some(description) = &description {
        validate_project_description(description)?;
    }
    let now = unix_timestamp_millis();

    // Dynamic query helper or just simple if checks?
    // Let's fetch first to maintain existing values if None passed
    let mut project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id = ?")
        .bind(&id)
        .fetch_one(pool.inner())
        .await
        .map_err(|e| e.to_string())?;

    if let Some(n) = name {
        project.name = n.trim().to_string();
    }
    if let Some(d) = description {
        project.description = Some(d);
    }
    project.updated_at = now;

    sqlx::query("UPDATE projects SET name = ?, description = ?, updated_at = ? WHERE id = ?")
        .bind(&project.name)
        .bind(&project.description)
        .bind(project.updated_at)
        .bind(&id)
        .execute(pool.inner())
        .await
        .map_err(|e| e.to_string())?;

    Ok(project)
}

#[tauri::command]
#[specta::specta]
pub async fn get_project_documents(
    pool: State<'_, SqlitePool>,
    project_id: String,
) -> Result<Vec<Document>, String> {
    validate_identifier("Project", &project_id)?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?)")
        .bind(&project_id)
        .fetch_one(pool.inner())
        .await
        .map_err(|error| error.to_string())?;
    if !exists {
        return Err("Project does not exist".to_string());
    }
    let docs = sqlx::query_as::<_, Document>("SELECT id, path, status, created_at, updated_at, project_id FROM documents WHERE project_id = ? ORDER BY created_at DESC LIMIT 10001")
        .bind(&project_id)
        .fetch_all(pool.inner())
        .await
        .map_err(|e| e.to_string())?;
    if docs.len() > 10_000 {
        return Err("Project document count exceeds the supported limit".to_string());
    }
    Ok(docs)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_document(
    pool: State<'_, SqlitePool>,
    vector_manager: State<'_, crate::vector_store::VectorStoreManager>,
    file_store: State<'_, crate::file_store::FileStore>,
    id: String,
) -> Result<(), String> {
    validate_identifier("Document", &id)?;
    let update_guard = vector_manager.lock_updates().await;
    let document: Option<(String, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT path, project_id, chat_id FROM documents WHERE id = ?")
            .bind(&id)
            .fetch_optional(pool.inner())
            .await
            .map_err(|e| e.to_string())?;
    let Some((doc_path, project_id, chat_id)) = document else {
        return Ok(());
    };
    let scope = crate::vector_store::VectorStoreManager::scope_for(&project_id, &chat_id);

    let mut transaction = pool.begin().await.map_err(|error| error.to_string())?;
    sqlx::query("DELETE FROM chunks WHERE document_id = ?")
        .bind(&id)
        .execute(&mut *transaction)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM documents WHERE id = ?")
        .bind(&id)
        .execute(&mut *transaction)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM direct_assets WHERE namespace = 'direct_workbench' AND id = ?")
        .bind(&id)
        .execute(&mut *transaction)
        .await
        .map_err(|e| e.to_string())?;
    transaction
        .commit()
        .await
        .map_err(|error| error.to_string())?;

    if let Err(error) = crate::rag::rebuild_vector_scope_with_lock_held(
        pool.inner(),
        vector_manager.inner(),
        &scope,
    )
    .await
    {
        // Fail closed: an empty scope loses semantic search temporarily but
        // cannot return the deleted document as a ghost vector.
        let _ = vector_manager.delete_scope(&scope);
        return Err(format!(
            "Document was deleted but its vector scope could not be rebuilt: {error}"
        ));
    }
    drop(update_guard);

    let path_still_referenced: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM documents WHERE path = ?)")
            .bind(&doc_path)
            .fetch_one(pool.inner())
            .await
            .map_err(|error| error.to_string())?;
    if !path_still_referenced {
        file_store
            .delete_absolute(std::path::Path::new(&doc_path))
            .await
            .map_err(|error| {
                format!("Document metadata was deleted, but its backing file remains: {error}")
            })?;
    }

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn update_projects_order(
    pool: State<'_, SqlitePool>,
    orders: Vec<(String, i32)>,
) -> Result<(), String> {
    if orders.len() > MAX_PROJECT_ORDER_UPDATES {
        return Err("Project order update exceeds the supported limit".to_string());
    }
    let mut seen = std::collections::HashSet::with_capacity(orders.len());
    for (id, order) in &orders {
        validate_identifier("Project", id)?;
        if *order < 0 || !seen.insert(id) {
            return Err("Project order entries are invalid or duplicated".to_string());
        }
    }
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    for (id, order) in orders {
        let result = sqlx::query("UPDATE projects SET sort_order = ? WHERE id = ?")
            .bind(order)
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        if result.rows_affected() != 1 {
            return Err(format!("Project {id} does not exist"));
        }
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}
