//! PostgreSQL store for persisting agent data.

use chrono::{DateTime, Utc};
#[cfg(feature = "postgres")]
use deadpool_postgres::{Config, Pool, Runtime};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
#[cfg(feature = "postgres")]
use tokio_postgres::NoTls;
use uuid::Uuid;

#[cfg(feature = "postgres")]
use crate::config::DatabaseConfig;
#[cfg(feature = "postgres")]
use crate::context::{ActionRecord, JobContext, JobState, StateTransition};
#[cfg(feature = "postgres")]
use crate::error::DatabaseError;

/// Record for an LLM call to be persisted.
#[derive(Debug, Clone)]
pub struct LlmCallRecord<'a> {
    pub job_id: Option<Uuid>,
    pub conversation_id: Option<Uuid>,
    pub provider: &'a str,
    pub model: &'a str,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost: Decimal,
    pub purpose: Option<&'a str>,
}

/// Whether a conversation is a one-to-one direct thread or a shared group thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationKind {
    Direct,
    Group,
}

impl ConversationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Group => "group",
        }
    }

    pub fn from_db(value: Option<&str>) -> Self {
        match value {
            Some("group") => Self::Group,
            _ => Self::Direct,
        }
    }
}

/// Stable conversation scope shared across channels for the same direct or group thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationScope {
    pub conversation_scope_id: Uuid,
    pub conversation_kind: ConversationKind,
    pub channel: String,
    pub stable_external_conversation_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_conversation_id: Option<String>,
}

impl ConversationScope {
    pub fn direct(
        conversation_scope_id: Uuid,
        channel: impl Into<String>,
        stable_external_conversation_key: impl Into<String>,
        external_conversation_id: Option<String>,
    ) -> Self {
        Self {
            conversation_scope_id,
            conversation_kind: ConversationKind::Direct,
            channel: channel.into(),
            stable_external_conversation_key: stable_external_conversation_key.into(),
            external_conversation_id,
        }
    }

    pub fn group(
        conversation_scope_id: Uuid,
        channel: impl Into<String>,
        stable_external_conversation_key: impl Into<String>,
        external_conversation_id: Option<String>,
    ) -> Self {
        Self {
            conversation_scope_id,
            conversation_kind: ConversationKind::Group,
            channel: channel.into(),
            stable_external_conversation_key: stable_external_conversation_key.into(),
            external_conversation_id,
        }
    }
}

/// Compact metadata used to carry work forward between turns and channels.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationHandoffMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_actor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_user_goal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_summary: Option<String>,
}

impl ConversationHandoffMetadata {
    pub fn is_empty(&self) -> bool {
        self.last_actor_id.is_none()
            && self.task_state.is_none()
            && self.last_user_goal.is_none()
            && self.handoff_summary.is_none()
    }
}

/// A search result hit from the conversation transcript index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSearchHit {
    pub conversation_id: Uuid,
    pub message_id: Uuid,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    pub channel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub conversation_kind: ConversationKind,
    pub role: String,
    pub content: String,
    pub excerpt: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

/// Durable record of an observed learning signal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningEvent {
    pub id: Uuid,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Uuid>,
    pub event_type: String,
    pub source: String,
    pub payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

