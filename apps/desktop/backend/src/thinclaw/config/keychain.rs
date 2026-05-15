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
//! `migrate_per_key_items()` reads each legacy Keychain item, consolidates
//! them into the single JSON blob, then deletes the old items.  This ONLY
//! runs when the unified blob doesn't exist yet — on subsequent launches,
//! the blob is found and migration is skipped entirely (avoiding 21 extra
//! Keychain access prompts).
//!
//! On first launch from pre-keychain builds, `migrate_from_identity()` imports
//! plaintext keys from `identity.json` into the blob.
//! On first launch after the Scrappy → ThinClaw Desktop rename, the legacy
//! `com.schack.scrappy/api_keys` blob is copied into the new service and left
//! in place for rollback.

use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};
use std::collections::HashMap;
use std::sync::Mutex;
use tracing::{info, warn};

/// The Keychain service name — matches the app bundle identifier.
const SERVICE: &str = "com.thinclaw.desktop";

/// Legacy Scrappy Keychain service, read once during the product rename.
const LEGACY_SERVICE: &str = "com.schack.scrappy";

/// The single Keychain account that holds all API keys as a JSON object.
const ACCOUNT: &str = "api_keys";

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

/// Whether the cache has been loaded from the Keychain yet.
fn cache_loaded() -> &'static Mutex<bool> {
    static LOADED: OnceLock<Mutex<bool>> = OnceLock::new();
    LOADED.get_or_init(|| Mutex::new(false))
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
    let mut loaded = cache_loaded().lock().unwrap_or_else(|e| e.into_inner());
    if *loaded {
        return; // Already loaded
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
        Err(e) if is_not_found(&e) => match get_keychain_blob(LEGACY_SERVICE, ACCOUNT) {
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
            Err(e) if is_not_found(&e) => {
                info!("[keychain] no existing api_keys entry — starting fresh");
            }
            Err(e) => {
                warn!("[keychain] failed to read legacy api_keys: {}", e);
            }
        },
        Err(e) => {
            warn!("[keychain] failed to read api_keys: {}", e);
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

    *loaded = true;
}

fn get_keychain_blob(
    service: &str,
    account: &str,
) -> Result<HashMap<String, String>, security_framework::base::Error> {
    match get_generic_password(service, account) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(json_str) => match serde_json::from_str::<HashMap<String, String>>(&json_str) {
                Ok(map) => Ok(map),
                Err(e) => {
                    warn!("[keychain] failed to parse JSON blob: {}", e);
                    Ok(HashMap::new())
                }
            },
            Err(e) => {
                warn!("[keychain] UTF-8 decode error: {}", e);
                Ok(HashMap::new())
            }
        },
        Err(e) => Err(e),
    }
}

/// Store `value` in the Keychain under the given key name.
/// Passing `None` or an empty string removes the entry.
///
/// This updates the in-memory cache and flushes the entire JSON blob
/// back to the Keychain (one write operation).
pub fn set_key(key: &str, value: Option<&str>) -> Result<(), String> {
    // Ensure cache is loaded
    ensure_loaded();

    let mut cache = key_cache().lock().unwrap_or_else(|e| e.into_inner());
    let canonical_key = canonical_key_name(key);

    match value {
        Some(v) if !v.is_empty() => {
            cache.insert(canonical_key.to_string(), v.to_string());
            for alias in legacy_aliases_for(canonical_key) {
                if *alias != canonical_key {
                    cache.remove(*alias);
                }
            }
            info!("[keychain] stored '{}'", canonical_key);
        }
        _ => {
            cache.remove(canonical_key);
            cache.remove(key);
            for alias in legacy_aliases_for(canonical_key) {
                cache.remove(*alias);
            }
            info!("[keychain] removed '{}'", canonical_key);
        }
    }

    // Flush entire blob back to Keychain
    flush_cache(&cache)
}

/// Retrieve a value by key name. Returns `None` if not found.
///
/// Reads from the in-memory cache (no Keychain access).
pub fn get_key(key: &str) -> Option<String> {
    ensure_loaded();

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

// ─────────────────────────────────────────────────────────────────────────────
// Legacy identity.json migration
// ─────────────────────────────────────────────────────────────────────────────

/// Migrate plaintext API keys from a legacy `identity.json` value into the Keychain.
///
/// For every field that is `Some(non-empty)`, it is stored in the cache and
/// cleared from `identity` so the caller can write back a sanitised JSON.
///
/// Returns `true` if any migration was performed (caller should `save_identity`).
pub fn migrate_from_identity(identity: &mut LegacyKeys) -> bool {
    ensure_loaded();

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
        if let Err(e) = flush_cache(&cache) {
            warn!("[keychain] flush after identity migration failed: {}", e);
        }
    }

    migrated
}

// ─────────────────────────────────────────────────────────────────────────────
// Legacy per-key Keychain migration
// ─────────────────────────────────────────────────────────────────────────────

/// Migrate from the old per-key Keychain format (one item per provider)
/// to the unified JSON blob format.
///
/// For each known provider slug, checks if a legacy Keychain item exists.
/// If so, imports it into the cache and deletes the old item.
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
                    // Delete only per-key items in the active service. Legacy
                    // Scrappy items stay in place so users can roll back.
                    if service == SERVICE {
                        match delete_generic_password(service, provider) {
                            Ok(()) => {
                                info!("[keychain] deleted legacy per-key item: '{}'", provider)
                            }
                            Err(e) if is_not_found(&e) => {}
                            Err(e) => {
                                warn!("[keychain] failed to delete legacy '{}': {}", provider, e)
                            }
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
fn ensure_loaded() {
    let loaded = cache_loaded().lock().unwrap_or_else(|e| e.into_inner());
    if !*loaded {
        drop(loaded); // Release lock before calling load_all
        load_all();
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
            Err(e) => warn!("[keychain] delete api_keys error: {}", e),
        }
        return Ok(());
    }

    let json = serde_json::to_string(&clean)
        .map_err(|e| format!("Failed to serialize API keys: {}", e))?;

    set_generic_password(SERVICE, ACCOUNT, json.as_bytes())
        .map_err(|e| format!("Keychain write failed: {}", e))?;

    info!("[keychain] flushed {} keys to Keychain", clean.len());
    Ok(())
}

fn is_not_found(e: &security_framework::base::Error) -> bool {
    // errSecItemNotFound = -25300
    e.code() == -25300
}
