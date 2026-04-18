#[cfg(feature = "postgres")]
use chrono::{DateTime, Utc};
#[cfg(feature = "postgres")]
use rust_decimal::{Decimal, prelude::FromPrimitive};
#[cfg(feature = "postgres")]
use uuid::Uuid;

#[cfg(feature = "postgres")]
use crate::error::DatabaseError;
#[cfg(feature = "postgres")]
use crate::experiments::{
    ExperimentArtifactRef, ExperimentAutonomyMode, ExperimentCampaign, ExperimentCampaignStatus,
    ExperimentLease, ExperimentLeaseStatus, ExperimentMetricComparator, ExperimentMetricDefinition,
    ExperimentModelUsageRecord, ExperimentPreset, ExperimentProject, ExperimentProjectStatus,
    ExperimentRunnerBackend, ExperimentRunnerProfile, ExperimentRunnerStatus, ExperimentTarget,
    ExperimentTargetLink, ExperimentTrial, ExperimentTrialStatus,
};

#[cfg(feature = "postgres")]
use super::Store;

#[cfg(feature = "postgres")]
impl Store {
    pub async fn create_experiment_project(
        &self,
        project: &ExperimentProject,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO experiment_projects (
                id, name, workspace_path, git_remote_name, base_branch, preset,
                strategy_prompt, workdir, prepare_command, run_command,
                mutable_paths, fixed_paths, primary_metric, secondary_metrics,
                comparison_policy, stop_policy, default_runner_profile_id,
                promotion_mode, autonomy_mode, status, created_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9, $10,
                $11, $12, $13, $14,
                $15, $16, $17,
                $18, $19, $20, $21, $22
            )
            "#,
            &[
                &project.id,
                &project.name,
                &project.workspace_path,
                &project.git_remote_name,
                &project.base_branch,
                &serde_json::to_value(project.preset).unwrap_or(serde_json::Value::Null),
                &project.strategy_prompt,
                &project.workdir,
                &project.prepare_command,
                &project.run_command,
                &serde_json::json!(project.mutable_paths),
                &serde_json::json!(project.fixed_paths),
                &serde_json::to_value(&project.primary_metric).unwrap_or(serde_json::Value::Null),
                &serde_json::to_value(&project.secondary_metrics)
                    .unwrap_or(serde_json::Value::Array(Vec::new())),
                &serde_json::to_value(&project.comparison_policy)
                    .unwrap_or(serde_json::Value::Null),
                &serde_json::to_value(&project.stop_policy).unwrap_or(serde_json::Value::Null),
                &project.default_runner_profile_id,
                &project.promotion_mode,
                &status_json(project.autonomy_mode),
                &status_json(project.status),
                &project.created_at,
                &project.updated_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn get_experiment_project(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentProject>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt("SELECT * FROM experiment_projects WHERE id = $1", &[&id])
            .await?;
        row.map(|row| row_to_experiment_project(&row)).transpose()
    }

    pub async fn list_experiment_projects(&self) -> Result<Vec<ExperimentProject>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT * FROM experiment_projects ORDER BY updated_at DESC, name ASC",
                &[],
            )
            .await?;
        rows.iter().map(row_to_experiment_project).collect()
    }

