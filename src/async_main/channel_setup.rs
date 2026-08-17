//! Native, WASM, and local channel construction for runtime startup.

use std::path::PathBuf;
use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};
use thinclaw::app::{
    LocalRuntimeChannel, NativeChannelActivationInput, NativeChannelActivationPlan,
};
#[cfg(target_os = "macos")]
use thinclaw::channels::IMessageChannel;
use thinclaw::channels::{
    BlueBubblesChannel, BlueBubblesConfig, ChannelManager, DiscordChannel, DiscordConfig,
    GmailChannel, HttpChannel, LinqChannel, LinqConfig, LinqPreferredService, ReplChannel,
    SignalChannel, WebhookServer, WebhookServerConfig,
    canvas_gateway::CanvasStore,
    wasm::{WasmChannelLoader, WasmChannelRouter, WasmChannelRuntime},
};
use thinclaw::cli::ResolvedRuntimeArgs;
use thinclaw::config::Config;
use thinclaw::extensions::ExtensionManager;
use thinclaw::pairing::PairingStore;
use thinclaw::secrets::SecretsStore;
use thinclaw::tools::ToolRegistry;

use crate::{
    native_lifecycle_channel_descriptors, register_native_lifecycle_channels, setup_wasm_channels,
};

pub(super) type WasmChannelRuntimeState = (
    Arc<WasmChannelRuntime>,
    Arc<PairingStore>,
    Arc<WasmChannelRouter>,
    Arc<WasmChannelLoader>,
    PathBuf,
);

async fn resolve_linq_secret(
    configured: Option<SecretString>,
    secrets_store: &Option<Arc<dyn SecretsStore + Send + Sync>>,
    name: &str,
    purpose: &str,
    target: Option<(&str, &str)>,
) -> anyhow::Result<SecretString> {
    if let Some(value) = configured {
        let trimmed = value.expose_secret().trim();
        if !trimmed.is_empty() {
            return Ok(SecretString::from(trimmed.to_string()));
        }
    }
    let store = secrets_store.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "Linq is enabled but secret '{name}' is unavailable; configure the encrypted secret store or its environment fallback"
        )
    })?;
    let mut context = thinclaw::secrets::SecretAccessContext::new("channel.linq", purpose)
        .auth_source("encrypted_secret_store");
    if let Some((host, path)) = target {
        context = context.target(host, path);
    }
    let secret = store
        .get_for_injection("default", name, context)
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Linq is enabled but required secret '{name}' is unavailable; store it with `thinclaw secrets set {name} --from-stdin --provider linq`"
            )
        })?;
    let trimmed = secret.expose().trim();
    if trimmed.is_empty() {
        anyhow::bail!("Linq secret '{name}' is empty");
    }
    Ok(SecretString::from(trimmed.to_string()))
}

pub(super) struct ChannelSetup {
    pub channels: Arc<ChannelManager>,
    pub channel_plan: NativeChannelActivationPlan,
    pub channel_names: Vec<String>,
    pub loaded_wasm_channel_names: Vec<String>,
    pub wasm_channel_runtime_state: Option<WasmChannelRuntimeState>,
    pub webhook_server: Option<Arc<tokio::sync::Mutex<WebhookServer>>>,
    pub gateway_webhook_routes: Vec<axum::Router>,
    pub canvas_store: CanvasStore,
}

