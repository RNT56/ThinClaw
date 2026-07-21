//! macOS Keychain integration for secure API key storage.
//!
//! **Storage model:** All API keys are stored as a **single JSON object** in
//! one Keychain item:
//!   - Service:  `com.thinclaw.desktop`
//!   - Account:  `api_keys`
//!   - Password: `{"anthropic":"sk-...","openai":"sk-...","huggingface":"hf_..."}`
//!
//! This means exactly **one** `get_generic_password()` call on app startup,
//! which triggers a single macOS Keychain authorization prompt — not 25+
//! individual prompts (one per key, as the previous per-key design caused).
//!
//! **Advantages:**
//!   - Encrypted at rest by the OS (protected by the user's login password / Secure Enclave)
//!   - Other processes cannot read without explicit Keychain access approval
//!   - Single unlock prompt on app launch
//!
//! # Migration
//! On first launch after upgrade from the per-key storage format,
//! `migrate_per_key_items()` reads each legacy Keychain item and consolidates
//! it into the single JSON blob while retaining the old item as a recovery
//! copy. This ONLY
//! runs when the unified blob doesn't exist yet — on subsequent launches,
//! the blob is found and migration is skipped entirely (avoiding 21 extra
//! Keychain access prompts).
//!
//! On first launch from pre-keychain builds, `migrate_from_identity()` imports
//! plaintext keys from `identity.json` into the blob.
//! On first launch after the Scrappy → ThinClaw Desktop rename, the legacy
//! `com.schack.scrappy/api_keys` blob is copied into the new service and left
//! in place for rollback.

use std::collections::HashMap;
#[cfg(not(target_os = "macos"))]
use std::fmt;
use std::sync::Mutex;
use tracing::{info, warn};
use zeroize::Zeroizing;

#[cfg(target_os = "macos")]
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};
#[cfg(target_os = "macos")]
type KeychainError = security_framework::base::Error;

#[cfg(not(target_os = "macos"))]
#[derive(Debug, Clone)]
struct KeychainError {
    code: i32,
    message: String,
}

#[cfg(not(target_os = "macos"))]
impl KeychainError {
    fn code(&self) -> i32 {
        self.code
    }

    fn not_found() -> Self {
        Self {
            code: -25300,
            message: "credential entry not found".to_string(),
        }
    }

    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            code: -1,
            message: message.into(),
        }
    }
}

#[cfg(not(target_os = "macos"))]
impl fmt::Display for KeychainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

#[cfg(not(target_os = "macos"))]
fn run_platform_keychain<T, F, Fut>(operation: F) -> Result<T, KeychainError>
where
    T: Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, thinclaw_secrets::SecretError>> + 'static,
{
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| {
                KeychainError::unavailable(format!(
                    "failed to initialize credential runtime: {error}"
                ))
            })?;
        runtime
            .block_on(operation())
            .map_err(|error| KeychainError::unavailable(error.to_string()))
    })
    .join()
    .map_err(|_| KeychainError::unavailable("credential-store worker panicked"))?
}

#[cfg(not(target_os = "macos"))]
fn platform_account(service: &str, account: &str) -> String {
    format!("desktop:{service}:{account}")
}

#[cfg(not(target_os = "macos"))]
fn get_generic_password(service: &str, account: &str) -> Result<Vec<u8>, KeychainError> {
    let account = platform_account(service, account);
    match run_platform_keychain(move || async move {
        thinclaw_secrets::keychain::get_api_key_result(&account).await
    })? {
        Some(value) => Ok(value.into_bytes()),
        None => Err(KeychainError::not_found()),
    }
}

#[cfg(not(target_os = "macos"))]
fn set_generic_password(
    service: &str,
    account: &str,
    password: &[u8],
) -> Result<(), KeychainError> {
    let account = platform_account(service, account);
    let value = Zeroizing::new(
        String::from_utf8(password.to_vec())
            .map_err(|_| KeychainError::unavailable("credential value is not valid UTF-8"))?,
    );
    run_platform_keychain(move || async move {
        thinclaw_secrets::keychain::store_api_key(&account, value.as_str()).await
    })
}

#[cfg(not(target_os = "macos"))]
fn delete_generic_password(service: &str, account: &str) -> Result<(), KeychainError> {
    let account = platform_account(service, account);
    let account_for_read = account.clone();
    let existing = run_platform_keychain(move || async move {
        thinclaw_secrets::keychain::get_api_key_result(&account_for_read).await
    })?;
    if existing.is_none() {
        return Err(KeychainError::not_found());
    }
    run_platform_keychain(move || async move {
        thinclaw_secrets::keychain::delete_api_key(&account).await
    })
}

