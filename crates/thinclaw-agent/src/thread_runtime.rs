//! Root-independent helpers for thread runtime metadata envelopes.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, Weak};

use serde::Serialize;
use serde::de::DeserializeOwned;
use thinclaw_types::error::DatabaseError;
use tokio::sync::{Mutex, OwnedMutexGuard};
use uuid::Uuid;

pub const THREAD_RUNTIME_METADATA_KEY: &str = "runtime";

static RUNTIME_MUTATION_LOCKS: OnceLock<Mutex<HashMap<Uuid, Weak<Mutex<()>>>>> = OnceLock::new();

/// Acquire the process-wide mutation lock for one thread runtime envelope.
///
/// The desktop runtime has several root and portable adapters that all update
/// different fields of the same JSON object. Serializing their read/modify/
/// write cycles prevents a late writer from silently clobbering another
/// subsystem's approval, prompt, history, or subagent state.
pub async fn acquire_runtime_mutation_lock(thread_id: Uuid) -> OwnedMutexGuard<()> {
    let locks = RUNTIME_MUTATION_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let lock = {
        let mut locks = locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        match locks.get(&thread_id).and_then(Weak::upgrade) {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(Mutex::new(()));
                locks.insert(thread_id, Arc::downgrade(&lock));
                lock
            }
        }
    };
    lock.lock_owned().await
}

pub fn decode_thread_runtime<T>(metadata: &serde_json::Value) -> Result<Option<T>, DatabaseError>
where
    T: DeserializeOwned,
{
    let runtime = metadata
        .get(THREAD_RUNTIME_METADATA_KEY)
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    if runtime.is_null() {
        return Ok(None);
    }

    serde_json::from_value(runtime).map(Some).map_err(|error| {
        DatabaseError::Serialization(format!("thread runtime decode failed: {error}"))
    })
}

pub fn encode_thread_runtime<T>(runtime: &T) -> Result<serde_json::Value, DatabaseError>
where
    T: Serialize,
{
    serde_json::to_value(runtime).map_err(|error| {
        DatabaseError::Serialization(format!("thread runtime encode failed: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::ThreadRuntimeSnapshot;

    #[test]
    fn missing_runtime_metadata_decodes_to_none() {
        let decoded: Option<ThreadRuntimeSnapshot> =
            decode_thread_runtime(&serde_json::json!({})).unwrap();
        assert!(decoded.is_none());
    }

    #[test]
    fn runtime_round_trips_through_metadata_value() {
        let runtime = ThreadRuntimeSnapshot {
            owner_agent_id: Some("agent-1".to_string()),
            auto_approved_tools: vec!["echo".to_string()],
            ..Default::default()
        };

        let value = encode_thread_runtime(&runtime).unwrap();
        let decoded: ThreadRuntimeSnapshot = decode_thread_runtime(&serde_json::json!({
            THREAD_RUNTIME_METADATA_KEY: value
        }))
        .unwrap()
        .unwrap();

        assert_eq!(decoded.owner_agent_id.as_deref(), Some("agent-1"));
        assert_eq!(decoded.auto_approved_tools, ["echo"]);
    }
}
