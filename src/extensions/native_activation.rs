//! Native dynamic-library plugin activation state and panic-isolated invocation.
//!
//! # Security model
//!
//! Native plugins are the **operator-only**, opt-in, NON-sandboxed extension
//! path. They run in-process with full host privileges (this is the deliberate
//! trade for native performance). The entire pipeline is gated so that **no
//! native code is ever loaded unless an operator explicitly opts in**:
//!
//! - **Default-off.** Nothing here loads a library unless
//!   `extensions.allow_native_plugins` is `true`. This module never
//!   auto-discovers or auto-loads anything on its own; the manager only feeds it
//!   manifests an operator placed in an allowlisted directory.
//! - **Signature-before-load.** [`NativePluginRuntime::load`] performs manifest
//!   validation, ed25519 signature verification (when
//!   `require_plugin_signatures` is `true`, the default), ABI/version checks,
//!   path-traversal/allowlist enforcement, and a SHA-256 pin **before** the
//!   `libloading::Library::new` call. This module calls `load` and trusts those
//!   gates rather than re-implementing or weakening them. If any gate fails, no
//!   `dlopen` happens.
//! - **Best-effort unwind handling.** Every `invoke_json` call is wrapped in
//!   [`std::panic::catch_unwind`], so an unwind that reaches Rust is recorded as
//!   a health failure. Native code still runs in-process: an abort, segfault, or
//!   a panic crossing a non-unwind C ABI can terminate the host. Strong fault
//!   isolation requires moving native plugins to a separate process.
//! - **Health-tracked.** Each loaded plugin is registered with an
//!   [`ExtensionHealthMonitor`]; successes/failures (including caught panics)
//!   drive its state machine so a repeatedly-failing plugin becomes `Unhealthy`
//!   and is observable, instead of being silently re-invoked forever.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::extensions::ext_health_monitor::{ExtensionHealthConfig, ExtensionHealthMonitor};
use crate::extensions::manifest::{NativePluginContribution, PluginManifest};
use crate::extensions::native::NativePluginRuntime;
use crate::extensions::{ActivateResult, ExtensionError, ExtensionKind};
use crate::settings::ExtensionsSettings;

/// A native-plugin contribution that has been registered from a manifest scan.
///
/// Registration does **not** load any code; it only records the manifest, the
/// contribution, and the plugin root directory so the operator can later
/// explicitly activate it. The manifest still has to pass every gate inside
/// [`NativePluginRuntime::load`] at activation time.
#[derive(Clone)]
pub struct RegisteredNativePlugin {
    pub manifest: PluginManifest,
    pub contribution: NativePluginContribution,
    pub plugin_root: PathBuf,
}

/// All native-plugin runtime state owned by the extension manager.
///
/// Holds the registered (but not yet loaded) contributions, the loaded runtimes
/// keyed by contribution id, and the shared health monitor. This is intentionally
/// a focused submodule struct so the 3000+ line `manager.rs` does not grow.
pub struct NativePluginState {
    /// Native contributions registered from manifest scans, keyed by contribution id.
    /// Recording here loads nothing — activation is a separate, explicit step.
    registered: HashMap<String, RegisteredNativePlugin>,
    /// Loaded native runtimes keyed by contribution id.
    loaded: HashMap<String, Arc<NativePluginRuntime>>,
    /// Health state for active extensions (native and, optionally, others).
    health: ExtensionHealthMonitor,
}

impl Default for NativePluginState {
    fn default() -> Self {
        Self::new()
    }
}

impl NativePluginState {
    pub fn new() -> Self {
        Self {
            registered: HashMap::new(),
            loaded: HashMap::new(),
            health: ExtensionHealthMonitor::new(ExtensionHealthConfig::default()),
        }
    }

    /// Record a native contribution for later, operator-initiated activation.
    ///
    /// This stores metadata only. No library is loaded here.
    pub fn register(&mut self, plugin: RegisteredNativePlugin) {
        self.registered
            .insert(plugin.contribution.id.clone(), plugin);
    }

    /// True if a native contribution with this id has been registered.
    pub fn is_registered(&self, contribution_id: &str) -> bool {
        self.registered.contains_key(contribution_id)
    }

    /// True if a native contribution with this id is currently loaded.
    pub fn is_loaded(&self, contribution_id: &str) -> bool {
        self.loaded.contains_key(contribution_id)
    }

    /// Names of all registered native contributions.
    pub fn registered_ids(&self) -> Vec<String> {
        self.registered.keys().cloned().collect()
    }

    /// Health monitor summary, for status surfaces.
    pub fn health_summary(&self) -> crate::extensions::ext_health_monitor::HealthSummary {
        self.health.summary()
    }

