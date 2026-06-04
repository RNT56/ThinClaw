use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
#[cfg(feature = "nostr")]
use nostr_sdk::{ToBech32, prelude::Keys};

use crate::channels::web::identity_helpers::GatewayRequestIdentity;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use thinclaw_gateway::web::status::{
    GatewayRuntimeStatusInput, GatewayStatusResponseInput, GmailSetupStatusInput,
    NativeLifecycleSetupStatusInput, NostrSetupStatusInput, SetupFieldStatus,
    build_cache_stats_response, build_gmail_setup_status as gateway_build_gmail_setup_status,
    build_native_lifecycle_setup_status as gateway_build_native_lifecycle_setup_status,
    build_nostr_setup_status as gateway_build_nostr_setup_status, cost_tracker_unavailable_status,
    format_budget_limit_cents, format_daily_cost, gateway_restart_accepted_response,
    gateway_restart_already_in_progress_response, gateway_status_response, health_response,
    model_usage_entry, unavailable_cache_stats_response,
};

pub(crate) async fn health_handler() -> Json<HealthResponse> {
    Json(health_response())
}

pub(crate) async fn gateway_restart_handler(
    State(state): State<Arc<GatewayState>>,
) -> Json<ActionResponse> {
    if state
        .restart_requested
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_err()
    {
        return Json(gateway_restart_already_in_progress_response());
    }

    if let Some(tx) = state.shutdown_tx.write().await.take() {
        let _ = tx.send(());
        tracing::info!("Gateway restart requested via API");
    }

    Json(gateway_restart_accepted_response())
}

pub(crate) async fn gateway_status_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Json<GatewayStatusResponse> {
    let sse_connections = state.sse.connection_count();
    let ws_connections = state
        .ws_tracker
        .as_ref()
        .map(|t| t.connection_count())
        .unwrap_or(0);

    let uptime_secs = state.startup_time.elapsed().as_secs();
    let runtime_status = state.llm_runtime.as_ref().map(|runtime| runtime.status());
    let channel_setup =
        load_channel_setup_status(state.as_ref(), &request_identity.principal_id).await;

    let (daily_cost, actions_this_hour, model_usage, budget_limit_usd, hourly_action_limit) =
        if let Some(ref cg) = state.cost_guard {
            let cost = cg.daily_spend().await;
            let actions = cg.actions_this_hour().await;
            let usage = cg.model_usage().await;
            let models: Vec<ModelUsageEntry> = usage
                .into_iter()
                .map(|(model, tokens)| {
                    model_usage_entry(
                        model,
                        tokens.input_tokens,
                        tokens.output_tokens,
                        tokens.cost,
                    )
                })
                .collect();
            let budget = cg.daily_budget_cents().map(format_budget_limit_cents);
            let rate_limit = cg.hourly_action_limit();
            (
                Some(format_daily_cost(cost)),
                Some(actions),
                Some(models),
                budget,
                rate_limit,
            )
        } else {
            (None, None, None, None, None)
        };

    let runtime_status = runtime_status.map(|status| GatewayRuntimeStatusInput {
        revision: status.revision,
        primary_model: status.primary_model,
        cheap_model: status.cheap_model,
        routing_enabled: status.routing_enabled,
        routing_mode: status.routing_mode.as_str().to_string(),
        primary_provider: status.primary_provider,
        last_error: status.last_error,
    });

    Json(gateway_status_response(GatewayStatusResponseInput {
        sse_connections,
        ws_connections,
        uptime_secs,
        daily_cost,
        actions_this_hour,
        model_usage,
        budget_limit_usd,
        hourly_action_limit,
        runtime_status,
        channel_setup,
    }))
}

pub(crate) async fn cache_stats_handler(
    State(state): State<Arc<GatewayState>>,
) -> Json<CacheStatsResponse> {
    if let Some(cache) = state.response_cache.as_ref() {
        let stats = cache.read().await.stats();
        return Json(build_cache_stats_response(
            stats.hits,
            stats.misses,
            stats.evictions,
            stats.size,
            stats.hit_rate.into(),
        ));
    }

    Json(unavailable_cache_stats_response(
        "unavailable: response cache is not attached to this gateway",
    ))
}

