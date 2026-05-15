use serde::{Deserialize, Serialize};

use crate::provider::ProviderEndpoint;

/// Runtime family requesting secret access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum SecretConsumer {
    DirectWorkbench,
    ThinClawAgent,
    GatewayProxy,
    Extension,
    System,
}

/// How a secret may be used by a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum SecretAccessMode {
    Status,
    ExplicitUse,
    RuntimeInjection,
}

/// Canonical secret identity plus compatibility aliases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SecretDescriptor {
    pub canonical_name: String,
    #[serde(default)]
    pub provider_slug: Option<String>,
    #[serde(default)]
    pub env_key_name: Option<String>,
    #[serde(default)]
    pub legacy_aliases: Vec<String>,
    #[serde(default)]
    pub allowed_consumers: Vec<SecretConsumer>,
}

/// Provider credential metadata safe to expose to clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProviderCredentialDescriptor {
    pub provider_slug: String,
    pub display_name: String,
    pub secret_name: String,
    pub env_key_name: String,
    #[serde(default)]
    pub setup_url: Option<String>,
    #[serde(default)]
    pub credential_ready: bool,
}

impl SecretDescriptor {
    pub fn new(
        canonical_name: impl Into<String>,
        provider_slug: Option<impl Into<String>>,
        env_key_name: Option<impl Into<String>>,
        legacy_aliases: Vec<String>,
        allowed_consumers: Vec<SecretConsumer>,
    ) -> Self {
        Self {
            canonical_name: canonical_name.into(),
            provider_slug: provider_slug.map(Into::into),
            env_key_name: env_key_name.map(Into::into),
            legacy_aliases,
            allowed_consumers,
        }
    }

    pub fn provider(endpoint: &ProviderEndpoint) -> Self {
        let mut aliases = vec![endpoint.slug.clone(), endpoint.env_key_name.clone()];
        aliases.extend(
            built_in_legacy_aliases(&endpoint.secret_name)
                .iter()
                .map(|alias| (*alias).to_string()),
        );
        aliases.sort();
        aliases.dedup();
        aliases.retain(|alias| alias != &endpoint.secret_name);

        Self::new(
            endpoint.secret_name.clone(),
            Some(endpoint.slug.clone()),
            Some(endpoint.env_key_name.clone()),
            aliases,
            vec![
                SecretConsumer::DirectWorkbench,
                SecretConsumer::ThinClawAgent,
            ],
        )
    }
}

impl ProviderCredentialDescriptor {
    pub fn from_provider_endpoint(endpoint: &ProviderEndpoint, credential_ready: bool) -> Self {
        Self {
            provider_slug: endpoint.slug.clone(),
            display_name: endpoint.display_name.clone(),
            secret_name: endpoint.secret_name.clone(),
            env_key_name: endpoint.env_key_name.clone(),
            setup_url: endpoint.setup_url.clone(),
            credential_ready,
        }
    }
}

pub fn provider_secret_descriptors<'a>(
    endpoints: impl IntoIterator<Item = &'a ProviderEndpoint>,
) -> Vec<SecretDescriptor> {
    endpoints
        .into_iter()
        .map(SecretDescriptor::provider)
        .collect()
}

pub fn provider_credential_descriptors<'a>(
    endpoints: impl IntoIterator<Item = &'a ProviderEndpoint>,
) -> Vec<ProviderCredentialDescriptor> {
    endpoints
        .into_iter()
        .map(|endpoint| ProviderCredentialDescriptor::from_provider_endpoint(endpoint, false))
        .collect()
}

