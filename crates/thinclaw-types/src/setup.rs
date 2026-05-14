use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SetupAuthMode {
    #[default]
    None,
    ManualSecrets,
    OAuth,
    SharedOAuth,
    NativePlugin,
    RemoteSecretBackend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SetupState {
    NotInstalled,
    #[default]
    InstalledUnconfigured,
    NeedsAuth,
    Ready,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupAction {
    Install,
    ConfigureSecrets,
    StartOAuth,
    Validate,
    Activate,
    Disable,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupSecretDescriptor {
    pub name: String,
    pub prompt: String,
    #[serde(default)]
    pub optional: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntegrationSetupStatus {
    pub state: SetupState,
    pub auth_mode: SetupAuthMode,
    pub actions: Vec<SetupAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_secrets: Vec<SetupSecretDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl Default for IntegrationSetupStatus {
    fn default() -> Self {
        Self {
            state: SetupState::InstalledUnconfigured,
            auth_mode: SetupAuthMode::None,
            actions: vec![SetupAction::Validate],
            required_secrets: Vec::new(),
            setup_url: None,
            validation_url: None,
            message: None,
        }
    }
}

impl IntegrationSetupStatus {
    pub fn ready(auth_mode: SetupAuthMode) -> Self {
        Self {
            state: SetupState::Ready,
            auth_mode,
            actions: vec![
                SetupAction::Validate,
                SetupAction::Disable,
                SetupAction::Remove,
            ],
            ..Self::default()
        }
    }

    pub fn not_installed(auth_mode: SetupAuthMode) -> Self {
        Self {
            state: SetupState::NotInstalled,
            auth_mode,
            actions: vec![SetupAction::Install],
            ..Self::default()
        }
    }

    pub fn failed(auth_mode: SetupAuthMode, message: impl Into<String>) -> Self {
        Self {
            state: SetupState::Failed,
            auth_mode,
            actions: vec![
                SetupAction::Validate,
                SetupAction::Activate,
                SetupAction::Remove,
            ],
            message: Some(message.into()),
            ..Self::default()
        }
    }
}
