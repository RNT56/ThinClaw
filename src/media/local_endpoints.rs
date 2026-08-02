//! In-process registry for managed authenticated loopback media endpoints.

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, RwLock};

use secrecy::{ExposeSecret, SecretString};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ManagedLocalEndpointKind {
    SpeechToText,
}

#[derive(Clone)]
pub struct ManagedLocalEndpoint {
    id: String,
    endpoint: String,
    model: String,
    credential: Arc<SecretString>,
}

impl std::fmt::Debug for ManagedLocalEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedLocalEndpoint")
            .field("id", &self.id)
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("credential", &"[REDACTED]")
            .finish()
    }
}

impl ManagedLocalEndpoint {
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn credential(&self) -> &str {
        self.credential.expose_secret()
    }
}

#[derive(Default)]
pub struct ManagedLocalEndpointRegistry {
    endpoints: RwLock<BTreeMap<ManagedLocalEndpointKind, ManagedLocalEndpoint>>,
}

impl ManagedLocalEndpointRegistry {
    pub fn install(
        &self,
        kind: ManagedLocalEndpointKind,
        id: impl Into<String>,
        endpoint: impl Into<String>,
        model: impl Into<String>,
        credential: SecretString,
    ) {
        self.endpoints
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                kind,
                ManagedLocalEndpoint {
                    id: id.into(),
                    endpoint: endpoint.into(),
                    model: model.into(),
                    credential: Arc::new(credential),
                },
            );
    }

    pub fn remove(&self, kind: ManagedLocalEndpointKind) {
        self.endpoints
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&kind);
    }

    pub fn get(&self, kind: ManagedLocalEndpointKind) -> Option<ManagedLocalEndpoint> {
        self.endpoints
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&kind)
            .cloned()
    }
}

static MANAGED_LOCAL_ENDPOINTS: LazyLock<ManagedLocalEndpointRegistry> =
    LazyLock::new(ManagedLocalEndpointRegistry::default);

pub fn managed_local_endpoints() -> &'static ManagedLocalEndpointRegistry {
    &MANAGED_LOCAL_ENDPOINTS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_exposes_credential() {
        let registry = ManagedLocalEndpointRegistry::default();
        registry.install(
            ManagedLocalEndpointKind::SpeechToText,
            "test",
            "http://127.0.0.1:1/v1/audio/transcriptions",
            "whisper",
            SecretString::from("endpoint-secret-sentinel"),
        );
        let endpoint = registry
            .get(ManagedLocalEndpointKind::SpeechToText)
            .unwrap();
        assert!(!format!("{endpoint:?}").contains("endpoint-secret-sentinel"));
    }
}
