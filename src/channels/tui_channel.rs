use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::channels::{
    Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate, StreamMode,
};
use crate::tui::{TuiApp, TuiEvent, TuiUpdate};

fn portable_capabilities(
    snapshot: crate::tools::RegistrySnapshot,
) -> thinclaw_channels::tui::TuiCapabilitySnapshot {
    thinclaw_channels::tui::TuiCapabilitySnapshot {
        revision: snapshot.revision,
        sealed: snapshot.sealed,
        identities: snapshot
            .identities
            .into_iter()
            .map(|identity| thinclaw_channels::tui::TuiCapabilityIdentity {
                name: identity.name,
                origin: identity.origin.to_string(),
                source_id: identity.source_id,
                revision: identity.revision,
                compiled: identity.compiled,
                configured: identity.configured,
                registered: identity.registered,
                dependency: identity.dependency,
                exposed: identity.exposed,
                ready: identity.ready,
                approval: identity.approval,
                health: identity.health,
                reasons: identity.reasons,
            })
            .collect(),
    }
}

struct RootTuiRuntime {
    tools: Arc<crate::tools::ToolRegistry>,
}

impl thinclaw_channels::tui::TuiRuntime for RootTuiRuntime {
    fn start(
        &self,
        mut bootstrap: thinclaw_channels::tui::TuiBootstrap,
        outgoing_tx: mpsc::Sender<TuiEvent>,
        mut incoming_rx: mpsc::Receiver<TuiUpdate>,
    ) -> tokio::task::JoinHandle<()> {
        let mut registry_rx = self.tools.subscribe_registry();
        tokio::spawn(async move {
            while !registry_rx.borrow().sealed {
                if registry_rx.changed().await.is_err() {
                    tracing::error!("Registry snapshot feed closed before startup seal");
                    return;
                }
            }
            bootstrap.capabilities = portable_capabilities(registry_rx.borrow().clone());

            let (merged_tx, merged_rx) = mpsc::channel(1024);
            let incoming_tx = merged_tx.clone();
            let incoming_relay = tokio::spawn(async move {
                while let Some(update) = incoming_rx.recv().await {
                    if incoming_tx.send(update).await.is_err() {
                        break;
                    }
                }
            });
            let capability_relay = tokio::spawn(async move {
                let mut delivered = registry_rx.borrow().revision;
                loop {
                    if registry_rx.changed().await.is_err() {
                        break;
                    }
                    let snapshot = registry_rx.borrow().clone();
                    if snapshot.revision <= delivered {
                        continue;
                    }
                    delivered = snapshot.revision;
                    if merged_tx
                        .send(TuiUpdate::CapabilitySnapshot(portable_capabilities(
                            snapshot,
                        )))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });

            let mut app = TuiApp::new(outgoing_tx, merged_rx, bootstrap);
            if let Err(error) = app.run().await {
                tracing::error!(error = %error, "TUI runtime exited with an error");
            }
            incoming_relay.abort();
            capability_relay.abort();
        })
    }
}

struct RootTuiHistoryPort {
    store: Arc<dyn crate::db::Database>,
}

#[async_trait]
impl thinclaw_channels::tui::TuiHistoryPort for RootTuiHistoryPort {
    async fn load_before(
        &self,
        conversation_id: &str,
        before_cursor: Option<&str>,
        limit: usize,
    ) -> Result<thinclaw_channels::tui::TuiHistoryPage, String> {
        let conversation_id = uuid::Uuid::parse_str(conversation_id)
            .map_err(|_| "conversation history identity is invalid".to_string())?;
        let limit = limit.clamp(1, thinclaw_channels::tui::TUI_HISTORY_MAX_PAGE_SIZE);
        let before_row = match before_cursor {
            Some(cursor) => parse_history_row_cursor(cursor)?,
            None => self
                .store
                .count_conversation_messages(conversation_id)
                .await
                .map_err(|error| format!("failed to count durable conversation history: {error}"))?
                .max(0) as usize,
        };
        let start_row = before_row.saturating_sub(limit);
        let page_len = before_row.saturating_sub(start_row);
        let messages = self
            .store
            .list_conversation_messages_window(conversation_id, start_row as i64, page_len as i64)
            .await
            .map_err(|error| format!("failed to load durable conversation history: {error}"))?;
        Ok(thinclaw_channels::tui::TuiHistoryPage {
            conversation_id: conversation_id.to_string(),
            messages: messages.into_iter().map(portable_history_message).collect(),
            before_cursor: Some(history_row_cursor(start_row)),
            has_more: start_row > 0,
        })
    }
}

fn history_row_cursor(row: usize) -> String {
    format!("row:{row}")
}

fn parse_history_row_cursor(cursor: &str) -> Result<usize, String> {
    cursor
        .strip_prefix("row:")
        .and_then(|row| row.parse::<usize>().ok())
        .ok_or_else(|| "conversation history cursor is invalid".to_string())
}

fn portable_history_message(
    message: crate::history::ConversationMessage,
) -> thinclaw_channels::tui::TuiHistoryMessage {
    let model = message
        .metadata
        .get("provider_model")
        .or_else(|| message.metadata.get("model"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    thinclaw_channels::tui::TuiHistoryMessage {
        id: message.id.to_string(),
        role: message.role,
        content: message.content,
        model,
        created_at: message.created_at.to_rfc3339(),
    }
}

pub struct TuiChannel {
    inner: thinclaw_channels::tui::TuiChannel,
}

impl TuiChannel {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        principal_id: String,
        actor_id: String,
        agent_id: String,
        agent_name: String,
        model: String,
        provider: String,
        workspace: Option<String>,
        profile: String,
        gateway_origin: Option<String>,
        store: Option<Arc<dyn crate::db::Database>>,
        tools: Arc<crate::tools::ToolRegistry>,
    ) -> Self {
        let history_port = store.as_ref().map(|store| {
            Arc::new(RootTuiHistoryPort {
                store: Arc::clone(store),
            }) as Arc<dyn thinclaw_channels::tui::TuiHistoryPort>
        });
        let history = load_initial_history(store.as_deref(), &principal_id, &actor_id).await;
        let bootstrap = thinclaw_channels::tui::TuiBootstrap {
            principal_id,
            actor_id,
            agent_id,
            agent_name,
            model,
            provider,
            workspace,
            profile,
            gateway_origin: gateway_origin
                .and_then(|origin| crate::tui::credential_free_gateway_url(&origin)),
            history,
            capabilities: portable_capabilities(tools.registry_snapshot()),
        };
        Self {
            inner: thinclaw_channels::tui::TuiChannel::new(
                Arc::new(RootTuiRuntime { tools }),
                bootstrap,
                history_port,
            ),
        }
    }
}

async fn load_initial_history(
    store: Option<&dyn crate::db::Database>,
    principal_id: &str,
    actor_id: &str,
) -> thinclaw_channels::tui::TuiHistoryPage {
    let Some(store) = store else {
        return empty_history_page();
    };
    let summaries = match store
        .list_actor_conversations_for_recall(principal_id, actor_id, false, 50)
        .await
    {
        Ok(summaries) => summaries,
        Err(error) => {
            tracing::warn!(%error, "Failed to identify the primary TUI conversation");
            return empty_history_page();
        }
    };
    let mut fallback = None;
    let mut primary = None;
    for summary in summaries {
        fallback.get_or_insert(summary.id);
        let metadata = store
            .get_conversation_metadata(summary.id)
            .await
            .ok()
            .flatten();
        if metadata.as_ref().is_some_and(|metadata| {
            thinclaw_agent::thread_ops::direct_conversation_candidate_is_primary(
                metadata,
                summary.thread_type.as_deref(),
            )
        }) {
            primary = Some(summary.id);
            break;
        }
    }
    let Some(conversation_id) = primary.or(fallback) else {
        return empty_history_page();
    };
    let total = match store.count_conversation_messages(conversation_id).await {
        Ok(total) => total.max(0) as usize,
        Err(error) => {
            tracing::warn!(%error, "Failed to count initial TUI conversation history");
            return empty_history_page();
        }
    };
    let start_row = total.saturating_sub(thinclaw_channels::tui::TUI_HISTORY_PAGE_SIZE);
    match store
        .list_conversation_messages_window(
            conversation_id,
            start_row as i64,
            (total - start_row) as i64,
        )
        .await
    {
        Ok(messages) => thinclaw_channels::tui::TuiHistoryPage {
            conversation_id: conversation_id.to_string(),
            before_cursor: Some(history_row_cursor(start_row)),
            messages: messages.into_iter().map(portable_history_message).collect(),
            has_more: start_row > 0,
        },
        Err(error) => {
            tracing::warn!(%error, "Failed to hydrate initial TUI conversation history");
            empty_history_page()
        }
    }
}

fn empty_history_page() -> thinclaw_channels::tui::TuiHistoryPage {
    thinclaw_channels::tui::TuiHistoryPage {
        conversation_id: String::new(),
        messages: Vec::new(),
        before_cursor: None,
        has_more: false,
    }
}

#[async_trait]
impl Channel for TuiChannel {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn stream_mode(&self) -> StreamMode {
        self.inner.stream_mode()
    }

    async fn start(&self) -> Result<MessageStream, crate::error::ChannelError> {
        self.inner.start().await
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), crate::error::ChannelError> {
        self.inner.respond(msg, response).await
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), crate::error::ChannelError> {
        self.inner.send_status(status, metadata).await
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), crate::error::ChannelError> {
        self.inner.broadcast(user_id, response).await
    }

    async fn health_check(&self) -> Result<(), crate::error::ChannelError> {
        self.inner.health_check().await
    }

    async fn shutdown(&self) -> Result<(), crate::error::ChannelError> {
        self.inner.shutdown().await
    }
}

#[cfg(test)]
mod history_cursor_tests {
    use super::*;

    #[test]
    fn row_cursor_round_trips_without_timestamp_ambiguity() {
        for row in [0, 1, 99, 100, usize::MAX] {
            let cursor = history_row_cursor(row);
            assert_eq!(parse_history_row_cursor(&cursor).unwrap(), row);
        }
    }

    #[test]
    fn row_cursor_rejects_unversioned_or_malformed_values() {
        for cursor in ["", "0", "row:", "row:-1", "timestamp:123", "row:1:2"] {
            assert!(parse_history_row_cursor(cursor).is_err(), "{cursor}");
        }
    }
}