pub fn platform_secret_descriptors() -> Vec<SecretDescriptor> {
    vec![
        descriptor(
            "search_brave_api_key",
            Some("brave"),
            Some("BRAVE_SEARCH_API_KEY"),
            &["brave"],
            &[
                SecretConsumer::DirectWorkbench,
                SecretConsumer::ThinClawAgent,
            ],
        ),
        descriptor(
            "hf_token",
            Some("huggingface"),
            Some("HF_TOKEN"),
            &["huggingface", "HUGGINGFACE_TOKEN"],
            &[
                SecretConsumer::DirectWorkbench,
                SecretConsumer::ThinClawAgent,
            ],
        ),
        descriptor(
            "bedrock_api_key",
            Some("bedrock"),
            Some("BEDROCK_API_KEY"),
            &["llm_bedrock_api_key"],
            &[
                SecretConsumer::DirectWorkbench,
                SecretConsumer::ThinClawAgent,
            ],
        ),
        descriptor(
            "bedrock_proxy_api_key",
            Some("bedrock_proxy"),
            Some("BEDROCK_PROXY_API_KEY"),
            &["llm_bedrock_proxy_api_key"],
            &[
                SecretConsumer::DirectWorkbench,
                SecretConsumer::ThinClawAgent,
            ],
        ),
        descriptor(
            "bedrock_access_key_id",
            Some("amazon-bedrock"),
            Some("AWS_ACCESS_KEY_ID"),
            &["amazon-bedrock", "bedrock"],
            &[
                SecretConsumer::DirectWorkbench,
                SecretConsumer::ThinClawAgent,
            ],
        ),
        descriptor(
            "bedrock_secret_access_key",
            Some("amazon-bedrock"),
            Some("AWS_SECRET_ACCESS_KEY"),
            &[],
            &[
                SecretConsumer::DirectWorkbench,
                SecretConsumer::ThinClawAgent,
            ],
        ),
        descriptor(
            "bedrock_region",
            Some("amazon-bedrock"),
            Some("AWS_REGION"),
            &["AWS_DEFAULT_REGION"],
            &[
                SecretConsumer::DirectWorkbench,
                SecretConsumer::ThinClawAgent,
            ],
        ),
        descriptor(
            "custom_llm_key",
            Some("custom_llm"),
            Some("THINCLAW_CUSTOM_LLM_KEY"),
            &[],
            &[
                SecretConsumer::DirectWorkbench,
                SecretConsumer::ThinClawAgent,
            ],
        ),
        descriptor(
            "remote_token",
            Some("remote_gateway"),
            Some("THINCLAW_REMOTE_TOKEN"),
            &[],
            &[SecretConsumer::GatewayProxy],
        ),
        descriptor(
            "voyage",
            Some("voyage"),
            Some("VOYAGE_API_KEY"),
            &[],
            &[
                SecretConsumer::DirectWorkbench,
                SecretConsumer::ThinClawAgent,
            ],
        ),
        descriptor(
            "deepgram",
            Some("deepgram"),
            Some("DEEPGRAM_API_KEY"),
            &[],
            &[
                SecretConsumer::DirectWorkbench,
                SecretConsumer::ThinClawAgent,
            ],
        ),
        descriptor(
            "elevenlabs",
            Some("elevenlabs"),
            Some("ELEVENLABS_API_KEY"),
            &[],
            &[
                SecretConsumer::DirectWorkbench,
                SecretConsumer::ThinClawAgent,
            ],
        ),
        descriptor(
            "stability",
            Some("stability"),
            Some("STABILITY_API_KEY"),
            &[],
            &[
                SecretConsumer::DirectWorkbench,
                SecretConsumer::ThinClawAgent,
            ],
        ),
        descriptor(
            "fal",
            Some("fal"),
            Some("FAL_KEY"),
            &["FAL_API_KEY"],
            &[
                SecretConsumer::DirectWorkbench,
                SecretConsumer::ThinClawAgent,
            ],
        ),
    ]
}

pub fn descriptor_for_secret_name(name: &str) -> Option<SecretDescriptor> {
    let canonical = canonical_secret_name(name);
    platform_secret_descriptors()
        .into_iter()
        .find(|descriptor| descriptor_matches(descriptor, canonical, name))
        .or_else(|| provider_descriptor_for_known_name(canonical, name))
}

