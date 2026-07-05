//! Deterministic task packets for Codex/Claude repo-project worker jobs.

use serde::{Deserialize, Serialize};
use thinclaw_repo_projects::{
    CodingBackend, MergeGateDecision, MergeGateDenialReason, RepoProject, RepoProjectRepo,
    RepoProjectTask,
};
use uuid::Uuid;

use super::ci::{CiClassification, CiFailureKind, redact_sensitive_text};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPacketKind {
    Implementation,
    Review,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoTaskPacket {
    pub packet_id: String,
    pub kind: TaskPacketKind,
    pub backend: CodingBackend,
    pub project_id: Uuid,
    pub repo_id: Uuid,
    pub task_id: Uuid,
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Copy)]
pub struct RepoTaskPacketInput<'a> {
    pub project: &'a RepoProject,
    pub repo: &'a RepoProjectRepo,
    pub task: &'a RepoProjectTask,
    pub worktree_path: Option<&'a str>,
    pub ci: Option<&'a CiClassification>,
    pub merge_gate: Option<&'a MergeGateDecision>,
    pub extra_context: &'a [(&'a str, &'a str)],
}

pub fn build_implementation_packet(
    input: RepoTaskPacketInput<'_>,
    backend: CodingBackend,
) -> RepoTaskPacket {
    build_packet(input, backend, TaskPacketKind::Implementation)
}

pub fn build_review_packet(
    input: RepoTaskPacketInput<'_>,
    backend: CodingBackend,
) -> RepoTaskPacket {
    build_packet(input, backend, TaskPacketKind::Review)
}

pub fn packet_kind_label(kind: TaskPacketKind) -> &'static str {
    match kind {
        TaskPacketKind::Implementation => "implementation",
        TaskPacketKind::Review => "review",
    }
}

pub fn backend_label(backend: CodingBackend) -> &'static str {
    match backend {
        CodingBackend::CodexCode => "codex_code",
        CodingBackend::ClaudeCode => "claude_code",
        CodingBackend::Worker => "worker",
    }
}

fn build_packet(
    input: RepoTaskPacketInput<'_>,
    backend: CodingBackend,
    kind: TaskPacketKind,
) -> RepoTaskPacket {
    let packet_id = packet_id(input.project.id, input.task.id, kind, backend);
    let prompt = build_prompt(&packet_id, input, backend, kind);
    let metadata = serde_json::json!({
        "packet_id": packet_id,
        "kind": packet_kind_label(kind),
        "backend": backend_label(backend),
        "project_id": input.project.id,
        "repo_id": input.repo.id,
        "task_id": input.task.id,
        "branch_name": input.task.branch_name,
        "base_branch": input.task.base_branch,
        "write_mode": input.project.policy.write_mode.as_str(),
        "pull_request_number": input.task.pull_request_number,
    });

    RepoTaskPacket {
        packet_id,
        kind,
        backend,
        project_id: input.project.id,
        repo_id: input.repo.id,
        task_id: input.task.id,
        title: redact_sensitive_text(&input.task.title),
        prompt,
        metadata,
    }
}