/// Evaluation result for a learning event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningEvaluation {
    pub id: Uuid,
    pub learning_event_id: Uuid,
    pub user_id: String,
    pub evaluator: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    pub details: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Distilled improvement candidate derived from one or more learning events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningCandidate {
    pub id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learning_event_id: Option<Uuid>,
    pub user_id: String,
    pub candidate_type: String,
    pub risk_tier: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub proposal: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Versioned snapshot of a learned artifact mutation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningArtifactVersion {
    pub id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<Uuid>,
    pub user_id: String,
    pub artifact_type: String,
    pub artifact_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_label: Option<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_content: Option<String>,
    pub provenance: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Explicit user/operator feedback on a candidate or artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningFeedbackRecord {
    pub id: Uuid,
    pub user_id: String,
    pub target_type: String,
    pub target_id: String,
    pub verdict: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Recorded rollback operations for learned artifacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningRollbackRecord {
    pub id: Uuid,
    pub user_id: String,
    pub artifact_type: String,
    pub artifact_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_version_id: Option<Uuid>,
    pub reason: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Approval-gated code change proposal generated by the learning loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningCodeProposal {
    pub id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learning_event_id: Option<Uuid>,
    pub user_id: String,
    pub status: String,
    pub title: String,
    pub rationale: String,
    pub target_files: Vec<String>,
    pub diff: String,
    pub validation_results: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Database store for the agent.
#[cfg(feature = "postgres")]
pub struct Store {
    pool: Pool,
}

#[cfg(feature = "postgres")]
impl Store {
    /// Wrap an existing pool (useful when the caller already has a connection).
    pub fn from_pool(pool: Pool) -> Self {
        Self { pool }
    }

    /// Create a new store and connect to the database.
    pub async fn new(config: &DatabaseConfig) -> Result<Self, DatabaseError> {
        let mut cfg = Config::new();
        cfg.url = Some(config.url().to_string());
        cfg.pool = Some(deadpool_postgres::PoolConfig {
            max_size: config.pool_size,
            ..Default::default()
        });

        let pool = {
            // Try TLS first ("prefer" semantics) — uses system CA roots.
            // Falls back to NoTls if TLS negotiation fails (e.g. local dev PG without certs).
            let tls_result = (|| -> Result<_, Box<dyn std::error::Error>> {
                let certs = rustls_native_certs::load_native_certs();
                let mut root_store = rustls::RootCertStore::empty();
                for cert in certs.certs {
                    root_store.add(cert)?;
                }
                let tls_config = rustls::ClientConfig::builder()
                    .with_root_certificates(root_store)
                    .with_no_client_auth();
                let tls = tokio_postgres_rustls::MakeRustlsConnect::new(tls_config);
                Ok(cfg.create_pool(Some(Runtime::Tokio1), tls)?)
            })();

            match tls_result {
                Ok(pool) => {
                    tracing::debug!("PostgreSQL pool created with TLS (prefer mode)");
                    pool
                }
                Err(tls_err) => {
                    tracing::debug!("TLS pool creation failed ({tls_err}), falling back to NoTls");
                    cfg.create_pool(Some(Runtime::Tokio1), NoTls)
                        .map_err(|e| DatabaseError::Pool(e.to_string()))?
                }
            }
        };

        // Test connection
        let _ = pool.get().await?;

        Ok(Self { pool })
    }

    /// Run database migrations (embedded via refinery).
    pub async fn run_migrations(&self) -> Result<(), DatabaseError> {
        use refinery::embed_migrations;
        embed_migrations!("migrations");

        let mut client = self.pool.get().await?;
        migrations::runner()
            .run_async(&mut **client)
            .await
            .map_err(|e| DatabaseError::Migration(e.to_string()))?;
        Ok(())
    }

    /// Get a connection from the pool.
    pub async fn conn(&self) -> Result<deadpool_postgres::Object, DatabaseError> {
        Ok(self.pool.get().await?)
    }

    /// Get a clone of the database pool.
    ///
    /// Useful for sharing the pool with other components like Workspace.
    pub fn pool(&self) -> Pool {
        self.pool.clone()
    }

    // ==================== Conversations ====================

    /// Create a new conversation.
    pub async fn create_conversation(
        &self,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = Uuid::new_v4();
        let stable_external_conversation_key = conversation_stable_key(channel, thread_id, id);

        conn.execute(
            r#"
            INSERT INTO conversations (
                id, channel, user_id, actor_id, conversation_scope_id, conversation_kind,
                thread_id, stable_external_conversation_key, metadata
            ) VALUES ($1, $2, $3, NULL, $4, $5, $6, $7, '{}'::jsonb)
            "#,
            &[
                &id,
                &channel,
                &user_id,
                &id,
                &ConversationKind::Direct.as_str(),
                &thread_id,
                &stable_external_conversation_key,
            ],
        )
        .await?;

        Ok(id)
    }

    /// Update conversation last activity.
    pub async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            "UPDATE conversations SET last_activity = NOW() WHERE id = $1",
            &[&id],
        )
        .await?;
        Ok(())
    }

    /// Add a message to a conversation.
    pub async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.add_conversation_message_with_attribution(
            conversation_id,
            role,
            content,
            None,
            None,
            None,
            None,
        )
        .await
    }

    /// Add a message with actor attribution and message-level metadata.
    pub async fn add_conversation_message_with_attribution(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
        actor_id: Option<&str>,
        actor_display_name: Option<&str>,
        raw_sender_id: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = Uuid::new_v4();
        let metadata_value = metadata.cloned().unwrap_or_else(|| serde_json::json!({}));

        conn.execute(
            r#"
            INSERT INTO conversation_messages (
                id, conversation_id, role, content, actor_id, actor_display_name,
                raw_sender_id, metadata
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
            &[
                &id,
                &conversation_id,
                &role,
                &content,
                &actor_id,
                &actor_display_name,
                &raw_sender_id,
                &metadata_value,
            ],
        )
        .await?;

        // Update conversation activity
        self.touch_conversation(conversation_id).await?;

        Ok(id)
    }

    /// Delete a conversation and all its messages (cascading via FK).
    pub async fn delete_conversation(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .execute("DELETE FROM conversations WHERE id = $1", &[&id])
            .await?;
        Ok(rows > 0)
    }

    /// Delete all messages from a conversation without deleting the conversation.
    pub async fn delete_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<u64, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .execute(
                "DELETE FROM conversation_messages WHERE conversation_id = $1",
                &[&conversation_id],
            )
            .await?;
        Ok(rows)
    }

    /// Update the actor-aware identity fields for a conversation.
    pub async fn update_conversation_identity(
        &self,
        id: Uuid,
        actor_id: Option<&str>,
        conversation_scope_id: Option<Uuid>,
        conversation_kind: ConversationKind,
        stable_external_conversation_key: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE conversations
            SET actor_id = $2,
                conversation_scope_id = COALESCE($3, conversation_scope_id),
                conversation_kind = $4,
                stable_external_conversation_key = COALESCE($5, stable_external_conversation_key)
            WHERE id = $1
            "#,
            &[
                &id,
                &actor_id,
                &conversation_scope_id,
                &conversation_kind.as_str(),
                &stable_external_conversation_key,
            ],
        )
        .await?;
        Ok(())
    }

    /// Update the compact handoff metadata for a conversation.
    pub async fn set_conversation_handoff_metadata(
        &self,
        id: Uuid,
        handoff: &ConversationHandoffMetadata,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let handoff_value = serde_json::to_value(handoff)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        conn.execute(
            "UPDATE conversations SET metadata = jsonb_set(coalesce(metadata, '{}'::jsonb), '{handoff}', $2::jsonb, true) WHERE id = $1",
            &[&id, &handoff_value],
        )
        .await?;
        Ok(())
    }

    /// List conversations that are linked to an actor across channels.
    ///
    /// When `include_group_history` is false, this intentionally filters to
    /// direct conversations only so automatic recall never pulls group history.
    pub async fn list_actor_conversations_for_recall(
        &self,
        principal_id: &str,
        actor_id: &str,
        include_group_history: bool,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        let conn = self.conn().await?;
        let kind_filter = if include_group_history {
            "('direct', 'group')"
        } else {
            "('direct')"
        };
        let rows = conn
            .query(
                &format!(
                    r#"
                    SELECT
                        c.id,
                        c.user_id,
                        c.actor_id,
                        c.conversation_scope_id,
                        c.conversation_kind,
                        c.channel,
                        c.started_at,
                        c.last_activity,
                        c.metadata,
                        c.stable_external_conversation_key,
                        (SELECT COUNT(*) FROM conversation_messages m WHERE m.conversation_id = c.id) AS message_count,
                        (SELECT LEFT(m2.content, 100)
                         FROM conversation_messages m2
                         WHERE m2.conversation_id = c.id AND m2.role = 'user'
                         ORDER BY m2.created_at ASC
                         LIMIT 1
                        ) AS title
                    FROM conversations c
                    WHERE c.user_id = $1
                      AND c.actor_id = $2
                      AND c.conversation_kind IN {}
                    ORDER BY c.last_activity DESC
                    LIMIT $3
                    "#,
                    kind_filter
                ),
                &[&principal_id, &actor_id, &limit],
            )
            .await?;

        Ok(rows.iter().map(conversation_summary_from_row).collect())
    }

    /// Check whether a conversation belongs to the given actor.
    pub async fn conversation_belongs_to_actor(
        &self,
        conversation_id: Uuid,
        principal_id: &str,
        actor_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                r#"
                SELECT 1 FROM conversations
                WHERE id = $1
                  AND user_id = $2
                  AND (
                    actor_id = $3
                    OR ((actor_id IS NULL OR btrim(actor_id) = '') AND $3 = $2)
                  )
                "#,
                &[&conversation_id, &principal_id, &actor_id],
            )
            .await?;
        Ok(row.is_some())
    }

    // ==================== Jobs ====================

    /// Save a job context to the database.
    pub async fn save_job(&self, ctx: &JobContext) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        let status = ctx.state.to_string();
        let estimated_time_secs = ctx.estimated_duration.map(|d| d.as_secs() as i32);
        let total_tokens_used = ctx.total_tokens_used.min(i64::MAX as u64) as i64;
        let max_tokens = ctx.max_tokens.min(i64::MAX as u64) as i64;
        let transitions = serde_json::to_value(&ctx.transitions)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        conn.execute(
            r#"
            INSERT INTO agent_jobs (
                id, conversation_id, title, description, category, status, source, user_id, principal_id, actor_id,
                budget_amount, budget_token, bid_amount, estimated_cost, estimated_time_secs,
                actual_cost, total_tokens_used, max_tokens, metadata, transitions,
                repair_attempts, created_at, started_at, completed_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24)
            ON CONFLICT (id) DO UPDATE SET
                title = EXCLUDED.title,
                description = EXCLUDED.description,
                category = EXCLUDED.category,
                status = EXCLUDED.status,
                user_id = EXCLUDED.user_id,
                principal_id = EXCLUDED.principal_id,
                actor_id = EXCLUDED.actor_id,
                estimated_cost = EXCLUDED.estimated_cost,
                estimated_time_secs = EXCLUDED.estimated_time_secs,
                actual_cost = EXCLUDED.actual_cost,
                total_tokens_used = EXCLUDED.total_tokens_used,
                max_tokens = EXCLUDED.max_tokens,
                metadata = EXCLUDED.metadata,
                transitions = EXCLUDED.transitions,
                repair_attempts = EXCLUDED.repair_attempts,
                started_at = EXCLUDED.started_at,
                completed_at = EXCLUDED.completed_at
            "#,
            &[
                &ctx.job_id,
                &ctx.conversation_id,
                &ctx.title,
                &ctx.description,
                &ctx.category,
                &status,
                &"direct", // source
                &ctx.user_id,
                &ctx.principal_id,
                &ctx.actor_id,
                &ctx.budget,
                &ctx.budget_token,
                &ctx.bid_amount,
                &ctx.estimated_cost,
                &estimated_time_secs,
                &ctx.actual_cost,
                &total_tokens_used,
                &max_tokens,
                &ctx.metadata,
                &transitions,
                &(ctx.repair_attempts as i32),
                &ctx.created_at,
                &ctx.started_at,
                &ctx.completed_at,
            ],
        )
        .await?;

        Ok(())
    }

    /// Get a job by ID.
    pub async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError> {
        let conn = self.conn().await?;

        let row = conn
            .query_opt(
                r#"
                SELECT id, conversation_id, title, description, category, status, user_id, principal_id, actor_id,
                       budget_amount, budget_token, bid_amount, estimated_cost, estimated_time_secs,
                       actual_cost, total_tokens_used, max_tokens, metadata, transitions,
                       repair_attempts, created_at, started_at, completed_at
                FROM agent_jobs WHERE id = $1
                "#,
                &[&id],
            )
            .await?;

        match row {
            Some(row) => {
                let status_str: String = row.get("status");
                let state = parse_job_state(&status_str);
                let estimated_time_secs: Option<i32> = row.get("estimated_time_secs");
                let transitions_json: serde_json::Value = row.get("transitions");
                let transitions = serde_json::from_value::<Vec<StateTransition>>(transitions_json)
                    .unwrap_or_default();
                let metadata: serde_json::Value = row.get("metadata");

                Ok(Some(JobContext {
                    job_id: row.get("id"),
                    state,
                    user_id: row.get::<_, String>("user_id"),
                    principal_id: row.get::<_, String>("principal_id"),
                    actor_id: row.get("actor_id"),
                    conversation_id: row.get("conversation_id"),
                    title: row.get("title"),
                    description: row.get("description"),
                    category: row.get("category"),
                    budget: row.get("budget_amount"),
                    budget_token: row.get("budget_token"),
                    bid_amount: row.get("bid_amount"),
                    estimated_cost: row.get("estimated_cost"),
                    estimated_duration: estimated_time_secs
                        .map(|s| std::time::Duration::from_secs(s as u64)),
                    actual_cost: row
                        .get::<_, Option<Decimal>>("actual_cost")
                        .unwrap_or_default(),
                    total_tokens_used: row.get::<_, i64>("total_tokens_used").max(0) as u64,
                    max_tokens: row.get::<_, i64>("max_tokens").max(0) as u64,
                    repair_attempts: row.get::<_, i32>("repair_attempts") as u32,
                    created_at: row.get("created_at"),
                    started_at: row.get("started_at"),
                    completed_at: row.get("completed_at"),
                    transitions,
                    metadata,
                    extra_env: std::sync::Arc::new(std::collections::HashMap::new()),
                }))
            }
            None => Ok(None),
        }
    }

    /// Update job status.
    pub async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let status_str = status.to_string();

        conn.execute(
            "UPDATE agent_jobs SET status = $2, failure_reason = $3 WHERE id = $1",
            &[&id, &status_str, &failure_reason],
        )
        .await?;

        Ok(())
    }

    /// Mark job as stuck.
    pub async fn mark_job_stuck(&self, id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        conn.execute(
            "UPDATE agent_jobs SET status = 'stuck', stuck_since = NOW() WHERE id = $1",
            &[&id],
        )
        .await?;

        Ok(())
    }

    /// Get stuck jobs.
    pub async fn get_stuck_jobs(&self) -> Result<Vec<Uuid>, DatabaseError> {
        let conn = self.conn().await?;

        let rows = conn
            .query("SELECT id FROM agent_jobs WHERE status = 'stuck'", &[])
            .await?;

        Ok(rows.iter().map(|r| r.get("id")).collect())
    }

    // ==================== Actions ====================

    /// Save a job action.
    pub async fn save_action(
        &self,
        job_id: Uuid,
        action: &ActionRecord,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        let duration_ms = action.duration.as_millis() as i32;
        let warnings_json = serde_json::to_value(&action.sanitization_warnings)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        conn.execute(
            r#"
            INSERT INTO job_actions (
                id, job_id, sequence_num, tool_name, input, output_raw, output_sanitized,
                sanitization_warnings, cost, duration_ms, success, error_message, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            "#,
            &[
                &action.id,
                &job_id,
                &(action.sequence as i32),
                &action.tool_name,
                &action.input,
                &action.output_raw,
                &action.output_sanitized,
                &warnings_json,
                &action.cost,
                &duration_ms,
                &action.success,
                &action.error,
                &action.executed_at,
            ],
        )
        .await?;

        Ok(())
    }

    /// Get actions for a job.
    pub async fn get_job_actions(&self, job_id: Uuid) -> Result<Vec<ActionRecord>, DatabaseError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT id, sequence_num, tool_name, input, output_raw, output_sanitized,
                       sanitization_warnings, cost, duration_ms, success, error_message, created_at
                FROM job_actions WHERE job_id = $1 ORDER BY sequence_num
                "#,
                &[&job_id],
            )
            .await?;

        let mut actions = Vec::new();
        for row in rows {
            let duration_ms: i32 = row.get("duration_ms");
            let warnings_json: serde_json::Value = row.get("sanitization_warnings");
            let warnings: Vec<String> = serde_json::from_value(warnings_json).unwrap_or_default();

            actions.push(ActionRecord {
                id: row.get("id"),
                sequence: row.get::<_, i32>("sequence_num") as u32,
                tool_name: row.get("tool_name"),
                input: row.get("input"),
                output_raw: row.get("output_raw"),
                output_sanitized: row.get("output_sanitized"),
                sanitization_warnings: warnings,
                cost: row.get("cost"),
                duration: std::time::Duration::from_millis(duration_ms as u64),
                success: row.get("success"),
                error: row.get("error_message"),
                executed_at: row.get("created_at"),
            });
        }

        Ok(actions)
    }

    // ==================== LLM Calls ====================

    /// Record an LLM call.
    pub async fn record_llm_call(&self, record: &LlmCallRecord<'_>) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = Uuid::new_v4();

        conn.execute(
            r#"
            INSERT INTO llm_calls (id, job_id, conversation_id, provider, model, input_tokens, output_tokens, cost, purpose)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
            &[
                &id,
                &record.job_id,
                &record.conversation_id,
                &record.provider,
                &record.model,
                &(record.input_tokens as i32),
                &(record.output_tokens as i32),
                &record.cost,
                &record.purpose,
            ],
        )
        .await?;

        Ok(id)
    }

    // ==================== Estimation Snapshots ====================

    /// Save an estimation snapshot for learning.
    pub async fn save_estimation_snapshot(
        &self,
        job_id: Uuid,
        category: &str,
        tool_names: &[String],
        estimated_cost: Decimal,
        estimated_time_secs: i32,
        estimated_value: Decimal,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = Uuid::new_v4();

        conn.execute(
            r#"
            INSERT INTO estimation_snapshots (id, job_id, category, tool_names, estimated_cost, estimated_time_secs, estimated_value)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
            &[
                &id,
                &job_id,
                &category,
                &tool_names,
                &estimated_cost,
                &estimated_time_secs,
                &estimated_value,
            ],
        )
        .await?;

        Ok(id)
    }

    /// Update estimation snapshot with actual values.
    pub async fn update_estimation_actuals(
        &self,
        id: Uuid,
        actual_cost: Decimal,
        actual_time_secs: i32,
        actual_value: Option<Decimal>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        conn.execute(
            "UPDATE estimation_snapshots SET actual_cost = $2, actual_time_secs = $3, actual_value = $4 WHERE id = $1",
            &[&id, &actual_cost, &actual_time_secs, &actual_value],
        )
        .await?;

        Ok(())
    }
}

