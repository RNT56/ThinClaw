//! In-memory, TTL-bound multi-device presence registry.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::rate_limiter::RateLimiter;
use super::types::{
    DEFAULT_PRESENCE_TTL_SECONDS, MAX_PRESENCE_SESSIONS_PER_ACTOR, MAX_PRESENCE_TTL_SECONDS,
    MIN_PRESENCE_TTL_SECONDS, PresenceAggregate, PresenceClearResponse, PresenceEvent,
    PresenceEventCause, PresenceEventKind, PresencePublishRequest, PresencePublishResponse,
    PresenceScope, PresenceState, PresenceSurface, SseEvent,
};

const PRESENCE_SWEEP_INTERVAL: Duration = Duration::from_secs(1);
const PRESENCE_MUTATIONS_PER_MINUTE: u64 = 120;
const MAX_PRESENCE_SESSIONS_PER_PRINCIPAL: usize = 256;
const MAX_PRESENCE_SESSIONS_TOTAL: usize = 4096;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PresenceError {
    #[error(
        "presence TTL must be between {MIN_PRESENCE_TTL_SECONDS} and {MAX_PRESENCE_TTL_SECONDS} seconds"
    )]
    InvalidTtl,
    #[error("typing presence requires a thread scope")]
    TypingRequiresThread,
    #[error("presence session limit reached ({MAX_PRESENCE_SESSIONS_PER_ACTOR} per actor)")]
    SessionLimit,
    #[error(
        "presence principal session limit reached ({MAX_PRESENCE_SESSIONS_PER_PRINCIPAL} per principal)"
    )]
    PrincipalSessionLimit,
    #[error("gateway presence capacity reached ({MAX_PRESENCE_SESSIONS_TOTAL} sessions)")]
    Capacity,
    #[error("presence mutation rate limit exceeded")]
    RateLimited,
    #[error("presence session was superseded by a newer connection")]
    StaleLease,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SessionKey {
    principal_id: String,
    actor_id: String,
    session_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AggregateKey {
    principal_id: String,
    actor_id: String,
    scope: PresenceScope,
}

#[derive(Debug, Clone)]
struct SessionEntry {
    scope: PresenceScope,
    state: PresenceState,
    surface: PresenceSurface,
    updated_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    /// TTL enforcement must not depend on wall-clock adjustments.
    deadline: Instant,
    lease_id: Option<u64>,
}

struct PresenceInner {
    entries: Mutex<HashMap<SessionKey, SessionEntry>>,
    event_tx: broadcast::Sender<SseEvent>,
    sweeper_started: AtomicBool,
    mutation_limiter: RateLimiter,
    next_lease_id: AtomicU64,
}

/// Cheaply cloneable transient registry. It shares the gateway event sender so
/// TTL expiry is emitted over the same privacy-filtered SSE/WS plane as joins.
#[derive(Clone)]
pub struct PresenceRegistry {
    inner: Arc<PresenceInner>,
}

impl PresenceRegistry {
    pub fn new(event_tx: broadcast::Sender<SseEvent>) -> Self {
        Self {
            inner: Arc::new(PresenceInner {
                entries: Mutex::new(HashMap::new()),
                event_tx,
                sweeper_started: AtomicBool::new(false),
                mutation_limiter: RateLimiter::new(PRESENCE_MUTATIONS_PER_MINUTE, 60),
                next_lease_id: AtomicU64::new(1),
            }),
        }
    }

    /// Allocate a monotonically ordered WebSocket lease. When two sockets
    /// overlap, the newer connection can take over a reused session UUID and
    /// subsequent publishes from the older socket fail closed.
    pub fn open_lease(&self) -> u64 {
        self.inner.next_lease_id.fetch_add(1, Ordering::Relaxed)
    }

    pub async fn publish(
        &self,
        principal_id: &str,
        actor_id: &str,
        session_id: Uuid,
        request: PresencePublishRequest,
        lease_id: Option<u64>,
    ) -> Result<PresencePublishResponse, PresenceError> {
        let ttl = request.ttl_seconds.unwrap_or(DEFAULT_PRESENCE_TTL_SECONDS);
        if !(MIN_PRESENCE_TTL_SECONDS..=MAX_PRESENCE_TTL_SECONDS).contains(&ttl) {
            return Err(PresenceError::InvalidTtl);
        }
        let scope = request.scope();
        if request.state == PresenceState::Typing && scope == PresenceScope::Principal {
            return Err(PresenceError::TypingRequiresThread);
        }
        let rate_limit_key = format!(
            "presence:{}",
            blake3::hash(principal_id.as_bytes()).to_hex()
        );
        self.ensure_sweeper();

        let now = Utc::now();
        let monotonic_now = Instant::now();
        let expires_at = now + chrono::Duration::seconds(ttl as i64);
        let deadline = monotonic_now + Duration::from_secs(ttl);
        let key = SessionKey {
            principal_id: principal_id.to_string(),
            actor_id: actor_id.to_string(),
            session_id,
        };
        let new_group = AggregateKey {
            principal_id: principal_id.to_string(),
            actor_id: actor_id.to_string(),
            scope: scope.clone(),
        };

        let response = {
            let mut entries = self.inner.entries.lock().unwrap_or_else(|p| p.into_inner());
            let mut events = expire_locked(&mut entries, monotonic_now);
            let existing = entries.get(&key).cloned();
            if let Some(incoming) = lease_id
                && let Some(current) = existing.as_ref().and_then(|entry| entry.lease_id)
                && incoming < current
            {
                send_events(&self.inner.event_tx, events);
                return Err(PresenceError::StaleLease);
            }
            if !self.inner.mutation_limiter.check_for(&rate_limit_key) {
                send_events(&self.inner.event_tx, events);
                return Err(PresenceError::RateLimited);
            }
            if existing.is_none()
                && entries
                    .keys()
                    .filter(|candidate| {
                        candidate.principal_id == principal_id && candidate.actor_id == actor_id
                    })
                    .count()
                    >= MAX_PRESENCE_SESSIONS_PER_ACTOR
            {
                // Expiry is a global sweep. Do not lose unrelated expiry
                // transitions merely because this publisher is over limit.
                send_events(&self.inner.event_tx, events);
                return Err(PresenceError::SessionLimit);
            }
            if existing.is_none()
                && entries
                    .keys()
                    .filter(|candidate| candidate.principal_id == principal_id)
                    .count()
                    >= MAX_PRESENCE_SESSIONS_PER_PRINCIPAL
            {
                send_events(&self.inner.event_tx, events);
                return Err(PresenceError::PrincipalSessionLimit);
            }
            if existing.is_none() && entries.len() >= MAX_PRESENCE_SESSIONS_TOTAL {
                send_events(&self.inner.event_tx, events);
                return Err(PresenceError::Capacity);
            }

            let old_group = existing.as_ref().map(|entry| AggregateKey {
                principal_id: principal_id.to_string(),
                actor_id: actor_id.to_string(),
                scope: entry.scope.clone(),
            });
            let old_aggregate = aggregate_for(&entries, &new_group);
            let previous_group_aggregate = old_group
                .as_ref()
                .filter(|old| *old != &new_group)
                .and_then(|old| aggregate_for(&entries, old));

            entries.insert(
                key,
                SessionEntry {
                    scope,
                    state: request.state,
                    surface: request.surface,
                    updated_at: now,
                    expires_at,
                    deadline,
                    // A REST renewal of an existing WS-owned UUID must not
                    // erase its ordering guard; an explicit REST clear remains
                    // owner-bound and can still intentionally remove it.
                    lease_id: lease_id.or(existing.as_ref().and_then(|entry| entry.lease_id)),
                },
            );

            if let (Some(old_group), Some(before)) = (old_group.as_ref(), previous_group_aggregate)
                && old_group != &new_group
            {
                let after = aggregate_for(&entries, old_group);
                events.push(changed_event(
                    old_group,
                    Some(before),
                    after,
                    PresenceEventCause::ScopeChanged,
                ));
            }

            let aggregate = aggregate_for(&entries, &new_group)
                .expect("newly inserted presence must contribute to its aggregate");
            let aggregate_changed = old_aggregate
                .as_ref()
                .is_none_or(|old| !same_visible_state(old, &aggregate));
            if aggregate_changed || old_group.as_ref().is_some_and(|old| old != &new_group) {
                events.push(PresenceEvent {
                    event: if old_aggregate.is_some() {
                        PresenceEventKind::Updated
                    } else {
                        PresenceEventKind::Joined
                    },
                    cause: if old_group.as_ref().is_some_and(|old| old != &new_group) {
                        PresenceEventCause::ScopeChanged
                    } else {
                        PresenceEventCause::Publish
                    },
                    presence: aggregate.clone(),
                    principal_id: principal_id.to_string(),
                });
            }
            let response = PresencePublishResponse {
                presence: aggregate,
                event_emitted: aggregate_changed
                    || old_group.as_ref().is_some_and(|old| old != &new_group),
            };
            // Broadcast while the mutation-order lock is held. Broadcast send
            // is synchronous and non-blocking; this prevents a later mutation
            // from publishing ahead of this one on concurrent requests.
            send_events(&self.inner.event_tx, events);
            response
        };
        Ok(response)
    }

    pub async fn clear(
        &self,
        principal_id: &str,
        actor_id: &str,
        session_id: Uuid,
        cause: PresenceEventCause,
    ) -> PresenceClearResponse {
        self.clear_internal(principal_id, actor_id, session_id, None, cause)
            .await
    }

    /// Clear one session only if it is still owned by this WebSocket lease.
    /// A stale overlapping socket therefore cannot clear a newer reconnect.
    pub async fn clear_lease_session(
        &self,
        principal_id: &str,
        actor_id: &str,
        session_id: Uuid,
        lease_id: u64,
        cause: PresenceEventCause,
    ) -> PresenceClearResponse {
        self.clear_internal(principal_id, actor_id, session_id, Some(lease_id), cause)
            .await
    }

    async fn clear_internal(
        &self,
        principal_id: &str,
        actor_id: &str,
        session_id: Uuid,
        expected_lease: Option<u64>,
        cause: PresenceEventCause,
    ) -> PresenceClearResponse {
        let monotonic_now = Instant::now();
        let mut entries = self.inner.entries.lock().unwrap_or_else(|p| p.into_inner());
        let mut events = expire_locked(&mut entries, monotonic_now);
        let key = SessionKey {
            principal_id: principal_id.to_string(),
            actor_id: actor_id.to_string(),
            session_id,
        };
        let Some(entry) = entries.get(&key).cloned() else {
            send_events(&self.inner.event_tx, events);
            return PresenceClearResponse {
                cleared: false,
                presence: None,
            };
        };
        if expected_lease.is_some_and(|lease| entry.lease_id != Some(lease)) {
            send_events(&self.inner.event_tx, events);
            return PresenceClearResponse {
                cleared: false,
                presence: None,
            };
        }
        let group = AggregateKey {
            principal_id: principal_id.to_string(),
            actor_id: actor_id.to_string(),
            scope: entry.scope,
        };
        let before = aggregate_for(&entries, &group);
        entries.remove(&key);
        let after = aggregate_for(&entries, &group);
        events.push(changed_event(&group, before, after.clone(), cause));
        send_events(&self.inner.event_tx, events);
        PresenceClearResponse {
            cleared: true,
            presence: after,
        }
    }

    /// Clear only entries still owned by this WebSocket lease. If a reconnect
    /// reused the same session ID, its newer lease wins and the old socket's
    /// disconnect cannot erase it.
    pub async fn clear_lease(&self, principal_id: &str, actor_id: &str, lease_id: u64) {
        let monotonic_now = Instant::now();
        {
            let mut entries = self.inner.entries.lock().unwrap_or_else(|p| p.into_inner());
            let mut events = expire_locked(&mut entries, monotonic_now);
            let keys = entries
                .iter()
                .filter(|(key, entry)| {
                    key.principal_id == principal_id
                        && key.actor_id == actor_id
                        && entry.lease_id == Some(lease_id)
                })
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            let affected = keys
                .iter()
                .filter_map(|key| {
                    entries.get(key).map(|entry| AggregateKey {
                        principal_id: key.principal_id.clone(),
                        actor_id: key.actor_id.clone(),
                        scope: entry.scope.clone(),
                    })
                })
                .collect::<HashSet<_>>();
            let before = affected
                .iter()
                .filter_map(|group| {
                    aggregate_for(&entries, group).map(|value| (group.clone(), value))
                })
                .collect::<HashMap<_, _>>();
            for key in keys {
                entries.remove(&key);
            }
            for group in affected {
                events.push(changed_event(
                    &group,
                    before.get(&group).cloned(),
                    aggregate_for(&entries, &group),
                    PresenceEventCause::Disconnect,
                ));
            }
            send_events(&self.inner.event_tx, events);
        }
    }

    pub async fn snapshot(
        &self,
        principal_id: &str,
        scope: &PresenceScope,
    ) -> Vec<PresenceAggregate> {
        let monotonic_now = Instant::now();
        let mut aggregates = {
            let mut entries = self.inner.entries.lock().unwrap_or_else(|p| p.into_inner());
            let events = expire_locked(&mut entries, monotonic_now);
            let groups = entries
                .iter()
                .filter(|(key, entry)| key.principal_id == principal_id && &entry.scope == scope)
                .map(|(key, entry)| AggregateKey {
                    principal_id: key.principal_id.clone(),
                    actor_id: key.actor_id.clone(),
                    scope: entry.scope.clone(),
                })
                .collect::<HashSet<_>>();
            let aggregates = groups
                .iter()
                .filter_map(|group| aggregate_for(&entries, group))
                .collect::<Vec<_>>();
            send_events(&self.inner.event_tx, events);
            aggregates
        };
        aggregates.sort_by(|left, right| left.actor_id.cmp(&right.actor_id));
        aggregates
    }

    fn ensure_sweeper(&self) {
        if self
            .inner
            .sweeper_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let inner = Arc::downgrade(&self.inner);
        tokio::spawn(async move { sweep_loop(inner).await });
    }
}

async fn sweep_loop(inner: Weak<PresenceInner>) {
    let mut interval = tokio::time::interval(PRESENCE_SWEEP_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await;
    loop {
        interval.tick().await;
        let Some(inner) = inner.upgrade() else { return };
        let mut entries = inner.entries.lock().unwrap_or_else(|p| p.into_inner());
        let events = expire_locked(&mut entries, Instant::now());
        send_events(&inner.event_tx, events);
    }
}

fn send_events(event_tx: &broadcast::Sender<SseEvent>, events: Vec<PresenceEvent>) {
    for event in events {
        let _ = event_tx.send(SseEvent::Presence { event });
    }
}

fn expire_locked(
    entries: &mut HashMap<SessionKey, SessionEntry>,
    now: Instant,
) -> Vec<PresenceEvent> {
    let expired = entries
        .iter()
        .filter(|(_, entry)| entry.deadline <= now)
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    if expired.is_empty() {
        return Vec::new();
    }
    let affected = expired
        .iter()
        .filter_map(|key| {
            entries.get(key).map(|entry| AggregateKey {
                principal_id: key.principal_id.clone(),
                actor_id: key.actor_id.clone(),
                scope: entry.scope.clone(),
            })
        })
        .collect::<HashSet<_>>();
    let before = affected
        .iter()
        .filter_map(|group| aggregate_for(entries, group).map(|value| (group.clone(), value)))
        .collect::<HashMap<_, _>>();
    for key in expired {
        entries.remove(&key);
    }
    affected
        .into_iter()
        .map(|group| {
            changed_event(
                &group,
                before.get(&group).cloned(),
                aggregate_for(entries, &group),
                PresenceEventCause::Ttl,
            )
        })
        .collect()
}

fn changed_event(
    group: &AggregateKey,
    before: Option<PresenceAggregate>,
    after: Option<PresenceAggregate>,
    cause: PresenceEventCause,
) -> PresenceEvent {
    match after {
        Some(presence) => PresenceEvent {
            event: if before.is_some() {
                PresenceEventKind::Updated
            } else {
                PresenceEventKind::Joined
            },
            cause,
            presence,
            principal_id: group.principal_id.clone(),
        },
        None => PresenceEvent {
            event: PresenceEventKind::Expired,
            cause,
            presence: before.expect("a removed aggregate must have existed"),
            principal_id: group.principal_id.clone(),
        },
    }
}

fn aggregate_for(
    entries: &HashMap<SessionKey, SessionEntry>,
    group: &AggregateKey,
) -> Option<PresenceAggregate> {
    let matching = entries
        .iter()
        .filter(|(key, entry)| {
            key.principal_id == group.principal_id
                && key.actor_id == group.actor_id
                && entry.scope == group.scope
        })
        .map(|(_, entry)| entry)
        .collect::<Vec<_>>();
    let first = matching.first()?;
    let state = matching
        .iter()
        .map(|entry| entry.state)
        .max_by_key(|state| state.priority())
        .unwrap_or(first.state);
    let mut surfaces = matching
        .iter()
        .map(|entry| entry.surface)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    surfaces.sort();
    let updated_at = matching
        .iter()
        .map(|entry| entry.updated_at)
        .max()
        .unwrap_or(first.updated_at);
    let expires_at = matching
        .iter()
        .map(|entry| entry.expires_at)
        .min()
        .unwrap_or(first.expires_at);
    Some(PresenceAggregate {
        actor_id: group.actor_id.clone(),
        scope: group.scope.clone(),
        state,
        surfaces,
        session_count: matching.len(),
        updated_at: updated_at.to_rfc3339(),
        expires_at: expires_at.to_rfc3339(),
    })
}

fn same_visible_state(left: &PresenceAggregate, right: &PresenceAggregate) -> bool {
    left.actor_id == right.actor_id
        && left.scope == right.scope
        && left.state == right.state
        && left.surfaces == right.surfaces
        && left.session_count == right.session_count
        && left.updated_at == right.updated_at
        && left.expires_at == right.expires_at
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> (PresenceRegistry, broadcast::Receiver<SseEvent>) {
        let (tx, rx) = broadcast::channel(32);
        (PresenceRegistry::new(tx), rx)
    }

    fn request(state: PresenceState, surface: PresenceSurface) -> PresencePublishRequest {
        PresencePublishRequest {
            state,
            surface,
            thread_id: None,
            ttl_seconds: Some(30),
        }
    }

    #[tokio::test]
    async fn aggregates_duplicate_devices_and_emits_fresh_deadlines() {
        let (registry, mut events) = registry();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        registry
            .publish(
                "p1",
                "a1",
                first,
                request(PresenceState::Online, PresenceSurface::Web),
                None,
            )
            .await
            .unwrap();
        let mut refreshed_request = request(PresenceState::Online, PresenceSurface::Web);
        refreshed_request.ttl_seconds = Some(31);
        let refresh = registry
            .publish("p1", "a1", first, refreshed_request, None)
            .await
            .unwrap();
        assert!(refresh.event_emitted);
        registry
            .publish(
                "p1",
                "a1",
                second,
                request(PresenceState::Away, PresenceSurface::Ios),
                None,
            )
            .await
            .unwrap();

        let snapshot = registry.snapshot("p1", &PresenceScope::Principal).await;
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].session_count, 2);
        assert_eq!(snapshot[0].state, PresenceState::Online);
        assert_eq!(
            snapshot[0].surfaces,
            [PresenceSurface::Web, PresenceSurface::Ios]
        );
        assert!(matches!(
            events.recv().await.unwrap(),
            SseEvent::Presence { .. }
        ));
        assert!(matches!(
            events.recv().await.unwrap(),
            SseEvent::Presence { .. }
        ));
        assert!(matches!(
            events.recv().await.unwrap(),
            SseEvent::Presence { .. }
        ));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn owners_and_scopes_are_isolated() {
        let (registry, _events) = registry();
        let thread = Uuid::new_v4();
        let mut scoped = request(PresenceState::Typing, PresenceSurface::Desktop);
        scoped.thread_id = Some(thread);
        registry
            .publish("p1", "a1", Uuid::new_v4(), scoped, None)
            .await
            .unwrap();
        registry
            .publish(
                "p2",
                "a2",
                Uuid::new_v4(),
                request(PresenceState::Online, PresenceSurface::Web),
                None,
            )
            .await
            .unwrap();

        assert!(
            registry
                .snapshot("p1", &PresenceScope::Principal)
                .await
                .is_empty()
        );
        assert_eq!(
            registry
                .snapshot("p1", &PresenceScope::Thread { thread_id: thread })
                .await
                .len(),
            1
        );
        assert_eq!(
            registry
                .snapshot("p2", &PresenceScope::Principal)
                .await
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn stale_websocket_disconnect_cannot_clear_reconnected_session() {
        let (registry, _events) = registry();
        let session = Uuid::new_v4();
        let stale_lease = registry.open_lease();
        let current_lease = registry.open_lease();
        registry
            .publish(
                "p1",
                "a1",
                session,
                request(PresenceState::Online, PresenceSurface::Desktop),
                Some(stale_lease),
            )
            .await
            .unwrap();
        registry
            .publish(
                "p1",
                "a1",
                session,
                request(PresenceState::Online, PresenceSurface::Desktop),
                Some(current_lease),
            )
            .await
            .unwrap();
        assert_eq!(
            registry
                .publish(
                    "p1",
                    "a1",
                    session,
                    request(PresenceState::Away, PresenceSurface::Desktop),
                    Some(stale_lease),
                )
                .await,
            Err(PresenceError::StaleLease)
        );
        registry.clear_lease("p1", "a1", stale_lease).await;
        assert_eq!(
            registry
                .snapshot("p1", &PresenceScope::Principal)
                .await
                .len(),
            1
        );
        assert!(
            !registry
                .clear_lease_session("p1", "a1", session, stale_lease, PresenceEventCause::Clear,)
                .await
                .cleared
        );
        registry.clear_lease("p1", "a1", current_lease).await;
        assert!(
            registry
                .snapshot("p1", &PresenceScope::Principal)
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn clear_is_owner_bound_and_idempotent() {
        let (registry, _events) = registry();
        let session = Uuid::new_v4();
        registry
            .publish(
                "p1",
                "a1",
                session,
                request(PresenceState::Online, PresenceSurface::Cli),
                None,
            )
            .await
            .unwrap();

        let foreign = registry
            .clear("p1", "a2", session, PresenceEventCause::Clear)
            .await;
        assert!(!foreign.cleared);
        assert_eq!(
            registry
                .snapshot("p1", &PresenceScope::Principal)
                .await
                .len(),
            1
        );

        let cleared = registry
            .clear("p1", "a1", session, PresenceEventCause::Clear)
            .await;
        assert!(cleared.cleared);
        assert!(cleared.presence.is_none());
        assert!(
            !registry
                .clear("p1", "a1", session, PresenceEventCause::Clear)
                .await
                .cleared
        );
    }

    #[tokio::test]
    async fn validates_ttl_and_typing_scope() {
        let (registry, _events) = registry();
        let mut invalid = request(PresenceState::Online, PresenceSurface::Cli);
        invalid.ttl_seconds = Some(MAX_PRESENCE_TTL_SECONDS + 1);
        assert_eq!(
            registry
                .publish("p", "a", Uuid::new_v4(), invalid, None)
                .await,
            Err(PresenceError::InvalidTtl)
        );
        assert_eq!(
            registry
                .publish(
                    "p",
                    "a",
                    Uuid::new_v4(),
                    request(PresenceState::Typing, PresenceSurface::Cli),
                    None,
                )
                .await,
            Err(PresenceError::TypingRequiresThread)
        );
    }

    #[tokio::test]
    async fn session_limit_does_not_drop_other_actor_expiry_event() {
        let (registry, mut events) = registry();
        let monotonic_now = Instant::now();
        let wall_now = Utc::now();
        {
            let mut entries = registry.inner.entries.lock().unwrap();
            for _ in 0..MAX_PRESENCE_SESSIONS_PER_ACTOR {
                entries.insert(
                    SessionKey {
                        principal_id: "p".to_string(),
                        actor_id: "at-limit".to_string(),
                        session_id: Uuid::new_v4(),
                    },
                    SessionEntry {
                        scope: PresenceScope::Principal,
                        state: PresenceState::Online,
                        surface: PresenceSurface::Cli,
                        updated_at: wall_now,
                        expires_at: wall_now + chrono::Duration::seconds(30),
                        deadline: monotonic_now + Duration::from_secs(30),
                        lease_id: None,
                    },
                );
            }
            entries.insert(
                SessionKey {
                    principal_id: "p".to_string(),
                    actor_id: "expired".to_string(),
                    session_id: Uuid::new_v4(),
                },
                SessionEntry {
                    scope: PresenceScope::Principal,
                    state: PresenceState::Away,
                    surface: PresenceSurface::Web,
                    updated_at: wall_now,
                    // Intentionally contradict the monotonic deadline to prove
                    // wall-clock jumps cannot extend the lease.
                    expires_at: wall_now + chrono::Duration::hours(1),
                    deadline: monotonic_now,
                    lease_id: None,
                },
            );
        }

        assert_eq!(
            registry
                .publish(
                    "p",
                    "at-limit",
                    Uuid::new_v4(),
                    request(PresenceState::Online, PresenceSurface::Cli),
                    None,
                )
                .await,
            Err(PresenceError::SessionLimit)
        );
        let event = events.recv().await.unwrap();
        assert!(matches!(
            event,
            SseEvent::Presence {
                event: PresenceEvent {
                    event: PresenceEventKind::Expired,
                    ..
                }
            }
        ));
    }

    #[tokio::test]
    async fn mutation_rate_limit_bounds_shared_event_channel_pressure() {
        let (registry, _events) = registry();
        let session = Uuid::new_v4();
        for _ in 0..PRESENCE_MUTATIONS_PER_MINUTE {
            registry
                .publish(
                    "rate-principal",
                    "rate-actor",
                    session,
                    request(PresenceState::Online, PresenceSurface::Cli),
                    None,
                )
                .await
                .unwrap();
        }
        assert_eq!(
            registry
                .publish(
                    "rate-principal",
                    "rate-actor",
                    session,
                    request(PresenceState::Busy, PresenceSurface::Cli),
                    None,
                )
                .await,
            Err(PresenceError::RateLimited)
        );
    }

    #[tokio::test]
    async fn concurrent_mutations_broadcast_in_committed_order() {
        let (event_tx, mut events) = broadcast::channel(128);
        let registry = PresenceRegistry::new(event_tx);
        let session = Uuid::new_v4();
        let barrier = Arc::new(tokio::sync::Barrier::new(61));
        let mut tasks = Vec::new();
        for index in 0..60 {
            let registry = registry.clone();
            let barrier = Arc::clone(&barrier);
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                if index % 2 == 0 {
                    registry
                        .publish(
                            "ordered-principal",
                            "ordered-actor",
                            session,
                            request(PresenceState::Online, PresenceSurface::Web),
                            None,
                        )
                        .await
                        .unwrap();
                } else {
                    registry
                        .clear(
                            "ordered-principal",
                            "ordered-actor",
                            session,
                            PresenceEventCause::Clear,
                        )
                        .await;
                }
            }));
        }
        barrier.wait().await;
        for task in tasks {
            task.await.unwrap();
        }

        // Replaying the channel in receive order must reproduce authoritative
        // registry state. Sending after unlocking makes this assertion race:
        // a later reconnect can be delivered before an earlier clear.
        let mut replayed = None;
        while let Ok(event) = events.try_recv() {
            let SseEvent::Presence { event } = event else {
                continue;
            };
            replayed = match event.event {
                PresenceEventKind::Expired => None,
                PresenceEventKind::Joined | PresenceEventKind::Updated => Some(event.presence),
            };
        }
        let snapshot = registry
            .snapshot("ordered-principal", &PresenceScope::Principal)
            .await;
        assert_eq!(replayed.as_ref(), snapshot.first());
    }

    #[test]
    fn expiration_recomputes_then_expires_multi_device_aggregate() {
        let (tx, _rx) = broadcast::channel(8);
        let registry = PresenceRegistry::new(tx);
        let now = Utc::now();
        let monotonic_now = Instant::now();
        let group = AggregateKey {
            principal_id: "p".to_string(),
            actor_id: "a".to_string(),
            scope: PresenceScope::Principal,
        };
        let mut entries = registry.inner.entries.lock().unwrap();
        for (seconds, surface) in [(0, PresenceSurface::Web), (30, PresenceSurface::Ios)] {
            entries.insert(
                SessionKey {
                    principal_id: "p".to_string(),
                    actor_id: "a".to_string(),
                    session_id: Uuid::new_v4(),
                },
                SessionEntry {
                    scope: PresenceScope::Principal,
                    state: PresenceState::Online,
                    surface,
                    updated_at: now,
                    expires_at: now + chrono::Duration::seconds(seconds),
                    deadline: monotonic_now + Duration::from_secs(seconds as u64),
                    lease_id: None,
                },
            );
        }
        assert_eq!(
            aggregate_for(&entries, &group).unwrap().expires_at,
            now.to_rfc3339()
        );
        let first = expire_locked(&mut entries, monotonic_now);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].event, PresenceEventKind::Updated);
        assert_eq!(first[0].presence.session_count, 1);
        let last = expire_locked(&mut entries, monotonic_now + Duration::from_secs(31));
        assert_eq!(last.len(), 1);
        assert_eq!(last[0].event, PresenceEventKind::Expired);
        assert!(aggregate_for(&entries, &group).is_none());
    }

    #[test]
    fn serialized_event_does_not_leak_principal_or_session_id() {
        let event = PresenceEvent {
            event: PresenceEventKind::Joined,
            cause: PresenceEventCause::Publish,
            presence: PresenceAggregate {
                actor_id: "actor-a".to_string(),
                scope: PresenceScope::Principal,
                state: PresenceState::Online,
                surfaces: vec![PresenceSurface::Web],
                session_count: 1,
                updated_at: "2026-08-13T00:00:00Z".to_string(),
                expires_at: "2026-08-13T00:00:45Z".to_string(),
            },
            principal_id: "secret-principal".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("secret-principal"));
        assert!(!json.contains("session_id"));

        let envelope = serde_json::to_value(SseEvent::Presence { event }).unwrap();
        assert_eq!(envelope["type"], "presence");
        assert_eq!(envelope["event"], "joined");
        assert_eq!(envelope["presence"]["actor_id"], "actor-a");
        assert!(envelope.get("principal_id").is_none());
    }
}
