# WS-09 — Repo-Project Supervisor & Experiments

| | |
|---|---|
| **Priority** | P1 (9.1–9.6, 9.8) · P2 (rest) |
| **Effort** | M |
| **Status** | ⏳ Not started |
| **Depends on** | WS-01 |
| **Findings** | H6 (DATA-1, DATA-2), DATA-3, DATA-5, DATA-6, DATA-7, DATA-9, DATA-10; open follow-up F-13 |
| **Canonical docs** | `docs/REPO_PROJECT_SUPERVISOR.md`, `docs/RESEARCH_AND_EXPERIMENTS.md`, `docs/remediation/FOLLOWUPS.md` (F-13 pointer only), `FEATURE_PARITY.md` |

## Goal

The supervisor (default-off) never merges code that CI and an authorized human haven't vetted at that exact SHA, and its workers hold only the credentials their role needs. Autonomous experiment campaigns have hard spend limits by default.

## Tasks

### Repo-project supervisor

- [ ] **9.1 — No merge without CI on the merge SHA (H6/DATA-1).**
  `src/repo_projects/pipeline.rs:~384` blocks only when checks are non-empty and non-green; `:~436` sets `checks_green: checks.is_empty() || checks_green` in the review phase. `has_recorded_merge_gate_event` (`crates/thinclaw-repo-projects/src/lib.rs:~637-641`) matches per task, not per SHA, so an older SHA's evaluation satisfies "two-phase". Treat empty checks during review as a hold (same grace/human path as `handle_empty_ci`, `pipeline.rs:~191`); key gate events and approvals to `head_sha`.
  *Accept:* test — task approved on SHA A, push B with no check runs → no merge.

- [ ] **9.2 — Count only authorized approvals (H6/DATA-2).**
  `pipeline.rs:~955-981` counts the latest APPROVED per login with no `author_association`/permission check; only the first 100 reviews are read; `unresolved_threads: 0` is hard-coded. Count only reviewers with write access (or defer entirely to branch-protection required reviews), paginate, and collect unresolved review threads (GraphQL).

- [ ] **9.3 — Least-privilege worker credentials (DATA-3).**
  `src/repo_projects/executor.rs:~592` (`dispatch_review_task`) and `:~1038-1056` give review jobs the same grant as maintainer workers (long-lived `github_token` as `GITHUB_TOKEN`/`GH_TOKEN`); the only restriction is prompt text. Give review jobs no write credential; prefer short-lived installation tokens scoped to one repo's contents; drop `api.github.com` write egress from review sandboxes (see WS-03.9).

- [ ] **9.4 — Default paths that actually dispatch (DATA-5).**
  Default `workspace_base_dir = resolve_data_dir("repo_projects")` (`crates/thinclaw-config/src/repo_projects.rs:~110-111`) is outside the bind-mount validator's allowed base `resolve_data_dir("projects")` (`src/orchestrator/job_manager.rs:~1207-1208`) → every default dispatch fails "outside allowed base" (the docs example fails too). Only the linked worktree is mounted; its `.git` gitdir and `thinclaw-write` remote live in the unmounted common `.git`, so in-container commit/push likely fails. Fix the base (or the validator), mount what git needs (or use per-task clones), and extend `tests/repo_project_docker_e2e.rs:~270-274` past "container reaches Running" to an actual commit + push to a local bare remote.

- [ ] **9.5 — Make the merge-gate contract true (DATA-6).**
  `docs/REPO_PROJECT_SUPERVISOR.md` "Minimum merge gate contract" claims checks the code doesn't do: no commit-status API call (legacy statuses ignored, yet "Commit statuses: Read" is a required permission); PR base branch and installation-id mapping not re-verified at merge; `security_findings`/`secrets_findings` can only be non-zero when CI is already red. Implement or remove each claim.

- [ ] **9.6 — Escalating write mode needs explicit approval.**
  Repo-project tools are `UnlessAutoApproved`, so with auto-approve the model can set `default_write_mode=maintainer_auto_merge` and `auto_merge_default=true` via `repo_project_setup`. Make any change that widens write mode or enables auto-merge `Always`.

- [ ] **9.7 — Webhook wiring gaps.**
  Webhook payloads are not mapped to state transitions (documented as pending). `REPO_PROJECTS_GITHUB_WEBHOOK_SECRET_SECRET` is honored by config but not by the webhook handler, which reads only DB settings plus an undocumented raw `THINCLAW_REPO_PROJECTS_GITHUB_WEBHOOK_SECRET`. Unify secret resolution; document; enabling via setup/tool currently needs a restart — document or hot-enable.

### Experiments

- [ ] **9.8 — Default spend ceilings (DATA-7).**
  Default `ExperimentStopPolicy` has `max_trials`, `max_total_runtime_secs`, `max_total_cost_usd` all `None` (`crates/thinclaw-experiments/src/types.rs:~184-192`, `crates/thinclaw-settings/src/experiments.rs:~45-55`); the controller stops only on plateau or 3 infra failures (`src/api/experiments/controller.rs:~222-245`). Ship conservative defaults and an instance-wide experiments budget enforced through CostGuard.

- [ ] **9.9 — Validate SSH/Slurm launch arguments (DATA-9).**
  `src/experiments/adapters.rs:~716-756, ~1017-1049`: `host`, `user`, `port`, `identity_file`, `login_host` go to `ssh` without rejecting a leading `-` (e.g. `-oProxyCommand=…`); `sbatch_args` is spliced into a remote shell. Validate and insert `--` before the host. Must land before 9.10 re-enables launch.

- [ ] **9.10 — Decide remote auto-launch.**
  `try_auto_launch` hard-fails every backend with `secure_delivery_required` (`adapters.rs:~661`), making all RunPod/Vast/Lambda/SSH/Slurm/Kubernetes launch code after the guard dead, while validation still reports `launch_ready` and `docs/RESEARCH_AND_EXPERIMENTS.md` describes the helpers. Either design secure credential delivery and re-enable, or remove the dead code and fix validation + docs.

- [ ] **9.11 — Fix the quarantined campaign race.**
  `#[ignore]` remains on the autonomous-campaign end-to-end test (`src/api/experiments/tests.rs:~858`); `prepare_campaign_worktree` still discards git results (`let _ =`) and has no per-campaign lock (`src/api/experiments/git.rs:~12-48`). Fix the race and un-ignore.

- [ ] **9.12 — Scope global records (DATA-10).**
  Experiment targets/target links/model usage/opportunities (`src/api/experiments/crud.rs:~567-870`) and all repo-project reads (`src/channels/web/handlers/repo_projects.rs:~103, ~114, ~266, ~280`) ignore identity and are readable by `ReadOnly` principals on a shared gateway. Scope by owner or document as instance-global admin data. Desktop `api::sessions::delete_thread`/`clear_thread` (`src/api/sessions.rs:~302, ~337`) delete by id without an owner check — add it for parity with the gateway path (`chat.rs:~1000`).

- [ ] **9.13 — F-13 object-store `ArtifactStore` (carried, deferred).**
  `LocalArtifactStore` remains the only implementation (`src/experiments/artifact_store.rs:~119`). Keep deferred pending the `opendal` dependency/license review; leave this box unchecked with status ⏸️ unless picked up.