/// The Keychain service name — matches the app bundle identifier.
const SERVICE: &str = "com.thinclaw.desktop";

/// Legacy Scrappy Keychain service, read once during the product rename.
const LEGACY_SERVICE: &str = "com.schack.scrappy";

/// The single Keychain account that holds all API keys as a JSON object.
const ACCOUNT: &str = "api_keys";

const MAX_KEYCHAIN_BLOB_BYTES: usize = 4 * 1024 * 1024;
const MAX_KEYCHAIN_ENTRIES: usize = 4_096;
const MAX_KEYCHAIN_KEY_BYTES: usize = 1_024;
const MAX_KEYCHAIN_VALUE_BYTES: usize = 1024 * 1024;

/// Provider slugs — used for migration and as JSON map keys.
///
/// This list is intentionally explicit so it's easy to audit.
pub const PROVIDERS: &[&str] = &[
    "anthropic",
    "openai",
    "openrouter",
    "gemini",
    "groq",
    "brave",
    "huggingface",
    "xai",
    "venice",
    "together",
    "moonshot",
    "minimax",
    "nvidia",
    "qianfan",
    "mistral",
    "xiaomi",
    "cohere",
    "voyage",
    "deepgram",
    "elevenlabs",
    "stability",
    "fal",
    // Bedrock stores three separate fields
    "bedrock_api_key",
    "bedrock_proxy_api_key",
    "bedrock_access_key_id",
    "bedrock_secret_access_key",
    "bedrock_region",
    // Custom LLM
    "custom_llm_key",
    // Remote gateway token
    "remote_token",
    // Desktop gateway handshake token and protocol signing key
    "desktop_gateway_auth_token",
    "desktop_device_private_key",
    // Google Workspace OAuth state (access/refresh/client/scope metadata)
    "google_oauth_token",
    "google_oauth_token_refresh_token",
    "google_oauth_token_scopes",
    "google_oauth_token_client_id",
    "google_oauth_token_client_secret",
];

// ─────────────────────────────────────────────────────────────────────────────
// In-memory cache — loaded once, written back on mutation
// ─────────────────────────────────────────────────────────────────────────────

use std::sync::OnceLock;

/// In-memory cache of all API keys. Populated by `load_all()` on startup,
/// mutated by `set_key()`, and flushed to the Keychain on every write.
fn key_cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug)]
enum CacheState {
    Uninitialized,
    Loaded,
    Unavailable(String),
}

fn cache_state() -> &'static Mutex<CacheState> {
    static STATE: OnceLock<Mutex<CacheState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(CacheState::Uninitialized))
}

#[derive(Debug)]
enum BlobReadError {
    NotFound,
    Unavailable(String),
    Invalid(String),
}

impl std::fmt::Display for BlobReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("credential entry not found"),
            Self::Unavailable(message) | Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

/// Current ThinClaw secret identifiers used for new writes.
///
/// The shorter Scrappy/ThinClaw-era provider slugs remain readable as fallback
/// aliases so existing users do not lose credentials during the rename.
pub(crate) fn canonical_key_name(key: &str) -> &str {
    thinclaw_runtime_contracts::canonical_secret_name(key)
}

