use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::{FromRow, SqlitePool};
use tauri::State;

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
    request: CreateProjectRequest,
) -> Result<Project, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

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
        "SELECT * FROM projects ORDER BY sort_order ASC, updated_at DESC",
    )
    .fetch_all(pool.inner())
    .await
    .map_err(|e| e.to_string())?;

    Ok(projects)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_project(
    pool: State<'_, SqlitePool>,
    vector_manager: State<'_, crate::vector_store::VectorStoreManager>,
    id: String,
) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    // 1. Delete messages associated with conversations in this project
    // This is important because while conversations have ON DELETE CASCADE from messages,
    // we want to be explicit about what's happening.
    sqlx::query("DELETE FROM messages WHERE conversation_id IN (SELECT id FROM conversations WHERE project_id = ?)")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    // 2. Delete conversations
    sqlx::query("DELETE FROM conversations WHERE project_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    // 3. Delete chunks for documents in this project
    sqlx::query(
        "DELETE FROM chunks WHERE document_id IN (SELECT id FROM documents WHERE project_id = ?)",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    // 4. Delete documents
    sqlx::query("DELETE FROM documents WHERE project_id = ?")
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
    let scope = crate::vector_store::VectorScope::Project(id);
    if let Err(e) = vector_manager.delete_scope(&scope) {
        eprintln!(
            "[projects] Failed to delete vector scope {:?}: {}",
            scope, e
        );
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
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    // Dynamic query helper or just simple if checks?
    // Let's fetch first to maintain existing values if None passed
    let mut project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id = ?")
        .bind(&id)
        .fetch_one(pool.inner())
        .await
        .map_err(|e| e.to_string())?;

    if let Some(n) = name {
        project.name = n;
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
    let docs = sqlx::query_as::<_, Document>("SELECT id, path, status, created_at, updated_at, project_id FROM documents WHERE project_id = ? ORDER BY created_at DESC")
        .bind(project_id)
        .fetch_all(pool.inner())
        .await
        .map_err(|e| e.to_string())?;
    Ok(docs)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_document(pool: State<'_, SqlitePool>, id: String) -> Result<(), String> {
    // 1. Look up document path before deleting (for file cleanup)
    let doc_path: Option<(String,)> = sqlx::query_as("SELECT path FROM documents WHERE id = ?")
        .bind(&id)
        .fetch_optional(pool.inner())
        .await
        .map_err(|e| e.to_string())?;

    // 2. Count chunks for vector store diagnostics
    let chunk_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM chunks WHERE document_id = ?")
        .bind(&id)
        .fetch_one(pool.inner())
        .await
        .map_err(|e| e.to_string())?;

    // 3. Delete chunks (DB records — vector entries remain as ghosts since
    //    USearch does not support efficient per-key removal)
    sqlx::query("DELETE FROM chunks WHERE document_id = ?")
        .bind(&id)
        .execute(pool.inner())
        .await
        .map_err(|e| e.to_string())?;

    // 4. Delete document record
    sqlx::query("DELETE FROM documents WHERE id = ?")
        .bind(&id)
        .execute(pool.inner())
        .await
        .map_err(|e| e.to_string())?;

    if chunk_count.0 > 0 {
        println!(
            "[projects] deleted document {} ({} chunks removed from DB; vectors remain as ghosts until scope reset)",
            id, chunk_count.0
        );
    }

    // 5. Clean up source file from disk
    if let Some((path,)) = doc_path {
        if std::path::Path::new(&path).exists() {
            match tokio::fs::remove_file(&path).await {
                Ok(()) => println!("[projects] deleted document file: {}", path),
                Err(e) => eprintln!("[projects] failed to delete file {}: {}", path, e),
            }
        }
    }

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn update_projects_order(
    pool: State<'_, SqlitePool>,
    orders: Vec<(String, i32)>,
) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    for (id, order) in orders {
        sqlx::query("UPDATE projects SET sort_order = ? WHERE id = ?")
            .bind(order)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}
