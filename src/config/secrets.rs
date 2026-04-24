use secrecy::{ExposeSecret, SecretString};

use crate::config::helpers::optional_env;
use crate::error::ConfigError;

/// Secrets management configuration.
#[derive(Clone, Default)]
pub struct SecretsConfig {
    /// Master key for encrypting secrets.
    pub master_key: Option<SecretString>,
    /// Whether secrets management is enabled.
    pub enabled: bool,
    /// Source of the master key.
    pub source: crate::settings::KeySource,
}

impl std::fmt::Debug for SecretsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretsConfig")
            .field("master_key", &self.master_key.is_some())
            .field("enabled", &self.enabled)
            .field("source", &self.source)
            .finish()
    }
}

impl SecretsConfig {
    /// Resolve the secrets master key according to the strict secrets settings.
    ///
    /// The default source is the OS secure store. `SECRETS_MASTER_KEY` is only
    /// honored when settings or `THINCLAW_ALLOW_ENV_MASTER_KEY=1` explicitly
    /// allow the environment fallback.
    pub(crate) async fn resolve(settings: &crate::settings::Settings) -> Result<Self, ConfigError> {
        use crate::settings::{KeySource, SecretsMasterKeySource};

        let env_allowed = settings.secrets.allow_env_master_key
            || std::env::var("THINCLAW_ALLOW_ENV_MASTER_KEY")
                .ok()
                .map(|value| {
                    matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "1" | "true" | "yes" | "on"
                    )
                })
                .unwrap_or(false);

        let (master_key, source) = match settings.secrets.master_key_source {
            SecretsMasterKeySource::None => (None, KeySource::None),
            SecretsMasterKeySource::Env if env_allowed => {
                if let Some(env_key) = optional_env("SECRETS_MASTER_KEY")? {
                    (Some(SecretString::from(env_key)), KeySource::Env)
                } else {
                    (None, KeySource::None)
                }
            }
            SecretsMasterKeySource::Env => {
                tracing::warn!(
                    "SECRETS_MASTER_KEY ignored because env master keys are disabled; set secrets.allow_env_master_key=true or THINCLAW_ALLOW_ENV_MASTER_KEY=1 to allow it"
                );
                (None, KeySource::None)
            }
            SecretsMasterKeySource::OsSecureStore => {
                match crate::platform::secure_store::get_master_key().await {
                    Ok(key_bytes) => {
                        let key_hex: String =
                            key_bytes.iter().map(|b| format!("{:02x}", b)).collect();
                        (Some(SecretString::from(key_hex)), KeySource::Keychain)
                    }
                    Err(_) if env_allowed => {
                        if let Some(env_key) = optional_env("SECRETS_MASTER_KEY")? {
                            tracing::warn!(
                                "Using SECRETS_MASTER_KEY fallback because OS secure store key is unavailable and env fallback is explicitly allowed"
                            );
                            (Some(SecretString::from(env_key)), KeySource::Env)
                        } else {
                            (None, KeySource::None)
                        }
                    }
                    Err(_) => (None, KeySource::None),
                }
            }
        };

        let enabled = master_key.is_some();

        if let Some(ref key) = master_key
            && key.expose_secret().len() < 32
        {
            return Err(ConfigError::InvalidValue {
                key: "SECRETS_MASTER_KEY".to_string(),
                message: "must be at least 32 bytes for AES-256-GCM".to_string(),
            });
        }

        Ok(Self {
            master_key,
            enabled,
            source,
        })
    }

    /// Get the master key if configured.
    pub fn master_key(&self) -> Option<&SecretString> {
        self.master_key.as_ref()
    }
}
