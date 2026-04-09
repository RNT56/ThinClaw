//! DM pairing CLI commands.
//!
//! Manage pairing requests for channels (Telegram, Slack, etc.).

use std::sync::Arc;

use async_trait::async_trait;
use clap::Subcommand;

use crate::db::{Database, IdentityRegistryStore, IdentityStore};
use crate::error::DatabaseError;
use crate::identity::{
    ActorEndpointRecord, ActorEndpointRef, ActorRecord, EndpointApprovalStatus,
    NewActorEndpointRecord, NewActorRecord,
};
use crate::pairing::{PairingRequest, PairingStore};

const DEFAULT_PRINCIPAL_ID: &str = "default";

#[async_trait]
trait PairingIdentityDb: Send + Sync {
    async fn get_actor(&self, actor_id: &str) -> Result<Option<ActorRecord>, DatabaseError>;
    async fn create_actor(&self, actor: &NewActorRecord) -> Result<ActorRecord, DatabaseError>;
    async fn delete_actor(&self, actor_id: &str) -> Result<bool, DatabaseError>;
    async fn rename_actor(&self, actor_id: &str, display_name: &str) -> Result<(), DatabaseError>;
    async fn set_actor_preferred_delivery_endpoint(
        &self,
        actor_id: &str,
        endpoint: Option<&ActorEndpointRef>,
    ) -> Result<(), DatabaseError>;
    async fn upsert_actor_endpoint(
        &self,
        record: &NewActorEndpointRecord,
    ) -> Result<ActorEndpointRecord, DatabaseError>;
    async fn get_actor_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<Option<ActorEndpointRecord>, DatabaseError>;
    async fn unlink_actor_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<bool, DatabaseError>;
}

#[async_trait]
impl<T> PairingIdentityDb for T
where
    T: Database + ?Sized,
{
    async fn get_actor(&self, actor_id: &str) -> Result<Option<ActorRecord>, DatabaseError> {
        IdentityStore::get_actor(self, actor_id).await
    }

    async fn create_actor(&self, actor: &NewActorRecord) -> Result<ActorRecord, DatabaseError> {
        IdentityRegistryStore::create_actor(self, actor).await
    }

    async fn delete_actor(&self, actor_id: &str) -> Result<bool, DatabaseError> {
        let actor_uuid = uuid::Uuid::parse_str(actor_id)
            .map_err(|e| DatabaseError::Pool(format!("invalid actor_id: {e}")))?;
        IdentityRegistryStore::delete_actor(self, actor_uuid).await
    }

    async fn rename_actor(&self, actor_id: &str, display_name: &str) -> Result<(), DatabaseError> {
        IdentityStore::rename_actor(self, actor_id, display_name).await
    }

    async fn set_actor_preferred_delivery_endpoint(
        &self,
        actor_id: &str,
        endpoint: Option<&ActorEndpointRef>,
    ) -> Result<(), DatabaseError> {
        let actor_uuid = uuid::Uuid::parse_str(actor_id)
            .map_err(|e| DatabaseError::Pool(format!("invalid actor_id: {e}")))?;
        IdentityRegistryStore::set_actor_preferred_delivery_endpoint(self, actor_uuid, endpoint)
            .await
    }

    async fn upsert_actor_endpoint(
        &self,
        record: &NewActorEndpointRecord,
    ) -> Result<ActorEndpointRecord, DatabaseError> {
        IdentityRegistryStore::upsert_actor_endpoint(self, record).await
    }

    async fn get_actor_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<Option<ActorEndpointRecord>, DatabaseError> {
        IdentityRegistryStore::get_actor_endpoint(self, channel, external_user_id).await
    }

    async fn unlink_actor_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<bool, DatabaseError> {
        IdentityStore::unlink_actor_endpoint(self, channel, external_user_id).await
    }
}

