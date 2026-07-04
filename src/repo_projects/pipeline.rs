//! GitHub-driven continuation of the repo project task lifecycle.
//!
//! The sandbox executor moves a task `Running -> WaitingCi` once a coding job
//! finishes. From there this pipeline owns the *GitHub* side of the loop:
//!
//! 1. **WaitingCi**: ensure a pull request exists for the task branch, poll the
//!    head SHA's check runs / workflow runs, classify the result, and either
//!    advance to review (all green), keep waiting (pending), or request a
//!    bounded repair (failing).
//! 2. **WaitingReview**: gather live review evidence, re-confirm CI, evaluate the
//!    guarded merge gate against real evidence, record the audit event, and —
//!    only when the gate approves — perform a squash merge and delete the branch.
//!
//! Every state change is persisted, recorded as a [`RepoProjectEvent`], and (when
//! wired) broadcast over SSE. The pipeline never pushes code or merges outside an
//! approved gate decision.

use std::sync::Arc;

use chrono::Utc;
use thinclaw_repo_projects::{
    CodingBackend, MergeMethod, RepoProject, RepoProjectEvent, RepoProjectEventKind,
    RepoProjectRepo, RepoProjectTask, RepoProjectTaskState, RepoWriteMode,
    validate_task_state_transition,
};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::channels::web::types::SseEvent;
use crate::db::Database;

use super::ci::{
    CiOutcome, CiSuiteClassification, GitHubCiCheck, GitHubCiScope, classify_ci_checks,
    redact_sensitive_text,
};
use super::github::{
    GitHubApiClient, GitHubApiError, GitHubCheckRunsQuery, GitHubCreatePullRequestRequest,
    GitHubListQuery, GitHubMergeMethod, GitHubMergePullRequestRequest, GitHubPullRequest,
    GitHubPullRequestListQuery, GitHubWorkflowJobsQuery, GitHubWorkflowRunsQuery,
};
use super::github_provider::RepoGitHubClientProvider;
use super::merge_gate::{MergeGateEvidence, ReviewEvidence, evaluate_merge_gate_evidence};
use super::merge_metadata;

/// How many empty CI polls to tolerate before treating a head SHA as having no
/// configured checks and escalating to a human (never silently auto-merging
/// without CI signal).
const EMPTY_CI_GRACE_POLLS: u64 = 5;
/// Cap on failing-job log downloads per poll to bound GitHub API usage.
const MAX_FAILING_JOB_LOGS: usize = 3;
const SUPERVISOR_BOT_MARKER: &str = "_Managed by the ThinClaw repo project supervisor._";

#[derive(Debug, Clone, Copy)]
pub struct PipelineConfig {
    pub max_ci_repair_attempts: u32,
    /// Cap on how many times an *approved* merge gate may attempt a merge that
    /// GitHub accepts-without-merging or rejects before the task is escalated to
    /// a human. Without this bound a structurally-unmergeable-but-gate-approved
    /// PR (protected branch, required-status mismatch, repeated 405) would be
    /// retried on every watchdog tick forever. Reset on a new head SHA.
    pub max_merge_attempts: u32,
    /// When enabled, the supervisor posts a one-shot "review readiness" summary
    /// comment (CI + merge-gate status) to the PR at the review stage. This is an
    /// automated readiness signal, not a full code review.
    pub post_review_summary: bool,
    /// When set, the supervisor dispatches a one-shot sandbox code review (using
    /// this coding backend) of the pushed branch before evaluating the merge
    /// gate. The review posts findings to the PR; it is advisory and does not by
    /// itself block the gate.
    pub reviewer_backend: Option<CodingBackend>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            max_ci_repair_attempts: 3,
            max_merge_attempts: 3,
            post_review_summary: false,
            reviewer_backend: None,
        }
    }
}

/// Result of advancing a single task through the GitHub pipeline. The supervisor
/// store only needs to *act* on [`PipelineOutcome::CiRepairRequested`] (it owns
/// the sandbox executor used to re-dispatch); every other outcome is terminal
/// for this tick and is surfaced for logging/decisions.
#[derive(Debug, Clone)]
pub enum PipelineOutcome {
    /// Task is not in a GitHub-driven state.
    Skipped,
    /// CI has not produced a conclusive result yet.
    WaitingForCi,
    /// The task branch was not found on the remote; escalated to a human.
    PullRequestMissing,
    /// CI is green; task advanced to `WaitingReview`.
    AdvancedToReview,
    /// CI failed and a bounded repair should be dispatched by the caller.
    CiRepairRequested(CiSuiteClassification),
    /// A merge-gate decision was recorded but the task was not merged.
    MergeGateRecorded { approved: bool },
    /// A one-shot sandbox review of the pushed branch was requested; the caller
    /// (which owns the sandbox executor) should dispatch it.
    ReviewRequested { backend: CodingBackend },
    /// The pull request was merged and the task is done.
    Merged { sha: String },
    /// The task needs human attention; it was moved to `Blocked`.
    AwaitingHuman { reason: String },
}

#[derive(Clone)]
pub struct GitHubPipeline {
    db: Arc<dyn Database>,
    github: Arc<dyn RepoGitHubClientProvider>,
    sse: Option<broadcast::Sender<SseEvent>>,
    config: PipelineConfig,
}

impl GitHubPipeline {
    pub fn new(
        db: Arc<dyn Database>,
        github: Arc<dyn RepoGitHubClientProvider>,
        config: PipelineConfig,
    ) -> Self {
        Self {
            db,
            github,
            sse: None,
            config,
        }
    }

    pub fn with_sse(mut self, sse: Option<broadcast::Sender<SseEvent>>) -> Self {
        self.sse = sse;
        self
    }

    /// Advance a task that is waiting on GitHub. Persists all task changes and
    /// records events; returns a [`PipelineOutcome`] for orchestration/logging.
    pub async fn advance_task(
        &self,
        project: &RepoProject,
        repo: &RepoProjectRepo,
        task: &mut RepoProjectTask,
    ) -> Result<PipelineOutcome, String> {
        match task.state {
            RepoProjectTaskState::WaitingCi => self.advance_waiting_ci(project, repo, task).await,
            RepoProjectTaskState::WaitingReview => {
                self.advance_waiting_review(project, repo, task).await
            }
            _ => Ok(PipelineOutcome::Skipped),
        }
    }