fn legacy_aliases_for(canonical: &str) -> &'static [&'static str] {
    thinclaw_runtime_contracts::legacy_secret_aliases(canonical)
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Load ALL API keys from the Keychain in a single read.
///
/// Call this **once** during app startup (before any `get_key` / `set_key`).
/// This triggers exactly one macOS Keychain authorization prompt.
pub fn load_all() {
    let mut state = cache_state().lock().unwrap_or_else(|e| e.into_inner());
    if !matches!(*state, CacheState::Uninitialized) {
        return;
    }

    let mut cache = key_cache().lock().unwrap_or_else(|e| e.into_inner());

    // Track whether we found an existing unified blob — if so, skip legacy migration
    let mut blob_existed = false;

    match get_keychain_blob(SERVICE, ACCOUNT) {
        Ok(map) => {
            let count = map.len();
            *cache = map;
            blob_existed = true;
            info!(
                "[keychain] loaded {} keys from unified Keychain entry",
                count
            );
        }
        Err(BlobReadError::NotFound) => match get_keychain_blob(LEGACY_SERVICE, ACCOUNT) {
            Ok(map) => {
                let count = map.len();
                *cache = map;
                blob_existed = true;
                info!(
                    "[keychain] migrated {} keys from legacy Scrappy Keychain service",
                    count
                );
                if let Err(e) = flush_cache(&cache) {
                    warn!("[keychain] flush after service migration failed: {}", e);
                }
            }
            Err(BlobReadError::NotFound) => {
                info!("[keychain] no existing api_keys entry — starting fresh");
            }
            Err(error) => {
                let message = format!("failed to read legacy credentials: {error}");
                warn!("[keychain] {message}");
                *state = CacheState::Unavailable(message);
                return;
            }
        },
        Err(error) => {
            let message = format!("failed to read credentials: {error}");
            warn!("[keychain] {message}");
            *state = CacheState::Unavailable(message);
            return;
        }
    }

    // Migrate from legacy per-key Keychain items ONLY if the unified blob
    // didn't exist yet.  This avoids extra `get_generic_password` calls on
    // every launch, each of which macOS treats as a separate Keychain access
    // that may trigger an additional authorization prompt.
    if !blob_existed {
        let migrated = migrate_per_key_items(&mut cache);
        if migrated {
            // Flush the consolidated blob back to Keychain
            if let Err(e) = flush_cache(&cache) {
                warn!("[keychain] flush after per-key migration failed: {}", e);
            }
        }
    }

    *state = CacheState::Loaded;
}

fn get_keychain_blob(
    service: &str,
    account: &str,
) -> Result<HashMap<String, String>, BlobReadError> {
    match get_generic_password(service, account) {
        Ok(bytes) => {
            if bytes.len() > MAX_KEYCHAIN_BLOB_BYTES {
                return Err(BlobReadError::Invalid(format!(
                    "credential entry exceeds {MAX_KEYCHAIN_BLOB_BYTES} bytes"
                )));
            }
            let json = Zeroizing::new(String::from_utf8(bytes).map_err(|_| {
                BlobReadError::Invalid("credential entry is not valid UTF-8".to_string())
            })?);
            let values = serde_json::from_str::<HashMap<String, String>>(json.as_str()).map_err(
                |error| {
                    BlobReadError::Invalid(format!(
                        "credential entry contains invalid JSON: {error}"
                    ))
                },
            )?;
            validate_credential_map(&values).map_err(BlobReadError::Invalid)?;
            Ok(values)
        }
        Err(error) if is_not_found(&error) => Err(BlobReadError::NotFound),
        Err(error) => Err(BlobReadError::Unavailable(format!(
            "platform credential store read failed: {error}"
        ))),
    }
}

/// Store `value` in the Keychain under the given key name.
/// Passing `None` or an empty string removes the entry.
///
/// This updates the in-memory cache and flushes the entire JSON blob
/// back to the Keychain (one write operation).
pub fn set_key(key: &str, value: Option<&str>) -> Result<(), String> {
    set_keys(&[(key, value)])
}

/// Atomically update several values in the unified credential blob. The
/// in-memory cache is replaced only after the platform write succeeds, so a
/// failed durable write cannot leave runtime reads observing phantom values.
pub fn set_keys(entries: &[(&str, Option<&str>)]) -> Result<(), String> {
    // Ensure cache is loaded
    ensure_loaded()?;
    if entries.len() > MAX_KEYCHAIN_ENTRIES {
        return Err("Too many credential updates in one operation".to_string());
    }
    for (key, value) in entries {
        validate_credential_entry(key, value.unwrap_or_default())?;
    }

    let mut cache = key_cache().lock().unwrap_or_else(|e| e.into_inner());
    let mut candidate = cache.clone();
    for (key, value) in entries {
        let canonical_key = canonical_key_name(key);
        match value {
            Some(value) if !value.is_empty() => {
                candidate.insert(canonical_key.to_string(), (*value).to_string());
                for alias in legacy_aliases_for(canonical_key) {
                    if *alias != canonical_key {
                        candidate.remove(*alias);
                    }
                }
            }
            _ => {
                candidate.remove(canonical_key);
                candidate.remove(*key);
                for alias in legacy_aliases_for(canonical_key) {
                    candidate.remove(*alias);
                }
            }
        }
    }

    validate_credential_map(&candidate)?;
    flush_cache(&candidate)?;
    *cache = candidate;
    info!(
        "[keychain] committed {} credential update(s)",
        entries.len()
    );
    Ok(())
}

/// Retrieve a value by key name. Returns `None` if not found.
///
/// Reads from the in-memory cache (no Keychain access).
pub fn get_key(key: &str) -> Option<String> {
    if ensure_loaded().is_err() {
        return None;
    }

    let cache = key_cache().lock().unwrap_or_else(|e| e.into_inner());
    let canonical_key = canonical_key_name(key);
    cache
        .get(canonical_key)
        .or_else(|| cache.get(key))
        .or_else(|| {
            legacy_aliases_for(canonical_key)
                .iter()
                .find_map(|alias| cache.get(*alias))
        })
        .cloned()
}

/// Stable, non-identifying secure-store key for a remote agent profile token.
/// Hashing also prevents an attacker-controlled profile ID from colliding with
/// built-in provider secret names.
pub fn profile_token_key(profile_id: &str) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(profile_id.as_bytes());
    format!("desktop_agent_profile_token_{}", hex::encode(digest))
}