fn build_prompt(
    packet_id: &str,
    input: RepoTaskPacketInput<'_>,
    backend: CodingBackend,
    kind: TaskPacketKind,
) -> String {
    let mut lines = Vec::new();
    lines.push("# Repo Project Task Packet".to_string());
    lines.push(format!("packet_id: {packet_id}"));
    lines.push(format!("mode: {}", packet_kind_label(kind)));
    lines.push(format!("backend: {}", backend_label(backend)));
    lines.push(format!(
        "write_mode: {}",
        input.project.policy.write_mode.as_str()
    ));
    lines.push(format!(
        "project: {} ({})",
        redact_sensitive_text(&input.project.name),
        redact_sensitive_text(&input.project.slug)
    ));
    lines.push(format!(
        "repo: {}/{}",
        redact_sensitive_text(&input.repo.owner),
        redact_sensitive_text(&input.repo.repo)
    ));
    lines.push(format!("task_id: {}", input.task.id));
    lines.push(format!(
        "base_branch: {}",
        redact_sensitive_text(&input.task.base_branch)
    ));
    lines.push(format!(
        "task_branch: {}",
        redact_sensitive_text(&input.task.branch_name)
    ));
    if let Some(head_sha) = &input.task.head_sha {
        lines.push(format!("head_sha: {}", redact_sensitive_text(head_sha)));
    }
    if let Some(pr_number) = input.task.pull_request_number {
        lines.push(format!("pull_request: #{pr_number}"));
    }
    if let Some(pr_url) = &input.task.pull_request_url {
        lines.push(format!(
            "pull_request_url: {}",
            redact_sensitive_text(pr_url)
        ));
    }
    if let Some(worktree_path) = input.worktree_path {
        lines.push(format!(
            "worktree_path: {}",
            redact_sensitive_text(worktree_path)
        ));
    }
    lines.push(format!("priority: {}", input.task.priority));
    lines.push(format!("labels: {}", sorted_labels(&input.task.labels)));
    lines.push(String::new());

    lines.push("## Task".to_string());
    lines.push(redact_sensitive_text(&input.task.title));
    if let Some(body) = &input.task.body {
        lines.push(String::new());
        lines.push(redact_sensitive_text(body));
    }
    lines.push(String::new());

    if let Some(ci) = input.ci {
        append_ci_section(&mut lines, ci);
    }
    if let Some(decision) = input.merge_gate {
        append_merge_gate_section(&mut lines, decision);
    }
    append_extra_context(&mut lines, input.extra_context);
    append_instructions(&mut lines, kind);

    lines.join("\n")
}

fn append_ci_section(lines: &mut Vec<String>, ci: &CiClassification) {
    lines.push("## CI Context".to_string());
    lines.push(format!("check: {}", redact_sensitive_text(&ci.name)));
    lines.push(format!("outcome: {:?}", ci.outcome));
    if let Some(kind) = ci.failure_kind {
        lines.push(format!("failure_kind: {}", ci_failure_kind_label(kind)));
    }
    if let Some(recommendation) = &ci.recommendation {
        lines.push(format!("repair_action: {:?}", recommendation.action));
        lines.push(format!(
            "repair_summary: {}",
            redact_sensitive_text(&recommendation.summary)
        ));
        lines.push(format!(
            "repair_hint: {}",
            redact_sensitive_text(&recommendation.prompt_hint)
        ));
    }
    if !ci.evidence.is_empty() {
        lines.push("evidence:".to_string());
        for line in &ci.evidence {
            lines.push(format!("- {}", redact_sensitive_text(line)));
        }
    }
    lines.push(String::new());
}

fn append_merge_gate_section(lines: &mut Vec<String>, decision: &MergeGateDecision) {
    lines.push("## Merge Gate".to_string());
    lines.push(format!("approved: {}", decision.approved));
    lines.push(format!("merge_method: {:?}", decision.merge_method));
    if !decision.reasons.is_empty() {
        lines.push(format!(
            "denial_reasons: {}",
            denial_reason_list(&decision.reasons)
        ));
    }
    lines.push(String::new());
}

fn append_extra_context(lines: &mut Vec<String>, context: &[(&str, &str)]) {
    if context.is_empty() {
        return;
    }

    let mut sorted = context.to_vec();
    sorted.sort_by(|left, right| left.0.cmp(right.0).then_with(|| left.1.cmp(right.1)));
    lines.push("## Additional Context".to_string());
    for (key, value) in sorted {
        lines.push(format!(
            "{}: {}",
            redact_sensitive_text(key),
            redact_sensitive_text(value)
        ));
    }
    lines.push(String::new());
}

fn append_instructions(lines: &mut Vec<String>, kind: TaskPacketKind) {
    lines.push("## Operating Instructions".to_string());
    match kind {
        TaskPacketKind::Implementation => {
            lines.push("- Implement the task on the task branch only.".to_string());
            lines.push(
                "- Preserve unrelated uncommitted changes and do not revert user edits."
                    .to_string(),
            );
            lines.push(
                "- Run focused checks that cover the changed behavior when available.".to_string(),
            );
            lines.push("- Summarize changed files, tests run, and remaining gaps.".to_string());
        }
        TaskPacketKind::Review => {
            lines.push("- Review the task branch against the base branch.".to_string());
            lines.push(
                "- Prioritize correctness, security, regression risk, and missing tests."
                    .to_string(),
            );
            lines.push(
                "- Return findings first with concrete file and line references when possible."
                    .to_string(),
            );
            lines.push("- Do not make code changes during review.".to_string());
        }
    }
}

