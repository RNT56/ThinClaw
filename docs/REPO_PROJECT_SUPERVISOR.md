# Repository Project Supervisor Runbook

This runbook covers the Continuous GitHub Project Supervisor subsystem from an operator perspective. It is intentionally conservative: it separates what is implemented in this branch from what the GitHub integration still needs before unattended repository work is safe.

## Current Status

Implemented:

- Durable repo-project, repo, task, worker-run, event, and merge-gate tables for LibSQL and Postgres.
- Settings and env resolution for the feature flag, concurrency limits, coding backend default, write-mode default, auto-merge default, workspace root, and GitHub App fields.
- Local/web/agent-tool APIs for creating projects, changing project state, enqueueing backlog tasks, listing events, and reading merge-gate status.
- A background supervisor loop that wakes from a bounded channel and a watchdog interval, reconciles durable state, dispatches queued tasks to sandbox jobs when a job manager is available, and records worker-run/task events.
- Deterministic task packets for sandbox Codex/Claude/worker jobs, isolated git worktree provisioning, restart-visible worker-run records, sandbox job persistence, and write-mode-specific credential grants (`read_only_clone`, `fork_pr`, `maintainer_branch_pr`, `maintainer_auto_merge`).
- GitHub App helper code for webhook HMAC verification, delivery dedupe, webhook envelope parsing, App JWT signing, installation-token retrieval, and typed GitHub REST calls for repository metadata, refs, pull requests, checks, workflow runs/jobs/logs, reviews, comments, labels, issues, branch compare, ref deletion, and merge requests.
- Public GitHub webhook route for repo projects that verifies signatures, dedupes in memory, normalizes envelopes, broadcasts project events, and wakes the live supervisor (the supervisor handle is now plumbed into the gateway through a shared cell).
- An authenticated GitHub client provider that mints a GitHub App installation-token client (private key resolved from the secrets store) or falls back to a `github_token` client, selected per enrolled repo.
- A live GitHub pipeline that advances `WaitingCi`/`WaitingReview` tasks: ensure/open a PR from the configured write target, poll and classify check runs / workflow jobs (with redacted log triage and PR comments), bounded CI-repair re-dispatch into the sandbox, gather live review + branch-freshness evidence, evaluate the guarded merge gate against real evidence, and perform a squash merge with branch deletion only when the gate approves and the project is in `maintainer_auto_merge`.
- Restart recovery that reconciles sandbox jobs completed while the supervisor was down and blocks any task left `Running` with no worker record.
- Live SSE for `repo_project_updated`, `repo_task_updated`, `repo_worker_run_updated`, `repo_project_event`, and `repo_merge_gate_updated`.
- Durable, restart-surviving webhook delivery storage (`repo_webhook_deliveries`) used for idempotency and audit; the in-memory deduper remains a fast pre-check.
- Durable project-run records (`repo_project_runs`): the supervisor opens a run on first dispatch and closes it with final task tallies on project completion (`ProjectRunStarted`/`ProjectRunCompleted` events).
- Optional one-shot "review readiness" PR comment at the review stage (env `REPO_PROJECTS_REVIEW_SUMMARY=true`) summarizing CI + merge-gate status.
- Optional one-shot **sandbox code review** of the pushed branch (env `REPO_PROJECTS_REVIEWER_BACKEND=claude_code|codex_code|worker`): the executor checks out the *pushed* branch content into a detached review worktree, dispatches a read-only review job that posts findings to the PR, and the worker run is skipped by task-state reconciliation. Advisory — it does not by itself block the gate.
- Local Git workspace provisioning helpers that clone/fetch a repo and create per-task worktrees (impl + review) under a supervisor workspace directory.
- Supervisor startup honors `repo_projects.enabled`, uses the resolved watchdog interval, and passes the resolved workspace root into the executor.
- A cross-surface **GitHub connector**: discover the repositories a credential can act on (`GET /installation/repositories` for a GitHub App, `GET /user/repos` for a `github_token`), mark which are already supervised, and bring selected repos — or all of them — under supervision in one step. Exposed everywhere it matters: agent tools, CLI, gateway HTTP, a desktop connector card, and a WebUI connector panel (see *Surfaces and the GitHub Connector* below).

