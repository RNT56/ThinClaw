//! Versioned plugin manifest model and policy validation.
//!
//! This is the broad plugin surface used above the older registry manifests.
//! It describes what a plugin contributes before any runtime-specific loader is
//! allowed to install or activate those contributions.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::extensions::signing::{
    parse_hex_public_key, parse_hex_signature, verify_manifest_signature,
};
use crate::settings::ExtensionsSettings;

pub const PLUGIN_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const NATIVE_PLUGIN_ABI_VERSION: u32 = 1;
pub const EXTENSION_HTTP_PERMISSION: &str = "contribution.http";
pub const EXTENSION_SECRETS_PERMISSION: &str = "contribution.secrets";
const MAX_PLUGIN_CONTRIBUTIONS: usize = 512;
const MAX_PLUGIN_ARTIFACTS: usize = 256;
const MAX_PLUGIN_PERMISSIONS: usize = 128;
const MAX_TRUSTED_MANIFEST_KEYS: usize = 256;

fn valid_manifest_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_manifest_text(value: &str, max_bytes: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.trim().is_empty())
        && value.len() <= max_bytes
        && !value.contains('\0')
        && !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub contributions: PluginContributions,
    #[serde(default)]
    pub artifacts: Vec<PluginArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<PluginSignature>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginContributions {
    #[serde(default)]
    pub tools: Vec<ToolContribution>,
    #[serde(default)]
    pub channels: Vec<ChannelContribution>,
    #[serde(default)]
    pub memory_providers: Vec<MemoryProviderContribution>,
    #[serde(default)]
    pub context_providers: Vec<ContextProviderContribution>,
    #[serde(default)]
    pub auth_providers: Vec<AuthProviderContribution>,
    #[serde(default)]
    pub llm_providers: Vec<LlmProviderContribution>,
    #[serde(default)]
    pub http_routes: Vec<HttpRouteContribution>,
    #[serde(default)]
    pub native_plugins: Vec<NativePluginContribution>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolContribution {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm_artifact: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelContribution {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm_artifact: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryProviderContribution {
    pub id: String,
    /// Legacy descriptive field. A provider is executable only when `runtime`
    /// and `operations` are present and pass validation.
    #[serde(default)]
    pub provider_type: String,
    #[serde(default)]
    pub config_schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<HttpJsonProviderRuntime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operations: Option<MemoryProviderOperations>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextProviderContribution {
    pub id: String,
    /// Legacy descriptive field. A provider is executable only when `runtime`
    /// and `operations` are present and pass validation.
    #[serde(default)]
    pub provider_type: String,
    #[serde(default)]
    pub config_schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<HttpJsonProviderRuntime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operations: Option<ContextProviderOperations>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpJsonProviderRuntime {
    pub base_url: String,
    #[serde(default)]
    pub auth: ExtensionHttpAuth,
    #[serde(default = "default_extension_http_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_extension_http_max_request_bytes")]
    pub max_request_bytes: u64,
    #[serde(default = "default_extension_http_max_response_bytes")]
    pub max_response_bytes: u64,
    #[serde(default)]
    pub lifecycle: ProviderLifecycle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExtensionHttpAuth {
    #[default]
    None,
    BearerSecret {
        /// Opaque name resolved through the host secrets store. The value must
        /// remain inside the manifest/contribution namespace.
        binding: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderLifecycle {
    /// Selected providers are health-checked and activated during startup.
    #[default]
    Startup,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryProviderOperations {
    pub health_path: String,
    pub recall_path: String,
    pub store_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextProviderOperations {
    pub health_path: String,
    pub resolve_path: String,
}

/// Typed declarations for surfaces that are deliberately not executable in
/// manifest schema v1. Their presence is rejected by validation; keeping the
/// shape explicit prevents clients from treating unknown JSON as a capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthProviderContribution {
    pub id: String,
    pub protocol: AuthProviderProtocol,
    pub binding_namespace: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthProviderProtocol {
    OAuth2Pkce,
    ApiKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LlmProviderContribution {
    pub id: String,
    pub protocol: LlmProviderProtocol,
    pub binding_namespace: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderProtocol {
    OpenAiCompatible,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpRouteContribution {
    pub id: String,
    pub route_namespace: String,
    pub auth: HttpRouteAuth,
    pub max_body_bytes: u64,
    pub timeout_ms: u64,
    pub lifecycle: HttpRouteLifecycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpRouteAuth {
    GatewaySession,
    SignedWebhook,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpRouteLifecycle {
    AgentRuntime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContributionSurface {
    MemoryProvider,
    ContextProvider,
    AuthProvider,
    LlmProvider,
    HttpRoute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum ContributionSupportDecision {
    Executable { contract: &'static str },
    Unsupported { reason: &'static str },
}

pub fn contribution_support(surface: ContributionSurface) -> ContributionSupportDecision {
    match surface {
        ContributionSurface::MemoryProvider | ContributionSurface::ContextProvider => {
            ContributionSupportDecision::Executable {
                contract: "host_http_json_v1",
            }
        }
        ContributionSurface::AuthProvider => ContributionSupportDecision::Unsupported {
            reason: "generic auth providers are deferred until host-owned callback, consent, and token lifecycle integration exists",
        },
        ContributionSurface::LlmProvider => ContributionSupportDecision::Unsupported {
            reason: "generic LLM providers are deferred until routing, billing, streaming, and policy integration exists",
        },
        ContributionSurface::HttpRoute => ContributionSupportDecision::Unsupported {
            reason: "generic inbound HTTP routes are deferred until gateway auth, namespace ownership, rate limiting, and shutdown integration exists",
        },
    }
}

fn default_extension_http_timeout_ms() -> u64 {
    5_000
}

fn default_extension_http_max_request_bytes() -> u64 {
    256 * 1024
}

fn default_extension_http_max_response_bytes() -> u64 {
    512 * 1024
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativePluginContribution {
    pub id: String,
    pub artifact: String,
    #[serde(default = "default_native_abi")]
    pub abi: NativePluginAbi,
    #[serde(default = "default_native_abi_version")]
    pub abi_version: u32,
    #[serde(default = "default_native_max_request_bytes")]
    pub max_request_bytes: u64,
    #[serde(default = "default_native_max_response_bytes")]
    pub max_response_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativePluginAbi {
    CAbiJsonV1,
}

fn default_native_abi() -> NativePluginAbi {
    NativePluginAbi::CAbiJsonV1
}

fn default_native_abi_version() -> u32 {
    NATIVE_PLUGIN_ABI_VERSION
}

fn default_native_max_request_bytes() -> u64 {
    1024 * 1024
}

fn default_native_max_response_bytes() -> u64 {
    4 * 1024 * 1024
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginArtifact {
    pub id: String,
    pub kind: PluginArtifactKind,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginArtifactKind {
    Wasm,
    NativeDylib,
    Manifest,
    Data,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSignature {
    pub key_id: String,
    pub algorithm: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManifestValidation {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn validate_plugin_manifest(
    manifest: &PluginManifest,
    settings: &ExtensionsSettings,
) -> PluginManifestValidation {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    let contribution_count = manifest
        .contributions
        .tools
        .len()
        .saturating_add(manifest.contributions.channels.len())
        .saturating_add(manifest.contributions.memory_providers.len())
        .saturating_add(manifest.contributions.context_providers.len())
        .saturating_add(manifest.contributions.auth_providers.len())
        .saturating_add(manifest.contributions.llm_providers.len())
        .saturating_add(manifest.contributions.http_routes.len())
        .saturating_add(manifest.contributions.native_plugins.len());
    if contribution_count > MAX_PLUGIN_CONTRIBUTIONS
        || manifest.artifacts.len() > MAX_PLUGIN_ARTIFACTS
        || manifest.permissions.len() > MAX_PLUGIN_PERMISSIONS
        || settings.trusted_manifest_keys.len() > MAX_TRUSTED_MANIFEST_KEYS
        || settings.trusted_manifest_public_keys.len() > MAX_TRUSTED_MANIFEST_KEYS
    {
        errors.push(
            "plugin manifest exceeds the contribution, artifact, or permission limit".to_string(),
        );
        return PluginManifestValidation {
            valid: false,
            errors,
            warnings,
        };
    }

    if manifest.schema_version != PLUGIN_MANIFEST_SCHEMA_VERSION {
        errors.push(format!(
            "unsupported plugin manifest schema_version {}; expected {}",
            manifest.schema_version, PLUGIN_MANIFEST_SCHEMA_VERSION
        ));
    }
    if !valid_manifest_id(&manifest.id) {
        errors.push("plugin id is missing or invalid".to_string());
    }
    if !valid_manifest_text(&manifest.name, 256, false) {
        errors.push("plugin name is missing or invalid".to_string());
    }
    if manifest.version.len() > 64 || !is_valid_semver(&manifest.version) {
        errors.push(format!("invalid plugin version '{}'", manifest.version));
    }
    let mut permissions = HashSet::new();
    if manifest
        .publisher
        .as_deref()
        .is_some_and(|value| !valid_manifest_text(value, 256, false))
        || manifest
            .description
            .as_deref()
            .is_some_and(|value| !valid_manifest_text(value, 16 * 1024, true))
        || manifest.permissions.iter().any(|permission| {
            !valid_manifest_id(permission) || !permissions.insert(permission.as_str())
        })
    {
        errors.push("plugin metadata or permissions are malformed or oversized".to_string());
    }

    let mut artifact_ids = HashSet::new();
    for artifact in &manifest.artifacts {
        if !valid_manifest_id(&artifact.id)
            || !artifact_ids.insert(artifact.id.as_str())
            || !valid_manifest_text(&artifact.path, 4_096, false)
            || artifact.sha256.as_deref().is_some_and(|digest| {
                digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            errors.push("plugin artifact metadata is invalid or duplicated".to_string());
            break;
        }
    }

    let mut contribution_ids = HashSet::new();

    let has_native = !manifest.contributions.native_plugins.is_empty()
        || manifest
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == PluginArtifactKind::NativeDylib);
    if has_native && !settings.allow_native_plugins {
        errors.push(
            "native plugin contributions require extensions.allow_native_plugins=true".to_string(),
        );
    }
    if settings.require_plugin_signatures && manifest.signature.is_none() {
        errors.push(
            "plugin signature is required by extensions.require_plugin_signatures".to_string(),
        );
    }
    if let Some(signature) = &manifest.signature {
        if !valid_manifest_id(&signature.key_id)
            || signature.algorithm.len() > 32
            || signature.signature.len() != 128
            || !signature
                .signature
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            errors.push("plugin signature metadata is malformed or oversized".to_string());
        }
        if !signature.algorithm.eq_ignore_ascii_case("ed25519") {
            errors.push(format!(
                "unsupported plugin signature algorithm '{}'",
                signature.algorithm
            ));
        }
        if settings.require_plugin_signatures
            && !settings
                .trusted_manifest_keys
                .iter()
                .any(|key| key == &signature.key_id)
        {
            errors.push(format!(
                "plugin signature key '{}' is not trusted",
                signature.key_id
            ));
        }
    }

    for tool in &manifest.contributions.tools {
        if !valid_manifest_id(&tool.id) || !contribution_ids.insert(tool.id.as_str()) {
            errors.push("tool contribution id is invalid or duplicated".to_string());
        }
        if !valid_manifest_text(&tool.name, 256, false) {
            errors.push(format!("tool contribution '{}' name is required", tool.id));
        }
        if let Some(artifact_id) = tool.wasm_artifact.as_deref() {
            validate_artifact_reference(
                manifest,
                artifact_id,
                PluginArtifactKind::Wasm,
                "tool",
                &tool.id,
                &mut errors,
            );
        } else {
            errors.push(format!(
                "tool contribution '{}' is not executable without wasm_artifact",
                tool.id
            ));
        }
    }

    for channel in &manifest.contributions.channels {
        if !valid_manifest_id(&channel.id) || !contribution_ids.insert(channel.id.as_str()) {
            errors.push("channel contribution id is invalid or duplicated".to_string());
        }
        if !valid_manifest_text(&channel.name, 256, false) {
            errors.push(format!(
                "channel contribution '{}' name is required",
                channel.id
            ));
        }
        if let Some(artifact_id) = channel.wasm_artifact.as_deref() {
            validate_artifact_reference(
                manifest,
                artifact_id,
                PluginArtifactKind::Wasm,
                "channel",
                &channel.id,
                &mut errors,
            );
        } else {
            errors.push(format!(
                "channel contribution '{}' is not executable without wasm_artifact",
                channel.id
            ));
        }
    }

    for provider in &manifest.contributions.memory_providers {
        if !valid_manifest_id(&provider.id) || !contribution_ids.insert(provider.id.as_str()) {
            errors.push(
                "memory provider contribution id is required, invalid, or duplicated".to_string(),
            );
        }
        validate_provider_namespace(manifest, &provider.id, "memory provider", &mut errors);
        match (&provider.runtime, &provider.operations) {
            (Some(runtime), Some(operations)) => {
                validate_http_provider_runtime(manifest, &provider.id, runtime, &mut errors);
                validate_operation_path(
                    &provider.id,
                    "health_path",
                    &operations.health_path,
                    &mut errors,
                );
                validate_operation_path(
                    &provider.id,
                    "recall_path",
                    &operations.recall_path,
                    &mut errors,
                );
                validate_operation_path(
                    &provider.id,
                    "store_path",
                    &operations.store_path,
                    &mut errors,
                );
            }
            _ => errors.push(format!(
                "memory provider contribution '{}' uses the unsupported registration-only contract; runtime and operations are required",
                provider.id
            )),
        }
    }

    for provider in &manifest.contributions.context_providers {
        if !valid_manifest_id(&provider.id) || !contribution_ids.insert(provider.id.as_str()) {
            errors.push(
                "context provider contribution id is required, invalid, or duplicated".to_string(),
            );
        }
        validate_provider_namespace(manifest, &provider.id, "context provider", &mut errors);
        match (&provider.runtime, &provider.operations) {
            (Some(runtime), Some(operations)) => {
                validate_http_provider_runtime(manifest, &provider.id, runtime, &mut errors);
                validate_operation_path(
                    &provider.id,
                    "health_path",
                    &operations.health_path,
                    &mut errors,
                );
                validate_operation_path(
                    &provider.id,
                    "resolve_path",
                    &operations.resolve_path,
                    &mut errors,
                );
            }
            _ => errors.push(format!(
                "context provider contribution '{}' uses the unsupported registration-only contract; runtime and operations are required",
                provider.id
            )),
        }
    }

    for provider in &manifest.contributions.auth_providers {
        validate_deferred_contribution_id(
            manifest,
            &provider.id,
            "auth provider",
            &mut contribution_ids,
            &mut errors,
        );
        if !valid_deferred_secret_namespace(manifest, &provider.id, &provider.binding_namespace) {
            errors.push(format!(
                "auth provider contribution '{}' has an invalid secret namespace",
                provider.id
            ));
        }
        if let ContributionSupportDecision::Unsupported { reason } =
            contribution_support(ContributionSurface::AuthProvider)
        {
            errors.push(format!(
                "auth provider contribution '{}' is unsupported: {reason}",
                provider.id
            ));
        }
    }

    for provider in &manifest.contributions.llm_providers {
        validate_deferred_contribution_id(
            manifest,
            &provider.id,
            "LLM provider",
            &mut contribution_ids,
            &mut errors,
        );
        if !valid_deferred_secret_namespace(manifest, &provider.id, &provider.binding_namespace)
            || !(100..=120_000).contains(&provider.timeout_ms)
        {
            errors.push(format!(
                "LLM provider contribution '{}' has invalid secret or timeout boundaries",
                provider.id
            ));
        }
        if let ContributionSupportDecision::Unsupported { reason } =
            contribution_support(ContributionSurface::LlmProvider)
        {
            errors.push(format!(
                "LLM provider contribution '{}' is unsupported: {reason}",
                provider.id
            ));
        }
    }

    for route in &manifest.contributions.http_routes {
        validate_deferred_contribution_id(
            manifest,
            &route.id,
            "HTTP route",
            &mut contribution_ids,
            &mut errors,
        );
        let expected_namespace = format!("/extensions/{}/{}", manifest.id, route.id);
        if route.route_namespace != expected_namespace
            || !(1..=1024 * 1024).contains(&route.max_body_bytes)
            || !(100..=30_000).contains(&route.timeout_ms)
        {
            errors.push(format!(
                "HTTP route contribution '{}' violates namespace, body, or timeout boundaries",
                route.id
            ));
        }
        if let ContributionSupportDecision::Unsupported { reason } =
            contribution_support(ContributionSurface::HttpRoute)
        {
            errors.push(format!(
                "HTTP route contribution '{}' is unsupported: {reason}",
                route.id
            ));
        }
    }

    for native in &manifest.contributions.native_plugins {
        if !valid_manifest_id(&native.id) || !contribution_ids.insert(native.id.as_str()) {
            errors.push("native plugin contribution id is invalid or duplicated".to_string());
        }
        if !valid_manifest_id(&native.artifact) {
            errors.push(format!(
                "native plugin '{}' has an invalid artifact reference",
                native.id
            ));
        }
        if native.abi != NativePluginAbi::CAbiJsonV1 {
            errors.push(format!(
                "native plugin '{}' uses unsupported ABI",
                native.id
            ));
        }
        if native.abi_version != NATIVE_PLUGIN_ABI_VERSION {
            errors.push(format!(
                "native plugin '{}' uses ABI version {}; expected {}",
                native.id, native.abi_version, NATIVE_PLUGIN_ABI_VERSION
            ));
        }
        if native.max_request_bytes == 0 || native.max_response_bytes == 0 {
            errors.push(format!(
                "native plugin '{}' must declare non-zero JSON request/response limits",
                native.id
            ));
        }
        validate_artifact_reference(
            manifest,
            &native.artifact,
            PluginArtifactKind::NativeDylib,
            "native plugin",
            &native.id,
            &mut errors,
        );
    }

    if manifest.contributions.tools.is_empty()
        && manifest.contributions.channels.is_empty()
        && manifest.contributions.memory_providers.is_empty()
        && manifest.contributions.context_providers.is_empty()
        && manifest.contributions.auth_providers.is_empty()
        && manifest.contributions.llm_providers.is_empty()
        && manifest.contributions.http_routes.is_empty()
        && manifest.contributions.native_plugins.is_empty()
    {
        warnings.push("plugin declares no contributions".to_string());
    }

    PluginManifestValidation {
        valid: errors.is_empty(),
        errors,
        warnings,
    }
}

fn validate_deferred_contribution_id<'a>(
    manifest: &PluginManifest,
    id: &'a str,
    kind: &str,
    contribution_ids: &mut HashSet<&'a str>,
    errors: &mut Vec<String>,
) {
    if !valid_manifest_id(id) || !contribution_ids.insert(id) {
        errors.push(format!("{kind} contribution id is invalid or duplicated"));
    }
    validate_provider_namespace(manifest, id, kind, errors);
}

fn validate_provider_namespace(
    manifest: &PluginManifest,
    id: &str,
    kind: &str,
    errors: &mut Vec<String>,
) {
    let prefix = format!("{}.", manifest.id);
    if !id.starts_with(&prefix) {
        errors.push(format!(
            "{kind} contribution '{id}' must be namespaced under '{}.'",
            manifest.id
        ));
    }
}

fn valid_deferred_secret_namespace(
    manifest: &PluginManifest,
    contribution_id: &str,
    namespace: &str,
) -> bool {
    manifest
        .permissions
        .iter()
        .any(|permission| permission == EXTENSION_SECRETS_PERMISSION)
        && namespace == format!("extension.{}.{contribution_id}", manifest.id)
}

fn validate_http_provider_runtime(
    manifest: &PluginManifest,
    contribution_id: &str,
    runtime: &HttpJsonProviderRuntime,
    errors: &mut Vec<String>,
) {
    if !manifest
        .permissions
        .iter()
        .any(|permission| permission == EXTENSION_HTTP_PERMISSION)
    {
        errors.push(format!(
            "provider contribution '{contribution_id}' requires permission '{EXTENSION_HTTP_PERMISSION}'"
        ));
    }
    let options = thinclaw_tools_core::OutboundUrlGuardOptions {
        require_https: true,
        upgrade_http_to_https: false,
        allowlist: Vec::new(),
    };
    match thinclaw_tools_core::validate_outbound_url_structure(&runtime.base_url, &options) {
        Ok(url) if url.query().is_none() => {}
        Ok(_) => errors.push(format!(
            "provider contribution '{contribution_id}' has an unsafe base_url: query parameters are not allowed"
        )),
        Err(error) => errors.push(format!(
            "provider contribution '{contribution_id}' has an unsafe base_url: {error}"
        )),
    }
    if !(100..=30_000).contains(&runtime.timeout_ms)
        || !(1..=256 * 1024).contains(&runtime.max_request_bytes)
        || !(1..=1024 * 1024).contains(&runtime.max_response_bytes)
    {
        errors.push(format!(
            "provider contribution '{contribution_id}' violates request, response, or timeout limits"
        ));
    }
    if let ExtensionHttpAuth::BearerSecret { binding } = &runtime.auth {
        let expected_prefix = format!("extension.{}.{contribution_id}.", manifest.id);
        if !manifest
            .permissions
            .iter()
            .any(|permission| permission == EXTENSION_SECRETS_PERMISSION)
            || !valid_manifest_text(binding, 256, false)
            || !binding.starts_with(&expected_prefix)
        {
            errors.push(format!(
                "provider contribution '{contribution_id}' uses a secret outside its declared namespace or without permission '{EXTENSION_SECRETS_PERMISSION}'"
            ));
        }
    }
}

fn validate_operation_path(
    contribution_id: &str,
    field: &str,
    value: &str,
    errors: &mut Vec<String>,
) {
    if value.is_empty()
        || value.len() > 2_048
        || !value.starts_with('/')
        || value.starts_with("//")
        || value.contains("..")
        || value.contains('?')
        || value.contains('#')
        || value.contains('\0')
    {
        errors.push(format!(
            "provider contribution '{contribution_id}' has an invalid {field}"
        ));
    }
}

fn validate_artifact_reference(
    manifest: &PluginManifest,
    artifact_id: &str,
    expected_kind: PluginArtifactKind,
    contribution_kind: &str,
    contribution_id: &str,
    errors: &mut Vec<String>,
) {
    match manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.id == artifact_id)
    {
        Some(artifact) if artifact.kind == expected_kind => {}
        Some(artifact) => errors.push(format!(
            "{contribution_kind} contribution '{contribution_id}' references artifact '{artifact_id}' with kind {:?}; expected {:?}",
            artifact.kind, expected_kind
        )),
        None => errors.push(format!(
            "{contribution_kind} contribution '{contribution_id}' references missing artifact '{artifact_id}'"
        )),
    }
}

pub fn verify_plugin_manifest_signature(
    manifest: &PluginManifest,
    settings: &ExtensionsSettings,
) -> Result<(), String> {
    let Some(signature) = &manifest.signature else {
        return Err("plugin signature is missing".to_string());
    };
    if settings.trusted_manifest_keys.len() > MAX_TRUSTED_MANIFEST_KEYS
        || settings.trusted_manifest_public_keys.len() > MAX_TRUSTED_MANIFEST_KEYS
    {
        return Err("too many trusted manifest keys are configured".to_string());
    }
    if !valid_manifest_id(&signature.key_id)
        || signature.algorithm.len() > 32
        || signature.signature.len() != 128
        || !signature
            .signature
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("plugin signature metadata is malformed or oversized".to_string());
    }
    if !signature.algorithm.eq_ignore_ascii_case("ed25519") {
        return Err(format!(
            "unsupported plugin signature algorithm '{}'",
            signature.algorithm
        ));
    }
    let Some(public_key_hex) = settings
        .trusted_manifest_public_keys
        .get(&signature.key_id)
        .or_else(|| {
            settings
                .trusted_manifest_keys
                .iter()
                .find(|key| key.as_str() == signature.key_id && parse_hex_public_key(key).is_some())
        })
    else {
        return Err(format!(
            "plugin signature key '{}' has no configured public key",
            signature.key_id
        ));
    };
    let public_key = parse_hex_public_key(public_key_hex).ok_or_else(|| {
        format!(
            "trusted manifest key '{}' is not a 32-byte hex public key",
            signature.key_id
        )
    })?;
    let signature_bytes = parse_hex_signature(&signature.signature)
        .ok_or_else(|| "plugin signature is not a 64-byte hex ed25519 signature".to_string())?;
    let mut signed_manifest = manifest.clone();
    signed_manifest.signature = None;
    let manifest_bytes = serde_json::to_vec(&signed_manifest)
        .map_err(|err| format!("failed to serialize plugin manifest for signature check: {err}"))?;
    if verify_manifest_signature(&public_key, &manifest_bytes, &signature_bytes) {
        Ok(())
    } else {
        Err("plugin manifest signature verification failed".to_string())
    }
}

fn is_valid_semver(version: &str) -> bool {
    let parts = version.split('.').collect::<Vec<_>>();
    parts.len() == 3 && parts.iter().all(|part| part.parse::<u64>().is_ok())
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    fn signed_settings() -> ExtensionsSettings {
        ExtensionsSettings {
            require_plugin_signatures: true,
            trusted_manifest_keys: vec!["test-key".to_string()],
            ..ExtensionsSettings::default()
        }
    }

    fn sample_manifest() -> PluginManifest {
        PluginManifest {
            schema_version: PLUGIN_MANIFEST_SCHEMA_VERSION,
            id: "example".to_string(),
            name: "Example".to_string(),
            version: "1.0.0".to_string(),
            publisher: Some("ThinClaw".to_string()),
            description: Some("Example plugin".to_string()),
            permissions: vec!["tools".to_string()],
            contributions: PluginContributions {
                tools: vec![ToolContribution {
                    id: "example.echo".to_string(),
                    name: "example_echo".to_string(),
                    wasm_artifact: Some("echo-wasm".to_string()),
                }],
                ..PluginContributions::default()
            },
            artifacts: vec![PluginArtifact {
                id: "echo-wasm".to_string(),
                kind: PluginArtifactKind::Wasm,
                path: "echo.wasm".to_string(),
                sha256: Some("00".repeat(32)),
            }],
            signature: Some(PluginSignature {
                key_id: "test-key".to_string(),
                algorithm: "ed25519".to_string(),
                signature: "aa".repeat(64),
            }),
        }
    }

    fn test_http_runtime() -> HttpJsonProviderRuntime {
        HttpJsonProviderRuntime {
            base_url: "https://api.example.com".to_string(),
            auth: ExtensionHttpAuth::None,
            timeout_ms: 1_000,
            max_request_bytes: 16 * 1024,
            max_response_bytes: 32 * 1024,
            lifecycle: ProviderLifecycle::Startup,
        }
    }

    fn sign_manifest(manifest: &mut PluginManifest, key_id: &str, signing_key: &SigningKey) {
        manifest.signature = None;
        let bytes = serde_json::to_vec(manifest).expect("serialize signed payload");
        let signature = signing_key.sign(&bytes);
        manifest.signature = Some(PluginSignature {
            key_id: key_id.to_string(),
            algorithm: "ed25519".to_string(),
            signature: hex::encode(signature.to_bytes()),
        });
    }

    #[test]
    fn broad_plugin_manifest_accepts_executable_contribution_kinds() {
        let mut manifest = sample_manifest();
        manifest
            .permissions
            .push(EXTENSION_HTTP_PERMISSION.to_string());
        manifest.contributions.channels.push(ChannelContribution {
            id: "example.channel".to_string(),
            name: "Example Channel".to_string(),
            wasm_artifact: Some("channel-wasm".to_string()),
        });
        manifest.artifacts.push(PluginArtifact {
            id: "channel-wasm".to_string(),
            kind: PluginArtifactKind::Wasm,
            path: "channel.wasm".to_string(),
            sha256: Some("22".repeat(32)),
        });
        manifest
            .contributions
            .memory_providers
            .push(MemoryProviderContribution {
                id: "example.memory".to_string(),
                provider_type: "custom_http".to_string(),
                config_schema: serde_json::json!({ "type": "object" }),
                runtime: Some(test_http_runtime()),
                operations: Some(MemoryProviderOperations {
                    health_path: "/health".to_string(),
                    recall_path: "/memory/recall".to_string(),
                    store_path: "/memory/store".to_string(),
                }),
            });
        manifest
            .contributions
            .context_providers
            .push(ContextProviderContribution {
                id: "example.context".to_string(),
                provider_type: "workspace".to_string(),
                config_schema: serde_json::json!({ "type": "object" }),
                runtime: Some(test_http_runtime()),
                operations: Some(ContextProviderOperations {
                    health_path: "/health".to_string(),
                    resolve_path: "/context/resolve".to_string(),
                }),
            });

        let validation = validate_plugin_manifest(&manifest, &signed_settings());
        assert!(validation.valid, "{:?}", validation.errors);
        let json = serde_json::to_string(&manifest).expect("serialize");
        let reparsed: PluginManifest = serde_json::from_str(&json).expect("parse");
        assert_eq!(reparsed.contributions.tools.len(), 1);
    }

    #[test]
    fn manifest_validation_rejects_bad_contribution_artifacts() {
        let mut manifest = sample_manifest();
        manifest.contributions.tools[0].wasm_artifact = Some("missing-wasm".to_string());
        let validation = validate_plugin_manifest(&manifest, &signed_settings());
        assert!(!validation.valid);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("missing artifact 'missing-wasm'"))
        );

        let mut manifest = sample_manifest();
        manifest.contributions.channels.push(ChannelContribution {
            id: "example.channel".to_string(),
            name: "Example Channel".to_string(),
            wasm_artifact: Some("native-lib".to_string()),
        });
        manifest.artifacts.push(PluginArtifact {
            id: "native-lib".to_string(),
            kind: PluginArtifactKind::NativeDylib,
            path: "libexample.dylib".to_string(),
            sha256: Some("11".repeat(32)),
        });
        let validation = validate_plugin_manifest(&manifest, &signed_settings());
        assert!(!validation.valid);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("expected Wasm"))
        );
    }

    #[test]
    fn manifest_validation_requires_provider_contribution_ids() {
        let mut manifest = sample_manifest();
        manifest
            .contributions
            .memory_providers
            .push(MemoryProviderContribution {
                id: "".to_string(),
                provider_type: "".to_string(),
                config_schema: serde_json::json!({ "type": "object" }),
                runtime: None,
                operations: None,
            });
        manifest
            .contributions
            .context_providers
            .push(ContextProviderContribution {
                id: "".to_string(),
                provider_type: "".to_string(),
                config_schema: serde_json::json!({ "type": "object" }),
                runtime: None,
                operations: None,
            });

        let validation = validate_plugin_manifest(&manifest, &signed_settings());
        assert!(!validation.valid);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("memory provider contribution id is required"))
        );
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("context provider contribution id is required"))
        );
    }

    #[test]
    fn registration_only_provider_contract_is_rejected() {
        let mut manifest = sample_manifest();
        manifest
            .contributions
            .memory_providers
            .push(MemoryProviderContribution {
                id: "example.memory".to_string(),
                provider_type: "custom_http".to_string(),
                config_schema: serde_json::json!({ "type": "object" }),
                runtime: None,
                operations: None,
            });

        let validation = validate_plugin_manifest(&manifest, &signed_settings());
        assert!(!validation.valid);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| { error.contains("unsupported registration-only contract") })
        );
    }

    #[test]
    fn deferred_generic_surfaces_fail_manifest_validation() {
        let mut manifest = sample_manifest();
        manifest
            .contributions
            .auth_providers
            .push(AuthProviderContribution {
                id: "example.auth".to_string(),
                protocol: AuthProviderProtocol::OAuth2Pkce,
                binding_namespace: "example.auth".to_string(),
            });
        manifest
            .contributions
            .llm_providers
            .push(LlmProviderContribution {
                id: "example.llm".to_string(),
                protocol: LlmProviderProtocol::OpenAiCompatible,
                binding_namespace: "example.llm".to_string(),
                timeout_ms: 5_000,
            });
        manifest
            .contributions
            .http_routes
            .push(HttpRouteContribution {
                id: "example.route".to_string(),
                route_namespace: "/extensions/example/example.route".to_string(),
                auth: HttpRouteAuth::GatewaySession,
                max_body_bytes: 64 * 1024,
                timeout_ms: 5_000,
                lifecycle: HttpRouteLifecycle::AgentRuntime,
            });

        let validation = validate_plugin_manifest(&manifest, &signed_settings());
        assert!(!validation.valid);
        for surface in ["auth provider", "LLM provider", "HTTP route"] {
            assert!(
                validation
                    .errors
                    .iter()
                    .any(|error| { error.contains(surface) && error.contains("unsupported") })
            );
        }
        assert!(matches!(
            contribution_support(ContributionSurface::MemoryProvider),
            ContributionSupportDecision::Executable { .. }
        ));
        assert!(matches!(
            contribution_support(ContributionSurface::HttpRoute),
            ContributionSupportDecision::Unsupported { .. }
        ));
    }

    #[test]
    fn provider_contract_enforces_http_secret_namespace_and_bounds() {
        let mut manifest = sample_manifest();
        manifest
            .permissions
            .push(EXTENSION_HTTP_PERMISSION.to_string());
        manifest
            .permissions
            .push(EXTENSION_SECRETS_PERMISSION.to_string());
        let mut runtime = test_http_runtime();
        runtime.base_url = "http://127.0.0.1/internal".to_string();
        runtime.timeout_ms = 60_000;
        runtime.auth = ExtensionHttpAuth::BearerSecret {
            binding: "other.plugin.token".to_string(),
        };
        manifest
            .contributions
            .context_providers
            .push(ContextProviderContribution {
                id: "example.context".to_string(),
                provider_type: "host_http_json_v1".to_string(),
                config_schema: serde_json::Value::Null,
                runtime: Some(runtime),
                operations: Some(ContextProviderOperations {
                    health_path: "https://other.example/health".to_string(),
                    resolve_path: "/../resolve?token=x".to_string(),
                }),
            });

        let validation = validate_plugin_manifest(&manifest, &signed_settings());
        assert!(!validation.valid);
        for expected in [
            "unsafe base_url",
            "violates request, response, or timeout limits",
            "outside its declared namespace",
            "invalid health_path",
            "invalid resolve_path",
        ] {
            assert!(
                validation
                    .errors
                    .iter()
                    .any(|error| error.contains(expected)),
                "missing validation error containing {expected:?}: {:?}",
                validation.errors
            );
        }
    }

    #[test]
    fn native_plugin_is_disabled_by_default() {
        let mut manifest = sample_manifest();
        manifest
            .contributions
            .native_plugins
            .push(NativePluginContribution {
                id: "example.native".to_string(),
                artifact: "native-lib".to_string(),
                abi: NativePluginAbi::CAbiJsonV1,
                abi_version: NATIVE_PLUGIN_ABI_VERSION,
                max_request_bytes: 1024,
                max_response_bytes: 1024,
            });
        manifest.artifacts.push(PluginArtifact {
            id: "native-lib".to_string(),
            kind: PluginArtifactKind::NativeDylib,
            path: "libexample.dylib".to_string(),
            sha256: Some("11".repeat(32)),
        });

        let validation = validate_plugin_manifest(&manifest, &signed_settings());
        assert!(!validation.valid);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("allow_native_plugins"))
        );
    }

    #[test]
    fn signature_policy_rejects_missing_or_untrusted_keys() {
        let mut manifest = sample_manifest();
        manifest.signature = None;
        let validation = validate_plugin_manifest(&manifest, &signed_settings());
        assert!(!validation.valid);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("signature"))
        );

        let mut manifest = sample_manifest();
        manifest.signature.as_mut().unwrap().key_id = "unknown".to_string();
        let validation = validate_plugin_manifest(&manifest, &signed_settings());
        assert!(!validation.valid);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("not trusted"))
        );
    }

    #[test]
    fn manifest_validation_rejects_duplicates_and_malformed_signatures() {
        let mut manifest = sample_manifest();
        manifest.artifacts.push(manifest.artifacts[0].clone());
        let validation = validate_plugin_manifest(&manifest, &signed_settings());
        assert!(!validation.valid);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("artifact metadata"))
        );

        let mut manifest = sample_manifest();
        manifest.contributions.channels.push(ChannelContribution {
            id: manifest.contributions.tools[0].id.clone(),
            name: "Duplicate".to_string(),
            wasm_artifact: None,
        });
        manifest.signature.as_mut().unwrap().signature = "not-hex".to_string();
        let validation = validate_plugin_manifest(&manifest, &signed_settings());
        assert!(!validation.valid);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("duplicated"))
        );
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("signature metadata"))
        );
    }

    #[test]
    fn manifest_signature_verifies_against_configured_public_key() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let mut manifest = sample_manifest();
        sign_manifest(&mut manifest, "test-key", &signing_key);
        let mut settings = signed_settings();
        settings.trusted_manifest_public_keys.insert(
            "test-key".to_string(),
            hex::encode(signing_key.verifying_key().to_bytes()),
        );

        verify_plugin_manifest_signature(&manifest, &settings).expect("signature should verify");

        manifest.name = "Tampered".to_string();
        assert!(verify_plugin_manifest_signature(&manifest, &settings).is_err());
    }
}