// ==================== Sandbox Jobs ====================

/// Record for a sandbox container job, persisted in the `agent_jobs` table
/// with `source = 'sandbox'`.
#[derive(Debug, Clone)]
pub struct SandboxJobRecord {
    pub id: Uuid,
    pub task: String,
    pub status: String,
    pub user_id: String,
    pub actor_id: String,
    pub project_dir: String,
    pub success: Option<bool>,
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    /// Serialized JSON of `Vec<CredentialGrant>` for restart support.
    /// Stored in the `description` column of `agent_jobs` (unused for sandbox jobs).
    pub credential_grants_json: String,
}

/// Summary of sandbox job counts grouped by status.
#[derive(Debug, Clone, Default)]
pub struct SandboxJobSummary {
    pub total: usize,
    pub creating: usize,
    pub running: usize,
    pub completed: usize,
    pub failed: usize,
    pub interrupted: usize,
}

#[cfg(feature = "postgres")]
impl Store {
    /// Insert a new sandbox job into `agent_jobs`.
    pub async fn save_sandbox_job(&self, job: &SandboxJobRecord) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO agent_jobs (
                id, title, description, status, source, user_id, actor_id, project_dir,
                success, failure_reason, created_at, started_at, completed_at
            ) VALUES ($1, $2, $3, $4, 'sandbox', $5, $6, $7, $8, $9, $10, $11, $12)
            ON CONFLICT (id) DO UPDATE SET
                status = EXCLUDED.status,
                success = EXCLUDED.success,
                failure_reason = EXCLUDED.failure_reason,
                actor_id = EXCLUDED.actor_id,
                started_at = EXCLUDED.started_at,
                completed_at = EXCLUDED.completed_at
            "#,
            &[
                &job.id,
                &job.task,
                &job.credential_grants_json,
                &job.status,
                &job.user_id,
                &job.actor_id,
                &job.project_dir,
                &job.success,
                &job.failure_reason,
                &job.created_at,
                &job.started_at,
                &job.completed_at,
            ],
        )
        .await?;
        Ok(())
    }

    /// Get a sandbox job by ID.
    pub async fn get_sandbox_job(
        &self,
        id: Uuid,
    ) -> Result<Option<SandboxJobRecord>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                r#"
                SELECT id, title, description, status, user_id, actor_id, project_dir,
                       success, failure_reason, created_at, started_at, completed_at
                FROM agent_jobs WHERE id = $1 AND source = 'sandbox'
                "#,
                &[&id],
            )
            .await?;

        Ok(row.map(|r| SandboxJobRecord {
            id: r.get("id"),
            task: r.get("title"),
            status: r.get("status"),
            user_id: r.get("user_id"),
            actor_id: r.get("actor_id"),
            project_dir: r
                .get::<_, Option<String>>("project_dir")
                .unwrap_or_default(),
            success: r.get("success"),
            failure_reason: r.get("failure_reason"),
            created_at: r.get("created_at"),
            started_at: r.get("started_at"),
            completed_at: r.get("completed_at"),
            credential_grants_json: r.get::<_, String>("description"),
        }))
    }

    /// List all sandbox jobs, most recent first.
    pub async fn list_sandbox_jobs(&self) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT id, title, description, status, user_id, actor_id, project_dir,
                       success, failure_reason, created_at, started_at, completed_at
                FROM agent_jobs WHERE source = 'sandbox'
                ORDER BY created_at DESC
                "#,
                &[],
            )
            .await?;

        Ok(rows
            .iter()
            .map(|r| SandboxJobRecord {
                id: r.get("id"),
                task: r.get("title"),
                status: r.get("status"),
                user_id: r.get("user_id"),
                actor_id: r.get("actor_id"),
                project_dir: r
                    .get::<_, Option<String>>("project_dir")
                    .unwrap_or_default(),
                success: r.get("success"),
                failure_reason: r.get("failure_reason"),
                created_at: r.get("created_at"),
                started_at: r.get("started_at"),
                completed_at: r.get("completed_at"),
                credential_grants_json: r.get::<_, String>("description"),
            })
            .collect())
    }

    /// List sandbox jobs for a specific user, most recent first.
    pub async fn list_sandbox_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT id, title, description, status, user_id, actor_id, project_dir,
                       success, failure_reason, created_at, started_at, completed_at
                FROM agent_jobs WHERE source = 'sandbox' AND user_id = $1
                ORDER BY created_at DESC
                "#,
                &[&user_id],
            )
            .await?;

        Ok(rows
            .iter()
            .map(|r| SandboxJobRecord {
                id: r.get("id"),
                task: r.get("title"),
                status: r.get("status"),
                user_id: r.get("user_id"),
                actor_id: r.get("actor_id"),
                project_dir: r
                    .get::<_, Option<String>>("project_dir")
                    .unwrap_or_default(),
                success: r.get("success"),
                failure_reason: r.get("failure_reason"),
                created_at: r.get("created_at"),
                started_at: r.get("started_at"),
                completed_at: r.get("completed_at"),
                credential_grants_json: r.get::<_, String>("description"),
            })
            .collect())
    }

    /// Get a summary of sandbox job counts by status for a specific user.
    pub async fn sandbox_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<SandboxJobSummary, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT status, COUNT(*) as cnt FROM agent_jobs WHERE source = 'sandbox' AND user_id = $1 GROUP BY status",
                &[&user_id],
            )
            .await?;

        let mut summary = SandboxJobSummary::default();
        for row in &rows {
            let status: String = row.get("status");
            let count: i64 = row.get("cnt");
            let c = count as usize;
            summary.total += c;
            match status.as_str() {
                "creating" => summary.creating += c,
                "running" => summary.running += c,
                "completed" => summary.completed += c,
                "failed" => summary.failed += c,
                "interrupted" => summary.interrupted += c,
                _ => {}
            }
        }
        Ok(summary)
    }

    /// Check if a sandbox job belongs to a specific user.
    pub async fn sandbox_job_belongs_to_user(
        &self,
        job_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                "SELECT 1 FROM agent_jobs WHERE id = $1 AND user_id = $2 AND source = 'sandbox'",
                &[&job_id, &user_id],
            )
            .await?;
        Ok(row.is_some())
    }

    /// Update sandbox job status and optional timestamps/result.
    pub async fn update_sandbox_job_status(
        &self,
        id: Uuid,
        status: &str,
        success: Option<bool>,
        message: Option<&str>,
        started_at: Option<DateTime<Utc>>,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE agent_jobs SET
                status = $2,
                success = COALESCE($3, success),
                failure_reason = COALESCE($4, failure_reason),
                started_at = COALESCE($5, started_at),
                completed_at = COALESCE($6, completed_at)
            WHERE id = $1 AND source = 'sandbox'
            "#,
            &[&id, &status, &success, &message, &started_at, &completed_at],
        )
        .await?;
        Ok(())
    }

    /// Mark any sandbox jobs left in "running" or "creating" as "interrupted".
    ///
    /// Called on startup to handle jobs that were running when the process died.
    pub async fn cleanup_stale_sandbox_jobs(&self) -> Result<u64, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn
            .execute(
                r#"
                UPDATE agent_jobs SET
                    status = 'interrupted',
                    failure_reason = 'Process restarted',
                    completed_at = NOW()
                WHERE source = 'sandbox' AND status IN ('running', 'creating')
                "#,
                &[],
            )
            .await?;
        if count > 0 {
            tracing::info!("Marked {} stale sandbox jobs as interrupted", count);
        }
        Ok(count)
    }

    /// Get a summary of sandbox job counts by status.
    pub async fn sandbox_job_summary(&self) -> Result<SandboxJobSummary, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT status, COUNT(*) as cnt FROM agent_jobs WHERE source = 'sandbox' GROUP BY status",
                &[],
            )
            .await?;

        let mut summary = SandboxJobSummary::default();
        for row in &rows {
            let status: String = row.get("status");
            let count: i64 = row.get("cnt");
            let c = count as usize;
            summary.total += c;
            match status.as_str() {
                "creating" => summary.creating += c,
                "running" => summary.running += c,
                "completed" => summary.completed += c,
                "failed" => summary.failed += c,
                "interrupted" => summary.interrupted += c,
                _ => {}
            }
        }
        Ok(summary)
    }
}

// ==================== Job Events ====================

/// A persisted job streaming event (from worker or Claude Code bridge).
#[derive(Debug, Clone)]
pub struct JobEventRecord {
    pub id: i64,
    pub job_id: Uuid,
    pub event_type: String,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[cfg(feature = "postgres")]
impl Store {
    /// Persist a job event (fire-and-forget from orchestrator handler).
    pub async fn save_job_event(
        &self,
        job_id: Uuid,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO job_events (job_id, event_type, data)
            VALUES ($1, $2, $3)
            "#,
            &[&job_id, &event_type, data],
        )
        .await?;
        Ok(())
    }

