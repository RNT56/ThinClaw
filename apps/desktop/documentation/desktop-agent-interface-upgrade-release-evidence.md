# Desktop Agent Interface Upgrade — Release Evidence

> **Status:** Code-level verification complete; credentialed and manual acceptance pending
> **Date:** 2026-08-01
> **Scope:** Agent Cockpit migration described in [the upgrade roadmap](desktop-agent-interface-upgrade-roadmap.md)

## What shipped

- A ten-destination Agent Cockpit: Home, Chat, Workspace, Channels,
  Automations, Jobs, Capabilities, Usage, Operations, and Advanced.
- A typed primary-route registry with aliases for all 32 former destinations;
  deep links now resolve to their appropriate center/tab without restoring a
  32-item sidebar.
- Shared Cockpit status, capability gating, async state, tabs, notices, status
  badges, metrics, and destructive-action confirmation patterns.
- Truthful operational states: no production shell fixtures, no optimistic
  success for rejected/partial outcomes, no implied target dispatch, and no
  stored secret values rendered to React.
- Desktop-managed sub-agent spawn, list, and status operations are explicitly
  Local Core-only. Remote profiles neither invoke those local commands nor
  show an enabled manual spawn control; they receive a typed unavailable state
  and a Local Core remediation.
- Lazy-loaded centers that preserve real existing feature panels under the
  consolidated information architecture, plus a static semantic-style check for
  the rebuilt Agent surfaces.

## Automated verification

All commands below were run from `apps/desktop/` on 2026-08-01 unless noted.

| Gate | Result | Evidence |
| --- | --- | --- |
| `npm run lint:ts` | Pass | Frontend and browser-E2E TypeScript checks passed. |
| `npm test` | Pass | 63 test files; 261 tests passed. |
| `npm run test:performance` | Pass | 3 test files; 6 tests passed. |
| `npm run lint:agent-styles` | Pass | 24 rebuilt Agent Cockpit surfaces checked for structural palette hardcodes. |
| `npm run lint:fmt` | Pass | `cargo fmt --all -- --check` passed. |
| `npm run build` | Pass | 154 chunks; largest initial chunk is `react-vendor` at 185.2 KiB, within the configured bundle budget. |
| `npm run test:backend` | Pass | 585 tests passed; 2 intentionally ignored. |
| `npm run lint:thinclaw` | Pass | Full workspace Clippy completed with `-D warnings`. |
| `npm run contracts:check` | Pass | Generated runtime contract check completed successfully. |
| `cargo run --locked --example export_bindings` | Pass | Desktop bindings and the route matrix regenerated from the final registry; Desktop-managed session spawn is `LocalOnly`. |
| `npm run test:e2e:browser` | Pass | 3 spec files / 21 browser tests passed: 12 top journeys, 6 model-browser journeys, and 3 onboarding journeys. |

The browser run emitted one development telemetry warning that renderer-ready
time was 3153 ms against a 2500 ms observation budget. It did not fail the
configured performance or browser gates, and the production bundle budget
passed; retain it as a release-performance observation when testing on a clean
release build.

## Intentional capability gates

| Surface | Current state | Reason / required next step |
| --- | --- | --- |
| Fleet targeted task assignment and abort | Unavailable | The current Desktop command path cannot prove dispatch to the selected remote profile. Fleet shows authenticated status and broadcast receipts only. Add a typed selected-profile dispatch/abort contract before enabling controls. |
| Channel-secret editing | Unavailable | Channel schema responses omit password values and the Desktop does not yet have an encrypted channel-secret binding. Non-secret schema fields may be edited; configure secrets through the gateway/host until the binding exists. |
| Repo Projects | Quarantined in Advanced | The backend flow remains available, but the Cockpit does not show unproven project state or production fixtures as live. Re-expose only after an end-to-end runtime proof. |
| Host-only operations in remote mode | Capability-gated | The Cockpit names the mode, freshness, and remediation instead of implying remote parity for local-only commands. |
| Desktop-managed sub-agents in remote mode | Local Core-only | Spawn, list, and status backend routes return a typed `LocalOnly` unavailable response in remote mode. The Cockpit suppresses child-session loading and exposes a named disabled control with a Local Core remediation. |

## Required credentialed smoke before external release

The automated suite uses deterministic local/browser fixtures and deliberately
does not use real credentials. A release operator must run the dated manual
checklist with an embedded runtime and a credentialed remote profile:

1. Verify local and remote profile status, source, freshness, reconnect/error
   behavior, and gateway lifecycle actions.
2. Verify a Fleet broadcast receipt succeeds once per reachable profile and a
   disconnected profile reports a named failure; confirm no targeted task or
   abort control appears.
3. Verify Channel Setup shows non-secret current values only and does not
   render, export, or copy stored password values.
4. Exercise a supported Job action plus an unavailable job capability; verify
   the shared confirmation dialog and outcome copy distinguish cancellation,
   persistence, forwarding, and restart requirements.
5. In a remote profile, verify Desktop-managed sub-agent controls remain
   unavailable, do not request child sessions, and name Local Core as the
   remediation. In Local Core, verify a supported sub-agent workflow and its
   job/status lifecycle.
6. Review default/compact density, light/dark/system palettes, forced colors,
   reduced motion, zoom, screen-reader announcement, and keyboard/focus
   behavior against the accessibility contract.

Use [the manual smoke checklist](manual-smoke-checklist.md) to record the
profile, environment, operator, and outcome. Do not record bearer tokens,
channel credentials, or secret values in that evidence.

## Rollback boundary

- The truth repairs are forward-only: do not restore fake Repo Projects data,
  optimistic success copy, secret rendering, or unsupported Fleet dispatch in a
  navigation rollback.
- Route aliases make former deep links resolve inside the new Cockpit; rollback
  the migration as one reviewed change set if necessary rather than reviving
  individual deleted sidebar pages.
- Preserve Operations access to gateway stop and checkpoints throughout any
  rollback. Verify the target profile and checkpoint before a destructive
  restore.