Validation:

- Two-backend DB parity is checked in CI/Docker: `schema_divergence` confirms the libSQL schema block and the Postgres `V*.sql` migrations (incl. `V26` runs + webhook deliveries) match, and a `db_contract` repo-projects test round-trips the full store (projects/repos/tasks/runs/worker-runs/events/merge-gates/webhook deliveries) against real Postgres.

Integration-pending:

- Webhook **replay** tooling. Deliveries are now stored durably for idempotency/audit, but there is no operator command to replay a stored delivery.
- Mapping individual GitHub webhook payloads directly into task state transitions; today a webhook wakes the supervisor, which then re-derives state from the GitHub API on the next reconcile.
- A running fake-Docker coding-bridge end-to-end in CI. The GitHub side has a pipeline E2E and a supervisor-reconcile E2E; the sandbox dispatch path has a real-Docker E2E (`tests/repo_project_docker_e2e.rs`, dispatches an actual container via `RepoProjectExecutor`) that is `#[ignore]`d by default because it needs a local `thinclaw-worker:latest` image (`docker build -f Dockerfile.worker -t thinclaw-worker .`).

## Surfaces and the GitHub Connector

The supervisor is reachable from every operator surface. The framework-free
`crate::api::repo_projects` layer is the single source of truth; each surface is
a thin adapter over it.

**Connector flow (connect → select → engage):**

1. **Connect** — store a credential and/or configure a GitHub App. Credential
   *values* go straight to the encrypted secrets store and are referenced by
   *name* only; they are never written to settings, events, logs, or sent
   through the LLM. A configured `app_slug` yields an install URL
   (`https://github.com/apps/<slug>/installations/new`) so the user can install
   the App and grant repo access.
2. **Select** — `list_connectable_repos` lists the repositories the credential
   can act on, each flagged with whether it is already supervised, its
   permissions, and the recommended write mode. The user picks specific repos
   or selects all.
3. **Engage** — `connect_repos` creates a draft project per selected repo
   (skipping ones already enrolled); the project is then started/planned with
   the normal controls. Without an explicit override, discovered repos use the
   safe recommendation: public repos discovered with a user token default to
   fork PR, GitHub App-only public repos stay read-only until fork metadata is
   supplied or the operator overrides, archived/disabled repos are read-only,
   and private repos only use maintainer branch PR when the credential has push
   permission.

**Per-surface entry points:**

| Surface | How to use the connector |
|---|---|
| Agent chat (all surfaces) | Tools `repo_project_setup`, `repo_project_set_credential`, `repo_project_list_repos`, `repo_project_connect`, `repo_project_enroll` (+ `create/plan/status/pause/resume/approve`). The credential tool/value is stored encrypted and never echoed. |
| CLI | `thinclaw repo-projects setup`, `set-credential`, `repos`, `connect [repos…] [--all]`, `enroll`, plus `list/show/status/create/start/pause/resume/cancel/events`. |
| Gateway HTTP | `GET /api/repo-projects/readiness`, `POST /api/repo-projects/setup`, `POST /api/repo-projects/credentials`, `GET /api/repo-projects/connectable-repos`, `POST /api/repo-projects/connect`, `POST /api/repo-projects/{id}/enroll` (+ the existing project CRUD/command routes). |
| WebUI (gateway dashboard) | *Operations → Repo Projects* tab: a GitHub Connector card (enable, readiness checklist, masked credential entry, App config + install link, repo picker with select-all/specific) plus a projects list/detail with start/pause/resume/cancel. |
| Desktop app | The Repo Projects view embeds a `ThinClawRepoConnector` card (same connector flow) and can spawn coding/worker containers locally when the Docker sandbox is enabled. |

