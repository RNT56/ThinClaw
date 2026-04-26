//! Unsafe native plugin boundary.
//!
//! Native plugins are deliberately separate from the WASM extension path. Loading
//! a dynamic library is only allowed when settings opt in, the library lives in an
//! allowlisted directory, the manifest policy passes, and the artifact hash
//! matches when one is declared.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use libloading::Library;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::extensions::manifest::{
    NativePluginAbi, NativePluginContribution, PluginArtifactKind, PluginManifest,
    validate_plugin_manifest, verify_plugin_manifest_signature,
};
use crate::settings::ExtensionsSettings;

const INVOKE_SYMBOL_V1: &[u8] = b"thinclaw_native_plugin_invoke_v1\0";

type NativePluginInvokeV1 =
    unsafe extern "C" fn(*const u8, usize, *mut u8, usize, *mut usize) -> i32;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativePluginRequest {
    pub plugin_id: String,
    pub operation: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativePluginResponse {
    pub ok: bool,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub error: Option<String>,
}

pub struct NativePluginRuntime {
    plugin_id: String,
    max_request_bytes: usize,
    max_response_bytes: usize,
    _library: Library,
    invoke: NativePluginInvokeV1,
}

impl fmt::Debug for NativePluginRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativePluginRuntime")
            .field("plugin_id", &self.plugin_id)
            .field("max_request_bytes", &self.max_request_bytes)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish_non_exhaustive()
    }
}

impl NativePluginRuntime {
    /// # Safety
    ///
    /// Loading native plugin libraries executes process-local dynamic code from
    /// the resolved plugin artifact. Callers must only invoke this after the
    /// manifest has been trusted and the artifact path has passed allowlist
    /// validation.
    pub unsafe fn load(
        manifest: &PluginManifest,
        contribution: &NativePluginContribution,
        plugin_root: &Path,
        settings: &ExtensionsSettings,
    ) -> Result<Self> {
        if !settings.allow_native_plugins {
            bail!("native plugin loading requires extensions.allow_native_plugins=true");
        }
        let validation = validate_plugin_manifest(manifest, settings);
        if !validation.valid {
            bail!(
                "plugin manifest failed validation: {}",
                validation.errors.join("; ")
            );
        }
        if settings.require_plugin_signatures {
            verify_plugin_manifest_signature(manifest, settings)
                .map_err(|err| anyhow!("plugin manifest signature check failed: {err}"))?;
        }
        if contribution.abi != NativePluginAbi::CAbiJsonV1 {
            bail!("native plugin '{}' uses unsupported ABI", contribution.id);
        }

        let artifact = manifest
            .artifacts
            .iter()
            .find(|artifact| artifact.id == contribution.artifact)
            .ok_or_else(|| {
                anyhow!(
                    "native plugin artifact '{}' was not declared",
                    contribution.artifact
                )
            })?;
        if artifact.kind != PluginArtifactKind::NativeDylib {
            bail!(
                "native plugin artifact '{}' must have kind native_dylib",
                artifact.id
            );
        }

        let library_path = resolve_plugin_artifact_path(plugin_root, &artifact.path)?;
        ensure_native_path_allowed(&library_path, settings)?;
        if let Some(expected_sha256) = artifact.sha256.as_deref() {
            verify_sha256(&library_path, expected_sha256)?;
        }

        let library = unsafe { Library::new(&library_path) }.with_context(|| {
            format!(
                "failed to load native plugin library {}",
                library_path.display()
            )
        })?;
        let invoke_symbol = unsafe { library.get::<NativePluginInvokeV1>(INVOKE_SYMBOL_V1) }
            .with_context(|| {
                format!(
                    "native plugin library {} does not export thinclaw_native_plugin_invoke_v1",
                    library_path.display()
                )
            })?;
        let invoke = *invoke_symbol;

        Ok(Self {
            plugin_id: contribution.id.clone(),
            max_request_bytes: usize::try_from(contribution.max_request_bytes)
                .map_err(|_| anyhow!("native plugin request byte limit is too large"))?,
            max_response_bytes: usize::try_from(contribution.max_response_bytes)
                .map_err(|_| anyhow!("native plugin response byte limit is too large"))?,
            _library: library,
            invoke,
        })
    }