    /// Load job events for a job, ordered by id.
    ///
    /// When `limit` is `Some(n)`, returns the **most recent** `n` events
    /// (ordered ascending by id). When `None`, returns all events.
    pub async fn list_job_events(
        &self,
        job_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<JobEventRecord>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = if let Some(n) = limit {
            // Sub-select the last N rows by id DESC, then re-sort ASC.
            conn.query(
                r#"
                SELECT id, job_id, event_type, data, created_at
                FROM (
                    SELECT id, job_id, event_type, data, created_at
                    FROM job_events
                    WHERE job_id = $1
                    ORDER BY id DESC
                    LIMIT $2
                ) sub
                ORDER BY id ASC
                "#,
                &[&job_id, &n],
            )
            .await?
        } else {
            conn.query(
                r#"
                SELECT id, job_id, event_type, data, created_at
                FROM job_events
                WHERE job_id = $1
                ORDER BY id ASC
                "#,
                &[&job_id],
            )
            .await?
        };
        Ok(rows
            .iter()
            .map(|r| JobEventRecord {
                id: r.get("id"),
                job_id: r.get("job_id"),
                event_type: r.get("event_type"),
                data: r.get("data"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    /// Update the job_mode column for a sandbox job.
    pub async fn update_sandbox_job_mode(&self, id: Uuid, mode: &str) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            "UPDATE agent_jobs SET job_mode = $2 WHERE id = $1",
            &[&id, &mode],
        )
        .await?;
        Ok(())
    }

    /// Get the job_mode for a sandbox job.
    pub async fn get_sandbox_job_mode(&self, id: Uuid) -> Result<Option<String>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt("SELECT job_mode FROM agent_jobs WHERE id = $1", &[&id])
            .await?;
        Ok(row.map(|r| r.get("job_mode")))
    }
}

// ==================== Routines ====================

#[cfg(feature = "postgres")]
use crate::agent::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineGuardrails, RoutineRun, RunStatus, Trigger,
};

#[cfg(feature = "postgres")]
impl Store {
    /// Create a new routine.
    pub async fn create_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let trigger_type = routine.trigger.type_tag();
        let trigger_config = routine.trigger.to_config_json();
        let action_type = routine.action.type_tag();
        let action_config = routine.action.to_config_json();
        let cooldown_secs = routine.guardrails.cooldown.as_secs() as i32;
        let max_concurrent = routine.guardrails.max_concurrent as i32;
        let dedup_window_secs = routine.guardrails.dedup_window.map(|d| d.as_secs() as i32);

        conn.execute(
            r#"
            INSERT INTO routines (
                id, name, description, user_id, actor_id, enabled,
                trigger_type, trigger_config, action_type, action_config,
                cooldown_secs, max_concurrent, dedup_window_secs,
                notify_channel, notify_user, notify_on_success, notify_on_failure, notify_on_attention,
                state, next_fire_at, created_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9, $10,
                $11, $12, $13,
                $14, $15, $16, $17, $18,
                $19, $20, $21, $22
            )
            "#,
            &[
                &routine.id,
                &routine.name,
                &routine.description,
                &routine.user_id,
                &routine.actor_id,
                &routine.enabled,
                &trigger_type,
                &trigger_config,
                &action_type,
                &action_config,
                &cooldown_secs,
                &max_concurrent,
                &dedup_window_secs,
                &routine.notify.channel,
                &routine.notify.user,
                &routine.notify.on_success,
                &routine.notify.on_failure,
                &routine.notify.on_attention,
                &routine.state,
                &routine.next_fire_at,
                &routine.created_at,
                &routine.updated_at,
            ],
        )
        .await?;

        Ok(())
    }

    /// Get a routine by ID.
    pub async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt("SELECT * FROM routines WHERE id = $1", &[&id])
            .await?;
        row.map(|r| row_to_routine(&r)).transpose()
    }

    /// Get a routine by user_id and name.
    pub async fn get_routine_by_name(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                "SELECT * FROM routines WHERE user_id = $1 AND name = $2",
                &[&user_id, &name],
            )
            .await?;
        row.map(|r| row_to_routine(&r)).transpose()
    }

    /// List routines for a user.
    pub async fn list_routines(&self, user_id: &str) -> Result<Vec<Routine>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT * FROM routines WHERE user_id = $1 ORDER BY name",
                &[&user_id],
            )
            .await?;
        rows.iter().map(row_to_routine).collect()
    }

    /// List all enabled routines with event triggers (for event matching).
    pub async fn list_event_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT * FROM routines WHERE enabled AND trigger_type = 'event'",
                &[],
            )
            .await?;
        rows.iter().map(row_to_routine).collect()
    }

    /// List all enabled cron/system_event routines whose next_fire_at <= now.
    pub async fn list_due_cron_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        let conn = self.conn().await?;
        let now = Utc::now();
        let rows = conn
            .query(
                r#"
                SELECT * FROM routines
                WHERE enabled
                  AND trigger_type IN ('cron', 'system_event')
                  AND next_fire_at IS NOT NULL
                  AND next_fire_at <= $1
                "#,
                &[&now],
            )
            .await?;
        rows.iter().map(row_to_routine).collect()
    }

    /// Update a routine (full replacement of mutable fields).
    pub async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let trigger_type = routine.trigger.type_tag();
        let trigger_config = routine.trigger.to_config_json();
        let action_type = routine.action.type_tag();
        let action_config = routine.action.to_config_json();
        let cooldown_secs = routine.guardrails.cooldown.as_secs() as i32;
        let max_concurrent = routine.guardrails.max_concurrent as i32;
        let dedup_window_secs = routine.guardrails.dedup_window.map(|d| d.as_secs() as i32);

        conn.execute(
            r#"
            UPDATE routines SET
                name = $2, description = $3, actor_id = $4, enabled = $5,
                trigger_type = $6, trigger_config = $7,
                action_type = $8, action_config = $9,
                cooldown_secs = $10, max_concurrent = $11, dedup_window_secs = $12,
                notify_channel = $13, notify_user = $14,
                notify_on_success = $15, notify_on_failure = $16, notify_on_attention = $17,
                state = $18, next_fire_at = $19,
                updated_at = now()
            WHERE id = $1
            "#,
            &[
                &routine.id,
                &routine.name,
                &routine.description,
                &routine.actor_id,
                &routine.enabled,
                &trigger_type,
                &trigger_config,
                &action_type,
                &action_config,
                &cooldown_secs,
                &max_concurrent,
                &dedup_window_secs,
                &routine.notify.channel,
                &routine.notify.user,
                &routine.notify.on_success,
                &routine.notify.on_failure,
                &routine.notify.on_attention,
                &routine.state,
                &routine.next_fire_at,
            ],
        )
        .await?;
        Ok(())
    }

    /// Update runtime state after a routine fires.
    pub async fn update_routine_runtime(
        &self,
        id: Uuid,
        last_run_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
        run_count: u64,
        consecutive_failures: u32,
        state: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE routines SET
                last_run_at = $2, next_fire_at = $3,
                run_count = $4, consecutive_failures = $5,
                state = $6, updated_at = now()
            WHERE id = $1
            "#,
            &[
                &id,
                &last_run_at,
                &next_fire_at,
                &(run_count as i64),
                &(consecutive_failures as i32),
                state,
            ],
        )
        .await?;
        Ok(())
    }

    /// Delete a routine.
    pub async fn delete_routine(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn
            .execute("DELETE FROM routines WHERE id = $1", &[&id])
            .await?;
        Ok(count > 0)
    }

    // ==================== Routine Runs ====================

    /// Record a routine run starting.
    pub async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let status = run.status.to_string();
        conn.execute(
            r#"
            INSERT INTO routine_runs (
                id, routine_id, trigger_type, trigger_detail,
                started_at, status, job_id
            ) VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
            &[
                &run.id,
                &run.routine_id,
                &run.trigger_type,
                &run.trigger_detail,
                &run.started_at,
                &status,
                &run.job_id,
            ],
        )
        .await?;
        Ok(())
    }

    /// Complete a routine run.
    pub async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let status_str = status.to_string();
        let now = Utc::now();
        conn.execute(
            r#"
            UPDATE routine_runs SET
                completed_at = $2, status = $3,
                result_summary = $4, tokens_used = $5
            WHERE id = $1
            "#,
            &[&id, &now, &status_str, &result_summary, &tokens_used],
        )
        .await?;
        Ok(())
    }

    /// List recent runs for a routine.
    pub async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT * FROM routine_runs
                WHERE routine_id = $1
                ORDER BY started_at DESC
                LIMIT $2
                "#,
                &[&routine_id, &limit],
            )
            .await?;
        rows.iter().map(row_to_routine_run).collect()
    }

    /// Count currently running runs for a routine.
    pub async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_one(
                "SELECT COUNT(*) as cnt FROM routine_runs WHERE routine_id = $1 AND status = 'running'",
                &[&routine_id],
            )
            .await?;
        Ok(row.get("cnt"))
    }

    /// Link a routine run to a dispatched job.
    pub async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            "UPDATE routine_runs SET job_id = $1 WHERE id = $2",
            &[&job_id, &run_id],
        )
        .await?;
        Ok(())
    }

    /// Mark all RUNNING routine runs as failed (startup cleanup).
    pub async fn cleanup_stale_routine_runs(&self) -> Result<u64, DatabaseError> {
        let conn = self.conn().await?;
        let now = Utc::now();
        let count = conn
            .execute(
                r#"
                UPDATE routine_runs SET
                    status = 'failed',
                    completed_at = $1,
                    result_summary = 'Orphaned: process restarted while routine was running'
                WHERE status = 'running'
                "#,
                &[&now],
            )
            .await?;
        Ok(count)
    }

    pub async fn delete_routine_runs(&self, routine_id: Uuid) -> Result<u64, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn
            .execute(
                "DELETE FROM routine_runs WHERE routine_id = $1",
                &[&routine_id],
            )
            .await?;
        Ok(count)
    }

    pub async fn delete_all_routine_runs(&self) -> Result<u64, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn.execute("DELETE FROM routine_runs", &[]).await?;
        Ok(count)
    }
}

