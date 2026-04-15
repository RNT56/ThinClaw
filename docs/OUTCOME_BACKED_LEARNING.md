# Outcome-Backed Learning

This document describes ThinClaw's deferred consequence-learning layer inside Memory & Growth and the Learning Ledger.

It is the canonical reference for:

- what outcome-backed learning currently does
- which user-facing surfaces expose it
- how manual outcome review behaves
- which follow-ups are still optional rather than required for v1

## Status

Outcome-backed learning v1 is implemented behind `learning.outcomes.enabled`.

It extends the existing learning stack instead of replacing it:

- outcomes become `outcome_contracts` plus `outcome_observations`
- evaluated outcomes write back into the existing Learning Ledger as `learning_evaluations`
- non-`learning_event` outcome sources create synthetic ledger events so the Learning Ledger stays canonical
- repeated high-confidence negative patterns can still flow into the existing candidate/proposal machinery
- the background evaluator now runs per user and respects each enabled user's `learning.outcomes.evaluation_interval_secs` and `max_due_per_tick`
- `/api/learning/status` now reports real evaluator health instead of a placeholder
- outcome detail payloads now include normalized provenance fields for UI navigation and review
- assistant-turn outcome evaluations now carry explicit trajectory identifiers so turn-linked exports can match them deterministically
- the only supported outcome-driven routine auto-apply in this tranche is `notification_noise_reduction`, which disables `routine.notify.on_success`
- routine auto-applies now create `learning_artifact_versions` too, so they appear in the Learning Ledger as auditable applied artifacts instead of silent store-only mutations
- Research opportunities now consume recent negative evaluated outcome patterns, expose them as `source = "outcome_learning"`, and carry project-prefill hints so the Research project form can be seeded directly from deferred-reality evidence
- the gateway now ships a browser automation harness that verifies auth, Learning outcome rendering, outcome detail provenance, and outcome-backed Research project prefilling end to end

There is no historical backfill in v1. Contracts are only created for new activity after rollout.

## Current Scope

### Contract Types

V1 supports exactly three contract types:

- `turn_usefulness`
- `tool_durability`
- `routine_usefulness`

### Source Anchors

Outcome contracts currently anchor to existing runtime records:

- assistant turn usefulness -> `learning_event`
- tool durability -> `artifact_version` or `learning_code_proposal`
- routine usefulness -> `routine_run`

### Deterministic Evaluation Rules

The evaluator runs deterministically first and only uses the cheap model when observations conflict and `learning.outcomes.llm_assist_enabled` is true.

Important v1 rules:

- silence is neutral for turn and routine contracts
- tool durability surviving until due is a mild positive
- rollback, proposal rejection, explicit correction, repeated same-request follow-up, and routine disable/pause/mute after a visible routine run are strong negatives
- explicit thanks/approval and next-step continuation without correction are mild positives

### Matching Scope

V1 only matches within the same:

- `user_id`
- `actor_id`
- `thread_id`

There is no cross-thread or household-wide inference in this version.

## User-Facing Surfaces

### WebUI

The Learning Ledger includes an Outcomes section with:

- summary cards
- recent outcome contracts
- outcome detail inspection
- source provenance and ledger-event provenance
- direct in-app source navigation for history, artifacts, proposals, and routine runs
- manual review actions
- `Evaluate Now`

### HTTP API

The current outcome endpoints are:

- `GET /api/learning/outcomes`
- `GET /api/learning/outcomes/{id}`
- `POST /api/learning/outcomes/{id}/review`
- `POST /api/learning/outcomes/evaluate-now`

`/api/learning/status` also includes outcome summary counters.

Outcome payloads now expose normalized provenance fields:

- `ledger_learning_event_id`
- `last_evaluator`
- `source_ref`

### Agent Tools

ThinClaw exposes read-only outcome inspection through:

- `learning_status`
- `learning_outcomes`

There is intentionally no dedicated `thinclaw learning outcomes ...` CLI surface in v1.

### Research

Research now consumes outcome-backed evidence in the opportunities surface:

- recent negative evaluated outcome patterns can produce Research opportunities
- outcome-backed opportunities include `signals`, `confidence`, and `project_hint`
- the Research UI can prefill the project form directly from those hints
- this currently focuses on opportunity discovery and benchmark seeding rather than automatic campaign launch

### Heartbeat

Heartbeat can surface a deterministic `Outcome Review Queue` summary when:

- `learning.enabled = true`
- `learning.outcomes.enabled = true`
- `learning.outcomes.heartbeat_summary_enabled = true`

Heartbeat is a consumer of outcome state, not the source of truth for evaluation.

## Manual Review Semantics

Manual review is part of the canonical learning history now.

When an operator calls `confirm`, `dismiss`, or `requeue` from the Outcomes API or Learning Ledger UI:

1. The outcome contract is updated.
2. A matching `learning_evaluation` is written with evaluator `outcome_manual_review_v1`.
3. If the contract source is already a `learning_event`, that original event is reused.
4. If the contract source is not a `learning_event`, ThinClaw creates a synthetic `outcome_review::<source_kind>` learning event and evaluates against that.
5. The chosen ledger event id is stored back on the contract as `ledger_learning_event_id` so later review actions can reuse it instead of creating duplicates.

Current manual-review statuses:

- `confirm` -> `positive`, `neutral`, or `negative`
- `dismiss` -> `review`
- `requeue` -> `review`

## Runtime Behavior

### Evaluator Scheduling

The background evaluator now:

- discovers users with due or evaluating outcome work
- loads each user's `learning.outcomes` settings
- sleeps on the minimum enabled per-user evaluation interval for the current cycle
- evaluates each due user independently via `run_once_for_user(user_id)`
- reports stale health when open or evaluating contracts sit past `2 * evaluation_interval_secs`

Disabled outcome learning is treated as healthy/not-applicable for status reporting.

### Trajectory And Export Wiring

Assistant-side learning events now persist turn identity fields when available:

- `trajectory_target_id`
- `turn_number`
- `session_id`
- `thread_id`

`turn_usefulness` contracts copy those identifiers into contract metadata, and synthetic/manual outcome ledger events only propagate them for turn-linked contracts. Tool and routine synthetic events remain visible in the Learning Ledger but intentionally do not relabel unrelated trajectory turns.

### Routine Auto-Apply Guardrail

Outcome-driven routine auto-apply remains opt-in and narrow.

Current behavior:

- routine candidates stay review-only unless `routine` is explicitly present in `learning.auto_apply_classes`
- the only supported patch type is `notification_noise_reduction`
- that patch is only emitted when the negative routine pattern looks like notification noise from an `Ok` routine run with `notify.on_success = true`
- auto-apply updates the routine through the existing store/update path and refreshes the routine event cache afterward
- auto-applied routine patches are also recorded as `artifact_type = "routine"` artifact versions with before/after serialized routine content and provenance including `routine_id` plus `patch_type`

Still intentionally unsupported in this tranche:

- schedule changes
- disable/delete actions
- prompt rewrites
- any routine patch type besides `notification_noise_reduction`

## Boundaries In V1

These constraints are intentional in the current release:

- no historical backfill
- no cross-thread inference
- no dedicated CLI surface for outcome review
- no outcome-driven mutation of `IDENTITY.md`, `SOUL.md`, `AGENTS.md`, or `context/profile.json`
- prompt mutation remains gated by `learning.prompt_mutation.enabled`
- routine candidates remain review-only unless `routine` is explicitly included in `learning.auto_apply_classes`
- Research campaign execution still does not depend on outcomes in v1, but the opportunities surface now consumes outcome-backed signals for benchmark discovery and project prefilling

## Verification Status

The v1 tranche now includes regression coverage for:

- `Evaluate Now` processing only the requested user
- rollback-driven durability negatives
- proposal-rejection-driven durability negatives
- routine state-change-driven routine negatives
- heartbeat summary visibility
- repeated manual review reusing `ledger_learning_event_id`
- per-user pending-work discovery and evaluator health
- trajectory metadata propagation for turn-linked outcome events
- routine auto-apply patch emission staying limited to `notification_noise_reduction`
- routine auto-apply artifact recording for applied routine mutations
- outcome-backed Research opportunity generation and project-hint payloads
- browser-driven gateway verification for Learning outcome detail rendering and outcome-backed Research prefilling

The gateway now ships a browser automation harness in `tests/web_gateway_ui_browser_integration.rs`. The manual checklist below remains useful as a fast operator-facing smoke pass after UI changes.

## Optional Follow-Ups

These are no longer blockers for the shipped v1 pipeline:

- add a formal browser test harness for Learning Ledger interactions
- revisit whether per-class safe-mode thresholds are needed after real-world outcome data accumulates
- let Research consume outcome-backed labels beyond opportunity discovery and project prefilling in a later tranche

## Manual QA Checklist

Use this as a quick smoke test after UI changes or when browser automation is unavailable:

- open Learning Ledger, confirm the Outcomes summary shows evaluator health and non-placeholder counts
- open an outcome detail and verify `Source`, `Ledger Event`, `Last Evaluator`, `Due`, and `Verdict` all render
- use `Open Source` for a `learning_event` outcome and confirm the matching History row is highlighted
- use `Open Source` for an `artifact_version` outcome and confirm the matching Artifact row is highlighted
- use `Open Source` for a `learning_code_proposal` outcome and confirm the matching Proposal row is highlighted
- use `Open Source` for a `routine_run` outcome and confirm the Routines tab opens the owning routine and highlights the matching run
- confirm unsupported or context-missing source links render as disabled instead of silently failing
- submit an invalid outcome review request and confirm the UI surfaces a non-500 error toast
- open Research, confirm an outcome-backed opportunity renders with `Source: outcome_learning`, and verify `Create Project` prefills the Research project form from `project_hint`