    // ── WaitingCi ───────────────────────────────────────────────────────────

    async fn advance_waiting_ci(
        &self,
        project: &RepoProject,
        repo: &RepoProjectRepo,
        task: &mut RepoProjectTask,
    ) -> Result<PipelineOutcome, String> {
        if project.policy.write_mode == RepoWriteMode::ReadOnlyClone {
            self.block_task(
                repo,
                task,
                "Project is in read-only clone mode; ThinClaw will not push branches or open pull requests.",
            )
            .await?;
            return Ok(PipelineOutcome::AwaitingHuman {
                reason: "read-only clone mode".to_string(),
            });
        }

        let client = self.github.client_for(repo).await?;

        let Some(head_sha) = self
            .ensure_pull_request(&client, project, repo, task)
            .await?
        else {
            self.block_task(
                repo,
                task,
                "Task branch was not pushed to the configured write target; a pull request cannot be opened.",
            )
            .await?;
            return Ok(PipelineOutcome::PullRequestMissing);
        };

        let checks = self.collect_ci_checks(&client, repo, &head_sha).await?;
        if checks.is_empty() {
            return self.handle_empty_ci(repo, task).await;
        }
        // A non-empty poll resets the empty-poll grace counter.
        self.reset_empty_ci_polls(task).await?;

        let suite = classify_ci_checks(&checks);
        let has_failure = suite
            .checks
            .iter()
            .any(|check| check.failure_kind.is_some());
        let has_pending = suite
            .checks
            .iter()
            .any(|check| matches!(check.outcome, CiOutcome::Pending));

        if suite.checks_green {
            self.record_ci_summary(task, &suite).await?;
            self.record_event(
                project.id,
                Some(repo.id),
                Some(task.id),
                RepoProjectEventKind::TaskStateChanged,
                &format!("CI is green for {}", short_sha(&head_sha)),
                serde_json::json!({ "head_sha": head_sha, "summary": suite.summary }),
            )
            .await?;
            self.transition_task(
                repo,
                task,
                RepoProjectTaskState::WaitingReview,
                "CI is green",
            )
            .await?;
            return Ok(PipelineOutcome::AdvancedToReview);
        }

        if has_failure {
            return self
                .handle_ci_failure(project, repo, task, &head_sha, suite)
                .await;
        }

        if has_pending {
            return Ok(PipelineOutcome::WaitingForCi);
        }

        // Not green, not failing, not pending (e.g. all neutral/unknown without a
        // failure class): keep waiting rather than failing spuriously.
        Ok(PipelineOutcome::WaitingForCi)
    }

    async fn handle_empty_ci(
        &self,
        repo: &RepoProjectRepo,
        task: &mut RepoProjectTask,
    ) -> Result<PipelineOutcome, String> {
        let polls = metadata_u64(&task.metadata, "ci_empty_polls") + 1;
        task.metadata = merge_metadata(
            &task.metadata,
            serde_json::json!({ "ci_empty_polls": polls }),
        );
        task.updated_at = Utc::now();
        self.persist_task(task).await?;

        if polls >= EMPTY_CI_GRACE_POLLS {
            self.block_task(
                repo,
                task,
                "No CI checks were reported for the head commit after the grace period; \
                 human review is required before merge.",
            )
            .await?;
            return Ok(PipelineOutcome::AwaitingHuman {
                reason: "no CI checks reported".to_string(),
            });
        }
        Ok(PipelineOutcome::WaitingForCi)
    }

    async fn reset_empty_ci_polls(&self, task: &mut RepoProjectTask) -> Result<(), String> {
        if metadata_u64(&task.metadata, "ci_empty_polls") != 0 {
            task.metadata =
                merge_metadata(&task.metadata, serde_json::json!({ "ci_empty_polls": 0 }));
            self.persist_task(task).await?;
        }
        Ok(())
    }

    async fn handle_ci_failure(
        &self,
        project: &RepoProject,
        repo: &RepoProjectRepo,
        task: &mut RepoProjectTask,
        head_sha: &str,
        suite: CiSuiteClassification,
    ) -> Result<PipelineOutcome, String> {
        let attempts = metadata_u64(&task.metadata, "ci_repair_attempts");
        let primary = suite
            .primary_failure_kind
            .map(super::ci::failure_kind_label)
            .unwrap_or("unknown");

        // Once per (head_sha, primary failure): post a triage comment and record
        // security/secret finding events for audit. Guarded so repeated watchdog
        // ticks on the same failing commit do not spam the PR or the event log.
        self.report_ci_failure_once(project, repo, task, head_sha, &suite, primary)
            .await?;

        if attempts >= u64::from(self.config.max_ci_repair_attempts) {
            self.block_task(
                repo,
                task,
                &format!(
                    "CI is still failing ({primary}) after {attempts} automated repair attempt(s); \
                     human intervention required."
                ),
            )
            .await?;
            return Ok(PipelineOutcome::AwaitingHuman {
                reason: format!("CI failing after {attempts} repair attempts"),
            });
        }

        task.metadata = merge_metadata(
            &task.metadata,
            serde_json::json!({
                "ci_repair_attempts": attempts + 1,
                "last_ci_failure_kind": primary,
                "last_ci_summary": suite.summary,
            }),
        );
        task.updated_at = Utc::now();
        self.persist_task(task).await?;
        self.record_event(
            project.id,
            Some(repo.id),
            Some(task.id),
            RepoProjectEventKind::TaskStateChanged,
            &format!(
                "CI failed ({primary}); requesting repair attempt {}",
                attempts + 1
            ),
            serde_json::json!({
                "head_sha": head_sha,
                "summary": suite.summary,
                "attempt": attempts + 1,
                "failure_kind": primary,
            }),
        )
        .await?;
        Ok(PipelineOutcome::CiRepairRequested(suite))
    }

    // ── WaitingReview ───────────────────────────────────────────────────────