    pub fn invoke_json(
        &self,
        operation: impl Into<String>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let request = NativePluginRequest {
            plugin_id: self.plugin_id.clone(),
            operation: operation.into(),
            payload,
        };
        let request_bytes =
            serde_json::to_vec(&request).context("failed to serialize native plugin request")?;
        if request_bytes.len() > self.max_request_bytes {
            bail!(
                "native plugin request exceeds {} byte limit",
                self.max_request_bytes
            );
        }
        let mut response_bytes = vec![0u8; self.max_response_bytes];
        let mut response_len = 0usize;
        let code = unsafe {
            (self.invoke)(
                request_bytes.as_ptr(),
                request_bytes.len(),
                response_bytes.as_mut_ptr(),
                response_bytes.len(),
                &mut response_len,
            )
        };
        if code != 0 {
            bail!("native plugin returned ABI error code {code}");
        }
        if response_len > response_bytes.len() {
            bail!("native plugin response exceeded declared buffer");
        }
        response_bytes.truncate(response_len);
        let response: NativePluginResponse = serde_json::from_slice(&response_bytes)
            .context("native plugin returned invalid JSON response")?;
        if !response.ok {
            bail!(
                "{}",
                response
                    .error
                    .unwrap_or_else(|| "native plugin reported failure".to_string())
            );
        }
        Ok(response.payload)
    }
}

pub fn resolve_plugin_artifact_path(plugin_root: &Path, artifact_path: &str) -> Result<PathBuf> {
    let path = Path::new(artifact_path);
    if path.is_absolute() {
        bail!("plugin artifact paths must be relative to the plugin root");
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("plugin artifact paths may not contain '..'");
    }
    plugin_root.join(path).canonicalize().with_context(|| {
        format!(
            "failed to resolve plugin artifact path {} under {}",
            artifact_path,
            plugin_root.display()
        )
    })
}

pub fn ensure_native_path_allowed(path: &Path, settings: &ExtensionsSettings) -> Result<()> {
    if settings.native_plugin_allowlist_dirs.is_empty() {
        bail!("native plugin loading requires extensions.native_plugin_allowlist_dirs");
    }
    let allowed = settings
        .native_plugin_allowlist_dirs
        .iter()
        .filter_map(|dir| Path::new(dir).canonicalize().ok())
        .any(|dir| path.starts_with(dir));
    if !allowed {
        bail!(
            "native plugin library {} is outside extensions.native_plugin_allowlist_dirs",
            path.display()
        );
    }
    Ok(())
}

