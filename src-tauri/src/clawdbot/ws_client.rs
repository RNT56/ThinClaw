//! WebSocket client for Moltbot Gateway
//!
//! Implements:
//! - Connection with challenge/response handshake
//! - Automatic reconnection with exponential backoff
//! - RPC request/response correlation
//! - Event streaming to UI

use std::collections::HashMap;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};
use tungstenite::Message;

use super::frames::{self, WsFrame};
use super::normalizer::{self, UiEvent};

use ed25519_dalek::{Signature, Signer, SigningKey};
use pkcs8::DecodePrivateKey;

/// Client errors
#[derive(Debug, Error)]
pub enum ClientError {
    #[error("WebSocket error: {0}")]
    Ws(#[from] tungstenite::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Gateway protocol error: {0}")]
    Protocol(String),

    #[error("Timeout")]
    Timeout,

    #[error("Channel closed")]
    ChannelClosed,
}

/// Commands that can be sent to the WS client actor
#[derive(Debug)]
pub enum ClientCommand {
    /// Send an RPC request and await response
    Rpc {
        method: String,
        params: Value,
        response_tx: oneshot::Sender<Result<Value, ClientError>>,
    },
    /// Disconnect and stop the client
    Shutdown,
}

/// WebSocket client for Moltbot Gateway
pub struct ClawdbotWsClient {
    gateway_url: String,
    token: String,
    device_id: String,
    private_key_pem: Option<String>,
    public_key_pem: Option<String>,

    /// Channel to send UI events
    ui_tx: mpsc::Sender<UiEvent>,

    /// Channel to receive commands
    cmd_rx: mpsc::Receiver<ClientCommand>,

    /// Pending RPC requests awaiting responses
    pending: HashMap<String, oneshot::Sender<Result<Value, ClientError>>>,

    /// Whether the client should continue running
    running: bool,
}

/// Handle for sending commands to the WS client
#[derive(Clone)]
pub struct ClawdbotWsHandle {
    cmd_tx: mpsc::Sender<ClientCommand>,
}

impl ClawdbotWsHandle {
    /// Send an RPC request and await response
    pub async fn rpc(&self, method: &str, params: Value) -> Result<Value, ClientError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(ClientCommand::Rpc {
                method: method.into(),
                params,
                response_tx: tx,
            })
            .await
            .map_err(|_| ClientError::ChannelClosed)?;

        rx.await.map_err(|_| ClientError::ChannelClosed)?
    }

    /// Request gateway status
    #[allow(dead_code)]
    pub async fn status(&self) -> Result<Value, ClientError> {
        self.rpc("status", serde_json::json!({})).await
    }

    /// Get session list
    pub async fn sessions_list(&self) -> Result<Value, ClientError> {
        let res = self.rpc("sessions.list", serde_json::json!({})).await?;
        info!("Sessions list raw response: {:?}", res);
        Ok(res)
    }

    /// Get chat history
    pub async fn chat_history(
        &self,
        session_key: &str,
        limit: u32,
        _before: Option<&str>,
    ) -> Result<Value, ClientError> {
        self.rpc(
            "chat.history",
            serde_json::json!({
                "sessionKey": session_key,
                "limit": limit
            }),
        )
        .await
    }

    /// Send a chat message
    pub async fn chat_send(
        &self,
        session_key: &str,
        idempotency_key: &str,
        text: &str,
        _deliver: bool,
    ) -> Result<Value, ClientError> {
        self.rpc(
            "chat.send",
            serde_json::json!({
                "sessionKey": session_key,
                "idempotencyKey": idempotency_key,
                "message": text
            }),
        )
        .await
    }

    /// Delete a session
    pub async fn session_delete(&self, session_key: &str) -> Result<Value, ClientError> {
        info!("Sending sessions.delete for key: {}", session_key);
        let res = self
            .rpc(
                "sessions.delete",
                serde_json::json!({
                    "key": session_key
                }),
            )
            .await?;
        info!("sessions.delete response: {:?}", res);
        Ok(res)
    }

    /// Reset a session (clear history)
    pub async fn session_reset(&self, session_key: &str) -> Result<Value, ClientError> {
        info!("Sending sessions.reset for key: {}", session_key);
        let res = self
            .rpc(
                "sessions.reset",
                serde_json::json!({
                    "key": session_key
                }),
            )
            .await?;
        info!("sessions.reset response: {:?}", res);
        Ok(res)
    }