fn descriptor(
    canonical_name: &str,
    provider_slug: Option<&str>,
    env_key_name: Option<&str>,
    aliases: &[&str],
    allowed_consumers: &[SecretConsumer],
) -> SecretDescriptor {
    let mut legacy_aliases = aliases
        .iter()
        .map(|alias| (*alias).to_string())
        .collect::<Vec<_>>();
    if let Some(provider_slug) = provider_slug {
        legacy_aliases.push(provider_slug.to_string());
    }
    if let Some(env_key_name) = env_key_name {
        legacy_aliases.push(env_key_name.to_string());
    }
    legacy_aliases.sort();
    legacy_aliases.dedup();
    legacy_aliases.retain(|alias| alias != canonical_name);

    SecretDescriptor::new(
        canonical_name.to_string(),
        provider_slug.map(str::to_string),
        env_key_name.map(str::to_string),
        legacy_aliases,
        allowed_consumers.to_vec(),
    )
}

fn descriptor_matches(descriptor: &SecretDescriptor, canonical: &str, name: &str) -> bool {
    descriptor.canonical_name == canonical
        || descriptor.canonical_name == name
        || descriptor.provider_slug.as_deref() == Some(name)
        || descriptor.env_key_name.as_deref() == Some(name)
        || descriptor.legacy_aliases.iter().any(|alias| alias == name)
}

fn provider_descriptor_for_known_name(canonical: &str, name: &str) -> Option<SecretDescriptor> {
    let (provider_slug, env_key_name) = match canonical {
        "llm_anthropic_api_key" => ("anthropic", "ANTHROPIC_API_KEY"),
        "llm_openai_api_key" => ("openai", "OPENAI_API_KEY"),
        "llm_compatible_api_key" => ("openrouter", "OPENROUTER_API_KEY"),
        "gemini" => ("gemini", "GEMINI_API_KEY"),
        "groq" => ("groq", "GROQ_API_KEY"),
        "mistral" => ("mistral", "MISTRAL_API_KEY"),
        "xai" => ("xai", "XAI_API_KEY"),
        "together" => ("together", "TOGETHER_API_KEY"),
        "venice" => ("venice", "VENICE_API_KEY"),
        "moonshot" => ("moonshot", "MOONSHOT_API_KEY"),
        "minimax" => ("minimax", "MINIMAX_API_KEY"),
        "nvidia" => ("nvidia", "NVIDIA_API_KEY"),
        "deepseek" => ("deepseek", "DEEPSEEK_API_KEY"),
        "cerebras" => ("cerebras", "CEREBRAS_API_KEY"),
        "cohere" => ("cohere", "COHERE_API_KEY"),
        "llm_tinfoil_api_key" => ("tinfoil", "TINFOIL_API_KEY"),
        _ => return None,
    };

    let mut aliases = vec![provider_slug.to_string(), env_key_name.to_string()];
    aliases.extend(
        built_in_legacy_aliases(canonical)
            .iter()
            .map(|alias| (*alias).to_string()),
    );
    aliases.push(name.to_string());
    aliases.sort();
    aliases.dedup();
    aliases.retain(|alias| alias != canonical);

    Some(SecretDescriptor::new(
        canonical.to_string(),
        Some(provider_slug.to_string()),
        Some(env_key_name.to_string()),
        aliases,
        vec![
            SecretConsumer::DirectWorkbench,
            SecretConsumer::ThinClawAgent,
        ],
    ))
}