// ─────────────────────────────────────────────────────────────────────────────
// Legacy identity.json migration
// ─────────────────────────────────────────────────────────────────────────────

/// Migrate plaintext API keys from a legacy `identity.json` value into the Keychain.
///
/// For every field that is `Some(non-empty)`, it is stored in the cache and
/// cleared from `identity` so the caller can write back a sanitised JSON.
///
/// Returns whether any migration was performed. A failed credential-store
/// flush is returned to the caller so it must not scrub the plaintext source.
pub fn migrate_from_identity(identity: &mut LegacyKeys) -> Result<bool, String> {
    ensure_loaded()?;

    let mut cache = key_cache().lock().unwrap_or_else(|e| e.into_inner());
    let mut migrated = false;

    macro_rules! migrate {
        ($field:expr, $key:expr) => {
            if let Some(ref val) = $field {
                if !val.is_empty() {
                    let canonical = canonical_key_name($key);
                    cache.insert(canonical.to_string(), val.clone());
                    info!("[keychain] migrated '{}' from identity.json", canonical);
                    $field = None;
                    migrated = true;
                }
            }
        };
    }

    migrate!(identity.anthropic_api_key, "anthropic");
    migrate!(identity.openai_api_key, "openai");
    migrate!(identity.openrouter_api_key, "openrouter");
    migrate!(identity.gemini_api_key, "gemini");
    migrate!(identity.groq_api_key, "groq");
    migrate!(identity.brave_search_api_key, "brave");
    migrate!(identity.huggingface_token, "huggingface");
    migrate!(identity.xai_api_key, "xai");
    migrate!(identity.venice_api_key, "venice");
    migrate!(identity.together_api_key, "together");
    migrate!(identity.moonshot_api_key, "moonshot");
    migrate!(identity.minimax_api_key, "minimax");
    migrate!(identity.nvidia_api_key, "nvidia");
    migrate!(identity.qianfan_api_key, "qianfan");
    migrate!(identity.mistral_api_key, "mistral");
    migrate!(identity.xiaomi_api_key, "xiaomi");
    migrate!(identity.bedrock_access_key_id, "bedrock_access_key_id");
    migrate!(
        identity.bedrock_secret_access_key,
        "bedrock_secret_access_key"
    );
    migrate!(identity.bedrock_region, "bedrock_region");
    migrate!(identity.custom_llm_key, "custom_llm_key");
    migrate!(identity.remote_token, "remote_token");

    if migrated {
        flush_cache(&cache)?;
    }

    Ok(migrated)
}

// ─────────────────────────────────────────────────────────────────────────────
// Legacy per-key Keychain migration
// ─────────────────────────────────────────────────────────────────────────────

/// Migrate from the old per-key Keychain format (one item per provider)
/// to the unified JSON blob format.
///
/// For each known provider slug, checks if a legacy Keychain item exists.
/// If so, imports it into the cache. Legacy entries are intentionally retained
/// as recovery copies until the consolidated blob has been proven durable.
///
/// Returns `true` if any legacy items were found and migrated.
fn migrate_per_key_items(cache: &mut HashMap<String, String>) -> bool {
    let mut migrated = false;

    for service in [SERVICE, LEGACY_SERVICE] {
        for &provider in PROVIDERS {
            // Try to read a legacy per-key item
            match get_generic_password(service, provider) {
                Ok(bytes) => {
                    if let Ok(value) = String::from_utf8(bytes) {
                        if !value.is_empty() && !cache.contains_key(provider) {
                            info!(
                                "[keychain] migrating legacy per-key item '{}' from service '{}'",
                                provider, service
                            );
                            cache.insert(provider.to_string(), value);
                            migrated = true;
                        }
                    }
                }
                Err(e) if is_not_found(&e) => {} // No legacy item
                Err(_) => {}                     // Ignore read errors during migration
            }
        }
    }

    if migrated {
        info!("[keychain] per-key migration complete — consolidated into single entry");
    }

    migrated
}

// ─────────────────────────────────────────────────────────────────────────────
// Migration shim — fields that were previously in ThinClawIdentity
// ─────────────────────────────────────────────────────────────────────────────