pub(super) async fn setup_channels(
    config: &Config,
    runtime_args: &ResolvedRuntimeArgs,
    local_channel: Option<LocalRuntimeChannel>,
    tools: &Arc<ToolRegistry>,
    secrets_store: &Option<Arc<dyn SecretsStore + Send + Sync>>,
    extension_manager: Option<&Arc<ExtensionManager>>,
) -> anyhow::Result<ChannelSetup> {
    // ── Channel setup ──────────────────────────────────────────────────

    let channels = Arc::new(ChannelManager::new());
    let mut channel_names: Vec<String> = Vec::new();
    for descriptor in native_lifecycle_channel_descriptors(config) {
        channels.add_descriptor(descriptor).await;
    }
    let channel_plan = NativeChannelActivationPlan::from_input(NativeChannelActivationInput {
        cli_only: runtime_args.channels.disables_external_ingress(),
        signal_configured: config.channels.signal.is_some()
            && runtime_args.channels.allows("signal"),
        nostr_configured: {
            #[cfg(feature = "nostr")]
            {
                config.channels.nostr.is_some() && runtime_args.channels.allows("nostr")
            }
            #[cfg(not(feature = "nostr"))]
            {
                false
            }
        },
        discord_configured: config.channels.discord.is_some()
            && runtime_args.channels.allows("discord"),
        imessage_configured: {
            #[cfg(target_os = "macos")]
            {
                config.channels.imessage.is_some() && runtime_args.channels.allows("imessage")
            }
            #[cfg(not(target_os = "macos"))]
            {
                false
            }
        },
        apple_mail_configured: {
            #[cfg(target_os = "macos")]
            {
                config.channels.apple_mail.is_some() && runtime_args.channels.allows("apple-mail")
            }
            #[cfg(not(target_os = "macos"))]
            {
                false
            }
        },
        bluebubbles_configured: config.channels.bluebubbles.is_some()
            && runtime_args.channels.allows("bluebubbles"),
        linq_configured: config.channels.linq.is_some() && runtime_args.channels.allows("linq"),
        gmail_configured: config.channels.gmail.is_some() && runtime_args.channels.allows("gmail"),
        http_configured: config.channels.http.is_some() && runtime_args.channels.allows("http"),
        gateway_configured: config.channels.gateway.is_some()
            && runtime_args.channels.allows("gateway"),
        wasm_channels_enabled: config.channels.wasm_channels_enabled,
        wasm_channels_dir_exists: config.channels.wasm_channels_dir.exists(),
    });
    #[cfg(feature = "nostr")]
    let mut nostr_channel: Option<thinclaw::channels::NostrChannel> = None;
    #[cfg(feature = "nostr")]
    let mut nostr_runtime = None;

    #[cfg(feature = "nostr")]
    if let Some(ref nostr_config) = config.channels.nostr {
        let channel_config = thinclaw::channels::NostrConfig {
            private_key: nostr_config.private_key.clone(),
            relays: nostr_config.relays.clone(),
            owner_pubkey: nostr_config.owner_pubkey.clone(),
            social_dm_enabled: nostr_config.social_dm_enabled,
            allow_from: nostr_config.allow_from.clone(),
        };
        match thinclaw::channels::NostrChannel::new(channel_config) {
            Ok(channel) => {
                nostr_runtime = Some(channel.runtime());
                nostr_channel = Some(channel);
            }
            Err(error) => {
                tracing::error!(error = %error, "Failed to initialize Nostr runtime");
            }
        }
    }
    let mut loaded_wasm_channel_names: Vec<String> = Vec::new();
    #[allow(clippy::type_complexity)]
    let mut wasm_channel_runtime_state: Option<(
        Arc<WasmChannelRuntime>,
        Arc<PairingStore>,
        Arc<WasmChannelRouter>,
        Arc<thinclaw::channels::wasm::WasmChannelLoader>,
        std::path::PathBuf,
    )> = None;

    match local_channel {
        Some(LocalRuntimeChannel::SingleMessage) => {
            if let Some(ref msg) = runtime_args.one_shot_message {
                channels
                    .add(Box::new(ReplChannel::with_message(msg.clone())))
                    .await;
                tracing::info!("Single message mode");
            }
        }
        Some(LocalRuntimeChannel::Tui) => {
            channel_names.push("tui".to_string());
            tracing::info!("Full-screen TUI mode selected; startup waits for the sealed registry");
        }
        Some(LocalRuntimeChannel::Repl) => {
            let repl = ReplChannel::new().with_tool_registry(Arc::clone(tools));
            repl.suppress_banner();
            channels.add(Box::new(repl)).await;
            channel_names.push("repl".to_string());
            tracing::info!("REPL mode enabled");
        }
        None => {}
    }

    // Collect webhook route fragments; a single WebhookServer hosts them all.
    let mut webhook_routes: Vec<axum::Router> = Vec::new();
    let mut gateway_webhook_routes: Vec<axum::Router> = Vec::new();
    let mut webhook_server_addr: Option<std::net::SocketAddr> = None;
    let mut canvas_http_auth_token: Option<String> = None;
    if !runtime_args.channels.disables_external_ingress() {
        webhook_routes.extend(
            register_native_lifecycle_channels(
                &config,
                &runtime_args.channels,
                Arc::clone(&channels),
                &mut channel_names,
            )
            .await,
        );
    }

    // Load WASM channels and register their webhook routes.
    if channel_plan.wasm_channels {
        let wasm_result = setup_wasm_channels(&config, secrets_store, extension_manager).await;

        if let Some(result) = wasm_result {
            loaded_wasm_channel_names = result.channel_names;
            wasm_channel_runtime_state = Some((
                result.wasm_channel_runtime,
                result.pairing_store,
                result.wasm_channel_router,
                result.wasm_channel_loader,
                result.channels_dir,
            ));
            for (name, channel) in result.channels {
                channel_names.push(name);
                channels.add(channel).await;
            }
            if let Some(routes) = result.webhook_routes {
                webhook_routes.push(routes);
            }
        }
    }

    // Add Signal channel if configured and not CLI-only mode.
    if channel_plan.signal
        && let Some(ref signal_config) = config.channels.signal
    {
        let channel_config = thinclaw::channels::SignalConfig {
            http_url: signal_config.http_url.clone(),
            account: signal_config.account.clone(),
            allow_from: signal_config.allow_from.clone(),
            allow_from_groups: signal_config.allow_from_groups.clone(),
            dm_policy: signal_config.dm_policy.clone(),
            group_policy: signal_config.group_policy.clone(),
            group_allow_from: signal_config.group_allow_from.clone(),
            ignore_attachments: signal_config.ignore_attachments,
            ignore_stories: signal_config.ignore_stories,
        };
        let signal_channel = SignalChannel::new_pinned(channel_config).await?;
        channel_names.push("signal".to_string());
        channels.add(Box::new(signal_channel)).await;
        let safe_url = SignalChannel::redact_url(&signal_config.http_url);
        tracing::info!(
            url = %safe_url,
            "Signal channel enabled"
        );
        if signal_config.allow_from.is_empty() {
            tracing::warn!(
                "Signal channel has empty allow_from list - ALL messages will be DENIED."
            );
        }
    }

    // Add Nostr channel if configured and not CLI-only mode.
    #[cfg(feature = "nostr")]
    if channel_plan.nostr
        && let Some(nostr_channel) = nostr_channel.take()
        && let Some(ref nostr_config) = config.channels.nostr
    {
        channel_names.push("nostr".to_string());
        channels.add(Box::new(nostr_channel)).await;
        tracing::info!(
            relays = nostr_config.relays.len(),
            owner_pubkey = ?nostr_config.owner_pubkey,
            control_ready = nostr_config.owner_pubkey.is_some(),
            social_dm_enabled = nostr_config.social_dm_enabled,
            "Nostr channel enabled"
        );
        if nostr_config.owner_pubkey.is_none() {
            tracing::warn!(
                "Nostr channel has no owner pubkey configured — inbound commands are denied until NOSTR_OWNER_PUBKEY is set"
            );
        }
    }

    // Add Discord channel if configured and not CLI-only mode.
    if channel_plan.discord
        && let Some(ref discord_config) = config.channels.discord
    {
        let channel_config = DiscordConfig {
            bot_token: discord_config.bot_token.clone(),
            guild_id: discord_config.guild_id.clone(),
            allow_from: discord_config.allow_from.clone(),
            stream_mode: discord_config.stream_mode,
        };
        match DiscordChannel::new(channel_config) {
            Ok(discord_channel) => {
                channel_names.push("discord".to_string());
                channels.add(Box::new(discord_channel)).await;
                tracing::info!(
                    guild_id = discord_config.guild_id.as_deref().unwrap_or("all"),
                    "Discord channel enabled (Gateway WS)"
                );
                if discord_config.allow_from.is_empty() {
                    tracing::info!(
                        "Discord channel allow_from is empty — accepting messages from all channels."
                    );
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize Discord channel");
            }
        }
    }

    // Add iMessage channel if configured (macOS only) and not CLI-only mode.
    #[cfg(target_os = "macos")]
    if channel_plan.imessage
        && let Some(ref imessage_config) = config.channels.imessage
    {
        use thinclaw::channels::IMessageConfig;

        // Auto-start Messages.app if not running
        thinclaw::channels::ensure_app_running("Messages").await;

        let channel_config = IMessageConfig {
            allow_from: imessage_config.allow_from.clone(),
            poll_interval_secs: imessage_config.poll_interval_secs,
            ..IMessageConfig::default()
        };
        match IMessageChannel::new(channel_config) {
            Ok(imessage_channel) => {
                channel_names.push("imessage".to_string());
                channels.add(Box::new(imessage_channel)).await;
                tracing::info!("iMessage channel enabled (chat.db polling)");
                if imessage_config.allow_from.is_empty() {
                    tracing::warn!(
                        "iMessage channel has empty allow_from list — ALL messages will be accepted."
                    );
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize iMessage channel");
            }
        }
    }

    // Add Apple Mail channel if configured (macOS only) and not CLI-only mode.
    #[cfg(target_os = "macos")]
    if channel_plan.apple_mail
        && let Some(ref mail_config) = config.channels.apple_mail
    {
        use thinclaw::channels::{AppleMailChannel, AppleMailConfig};

        // Auto-start Mail.app if not running
        thinclaw::channels::ensure_app_running("Mail").await;

        let channel_config = AppleMailConfig {
            allow_from: mail_config.allow_from.clone(),
            poll_interval_secs: mail_config.poll_interval_secs,
            unread_only: mail_config.unread_only,
            mark_as_read: mail_config.mark_as_read,
            ..AppleMailConfig::default()
        };
        match AppleMailChannel::new(channel_config) {
            Ok(mail_channel) => {
                channel_names.push("apple_mail".to_string());
                channels.add(Box::new(mail_channel)).await;
                tracing::info!("Apple Mail channel enabled (Envelope Index polling)");
                if mail_config.allow_from.is_empty() {
                    tracing::warn!(
                        "Apple Mail channel has empty allow_from list — ALL emails will be accepted."
                    );
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize Apple Mail channel");
            }
        }
    }

    // Add BlueBubbles iMessage bridge if configured and not CLI-only mode.
    // Cross-platform — works on any OS with a BlueBubbles server on a Mac.
    if channel_plan.bluebubbles
        && let Some(ref bb_config) = config.channels.bluebubbles
    {
        let channel_config = BlueBubblesConfig::new(
            bb_config.server_url.clone(),
            bb_config.password.clone(),
            bb_config.webhook_host.clone(),
            bb_config.webhook_port,
            bb_config.webhook_path.clone(),
            bb_config.allow_from.clone(),
            bb_config.send_read_receipts,
        );
        match BlueBubblesChannel::init(channel_config).await {
            Ok(bb_channel) => {
                channel_names.push("bluebubbles".to_string());
                channels.add(Box::new(bb_channel)).await;
                tracing::info!("BlueBubbles iMessage channel enabled (webhook mode)");
                if bb_config.allow_from.is_empty() {
                    tracing::warn!(
                        "BlueBubbles channel has empty allow_from list — ALL messages will be accepted."
                    );
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize BlueBubbles channel");
            }
        }
    }

    // Add the managed Linq Partner API v3 channel. Its signed webhook route is
    // mounted both on the shared local listener and on the authenticated
    // gateway/tunnel listener. Signature verification remains mandatory on
    // both paths, and an empty sender allowlist denies all inbound messages.
    if channel_plan.linq
        && let Some(ref linq_config) = config.channels.linq
    {
        let api_base_url = url::Url::parse(&linq_config.api_base_url)
            .map_err(|error| anyhow::anyhow!("configured Linq API endpoint is invalid: {error}"))?;
        let api_host = api_base_url
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("configured Linq API endpoint has no host"))?
            .to_string();
        let api_key = resolve_linq_secret(
            linq_config.api_key.clone(),
            secrets_store,
            thinclaw::channels::LINQ_API_KEY_SECRET,
            "partner_api_authentication",
            Some((&api_host, api_base_url.path())),
        )
        .await?;
        let webhook_secret = resolve_linq_secret(
            linq_config.webhook_secret.clone(),
            secrets_store,
            thinclaw::channels::LINQ_WEBHOOK_SECRET,
            "webhook_signature_verification",
            None,
        )
        .await?;
        let preferred_service = LinqPreferredService::parse(&linq_config.preferred_service)
            .map_err(|message| anyhow::anyhow!("LINQ_PREFERRED_SERVICE {message}"))?;
        let channel_config = LinqConfig::new(
            api_base_url,
            api_key,
            webhook_secret,
            linq_config.from_number.clone(),
            linq_config.webhook_host.clone(),
            linq_config.webhook_port,
            linq_config.webhook_path.clone(),
            linq_config.allow_from.clone(),
            preferred_service,
            thinclaw::platform::state_paths()
                .home
                .join("linq-channel-state.json"),
        )?;
        let linq_channel = LinqChannel::new(channel_config)?;
        let linq_addr = linq_channel.webhook_addr()?;
        if let Some(existing_addr) = webhook_server_addr
            && existing_addr != linq_addr
        {
            anyhow::bail!(
                "Linq webhook address {linq_addr} conflicts with shared webhook address {existing_addr}"
            );
        }
        webhook_server_addr = Some(linq_addr);
        webhook_routes.push(linq_channel.webhook_routes());
        gateway_webhook_routes.push(linq_channel.webhook_routes());
        channel_names.push("linq".to_string());
        channels.add(Box::new(linq_channel)).await;
        tracing::info!(
            webhook = %format_args!("{}{}?version={}", linq_addr, linq_config.webhook_path, thinclaw::channels::LINQ_WEBHOOK_VERSION),
            preferred_service = linq_config.preferred_service,
            "Linq Partner API v3 channel enabled"
        );
        if linq_config.allow_from.is_empty() {
            tracing::warn!(
                "Linq allow_from is empty — all inbound messages are denied until LINQ_ALLOW_FROM is configured"
            );
        }
    }

    // Add Gmail channel if configured and not CLI-only mode.
    if channel_plan.gmail
        && let Some(ref gmail_config) = config.channels.gmail
    {
        use thinclaw::channels::gmail_wiring::GmailConfig;

        let gmail_wiring_config = GmailConfig {
            enabled: true,
            project_id: gmail_config.project_id.clone(),
            subscription_id: gmail_config.subscription_id.clone(),
            topic_id: gmail_config.topic_id.clone(),
            oauth_token: gmail_config.oauth_token.clone(),
            refresh_token: gmail_config.refresh_token.clone(),
            client_id: gmail_config.client_id.clone(),
            client_secret: gmail_config.client_secret.clone(),
            allowed_senders: gmail_config.allowed_senders.clone(),
            label_filters: gmail_config.label_filters.clone(),
            max_message_size_bytes: gmail_config.max_message_size_bytes,
            ..GmailConfig::default()
        };

        match GmailChannel::new(gmail_wiring_config) {
            Ok(gmail_channel) => {
                channel_names.push("gmail".to_string());
                channels.add(Box::new(gmail_channel)).await;
                tracing::info!(
                    project = %gmail_config.project_id,
                    subscription = %gmail_config.subscription_id,
                    "Gmail channel enabled (Pub/Sub pull)"
                );
                if gmail_config.allowed_senders.is_empty() {
                    tracing::warn!(
                        "Gmail channel has empty allowed_senders list — ALL incoming emails will be processed."
                    );
                }
                if gmail_config.oauth_token.is_none() && gmail_config.refresh_token.is_none() {
                    tracing::warn!(
                        "Gmail channel has no OAuth token. Authenticate via the ThinClaw Desktop \
                         Gmail setup, or set GMAIL_OAUTH_TOKEN (and GMAIL_REFRESH_TOKEN / \
                         GMAIL_CLIENT_ID / GMAIL_CLIENT_SECRET for unattended auto-refresh)."
                    );
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize Gmail channel");
            }
        }
    }

    // Add HTTP channel if configured and not CLI-only mode.
    if channel_plan.http
        && let Some(ref http_config) = config.channels.http
    {
        let http_channel = HttpChannel::new(http_config.clone());
        webhook_routes.push(http_channel.routes());
        let (host, port) = http_channel.addr();
        let http_addr: std::net::SocketAddr =
            format!("{}:{}", host, port).parse().map_err(|e| {
                anyhow::anyhow!(
                    "HTTP channel bind address '{host}:{port}' is not a valid SocketAddr: {e}"
                )
            })?;
        if let Some(existing_addr) = webhook_server_addr
            && existing_addr != http_addr
        {
            anyhow::bail!(
                "HTTP webhook address {http_addr} conflicts with shared webhook address {existing_addr}"
            );
        }
        webhook_server_addr = Some(http_addr);
        channel_names.push("http".to_string());
        channels.add(Box::new(http_channel)).await;
        canvas_http_auth_token = http_config
            .webhook_secret
            .as_ref()
            .map(|secret| secret.expose_secret().to_string())
            .filter(|secret| !secret.is_empty());
        tracing::info!(
            "HTTP channel enabled on {}:{}",
            http_config.host,
            http_config.port
        );
    }

    // Create the shared canvas store. HTTP access is mounted only when the
    // explicitly enabled HTTP channel has a non-empty authentication secret;
    // Canvas must never open an otherwise-unused port or expose panels without
    // authentication.
    let canvas_store = thinclaw::channels::canvas_gateway::CanvasStore::default();
    canvas_store
        .set_submission_sender(channels.inject_sender())
        .await;
    if let Some(auth_token) = canvas_http_auth_token {
        webhook_routes.push(thinclaw::channels::canvas_gateway::canvas_routes(
            canvas_store.clone(),
            auth_token,
        ));
    } else if channel_plan.http {
        tracing::warn!("Canvas HTTP routes disabled because HTTP_WEBHOOK_SECRET is not configured");
    }

    // Start the unified webhook server if any routes were registered.
    let webhook_server: Option<Arc<tokio::sync::Mutex<WebhookServer>>> = if !webhook_routes
        .is_empty()
    {
        let addr = webhook_server_addr
            .unwrap_or_else(|| std::net::SocketAddr::from(([127, 0, 0, 1], 8080)));
        if addr.ip().is_unspecified() {
            tracing::warn!(
                "Webhook server is binding to {} — it will be reachable from all network interfaces. \
                     Set HTTP_HOST=127.0.0.1 to restrict to localhost.",
                addr.ip()
            );
        }
        let mut server = WebhookServer::new(WebhookServerConfig { addr });
        for routes in webhook_routes {
            server.add_routes(routes);
        }
        server.start().await?;
        Some(Arc::new(tokio::sync::Mutex::new(server)))
    } else {
        None
    };

    // Register lifecycle hooks.
    let send_message_channels = Arc::clone(&channels);
    let email_channel = {
        #[cfg(target_os = "macos")]
        {
            if config.channels.apple_mail.is_some() {
                Some("apple_mail".to_string())
            } else if config.channels.gmail.is_some() {
                Some("gmail".to_string())
            } else {
                None
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if config.channels.gmail.is_some() {
                Some("gmail".to_string())
            } else {
                None
            }
        }
    };
    tools.register_send_message_tool(Some(Arc::new(
        move |platform, recipient, text, thread_id, attachments| {
            let channels = Arc::clone(&send_message_channels);
            let email_channel = email_channel.clone();
            Box::pin(async move {
                let channel_name = match platform.as_str() {
                    "email" => email_channel
                        .as_deref()
                        .ok_or_else(|| "No email channel is configured.".to_string())?,
                    other => other,
                };

                channels
                    .broadcast(
                        channel_name,
                        &recipient,
                        thinclaw::channels::OutgoingResponse {
                            delivery_id: uuid::Uuid::new_v4(),
                            content: text,
                            thread_id,
                            metadata: serde_json::Value::Null,
                            attachments,
                        },
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                Ok(uuid::Uuid::new_v4().to_string())
            })
        },
    )));

    #[cfg(feature = "nostr")]
    if let Some(runtime) = nostr_runtime {
        tools.register_sync(Arc::new(thinclaw::tools::builtin::NostrActionsTool::new(
            runtime,
        )));
        tracing::info!("Registered nostr_actions tool");
    }

    // NOTE: bootstrap_hooks() is already called inside AppBuilder::build_all()
    // (app.rs). Do NOT call it again here — that would double-register bundled
    // hooks and emit a spurious "Replacing existing hook" WARN.

    Ok(ChannelSetup {
        channels,
        channel_plan,
        channel_names,
        loaded_wasm_channel_names,
        wasm_channel_runtime_state,
        webhook_server,
        gateway_webhook_routes,
        canvas_store,
    })
}
