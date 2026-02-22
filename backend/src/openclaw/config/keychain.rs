//! macOS Keychain integration for secure API key storage.
//!
//! Each provider API key is stored as a separate Keychain item:
//!   - Service:  `com.schack.scrappy`
//!   - Account:  the provider slug, e.g. `"anthropic"`, `"openai"`, `"bedrock_access_key_id"`
//!   - Password: the raw key/token string (UTF-8)
//!
//! Advantages over plaintext `identity.json`:
//!   - Encrypted at rest by the OS (protected by the user's login password / Secure Enclave)
//!   - Other processes cannot read these values without explicit Keychain access approval
//!   - Not included in unencrypted backups (iCloud Keychain syncs, but that's user-controlled)
//!
//! # Migration
//! On first launch after upgrade, `migrate_from_identity` checks each provider field in the
//! legacy `identity.json` dict and, if a value is present, imports it into the Keychain and
//! then clears it from the JSON file so it is no longer stored in plaintext.

use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};
use tracing::{info, warn};

/// The Keychain service name — matches the app bundle identifier.
const SERVICE: &str = "com.schack.scrappy";

/// Provider slugs — each maps to a Keychain `account` string.
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
    // Bedrock stores three separate fields
    "bedrock_access_key_id",
    "bedrock_secret_access_key",
    "bedrock_region",
    // Custom LLM
    "custom_llm_key",
    // Remote gateway token
    "remote_token",
];

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Store `value` in the Keychain under `account`.
/// Passing `None` or an empty string deletes the entry.
pub fn set_key(account: &str, value: Option<&str>) -> Result<(), String> {
    match value {
        Some(v) if !v.is_empty() => {
            set_generic_password(SERVICE, account, v.as_bytes())
                .map_err(|e| format!("Keychain write failed for '{}': {}", account, e))?;
            info!("[keychain] stored '{}'", account);
        }
        _ => {
            // Ignore NotFound errors on delete — key may not exist yet
            match delete_generic_password(SERVICE, account) {
                Ok(()) => info!("[keychain] deleted '{}'", account),
                Err(e) if is_not_found(&e) => {} // already absent — fine
                Err(e) => warn!("[keychain] delete '{}' failed: {}", account, e),
            }
        }
    }
    Ok(())
}

/// Retrieve a value from the Keychain. Returns `None` if not found.
pub fn get_key(account: &str) -> Option<String> {
    match get_generic_password(SERVICE, account) {
        Ok(bytes) => String::from_utf8(bytes)
            .map_err(|e| warn!("[keychain] utf8 decode error for '{}': {}", account, e))
            .ok(),
        Err(e) if is_not_found(&e) => None,
        Err(e) => {
            warn!("[keychain] read '{}' failed: {}", account, e);
            None
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Migration helper
// ─────────────────────────────────────────────────────────────────────────────

/// Migrate plaintext API keys from a legacy `identity.json` value into the Keychain.
///
/// Call this once in `OpenClawConfig::new()` after loading the identity file.
/// For every field that is `Some(non-empty)`, it is written to the Keychain and
/// cleared from `identity` so the caller can write back a sanitised JSON.
///
/// Returns `true` if any migration was performed (caller should `save_identity`).
pub fn migrate_from_identity(identity: &mut LegacyKeys) -> bool {
    let mut migrated = false;

    macro_rules! migrate {
        ($field:expr, $account:expr) => {
            if let Some(ref val) = $field {
                if !val.is_empty() {
                    if let Err(e) = set_key($account, Some(val)) {
                        warn!("[keychain] migration failed for '{}': {}", $account, e);
                    } else {
                        info!("[keychain] migrated '{}' from identity.json", $account);
                        $field = None;
                        migrated = true;
                    }
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

    migrated
}

// ─────────────────────────────────────────────────────────────────────────────
// Migration shim — fields that were previously in OpenClawIdentity
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

fn is_not_found(e: &security_framework::base::Error) -> bool {
    // errSecItemNotFound = -25300
    e.code() == -25300
}
