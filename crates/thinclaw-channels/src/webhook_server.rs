//! Unified HTTP server for all webhook routes.
//!
//! Composes route fragments from HttpChannel, WASM channel router, etc.
//! into a single axum server. Channels define routes but never spawn servers.

use std::net::SocketAddr;

use axum::Router;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use thinclaw_types::error::ChannelError;

/// Configuration for the unified webhook server.
pub struct WebhookServerConfig {
    /// Address to bind the server to.
    pub addr: SocketAddr,
}

/// A single HTTP server that hosts all webhook routes.
///
/// Channels contribute route fragments via `add_routes()`, then a single
/// `start()` call binds the listener and spawns the server task.
pub struct WebhookServer {
    config: WebhookServerConfig,
    routes: Vec<Router>,
    /// Fully merged router (retained for hot-swap via SIGHUP).
    merged_app: Option<Router>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl WebhookServer {
    /// Create a new webhook server with the given bind address.
    pub fn new(config: WebhookServerConfig) -> Self {
        Self {
            config,
            routes: Vec::new(),
            merged_app: None,
            shutdown_tx: None,
            handle: None,
        }
    }

    /// Accumulate a route fragment. Each fragment should already have its
    /// state applied via `.with_state()`.
    pub fn add_routes(&mut self, router: Router) {
        self.routes.push(router);
    }

    /// Return the current listen address.
    pub fn current_addr(&self) -> SocketAddr {
        self.config.addr
    }

    /// Clone the merged axum Router for use in a listener swap.
    ///
    /// Returns `None` if the server has not been started (no merged router).
    pub fn merged_router_clone(&self) -> Option<Router> {
        self.merged_app.clone()
    }

    /// Hot-swap the running listener with a new address and pre-bound listener.
    ///
    /// Returns the old shutdown sender and task handle so the caller can
    /// gracefully shut down the previous listener *outside* the lock.
    pub fn install_listener(
        &mut self,
        addr: SocketAddr,
        listener: tokio::net::TcpListener,
        app: Router,
    ) -> (Option<oneshot::Sender<()>>, Option<JoinHandle<()>>) {
        // Capture old state for caller to shut down
        let old_shutdown_tx = self.shutdown_tx.take();
        let old_handle = self.handle.take();

        // Update config
        self.config.addr = addr;
        self.merged_app = Some(app.clone());

        // Spawn new server
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                    tracing::info!("Webhook server shutting down (hot-swap)");
                })
                .await
            {
                tracing::error!("Webhook server error: {}", e);
            }
        });

        self.handle = Some(handle);
        (old_shutdown_tx, old_handle)
    }

    /// Bind the listener, merge all route fragments, and spawn the server.
    pub async fn start(&mut self) -> Result<(), ChannelError> {
        let mut app = Router::new();
        for fragment in self.routes.drain(..) {
            app = app.merge(fragment);
        }

        // Retain merged router for SIGHUP hot-swap
        self.merged_app = Some(app.clone());

        let listener = tokio::net::TcpListener::bind(self.config.addr)
            .await
            .map_err(|e| ChannelError::StartupFailed {
                name: "webhook_server".to_string(),
                reason: format!("Failed to bind to {}: {}", self.config.addr, e),
            })?;

        tracing::info!("Webhook server listening on {}", self.config.addr);

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                    tracing::info!("Webhook server shutting down");
                })
                .await
            {
                tracing::error!("Webhook server error: {}", e);
            }
        });

        self.handle = Some(handle);
        Ok(())
    }

    /// Signal graceful shutdown and wait for the server task to finish.
    pub async fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}
