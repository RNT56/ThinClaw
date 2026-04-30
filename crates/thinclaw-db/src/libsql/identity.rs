//! Identity registry implementation for `LibSqlBackend`.

use async_trait::async_trait;
use libsql::params;
use uuid::Uuid;

use super::{LibSqlBackend, fmt_ts, get_json, get_opt_text, get_text, get_ts, opt_text};
use crate::IdentityRegistryStore;
use thinclaw_identity::{
    ActorEndpointRecord, ActorEndpointRef, ActorRecord, ActorStatus, EndpointApprovalStatus,
    NewActorEndpointRecord, NewActorRecord,
};
use thinclaw_types::error::DatabaseError;

const ACTOR_COLUMNS: &str = "\
    actor_id, principal_id, display_name, status, \
    preferred_delivery_channel, preferred_delivery_external_user_id, \
    last_active_direct_channel, last_active_direct_external_user_id, \
    created_at, updated_at";

const ACTOR_COLUMNS_SELECT: &str = "\
    a.actor_id, a.principal_id, a.display_name, a.status, \
    a.preferred_delivery_channel, a.preferred_delivery_external_user_id, \
    a.last_active_direct_channel, a.last_active_direct_external_user_id, \
    a.created_at, a.updated_at";

const ACTOR_ENDPOINT_COLUMNS: &str = "\
    channel, external_user_id, actor_id, endpoint_metadata, approval_status, \
    created_at, updated_at";

fn endpoint_ref_from_row(
    channel: Option<String>,
    external_user_id: Option<String>,
) -> Option<ActorEndpointRef> {
    match (channel, external_user_id) {
        (Some(channel), Some(external_user_id)) => {
            Some(ActorEndpointRef::new(channel, external_user_id))
        }
        _ => None,
    }
}

fn row_to_actor(row: &libsql::Row) -> Result<ActorRecord, DatabaseError> {
    let status = get_text(row, 3)
        .parse::<ActorStatus>()
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

    Ok(ActorRecord {
        actor_id: get_text(row, 0)
            .parse()
            .map_err(|_| DatabaseError::Serialization("invalid actor_id".into()))?,
        principal_id: get_text(row, 1),
        display_name: get_text(row, 2),
        status,
        preferred_delivery_endpoint: endpoint_ref_from_row(
            get_opt_text(row, 4),
            get_opt_text(row, 5),
        ),
        last_active_direct_endpoint: endpoint_ref_from_row(
            get_opt_text(row, 6),
            get_opt_text(row, 7),
        ),
        created_at: get_ts(row, 8),
        updated_at: get_ts(row, 9),
    })
}

fn row_to_actor_endpoint(row: &libsql::Row) -> Result<ActorEndpointRecord, DatabaseError> {
    let approval_status = get_text(row, 4)
        .parse::<EndpointApprovalStatus>()
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

    Ok(ActorEndpointRecord {
        endpoint: ActorEndpointRef::new(get_text(row, 0), get_text(row, 1)),
        actor_id: get_text(row, 2)
            .parse()
            .map_err(|_| DatabaseError::Serialization("invalid actor_id".into()))?,
        metadata: get_json(row, 3),
        approval_status,
        created_at: get_ts(row, 5),
        updated_at: get_ts(row, 6),
    })
}

#[async_trait]
impl IdentityRegistryStore for LibSqlBackend {
    async fn create_actor(&self, actor: &NewActorRecord) -> Result<ActorRecord, DatabaseError> {
        let conn = self.connect().await?;
        let actor_id = Uuid::new_v4();
        let now = fmt_ts(&chrono::Utc::now());

        let mut rows = conn
            .query(
            &format!(
                "INSERT INTO actors ({ACTOR_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) RETURNING {ACTOR_COLUMNS}"
            ),
            params![
                actor_id.to_string(),
                actor.principal_id.as_str(),
                actor.display_name.as_str(),
                actor.status.as_str(),
                opt_text(actor.preferred_delivery_endpoint.as_ref().map(|e| e.channel.as_str())),
                opt_text(
                    actor
                        .preferred_delivery_endpoint
                        .as_ref()
                        .map(|e| e.external_user_id.as_str())
                ),
                opt_text(actor.last_active_direct_endpoint.as_ref().map(|e| e.channel.as_str())),
                opt_text(
                    actor
                        .last_active_direct_endpoint
                        .as_ref()
                        .map(|e| e.external_user_id.as_str())
                ),
                now.as_str(),
                now.as_str(),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => row_to_actor(&row),
            None => Err(DatabaseError::NotFound {
                entity: "actor".to_string(),
                id: actor_id.to_string(),
            }),
        }
    }

    async fn get_actor(&self, actor_id: Uuid) -> Result<Option<ActorRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!("SELECT {ACTOR_COLUMNS} FROM actors WHERE actor_id = ?1"),
                params![actor_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => row_to_actor(&row).map(Some),
            None => Ok(None),
        }
    }

