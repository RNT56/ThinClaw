//! ThinClaw - Main entry point.

mod main_helpers;

use std::path::PathBuf;
use std::sync::Arc;

use thinclaw::{
    app::{RuntimeCommandIntent, RuntimeEntryMode, RuntimeEnvBootstrapPlan, run_async_entrypoint},
    channels::{
        ChannelDescriptor, ChannelManager, NativeEndpointRegistry, NativeHttpClient,
        NativeLifecycleChannel, NativeLifecycleChannelConfig, NativeLifecycleWebhookConfig,
        ReqwestNativeHttpClient, native_lifecycle_webhook_routes,
    },
    cli::Command,
    config::Config,
};

use thinclaw::channels::{
    ApnsNativeClient, ApnsNativeConfig, BrowserPushNativeClient, BrowserPushNativeConfig,
    MatrixNativeClient, MatrixNativeConfig, VoiceCallNativeClient, VoiceCallNativeConfig,
};

#[cfg(any(feature = "postgres", feature = "libsql"))]
use thinclaw::setup::{SetupConfig, UiMode};

use main_helpers::*;

fn main() -> anyhow::Result<()> {
    run_async_entrypoint(async_main())
}

fn runtime_command_intent(command: Option<&Command>) -> RuntimeCommandIntent {
    match command {
        None | Some(Command::Run) => RuntimeCommandIntent::AgentRuntime,
        Some(Command::Tui) => RuntimeCommandIntent::TuiRuntime,
        Some(Command::Onboard { .. }) => RuntimeCommandIntent::Onboarding,
        #[cfg(feature = "docker-sandbox")]
        Some(Command::Worker { .. })
        | Some(Command::ClaudeBridge { .. })
        | Some(Command::CodexBridge { .. }) => RuntimeCommandIntent::WorkerRuntime,
        #[cfg(all(feature = "repl", target_os = "windows"))]
        Some(Command::WindowsServiceRuntime { .. }) => RuntimeCommandIntent::ServiceRuntime,
        _ => RuntimeCommandIntent::ImmediateCli,
    }
}

