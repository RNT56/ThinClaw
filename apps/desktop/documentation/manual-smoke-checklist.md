# ThinClaw Desktop Manual Smoke Checklist

Last updated: 2026-05-15

This checklist is the repeatable P3-W2 manual acceptance run for ThinClaw
Desktop. It covers local and remote mode with the same product surfaces and
records unsupported behavior as explicit gated or unavailable states.

## Evidence Rules

Create one dated report per run. Record the commit SHA, platform, backend mode,
provider/model, remote gateway URL if used, screenshots or log paths, and a
pass/fail/blocked result for each surface. Do not mark a row as passed when it
only rendered in a plain browser without Tauri IPC.

Use these result values:

- `pass`: the workflow completed and the expected UI/event/backend state was
  observed.
- `fail`: the workflow was attempted and produced an incorrect result.
- `blocked`: prerequisites were missing, such as no remote gateway token, no
  provider key, no model, or no host permission grant.
- `gated`: the product deliberately disabled a dangerous or unsupported action
  and displayed a concrete reason.
- `unavailable`: the gateway or local runtime returned an explicit typed
  unavailable response with a concrete reason.

## Preconditions

1. Work from the Desktop repository root:

   ```bash
   cd /Users/mt/Programming/Schtack/ThinClaw/thinclaw-desktop
   ```

2. Install dependencies and sidecars:

   ```bash
   cd apps/desktop
   npm install
   npm run setup:all
   ```

3. Configure at least one usable local or cloud model provider in Settings.
   Provider keys must be saved and granted before agent injection is expected to
   work.

4. For remote mode, start or identify a root ThinClaw gateway and token:

   ```bash
   thinclaw gateway start --host 0.0.0.0 --port 18789 --foreground
   thinclaw gateway access --show-token
   curl http://REMOTE_HOST:18789/api/health
   ```

5. Leave desktop autonomy execution disabled unless running on a disposable
   host. For execution testing, enable reckless desktop autonomy in the host
   config and grant the required host permissions before launching Desktop.

## Automated Preflight

Run these before opening the app:

```bash
cd apps/desktop && npm run lint:ts
cd apps/desktop && npm test
cd apps/desktop && npm run build
cd apps/desktop/backend && cargo check --locked
cd apps/desktop/backend && cargo test --locked --lib -- --skip web_search
```

Expected result: all commands pass. Build warnings are acceptable only when they
are known Vite chunk or dynamic import warnings and do not change runtime
behavior.

## Local Mode Launch

Launch the full Tauri app. A plain Vite browser page is not sufficient for this
checklist because Tauri IPC and the Rust backend are required.

```bash
cd apps/desktop
npm run tauri:dev:llamacpp
```

If a different engine is under test, record the exact command and Cargo feature
set in the report.

## Local Mode Smoke

### Engine And Gateway Lifecycle

- Start Desktop and wait for the ThinClaw status indicator to settle.
- Open diagnostics/status and verify local mode, gateway health, engine family,
  active model, sidecar state, runtime revision, and cache stats.
- Restart the gateway or engine from the UI if the control is available.
- Expected events: lifecycle/status updates appear through `openclaw-event`.

### Chat Streaming

- Create a new session and send a simple prompt.
- Verify token streaming, final assistant message, transcript persistence, and
  no cross-session message leakage when switching sessions.
- Expected events: lifecycle, agent message, token/chat delta, and completion
  events on `openclaw-event`.

### Plan, Usage, And Cost

- Send a prompt that naturally asks the agent to plan several steps.
- Verify plan display, usage counters, model/provider metadata, and cost panel
  updates.
- Export or reset costs only if that action is part of the run. Record the
  exact result.
- Expected events: plan, usage, and cost events on `openclaw-event`.

### Approvals And Auth

- Trigger an action that requires approval, such as a shell or file write tool.
- Verify the approval card, approve path, reject path, transcript annotation,
  and absence of silent execution before approval.
