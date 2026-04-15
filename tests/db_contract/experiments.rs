use std::sync::Arc;

use thinclaw::api::experiments as experiments_api;

use crate::db_contract::fixtures;
use crate::db_contract::support::contract_db_or_skip;

#[tokio::test]
async fn experiments_core_crud_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let mut project = fixtures::experiment_project();
    ctx.db
        .create_experiment_project(&project)
        .await
        .expect("create_experiment_project should succeed");
    let loaded_project = ctx
        .db
        .get_experiment_project(project.id)
        .await
        .expect("get_experiment_project should succeed")
        .expect("project should exist");
    assert_eq!(loaded_project.id, project.id);

    let runner = fixtures::experiment_runner_profile();
    ctx.db
        .create_experiment_runner_profile(&runner)
        .await
        .expect("create_experiment_runner_profile should succeed");

    project.default_runner_profile_id = Some(runner.id);
    ctx.db
        .update_experiment_project(&project)
        .await
        .expect("update_experiment_project should succeed");

    let campaign = fixtures::experiment_campaign(project.id, runner.id);
    ctx.db
        .create_experiment_campaign(&campaign)
        .await
        .expect("create_experiment_campaign should succeed");
    let loaded_campaign = ctx
        .db
        .get_experiment_campaign(campaign.id)
        .await
        .expect("get_experiment_campaign should succeed")
        .expect("campaign should exist");
    assert_eq!(loaded_campaign.project_id, project.id);

    let trial = fixtures::experiment_trial(campaign.id, 1);
    ctx.db
        .create_experiment_trial(&trial)
        .await
        .expect("create_experiment_trial should succeed");

    let trials = ctx
        .db
        .list_experiment_trials(campaign.id)
        .await
        .expect("list_experiment_trials should succeed");
    assert_eq!(trials.len(), 1);
    assert_eq!(trials[0].id, trial.id);

    ctx.db
        .replace_experiment_artifacts(
            trial.id,
            &[thinclaw::experiments::ExperimentArtifactRef {
                id: uuid::Uuid::new_v4(),
                trial_id: trial.id,
                kind: "log".to_string(),
                uri_or_local_path: "/tmp/contract.log".to_string(),
                size_bytes: Some(12),
                fetchable: true,
                metadata: serde_json::json!({"source":"contract"}),
                created_at: chrono::Utc::now(),
            }],
        )
        .await
        .expect("replace_experiment_artifacts should succeed");
    let artifacts = ctx
        .db
        .list_experiment_artifacts(trial.id)
        .await
        .expect("list_experiment_artifacts should succeed");
    assert_eq!(artifacts.len(), 1);
}

#[tokio::test]
async fn experiments_targets_usage_and_lease_contract() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let project = fixtures::experiment_project();
    let runner = fixtures::experiment_runner_profile();
    ctx.db
        .create_experiment_project(&project)
        .await
        .expect("create_experiment_project should succeed");
    ctx.db
        .create_experiment_runner_profile(&runner)
        .await
        .expect("create_experiment_runner_profile should succeed");

    let campaign = fixtures::experiment_campaign(project.id, runner.id);
    let trial = fixtures::experiment_trial(campaign.id, 1);
    ctx.db
        .create_experiment_campaign(&campaign)
        .await
        .expect("create_experiment_campaign should succeed");
    ctx.db
        .create_experiment_trial(&trial)
        .await
        .expect("create_experiment_trial should succeed");

    let target = fixtures::experiment_target();
    ctx.db
        .create_experiment_target(&target)
        .await
        .expect("create_experiment_target should succeed");

    let link = fixtures::experiment_target_link(target.id);
    ctx.db
        .upsert_experiment_target_link(&link)
        .await
        .expect("upsert_experiment_target_link should succeed");

    let links = ctx
        .db
        .list_experiment_target_links()
        .await
        .expect("list_experiment_target_links should succeed");
    assert!(links.iter().any(|entry| entry.id == link.id));

    let usage = fixtures::experiment_model_usage();
    ctx.db
        .create_experiment_model_usage(&usage)
        .await
        .expect("create_experiment_model_usage should succeed");
    let usage_rows = ctx
        .db
        .list_experiment_model_usage(50)
        .await
        .expect("list_experiment_model_usage should succeed");
    assert!(usage_rows.iter().any(|entry| entry.id == usage.id));

    let lease = fixtures::experiment_lease(campaign.id, trial.id, runner.id);
    ctx.db
        .create_experiment_lease(&lease)
        .await
        .expect("create_experiment_lease should succeed");
    let loaded_lease = ctx
        .db
        .get_experiment_lease(lease.id)
        .await
        .expect("get_experiment_lease should succeed")
        .expect("lease should exist");
    assert_eq!(loaded_lease.trial_id, trial.id);
}

#[tokio::test]
async fn experiments_opportunities_include_outcome_backed_signals() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };

    let user_id = fixtures::user("experiments_outcome_user");
    ctx.db
        .set_setting(&user_id, "experiments.enabled", &serde_json::json!(true))
        .await
        .expect("experiments.enabled should be set");

    let mut contract = fixtures::outcome_contract(&user_id);
    contract.status = "evaluated".to_string();
    contract.final_verdict = Some("negative".to_string());
    contract.final_score = Some(-1.0);
    contract.contract_type = "tool_durability".to_string();
    contract.source_kind = "artifact_version".to_string();
    contract.metadata = serde_json::json!({
        "pattern_key": "artifact:prompt:USER.md",
        "artifact_type": "prompt",
        "artifact_name": "USER.md"
    });
    contract.evaluated_at = Some(chrono::Utc::now());
    ctx.db
        .insert_outcome_contract(&contract)
        .await
        .expect("insert_outcome_contract should succeed");

    let response = experiments_api::list_opportunities(&Arc::clone(&ctx.db), &user_id, 20)
        .await
        .expect("list_opportunities should succeed");
    let opportunity = response
        .opportunities
        .iter()
        .find(|entry| entry.source.as_deref() == Some("outcome_learning"))
        .expect("expected outcome-backed opportunity");

    assert_eq!(
        opportunity.opportunity_type,
        thinclaw::experiments::ExperimentTargetKind::PromptAsset
    );
    assert!(
        opportunity
            .signals
            .iter()
            .any(|signal| signal.contains("negative outcome")),
        "expected negative outcome signal chips"
    );
    assert!(
        opportunity
            .summary
            .to_ascii_lowercase()
            .contains("negative outcome"),
        "expected outcome-backed summary"
    );
    let project_hint = opportunity
        .project_hint
        .as_ref()
        .expect("expected outcome-backed project hint");
    assert_eq!(
        project_hint
            .get("metric_name")
            .and_then(|value| value.as_str()),
        Some("outcome_success_rate")
    );
    assert!(
        project_hint
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .contains("USER.md"),
        "expected prompt benchmark project hint name"
    );
}
