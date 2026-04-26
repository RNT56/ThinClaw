//! Experiment-related ExperimentStore implementation for LibSqlBackend.

use async_trait::async_trait;
use libsql::params;
use uuid::Uuid;

use super::{
    LibSqlBackend, fmt_opt_ts, fmt_ts, get_i64, get_json, get_opt_text, get_opt_ts, get_text,
    get_ts, row_json_to,
};
use crate::db::ExperimentStore;
use crate::error::DatabaseError;
use crate::experiments::{
    ExperimentArtifactRef, ExperimentCampaign, ExperimentLease, ExperimentModelUsageRecord,
    ExperimentProject, ExperimentRunnerProfile, ExperimentTarget, ExperimentTargetLink,
    ExperimentTrial,
};

#[async_trait]
impl ExperimentStore for LibSqlBackend {
    async fn create_experiment_project(
        &self,
        project: &ExperimentProject,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO experiment_projects (
                id, name, workspace_path, git_remote_name, base_branch,
                preset, strategy_prompt, workdir, prepare_command, run_command,
                mutable_paths, fixed_paths, primary_metric, secondary_metrics,
                comparison_policy, stop_policy, default_runner_profile_id,
                promotion_mode, autonomy_mode, status, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9, ?10,
                ?11, ?12, ?13, ?14,
                ?15, ?16, ?17,
                ?18, ?19, ?20, ?21, ?22
            )
            "#,
            params![
                project.id.to_string(),
                project.name.as_str(),
                project.workspace_path.as_str(),
                project.git_remote_name.as_str(),
                project.base_branch.as_str(),
                serde_json::to_string(&project.preset)
                    .unwrap_or_else(|_| "\"autoresearch_single_file\"".to_string()),
                project.strategy_prompt.as_str(),
                project.workdir.as_str(),
                super::opt_text(project.prepare_command.as_deref()),
                project.run_command.as_str(),
                serde_json::to_string(&project.mutable_paths).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&project.fixed_paths).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&project.primary_metric).unwrap_or_else(|_| "{}".to_string()),
                serde_json::to_string(&project.secondary_metrics)
                    .unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&project.comparison_policy)
                    .unwrap_or_else(|_| "{}".to_string()),
                serde_json::to_string(&project.stop_policy).unwrap_or_else(|_| "{}".to_string()),
                super::opt_text_owned(project.default_runner_profile_id.map(|id| id.to_string())),
                project.promotion_mode.as_str(),
                serde_json::to_string(&project.autonomy_mode)
                    .unwrap_or_else(|_| "\"autonomous\"".to_string()),
                serde_json::to_string(&project.status).unwrap_or_else(|_| "\"draft\"".to_string()),
                fmt_ts(&project.created_at),
                fmt_ts(&project.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_experiment_project(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentProject>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_projects WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_project(&row)?)),
            None => Ok(None),
        }
    }

    async fn list_experiment_projects(&self) -> Result<Vec<ExperimentProject>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_projects ORDER BY updated_at DESC, name ASC",
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_project(&row)?);
        }
        Ok(items)
    }

    async fn update_experiment_project(
        &self,
        project: &ExperimentProject,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            UPDATE experiment_projects SET
                name = ?2,
                workspace_path = ?3,
                git_remote_name = ?4,
                base_branch = ?5,
                preset = ?6,
                strategy_prompt = ?7,
                workdir = ?8,
                prepare_command = ?9,
                run_command = ?10,
                mutable_paths = ?11,
                fixed_paths = ?12,
                primary_metric = ?13,
                secondary_metrics = ?14,
                comparison_policy = ?15,
                stop_policy = ?16,
                default_runner_profile_id = ?17,
                promotion_mode = ?18,
                autonomy_mode = ?19,
                status = ?20,
                updated_at = ?21
            WHERE id = ?1
            "#,
            params![
                project.id.to_string(),
                project.name.as_str(),
                project.workspace_path.as_str(),
                project.git_remote_name.as_str(),
                project.base_branch.as_str(),
                serde_json::to_string(&project.preset)
                    .unwrap_or_else(|_| "\"autoresearch_single_file\"".to_string()),
                project.strategy_prompt.as_str(),
                project.workdir.as_str(),
                super::opt_text(project.prepare_command.as_deref()),
                project.run_command.as_str(),
                serde_json::to_string(&project.mutable_paths).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&project.fixed_paths).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&project.primary_metric).unwrap_or_else(|_| "{}".to_string()),
                serde_json::to_string(&project.secondary_metrics)
                    .unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&project.comparison_policy)
                    .unwrap_or_else(|_| "{}".to_string()),
                serde_json::to_string(&project.stop_policy).unwrap_or_else(|_| "{}".to_string()),
                super::opt_text_owned(project.default_runner_profile_id.map(|id| id.to_string())),
                project.promotion_mode.as_str(),
                serde_json::to_string(&project.autonomy_mode)
                    .unwrap_or_else(|_| "\"autonomous\"".to_string()),
                serde_json::to_string(&project.status).unwrap_or_else(|_| "\"draft\"".to_string()),
                fmt_ts(&project.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn delete_experiment_project(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let count = conn
            .execute(
                "DELETE FROM experiment_projects WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(count > 0)
    }

    async fn create_experiment_runner_profile(
        &self,
        profile: &ExperimentRunnerProfile,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO experiment_runner_profiles (
                id, name, backend, backend_config, image_or_runtime,
                gpu_requirements, env_grants, secret_references,
                cache_policy, status, readiness_class, launch_eligible, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8,
                ?9, ?10, ?11, ?12, ?13, ?14
            )
            "#,
            params![
                profile.id.to_string(),
                profile.name.as_str(),
                serde_json::to_string(&profile.backend)
                    .unwrap_or_else(|_| "\"local_docker\"".to_string()),
                profile.backend_config.to_string(),
                super::opt_text(profile.image_or_runtime.as_deref()),
                profile.gpu_requirements.to_string(),
                profile.env_grants.to_string(),
                serde_json::to_string(&profile.secret_references)
                    .unwrap_or_else(|_| "[]".to_string()),
                profile.cache_policy.to_string(),
                serde_json::to_string(&profile.status).unwrap_or_else(|_| "\"draft\"".to_string()),
                serde_json::to_string(&profile.readiness_class)
                    .unwrap_or_else(|_| "\"manual_only\"".to_string()),
                if profile.launch_eligible { 1 } else { 0 },
                fmt_ts(&profile.created_at),
                fmt_ts(&profile.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_experiment_runner_profile(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentRunnerProfile>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_runner_profiles WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_runner(&row)?)),
            None => Ok(None),
        }
    }

    async fn list_experiment_runner_profiles(
        &self,
    ) -> Result<Vec<ExperimentRunnerProfile>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_runner_profiles ORDER BY updated_at DESC, name ASC",
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_runner(&row)?);
        }
        Ok(items)
    }

    async fn update_experiment_runner_profile(
        &self,
        profile: &ExperimentRunnerProfile,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            UPDATE experiment_runner_profiles SET
                name = ?2,
                backend = ?3,
                backend_config = ?4,
                image_or_runtime = ?5,
                gpu_requirements = ?6,
                env_grants = ?7,
                secret_references = ?8,
                cache_policy = ?9,
                status = ?10,
                readiness_class = ?11,
                launch_eligible = ?12,
                updated_at = ?13
            WHERE id = ?1
            "#,
            params![
                profile.id.to_string(),
                profile.name.as_str(),
                serde_json::to_string(&profile.backend)
                    .unwrap_or_else(|_| "\"local_docker\"".to_string()),
                profile.backend_config.to_string(),
                super::opt_text(profile.image_or_runtime.as_deref()),
                profile.gpu_requirements.to_string(),
                profile.env_grants.to_string(),
                serde_json::to_string(&profile.secret_references)
                    .unwrap_or_else(|_| "[]".to_string()),
                profile.cache_policy.to_string(),
                serde_json::to_string(&profile.status).unwrap_or_else(|_| "\"draft\"".to_string()),
                serde_json::to_string(&profile.readiness_class)
                    .unwrap_or_else(|_| "\"manual_only\"".to_string()),
                if profile.launch_eligible { 1 } else { 0 },
                fmt_ts(&profile.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn delete_experiment_runner_profile(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let count = conn
            .execute(
                "DELETE FROM experiment_runner_profiles WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(count > 0)
    }

    async fn create_experiment_campaign(
        &self,
        campaign: &ExperimentCampaign,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO experiment_campaigns (
                id, project_id, runner_profile_id, owner_user_id, status, baseline_commit,
                best_commit, best_metrics, experiment_branch, remote_ref,
                worktree_path, started_at, ended_at, trial_count,
                failure_count, pause_reason, queue_state, queue_position,
                active_trial_id, total_runtime_ms, total_cost_usd,
                consecutive_non_improving_trials, max_trials_override, gateway_url,
                metadata, created_at, updated_at, total_llm_cost_usd, total_runner_cost_usd
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6,
                ?7, ?8, ?9, ?10,
                ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18,
                ?19, ?20, ?21,
                ?22, ?23, ?24,
                ?25, ?26, ?27, ?28, ?29
            )
            "#,
            params![
                campaign.id.to_string(),
                campaign.project_id.to_string(),
                campaign.runner_profile_id.to_string(),
                campaign.owner_user_id.as_str(),
                serde_json::to_string(&campaign.status)
                    .unwrap_or_else(|_| "\"pending_baseline\"".to_string()),
                super::opt_text(campaign.baseline_commit.as_deref()),
                super::opt_text(campaign.best_commit.as_deref()),
                campaign.best_metrics.to_string(),
                super::opt_text(campaign.experiment_branch.as_deref()),
                super::opt_text(campaign.remote_ref.as_deref()),
                super::opt_text(campaign.worktree_path.as_deref()),
                fmt_opt_ts(&campaign.started_at),
                fmt_opt_ts(&campaign.ended_at),
                campaign.trial_count as i64,
                campaign.failure_count as i64,
                super::opt_text(campaign.pause_reason.as_deref()),
                campaign.queue_state.as_str(),
                campaign.queue_position as i64,
                super::opt_text_owned(campaign.active_trial_id.map(|id| id.to_string())),
                campaign.total_runtime_ms as i64,
                campaign.total_cost_usd,
                campaign.consecutive_non_improving_trials as i64,
                campaign.max_trials_override,
                super::opt_text(campaign.gateway_url.as_deref()),
                campaign.metadata.to_string(),
                fmt_ts(&campaign.created_at),
                fmt_ts(&campaign.updated_at),
                campaign.total_llm_cost_usd,
                campaign.total_runner_cost_usd,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_experiment_campaign(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentCampaign>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_campaigns WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_campaign(&row)?)),
            None => Ok(None),
        }
    }

    async fn get_experiment_campaign_for_owner(
        &self,
        id: Uuid,
        owner_user_id: &str,
    ) -> Result<Option<ExperimentCampaign>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_campaigns WHERE id = ?1 AND owner_user_id = ?2",
                params![id.to_string(), owner_user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_campaign(&row)?)),
            None => Ok(None),
        }
    }

    async fn list_experiment_campaigns(&self) -> Result<Vec<ExperimentCampaign>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_campaigns ORDER BY created_at DESC",
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_campaign(&row)?);
        }
        Ok(items)
    }

    async fn list_experiment_campaigns_for_owner(
        &self,
        owner_user_id: &str,
    ) -> Result<Vec<ExperimentCampaign>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_campaigns WHERE owner_user_id = ?1 ORDER BY created_at DESC",
                params![owner_user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_campaign(&row)?);
        }
        Ok(items)
    }

    async fn update_experiment_campaign(
        &self,
        campaign: &ExperimentCampaign,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            UPDATE experiment_campaigns SET
                project_id = ?2,
                runner_profile_id = ?3,
                owner_user_id = ?4,
                status = ?5,
                baseline_commit = ?6,
                best_commit = ?7,
                best_metrics = ?8,
                experiment_branch = ?9,
                remote_ref = ?10,
                worktree_path = ?11,
                started_at = ?12,
                ended_at = ?13,
                trial_count = ?14,
                failure_count = ?15,
                pause_reason = ?16,
                queue_state = ?17,
                queue_position = ?18,
                active_trial_id = ?19,
                total_runtime_ms = ?20,
                total_cost_usd = ?21,
                consecutive_non_improving_trials = ?22,
                max_trials_override = ?23,
                gateway_url = ?24,
                metadata = ?25,
                updated_at = ?26,
                total_llm_cost_usd = ?27,
                total_runner_cost_usd = ?28
            WHERE id = ?1
            "#,
            params![
                campaign.id.to_string(),
                campaign.project_id.to_string(),
                campaign.runner_profile_id.to_string(),
                campaign.owner_user_id.as_str(),
                serde_json::to_string(&campaign.status)
                    .unwrap_or_else(|_| "\"pending_baseline\"".to_string()),
                super::opt_text(campaign.baseline_commit.as_deref()),
                super::opt_text(campaign.best_commit.as_deref()),
                campaign.best_metrics.to_string(),
                super::opt_text(campaign.experiment_branch.as_deref()),
                super::opt_text(campaign.remote_ref.as_deref()),
                super::opt_text(campaign.worktree_path.as_deref()),
                fmt_opt_ts(&campaign.started_at),
                fmt_opt_ts(&campaign.ended_at),
                campaign.trial_count as i64,
                campaign.failure_count as i64,
                super::opt_text(campaign.pause_reason.as_deref()),
                campaign.queue_state.as_str(),
                campaign.queue_position as i64,
                super::opt_text_owned(campaign.active_trial_id.map(|id| id.to_string())),
                campaign.total_runtime_ms as i64,
                campaign.total_cost_usd,
                campaign.consecutive_non_improving_trials as i64,
                campaign.max_trials_override,
                super::opt_text(campaign.gateway_url.as_deref()),
                campaign.metadata.to_string(),
                fmt_ts(&campaign.updated_at),
                campaign.total_llm_cost_usd,
                campaign.total_runner_cost_usd,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn create_experiment_trial(&self, trial: &ExperimentTrial) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO experiment_trials (
                id, campaign_id, sequence, candidate_commit, parent_best_commit,
                status, runner_backend, exit_code, metrics_json, summary,
                decision_reason, log_preview_path, artifact_manifest_json,
                runtime_ms, attributed_cost_usd, hypothesis, mutation_summary,
                reviewer_decision, provider_job_id, provider_job_metadata,
                started_at, completed_at, created_at, updated_at,
                llm_cost_usd, runner_cost_usd
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9, ?10,
                ?11, ?12, ?13, ?14, ?15,
                ?16, ?17, ?18, ?19,
                ?20, ?21, ?22, ?23, ?24, ?25, ?26
            )
            "#,
            params![
                trial.id.to_string(),
                trial.campaign_id.to_string(),
                trial.sequence as i64,
                super::opt_text(trial.candidate_commit.as_deref()),
                super::opt_text(trial.parent_best_commit.as_deref()),
                serde_json::to_string(&trial.status)
                    .unwrap_or_else(|_| "\"preparing\"".to_string()),
                serde_json::to_string(&trial.runner_backend)
                    .unwrap_or_else(|_| "\"local_docker\"".to_string()),
                trial.exit_code,
                trial.metrics_json.to_string(),
                super::opt_text(trial.summary.as_deref()),
                super::opt_text(trial.decision_reason.as_deref()),
                super::opt_text(trial.log_preview_path.as_deref()),
                trial.artifact_manifest_json.to_string(),
                trial.runtime_ms.map(|value| value as i64),
                trial.attributed_cost_usd,
                super::opt_text(trial.hypothesis.as_deref()),
                super::opt_text(trial.mutation_summary.as_deref()),
                super::opt_text(trial.reviewer_decision.as_deref()),
                super::opt_text(trial.provider_job_id.as_deref()),
                trial.provider_job_metadata.to_string(),
                fmt_opt_ts(&trial.started_at),
                fmt_opt_ts(&trial.completed_at),
                fmt_ts(&trial.created_at),
                fmt_ts(&trial.updated_at),
                trial.llm_cost_usd,
                trial.runner_cost_usd,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_experiment_trial(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentTrial>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_trials WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_trial(&row)?)),
            None => Ok(None),
        }
    }

    async fn get_experiment_trial_for_owner(
        &self,
        id: Uuid,
        owner_user_id: &str,
    ) -> Result<Option<ExperimentTrial>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT t.* FROM experiment_trials t
                INNER JOIN experiment_campaigns c ON c.id = t.campaign_id
                WHERE t.id = ?1 AND c.owner_user_id = ?2
                "#,
                params![id.to_string(), owner_user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_trial(&row)?)),
            None => Ok(None),
        }
    }

    async fn list_experiment_trials(
        &self,
        campaign_id: Uuid,
    ) -> Result<Vec<ExperimentTrial>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_trials WHERE campaign_id = ?1 ORDER BY sequence ASC",
                params![campaign_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_trial(&row)?);
        }
        Ok(items)
    }

    async fn list_experiment_trials_for_owner(
        &self,
        campaign_id: Uuid,
        owner_user_id: &str,
    ) -> Result<Vec<ExperimentTrial>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT t.* FROM experiment_trials t
                INNER JOIN experiment_campaigns c ON c.id = t.campaign_id
                WHERE t.campaign_id = ?1 AND c.owner_user_id = ?2
                ORDER BY t.sequence ASC
                "#,
                params![campaign_id.to_string(), owner_user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_trial(&row)?);
        }
        Ok(items)
    }

    async fn update_experiment_trial(&self, trial: &ExperimentTrial) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            UPDATE experiment_trials SET
                campaign_id = ?2,
                sequence = ?3,
                candidate_commit = ?4,
                parent_best_commit = ?5,
                status = ?6,
                runner_backend = ?7,
                exit_code = ?8,
                metrics_json = ?9,
                summary = ?10,
                decision_reason = ?11,
                log_preview_path = ?12,
                artifact_manifest_json = ?13,
                runtime_ms = ?14,
                attributed_cost_usd = ?15,
                hypothesis = ?16,
                mutation_summary = ?17,
                reviewer_decision = ?18,
                provider_job_id = ?19,
                provider_job_metadata = ?20,
                started_at = ?21,
                completed_at = ?22,
                updated_at = ?23,
                llm_cost_usd = ?24,
                runner_cost_usd = ?25
            WHERE id = ?1
            "#,
            params![
                trial.id.to_string(),
                trial.campaign_id.to_string(),
                trial.sequence as i64,
                super::opt_text(trial.candidate_commit.as_deref()),
                super::opt_text(trial.parent_best_commit.as_deref()),
                serde_json::to_string(&trial.status)
                    .unwrap_or_else(|_| "\"preparing\"".to_string()),
                serde_json::to_string(&trial.runner_backend)
                    .unwrap_or_else(|_| "\"local_docker\"".to_string()),
                trial.exit_code,
                trial.metrics_json.to_string(),
                super::opt_text(trial.summary.as_deref()),
                super::opt_text(trial.decision_reason.as_deref()),
                super::opt_text(trial.log_preview_path.as_deref()),
                trial.artifact_manifest_json.to_string(),
                trial.runtime_ms.map(|value| value as i64),
                trial.attributed_cost_usd,
                super::opt_text(trial.hypothesis.as_deref()),
                super::opt_text(trial.mutation_summary.as_deref()),
                super::opt_text(trial.reviewer_decision.as_deref()),
                super::opt_text(trial.provider_job_id.as_deref()),
                trial.provider_job_metadata.to_string(),
                fmt_opt_ts(&trial.started_at),
                fmt_opt_ts(&trial.completed_at),
                fmt_ts(&trial.updated_at),
                trial.llm_cost_usd,
                trial.runner_cost_usd,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn replace_experiment_artifacts(
        &self,
        trial_id: Uuid,
        artifacts: &[ExperimentArtifactRef],
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute("BEGIN", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let delete_result = conn
            .execute(
                "DELETE FROM experiment_artifact_refs WHERE trial_id = ?1",
                params![trial_id.to_string()],
            )
            .await;
        if let Err(error) = delete_result {
            let _ = conn.execute("ROLLBACK", ()).await;
            return Err(DatabaseError::Query(error.to_string()));
        }
        for artifact in artifacts {
            if let Err(error) = conn
                .execute(
                    r#"
                INSERT INTO experiment_artifact_refs (
                    id, trial_id, kind, uri_or_local_path, size_bytes,
                    fetchable, metadata, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
                    params![
                        artifact.id.to_string(),
                        artifact.trial_id.to_string(),
                        artifact.kind.as_str(),
                        artifact.uri_or_local_path.as_str(),
                        artifact.size_bytes.map(|v| v as i64),
                        artifact.fetchable as i64,
                        artifact.metadata.to_string(),
                        fmt_ts(&artifact.created_at),
                    ],
                )
                .await
            {
                let _ = conn.execute("ROLLBACK", ()).await;
                return Err(DatabaseError::Query(error.to_string()));
            }
        }
        conn.execute("COMMIT", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_experiment_artifacts(
        &self,
        trial_id: Uuid,
    ) -> Result<Vec<ExperimentArtifactRef>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_artifact_refs WHERE trial_id = ?1 ORDER BY created_at ASC",
                params![trial_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_artifact(&row)?);
        }
        Ok(items)
    }

    async fn list_experiment_artifacts_for_owner(
        &self,
        trial_id: Uuid,
        owner_user_id: &str,
    ) -> Result<Vec<ExperimentArtifactRef>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT a.* FROM experiment_artifact_refs a
                INNER JOIN experiment_trials t ON t.id = a.trial_id
                INNER JOIN experiment_campaigns c ON c.id = t.campaign_id
                WHERE a.trial_id = ?1 AND c.owner_user_id = ?2
                ORDER BY a.created_at ASC
                "#,
                params![trial_id.to_string(), owner_user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_artifact(&row)?);
        }
        Ok(items)
    }

    async fn create_experiment_target(
        &self,
        target: &ExperimentTarget,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO experiment_targets (
                id, name, kind, location, metadata, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7
            )
            "#,
            params![
                target.id.to_string(),
                target.name.as_str(),
                serde_json::to_string(&target.kind)
                    .unwrap_or_else(|_| "\"prompt_asset\"".to_string()),
                super::opt_text(target.location.as_deref()),
                target.metadata.to_string(),
                fmt_ts(&target.created_at),
                fmt_ts(&target.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_experiment_target(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentTarget>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_targets WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_target(&row)?)),
            None => Ok(None),
        }
    }

    async fn list_experiment_targets(&self) -> Result<Vec<ExperimentTarget>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_targets ORDER BY updated_at DESC, name ASC",
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_target(&row)?);
        }
        Ok(items)
    }

    async fn update_experiment_target(
        &self,
        target: &ExperimentTarget,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            UPDATE experiment_targets SET
                name = ?2,
                kind = ?3,
                location = ?4,
                metadata = ?5,
                updated_at = ?6
            WHERE id = ?1
            "#,
            params![
                target.id.to_string(),
                target.name.as_str(),
                serde_json::to_string(&target.kind)
                    .unwrap_or_else(|_| "\"prompt_asset\"".to_string()),
                super::opt_text(target.location.as_deref()),
                target.metadata.to_string(),
                fmt_ts(&target.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn delete_experiment_target(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let count = conn
            .execute(
                "DELETE FROM experiment_targets WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(count > 0)
    }

    async fn upsert_experiment_target_link(
        &self,
        link: &ExperimentTargetLink,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let route_key = link.route_key.clone().unwrap_or_default();
        let logical_role = link.logical_role.clone().unwrap_or_default();
        conn.execute(
            r#"
            INSERT INTO experiment_target_links (
                id, target_id, kind, provider, model, route_key, logical_role,
                metadata, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                ?8, ?9, ?10
            )
            ON CONFLICT(target_id, kind, provider, model, route_key, logical_role)
            DO UPDATE SET
                metadata = excluded.metadata,
                updated_at = excluded.updated_at
            "#,
            params![
                link.id.to_string(),
                link.target_id.to_string(),
                serde_json::to_string(&link.kind)
                    .unwrap_or_else(|_| "\"prompt_asset\"".to_string()),
                link.provider.as_str(),
                link.model.as_str(),
                route_key,
                logical_role,
                link.metadata.to_string(),
                fmt_ts(&link.created_at),
                fmt_ts(&link.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_experiment_target_links(
        &self,
    ) -> Result<Vec<ExperimentTargetLink>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_target_links ORDER BY updated_at DESC, provider ASC, model ASC",
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_target_link(&row)?);
        }
        Ok(items)
    }

    async fn delete_experiment_target_links_for_target(
        &self,
        target_id: Uuid,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "DELETE FROM experiment_target_links WHERE target_id = ?1",
            params![target_id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn create_experiment_model_usage(
        &self,
        usage: &ExperimentModelUsageRecord,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO experiment_model_usage_records (
                id, provider, model, route_key, logical_role, endpoint_type,
                workload_tag, latency_ms, cost_usd, success,
                prompt_asset_ids, retrieval_asset_ids, tool_policy_ids,
                evaluator_ids, parser_ids,
                metadata, created_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6,
                ?7, ?8, ?9, ?10,
                ?11, ?12, ?13,
                ?14, ?15,
                ?16, ?17
            )
            "#,
            params![
                usage.id.to_string(),
                usage.provider.as_str(),
                usage.model.as_str(),
                super::opt_text(usage.route_key.as_deref()),
                super::opt_text(usage.logical_role.as_deref()),
                super::opt_text(usage.endpoint_type.as_deref()),
                super::opt_text(usage.workload_tag.as_deref()),
                usage.latency_ms.map(|v| v as i64),
                usage.cost_usd.map(|v| v.to_string()),
                usage.success as i64,
                serde_json::to_string(&usage.prompt_asset_ids).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&usage.retrieval_asset_ids)
                    .unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&usage.tool_policy_ids).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&usage.evaluator_ids).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&usage.parser_ids).unwrap_or_else(|_| "[]".to_string()),
                usage.metadata.to_string(),
                fmt_ts(&usage.created_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_experiment_model_usage(
        &self,
        limit: usize,
    ) -> Result<Vec<ExperimentModelUsageRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_model_usage_records ORDER BY created_at DESC LIMIT ?1",
                params![limit as i64],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_model_usage(&row)?);
        }
        Ok(items)
    }

    async fn list_experiment_model_usage_for_campaign(
        &self,
        campaign_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ExperimentModelUsageRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_model_usage_records WHERE json_extract(metadata, '$.experiment_campaign_id') = ?1 ORDER BY created_at ASC LIMIT ?2",
                params![campaign_id.to_string(), limit as i64],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_model_usage(&row)?);
        }
        Ok(items)
    }

    async fn list_experiment_model_usage_for_trial(
        &self,
        trial_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ExperimentModelUsageRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_model_usage_records WHERE json_extract(metadata, '$.experiment_trial_id') = ?1 ORDER BY created_at ASC LIMIT ?2",
                params![trial_id.to_string(), limit as i64],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_model_usage(&row)?);
        }
        Ok(items)
    }

    async fn create_experiment_lease(&self, lease: &ExperimentLease) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO experiment_leases (
                id, campaign_id, trial_id, runner_profile_id, status,
                token_hash, job_payload, credentials_payload, expires_at,
                claimed_at, completed_at, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9,
                ?10, ?11, ?12, ?13
            )
            "#,
            params![
                lease.id.to_string(),
                lease.campaign_id.to_string(),
                lease.trial_id.to_string(),
                lease.runner_profile_id.to_string(),
                serde_json::to_string(&lease.status).unwrap_or_else(|_| "\"pending\"".to_string()),
                lease.token_hash.as_str(),
                lease.job_payload.to_string(),
                lease.credentials_payload.to_string(),
                fmt_ts(&lease.expires_at),
                fmt_opt_ts(&lease.claimed_at),
                fmt_opt_ts(&lease.completed_at),
                fmt_ts(&lease.created_at),
                fmt_ts(&lease.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_experiment_lease(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentLease>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_leases WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_lease(&row)?)),
            None => Ok(None),
        }
    }

    async fn get_experiment_lease_for_trial(
        &self,
        trial_id: Uuid,
    ) -> Result<Option<ExperimentLease>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT * FROM experiment_leases WHERE trial_id = ?1 ORDER BY created_at DESC LIMIT 1",
                params![trial_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_lease(&row)?)),
            None => Ok(None),
        }
    }

    async fn update_experiment_lease(&self, lease: &ExperimentLease) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            UPDATE experiment_leases SET
                campaign_id = ?2,
                trial_id = ?3,
                runner_profile_id = ?4,
                status = ?5,
                token_hash = ?6,
                job_payload = ?7,
                credentials_payload = ?8,
                expires_at = ?9,
                claimed_at = ?10,
                completed_at = ?11,
                updated_at = ?12
            WHERE id = ?1
            "#,
            params![
                lease.id.to_string(),
                lease.campaign_id.to_string(),
                lease.trial_id.to_string(),
                lease.runner_profile_id.to_string(),
                serde_json::to_string(&lease.status).unwrap_or_else(|_| "\"pending\"".to_string()),
                lease.token_hash.as_str(),
                lease.job_payload.to_string(),
                lease.credentials_payload.to_string(),
                fmt_ts(&lease.expires_at),
                fmt_opt_ts(&lease.claimed_at),
                fmt_opt_ts(&lease.completed_at),
                fmt_ts(&lease.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }
}

