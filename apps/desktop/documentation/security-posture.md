# Desktop Security Posture

Last updated: 2026-07-13

The Settings **Security** page is a read-only explanation surface for controls
that the active embedded ThinClaw runtime is actually using. It does not grant,
revoke, approve, block, or reconfigure anything.

## Evidence Sources

In local mode, `thinclaw_security_posture` reads three live sources:

1. `SafetyLayer` counters and its in-memory ring of the 50 most recent safety
   decisions. Each event contains only time, action, tool source, rule/reason,
   and severity.
2. The effective `SandboxModeConfig` captured when the current runtime was
   assembled, including policy, memory/time limits, and outbound network
   allowlist.
3. The active tool registry's descriptor metadata: tool name, read/write side
   effect, coarse approval class, empty-parameter approval result, and whether
   output sanitation is required.

The command and frontend never receive prompts, tool parameters, tool output,
or secret values. Telemetry is process-local, resets with the runtime, is not
uploaded, and is not a durable audit log.

## Runtime Modes

| Mode | Behavior |
| --- | --- |
| Local runtime active | Returns live evidence from the embedded runtime. |
| Local runtime stopped | Returns an explicit unavailable reason and empty summaries. |
| Remote gateway | Returns an explicit unavailable reason because the gateway contract does not expose authoritative remote security evidence. |

The panel polls every five seconds and supports manual refresh. A failed
refresh keeps the last successful snapshot visible and displays the error.

## Interpretation Limits

- Descriptor approval classes are coarse registry metadata. Concrete tool
  parameters and runtime validators remain authoritative for conditional
  decisions.
- Automatic approval status is shown because it can bypass conditional prompts;
  tools classified as always-approval still require an explicit decision.
- A write-capable tool classified as coarse `never` is highlighted for review,
  but this does not claim that sandbox, grant, or tool-specific validators are
  absent.
- The former `DangerousToolTracker` was never wired into enforcement and was
  removed. This panel deliberately does not recreate or represent it as a live
  control.

## Contract Maintenance

The command is registered in `backend/src/setup/commands.rs`, classified as
`LocalOnly`, represented in the generated TypeScript bindings, and covered
by the command-binding contract test. Regenerate bindings and the remote route
matrix whenever its DTO or route changes.
