//! macOS Keychain integration for secure API key storage.
//!
//! **Storage model:** All API keys are serialized into one JSON object, then
//! encrypted as a single AES-256-GCM envelope before that envelope is stored in
//! one Keychain item:
//!   - Service:  `com.thinclaw.desktop`
//!   - Account:  `api_keys`
//!   - Password: versioned ciphertext metadata (never raw provider keys)
//!   - Master key: shared core `thinclaw/master_key` Keychain item
//!
//! Normal startup reads exactly **two** Keychain items (master key + encrypted
//! envelope), rather than 25+ individual provider items as the previous
//! per-key design did.
//!
//! **Advantages:**
//!   - Core `SecretsCrypto` AES-256-GCM protection plus OS Keychain protection
//!   - Other processes cannot read without explicit Keychain access approval
//!   - A bounded two-item startup read instead of one prompt per provider
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

use secrecy::SecretString;
use std::collections::HashMap;
#[cfg(not(target_os = "macos"))]
use std::fmt;
use std::sync::{Arc, Mutex};
use thinclaw_core::secrets::SecretsCrypto;
use tracing::{info, warn};
use zeroize::Zeroize;

#[cfg(target_os = "macos")]
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};
#[cfg(target_os = "macos")]
type KeychainError = security_framework::base::Error;

#[cfg(not(target_os = "macos"))]
#[derive(Debug, Clone, Copy)]
struct KeychainError;

#[cfg(not(target_os = "macos"))]
impl KeychainError {
    fn code(&self) -> i32 {
        -25300
    }
}

#[cfg(not(target_os = "macos"))]
impl fmt::Display for KeychainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("platform keychain is unavailable")
    }
}

#[cfg(not(target_os = "macos"))]
fn get_generic_password(_service: &str, _account: &str) -> Result<Vec<u8>, KeychainError> {
    Err(KeychainError)
}

#[cfg(not(target_os = "macos"))]
fn set_generic_password(
    _service: &str,
    _account: &str,
    _password: &[u8],
) -> Result<(), KeychainError> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn delete_generic_password(_service: &str, _account: &str) -> Result<(), KeychainError> {
    Err(KeychainError)
}

/// The Keychain service name — matches the app bundle identifier.
const SERVICE: &str = "com.thinclaw.desktop";

/// Legacy Scrappy Keychain service, read once during the product rename.
const LEGACY_SERVICE: &str = "com.schack.scrappy";

/// The single Keychain account that holds all API keys as a JSON object.
const ACCOUNT: &str = "api_keys";

/// Core ThinClaw secure-store coordinates for the random AES master key. Using
/// the same item as the CLI/runtime keeps the `SecretsStore::rotate_master_key`
/// contract durable across process restarts.
const MASTER_KEY_SERVICE: &str = "thinclaw";
const MASTER_KEY_ACCOUNT: &str = "master_key";

const ENCRYPTED_BLOB_VERSION: u32 = 1;
const INITIAL_KEY_VERSION: i32 = 1;
const ENCRYPTED_BLOB_CIPHER: &str = "aes-256-gcm";
const ENCRYPTED_BLOB_KDF: &str = "hkdf-sha256";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct EncryptedKeychainBlob {
    version: u32,
    key_version: i32,
    cipher: String,
    kdf: String,
    ciphertext: String,
    salt: String,
}

struct KeychainCryptoState {
    crypto: Arc<SecretsCrypto>,
    key_version: i32,
}

struct DecodedKeychainBlob {
    secrets: HashMap<String, String>,
    key_version: i32,
    encrypted: bool,
}

type LegacyKeychainItem = (&'static str, &'static str);
type PerKeyMigration = (bool, Vec<LegacyKeychainItem>);

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

/// AES state for the encrypted envelope. The master key itself remains inside
/// `SecretsCrypto` and is redacted/zeroized by the core secrets implementation.
fn keychain_crypto() -> &'static Mutex<Option<KeychainCryptoState>> {
    static CRYPTO: OnceLock<Mutex<Option<KeychainCryptoState>>> = OnceLock::new();
    CRYPTO.get_or_init(|| Mutex::new(None))
}