**Credential security:** every surface routes credential *values* through
`store_repo_credential` → the encrypted secrets store. The masked inputs in the
desktop card and WebUI panel submit directly to the secrets store
(`thinclaw_repo_projects_set_credential` / `POST /api/repo-projects/credentials`)
so the value bypasses the model entirely; settings only ever hold the secret's
*name*. Fork PR workers receive `github_fork_token`; upstream maintainer-mode
workers receive `github_token`; read-only workers receive no GitHub write
credential. GitHub App installation tokens can handle supervisor-side API calls,
but they are not injected into sandbox jobs as git push credentials.

## Enablement

Repo-project API writes are off by default. Enable them in settings or with env vars before creating projects:

```toml
[repo_projects]
enabled = true
max_concurrent_projects = 1
max_concurrent_tasks_per_project = 1
default_coding_backend = "worker"
default_write_mode = "fork_pr"
auto_merge_default = false
watchdog_interval_secs = 60
workspace_base_dir = "/var/lib/thinclaw/repo-projects"

[repo_projects.github_app]
app_id = 123456
installation_id = 987654
private_key_secret = "repo_projects_github_private_key"
webhook_secret_secret = "repo_projects_github_webhook_secret"
# Public App slug; used to build the connector install URL.
app_slug = "thinclaw-supervisor"
```

The connector surfaces (CLI `setup --app-slug`, the desktop card, and the WebUI
panel) write these same settings, so the App can also be configured entirely
from the UI without hand-editing config.

Env overrides:

| Env var | Purpose | Default |
|---|---|---|
| `REPO_PROJECTS_ENABLED` | Master feature flag. API writes fail unless enabled. | `false` |
| `REPO_PROJECTS_MAX_CONCURRENT_PROJECTS` | Intended project-level concurrency ceiling. | `1` |
| `REPO_PROJECTS_MAX_CONCURRENT_TASKS_PER_PROJECT` | Intended per-project task concurrency and new project policy default. | `1` |
| `REPO_PROJECTS_DEFAULT_CODING_BACKEND` | `worker`, `claude_code`, or `codex_code`. | `worker` |
| `REPO_PROJECTS_DEFAULT_WRITE_MODE` | `read_only_clone`, `fork_pr`, `maintainer_branch_pr`, or `maintainer_auto_merge`. | `fork_pr` |
| `REPO_PROJECTS_AUTO_MERGE_DEFAULT` | New project policy default for guarded auto-merge. | `false` |
| `REPO_PROJECTS_WATCHDOG_INTERVAL_SECS` | Intended supervisor watchdog cadence. | `60` |
| `REPO_PROJECTS_WORKSPACE_BASE_DIR` | Base directory for repo clones/worktrees. | platform ThinClaw data dir under `repo_projects` |
| `REPO_PROJECTS_GITHUB_APP_ID` | GitHub App id. | unset |
| `REPO_PROJECTS_GITHUB_INSTALLATION_ID` | GitHub App installation id. | unset |
| `REPO_PROJECTS_GITHUB_PRIVATE_KEY_SECRET` | Secret-store key name for the PEM private key. | unset |
| `REPO_PROJECTS_GITHUB_WEBHOOK_SECRET_SECRET` | Secret-store key name for the webhook secret. | unset |
| `REPO_PROJECTS_REVIEW_SUMMARY` | Post a one-shot review-readiness summary comment on PRs at the review stage. | `false` |
| `REPO_PROJECTS_REVIEWER_BACKEND` | Run a one-shot sandbox code review (`claude_code`/`codex_code`/`worker`) of the pushed branch before the merge gate. Unset = off. | unset |

