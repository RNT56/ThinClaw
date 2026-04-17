# Complete Subagent Audit Report

Scope completed:
- Build/debug/log/artifact cleanup requested by user
- Parallel subagent audit across startup/config, channels, runtime, tools/policy, experiments, and cleanup/docs lanes

Cleanup outcome:
- Removed `./target`, `./channels-src/*/target`, and `./tools-src/*/target`
- Removed repo-local `.DS_Store`
- No code/test changes were made during this cleanup pass

## Critical Findings (P1)

1) `src/tools/builtin/extract_document.rs` still allows SSRF-like URL fetches
- User-supplied URLs are fetched directly in `extract_document`, with no call into `MediaUrlConfig` validation.
- `media_url.rs` currently validates literal hosts only and does not handle DNS resolution or IPv4-mapped IPv6 addresses.
- Risk: bypass of media URL hardening via hostnames / mapped IPv6 forms.
- References:
  - [extract_document.rs](/Users/vespian/coding/ThinClaw-main/src/tools/builtin/extract_document.rs:31)
  - [media_url.rs](/Users/vespian/coding/ThinClaw-main/src/safety/media_url.rs:82)

2) Remote experiment finalization can leave leases terminally stuck
- In `src/experiments/runner.rs`, completion attempts are marked as attempted before `/complete` succeeds, and an error path can skip fallback completion.
- In `src/api/experiments.rs`, terminal leases are not rejected in lease verification and can be finalized repeatedly.
- Risk: claimed lease reuse, duplicate cost/runtime updates, stale/stuck terminal state.
- References:
  - [runner.rs](/Users/vespian/coding/ThinClaw-main/src/experiments/runner.rs:183)
  - [runner.rs](/Users/vespian/coding/ThinClaw-main/src/experiments/runner.rs:221)
  - [experiments.rs](/Users/vespian/coding/ThinClaw-main/src/api/experiments.rs:3043)
  - [experiments.rs](/Users/vespian/coding/ThinClaw-main/src/api/experiments.rs:2582)

3) Hot reload cache staleness for WASM channels
- `src/channels/wasm/channel_watcher.rs` and `src/channels/wasm/runtime.rs` still can retain prepared module cache by name on reload, so updated `.wasm` bytes may not recompile.
- Risk: stale runtime behavior after hot-replace.
- References:
  - [channel_watcher.rs](/Users/vespian/coding/ThinClaw-main/src/channels/wasm/channel_watcher.rs:305)
  - [runtime.rs](/Users/vespian/coding/ThinClaw-main/src/channels/wasm/runtime.rs:189)

4) Trusted-proxy auth bypass and transport identity leakage
- `src/channels/web/auth.rs` can derive client source from forgeable `X-Real-IP` when trusted proxy mode is enabled.
- `src/channels/web/handlers/chat.rs` + `src/channels/web/sse.rs` + `src/channels/web/ws.rs` use global event fanout without principal scoping.
- Risk: forged IP can alter auth trust decisions and event cross-actor leakage.
- References:
  - [auth.rs](/Users/vespian/coding/ThinClaw-main/src/channels/web/auth.rs:74)
  - [chat.rs](/Users/vespian/coding/ThinClaw-main/src/channels/web/handlers/chat.rs:40)
  - [sse.rs](/Users/vespian/coding/ThinClaw-main/src/channels/web/sse.rs:29)
  - [ws.rs](/Users/vespian/coding/ThinClaw-main/src/channels/web/ws.rs:76)

5) Gmail delta/history fallback can drop messages
- Fallback unread scan currently can advance history cursor even when no bounded catch-up actually covered all messages.
- Risk: permanent skips across history expiry or cold-start recovery.
- References:
  - [gmail.rs](/Users/vespian/coding/ThinClaw-main/src/channels/gmail.rs:392)
  - [gmail.rs](/Users/vespian/coding/ThinClaw-main/src/channels/gmail.rs:501)
  - [gmail.rs](/Users/vespian/coding/ThinClaw-main/src/channels/gmail.rs:966)

6) Tool policy path remains partially inconsistent
- `ToolPolicyManager` appears loaded from legacy settings path only and not full DB/TOML precedence in some paths.
- Post-compaction context is written but then overridden (`Thread::runtime_state()` hard-codes `post_compaction_context: None`), breaking reinjection.
- Subagent execution enforces policy at exposure time only, not at execution dispatch.
- Risk: drift between configured policy and active enforcement.
- References:
  - [policy.rs](/Users/vespian/coding/ThinClaw-main/src/tools/policy.rs:176)
  - [dispatcher.rs](/Users/vespian/coding/ThinClaw-main/src/agent/dispatcher.rs:421)
  - [subagent_executor.rs](/Users/vespian/coding/ThinClaw-main/src/agent/subagent_executor.rs:1156)
  - [thread_ops.rs](/Users/vespian/coding/ThinClaw-main/src/agent/thread_ops.rs:129)

7) Startup/service edge-case still unsafe
- `service install` can still install `run --no-onboard` even when onboarding is incomplete.
- Gateway boot URL prints token in logs/screen output.
- Risk: unrecoverable crash-looping service installs; token exposure in logs.
- References:
  - [service.rs](/Users/vespian/coding/ThinClaw-main/src/service.rs:48)
  - [service.rs](/Users/vespian/coding/ThinClaw-main/src/service.rs:203)
  - [main.rs](/Users/vespian/coding/ThinClaw-main/src/main.rs:1013)
  - [boot_screen.rs](/Users/vespian/coding/ThinClaw-main/src/boot_screen.rs:205)

## High/Medium Priority Findings (P2)

- WASM webhook policy `require_secret` is not consistently enforced on router boundary; secret-only routes can become publicly reachable.
- HTTP channel docs still mismatch runtime secret location (header vs JSON body contract), despite earlier cleanup.
- Channel telemetry can report stale healthy status with limited failure tracking.
- Gmail cursor fallback path still needs a more conservative and test-proven policy.
- `process_switch_thread` currently relies on in-memory threads and does not always hydrate persisted thread IDs.
- Docs parity: README still classifies channels inconsistently versus runtime docs and architecture docs.

## Test Coverage Gaps (from all lanes)

- No complete run of targeted runtime/regression tests was produced in this pass due cargo lock contention in the environment.
- Remaining highest-value tests:
  - `extract_document` URL hardening enforcement tests
  - watcher hot-reload add/reload/remove + webhook availability
  - experiment completion idempotency and terminal lease finalization
  - tool policy runtime end-to-end (main, subagent, worker, resume)
  - post-compaction context persistence/clear behavior across `/clear`, `/undo`, `/redo`, `/resume`
  - authenticated WS/SSE actor scoping
  - Gmail history cursor progression + expired-history fallback