/// Pairing subcommands.
#[derive(Subcommand, Debug, Clone)]
pub enum PairingCommand {
    /// List pending pairing requests
    List {
        /// Channel name (e.g., telegram, slack)
        #[arg(required = true)]
        channel: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Approve a pairing request by code
    Approve {
        /// Channel name (e.g., telegram, slack)
        #[arg(required = true)]
        channel: String,

        /// Pairing code (e.g., ABC12345)
        #[arg(required = true)]
        code: String,

        /// Existing actor ID to link this endpoint to
        #[arg(long)]
        actor: Option<String>,

        /// Create a new actor with this display name and link the endpoint
        #[arg(long)]
        name: Option<String>,
    },

    /// Block a sender on a channel (blocklist takes precedence over allowlist)
    Block {
        /// Channel name (e.g., telegram, slack)
        #[arg(required = true)]
        channel: String,

        /// Sender ID or username to block
        #[arg(required = true)]
        sender: String,
    },

    /// Unblock a sender on a channel
    Unblock {
        /// Channel name (e.g., telegram, slack)
        #[arg(required = true)]
        channel: String,

        /// Sender ID or username to unblock
        #[arg(required = true)]
        sender: String,
    },

    /// List blocked senders on a channel
    Blocked {
        /// Channel name (e.g., telegram, slack)
        #[arg(required = true)]
        channel: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Run pairing CLI command.
pub async fn run_pairing_command(cmd: PairingCommand) -> Result<(), String> {
    let db = if matches!(
        &cmd,
        PairingCommand::Approve { actor: Some(_), .. }
            | PairingCommand::Approve { name: Some(_), .. }
    ) {
        Some(connect_db().await?)
    } else {
        None
    };

    run_pairing_command_with_store_and_db(&PairingStore::new(), db, cmd).await
}

/// Run pairing CLI command with a given store (for testing).
pub fn run_pairing_command_with_store(
    store: &PairingStore,
    cmd: PairingCommand,
) -> Result<(), String> {
    match cmd {
        PairingCommand::List { channel, json } => run_list(store, &channel, json),
        PairingCommand::Approve {
            channel,
            code,
            actor,
            name,
        } => {
            if actor.is_some() || name.is_some() {
                return Err(
                    "Identity linking requires a database-backed run. Use `thinclaw pairing approve ...` directly."
                        .to_string(),
                );
            }
            run_approve(store, &channel, &code)
        }
        PairingCommand::Block { channel, sender } => run_block(store, &channel, &sender),
        PairingCommand::Unblock { channel, sender } => run_unblock(store, &channel, &sender),
        PairingCommand::Blocked { channel, json } => run_blocked(store, &channel, json),
    }
}

async fn run_pairing_command_with_store_and_db(
    store: &PairingStore,
    db: Option<Arc<dyn Database>>,
    cmd: PairingCommand,
) -> Result<(), String> {
    match cmd {
        PairingCommand::List { channel, json } => run_list(store, &channel, json),
        PairingCommand::Approve {
            channel,
            code,
            actor,
            name,
        } => run_approve_with_identity(store, db.as_deref(), &channel, &code, actor, name).await,
        PairingCommand::Block { channel, sender } => run_block(store, &channel, &sender),
        PairingCommand::Unblock { channel, sender } => run_unblock(store, &channel, &sender),
        PairingCommand::Blocked { channel, json } => run_blocked(store, &channel, json),
    }
}

fn run_list(store: &PairingStore, channel: &str, json: bool) -> Result<(), String> {
    let requests = store.list_pending(channel).map_err(|e| e.to_string())?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&requests).map_err(|e| e.to_string())?
        );
        return Ok(());
    }

    if requests.is_empty() {
        println!("No pending {} pairing requests.", channel);
        return Ok(());
    }

    println!("Pairing requests ({}):", requests.len());
    for r in &requests {
        let meta = r
            .meta
            .as_ref()
            .and_then(|m| m.as_object())
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| format!("{}={}", k, s)))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        println!("  {}  {}  {}  {}", r.code, r.id, meta, r.created_at);
    }

    Ok(())
}