fn row_to_project(row: &libsql::Row) -> Result<ExperimentProject, DatabaseError> {
    Ok(ExperimentProject {
        id: Uuid::parse_str(&get_text(row, 0))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        name: get_text(row, 1),
        workspace_path: get_text(row, 2),
        git_remote_name: get_text(row, 3),
        base_branch: get_text(row, 4),
        preset: row_json_to(row, 5)?,
        strategy_prompt: get_text(row, 6),
        workdir: get_text(row, 7),
        prepare_command: get_opt_text(row, 8),
        run_command: get_text(row, 9),
        mutable_paths: row_json_to(row, 10)?,
        fixed_paths: row_json_to(row, 11)?,
        primary_metric: row_json_to(row, 12)?,
        secondary_metrics: row_json_to(row, 13)?,
        comparison_policy: row_json_to(row, 14)?,
        stop_policy: row_json_to(row, 15)?,
        default_runner_profile_id: get_opt_text(row, 16)
            .map(|value| {
                Uuid::parse_str(&value).map_err(|e| DatabaseError::Serialization(e.to_string()))
            })
            .transpose()?,
        promotion_mode: get_text(row, 17),
        autonomy_mode: row_json_to(row, 18)?,
        status: row_json_to(row, 19)?,
        created_at: get_ts(row, 20),
        updated_at: get_ts(row, 21),
    })
}

