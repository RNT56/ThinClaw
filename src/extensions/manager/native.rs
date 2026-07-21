//! Native dynamic-library plugins (operator-only, default-off).
//!
//! SECURITY MODEL (see also `native_activation.rs` and `native.rs`):
//!   * Default-off: nothing here loads native code unless
//!     `extensions.allow_native_plugins` is explicitly true.
//!   * No auto-discovery/auto-load: a manifest is only registered when the
//!     operator places a signed manifest in an allowlisted directory and a
//!     scan is invoked. Registration loads NO code.
//!   * Signature-before-load: `NativePluginRuntime::load` verifies the
//!     ed25519 manifest signature (and ABI/version, path allowlist, SHA-256)
//!     BEFORE any `dlopen`. We never bypass or weaken those gates.
//!   * In-process, full host privileges: native plugins are NOT sandboxed
//!     like WASM. Only signed, audited binaries should be trusted.

use crate::extensions::native_activation::RegisteredNativePlugin;
use crate::extensions::{ActivateResult, ExtensionError};
use crate::settings::Settings;

use super::ExtensionManager;

const MAX_NATIVE_MANIFEST_DIRECTORY_ENTRIES: usize = 4_096;
const MAX_NATIVE_MANIFESTS_PER_DIRECTORY: usize = 256;
const MAX_NATIVE_PLUGIN_ALLOWLIST_DIRS: usize = 64;

impl ExtensionManager {
    /// Load the live extension settings (DB-backed if available, else file).
    ///
    /// Used to gate native-plugin loading against the operator's current
    /// `allow_native_plugins`/signature/allowlist configuration rather than a
    /// stale snapshot.
    async fn current_extensions_settings(&self) -> crate::settings::ExtensionsSettings {
        if let Some(ref store) = self.store
            && let Ok(map) = store.get_all_settings(&self.user_id).await
        {
            return Settings::from_db_map(&map).extensions;
        }
        Settings::load().extensions
    }

    /// Scan a directory for signed plugin manifests and register any native
    /// contributions for later, operator-initiated activation.
    ///
    /// This is the entrypoint that makes the native pipeline reachable. It is
    /// strictly gated: when `allow_native_plugins` is false (the default), native
    /// contributions are skipped and nothing is registered or loaded. When it is
    /// enabled, each manifest must pass full policy + signature validation (via
    /// `register_plugin_manifest_contributions`) before its native contributions
    /// are recorded. Recording still loads NO code — `activate` is a separate,
    /// equally-gated step.
    ///
    /// Returns the list of native contribution ids that were registered.
    pub async fn scan_and_register_plugin_manifests(
        &self,
        plugins_dir: &std::path::Path,
    ) -> Vec<String> {
        let settings = self.current_extensions_settings().await;

        let mut registered_ids = Vec::new();
        if !tokio::fs::symlink_metadata(plugins_dir)
            .await
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        {
            return registered_ids;
        }
        let Ok(mut read_dir) = tokio::fs::read_dir(plugins_dir).await else {
            return registered_ids;
        };

        let mut scanned_entries = 0_usize;
        let mut manifest_count = 0_usize;
        loop {
            let entry = match read_dir.next_entry().await {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(directory = %plugins_dir.display(), %error, "Native manifest scan failed");
                    break;
                }
            };
            scanned_entries = scanned_entries.saturating_add(1);
            if scanned_entries > MAX_NATIVE_MANIFEST_DIRECTORY_ENTRIES {
                tracing::warn!(directory = %plugins_dir.display(), "Native manifest directory exceeds the scan limit");
                break;
            }
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if !tokio::fs::symlink_metadata(&path)
                .await
                .is_ok_and(|metadata| {
                    metadata.is_file()
                        && !metadata.file_type().is_symlink()
                        && metadata.len() <= 4 * 1024 * 1024
                })
            {
                continue;
            }
            manifest_count = manifest_count.saturating_add(1);
            if manifest_count > MAX_NATIVE_MANIFESTS_PER_DIRECTORY {
                tracing::warn!(directory = %plugins_dir.display(), "Native manifest count exceeds the scan limit");
                break;
            }
            let Some(manifest) = crate::extensions::native_activation::parse_native_manifest(&path)
            else {
                continue;
            };

            // Route non-native contributions through the existing registry path
            // and let it apply policy + signature validation. This also produces
            // the `native_plugins_available`/`native_plugins_skipped` accounting.
            match self
                .registry
                .register_plugin_manifest_contributions(&manifest, &settings)
                .await
            {
                Ok(registration) => {
                    // Default-off: only register native contributions for
                    // activation when the operator opted in. `native_plugins_available`
                    // is empty unless `allow_native_plugins` is true.
                    if registration.native_plugins_available.is_empty() {
                        if !registration.native_plugins_skipped.is_empty() {
                            tracing::debug!(
                                manifest = %manifest.id,
                                skipped = ?registration.native_plugins_skipped,
                                "Native plugin contributions skipped (allow_native_plugins disabled)"
                            );
                        }
                        continue;
                    }

                    let plugin_root = path
                        .parent()
                        .map(std::path::Path::to_path_buf)
                        .unwrap_or_else(|| plugins_dir.to_path_buf());
                    let mut state = self.native_plugins.write().await;
                    for contribution in &manifest.contributions.native_plugins {
                        if registration
                            .native_plugins_available
                            .iter()
                            .any(|id| id == &contribution.id)
                        {
                            state.register(RegisteredNativePlugin {
                                manifest: manifest.clone(),
                                contribution: contribution.clone(),
                                plugin_root: plugin_root.clone(),
                            });
                            registered_ids.push(contribution.id.clone());
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        manifest = %manifest.id,
                        error = %e,
                        "Plugin manifest failed validation during native scan"
                    );
                }
            }
        }

        registered_ids
    }

