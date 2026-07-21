//! Secrets management configuration.

use secrecy::{ExposeSecret, SecretString};
use thinclaw_settings::{KeySource, SecretsMasterKeySource, Settings};
use thinclaw_types::error::ConfigError;

use crate::helpers::optional_env;

/// Secrets management configuration.
#[derive(Clone, Default)]
pub struct SecretsConfig {
    /// Master key for encrypting secrets.
    pub master_key: Option<SecretString>,
    /// Whether secrets management is enabled.
    pub enabled: bool,
    /// Source of the master key.
    pub source: KeySource,
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
    pub async fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
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
                match thinclaw_secrets::keychain::get_master_key().await {
                    Ok(key_bytes) => {
                        let key_hex: String =
                            key_bytes.iter().map(|b| format!("{:02x}", b)).collect();
                        (Some(SecretString::from(key_hex)), KeySource::Keychain)
                    }
                    Err(error) if env_allowed => {
                        tracing::warn!(
                            error = %error,
                            "OS secure store master key is unavailable; considering the explicitly enabled environment fallback"
                        );
                        if let Some(env_key) = optional_env("SECRETS_MASTER_KEY")? {
                            tracing::warn!(
                                "Using SECRETS_MASTER_KEY fallback because OS secure store key is unavailable and env fallback is explicitly allowed"
                            );
                            (Some(SecretString::from(env_key)), KeySource::Env)
                        } else {
                            (None, KeySource::None)
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            error = %error,
                            "OS secure store master key is unavailable and no environment fallback is enabled"
                        );
                        (None, KeySource::None)
                    }
                }
            }
        };

        let enabled = master_key.is_some();

        if let Some(ref key) = master_key
            && !(32..=4096).contains(&key.expose_secret().len())
        {
            return Err(ConfigError::InvalidValue {
                key: "SECRETS_MASTER_KEY".to_string(),
                message: "must contain 32-4096 bytes for AES-256-GCM key derivation".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::lock_env;

    fn clear_secrets_env() {
        unsafe {
            std::env::remove_var("SECRETS_MASTER_KEY");
            std::env::remove_var("THINCLAW_ALLOW_ENV_MASTER_KEY");
        }
    }

    // The `lock_env()` guard must stay held across `resolve().await` because the
    // process environment it sets up must remain in place for the duration of the
    // async resolution. The crate-wide mutex serializes env mutation across tests
    // running in parallel, so the guard intentionally crosses the await point.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn env_source_requires_explicit_allowance() {
        let _guard = lock_env();
        clear_secrets_env();
        unsafe {
            std::env::set_var("SECRETS_MASTER_KEY", "0123456789abcdef0123456789abcdef");
        }

        let mut settings = Settings::default();
        settings.secrets.master_key_source = SecretsMasterKeySource::Env;
        settings.secrets.allow_env_master_key = false;

        let cfg = SecretsConfig::resolve(&settings)
            .await
            .expect("secrets config");
        assert!(!cfg.enabled);
        assert_eq!(cfg.source, KeySource::None);

        clear_secrets_env();
    }

    // See `env_source_requires_explicit_allowance`: the env guard must remain held
    // across the `resolve().await` so the configured environment survives the call.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn env_source_uses_allowed_key() {
        let _guard = lock_env();
        clear_secrets_env();
        unsafe {
            std::env::set_var("SECRETS_MASTER_KEY", "0123456789abcdef0123456789abcdef");
        }

        let mut settings = Settings::default();
        settings.secrets.master_key_source = SecretsMasterKeySource::Env;
        settings.secrets.allow_env_master_key = true;

        let cfg = SecretsConfig::resolve(&settings)
            .await
            .expect("secrets config");
        assert!(cfg.enabled);
        assert_eq!(cfg.source, KeySource::Env);
        assert!(cfg.master_key().is_some());

        clear_secrets_env();
    }

    // See `env_source_requires_explicit_allowance`: the env guard must remain held
    // across the `resolve().await` so the configured environment survives the call.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn short_master_key_is_rejected() {
        let _guard = lock_env();
        clear_secrets_env();
        unsafe {
            std::env::set_var("SECRETS_MASTER_KEY", "short");
        }

        let mut settings = Settings::default();
        settings.secrets.master_key_source = SecretsMasterKeySource::Env;
        settings.secrets.allow_env_master_key = true;

        let err = SecretsConfig::resolve(&settings)
            .await
            .expect_err("short key rejected");
        assert!(err.to_string().contains("AES-256-GCM"));

        clear_secrets_env();
    }
}