fn row_to_runner(row: &libsql::Row) -> Result<ExperimentRunnerProfile, DatabaseError> {
    Ok(ExperimentRunnerProfile {
        id: Uuid::parse_str(&get_text(row, 0))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        name: get_text(row, 1),
        backend: row_json_to(row, 2)?,
        backend_config: get_json(row, 3),
        image_or_runtime: get_opt_text(row, 4),
        gpu_requirements: get_json(row, 5),
        env_grants: get_json(row, 6),
        secret_references: row_json_to(row, 7)?,
        cache_policy: get_json(row, 8),
        status: row_json_to(row, 9)?,
        readiness_class: row_json_to(row, 10)
            .unwrap_or(crate::experiments::ExperimentRunnerReadinessClass::ManualOnly),
        launch_eligible: crate::db::libsql::get_i64(row, 11) != 0,
        created_at: get_ts(row, 12),
        updated_at: get_ts(row, 13),
    })
}

fn row_to_campaign(row: &libsql::Row) -> Result<ExperimentCampaign, DatabaseError> {
    Ok(ExperimentCampaign {
        id: Uuid::parse_str(&get_text(row, 0))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        project_id: Uuid::parse_str(&get_text(row, 1))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        runner_profile_id: Uuid::parse_str(&get_text(row, 2))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        owner_user_id: get_text(row, 3),
        status: row_json_to(row, 4)?,
        baseline_commit: get_opt_text(row, 5),
        best_commit: get_opt_text(row, 6),
        best_metrics: get_json(row, 7),
        experiment_branch: get_opt_text(row, 8),
        remote_ref: get_opt_text(row, 9),
        worktree_path: get_opt_text(row, 10),
        started_at: get_opt_ts(row, 11),
        ended_at: get_opt_ts(row, 12),
        trial_count: get_i64(row, 13) as u32,
        failure_count: get_i64(row, 14) as u32,
        pause_reason: get_opt_text(row, 15),
        queue_state: get_text(row, 16)
            .parse()
            .map_err(DatabaseError::Serialization)?,
        queue_position: get_i64(row, 17) as u32,
        active_trial_id: get_opt_text(row, 18).and_then(|value| Uuid::parse_str(&value).ok()),
        total_runtime_ms: get_i64(row, 19) as u64,
        total_cost_usd: get_opt_text(row, 20)
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0),
        total_llm_cost_usd: get_opt_text(row, 27)
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0),
        total_runner_cost_usd: get_opt_text(row, 28)
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0),
        consecutive_non_improving_trials: get_i64(row, 21) as u32,
        max_trials_override: row.get::<i64>(22).ok().map(|value| value as u32),
        gateway_url: get_opt_text(row, 23),
        metadata: get_json(row, 24),
        created_at: get_ts(row, 25),
        updated_at: get_ts(row, 26),
    })
}