/// Temporary struct used only during migration — matches the legacy `identity.json`
/// API-key fields so we can deserialise old files and pull keys out of them.
#[derive(serde::Deserialize, serde::Serialize, Default)]
pub struct LegacyKeys {
    #[serde(default)]
    pub anthropic_api_key: Option<String>,
    #[serde(default)]
    pub openai_api_key: Option<String>,
    #[serde(default)]
    pub openrouter_api_key: Option<String>,
    #[serde(default)]
    pub gemini_api_key: Option<String>,
    #[serde(default)]
    pub groq_api_key: Option<String>,
    #[serde(default)]
    pub brave_search_api_key: Option<String>,
    #[serde(default)]
    pub huggingface_token: Option<String>,
    #[serde(default)]
    pub xai_api_key: Option<String>,
    #[serde(default)]
    pub venice_api_key: Option<String>,
    #[serde(default)]
    pub together_api_key: Option<String>,
    #[serde(default)]
    pub moonshot_api_key: Option<String>,
    #[serde(default)]
    pub minimax_api_key: Option<String>,
    #[serde(default)]
    pub nvidia_api_key: Option<String>,
    #[serde(default)]
    pub qianfan_api_key: Option<String>,
    #[serde(default)]
    pub mistral_api_key: Option<String>,
    #[serde(default)]
    pub xiaomi_api_key: Option<String>,
    #[serde(default)]
    pub bedrock_access_key_id: Option<String>,
    #[serde(default)]
    pub bedrock_secret_access_key: Option<String>,
    #[serde(default)]
    pub bedrock_region: Option<String>,
    #[serde(default)]
    pub custom_llm_key: Option<String>,
    #[serde(default)]
    pub remote_token: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Ensure the cache has been loaded from the Keychain.
fn ensure_loaded() -> Result<(), String> {
    let needs_load = {
        let state = cache_state().lock().unwrap_or_else(|e| e.into_inner());
        matches!(*state, CacheState::Uninitialized)
    };
    if needs_load {
        load_all();
    }
    let state = cache_state().lock().unwrap_or_else(|e| e.into_inner());
    match &*state {
        CacheState::Loaded => Ok(()),
        CacheState::Unavailable(message) => Err(message.clone()),
        CacheState::Uninitialized => {
            Err("platform credential store did not initialize".to_string())
        }
    }
}

/// Flush the in-memory cache to the Keychain as a single JSON blob.
fn flush_cache(cache: &HashMap<String, String>) -> Result<(), String> {
    // Only write non-empty values
    let clean: HashMap<&String, &String> = cache.iter().filter(|(_, v)| !v.is_empty()).collect();

    if clean.is_empty() {
        // No keys → delete the Keychain item
        match delete_generic_password(SERVICE, ACCOUNT) {
            Ok(()) => info!("[keychain] deleted api_keys entry (no keys stored)"),
            Err(e) if is_not_found(&e) => {}
            Err(e) => return Err(format!("Keychain delete failed: {e}")),
        }
        return Ok(());
    }

    let json = Zeroizing::new(
        serde_json::to_string(&clean)
            .map_err(|e| format!("Failed to serialize API keys: {}", e))?,
    );
    if json.len() > MAX_KEYCHAIN_BLOB_BYTES {
        return Err(format!(
            "Credential blob exceeds {MAX_KEYCHAIN_BLOB_BYTES} bytes"
        ));
    }

    set_generic_password(SERVICE, ACCOUNT, json.as_bytes())
        .map_err(|e| format!("Keychain write failed: {}", e))?;

    info!("[keychain] flushed {} keys to Keychain", clean.len());
    Ok(())
}

fn validate_credential_map(values: &HashMap<String, String>) -> Result<(), String> {
    if values.len() > MAX_KEYCHAIN_ENTRIES {
        return Err(format!(
            "credential entry exceeds {MAX_KEYCHAIN_ENTRIES} values"
        ));
    }
    for (key, value) in values {
        validate_credential_entry(key, value)?;
    }
    Ok(())
}

fn validate_credential_entry(key: &str, value: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > MAX_KEYCHAIN_KEY_BYTES || key.chars().any(char::is_control) {
        return Err("Credential key is empty, oversized, or contains controls".to_string());
    }
    if value.len() > MAX_KEYCHAIN_VALUE_BYTES {
        return Err(format!(
            "Credential value for '{}' exceeds {MAX_KEYCHAIN_VALUE_BYTES} bytes",
            key.chars().take(64).collect::<String>()
        ));
    }
    Ok(())
}

fn is_not_found(e: &KeychainError) -> bool {
    // errSecItemNotFound = -25300
    e.code() == -25300
}
