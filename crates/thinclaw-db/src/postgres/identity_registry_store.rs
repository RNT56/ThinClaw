//! postgres: identity_registry_store.

use super::*;

#[async_trait]
impl IdentityRegistryStore for PgBackend {
    async fn create_actor(&self, actor: &NewActorRecord) -> Result<ActorRecord, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let row = client
            .query_one(
                &format!(
                    "INSERT INTO actors (principal_id, display_name, status, preferred_delivery_channel, preferred_delivery_external_user_id, last_active_direct_channel, last_active_direct_external_user_id) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING {PG_ACTOR_COLUMNS}"
                ),
                &[
                    &actor.principal_id,
                    &actor.display_name,
                    &actor.status.as_str(),
                    &actor
                        .preferred_delivery_endpoint
                        .as_ref()
                        .map(|e| e.channel.as_str()),
                    &actor
                        .preferred_delivery_endpoint
                        .as_ref()
                        .map(|e| e.external_user_id.as_str()),
                    &actor
                        .last_active_direct_endpoint
                        .as_ref()
                        .map(|e| e.channel.as_str()),
                    &actor
                        .last_active_direct_endpoint
                        .as_ref()
                        .map(|e| e.external_user_id.as_str()),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to create actor: {e}")))?;

        pg_row_to_actor(&row)
    }

    async fn get_actor(&self, actor_id: Uuid) -> Result<Option<ActorRecord>, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let row = client
            .query_opt(
                &format!("SELECT {PG_ACTOR_COLUMNS} FROM actors WHERE actor_id = $1"),
                &[&actor_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to get actor: {e}")))?;

        row.map(|row| pg_row_to_actor(&row)).transpose()
    }

    async fn list_actors(&self, principal_id: &str) -> Result<Vec<ActorRecord>, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let rows = client
            .query(
                &format!(
                    "SELECT {PG_ACTOR_COLUMNS} FROM actors WHERE principal_id = $1 ORDER BY created_at ASC"
                ),
                &[&principal_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to list actors: {e}")))?;

        rows.iter()
            .map(pg_row_to_actor)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn update_actor(&self, actor: &ActorRecord) -> Result<(), DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let affected = client
            .execute(
                r#"
                UPDATE actors SET
                    principal_id = $2,
                    display_name = $3,
                    status = $4,
                    preferred_delivery_channel = $5,
                    preferred_delivery_external_user_id = $6,
                    last_active_direct_channel = $7,
                    last_active_direct_external_user_id = $8,
                    updated_at = $9
                WHERE actor_id = $1
                "#,
                &[
                    &actor.actor_id,
                    &actor.principal_id,
                    &actor.display_name,
                    &actor.status.as_str(),
                    &actor
                        .preferred_delivery_endpoint
                        .as_ref()
                        .map(|e| e.channel.as_str()),
                    &actor
                        .preferred_delivery_endpoint
                        .as_ref()
                        .map(|e| e.external_user_id.as_str()),
                    &actor
                        .last_active_direct_endpoint
                        .as_ref()
                        .map(|e| e.channel.as_str()),
                    &actor
                        .last_active_direct_endpoint
                        .as_ref()
                        .map(|e| e.external_user_id.as_str()),
                    &actor.updated_at,
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to update actor: {e}")))?;

        if affected == 0 {
            return Err(DatabaseError::NotFound {
                entity: "actor".to_string(),
                id: actor.actor_id.to_string(),
            });
        }
        Ok(())
    }

    async fn delete_actor(&self, actor_id: Uuid) -> Result<bool, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let affected = client
            .execute("DELETE FROM actors WHERE actor_id = $1", &[&actor_id])
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to delete actor: {e}")))?;
        Ok(affected > 0)
    }

    async fn rename_actor(&self, actor_id: Uuid, display_name: &str) -> Result<(), DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let affected = client
            .execute(
                "UPDATE actors SET display_name = $2, updated_at = NOW() WHERE actor_id = $1",
                &[&actor_id, &display_name],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to rename actor: {e}")))?;
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
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let affected = client
            .execute(
                "UPDATE actors SET status = $2, updated_at = NOW() WHERE actor_id = $1",
                &[&actor_id, &status.as_str()],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to update actor status: {e}")))?;
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
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let affected = client
            .execute(
                r#"
                UPDATE actors SET
                    preferred_delivery_channel = $2,
                    preferred_delivery_external_user_id = $3,
                    updated_at = NOW()
                WHERE actor_id = $1
                "#,
                &[
                    &actor_id,
                    &endpoint.as_ref().map(|e| e.channel.as_str()),
                    &endpoint.as_ref().map(|e| e.external_user_id.as_str()),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to set preferred endpoint: {e}")))?;
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
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let affected = client
            .execute(
                r#"
                UPDATE actors SET
                    last_active_direct_channel = $2,
                    last_active_direct_external_user_id = $3,
                    updated_at = NOW()
                WHERE actor_id = $1
                "#,
                &[
                    &actor_id,
                    &endpoint.as_ref().map(|e| e.channel.as_str()),
                    &endpoint.as_ref().map(|e| e.external_user_id.as_str()),
                ],
            )
            .await
            .map_err(|e| {
                DatabaseError::Query(format!("Failed to set last active endpoint: {e}"))
            })?;
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
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let row = client
            .query_one(
                &format!(
                    "INSERT INTO actor_endpoints (channel, external_user_id, actor_id, endpoint_metadata, approval_status) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (channel, external_user_id) DO UPDATE SET actor_id = EXCLUDED.actor_id, endpoint_metadata = EXCLUDED.endpoint_metadata, approval_status = EXCLUDED.approval_status, updated_at = NOW() RETURNING {PG_ACTOR_ENDPOINT_COLUMNS}"
                ),
                &[
                    &record.endpoint.channel,
                    &record.endpoint.external_user_id,
                    &record.actor_id,
                    &record.metadata,
                    &record.approval_status.as_str(),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to upsert actor endpoint: {e}")))?;

        pg_row_to_actor_endpoint(&row)
    }

    async fn get_actor_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<Option<ActorEndpointRecord>, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let row = client
            .query_opt(
                &format!(
                    "SELECT {PG_ACTOR_ENDPOINT_COLUMNS} FROM actor_endpoints WHERE channel = $1 AND external_user_id = $2"
                ),
                &[&channel, &external_user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to get actor endpoint: {e}")))?;

        row.map(|row| pg_row_to_actor_endpoint(&row)).transpose()
    }

    async fn list_actor_endpoints(
        &self,
        actor_id: Uuid,
    ) -> Result<Vec<ActorEndpointRecord>, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let rows = client
            .query(
                &format!(
                    "SELECT {PG_ACTOR_ENDPOINT_COLUMNS} FROM actor_endpoints WHERE actor_id = $1 ORDER BY channel, external_user_id"
                ),
                &[&actor_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to list actor endpoints: {e}")))?;

        rows.iter()
            .map(pg_row_to_actor_endpoint)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn delete_actor_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let affected = client
            .execute(
                "DELETE FROM actor_endpoints WHERE channel = $1 AND external_user_id = $2",
                &[&channel, &external_user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to delete actor endpoint: {e}")))?;
        Ok(affected > 0)
    }

    async fn resolve_actor_for_endpoint(
        &self,
        channel: &str,
        external_user_id: &str,
    ) -> Result<Option<ActorRecord>, DatabaseError> {
        let client = self
            .store
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get PG connection: {e}")))?;

        let row = client
            .query_opt(
                "SELECT a.actor_id, a.principal_id, a.display_name, a.status, \
                 a.preferred_delivery_channel, a.preferred_delivery_external_user_id, \
                 a.last_active_direct_channel, a.last_active_direct_external_user_id, \
                 a.created_at, a.updated_at \
                 FROM actor_endpoints e JOIN actors a ON a.actor_id = e.actor_id \
                 WHERE e.channel = $1 AND e.external_user_id = $2",
                &[&channel, &external_user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("Failed to resolve actor: {e}")))?;

        row.map(|row| pg_row_to_actor(&row)).transpose()
    }
}

// ==================== AgentRegistryStore ====================