fn run_approve(store: &PairingStore, channel: &str, code: &str) -> Result<(), String> {
    match store.approve(channel, code) {
        Ok(Some(entry)) => {
            println!("Approved {} sender {}.", channel, entry.id);
            Ok(())
        }
        Ok(None) => Err(format!(
            "No pending pairing request found for code: {}",
            code
        )),
        Err(crate::pairing::PairingStoreError::ApproveRateLimited) => Err(
            "Too many failed approve attempts. Wait a few minutes before trying again.".to_string(),
        ),
        Err(e) => Err(e.to_string()),
    }
}

async fn run_approve_with_identity<T>(
    store: &PairingStore,
    db: Option<&T>,
    channel: &str,
    code: &str,
    actor_id: Option<String>,
    display_name: Option<String>,
) -> Result<(), String>
where
    T: PairingIdentityDb + ?Sized,
{
    if actor_id.is_none() && display_name.is_none() {
        return run_approve(store, channel, code);
    }

    let db =
        db.ok_or_else(|| "Identity linking requires a configured database backend.".to_string())?;

    let pending_entry = store
        .find_pending_by_code(channel, code)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("No pending pairing request found for code: {}", code))?;

    let endpoint_id = pending_entry
        .meta
        .as_ref()
        .and_then(|meta| meta.get("stable_sender_id"))
        .and_then(|value| value.as_str())
        .unwrap_or(&pending_entry.id)
        .to_string();
    let endpoint_ref = ActorEndpointRef::new(channel, &endpoint_id);
    let prior_endpoint = db
        .get_actor_endpoint(channel, &endpoint_id)
        .await
        .map_err(|e| e.to_string())?;

    let requested_name = display_name.and_then(|name| {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });

    let mut actor_created_for_link = None;
    let mut actor_renamed_from = None;
    let mut target_actor_id_for_rollback = None;
    let mut prior_preferred_endpoint = None;
    let (actor, rename_after_link, principal_id_for_create) = if let Some(actor_id) = actor_id {
        let actor = db
            .get_actor(&actor_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Actor not found: {}", actor_id))?;
        target_actor_id_for_rollback = Some(actor.actor_id.to_string());
        prior_preferred_endpoint = actor.preferred_delivery_endpoint.clone();
        let rename_after_link = requested_name.filter(|name| name != &actor.display_name);
        (actor, rename_after_link, None)
    } else {
        let name =
            requested_name.ok_or_else(|| "Provide `--name` to create a new actor.".to_string())?;
        (
            // Created after channel approval succeeds so we can roll back cleanly.
            placeholder_actor(name.clone(), endpoint_ref.clone()),
            None,
            Some(resolve_default_principal_id().await),
        )
    };

    let approved_entry = match store.approve(channel, code) {
        Ok(Some(entry)) => entry,
        Ok(None) => {
            return Err(format!(
                "No pending pairing request found for code: {}",
                code
            ));
        }
        Err(crate::pairing::PairingStoreError::ApproveRateLimited) => {
            return Err(
                "Too many failed approve attempts. Wait a few minutes before trying again."
                    .to_string(),
            );
        }
        Err(e) => return Err(e.to_string()),
    };

    let link_result = async {
        let actor = if let Some(principal_id) = principal_id_for_create {
            let created = db
                .create_actor(&NewActorRecord {
                    principal_id,
                    display_name: actor.display_name.clone(),
                    status: Default::default(),
                    preferred_delivery_endpoint: Some(endpoint_ref.clone()),
                    last_active_direct_endpoint: Some(endpoint_ref.clone()),
                })
                .await
                .map_err(|e| e.to_string())?;
            actor_created_for_link = Some(created.actor_id.to_string());
            created
        } else {
            actor
        };

        db.upsert_actor_endpoint(&NewActorEndpointRecord {
            endpoint: endpoint_ref.clone(),
            actor_id: actor.actor_id,
            metadata: approved_entry
                .meta
                .clone()
                .unwrap_or_else(|| serde_json::json!({})),
            approval_status: EndpointApprovalStatus::Approved,
        })
        .await
        .map_err(|e| e.to_string())?;
        db.set_actor_preferred_delivery_endpoint(&actor.actor_id.to_string(), Some(&endpoint_ref))
            .await
            .map_err(|e| e.to_string())?;

        if let Some(name) = rename_after_link.as_deref() {
            actor_renamed_from = Some(actor.display_name.clone());
            db.rename_actor(&actor.actor_id.to_string(), name)
                .await
                .map_err(|e| e.to_string())?;
        }

        Ok::<_, String>(actor)
    }
    .await;

    let actor = match link_result {
        Ok(actor) => actor,
        Err(err) => {
            let _ = rollback_pairing_approval(store, channel, &approved_entry);
            if let Some(actor_id) = actor_created_for_link.as_deref() {
                let _ = delete_actor_if_exists(db, actor_id).await;
            }
            if let Some(target_actor_id) = target_actor_id_for_rollback.as_deref() {
                let _ = db
                    .set_actor_preferred_delivery_endpoint(
                        target_actor_id,
                        prior_preferred_endpoint.as_ref(),
                    )
                    .await;
            }
            if let Some(prior) = prior_endpoint.as_ref() {
                let restore = NewActorEndpointRecord {
                    endpoint: prior.endpoint.clone(),
                    actor_id: prior.actor_id,
                    metadata: prior.metadata.clone(),
                    approval_status: prior.approval_status,
                };
                let _ = db.upsert_actor_endpoint(&restore).await;
            } else {
                let _ = db.unlink_actor_endpoint(channel, &endpoint_id).await;
            }
            if let Some(previous_name) = actor_renamed_from.as_deref()
                && let Some(target_actor_id) = target_actor_id_for_rollback.as_deref()
            {
                let _ = db.rename_actor(target_actor_id, previous_name).await;
            }
            return Err(err);
        }
    };

    println!(
        "Approved {} sender {} and linked {}:{} to actor '{}' ({}).",
        channel,
        approved_entry.id,
        channel,
        endpoint_id,
        rename_after_link.unwrap_or_else(|| actor.display_name.clone()),
        actor.actor_id
    );
    Ok(())
}