#[cfg(feature = "postgres")]
fn row_to_routine(row: &tokio_postgres::Row) -> Result<Routine, DatabaseError> {
    let trigger_type: String = row.get("trigger_type");
    let trigger_config: serde_json::Value = row.get("trigger_config");
    let action_type: String = row.get("action_type");
    let action_config: serde_json::Value = row.get("action_config");
    let cooldown_secs: i32 = row.get("cooldown_secs");
    let max_concurrent: i32 = row.get("max_concurrent");
    let dedup_window_secs: Option<i32> = row.get("dedup_window_secs");

    let trigger = Trigger::from_db(&trigger_type, trigger_config)
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    let action = RoutineAction::from_db(&action_type, action_config)
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

    Ok(Routine {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        user_id: row.get("user_id"),
        actor_id: row
            .try_get::<_, Option<String>>("actor_id")
            .ok()
            .flatten()
            .unwrap_or_else(|| row.get("user_id")),
        enabled: row.get("enabled"),
        trigger,
        action,
        guardrails: RoutineGuardrails {
            cooldown: std::time::Duration::from_secs(cooldown_secs as u64),
            max_concurrent: max_concurrent as u32,
            dedup_window: dedup_window_secs.map(|s| std::time::Duration::from_secs(s as u64)),
        },
        notify: NotifyConfig {
            channel: row.get("notify_channel"),
            user: row.get("notify_user"),
            on_attention: row.get("notify_on_attention"),
            on_failure: row.get("notify_on_failure"),
            on_success: row.get("notify_on_success"),
        },
        last_run_at: row.get("last_run_at"),
        next_fire_at: row.get("next_fire_at"),
        run_count: row.get::<_, i64>("run_count") as u64,
        consecutive_failures: row.get::<_, i32>("consecutive_failures") as u32,
        state: row.get("state"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

#[cfg(feature = "postgres")]
fn row_to_routine_run(row: &tokio_postgres::Row) -> Result<RoutineRun, DatabaseError> {
    let status_str: String = row.get("status");
    let status: RunStatus = status_str
        .parse()
        .map_err(|e: crate::error::RoutineError| DatabaseError::Serialization(e.to_string()))?;

    Ok(RoutineRun {
        id: row.get("id"),
        routine_id: row.get("routine_id"),
        trigger_type: row.get("trigger_type"),
        trigger_detail: row.get("trigger_detail"),
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
        status,
        result_summary: row.get("result_summary"),
        tokens_used: row.get("tokens_used"),
        job_id: row.get("job_id"),
        created_at: row.get("created_at"),
    })
}

// ==================== Conversation Persistence ====================

/// Summary of a conversation for the thread list.
#[derive(Debug, Clone)]
pub struct ConversationSummary {
    pub id: Uuid,
    pub user_id: String,
    pub actor_id: Option<String>,
    pub conversation_scope_id: Option<Uuid>,
    pub conversation_kind: ConversationKind,
    pub channel: String,
    /// First user message, truncated to 100 chars.
    pub title: Option<String>,
    pub message_count: i64,
    pub started_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    /// Thread type extracted from metadata (e.g. "assistant", "thread").
    pub thread_type: Option<String>,
    pub handoff: Option<ConversationHandoffMetadata>,
    pub stable_external_conversation_key: Option<String>,
}

/// A single message in a conversation.
#[derive(Debug, Clone)]
pub struct ConversationMessage {
    pub id: Uuid,
    pub role: String,
    pub content: String,
    pub actor_id: Option<String>,
    pub actor_display_name: Option<String>,
    pub raw_sender_id: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Lightweight linked-DM recall payload used by the prompt assembler.
#[derive(Debug, Clone)]
pub struct LinkedConversationRecall {
    pub principal_id: String,
    pub actor_id: String,
    pub include_group_history: bool,
    pub conversations: Vec<ConversationSummary>,
}

impl LinkedConversationRecall {
    pub fn new(
        principal_id: impl Into<String>,
        actor_id: impl Into<String>,
        include_group_history: bool,
        conversations: Vec<ConversationSummary>,
    ) -> Self {
        Self {
            principal_id: principal_id.into(),
            actor_id: actor_id.into(),
            include_group_history,
            conversations,
        }
    }

    /// Render a compact handoff block that summarizes only the ongoing work.
    pub fn compact_block(&self) -> Option<String> {
        if self.conversations.is_empty() {
            return None;
        }

        let mut lines = vec![format!(
            "Linked recall for actor {} (principal {}):",
            self.actor_id, self.principal_id
        )];

        for convo in &self.conversations {
            let kind = convo.conversation_kind.as_str();
            let handoff = convo
                .handoff
                .as_ref()
                .and_then(|h| h.handoff_summary.as_deref())
                .unwrap_or_default();
            let goal = convo
                .handoff
                .as_ref()
                .and_then(|h| h.last_user_goal.as_deref())
                .unwrap_or_default();
            let state = convo
                .handoff
                .as_ref()
                .and_then(|h| h.task_state.as_deref())
                .unwrap_or_default();

            let mut parts = vec![format!(
                "{} / {} / {} messages",
                convo.channel, kind, convo.message_count
            )];
            if let Some(title) = convo.title.as_deref() {
                parts.push(format!("title={title}"));
            }
            if !goal.is_empty() {
                parts.push(format!("goal={goal}"));
            }
            if !state.is_empty() {
                parts.push(format!("state={state}"));
            }
            if !handoff.is_empty() {
                parts.push(format!("handoff={handoff}"));
            }
            lines.push(format!("- {}", parts.join(" | ")));
        }

        Some(lines.join("\n"))
    }
}

fn conversation_stable_key(channel: &str, thread_id: Option<&str>, fallback: Uuid) -> String {
    match thread_id {
        Some(thread_id) if !thread_id.is_empty() => format!("{channel}:{thread_id}"),
        _ => format!("{channel}:{fallback}"),
    }
}

fn conversation_handoff_from_metadata(
    metadata: &serde_json::Value,
) -> Option<ConversationHandoffMetadata> {
    let value = metadata.get("handoff").cloned().or_else(|| {
        let direct = serde_json::json!({
            "last_actor_id": metadata.get("last_actor_id"),
            "task_state": metadata.get("task_state"),
            "last_user_goal": metadata.get("last_user_goal"),
            "handoff_summary": metadata.get("handoff_summary"),
        });
        if direct
            .as_object()
            .map(|m| m.values().any(|v| !v.is_null()))
            .unwrap_or(false)
        {
            Some(direct)
        } else {
            None
        }
    })?;

    serde_json::from_value(value)
        .ok()
        .filter(|handoff: &ConversationHandoffMetadata| !handoff.is_empty())
}

#[allow(dead_code)] // Prepared for conversation handoff persistence path
fn conversation_metadata_with_handoff(
    metadata: &serde_json::Value,
    handoff: &ConversationHandoffMetadata,
) -> serde_json::Value {
    let mut merged = metadata.clone();
    if merged.is_null() || !merged.is_object() {
        merged = serde_json::json!({});
    }

    let mut handoff_value = match serde_json::to_value(handoff) {
        Ok(value) => value,
        Err(_) => serde_json::json!({}),
    };
    if handoff_value.is_null() {
        handoff_value = serde_json::json!({});
    }

    if let Some(obj) = merged.as_object_mut() {
        obj.insert("handoff".to_string(), handoff_value);
    }
    merged
}

#[cfg(feature = "postgres")]
fn conversation_summary_from_row(row: &tokio_postgres::Row) -> ConversationSummary {
    let metadata: serde_json::Value = row.get("metadata");
    let thread_type = metadata
        .get("thread_type")
        .and_then(|v: &serde_json::Value| v.as_str())
        .map(String::from);
    let handoff = conversation_handoff_from_metadata(&metadata);
    let conversation_kind = ConversationKind::from_db(
        row.try_get::<_, Option<String>>("conversation_kind")
            .ok()
            .flatten()
            .as_deref(),
    );
    let actor_id = row.try_get::<_, Option<String>>("actor_id").ok().flatten();
    let conversation_scope_id = row
        .try_get::<_, Option<Uuid>>("conversation_scope_id")
        .ok()
        .flatten();
    let stable_external_conversation_key = row
        .try_get::<_, Option<String>>("stable_external_conversation_key")
        .ok()
        .flatten();

    ConversationSummary {
        id: row.get("id"),
        user_id: row.get("user_id"),
        actor_id,
        conversation_scope_id,
        conversation_kind,
        channel: row.get("channel"),
        title: row.get("title"),
        message_count: row.get("message_count"),
        started_at: row.get("started_at"),
        last_activity: row.get("last_activity"),
        thread_type,
        handoff,
        stable_external_conversation_key,
    }
}

#[cfg(feature = "postgres")]
fn conversation_message_from_row(row: &tokio_postgres::Row) -> ConversationMessage {
    ConversationMessage {
        id: row.get("id"),
        role: row.get("role"),
        content: row.get("content"),
        actor_id: row.try_get::<_, Option<String>>("actor_id").ok().flatten(),
        actor_display_name: row
            .try_get::<_, Option<String>>("actor_display_name")
            .ok()
            .flatten(),
        raw_sender_id: row
            .try_get::<_, Option<String>>("raw_sender_id")
            .ok()
            .flatten(),
        metadata: row
            .try_get::<_, Option<serde_json::Value>>("metadata")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: row.get("created_at"),
    }
}

#[cfg(feature = "postgres")]
impl Store {
    /// Ensure a conversation row exists for a given UUID.
    ///
    /// Idempotent: inserts on first call, bumps `last_activity` on subsequent calls.
    pub async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO conversations (id, channel, user_id, thread_id)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (id) DO UPDATE SET last_activity = NOW()
            "#,
            &[&id, &channel, &user_id, &thread_id],
        )
        .await?;
        Ok(())
    }

    /// List conversations with a title derived from the first user message.
    pub async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    c.id,
                    c.user_id,
                    c.actor_id,
                    c.conversation_scope_id,
                    c.conversation_kind,
                    c.channel,
                    c.started_at,
                    c.last_activity,
                    c.metadata,
                    c.stable_external_conversation_key,
                    (SELECT COUNT(*) FROM conversation_messages m WHERE m.conversation_id = c.id) AS message_count,
                    (SELECT LEFT(m2.content, 100)
                     FROM conversation_messages m2
                     WHERE m2.conversation_id = c.id AND m2.role = 'user'
                     ORDER BY m2.created_at ASC
                     LIMIT 1
                    ) AS title
                FROM conversations c
                WHERE c.user_id = $1 AND c.channel = $2
                ORDER BY c.last_activity DESC
                LIMIT $3
                "#,
                &[&user_id, &channel, &limit],
            )
            .await?;

        Ok(rows.iter().map(conversation_summary_from_row).collect())
    }

    /// Infer the principal that owns the majority of history for a channel.
    ///
    /// When the placeholder `"default"` principal and a real principal both
    /// exist, this prefers the non-default principal so upgraded gateway chat
    /// UIs reconnect to the historical owner automatically.
    pub async fn infer_primary_user_id_for_channel(
        &self,
        channel: &str,
    ) -> Result<Option<String>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT c.user_id
                FROM conversations c
                WHERE c.channel = $1
                  AND c.user_id IS NOT NULL
                  AND btrim(c.user_id) <> ''
                GROUP BY c.user_id
                ORDER BY COUNT(*) DESC, MAX(c.last_activity) DESC, c.user_id ASC
                LIMIT 2
                "#,
                &[&channel],
            )
            .await?;

        let candidates: Vec<String> = rows
            .iter()
            .filter_map(|row| row.try_get::<_, String>("user_id").ok())
            .filter(|user_id| !user_id.trim().is_empty())
            .collect();

        let Some(primary) = candidates.first() else {
            return Ok(None);
        };

        if primary == "default" && candidates.len() > 1 {
            return Ok(candidates.get(1).cloned());
        }

        Ok(Some(primary.clone()))
    }

    /// Get or create the singleton "assistant" conversation for a user+channel.
    ///
    /// Looks for a conversation where `metadata->>'thread_type' = 'assistant'`.
    /// Creates one if it doesn't exist.
    pub async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;

        // Try to find existing assistant conversation
        let row = conn
            .query_opt(
                r#"
                SELECT id FROM conversations
                WHERE user_id = $1 AND channel = $2 AND metadata->>'thread_type' = 'assistant'
                LIMIT 1
                "#,
                &[&user_id, &channel],
            )
            .await?;

        if let Some(row) = row {
            return Ok(row.get("id"));
        }

        // Create a new assistant conversation
        let id = Uuid::new_v4();
        let metadata = serde_json::json!({"thread_type": "assistant", "title": "Assistant"});
        conn.execute(
            r#"
            INSERT INTO conversations (
                id, channel, user_id, actor_id, conversation_scope_id, conversation_kind,
                stable_external_conversation_key, metadata
            ) VALUES ($1, $2, $3, NULL, $4, $5, $6, $7)
            "#,
            &[
                &id,
                &channel,
                &user_id,
                &id,
                &ConversationKind::Direct.as_str(),
                &conversation_stable_key(channel, Some("assistant"), id),
                &metadata,
            ],
        )
        .await?;

        Ok(id)
    }

    /// Create a conversation with specific metadata.
    pub async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = Uuid::new_v4();
        let stable_external_conversation_key = conversation_stable_key(channel, None, id);

        conn.execute(
            r#"
            INSERT INTO conversations (
                id, channel, user_id, actor_id, conversation_scope_id, conversation_kind,
                stable_external_conversation_key, metadata
            ) VALUES ($1, $2, $3, NULL, $4, $5, $6, $7)
            "#,
            &[
                &id,
                &channel,
                &user_id,
                &id,
                &ConversationKind::Direct.as_str(),
                &stable_external_conversation_key,
                &metadata,
            ],
        )
        .await?;

        Ok(id)
    }

    /// Check whether a conversation belongs to the given user.
    pub async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                "SELECT 1 FROM conversations WHERE id = $1 AND user_id = $2",
                &[&conversation_id, &user_id],
            )
            .await?;
        Ok(row.is_some())
    }

    /// Load messages for a conversation with cursor-based pagination.
    ///
    /// Returns `(messages_oldest_first, has_more)`.
    /// Pass `before` as a cursor to load older messages.
    pub async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError> {
        let conn = self.conn().await?;
        let fetch_limit = limit + 1; // Fetch one extra to determine has_more

        let rows = if let Some(before_ts) = before {
            conn.query(
                r#"
                SELECT id, role, content, actor_id, actor_display_name, raw_sender_id,
                       metadata, created_at
                FROM conversation_messages
                WHERE conversation_id = $1 AND created_at < $2
                ORDER BY created_at DESC
                LIMIT $3
                "#,
                &[&conversation_id, &before_ts, &fetch_limit],
            )
            .await?
        } else {
            conn.query(
                r#"
                SELECT id, role, content, actor_id, actor_display_name, raw_sender_id,
                       metadata, created_at
                FROM conversation_messages
                WHERE conversation_id = $1
                ORDER BY created_at DESC
                LIMIT $2
                "#,
                &[&conversation_id, &fetch_limit],
            )
            .await?
        };

        let has_more = rows.len() as i64 > limit;
        let take_count = (rows.len() as i64).min(limit) as usize;

        // Rows come newest-first from DB; reverse so caller gets oldest-first
        let mut messages: Vec<ConversationMessage> = rows
            .iter()
            .take(take_count)
            .map(conversation_message_from_row)
            .collect();
        messages.reverse();

        Ok((messages, has_more))
    }

    /// Merge a single key into a conversation's metadata JSONB.
    pub async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        if key == "handoff" {
            conn.execute(
                "UPDATE conversations SET metadata = jsonb_set(coalesce(metadata, '{}'::jsonb), '{handoff}', $2::jsonb, true) WHERE id = $1",
                &[&id, &value],
            )
            .await?;
        } else {
            let patch = serde_json::json!({ key: value });
            conn.execute(
                "UPDATE conversations SET metadata = metadata || $2 WHERE id = $1",
                &[&id, &patch],
            )
            .await?;
        }
        Ok(())
    }

    /// Read the metadata JSONB for a conversation.
    pub async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt("SELECT metadata FROM conversations WHERE id = $1", &[&id])
            .await?;
        Ok(row.map(|r| r.get::<_, serde_json::Value>(0)))
    }

    /// Load all messages for a conversation, ordered chronologically.
    pub async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT id, role, content, actor_id, actor_display_name, raw_sender_id,
                       metadata, created_at
                FROM conversation_messages
                WHERE conversation_id = $1
                ORDER BY created_at ASC
                "#,
                &[&conversation_id],
            )
            .await?;

        Ok(rows.iter().map(conversation_message_from_row).collect())
    }

    /// Search conversation messages for a user across transcripts.
    pub async fn search_conversation_messages(
        &self,
        user_id: &str,
        query: &str,
        actor_id: Option<&str>,
        channel: Option<&str>,
        thread_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<SessionSearchHit>, DatabaseError> {
        let query = query.trim();
        if query.is_empty() || limit <= 0 {
            return Ok(Vec::new());
        }

        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    c.id AS conversation_id,
                    m.id AS message_id,
                    c.user_id,
                    c.actor_id,
                    c.channel,
                    c.thread_id,
                    c.conversation_kind,
                    m.role,
                    m.content,
                    LEFT(m.content, 240) AS excerpt,
                    COALESCE(m.metadata, '{}'::jsonb) AS metadata,
                    m.created_at,
                    ts_rank_cd(
                        to_tsvector('simple', COALESCE(m.content, '')),
                        websearch_to_tsquery('simple', $2)
                    )::float8 AS score
                FROM conversation_messages m
                JOIN conversations c ON c.id = m.conversation_id
                WHERE c.user_id = $1
                  AND to_tsvector('simple', COALESCE(m.content, ''))
                        @@ websearch_to_tsquery('simple', $2)
                  AND ($3::text IS NULL OR COALESCE(NULLIF(c.actor_id, ''), c.user_id) = $3)
                  AND ($4::text IS NULL OR c.channel = $4)
                  AND ($5::text IS NULL OR c.thread_id = $5)
                ORDER BY score DESC, m.created_at DESC, m.id DESC
                LIMIT $6
                "#,
                &[&user_id, &query, &actor_id, &channel, &thread_id, &limit],
            )
            .await?;

        Ok(rows.iter().map(session_search_hit_from_row).collect())
    }

    /// Persist a learning event for later retrieval and evaluation.
    pub async fn insert_learning_event(
        &self,
        event: &LearningEvent,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = if event.id.is_nil() {
            Uuid::new_v4()
        } else {
            event.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_events (
                id, user_id, actor_id, channel, thread_id, conversation_id,
                message_id, job_id, event_type, source, payload, metadata, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9, $10, $11::jsonb, $12::jsonb, $13
            )
            "#,
            &[
                &id,
                &event.user_id,
                &event.actor_id,
                &event.channel,
                &event.thread_id,
                &event.conversation_id,
                &event.message_id,
                &event.job_id,
                &event.event_type,
                &event.source,
                &event.payload,
                &event
                    .metadata
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({})),
                &event.created_at,
            ],
        )
        .await?;
        Ok(id)
    }

    /// List learning events for a user, optionally scoped to actor/channel/thread.
    pub async fn list_learning_events(
        &self,
        user_id: &str,
        actor_id: Option<&str>,
        channel: Option<&str>,
        thread_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningEvent>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }

        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    id,
                    user_id,
                    actor_id,
                    channel,
                    thread_id,
                    conversation_id,
                    message_id,
                    job_id,
                    event_type,
                    source,
                    payload,
                    metadata,
                    created_at
                FROM learning_events
                WHERE user_id = $1
                  AND ($2::text IS NULL OR COALESCE(NULLIF(actor_id, ''), user_id) = $2)
                  AND ($3::text IS NULL OR channel = $3)
                  AND ($4::text IS NULL OR thread_id = $4)
                ORDER BY created_at DESC, id DESC
                LIMIT $5
                "#,
                &[&user_id, &actor_id, &channel, &thread_id, &limit],
            )
            .await?;

        Ok(rows.iter().map(learning_event_from_row).collect())
    }

    /// Persist an evaluator record linked to a learning event.
    pub async fn insert_learning_evaluation(
        &self,
        evaluation: &LearningEvaluation,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = if evaluation.id.is_nil() {
            Uuid::new_v4()
        } else {
            evaluation.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_evaluations (
                id, learning_event_id, user_id, evaluator, status, score, details, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7::jsonb, $8
            )
            "#,
            &[
                &id,
                &evaluation.learning_event_id,
                &evaluation.user_id,
                &evaluation.evaluator,
                &evaluation.status,
                &evaluation.score,
                &evaluation.details,
                &evaluation.created_at,
            ],
        )
        .await?;
        Ok(id)
    }

    /// List evaluator records for a user.
    pub async fn list_learning_evaluations(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<LearningEvaluation>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT id, learning_event_id, user_id, evaluator, status, score, details, created_at
                FROM learning_evaluations
                WHERE user_id = $1
                ORDER BY created_at DESC, id DESC
                LIMIT $2
                "#,
                &[&user_id, &limit],
            )
            .await?;
        Ok(rows.iter().map(learning_evaluation_from_row).collect())
    }

    /// Persist a distilled learning candidate.
    pub async fn insert_learning_candidate(
        &self,
        candidate: &LearningCandidate,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = if candidate.id.is_nil() {
            Uuid::new_v4()
        } else {
            candidate.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_candidates (
                id, learning_event_id, user_id, candidate_type, risk_tier, confidence,
                target_type, target_name, summary, proposal, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9, $10::jsonb, $11
            )
            "#,
            &[
                &id,
                &candidate.learning_event_id,
                &candidate.user_id,
                &candidate.candidate_type,
                &candidate.risk_tier,
                &candidate.confidence,
                &candidate.target_type,
                &candidate.target_name,
                &candidate.summary,
                &candidate.proposal,
                &candidate.created_at,
            ],
        )
        .await?;
        Ok(id)
    }

    /// List candidates with optional type/tier filters.
    pub async fn list_learning_candidates(
        &self,
        user_id: &str,
        candidate_type: Option<&str>,
        risk_tier: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningCandidate>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    id, learning_event_id, user_id, candidate_type, risk_tier, confidence,
                    target_type, target_name, summary, proposal, created_at
                FROM learning_candidates
                WHERE user_id = $1
                  AND ($2::text IS NULL OR candidate_type = $2)
                  AND ($3::text IS NULL OR risk_tier = $3)
                ORDER BY created_at DESC, id DESC
                LIMIT $4
                "#,
                &[&user_id, &candidate_type, &risk_tier, &limit],
            )
            .await?;
        Ok(rows.iter().map(learning_candidate_from_row).collect())
    }

    /// Persist a versioned artifact mutation.
    pub async fn insert_learning_artifact_version(
        &self,
        version: &LearningArtifactVersion,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = if version.id.is_nil() {
            Uuid::new_v4()
        } else {
            version.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_artifact_versions (
                id, candidate_id, user_id, artifact_type, artifact_name, version_label,
                status, diff_summary, before_content, after_content, provenance, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9, $10, $11::jsonb, $12
            )
            "#,
            &[
                &id,
                &version.candidate_id,
                &version.user_id,
                &version.artifact_type,
                &version.artifact_name,
                &version.version_label,
                &version.status,
                &version.diff_summary,
                &version.before_content,
                &version.after_content,
                &version.provenance,
                &version.created_at,
            ],
        )
        .await?;
        Ok(id)
    }

    /// List versioned artifacts with optional artifact filters.
    pub async fn list_learning_artifact_versions(
        &self,
        user_id: &str,
        artifact_type: Option<&str>,
        artifact_name: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningArtifactVersion>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    id, candidate_id, user_id, artifact_type, artifact_name, version_label,
                    status, diff_summary, before_content, after_content, provenance, created_at
                FROM learning_artifact_versions
                WHERE user_id = $1
                  AND ($2::text IS NULL OR artifact_type = $2)
                  AND ($3::text IS NULL OR artifact_name = $3)
                ORDER BY created_at DESC, id DESC
                LIMIT $4
                "#,
                &[&user_id, &artifact_type, &artifact_name, &limit],
            )
            .await?;
        Ok(rows.iter().map(learning_artifact_version_from_row).collect())
    }

    /// Persist explicit feedback on a learned target.
    pub async fn insert_learning_feedback(
        &self,
        feedback: &LearningFeedbackRecord,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = if feedback.id.is_nil() {
            Uuid::new_v4()
        } else {
            feedback.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_feedback (
                id, user_id, target_type, target_id, verdict, note, metadata, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7::jsonb, $8
            )
            "#,
            &[
                &id,
                &feedback.user_id,
                &feedback.target_type,
                &feedback.target_id,
                &feedback.verdict,
                &feedback.note,
                &feedback.metadata,
                &feedback.created_at,
            ],
        )
        .await?;
        Ok(id)
    }

    /// List feedback entries, optionally scoped to a target.
    pub async fn list_learning_feedback(
        &self,
        user_id: &str,
        target_type: Option<&str>,
        target_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningFeedbackRecord>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT id, user_id, target_type, target_id, verdict, note, metadata, created_at
                FROM learning_feedback
                WHERE user_id = $1
                  AND ($2::text IS NULL OR target_type = $2)
                  AND ($3::text IS NULL OR target_id = $3)
                ORDER BY created_at DESC, id DESC
                LIMIT $4
                "#,
                &[&user_id, &target_type, &target_id, &limit],
            )
            .await?;
        Ok(rows.iter().map(learning_feedback_from_row).collect())
    }

    /// Persist rollback metadata for a learned artifact.
    pub async fn insert_learning_rollback(
        &self,
        rollback: &LearningRollbackRecord,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = if rollback.id.is_nil() {
            Uuid::new_v4()
        } else {
            rollback.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_rollbacks (
                id, user_id, artifact_type, artifact_name, artifact_version_id, reason, metadata, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7::jsonb, $8
            )
            "#,
            &[
                &id,
                &rollback.user_id,
                &rollback.artifact_type,
                &rollback.artifact_name,
                &rollback.artifact_version_id,
                &rollback.reason,
                &rollback.metadata,
                &rollback.created_at,
            ],
        )
        .await?;
        Ok(id)
    }

    /// List rollback records for a user.
    pub async fn list_learning_rollbacks(
        &self,
        user_id: &str,
        artifact_type: Option<&str>,
        artifact_name: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningRollbackRecord>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    id, user_id, artifact_type, artifact_name, artifact_version_id, reason, metadata, created_at
                FROM learning_rollbacks
                WHERE user_id = $1
                  AND ($2::text IS NULL OR artifact_type = $2)
                  AND ($3::text IS NULL OR artifact_name = $3)
                ORDER BY created_at DESC, id DESC
                LIMIT $4
                "#,
                &[&user_id, &artifact_type, &artifact_name, &limit],
            )
            .await?;
        Ok(rows.iter().map(learning_rollback_from_row).collect())
    }

    /// Persist a code-change proposal package.
    pub async fn insert_learning_code_proposal(
        &self,
        proposal: &LearningCodeProposal,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = if proposal.id.is_nil() {
            Uuid::new_v4()
        } else {
            proposal.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_code_proposals (
                id, learning_event_id, user_id, status, title, rationale, target_files, diff,
                validation_results, rollback_note, confidence, branch_name, pr_url, metadata, created_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7::jsonb, $8,
                $9::jsonb, $10, $11, $12, $13, $14::jsonb, $15, $16
            )
            "#,
            &[
                &id,
                &proposal.learning_event_id,
                &proposal.user_id,
                &proposal.status,
                &proposal.title,
                &proposal.rationale,
                &proposal.target_files,
                &proposal.diff,
                &proposal.validation_results,
                &proposal.rollback_note,
                &proposal.confidence,
                &proposal.branch_name,
                &proposal.pr_url,
                &proposal.metadata,
                &proposal.created_at,
                &proposal.updated_at,
            ],
        )
        .await?;
        Ok(id)
    }

    /// Retrieve a single code proposal belonging to a user.
    pub async fn get_learning_code_proposal(
        &self,
        user_id: &str,
        proposal_id: Uuid,
    ) -> Result<Option<LearningCodeProposal>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                r#"
                SELECT
                    id, learning_event_id, user_id, status, title, rationale, target_files, diff,
                    validation_results, rollback_note, confidence, branch_name, pr_url, metadata, created_at, updated_at
                FROM learning_code_proposals
                WHERE id = $1 AND user_id = $2
                "#,
                &[&proposal_id, &user_id],
            )
            .await?;
        Ok(row.as_ref().map(learning_code_proposal_from_row))
    }

    /// List code proposals for a user with optional status filtering.
    pub async fn list_learning_code_proposals(
        &self,
        user_id: &str,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningCodeProposal>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    id, learning_event_id, user_id, status, title, rationale, target_files, diff,
                    validation_results, rollback_note, confidence, branch_name, pr_url, metadata, created_at, updated_at
                FROM learning_code_proposals
                WHERE user_id = $1
                  AND ($2::text IS NULL OR status = $2)
                ORDER BY created_at DESC, id DESC
                LIMIT $3
                "#,
                &[&user_id, &status, &limit],
            )
            .await?;
        Ok(rows.iter().map(learning_code_proposal_from_row).collect())
    }

    /// Update proposal review status and publish metadata.
    pub async fn update_learning_code_proposal(
        &self,
        proposal_id: Uuid,
        status: &str,
        branch_name: Option<&str>,
        pr_url: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE learning_code_proposals
            SET status = $2,
                branch_name = COALESCE($3, branch_name),
                pr_url = COALESCE($4, pr_url),
                metadata = CASE
                    WHEN $5::jsonb IS NULL THEN metadata
                    ELSE coalesce(metadata, '{}'::jsonb) || $5::jsonb
                END,
                updated_at = NOW()
            WHERE id = $1
            "#,
            &[&proposal_id, &status, &branch_name, &pr_url, &metadata],
        )
        .await?;
        Ok(())
    }
}

