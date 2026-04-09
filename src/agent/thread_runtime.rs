use std::sync::Arc;

use uuid::Uuid;

use crate::agent::ThreadRuntimeState;
use crate::db::Database;
use crate::error::DatabaseError;

pub const THREAD_RUNTIME_METADATA_KEY: &str = "runtime";

pub async fn load_thread_runtime(
    store: &Arc<dyn Database>,
    thread_id: Uuid,
) -> Result<Option<ThreadRuntimeState>, DatabaseError> {
    let metadata = match store.get_conversation_metadata(thread_id).await? {
        Some(metadata) => metadata,
        None => return Ok(None),
    };
    let runtime = metadata
        .get(THREAD_RUNTIME_METADATA_KEY)
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    if runtime.is_null() {
        return Ok(None);
    }
    let parsed = serde_json::from_value(runtime)
        .map_err(|e| DatabaseError::Serialization(format!("thread runtime decode failed: {e}")))?;
    Ok(Some(parsed))
}

pub async fn save_thread_runtime(
    store: &Arc<dyn Database>,
    thread_id: Uuid,
    runtime: &ThreadRuntimeState,
) -> Result<(), DatabaseError> {
    let value = serde_json::to_value(runtime)
        .map_err(|e| DatabaseError::Serialization(format!("thread runtime encode failed: {e}")))?;
    store
        .update_conversation_metadata_field(thread_id, THREAD_RUNTIME_METADATA_KEY, &value)
        .await
}

pub async fn mutate_thread_runtime<F>(
    store: &Arc<dyn Database>,
    thread_id: Uuid,
    mutate: F,
) -> Result<ThreadRuntimeState, DatabaseError>
where
    F: FnOnce(&mut ThreadRuntimeState),
{
    let mut runtime = load_thread_runtime(store, thread_id)
        .await?
        .unwrap_or_default();
    mutate(&mut runtime);
    save_thread_runtime(store, thread_id, &runtime).await?;
    Ok(runtime)
}
