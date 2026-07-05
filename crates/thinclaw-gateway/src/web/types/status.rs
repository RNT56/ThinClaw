//! Gateway control-plane status, cost, cache, and channel-setup DTOs.

use serde::Serialize;

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ModelUsageEntry {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GatewayStatusResponse {
    pub sse_connections: u64,
    pub ws_connections: u64,
    pub total_connections: u64,
    pub uptime_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_cost: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions_this_hour: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_usage: Option<Vec<ModelUsageEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_limit_usd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hourly_action_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_cheap_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_reload_error: Option<String>,
    pub channel_setup: ChannelSetupStatus,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct CacheStatsResponse {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub size_bytes: usize,
    pub size: usize,
    pub hit_rate: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChannelSetupStatus {
    pub slack: PartialChannelSetupStatus,
    pub telegram: PartialChannelSetupStatus,
    pub gmail: PartialChannelSetupStatus,
    pub apple_mail: PartialChannelSetupStatus,
    pub nostr: PartialChannelSetupStatus,
    pub matrix: PartialChannelSetupStatus,
    pub voice_call: PartialChannelSetupStatus,
    pub apns: PartialChannelSetupStatus,
    pub browser_push: PartialChannelSetupStatus,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PartialChannelSetupStatus {
    pub enabled: bool,
    pub configured: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub needs_oauth: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub needs_private_key: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub owner_configured: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub tool_ready: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub control_ready: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub social_dm_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected_relay_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_health: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key_npub: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_pubkey_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_pubkey_npub: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub invalid_private_key: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}
