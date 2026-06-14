//! Merge-gate evidence aggregation for repo project supervision.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thinclaw_repo_projects::{
    MergeGateDecision, MergeGateDenialReason, MergeGateInput, ProjectPolicy, RepoProjectEvent,
    RepoProjectEventKind, RepoProjectRepo, RepoProjectTask, evaluate_repo_project_merge_gate,
    has_recorded_merge_gate_event,
};
use uuid::Uuid;

use super::ci::CiSuiteClassification;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewEvidence {
    #[serde(default)]
    pub approvals: u32,
    #[serde(default)]
    pub required_approvals: u32,
    #[serde(default)]
    pub changes_requested: u32,
    #[serde(default)]
    pub unresolved_threads: u32,
}

impl ReviewEvidence {
    pub fn blocking_reviews(&self) -> bool {
        self.changes_requested > 0
            || self.unresolved_threads > 0
            || self.approvals < self.required_approvals
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeGateEvidence {
    pub checks_green: bool,
    pub branch_up_to_date: bool,
    pub reviews: ReviewEvidence,
    #[serde(default)]
    pub security_findings: u32,
    #[serde(default)]
    pub secrets_findings: u32,
    #[serde(default)]
    pub gate_event_recorded: bool,
}

impl MergeGateEvidence {
    pub fn clean() -> Self {
        Self {
            checks_green: true,
            branch_up_to_date: true,
            reviews: ReviewEvidence {
                approvals: 1,
                required_approvals: 0,
                changes_requested: 0,
                unresolved_threads: 0,
            },
            security_findings: 0,
            secrets_findings: 0,
            gate_event_recorded: true,
        }
    }

    pub fn with_ci_suite(mut self, suite: &CiSuiteClassification) -> Self {
        self.checks_green = suite.checks_green;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergeGateEvaluationRecord {
    pub input: MergeGateInput,
    pub decision: MergeGateDecision,
    pub event: RepoProjectEvent,
}

pub fn merge_gate_input_from_evidence(
    repo: &RepoProjectRepo,
    task_id: Uuid,
    evidence: &MergeGateEvidence,
    prior_events: &[RepoProjectEvent],
) -> MergeGateInput {
    MergeGateInput {
        repo_enrolled: repo.enrolled,
        checks_green: evidence.checks_green,
        branch_up_to_date: evidence.branch_up_to_date,
        blocking_reviews: evidence.reviews.blocking_reviews(),
        security_findings: evidence.security_findings > 0,
        secrets_findings: evidence.secrets_findings > 0,
        gate_event_recorded: evidence.gate_event_recorded
            || has_recorded_merge_gate_event(prior_events, task_id),
    }
}

pub fn evaluate_merge_gate_evidence(
    policy: &ProjectPolicy,
    repo: &RepoProjectRepo,
    task: &RepoProjectTask,
    evidence: &MergeGateEvidence,
    prior_events: &[RepoProjectEvent],
) -> MergeGateEvaluationRecord {
    evaluate_merge_gate_evidence_at(
        policy,
        repo,
        task,
        evidence,
        prior_events,
        Uuid::new_v4(),
        Utc::now(),
    )
}

pub fn evaluate_merge_gate_evidence_at(
    policy: &ProjectPolicy,
    repo: &RepoProjectRepo,
    task: &RepoProjectTask,
    evidence: &MergeGateEvidence,
    prior_events: &[RepoProjectEvent],
    event_id: Uuid,
    now: DateTime<Utc>,
) -> MergeGateEvaluationRecord {
    let input = merge_gate_input_from_evidence(repo, task.id, evidence, prior_events);
    let decision = evaluate_repo_project_merge_gate(policy, input);
    let message = merge_gate_message(task, &decision);
    let event = RepoProjectEvent {
        id: event_id,
        project_id: task.project_id,
        repo_id: Some(repo.id),
        task_id: Some(task.id),
        project_run_id: None,
        worker_run_id: None,
        kind: RepoProjectEventKind::MergeGateEvaluated,
        message,
        details: serde_json::json!({
            "input": input,
            "decision": decision,
            "evidence": evidence,
            "repo": {
                "owner": repo.owner,
                "repo": repo.repo,
            },
            "task": {
                "title": task.title,
                "base_branch": task.base_branch,
                "branch_name": task.branch_name,
                "head_sha": task.head_sha,
                "pull_request_number": task.pull_request_number,
            },
        }),
        created_at: now,
    };

    MergeGateEvaluationRecord {
        input,
        decision,
        event,
    }
}

pub fn merge_gate_message(task: &RepoProjectTask, decision: &MergeGateDecision) -> String {
    if decision.approved {
        format!("merge gate approved for task {}", task.id)
    } else {
        format!(
            "merge gate denied for task {}: {}",
            task.id,
            denial_reason_list(&decision.reasons)
        )
    }
}

pub fn denial_reason_label(reason: MergeGateDenialReason) -> &'static str {
    match reason {
        MergeGateDenialReason::AutoMergeDisabled => "auto_merge_disabled",
        MergeGateDenialReason::RepoNotEnrolled => "repo_not_enrolled",
        MergeGateDenialReason::ChecksNotGreen => "checks_not_green",
        MergeGateDenialReason::BranchOutOfDate => "branch_out_of_date",
        MergeGateDenialReason::BlockingReviews => "blocking_reviews",
        MergeGateDenialReason::SecurityFindings => "security_findings",
        MergeGateDenialReason::SecretsFindings => "secrets_findings",
        MergeGateDenialReason::GateEventMissing => "gate_event_missing",
    }
}

fn denial_reason_list(reasons: &[MergeGateDenialReason]) -> String {
    if reasons.is_empty() {
        return "none".to_string();
    }

    reasons
        .iter()
        .copied()
        .map(denial_reason_label)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use thinclaw_repo_projects::{
        CodingBackend, GitHubAuthMode, MergeMethod, RepoProjectEvent, RepoProjectTaskState,
    };

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("test timestamp should be valid")
    }

    fn policy(auto_merge: bool) -> ProjectPolicy {
        ProjectPolicy {
            auto_merge,
            merge_method: MergeMethod::Squash,
            default_coding_backend: CodingBackend::CodexCode,
            github_auth_mode: GitHubAuthMode::GitHubApp,
            max_parallel_tasks: 1,
        }
    }

    fn repo(project_id: Uuid, enrolled: bool) -> RepoProjectRepo {
        RepoProjectRepo {
            id: Uuid::parse_str("018fda1d-6b19-7f5f-b5bb-a7d997fe1002").unwrap(),
            project_id,
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            github_repo_id: None,
            installation_id: None,
            default_branch: "main".to_string(),
            base_branch: Some("main".to_string()),
            enrolled,
            local_path: None,
            auth_mode: GitHubAuthMode::GitHubApp,
            metadata: serde_json::json!({}),
            created_at: now(),
            updated_at: now(),
        }
    }

    fn task(project_id: Uuid, repo_id: Uuid) -> RepoProjectTask {
        RepoProjectTask {
            id: Uuid::parse_str("018fda1d-6b19-7f5f-b5bb-a7d997fe1003").unwrap(),
            project_id,
            repo_id,
            title: "Merge me".to_string(),
            body: None,
            state: RepoProjectTaskState::WaitingReview,
            coding_backend: CodingBackend::CodexCode,
            base_branch: "main".to_string(),
            branch_name: "thinclaw/project/task".to_string(),
            head_sha: Some("abc123".to_string()),
            pull_request_number: Some(7),
            pull_request_url: None,
            github_issue_number: None,
            assigned_worker_id: None,
            priority: 0,
            labels: vec![],
            metadata: serde_json::json!({}),
            created_at: now(),
            updated_at: now(),
            queued_at: None,
            started_at: None,
            completed_at: None,
        }
    }

    fn recorded_event(project_id: Uuid, task_id: Uuid) -> RepoProjectEvent {
        RepoProjectEvent {
            id: Uuid::parse_str("018fda1d-6b19-7f5f-b5bb-a7d997fe1004").unwrap(),
            project_id,
            repo_id: None,
            task_id: Some(task_id),
            project_run_id: None,
            worker_run_id: None,
            kind: RepoProjectEventKind::MergeGateEvaluated,
            message: "previous gate".to_string(),
            details: serde_json::json!({}),
            created_at: now(),
        }
    }

    #[test]
    fn merge_gate_denies_all_missing_or_blocking_evidence_in_domain_order() {
        let project_id = Uuid::parse_str("018fda1d-6b19-7f5f-b5bb-a7d997fe1001").unwrap();
        let repo = repo(project_id, false);
        let task = task(project_id, repo.id);
        let evidence = MergeGateEvidence {
            checks_green: false,
            branch_up_to_date: false,
            reviews: ReviewEvidence {
                approvals: 0,
                required_approvals: 1,
                changes_requested: 1,
                unresolved_threads: 2,
            },
            security_findings: 1,
            secrets_findings: 1,
            gate_event_recorded: false,
        };

        let record = evaluate_merge_gate_evidence_at(
            &policy(false),
            &repo,
            &task,
            &evidence,
            &[],
            Uuid::parse_str("018fda1d-6b19-7f5f-b5bb-a7d997fe1005").unwrap(),
            now(),
        );

        assert!(!record.decision.approved);
        assert_eq!(
            record.decision.reasons,
            vec![
                MergeGateDenialReason::AutoMergeDisabled,
                MergeGateDenialReason::RepoNotEnrolled,
                MergeGateDenialReason::ChecksNotGreen,
                MergeGateDenialReason::BranchOutOfDate,
                MergeGateDenialReason::BlockingReviews,
                MergeGateDenialReason::SecurityFindings,
                MergeGateDenialReason::SecretsFindings,
                MergeGateDenialReason::GateEventMissing,
            ]
        );
        assert_eq!(record.event.kind, RepoProjectEventKind::MergeGateEvaluated);
        assert!(record.event.message.contains("checks_not_green"));
    }

    #[test]
    fn merge_gate_uses_prior_gate_event_as_recorded_evidence() {
        let project_id = Uuid::parse_str("018fda1d-6b19-7f5f-b5bb-a7d997fe1001").unwrap();
        let repo = repo(project_id, true);
        let task = task(project_id, repo.id);
        let evidence = MergeGateEvidence {
            gate_event_recorded: false,
            ..MergeGateEvidence::clean()
        };
        let prior = [recorded_event(project_id, task.id)];

        let record = evaluate_merge_gate_evidence_at(
            &policy(true),
            &repo,
            &task,
            &evidence,
            &prior,
            Uuid::parse_str("018fda1d-6b19-7f5f-b5bb-a7d997fe1006").unwrap(),
            now(),
        );

        assert!(record.decision.approved);
        assert!(record.input.gate_event_recorded);
        assert_eq!(
            record.event.message,
            format!("merge gate approved for task {}", task.id)
        );
    }

    #[test]
    fn review_evidence_blocks_on_missing_required_approval() {
        let reviews = ReviewEvidence {
            approvals: 0,
            required_approvals: 1,
            changes_requested: 0,
            unresolved_threads: 0,
        };

        assert!(reviews.blocking_reviews());
    }
}