fn packet_id(
    project_id: Uuid,
    task_id: Uuid,
    kind: TaskPacketKind,
    backend: CodingBackend,
) -> String {
    format!(
        "repo-project:{project_id}:task:{task_id}:{}:{}",
        packet_kind_label(kind),
        backend_label(backend)
    )
}

fn sorted_labels(labels: &[String]) -> String {
    if labels.is_empty() {
        return "none".to_string();
    }

    let mut labels = labels
        .iter()
        .map(|label| redact_sensitive_text(label))
        .collect::<Vec<_>>();
    labels.sort();
    labels.dedup();
    labels.join(", ")
}

fn denial_reason_list(reasons: &[MergeGateDenialReason]) -> String {
    reasons
        .iter()
        .copied()
        .map(denial_reason_label)
        .collect::<Vec<_>>()
        .join(", ")
}

fn denial_reason_label(reason: MergeGateDenialReason) -> &'static str {
    match reason {
        MergeGateDenialReason::AutoMergeDisabled => "auto_merge_disabled",
        MergeGateDenialReason::WriteModeDisallowsMerge => "write_mode_disallows_merge",
        MergeGateDenialReason::RepoNotEnrolled => "repo_not_enrolled",
        MergeGateDenialReason::ChecksNotGreen => "checks_not_green",
        MergeGateDenialReason::BranchOutOfDate => "branch_out_of_date",
        MergeGateDenialReason::BlockingReviews => "blocking_reviews",
        MergeGateDenialReason::SecurityFindings => "security_findings",
        MergeGateDenialReason::SecretsFindings => "secrets_findings",
        MergeGateDenialReason::GateEventMissing => "gate_event_missing",
    }
}