    /// Load and activate a previously-registered native plugin.
    ///
    /// # Safety / trust boundary
    ///
    /// This is the single reachable `dlopen` path. It only proceeds for a
    /// contribution that was registered from a scanned manifest, and the actual
    /// load is delegated to [`NativePluginRuntime::load`], which runs every
    /// signature/ABI/allowlist/hash gate **before** opening the library. The
    /// `settings` passed here are the live extension settings; if
    /// `allow_native_plugins` is `false`, `load` refuses before any `dlopen`.
    pub fn activate(
        &mut self,
        contribution_id: &str,
        settings: &ExtensionsSettings,
    ) -> Result<ActivateResult, ExtensionError> {
        // Fail-closed: never reach `dlopen` when the operator has not opted in.
        if !settings.allow_native_plugins {
            return Err(ExtensionError::ActivationFailed(
                "native plugins are disabled; set extensions.allow_native_plugins=true to enable"
                    .to_string(),
            ));
        }

        let registered = self.registered.get(contribution_id).cloned().ok_or_else(|| {
            ExtensionError::NotInstalled(format!(
                "native plugin '{contribution_id}' is not registered; place a signed manifest in an allowlisted directory and rescan"
            ))
        })?;

        // SAFETY: every trust gate (allow flag, manifest validation, ed25519
        // signature verification, ABI/version, path traversal/allowlist, SHA-256)
        // runs inside `load` before `libloading::Library::new`. We do not load on
        // any gate failure.
        let runtime = unsafe {
            NativePluginRuntime::load(
                &registered.manifest,
                &registered.contribution,
                &registered.plugin_root,
                settings,
            )
        }
        .map_err(|err| {
            ExtensionError::ActivationFailed(format!(
                "native plugin '{contribution_id}' failed to load: {err}"
            ))
        })?;

        self.loaded
            .insert(contribution_id.to_string(), Arc::new(runtime));

        // Register for health monitoring on first activation; starts as Unknown.
        self.health.register(contribution_id);

        Ok(ActivateResult {
            name: contribution_id.to_string(),
            kind: ExtensionKind::NativePlugin,
            tools_loaded: vec![contribution_id.to_string()],
            message: format!(
                "Native plugin '{contribution_id}' loaded (in-process, full host privileges). \
                 Signature, ABI, allowlist, and hash checks passed before load."
            ),
        })
    }

    /// Deactivate (unload) a native plugin, dropping its loaded runtime.
    ///
    /// Dropping the [`NativePluginRuntime`] drops the underlying `libloading`
    /// handle. Returns `true` if a runtime was actually unloaded.
    pub fn deactivate(&mut self, contribution_id: &str) -> bool {
        self.loaded.remove(contribution_id).is_some()
    }

    /// Invoke a loaded native plugin operation with best-effort unwind handling.
    ///
    /// An unwind that reaches this Rust frame is caught, recorded as a health
    /// failure, and converted into an error. Native aborts, faults, and panics
    /// that cannot unwind across the C ABI can still terminate the process.
    pub fn invoke(
        &mut self,
        contribution_id: &str,
        operation: &str,
        payload: serde_json::Value,
        timestamp: &str,
    ) -> Result<serde_json::Value, ExtensionError> {
        let runtime = self.loaded.get(contribution_id).cloned().ok_or_else(|| {
            ExtensionError::ActivationFailed(format!(
                "native plugin '{contribution_id}' is not active"
            ))
        })?;

        // Catch only unwinds that reach this Rust frame. Native aborts/faults
        // remain process-fatal because plugins currently execute in-process.
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            runtime.invoke_json(operation.to_string(), payload)
        }));

        match outcome {
            Ok(Ok(value)) => {
                self.health.record_success(contribution_id, timestamp);
                Ok(value)
            }
            Ok(Err(err)) => {
                self.health.record_failure(contribution_id, timestamp);
                Err(ExtensionError::Other(format!(
                    "native plugin '{contribution_id}' invocation failed: {err}"
                )))
            }
            Err(_panic) => {
                self.health.record_failure(contribution_id, timestamp);
                Err(ExtensionError::Other(format!(
                    "native plugin '{contribution_id}' unwound during invocation"
                )))
            }
        }
    }

    /// Current health status label for a contribution, if tracked.
    pub fn health_label(&self, contribution_id: &str) -> Option<&'static str> {
        self.health
            .get_status(contribution_id)
            .map(|status| status.status.label())
    }
}

