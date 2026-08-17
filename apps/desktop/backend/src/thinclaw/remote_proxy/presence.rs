//! Transient desktop presence published alongside the remote SSE session.

use crate::thinclaw::bridge::BridgeError;

use super::core::RemoteGatewayProxy;

pub(super) const DESKTOP_PRESENCE_TTL_SECONDS: u64 = 45;
pub(super) const DESKTOP_PRESENCE_REFRESH_SECONDS: u64 = 20;

fn online_payload() -> serde_json::Value {
    serde_json::json!({
        "state": "online",
        "surface": "desktop",
        "ttl_seconds": DESKTOP_PRESENCE_TTL_SECONDS,
    })
}

impl RemoteGatewayProxy {
    pub(super) async fn publish_presence(&self) -> Result<(), BridgeError> {
        self.put_json(
            &format!("/api/presence/{}", self.inner.presence_session_id),
            &online_payload(),
        )
        .await?;
        Ok(())
    }

    pub(super) async fn clear_presence(&self) -> Result<(), BridgeError> {
        self.delete_json(&format!(
            "/api/presence/{}",
            self.inner.presence_session_id
        ))
        .await?;
        Ok(())
    }

    pub(super) async fn presence_snapshot(&self) -> Result<serde_json::Value, BridgeError> {
        self.get_json("/api/presence").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_presence_is_bounded_and_content_free() {
        let payload = online_payload();
        assert_eq!(payload["state"], "online");
        assert_eq!(payload["surface"], "desktop");
        assert_eq!(payload["ttl_seconds"], DESKTOP_PRESENCE_TTL_SECONDS);
        assert_eq!(payload.as_object().unwrap().len(), 3);
        assert!(DESKTOP_PRESENCE_REFRESH_SECONDS < DESKTOP_PRESENCE_TTL_SECONDS);
    }
}
