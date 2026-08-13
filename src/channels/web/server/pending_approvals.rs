//! Durable ownership-aware pending-approval projection for the web gateway.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use thinclaw_gateway::web::identity::{GatewayRequestIdentity, valid_gateway_identity_component};
use thinclaw_gateway::web::types::PendingApprovalEntry;
use uuid::Uuid;

use super::GatewayState;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PendingApprovalOwner {
    pub principal_id: String,
    pub actor_id: String,
}

impl PendingApprovalOwner {
    pub fn new(principal_id: impl Into<String>, actor_id: impl Into<String>) -> Self {
        Self {
            principal_id: principal_id.into(),
            actor_id: actor_id.into(),
        }
    }

    fn matches(&self, identity: &GatewayRequestIdentity) -> bool {
        self.principal_id == identity.principal_id && self.actor_id == identity.actor_id
    }

    fn from_resolved(identity: &thinclaw_identity::ResolvedIdentity) -> Option<Self> {
        (valid_gateway_identity_component(&identity.principal_id)
            && valid_gateway_identity_component(&identity.actor_id))
        .then(|| Self::new(identity.principal_id.clone(), identity.actor_id.clone()))
    }
}

/// On-disk compatibility wrapper. `flatten` preserves the original entry
/// shape, so previous installations deserialize with `owner = None`. Such
/// legacy records remain hidden and non-actionable (fail closed).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct PendingApprovalRecord {
    #[serde(flatten)]
    pub(super) entry: PendingApprovalEntry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) owner: Option<PendingApprovalOwner>,
}

/// Durable registry of pending approval requests. Mutations are published to
/// disk before becoming visible in memory, so an I/O failure cannot produce an
/// approval that is only transiently tracked.
pub struct PendingApprovalsStore {
    pub(super) entries: Mutex<HashMap<String, PendingApprovalRecord>>,
    path: Option<PathBuf>,
    load_error: Option<String>,
}

pub type PendingApprovalsCache = Arc<PendingApprovalsStore>;

impl PendingApprovalsStore {
    pub fn persisted_default() -> Self {
        Self::with_path(crate::platform::resolve_data_dir("mobile").join("pending-approvals.json"))
    }

    pub fn in_memory() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            path: None,
            load_error: None,
        }
    }

    pub(super) fn with_path(path: PathBuf) -> Self {
        const MAX_PENDING_APPROVAL_STORE_BYTES: u64 = 8 * 1024 * 1024;
        let (entries, load_error) = match crate::platform::read_regular_file_bounded_single_link(
            &path,
            MAX_PENDING_APPROVAL_STORE_BYTES,
        ) {
            Ok(data) => match crate::platform::harden_private_regular_file(&path) {
                Ok(()) => match serde_json::from_slice(&data) {
                    Ok(entries) => (entries, None),
                    Err(error) => (
                        HashMap::new(),
                        Some(format!("invalid pending approval store: {error}")),
                    ),
                },
                Err(error) => (
                    HashMap::new(),
                    Some(format!("failed to harden pending approval store: {error}")),
                ),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (HashMap::new(), None),
            Err(error) => (
                HashMap::new(),
                Some(format!("failed to read pending approval store: {error}")),
            ),
        };
        Self {
            entries: Mutex::new(entries),
            path: Some(path),
            load_error,
        }
    }

    pub fn ensure_ready(&self) -> std::io::Result<()> {
        match &self.load_error {
            Some(error) => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                error.clone(),
            )),
            None => Ok(()),
        }
    }

    fn locked(
        &self,
    ) -> std::io::Result<std::sync::MutexGuard<'_, HashMap<String, PendingApprovalRecord>>> {
        self.ensure_ready()?;
        self.entries
            .lock()
            .map_err(|_| std::io::Error::other("pending approval store lock poisoned"))
    }

    fn mutate(
        &self,
        operation: impl FnOnce(&mut HashMap<String, PendingApprovalRecord>),
    ) -> std::io::Result<()> {
        let mut current = self.locked()?;
        let mut next = current.clone();
        operation(&mut next);
        self.persist(&next)?;
        *current = next;
        Ok(())
    }

    pub(crate) fn upsert(
        &self,
        mut entry: PendingApprovalEntry,
        owner: PendingApprovalOwner,
    ) -> std::io::Result<()> {
        self.mutate(|entries| {
            if let Some(existing) = entries.get(&entry.request_id) {
                entry.created_at.clone_from(&existing.entry.created_at);
            }
            entries.insert(
                entry.request_id.clone(),
                PendingApprovalRecord {
                    entry,
                    owner: Some(owner),
                },
            );
        })
    }

    pub fn entries_for(
        &self,
        identity: &GatewayRequestIdentity,
    ) -> std::io::Result<Vec<PendingApprovalEntry>> {
        let entries = self.locked()?;
        Ok(entries
            .values()
            .filter(|record| {
                record.owner.as_ref().is_some_and(|owner| {
                    identity.is_legacy_primary_bearer() || owner.matches(identity)
                })
            })
            .map(|record| record.entry.clone())
            .collect())
    }

    pub(crate) fn entry_for(
        &self,
        request_id: &str,
        identity: &GatewayRequestIdentity,
    ) -> std::io::Result<Option<(PendingApprovalEntry, PendingApprovalOwner)>> {
        let entries = self.locked()?;
        Ok(entries.get(request_id).and_then(|record| {
            record.owner.as_ref().and_then(|owner| {
                (identity.is_legacy_primary_bearer() || owner.matches(identity))
                    .then(|| (record.entry.clone(), owner.clone()))
            })
        }))
    }

    pub fn remove_authorized(
        &self,
        request_id: &str,
        identity: &GatewayRequestIdentity,
    ) -> std::io::Result<bool> {
        let mut current = self.locked()?;
        let authorized = current.get(request_id).is_some_and(|record| {
            record
                .owner
                .as_ref()
                .is_some_and(|owner| identity.is_legacy_primary_bearer() || owner.matches(identity))
        });
        if !authorized {
            return Ok(false);
        }
        let mut next = current.clone();
        next.remove(request_id);
        self.persist(&next)?;
        *current = next;
        Ok(true)
    }

    pub fn remove_for_thread(&self, thread_id: &str) -> std::io::Result<()> {
        self.mutate(|entries| {
            entries.retain(|_, record| record.entry.thread_id.as_deref() != Some(thread_id));
        })
    }

    fn entries_all(&self) -> std::io::Result<Vec<PendingApprovalEntry>> {
        Ok(self
            .locked()?
            .values()
            .map(|record| record.entry.clone())
            .collect())
    }

    fn reconcile_records(
        &self,
        request_ids: &[String],
        recovered_owners: &[(String, PendingApprovalOwner)],
    ) -> std::io::Result<()> {
        self.mutate(|entries| {
            for request_id in request_ids {
                entries.remove(request_id);
            }
            for (request_id, owner) in recovered_owners {
                if let Some(record) = entries.get_mut(request_id)
                    && record.owner.is_none()
                {
                    record.owner = Some(owner.clone());
                }
            }
        })
    }

    fn persist(&self, entries: &HashMap<String, PendingApprovalRecord>) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        persist_pending_approvals(path, entries)
    }

    #[cfg(test)]
    pub(crate) fn contains(&self, request_id: &str) -> bool {
        self.locked()
            .is_ok_and(|entries| entries.contains_key(request_id))
    }

    #[cfg(test)]
    pub(super) fn ownerless_count(&self) -> usize {
        self.locked()
            .map(|entries| {
                entries
                    .values()
                    .filter(|record| record.owner.is_none())
                    .count()
            })
            .unwrap_or_default()
    }
}

