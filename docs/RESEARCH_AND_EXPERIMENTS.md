# Research And Experiments

This page is the operator-facing overview for ThinClaw's research workspace, experiments surface, and remote-runner orchestration.

## What It Covers

- the WebUI `Research` tab
- research projects, runners, and campaigns
- experiment and opportunity review
- GPU cloud launch helpers and lease/reissue flows
- Phase 1 `AgentEnv` rollouts for eval and SFT data collection

## Enablement

The research surface is gated behind the experiments feature flag.

- Enable `experiments.enabled` in Settings → Advanced → Experiments
- Once enabled, the `Research` tab becomes available in the WebUI
- The experiments API supports both configured database backends (`postgres` and `libsql`); there is no hidden postgres-only read path for campaign/trial/artifact retrieval

## Mental Model

Use the research stack when you want ThinClaw to manage benchmarked, repeatable improvement work rather than one-off chat turns.

- A **project** defines the code workspace, harness, mutable paths, and scoring rules
- A **runner** defines where that work executes: local, remote, or GPU-backed infrastructure
- A **campaign** is a bounded search/execution loop against a project + runner pair
- **Opportunities** and experiment summaries help decide what to run next

## WebUI Areas

- `Overview`: recent status, next actions, and high-level research telemetry
- `Opportunities`: suggested directions and open improvement slots
- `Projects`: managed benchmark projects and their workspace rules
- `Runners`: execution backends, images, env grants, and GPU requirements
- `Campaigns`: active and historical campaign runs, trial details, and lease commands
- `GPU Clouds`: quick-launch helpers for external GPU providers

## AgentEnv Phase 1

The `src/agent/env/` framework packages ThinClaw's normal multi-turn agent loop as a reusable rollout environment.

- `AgentLoopEnv` sends synthetic `IncomingMessage` turns through `Agent::handle_message_external`
- `EnvRunner::evaluate` runs scripted episodes and stores `agent_env` run artifacts
- `EnvRunner::collect_sft_jsonl` exports positive trajectories as chat-format JSONL
- `EnvRunner::serve_openai_compatible` exposes `/v1/chat/completions` for external eval harnesses
- `TerminalBenchEnv` and `SkillBenchEnv` can now run through Research campaigns by creating an `agent_env` runner with `backend_config.benchmark` set to `terminal_bench` or `skill_bench`
- trajectory steps carry token/logprob capture metadata when a provider can supply exact token IDs and logprobs; unsupported providers mark the capability flags false rather than pretending synthetic data is exact

Example `backend_config` for a terminal benchmark runner:

```json
{
  "benchmark": "terminal_bench",
  "cases": [
    {
      "name": "smoke",
      "command": "printf bench-ok",
      "expectedStdoutContains": ["bench-ok"],
      "expectedExitCode": 0,
      "timeoutSecs": 30
    }
  ]
}
```

Example `backend_config` for a skill benchmark runner:

```json
{
  "benchmark": "skill_bench",
  "cases": [
    {
      "name": "skill-readiness",
      "skillContent": "# Skill Name\n\nDescribe when to use this skill and the concrete workflow.",
      "requiredSubstrings": ["when to use", "workflow"]
    }
  ]
}
```

## Candidate Generation And Run History

Planner, mutator, reviewer, and runner executions now emit durable run-artifact records even when autonomous candidate generation fails before a trial is created.

- campaign metadata keeps the latest candidate-generation status and the related run-artifact IDs
- failed planner/mutator/reviewer attempts are preserved instead of disappearing on early exits
- successful autonomous runs still carry their planner/mutator/reviewer artifacts into the prepared trial manifest
- reviewer rejection and "no candidate diff" outcomes are recorded as failed subagent runs rather than misleadingly staying `completed`

The autonomous path also has an end-to-end regression harness now. The library test suite includes a real planner -> mutator -> reviewer -> runner flow that:

- creates a temporary git-backed project workspace
- lets the mutator write an actual candidate change through the subagent tool path
- launches the candidate through the `local_docker` runner path
- verifies the accepted trial, score improvement, artifact lineage, and recorded reviewer decision end to end

## Execution And Isolation Notes

The research stack now uses the shared execution/runtime hardening path more consistently than it used to.

- `local_docker` trials execute through the shared Docker-backed execution backend instead of a bespoke subprocess path
- remote launch and revoke helpers now use the same backend-style command execution path for `ssh`, `slurm`, and `kubectl` helpers rather than raw ad hoc process spawning
- runner validation now classifies backends into `manual_only`, `bootstrap_ready`, and `launch_ready`, and queued auto-launch only proceeds for `launch_ready` runners
- queued campaign launch is owner-aware, so one operator's controller pass does not launch another operator's queued campaign under the wrong settings or secrets
- campaign, trial, and artifact reads are owner-scoped at the storage boundary
- remote-runner launch now fails closed when `campaign.gateway_url` is missing/empty instead of generating a broken bootstrap command
- research subagents are worktree-scoped and their shared denylist now blocks `memory_read`, `memory_search`, and `session_search` so planner/mutator/reviewer runs do not pull unrelated recall into benchmark work
- project `workdir` is validated as a relative in-workspace path and is re-checked against the campaign worktree before local trial execution
- local trials now persist `summary.json` into the experiments artifact store and restore the dedicated campaign worktree back to a clean committed state after each run, so benchmark artifacts do not leak into later candidate diffs
- local trial completion and remote lease `/complete` now flow through the same terminal finalization path for stage normalization, cost/runtime aggregation, canonical run-artifact append, and repeated-terminal rejection
- campaign, trial, and job-adjacent research surfaces now expose runtime backend metadata more consistently, so `local_docker` and sandbox job executions are distinguishable in the UI and APIs without reverse-engineering the launch path

## Relationship To Other Surfaces

- Use [MEMORY_AND_GROWTH.md](MEMORY_AND_GROWTH.md) for durable memory, recall, learning, and prompt mutation
- Use [AGENT_ENV.md](AGENT_ENV.md) for the eval/SFT environment API
- Use [SURFACES_AND_COMMANDS.md](SURFACES_AND_COMMANDS.md) for shared cross-surface command vocabulary
- Use [DEPLOYMENT.md](DEPLOYMENT.md) for service mode, remote execution, and operator setup details

Research is complementary to day-to-day chat. The chat surface handles the current conversation; the research surface handles structured, benchmarked, and repeatable improvement work.