    async fn advance_waiting_review(
        &self,
        project: &RepoProject,
        repo: &RepoProjectRepo,
        task: &mut RepoProjectTask,
    ) -> Result<PipelineOutcome, String> {
        let client = self.github.client_for(repo).await?;

        let Some(number) = task.pull_request_number else {
            // No PR recorded — fall back to WaitingCi to re-establish one.
            self.transition_task(
                repo,
                task,
                RepoProjectTaskState::WaitingCi,
                "Pull request reference missing; re-checking CI",
            )
            .await?;
            return Ok(PipelineOutcome::WaitingForCi);
        };

        let pr = client
            .get_pull_request(&repo.owner, &repo.repo, number)
            .await
            .map_err(stringify_github_error)?;
        let head_sha = pr.head.sha.clone();
        self.sync_pr_fields(task, &pr).await?;

        // If the PR was already merged (by us on a previous tick or by a human),
        // finish the task.
        if pr.merged == Some(true) || (pr.state == "closed" && pr.merged_at.is_some()) {
            return self.finish_merged(task, &head_sha).await;
        }

        // Re-confirm CI; a push during review can invalidate a previously green run.
        let checks = self.collect_ci_checks(&client, repo, &head_sha).await?;
        let suite = classify_ci_checks(&checks);
        let checks_green = suite.checks_green;
        if !checks.is_empty() && !checks_green {
            self.record_event(
                project.id,
                Some(repo.id),
                Some(task.id),
                RepoProjectEventKind::TaskStateChanged,
                "CI is no longer green during review; returning to CI wait",
                serde_json::json!({ "head_sha": head_sha, "summary": suite.summary }),
            )
            .await?;
            self.transition_task(
                repo,
                task,
                RepoProjectTaskState::WaitingCi,
                "CI regressed during review",
            )
            .await?;
            return Ok(PipelineOutcome::WaitingForCi);
        }

        // Optional one-shot sandbox code review of the pushed branch, before the
        // gate. Requested once per head SHA; the caller dispatches it.
        if let Some(backend) = self.config.reviewer_backend
            && metadata_string(&task.metadata, "review_requested_sha").as_deref()
                != Some(head_sha.as_str())
        {
            task.metadata = merge_metadata(
                &task.metadata,
                serde_json::json!({ "review_requested_sha": head_sha }),
            );
            task.updated_at = Utc::now();
            self.persist_task(task).await?;
            self.record_event(
                project.id,
                Some(repo.id),
                Some(task.id),
                RepoProjectEventKind::TaskStateChanged,
                "Requested ThinClaw sandbox review of the pull request",
                serde_json::json!({ "head_sha": head_sha }),
            )
            .await?;
            return Ok(PipelineOutcome::ReviewRequested { backend });
        }

        // Gather live review + branch-freshness + findings evidence. Findings are
        // derived from the *current* CI suite (which is green here), not from
        // historical events, so a long-since-fixed finding cannot block forever.
        let reviews = self.gather_review_evidence(&client, repo, number).await?;
        let branch_up_to_date = self.branch_up_to_date(&client, repo, task, &pr).await?;
        let (security_findings, secrets_findings) = count_findings(&suite);

        let evidence = MergeGateEvidence {
            checks_green: checks.is_empty() || checks_green,
            branch_up_to_date,
            reviews,
            security_findings,
            secrets_findings,
            gate_event_recorded: false,
        };

        let prior_events = self
            .db
            .list_repo_project_events(project.id, 500)
            .await
            .map_err(|error| error.to_string())?;
        let record =
            evaluate_merge_gate_evidence(&project.policy, repo, task, &evidence, &prior_events);

        // Dedupe the audit/SSE noise: only append an event when the decision for
        // this head SHA changes. The two-phase gate (record-then-approve) still
        // works because the first evaluation records `gate_event_missing`.
        let signature = decision_signature(&head_sha, &record.decision);
        let last_signature =
            metadata_string(&task.metadata, "last_merge_gate_signature").unwrap_or_default();
        if signature != last_signature {
            self.db
                .append_repo_project_event(&record.event)
                .await
                .map_err(|error| error.to_string())?;
            self.emit_sse(SseEvent::RepoMergeGateUpdated {
                project_id: project.id.to_string(),
                task_id: Some(task.id.to_string()),
                state: if record.decision.approved {
                    "approved".to_string()
                } else {
                    "denied".to_string()
                },
                message: record.event.message.clone(),
            });
            task.metadata = merge_metadata(
                &task.metadata,
                serde_json::json!({ "last_merge_gate_signature": signature }),
            );
            task.updated_at = Utc::now();
            self.persist_task(task).await?;
        }

        // Optional one-shot review-readiness summary comment (per head SHA).
        self.maybe_post_review_summary(project, repo, task, &head_sha, &record.decision)
            .await?;

        if record.decision.approved {
            return self
                .perform_merge(repo, task, number, &head_sha, record.decision.merge_method)
                .await;
        }

        // Denied. Distinguish "human will merge" (the write/auto-merge policy
        // intentionally forbids supervisor merge, otherwise clean) from a
        // genuine blocker.
        if is_human_merge_hold(&record.decision.reasons) {
            return Ok(PipelineOutcome::AwaitingHuman {
                reason: "supervisor auto-merge is disabled by write policy; pull request is green and awaiting human merge"
                    .to_string(),
            });
        }
        Ok(PipelineOutcome::MergeGateRecorded {
            approved: record.decision.approved,
        })
    }

