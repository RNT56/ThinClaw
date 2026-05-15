# ThinClaw Desktop Manual Smoke Report - 2026-05-15

## Run Metadata

| Field | Value |
| --- | --- |
| Date | 2026-05-15 |
| Operator | Codex |
| Repository | `/Users/mt/Programming/Schtack/ThinClaw/thinclaw-desktop` |
| Commit | `6adca864` |
| Branch | `codex/thinclaw-desktop-parity` |
| Platform | Darwin Ad-Aspera-411.local 25.3.0 arm64 |
| Rust | rustc 1.92.0 (ded5c06cf 2025-12-08) |
| Node | v23.1.0 |
| npm | 11.11.0 |
| Local backend mode | Not launched as a Tauri app in this session |
| Remote gateway | Not configured in this session |

## Automated Checks

| Check | Result | Notes |
| --- | --- | --- |
| `cd apps/desktop/backend && cargo fmt` | pass | Ran after adding P3 contract tests. |
| `cd apps/desktop/backend && cargo check --locked` | pass | Completed after the backend lockfile had already been refreshed by Cargo. |
| `cd apps/desktop && npm run lint:ts` | pass | TypeScript typecheck completed. |
| `cd apps/desktop && npm test` | pass | 45 Vitest tests passed. |
| `cd apps/desktop && npm run build` | pass | Vite build completed; only known dynamic import/chunk-size warnings were observed. |
| `cd apps/desktop/backend && cargo test --locked --lib -- --skip web_search` | pass | 296 passed, 0 failed, 1 filtered out. |
| `cd /Users/mt/Programming/Schtack/ThinClaw/thinclaw-desktop && cargo test --workspace` | pass | Root workspace tests and doctests completed successfully; ignored live/Docker smoke tests remained ignored. |
| `cd apps/desktop && npx tauri info` | pass with warnings | Command completed. It reports Xcode Command Line Tools installed, full Xcode not installed, Rust `tauri-cli` outdated relative to JS CLI, missing JS packages for global-shortcut and shell plugins, and app CSP/devUrl/frontendDist metadata. |
| `cd apps/desktop && npm run validate:packaging` | pass with warnings | Static bundle contract, engine override matrix, and focused platform tests passed. The script reports missing native sidecars for llama.cpp/MLX/vLLM until setup scripts are run, plus the same Tauri version/Xcode warnings. |

## Browser Probe

| Check | Result | Notes |
| --- | --- | --- |
| `cd apps/desktop && npm run dev -- --host 127.0.0.1` | pass | Vite served the frontend at `http://127.0.0.1:1420/`. |
| In-app browser opened Vite URL | blocked | The page rendered the onboarding shell, but this is not a valid Desktop runtime smoke because the browser page has no Tauri IPC or Rust backend access. |

## Local Mode Manual Smoke

| Surface | Result | Evidence / Next Action |
| --- | --- | --- |
| Engine and gateway lifecycle | blocked | Full `npm run tauri:dev:llamacpp` GUI was not launched interactively in this session. Run the checklist on a host with configured sidecars/model. |
| Chat streaming | blocked | Requires interactive Tauri runtime and configured provider/model. |
| Plan, usage, and cost | blocked | Requires a completed local agent run with usage/cost events. |
| Approvals and auth | blocked | Requires interactive approval flow in the Tauri app. |
| Provider vault, routing, and model selection | blocked | Requires configured provider credentials and manual Settings validation. |
| Subagents and jobs | blocked | Jobs UI and IPC were implemented in P2-W2, but manual local job execution was not exercised. |
| Routines and channels | blocked | Requires configured routine/channel state and Tauri runtime. |
| Extensions, skills, and MCP | blocked | Requires Tauri runtime and root ThinClaw registry state. |
| Memory, canvas, and A2UI | blocked | Requires interactive local session and artifact trigger. |
| Experiments and learning | blocked | Requires root ThinClaw experiment/learning data or configured test fixtures. |
| Autonomy default gated state | blocked | Autonomy UI and IPC were implemented in P2-W2, but the full Tauri app was not launched to inspect gated controls. |
| Autonomy reckless execution | gated | Not attempted. This must only run on a disposable host with explicit reckless desktop autonomy config and host permission grants. |

## Remote Mode Manual Smoke

| Surface | Result | Evidence / Next Action |
| --- | --- | --- |
| Gateway health and connection | blocked | No remote gateway URL/token was provided or discovered for this run. |
| Remote chat and SSE events | blocked | Requires remote gateway and token. |
| Remote approvals | blocked | Requires remote gateway and approval-producing chat run. |
| Remote sessions | blocked | Requires remote gateway with session data. |
| Remote plan, usage, lifecycle, auth, canvas, job, routine, subagent, and cost events | blocked | Requires remote SSE stream. |
| Remote provider vault | blocked | Requires remote gateway provider config. Raw secret reads must remain denied. |
| Remote jobs | blocked | Requires remote gateway job data. Expected Desktop commands are present; unsupported remote job operations must return explicit unavailable reasons. |
| Remote autonomy | blocked | Requires remote gateway autonomy endpoints. Host-executing mutation must remain gated unless remote host policy allows it. |
| Remote experiments and learning | blocked | Requires root gateway endpoints and data. |
| Remote unsupported operations | blocked | Must be checked against the route matrix once a remote gateway is available. |

## P3-W2 Outcome

The repeatable manual smoke checklist was added in
`apps/desktop/documentation/manual-smoke-checklist.md`.

This report records the executable checks and the current manual blockers. It is
not a final product acceptance report: local and remote manual GUI workflows
still need to be run in a configured Tauri environment with at least one working
provider/model, and remote coverage needs a reachable ThinClaw gateway token.