    pub async fn update_experiment_project(
        &self,
        project: &ExperimentProject,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE experiment_projects SET
                name = $2,
                workspace_path = $3,
                git_remote_name = $4,
                base_branch = $5,
                preset = $6,
                strategy_prompt = $7,
                workdir = $8,
                prepare_command = $9,
                run_command = $10,
                mutable_paths = $11,
                fixed_paths = $12,
                primary_metric = $13,
                secondary_metrics = $14,
                comparison_policy = $15,
                stop_policy = $16,
                default_runner_profile_id = $17,
                promotion_mode = $18,
                autonomy_mode = $19,
                status = $20,
                updated_at = $21
            WHERE id = $1
            "#,
            &[
                &project.id,
                &project.name,
                &project.workspace_path,
                &project.git_remote_name,
                &project.base_branch,
                &serde_json::to_value(project.preset).unwrap_or(serde_json::Value::Null),
                &project.strategy_prompt,
                &project.workdir,
                &project.prepare_command,
                &project.run_command,
                &serde_json::json!(project.mutable_paths),
                &serde_json::json!(project.fixed_paths),
                &serde_json::to_value(&project.primary_metric).unwrap_or(serde_json::Value::Null),
                &serde_json::to_value(&project.secondary_metrics)
                    .unwrap_or(serde_json::Value::Array(Vec::new())),
                &serde_json::to_value(&project.comparison_policy)
                    .unwrap_or(serde_json::Value::Null),
                &serde_json::to_value(&project.stop_policy).unwrap_or(serde_json::Value::Null),
                &project.default_runner_profile_id,
                &project.promotion_mode,
                &status_json(project.autonomy_mode),
                &status_json(project.status),
                &project.updated_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn delete_experiment_project(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn
            .execute("DELETE FROM experiment_projects WHERE id = $1", &[&id])
            .await?;
        Ok(count > 0)
    }

    pub async fn create_experiment_runner_profile(
        &self,
        profile: &ExperimentRunnerProfile,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO experiment_runner_profiles (
                id, name, backend, backend_config, image_or_runtime,
                gpu_requirements, env_grants, secret_references,
                cache_policy, status, readiness_class, launch_eligible, created_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5,
                $6, $7, $8,
                $9, $10, $11, $12, $13, $14
            )
            "#,
            &[
                &profile.id,
                &profile.name,
                &status_json(profile.backend),
                &profile.backend_config,
                &profile.image_or_runtime,
                &profile.gpu_requirements,
                &profile.env_grants,
                &serde_json::json!(profile.secret_references),
                &profile.cache_policy,
                &status_json(profile.status),
                &status_json(profile.readiness_class),
                &profile.launch_eligible,
                &profile.created_at,
                &profile.updated_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn get_experiment_runner_profile(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentRunnerProfile>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                "SELECT * FROM experiment_runner_profiles WHERE id = $1",
                &[&id],
            )
            .await?;
        row.map(|row| row_to_experiment_runner_profile(&row))
            .transpose()
    }

    pub async fn list_experiment_runner_profiles(
        &self,
    ) -> Result<Vec<ExperimentRunnerProfile>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT * FROM experiment_runner_profiles ORDER BY updated_at DESC, name ASC",
                &[],
            )
            .await?;
        rows.iter().map(row_to_experiment_runner_profile).collect()
    }

    pub async fn update_experiment_runner_profile(
        &self,
        profile: &ExperimentRunnerProfile,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE experiment_runner_profiles SET
                name = $2,
                backend = $3,
                backend_config = $4,
                image_or_runtime = $5,
                gpu_requirements = $6,
                env_grants = $7,
                secret_references = $8,
                cache_policy = $9,
                status = $10,
                readiness_class = $11,
                launch_eligible = $12,
                updated_at = $13
            WHERE id = $1
            "#,
            &[
                &profile.id,
                &profile.name,
                &status_json(profile.backend),
                &profile.backend_config,
                &profile.image_or_runtime,
                &profile.gpu_requirements,
                &profile.env_grants,
                &serde_json::json!(profile.secret_references),
                &profile.cache_policy,
                &status_json(profile.status),
                &status_json(profile.readiness_class),
                &profile.launch_eligible,
                &profile.updated_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn delete_experiment_runner_profile(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn
            .execute(
                "DELETE FROM experiment_runner_profiles WHERE id = $1",
                &[&id],
            )
            .await?;
        Ok(count > 0)
    }

    pub async fn create_experiment_campaign(
        &self,
        campaign: &ExperimentCampaign,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let queue_position = campaign.queue_position as i32;
        let consecutive_non_improving_trials = campaign.consecutive_non_improving_trials as i32;
        let max_trials_override = campaign.max_trials_override.map(|value| value as i32);
        let total_cost_usd = decimal_from_f64(campaign.total_cost_usd, "total_cost_usd")?;
        let total_llm_cost_usd =
            decimal_from_f64(campaign.total_llm_cost_usd, "total_llm_cost_usd")?;
        let total_runner_cost_usd =
            decimal_from_f64(campaign.total_runner_cost_usd, "total_runner_cost_usd")?;
        conn.execute(
            r#"
            INSERT INTO experiment_campaigns (
                id, project_id, runner_profile_id, owner_user_id, status,
                baseline_commit, best_commit, best_metrics, experiment_branch,
                remote_ref, worktree_path, started_at, ended_at,
                trial_count, failure_count, pause_reason, queue_state,
                queue_position, active_trial_id, total_runtime_ms,
                total_cost_usd, consecutive_non_improving_trials,
                max_trials_override, gateway_url, metadata,
                created_at, updated_at, total_llm_cost_usd, total_runner_cost_usd
            ) VALUES (
                $1, $2, $3, $4, $5,
                $6, $7, $8, $9,
                $10, $11, $12, $13,
                $14, $15, $16, $17,
                $18, $19, $20, $21,
                $22, $23, $24, $25,
                $26, $27, $28, $29
            )
            "#,
            &[
                &campaign.id,
                &campaign.project_id,
                &campaign.runner_profile_id,
                &campaign.owner_user_id,
                &status_json(campaign.status),
                &campaign.baseline_commit,
                &campaign.best_commit,
                &campaign.best_metrics,
                &campaign.experiment_branch,
                &campaign.remote_ref,
                &campaign.worktree_path,
                &campaign.started_at,
                &campaign.ended_at,
                &(campaign.trial_count as i64),
                &(campaign.failure_count as i32),
                &campaign.pause_reason,
                &campaign.queue_state.as_str(),
                &queue_position,
                &campaign.active_trial_id,
                &(campaign.total_runtime_ms as i64),
                &total_cost_usd,
                &consecutive_non_improving_trials,
                &max_trials_override,
                &campaign.gateway_url,
                &campaign.metadata,
                &campaign.created_at,
                &campaign.updated_at,
                &total_llm_cost_usd,
                &total_runner_cost_usd,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn get_experiment_campaign(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentCampaign>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt("SELECT * FROM experiment_campaigns WHERE id = $1", &[&id])
            .await?;
        row.map(|row| row_to_experiment_campaign(&row)).transpose()
    }

    pub async fn list_experiment_campaigns(
        &self,
    ) -> Result<Vec<ExperimentCampaign>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT * FROM experiment_campaigns ORDER BY created_at DESC",
                &[],
            )
            .await?;
        rows.iter().map(row_to_experiment_campaign).collect()
    }

    pub async fn update_experiment_campaign(
        &self,
        campaign: &ExperimentCampaign,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let queue_position = campaign.queue_position as i32;
        let consecutive_non_improving_trials = campaign.consecutive_non_improving_trials as i32;
        let max_trials_override = campaign.max_trials_override.map(|value| value as i32);
        let total_cost_usd = decimal_from_f64(campaign.total_cost_usd, "total_cost_usd")?;
        let total_llm_cost_usd =
            decimal_from_f64(campaign.total_llm_cost_usd, "total_llm_cost_usd")?;
        let total_runner_cost_usd =
            decimal_from_f64(campaign.total_runner_cost_usd, "total_runner_cost_usd")?;
        conn.execute(
            r#"
            UPDATE experiment_campaigns SET
                project_id = $2,
                runner_profile_id = $3,
                owner_user_id = $4,
                status = $5,
                baseline_commit = $6,
                best_commit = $7,
                best_metrics = $8,
                experiment_branch = $9,
                remote_ref = $10,
                worktree_path = $11,
                started_at = $12,
                ended_at = $13,
                trial_count = $14,
                failure_count = $15,
                pause_reason = $16,
                queue_state = $17,
                queue_position = $18,
                active_trial_id = $19,
                total_runtime_ms = $20,
                total_cost_usd = $21,
                consecutive_non_improving_trials = $22,
                max_trials_override = $23,
                gateway_url = $24,
                metadata = $25,
                updated_at = $26,
                total_llm_cost_usd = $27,
                total_runner_cost_usd = $28
            WHERE id = $1
            "#,
            &[
                &campaign.id,
                &campaign.project_id,
                &campaign.runner_profile_id,
                &campaign.owner_user_id,
                &status_json(campaign.status),
                &campaign.baseline_commit,
                &campaign.best_commit,
                &campaign.best_metrics,
                &campaign.experiment_branch,
                &campaign.remote_ref,
                &campaign.worktree_path,
                &campaign.started_at,
                &campaign.ended_at,
                &(campaign.trial_count as i64),
                &(campaign.failure_count as i32),
                &campaign.pause_reason,
                &campaign.queue_state.as_str(),
                &queue_position,
                &campaign.active_trial_id,
                &(campaign.total_runtime_ms as i64),
                &total_cost_usd,
                &consecutive_non_improving_trials,
                &max_trials_override,
                &campaign.gateway_url,
                &campaign.metadata,
                &campaign.updated_at,
                &total_llm_cost_usd,
                &total_runner_cost_usd,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn create_experiment_trial(
        &self,
        trial: &ExperimentTrial,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let attributed_cost_usd =
            optional_decimal_from_f64(trial.attributed_cost_usd, "attributed_cost_usd")?;
        let llm_cost_usd = optional_decimal_from_f64(trial.llm_cost_usd, "llm_cost_usd")?;
        let runner_cost_usd = optional_decimal_from_f64(trial.runner_cost_usd, "runner_cost_usd")?;
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
                $1, $2, $3, $4, $5,
                $6, $7, $8, $9, $10,
                $11, $12, $13, $14,
                $15, $16, $17, $18,
                $19, $20,
                $21, $22, $23, $24,
                $25, $26
            )
            "#,
            &[
                &trial.id,
                &trial.campaign_id,
                &(trial.sequence as i32),
                &trial.candidate_commit,
                &trial.parent_best_commit,
                &status_json(trial.status),
                &status_json(trial.runner_backend),
                &trial.exit_code,
                &trial.metrics_json,
                &trial.summary,
                &trial.decision_reason,
                &trial.log_preview_path,
                &trial.artifact_manifest_json,
                &trial.runtime_ms.map(|value| value as i64),
                &attributed_cost_usd,
                &trial.hypothesis,
                &trial.mutation_summary,
                &trial.reviewer_decision,
                &trial.provider_job_id,
                &trial.provider_job_metadata,
                &trial.started_at,
                &trial.completed_at,
                &trial.created_at,
                &trial.updated_at,
                &llm_cost_usd,
                &runner_cost_usd,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn get_experiment_trial(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentTrial>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt("SELECT * FROM experiment_trials WHERE id = $1", &[&id])
            .await?;
        row.map(|row| row_to_experiment_trial(&row)).transpose()
    }

    pub async fn list_experiment_trials(
        &self,
        campaign_id: Uuid,
    ) -> Result<Vec<ExperimentTrial>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT * FROM experiment_trials WHERE campaign_id = $1 ORDER BY sequence ASC",
                &[&campaign_id],
            )
            .await?;
        rows.iter().map(row_to_experiment_trial).collect()
    }

    pub async fn update_experiment_trial(
        &self,
        trial: &ExperimentTrial,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let attributed_cost_usd =
            optional_decimal_from_f64(trial.attributed_cost_usd, "attributed_cost_usd")?;
        let llm_cost_usd = optional_decimal_from_f64(trial.llm_cost_usd, "llm_cost_usd")?;
        let runner_cost_usd = optional_decimal_from_f64(trial.runner_cost_usd, "runner_cost_usd")?;
        conn.execute(
            r#"
            UPDATE experiment_trials SET
                campaign_id = $2,
                sequence = $3,
                candidate_commit = $4,
                parent_best_commit = $5,
                status = $6,
                runner_backend = $7,
                exit_code = $8,
                metrics_json = $9,
                summary = $10,
                decision_reason = $11,
                log_preview_path = $12,
                artifact_manifest_json = $13,
                runtime_ms = $14,
                attributed_cost_usd = $15,
                hypothesis = $16,
                mutation_summary = $17,
                reviewer_decision = $18,
                provider_job_id = $19,
                provider_job_metadata = $20,
                started_at = $21,
                completed_at = $22,
                updated_at = $23,
                llm_cost_usd = $24,
                runner_cost_usd = $25
            WHERE id = $1
            "#,
            &[
                &trial.id,
                &trial.campaign_id,
                &(trial.sequence as i32),
                &trial.candidate_commit,
                &trial.parent_best_commit,
                &status_json(trial.status),
                &status_json(trial.runner_backend),
                &trial.exit_code,
                &trial.metrics_json,
                &trial.summary,
                &trial.decision_reason,
                &trial.log_preview_path,
                &trial.artifact_manifest_json,
                &trial.runtime_ms.map(|value| value as i64),
                &attributed_cost_usd,
                &trial.hypothesis,
                &trial.mutation_summary,
                &trial.reviewer_decision,
                &trial.provider_job_id,
                &trial.provider_job_metadata,
                &trial.started_at,
                &trial.completed_at,
                &trial.updated_at,
                &llm_cost_usd,
                &runner_cost_usd,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn replace_experiment_artifacts(
        &self,
        trial_id: Uuid,
        artifacts: &[ExperimentArtifactRef],
    ) -> Result<(), DatabaseError> {
        let mut conn = self.conn().await?;
        let tx = conn.transaction().await?;
        tx.execute(
            "DELETE FROM experiment_artifact_refs WHERE trial_id = $1",
            &[&trial_id],
        )
        .await?;
        for artifact in artifacts {
            tx.execute(
                r#"
                INSERT INTO experiment_artifact_refs (
                    id, trial_id, kind, uri_or_local_path, size_bytes,
                    fetchable, metadata, created_at
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                "#,
                &[
                    &artifact.id,
                    &artifact.trial_id,
                    &artifact.kind,
                    &artifact.uri_or_local_path,
                    &artifact.size_bytes.map(|v| v as i64),
                    &artifact.fetchable,
                    &artifact.metadata,
                    &artifact.created_at,
                ],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_experiment_artifacts(
        &self,
        trial_id: Uuid,
    ) -> Result<Vec<ExperimentArtifactRef>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT * FROM experiment_artifact_refs WHERE trial_id = $1 ORDER BY created_at ASC",
                &[&trial_id],
            )
            .await?;
        rows.iter().map(row_to_experiment_artifact).collect()
    }

    pub async fn create_experiment_target(
        &self,
        target: &ExperimentTarget,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO experiment_targets (
                id, name, kind, location, metadata, created_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7
            )
            "#,
            &[
                &target.id,
                &target.name,
                &status_json(target.kind),
                &target.location,
                &target.metadata,
                &target.created_at,
                &target.updated_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn get_experiment_target(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentTarget>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt("SELECT * FROM experiment_targets WHERE id = $1", &[&id])
            .await?;
        row.map(|row| row_to_experiment_target(&row)).transpose()
    }

    pub async fn list_experiment_targets(&self) -> Result<Vec<ExperimentTarget>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT * FROM experiment_targets ORDER BY updated_at DESC, name ASC",
                &[],
            )
            .await?;
        rows.iter().map(row_to_experiment_target).collect()
    }

    pub async fn update_experiment_target(
        &self,
        target: &ExperimentTarget,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE experiment_targets SET
                name = $2,
                kind = $3,
                location = $4,
                metadata = $5,
                updated_at = $6
            WHERE id = $1
            "#,
            &[
                &target.id,
                &target.name,
                &status_json(target.kind),
                &target.location,
                &target.metadata,
                &target.updated_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn delete_experiment_target(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn
            .execute("DELETE FROM experiment_targets WHERE id = $1", &[&id])
            .await?;
        Ok(count > 0)
    }

    pub async fn upsert_experiment_target_link(
        &self,
        link: &ExperimentTargetLink,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let route_key = link.route_key.clone().unwrap_or_default();
        let logical_role = link.logical_role.clone().unwrap_or_default();
        conn.execute(
            r#"
            INSERT INTO experiment_target_links (
                id, target_id, kind, provider, model, route_key, logical_role,
                metadata, created_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7,
                $8, $9, $10
            )
            ON CONFLICT (target_id, kind, provider, model, route_key, logical_role)
            DO UPDATE SET
                metadata = EXCLUDED.metadata,
                updated_at = EXCLUDED.updated_at
            "#,
            &[
                &link.id,
                &link.target_id,
                &status_json(link.kind),
                &link.provider,
                &link.model,
                &route_key,
                &logical_role,
                &link.metadata,
                &link.created_at,
                &link.updated_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn list_experiment_target_links(
        &self,
    ) -> Result<Vec<ExperimentTargetLink>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT * FROM experiment_target_links ORDER BY updated_at DESC, provider ASC, model ASC",
                &[],
            )
            .await?;
        rows.iter().map(row_to_experiment_target_link).collect()
    }

    pub async fn delete_experiment_target_links_for_target(
        &self,
        target_id: Uuid,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            "DELETE FROM experiment_target_links WHERE target_id = $1",
            &[&target_id],
        )
        .await?;
        Ok(())
    }

    pub async fn create_experiment_model_usage(
        &self,
        usage: &ExperimentModelUsageRecord,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let cost_usd = usage.cost_usd.and_then(rust_decimal::Decimal::from_f64);
        conn.execute(
            r#"
            INSERT INTO experiment_model_usage_records (
                id, provider, model, route_key, logical_role, endpoint_type,
                workload_tag, latency_ms, cost_usd, success,
                prompt_asset_ids, retrieval_asset_ids, tool_policy_ids,
                evaluator_ids, parser_ids,
                metadata, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9, $10,
                $11, $12, $13,
                $14, $15,
                $16, $17
            )
            "#,
            &[
                &usage.id,
                &usage.provider,
                &usage.model,
                &usage.route_key,
                &usage.logical_role,
                &usage.endpoint_type,
                &usage.workload_tag,
                &(usage.latency_ms.map(|v| v as i64)),
                &cost_usd,
                &usage.success,
                &serde_json::to_value(&usage.prompt_asset_ids)
                    .unwrap_or(serde_json::Value::Array(Vec::new())),
                &serde_json::to_value(&usage.retrieval_asset_ids)
                    .unwrap_or(serde_json::Value::Array(Vec::new())),
                &serde_json::to_value(&usage.tool_policy_ids)
                    .unwrap_or(serde_json::Value::Array(Vec::new())),
                &serde_json::to_value(&usage.evaluator_ids)
                    .unwrap_or(serde_json::Value::Array(Vec::new())),
                &serde_json::to_value(&usage.parser_ids)
                    .unwrap_or(serde_json::Value::Array(Vec::new())),
                &usage.metadata,
                &usage.created_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn list_experiment_model_usage(
        &self,
        limit: usize,
    ) -> Result<Vec<ExperimentModelUsageRecord>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT * FROM experiment_model_usage_records ORDER BY created_at DESC LIMIT $1",
                &[&(limit as i64)],
            )
            .await?;
        rows.iter().map(row_to_experiment_model_usage).collect()
    }

    pub async fn list_experiment_model_usage_for_campaign(
        &self,
        campaign_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ExperimentModelUsageRecord>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT * FROM experiment_model_usage_records WHERE metadata->>'experiment_campaign_id' = $1 ORDER BY created_at ASC LIMIT $2",
                &[&campaign_id.to_string(), &(limit as i64)],
            )
            .await?;
        rows.iter().map(row_to_experiment_model_usage).collect()
    }

    pub async fn list_experiment_model_usage_for_trial(
        &self,
        trial_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ExperimentModelUsageRecord>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT * FROM experiment_model_usage_records WHERE metadata->>'experiment_trial_id' = $1 ORDER BY created_at ASC LIMIT $2",
                &[&trial_id.to_string(), &(limit as i64)],
            )
            .await?;
        rows.iter().map(row_to_experiment_model_usage).collect()
    }

    pub async fn create_experiment_lease(
        &self,
        lease: &ExperimentLease,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO experiment_leases (
                id, campaign_id, trial_id, runner_profile_id, status,
                token_hash, job_payload, credentials_payload,
                expires_at, claimed_at, completed_at,
                created_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5,
                $6, $7, $8,
                $9, $10, $11,
                $12, $13
            )
            "#,
            &[
                &lease.id,
                &lease.campaign_id,
                &lease.trial_id,
                &lease.runner_profile_id,
                &status_json(lease.status),
                &lease.token_hash,
                &lease.job_payload,
                &lease.credentials_payload,
                &lease.expires_at,
                &lease.claimed_at,
                &lease.completed_at,
                &lease.created_at,
                &lease.updated_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn get_experiment_lease(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentLease>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt("SELECT * FROM experiment_leases WHERE id = $1", &[&id])
            .await?;
        row.map(|row| row_to_experiment_lease(&row)).transpose()
    }

    pub async fn get_experiment_lease_for_trial(
        &self,
        trial_id: Uuid,
    ) -> Result<Option<ExperimentLease>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                "SELECT * FROM experiment_leases WHERE trial_id = $1 ORDER BY created_at DESC LIMIT 1",
                &[&trial_id],
            )
            .await?;
        row.map(|row| row_to_experiment_lease(&row)).transpose()
    }

    pub async fn update_experiment_lease(
        &self,
        lease: &ExperimentLease,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE experiment_leases SET
                campaign_id = $2,
                trial_id = $3,
                runner_profile_id = $4,
                status = $5,
                token_hash = $6,
                job_payload = $7,
                credentials_payload = $8,
                expires_at = $9,
                claimed_at = $10,
                completed_at = $11,
                updated_at = $12
            WHERE id = $1
            "#,
            &[
                &lease.id,
                &lease.campaign_id,
                &lease.trial_id,
                &lease.runner_profile_id,
                &status_json(lease.status),
                &lease.token_hash,
                &lease.job_payload,
                &lease.credentials_payload,
                &lease.expires_at,
                &lease.claimed_at,
                &lease.completed_at,
                &lease.updated_at,
            ],
        )
        .await?;
        Ok(())
    }
}

#[cfg(feature = "postgres")]
fn row_to_experiment_project(
    row: &tokio_postgres::Row,
) -> Result<ExperimentProject, DatabaseError> {
    Ok(ExperimentProject {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        workspace_path: row.try_get("workspace_path")?,
        git_remote_name: row.try_get("git_remote_name")?,
        base_branch: row.try_get("base_branch")?,
        preset: from_json_value(row.try_get("preset")?)?,
        strategy_prompt: row.try_get("strategy_prompt")?,
        workdir: row.try_get("workdir")?,
        prepare_command: row.try_get("prepare_command")?,
        run_command: row.try_get("run_command")?,
        mutable_paths: from_json_value(row.try_get("mutable_paths")?)?,
        fixed_paths: from_json_value(row.try_get("fixed_paths")?)?,
        primary_metric: from_json_value(row.try_get("primary_metric")?)?,
        secondary_metrics: from_json_value(row.try_get("secondary_metrics")?)?,
        comparison_policy: from_json_value(row.try_get("comparison_policy")?)?,
        stop_policy: from_json_value(row.try_get("stop_policy")?)?,
        default_runner_profile_id: row.try_get("default_runner_profile_id")?,
        promotion_mode: row.try_get("promotion_mode")?,
        autonomy_mode: row
            .try_get::<_, Option<serde_json::Value>>("autonomy_mode")?
            .map(from_json_value)
            .transpose()?
            .unwrap_or(ExperimentAutonomyMode::Autonomous),
        status: from_json_value(row.try_get("status")?)?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

#[cfg(feature = "postgres")]
fn row_to_experiment_runner_profile(
    row: &tokio_postgres::Row,
) -> Result<ExperimentRunnerProfile, DatabaseError> {
    Ok(ExperimentRunnerProfile {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        backend: from_json_value(row.try_get("backend")?)?,
        backend_config: row.try_get("backend_config")?,
        image_or_runtime: row.try_get("image_or_runtime")?,
        gpu_requirements: row.try_get("gpu_requirements")?,
        env_grants: row.try_get("env_grants")?,
        secret_references: from_json_value(row.try_get("secret_references")?)?,
        cache_policy: row.try_get("cache_policy")?,
        status: from_json_value(row.try_get("status")?)?,
        readiness_class: row
            .try_get::<_, serde_json::Value>("readiness_class")
            .ok()
            .map(from_json_value)
            .transpose()?
            .unwrap_or(crate::experiments::ExperimentRunnerReadinessClass::ManualOnly),
        launch_eligible: row.try_get("launch_eligible").unwrap_or(false),
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

#[cfg(feature = "postgres")]
fn row_to_experiment_campaign(
    row: &tokio_postgres::Row,
) -> Result<ExperimentCampaign, DatabaseError> {
    Ok(ExperimentCampaign {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        runner_profile_id: row.try_get("runner_profile_id")?,
        owner_user_id: row
            .try_get("owner_user_id")
            .unwrap_or_else(|_| "default".to_string()),
        status: from_json_value(row.try_get("status")?)?,
        baseline_commit: row.try_get("baseline_commit")?,
        best_commit: row.try_get("best_commit")?,
        best_metrics: row.try_get("best_metrics")?,
        experiment_branch: row.try_get("experiment_branch")?,
        remote_ref: row.try_get("remote_ref")?,
        worktree_path: row.try_get("worktree_path")?,
        started_at: row.try_get("started_at")?,
        ended_at: row.try_get("ended_at")?,
        trial_count: row.try_get::<_, i64>("trial_count")? as u32,
        failure_count: row.try_get::<_, i32>("failure_count")? as u32,
        pause_reason: row.try_get("pause_reason")?,
        queue_state: row
            .try_get::<_, String>("queue_state")?
            .parse()
            .map_err(DatabaseError::Serialization)?,
        queue_position: row.try_get::<_, i32>("queue_position")? as u32,
        active_trial_id: row.try_get("active_trial_id")?,
        total_runtime_ms: row.try_get::<_, i64>("total_runtime_ms")? as u64,
        total_cost_usd: row
            .try_get::<_, Option<rust_decimal::Decimal>>("total_cost_usd")?
            .and_then(|value| value.to_string().parse::<f64>().ok())
            .unwrap_or(0.0),
        total_llm_cost_usd: row
            .try_get::<_, Option<rust_decimal::Decimal>>("total_llm_cost_usd")?
            .and_then(|value| value.to_string().parse::<f64>().ok())
            .unwrap_or(0.0),
        total_runner_cost_usd: row
            .try_get::<_, Option<rust_decimal::Decimal>>("total_runner_cost_usd")?
            .and_then(|value| value.to_string().parse::<f64>().ok())
            .unwrap_or(0.0),
        consecutive_non_improving_trials: row
            .try_get::<_, i32>("consecutive_non_improving_trials")?
            as u32,
        max_trials_override: row
            .try_get::<_, Option<i32>>("max_trials_override")?
            .map(|value| value as u32),
        gateway_url: row.try_get("gateway_url")?,
        metadata: row.try_get("metadata")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

#[cfg(feature = "postgres")]
fn row_to_experiment_trial(row: &tokio_postgres::Row) -> Result<ExperimentTrial, DatabaseError> {
    Ok(ExperimentTrial {
        id: row.try_get("id")?,
        campaign_id: row.try_get("campaign_id")?,
        sequence: row.try_get::<_, i32>("sequence")? as u32,
        candidate_commit: row.try_get("candidate_commit")?,
        parent_best_commit: row.try_get("parent_best_commit")?,
        status: from_json_value(row.try_get("status")?)?,
        runner_backend: from_json_value(row.try_get("runner_backend")?)?,
        exit_code: row.try_get("exit_code")?,
        metrics_json: row.try_get("metrics_json")?,
        summary: row.try_get("summary")?,
        decision_reason: row.try_get("decision_reason")?,
        log_preview_path: row.try_get("log_preview_path")?,
        artifact_manifest_json: row.try_get("artifact_manifest_json")?,
        runtime_ms: row
            .try_get::<_, Option<i64>>("runtime_ms")?
            .map(|value| value as u64),
        attributed_cost_usd: row
            .try_get::<_, Option<Decimal>>("attributed_cost_usd")?
            .and_then(|value| value.to_string().parse::<f64>().ok()),
        llm_cost_usd: row
            .try_get::<_, Option<Decimal>>("llm_cost_usd")?
            .and_then(|value| value.to_string().parse::<f64>().ok()),
        runner_cost_usd: row
            .try_get::<_, Option<Decimal>>("runner_cost_usd")?
            .and_then(|value| value.to_string().parse::<f64>().ok()),
        hypothesis: row.try_get("hypothesis")?,
        mutation_summary: row.try_get("mutation_summary")?,
        reviewer_decision: row.try_get("reviewer_decision")?,
        provider_job_id: row.try_get("provider_job_id")?,
        provider_job_metadata: row.try_get("provider_job_metadata")?,
        started_at: row.try_get("started_at")?,
        completed_at: row.try_get("completed_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

#[cfg(feature = "postgres")]
fn row_to_experiment_artifact(
    row: &tokio_postgres::Row,
) -> Result<ExperimentArtifactRef, DatabaseError> {
    Ok(ExperimentArtifactRef {
        id: row.try_get("id")?,
        trial_id: row.try_get("trial_id")?,
        kind: row.try_get("kind")?,
        uri_or_local_path: row.try_get("uri_or_local_path")?,
        size_bytes: row
            .try_get::<_, Option<i64>>("size_bytes")?
            .map(|v| v as u64),
        fetchable: row.try_get("fetchable")?,
        metadata: row.try_get("metadata")?,
        created_at: row.try_get("created_at")?,
    })
}

#[cfg(feature = "postgres")]
fn row_to_experiment_target(row: &tokio_postgres::Row) -> Result<ExperimentTarget, DatabaseError> {
    Ok(ExperimentTarget {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        kind: from_json_value(row.try_get("kind")?)?,
        location: row.try_get("location")?,
        metadata: row.try_get("metadata")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

#[cfg(feature = "postgres")]
fn row_to_experiment_target_link(
    row: &tokio_postgres::Row,
) -> Result<ExperimentTargetLink, DatabaseError> {
    let route_key: String = row.try_get("route_key")?;
    let logical_role: String = row.try_get("logical_role")?;
    Ok(ExperimentTargetLink {
        id: row.try_get("id")?,
        target_id: row.try_get("target_id")?,
        kind: from_json_value(row.try_get("kind")?)?,
        provider: row.try_get("provider")?,
        model: row.try_get("model")?,
        route_key: (!route_key.is_empty()).then_some(route_key),
        logical_role: (!logical_role.is_empty()).then_some(logical_role),
        metadata: row.try_get("metadata")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

#[cfg(feature = "postgres")]
fn row_to_experiment_model_usage(
    row: &tokio_postgres::Row,
) -> Result<ExperimentModelUsageRecord, DatabaseError> {
    Ok(ExperimentModelUsageRecord {
        id: row.try_get("id")?,
        provider: row.try_get("provider")?,
        model: row.try_get("model")?,
        route_key: row.try_get("route_key")?,
        logical_role: row.try_get("logical_role")?,
        endpoint_type: row.try_get("endpoint_type")?,
        workload_tag: row.try_get("workload_tag")?,
        latency_ms: row
            .try_get::<_, Option<i64>>("latency_ms")?
            .map(|v| v as u64),
        cost_usd: row
            .try_get::<_, Option<rust_decimal::Decimal>>("cost_usd")?
            .and_then(|value| value.to_string().parse::<f64>().ok()),
        success: row.try_get("success")?,
        prompt_asset_ids: from_json_value(row.try_get("prompt_asset_ids")?)?,
        retrieval_asset_ids: from_json_value(row.try_get("retrieval_asset_ids")?)?,
        tool_policy_ids: from_json_value(row.try_get("tool_policy_ids")?)?,
        evaluator_ids: from_json_value(row.try_get("evaluator_ids")?)?,
        parser_ids: from_json_value(row.try_get("parser_ids")?)?,
        metadata: row.try_get("metadata")?,
        created_at: row.try_get("created_at")?,
    })
}

#[cfg(feature = "postgres")]
fn row_to_experiment_lease(row: &tokio_postgres::Row) -> Result<ExperimentLease, DatabaseError> {
    Ok(ExperimentLease {
        id: row.try_get("id")?,
        campaign_id: row.try_get("campaign_id")?,
        trial_id: row.try_get("trial_id")?,
        runner_profile_id: row.try_get("runner_profile_id")?,
        status: from_json_value(row.try_get("status")?)?,
        token_hash: row.try_get("token_hash")?,
        job_payload: row.try_get("job_payload")?,
        credentials_payload: row.try_get("credentials_payload")?,
        expires_at: row.try_get("expires_at")?,
        claimed_at: row.try_get("claimed_at")?,
        completed_at: row.try_get("completed_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

#[cfg(feature = "postgres")]
fn status_json<T: serde::Serialize>(value: T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
}

#[cfg(feature = "postgres")]
fn from_json_value<T: serde::de::DeserializeOwned>(
    value: serde_json::Value,
) -> Result<T, DatabaseError> {
    serde_json::from_value(value).map_err(|e| DatabaseError::Serialization(e.to_string()))
}

#[cfg(feature = "postgres")]
fn decimal_from_f64(value: f64, field: &str) -> Result<Decimal, DatabaseError> {
    Decimal::from_f64(value)
        .ok_or_else(|| DatabaseError::Serialization(format!("invalid decimal in {field}: {value}")))
}

#[cfg(feature = "postgres")]
fn optional_decimal_from_f64(
    value: Option<f64>,
    field: &str,
) -> Result<Option<Decimal>, DatabaseError> {
    value
        .map(|inner| decimal_from_f64(inner, field))
        .transpose()
}

#[cfg(feature = "postgres")]
#[allow(dead_code)]
fn _type_use_sanity(
    _a: ExperimentCampaignStatus,
    _b: ExperimentLeaseStatus,
    _c: ExperimentMetricComparator,
    _d: ExperimentMetricDefinition,
    _e: ExperimentPreset,
    _f: ExperimentProjectStatus,
    _g: ExperimentRunnerStatus,
    _h: ExperimentTrialStatus,
    _i: ExperimentRunnerBackend,
    _j: DateTime<Utc>,
) {
}
