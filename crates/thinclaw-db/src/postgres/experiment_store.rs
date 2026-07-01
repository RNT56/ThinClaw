//! postgres: experiment_store.

use super::*;

#[async_trait]
impl ExperimentStore for PgBackend {
    async fn create_experiment_project(
        &self,
        project: &ExperimentProject,
    ) -> Result<(), DatabaseError> {
        self.store.create_experiment_project(project).await
    }

    async fn get_experiment_project(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentProject>, DatabaseError> {
        self.store.get_experiment_project(id).await
    }

    async fn list_experiment_projects(&self) -> Result<Vec<ExperimentProject>, DatabaseError> {
        self.store.list_experiment_projects().await
    }

    async fn update_experiment_project(
        &self,
        project: &ExperimentProject,
    ) -> Result<(), DatabaseError> {
        self.store.update_experiment_project(project).await
    }

    async fn delete_experiment_project(&self, id: Uuid) -> Result<bool, DatabaseError> {
        self.store.delete_experiment_project(id).await
    }

    async fn create_experiment_runner_profile(
        &self,
        profile: &ExperimentRunnerProfile,
    ) -> Result<(), DatabaseError> {
        self.store.create_experiment_runner_profile(profile).await
    }

    async fn get_experiment_runner_profile(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentRunnerProfile>, DatabaseError> {
        self.store.get_experiment_runner_profile(id).await
    }

    async fn list_experiment_runner_profiles(
        &self,
    ) -> Result<Vec<ExperimentRunnerProfile>, DatabaseError> {
        self.store.list_experiment_runner_profiles().await
    }

    async fn update_experiment_runner_profile(
        &self,
        profile: &ExperimentRunnerProfile,
    ) -> Result<(), DatabaseError> {
        self.store.update_experiment_runner_profile(profile).await
    }

    async fn delete_experiment_runner_profile(&self, id: Uuid) -> Result<bool, DatabaseError> {
        self.store.delete_experiment_runner_profile(id).await
    }

    async fn create_experiment_campaign(
        &self,
        campaign: &ExperimentCampaign,
    ) -> Result<(), DatabaseError> {
        self.store.create_experiment_campaign(campaign).await
    }

    async fn get_experiment_campaign(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentCampaign>, DatabaseError> {
        self.store.get_experiment_campaign(id).await
    }

    async fn get_experiment_campaign_for_owner(
        &self,
        id: Uuid,
        owner_user_id: &str,
    ) -> Result<Option<ExperimentCampaign>, DatabaseError> {
        self.store
            .get_experiment_campaign_for_owner(id, owner_user_id)
            .await
    }

    async fn list_experiment_campaigns(&self) -> Result<Vec<ExperimentCampaign>, DatabaseError> {
        self.store.list_experiment_campaigns().await
    }

    async fn list_experiment_campaigns_for_owner(
        &self,
        owner_user_id: &str,
    ) -> Result<Vec<ExperimentCampaign>, DatabaseError> {
        self.store
            .list_experiment_campaigns_for_owner(owner_user_id)
            .await
    }

    async fn update_experiment_campaign(
        &self,
        campaign: &ExperimentCampaign,
    ) -> Result<(), DatabaseError> {
        self.store.update_experiment_campaign(campaign).await
    }

    async fn create_experiment_trial(&self, trial: &ExperimentTrial) -> Result<(), DatabaseError> {
        self.store.create_experiment_trial(trial).await
    }

    async fn get_experiment_trial(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentTrial>, DatabaseError> {
        self.store.get_experiment_trial(id).await
    }

    async fn get_experiment_trial_for_owner(
        &self,
        id: Uuid,
        owner_user_id: &str,
    ) -> Result<Option<ExperimentTrial>, DatabaseError> {
        self.store
            .get_experiment_trial_for_owner(id, owner_user_id)
            .await
    }

    async fn list_experiment_trials(
        &self,
        campaign_id: Uuid,
    ) -> Result<Vec<ExperimentTrial>, DatabaseError> {
        self.store.list_experiment_trials(campaign_id).await
    }

    async fn list_experiment_trials_for_owner(
        &self,
        campaign_id: Uuid,
        owner_user_id: &str,
    ) -> Result<Vec<ExperimentTrial>, DatabaseError> {
        self.store
            .list_experiment_trials_for_owner(campaign_id, owner_user_id)
            .await
    }

    async fn update_experiment_trial(&self, trial: &ExperimentTrial) -> Result<(), DatabaseError> {
        self.store.update_experiment_trial(trial).await
    }

    async fn replace_experiment_artifacts(
        &self,
        trial_id: Uuid,
        artifacts: &[ExperimentArtifactRef],
    ) -> Result<(), DatabaseError> {
        self.store
            .replace_experiment_artifacts(trial_id, artifacts)
            .await
    }

    async fn list_experiment_artifacts(
        &self,
        trial_id: Uuid,
    ) -> Result<Vec<ExperimentArtifactRef>, DatabaseError> {
        self.store.list_experiment_artifacts(trial_id).await
    }

    async fn list_experiment_artifacts_for_owner(
        &self,
        trial_id: Uuid,
        owner_user_id: &str,
    ) -> Result<Vec<ExperimentArtifactRef>, DatabaseError> {
        self.store
            .list_experiment_artifacts_for_owner(trial_id, owner_user_id)
            .await
    }

    async fn create_experiment_target(
        &self,
        target: &ExperimentTarget,
    ) -> Result<(), DatabaseError> {
        self.store.create_experiment_target(target).await
    }

    async fn get_experiment_target(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentTarget>, DatabaseError> {
        self.store.get_experiment_target(id).await
    }

    async fn list_experiment_targets(&self) -> Result<Vec<ExperimentTarget>, DatabaseError> {
        self.store.list_experiment_targets().await
    }

    async fn update_experiment_target(
        &self,
        target: &ExperimentTarget,
    ) -> Result<(), DatabaseError> {
        self.store.update_experiment_target(target).await
    }

    async fn delete_experiment_target(&self, id: Uuid) -> Result<bool, DatabaseError> {
        self.store.delete_experiment_target(id).await
    }

    async fn upsert_experiment_target_link(
        &self,
        link: &ExperimentTargetLink,
    ) -> Result<(), DatabaseError> {
        self.store.upsert_experiment_target_link(link).await
    }

    async fn list_experiment_target_links(
        &self,
    ) -> Result<Vec<ExperimentTargetLink>, DatabaseError> {
        self.store.list_experiment_target_links().await
    }

    async fn delete_experiment_target_links_for_target(
        &self,
        target_id: Uuid,
    ) -> Result<(), DatabaseError> {
        self.store
            .delete_experiment_target_links_for_target(target_id)
            .await
    }

    async fn create_experiment_model_usage(
        &self,
        usage: &ExperimentModelUsageRecord,
    ) -> Result<(), DatabaseError> {
        self.store.create_experiment_model_usage(usage).await
    }

    async fn list_experiment_model_usage(
        &self,
        limit: usize,
    ) -> Result<Vec<ExperimentModelUsageRecord>, DatabaseError> {
        self.store.list_experiment_model_usage(limit).await
    }

    async fn list_experiment_model_usage_for_campaign(
        &self,
        campaign_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ExperimentModelUsageRecord>, DatabaseError> {
        self.store
            .list_experiment_model_usage_for_campaign(campaign_id, limit)
            .await
    }

    async fn list_experiment_model_usage_for_trial(
        &self,
        trial_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ExperimentModelUsageRecord>, DatabaseError> {
        self.store
            .list_experiment_model_usage_for_trial(trial_id, limit)
            .await
    }

    async fn create_experiment_lease(&self, lease: &ExperimentLease) -> Result<(), DatabaseError> {
        self.store.create_experiment_lease(lease).await
    }

    async fn get_experiment_lease(
        &self,
        id: Uuid,
    ) -> Result<Option<ExperimentLease>, DatabaseError> {
        self.store.get_experiment_lease(id).await
    }

    async fn get_experiment_lease_for_trial(
        &self,
        trial_id: Uuid,
    ) -> Result<Option<ExperimentLease>, DatabaseError> {
        self.store.get_experiment_lease_for_trial(trial_id).await
    }

    async fn update_experiment_lease(&self, lease: &ExperimentLease) -> Result<(), DatabaseError> {
        self.store.update_experiment_lease(lease).await
    }
}

// ==================== RepoProjectStore ====================