Operational note: the webhook route resolves the configured webhook secret, and the supervisor now constructs a live GitHub App installation-token client by resolving `private_key_secret` from the secrets store at startup. If the App id or private key is missing/unreadable, the supervisor logs a warning and falls back to a `github_token` secret for API calls, so a misconfigured App degrades gracefully rather than disabling the pipeline. In the default `fork_pr` mode, setup is not live-ready until both upstream API credentials (GitHub App or `github_token`) and `github_fork_token` are present. In maintainer modes, setup also requires `github_token` because sandbox workers need an upstream git push credential.

## Repository Write Modes

Write mode is a project policy. It controls both worker instructions and which
GitHub secret is injected into sandbox jobs.

| Mode | Worker GitHub credential | Push target | PR target | Supervisor merge |
|---|---|---|---|---|
| `read_only_clone` | none | none | none | no |
| `fork_pr` | `github_fork_token` | fork remote `thinclaw-write` | upstream base branch | no |
| `maintainer_branch_pr` | `github_token` | upstream `origin` task branch | upstream base branch | no |
| `maintainer_auto_merge` | `github_token` | upstream `origin` task branch | upstream base branch | yes, after gates pass |

Use `fork_pr` for public repositories by default. It lets ThinClaw clone the
upstream repo, push task branches to a fork, and open PRs without handing the
sandbox a credential that can push to the upstream repository. `connect_repos`
stores fork metadata automatically when discovery used a user token and the
authenticated user can be identified. Otherwise set `fork_owner`/`fork_repo`
when creating or enrolling the repo, and ensure the fork exists.

## GitHub App Setup

Use a dedicated GitHub App, installed only on repositories you intend ThinClaw to supervise.

Recommended repository permissions for the target design:

| Permission | Access | Why |
|---|---|---|
| Contents | Read-only for fork/read-only modes; read/write only for maintainer modes | Clone/fetch, push upstream task branches only when explicitly opted in, and merge via GitHub API only in `maintainer_auto_merge`. |
| Pull requests | Read and write | Create/update PRs, read reviews, and merge PRs only in `maintainer_auto_merge`. |
| Checks | Read-only | Read CI/check-run status for merge gates. |
| Commit statuses | Read-only | Read legacy status checks for merge gates. |
| Issues | Read and write, optional | Link tasks to GitHub issues if issue-backed backlog is enabled later. |
| Metadata | Read-only | Required by GitHub Apps. |

Recommended webhook events for the target design:

- `pull_request`
- `pull_request_review`
- `check_run`
- `check_suite`
- `status`
- `workflow_run`
- `push`
- `installation`
- `installation_repositories`

Current code verifies `X-Hub-Signature-256`, parses `X-GitHub-Delivery`, dedupes recent deliveries in memory, extracts `installation.id`, `repository.full_name`, and `action`, broadcasts a project event, and wakes the live supervisor, which then re-derives PR/CI/review state from the GitHub API on its next reconcile. The durable delivery replay path still needs to be wired.

## Workspace Layout

The runtime config default workspace root is the platform ThinClaw data directory plus `repo_projects`. The lower-level provisioner fallback is `~/.thinclaw/projects`; prefer the resolved `repo_projects.workspace_base_dir` so all operators see the same layout.

For repository `owner/repo`, the local clone path is:

```text
<workspace_base_dir>/owner__repo
```

For a task with short id `abcdef123456`, the worktree path is:

```text
<workspace_base_dir>/owner__repo__wt__abcdef123456
```

Task branch names use:

```text
thinclaw/<project_slug>/<task_short_id>
```

Safety properties already implemented in the workspace helper:

- Owner, repo, project slug, and task id are validated before paths or branch names are built.
- Path traversal-like repo components are rejected.
- Existing task worktrees are force-removed before recreation.

Operational expectations:

- Put the workspace on a filesystem with enough space for multiple clones and worktrees.
- Do not manually edit supervisor worktrees during a running project unless you are recovering from a known failure.
- If a worktree is wedged, pause the project, inspect `git worktree list`, remove the stale worktree, then resume after confirming durable task state.

## Safety Model

The supervisor is trusted host runtime code. It is not a sandbox boundary. Safety comes from layered controls:

- Feature-gated API writes and supervisor startup.
- Durable state machine transitions for projects and tasks.
- Human approval requirements on repo-project tools that mutate state.
- Default `auto_merge_default = false`.
- Default `default_write_mode = "fork_pr"`.
- Separate worker secrets: `github_fork_token` for fork PR workers, upstream `github_token` / App credentials only for maintainer modes.
- `read_only_clone` injects no GitHub write credential into sandbox jobs.
- Per-project concurrency defaults of one task at a time.
- Bounded supervisor wake channel.
- Watchdog reconciliation from durable state after restart.
- Workspace path and branch-fragment validation.
- Short-lived GitHub App installation tokens where configured, with PAT fallback isolated by write mode.

Current limitations:

- The in-memory webhook deduper is not a replay-proof audit log. It survives only within a process; use durable event storage before treating webhook delivery as auditable.
- The setup checklist contains readiness placeholders for docker agents, credentials, and notifications; treat it as UI state, not proof of an external integration.
- CI/branch/review/findings evidence is re-derived from the GitHub API at reconcile time rather than from a durable per-delivery webhook log, so a permanently unreachable GitHub API stalls a task in its current state (visible via events) rather than advancing it.

## Auto-Merge Threat Model

Auto-merge should remain disabled until all merge gates are end-to-end and observable.

Threats to protect against before enabling real merge execution:

- Forged or replayed webhooks causing stale CI success or fake PR state.
- Compromised task branch pushing unexpected commits after approval.
- CI bypass through missing required checks, skipped workflows, or branch protection drift.
- Malicious or accidental changes to generated workflows, release scripts, or secret-reading code.
- Confused-deputy merges into the wrong repo, base branch, or installation.
- Review dismissal or force-push behavior invalidating a previously approved gate.
- Secret exfiltration through tests, build logs, PR comments, or generated artifacts.

Minimum merge gate contract:

- Project policy has `auto_merge = true`.
- Project policy has `write_mode = maintainer_auto_merge`.
- Repository is enrolled and mapped to the expected installation id.
- PR head branch matches the supervisor branch pattern and expected task id.
- PR base branch matches the enrolled repo base branch.
- Required checks and statuses are green for the exact head SHA to be merged.
- Branch is up to date with the base branch or branch protection allows the chosen state.
- No blocking reviews.
- Security and secret scanning checks have no blocking findings.
- A `MergeGateEvaluated` event is recorded for the task before merge.
- The merge method is the project policy method: `squash`, `merge`, or `rebase`.

The merge-gate evaluator models those denial reasons, the decision is persisted as a `MergeGateEvaluated` event, and the supervisor now executes the squash merge **only** when the gate approves. Approval is two-phase by construction: the first review reconcile records the `MergeGateEvaluated` audit event (denied with `gate_event_missing`), and only a subsequent reconcile that sees that recorded event can approve and merge. CI is re-confirmed green inside the review step, so a push that lands during review returns the task to `WaitingCi` rather than merging stale state. Auto-merge remains gated on project `auto_merge = true`, `write_mode = maintainer_auto_merge`, and repo enrollment; with fork/manual modes, a green/reviewed PR is held for a human merge.

## Local Smoke Checklist

Use this for a local non-GitHub smoke pass.

1. Start from a clean-enough worktree and note any unrelated uncommitted changes.
2. Enable repo projects:

   ```sh
   export REPO_PROJECTS_ENABLED=true
   export REPO_PROJECTS_WORKSPACE_BASE_DIR="$(mktemp -d)/repo-projects"
   ```

3. Start ThinClaw with the LibSQL or Postgres backend you are validating.
4. Create a repository project through one available surface:

   ```json
   {
     "name": "Supervisor smoke",
     "repo_url": "github.com/example/example",
     "default_branch": "main"
   }
   ```