fn execute_env_bootstrap_plan(plan: RuntimeEnvBootstrapPlan) {
    if plan.load_dotenv {
        let _ = dotenvy::dotenv();
    }
    if plan.load_thinclaw_env {
        thinclaw::bootstrap::load_thinclaw_env();
    }
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn runtime_entry_mode_from_ui_mode(ui_mode: UiMode) -> RuntimeEntryMode {
    match ui_mode {
        UiMode::Tui => RuntimeEntryMode::Tui,
        UiMode::Cli | UiMode::Auto => RuntimeEntryMode::Cli,
    }
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn setup_config_for_onboard_command(
    skip_auth: bool,
    channels_only: bool,
    guide_topic: Option<thinclaw::setup::GuideTopic>,
    ui_mode: UiMode,
    profile: Option<thinclaw::setup::OnboardingProfile>,
) -> SetupConfig {
    SetupConfig {
        skip_auth,
        channels_only,
        guide_topic,
        ui_mode,
        profile,
        pause_after_completion: false,
    }
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn setup_config_for_startup_onboarding(runtime_entry_mode: RuntimeEntryMode) -> SetupConfig {
    let ui_mode = match runtime_entry_mode {
        RuntimeEntryMode::Tui => UiMode::Tui,
        RuntimeEntryMode::Cli => UiMode::Cli,
        RuntimeEntryMode::Default => UiMode::Auto,
    };

    SetupConfig {
        ui_mode,
        ..SetupConfig::default()
    }
}

fn native_lifecycle_channel_descriptors(config: &Config) -> Vec<ChannelDescriptor> {
    thinclaw::channels::native_lifecycle_channel_descriptors(&NativeLifecycleChannelConfig {
        matrix_enabled: config.channels.matrix_enabled,
        voice_call_enabled: config.channels.voice_call_enabled,
        voice_call_available: config.channels.voice_call_available,
        apns_enabled: config.channels.apns_enabled,
        browser_push_enabled: config.channels.browser_push_enabled,
        browser_push_available: config.channels.browser_push_available,
    })
}

async fn register_native_lifecycle_channels(
    config: &Config,
    channels: Arc<ChannelManager>,
    channel_names: &mut Vec<String>,
) -> Vec<axum::Router> {
    let http: Arc<dyn NativeHttpClient> = Arc::new(ReqwestNativeHttpClient::new());
    let mut webhook_config = NativeLifecycleWebhookConfig::default();

    if config.channels.matrix_enabled {
        match matrix_native_config_from_env() {
            Ok(Some(matrix_config)) => {
                let client = Arc::new(MatrixNativeClient::new(matrix_config, Arc::clone(&http)));
                let channel = NativeLifecycleChannel::matrix(client);
                webhook_config.matrix = Some(channel.ingress());
                webhook_config.matrix_secret = env_value("MATRIX_WEBHOOK_SECRET");
                channels.add(Box::new(channel)).await;
                channel_names.push("matrix".to_string());
                tracing::info!("Matrix native lifecycle channel enabled");
            }
            Ok(None) => {
                tracing::warn!(
                    "Matrix native lifecycle is enabled but MATRIX_HOMESERVER or MATRIX_ACCESS_TOKEN is missing"
                );
            }
            Err(error) => {
                tracing::warn!(error = %error, "Matrix native lifecycle configuration is invalid")
            }
        }
    }

    if config.channels.voice_call_enabled {
        if !config.channels.voice_call_available {
            tracing::warn!(
                "Voice-call native lifecycle is enabled but the binary was built without the voice feature"
            );
        } else {
            match voice_call_native_config_from_env() {
                Ok(Some(voice_config)) => {
                    webhook_config.voice_call_secret = voice_config.webhook_secret.clone();
                    let client =
                        Arc::new(VoiceCallNativeClient::new(voice_config, Arc::clone(&http)));
                    let channel = NativeLifecycleChannel::voice_call(client);
                    webhook_config.voice_call = Some(channel.ingress());
                    channels.add(Box::new(channel)).await;
                    channel_names.push("voice-call".to_string());
                    tracing::info!("Voice-call native lifecycle channel enabled");
                }
                Ok(None) => {
                    tracing::warn!(
                        "Voice-call native lifecycle is enabled but VOICE_CALL_RESPONSE_URL is missing"
                    );
                }
                Err(error) => {
                    tracing::warn!(error = %error, "Voice-call native lifecycle configuration is invalid")
                }
            }
        }
    }

    if config.channels.apns_enabled {
        match apns_native_config_from_env() {
            Ok(Some(apns_config)) => {
                if let Some(registration_secret) = env_value("APNS_REGISTRATION_SECRET") {
                    match native_endpoint_registry_from_env("apns", "APNS_ENDPOINT_REGISTRY_PATH")
                        .await
                    {
                        Ok(registry) => {
                            webhook_config.apns_registry = Some(registry.clone());
                            webhook_config.apns_registration_secret = Some(registration_secret);
                            let client = Arc::new(ApnsNativeClient::with_registry(
                                apns_config,
                                Arc::clone(&http),
                                registry,
                            ));
                            channels
                                .add(Box::new(NativeLifecycleChannel::apns(client)))
                                .await;
                            channel_names.push("apns".to_string());
                            tracing::info!("APNs native lifecycle channel enabled");
                        }
                        Err(error) => {
                            tracing::warn!(error = %error, "APNs native lifecycle endpoint registry is invalid")
                        }
                    }
                } else {
                    tracing::warn!(
                        "APNs native lifecycle is enabled but APNS_REGISTRATION_SECRET is missing; refusing to expose an unusable registration endpoint"
                    );
                }
            }
            Ok(None) => {
                tracing::warn!(
                    "APNs native lifecycle is enabled but APNS_TEAM_ID, APNS_KEY_ID, APNS_BUNDLE_ID, or APNS_PRIVATE_KEY is missing"
                );
            }
            Err(error) => {
                tracing::warn!(error = %error, "APNs native lifecycle configuration is invalid")
            }
        }
    }

    if config.channels.browser_push_enabled {
        if !config.channels.browser_push_available {
            tracing::warn!(
                "Browser-push native lifecycle is enabled but the binary was built without the browser feature"
            );
        } else {
            match browser_push_native_config_from_env() {
                Ok(Some(push_config)) => {
                    if let Some(webhook_secret) = env_value("BROWSER_PUSH_WEBHOOK_SECRET") {
                        match native_endpoint_registry_from_env(
                            "browser-push",
                            "BROWSER_PUSH_ENDPOINT_REGISTRY_PATH",
                        )
                        .await
                        {
                            Ok(registry) => {
                                let client = Arc::new(BrowserPushNativeClient::with_registry(
                                    push_config,
                                    Arc::clone(&http),
                                    registry.clone(),
                                ));
                                let channel = NativeLifecycleChannel::browser_push(client);
                                webhook_config.browser_push = Some(channel.ingress());
                                webhook_config.browser_push_registry = Some(registry);
                                webhook_config.browser_push_secret = Some(webhook_secret);
                                channels.add(Box::new(channel)).await;
                                channel_names.push("browser-push".to_string());
                                tracing::info!("Browser-push native lifecycle channel enabled");
                            }
                            Err(error) => {
                                tracing::warn!(error = %error, "Browser-push native lifecycle endpoint registry is invalid")
                            }
                        };
                    } else {
                        tracing::warn!(
                            "Browser-push native lifecycle is enabled but BROWSER_PUSH_WEBHOOK_SECRET is missing; refusing to expose unauthenticated ingress"
                        );
                    }
                }
                Ok(None) => {
                    tracing::warn!(
                        "Browser-push native lifecycle is enabled but BROWSER_PUSH_VAPID_PUBLIC_KEY, BROWSER_PUSH_VAPID_PRIVATE_KEY, or BROWSER_PUSH_VAPID_SUBJECT is missing"
                    );
                }
                Err(error) => {
                    tracing::warn!(error = %error, "Browser-push native lifecycle configuration is invalid")
                }
            }
        }
    }

    if webhook_config.matrix.is_some()
        || webhook_config.voice_call.is_some()
        || webhook_config.browser_push.is_some()
        || webhook_config.apns_registry.is_some()
        || webhook_config.browser_push_registry.is_some()
    {
        vec![native_lifecycle_webhook_routes(webhook_config)]
    } else {
        Vec::new()
    }
}

fn matrix_native_config_from_env() -> Result<Option<MatrixNativeConfig>, String> {
    let Some(homeserver) = env_value("MATRIX_HOMESERVER") else {
        return Ok(None);
    };
    let Some(access_token) = env_value("MATRIX_ACCESS_TOKEN") else {
        return Ok(None);
    };
    if env_value("MATRIX_WEBHOOK_SECRET").is_none() {
        return Err(
            "MATRIX_WEBHOOK_SECRET is required to authenticate Matrix webhook ingress".to_string(),
        );
    }
    Ok(Some(MatrixNativeConfig {
        homeserver,
        access_token,
    }))
}

fn voice_call_native_config_from_env() -> Result<Option<VoiceCallNativeConfig>, String> {
    let Some(response_url) = env_value("VOICE_CALL_RESPONSE_URL") else {
        return Ok(None);
    };
    let webhook_secret = env_value("VOICE_CALL_WEBHOOK_SECRET").ok_or_else(|| {
        "VOICE_CALL_WEBHOOK_SECRET is required to authenticate voice-call ingress".to_string()
    })?;
    Ok(Some(VoiceCallNativeConfig {
        response_url,
        webhook_secret: Some(webhook_secret),
    }))
}

/// APNs provider config from the environment. Delegates to the shared helper
/// in `thinclaw::channels::first_party_push` so the native APNs lifecycle
/// channel here and the first-party push notifier read one identical config.
fn apns_native_config_from_env() -> Result<Option<ApnsNativeConfig>, String> {
    thinclaw::channels::first_party_push::apns_native_config_from_env()
}

fn browser_push_native_config_from_env() -> Result<Option<BrowserPushNativeConfig>, String> {
    let Some(vapid_public_key) = env_value("BROWSER_PUSH_VAPID_PUBLIC_KEY") else {
        return Ok(None);
    };
    let Some(vapid_private_key_pem) = env_value_or_file(
        "BROWSER_PUSH_VAPID_PRIVATE_KEY",
        "BROWSER_PUSH_VAPID_PRIVATE_KEY_PATH",
    )?
    else {
        return Ok(None);
    };
    let Some(subject) = env_value("BROWSER_PUSH_VAPID_SUBJECT") else {
        return Ok(None);
    };
    let ttl_seconds = match env_value("BROWSER_PUSH_TTL_SECONDS") {
        Some(value) => value.parse::<u32>().map_err(|error| {
            format!("BROWSER_PUSH_TTL_SECONDS must be a positive integer: {error}")
        })?,
        None => 60,
    };
    Ok(Some(BrowserPushNativeConfig {
        vapid_public_key,
        vapid_private_key_pem,
        subject,
        ttl_seconds,
    }))
}

async fn native_endpoint_registry_from_env(
    provider: &str,
    path_env: &str,
) -> Result<NativeEndpointRegistry, String> {
    let path = env_value(path_env).map(PathBuf::from).unwrap_or_else(|| {
        thinclaw_platform::resolve_thinclaw_home()
            .join("native-endpoints")
            .join(format!("{provider}.json"))
    });
    NativeEndpointRegistry::persistent(path)
        .await
        .map_err(|error| error.to_string())
}

fn env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_value_or_file(value_key: &str, path_key: &str) -> Result<Option<String>, String> {
    if let Some(value) = env_value(value_key) {
        return Ok(Some(value.replace("\\n", "\n")));
    }
    let Some(path) = env_value(path_key) else {
        return Ok(None);
    };
    std::fs::read_to_string(&path)
        .map(|value| Some(value.replace("\\n", "\n")))
        .map_err(|error| format!("failed to read {path_key}={path}: {error}"))
}

mod async_main;
use async_main::async_main;

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