    async fn list_actors(&self, principal_id: &str) -> Result<Vec<ActorRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {ACTOR_COLUMNS} FROM actors WHERE principal_id = ?1 ORDER BY created_at ASC"
                ),
                params![principal_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut actors = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            actors.push(row_to_actor(&row)?);
        }
        Ok(actors)
    }

    async fn update_actor(&self, actor: &ActorRecord) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let affected = conn
            .execute(
                r#"
                UPDATE actors SET
                    principal_id = ?2,
                    display_name = ?3,
                    status = ?4,
                    preferred_delivery_channel = ?5,
                    preferred_delivery_external_user_id = ?6,
                    last_active_direct_channel = ?7,
                    last_active_direct_external_user_id = ?8,
                    updated_at = ?9
                WHERE actor_id = ?1
                "#,
                params![
                    actor.actor_id.to_string(),
                    actor.principal_id.as_str(),
                    actor.display_name.as_str(),
                    actor.status.as_str(),
                    opt_text(
                        actor
                            .preferred_delivery_endpoint
                            .as_ref()
                            .map(|e| e.channel.as_str())
                    ),
                    opt_text(
                        actor
                            .preferred_delivery_endpoint
                            .as_ref()
                            .map(|e| e.external_user_id.as_str())
                    ),
                    opt_text(
                        actor
                            .last_active_direct_endpoint
                            .as_ref()
                            .map(|e| e.channel.as_str())
                    ),
                    opt_text(
                        actor
                            .last_active_direct_endpoint
                            .as_ref()
                            .map(|e| e.external_user_id.as_str())
                    ),
                    fmt_ts(&actor.updated_at),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        if affected == 0 {
            return Err(DatabaseError::NotFound {
                entity: "actor".to_string(),
                id: actor.actor_id.to_string(),
            });
        }
        Ok(())
    }

    async fn delete_actor(&self, actor_id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let rows = conn
            .execute(
                "DELETE FROM actors WHERE actor_id = ?1",
                params![actor_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(rows > 0)
    }

    async fn rename_actor(&self, actor_id: Uuid, display_name: &str) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let affected = conn
            .execute(
                "UPDATE actors SET display_name = ?2, updated_at = ?3 WHERE actor_id = ?1",
                params![
                    actor_id.to_string(),
                    display_name,
                    fmt_ts(&chrono::Utc::now())
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if affected == 0 {
            return Err(DatabaseError::NotFound {
                entity: "actor".to_string(),
                id: actor_id.to_string(),
            });
        }
        Ok(())
    }

    async fn set_actor_status(
        &self,
        actor_id: Uuid,
        status: ActorStatus,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let affected = conn
            .execute(
                "UPDATE actors SET status = ?2, updated_at = ?3 WHERE actor_id = ?1",
                params![
                    actor_id.to_string(),
                    status.as_str(),
                    fmt_ts(&chrono::Utc::now())
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if affected == 0 {
            return Err(DatabaseError::NotFound {
                entity: "actor".to_string(),
                id: actor_id.to_string(),
            });
        }
        Ok(())
    }

    async fn set_actor_preferred_delivery_endpoint(
        &self,
        actor_id: Uuid,
        endpoint: Option<&ActorEndpointRef>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let affected = conn
            .execute(
                r#"
                UPDATE actors SET
                    preferred_delivery_channel = ?2,
                    preferred_delivery_external_user_id = ?3,
                    updated_at = ?4
                WHERE actor_id = ?1
                "#,
                params![
                    actor_id.to_string(),
                    opt_text(endpoint.map(|e| e.channel.as_str())),
                    opt_text(endpoint.map(|e| e.external_user_id.as_str())),
                    fmt_ts(&chrono::Utc::now()),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if affected == 0 {
            return Err(DatabaseError::NotFound {
                entity: "actor".to_string(),
                id: actor_id.to_string(),
            });
        }
        Ok(())
    }

    async fn set_actor_last_active_direct_endpoint(
        &self,
        actor_id: Uuid,
        endpoint: Option<&ActorEndpointRef>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let affected = conn
            .execute(
                r#"
                UPDATE actors SET
                    last_active_direct_channel = ?2,
                    last_active_direct_external_user_id = ?3,
                    updated_at = ?4
                WHERE actor_id = ?1
                "#,
                params![
                    actor_id.to_string(),
                    opt_text(endpoint.map(|e| e.channel.as_str())),
                    opt_text(endpoint.map(|e| e.external_user_id.as_str())),
                    fmt_ts(&chrono::Utc::now()),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if affected == 0 {
            return Err(DatabaseError::NotFound {
                entity: "actor".to_string(),
                id: actor_id.to_string(),
            });
        }
        Ok(())
    }

    async fn upsert_actor_endpoint(
        &self,
        record: &NewActorEndpointRecord,
    ) -> Result<ActorEndpointRecord, DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&chrono::Utc::now());
        let mut rows = conn
            .query(
            r#"
            INSERT INTO actor_endpoints (
                channel, external_user_id, actor_id, endpoint_metadata, approval_status,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT (channel, external_user_id) DO UPDATE SET
                actor_id = excluded.actor_id,
                endpoint_metadata = excluded.endpoint_metadata,
                approval_status = excluded.approval_status,
                updated_at = excluded.updated_at
            RETURNING channel, external_user_id, actor_id, endpoint_metadata, approval_status, created_at, updated_at
            "#,
            params![
                record.endpoint.channel.as_str(),
                record.endpoint.external_user_id.as_str(),
                record.actor_id.to_string(),
                record.metadata.to_string(),
                record.approval_status.as_str(),
                now.as_str(),
                now.as_str(),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => row_to_actor_endpoint(&row),
            None => Err(DatabaseError::NotFound {
                entity: "actor_endpoint".to_string(),
                id: format!(
                    "{}/{}",
                    record.endpoint.channel, record.endpoint.external_user_id
                ),
            }),
        }
    }

    async fn get_actor_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<Option<ActorEndpointRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!("SELECT {ACTOR_ENDPOINT_COLUMNS} FROM actor_endpoints WHERE channel = ?1 AND external_user_id = ?2"),
                params![channel, external_user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => row_to_actor_endpoint(&row).map(Some),
            None => Ok(None),
        }
    }

    async fn list_actor_endpoints(
        &self,
        actor_id: Uuid,
    ) -> Result<Vec<ActorEndpointRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!("SELECT {ACTOR_ENDPOINT_COLUMNS} FROM actor_endpoints WHERE actor_id = ?1 ORDER BY channel, external_user_id"),
                params![actor_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut endpoints = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            endpoints.push(row_to_actor_endpoint(&row)?);
        }
        Ok(endpoints)
    }

    async fn delete_actor_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let rows = conn
            .execute(
                "DELETE FROM actor_endpoints WHERE channel = ?1 AND external_user_id = ?2",
                params![channel, external_user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(rows > 0)
    }

    async fn resolve_actor_for_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<Option<ActorRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!("SELECT {ACTOR_COLUMNS_SELECT} FROM actor_endpoints e JOIN actors a ON a.actor_id = e.actor_id WHERE e.channel = ?1 AND e.external_user_id = ?2"),
                params![channel, external_user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => row_to_actor(&row).map(Some),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use thinclaw_identity::{ActorEndpointRef, ActorStatus, EndpointApprovalStatus};

    async fn test_backend() -> (LibSqlBackend, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("identity_registry.db");
        let backend = LibSqlBackend::new_local(&path).await.unwrap();
        backend.run_migrations().await.unwrap();
        (backend, dir)
    }

    #[tokio::test]
    async fn create_list_and_lookup_actor() {
        let (backend, _dir) = test_backend().await;
        let created = backend
            .create_actor(&NewActorRecord {
                principal_id: "default".into(),
                display_name: "Alex".into(),
                status: ActorStatus::Active,
                preferred_delivery_endpoint: None,
                last_active_direct_endpoint: None,
            })
            .await
            .unwrap();

        let loaded = backend.get_actor(created.actor_id).await.unwrap().unwrap();
        assert_eq!(loaded.display_name, "Alex");
        assert_eq!(loaded.principal_id, "default");

        let list = backend.list_actors("default").await.unwrap();
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn endpoint_link_and_resolve_roundtrip() {
        let (backend, _dir) = test_backend().await;
        let actor = backend
            .create_actor(&NewActorRecord {
                principal_id: "default".into(),
                display_name: "Jordan".into(),
                status: ActorStatus::Active,
                preferred_delivery_endpoint: None,
                last_active_direct_endpoint: None,
            })
            .await
            .unwrap();

        let record = backend
            .upsert_actor_endpoint(&NewActorEndpointRecord {
                endpoint: ActorEndpointRef::new("telegram", "1234"),
                actor_id: actor.actor_id,
                metadata: serde_json::json!({"alias": "work"}),
                approval_status: EndpointApprovalStatus::Approved,
            })
            .await
            .unwrap();

        assert_eq!(record.actor_id, actor.actor_id);
        assert_eq!(record.approval_status, EndpointApprovalStatus::Approved);

        let resolved = backend
            .resolve_actor_for_endpoint("telegram", "1234")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.actor_id, actor.actor_id);

        assert!(
            backend
                .delete_actor_endpoint("telegram", "1234")
                .await
                .unwrap()
        );
        assert!(
            backend
                .get_actor_endpoint("telegram", "1234")
                .await
                .unwrap()
                .is_none()
        );
    }
}