/// Recognize whether a path is a native-plugin manifest the manager can scan.
///
/// A native manifest is a JSON file declaring at least one native contribution.
/// Returns the parsed manifest only when it actually contributes native plugins,
/// so non-native manifests are ignored by the native scan path.
pub fn parse_native_manifest(path: &Path) -> Option<PluginManifest> {
    let bytes =
        thinclaw_platform::read_regular_file_bounded_single_link(path, 4 * 1024 * 1024).ok()?;
    let manifest: PluginManifest = serde_json::from_slice(&bytes).ok()?;
    if manifest.contributions.native_plugins.is_empty() {
        return None;
    }
    Some(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::manifest::{
        NATIVE_PLUGIN_ABI_VERSION, NativePluginAbi, PLUGIN_MANIFEST_SCHEMA_VERSION, PluginArtifact,
        PluginArtifactKind, PluginContributions, PluginSignature,
    };

    fn native_manifest(path: &str) -> PluginManifest {
        PluginManifest {
            schema_version: PLUGIN_MANIFEST_SCHEMA_VERSION,
            id: "native-example".to_string(),
            name: "Native Example".to_string(),
            version: "1.0.0".to_string(),
            publisher: None,
            description: None,
            permissions: vec!["native_plugin".to_string()],
            contributions: PluginContributions {
                native_plugins: vec![NativePluginContribution {
                    id: "native.echo".to_string(),
                    artifact: "native-lib".to_string(),
                    abi: NativePluginAbi::CAbiJsonV1,
                    abi_version: NATIVE_PLUGIN_ABI_VERSION,
                    max_request_bytes: 1024,
                    max_response_bytes: 1024,
                }],
                ..PluginContributions::default()
            },
            artifacts: vec![PluginArtifact {
                id: "native-lib".to_string(),
                kind: PluginArtifactKind::NativeDylib,
                path: path.to_string(),
                sha256: None,
            }],
            signature: Some(PluginSignature {
                key_id: "test-key".to_string(),
                algorithm: "ed25519".to_string(),
                signature: "aa".repeat(64),
            }),
        }
    }

    fn registered(manifest: PluginManifest) -> RegisteredNativePlugin {
        let contribution = manifest.contributions.native_plugins[0].clone();
        RegisteredNativePlugin {
            manifest,
            contribution,
            plugin_root: PathBuf::from("."),
        }
    }

    #[test]
    fn default_settings_never_activate() {
        // Default-off: even a registered contribution must not load with default
        // settings (allow_native_plugins=false).
        let mut state = NativePluginState::new();
        state.register(registered(native_manifest("libmissing.dylib")));
        let settings = ExtensionsSettings::default();
        assert!(!settings.allow_native_plugins);
        let err = state
            .activate("native.echo", &settings)
            .expect_err("default settings must refuse activation");
        assert!(matches!(err, ExtensionError::ActivationFailed(_)));
        assert!(err.to_string().contains("allow_native_plugins"));
        // Nothing was loaded.
        assert!(!state.is_loaded("native.echo"));
    }

    #[test]
    fn unsigned_manifest_is_refused_before_load() {
        // Signature gate: an unsigned manifest with require_plugin_signatures=true
        // must be refused before any dlopen. We point at a missing artifact so
        // that, if (incorrectly) the loader proceeded to `libloading`, it would
        // fail with a different (load) error — proving the rejection is the
        // signature gate, not a load failure.
        let mut manifest = native_manifest("libmissing.dylib");
        manifest.signature = None;
        let mut state = NativePluginState::new();
        state.register(registered(manifest));
        let settings = ExtensionsSettings {
            allow_native_plugins: true,
            require_plugin_signatures: true,
            trusted_manifest_keys: vec!["test-key".to_string()],
            ..ExtensionsSettings::default()
        };
        let err = state
            .activate("native.echo", &settings)
            .expect_err("unsigned manifest must be refused");
        // The failure is the signature gate, not a libloading error.
        assert!(err.to_string().contains("signature"));
        assert!(!err.to_string().contains("libloading"));
        assert!(!state.is_loaded("native.echo"));
    }

    #[test]
    fn unregistered_contribution_is_not_installed() {
        let mut state = NativePluginState::new();
        let settings = ExtensionsSettings {
            allow_native_plugins: true,
            require_plugin_signatures: false,
            ..ExtensionsSettings::default()
        };
        let err = state
            .activate("native.unknown", &settings)
            .expect_err("unregistered contribution must not activate");
        assert!(matches!(err, ExtensionError::NotInstalled(_)));
    }

    #[test]
    fn parse_native_manifest_ignores_non_native() {
        // A manifest with no native contributions is not a native scan target.
        let mut manifest = native_manifest("libmissing.dylib");
        manifest.contributions.native_plugins.clear();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("plugin.json");
        std::fs::write(&path, serde_json::to_vec(&manifest).expect("ser")).expect("write");
        assert!(parse_native_manifest(&path).is_none());
    }

    #[test]
    fn parse_native_manifest_recognizes_native() {
        let manifest = native_manifest("libmissing.dylib");
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("plugin.json");
        std::fs::write(&path, serde_json::to_vec(&manifest).expect("ser")).expect("write");
        let parsed = parse_native_manifest(&path).expect("native manifest recognized");
        assert_eq!(parsed.contributions.native_plugins.len(), 1);
    }
}