#[cfg(feature = "postgres")]
fn session_search_hit_from_row(row: &tokio_postgres::Row) -> SessionSearchHit {
    SessionSearchHit {
        conversation_id: row.get("conversation_id"),
        message_id: row.get("message_id"),
        user_id: row.get("user_id"),
        actor_id: row.try_get::<_, Option<String>>("actor_id").ok().flatten(),
        channel: row.get("channel"),
        thread_id: row.try_get::<_, Option<String>>("thread_id").ok().flatten(),
        conversation_kind: ConversationKind::from_db(
            row.try_get::<_, Option<String>>("conversation_kind")
                .ok()
                .flatten()
                .as_deref(),
        ),
        role: row.get("role"),
        content: row.get("content"),
        excerpt: row.get("excerpt"),
        metadata: row
            .try_get::<_, Option<serde_json::Value>>("metadata")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: row.get("created_at"),
        score: row.try_get::<_, Option<f64>>("score").ok().flatten(),
    }
}

#[cfg(feature = "postgres")]
fn learning_event_from_row(row: &tokio_postgres::Row) -> LearningEvent {
    LearningEvent {
        id: row.get("id"),
        user_id: row.get("user_id"),
        actor_id: row.try_get::<_, Option<String>>("actor_id").ok().flatten(),
        channel: row.try_get::<_, Option<String>>("channel").ok().flatten(),
        thread_id: row.try_get::<_, Option<String>>("thread_id").ok().flatten(),
        conversation_id: row
            .try_get::<_, Option<Uuid>>("conversation_id")
            .ok()
            .flatten(),
        message_id: row.try_get::<_, Option<Uuid>>("message_id").ok().flatten(),
        job_id: row.try_get::<_, Option<Uuid>>("job_id").ok().flatten(),
        event_type: row.get("event_type"),
        source: row.get("source"),
        payload: row.get("payload"),
        metadata: row
            .try_get::<_, Option<serde_json::Value>>("metadata")
            .ok()
            .flatten(),
        created_at: row.get("created_at"),
    }
}