    pub(super) async fn perform_merge(
        &self,
        repo: &RepoProjectRepo,
        task: &mut RepoProjectTask,
        number: u64,
        head_sha: &str,
        merge_method: MergeMethod,
    ) -> Result<PipelineOutcome, String> {
        // Bound the approved-merge retry loop. The counter is keyed to the head
        // SHA so a new push (a fresh merge target) resets the budget, mirroring
        // the per-SHA pattern used for the CI-repair and empty-CI counters.
        let attempts =
            if metadata_string(&task.metadata, "merge_attempts_sha").as_deref() == Some(head_sha) {
                metadata_u64(&task.metadata, "merge_attempts")
            } else {
                0
            };
        if attempts >= u64::from(self.config.max_merge_attempts) {
            self.block_task(
                repo,
                task,
                &format!(
                    "Merge gate approved but #{number} failed to merge after {attempts} attempt(s); \
                     human intervention required."
                ),
            )
            .await?;
            return Ok(PipelineOutcome::AwaitingHuman {
                reason: format!(
                    "merge gate approved but unable to merge after {attempts} attempts"
                ),
            });
        }

        let request = GitHubMergePullRequestRequest {
            commit_title: Some(redact_sensitive_text(&format!(
                "{} (#{number})",
                task.title
            ))),
            commit_message: Some(format!(
                "Auto-merged by the ThinClaw repo project supervisor for task {}.",
                task.id
            )),
            sha: Some(head_sha.to_string()),
            merge_method: Some(github_merge_method(merge_method)),
        };

        match client_merge(&self.github, repo, number, &request).await {
            Ok(response) if response.merged => {
                self.record_event(
                    task.project_id,
                    Some(repo.id),
                    Some(task.id),
                    RepoProjectEventKind::Merged,
                    &format!(
                        "Pull request #{number} merged ({})",
                        short_sha(&response.sha)
                    ),
                    serde_json::json!({
                        "pull_request_number": number,
                        "merge_sha": response.sha,
                        "merge_method": format!("{merge_method:?}").to_lowercase(),
                    }),
                )
                .await?;
                self.best_effort_delete_branch(repo, task).await;
                self.best_effort_comment(
                    repo,
                    number,
                    &format!(
                        "Merged by the ThinClaw repo project supervisor as `{}`.\n\n{SUPERVISOR_BOT_MARKER}",
                        short_sha(&response.sha)
                    ),
                )
                .await;
                // A successful merge clears the merge-attempt budget.
                task.metadata =
                    merge_metadata(&task.metadata, serde_json::json!({ "merge_attempts": 0 }));
                self.finish_merged(task, &response.sha).await
            }
            Ok(response) => {
                // GitHub accepted the call but did not merge.
                self.record_event(
                    task.project_id,
                    Some(repo.id),
                    Some(task.id),
                    RepoProjectEventKind::MergeDenied,
                    &format!("GitHub declined to merge #{number}: {}", response.message),
                    serde_json::json!({ "pull_request_number": number, "message": response.message }),
                )
                .await?;
                self.record_merge_attempt(task, head_sha, attempts).await?;
                Ok(PipelineOutcome::MergeGateRecorded { approved: true })
            }
            Err(error) => {
                let message = stringify_github_error(error);
                self.record_event(
                    task.project_id,
                    Some(repo.id),
                    Some(task.id),
                    RepoProjectEventKind::MergeDenied,
                    &format!("Merge attempt for #{number} failed: {message}"),
                    serde_json::json!({ "pull_request_number": number, "error": message }),
                )
                .await?;
                self.record_merge_attempt(task, head_sha, attempts).await?;
                Ok(PipelineOutcome::MergeGateRecorded { approved: true })
            }
        }
    }

    /// Persist an incremented merge-attempt counter keyed to the current head
    /// SHA. The bound therefore survives a supervisor restart (metadata is
    /// durable) and resets whenever a new commit becomes the merge target.
    async fn record_merge_attempt(
        &self,
        task: &mut RepoProjectTask,
        head_sha: &str,
        attempts: u64,
    ) -> Result<(), String> {
        task.metadata = merge_metadata(
            &task.metadata,
            serde_json::json!({
                "merge_attempts": attempts + 1,
                "merge_attempts_sha": head_sha,
            }),
        );
        task.updated_at = Utc::now();
        self.persist_task(task).await
    }

    async fn finish_merged(
        &self,
        task: &mut RepoProjectTask,
        merge_sha: &str,
    ) -> Result<PipelineOutcome, String> {
        let now = Utc::now();
        if validate_task_state_transition(task.state, RepoProjectTaskState::Done).is_ok() {
            task.state = RepoProjectTaskState::Done;
        }
        task.completed_at = Some(now);
        task.updated_at = now;
        task.metadata = merge_metadata(
            &task.metadata,
            serde_json::json!({ "merged_sha": merge_sha }),
        );
        self.persist_task(task).await?;
        self.emit_task_updated(task, "Pull request merged");
        Ok(PipelineOutcome::Merged {
            sha: merge_sha.to_string(),
        })
    }

    // ── PR provisioning ─────────────────────────────────────────────────────

    /// Ensure an open pull request exists for the task branch and return the head
    /// SHA. Returns `Ok(None)` when the branch has not been pushed to the
    /// configured write target.
    async fn ensure_pull_request(
        &self,
        client: &GitHubApiClient,
        project: &RepoProject,
        repo: &RepoProjectRepo,
        task: &mut RepoProjectTask,
    ) -> Result<Option<String>, String> {
        let head = pull_request_head(project, repo, task)?;

        // 1. Known PR number — refresh it.
        if let Some(number) = task.pull_request_number {
            match client
                .get_pull_request(&repo.owner, &repo.repo, number)
                .await
            {
                Ok(pr) => {
                    let sha = pr.head.sha.clone();
                    self.sync_pr_fields(task, &pr).await?;
                    return Ok(Some(sha));
                }
                Err(GitHubApiError::Api { status, .. }) if status.as_u16() == 404 => {
                    // Fall through to rediscovery.
                }
                Err(error) => return Err(stringify_github_error(error)),
            }
        }

        // 2. Discover an existing open PR for the head branch.
        let query = GitHubPullRequestListQuery {
            state: Some("open".to_string()),
            head: Some(head.pr_head.clone()),
            per_page: Some(10),
            ..GitHubPullRequestListQuery::default()
        };
        let existing = client
            .list_pull_requests(&repo.owner, &repo.repo, &query)
            .await
            .map_err(stringify_github_error)?;
        if let Some(pr) = existing.into_iter().next() {
            let sha = pr.head.sha.clone();
            self.sync_pr_fields(task, &pr).await?;
            self.record_event(
                task.project_id,
                Some(repo.id),
                Some(task.id),
                RepoProjectEventKind::TaskStateChanged,
                &format!("Adopted existing pull request #{}", pr.number),
                serde_json::json!({ "pull_request_number": pr.number }),
            )
            .await?;
            return Ok(Some(sha));
        }

        // 3. No PR yet — confirm the branch exists with commits ahead of base.
        let branch_ref = match client
            .get_branch_ref(&head.ref_owner, &head.ref_repo, &task.branch_name)
            .await
        {
            Ok(reference) => reference,
            Err(GitHubApiError::Api { status, .. }) if status.as_u16() == 404 => return Ok(None),
            Err(error) => return Err(stringify_github_error(error)),
        };

        let base_branch = task.base_branch.clone();
        let comparison = client
            .compare_commits(&repo.owner, &repo.repo, &base_branch, &head.compare_head)
            .await
            .map_err(stringify_github_error)?;
        if comparison.ahead_by <= 0 {
            // Branch exists but has no new commits — nothing to open a PR for.
            return Ok(None);
        }

        let body = build_pr_body(task);
        let request = GitHubCreatePullRequestRequest {
            title: redact_sensitive_text(&task.title),
            head: head.pr_head,
            base: base_branch,
            body: Some(body),
            draft: Some(false),
            maintainer_can_modify: Some(head.maintainer_can_modify),
        };
        let pr = client
            .create_pull_request(&repo.owner, &repo.repo, &request)
            .await
            .map_err(stringify_github_error)?;
        let sha = branch_ref.object.sha.clone();
        self.sync_pr_fields(task, &pr).await?;
        self.record_event(
            task.project_id,
            Some(repo.id),
            Some(task.id),
            RepoProjectEventKind::TaskStateChanged,
            &format!("Opened pull request #{}", pr.number),
            serde_json::json!({
                "pull_request_number": pr.number,
                "pull_request_url": pr.html_url,
            }),
        )
        .await?;
        Ok(Some(if pr.head.sha.is_empty() {
            sha
        } else {
            pr.head.sha
        }))
    }