fn persist_pending_approvals(
    path: &Path,
    entries: &HashMap<String, PendingApprovalRecord>,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        crate::platform::ensure_private_directory(parent)?;
    }
    let data = serde_json::to_vec(entries).map_err(std::io::Error::other)?;
    crate::platform::write_private_file_atomic(path, &data, true)
}

/// Reconcile the durable projection with the authoritative thread runtime.
/// Unknown state is preserved; only a definitive runtime mismatch removes a
/// record, so a long-running approval cannot disappear because of a guessed
/// wall-clock expiry.
pub(crate) async fn reconcile_pending_approvals(state: &GatewayState) {
    let entries: Vec<PendingApprovalEntry> = match state.pending_approvals.entries_all() {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mut resolved = Vec::new();
    let mut recovered_owners = Vec::new();

    for entry in entries {
        let Some(thread_id) = entry
            .thread_id
            .as_deref()
            .and_then(|value| Uuid::parse_str(value).ok())
        else {
            continue;
        };

        let in_memory = if let Some(manager) = &state.session_manager {
            if let Some(session) = manager.session_for_thread(thread_id).await {
                let session = session.lock().await;
                Some(
                    match session
                        .threads
                        .get(&thread_id)
                        .and_then(|thread| thread.pending_approval.as_ref())
                    {
                        Some(pending) if pending.request_id.to_string() == entry.request_id => (
                            true,
                            pending
                                .requesting_identity
                                .as_ref()
                                .and_then(PendingApprovalOwner::from_resolved),
                        ),
                        _ => (false, None),
                    },
                )
            } else {
                None
            }
        } else {
            None
        };

        let runtime_state = match in_memory {
            Some(value) => Some(value),
            None => {
                if let Some(store) = &state.store {
                    match crate::agent::load_thread_runtime(store, thread_id).await {
                        Ok(Some(runtime)) => Some(match runtime.pending_approval {
                            Some(pending) if pending.request_id.to_string() == entry.request_id => {
                                (
                                    true,
                                    pending
                                        .requesting_identity
                                        .as_ref()
                                        .and_then(PendingApprovalOwner::from_resolved),
                                )
                            }
                            _ => (false, None),
                        }),
                        Ok(None) => None,
                        Err(error) => {
                            tracing::warn!(
                                thread_id = %thread_id,
                                %error,
                                "failed to reconcile pending approval runtime"
                            );
                            None
                        }
                    }
                } else {
                    None
                }
            }
        };

        match runtime_state {
            Some((false, _)) => resolved.push(entry.request_id),
            Some((true, Some(owner))) => recovered_owners.push((entry.request_id, owner)),
            _ => {}
        }
    }

    if (!resolved.is_empty() || !recovered_owners.is_empty())
        && let Err(error) = state
            .pending_approvals
            .reconcile_records(&resolved, &recovered_owners)
    {
        tracing::error!(%error, "failed to durably reconcile pending approvals");
    }
}