fn verify_sha256(path: &Path, expected_hex: &str) -> Result<()> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read native plugin artifact {}", path.display()))?;
    let digest = Sha256::digest(&bytes);
    let actual = hex::encode(digest);
    if !actual.eq_ignore_ascii_case(expected_hex) {
        bail!(
            "native plugin artifact hash mismatch for {}: expected {}, got {}",
            path.display(),
            expected_hex,
            actual
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use tempfile::tempdir;

    use super::*;
    use crate::extensions::manifest::{
        NATIVE_PLUGIN_ABI_VERSION, PluginArtifact, PluginContributions, PluginSignature,
    };

    fn native_manifest(path: &str, sha256: Option<String>) -> PluginManifest {
        PluginManifest {
            schema_version: crate::extensions::manifest::PLUGIN_MANIFEST_SCHEMA_VERSION,
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
                sha256,
            }],
            signature: Some(PluginSignature {
                key_id: "test-key".to_string(),
                algorithm: "ed25519".to_string(),
                signature: "aa".repeat(64),
            }),
        }
    }

    #[test]
    fn native_loading_is_disabled_by_default() {
        let manifest = native_manifest("libmissing.dylib", None);
        let contribution = manifest.contributions.native_plugins.first().unwrap();
        let err = unsafe {
            NativePluginRuntime::load(
                &manifest,
                contribution,
                Path::new("."),
                &ExtensionsSettings::default(),
            )
        }
        .expect_err("native loading should require explicit opt in");
        assert!(err.to_string().contains("allow_native_plugins"));
    }

    #[test]
    fn native_artifact_paths_must_be_relative() {
        let err = resolve_plugin_artifact_path(Path::new("."), "/tmp/libexample.dylib")
            .expect_err("absolute paths should be rejected");
        assert!(err.to_string().contains("relative"));
        let err = resolve_plugin_artifact_path(Path::new("."), "../libexample.dylib")
            .expect_err("parent traversal should be rejected");
        assert!(err.to_string().contains(".."));
    }

    #[test]
    fn native_path_requires_allowlisted_directory() {
        let dir = tempdir().expect("tempdir");
        let lib = dir.path().join("libexample.dylib");
        fs::write(&lib, b"not a real library").expect("write");
        let mut settings = ExtensionsSettings {
            allow_native_plugins: true,
            require_plugin_signatures: false,
            ..ExtensionsSettings::default()
        };
        let path = lib.canonicalize().expect("canonicalize");
        let err =
            ensure_native_path_allowed(&path, &settings).expect_err("empty allowlist rejects");
        assert!(err.to_string().contains("allowlist"));
        settings
            .native_plugin_allowlist_dirs
            .push(dir.path().display().to_string());
        ensure_native_path_allowed(&path, &settings).expect("allowlisted path should pass");
    }

    #[test]
    fn native_artifact_hash_is_checked_before_loading() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("libexample.dylib"), b"not a real library").expect("write");
        let manifest = native_manifest("libexample.dylib", Some("00".repeat(32)));
        let contribution = manifest.contributions.native_plugins.first().unwrap();
        let settings = ExtensionsSettings {
            allow_native_plugins: true,
            require_plugin_signatures: false,
            native_plugin_allowlist_dirs: vec![dir.path().display().to_string()],
            ..ExtensionsSettings::default()
        };
        let err =
            unsafe { NativePluginRuntime::load(&manifest, contribution, dir.path(), &settings) }
                .expect_err("hash mismatch should reject before libloading");
        assert!(err.to_string().contains("hash mismatch"));
    }

    #[test]
    fn native_c_abi_json_v1_invokes_successfully_when_allowlisted() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("native_echo.c");
        let library_name = format!(
            "{}native_echo.{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_EXTENSION
        );
        let library = dir.path().join(&library_name);
        fs::write(
            &source,
            r#"
#include <stddef.h>
#include <stdint.h>
#include <string.h>

int thinclaw_native_plugin_invoke_v1(
    const uint8_t* request,
    size_t request_len,
    uint8_t* response,
    size_t response_cap,
    size_t* response_len
) {
    (void)request;
    (void)request_len;
    const char* payload = "{\"ok\":true,\"payload\":{\"echo\":\"native-ok\"}}";
    size_t len = strlen(payload);
    if (response_cap < len) {
        *response_len = len;
        return 2;
    }
    memcpy(response, payload, len);
    *response_len = len;
    return 0;
}
"#,
        )
        .expect("write C fixture");

        let mut command = Command::new("cc");
        #[cfg(target_os = "macos")]
        command.arg("-dynamiclib");
        #[cfg(not(target_os = "macos"))]
        command.args(["-shared", "-fPIC"]);
        let status = command.arg(&source).arg("-o").arg(&library).status();
        let Ok(status) = status else {
            eprintln!("skipping native C ABI smoke: cc is unavailable");
            return;
        };
        if !status.success() {
            eprintln!("skipping native C ABI smoke: cc failed with {status}");
            return;
        }

        let manifest = native_manifest(&library_name, None);
        let contribution = manifest.contributions.native_plugins.first().unwrap();
        let settings = ExtensionsSettings {
            allow_native_plugins: true,
            require_plugin_signatures: false,
            native_plugin_allowlist_dirs: vec![dir.path().display().to_string()],
            ..ExtensionsSettings::default()
        };
        let runtime =
            unsafe { NativePluginRuntime::load(&manifest, contribution, dir.path(), &settings) }
                .expect("load native smoke dylib");
        let payload = runtime
            .invoke_json("echo", serde_json::json!({ "message": "hello" }))
            .expect("invoke native smoke dylib");
        assert_eq!(payload, serde_json::json!({ "echo": "native-ok" }));
    }
}