    async fn sync_pr_fields(
        &self,
        task: &mut RepoProjectTask,
        pr: &GitHubPullRequest,
    ) -> Result<(), String> {
        let mut changed = false;
        if task.pull_request_number != Some(pr.number) {
            task.pull_request_number = Some(pr.number);
            changed = true;
        }
        if !pr.head.sha.is_empty() && task.head_sha.as_deref() != Some(pr.head.sha.as_str()) {
            task.head_sha = Some(pr.head.sha.clone());
            changed = true;
        }
        if let Some(url) = pr.html_url.as_ref()
            && task.pull_request_url.as_deref() != Some(url.as_str())
        {
            task.pull_request_url = Some(url.clone());
            changed = true;
        }
        if changed {
            task.updated_at = Utc::now();
            self.persist_task(task).await?;
            self.emit_task_updated(task, "Pull request updated");
        }
        Ok(())
    }

    // ── CI collection ───────────────────────────────────────────────────────

    async fn collect_ci_checks(
        &self,
        client: &GitHubApiClient,
        repo: &RepoProjectRepo,
        head_sha: &str,
    ) -> Result<Vec<GitHubCiCheck>, String> {
        let mut checks = Vec::new();

        let check_runs = client
            .list_check_runs_for_ref(
                &repo.owner,
                &repo.repo,
                head_sha,
                &GitHubCheckRunsQuery {
                    per_page: Some(100),
                    ..GitHubCheckRunsQuery::default()
                },
            )
            .await
            .map_err(stringify_github_error)?;
        for run in check_runs.check_runs {
            let log = run.output.as_ref().and_then(|output| {
                let mut parts = Vec::new();
                if let Some(summary) = output.summary.as_deref() {
                    parts.push(summary.to_string());
                }
                if let Some(text) = output.text.as_deref() {
                    parts.push(text.to_string());
                }
                if parts.is_empty() {
                    None
                } else {
                    Some(parts.join("\n"))
                }
            });
            checks.push(GitHubCiCheck {
                scope: GitHubCiScope::Check,
                name: run.name,
                status: Some(run.status),
                conclusion: run.conclusion,
                log: log.map(|raw| redact_sensitive_text(&raw)),
            });
        }

        let workflow_runs = client
            .list_workflow_runs(
                &repo.owner,
                &repo.repo,
                &GitHubWorkflowRunsQuery {
                    head_sha: Some(head_sha.to_string()),
                    per_page: Some(50),
                    ..GitHubWorkflowRunsQuery::default()
                },
            )
            .await
            .map_err(stringify_github_error)?;

        let mut log_downloads = 0usize;
        for run in workflow_runs.workflow_runs {
            let failing = run
                .conclusion
                .as_deref()
                .map(|conclusion| !matches!(conclusion, "success" | "neutral" | "skipped"))
                .unwrap_or(false);
            checks.push(GitHubCiCheck {
                scope: GitHubCiScope::Workflow,
                name: run.name.clone().unwrap_or_else(|| "workflow".to_string()),
                status: Some(run.status.clone()),
                conclusion: run.conclusion.clone(),
                log: None,
            });

            if failing
                && log_downloads < MAX_FAILING_JOB_LOGS
                && let Ok(jobs) = client
                    .list_workflow_run_jobs(
                        &repo.owner,
                        &repo.repo,
                        run.id,
                        &GitHubWorkflowJobsQuery {
                            filter: Some("latest".to_string()),
                            per_page: Some(100),
                            ..GitHubWorkflowJobsQuery::default()
                        },
                    )
                    .await
            {
                for job in jobs.jobs {
                    let job_failing = job
                        .conclusion
                        .as_deref()
                        .map(|conclusion| !matches!(conclusion, "success" | "neutral" | "skipped"))
                        .unwrap_or(false);
                    let mut log = None;
                    if job_failing && log_downloads < MAX_FAILING_JOB_LOGS {
                        log = self.download_job_log(client, repo, job.id).await;
                        log_downloads += 1;
                    }
                    checks.push(GitHubCiCheck {
                        scope: GitHubCiScope::Job,
                        name: job.name,
                        status: Some(job.status),
                        conclusion: job.conclusion,
                        log,
                    });
                }
            }
        }

        Ok(checks)
    }

