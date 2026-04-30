use thinclaw_types::error::ConfigError;

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

static INJECTED_VARS: LazyLock<RwLock<HashMap<String, String>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static SYNCED_OAUTH_VARS: LazyLock<RwLock<HashMap<String, String>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static BRIDGE_VARS: LazyLock<RwLock<HashMap<String, String>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Crate-wide mutex for tests that mutate process environment variables.
///
/// The process environment is global state shared across all threads.
/// Per-module mutexes do NOT prevent races between modules running in
/// parallel.  Every `unsafe { set_var / remove_var }` call in tests
/// MUST hold this single lock.
pub static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Acquire the crate-wide env-variable mutex for tests.
///
/// Recovers a poisoned mutex rather than panicking — a single failing test
/// should never cascade into mass failures across the entire test suite.
pub fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner())
}

/// Inject bridge configuration variables into the high-priority overlay.
pub fn inject_bridge_vars(vars: HashMap<String, String>) {
    merge_into(&BRIDGE_VARS, vars);
}

/// Remove specific keys from the bridge overlay.
pub fn remove_bridge_vars(keys: &[&str]) {
    match BRIDGE_VARS.write() {
        Ok(mut guard) => {
            for key in keys {
                guard.remove(*key);
            }
        }
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            for key in keys {
                guard.remove(*key);
            }
        }
    }
}

/// Clear all bridge overlay variables.
pub fn clear_bridge_vars() {
    clear_map(&BRIDGE_VARS);
}

/// Check whether a key exists in the bridge overlay or process environment.
pub fn bridge_var_exists(key: &str) -> bool {
    if let Ok(guard) = BRIDGE_VARS.read()
        && guard.contains_key(key)
    {
        return true;
    }
    std::env::var(key).is_ok()
}

/// Replace the legacy injected runtime overlay atomically.
pub fn replace_injected_vars(new_vars: HashMap<String, String>) -> usize {
    let count = new_vars.len();
    replace_map(&INJECTED_VARS, new_vars);
    count
}

/// Merge values into the legacy injected runtime overlay.
pub fn merge_injected_vars(vars: HashMap<String, String>) -> usize {
    let count = vars.len();
    merge_into(&INJECTED_VARS, vars);
    count
}

/// Replace the synced external-auth overlay atomically.
pub fn replace_synced_oauth_vars(new_vars: HashMap<String, String>) -> usize {
    let count = new_vars.len();
    replace_map(&SYNCED_OAUTH_VARS, new_vars);
    count
}

/// Clear all synced external-auth values.
pub fn clear_synced_oauth_vars() {
    clear_map(&SYNCED_OAUTH_VARS);
}

/// Clear all injected overlays. Intended for tests and legacy compatibility.
pub fn clear_injected_vars_for_tests() {
    clear_map(&INJECTED_VARS);
    clear_map(&SYNCED_OAUTH_VARS);
}

fn replace_map(
    target: &LazyLock<RwLock<HashMap<String, String>>>,
    new_vars: HashMap<String, String>,
) {
    match target.write() {
        Ok(mut guard) => {
            *guard = new_vars;
        }
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            *guard = new_vars;
        }
    }
}

fn merge_into(target: &LazyLock<RwLock<HashMap<String, String>>>, vars: HashMap<String, String>) {
    match target.write() {
        Ok(mut guard) => {
            guard.extend(vars);
        }
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            guard.extend(vars);
        }
    }
}

fn clear_map(target: &LazyLock<RwLock<HashMap<String, String>>>) {
    match target.write() {
        Ok(mut guard) => guard.clear(),
        Err(poisoned) => poisoned.into_inner().clear(),
    }
}

pub fn optional_env(key: &str) -> Result<Option<String>, ConfigError> {
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
pub fn synced_oauth_env(key: &str) -> Option<String> {
    if let Ok(guard) = SYNCED_OAUTH_VARS.read()
        && let Some(val) = guard.get(key)
        && !val.is_empty()
    {
        return Some(val.clone());
    }
    None
}

pub fn parse_optional_env<T>(key: &str, default: T) -> Result<T, ConfigError>
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
pub fn parse_bool_env(key: &str, default: bool) -> Result<bool, ConfigError> {
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
pub fn parse_option_env<T>(key: &str) -> Result<Option<T>, ConfigError>
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
pub fn parse_string_env(key: &str, default: impl Into<String>) -> Result<String, ConfigError> {
    Ok(optional_env(key)?.unwrap_or_else(|| default.into()))
}