fn row_to_trial(row: &libsql::Row) -> Result<ExperimentTrial, DatabaseError> {
    Ok(ExperimentTrial {
        id: Uuid::parse_str(&get_text(row, 0))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        campaign_id: Uuid::parse_str(&get_text(row, 1))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        sequence: get_i64(row, 2) as u32,
        candidate_commit: get_opt_text(row, 3),
        parent_best_commit: get_opt_text(row, 4),
        status: row_json_to(row, 5)?,
        runner_backend: row_json_to(row, 6)?,
        exit_code: row.get::<i64>(7).ok().map(|v| v as i32),
        metrics_json: get_json(row, 8),
        summary: get_opt_text(row, 9),
        decision_reason: get_opt_text(row, 10),
        log_preview_path: get_opt_text(row, 11),
        artifact_manifest_json: get_json(row, 12),
        runtime_ms: row.get::<i64>(13).ok().map(|value| value as u64),
        attributed_cost_usd: row
            .get::<String>(14)
            .ok()
            .and_then(|value| value.parse::<f64>().ok()),
        llm_cost_usd: row
            .get::<String>(24)
            .ok()
            .and_then(|value| value.parse::<f64>().ok()),
        runner_cost_usd: row
            .get::<String>(25)
            .ok()
            .and_then(|value| value.parse::<f64>().ok()),
        hypothesis: get_opt_text(row, 15),
        mutation_summary: get_opt_text(row, 16),
        reviewer_decision: get_opt_text(row, 17),
        provider_job_id: get_opt_text(row, 18),
        provider_job_metadata: get_json(row, 19),
        started_at: get_opt_ts(row, 20),
        completed_at: get_opt_ts(row, 21),
        created_at: get_ts(row, 22),
        updated_at: get_ts(row, 23),
    })
}

