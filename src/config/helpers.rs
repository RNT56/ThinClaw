use crate::error::ConfigError;

use super::{BRIDGE_VARS, INJECTED_VARS, SYNCED_OAUTH_VARS};

/// Crate-wide mutex for tests that mutate process environment variables.
///
/// The process environment is global state shared across all threads.
/// Per-module mutexes do NOT prevent races between modules running in
/// parallel.  Every `unsafe { set_var / remove_var }` call in tests
/// MUST hold this single lock.
#[cfg(test)]
pub(crate) static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Acquire the crate-wide env-variable mutex for tests.
///
/// Recovers a poisoned mutex rather than panicking — a single failing test
/// should never cascade into mass failures across the entire test suite.
#[cfg(test)]
pub(crate) fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn optional_env(key: &str) -> Result<Option<String>, ConfigError> {
    // IC-007: Check bridge overlay FIRST (Tauri UI config — highest priority).
    //
    // This overlay is populated by `inject_bridge_vars()` from the Tauri bridge.
    // It takes priority over everything else because it represents the user's
    // current UI settings (LLM backend, workspace mode, etc.).
    //
    // Priority: BRIDGE_VARS (UI config) > INJECTED_VARS (legacy runtime overlay) > std::env::var > None
    if let Ok(guard) = BRIDGE_VARS.read()
        && let Some(val) = guard.get(key)
        && !val.is_empty()
    {
        return Ok(Some(val.clone()));
    }

    // Check legacy injected runtime overlay.
    //
    // Encrypted secrets are no longer preloaded here; callers that need a
    // stored credential must resolve it through `SecretsStore::get_for_injection`.
    if let Ok(guard) = INJECTED_VARS.read()
        && let Some(val) = guard.get(key)
        && !val.is_empty()
    {
        return Ok(Some(val.clone()));
    }

    // Fall back to real env vars (explicit shell exports + .env files)
    match std::env::var(key) {
        Ok(val) if val.is_empty() => {}
        Ok(val) => return Ok(Some(val)),
        Err(std::env::VarError::NotPresent) => {}
        Err(e) => {
            return Err(ConfigError::ParseError(format!(
                "failed to read {key}: {e}"
            )));
        }
    }

    Ok(None)
}

/// Read a token from the explicit external-auth sync overlay.
///
/// Unlike `optional_env()`, this never falls back to real env vars because the
/// caller is specifically asking for a synced external credential.
pub(crate) fn synced_oauth_env(key: &str) -> Option<String> {
    if let Ok(guard) = SYNCED_OAUTH_VARS.read()
        && let Some(val) = guard.get(key)
        && !val.is_empty()
    {
        return Some(val.clone());
    }
    None
}

pub(crate) fn parse_optional_env<T>(key: &str, default: T) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    optional_env(key)?
        .map(|s| {
            s.parse().map_err(|e| ConfigError::InvalidValue {
                key: key.to_string(),
                message: format!("{e}"),
            })
        })
        .transpose()
        .map(|opt| opt.unwrap_or(default))
}

/// Parse a boolean from an env var with a default.
///
/// Accepts "true"/"1" as true, "false"/"0" as false.
pub(crate) fn parse_bool_env(key: &str, default: bool) -> Result<bool, ConfigError> {
    match optional_env(key)? {
        Some(s) => match s.to_lowercase().as_str() {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            _ => Err(ConfigError::InvalidValue {
                key: key.to_string(),
                message: format!("must be 'true' or 'false', got '{s}'"),
            }),
        },
        None => Ok(default),
    }
}

/// Parse an env var into `Option<T>` — returns `None` when unset,
/// `Some(parsed)` when set to a valid value.
pub(crate) fn parse_option_env<T>(key: &str) -> Result<Option<T>, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    optional_env(key)?
        .map(|s| {
            s.parse().map_err(|e| ConfigError::InvalidValue {
                key: key.to_string(),
                message: format!("{e}"),
            })
        })
        .transpose()
}

/// Parse a string from an env var with a default.
pub(crate) fn parse_string_env(
    key: &str,
    default: impl Into<String>,
) -> Result<String, ConfigError> {
    Ok(optional_env(key)?.unwrap_or_else(|| default.into()))
}