/// Whether the cache has been loaded from the Keychain yet.
fn cache_loaded() -> &'static Mutex<bool> {
    static LOADED: OnceLock<Mutex<bool>> = OnceLock::new();
    LOADED.get_or_init(|| Mutex::new(false))
}

/// Current ThinClaw secret identifiers used for new writes.
///
/// The shorter Scrappy/ThinClaw-era provider slugs remain readable as fallback
/// aliases and are canonicalized when the unified keychain blob is loaded.
pub(crate) fn canonical_key_name(key: &str) -> &str {
    thinclaw_runtime_contracts::canonical_secret_name(key)
}

fn legacy_aliases_for(canonical: &str) -> &'static [&'static str] {
    thinclaw_runtime_contracts::legacy_secret_aliases(canonical)
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

fn blob_aad(version: u32, key_version: i32) -> Vec<u8> {
    format!(
        "thinclaw-desktop|account={ACCOUNT}|envelope_version={version}|key_version={key_version}"
    )
    .into_bytes()
}

fn initialize_crypto() -> Result<Arc<SecretsCrypto>, String> {
    let master_key_hex = match get_generic_password(MASTER_KEY_SERVICE, MASTER_KEY_ACCOUNT) {
        Ok(bytes) => String::from_utf8(bytes)
            .map_err(|_| "Keychain master key is not valid UTF-8".to_string())?,
        Err(error) if is_not_found(&error) => {
            use rand::RngCore as _;

            let mut key = [0_u8; 32];
            rand::thread_rng().fill_bytes(&mut key);
            let encoded = hex::encode(key);
            key.zeroize();
            set_generic_password(MASTER_KEY_SERVICE, MASTER_KEY_ACCOUNT, encoded.as_bytes())
                .map_err(|error| format!("Keychain master-key write failed: {error}"))?;
            encoded
        }
        Err(error) => return Err(format!("Keychain master-key read failed: {error}")),
    };

    let mut decoded = hex::decode(&master_key_hex)
        .map_err(|_| "Keychain master key is not valid hexadecimal data".to_string())?;
    if decoded.len() != 32 {
        let actual_len = decoded.len();
        decoded.zeroize();
        return Err(format!(
            "Keychain master key must contain 32 bytes, found {}",
            actual_len
        ));
    }
    decoded.zeroize();

    let crypto = Arc::new(
        SecretsCrypto::new(SecretString::from(master_key_hex))
            .map_err(|error| format!("Failed to initialize secrets crypto: {error}"))?,
    );
    let mut state = keychain_crypto().lock().unwrap_or_else(|e| e.into_inner());
    *state = Some(KeychainCryptoState {
        crypto: Arc::clone(&crypto),
        key_version: INITIAL_KEY_VERSION,
    });
    Ok(crypto)
}

fn decode_keychain_blob(
    bytes: &[u8],
    crypto: &SecretsCrypto,
) -> Result<DecodedKeychainBlob, String> {
    if let Ok(envelope) = serde_json::from_slice::<EncryptedKeychainBlob>(bytes) {
        if envelope.version != ENCRYPTED_BLOB_VERSION
            || envelope.cipher != ENCRYPTED_BLOB_CIPHER
            || envelope.kdf != ENCRYPTED_BLOB_KDF
        {
            return Err(format!(
                "Unsupported encrypted Keychain envelope v{} ({}/{})",
                envelope.version, envelope.cipher, envelope.kdf
            ));
        }

        let ciphertext = hex::decode(&envelope.ciphertext).map_err(|_| {
            "Encrypted Keychain ciphertext is not valid hexadecimal data".to_string()
        })?;
        let salt = hex::decode(&envelope.salt)
            .map_err(|_| "Encrypted Keychain salt is not valid hexadecimal data".to_string())?;
        let decrypted = crypto
            .decrypt_with_aad(
                &ciphertext,
                &salt,
                &blob_aad(envelope.version, envelope.key_version),
            )
            .map_err(|error| format!("Encrypted Keychain blob authentication failed: {error}"))?;
        let secrets = serde_json::from_str::<HashMap<String, String>>(decrypted.expose())
            .map_err(|error| format!("Decrypted Keychain blob is invalid JSON: {error}"))?;

        return Ok(DecodedKeychainBlob {
            secrets,
            key_version: envelope.key_version,
            encrypted: true,
        });
    }

    let secrets = serde_json::from_slice::<HashMap<String, String>>(bytes)
        .map_err(|error| format!("Keychain blob is neither encrypted nor legacy JSON: {error}"))?;
    Ok(DecodedKeychainBlob {
        secrets,
        key_version: INITIAL_KEY_VERSION,
        encrypted: false,
    })
}

fn encode_keychain_blob(
    secrets: &HashMap<String, String>,
    crypto: &SecretsCrypto,
    key_version: i32,
) -> Result<Vec<u8>, String> {
    let mut plaintext = serde_json::to_vec(secrets)
        .map_err(|error| format!("Failed to serialize API keys: {error}"))?;
    let encrypted =
        crypto.encrypt_with_aad(&plaintext, &blob_aad(ENCRYPTED_BLOB_VERSION, key_version));
    plaintext.zeroize();
    let (ciphertext, salt) =
        encrypted.map_err(|error| format!("Failed to encrypt API keys: {error}"))?;
    serde_json::to_vec(&EncryptedKeychainBlob {
        version: ENCRYPTED_BLOB_VERSION,
        key_version,
        cipher: ENCRYPTED_BLOB_CIPHER.to_string(),
        kdf: ENCRYPTED_BLOB_KDF.to_string(),
        ciphertext: hex::encode(ciphertext),
        salt: hex::encode(salt),
    })
    .map_err(|error| format!("Failed to serialize encrypted API keys: {error}"))
}

fn verify_keychain_blob(
    bytes: &[u8],
    crypto: &SecretsCrypto,
    key_version: i32,
    expected: &HashMap<String, String>,
) -> Result<(), String> {
    let decoded = decode_keychain_blob(bytes, crypto)?;
    if !decoded.encrypted || decoded.key_version != key_version || decoded.secrets != *expected {
        return Err(
            "Keychain rotation verification did not reproduce the expected envelope".to_string(),
        );
    }
    Ok(())
}

/// Load all API keys from the Keychain and authenticate/decrypt their single
/// AES envelope.
///
/// Call this **once** during app startup (before any `get_key` / `set_key`).
/// Existing plaintext JSON blobs are encrypted in place after a successful
/// read and canonical-name migration.
pub fn load_all() -> Result<(), String> {
    let mut loaded = cache_loaded().lock().unwrap_or_else(|e| e.into_inner());
    if *loaded {
        return Ok(()); // Already loaded
    }

    let crypto = initialize_crypto()?;
    let mut cache = key_cache().lock().unwrap_or_else(|e| e.into_inner());

    // Track whether we found an existing unified blob — if so, skip legacy migration
    let mut blob_existed = false;
    let mut needs_encrypted_flush = false;

    match get_generic_password(SERVICE, ACCOUNT) {
        Ok(bytes) => {
            let decoded = decode_keychain_blob(&bytes, crypto.as_ref())?;
            let count = decoded.secrets.len();
            needs_encrypted_flush = !decoded.encrypted;
            if let Some(state) = keychain_crypto()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
            {
                state.key_version = decoded.key_version;
            }
            *cache = decoded.secrets;
            blob_existed = true;
            info!(
                "[keychain] loaded {} keys from authenticated Keychain envelope",
                count,
            );
        }
        Err(e) if is_not_found(&e) => match get_generic_password(LEGACY_SERVICE, ACCOUNT) {
            Ok(bytes) => {
                let decoded = decode_keychain_blob(&bytes, crypto.as_ref())?;
                let count = decoded.secrets.len();
                if let Some(state) = keychain_crypto()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_mut()
                {
                    state.key_version = decoded.key_version;
                }
                *cache = decoded.secrets;
                blob_existed = true;
                needs_encrypted_flush = true;
                info!(
                    "[keychain] migrated {} keys from legacy Scrappy Keychain service",
                    count
                );
            }
            Err(e) if is_not_found(&e) => {
                info!("[keychain] no existing api_keys entry — starting fresh");
            }
            Err(e) => {
                return Err(format!("Legacy Keychain api_keys read failed: {e}"));
            }
        },
        Err(e) => {
            return Err(format!("Keychain api_keys read failed: {e}"));
        }
    }

    // Migrate from legacy per-key Keychain items ONLY if the unified blob
    // didn't exist yet.  This avoids extra `get_generic_password` calls on
    // every launch, each of which macOS treats as a separate Keychain access
    // that may trigger an additional authorization prompt.
    let mut legacy_items_to_delete = Vec::new();
    if !blob_existed {
        let (migrated, pending_deletions) = migrate_per_key_items(&mut cache)?;
        legacy_items_to_delete = pending_deletions;
        if migrated {
            needs_encrypted_flush = true;
        }
    }

    // Canonicalize provider aliases inside both current and imported blobs.
    // Preserve an existing canonical value if a stale alias also exists.
    if migrate_legacy_aliases(&mut cache) {
        needs_encrypted_flush = true;
    }

    if needs_encrypted_flush {
        flush_cache(&cache)?;
    }

    // Delete active-service legacy items only after every imported value has
    // reached the authenticated envelope. A failed encrypted write therefore
    // leaves the original credentials available for the next startup attempt.
    for (service, provider) in legacy_items_to_delete {
        match delete_generic_password(service, provider) {
            Ok(()) => info!("[keychain] deleted legacy per-key item: '{}'", provider),
            Err(error) if is_not_found(&error) => {}
            Err(error) => warn!(
                "[keychain] failed to delete legacy '{}': {}",
                provider, error
            ),
        }
    }

    *loaded = true;
    Ok(())
}

/// Store `value` in the Keychain under the given key name.
/// Passing `None` or an empty string removes the entry.
///
/// This updates the in-memory cache and flushes the entire JSON blob
/// back to the Keychain (one write operation).
pub fn set_key(key: &str, value: Option<&str>) -> Result<(), String> {
    // Ensure cache is loaded
    ensure_loaded()?;

    let mut cache = key_cache().lock().unwrap_or_else(|e| e.into_inner());
    let mut next = cache.clone();
    let canonical_key = canonical_key_name(key);

    match value {
        Some(v) if !v.is_empty() => {
            next.insert(canonical_key.to_string(), v.to_string());
            for alias in legacy_aliases_for(canonical_key) {
                if *alias != canonical_key {
                    next.remove(*alias);
                }
            }
        }
        _ => {
            next.remove(canonical_key);
            next.remove(key);
            for alias in legacy_aliases_for(canonical_key) {
                next.remove(*alias);
            }
        }
    }

    // Persist first, then publish the new cache state. This prevents a failed
    // Keychain write from making the process believe an unpersisted mutation
    // succeeded.
    flush_cache(&next)?;
    *cache = next;
    if value.is_some_and(|candidate| !candidate.is_empty()) {
        info!("[keychain] stored '{}'", canonical_key);
    } else {
        info!("[keychain] removed '{}'", canonical_key);
    }
    Ok(())
}

/// Retrieve a value by key name. Returns `None` if not found.
///
/// Reads from the in-memory cache (no Keychain access).
pub fn get_key(key: &str) -> Option<String> {
    if let Err(error) = ensure_loaded() {
        warn!("[keychain] refusing secret read: {error}");
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

// ─────────────────────────────────────────────────────────────────────────────
// Legacy identity.json migration
// ─────────────────────────────────────────────────────────────────────────────

/// Migrate plaintext API keys from a legacy `identity.json` value into the Keychain.
///
/// For every field that is `Some(non-empty)`, it is stored in the cache and
/// cleared from `identity` so the caller can write back a sanitised JSON.
///
/// Returns `Ok(true)` if migration was performed and durably written. On an
/// error, the input and cache are left unchanged so the plaintext source can be
/// retained for a later retry instead of being silently discarded.
pub fn migrate_from_identity(identity: &mut LegacyKeys) -> Result<bool, String> {
    ensure_loaded()?;

    let mut cache = key_cache().lock().unwrap_or_else(|e| e.into_inner());
    let mut next = cache.clone();
    let original_identity = identity.clone();
    let mut migrated = false;

    macro_rules! migrate {
        ($field:expr, $key:expr) => {
            if let Some(ref val) = $field {
                if !val.is_empty() {
                    let canonical = canonical_key_name($key);
                    next.insert(canonical.to_string(), val.clone());
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
        if let Err(error) = flush_cache(&next) {
            *identity = original_identity;
            return Err(error);
        }
        *cache = next;
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
/// If so, stages it in the cache and returns active-service items for deletion
/// after the consolidated encrypted envelope has been written successfully.
///
/// Returns whether any value was imported plus the items safe to delete after
/// a successful flush. Scrappy service items are retained for rollback.
fn migrate_per_key_items(cache: &mut HashMap<String, String>) -> Result<PerKeyMigration, String> {
    let mut migrated = false;
    let mut pending_deletions = Vec::new();

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
                    // Delete only per-key items in the active service, and only
                    // after load_all has successfully flushed the encrypted
                    // envelope. Legacy Scrappy items stay for rollback.
                    if service == SERVICE {
                        pending_deletions.push((service, provider));
                    }
                }
                Err(e) if is_not_found(&e) => {} // No legacy item
                Err(error) => {
                    return Err(format!(
                        "Legacy Keychain item '{service}/{provider}' read failed: {error}"
                    ));
                }
            }
        }
    }

    if migrated {
        info!("[keychain] per-key migration complete — consolidated into single entry");
    }

    Ok((migrated, pending_deletions))
}

/// Move provider aliases in a unified blob to their canonical ThinClaw names.
fn migrate_legacy_aliases(cache: &mut HashMap<String, String>) -> bool {
    let keys: Vec<String> = cache.keys().cloned().collect();
    let mut migrated = false;

    for key in keys {
        let canonical = canonical_key_name(&key);
        if canonical == key {
            continue;
        }

        if let Some(value) = cache.remove(&key) {
            cache.entry(canonical.to_string()).or_insert(value);
            info!(
                "[keychain] migrated legacy alias '{}' to '{}'",
                key, canonical
            );
            migrated = true;
        }
    }

    migrated
}

// ─────────────────────────────────────────────────────────────────────────────
// Migration shim — fields that were previously in ThinClawIdentity
// ─────────────────────────────────────────────────────────────────────────────

/// Temporary struct used only during migration — matches the legacy `identity.json`
/// API-key fields so we can deserialise old files and pull keys out of them.
#[derive(Clone, serde::Deserialize, serde::Serialize, Default)]
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
    let loaded = cache_loaded().lock().unwrap_or_else(|e| e.into_inner());
    if !*loaded {
        drop(loaded); // Release lock before calling load_all
        load_all()?;
    }
    Ok(())
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

    let crypto = keychain_crypto().lock().unwrap_or_else(|e| e.into_inner());
    let state = crypto
        .as_ref()
        .ok_or_else(|| "Keychain encryption is not initialized".to_string())?;
    let clean = clean
        .into_iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<HashMap<_, _>>();
    let envelope = encode_keychain_blob(&clean, state.crypto.as_ref(), state.key_version)?;

    set_generic_password(SERVICE, ACCOUNT, &envelope)
        .map_err(|e| format!("Keychain write failed: {}", e))?;

    info!("[keychain] flushed {} keys to Keychain", clean.len());
    Ok(())
}

/// Re-encrypt the entire Desktop envelope under a newly generated core master
/// key. Persisting the new master key itself is deliberately owned by the
/// caller so it can roll back that Keychain write if envelope rotation fails.
pub fn rotate_encryption_key(new_crypto: Arc<SecretsCrypto>) -> Result<(i32, i32, usize), String> {
    ensure_loaded()?;
    let cache = key_cache().lock().unwrap_or_else(|e| e.into_inner());
    let mut state = keychain_crypto().lock().unwrap_or_else(|e| e.into_inner());
    let current = state
        .as_ref()
        .ok_or_else(|| "Keychain encryption is not initialized".to_string())?;
    let old_key_version = current.key_version;
    let new_key_version = old_key_version
        .checked_add(1)
        .ok_or_else(|| "Keychain encryption key version overflow".to_string())?;
    let previous_envelope = match get_generic_password(SERVICE, ACCOUNT) {
        Ok(bytes) => Some(bytes),
        Err(error) if is_not_found(&error) => None,
        Err(error) => return Err(format!("Keychain rotation read failed: {error}")),
    };
    let envelope = encode_keychain_blob(&cache, new_crypto.as_ref(), new_key_version)?;
    set_generic_password(SERVICE, ACCOUNT, &envelope)
        .map_err(|error| format!("Keychain rotation write failed: {error}"))?;

    let verification = get_generic_password(SERVICE, ACCOUNT)
        .map_err(|error| format!("Keychain rotation verification read failed: {error}"))
        .and_then(|bytes| {
            verify_keychain_blob(&bytes, new_crypto.as_ref(), new_key_version, &cache)
        });
    if let Err(error) = verification {
        let rollback = match previous_envelope {
            Some(bytes) => set_generic_password(SERVICE, ACCOUNT, &bytes)
                .map_err(|rollback_error| rollback_error.to_string()),
            None => delete_generic_password(SERVICE, ACCOUNT)
                .or_else(|rollback_error| {
                    if is_not_found(&rollback_error) {
                        Ok(())
                    } else {
                        Err(rollback_error)
                    }
                })
                .map_err(|rollback_error| rollback_error.to_string()),
        };
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(format!(
                "{error}; previous Keychain envelope rollback failed: {rollback_error}"
            )),
        };
    }

    *state = Some(KeychainCryptoState {
        crypto: new_crypto,
        key_version: new_key_version,
    });
    Ok((old_key_version, new_key_version, cache.len()))
}

pub fn encryption_key_version() -> i32 {
    keychain_crypto()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|state| state.key_version)
        .unwrap_or(INITIAL_KEY_VERSION)
}

pub fn encryption_metadata() -> Result<(i32, usize), String> {
    ensure_loaded()?;
    let key_version = encryption_key_version();
    let stored_secrets = key_cache()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .values()
        .filter(|value| !value.is_empty())
        .count();
    Ok((key_version, stored_secrets))
}

fn is_not_found(e: &KeychainError) -> bool {
    // errSecItemNotFound = -25300
    e.code() == -25300
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_crypto(fill: char) -> SecretsCrypto {
        SecretsCrypto::new(SecretString::from(
            std::iter::repeat_n(fill, 64).collect::<String>(),
        ))
        .expect("test crypto")
    }

    #[test]
    fn encrypted_blob_round_trips_without_plaintext_at_rest() {
        let crypto = test_crypto('a');
        let secrets = HashMap::from([
            ("llm_openai_api_key".to_string(), "sk-sensitive".to_string()),
            ("hf_token".to_string(), "hf-private".to_string()),
        ]);

        let encoded = encode_keychain_blob(&secrets, &crypto, 7).expect("encrypt blob");
        let persisted = String::from_utf8(encoded.clone()).expect("json envelope");
        assert!(!persisted.contains("sk-sensitive"));
        assert!(!persisted.contains("hf-private"));

        let decoded = decode_keychain_blob(&encoded, &crypto).expect("decrypt blob");
        assert!(decoded.encrypted);
        assert_eq!(decoded.key_version, 7);
        assert_eq!(decoded.secrets, secrets);
    }

    #[test]
    fn encrypted_blob_fails_closed_after_tampering() {
        let crypto = test_crypto('b');
        let mut envelope: EncryptedKeychainBlob = serde_json::from_slice(
            &encode_keychain_blob(
                &HashMap::from([("secret".to_string(), "value".to_string())]),
                &crypto,
                1,
            )
            .expect("encrypt blob"),
        )
        .expect("parse envelope");
        let replacement = if envelope.ciphertext.starts_with('0') {
            '1'
        } else {
            '0'
        };
        envelope
            .ciphertext
            .replace_range(..1, &replacement.to_string());
        let tampered = serde_json::to_vec(&envelope).expect("serialize tampered envelope");

        assert!(decode_keychain_blob(&tampered, &crypto).is_err());
    }

    #[test]
    fn legacy_plaintext_blob_is_detected_for_one_time_encryption() {
        let legacy = serde_json::to_vec(&HashMap::from([(
            "openai".to_string(),
            "legacy-value".to_string(),
        )]))
        .expect("legacy json");
        let decoded = decode_keychain_blob(&legacy, &test_crypto('c')).expect("decode legacy");

        assert!(!decoded.encrypted);
        assert_eq!(decoded.key_version, INITIAL_KEY_VERSION);
        assert_eq!(
            decoded.secrets.get("openai").map(String::as_str),
            Some("legacy-value")
        );
    }

    #[test]
    fn rotated_blob_requires_the_new_master_key() {
        let old_crypto = test_crypto('d');
        let new_crypto = test_crypto('e');
        let secrets = HashMap::from([("token".to_string(), "private".to_string())]);
        let rotated = encode_keychain_blob(&secrets, &new_crypto, 2).expect("rotate blob");

        assert!(decode_keychain_blob(&rotated, &old_crypto).is_err());
        let decoded = decode_keychain_blob(&rotated, &new_crypto).expect("new key decrypts");
        assert_eq!(decoded.key_version, 2);
        assert_eq!(decoded.secrets, secrets);
        verify_keychain_blob(&rotated, &new_crypto, 2, &secrets).expect("rotation verifies");
    }

    #[test]
    fn legacy_aliases_are_moved_to_canonical_names() {
        let mut cache = HashMap::from([
            ("anthropic".to_string(), "legacy-anthropic".to_string()),
            ("OPENAI_API_KEY".to_string(), "legacy-openai".to_string()),
            ("custom-id".to_string(), "custom".to_string()),
        ]);

        assert!(migrate_legacy_aliases(&mut cache));
        assert_eq!(
            cache.get("llm_anthropic_api_key").map(String::as_str),
            Some("legacy-anthropic")
        );
        assert_eq!(
            cache.get("llm_openai_api_key").map(String::as_str),
            Some("legacy-openai")
        );
        assert_eq!(cache.get("custom-id").map(String::as_str), Some("custom"));
        assert!(!cache.contains_key("anthropic"));
        assert!(!cache.contains_key("OPENAI_API_KEY"));
    }

    #[test]
    fn canonical_value_wins_when_alias_also_exists() {
        let mut cache = HashMap::from([
            ("llm_anthropic_api_key".to_string(), "canonical".to_string()),
            ("anthropic".to_string(), "legacy".to_string()),
        ]);

        assert!(migrate_legacy_aliases(&mut cache));
        assert_eq!(
            cache.get("llm_anthropic_api_key").map(String::as_str),
            Some("canonical")
        );
        assert!(!cache.contains_key("anthropic"));
    }
}
