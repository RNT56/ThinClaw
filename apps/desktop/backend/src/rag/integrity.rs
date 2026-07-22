use super::*;

pub async fn list_project_files(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<Vec<String>, String> {
    sqlx::query_scalar("SELECT path FROM documents WHERE project_id = ? ORDER BY path ASC LIMIT 50")
        .bind(project_id)
        .fetch_all(pool)
        .await
        .map_err(|error| format!("Failed to list project files: {error}"))
}

pub async fn perform_integrity_check(
    pool: &SqlitePool,
    vector_manager: &crate::vector_store::VectorStoreManager,
) -> Result<String, String> {
    let _guard = vector_manager.lock_updates().await;
    let persisted_profile: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
            .bind(EMBEDDING_PROFILE_SETTING)
            .fetch_optional(pool)
            .await
            .map_err(|error| format!("Failed to read embedding profile: {error}"))?;
    let profile =
        persisted_profile.unwrap_or_else(|| format!("unselected:{}", vector_manager.dimensions()));
    let invalidated = activate_embedding_profile_locked(
        pool,
        vector_manager,
        &profile,
        vector_manager.dimensions(),
    )
    .await?;
    if invalidated {
        return Ok(
            "invalidated incompatible or unprofiled embeddings; FTS remains available".to_string(),
        );
    }

    let scope_rows = sqlx::query(
        "SELECT DISTINCT d.project_id, d.chat_id FROM documents d JOIN chunks c ON c.document_id = d.id WHERE c.embedding IS NOT NULL AND c.embedding_profile = ? LIMIT 10001",
    )
    .bind(&profile)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("Failed to enumerate vector scopes: {error}"))?;
    if scope_rows.len() > 10_000 {
        return Err("Vector scope count exceeds the integrity-check limit".to_string());
    }

    vector_manager.reset_all()?;
    let mut scopes = std::collections::HashSet::new();
    for row in scope_rows {
        let project_id: Option<String> = row.get("project_id");
        let chat_id: Option<String> = row.get("chat_id");
        scopes.insert(crate::vector_store::VectorStoreManager::scope_for(
            &project_id,
            &chat_id,
        ));
    }
    let mut rebuilt = 0_usize;
    for scope in &scopes {
        rebuilt = rebuilt.saturating_add(
            rebuild_vector_scope_locked(pool, vector_manager, scope, &profile).await?,
        );
    }

    let expected: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM chunks WHERE embedding IS NOT NULL AND embedding_profile = ?",
    )
    .bind(&profile)
    .fetch_one(pool)
    .await
    .map_err(|error| format!("Failed to count stored embeddings: {error}"))?;
    if i64::try_from(rebuilt).unwrap_or(i64::MAX) != expected {
        return Err(format!(
            "Vector integrity rebuild produced {rebuilt} entries for {expected} stored embeddings"
        ));
    }
    sqlx::query(
        "UPDATE documents SET status = 'indexed', updated_at = ? WHERE EXISTS (SELECT 1 FROM chunks c WHERE c.document_id = documents.id AND c.embedding IS NOT NULL AND c.embedding_profile = ?)",
    )
    .bind(unix_timestamp_millis())
    .bind(&profile)
    .execute(pool)
    .await
    .map_err(|error| format!("Failed to finalize rebuilt documents: {error}"))?;

    Ok(format!(
        "ok: rebuilt {rebuilt} vectors across {} scopes",
        scopes.len()
    ))
}