- Open auth/channel status and verify OAuth or pairing states are visible or
  explicitly gated.
- Expected events: approval requested/resolved and auth status updates.

### Provider Vault, Routing, And Model Selection

- Save a provider key, grant it, revoke it, and reload secrets.
- Verify legacy fallback reads still work where configured, while new writes use
  ThinClaw identifiers.
- Run provider status, model discovery, advisor readiness, route simulation,
  routing rules, primary model pool, and cheap model pool checks.
- Expected result: raw secrets are never shown in remote surfaces and ungranted
  access is denied.

### Subagents And Jobs

- Trigger a workflow that creates a subagent or background job.
- Open Jobs and verify list, summary, detail, event history, cancel behavior,
  and explicit reasons for unsupported restart, prompt, or files in local direct
  jobs.
- If a sandbox job is available, test restart, interactive prompt/done, file
  list, and file read.
- Expected events: subagent and job lifecycle events on `openclaw-event`.

### Routines And Channels

- Open Routines and verify list, create, toggle, run, history, audit, and clear
  controls.
- Open Channels and verify Gmail OAuth/status, Apple Mail settings, Slack,
  Telegram, and pairing screens.
- Expected events: routine lifecycle events forward to `openclaw-event`.

### Extensions, Skills, And MCP

- Open Extensions and verify installed extension list, ClawHub search/install,
  manifest validation, lifecycle audit, setup/reconnect/remove/activate, and
  extension tools.
- Open Skills and verify search/install/reload/trust/inspect/publish states.
- Open MCP and verify server list, interaction history, auth state, reconnect,
  and explicit unavailable reasons where a backend route is absent.

### Memory, Canvas, And A2UI

- Run memory tree, read, write, and search workflows.
- Verify delete/export controls are either supported or explicitly unavailable.
- Trigger a canvas/A2UI artifact, then verify open, update, dismiss, and
  availability states.
- Expected events: canvas events on `openclaw-event`.

### Experiments And Learning

- Open Experiments and verify projects, campaigns, runners, trials, targets,
  model usage, opportunities, GPU cloud validation, and test launch controls.
- Open Learning and verify status, history, candidates, outcomes, proposals,
  reviews, and rollbacks.
- Mutations must be enabled only when the root ThinClaw APIs and config allow
  them; otherwise record the visible gated reason.

### Autonomy

- With default config, open Autonomy and verify status, permissions, rollouts,
  checks, evidence, and disabled mutations with concrete reasons.
- On a disposable host with reckless desktop autonomy enabled, test bootstrap,
  pause, resume, permissions, checks, evidence, rollout promotion visibility,
  and rollback.
- Expected result: execution is never available without explicit reckless host
  config and host permission checks.

## Remote Mode Smoke

Connect Desktop to a remote ThinClaw gateway from Settings, then repeat the
local checklist. Record the gateway URL, auth method, and root ThinClaw commit
or version when known.

Additional remote-specific checks:

- Chat uses `POST /api/chat/send`; approvals use `POST /api/chat/approval`.
- Remote SSE events re-emit as `openclaw-event` with the same `UiEvent` schema
  as local mode.
- Session routing is metadata-first by `thread_id`, `session_key`, and `run_id`.
- Unsupported operations return typed unavailable responses, not success:
  abort, reset, export, compact, memory delete, hook management, local-only log
  snapshots, and any unsupported job/autonomy mutation.
- Provider vault surfaces expose save/delete/status only; raw secret reads are
  denied.
- Autonomy host-executing mutation remains gated unless the remote host policy
  explicitly allows it.

## Completion Criteria

The P3-W2 smoke run is complete when:

- Local and remote reports exist or the missing mode is explicitly blocked with
  concrete prerequisites.
- Every listed surface has `pass`, `fail`, `blocked`, `gated`, or
  `unavailable`.
- No desktop control silently succeeds without doing the work.
- Every failed or blocked item includes a next action, owner, or known upstream
  dependency.
