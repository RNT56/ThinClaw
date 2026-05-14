//! Per-thread turn cancellation registry.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, watch};
use uuid::Uuid;

#[derive(Debug, Default, Clone)]
pub struct TurnCancellationRegistry {
    inner: Arc<Mutex<HashMap<Uuid, watch::Sender<bool>>>>,
}

impl TurnCancellationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn begin(&self, thread_id: Uuid) {
        let (tx, _rx) = watch::channel(false);
        self.inner.lock().await.insert(thread_id, tx);
    }

    pub async fn finish(&self, thread_id: Uuid) {
        self.inner.lock().await.remove(&thread_id);
    }

    pub async fn signal(&self, thread_id: Uuid) {
        let tx = self.inner.lock().await.get(&thread_id).cloned();
        if let Some(tx) = tx {
            let _ = tx.send(true);
        }
    }

    pub async fn wait(&self, thread_id: Uuid) {
        let maybe_rx = self
            .inner
            .lock()
            .await
            .get(&thread_id)
            .map(watch::Sender::subscribe);
        let Some(mut rx) = maybe_rx else {
            std::future::pending::<()>().await;
            return;
        };

        loop {
            if *rx.borrow() {
                return;
            }
            if rx.changed().await.is_err() {
                std::future::pending::<()>().await;
                return;
            }
        }
    }

    pub async fn has_active_turn(&self, thread_id: Uuid) -> bool {
        self.inner.lock().await.contains_key(&thread_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn signal_releases_waiter() {
        let registry = TurnCancellationRegistry::new();
        let thread_id = Uuid::new_v4();
        registry.begin(thread_id).await;

        let waiting_registry = registry.clone();
        let waiter = tokio::spawn(async move {
            waiting_registry.wait(thread_id).await;
        });

        tokio::task::yield_now().await;
        registry.signal(thread_id).await;
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("waiter should release")
            .expect("waiter task should succeed");
    }

    #[tokio::test]
    async fn finish_removes_active_turn() {
        let registry = TurnCancellationRegistry::new();
        let thread_id = Uuid::new_v4();
        registry.begin(thread_id).await;
        assert!(registry.has_active_turn(thread_id).await);

        registry.finish(thread_id).await;
        assert!(!registry.has_active_turn(thread_id).await);
    }
}