async fn load_channel_setup_status(state: &GatewayState, user_id: &str) -> ChannelSetupStatus {
    let settings = if let Some(store) = state.store.as_ref()
        && let Ok(map) = store.get_all_settings(user_id).await
    {
        crate::settings::Settings::from_db_map(&map)
    } else {
        crate::settings::Settings::default()
    };

    let nostr_diagnostics = if let Some(channel_manager) = state.channel_manager.as_ref() {
        channel_manager.channel_diagnostics("nostr").await
    } else {
        None
    };

    ChannelSetupStatus {
        slack: build_native_lifecycle_setup_status(
            "SLACK_ENABLED",
            settings.channels.slack_enabled,
            true,
            &[
                ("bot_token", &["SLACK_BOT_TOKEN"][..]),
                ("app_token", &["SLACK_APP_TOKEN"][..]),
            ],
        ),
        telegram: build_native_lifecycle_setup_status(
            "TELEGRAM_ENABLED",
            settings.channels.telegram_owner_id.is_some(),
            true,
            &[("bot_token", &["TELEGRAM_BOT_TOKEN"][..])],
        ),
        gmail: build_gmail_setup_status(&settings),
        apple_mail: build_native_lifecycle_setup_status(
            "APPLE_MAIL_ENABLED",
            settings.channels.apple_mail_enabled,
            cfg!(target_os = "macos"),
            &[],
        ),
        nostr: build_nostr_setup_status(&settings, nostr_diagnostics.as_ref()),
        matrix: build_native_lifecycle_setup_status(
            "MATRIX_ENABLED",
            settings.channels.matrix_enabled,
            true,
            &[
                ("homeserver", &["MATRIX_HOMESERVER"][..]),
                ("access_token", &["MATRIX_ACCESS_TOKEN"][..]),
            ],
        ),
        voice_call: build_native_lifecycle_setup_status(
            "VOICE_CALL_ENABLED",
            settings.channels.voice_call_enabled,
            cfg!(feature = "voice"),
            &[
                ("response_url", &["VOICE_CALL_RESPONSE_URL"][..]),
                ("webhook_secret", &["VOICE_CALL_WEBHOOK_SECRET"][..]),
            ],
        ),
        apns: build_native_lifecycle_setup_status(
            "APNS_ENABLED",
            settings.channels.apns_enabled,
            true,
            &[
                ("team_id", &["APNS_TEAM_ID"][..]),
                ("key_id", &["APNS_KEY_ID"][..]),
                ("bundle_id", &["APNS_BUNDLE_ID"][..]),
                (
                    "private_key",
                    &["APNS_PRIVATE_KEY", "APNS_PRIVATE_KEY_PATH"][..],
                ),
                ("registration_secret", &["APNS_REGISTRATION_SECRET"][..]),
            ],
        ),
        browser_push: build_native_lifecycle_setup_status(
            "BROWSER_PUSH_ENABLED",
            settings.channels.browser_push_enabled,
            cfg!(feature = "browser"),
            &[
                ("vapid_public_key", &["BROWSER_PUSH_VAPID_PUBLIC_KEY"][..]),
                (
                    "vapid_private_key",
                    &[
                        "BROWSER_PUSH_VAPID_PRIVATE_KEY",
                        "BROWSER_PUSH_VAPID_PRIVATE_KEY_PATH",
                    ][..],
                ),
                ("vapid_subject", &["BROWSER_PUSH_VAPID_SUBJECT"][..]),
                ("webhook_secret", &["BROWSER_PUSH_WEBHOOK_SECRET"][..]),
            ],
        ),
    }
}

fn build_native_lifecycle_setup_status(
    enabled_env: &str,
    enabled_setting: bool,
    available: bool,
    required_fields: &[(&str, &[&str])],
) -> PartialChannelSetupStatus {
    let enabled = crate::config::helpers::parse_bool_env(enabled_env, enabled_setting)
        .unwrap_or(enabled_setting);
    let required_fields = required_fields.iter().map(|(field, env_vars)| {
        let present = env_vars.iter().any(|env_var| {
            crate::config::helpers::optional_env(env_var)
                .ok()
                .flatten()
                .is_some_and(|value| !value.trim().is_empty())
        });
        (*field, present)
    });

    gateway_build_native_lifecycle_setup_status(NativeLifecycleSetupStatusInput {
        enabled,
        available,
        required_fields: required_fields
            .map(|(field, present)| SetupFieldStatus::new(field, present))
            .collect(),
    })
}

