use super::*;
use async_trait::async_trait;
use futures::stream;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering as AtomicOrdering},
};
use tokio::sync::Mutex;

#[derive(Default)]
struct MockChannelState {
    broadcasts: Mutex<Vec<(String, String)>>,
    diagnostics_calls: Mutex<usize>,
}

struct MockChannel {
    name: String,
    state: Arc<MockChannelState>,
}

#[derive(Default)]
struct ForwardingChannelState {
    sender: Mutex<Option<mpsc::Sender<IncomingMessage>>>,
    shutdowns: AtomicUsize,
}

struct ForwardingChannel {
    name: String,
    state: Arc<ForwardingChannelState>,
}

struct FailingStartChannel {
    name: String,
}

struct HangingStatusChannel {
    name: String,
}

impl MockChannel {
    fn new(name: &str, state: Arc<MockChannelState>) -> Self {
        Self {
            name: name.to_string(),
            state,
        }
    }
}

impl ForwardingChannel {
    fn new(name: &str, state: Arc<ForwardingChannelState>) -> Self {
        Self {
            name: name.to_string(),
            state,
        }
    }
}

#[async_trait]
impl Channel for MockChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        Ok(Box::pin(stream::empty()))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        _response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.state
            .broadcasts
            .lock()
            .await
            .push((user_id.to_string(), response.content));
        Ok(())
    }

    async fn diagnostics(&self) -> Option<serde_json::Value> {
        let mut calls = self.state.diagnostics_calls.lock().await;
        *calls += 1;
        Some(serde_json::json!({"channel": self.name}))
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}

#[async_trait]
impl Channel for ForwardingChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel(1);
        *self.state.sender.lock().await = Some(tx);
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        _response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        self.state.shutdowns.fetch_add(1, AtomicOrdering::Relaxed);
        *self.state.sender.lock().await = None;
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}

#[async_trait]
impl Channel for FailingStartChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        Err(ChannelError::StartupFailed {
            name: self.name.clone(),
            reason: "intentional startup failure".to_string(),
        })
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        _response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Err(ChannelError::HealthCheckFailed {
            name: self.name.clone(),
        })
    }
}

#[async_trait]
impl Channel for HangingStatusChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        Ok(Box::pin(stream::empty()))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        _response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn send_status(
        &self,
        _status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        std::future::pending().await
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}

/// A channel whose `respond` signals when it starts, then blocks until
/// released — used to prove the manager isn't holding the channels lock
/// while a channel is mid-`respond`.
struct BlockingChannel {
    name: String,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl Channel for BlockingChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        Ok(Box::pin(stream::empty()))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        _response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}

#[tokio::test]
async fn respond_releases_channels_lock_before_awaiting_channel() {
    let manager = Arc::new(ChannelManager::new());
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    manager
        .add(Box::new(BlockingChannel {
            name: "slow".to_string(),
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
        }))
        .await;

    // Drive a respond that will block inside the channel.
    let m = Arc::clone(&manager);
    let slow = tokio::spawn(async move {
        let msg = IncomingMessage::new("slow", "user", "hi");
        m.respond(&msg, OutgoingResponse::text("x")).await
    });

    // Wait until the channel's respond is actually executing (mid-await).
    entered.notified().await;

    // A write-lock operation (hot_add) must succeed while the slow respond
    // is still in flight. If respond held the read guard across its await,
    // this write would block and the timeout would fire. This is the
    // regression guard for the lock-across-await fix.
    let added = tokio::time::timeout(
        Duration::from_secs(2),
        manager.hot_add(Box::new(MockChannel::new(
            "added",
            Arc::new(MockChannelState::default()),
        ))),
    )
    .await;
    assert!(
        added.is_ok(),
        "hot_add blocked — respond held the channels read guard across its await"
    );
    assert!(added.unwrap().is_ok());

    // Release the slow channel and let its task finish cleanly.
    release.notify_one();
    let _ = slow.await;
}

#[tokio::test]
async fn broadcast_resolves_legacy_web_alias_to_gateway() {
    let manager = ChannelManager::new();
    let state = Arc::new(MockChannelState::default());
    manager
        .add(Box::new(MockChannel::new("gateway", Arc::clone(&state))))
        .await;

    manager
        .broadcast("web", "user-1", OutgoingResponse::text("hello"))
        .await
        .expect("legacy web alias should reach gateway channel");

    let broadcasts = state.broadcasts.lock().await;
    assert_eq!(
        broadcasts.as_slice(),
        &[("user-1".to_string(), "hello".to_string())]
    );
}

#[tokio::test]
async fn status_delivery_is_bounded_at_the_manager_boundary() {
    let manager = ChannelManager::new();
    manager
        .add(Box::new(HangingStatusChannel {
            name: "hanging-status".to_string(),
        }))
        .await;

    let error = manager
        .send_status(
            "hanging-status",
            StatusUpdate::Thinking("working".to_string()),
            &serde_json::json!({}),
        )
        .await
        .expect_err("a transport that never resolves must time out");

    assert!(matches!(
        error,
        ChannelError::SendFailed { ref reason, .. }
            if reason.contains("status delivery timed out")
    ));
}