fn row_to_artifact(row: &libsql::Row) -> Result<ExperimentArtifactRef, DatabaseError> {
    Ok(ExperimentArtifactRef {
        id: Uuid::parse_str(&get_text(row, 0))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        trial_id: Uuid::parse_str(&get_text(row, 1))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        kind: get_text(row, 2),
        uri_or_local_path: get_text(row, 3),
        size_bytes: row.get::<i64>(4).ok().map(|v| v as u64),
        fetchable: get_i64(row, 5) != 0,
        metadata: get_json(row, 6),
        created_at: get_ts(row, 7),
    })
}

fn row_to_target(row: &libsql::Row) -> Result<ExperimentTarget, DatabaseError> {
    Ok(ExperimentTarget {
        id: Uuid::parse_str(&get_text(row, 0))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        name: get_text(row, 1),
        kind: row_json_to(row, 2)?,
        location: get_opt_text(row, 3),
        metadata: get_json(row, 4),
        created_at: get_ts(row, 5),
        updated_at: get_ts(row, 6),
    })
}

fn row_to_target_link(row: &libsql::Row) -> Result<ExperimentTargetLink, DatabaseError> {
    let route_key = get_text(row, 5);
    let logical_role = get_text(row, 6);
    Ok(ExperimentTargetLink {
        id: Uuid::parse_str(&get_text(row, 0))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        target_id: Uuid::parse_str(&get_text(row, 1))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        kind: row_json_to(row, 2)?,
        provider: get_text(row, 3),
        model: get_text(row, 4),
        route_key: (!route_key.is_empty()).then_some(route_key),
        logical_role: (!logical_role.is_empty()).then_some(logical_role),
        metadata: get_json(row, 7),
        created_at: get_ts(row, 8),
        updated_at: get_ts(row, 9),
    })
}

