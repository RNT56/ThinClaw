use std::sync::Arc;

use uuid::Uuid;

use crate::agent::ThreadRuntimeState;
use crate::db::Database;
use crate::error::DatabaseError;

pub use thinclaw_agent::thread_runtime::THREAD_RUNTIME_METADATA_KEY;

pub async fn load_thread_runtime(
    store: &Arc<dyn Database>,
    thread_id: Uuid,
) -> Result<Option<ThreadRuntimeState>, DatabaseError> {
    let metadata = match store.get_conversation_metadata(thread_id).await? {
        Some(metadata) => metadata,
        None => return Ok(None),
    };
    thinclaw_agent::thread_runtime::decode_thread_runtime(&metadata)
}

pub async fn save_thread_runtime(
    store: &Arc<dyn Database>,
    thread_id: Uuid,
    runtime: &ThreadRuntimeState,
) -> Result<(), DatabaseError> {
    let value = thinclaw_agent::thread_runtime::encode_thread_runtime(runtime)?;
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