5. Confirm the project appears in `setup_required` state and has `ProjectCreated` plus `RepoEnrolled` events.
6. Enqueue a task and confirm it appears in backlog state `queued` with a branch like `thinclaw/supervisor-smoke-<id>/<task_short_id>`.
7. Start or plan the project and watch logs for `repo project supervisor decision`.
8. Restart ThinClaw.
9. Confirm the project, task, events, worker runs, and merge gates still load from the database.
10. Pause, resume, and cancel the project. Confirm invalid transitions are rejected and valid transitions append events.

Expected current result: persistence and status surfaces work; when a sandbox job manager is available, the supervisor creates an isolated worktree and dispatches a bounded coding job. In `fork_pr`, the worker is instructed to push only to `thinclaw-write` using `github_fork_token`; in maintainer modes, it may push the task branch to `origin` only when explicitly configured. When GitHub credentials are configured and the worker pushes the task branch, the supervisor opens/updates the PR, polls and classifies CI, and — only for an enrolled project with `auto_merge = true`, `write_mode = maintainer_auto_merge`, and a fully satisfied merge gate — performs a single squash merge. Without the required credentials for the selected mode, the loop surfaces a blocker.

## Test Fixture Plan

Keep fixtures explicit and layered:

- Unit fixtures for state transitions, branch/path validation, merge-gate denial reasons, webhook signature verification, delivery dedupe, and envelope parsing.
- Store fixtures for LibSQL/Postgres round trips of project, repo, task, worker run, event, and merge-gate JSON.
- Restart fixtures that reopen a file-backed database and reconcile active, planning, blocked, waiting-CI, and completed task states.
- Workspace fixtures using a local bare Git repo so clone/fetch/worktree behavior can be tested without GitHub.
- GitHub API fixtures using recorded HTTP responses or a mock server for installation-token minting and eventual PR/check interactions.
- Webhook fixtures with signed payloads for `pull_request`, `check_run`, `workflow_run`, `status`, `push`, and duplicate deliveries.
- End-to-end gated fixtures only after PR creation and CI/merge execution exist.

Do not use live GitHub repositories for ordinary CI. Reserve live tests for a manually triggered integration suite against a disposable repository and GitHub App installation.

## Recovery Behavior

On process restart:

- LibSQL/Postgres migrations recreate missing repo-project tables and indexes when needed.
- Durable project/task/run/event/merge-gate rows are reloaded by API reads.
- The background supervisor loop runs a one-time recovery pass before steady state: it re-syncs sandbox jobs that completed during the outage and blocks any task left `Running` with no correlated worker record, then starts the watchdog.
- Queued/ready tasks can be converted into sandbox jobs and durable worker-run records when the runtime provides a `ContainerJobManager`.
- A queued or ready task produces a `DispatchTask` decision; a waiting-CI task produces a `WaitForCi` decision; a waiting-review task produces an `AwaitingReview` decision; a merged task produces a `Merged` decision; all-done tasks complete the project and produce a `Completed` decision.

Manual recovery steps:

1. Set `REPO_PROJECTS_ENABLED=false` or pause affected projects if the API is healthy.
2. Capture logs around `repo project supervisor decision` and `repo project reconcile failed`.
3. Snapshot the database before changing state by hand.
4. Inspect project events and task states through the API or DB.
5. Inspect workspace clones and worktrees with `git status` and `git worktree list`.
6. Remove only stale supervisor-created worktrees after preserving any useful diff.
7. Resume or restart ThinClaw and let the watchdog reconcile.

Escalate to code repair when:

- A task is repeatedly blocked by recovery as `Running` with no worker-run/job record (recovery blocks it once; repeated occurrences indicate a dispatch/persistence bug).
- A merge gate is approved for a task whose head SHA or PR number no longer matches GitHub.
- Webhook delivery replay becomes necessary; durable webhook storage is not implemented yet.
- The supervisor logs repeated GitHub API auth failures despite a configured App private-key secret or `github_token` (check the secret name, installation id, and App permissions).