    /// Subscribe to chat events for a session
    #[allow(dead_code)]
    pub async fn chat_subscribe(&self, session_key: &str) -> Result<Value, ClientError> {
        self.rpc(
            "chat.subscribe",
            serde_json::json!({ "sessionKey": session_key }),
        )
        .await
    }

    /// Abort a chat run
    pub async fn chat_abort(
        &self,
        session_key: &str,
        run_id: Option<&str>,
    ) -> Result<Value, ClientError> {
        self.rpc(
            "chat.abort",
            serde_json::json!({
                "sessionKey": session_key,
                "runId": run_id
            }),
        )
        .await
    }

    /// Resolve an approval request
    pub async fn approval_resolve(
        &self,
        approval_id: &str,
        approved: bool,
    ) -> Result<Value, ClientError> {
        self.rpc(
            "exec.approval.resolve",
            serde_json::json!({
                "approvalId": approval_id,
                "approved": approved
            }),
        )
        .await
    }

    /// List cron jobs
    pub async fn cron_list(&self) -> Result<Value, ClientError> {
        self.rpc("cron.list", serde_json::json!({})).await
    }

    /// Run a cron job
    pub async fn cron_run(&self, key: &str) -> Result<Value, ClientError> {
        self.rpc("cron.run", serde_json::json!({ "key": key }))
            .await
    }

    /// Get cron history
    pub async fn cron_history(&self, key: &str, limit: u32) -> Result<Value, ClientError> {
        self.rpc(
            "cron.history",
            serde_json::json!({ "key": key, "limit": limit }),
        )
        .await
    }

    /// List skills
    pub async fn skills_list(&self) -> Result<Value, ClientError> {
        self.rpc("skills.list", serde_json::json!({})).await
    }

    /// Get detailed skills status
    pub async fn skills_status(&self) -> Result<Value, ClientError> {
        self.rpc("skills.status", serde_json::json!({})).await
    }

    /// Update skill (toggle enabled state)
    pub async fn skills_update(
        &self,
        skill_key: &str,
        enabled: bool,
    ) -> Result<Value, ClientError> {
        self.rpc(
            "skills.update",
            serde_json::json!({ "skillKey": skill_key, "enabled": enabled }),
        )
        .await
    }

    /// Install skill dependencies
    pub async fn skills_install(
        &self,
        name: &str,
        install_id: Option<&str>,
    ) -> Result<Value, ClientError> {
        self.rpc(
            "skills.install",
            serde_json::json!({ "name": name, "installId": install_id }),
        )
        .await
    }

    /// Get config schema
    pub async fn config_schema(&self) -> Result<Value, ClientError> {
        self.rpc("config.schema", serde_json::json!({})).await
    }

    /// Get config value
    pub async fn config_get(&self) -> Result<Value, ClientError> {
        self.rpc("config.get", serde_json::json!({})).await
    }

    /// Set config value
    pub async fn config_set(&self, key: &str, value: Value) -> Result<Value, ClientError> {
        self.rpc(
            "config.set",
            serde_json::json!({ "key": key, "value": value }),
        )
        .await
    }

    /// Patch config
    pub async fn config_patch(&self, patch: Value) -> Result<Value, ClientError> {
        self.rpc("config.patch", patch).await
    }

    /// Get system presence (nodes/instances)
    pub async fn system_presence(&self) -> Result<Value, ClientError> {
        self.rpc("system.presence", serde_json::json!({})).await
    }

    /// Tail logs
    pub async fn logs_tail(&self, limit: u32) -> Result<Value, ClientError> {
        self.rpc("logs.tail", serde_json::json!({ "limit": limit }))
            .await
    }

    /// Trigger update
    pub async fn update_run(&self) -> Result<Value, ClientError> {
        self.rpc("update.run", serde_json::json!({})).await
    }

    /// WhatsApp login
    pub async fn web_login_whatsapp(&self) -> Result<Value, ClientError> {
        self.rpc("web.login.whatsapp", serde_json::json!({})).await
    }

    /// Telegram login
    pub async fn web_login_telegram(&self) -> Result<Value, ClientError> {
        self.rpc("web.login.telegram", serde_json::json!({})).await
    }

    /// Shutdown the client
    pub async fn shutdown(&self) -> Result<(), ClientError> {
        self.cmd_tx
            .send(ClientCommand::Shutdown)
            .await
            .map_err(|_| ClientError::ChannelClosed)
    }
}