fn build_gmail_setup_status(settings: &crate::settings::Settings) -> PartialChannelSetupStatus {
    let enabled =
        crate::config::helpers::parse_bool_env("GMAIL_ENABLED", settings.channels.gmail_enabled)
            .unwrap_or(settings.channels.gmail_enabled);
    let project_id = crate::config::helpers::optional_env("GMAIL_PROJECT_ID")
        .ok()
        .flatten()
        .or(settings.channels.gmail_project_id.clone());
    let subscription_id = crate::config::helpers::optional_env("GMAIL_SUBSCRIPTION_ID")
        .ok()
        .flatten()
        .or(settings.channels.gmail_subscription_id.clone());
    let topic_id = crate::config::helpers::optional_env("GMAIL_TOPIC_ID")
        .ok()
        .flatten()
        .or(settings.channels.gmail_topic_id.clone());

    let oauth_token = crate::config::helpers::optional_env("GMAIL_OAUTH_TOKEN")
        .ok()
        .flatten();

    gateway_build_gmail_setup_status(GmailSetupStatusInput {
        enabled,
        project_id,
        subscription_id,
        topic_id,
        oauth_token,
    })
}

fn build_nostr_setup_status(
    settings: &crate::settings::Settings,
    diagnostics: Option<&serde_json::Value>,
) -> PartialChannelSetupStatus {
    let enabled =
        crate::config::helpers::parse_bool_env("NOSTR_ENABLED", settings.channels.nostr_enabled)
            .unwrap_or(settings.channels.nostr_enabled);
    let private_key = crate::config::helpers::optional_env("NOSTR_PRIVATE_KEY")
        .ok()
        .flatten()
        .or_else(|| {
            crate::config::helpers::optional_env("NOSTR_SECRET_KEY")
                .ok()
                .flatten()
        });

    #[cfg(feature = "nostr")]
    let resolved = crate::config::ChannelsConfig::resolve_nostr(settings)
        .ok()
        .flatten();
    #[cfg(feature = "nostr")]
    let owner_configured = resolved
        .as_ref()
        .and_then(|config| config.owner_pubkey.as_ref())
        .is_some();
    #[cfg(not(feature = "nostr"))]
    let owner_configured = false;

    #[cfg(feature = "nostr")]
    let social_dm_enabled = resolved
        .as_ref()
        .map(|config| config.social_dm_enabled)
        .unwrap_or(settings.channels.nostr_social_dm_enabled);
    #[cfg(not(feature = "nostr"))]
    let social_dm_enabled = settings.channels.nostr_social_dm_enabled;

    #[cfg(feature = "nostr")]
    let relay_count = resolved
        .as_ref()
        .map(|config| config.relays.len())
        .or_else(|| {
            diagnostics
                .and_then(|value| value.get("relay_count"))
                .and_then(|value| value.as_u64())
                .map(|value| value as usize)
        });
    #[cfg(not(feature = "nostr"))]
    let relay_count = diagnostics
        .and_then(|value| value.get("relay_count"))
        .and_then(|value| value.as_u64())
        .map(|value| value as usize);

    let connected_relay_count = diagnostics
        .and_then(|value| value.get("connected_relay_count"))
        .and_then(|value| value.as_u64())
        .map(|value| value as usize);
    #[cfg_attr(not(feature = "nostr"), allow(unused_mut))]
    let mut public_key_hex = diagnostics
        .and_then(|value| value.get("public_key_hex"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    #[cfg_attr(not(feature = "nostr"), allow(unused_mut))]
    let mut public_key_npub = diagnostics
        .and_then(|value| value.get("public_key_npub"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    #[cfg_attr(not(feature = "nostr"), allow(unused_mut))]
    let mut owner_pubkey_hex = diagnostics
        .and_then(|value| value.get("owner_pubkey_hex"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    #[cfg_attr(not(feature = "nostr"), allow(unused_mut))]
    let mut owner_pubkey_npub = diagnostics
        .and_then(|value| value.get("owner_pubkey_npub"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    #[cfg_attr(not(feature = "nostr"), allow(unused_mut))]
    let mut invalid_private_key = false;

    if (public_key_hex.is_none() || public_key_npub.is_none())
        && let Some(secret) = private_key.as_deref()
    {
        #[cfg(feature = "nostr")]
        match Keys::parse(secret) {
            Ok(keys) => {
                let parsed_public_key_hex = keys.public_key().to_hex();
                let parsed_public_key_npub = keys
                    .public_key()
                    .to_bech32()
                    .unwrap_or_else(|_| parsed_public_key_hex.clone());
                public_key_hex = Some(parsed_public_key_hex);
                public_key_npub = Some(parsed_public_key_npub);
            }
            Err(_) => {
                invalid_private_key = true;
            }
        }
        #[cfg(not(feature = "nostr"))]
        {
            let _ = secret;
        }
    }

    #[cfg(feature = "nostr")]
    if (owner_pubkey_hex.is_none() || owner_pubkey_npub.is_none())
        && let Some(owner) = resolved
            .as_ref()
            .and_then(|config| config.owner_pubkey.as_ref())
    {
        owner_pubkey_hex = Some(owner.clone());
        if let Ok(parsed) = crate::channels::nostr_runtime::parse_public_key(owner) {
            owner_pubkey_npub = parsed.to_bech32().ok();
        }
    }

    gateway_build_nostr_setup_status(NostrSetupStatusInput {
        enabled,
        private_key_present: private_key.is_some(),
        owner_configured,
        social_dm_enabled,
        relay_count,
        connected_relay_count,
        public_key_hex,
        public_key_npub,
        owner_pubkey_hex,
        owner_pubkey_npub,
        invalid_private_key,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(feature = "nostr")]
    fn nostr_status_marks_missing_owner_when_secret_exists() {
        let mut settings = crate::settings::Settings::default();
        settings.channels.nostr_enabled = true;

        unsafe {
            std::env::remove_var("NOSTR_ENABLED");
            std::env::set_var(
                "NOSTR_PRIVATE_KEY",
                "1111111111111111111111111111111111111111111111111111111111111111",
            );
            std::env::remove_var("NOSTR_OWNER_PUBKEY");
            std::env::remove_var("NOSTR_SECRET_KEY");
        }

        let status = super::build_nostr_setup_status(&settings, None);
        assert!(status.enabled);
        assert!(status.tool_ready);
        assert!(!status.control_ready);
        assert!(
            status
                .missing_fields
                .iter()
                .any(|field| field == "owner_pubkey")
        );

        unsafe {
            std::env::remove_var("NOSTR_ENABLED");
            std::env::remove_var("NOSTR_PRIVATE_KEY");
            std::env::remove_var("NOSTR_SECRET_KEY");
        }
    }
}

pub(crate) async fn costs_summary_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<crate::llm::cost_tracker::CostSummary>, StatusCode> {
    let tracker = state
        .cost_tracker
        .as_ref()
        .ok_or_else(cost_tracker_unavailable_status)?;
    let now = chrono::Utc::now();
    let today = now.format("%Y-%m-%d").to_string();
    let this_month = now.format("%Y-%m").to_string();
    let guard = tracker.lock().await;
    Ok(Json(guard.summary(&today, &this_month)))
}

pub(crate) async fn costs_export_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<
    (
        StatusCode,
        [(axum::http::header::HeaderName, String); 2],
        String,
    ),
    StatusCode,
> {
    let tracker = state
        .cost_tracker
        .as_ref()
        .ok_or_else(cost_tracker_unavailable_status)?;
    let guard = tracker.lock().await;
    let csv = guard.export_csv();
    let filename = format!(
        "thinclaw-costs-{}.csv",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );
    Ok((
        StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                "text/csv; charset=utf-8".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            ),
        ],
        csv,
    ))
}

pub(crate) async fn costs_reset_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<StatusCode, StatusCode> {
    let tracker = state
        .cost_tracker
        .as_ref()
        .ok_or_else(cost_tracker_unavailable_status)?;
    {
        let mut guard = tracker.lock().await;
        guard.clear();
    }
    if let Some(ref cg) = state.cost_guard {
        cg.reset().await;
    }
    if let Some(ref db) = state.store {
        let snapshot = tracker.lock().await.to_json();
        if let Err(e) = db.set_setting("default", "cost_entries", &snapshot).await {
            tracing::warn!("Failed to persist cleared cost entries: {}", e);
        }
    }
    Ok(StatusCode::NO_CONTENT)
}
