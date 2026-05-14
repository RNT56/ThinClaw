//! Root-independent helpers for thread runtime metadata envelopes.

use serde::Serialize;
use serde::de::DeserializeOwned;
use thinclaw_types::error::DatabaseError;

pub const THREAD_RUNTIME_METADATA_KEY: &str = "runtime";

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