impl ClawdbotWsClient {
    /// Create a new WS client and return both the client and a handle
    pub fn new(
        gateway_url: String,
        token: String,
        device_id: String,
        private_key_pem: Option<String>,
        public_key_pem: Option<String>,
        ui_tx: mpsc::Sender<UiEvent>,
    ) -> (Self, ClawdbotWsHandle) {
        let (cmd_tx, cmd_rx) = mpsc::channel(32);

        let client = Self {
            gateway_url,
            token,
            device_id,
            private_key_pem,
            public_key_pem,
            ui_tx,
            cmd_rx,
            pending: HashMap::new(),
            running: true,
        };

        let handle = ClawdbotWsHandle { cmd_tx };

        (client, handle)
    }

    /// Run the client forever with automatic reconnection
    pub async fn run_forever(mut self) {
        let mut backoff = Duration::from_millis(250);
        let max_backoff = Duration::from_secs(10);

        while self.running {
            match self.run_once().await {
                Ok(_) => {
                    backoff = Duration::from_millis(250);
                }
                Err(e) => {
                    error!("WS connection error: {}", e);
                    let _ = self
                        .ui_tx
                        .send(UiEvent::Disconnected {
                            reason: e.to_string(),
                        })
                        .await;
                    tokio::time::sleep(backoff).await;
                    backoff = std::cmp::min(max_backoff, backoff * 2);
                }
            }
        }

        info!("Clawdbot WS client shutting down");
    }