    async fn download_job_log(
        &self,
        client: &GitHubApiClient,
        repo: &RepoProjectRepo,
        job_id: u64,
    ) -> Option<String> {
        match client
            .download_workflow_job_logs(&repo.owner, &repo.repo, job_id)
            .await
        {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes.body);
                Some(redact_sensitive_text(&tail(&text, 8_000)))
            }
            Err(error) => {
                tracing::debug!(job_id, error = %stringify_github_error(error), "failed to download CI job log");
                None
            }
        }
    }

    // ── Review / branch / findings evidence ─────────────────────────────────

    async fn gather_review_evidence(
        &self,
        client: &GitHubApiClient,
        repo: &RepoProjectRepo,
        number: u64,
    ) -> Result<ReviewEvidence, String> {
        let reviews = client
            .list_pull_request_reviews(
                &repo.owner,
                &repo.repo,
                number,
                &GitHubListQuery {
                    per_page: Some(100),
                    ..GitHubListQuery::default()
                },
            )
            .await
            .map_err(stringify_github_error)?;

        // Keep only the latest review per author so a stale CHANGES_REQUESTED that
        // was followed by an APPROVED does not block forever.
        use std::collections::HashMap;
        let mut latest: HashMap<String, String> = HashMap::new();
        for review in reviews {
            let login = review
                .user
                .as_ref()
                .map(|user| user.login.clone())
                .unwrap_or_default();
            let state = review.state.to_ascii_uppercase();
            // Skip COMMENTED/PENDING/DISMISSED which are not decisions.
            if matches!(state.as_str(), "APPROVED" | "CHANGES_REQUESTED") {
                latest.insert(login, state);
            }
        }
        let approvals = latest.values().filter(|state| *state == "APPROVED").count() as u32;
        let changes_requested = latest
            .values()
            .filter(|state| *state == "CHANGES_REQUESTED")
            .count() as u32;
        let required_approvals = metadata_u64(&repo.metadata, "required_approvals") as u32;

        Ok(ReviewEvidence {
            approvals,
            required_approvals,
            changes_requested,
            unresolved_threads: 0,
        })
    }

    async fn branch_up_to_date(
        &self,
        client: &GitHubApiClient,
        repo: &RepoProjectRepo,
        task: &RepoProjectTask,
        pr: &GitHubPullRequest,
    ) -> Result<bool, String> {
        // Conflicting PRs are never "up to date" in a mergeable sense.
        if pr.mergeable == Some(false) {
            return Ok(false);
        }
        let head = task
            .head_sha
            .clone()
            .unwrap_or_else(|| task.branch_name.clone());
        let comparison = client
            .compare_commits(&repo.owner, &repo.repo, &task.base_branch, &head)
            .await
            .map_err(stringify_github_error)?;
        Ok(comparison.is_up_to_date())
    }

    // ── CI failure reporting (comment + finding audit, deduped) ─────────────

    async fn report_ci_failure_once(
        &self,
        project: &RepoProject,
        repo: &RepoProjectRepo,
        task: &mut RepoProjectTask,
        head_sha: &str,
        suite: &CiSuiteClassification,
        primary: &str,
    ) -> Result<(), String> {
        let signature = format!("{head_sha}:{primary}");
        if metadata_string(&task.metadata, "last_ci_comment_sig").as_deref() == Some(&signature) {
            return Ok(());
        }

        // Record security / secret finding events for audit (not used as a gate
        // input — the gate reads the current CI poll).
        use super::ci::CiFailureKind;
        for check in &suite.checks {
            match check.failure_kind {
                Some(CiFailureKind::SecurityScan) => {
                    self.record_event(
                        project.id,
                        Some(repo.id),
                        Some(task.id),
                        RepoProjectEventKind::SecurityFindingRecorded,
                        &format!("Security finding reported by CI check '{}'", check.name),
                        serde_json::json!({ "head_sha": head_sha, "check": check.name }),
                    )
                    .await?;
                }
                Some(CiFailureKind::SecretLeak) => {
                    self.record_event(
                        project.id,
                        Some(repo.id),
                        Some(task.id),
                        RepoProjectEventKind::SecretsFindingRecorded,
                        &format!(
                            "Secret-scanning finding reported by CI check '{}'",
                            check.name
                        ),
                        serde_json::json!({ "head_sha": head_sha, "check": check.name }),
                    )
                    .await?;
                }
                _ => {}
            }
        }

        if let Some(number) = task.pull_request_number {
            let mut body = format!(
                "**ThinClaw CI triage** — {}\n\nPrimary failure class: `{primary}`.",
                redact_sensitive_text(&suite.summary)
            );
            if let Some(check) = suite
                .checks
                .iter()
                .find(|check| check.failure_kind.is_some())
                && let Some(recommendation) = &check.recommendation
            {
                body.push_str(&format!(
                    "\n\nRecommended action: {}\n{}",
                    redact_sensitive_text(&recommendation.summary),
                    redact_sensitive_text(&recommendation.prompt_hint),
                ));
            }
            body.push_str(&format!("\n\n{SUPERVISOR_BOT_MARKER}"));
            self.best_effort_comment(repo, number, &body).await;
        }

        task.metadata = merge_metadata(
            &task.metadata,
            serde_json::json!({ "last_ci_comment_sig": signature }),
        );
        task.updated_at = Utc::now();
        self.persist_task(task).await
    }

    async fn maybe_post_review_summary(
        &self,
        project: &RepoProject,
        repo: &RepoProjectRepo,
        task: &mut RepoProjectTask,
        head_sha: &str,
        decision: &thinclaw_repo_projects::MergeGateDecision,
    ) -> Result<(), String> {
        if !self.config.post_review_summary {
            return Ok(());
        }
        if metadata_string(&task.metadata, "review_summary_sha").as_deref() == Some(head_sha) {
            return Ok(());
        }
        let Some(number) = task.pull_request_number else {
            return Ok(());
        };
        let status = if decision.approved {
            "ready to merge".to_string()
        } else {
            format!(
                "not yet mergeable ({})",
                decision
                    .reasons
                    .iter()
                    .copied()
                    .map(super::merge_gate::denial_reason_label)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let body = format!(
            "**ThinClaw review readiness** for `{}` — {status}.\n\nCI is green and the merge \
             gate has been evaluated. This is an automated readiness summary, not a full code \
             review.\n\n{SUPERVISOR_BOT_MARKER}",
            short_sha(head_sha)
        );
        self.best_effort_comment(repo, number, &body).await;
        task.metadata = merge_metadata(
            &task.metadata,
            serde_json::json!({ "review_summary_sha": head_sha }),
        );
        task.updated_at = Utc::now();
        self.persist_task(task).await?;
        self.record_event(
            project.id,
            Some(repo.id),
            Some(task.id),
            RepoProjectEventKind::TaskStateChanged,
            "Posted ThinClaw review readiness summary",
            serde_json::json!({ "head_sha": head_sha, "approved": decision.approved }),
        )
        .await
    }

    async fn best_effort_comment(&self, repo: &RepoProjectRepo, number: u64, body: &str) {
        let client = match self.github.client_for(repo).await {
            Ok(client) => client,
            Err(error) => {
                tracing::debug!(error = %error, "skipping PR comment: no client");
                return;
            }
        };
        if let Err(error) = client
            .create_pull_request_comment(&repo.owner, &repo.repo, number, body.to_string())
            .await
        {
            tracing::debug!(number, error = %stringify_github_error(error), "failed to post PR comment");
        }
    }

    async fn best_effort_delete_branch(&self, repo: &RepoProjectRepo, task: &RepoProjectTask) {
        let client = match self.github.client_for(repo).await {
            Ok(client) => client,
            Err(_) => return,
        };
        if let Err(error) = client
            .delete_branch_ref(&repo.owner, &repo.repo, &task.branch_name)
            .await
        {
            tracing::debug!(branch = %task.branch_name, error = %stringify_github_error(error), "failed to delete task branch after merge");
        }
    }

    // ── Persistence + event/SSE helpers ─────────────────────────────────────

    async fn transition_task(
        &self,
        repo: &RepoProjectRepo,
        task: &mut RepoProjectTask,
        next: RepoProjectTaskState,
        message: &str,
    ) -> Result<(), String> {
        let previous = task.state;
        if previous == next {
            return Ok(());
        }
        if validate_task_state_transition(previous, next).is_err() {
            return Err(format!(
                "invalid task transition {:?} -> {:?}",
                previous, next
            ));
        }
        let now = Utc::now();
        task.state = next;
        task.updated_at = now;
        self.persist_task(task).await?;
        self.record_event(
            task.project_id,
            Some(repo.id),
            Some(task.id),
            RepoProjectEventKind::TaskStateChanged,
            message,
            serde_json::json!({
                "from": task_state_label(previous),
                "to": task_state_label(next),
            }),
        )
        .await?;
        self.emit_task_updated(task, message);
        Ok(())
    }

    async fn block_task(
        &self,
        repo: &RepoProjectRepo,
        task: &mut RepoProjectTask,
        reason: &str,
    ) -> Result<(), String> {
        let previous = task.state;
        if validate_task_state_transition(previous, RepoProjectTaskState::Blocked).is_err() {
            return Ok(());
        }
        let now = Utc::now();
        task.state = RepoProjectTaskState::Blocked;
        task.updated_at = now;
        task.metadata = merge_metadata(
            &task.metadata,
            serde_json::json!({ "blocked_reason": reason }),
        );
        self.persist_task(task).await?;
        self.record_event(
            task.project_id,
            Some(repo.id),
            Some(task.id),
            RepoProjectEventKind::TaskStateChanged,
            reason,
            serde_json::json!({
                "from": task_state_label(previous),
                "to": "blocked",
                "reason": reason,
            }),
        )
        .await?;
        self.emit_task_updated(task, reason);
        Ok(())
    }

    async fn record_ci_summary(
        &self,
        task: &mut RepoProjectTask,
        suite: &CiSuiteClassification,
    ) -> Result<(), String> {
        task.metadata = merge_metadata(
            &task.metadata,
            serde_json::json!({ "last_ci_summary": suite.summary }),
        );
        task.updated_at = Utc::now();
        self.persist_task(task).await
    }

    async fn persist_task(&self, task: &RepoProjectTask) -> Result<(), String> {
        self.db
            .upsert_repo_project_task(task)
            .await
            .map_err(|error| error.to_string())
    }

    async fn record_event(
        &self,
        project_id: Uuid,
        repo_id: Option<Uuid>,
        task_id: Option<Uuid>,
        kind: RepoProjectEventKind,
        message: &str,
        details: serde_json::Value,
    ) -> Result<(), String> {
        let event = RepoProjectEvent {
            id: Uuid::new_v4(),
            project_id,
            repo_id,
            task_id,
            project_run_id: None,
            worker_run_id: None,
            kind,
            message: message.to_string(),
            details,
            created_at: Utc::now(),
        };
        self.db
            .append_repo_project_event(&event)
            .await
            .map_err(|error| error.to_string())?;
        self.emit_sse(SseEvent::RepoProjectEvent {
            project_id: project_id.to_string(),
            event_type: event_kind_label(kind).to_string(),
            message: message.to_string(),
        });
        Ok(())
    }

    fn emit_task_updated(&self, task: &RepoProjectTask, message: &str) {
        self.emit_sse(SseEvent::RepoTaskUpdated {
            project_id: task.project_id.to_string(),
            task_id: task.id.to_string(),
            state: task_state_label(task.state).to_string(),
            message: message.to_string(),
        });
    }

    fn emit_sse(&self, event: SseEvent) {
        if let Some(sender) = self.sse.as_ref() {
            let _ = sender.send(event);
        }
    }
}

async fn client_merge(
    github: &Arc<dyn RepoGitHubClientProvider>,
    repo: &RepoProjectRepo,
    number: u64,
    request: &GitHubMergePullRequestRequest,
) -> Result<super::github::GitHubMergePullRequestResponse, GitHubApiError> {
    // Build a fresh client for the merge so an installation token close to expiry
    // is refreshed; provider errors surface as a generic API error.
    let client = github
        .client_for(repo)
        .await
        .map_err(GitHubApiError::InvalidHeader)?;
    client
        .merge_pull_request(&repo.owner, &repo.repo, number, request)
        .await
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PullRequestHead {
    pr_head: String,
    ref_owner: String,
    ref_repo: String,
    compare_head: String,
    maintainer_can_modify: bool,
}

fn pull_request_head(
    project: &RepoProject,
    repo: &RepoProjectRepo,
    task: &RepoProjectTask,
) -> Result<PullRequestHead, String> {
    match project.policy.write_mode {
        RepoWriteMode::ReadOnlyClone => {
            Err("read-only clone mode does not create pull requests".to_string())
        }
        RepoWriteMode::ForkPr => {
            let owner = metadata_string(&repo.metadata, "fork_owner").ok_or_else(|| {
                "fork_pr mode requires repo metadata fork_owner so ThinClaw knows where task branches are pushed".to_string()
            })?;
            let fork_repo =
                metadata_string(&repo.metadata, "fork_repo").unwrap_or_else(|| repo.repo.clone());
            Ok(PullRequestHead {
                pr_head: format!("{owner}:{}", task.branch_name),
                ref_owner: owner.clone(),
                ref_repo: fork_repo,
                compare_head: format!("{owner}:{}", task.branch_name),
                maintainer_can_modify: true,
            })
        }
        RepoWriteMode::MaintainerBranchPr | RepoWriteMode::MaintainerAutoMerge => {
            Ok(PullRequestHead {
                pr_head: format!("{}:{}", repo.owner, task.branch_name),
                ref_owner: repo.owner.clone(),
                ref_repo: repo.repo.clone(),
                compare_head: task.branch_name.clone(),
                maintainer_can_modify: true,
            })
        }
    }
}

fn build_pr_body(task: &RepoProjectTask) -> String {
    let mut body = String::new();
    if let Some(task_body) = task.body.as_deref() {
        body.push_str(&redact_sensitive_text(task_body));
        body.push_str("\n\n");
    }
    body.push_str(&format!(
        "---\n_Opened by the ThinClaw repo project supervisor for task `{}`._\n{SUPERVISOR_BOT_MARKER}",
        task.id
    ));
    body
}

/// Count current security / secret findings from a CI suite classification.
/// Used as live merge-gate input (it is `(0, 0)` when CI is green).
fn count_findings(suite: &CiSuiteClassification) -> (u32, u32) {
    use super::ci::CiFailureKind;
    let mut security = 0u32;
    let mut secrets = 0u32;
    for check in &suite.checks {
        match check.failure_kind {
            Some(CiFailureKind::SecurityScan) => security += 1,
            Some(CiFailureKind::SecretLeak) => secrets += 1,
            _ => {}
        }
    }
    (security, secrets)
}

fn github_merge_method(method: MergeMethod) -> GitHubMergeMethod {
    match method {
        MergeMethod::Merge => GitHubMergeMethod::Merge,
        MergeMethod::Squash => GitHubMergeMethod::Squash,
        MergeMethod::Rebase => GitHubMergeMethod::Rebase,
    }
}

fn is_human_merge_hold(reasons: &[thinclaw_repo_projects::MergeGateDenialReason]) -> bool {
    use thinclaw_repo_projects::MergeGateDenialReason::{
        AutoMergeDisabled, WriteModeDisallowsMerge,
    };
    !reasons.is_empty()
        && reasons
            .iter()
            .all(|reason| matches!(reason, AutoMergeDisabled | WriteModeDisallowsMerge))
}

fn decision_signature(
    head_sha: &str,
    decision: &thinclaw_repo_projects::MergeGateDecision,
) -> String {
    let mut reasons: Vec<&str> = decision
        .reasons
        .iter()
        .copied()
        .map(super::merge_gate::denial_reason_label)
        .collect();
    reasons.sort_unstable();
    format!(
        "{head_sha}:{}:{}",
        if decision.approved {
            "approved"
        } else {
            "denied"
        },
        reasons.join(",")
    )
}

fn stringify_github_error(error: GitHubApiError) -> String {
    error.to_string()
}

fn metadata_u64(metadata: &serde_json::Value, key: &str) -> u64 {
    metadata
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

fn metadata_string(metadata: &serde_json::Value, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn short_sha(sha: &str) -> &str {
    if sha.len() >= 7 { &sha[..7] } else { sha }
}

fn tail(value: &str, max_chars: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= max_chars {
        return value.to_string();
    }
    chars[chars.len() - max_chars..].iter().collect()
}

fn task_state_label(state: RepoProjectTaskState) -> &'static str {
    match state {
        RepoProjectTaskState::Queued => "queued",
        RepoProjectTaskState::Planning => "planning",
        RepoProjectTaskState::Ready => "ready",
        RepoProjectTaskState::Running => "running",
        RepoProjectTaskState::WaitingCi => "waiting_ci",
        RepoProjectTaskState::WaitingReview => "waiting_review",
        RepoProjectTaskState::Blocked => "blocked",
        RepoProjectTaskState::Done => "done",
        RepoProjectTaskState::Failed => "failed",
        RepoProjectTaskState::Cancelled => "cancelled",
    }
}

fn event_kind_label(kind: RepoProjectEventKind) -> &'static str {
    match kind {
        RepoProjectEventKind::ProjectCreated => "project_created",
        RepoProjectEventKind::ProjectStateChanged => "project_state_changed",
        RepoProjectEventKind::RepoEnrolled => "repo_enrolled",
        RepoProjectEventKind::RepoUnenrolled => "repo_unenrolled",
        RepoProjectEventKind::TaskCreated => "task_created",
        RepoProjectEventKind::TaskStateChanged => "task_state_changed",
        RepoProjectEventKind::ProjectRunStarted => "project_run_started",
        RepoProjectEventKind::ProjectRunCompleted => "project_run_completed",
        RepoProjectEventKind::WorkerRunQueued => "worker_run_queued",
        RepoProjectEventKind::WorkerRunStarted => "worker_run_started",
        RepoProjectEventKind::WorkerRunCompleted => "worker_run_completed",
        RepoProjectEventKind::MergeGateEvaluated => "merge_gate_evaluated",
        RepoProjectEventKind::MergeQueued => "merge_queued",
        RepoProjectEventKind::Merged => "merged",
        RepoProjectEventKind::MergeDenied => "merge_denied",
        RepoProjectEventKind::SecurityFindingRecorded => "security_finding_recorded",
        RepoProjectEventKind::SecretsFindingRecorded => "secrets_finding_recorded",
    }
}