#[tokio::test]
async fn shutdown_closes_channel_lifecycle_admission() {
    let manager = ChannelManager::new();
    manager
        .add(Box::new(MockChannel::new(
            "initial",
            Arc::new(MockChannelState::default()),
        )))
        .await;
    manager.shutdown_all().await.expect("shutdown succeeds");

    let error = manager
        .hot_add(Box::new(MockChannel::new(
            "late",
            Arc::new(MockChannelState::default()),
        )))
        .await
        .expect_err("late channel admission must be rejected");
    assert!(error.to_string().contains("shutting down"));

    manager
        .add(Box::new(MockChannel::new(
            "late-static",
            Arc::new(MockChannelState::default()),
        )))
        .await;
    assert!(
        !manager
            .channel_names()
            .await
            .iter()
            .any(|name| name == "late-static")
    );
}

#[tokio::test]
async fn channel_diagnostics_resolves_legacy_web_alias_to_gateway() {
    let manager = ChannelManager::new();
    let state = Arc::new(MockChannelState::default());
    manager
        .add(Box::new(MockChannel::new("gateway", Arc::clone(&state))))
        .await;

    let diagnostics = manager
        .channel_diagnostics("web")
        .await
        .expect("legacy web alias should resolve diagnostics");
    assert_eq!(
        diagnostics.get("channel").and_then(|value| value.as_str()),
        Some("gateway")
    );
    assert_eq!(*state.diagnostics_calls.lock().await, 1);
}

#[tokio::test]
async fn startup_failure_is_reported_as_failed_not_running() {
    let manager = ChannelManager::new();
    manager
        .add(Box::new(MockChannel::new(
            "healthy",
            Arc::new(MockChannelState::default()),
        )))
        .await;
    manager
        .add(Box::new(FailingStartChannel {
            name: "broken".to_string(),
        }))
        .await;

    let _stream = manager
        .start_all()
        .await
        .expect("one healthy channel should keep the runtime available");
    let entries = manager.status_entries().await;
    let broken = entries
        .iter()
        .find(|entry| entry.name == "broken")
        .expect("failed channel remains visible for diagnostics");

    assert!(matches!(broken.state, ChannelViewState::Failed { .. }));
    assert_eq!(broken.errors, 1);
    assert!(
        broken
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("intentional startup failure"))
    );
}

#[tokio::test]
async fn hot_add_emits_status_through_gateway_neutral_sink() {
    let manager = ChannelManager::new();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    manager
        .set_status_change_sink(move |event| {
            let _ = event_tx.send(event);
        })
        .await;

    manager
        .hot_add(Box::new(MockChannel::new(
            "dynamic",
            Arc::new(MockChannelState::default()),
        )))
        .await
        .expect("hot add succeeds");

    let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("status event should arrive")
        .expect("status sink remains open");
    assert_eq!(event.channel, "dynamic");
    assert_eq!(event.status, "online");
}

#[tokio::test]
async fn status_entries_include_native_lifecycle_surfaces_without_shadowing_active_channels() {
    let manager = ChannelManager::new();
    manager
        .add_descriptor(ChannelDescriptor::native_lifecycle(
            "matrix",
            true,
            true,
            "Matrix rooms and DMs",
        ))
        .await;
    manager
        .add_descriptor(ChannelDescriptor::native_lifecycle(
            "gateway",
            true,
            true,
            "Gateway lifecycle surface should be shadowed",
        ))
        .await;
    manager
        .add(Box::new(MockChannel::new(
            "gateway",
            Arc::new(MockChannelState::default()),
        )))
        .await;

    let entries = manager.status_entries().await;
    let matrix = entries
        .iter()
        .find(|entry| entry.name == "matrix")
        .expect("matrix lifecycle surface should be visible");
    assert_eq!(matrix.channel_type, "native-lifecycle");
    assert_eq!(matrix.state, ChannelViewState::Disabled);
    assert!(
        matrix
            .last_error
            .as_deref()
            .is_some_and(|err| err.contains("no native transport instance"))
    );

    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.name == "gateway")
            .count(),
        1
    );
    let gateway = entries
        .iter()
        .find(|entry| entry.name == "gateway")
        .expect("active gateway entry should remain");
    assert_eq!(gateway.channel_type, "gateway");
}

#[tokio::test]
async fn hot_remove_drains_stream_forwarder() {
    let manager = ChannelManager::new();
    let state = Arc::new(ForwardingChannelState::default());
    manager
        .hot_add(Box::new(ForwardingChannel::new(
            "forwarded",
            Arc::clone(&state),
        )))
        .await
        .unwrap();

    assert!(
        manager
            .stream_forwarders
            .read()
            .await
            .contains_key("forwarded")
    );

    manager.hot_remove("forwarded").await.unwrap();

    assert!(manager.stream_forwarders.read().await.is_empty());
    assert_eq!(state.shutdowns.load(AtomicOrdering::Relaxed), 1);
}