    /// Run a single connection attempt
    async fn run_once(&mut self) -> Result<(), ClientError> {
        info!("Connecting to Moltbot gateway: {}", self.gateway_url);

        let (ws_stream, _resp) = tokio_tungstenite::connect_async(&self.gateway_url).await?;
        let (mut write, mut read) = ws_stream.split();

        // Wait for connect.challenge
        let nonce = self.wait_for_challenge(&mut read).await?;
        debug!("Received challenge, nonce: {:?}", nonce);
        let signed_at = chrono::Utc::now().timestamp_millis();

        // Compute signature if key available
        let mut signature_str = None;
        if let (Some(nonce), Some(pem)) = (nonce.as_ref(), self.private_key_pem.as_ref()) {
            if let Ok(signing_key) = SigningKey::from_pkcs8_pem(pem) {
                // MATCH MOLTBOT buildDeviceAuthPayload
                // base = [version, deviceId, clientId, clientMode, role, scopes, signedAtMs, token, nonce] (joined by |)
                let scopes = "operator.read,operator.write,operator.approvals,operator.admin";
                let payload = format!(
                    "v2|{}|cli|cli|operator|{}|{}|{}|{}",
                    self.device_id, scopes, signed_at, self.token, nonce
                );

                let sig: Signature = signing_key.sign(payload.as_bytes());
                signature_str = Some(base64::Engine::encode(
                    &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                    sig.to_bytes(),
                ));
            }
        }

        // Send connect request
        let connect_id = uuid::Uuid::new_v4().to_string();
        let connect = frames::build_connect_req(
            connect_id.clone(),
            &self.token,
            &self.device_id,
            nonce.as_deref(),
            self.public_key_pem.as_deref(),
            signature_str.as_deref(),
            signed_at,
        );

        let msg = serde_json::to_string(&connect)?;
        write.send(Message::Text(msg.into())).await?;

        // Wait for connect response
        let protocol = self.wait_for_ok_response(&mut read, &connect_id).await?;
        info!("Connected to gateway, protocol version: {}", protocol);

        let _ = self.ui_tx.send(UiEvent::Connected { protocol }).await;

        // Main event loop
        loop {
            tokio::select! {
                // Handle incoming WS messages
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(txt))) => {
                            // Log first 200 chars to show what's coming in
                            let preview: String = txt.chars().take(200).collect();
                            info!("[ws_client] WS Received: {}...", preview);
                            match serde_json::from_str::<WsFrame>(&txt) {
                                Ok(frame) => self.handle_incoming_frame(&mut write, frame).await?,
                                Err(e) => warn!("Failed to parse WS frame: {}", e),
                            }
                        }
                        Some(Ok(Message::Ping(data))) => {
                            write.send(Message::Pong(data)).await?;
                        }
                        Some(Ok(Message::Close(_))) => {
                            return Err(ClientError::Protocol("Connection closed by server".into()));
                        }
                        Some(Err(e)) => return Err(e.into()),
                        None => return Err(ClientError::Protocol("WS stream ended".into())),
                        _ => {} // Ignore other message types
                    }
                }

                // Handle commands from the handle
                cmd = self.cmd_rx.recv() => {
                    match cmd {
                        Some(ClientCommand::Rpc { method, params, response_tx }) => {
                            self.send_rpc(&mut write, method, params, response_tx).await?;
                        }
                        Some(ClientCommand::Shutdown) => {
                            self.running = false;
                            return Ok(());
                        }
                        None => {
                            // Command channel closed, shutdown
                            self.running = false;
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    /// Wait for the connect.challenge event
    async fn wait_for_challenge<S>(&self, read: &mut S) -> Result<Option<String>, ClientError>
    where
        S: StreamExt<Item = Result<Message, tungstenite::Error>> + Unpin,
    {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let msg = tokio::time::timeout(remaining, read.next())
                .await
                .map_err(|_| ClientError::Timeout)?;

            let msg =
                msg.ok_or_else(|| ClientError::Protocol("WS closed before challenge".into()))??;

            if let Message::Text(txt) = msg {
                if let Ok(frame) = serde_json::from_str::<WsFrame>(&txt) {
                    if let WsFrame::Event { event, payload, .. } = frame {
                        if event == "connect.challenge" {
                            let nonce = payload
                                .get("nonce")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            return Ok(nonce);
                        }
                    }
                }
            }
        }
    }

    /// Wait for an OK response to a specific request
    async fn wait_for_ok_response<S>(&mut self, read: &mut S, id: &str) -> Result<u32, ClientError>
    where
        S: StreamExt<Item = Result<Message, tungstenite::Error>> + Unpin,
    {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let msg = tokio::time::timeout(remaining, read.next())
                .await
                .map_err(|_| ClientError::Timeout)?;

            let msg = msg.ok_or_else(|| ClientError::Protocol("WS closed".into()))??;

            if let Message::Text(txt) = msg {
                if let Ok(frame) = serde_json::from_str::<WsFrame>(&txt) {
                    match frame {
                        WsFrame::Res {
                            id: rid,
                            ok,
                            payload,
                            error,
                        } if rid == id => {
                            if ok {
                                let protocol = payload
                                    .get("protocol")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(3)
                                    as u32;
                                return Ok(protocol);
                            } else {
                                let e = error.unwrap_or_default();
                                return Err(ClientError::Protocol(format!(
                                    "connect failed: {} {}",
                                    e.code, e.message
                                )));
                            }
                        }
                        _ => {} // Ignore other frames
                    }
                }
            }
        }
    }

    /// Handle an incoming frame
    async fn handle_incoming_frame<W>(
        &mut self,
        _write: &mut W,
        frame: WsFrame,
    ) -> Result<(), ClientError>
    where
        W: SinkExt<Message> + Unpin,
        W::Error: std::fmt::Debug,
    {
        match &frame {
            WsFrame::Res {
                id,
                ok,
                payload,
                error,
            } => {
                if let Some(tx) = self.pending.remove(id) {
                    if *ok {
                        let _ = tx.send(Ok(payload.clone()));
                    } else {
                        let e = error.clone().unwrap_or_default();
                        let _ = tx.send(Err(ClientError::Protocol(format!(
                            "{}: {}",
                            e.code, e.message
                        ))));
                    }
                }
            }
            WsFrame::Event { event, .. } => {
                info!("[ws_client] Received Event frame: {}", event);
                if let Some(ui) = normalizer::normalize_event(&frame) {
                    info!("[ws_client] Forwarding normalized UI event to frontend");
                    let _ = self.ui_tx.send(ui).await;
                } else {
                    info!("[ws_client] Event was not normalized (dropped): {}", event);
                }
            }
            WsFrame::Req { .. } => {
                // Gateway shouldn't send requests to operator client
                warn!("Unexpected Req frame from gateway");
            }
        }

        Ok(())
    }

    /// Send an RPC request
    async fn send_rpc<W>(
        &mut self,
        write: &mut W,
        method: String,
        params: Value,
        response_tx: oneshot::Sender<Result<Value, ClientError>>,
    ) -> Result<(), ClientError>
    where
        W: SinkExt<Message> + Unpin,
        W::Error: std::fmt::Debug,
    {
        let id = uuid::Uuid::new_v4().to_string();
        self.pending.insert(id.clone(), response_tx);

        let req = WsFrame::Req { id, method, params };
        let msg = serde_json::to_string(&req)?;

        write
            .send(Message::Text(msg.into()))
            .await
            .map_err(|_| ClientError::Protocol("Failed to send".into()))?;

        Ok(())
    }
}