/// Return the canonical secret name for a provider or legacy secret alias.
pub fn canonical_secret_name(name: &str) -> &str {
    match name {
        "anthropic" | "ANTHROPIC_API_KEY" | "llm_anthropic_api_key" => "llm_anthropic_api_key",
        "openai" | "OPENAI_API_KEY" | "llm_openai_api_key" => "llm_openai_api_key",
        "openrouter" | "openai_compatible" | "LLM_API_KEY" | "llm_compatible_api_key" => {
            "llm_compatible_api_key"
        }
        "brave" | "BRAVE_SEARCH_API_KEY" | "search_brave_api_key" => "search_brave_api_key",
        "huggingface" | "HUGGINGFACE_TOKEN" | "HF_TOKEN" | "hf_token" => "hf_token",
        "gemini" | "GEMINI_API_KEY" => "gemini",
        "groq" | "GROQ_API_KEY" => "groq",
        "mistral" | "MISTRAL_API_KEY" => "mistral",
        "xai" | "XAI_API_KEY" => "xai",
        "together" | "TOGETHER_API_KEY" => "together",
        "venice" | "VENICE_API_KEY" => "venice",
        "moonshot" | "MOONSHOT_API_KEY" => "moonshot",
        "minimax" | "MINIMAX_API_KEY" => "minimax",
        "nvidia" | "NVIDIA_API_KEY" => "nvidia",
        "deepseek" | "DEEPSEEK_API_KEY" => "deepseek",
        "cerebras" | "CEREBRAS_API_KEY" => "cerebras",
        "cohere" | "COHERE_API_KEY" => "cohere",
        "tinfoil" | "TINFOIL_API_KEY" | "llm_tinfoil_api_key" => "llm_tinfoil_api_key",
        "voyage" | "VOYAGE_API_KEY" => "voyage",
        "deepgram" | "DEEPGRAM_API_KEY" => "deepgram",
        "elevenlabs" | "ELEVENLABS_API_KEY" => "elevenlabs",
        "stability" | "STABILITY_API_KEY" => "stability",
        "fal" | "FAL_KEY" | "FAL_API_KEY" => "fal",
        "llm_bedrock_api_key" | "BEDROCK_API_KEY" => "bedrock_api_key",
        "llm_bedrock_proxy_api_key" | "BEDROCK_PROXY_API_KEY" => "bedrock_proxy_api_key",
        "amazon-bedrock" | "bedrock" | "AWS_ACCESS_KEY_ID" => "bedrock_access_key_id",
        "AWS_SECRET_ACCESS_KEY" => "bedrock_secret_access_key",
        "AWS_REGION" | "AWS_DEFAULT_REGION" => "bedrock_region",
        "THINCLAW_CUSTOM_LLM_KEY" => "custom_llm_key",
        "THINCLAW_REMOTE_TOKEN" => "remote_token",
        // Stale Desktop-only provider: read legacy keychain entries, but do not
        // emit it as a registry-backed provider credential.
        "xiaomi" | "XIAOMI_API_KEY" => "xiaomi",
        other => other,
    }
}

/// Compatibility aliases that should be read during migration and lookup.
pub fn legacy_secret_aliases(canonical_name: &str) -> &'static [&'static str] {
    built_in_legacy_aliases(canonical_secret_name(canonical_name))
}

fn built_in_legacy_aliases(canonical_name: &str) -> &'static [&'static str] {
    match canonical_name {
        "llm_anthropic_api_key" => &["anthropic", "ANTHROPIC_API_KEY"],
        "llm_openai_api_key" => &["openai", "OPENAI_API_KEY"],
        "llm_compatible_api_key" => &[
            "openrouter",
            "openai_compatible",
            "LLM_API_KEY",
            "OPENROUTER_API_KEY",
        ],
        "search_brave_api_key" => &["brave", "BRAVE_SEARCH_API_KEY"],
        "hf_token" => &["huggingface", "HUGGINGFACE_TOKEN", "HF_TOKEN"],
        "llm_tinfoil_api_key" => &["tinfoil", "TINFOIL_API_KEY"],
        "bedrock_api_key" => &["llm_bedrock_api_key", "BEDROCK_API_KEY"],
        "bedrock_proxy_api_key" => &["llm_bedrock_proxy_api_key", "BEDROCK_PROXY_API_KEY"],
        "bedrock_access_key_id" => &["amazon-bedrock", "bedrock", "AWS_ACCESS_KEY_ID"],
        "bedrock_secret_access_key" => &["AWS_SECRET_ACCESS_KEY"],
        "bedrock_region" => &["AWS_REGION", "AWS_DEFAULT_REGION"],
        "fal" => &["FAL_KEY", "FAL_API_KEY"],
        "xiaomi" => &["XIAOMI_API_KEY"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_legacy_provider_names() {
        assert_eq!(canonical_secret_name("anthropic"), "llm_anthropic_api_key");
        assert_eq!(
            canonical_secret_name("LLM_API_KEY"),
            "llm_compatible_api_key"
        );
        assert_eq!(canonical_secret_name("HF_TOKEN"), "hf_token");
        assert_eq!(canonical_secret_name("cohere"), "cohere");
    }
}