    /// Scan every operator-configured native-plugin allowlist directory and
    /// register any signed manifests found.
    ///
    /// Default-off: returns immediately (registering nothing) unless
    /// `allow_native_plugins` is enabled, and the allowlist is empty by default.
    /// Registration loads no code — activation (which performs the signature-gated
    /// `dlopen`) remains an explicit, separate operator action.
    pub async fn register_native_plugins_from_allowlist(&self) -> Vec<String> {
        let settings = self.current_extensions_settings().await;
        if !settings.allow_native_plugins || settings.native_plugin_allowlist_dirs.is_empty() {
            return Vec::new();
        }
        let mut registered = Vec::new();
        for dir in settings
            .native_plugin_allowlist_dirs
            .iter()
            .take(MAX_NATIVE_PLUGIN_ALLOWLIST_DIRS)
        {
            if dir.is_empty() || dir.len() > 4_096 || dir.contains('\0') {
                continue;
            }
            let expanded = crate::platform::expand_home_dir(dir);
            registered.extend(
                self.scan_and_register_plugin_manifests(std::path::Path::new(&expanded))
                    .await,
            );
        }
        if settings.native_plugin_allowlist_dirs.len() > MAX_NATIVE_PLUGIN_ALLOWLIST_DIRS {
            tracing::warn!("Native plugin allowlist directory count exceeds the scan limit");
        }
        if !registered.is_empty() {
            tracing::info!(
                count = registered.len(),
                "Registered native plugin manifests from operator allowlist"
            );
        }
        registered
    }

    /// Activate (load) a registered native plugin.
    ///
    /// Delegates to [`NativePluginState::activate`], which calls the unsafe
    /// `NativePluginRuntime::load`. Every trust gate runs inside `load` BEFORE
    /// any `dlopen`; if `allow_native_plugins` is off, load refuses first.
    pub(super) async fn activate_native_plugin(
        &self,
        name: &str,
    ) -> Result<ActivateResult, ExtensionError> {
        let settings = self.current_extensions_settings().await;
        self.native_plugins.write().await.activate(name, &settings)
    }

    /// Invoke an operation on an active native plugin, with best-effort unwind
    /// handling and health tracking. Native aborts and faults remain process-fatal.
    ///
    /// A panic crossing the C-ABI boundary is caught and recorded as a health
    /// failure rather than crashing the host. Repeated failures drive the plugin
    /// to `Unhealthy` via the shared health monitor.
    pub async fn invoke_native_plugin(
        &self,
        name: &str,
        operation: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, ExtensionError> {
        let timestamp = chrono::Utc::now().to_rfc3339();
        self.native_plugins
            .write()
            .await
            .invoke(name, operation, payload, &timestamp)
    }

    /// Health status label for a native plugin, if it is being tracked.
    pub async fn native_plugin_health(&self, name: &str) -> Option<&'static str> {
        self.native_plugins.read().await.health_label(name)
    }
}
