//! Per-thread turn cancellation registry.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::watch;
use uuid::Uuid;

#[derive(Debug)]
struct CancellationEntry {
    generation: u64,
    sender: watch::Sender<bool>,
}

#[derive(Debug, Clone)]
pub struct TurnCancellationRegistry {
    inner: Arc<Mutex<HashMap<Uuid, CancellationEntry>>>,
    next_generation: Arc<AtomicU64>,
}

/// Drop guard for one active cancellation registration.
///
/// Cancellation cleanup must survive early returns and panics in an agent
/// turn. The generation prevents an old guard from removing a newer
/// registration if a caller ever replaces the entry for the same thread.
#[derive(Debug)]
pub struct TurnCancellationGuard {
    registry: TurnCancellationRegistry,
    thread_id: Uuid,
    generation: u64,
}

impl Drop for TurnCancellationGuard {
    fn drop(&mut self) {
        self.registry
            .finish_generation(self.thread_id, self.generation);
    }
}

impl TurnCancellationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<Uuid, CancellationEntry>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn insert(&self, thread_id: Uuid) -> u64 {
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let (tx, _rx) = watch::channel(false);
        self.entries().insert(
            thread_id,
            CancellationEntry {
                generation,
                sender: tx,
            },
        );
        generation
    }

    fn finish_generation(&self, thread_id: Uuid, generation: u64) {
        let mut entries = self.entries();
        if entries
            .get(&thread_id)
            .is_some_and(|entry| entry.generation == generation)
        {
            entries.remove(&thread_id);
        }
    }

    pub async fn begin(&self, thread_id: Uuid) {
        self.insert(thread_id);
    }

    pub async fn begin_guard(&self, thread_id: Uuid) -> TurnCancellationGuard {
        let generation = self.insert(thread_id);
        TurnCancellationGuard {
            registry: self.clone(),
            thread_id,
            generation,
        }
    }

    pub async fn finish(&self, thread_id: Uuid) {
        self.entries().remove(&thread_id);
    }

    pub async fn signal(&self, thread_id: Uuid) {
        let tx = self
            .entries()
            .get(&thread_id)
            .map(|entry| entry.sender.clone());
        if let Some(tx) = tx {
            let _ = tx.send(true);
        }
    }

    pub async fn wait(&self, thread_id: Uuid) {
        let maybe_rx = self
            .entries()
            .get(&thread_id)
            .map(|entry| entry.sender.subscribe());
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
        self.entries().contains_key(&thread_id)
    }
}

impl Default for TurnCancellationRegistry {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            // Zero is a valid generation; wrapping after u64::MAX registrations
            // is harmless in practice and still protected by thread identity.
            next_generation: Arc::new(AtomicU64::new(0)),
        }
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

    #[tokio::test]
    async fn guard_cleans_up_and_cannot_remove_a_replacement() {
        let registry = TurnCancellationRegistry::new();
        let thread_id = Uuid::new_v4();
        let old = registry.begin_guard(thread_id).await;
        let new = registry.begin_guard(thread_id).await;

        drop(old);
        assert!(registry.has_active_turn(thread_id).await);
        drop(new);
        assert!(!registry.has_active_turn(thread_id).await);
    }
}