#[cfg(feature = "postgres")]
fn learning_evaluation_from_row(row: &tokio_postgres::Row) -> LearningEvaluation {
    LearningEvaluation {
        id: row.get("id"),
        learning_event_id: row.get("learning_event_id"),
        user_id: row.get("user_id"),
        evaluator: row.get("evaluator"),
        status: row.get("status"),
        score: row.try_get::<_, Option<f64>>("score").ok().flatten(),
        details: row
            .try_get::<_, Option<serde_json::Value>>("details")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: row.get("created_at"),
    }
}

#[cfg(feature = "postgres")]
fn learning_candidate_from_row(row: &tokio_postgres::Row) -> LearningCandidate {
    LearningCandidate {
        id: row.get("id"),
        learning_event_id: row
            .try_get::<_, Option<Uuid>>("learning_event_id")
            .ok()
            .flatten(),
        user_id: row.get("user_id"),
        candidate_type: row.get("candidate_type"),
        risk_tier: row.get("risk_tier"),
        confidence: row.try_get::<_, Option<f64>>("confidence").ok().flatten(),
        target_type: row.try_get::<_, Option<String>>("target_type").ok().flatten(),
        target_name: row.try_get::<_, Option<String>>("target_name").ok().flatten(),
        summary: row.try_get::<_, Option<String>>("summary").ok().flatten(),
        proposal: row
            .try_get::<_, Option<serde_json::Value>>("proposal")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: row.get("created_at"),
    }
}