fn ci_failure_kind_label(kind: CiFailureKind) -> &'static str {
    match kind {
        CiFailureKind::Compilation => "compilation",
        CiFailureKind::Tests => "tests",
        CiFailureKind::Formatting => "formatting",
        CiFailureKind::Lint => "lint",
        CiFailureKind::DependencyResolution => "dependency_resolution",
        CiFailureKind::ToolchainSetup => "toolchain_setup",
        CiFailureKind::Permission => "permission",
        CiFailureKind::Timeout => "timeout",
        CiFailureKind::Cancelled => "cancelled",
        CiFailureKind::SecurityScan => "security_scan",
        CiFailureKind::SecretLeak => "secret_leak",
        CiFailureKind::Infrastructure => "infrastructure",
        CiFailureKind::ActionRequired => "action_required",
        CiFailureKind::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use thinclaw_repo_projects::{
        GitHubAuthMode, MergeGateDecision, MergeGateDenialReason, MergeMethod, ProjectPolicy,
        RepoProjectState, RepoProjectTaskState,
    };

    use crate::repo_projects::ci::{GitHubCiCheck, GitHubCiScope, classify_ci_check};

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("test timestamp should be valid")
    }

    fn project() -> RepoProject {
        RepoProject {
            id: Uuid::parse_str("018fda1d-6b19-7f5f-b5bb-a7d997fe0001").unwrap(),
            slug: "supervisor".to_string(),
            name: "Supervisor".to_string(),
            state: RepoProjectState::Active,
            policy: ProjectPolicy::default(),
            description: None,
            current_run_id: None,
            created_at: now(),
            updated_at: now(),
            started_at: Some(now()),
            completed_at: None,
        }
    }

    fn repo(project_id: Uuid) -> RepoProjectRepo {
        RepoProjectRepo {
            id: Uuid::parse_str("018fda1d-6b19-7f5f-b5bb-a7d997fe0002").unwrap(),
            project_id,
            owner: "RNT56".to_string(),
            repo: "ThinClaw".to_string(),
            github_repo_id: Some(123),
            installation_id: Some(456),
            default_branch: "main".to_string(),
            base_branch: Some("main".to_string()),
            enrolled: true,
            local_path: None,
            auth_mode: GitHubAuthMode::GitHubApp,
            metadata: serde_json::json!({}),
            created_at: now(),
            updated_at: now(),
        }
    }

    fn task(project_id: Uuid, repo_id: Uuid, labels: Vec<String>) -> RepoProjectTask {
        RepoProjectTask {
            id: Uuid::parse_str("018fda1d-6b19-7f5f-b5bb-a7d997fe0003").unwrap(),
            project_id,
            repo_id,
            title: "Fix CI".to_string(),
            body: Some("Token for reproduction: ghp_abcdefghijklmnopqrstuvwxyz123456".to_string()),
            state: RepoProjectTaskState::WaitingCi,
            coding_backend: CodingBackend::CodexCode,
            base_branch: "main".to_string(),
            branch_name: "thinclaw/supervisor/018fda1d6b19".to_string(),
            head_sha: Some("abc123".to_string()),
            pull_request_number: Some(42),
            pull_request_url: Some("https://github.com/RNT56/ThinClaw/pull/42".to_string()),
            github_issue_number: None,
            assigned_worker_id: None,
            priority: 5,
            labels,
            metadata: serde_json::json!({}),
            created_at: now(),
            updated_at: now(),
            queued_at: Some(now()),
            started_at: Some(now()),
            completed_at: None,
        }
    }

    #[test]
    fn task_packet_generation_is_deterministic_and_redacted() {
        let project = project();
        let repo = repo(project.id);
        let task_a = task(
            project.id,
            repo.id,
            vec!["ci".to_string(), "urgent".to_string(), "ci".to_string()],
        );
        let task_b = task(
            project.id,
            repo.id,
            vec!["urgent".to_string(), "ci".to_string()],
        );
        let ci = classify_ci_check(&GitHubCiCheck::new(
            GitHubCiScope::Job,
            "test",
            Some("failure"),
            Some("test result: FAILED. secret=sk-proj-abcdefghijklmnopqrstuvwxyz123456"),
        ));
        let gate = MergeGateDecision::denied(
            vec![MergeGateDenialReason::ChecksNotGreen],
            MergeMethod::Squash,
        );
        let context = [("zeta", "last"), ("alpha", "first")];

        let input_a = RepoTaskPacketInput {
            project: &project,
            repo: &repo,
            task: &task_a,
            worktree_path: Some("/tmp/worktree"),
            ci: Some(&ci),
            merge_gate: Some(&gate),
            extra_context: &context,
        };
        let input_b = RepoTaskPacketInput {
            task: &task_b,
            ..input_a
        };

        let packet_a = build_implementation_packet(input_a, CodingBackend::CodexCode);
        let packet_b = build_implementation_packet(input_b, CodingBackend::CodexCode);

        assert_eq!(packet_a.packet_id, packet_b.packet_id);
        assert_eq!(packet_a.prompt, packet_b.prompt);
        assert!(packet_a.prompt.contains("labels: ci, urgent"));
        assert!(packet_a.prompt.find("alpha: first") < packet_a.prompt.find("zeta: last"));
        assert!(
            !packet_a
                .prompt
                .contains("ghp_abcdefghijklmnopqrstuvwxyz123456")
        );
        assert!(
            !packet_a
                .prompt
                .contains("sk-proj-abcdefghijklmnopqrstuvwxyz123456")
        );
    }

    #[test]
    fn review_packet_uses_review_instructions() {
        let project = project();
        let repo = repo(project.id);
        let task = task(project.id, repo.id, vec![]);
        let input = RepoTaskPacketInput {
            project: &project,
            repo: &repo,
            task: &task,
            worktree_path: None,
            ci: None,
            merge_gate: None,
            extra_context: &[],
        };

        let packet = build_review_packet(input, CodingBackend::ClaudeCode);

        assert_eq!(packet.kind, TaskPacketKind::Review);
        assert!(packet.prompt.contains("backend: claude_code"));
        assert!(
            packet
                .prompt
                .contains("Do not make code changes during review.")
        );
    }
}