async fn connect_db() -> Result<Arc<dyn Database>, String> {
    let config = crate::config::Config::from_env()
        .await
        .map_err(|e| e.to_string())?;
    crate::db::connect_from_config(&config.database)
        .await
        .map_err(|e| e.to_string())
}

async fn resolve_default_principal_id() -> String {
    crate::config::helpers::optional_env("GATEWAY_USER_ID")
        .ok()
        .flatten()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PRINCIPAL_ID.to_string())
}

fn placeholder_actor(
    display_name: String,
    endpoint_ref: ActorEndpointRef,
) -> crate::identity::ActorRecord {
    crate::identity::ActorRecord {
        actor_id: uuid::Uuid::nil(),
        principal_id: String::new(),
        display_name,
        status: Default::default(),
        preferred_delivery_endpoint: Some(endpoint_ref.clone()),
        last_active_direct_endpoint: Some(endpoint_ref),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

fn rollback_pairing_approval(
    store: &PairingStore,
    channel: &str,
    entry: &PairingRequest,
) -> Result<(), String> {
    store
        .remove_allow_from(channel, &entry.id)
        .map_err(|e| e.to_string())?;
    store
        .restore_pending_request(channel, entry)
        .map_err(|e| e.to_string())
}

async fn delete_actor_if_exists<T>(db: &T, actor_id: &str) -> Result<(), String>
where
    T: PairingIdentityDb + ?Sized,
{
    db.delete_actor(actor_id).await.map_err(|e| e.to_string())?;
    Ok(())
}

fn run_block(store: &PairingStore, channel: &str, sender: &str) -> Result<(), String> {
    store
        .add_block_from(channel, sender)
        .map_err(|e| e.to_string())?;
    println!("Blocked {} sender: {}", channel, sender);
    Ok(())
}

fn run_unblock(store: &PairingStore, channel: &str, sender: &str) -> Result<(), String> {
    let removed = store
        .remove_block_from(channel, sender)
        .map_err(|e| e.to_string())?;
    if removed {
        println!("Unblocked {} sender: {}", channel, sender);
    } else {
        println!("{} sender {} was not blocked.", channel, sender);
    }
    Ok(())
}

fn run_blocked(store: &PairingStore, channel: &str, json: bool) -> Result<(), String> {
    let blocked = store.read_block_from(channel).map_err(|e| e.to_string())?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&blocked).map_err(|e| e.to_string())?
        );
        return Ok(());
    }

    if blocked.is_empty() {
        println!("No blocked senders on {}.", channel);
        return Ok(());
    }

    println!("Blocked senders on {} ({}):", channel, blocked.len());
    for entry in &blocked {
        println!("  {}", entry);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    use tempfile::TempDir;

    fn test_store() -> (PairingStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = PairingStore::with_base_dir(dir.path().to_path_buf());
        (store, dir)
    }

    #[derive(Default)]
    struct FailingPairingIdentityDb {
        state: Mutex<FailingPairingIdentityState>,
    }

    #[derive(Default)]
    struct FailingPairingIdentityState {
        actors: HashMap<String, ActorRecord>,
        endpoints: HashMap<(String, String), ActorEndpointRecord>,
        deleted_actor_ids: Vec<String>,
        unlinked_endpoints: Vec<(String, String)>,
    }

    impl FailingPairingIdentityDb {
        fn deleted_actor_ids(&self) -> Vec<String> {
            self.state.lock().unwrap().deleted_actor_ids.clone()
        }

        fn unlinked_endpoints(&self) -> Vec<(String, String)> {
            self.state.lock().unwrap().unlinked_endpoints.clone()
        }

        fn actor_count(&self) -> usize {
            self.state.lock().unwrap().actors.len()
        }
    }

    #[async_trait]
    impl PairingIdentityDb for FailingPairingIdentityDb {
        async fn get_actor(&self, actor_id: &str) -> Result<Option<ActorRecord>, DatabaseError> {
            Ok(self.state.lock().unwrap().actors.get(actor_id).cloned())
        }

        async fn create_actor(&self, actor: &NewActorRecord) -> Result<ActorRecord, DatabaseError> {
            let actor_id = uuid::Uuid::new_v4();
            let record = ActorRecord {
                actor_id,
                principal_id: actor.principal_id.clone(),
                display_name: actor.display_name.clone(),
                status: actor.status,
                preferred_delivery_endpoint: actor.preferred_delivery_endpoint.clone(),
                last_active_direct_endpoint: actor.last_active_direct_endpoint.clone(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };
            self.state
                .lock()
                .unwrap()
                .actors
                .insert(actor_id.to_string(), record.clone());
            Ok(record)
        }

        async fn delete_actor(&self, actor_id: &str) -> Result<bool, DatabaseError> {
            let mut state = self.state.lock().unwrap();
            state.deleted_actor_ids.push(actor_id.to_string());
            Ok(state.actors.remove(actor_id).is_some())
        }

        async fn rename_actor(
            &self,
            actor_id: &str,
            display_name: &str,
        ) -> Result<(), DatabaseError> {
            if let Some(actor) = self.state.lock().unwrap().actors.get_mut(actor_id) {
                actor.display_name = display_name.to_string();
                actor.updated_at = chrono::Utc::now();
            }
            Ok(())
        }

        async fn set_actor_preferred_delivery_endpoint(
            &self,
            actor_id: &str,
            endpoint: Option<&ActorEndpointRef>,
        ) -> Result<(), DatabaseError> {
            if let Some(actor) = self.state.lock().unwrap().actors.get_mut(actor_id) {
                actor.preferred_delivery_endpoint = endpoint.cloned();
                actor.updated_at = chrono::Utc::now();
            }
            Ok(())
        }

        async fn upsert_actor_endpoint(
            &self,
            _record: &NewActorEndpointRecord,
        ) -> Result<ActorEndpointRecord, DatabaseError> {
            Err(DatabaseError::Query(
                "simulated endpoint-link failure".to_string(),
            ))
        }

        async fn get_actor_endpoint(
            &self,
            channel: &str,
            external_user_id: &str,
        ) -> Result<Option<ActorEndpointRecord>, DatabaseError> {
            Ok(self
                .state
                .lock()
                .unwrap()
                .endpoints
                .get(&(channel.to_string(), external_user_id.to_string()))
                .cloned())
        }

        async fn unlink_actor_endpoint(
            &self,
            channel: &str,
            external_user_id: &str,
        ) -> Result<bool, DatabaseError> {
            let mut state = self.state.lock().unwrap();
            state
                .unlinked_endpoints
                .push((channel.to_string(), external_user_id.to_string()));
            Ok(state
                .endpoints
                .remove(&(channel.to_string(), external_user_id.to_string()))
                .is_some())
        }
    }

    #[test]
    fn test_list_empty_returns_ok() {
        let (store, _) = test_store();
        let result = run_pairing_command_with_store(
            &store,
            PairingCommand::List {
                channel: "telegram".to_string(),
                json: false,
            },
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_list_json_empty_returns_ok() {
        let (store, _) = test_store();
        let result = run_pairing_command_with_store(
            &store,
            PairingCommand::List {
                channel: "telegram".to_string(),
                json: true,
            },
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_approve_invalid_code_returns_err() {
        let (store, _) = test_store();
        // Create a pending request so the pairing file exists, then approve with wrong code
        store.upsert_request("telegram", "user1", None).unwrap();

        let result = run_pairing_command_with_store(
            &store,
            PairingCommand::Approve {
                channel: "telegram".to_string(),
                code: "BADCODE1".to_string(),
                actor: None,
                name: None,
            },
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No pending pairing request"));
    }

    #[test]
    fn test_approve_valid_code_returns_ok() {
        let (store, _) = test_store();
        let r = store.upsert_request("telegram", "user1", None).unwrap();
        assert!(r.created);

        let result = run_pairing_command_with_store(
            &store,
            PairingCommand::Approve {
                channel: "telegram".to_string(),
                code: r.code,
                actor: None,
                name: None,
            },
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_list_with_pending_returns_ok() {
        let (store, _) = test_store();
        store.upsert_request("telegram", "user1", None).unwrap();

        let result = run_pairing_command_with_store(
            &store,
            PairingCommand::List {
                channel: "telegram".to_string(),
                json: false,
            },
        );
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_approve_with_identity_rolls_back_on_link_failure() {
        let (store, _) = test_store();
        let db = FailingPairingIdentityDb::default();
        let request = store
            .upsert_request(
                "telegram",
                "sender-42",
                Some(serde_json::json!({
                    "stable_sender_id": "ext-42"
                })),
            )
            .unwrap();

        let result = run_approve_with_identity(
            &store,
            Some(&db),
            "telegram",
            &request.code,
            None,
            Some("Alex".to_string()),
        )
        .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("simulated endpoint-link failure")
        );

        let pending = store.list_pending("telegram").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "sender-42");
        assert_eq!(pending[0].code, request.code);

        let allow = store.read_allow_from("telegram").unwrap();
        assert!(
            allow.is_empty(),
            "sender should not remain approved after rollback"
        );

        assert_eq!(db.actor_count(), 0, "temporary actor should be deleted");
        assert_eq!(db.deleted_actor_ids().len(), 1);
        assert_eq!(
            db.unlinked_endpoints(),
            vec![("telegram".to_string(), "ext-42".to_string())]
        );
    }
}