#[cfg(feature = "postgres")]
fn learning_artifact_version_from_row(row: &tokio_postgres::Row) -> LearningArtifactVersion {
    LearningArtifactVersion {
        id: row.get("id"),
        candidate_id: row.try_get::<_, Option<Uuid>>("candidate_id").ok().flatten(),
        user_id: row.get("user_id"),
        artifact_type: row.get("artifact_type"),
        artifact_name: row.get("artifact_name"),
        version_label: row.try_get::<_, Option<String>>("version_label").ok().flatten(),
        status: row.get("status"),
        diff_summary: row.try_get::<_, Option<String>>("diff_summary").ok().flatten(),
        before_content: row.try_get::<_, Option<String>>("before_content").ok().flatten(),
        after_content: row.try_get::<_, Option<String>>("after_content").ok().flatten(),
        provenance: row
            .try_get::<_, Option<serde_json::Value>>("provenance")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: row.get("created_at"),
    }
}

#[cfg(feature = "postgres")]
fn learning_feedback_from_row(row: &tokio_postgres::Row) -> LearningFeedbackRecord {
    LearningFeedbackRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        target_type: row.get("target_type"),
        target_id: row.get("target_id"),
        verdict: row.get("verdict"),
        note: row.try_get::<_, Option<String>>("note").ok().flatten(),
        metadata: row
            .try_get::<_, Option<serde_json::Value>>("metadata")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: row.get("created_at"),
    }
}

#[cfg(feature = "postgres")]
fn learning_rollback_from_row(row: &tokio_postgres::Row) -> LearningRollbackRecord {
    LearningRollbackRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        artifact_type: row.get("artifact_type"),
        artifact_name: row.get("artifact_name"),
        artifact_version_id: row
            .try_get::<_, Option<Uuid>>("artifact_version_id")
            .ok()
            .flatten(),
        reason: row.get("reason"),
        metadata: row
            .try_get::<_, Option<serde_json::Value>>("metadata")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: row.get("created_at"),
    }
}

#[cfg(feature = "postgres")]
fn learning_code_proposal_from_row(row: &tokio_postgres::Row) -> LearningCodeProposal {
    LearningCodeProposal {
        id: row.get("id"),
        learning_event_id: row
            .try_get::<_, Option<Uuid>>("learning_event_id")
            .ok()
            .flatten(),
        user_id: row.get("user_id"),
        status: row.get("status"),
        title: row.get("title"),
        rationale: row.get("rationale"),
        target_files: row
            .try_get::<_, Option<serde_json::Value>>("target_files")
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
            .unwrap_or_default(),
        diff: row.get("diff"),
        validation_results: row
            .try_get::<_, Option<serde_json::Value>>("validation_results")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        rollback_note: row.try_get::<_, Option<String>>("rollback_note").ok().flatten(),
        confidence: row.try_get::<_, Option<f64>>("confidence").ok().flatten(),
        branch_name: row.try_get::<_, Option<String>>("branch_name").ok().flatten(),
        pr_url: row.try_get::<_, Option<String>>("pr_url").ok().flatten(),
        metadata: row
            .try_get::<_, Option<serde_json::Value>>("metadata")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

#[cfg(feature = "postgres")]
fn parse_job_state(s: &str) -> JobState {
    match s {
        "pending" => JobState::Pending,
        "in_progress" => JobState::InProgress,
        "completed" => JobState::Completed,
        "submitted" => JobState::Submitted,
        "accepted" => JobState::Accepted,
        "failed" => JobState::Failed,
        "stuck" => JobState::Stuck,
        "cancelled" => JobState::Cancelled,
        _ => JobState::Pending,
    }
}

// ==================== Tool Failures ====================

#[cfg(feature = "postgres")]
use crate::agent::BrokenTool;

#[cfg(feature = "postgres")]
impl Store {
    /// Record a tool failure (upsert: increment count if exists).
    pub async fn record_tool_failure(
        &self,
        tool_name: &str,
        error_message: &str,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        conn.execute(
            r#"
            INSERT INTO tool_failures (tool_name, error_message, error_count, last_failure)
            VALUES ($1, $2, 1, NOW())
            ON CONFLICT (tool_name) DO UPDATE SET
                error_message = $2,
                error_count = tool_failures.error_count + 1,
                last_failure = NOW()
            "#,
            &[&tool_name, &error_message],
        )
        .await?;

        Ok(())
    }

    /// Get tools that have failed more than `threshold` times and haven't been repaired.
    pub async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, DatabaseError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT tool_name, error_message, error_count, first_failure, last_failure,
                       last_build_result, repair_attempts
                FROM tool_failures
                WHERE error_count >= $1 AND repaired_at IS NULL
                ORDER BY error_count DESC
                "#,
                &[&threshold],
            )
            .await?;

        Ok(rows
            .iter()
            .map(|row| BrokenTool {
                name: row.get("tool_name"),
                last_error: row.get("error_message"),
                failure_count: row.get::<_, i32>("error_count") as u32,
                first_failure: row.get("first_failure"),
                last_failure: row.get("last_failure"),
                last_build_result: row.get("last_build_result"),
                repair_attempts: row.get::<_, i32>("repair_attempts") as u32,
            })
            .collect())
    }

    /// Mark a tool as repaired.
    pub async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        conn.execute(
            "UPDATE tool_failures SET repaired_at = NOW(), error_count = 0 WHERE tool_name = $1",
            &[&tool_name],
        )
        .await?;

        Ok(())
    }

    /// Increment repair attempts for a tool.
    pub async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        conn.execute(
            "UPDATE tool_failures SET repair_attempts = repair_attempts + 1 WHERE tool_name = $1",
            &[&tool_name],
        )
        .await?;

        Ok(())
    }
}

// ==================== Settings ====================

/// A single setting row from the database.
#[derive(Debug, Clone)]
pub struct SettingRow {
    pub key: String,
    pub value: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

#[cfg(feature = "postgres")]
impl Store {
    /// Get a single setting by key.
    pub async fn get_setting(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                "SELECT value FROM settings WHERE user_id = $1 AND key = $2",
                &[&user_id, &key],
            )
            .await?;
        Ok(row.map(|r| r.get("value")))
    }

    /// Get a single setting with full metadata.
    pub async fn get_setting_full(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<SettingRow>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                "SELECT key, value, updated_at FROM settings WHERE user_id = $1 AND key = $2",
                &[&user_id, &key],
            )
            .await?;
        Ok(row.map(|r| SettingRow {
            key: r.get("key"),
            value: r.get("value"),
            updated_at: r.get("updated_at"),
        }))
    }

    /// Set a single setting (upsert).
    pub async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO settings (user_id, key, value, updated_at)
            VALUES ($1, $2, $3, NOW())
            ON CONFLICT (user_id, key) DO UPDATE SET
                value = EXCLUDED.value,
                updated_at = NOW()
            "#,
            &[&user_id, &key, value],
        )
        .await?;
        Ok(())
    }

    /// Delete a single setting (reset to default).
    pub async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn
            .execute(
                "DELETE FROM settings WHERE user_id = $1 AND key = $2",
                &[&user_id, &key],
            )
            .await?;
        Ok(count > 0)
    }

    /// List all settings for a user (with metadata).
    pub async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingRow>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT key, value, updated_at FROM settings WHERE user_id = $1 ORDER BY key",
                &[&user_id],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| SettingRow {
                key: r.get("key"),
                value: r.get("value"),
                updated_at: r.get("updated_at"),
            })
            .collect())
    }

    /// Get all settings as a flat key-value map.
    pub async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT key, value FROM settings WHERE user_id = $1",
                &[&user_id],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                let key: String = r.get("key");
                let value: serde_json::Value = r.get("value");
                (key, value)
            })
            .collect())
    }

    /// Bulk-write settings (used for migration/import).
    ///
    /// Each entry is upserted individually within a single transaction.
    pub async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        let mut conn = self.conn().await?;
        let tx = conn.transaction().await?;

        for (key, value) in settings {
            tx.execute(
                r#"
                INSERT INTO settings (user_id, key, value, updated_at)
                VALUES ($1, $2, $3, NOW())
                ON CONFLICT (user_id, key) DO UPDATE SET
                    value = EXCLUDED.value,
                    updated_at = NOW()
                "#,
                &[&user_id, &key, value],
            )
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Check if the settings table has any rows for a user.
    pub async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_one(
                "SELECT COUNT(*) as cnt FROM settings WHERE user_id = $1",
                &[&user_id],
            )
            .await?;
        let count: i64 = row.get("cnt");
        Ok(count > 0)
    }
}
