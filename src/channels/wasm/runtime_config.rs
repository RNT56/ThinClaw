use std::ops::Deref;
use std::sync::Arc;

use super::wrapper::WasmChannel;

pub use thinclaw_channels::wasm::inject_channel_credentials_from_secrets;

type CoreWasmChannelHostConfig = thinclaw_channels::wasm::WasmChannelHostConfig;

/// Compatibility wrapper for root-owned config conversion.
///
/// `thinclaw-channels` owns the runtime host-config DTO and update policy.
/// The root package only knows how to translate its resolved `Config` into
/// those root-independent inputs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WasmChannelHostConfig {
    core: CoreWasmChannelHostConfig,
}

impl WasmChannelHostConfig {
    pub fn from_core(core: CoreWasmChannelHostConfig) -> Self {
        Self { core }
    }

    pub fn as_core(&self) -> CoreWasmChannelHostConfig {
        self.core.clone()
    }

    pub fn from_config(config: &crate::config::Config) -> Self {
        Self::from_core(CoreWasmChannelHostConfig::from_runtime_inputs(
            config.tunnel.public_url.clone(),
            config.channels.telegram_owner_id,
            config.channels.telegram_stream_mode.clone(),
            config.channels.telegram_transport_mode.clone(),
            config.channels.discord_stream_mode.clone(),
            tailscale_serve_tailnet_only(config.tunnel.provider.as_ref()),
        ))
    }

    pub fn updates_for_channel(
        &self,
        channel_name: &str,
        webhook_secret: Option<&str>,
    ) -> std::collections::HashMap<String, serde_json::Value> {
        self.core.updates_for_channel(channel_name, webhook_secret)
    }
}

impl Deref for WasmChannelHostConfig {
    type Target = CoreWasmChannelHostConfig;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl From<CoreWasmChannelHostConfig> for WasmChannelHostConfig {
    fn from(core: CoreWasmChannelHostConfig) -> Self {
        Self::from_core(core)
    }
}

impl From<WasmChannelHostConfig> for CoreWasmChannelHostConfig {
    fn from(config: WasmChannelHostConfig) -> Self {
        config.core
    }
}

pub async fn apply_channel_host_config(
    channel: &Arc<WasmChannel>,
    channel_name: &str,
    host_config: &WasmChannelHostConfig,
    webhook_secret: Option<&str>,
) -> usize {
    thinclaw_channels::wasm::apply_channel_host_config(
        channel,
        channel_name,
        &host_config.core,
        webhook_secret,
    )
    .await
}

fn tailscale_serve_tailnet_only(
    provider: Option<&crate::config::tunnel::TunnelProviderConfig>,
) -> bool {
    provider
        .filter(|provider| provider.provider.eq_ignore_ascii_case("tailscale"))
        .and_then(|provider| provider.tailscale.as_ref())
        .is_some_and(|tailscale| !tailscale.funnel)
}

#[cfg(test)]
mod tests {
    use super::WasmChannelHostConfig;
    use crate::settings::Settings;

    #[tokio::test]
    async fn from_config_matches_core_runtime_inputs() {
        let mut settings = Settings::default();
        settings.llm_backend = Some("openai_compatible".to_string());
        settings.openai_compatible_base_url = Some("http://localhost:12345/v1".to_string());
        settings.tunnel.public_url = Some("https://agent.ts.net".to_string());
        settings.tunnel.provider = Some("tailscale".to_string());
        settings.tunnel.ts_funnel = false;
        settings.channels.telegram_owner_id = Some(684480568);
        settings.channels.telegram_stream_mode = Some("streaming".to_string());
        settings.channels.telegram_transport_mode = "auto".to_string();
        settings.channels.discord_stream_mode = Some("chunks".to_string());

        let config = crate::config::Config::from_test_settings(&settings)
            .await
            .expect("test config resolves");

        let root = WasmChannelHostConfig::from_config(&config).as_core();
        let core = thinclaw_channels::wasm::WasmChannelHostConfig::from_runtime_inputs(
            Some("https://agent.ts.net".to_string()),
            Some(684480568),
            Some("streaming".to_string()),
            "auto".to_string(),
            Some("chunks".to_string()),
            true,
        );

        assert_eq!(root, core);
        assert!(!root.telegram_host_webhook_capable);
        assert!(
            root.telegram_host_transport_reason
                .as_deref()
                .unwrap_or_default()
                .contains("tailnet-only")
        );
    }
}
