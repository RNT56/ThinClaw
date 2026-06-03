use std::{convert::Infallible, sync::Arc};

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::StreamExt;

use crate::channels::web::server::GatewayState;
use crate::channels::web::types::{LogLevelRequest, LogLevelResponse, LogsRecentResponse};
use thinclaw_gateway::web::logs::{
    invalid_log_level_error, log_broadcaster_unavailable_error,
    log_level_control_unavailable_error, log_level_response, logs_recent_response,
};

pub(crate) async fn logs_events_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let broadcaster = state
        .log_broadcaster
        .as_ref()
        .ok_or_else(log_broadcaster_unavailable_error)?;

    let rx = broadcaster.subscribe();
    let history = broadcaster.recent_entries();

    let history_stream = futures::stream::iter(history).map(|entry| {
        let data = serde_json::to_string(&entry).unwrap_or_default();
        Ok::<_, Infallible>(Event::default().event("log").data(data))
    });

    let live_stream = tokio_stream::wrappers::BroadcastStream::new(rx)
        .filter_map(|result| futures::future::ready(result.ok()))
        .map(|entry| {
            let data = serde_json::to_string(&entry).unwrap_or_default();
            Ok::<_, Infallible>(Event::default().event("log").data(data))
        });

    let stream = history_stream.chain(live_stream);

    Ok((
        [("X-Accel-Buffering", "no"), ("Cache-Control", "no-cache")],
        Sse::new(stream).keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(30))
                .text(""),
        ),
    ))
}

pub(crate) async fn logs_recent_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<LogsRecentResponse>, (StatusCode, String)> {
    let broadcaster = state
        .log_broadcaster
        .as_ref()
        .ok_or_else(log_broadcaster_unavailable_error)?;
    let logs = broadcaster.recent_entries();
    Ok(Json(logs_recent_response(logs)))
}

pub(crate) async fn logs_level_get_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<LogLevelResponse>, (StatusCode, String)> {
    let handle = state
        .log_level_handle
        .as_ref()
        .ok_or_else(log_level_control_unavailable_error)?;
    Ok(Json(log_level_response(handle.current_level())))
}

pub(crate) async fn logs_level_set_handler(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<LogLevelRequest>,
) -> Result<Json<LogLevelResponse>, (StatusCode, String)> {
    let handle = state
        .log_level_handle
        .as_ref()
        .ok_or_else(log_level_control_unavailable_error)?;

    handle
        .set_level(&body.level)
        .map_err(invalid_log_level_error)?;

    tracing::info!("Log level changed to '{}'", handle.current_level());
    Ok(Json(log_level_response(handle.current_level())))
}
