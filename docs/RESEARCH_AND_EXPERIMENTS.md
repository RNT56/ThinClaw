# Research And Experiments

This page is the operator-facing overview for ThinClaw's research workspace, experiments surface, and remote-runner orchestration.

## What It Covers

- the WebUI `Research` tab
- research projects, runners, and campaigns
- experiment and opportunity review
- GPU cloud launch helpers and lease/reissue flows

## Enablement

The research surface is gated behind the experiments feature flag.

- Enable `experiments.enabled` in Settings → Advanced → Experiments
- Once enabled, the `Research` tab becomes available in the WebUI

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

## Relationship To Other Surfaces

- Use [MEMORY_AND_GROWTH.md](MEMORY_AND_GROWTH.md) for durable memory, recall, learning, and prompt mutation
- Use [SURFACES_AND_COMMANDS.md](SURFACES_AND_COMMANDS.md) for shared cross-surface command vocabulary
- Use [DEPLOYMENT.md](DEPLOYMENT.md) for service mode, remote execution, and operator setup details

Research is complementary to day-to-day chat. The chat surface handles the current conversation; the research surface handles structured, benchmarked, and repeatable improvement work.