fn row_to_model_usage(row: &libsql::Row) -> Result<ExperimentModelUsageRecord, DatabaseError> {
    Ok(ExperimentModelUsageRecord {
        id: Uuid::parse_str(&get_text(row, 0))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        provider: get_text(row, 1),
        model: get_text(row, 2),
        route_key: get_opt_text(row, 3),
        logical_role: get_opt_text(row, 4),
        endpoint_type: get_opt_text(row, 5),
        workload_tag: get_opt_text(row, 6),
        latency_ms: row.get::<i64>(7).ok().map(|v| v as u64),
        cost_usd: row
            .get::<String>(8)
            .ok()
            .and_then(|value| value.parse::<f64>().ok()),
        success: get_i64(row, 9) != 0,
        prompt_asset_ids: row_json_to(row, 10)?,
        retrieval_asset_ids: row_json_to(row, 11)?,
        tool_policy_ids: row_json_to(row, 12)?,
        evaluator_ids: row_json_to(row, 13)?,
        parser_ids: row_json_to(row, 14)?,
        metadata: get_json(row, 15),
        created_at: get_ts(row, 16),
    })
}

fn row_to_lease(row: &libsql::Row) -> Result<ExperimentLease, DatabaseError> {
    Ok(ExperimentLease {
        id: Uuid::parse_str(&get_text(row, 0))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        campaign_id: Uuid::parse_str(&get_text(row, 1))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        trial_id: Uuid::parse_str(&get_text(row, 2))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        runner_profile_id: Uuid::parse_str(&get_text(row, 3))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        status: row_json_to(row, 4)?,
        token_hash: get_text(row, 5),
        job_payload: get_json(row, 6),
        credentials_payload: get_json(row, 7),
        expires_at: get_ts(row, 8),
        claimed_at: get_opt_ts(row, 9),
        completed_at: get_opt_ts(row, 10),
        created_at: get_ts(row, 11),
        updated_at: get_ts(row, 12),
    })
}
