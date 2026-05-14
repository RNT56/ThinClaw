//! Compatibility facade for WASM channel capabilities and root tool adapters.

pub use thinclaw_channels::wasm::capabilities::*;

use std::sync::Arc;

pub(crate) fn to_root_tool_capabilities(
    capabilities: &ToolCapabilities,
    workspace_reader: Option<Arc<dyn crate::tools::wasm::WorkspaceReader>>,
) -> crate::tools::wasm::Capabilities {
    let mut root = crate::tools::wasm::Capabilities::default();

    if let Some(workspace) = &capabilities.workspace_read {
        root.workspace_read = Some(crate::tools::wasm::WorkspaceCapability {
            allowed_prefixes: workspace.allowed_prefixes.clone(),
            reader: workspace_reader,
        });
    }

    if let Some(http) = &capabilities.http {
        root.http = Some(crate::tools::wasm::HttpCapability {
            allowlist: http
                .allowlist
                .iter()
                .map(|pattern| crate::tools::wasm::EndpointPattern {
                    host: pattern.host.clone(),
                    path_prefix: pattern.path_prefix.clone(),
                    methods: pattern.methods.clone(),
                })
                .collect(),
            credentials: http
                .credentials
                .iter()
                .map(|(name, mapping)| {
                    (
                        name.clone(),
                        crate::secrets::CredentialMapping {
                            secret_name: mapping.secret_name.clone(),
                            location: credential_location_to_root(&mapping.location),
                            host_patterns: mapping.host_patterns.clone(),
                        },
                    )
                })
                .collect(),
            rate_limit: crate::tools::wasm::RateLimitConfig {
                requests_per_minute: http.rate_limit.requests_per_minute,
                requests_per_hour: http.rate_limit.requests_per_hour,
            },
            max_request_bytes: http.max_request_bytes,
            max_response_bytes: http.max_response_bytes,
            timeout: http.timeout,
        });
    }

    if let Some(tool_invoke) = &capabilities.tool_invoke {
        root.tool_invoke = Some(crate::tools::wasm::ToolInvokeCapability {
            aliases: tool_invoke.aliases.clone(),
            rate_limit: crate::tools::wasm::RateLimitConfig {
                requests_per_minute: tool_invoke.rate_limit.requests_per_minute,
                requests_per_hour: tool_invoke.rate_limit.requests_per_hour,
            },
        });
    }

    if let Some(secrets) = &capabilities.secrets {
        root.secrets = Some(crate::tools::wasm::SecretsCapability {
            allowed_names: secrets.allowed_names.clone(),
        });
    }

    root
}

impl From<ToolCapabilities> for crate::tools::wasm::Capabilities {
    fn from(capabilities: ToolCapabilities) -> Self {
        to_root_tool_capabilities(&capabilities, None)
    }
}

fn credential_location_to_root(
    location: &CredentialLocation,
) -> crate::secrets::CredentialLocation {
    match location {
        CredentialLocation::Bearer => crate::secrets::CredentialLocation::AuthorizationBearer,
        CredentialLocation::Basic { username } => {
            crate::secrets::CredentialLocation::AuthorizationBasic {
                username: username.clone(),
            }
        }
        CredentialLocation::Header { name, prefix } => crate::secrets::CredentialLocation::Header {
            name: name.clone(),
            prefix: prefix.clone(),
        },
        CredentialLocation::QueryParam { name } => {
            crate::secrets::CredentialLocation::QueryParam { name: name.clone() }
        }
        CredentialLocation::UrlPath { placeholder } => {
            crate::secrets::CredentialLocation::UrlPath {
                placeholder: placeholder.clone(),
            }
        }
    }
}
