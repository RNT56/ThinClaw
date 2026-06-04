use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum LocalRuntimeKind {
    LlamaCpp,
    Mlx,
    Vllm,
    Ollama,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum RuntimeCapability {
    Chat,
    Embedding,
    Tts,
    Stt,
    Diffusion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum RuntimeExposurePolicy {
    DirectOnly,
    SharedWhenEnabled,
    NetworkExposed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum RuntimeReadiness {
    Ready,
    Starting,
    SetupRequired,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct LocalRuntimeEndpoint {
    pub base_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub context_size: Option<u32>,
    #[serde(default)]
    pub model_family: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct LocalRuntimeSnapshot {
    pub kind: LocalRuntimeKind,
    pub display_name: String,
    pub readiness: RuntimeReadiness,
    #[serde(default)]
    pub endpoint: Option<LocalRuntimeEndpoint>,
    /// Capabilities that are active and ready in the current runtime snapshot.
    #[serde(default)]
    pub capabilities: Vec<RuntimeCapability>,
    /// Capabilities this runtime family can support when the relevant local
    /// services are configured and running.
    #[serde(default)]
    pub supported_capabilities: Vec<RuntimeCapability>,
    pub exposure_policy: RuntimeExposurePolicy,
    #[serde(default)]
    pub unavailable_reason: Option<String>,
}

impl LocalRuntimeSnapshot {
    pub fn unavailable(
        kind: LocalRuntimeKind,
        display_name: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            display_name: display_name.into(),
            readiness: RuntimeReadiness::Unavailable,
            endpoint: None,
            capabilities: Vec::new(),
            supported_capabilities: Vec::new(),
            exposure_policy: RuntimeExposurePolicy::DirectOnly,
            unavailable_reason: Some(reason.into()),
        }
    }

    pub fn redacted_for_public_clients(mut self) -> Self {
        if let Some(endpoint) = self.endpoint.as_mut() {
            endpoint.api_key = None;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_snapshot_serializes_camel_case_wire_shape() {
        let snapshot =
            LocalRuntimeSnapshot::unavailable(LocalRuntimeKind::Mlx, "MLX", "not started");
        let value = serde_json::to_value(snapshot).unwrap();
        assert_eq!(value["kind"], "mlx");
        assert_eq!(value["displayName"], "MLX");
        assert_eq!(value["unavailableReason"], "not started");
    }
}
