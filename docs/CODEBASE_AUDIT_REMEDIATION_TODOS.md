# Codebase Audit Remediation TODOs

This file tracks the remaining implementation work after the first remediation pass from `docs/CODEBASE_AUDIT_RESULTS.md`.

## Still Open

### Experiments / Remote Runners

- Unify experiment trial finalization behind one shared terminal-write path for local, remote, success, and failure exits.
- Add explicit remote-runner readiness classes:
  - `manual_only`
  - `bootstrap_ready`
  - `launch_ready`
- Restrict campaign planning and automatic launch selection to `launch_ready` adapters.
- Add targeted tests for:
  - failed prepare/run paths
  - completion-post failures
  - readiness-class selection in campaign planning

### Tool Policy Precedence and Coverage

- Route `tool_policies` through the full config precedence stack, not only persisted settings:
  - env
  - TOML
  - settings
- Add focused regression tests proving both prompt-time filtering and execution-time denial for:
  - main chat agent
  - approval resume path
  - subagents
  - worker/routine jobs

### Post-Compaction Context

- Extend post-compaction reinjection beyond workspace-rule appendix:
  - active skill context
  - pinned durable facts
- Add regression tests proving reinjection is cleared correctly on:
  - `/clear`
  - `/undo`
  - `/redo`
  - `/resume`
  - new compaction pass

### Docs / Repo Hygiene

- Sweep remaining docs for stale references to removed plugin-surface internals.
- Sweep audit-plan historical docs for deleted file paths where they are presented as current runtime structure.
- Remove generated/environment-specific artifacts from version control and keep them ignored.
- Parameterize any remaining deploy-time constants that should not live as repo defaults.

## Verification Follow-Up

- Run targeted tests for:
  - Gmail `history_id` delta progression and expired-history fallback
  - tool-policy enforcement
  - post-compaction reinjection
  - WASM channel hot-reload webhook registration
  - startup/onboarding migration and `--no-db`
- Run a broader `cargo test` pass once the local cargo test hang is understood in this environment.
